#!/usr/bin/env bash
# Renders the gallery (every colour scheme, Classic bands only) and every
# preview test on BASE and on the working tree, and lists each PNG that
# differs or exists on one side only. Exits 1 on any difference, and without
# a list when a side wrote no PNGs or more than 5 fewer than the other.
# --with-blue also renders the gallery with its Blue bands, every page saved as
# with-blue-<name>, and compares Blue previews; without it, PNGs with "blue"
# in the name are skipped on both sides. The gallery draws on one thread per
# heavy job (CARGO_BUILD_JOBS), else per CPU.
# BASE must include the gallery's REPRISE_SCHEME=all mode.
# Usage: scripts/pixels-unchanged.sh [--with-blue] [--no-cache] BASE
#
# Each BASE builds in its own kept worktree, ~/.cache/reprise-pixels/base-<sha12>;
# runs on the same commit serialize on its lock. A new tree seeds target/
# from the most recently used one; only that tree is kept.
# The base render is cached in out-classic/ (out-blue/) with a .complete file
# holding this script's sha256; --no-cache renders BASE afresh.
set -euo pipefail

# A caller's environment must not make a render vacuous. Both sides render
# with the system fonts: a BASE that predates REPRISE_TEST_FONTS ignores it.
unset REPRISE_SCHEME REPRISE_GALLERY_LOOK REPRISE_TEST_FONTS

with_blue=0
use_cache=1
while [[ ${1:-} == --* ]]; do
    case $1 in
        --with-blue) with_blue=1 ;;
        --no-cache) use_cache=0 ;;
        *) echo "pixels-unchanged.sh: unknown option $1" >&2; exit 1 ;;
    esac
    shift
done
base=${1:?usage: scripts/pixels-unchanged.sh [--with-blue] [--no-cache] BASE}

root=$(git rev-parse --show-toplevel)
base_sha=$(git -C "$root" rev-parse --verify "${base}^{commit}" 2>/dev/null) || {
    echo "pixels-unchanged.sh: BASE '$base' is not a commit in this repo" >&2
    exit 1
}
sha12=${base_sha:0:12}

cachedir=${XDG_CACHE_HOME:-$HOME/.cache}/reprise-pixels
basewt="$cachedir/base-$sha12"
kept="$basewt/out-classic"
((with_blue)) && kept="$basewt/out-blue"
script_hash=$(sha256sum "${BASH_SOURCE[0]}" | cut -d' ' -f1)
mkdir -p "$cachedir"

work=$(mktemp -d "${TMPDIR:-/tmp}/reprise-pixels.XXXXXX")
trap 'rm -rf "$work"' EXIT

# An empty folder for render_browse_preview to list, so both sides match.
mkdir -p "$work/preview-dir"
export REPRISE_PREVIEW_DIR="$work/preview-dir"

# seed_new_base DIR: copy target/ from the most recently used base tree so the
# first build is incremental. Skipped if that tree's lock is held.
seed_new_base() {
    local dir=$1
    local best="" best_time=0 d t
    for d in "$cachedir"/base-*; do
        [[ -d $d && $d != "$dir" && -f $d/.last-used ]] || continue
        t=$(stat -c %Y "$d/.last-used" 2>/dev/null) || continue
        if ((t > best_time)); then best_time=$t; best=$d; fi
    done
    [[ -n $best && -d $best/target ]] || return 0
    exec 8>"$best.lock"
    if flock -n 8; then
        mkdir -p "$dir/target"
        cp -a --reflink=auto "$best/target/." "$dir/target/" 2>/dev/null || true
    fi
    exec 8>&-
}

# A worktree removed by hand stays registered and makes a later
# `git worktree add` for the same path die, so prune first.
quiet_prune() {
    local out
    if ! out=$(git -C "$root" worktree prune 2>&1); then
        echo "pixels-unchanged.sh: git worktree prune failed:" >&2
        echo "$out" >&2
    fi
}

