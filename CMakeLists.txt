cmake_minimum_required(VERSION 3.22)
project(kio-protondrive)

set(KF_MIN_VERSION "6.0.0")
set(QT_MIN_VERSION "6.6.0")

find_package(ECM ${KF_MIN_VERSION} REQUIRED NO_MODULE)
list(APPEND CMAKE_MODULE_PATH ${ECM_MODULE_PATH})

include(KDEInstallDirs)
include(KDECMakeSettings)
include(KDECompilerSettings NO_POLICY_SCOPE)
include(ECMSetupVersion)
include(ECMAddTests)
include(FeatureSummary)

# Optional, not REQUIRED: core/'s bundled-sqlite3 (see core/Cargo.toml's
# rusqlite "bundled" feature) doesn't need a system sqlite3 to build the
# Rust side, and Corrosion doesn't propagate that native static-lib
# dependency to CMake automatically (a documented Corrosion limitation, not
# a bug) — worker/CMakeLists.txt links SQLite::SQLite3 into the plugins
# that need it, but only when this is actually available, so a system
# without libsqlite3-dev (the Debian/Ubuntu build environments today) keeps
# building exactly as before. Confirmed live that skipping this on Arch
# (where the dev package is present, `sqlite`) fails at link time with
# sqlite3_* undefined in protondrive.so's strict-by-default linker; Debian's
# looser default apparently tolerates the same gap and resolves those
# symbols at dlopen time instead, against whatever already-loaded process
# happens to have libsqlite3 linked in — fragile, but out of scope to
# harden here versus just fixing the newly-broken case.
find_package(SQLite3 QUIET)

find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Core DBus Test)
find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS CoreAddons KIO I18n WidgetsAddons)
find_package(Gettext REQUIRED)

enable_testing()

# Corrosion drives the actual `cargo build` of the core/ Rust crate and
# generates the cxx bridge glue consumed by worker/. Two ways to obtain it:
#   - vendored, at third_party/corrosion/ (see debian/scripts/
#     prepare-offline-build.sh): required for Launchpad PPA builds, whose
#     builders have no general internet access to reach GitHub at configure
#     time.
#   - fetched on the fly via FetchContent: what local dev builds and GitHub
#     Actions CI use, since they do have network access.
if(EXISTS "${CMAKE_SOURCE_DIR}/third_party/corrosion/CMakeLists.txt")
    add_subdirectory(third_party/corrosion)
else()
    include(FetchContent)
    FetchContent_Declare(
        Corrosion
        GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
        GIT_TAG v0.6.0
    )
    FetchContent_MakeAvailable(Corrosion)
endif()

# CRATES restricts the import to core/'s own package: without it, Corrosion
# resolves the whole Cargo workspace core/'s manifest belongs to and also
# builds daemon/'s and wizard's bin targets as an (unused here — they're
# built separately, see debian/rules' override_dh_auto_build) side effect.
# Harmless duplication on a GNU-ld host, but confirmed to fail outright on
# Arch's lld-by-default rustc: rusqlite's bundled-sqlite3 static lib built
# fine, but its cargo:rustc-link-lib directive didn't make it into the
# kio-protondrive-daemon link line lld produced, leaving every sqlite3_*
# symbol undefined — narrowing the import to just what the cxx bridge needs
# sidesteps it rather than chasing the link-line ordering bug itself.
corrosion_import_crate(MANIFEST_PATH core/Cargo.toml CRATES protondrive-core)

corrosion_add_cxxbridge(protondrive-core-cxxbridge
    CRATE protondrive_core
    FILES bridge.rs
)
kde_target_enable_exceptions(protondrive-core-cxxbridge PUBLIC)
set_target_properties(protondrive-core-cxxbridge PROPERTIES POSITION_INDEPENDENT_CODE ON)

add_subdirectory(worker)

ki18n_install(po)

feature_summary(WHAT ALL FATAL_ON_MISSING_REQUIRED_PACKAGES)
