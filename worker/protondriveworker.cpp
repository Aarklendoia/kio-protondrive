#include "protondriveworker.h"

#include <KIO/WorkerBase>
#include <KIO/WorkerFactory>
#include <KLocalizedString>
#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QHash>
#include <QMimeDatabase>
#include <QTemporaryDir>

#include <cstdio>
#include <cstdlib>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
// Everything the Rust side raises for an Err(String) arrives here as a
// thrown rust::Error (that's how cxx surfaces a bridged Result::Err on the
// C++ side) — turned into the KIO error the caller actually sees.
// DriveError::NotFound/::Timeout stringify (via thiserror, see
// core/src/cli.rs) as "path not found: ..."/"proton-drive did not respond
// within ...", the only two cases worth a specific KIO error code; everything
// else becomes a worker-defined error carrying the raw message so the user
// still sees *why* it failed.
KIO::WorkerResult resultFromRustError(const rust::Error &error)
{
    const QString message = QString::fromUtf8(error.what());
    if (message.startsWith(QLatin1String("path not found:"))) {
        return KIO::WorkerResult::fail(KIO::ERR_DOES_NOT_EXIST, message);
    }
    if (message.startsWith(QLatin1String("proton-drive did not respond within"))) {
        return KIO::WorkerResult::fail(KIO::ERR_SERVER_TIMEOUT, message);
    }
    return KIO::WorkerResult::fail(KIO::ERR_WORKER_DEFINED, message);
}

QString toQString(const rust::String &value)
{
    return QString::fromUtf8(value.data(), static_cast<int>(value.size()));
}

KIO::UDSEntry entryFromFfi(const FfiEntry &entry, const QString &nameOverride = QString())
{
    KIO::UDSEntry uds;
    uds.reserve(6);
    uds.fastInsert(KIO::UDSEntry::UDS_NAME, nameOverride.isEmpty() ? toQString(entry.name) : nameOverride);
    uds.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, entry.is_folder ? S_IFDIR : S_IFREG);
    uds.fastInsert(KIO::UDSEntry::UDS_ACCESS, entry.is_folder ? 0755 : 0644);

    if (!entry.is_folder) {
        uds.fastInsert(KIO::UDSEntry::UDS_SIZE, static_cast<long long>(entry.size));
    }

    const QString mediaType = toQString(entry.media_type);
    if (!mediaType.isEmpty()) {
        uds.fastInsert(KIO::UDSEntry::UDS_MIME_TYPE, mediaType);
    }

    const QString modificationTime = toQString(entry.modification_time);
    if (!modificationTime.isEmpty()) {
        const QDateTime dt = QDateTime::fromString(modificationTime, Qt::ISODate);
        if (dt.isValid()) {
            uds.fastInsert(KIO::UDSEntry::UDS_MODIFICATION_TIME, dt.toSecsSinceEpoch());
        }
    }

    return uds;
}

// Proton Drive's virtual root sections (`filesystem list -j /`) have fixed,
// English, machine-oriented path segments — translated here to match the
// labels Proton Drive's own web UI uses. The last four (albums and the
// photos-* variants) aren't shown as top-level sidebar entries there —
// they're nested under a "Photos" view the underlying CLI doesn't support
// browsing yet (see the README's Scope section) — so their translations are
// this project's best guess, not confirmed against Proton's own wording.
QString translatedSectionName(const QString &rawName)
{
    static const QHash<QString, QString> labels = {
        {QStringLiteral("my-files"), i18nd("kio_protondrive", "My files")},
        {QStringLiteral("devices"), i18nd("kio_protondrive", "Computers")},
        {QStringLiteral("photos"), i18nd("kio_protondrive", "Photos")},
        {QStringLiteral("shared-by-me"), i18nd("kio_protondrive", "Shared")},
        {QStringLiteral("shared-with-me"), i18nd("kio_protondrive", "Shared with me")},
        {QStringLiteral("trash"), i18nd("kio_protondrive", "Trash")},
        {QStringLiteral("albums"), i18nd("kio_protondrive", "Albums")},
        {QStringLiteral("photos-shared-by-me"), i18nd("kio_protondrive", "Photos shared by me")},
        {QStringLiteral("photos-shared-with-me"), i18nd("kio_protondrive", "Photos shared with me")},
        {QStringLiteral("photos-trash"), i18nd("kio_protondrive", "Photos trash")},
    };
    return labels.value(rawName);
}

// Strips a trailing slash left by QUrl::adjusted(QUrl::RemoveFilename) —
// the `proton-drive` CLI expects parent paths without one (e.g. "/my-files",
// never "/my-files/").
QString stripTrailingSlash(QString path)
{
    if (path.length() > 1 && path.endsWith(QLatin1Char('/'))) {
        path.chop(1);
    }
    return path;
}

}

ProtonDriveWorker::ProtonDriveWorker(const QByteArray &protocol, const QByteArray &poolSocket, const QByteArray &appSocket)
    : KIO::WorkerBase(protocol, poolSocket, appSocket)
{
}

ProtonDriveWorker::~ProtonDriveWorker() = default;

QString ProtonDriveWorker::drivePath(const QUrl &url)
{
    const QString path = url.path();
    return path.isEmpty() ? QStringLiteral("/") : path;
}

