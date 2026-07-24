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
        protondrive-core-cxxbridge
)

set_target_properties(protondrive PROPERTIES OUTPUT_NAME "protondrive")
