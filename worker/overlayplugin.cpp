#include <KOverlayIconPlugin>
#include <QDBusConnection>
#include <QDBusMessage>
#include <QUrl>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
// Three states (#60), OneDrive-style: cloud-only (no overlay), available
// locally — pinned *or* opportunistically cached, same badge either way
// (emblem-checked) — and pinned specifically, an *additional* badge on top
// of "available locally" rather than a replacement for it. getOverlays()
// already supports returning several overlays at once, which is exactly
// what stacking these two needs. Breeze has no "emblem-pinned" icon at all
// (confirmed against the installed theme's file list) — QIcon::fromTheme
// silently renders nothing for a missing name, no error, which is why the
// pin badge was invisible in testing. emblem-favorite (a star) is the
// closest existing Breeze emblem, and is already visually distinct from
// emblem-checked.
QStringList overlaysFor(const QString &remotePath)
{
    QStringList overlays;
    const std::string path = remotePath.toStdString();

    // Bare bool, not Result — a lookup failure (e.g. an unwritable
    // $XDG_DATA_HOME) is already folded into "false" on the Rust side, same
    // "no overlay shown, not an error" stance as the try/catch below.
    if (is_available_locally(path)) {
        overlays << QStringLiteral("emblem-checked");
    }

    try {
        const rust::String pinned = lookup_pin(path);
        if (!pinned.empty()) {
            overlays << QStringLiteral("emblem-favorite");
        }
    } catch (const rust::Error &) {
    }

    return overlays;
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
// job Dolphin initiated; a fresh opportunistic-cache entry (#60) happens
// *inside* a KIO job (protondriveworker.cpp's get()/put()) but in a
// different process instance than whichever one is running this plugin.
// Either way nothing tells *this* plugin instance anything changed without
// the D-Bus listener below: both the daemon and the worker broadcast
// org.kde.protondrive.OverlayIcon.PinChanged after a relevant change (a
// small, purpose-built signal, despite the pin-specific name — not
// KDirNotify::FilesChanged, which also makes Dolphin re-stat/re-list the
// whole item, visibly slower and no more effective at actually updating the
// overlay).
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
