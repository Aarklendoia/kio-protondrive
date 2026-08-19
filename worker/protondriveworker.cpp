#include "protondriveworker.h"

#include <KIO/WorkerBase>
#include <KIO/WorkerFactory>
#include <KLocalizedString>
#include <QCoreApplication>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QDateTime>
#include <QDebug>
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
// DriveError::NotFound/::NotAuthenticated/::Timeout stringify (via thiserror,
// see core/src/cli.rs) as "path not found: ...", "not logged in to Proton
// Drive — ...", "proton-drive did not respond within ..." — the only cases
// worth a specific KIO error code; everything else becomes a worker-defined
// error carrying the raw message so the user still sees *why* it failed.
// Split out from resultFromRustError() below so the same message-prefix
// mapping applies to errors surfaced through FfiTransferPoll::error (a plain
// rust::String from a poll_transfer() call, not a thrown rust::Error) as to
// every other Rust call site here.
KIO::WorkerResult resultFromErrorMessage(const QString &message)
{
    if (message.startsWith(QLatin1String("path not found:"))) {
        return KIO::WorkerResult::fail(KIO::ERR_DOES_NOT_EXIST, message);
    }
    if (message.startsWith(QLatin1String("not logged in to Proton Drive"))) {
        // KIO's own "could not log in" dialog — actionable (the message
        // tells the user to run `proton-drive auth login`), unlike the
        // generic ERR_WORKER_DEFINED fallback below. Translated rather than
        // passed through as-is (unlike the other branches here): this one is
        // a fixed, user-facing instruction, not data-carrying like a path or
        // a raw CLI message, so it's worth localizing like the section names
        // below.
        return KIO::WorkerResult::fail(
            KIO::ERR_CANNOT_LOGIN,
            i18nd("kio_protondrive", "Not logged in to Proton Drive. Run \"proton-drive auth login\" in a terminal, then try again."));
    }
    if (message.startsWith(QLatin1String("proton-drive did not respond within"))) {
        return KIO::WorkerResult::fail(KIO::ERR_SERVER_TIMEOUT, message);
    }
    // "a file or folder with this name already exists" isn't handled here:
    // the only caller that can produce it is mkdir(), which treats it as
    // success rather than an error at all — see its own comment for why.
    return KIO::WorkerResult::fail(KIO::ERR_WORKER_DEFINED, message);
}

KIO::WorkerResult resultFromRustError(const rust::Error &error)
{
    return resultFromErrorMessage(QString::fromUtf8(error.what()));
}

QString toQString(const rust::String &value)
{
    return QString::fromUtf8(value.data(), static_cast<int>(value.size()));
}

