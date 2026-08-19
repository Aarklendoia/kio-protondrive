# KIO workers are Qt plugins loaded at runtime by the generic `kioworker`
# host process shipped with KF6::KIO — not standalone executables — so this
# builds a MODULE library, following the same pattern as KDE/kio-admin's
# src/CMakeLists.txt (kcoreaddons_add_plugin + INSTALL_NAMESPACE "kf6/kio").
kcoreaddons_add_plugin(protondrive
    SOURCES
        protondriveworker.cpp
    INSTALL_NAMESPACE "kf6/kio"
)

target_link_libraries(protondrive
    PUBLIC
        KF6::KIOCore
    PRIVATE
        KF6::I18n
        Qt6::DBus
        protondrive-core-cxxbridge
)

set_target_properties(protondrive PROPERTIES OUTPUT_NAME "protondrive")

# See overlayplugin.cpp's doc comment for why this exists alongside the
# worker's own UDS_ICON_OVERLAY_NAMES — this is Dolphin's purpose-built,
# on-demand path to the same "is this pinned?" answer, the same mechanism
# Nextcloud's/ownCloud's desktop clients use for their sync-status emblems.
kcoreaddons_add_plugin(protondrive_overlayicon
    SOURCES
        overlayplugin.cpp
    INSTALL_NAMESPACE "kf6/overlayicon"
)

target_link_libraries(protondrive_overlayicon
    PRIVATE
        KF6::KIOCore
        Qt6::DBus
        protondrive-core-cxxbridge
)

set_target_properties(protondrive_overlayicon PROPERTIES OUTPUT_NAME "protondrive_overlayicon")

# Replaces daemon/kio-protondrive-pin.desktop's old static servicemenu (see
# fileitemactionplugin.cpp's doc comment for why): KAbstractFileItemAction is
# KIO's mechanism for a context menu whose entries depend on the selected
# items' state, which a declarative .desktop servicemenu cannot do.
kcoreaddons_add_plugin(protondrive_fileitemaction
    SOURCES
        fileitemactionplugin.cpp
        sharedialog.cpp
    INSTALL_NAMESPACE "kf6/kfileitemaction"
)

target_link_libraries(protondrive_fileitemaction
    PRIVATE
        KF6::KIOCore
        KF6::KIOWidgets
        KF6::I18n
        KF6::WidgetsAddons
        protondrive-core-cxxbridge
)

set_target_properties(protondrive_fileitemaction PROPERTIES OUTPUT_NAME "protondrive_fileitemaction")
