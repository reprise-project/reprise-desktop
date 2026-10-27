#!/bin/bash
# Upgrades the system and installs PKG... as root. ARCH_ARCHIVE (YYYY/MM/DD) pins the repos
# to the Arch Linux Archive; empty keeps the image's mirrors.
set -euo pipefail

if [ -n "${ARCH_ARCHIVE:-}" ]; then
  echo "Server = https://archive.archlinux.org/repos/$ARCH_ARCHIVE/\$repo/os/\$arch" > /etc/pacman.d/mirrorlist
fi

# Only the download is retried and timed out, so a kill never lands mid-install. A killed
# try's files are not reused; a call gives up within 42 min, inside every job's timeout-minutes.
backoff=(30s 2m 5m)
for try in 1 2 3 4; do
  if timeout -k 30s 8m pacman -Syyuuw --noconfirm --needed --disable-download-timeout "$@"; then
    break
  fi
  if [ "$try" = 4 ]; then
    exit 1
  fi
  rm -f /var/lib/pacman/db.lck
  echo "pacman download failed (try $try of 4), retrying in ${backoff[try - 1]}" >&2
  sleep "${backoff[try - 1]}"
done
pacman -Suu --noconfirm --needed "$@"
