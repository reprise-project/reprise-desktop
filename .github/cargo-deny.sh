#!/bin/bash
# Installs rust and cargo-deny $version as root. ARCH_ARCHIVE's date carries cargo-deny $version,
# which deny.toml is written for. When the archive does not answer, rust and git (the advisory
# fetch runs it) come from the image's mirrors and cargo-deny from its release tarball, checked
# against $sha256. Bumping ARCH_ARCHIVE means bumping both; the version check at the end catches
# a missed one.
set -euo pipefail

version=0.20.2
sha256=9f12ed4c49936e09b48bf862b595cde2fe64fcbd9d74dfacac6131ca824c8d5f

# Three tries of 20 s, 10 s apart: gives up within 80 s.
probe="https://archive.archlinux.org/repos/${ARCH_ARCHIVE:?}/core/os/x86_64/core.db"
if curl -fsS -o /dev/null --connect-timeout 10 --max-time 20 --retry 2 --retry-delay 10 "$probe"; then
  bash "$(dirname "$0")/pacman.sh" rust cargo-deny
else
  echo "::warning::archive.archlinux.org does not answer for $ARCH_ARCHIVE: rust and git from the image's mirrors, cargo-deny $version from its release tarball"
  ARCH_ARCHIVE='' bash "$(dirname "$0")/pacman.sh" rust git
  name="cargo-deny-$version-x86_64-unknown-linux-musl"
  dir=$(mktemp -d)
  curl -fsSL --retry 3 -o "$dir/$name.tar.gz" \
    "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/$name.tar.gz"
  echo "$sha256  $dir/$name.tar.gz" | sha256sum -c -
  tar -xzf "$dir/$name.tar.gz" -C "$dir"
  install -m 755 "$dir/$name/cargo-deny" /usr/local/bin/cargo-deny
fi

installed=$(cargo deny --version)
if [ "$installed" != "cargo-deny $version" ]; then
  echo "::error::$installed installed, deny.toml is written for cargo-deny $version" >&2
  exit 1
fi
