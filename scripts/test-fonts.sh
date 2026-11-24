#!/usr/bin/env bash
# Fetches the fonts the pixel tests are pinned to (Wine 9.0's Tahoma, LGPL; Noto Color Emoji
# 2.051, OFL) into the XDG cache, checks their sha256 and prints the directory. Idempotent.
# Usage: export REPRISE_TEST_FONTS=$(scripts/test-fonts.sh), or scripts/pin-env CMD.
set -euo pipefail

wine=https://gitlab.winehq.org/wine/wine/-/raw/wine-9.0/fonts
noto=https://raw.githubusercontent.com/googlefonts/noto-emoji/v2.051/fonts
dir=${XDG_CACHE_HOME:-$HOME/.cache}/reprise-test-fonts/wine-9.0-noto-2.051
declare -A urls=(
    [tahoma.ttf]=$wine/tahoma.ttf
    [tahomabd.ttf]=$wine/tahomabd.ttf
    [NotoColorEmoji.ttf]=$noto/NotoColorEmoji.ttf
)
declare -A sums=(
    [tahoma.ttf]=90f1abad4da0ce03da5d2277ee75a9600ce812d37c90eab76e0838ffa7937e55
    [tahomabd.ttf]=c29342ba99a04f983193756f01eb6de946fd4d33ba0bb7061aff11e70871b6f2
    [NotoColorEmoji.ttf]=72a635cb3d2f3524c51620cdde406b217204e8a6a06c6a096ff8ed4b5fd6e27b
)

mkdir -p "$dir"
for name in "${!sums[@]}"; do
    file="$dir/$name"
    sum=${sums[$name]}
    if [[ -f $file ]] && echo "$sum  $file" | sha256sum -c --status; then
        continue
    fi
    curl -fsSL --retry 3 -o "$file.part" "${urls[$name]}"
    if ! echo "$sum  $file.part" | sha256sum -c --status; then
        rm -f "$file.part"
        echo "test-fonts.sh: $name does not have sha256 $sum" >&2
        exit 1
    fi
    mv "$file.part" "$file"
done
echo "$dir"
