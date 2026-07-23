#include "protondriveworker.h"

#include <KIO/WorkerBase>
#include <KIO/WorkerFactory>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QMimeDatabase>
#include <QTemporaryDir>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
// Everything the Rust side raises for an Err(String) arrives here as a
// thrown rust::Error (that's how cxx surfaces a bridged Result::Err on the
// C++ side) — turned into the KIO error the caller actually sees.
// DriveError::NotFound stringifies (via thiserror, see core/src/cli.rs) as
// "path not found: ...", which is the only case worth a specific KIO error
// code; everything else becomes a worker-defined error carrying the raw
// message so the user still sees *why* it failed.
KIO::WorkerResult resultFromRustError(const rust::Error &error)
{
    const QString message = QString::fromUtf8(error.what());
    if (message.startsWith(QLatin1String("path not found:"))) {
        return KIO::WorkerResult::fail(KIO::ERR_DOES_NOT_EXIST, message);
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
    try {
        const rust::Vec<FfiEntry> entries = list_dir(path.toStdString());
        for (const FfiEntry &entry : entries) {
            listEntry(entryFromFfi(entry));
        }
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
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

// KIO workers are Qt plugins loaded by the generic `kioworker` host process
// (see KDE/kio-admin's src/worker.cpp for the reference pattern this
// mirrors): the JSON below is embedded into the compiled plugin's Qt
// metadata by moc, so KIO can discover the "protondrive" protocol without
// any separate file being installed at runtime.
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

#include "protondriveworker.moc"
