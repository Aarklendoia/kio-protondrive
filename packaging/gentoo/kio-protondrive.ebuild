# Copyright 2026 Gentoo Authors
# Distributed under the terms of the GNU General Public License v2

# Template: packaging/gentoo/make-overlay.sh fills CXXBRIDGE_PV and CRATES
# (and the crate part of LICENSE) from a freshly resolved Cargo.lock, since
# Cargo.lock isn't committed upstream.

EAPI=8

CXXBRIDGE_PV="@CXXBRIDGE_PV@"
CRATES="
"
# Corrosion runs `cargo install cxxbridge-cmd` at configure time unless a
# cxxbridge of the exact cxx version is on PATH: build that one ourselves,
# offline, from its own published lock (whose crates are in CRATES too).
CRATES+=" cxxbridge-cmd@${CXXBRIDGE_PV}"

RUST_MIN_VER="1.93.0"
KFMIN=6.9.0
QTMIN=6.6.0
inherit cargo desktop ecm flag-o-matic optfeature qt-utils systemd xdg

DESCRIPTION="KIO worker for Proton Drive: browse, download and upload files from Dolphin"
HOMEPAGE="https://github.com/Aarklendoia/kio-protondrive"
SRC_URI="
	https://github.com/Aarklendoia/kio-protondrive/archive/refs/tags/v${PV}.tar.gz
		-> ${P}.tar.gz
	${CARGO_CRATE_URIS}
"

LICENSE="GPL-3+"
# Dependent crate licenses
LICENSE+=""
SLOT="0"
KEYWORDS="~amd64 ~arm64"
IUSE="+daemon +wizard"
# The wizard writes the daemon's configuration (it links the daemon crate).
REQUIRED_USE="wizard? ( daemon )"

COMMON_DEPEND="
	>=dev-qt/qtbase-${QTMIN}:6[dbus,widgets]
	>=kde-frameworks/kcoreaddons-${KFMIN}:6
	>=kde-frameworks/ki18n-${KFMIN}:6
	>=kde-frameworks/kio-${KFMIN}:6
	>=kde-frameworks/kwidgetsaddons-${KFMIN}:6
	dev-db/sqlite:3
"
DEPEND="${COMMON_DEPEND}"
# curl: proton-drive CLI update checks (core). The wizard's UI runs in Qt's
# own qml6 runtime, with the org.kde.desktop Controls style forced.
RDEPEND="${COMMON_DEPEND}
	net-misc/curl
	wizard? (
		>=dev-qt/qtdeclarative-${QTMIN}:6
		>=kde-frameworks/kirigami-${KFMIN}:6
		>=kde-frameworks/qqc2-desktop-style-${KFMIN}:6
	)
"
BDEPEND="
	>=dev-build/corrosion-0.6
	sys-devel/gettext
	wizard? ( >=dev-qt/qttools-${QTMIN}:6[linguist] )
"

pkg_setup() {
	rust_pkg_setup
}

src_unpack() {
	cargo_src_unpack
}

src_configure() {
	# The cc crate honors CFLAGS, so with -flto the C/C++ parts of core/
	# (bundled sqlite3, cxx's runtime) become LTO bytecode that neither the
	# Rust link nor the CMake link of the plugins resolves (upstream #111).
	# cargo_env filters it for its own calls, not for Corrosion's.
	filter-lto

	cargo_env "${CARGO}" install --offline --locked \
		--path "${ECARGO_VENDOR}/cxxbridge-cmd-${CXXBRIDGE_PV}" \
		--target-dir "${T}/cxxbridge-target" \
		--root "${T}/cxxbridge" || die "building cxxbridge failed"
	export PATH="${T}/cxxbridge/bin:${PATH}"

	local mycmakeargs=(
		-DRust_COMPILER="${RUSTC}"
		-DRust_CARGO="${CARGO}"
	)
	ecm_src_configure

	local myargs=()
	use daemon && myargs+=( --package kio-protondrive-daemon )
	use wizard && myargs+=( --package kio-protondrive-wizard )
	cargo_src_configure "${myargs[@]}"
}

src_compile() {
	cmake_src_compile

	use daemon && cargo_src_compile

	if use wizard; then
		local ts
		for ts in wizard/translations/*.ts; do
			"$(qt_get_broot_binary 6 lrelease)" "${ts}" -qm "${ts%.ts}.qm" || die
		done
	fi
}

src_test() {
	cargo_src_test --workspace
	ecm_src_test
}

src_install() {
	ecm_src_install

	if use daemon; then
		dobin "$(cargo_target_dir)"/kio-protondrive-daemon
		systemd_newuserunit debian/kio-protondrive-sync-daemon.user.service \
			kio-protondrive-sync-daemon.service
	else
		# Pin/unpin menu: only useful with the daemon.
		find "${ED}" \( -name 'protondrive_fileitemaction.so' \
			-o -name 'kio_protondrive_daemon.mo' \) -delete || die
	fi

	if use wizard; then
		dobin "$(cargo_target_dir)"/kio-protondrive-wizard
		domenu wizard/kio-protondrive-wizard.desktop
		insinto /usr/share/metainfo
		doins wizard/kio-protondrive-wizard.metainfo.xml
		insinto /usr/share/kio-protondrive-wizard/qml
		doins wizard/qml/*.qml
		insinto /usr/share/kio-protondrive-wizard/translations
		doins wizard/translations/*.qm
	fi
}

pkg_postinst() {
	xdg_pkg_postinst

	elog "kio-protondrive drives Proton's own proton-drive CLI, which is not"
	elog "packaged in Gentoo: install it from https://proton.me/drive/download"
	use wizard && elog "or let kio-protondrive-wizard install it for you."
	if use daemon; then
		elog
		elog "Start the sync daemon for your user with:"
		elog "  systemctl --user enable --now kio-protondrive-sync-daemon.service"
	fi
	optfeature "pin/unpin progress notifications" x11-libs/libnotify
	use wizard && optfeature "GPG-encrypted session storage" app-admin/pass
}