// Tells protondrive_overlayicon.so (see overlayplugin.cpp) that
// remotePath's locally-available/pinned status may have changed, so it
// repaints just that item's badge instead of waiting for the view's next
// unrelated refresh. Same signal daemon/src/control.rs's notify_pin_changed
// already sends after a pin/unpin — reused here (issue #60) for "this path
// just got opportunistically cached" too, since both are "an overlay-
// relevant state changed" from the plugin's point of view. Best-effort:
// no session bus (e.g. inside a container) just means the badge goes stale
// until the next natural refresh, not a failure worth surfacing.
void notifyOverlayChanged(const QString &remotePath)
{
    QDBusMessage message = QDBusMessage::createSignal(QStringLiteral("/"), QStringLiteral("org.kde.protondrive.OverlayIcon"), QStringLiteral("PinChanged"));
    message << remotePath;
    QDBusConnection::sessionBus().send(message);
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
// labels Proton Drive's own web UI uses. "photos" is browsable (see
// listPhotos(), routed through a separate nodeUid-based CLI command family —
// see core/src/photos.rs and issue #18); the last four (albums and the
// photos-* variants) aren't shown as top-level sidebar entries in the web UI
// either — they're nested under its "Photos" view and still have no CLI
// support at all (see the README's Scope section) — so their translations
// are this project's best guess, not confirmed against Proton's own wording.
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

// Breeze icon names for the same fixed root sections translatedSectionName()
// labels — confirmed present in this theme's own icon set (as opposed to
// guessed freedesktop-spec names that might not resolve to anything and
// silently fall back to a generic folder). "folder-cloud" matches
// wizard::route_add_favorite's own choice for the protondrive:/ Places
// bookmark, so the root itself and "My files" read as the same concept.
// Several sections deliberately share an icon (both trash variants, all
// four shared/shared-photos variants) — the display *name* already
// disambiguates those, the icon only needs to signal "trash-like" or
// "sharing-like" at a glance.
QString translatedSectionIcon(const QString &rawName)
{
    static const QHash<QString, QString> icons = {
        {QStringLiteral("my-files"), QStringLiteral("folder-cloud")},
        {QStringLiteral("devices"), QStringLiteral("computer")},
        {QStringLiteral("photos"), QStringLiteral("folder-pictures")},
        {QStringLiteral("shared-by-me"), QStringLiteral("folder-publicshare")},
        {QStringLiteral("shared-with-me"), QStringLiteral("folder-publicshare")},
        {QStringLiteral("trash"), QStringLiteral("user-trash")},
        {QStringLiteral("albums"), QStringLiteral("folder-favorites")},
        {QStringLiteral("photos-shared-by-me"), QStringLiteral("folder-publicshare")},
        {QStringLiteral("photos-shared-with-me"), QStringLiteral("folder-publicshare")},
        {QStringLiteral("photos-trash"), QStringLiteral("user-trash")},
    };
    return icons.value(rawName);
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

const QString photosPrefix = QStringLiteral("/photos/");
const QString trashPrefix = QStringLiteral("/trash/");

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

    if (path == QLatin1String("/photos")) {
        return listPhotos();
    }

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
            const QString rawName = toQString(entry.name);
            const QString label = translatedSectionName(rawName);
            if (!label.isEmpty()) {
                uds.fastInsert(KIO::UDSEntry::UDS_DISPLAY_NAME, label);
            }
            const QString icon = translatedSectionIcon(rawName);
            if (!icon.isEmpty()) {
                uds.fastInsert(KIO::UDSEntry::UDS_ICON_NAME, icon);
            }
        }
        listEntry(uds);
    }
    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::stat(const QUrl &url)
{
    const QString path = drivePath(url);

    if (path.startsWith(photosPrefix)) {
        return statPhoto(path.mid(photosPrefix.length()));
    }
    if (path == QLatin1String("/photos")) {
        // No CLI-backed info call exists for the section itself (see
        // listPhotos()'s "." entry) — synthesized directly, same as the
        // pinned-cache fast path below does for a pinned file.
        KIO::UDSEntry uds;
        uds.reserve(5);
        uds.fastInsert(KIO::UDSEntry::UDS_NAME, QStringLiteral("."));
        uds.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, S_IFDIR);
        uds.fastInsert(KIO::UDSEntry::UDS_ACCESS, 0755);
        uds.fastInsert(KIO::UDSEntry::UDS_DISPLAY_NAME, translatedSectionName(QStringLiteral("photos")));
        uds.fastInsert(KIO::UDSEntry::UDS_ICON_NAME, translatedSectionIcon(QStringLiteral("photos")));
        statEntry(uds);
        return KIO::WorkerResult::pass();
    }

    // Same pinned-cache short-circuit as get() — a pinned file's size/mtime
    // come from its local copy instantly rather than a CLI round-trip.
    // Only a genuine cache hit takes this path; any error opening/querying
    // the pin cache (e.g. an unwritable $XDG_DATA_HOME, a corrupted index)
    // falls through to the normal stat_path() below instead of failing the
    // whole call — pinning is a best-effort accelerator on top of on-demand
    // browsing, not something browsing protondrive:/ should depend on.
    try {
        const rust::String pinned = lookup_pin(path.toStdString());
        if (!pinned.empty()) {
            const QString localPath = QString::fromUtf8(pinned.data(), static_cast<int>(pinned.size()));
            const QFileInfo info(localPath);
            KIO::UDSEntry uds;
            uds.reserve(4);
            uds.fastInsert(KIO::UDSEntry::UDS_NAME, QStringLiteral("."));
            uds.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, S_IFREG);
            uds.fastInsert(KIO::UDSEntry::UDS_ACCESS, 0644);
            uds.fastInsert(KIO::UDSEntry::UDS_SIZE, static_cast<long long>(info.size()));
            uds.fastInsert(KIO::UDSEntry::UDS_MODIFICATION_TIME, info.lastModified().toSecsSinceEpoch());
            // The "pinned" checkmark itself is protondrive_overlayicon.so's
            // job now (see overlayplugin.cpp) — kept out of this UDSEntry
            // since a re-fetched entry's overlay field turned out not to
            // reliably repaint an already-visible item in Dolphin (confirmed
            // live: the daemon's KDirNotify signal made Dolphin visibly
            // refresh, but the checkmark never actually updated either way).
            statEntry(uds);
            return KIO::WorkerResult::pass();
        }
    } catch (const rust::Error &error) {
        qWarning() << "pin cache lookup failed for" << path << "(falling back to a normal stat):" << error.what();
    }

    // KIO's convention for the entry describing the URL itself (as opposed
    // to an entry inside a directory listing) is to name it "." rather than
    // repeating the full path.
    try {
        const FfiEntry entry = stat_path(path.toStdString());
        KIO::UDSEntry uds = entryFromFfi(entry, QStringLiteral("."));
        // KUrlNavigator's breadcrumb label for the *current* directory is
        // read from here, not from the sibling entry listDir() emits for it
        // — without this, browsing into /my-files shows "Mes fichiers" in
        // the icon grid but reverts to the raw "my-files" in the breadcrumb
        // once you're inside it. A virtual root section is always exactly
        // one path segment deep (e.g. "/my-files", never "/my-files/sub").
        if (path.count(QLatin1Char('/')) == 1) {
            const QString rawName = path.mid(1);
            const QString label = translatedSectionName(rawName);
            if (!label.isEmpty()) {
                uds.fastInsert(KIO::UDSEntry::UDS_DISPLAY_NAME, label);
            }
            const QString icon = translatedSectionIcon(rawName);
            if (!icon.isEmpty()) {
                uds.fastInsert(KIO::UDSEntry::UDS_ICON_NAME, icon);
            }
        }
        statEntry(uds);
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::streamLocalFile(const QString &localPath, const QString &originalPath)
{
    QFile file(localPath);
    if (!file.open(QIODevice::ReadOnly)) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_READ, originalPath);
    }

    mimeType(QMimeDatabase().mimeTypeForFile(file.fileName()).name());
    totalSize(static_cast<KIO::filesize_t>(file.size()));

    constexpr qint64 chunkSize = 256 * 1024;
    while (!file.atEnd()) {
        if (wasKilled()) {
            return KIO::WorkerResult::fail(KIO::ERR_USER_CANCELED, originalPath);
        }
        data(file.read(chunkSize));
    }
    data(QByteArray());
    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::mimetype(const QUrl &url)
{
    const QString path = drivePath(url);

    if (path.startsWith(photosPrefix)) {
        try {
            const FfiEntry entry = stat_photo(path.mid(photosPrefix.length()).toStdString());
            const QString mediaType = toQString(entry.media_type);
            mimeType(mediaType.isEmpty() ? QMimeDatabase().mimeTypeForFile(path, QMimeDatabase::MatchExtension).name() : mediaType);
            return KIO::WorkerResult::pass();
        } catch (const rust::Error &error) {
            return resultFromRustError(error);
        }
    }

    try {
        const FfiEntry entry = stat_path(path.toStdString());
        if (entry.is_folder) {
            mimeType(QStringLiteral("inode/directory"));
        } else {
            const QString mediaType = toQString(entry.media_type);
            // Proton Drive doesn't always report a media type (e.g. files
            // uploaded by other clients) — fall back to a filename-only
            // guess rather than the full download() a generic get() would
            // require just to sniff the content.
            mimeType(mediaType.isEmpty() ? QMimeDatabase().mimeTypeForFile(path, QMimeDatabase::MatchExtension).name() : mediaType);
        }
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::get(const QUrl &url)
{
    const QString path = drivePath(url);

    if (path.startsWith(photosPrefix)) {
        return getPhoto(path.mid(photosPrefix.length()), path);
    }

    // Pinned files (kept local via the Dolphin "Garder en local" ServiceMenu
    // action — see issue #30) are served straight from their cached copy:
    // no CLI call at all, instant instead of a network round-trip every
    // time this path is opened. Same fall-through-on-error reasoning as
    // stat()'s identical lookup_pin() try block: a broken pin cache
    // shouldn't take down normal (unpinned) downloads with it.
    try {
        const rust::String pinned = lookup_pin(path.toStdString());
        if (!pinned.empty()) {
            return streamLocalFile(QString::fromUtf8(pinned.data(), static_cast<int>(pinned.size())), path);
        }
    } catch (const rust::Error &error) {
        qWarning() << "pin cache lookup failed for" << path << "(falling back to a normal download):" << error.what();
    }

    // Opportunistic cache (#60): a file downloaded once stays available
    // locally afterward instead of being deleted the moment this call
    // returns, until the daemon's retention sweep evicts it. lookup_cached()
    // itself re-verifies against the remote's modification time before
    // trusting the local copy — see core/src/bridge.rs's doc comment on why
    // that check exists here but not for the pinned path above.
    try {
        const rust::String cached = lookup_cached(path.toStdString());
        if (!cached.empty()) {
            return streamLocalFile(QString::fromUtf8(cached.data(), static_cast<int>(cached.size())), path);
        }
    } catch (const rust::Error &error) {
        qWarning() << "opportunistic cache lookup failed for" << path << "(falling back to a normal download):" << error.what();
    }

    const QString fileName = QFileInfo(path).fileName();

    QTemporaryDir tmpDir;
    if (!tmpDir.isValid()) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_READ, QStringLiteral("could not create a temporary directory"));
    }

    // Best-effort: gives totalSize()/the progress estimate a denominator and,
    // on success, the modification_time recorded alongside the cached copy
    // below — a failure here doesn't abort the download itself, since a real
    // problem (e.g. the path genuinely not existing) will surface from
    // start_download() below anyway. Already cheap thanks to the fs cache
    // (#8).
    KIO::filesize_t totalBytes = 0;
    QString modificationTime;
    try {
        const FfiEntry entry = stat_path(path.toStdString());
        totalBytes = entry.size;
        modificationTime = toQString(entry.modification_time);
    } catch (const rust::Error &) {
    }
    totalSize(totalBytes);

    // Downloads straight into the persistent, mirrored cache directory
    // rather than the temporary one above, so the file survives past this
    // call — falls back to the temporary directory only if the cache
    // directory itself can't be determined/created (e.g. an unwritable
    // XDG_CACHE_HOME), in which case this open just behaves like it always
    // did before #60.
    QString downloadDir = tmpDir.path();
    bool cachingEnabled = false;
    try {
        const rust::String dir = cache_target_dir(path.toStdString());
        downloadDir = QString::fromUtf8(dir.data(), static_cast<int>(dir.size()));
        cachingEnabled = true;
    } catch (const rust::Error &error) {
        qWarning() << "cache directory lookup failed for" << path << "(falling back to a temporary download):" << error.what();
    }

    try {
        rust::Box<TransferHandle> handle = start_download(path.toStdString(), downloadDir.toStdString(), totalBytes);
        while (true) {
            if (wasKilled()) {
                cancel_transfer(*handle);
                if (cachingEnabled) {
                    QFile::remove(QDir(downloadDir).filePath(fileName));
                }
                return KIO::WorkerResult::fail(KIO::ERR_USER_CANCELED, path);
            }
            const FfiTransferPoll poll = poll_transfer(*handle);
            if (poll.done) {
                if (!poll.ok) {
                    if (cachingEnabled) {
                        QFile::remove(QDir(downloadDir).filePath(fileName));
                    }
                    return resultFromErrorMessage(toQString(poll.error));
                }
                break;
            }
            processedSize(poll.processed_bytes);
        }
    } catch (const rust::Error &error) {
        if (cachingEnabled) {
            QFile::remove(QDir(downloadDir).filePath(fileName));
        }
        return resultFromRustError(error);
    }

    const QString downloadedPath = QDir(downloadDir).filePath(fileName);

    if (cachingEnabled && !modificationTime.isEmpty()) {
        try {
            store_cached(path.toStdString(), downloadedPath.toStdString(), modificationTime.toStdString());
            notifyOverlayChanged(path);
        } catch (const rust::Error &error) {
            qWarning() << "failed to record" << path << "in the opportunistic cache:" << error.what();
        }
    }

    return streamLocalFile(downloadedPath, path);
}

