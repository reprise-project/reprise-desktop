# Contributing

Issues and pull requests are welcome. For anything bigger than a fix, open
an issue first so we can agree on the shape before you write it.

## Build

[docs/install.md](docs/install.md) lists the requirements and how to build
and run Reprise, nested in your current session or from a login manager.

## Before you open a pull request

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace

CI runs the same three commands in an `archlinux:latest` container as an
unprivileged user (`.github/workflows/ci.yml` installs the packages). Run
the tests as a normal user too: some check that a read-only directory
refuses a write, which root ignores.

Pixel tests that depend on the UI font are pinned to Wine 9.0's Tahoma.
`scripts/test-fonts.sh` downloads it into your cache and prints the
directory; the tests load Tahoma from there instead of the system:

    export REPRISE_TEST_FONTS=$(scripts/test-fonts.sh)
    cargo test --workspace

Without `REPRISE_TEST_FONTS` those tests skip, as do the ones that pin
Trebuchet MS or Segoe UI glyphs when those fonts aren't installed. CI sets
it.

- **Pixel-neutral changes.** If a change shouldn't move a pixel, prove it
  with `scripts/pixels-unchanged.sh [--with-blue] BASE`. It renders the
  gallery and the preview tests on BASE and on your tree and lists every
  PNG that differs.
- **Dependency changes.** After changing a dependency or a feature in any
  `Cargo.toml`, run `scripts/kit-units`. It must report one reprisekit
  unit; otherwise add the missing features to `crates/reprisekit-pins`.
  A new dependency also goes into the Requirements section of
  docs/install.md and the PKGBUILDs under `packaging/arch/`.

## How the code is laid out

Everything drawn or interactive is a control in `crates/reprisekit`, in five
parts that each take the rect they fill:

- a pure model: state and keys
- a layout
- `draw`, into a `Canvas`
- `hit`, for the pointer
- `access`, which adds its nodes to the accessibility tree

reprisekit knows nothing of Smithay, seats, timers, the clipboard or the
file system. The hosts are glue: `src/*view.rs` in the compositor and
`crates/repriseapp` for the programs. When both hosts need something, it
goes into reprisekit. Extend an existing control before writing a new one.
Code for one look lives in its interface module (`classic`, `blue`,
`glass`); `crates/reprisekit/tests/layers.rs` checks which modules may
reach which.

A control is done when a unit test pins its pixels and
`cargo run -p reprisekit --example gallery` draws it.

## Art

Art is drawn in code from shapes and named colours, so it follows the
colour scheme. The exception is the small icons, which are
PNGs in `crates/reprisekit/src/art/icons`.

## Commits

[Conventional commits](https://www.conventionalcommits.org/):
`fix(settings): ...`, `feat(glass): ...`, `chore: ...`. Say what changed
and which tests pin it. One change per commit.
