#!/bin/sh
# Prints the CI image's name: the archive date and a hash of everything that builds it.
# --check fails unless every workflow names that image.
set -eu
cd "$(dirname "$0")/.."
date=$(sed -n 's|^ARG ARCH_ARCHIVE=||p' ci-image/Dockerfile | tr / -)
hash=$(cat ci-image/Dockerfile ci-image/packages ci-image/setup.sh pacman.sh | sha256sum | cut -c1-12)
image="ghcr.io/reprise-project/reprise-ci:$date-$hash"
if [ "${1:-}" = --check ]; then
  stale=$(grep -rhoE 'ghcr\.io/reprise-project/reprise-ci:[^ ]+' workflows | grep -vxF "$image" || true)
  if [ -n "$stale" ]; then
    echo "the workflows name $stale; .github/ci-image builds $image" >&2
    exit 1
  fi
fi
echo "$image"
