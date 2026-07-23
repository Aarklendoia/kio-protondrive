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
include(FeatureSummary)

find_package(Qt6 ${QT_MIN_VERSION} REQUIRED COMPONENTS Core)
find_package(KF6 ${KF_MIN_VERSION} REQUIRED COMPONENTS CoreAddons KIO)

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

corrosion_import_crate(MANIFEST_PATH core/Cargo.toml)

corrosion_add_cxxbridge(protondrive-core-cxxbridge
    CRATE protondrive_core
    FILES bridge.rs
)
kde_target_enable_exceptions(protondrive-core-cxxbridge PUBLIC)
set_target_properties(protondrive-core-cxxbridge PROPERTIES POSITION_INDEPENDENT_CODE ON)

add_subdirectory(worker)

feature_summary(WHAT ALL FATAL_ON_MISSING_REQUIRED_PACKAGES)
