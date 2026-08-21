# protondrive-core-cxxbridge alone pulls in libprotondrive_core.a itself
# but not that crate's OWN transitive dependencies:
#   - protondrive_core: its native link requirements (e.g. rusqlite's
#     bundled sqlite3 static lib) — corrosion_add_cxxbridge's linkage to the
#     crate isn't as complete as corrosion_import_crate's own imported
#     target, so link that directly too to pull in what it's missing.
#   - SQLite::SQLite3: confirmed live (Arch, strict-by-default linker) that
#     even the above isn't enough — Corrosion doesn't propagate a Rust
#     crate's own native-library requirements to CMake at all (a documented
#     Corrosion limitation, not a bug), so link the system sqlite3
#     ourselves, when available (see the top-level CMakeLists.txt's
#     find_package(SQLite3 QUIET) for why this is conditional rather than
#     required).
# Applies to all three plugins below, since all three link
# protondrive-core-cxxbridge and pull in the same core/ code path
# (core::cache's rusqlite usage) transitively either way.
set(_protondrive_core_link_libs protondrive-core-cxxbridge protondrive_core)
if(TARGET SQLite::SQLite3)
    list(APPEND _protondrive_core_link_libs SQLite::SQLite3)
endif()

# Even fully linked as above, confirmed live (Arch) that a handful of cxx's
# own CxxVector<T> C++ symbols (e.g. T=f64 — cxx's proc-macro expansion
# monomorphizes VectorElement for a fixed set of primitive types
# unconditionally, not just the ones bridge.rs's FFI surface actually uses)
# can still come up undefined: genuinely dead code, never called since
# nothing in this project exposes e.g. a Vec<f64> across the bridge, but
# still enough to trip ECM's KDECompilerSettings -Wl,--no-undefined
# (applied repo-wide, upheld everywhere else — this is the one place it's
# relaxed, not turned off project-wide). "-z,undefs" is GNU ld's documented
# inverse of "-z,defs"/"--no-undefined", placed after it on each of these
# targets' own link line so it wins (ld resolves this class of toggle by
# last-flag-wins, not by which was added first).
set(_protondrive_core_link_options "-Wl,-z,undefs")

function(protondrive_link_core target)
    target_link_libraries(${target} PRIVATE ${_protondrive_core_link_libs})
    target_link_options(${target} PRIVATE ${_protondrive_core_link_options})
endfunction()

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
)
protondrive_link_core(protondrive)

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
)
protondrive_link_core(protondrive_overlayicon)

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
)
protondrive_link_core(protondrive_fileitemaction)

set_target_properties(protondrive_fileitemaction PROPERTIES OUTPUT_NAME "protondrive_fileitemaction")

add_subdirectory(tests)
