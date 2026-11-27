#!/bin/sh
# makepkg on this checkout without makepkg touching it: the PKGBUILD is
# copied into a scratch startdir, so pkgver()'s result is written to the
# copy, and the build tree and the package land under the cache
# directory rather than beside the source. Arguments go to makepkg:
#
#     packaging/arch/local/build.sh -si
#
# REPRISE_MAKEPKG_DIR moves the scratch somewhere else.
#
# The build tree keeps cargo's target/ between runs. Cargo never deletes a
# unit, so a new rustc or Cargo.lock would leave the old copy of every
# dependency there: target/ is removed when either changed since the last run.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
scratch=${REPRISE_MAKEPKG_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/reprise-makepkg}
mkdir -p "$scratch/start" "$scratch/build" "$scratch/pkg"
key=$({ RUSTUP_TOOLCHAIN=stable rustc -vV 2>/dev/null || true; cat "$repo/Cargo.lock"; } | sha256sum | cut -d' ' -f1)
if [ "$(cat "$scratch/target.key" 2>/dev/null)" != "$key" ]; then
    rm -rf "$scratch"/build/*/src/target
    echo "$key" >"$scratch/target.key"
fi
cp "$here/PKGBUILD" "$scratch/start/PKGBUILD"
cd "$scratch/start"
REPRISE_REPO="$repo" BUILDDIR="$scratch/build" PKGDEST="$scratch/pkg" exec makepkg "$@"
