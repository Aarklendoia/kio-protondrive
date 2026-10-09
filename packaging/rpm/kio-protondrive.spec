#
# spec file for kio-protondrive
#
# One spec for openSUSE and Fedora, built on the Open Build Service (#201).
# Builds there are offline: Source1 holds the vendored crates (cxxbridge-cmd
# included, which Corrosion builds at configure time), the cargo config
# pointing at them and Corrosion itself (Fedora 44 only has 0.5), produced
# by packaging/rpm/make-sources.sh, the same way
# debian/scripts/prepare-offline-build.sh does it for the PPA. Both have
# fixed names: the OBS package's _service fetches them from the latest
# GitHub release.
#

# Both distributions build with -flto=auto by default, and the cc crate
# honors those flags: the C++ part of core/ (cxx's runtime) then becomes LTO
# bytecode that neither the Rust link nor the plugins' CMake link resolves
# (same as Arch's !lto, #111).
%global _lto_cflags %{nil}

Name:           kio-protondrive
# x-release-please-start-version
Version:        0.12.0
# x-release-please-end
Release:        0%{?dist}
Summary:        KIO worker for Proton Drive
License:        GPL-3.0-or-later
URL:            https://github.com/Aarklendoia/kio-protondrive
Source0:        %{name}-source.tar.gz
Source1:        %{name}-vendor.tar.xz

BuildRequires:  cargo
BuildRequires:  cmake >= 3.22
BuildRequires:  extra-cmake-modules >= 6.0
BuildRequires:  gcc-c++
BuildRequires:  gettext
BuildRequires:  rust >= 1.93
BuildRequires:  systemd-rpm-macros
BuildRequires:  cmake(KF6CoreAddons)
BuildRequires:  cmake(KF6I18n)
BuildRequires:  cmake(KF6KIO)
BuildRequires:  cmake(KF6WidgetsAddons)
BuildRequires:  cmake(Qt6Core) >= 6.6
BuildRequires:  cmake(Qt6DBus)
BuildRequires:  cmake(Qt6Test)
BuildRequires:  pkgconfig(sqlite3)
# lrelease for the wizard's translations, found through qtpaths6 (in
# qt6-base-common-devel on openSUSE, qt6-qtbase-devel on Fedora).
%if 0%{?suse_version}
BuildRequires:  qt6-base-common-devel
BuildRequires:  qt6-tools-linguist
%else
BuildRequires:  qt6-linguist
%endif

%description
Browse, download, upload and manage files stored on Proton Drive directly
from Dolphin (or any other KIO-aware application) via the protondrive://
protocol. Files are fetched on demand. Requires Proton's own proton-drive
CLI, which the setup wizard can install.

%package sync-daemon
Summary:        Background sync daemon for kio-protondrive
Requires:       %{name} = %{version}-%{release}
Requires:       curl

%description sync-daemon
Pins Proton Drive files for offline access, evicts the local cache, and
adds the pin/unpin actions to Dolphin's context menu. Enable it per user
with: systemctl --user enable --now kio-protondrive-sync-daemon.service

%package wizard
Summary:        First-run setup wizard for kio-protondrive
Requires:       %{name}-sync-daemon = %{version}-%{release}
Requires:       curl
# The UI runs in Qt's own QML runtime, with the org.kde.desktop style.
%if 0%{?suse_version}
Requires:       qt6-declarative-tools
%else
Requires:       qt6-qtdeclarative
%endif
Requires:       kf6-kirigami
Requires:       kf6-qqc2-desktop-style
Recommends:     pass

%description wizard
Signs you in to Proton Drive, installs or updates the proton-drive CLI,
and chooses how the sync daemon stores its session.

%prep
%autosetup -p1 -a1 -n %{name}-%{version}

%build
# Source1's .cargo/config.toml points crates-io at vendor/, but the
# `cargo install cxxbridge-cmd` Corrosion runs at configure time doesn't
# read the project's config (recent cargo, unlike Ubuntu's 1.93): put it in
# CARGO_HOME, which every cargo command reads, with vendor/'s absolute path.
export CARGO_HOME="$PWD/.cargo-home"
mkdir -p "$CARGO_HOME"
sed "s|^directory = \"vendor\"|directory = \"$PWD/vendor\"|" .cargo/config.toml > "$CARGO_HOME/config.toml"
export CARGO_NET_OFFLINE=true
cmake -S . -B build \
  -DCMAKE_BUILD_TYPE=RelWithDebInfo \
  -DCMAKE_INSTALL_PREFIX=%{_prefix} \
  -DKDE_INSTALL_USE_QT_SYS_PATHS=ON \
  -DBUILD_TESTING=ON
cmake --build build %{?_smp_mflags}
# daemon/ and wizard/ are plain Cargo, outside the CMake/Corrosion graph
# (same split build as debian/rules and the PKGBUILD).
cargo build --release --offline \
  --package kio-protondrive-daemon --package kio-protondrive-wizard
qt_bins="$(qtpaths6 --query QT_INSTALL_BINS)"
for ts in wizard/translations/*.ts; do
  "$qt_bins/lrelease" "$ts" -qm "${ts%%.ts}.qm"
done

%install
DESTDIR=%{buildroot} cmake --install build
install -Dm755 target/release/kio-protondrive-daemon %{buildroot}%{_bindir}/kio-protondrive-daemon
install -Dm644 debian/kio-protondrive-sync-daemon.user.service \
  %{buildroot}%{_userunitdir}/kio-protondrive-sync-daemon.service
install -Dm755 target/release/kio-protondrive-wizard %{buildroot}%{_bindir}/kio-protondrive-wizard
install -Dm644 wizard/kio-protondrive-wizard.desktop \
  %{buildroot}%{_datadir}/applications/kio-protondrive-wizard.desktop
install -Dm644 wizard/kio-protondrive-wizard.metainfo.xml \
  %{buildroot}%{_datadir}/metainfo/kio-protondrive-wizard.metainfo.xml
install -d %{buildroot}%{_datadir}/kio-protondrive-wizard/qml \
  %{buildroot}%{_datadir}/kio-protondrive-wizard/translations
install -m644 wizard/qml/*.qml %{buildroot}%{_datadir}/kio-protondrive-wizard/qml/
install -m644 wizard/translations/*.qm %{buildroot}%{_datadir}/kio-protondrive-wizard/translations/
%find_lang kio_protondrive
%find_lang kio_protondrive_daemon

%check
export CARGO_HOME="$PWD/.cargo-home"
export CARGO_NET_OFFLINE=true
cargo test --offline --workspace --profile ci
ctest --test-dir build --output-on-failure

%files -f kio_protondrive.lang
%license LICENSE
%doc README.md
%dir %{_libdir}/qt6/plugins/kf6/overlayicon
%{_libdir}/qt6/plugins/kf6/kio/protondrive.so
%{_libdir}/qt6/plugins/kf6/overlayicon/protondrive_overlayicon.so

%files sync-daemon -f kio_protondrive_daemon.lang
%license LICENSE
%{_bindir}/kio-protondrive-daemon
%{_libdir}/qt6/plugins/kf6/kfileitemaction/protondrive_fileitemaction.so
%{_userunitdir}/kio-protondrive-sync-daemon.service

%files wizard
%license LICENSE
%{_bindir}/kio-protondrive-wizard
%{_datadir}/applications/kio-protondrive-wizard.desktop
%{_datadir}/metainfo/kio-protondrive-wizard.metainfo.xml
%{_datadir}/kio-protondrive-wizard/

%changelog
