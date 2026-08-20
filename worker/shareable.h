#pragma once

#include <QString>

#include "photo_categories.h"

namespace protondrive
{

// Kept as plain duplicated literals rather than shared with
// protondriveworker.cpp's own trashPrefix/photosPrefix — single small
// stable strings not worth a shared header for (unlike photo_categories.h's
// 9-entry table, which both files genuinely need kept in sync).
inline const QString &shareableTrashPrefix()
{
    static const QString prefix = QStringLiteral("/trash/");
    return prefix;
}

inline const QString &shareablePhotosPrefix()
{
    static const QString prefix = QStringLiteral("/photos/");
    return prefix;
}

// A single selected item can be shared unless it's under /trash (sharing a
// trashed item makes no sense), is one of the fixed virtual root sections
// themselves (protondriveworker.cpp's translatedSectionName's raw-name
// set, e.g. "/my-files", "/photos" — not real nodes, one path depth level:
// exactly one '/'), or is a /photos/<category> filter folder (favorites,
// screenshots, ... — see photoCategorySlugs()): synthetic, client-side-only
// views (protondriveworker.cpp's splitPhotoPath/listPhotosCategory), not
// real Drive nodes the CLI can resolve at all. /photos/<name> *items* ARE
// shareable, and share the same one-path-depth-level shape as a category
// folder, so the category slug itself has to be checked explicitly rather
// than inferred from depth alone — confirmed live that `sharing
// set-url`/`filesystem info` both resolve a real /photos/<name> path fine;
// the "undefined" response that first looked like a photos-specific
// failure turned out to be `crate::cli::sharing_status`'s own bug (see its
// doc comment), reproducible on plain /my-files items too.
//
// Extracted out of fileitemactionplugin.cpp (an anonymous-namespace
// function there, so unreachable from a separate test binary) so it has a
// unit test — see worker/tests/tst_shareable.cpp.
inline bool isShareableItem(const QString &path)
{
    if (path.startsWith(shareableTrashPrefix()) || path == QLatin1String("/trash")) {
        return false;
    }
    if (path.startsWith(shareablePhotosPrefix()) && photoCategorySlugs().contains(path.mid(shareablePhotosPrefix().length()))) {
        return false;
    }
    return path.count(QLatin1Char('/')) > 1;
}

}
