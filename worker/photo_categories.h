#pragma once

#include <KLocalizedString>
#include <QHash>
#include <QString>
#include <QStringList>

namespace protondrive
{

// The web app's `/photos` filter tabs (see core/src/photos.rs's
// PhotoCategory — these slugs must match its `slug()` exactly, confirmed by
// core/src/photos.rs's own `photo_category_slugs_round_trip` test only
// covering the Rust side; there's no compile-time link between the two).
// Centralized here rather than duplicated a second time (unlike
// protondriveworker.cpp's own trashPrefix/photosPrefix, which really are
// single-line literals not worth a shared header for) — protondriveworker.cpp
// and fileitemactionplugin.cpp are separate plugin shared objects with no
// linkage between them, but both need this *same* 9-entry table, and
// letting two hand-copies drift out of sync is a real risk a one-line
// string never had.
//
// Icon choices pulled from Proton's own (public, open-source) web client
// source (`Tags.tsx`'s per-`PhotoTag` `iconName`,
// github.com/ProtonMail/WebClients) — mapped to the closest available
// Breeze icon rather than reused directly (different, proprietary icon
// set/style). All 9 confirmed present in this project's target Breeze
// theme.

inline const QStringList &photoCategorySlugs()
{
    static const QStringList slugs = {
        QStringLiteral("favorites"),
        QStringLiteral("screenshots"),
        QStringLiteral("videos"),
        QStringLiteral("live-photos"),
        QStringLiteral("selfies"),
        QStringLiteral("portraits"),
        QStringLiteral("bursts"),
        QStringLiteral("panoramas"),
        QStringLiteral("raw"),
    };
    return slugs;
}

inline QString photoCategoryLabel(const QString &slug)
{
    static const QHash<QString, QString> labels = {
        {QStringLiteral("favorites"), i18nd("kio_protondrive", "Favorites")},
        {QStringLiteral("screenshots"), i18nd("kio_protondrive", "Screenshots")},
        {QStringLiteral("videos"), i18nd("kio_protondrive", "Videos")},
        {QStringLiteral("live-photos"), i18nd("kio_protondrive", "Live Photos")},
        {QStringLiteral("selfies"), i18nd("kio_protondrive", "Selfies")},
        {QStringLiteral("portraits"), i18nd("kio_protondrive", "Portraits")},
        {QStringLiteral("bursts"), i18nd("kio_protondrive", "Bursts")},
        {QStringLiteral("panoramas"), i18nd("kio_protondrive", "Panoramas")},
        {QStringLiteral("raw"), i18nd("kio_protondrive", "RAW")},
    };
    return labels.value(slug);
}

inline QString photoCategoryIcon(const QString &slug)
{
    static const QHash<QString, QString> icons = {
        {QStringLiteral("favorites"), QStringLiteral("love")},
        {QStringLiteral("screenshots"), QStringLiteral("accessories-screenshot-tool")},
        {QStringLiteral("videos"), QStringLiteral("camera-video")},
        {QStringLiteral("live-photos"), QStringLiteral("media-record")},
        {QStringLiteral("selfies"), QStringLiteral("user-identity")},
        {QStringLiteral("portraits"), QStringLiteral("im-user")},
        {QStringLiteral("bursts"), QStringLiteral("dialog-layers")},
        {QStringLiteral("panoramas"), QStringLiteral("kipi-panorama")},
        {QStringLiteral("raw"), QStringLiteral("image-x-adobe-dng")},
    };
    return icons.value(slug);
}

}
