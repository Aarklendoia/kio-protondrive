#pragma once

#include <KIO/WorkerBase>
#include <QUrl>

/**
 * KIO worker implementing the protondrive:// protocol.
 *
 * This class is intentionally thin: every method translates a KIO call into
 * one call to the Rust `protondrive-core` library (see core/src/bridge.rs)
 * and converts the result into a KIO::WorkerResult / UDSEntry. All business
 * logic (talking to the `proton-drive` CLI, JSON parsing, path handling)
 * lives in Rust.
 */
class ProtonDriveWorker : public KIO::WorkerBase
{
public:
    ProtonDriveWorker(const QByteArray &protocol, const QByteArray &poolSocket, const QByteArray &appSocket);
    ~ProtonDriveWorker() override;

    KIO::WorkerResult listDir(const QUrl &url) override;
    KIO::WorkerResult stat(const QUrl &url) override;
    KIO::WorkerResult mimetype(const QUrl &url) override;
    KIO::WorkerResult get(const QUrl &url) override;
    KIO::WorkerResult put(const QUrl &url, int permissions, KIO::JobFlags flags) override;
    KIO::WorkerResult mkdir(const QUrl &url, int permissions) override;
    KIO::WorkerResult del(const QUrl &url, bool isFile) override;
    KIO::WorkerResult rename(const QUrl &src, const QUrl &dest, KIO::JobFlags flags) override;

private:
    // Proton Drive posix path for a protondrive:// URL, e.g.
    // "protondrive:/my-files/report.pdf" -> "/my-files/report.pdf".
    // protondrive:// is host-less (like trash:// or recentdocuments://): the
    // path component alone is the address, an empty path means the root "/".
    static QString drivePath(const QUrl &url);

    // Shared by get()'s two sources of file bytes (a pinned local cache hit,
    // or a freshly downloaded temp file) — both stream identically once the
    // bytes are sitting at some local path.
    KIO::WorkerResult streamLocalFile(const QString &localPath, const QString &originalPath);

    // /photos is addressed through a completely separate CLI command family
    // (nodeUid-based, not a real Drive path — see core/src/photos.rs and
    // issue #18), so it needs its own listDir/stat/get paths rather than
    // going through the generic ones above.
    KIO::WorkerResult listPhotos();
    // The web app's `/photos` filter tabs (favorites, screenshots, ...) —
    // see worker/photo_categories.h for the category table and
    // core/src/photos.rs's PhotoCategory for the tag-matching this filters
    // on. `category` is a validated slug (checked against
    // photoCategorySlugs() by every caller before this is reached).
    KIO::WorkerResult listPhotosCategory(const QString &category);
    KIO::WorkerResult statPhoto(const QString &name);
    KIO::WorkerResult getPhoto(const QString &name, const QString &originalPath);
};
