#include <KAbstractFileItemActionPlugin>
#include <KFileItem>
#include <KFileItemListProperties>
#include <KLocalizedString>
#include <KPluginFactory>
#include <QAction>
#include <QIcon>
#include <QProcess>
#include <QUrl>
#include <QWidget>

#include "rust/cxx.h"

#include "protondrive-core-cxxbridge/bridge.h"

using namespace protondrive;

namespace
{
// Mirrors overlayplugin.cpp's is_available_locally/lookup_pin split: the
// context menu only ever offers pin/unpin (there is no manual "evict from
// cache" action — eviction is automatic/time-based, see #60), so only the
// explicitly-pinned state decides which of the two actions to offer.
bool isPinned(const QString &remotePath)
{
    try {
        return !lookup_pin(remotePath.toStdString()).empty();
    } catch (const rust::Error &) {
        return false;
    }
}
}

// Replaces the old declarative daemon/kio-protondrive-pin.desktop
// (KonqPopupMenu/Plugin) servicemenu, which unconditionally listed both
// "Keep Available Offline" and "Remove Local Copy" for every protondrive://
// item regardless of whether it was actually pinned — a static .desktop
// servicemenu has no way to query our own pin state. KAbstractFileItemAction
// is KIO's mechanism for exactly this: a context menu that depends on the
// selected items' state, confirmed live as the fix for that.
//
// Multi-selection: `actions()` receives every selected item at once (not
// called per-item), so a mixed pinned/unpinned selection needs its own
// decision — show "pin" if *any* item isn't pinned yet (so the rest can
// join), and "unpin" if *any* item already is (so those can leave), matching
// the OneDrive-style behavior this whole feature is modeled on. Running
// "pin" on an already-pinned item, or "unpin" on a never-pinned one, is a
// harmless no-op (see core/src/cache.rs's pin/unpin tests), so the action
// handlers below apply to the whole selection rather than filtering it
// per-item.
class ProtonDriveFileItemActionPlugin : public KAbstractFileItemActionPlugin
{
    Q_OBJECT

public:
    ProtonDriveFileItemActionPlugin(QObject *parent, const KPluginMetaData &, const QVariantList &)
        : KAbstractFileItemActionPlugin(parent)
    {
    }

    QList<QAction *> actions(const KFileItemListProperties &fileItemInfos, QWidget *parentWidget) override
    {
        const KFileItemList items = fileItemInfos.items();
        if (items.isEmpty()) {
            return {};
        }

        // A selection mixing protondrive:// items with anything else can't
        // sensibly get a single pin/unpin action — bail out entirely rather
        // than silently acting on only part of the selection. The service
        // type's own X-KDE-Protocols filter (see
        // protondrive-fileitemaction.json) should already keep this plugin
        // from being asked in that case, but that's a hint to the loader,
        // not something this code should rely on for correctness.
        QStringList urls;
        bool anyPinned = false;
        bool anyNotPinned = false;
        for (const KFileItem &item : items) {
            const QUrl url = item.url();
            if (url.scheme() != QLatin1String("protondrive")) {
                return {};
            }
            urls << url.toString();
            if (isPinned(url.path())) {
                anyPinned = true;
            } else {
                anyNotPinned = true;
            }
        }

        QList<QAction *> result;

        if (anyNotPinned) {
            QAction *pin = new QAction(i18nd("kio_protondrive", "Keep Available Offline"), parentWidget);
            pin->setIcon(QIcon::fromTheme(QStringLiteral("folder-download")));
            connect(pin, &QAction::triggered, parentWidget, [urls]() {
                for (const QString &url : urls) {
                    QProcess::startDetached(QStringLiteral("kio-protondrive-daemon"), {QStringLiteral("pin"), url});
                }
            });
            result << pin;
        }

        if (anyPinned) {
            QAction *unpin = new QAction(i18nd("kio_protondrive", "Remove Local Copy"), parentWidget);
            unpin->setIcon(QIcon::fromTheme(QStringLiteral("edit-delete")));
            connect(unpin, &QAction::triggered, parentWidget, [urls]() {
                for (const QString &url : urls) {
                    QProcess::startDetached(QStringLiteral("kio-protondrive-daemon"), {QStringLiteral("unpin"), url});
                }
            });
            result << unpin;
        }

        return result;
    }
};

K_PLUGIN_CLASS_WITH_JSON(ProtonDriveFileItemActionPlugin, "protondrive-fileitemaction.json")

#include "fileitemactionplugin.moc"
