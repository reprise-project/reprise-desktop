#!/bin/bash
# Upgrades the system and installs PKG... as root. ARCH_ARCHIVE (YYYY/MM/DD) pins the repos
# to the Arch Linux Archive; empty keeps the image's mirrors.
set -euo pipefail

if [ -n "${ARCH_ARCHIVE:-}" ]; then
  echo "Server = https://archive.archlinux.org/repos/$ARCH_ARCHIVE/\$repo/os/\$arch" > /etc/pacman.d/mirrorlist
fi
# The archive throttles a single file to a stall; the options also cover makepkg -s.
sed -i -e '/^#\?ParallelDownloads\b/d' -e '/^#\?DisableDownloadTimeout\b/d' \
  -e '0,/^\[options\]/s//&\nParallelDownloads = 5\nDisableDownloadTimeout/' /etc/pacman.conf

# Only the download is retried and timed out, so a kill never lands mid-install; the cache
# keeps what earlier tries fetched.
for try in 1 2 3; do
  if timeout 20m pacman -Syyuuw --noconfirm --needed "$@"; then
    break
  fi
  if [ "$try" = 3 ]; then
    exit 1
  fi
  rm -f /var/lib/pacman/db.lck
  echo "pacman download failed (try $try of 3), retrying in 10 s" >&2
  sleep 10
done
pacman -Suu --noconfirm --needed "$@"
