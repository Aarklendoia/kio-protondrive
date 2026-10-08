# CI image for .github/workflows/build-gentoo.yml (#194): a Gentoo stage3
# with the Portage tree, the binhost set up and kde-misc/kio-protondrive's
# dependencies and the packaging tooling already installed, so a run only
# builds the package itself. That workflow builds and pushes it to GHCR
# when its tag (this file's hash + the ISO week) doesn't exist yet, so it's
# rebuilt weekly and whenever this file changes.
FROM gentoo/stage3:systemd

# GitHub's runners have no IPv6, but Portage's wget sometimes only tries
# distfiles.gentoo.org's IPv6 addresses and the whole emerge fails (#188).
RUN echo 'inet4_only = on' >> /etc/wgetrc

# The official binhost provides KF6/Qt6/Rust prebuilt for this profile.
# pkgcheck only reads /etc/portage/repos.conf, not Portage's built-in
# default for ::gentoo. Tests (src_test: cargo test, ctest) are enabled for
# this package only: a global FEATURES=test turns USE=test on for every
# dependency, which no longer matches the binhost's packages.
RUN emerge-webrsync --quiet \
 && eselect profile set default/linux/amd64/23.0/desktop/plasma/systemd \
 && getuto \
 && echo 'FEATURES="${FEATURES} getbinpkg binpkg-request-signature"' >> /etc/portage/make.conf \
 && mkdir -p /etc/portage/package.accept_keywords /etc/portage/repos.conf /etc/portage/env \
 && cp /usr/share/portage/config/repos.conf /etc/portage/repos.conf/gentoo.conf \
 && echo 'kde-misc/kio-protondrive ~amd64' > /etc/portage/package.accept_keywords/kio-protondrive \
 && echo 'dev-build/corrosion ~amd64' > /etc/portage/package.accept_keywords/corrosion \
 && echo 'FEATURES="test"' > /etc/portage/env/test.conf \
 && echo 'kde-misc/kio-protondrive test.conf' > /etc/portage/package.env

# The ebuild's dependencies (all USE flags on, see
# packaging/gentoo/kio-protondrive.ebuild) and the tooling the workflow
# uses. A dependency the ebuild gains later is still installed by the
# workflow's emerge, just not ahead of time. Portage's sandboxes need
# privileges `docker build` doesn't have; the workflow's container keeps
# them (--privileged).
RUN FEATURES="-ipc-sandbox -network-sandbox -pid-sandbox" \
    emerge --quiet-build --getbinpkg --usepkg --noreplace \
      dev-lang/rust-bin \
      dev-build/corrosion \
      dev-db/sqlite \
      dev-qt/qtbase:6 \
      dev-qt/qtdeclarative:6 \
      dev-qt/qttools:6 \
      kde-frameworks/extra-cmake-modules \
      kde-frameworks/kcoreaddons:6 \
      kde-frameworks/ki18n:6 \
      kde-frameworks/kio:6 \
      kde-frameworks/kirigami:6 \
      kde-frameworks/kwidgetsaddons:6 \
      kde-frameworks/qqc2-desktop-style:6 \
      net-misc/curl \
      sys-devel/gettext \
      dev-vcs/git \
      app-portage/pycargoebuild \
      app-portage/portage-utils \
      dev-util/pkgcheck \
 && rm -rf /var/cache/binhost/* /var/cache/binpkgs/* /var/cache/distfiles/* /var/tmp/portage/*
