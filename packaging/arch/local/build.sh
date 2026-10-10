#!/bin/sh
# makepkg on this checkout without makepkg touching it: the PKGBUILD is
# copied into a scratch startdir, so pkgver()'s result is written to the
# copy, and the build tree and the package land under the cache
# directory rather than beside the source. Arguments go to makepkg:
#
#     packaging/arch/local/build.sh -si
#
# REPRISE_MAKEPKG_DIR moves the scratch somewhere else.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
scratch=${REPRISE_MAKEPKG_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/reprise-makepkg}
mkdir -p "$scratch/start" "$scratch/build" "$scratch/pkg"
cp "$here/PKGBUILD" "$scratch/start/PKGBUILD"
cd "$scratch/start"
REPRISE_REPO="$repo" BUILDDIR="$scratch/build" PKGDEST="$scratch/pkg" exec makepkg "$@"
