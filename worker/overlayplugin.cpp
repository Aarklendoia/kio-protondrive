#include <KOverlayIconPlugin>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QUrl>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
QStringList overlaysFor(const QString &remotePath)
{
    // Best-effort, same stance as every lookup_pin() call in
    // protondriveworker.cpp — a cache read failure (e.g. an unwritable
    // $XDG_DATA_HOME) just means "no overlay shown", not an error worth
    // surfacing from an icon-decoration hook.
    try {
        const rust::String pinned = lookup_pin(remotePath.toStdString());
        if (!pinned.empty()) {
            return {QStringLiteral("emblem-checked")};
        }
    } catch (const rust::Error &) {
    }
    return {};
}
}

// KDE's purpose-built mechanism for a file manager-decorated sync-status
// icon (the same one Nextcloud's and ownCloud's desktop clients use):
// Dolphin calls getOverlays(url) on-demand, decoupled from whatever
// caching/merging its item model does internally to KIO's own stat/listDir
// results — which turned out to matter, since the KIO worker's own
// UDS_ICON_OVERLAY_NAMES field (see protondriveworker.cpp's earlier history)
// made Dolphin visibly refresh on a pin/unpin but never actually repaint the
// checkmark either way.
//
// Pin/unpin happens through the daemon's own control server (see
// daemon/src/control.rs's route_pin/route_unpin), entirely outside any KIO
// job Dolphin initiated, so nothing tells *this* plugin instance anything
// changed either, without the D-Bus listener below: the daemon broadcasts
// org.kde.protondrive.OverlayIcon.PinChanged after a successful pin/unpin
// (a small, purpose-built signal — not KDirNotify::FilesChanged, which also
// makes Dolphin re-stat/re-list the whole item, visibly slower and no more
// effective at actually updating the overlay).
class ProtonDriveOverlayPlugin : public KOverlayIconPlugin
{
    Q_PLUGIN_METADATA(IID "org.kde.overlayicon.protondrive" FILE "protondrive-overlay.json")
    Q_OBJECT

public:
    ProtonDriveOverlayPlugin()
    {
        QDBusConnection::sessionBus().connect(
            QString(),
            QStringLiteral("/"),
            QStringLiteral("org.kde.protondrive.OverlayIcon"),
            QStringLiteral("PinChanged"),
            this,
            SLOT(onPinChanged(QString)));
    }

    QStringList getOverlays(const QUrl &url) override
    {
        if (url.scheme() != QLatin1String("protondrive")) {
            return {};
        }
        return overlaysFor(url.path());
    }

private Q_SLOTS:
    // `remotePath` is the bare Drive path (e.g. "/my-files/a.pdf"), not a
    // full URL — emitted for both URL spellings Dolphin might hold an item
    // under (see daemon/src/control.rs's notify_files_changed doc comment
    // on the `protondrive:/...` vs `protondrive:///...` ambiguity; the same
    // uncertainty applies here in the push direction).
    void onPinChanged(const QString &remotePath)
    {
        const QStringList overlays = overlaysFor(remotePath);
        Q_EMIT overlaysChanged(QUrl(QStringLiteral("protondrive:") + remotePath), overlays);
        Q_EMIT overlaysChanged(QUrl(QStringLiteral("protondrive://") + remotePath), overlays);
    }
};

#include "overlayplugin.moc"
