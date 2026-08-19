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
#include <QDesktopServices>
#include <QIcon>
#include <QMenu>
#include <QProcess>
#include <QUrl>
#include <QWidget>

#include "rust/cxx.h"

#include "photo_categories.h"
#include "sharedialog.h"
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

// Kept as plain duplicated literals rather than shared with
// protondriveworker.cpp's own trashPrefix/photosPrefix — single small
// stable strings not worth a shared header for (unlike photo_categories.h's
// 9-entry table, which both files genuinely need kept in sync).
const QString trashPrefix = QStringLiteral("/trash/");
const QString photosPrefix = QStringLiteral("/photos/");

// A single selected item can be shared unless it's under /trash (sharing a
// trashed item makes no sense) or is one of the fixed virtual root sections
// themselves (protondriveworker.cpp's translatedSectionName's raw-name
// set, e.g. "/my-files", "/photos" — not real nodes, one path depth level:
// exactly one '/'). /photos/<name> paths are deliberately *not* excluded
// here — whether the CLI's `sharing` commands accept that path shape isn't
// confirmed yet; if not, ShareDialog's own error handling surfaces it.
bool isShareableItem(const QString &path)
{
    if (path.startsWith(trashPrefix) || path == QLatin1String("/trash")) {
        return false;
    }
    return path.count(QLatin1Char('/')) > 1;
}

// KAbstractFileItemActionPlugin loads into the host file manager's own
// process (confirmed live: this plugin's pin/unpin actions already run
// there today) — so for Dolphin specifically, we can skip
// QDesktopServices::openUrl's xdg-open dance (which spawns a fresh
// "dolphin <url>" invocation that, per live testing, lands in a *new
// window* rather than the window the click came from) and instead invoke
// Dolphin's own public D-Bus-exposed org.kde.dolphin.MainWindow method
// directly in-process via the meta-object system on the actual window
// that owns this context menu. `invokeMethod` returns false if no such
// method exists (any non-Dolphin KIO-aware file manager), in which case
// this falls back to the portable QDesktopServices path.
void openPhotosUrl(const QUrl &url, QWidget *parentWidget)
{
    if (QWidget *window = parentWidget ? parentWidget->window() : nullptr) {
        const bool invoked = QMetaObject::invokeMethod(
            window, "openDirectories", Q_ARG(QStringList, QStringList{url.toString()}), Q_ARG(bool, false));
        if (invoked) {
            return;
        }
    }
    QDesktopServices::openUrl(url);
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

        // Whether this is a background right-click (or a right-click on
        // the item itself while it's the only thing selected — same
        // KFileItemListProperties shape either way, see isEmptyTrashTarget
        // above) on /photos or one of its filter category folders
        // (photo_categories.h) — the only two places "Filter Photos" makes
        // sense. `photosFilterCategory` is empty for bare /photos itself.
        bool isPhotosBackground = false;
        QString photosFilterCategory;
        if (items.size() == 1) {
            const QString onlyPath = paths.first();
            if (onlyPath == QLatin1String("/photos")) {
                isPhotosBackground = true;
            } else if (onlyPath.startsWith(photosPrefix)) {
                const QString afterPrefix = onlyPath.mid(photosPrefix.length());
                if (photoCategorySlugs().contains(afterPrefix)) {
                    isPhotosBackground = true;
                    photosFilterCategory = afterPrefix;
                }
            }
        }

        // A single-item selection that isn't a virtual root section or
        // under /trash gets a "Share" action opening ShareDialog (see
        // isShareableItem above).
        QString shareablePath;
        QString shareableName;
        if (items.size() == 1 && isShareableItem(paths.first())) {
            shareablePath = paths.first();
            shareableName = items.first().text();
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

        // Dolphin/KIO has no toolbar extension point for a protocol plugin
        // (that would need a full Dolphin-specific plugin, not a KIO
        // worker) — this submenu is the substitute: a background
        // right-click on /photos or one of its filter category folders
        // (see isPhotosBackground above) offers every other category as a
        // one-click jump, opened via openPhotosUrl (new tab in the
        // originating Dolphin window when possible, see its doc comment
        // above; a plain new-tab/window "open" otherwise).
        if (isPhotosBackground) {
            QAction *filterMenuAction = new QAction(i18nd("kio_protondrive", "Filter Photos"), parentWidget);
            filterMenuAction->setIcon(QIcon::fromTheme(QStringLiteral("view-filter")));
            QMenu *filterMenu = new QMenu(parentWidget);
            filterMenuAction->setMenu(filterMenu);

            if (!photosFilterCategory.isEmpty()) {
                QAction *allPhotos = filterMenu->addAction(QIcon::fromTheme(QStringLiteral("folder-pictures")), i18nd("kio_protondrive", "All Photos"));
                connect(allPhotos, &QAction::triggered, parentWidget, [parentWidget]() {
                    openPhotosUrl(QUrl(QStringLiteral("protondrive:/photos")), parentWidget);
                });
            }
            for (const QString &slug : photoCategorySlugs()) {
                if (slug == photosFilterCategory) {
                    continue;
                }
                QAction *categoryAction = filterMenu->addAction(QIcon::fromTheme(photoCategoryIcon(slug)), photoCategoryLabel(slug));
                connect(categoryAction, &QAction::triggered, parentWidget, [slug, parentWidget]() {
                    openPhotosUrl(QUrl(QStringLiteral("protondrive:/photos/") + slug), parentWidget);
                });
            }
            result << filterMenuAction;
        }

        // Opens ShareDialog (see sharedialog.h) — modal, matching this
        // file's existing KMessageBox usage, since there's no other place
        // to route "the user is filling out a form" than blocking the
        // context menu's originating window.
        if (!shareablePath.isEmpty()) {
            QAction *shareAction = new QAction(i18nd("kio_protondrive", "Share"), parentWidget);
            shareAction->setIcon(QIcon::fromTheme(QStringLiteral("document-share")));
            connect(shareAction, &QAction::triggered, parentWidget, [shareablePath, shareableName, parentWidget]() {
                ShareDialog dialog(shareablePath, shareableName, parentWidget);
                dialog.exec();
            });
            result << shareAction;
        }

        return result;
    }
};

K_PLUGIN_CLASS_WITH_JSON(ProtonDriveFileItemActionPlugin, "protondrive-fileitemaction.json")

#include "fileitemactionplugin.moc"