# Keep the most recently used base tree; remove older ones whose lock is free.
prune_old_bases() {
    local keep=1 d t entries sorted rest entry
    entries=()
    for d in "$cachedir"/base-*; do
        [[ -d $d ]] || continue
        t=0
        [[ -f $d/.last-used ]] && t=$(stat -c %Y "$d/.last-used" 2>/dev/null || echo 0)
        entries+=("$t $d")
    done
    ((${#entries[@]} > keep)) || return 0
    mapfile -t sorted < <(printf '%s\n' "${entries[@]}" | sort -rn)
    rest=("${sorted[@]:$keep}")
    for entry in "${rest[@]}"; do
        d=${entry#* }
        exec 7>"$d.lock"
        if flock -n 7; then
            git -C "$root" worktree remove --force "$d" 2>/dev/null || rm -rf "$d"
            rm -f "$d.lock"
        fi
        exec 7>&-
    done
}

# Ignored tests that draw no PNG: session-bus tests claim live bus names, encoders fail on
# an old BASE without a GPU, and the rest time or probe the machine (typing_cost for minutes).
skips=(
    --skip _on_a_session_bus --skip encode::
    --skip greeterview::tests::typing_cost
    --skip taskbarview::tests::bar_redraw_cost
    --skip notifysheet::tests::droplets::opening_frame_cost
    --skip notifysheet::tests::droplets::droplet_memory
    --skip notifysheet::tests::repaint::stack_paint_cost
    --skip logon::transition::tests::transition_frame_cost
    --skip logon::transition::tests::click_and_settle_cost
    --skip glass::clock::tests::bench_glass_clock
    --skip a_jxl_encode_refused_any_one_allocation_fails_without_aborting
    --skip devices::tests::this_machine
)

# render TREE OUT: copy every PNG the tree writes into target/ to OUT. Failure
# prints the log tail first, since the work directory is trap-deleted.
render() {
    local tree=$1 out=$2
    mkdir -p "$tree/target" "$out"
    local stamp="$work/stamp-$(basename "$out")"
    local glog="$work/gallery-$(basename "$out").log"
    local tlog="$work/test-$(basename "$out").log"
    touch "$stamp"
    (
        cd "$tree"
        export REPRISE_GALLERY_THREADS=${CARGO_BUILD_JOBS:-$(nproc)}
        REPRISE_SCHEME=all REPRISE_GALLERY_LOOK=classic \
            cargo run -q -p reprisekit --example gallery >"$glog" 2>&1 ||
            { echo "gallery render failed in $tree"; tail -n 60 "$glog"; exit 1; }
        cargo test -q --workspace --lib --bins --tests -- --ignored "${skips[@]}" \
            >"$tlog" 2>&1 ||
            { echo "preview tests failed in $tree"; tail -n 60 "$tlog"; exit 1; }
        find target -maxdepth 1 -name '*.png' -newer "$stamp" -exec cp -t "$out" {} +
        if ((with_blue)); then
            touch "$stamp"
            cargo run -q -p reprisekit --example gallery >"$glog" 2>&1 ||
                { echo "blue gallery render failed in $tree"; tail -n 60 "$glog"; exit 1; }
            # Prefixed, so the pages both runs draw keep their Classic copy.
            local f
            for f in target/reprisekit-gallery*.png; do
                if [[ $f -nt $stamp ]]; then cp "$f" "$out/with-blue-${f#target/}"; fi
            done
        fi
    )
    if [[ -z $(find "$out" -maxdepth 1 -name '*.png' -print -quit) ]]; then
        echo "pixels-unchanged.sh: $tree wrote no PNGs into its target/" >&2
        exit 1
    fi
}

exec 9>"$basewt.lock"
flock 9

quiet_prune
started=$SECONDS
if ((use_cache)) && [[ -f $kept/.complete && $(<"$kept/.complete") == "$script_hash" ]]; then
    mkdir -p "$work/a"
    cp -t "$work/a" "$kept"/*.png
    touch "$basewt/.last-used"
    echo "base $sha12: cached"
else
    if [[ ! -d $basewt ]]; then
        git -C "$root" worktree add --detach -q "$basewt" "$base_sha"
        seed_new_base "$basewt"
    fi
    git -C "$basewt" checkout -qf --detach "$base_sha"
    touch "$basewt/.last-used"

    render "$basewt" "$work/a"

    rm -rf "$kept"
    mkdir -p "$kept"
    cp -t "$kept" "$work"/a/*.png
    echo "$script_hash" >"$kept/.complete"
    echo "base $sha12: rendered in $((SECONDS - started)) s"
fi

flock -u 9
exec 9>&-

prune_old_bases
quiet_prune

started=$SECONDS
render "$root" "$work/b"
echo "working tree: rendered in $((SECONDS - started)) s"

# A render that wrote into another checkout's target/ leaves its side short;
# report that rather than a list of hundreds of one-sided names.
na=$(find "$work/a" -maxdepth 1 -name '*.png' | wc -l)
nb=$(find "$work/b" -maxdepth 1 -name '*.png' | wc -l)
if ((na - nb > 5 || nb - na > 5)); then
    echo "pixels-unchanged.sh: $base rendered $na PNGs and the working tree $nb;" \
        "one render wrote elsewhere or skipped tests" >&2
    exit 1
fi

fail=0
compared=0
skipped=0
# Without nullglob an empty directory yields the literal "*.png" and defeats
# the no-PNGs guard below.
shopt -s nullglob
for f in "$work"/a/*.png; do
    n=$(basename "$f")
    if ((!with_blue)) && [[ ${n,,} == *blue* ]]; then
        skipped=$((skipped + 1))
        continue
    fi
    compared=$((compared + 1))
    if [[ ! -e $work/b/$n ]]; then
        echo "only in $base: $n"; fail=1
    elif ! cmp -s "$f" "$work/b/$n"; then
        echo "differs: $n"; fail=1
    fi
done
for f in "$work"/b/*.png; do
    n=$(basename "$f")
    if ((!with_blue)) && [[ ${n,,} == *blue* ]]; then
        [[ -e $work/a/$n ]] || skipped=$((skipped + 1))
        continue
    fi
    [[ -e $work/a/$n ]] || { echo "only in the working tree: $n"; fail=1; }
done
shopt -u nullglob
if ((compared == 0)); then
    echo "no PNGs compared"
    exit 1
fi
echo "$compared PNGs compared, $skipped Blue previews skipped"
exit $fail
