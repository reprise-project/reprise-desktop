#!/bin/bash
# Builds a .deb of this checkout for Ubuntu 26.04, on Ubuntu 26.04, with the
# packages in packaging/deb/build-depends installed:
#
#     packaging/deb/build.sh [OUT.deb]
#
# REPRISE_DEB_DIR moves the scratch tree (cargo's target/ and the package root).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
scratch=${REPRISE_DEB_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/reprise-deb}
root=$scratch/root

# The local PKGBUILD's build() and package() hold the crate list and the file
# list, so the .deb runs them as they are.
srcdir=$scratch
pkgdir=$root
pkgbase=reprise
REPRISE_REPO=$repo
. "$repo/packaging/arch/local/PKGBUILD"

version=$(pkgver)
out=${1:-$scratch/reprise_${version}_amd64.deb}
rm -rf "$root"
mkdir -p "$root/DEBIAN"
(build)
(package)

# Ubuntu has common-auth and the like where Arch has system-auth, and no
# system-login or system-local-login: those become reprise-system-login.
install -Dm644 "$here/pam.d/reprise-system-login" "$root/etc/pam.d/reprise-system-login"
sed -Ei '/^#/!{
  s/^(-?)(auth|account|password|session)(\s+)include(\s+)system-auth\b/\1\2\3include\4common-\2/
  s/include(\s+)system-(local-)?login\b/include\1reprise-system-login/
}' "$root"/etc/pam.d/*
if grep -nE '^[^#]*include\s+system-' "$root"/etc/pam.d/*; then
  echo "build.sh: a PAM file still includes an Arch stack" >&2
  exit 1
fi

mapfile -t elves < <(find "$root/usr" -type f -perm -u+x)
# The release profile's line tables make each binary about four times its
# size; backtraces keep function names without them.
strip --strip-debug "${elves[@]}"

# dpkg-shlibdeps finds the package's name in debian/control.
mkdir -p "$scratch/debian"
printf 'Source: reprise\n\nPackage: reprise\nArchitecture: amd64\n' >"$scratch/debian/control"
shlibs=$(cd "$scratch" && dpkg-shlibdeps -O "${elves[@]}" | sed -n 's/^shlibs:Depends=//p')

# Depends adds what Reprise needs but doesn't link: the daemons it talks to and
# the libraries and drivers it loads at run time.
cat >"$root/DEBIAN/control" <<EOF
Package: reprise
Version: $version
Architecture: amd64
Maintainer: reprise-project
Installed-Size: $(du -sk --exclude=DEBIAN "$root" | cut -f1)
Section: x11
Priority: optional
Homepage: https://github.com/reprise-project/reprise-desktop
Depends: $shlibs, systemd, libpam-systemd, libpam-modules, dbus (>= 1.15), pipewire (>= 1.4), xwayland, libegl1, libegl-mesa0, libgl1-mesa-dri, xkb-data
Recommends: wireplumber, pipewire-pulse, xdg-desktop-portal, xdg-utils, shared-mime-info, fonts-dejavu-core
Suggests: fonts-noto-cjk, xfonts-wqy, fonts-noto-color-emoji, bubblewrap, papers, ffmpegthumbnailer, gnome-font-viewer, libgsf-bin, libarchive-tools, upower, power-profiles-daemon, xdg-desktop-portal-wlr, xdg-desktop-portal-gtk, gnome-themes-extra, qt6ct, qt5ct, sound-theme-freedesktop, network-manager, dnsmasq-base, network-manager-openvpn, bluez, kdeconnect, udisks2, packagekit, flatpak, fwupd, accountsservice, cups, cups-filters, cups-pk-helper, avahi-daemon, polkitd, pkexec, gnome-keyring, libpam-gnome-keyring, libpam-kwallet5, fprintd, libpam-fprintd, ddcutil, at-spi2-core, libx11-data, hwdata, geoclue-2.0, tzdata, intel-media-va-driver
Description: Wayland desktop with a classic retro look
 Reprise is a Wayland desktop with a classic retro look, with Notepad,
 Explorer and a settings sheet.
EOF
(cd "$root" && find etc -type f | sed 's|^|/|') >"$root/DEBIAN/conffiles"
install -m755 "$here/postinst" "$root/DEBIAN/postinst"

dpkg-deb --root-owner-group --build "$root" "$out"
