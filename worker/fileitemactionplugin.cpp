#include <KAbstractFileItemActionPlugin>
#include <KFileItem>
#include <KFileItemListProperties>
#include <KGuiItem>
#include <KLocalizedString>
#include <KMessageBox>
#include <KPluginFactory>
#include <KStandardGuiItem>
#include <QAction>
#include <QDebug>
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

// Kept as a plain duplicated literal rather than shared with
// protondriveworker.cpp's own trashPrefix — same precedent as that file's
// photosPrefix, a single small stable string not worth a shared header for.
const QString trashPrefix = QStringLiteral("/trash/");
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
        QStringList paths;
        bool anyPinned = false;
        bool anyNotPinned = false;
        bool allUnderTrash = true;
        for (const KFileItem &item : items) {
            const QUrl url = item.url();
            if (url.scheme() != QLatin1String("protondrive")) {
                return {};
            }
            urls << url.toString();
            paths << url.path();
            if (isPinned(url.path())) {
                anyPinned = true;
            } else {
                anyNotPinned = true;
            }
            if (!url.path().startsWith(trashPrefix)) {
                allUnderTrash = false;
            }
        }
        const bool isEmptyTrashTarget = items.size() == 1 && paths.first() == QLatin1String("/trash");

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

        // Restore/empty-trash call the cxx bridge directly rather than
        // shelling out to kio-protondrive-daemon the way pin/unpin do —
        // unlike the pin index, there's no local SQLite state these need a
        // single-writer daemon for, just a remote API call plus the same
        // best-effort cache invalidation the worker itself already does
        // in-process for trash()/rename_or_move() (see core/src/bridge.rs).
        if (allUnderTrash) {
            QAction *restore = new QAction(i18nd("kio_protondrive", "Restore"), parentWidget);
            restore->setIcon(QIcon::fromTheme(QStringLiteral("edit-undo")));
            connect(restore, &QAction::triggered, parentWidget, [paths]() {
                for (const QString &path : paths) {
                    try {
                        restore_path(path.toStdString());
                    } catch (const rust::Error &error) {
                        qWarning() << "could not restore" << path << ":" << error.what();
                    }
                }
            });
            result << restore;
        }

        // Only offered on the /trash virtual section item itself (see
        // isEmptyTrashTarget above) — there's no Dolphin toolbar button for
        // this the way native trash:/ has one, see docs/DESIGN.md.
        // Irreversible and not routed through a KIO::DeleteJob (which is
        // what gives Dolphin's own delete actions their built-in "are you
        // sure?" confirmation) — this action invents its own UI from
        // scratch, so it needs an explicit confirmation of its own.
        if (isEmptyTrashTarget) {
            QAction *emptyTrashAction = new QAction(i18nd("kio_protondrive", "Empty Trash"), parentWidget);
            emptyTrashAction->setIcon(QIcon::fromTheme(QStringLiteral("trash-empty")));
            connect(emptyTrashAction, &QAction::triggered, parentWidget, [parentWidget]() {
                const auto answer = KMessageBox::warningTwoActions(
                    parentWidget,
                    i18nd("kio_protondrive", "Permanently delete everything in the trash? This cannot be undone."),
                    i18nd("kio_protondrive", "Empty Trash"),
                    KGuiItem(i18nd("kio_protondrive", "Empty Trash"), QStringLiteral("trash-empty")),
                    KStandardGuiItem::cancel());
                if (answer != KMessageBox::PrimaryAction) {
                    return;
                }
                try {
                    empty_trash();
                } catch (const rust::Error &error) {
                    qWarning() << "could not empty the trash:" << error.what();
                }
            });
            result << emptyTrashAction;
        }

        return result;
    }
};

K_PLUGIN_CLASS_WITH_JSON(ProtonDriveFileItemActionPlugin, "protondrive-fileitemaction.json")

#include "fileitemactionplugin.moc"