KIO::WorkerResult ProtonDriveWorker::listPhotos()
{
    rust::Vec<FfiEntry> entries;
    try {
        entries = list_photos();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    // No CLI-backed "." info call exists for this synthetic section (see
    // stat()'s equivalent special case for /photos itself) — synthesized
    // directly rather than skipped, since unlike listDir()'s generic path
    // there's no real stat_path() fallback to attempt first.
    KIO::UDSEntry self;
    self.reserve(3);
    self.fastInsert(KIO::UDSEntry::UDS_NAME, QStringLiteral("."));
    self.fastInsert(KIO::UDSEntry::UDS_FILE_TYPE, S_IFDIR);
    self.fastInsert(KIO::UDSEntry::UDS_ACCESS, 0755);
    listEntry(self);

    for (const FfiEntry &entry : entries) {
        listEntry(entryFromFfi(entry));
    }
    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::statPhoto(const QString &name)
{
    try {
        const FfiEntry entry = stat_photo(name.toStdString());
        statEntry(entryFromFfi(entry, QStringLiteral(".")));
        return KIO::WorkerResult::pass();
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::getPhoto(const QString &name, const QString &originalPath)
{
    QTemporaryDir tmpDir;
    if (!tmpDir.isValid()) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_READ, QStringLiteral("could not create a temporary directory"));
    }

    // The CLI names the downloaded file by its own decrypted name, which
    // isn't guaranteed to equal `name` (see core/src/photos.rs's " (2)"
    // suffix for same-named photos) — download_photo() reads back whatever
    // actually landed in tmpDir rather than assuming it matches.
    rust::String downloadedName;
    try {
        downloadedName = download_photo(name.toStdString(), tmpDir.path().toStdString());
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    return streamLocalFile(tmpDir.filePath(toQString(downloadedName)), originalPath);
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
        // dataReq() must be sent before each readData() call — it's what
        // actually asks the job for the next chunk. Without it, readData()
        // only ever sees whatever the job sent unprompted, which happens to
        // cover an entire tiny file but leaves both sides blocked forever on
        // anything bigger (confirmed live: uploads under ~1 MB completed,
        // larger ones hung indefinitely with zero bytes ever written).
        if (wasKilled()) {
            file.close();
            return KIO::WorkerResult::fail(KIO::ERR_USER_CANCELED, path);
        }
        dataReq();
        QByteArray chunk;
        result = readData(chunk);
        if (result < 0) {
            return KIO::WorkerResult::fail(KIO::ERR_CANNOT_WRITE, path);
        }
        file.write(chunk);
    } while (result > 0);
    file.close();

    const KIO::filesize_t totalBytes = static_cast<KIO::filesize_t>(QFileInfo(file).size());

    try {
        rust::Box<TransferHandle> handle = start_upload(file.fileName().toStdString(), parentPath.toStdString(), totalBytes);
        while (true) {
            if (wasKilled()) {
                cancel_transfer(*handle);
                return KIO::WorkerResult::fail(KIO::ERR_USER_CANCELED, path);
            }
            const FfiTransferPoll poll = poll_transfer(*handle);
            if (poll.done) {
                if (!poll.ok) {
                    return resultFromErrorMessage(toQString(poll.error));
                }
                break;
            }
            processedSize(poll.processed_bytes);
        }
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    // Opportunistic cache (#60): the bytes just uploaded are already sitting
    // right here in `file` — copying them into the persistent cache
    // directory means "save, then immediately reopen" is instant too,
    // instead of `tmpDir` discarding them the moment this call returns.
    // Best-effort throughout: a failure at any step here just means this
    // particular save doesn't get cached, not a failed upload — the upload
    // itself already succeeded above.
    try {
        const rust::String dir = cache_target_dir(path.toStdString());
        const QString cacheDir = QString::fromUtf8(dir.data(), static_cast<int>(dir.size()));
        const QString cachedPath = QDir(cacheDir).filePath(fileName);
        QFile::remove(cachedPath); // QFile::copy refuses to overwrite an existing file.
        if (QFile::copy(file.fileName(), cachedPath)) {
            // TransferSummary (from finish_upload) doesn't carry per-file
            // metadata, so the just-committed modification_time needs its
            // own stat — cheap, and freshly invalidated by poll_transfer's
            // own cache handling above.
            const FfiEntry entry = stat_path(path.toStdString());
            store_cached(path.toStdString(), cachedPath.toStdString(), entry.modification_time);
            notifyOverlayChanged(path);
        }
    } catch (const rust::Error &error) {
        qWarning() << "failed to record" << path << "in the opportunistic cache:" << error.what();
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
        const QString message = QString::fromUtf8(error.what());
        if (message.startsWith(QLatin1String("a file or folder with this name already exists"))) {
            // Tried mapping this to ERR_DIR_ALREADY_EXIST and letting
            // KIO::CopyJob's own merge handling take it from there — verified
            // live that it doesn't: kioclient still surfaced a hard "a folder
            // named ... already exists" dialog for a plain folder copy into
            // an existing destination. Treating "already there" as success
            // instead (this call's whole point was making sure the folder
            // exists, which it does) is what actually makes the copy proceed
            // into the existing folder, confirmed live.
            return KIO::WorkerResult::pass();
        }
        return resultFromRustError(error);
    }
}

KIO::WorkerResult ProtonDriveWorker::rename(const QUrl &src, const QUrl &dest, KIO::JobFlags flags)
{
    const QString oldPath = drivePath(src);
    const QString newPath = drivePath(dest);

    // /photos is addressed through a completely separate, read-only CLI
    // command family (see core/src/photos.rs) — there's no rename/move
    // support for it at all, and no combined path space with the rest of
    // Proton Drive to move into or out of it.
    if (oldPath.startsWith(photosPrefix) || newPath.startsWith(photosPrefix)) {
        return KIO::WorkerResult::fail(KIO::ERR_CANNOT_RENAME, i18nd("kio_protondrive", "Photos can't be renamed or moved."));
    }

    // KIO's contract for rename(): "for performance reasons no stat is done
    // in the destination beforehand, the worker must do it" — and only
    // needs to when Overwrite wasn't requested, since that's the only case
    // where an existing destination should block the rename.
    if (!flags.testFlag(KIO::Overwrite)) {
        try {
            const FfiEntry existing = stat_path(newPath.toStdString());
            return KIO::WorkerResult::fail(existing.is_folder ? KIO::ERR_DIR_ALREADY_EXIST : KIO::ERR_FILE_ALREADY_EXIST, newPath);
        } catch (const rust::Error &error) {
            const QString message = QString::fromUtf8(error.what());
            if (!message.startsWith(QLatin1String("path not found:"))) {
                // Some other failure (auth, timeout, ...) trying to check —
                // don't silently proceed as if the destination were free.
                return resultFromRustError(error);
            }
            // Destination doesn't exist — clear to proceed.
        }
    }

    try {
        rename_or_move(oldPath.toStdString(), newPath.toStdString());
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }
    return KIO::WorkerResult::pass();
}

KIO::WorkerResult ProtonDriveWorker::del(const QUrl &url, bool /*isFile*/)
{
    const QString path = drivePath(url);
    try {
        // An item already under /trash has nowhere further to be
        // soft-deleted to — the CLI's own `filesystem delete` refuses
        // anything not already trashed, so this is the only correct
        // direction: permanently delete instead of trashing again (which
        // previously just failed/no-opped, see #7).
        if (path.startsWith(trashPrefix)) {
            permanently_delete_path(path.toStdString());
        } else {
            trash(path.toStdString());
        }
    } catch (const rust::Error &error) {
        return resultFromRustError(error);
    }

    // Best-effort: if this path was pinned, drop its now-stale local cache
    // copy — without this, stat()/get() keep serving it from the pin cache
    // indefinitely after the remote it came from is gone (Cache::lookup()
    // only checks local file existence, never remote validity). Forced
    // unconditionally: the remote is already trashed, so there's no longer
    // an "upload local edits first" option to protect by refusing on dirty
    // local content (see Cache::unpin's normal, non-forced guard).
    try {
        unpin_path(path.toStdString(), true);
    } catch (const rust::Error &error) {
        qWarning() << "could not unpin" << path << "after trashing it:" << error.what();
    }
    return KIO::WorkerResult::pass();
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
