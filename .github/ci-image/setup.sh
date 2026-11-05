#!/bin/bash
# Installs what CI builds and tests with, as root: the PKGBUILD's depends, the fallback font
# and MIME database the tests read, cargo-nextest and the unprivileged build user.
set -euo pipefail
here=$(dirname "$0")
mapfile -t packages <"$here/packages"
bash "$here/../pacman.sh" "${packages[@]}"
curl -LsSf -o /tmp/nextest.tar.gz https://get.nexte.st/0.9.149/linux
echo '32d1d8c0567b7a80bee2bad8bb09125c846da96bc009b2de11ec08557290c770  /tmp/nextest.tar.gz' | sha256sum -c
tar xzf /tmp/nextest.tar.gz -C /usr/local/bin
rm /tmp/nextest.tar.gz
# Tests that assert a read-only directory refuses a write only mean something as an
# unprivileged user; the container runs as root.
id builder >/dev/null 2>&1 || useradd -m builder