KIO::WorkerResult ProtonDriveWorker::listDir(const QUrl &url)
{
    const QString path = drivePath(url);

    rust::Vec<FfiEntry> entries;
    try {
        entries = list_dir(path.toStdString());
    } catch (const rust::Error &error) {
        // Fails fast for unsupported paths (e.g. the CLI has no listing
        // support for the `/photos` virtual section) — checked before the
        // "." stat below, which for some of those same unsupported paths
        // hangs instead of erroring quickly (rather than failing the whole
        // listing on a slow, ultimately pointless stat).
        return resultFromRustError(error);
    }

    // KIO expects a "." entry describing the listed directory itself (used
    // for e.g. the item count/permissions of the folder being browsed) —
    // without it, KIO::WorkerBase logs "UDSEntry for '.' not found, creating
    // a default one" and falls back to a stub. Best-effort: `filesystem info`
    // doesn't support the virtual root's sections (`/`, `/my-files`, ...), so
    // skip the "." entry rather than failing the whole listing when it's
    // unavailable — list_dir() above having already succeeded is what makes
    // this safe to still attempt here.
    try {
        const FfiEntry self = stat_path(path.toStdString());
        listEntry(entryFromFfi(self, QStringLiteral(".")));
    } catch (const rust::Error &) {
    }

    // The virtual root's entries are Proton Drive's fixed sections
    // (my-files, devices, ...), never real user content — see entry.rs's
    // ListItem: `filesystem list` returns one shape or the other, never a
    // mix — so translating by raw name can't misfire on a real folder a
    // user happened to name e.g. "trash".
    const bool isVirtualRoot = path == QLatin1String("/");
    for (const FfiEntry &entry : entries) {
        KIO::UDSEntry uds = entryFromFfi(entry);
        if (isVirtualRoot) {
            const QString label = translatedSectionName(toQString(entry.name));
            if (!label.isEmpty()) {
                uds.fastInsert(KIO::UDSEntry::UDS_DISPLAY_NAME, label);
            }
        }
        listEntry(uds);
    }
    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::stat(const QUrl &url)
{
    const QString path = drivePath(url);
    // KIO's convention for the entry describing the URL itself (as opposed
    // to an entry inside a directory listing) is to name it "." rather than
    // repeating the full path.
    try {
        const FfiEntry entry = stat_path(path.toStdString());
        statEntry(entryFromFfi(entry, QStringLiteral(".")));
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::get(const QUrl &url)
{
    const QString path = drivePath(url);
    const QString fileName = QFileInfo(path).fileName();

    QTemporaryDir tmpDir;
    if (!tmpDir.isValid()) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_READ, QStringLiteral("could not create a temporary directory"));
    }

    try {
        download_to(path.toStdString(), tmpDir.path().toStdString());
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    QFile file(tmpDir.filePath(fileName));
    if (!file.open(QIODevice::ReadOnly)) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_READ, path);
    }

    mimeType(QMimeDatabase().mimeTypeForFile(file.fileName()).name());
    totalSize(static_cast<KIO::filesize_t>(file.size()));

    constexpr qint64 chunkSize = 256 * 1024;
    while (!file.atEnd()) {
        data(file.read(chunkSize));
    }
    data(QByteArray());

    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::put(const QUrl &url, int /*permissions*/, KIO::JobFlags /*flags*/)
{
    const QString path = drivePath(url);
    const QString fileName = QFileInfo(path).fileName();
    const QString parentPath = stripTrailingSlash(drivePath(url.adjusted(QUrl::RemoveFilename)));

    QTemporaryDir tmpDir;
    if (!tmpDir.isValid()) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_WRITE, QStringLiteral("could not create a temporary directory"));
    }

    QFile file(tmpDir.filePath(fileName));
    if (!file.open(QIODevice::WriteOnly)) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_WRITE, path);
    }

    int result = 0;
    do {
        QByteArray chunk;
        result = readData(chunk);
        if (result < 0) {
            return KIO::WorkerResult::fail(KIO::ERR_CANNOT_WRITE, path);
        }
        file.write(chunk);
    } while (result > 0);
    file.close();

    try {
        upload_from(file.fileName().toStdString(), parentPath.toStdString());
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::mkdir(const QUrl &url, int /*permissions*/)
{
    const QString name = QFileInfo(drivePath(url)).fileName();
    const QString parentPath = stripTrailingSlash(drivePath(url.adjusted(QUrl::RemoveFilename)));

    try {
        make_dir(parentPath.toStdString(), name.toStdString());
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::del(const QUrl &url, bool /*isFile*/)
{
    const QString path = drivePath(url);
    try {
        trash(path.toStdString());
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

// The JSON below is embedded into the compiled plugin's Qt metadata by moc,
// so KIO can discover which protocol(s) this plugin's .so supports without
// any separate file being installed at runtime — used for the in-process
// KIOWORKER_ENABLE_TESTMODE path and for protocol->library resolution.
// Actual out-of-process launches go through the kdemain() entry point below
// instead: the generic `kioworker` host process dlopen()s this plugin and
// calls kdemain() directly (the same convention every other in-tree KIO
// worker follows, e.g. KDE/kio-extras's sftp/kio_sftp.cpp) rather than
// instantiating this factory.
class ProtonDriveWorkerFactory : public KIO::WorkerFactory
{
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.kde.kio.worker.protondrive" FILE "protondrive.json")

public:
    std::unique_ptr<KIO::WorkerBase> createWorker(const QByteArray &pool, const QByteArray &app) override
    {
        qRegisterMetaType<KIO::UDSEntry>("KIO::UDSEntry");
        qRegisterMetaType<KIO::UDSEntryList>("KIO::UDSEntryList");
        return std::make_unique<ProtonDriveWorker>(QByteArrayLiteral("protondrive"), pool, app);
    }
};

extern "C" {
int Q_DECL_EXPORT kdemain(int argc, char **argv)
{
    QCoreApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("kio_protondrive"));

    if (argc != 4) {
        fprintf(stderr, "Usage: kio_protondrive protocol domain-socket1 domain-socket2\n");
        exit(-1);
    }

    ProtonDriveWorker worker(argv[1], argv[2], argv[3]);
    worker.dispatchLoop();
    return 0;
}
}

#include "protondriveworker.moc"
