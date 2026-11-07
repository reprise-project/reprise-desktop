# Contributing

Issues and pull requests are welcome. For anything bigger than a fix, open
an issue first so we can agree on the shape before you write it.

## Build

[docs/install.md](docs/install.md) lists the requirements and how to build
and run Reprise, nested in your current session or from a login manager.

## Before you open a pull request

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

`cargo nextest` ([cargo-nextest](https://nexte.st), 0.9.149 or newer) runs
every test in its own process across all test binaries at once and reads
`.config/nextest.toml`. Without it, `cargo test --workspace` runs the same
tests.
`scripts/affected-tests [BASE]` builds and runs only the tests of the crates
changed since BASE (default `origin/main`) and the crates that depend on them.

CI runs the same three commands as an unprivileged user in an Arch Linux
image built from `.github/ci-image/` (`packages` lists what it installs). On
a pull request it tests through `scripts/affected-tests`; main tests the
whole workspace. Run
the tests as a normal user too: some check that a read-only directory
refuses a write, which root ignores.

A change under `.github/ci-image/` or to `.github/pacman.sh` needs a new image
tag: put the output of `.github/ci-image/tag.sh` in the workflows. Pushing the
branch builds the image; rerun the pull request's checks once that finishes.

A crate's tests never see your session: `#[cfg(test)]
reprisebus::test_session!();` at its root gives the test binary an empty
HOME, XDG folders and runtime folder of its own and no session bus, system
bus or display. A new crate whose tests touch any of them adds that line.
reprisekit, which links no bus, sets up its HOME and XDG folders the same
way in `src/testsession.rs`.

Pixel tests are pinned to Wine 9.0's Tahoma and DejaVu, the CI image's one
system font. `scripts/pin-env` runs a command in CI's font environment. It
sets `REPRISE_TEST_FONTS` to the Tahoma that `scripts/test-fonts.sh`
downloads into your cache and `REPRISE_TEST_STRICT=1`, under a fontconfig
that sees only DejaVu. It drops your session's `REPRISE_*` variables and
keeps the test and gallery ones such as `REPRISE_BLESS`:

    scripts/pin-env cargo nextest run --workspace

A pixel test without `REPRISE_TEST_FONTS` fails, as does one that draws a
glyph from a font outside that set; a painter that asks for another family
takes its font as a parameter so its test can pass Tahoma. Under
`REPRISE_TEST_STRICT=1` a skipped test fails too; only the tests for
Trebuchet MS, Franklin Gothic Medium, Segoe UI and Wine's `sserife.fon`
still skip where those fonts aren't installed.

- **Pixel-neutral changes.** If a change shouldn't move a pixel, prove it
  with `scripts/pixels-unchanged.sh [--with-blue] BASE`. It renders the
  gallery and the preview tests on BASE and on your tree and lists every
  PNG that differs.
- **Pinned hashes.** Tests that call `pins::check` keep their image hashes
  in `crates/reprisekit/tests/pins/`, one `key hash` line per image. After a
  deliberate drawing change, or a new Blue scheme,
  `REPRISE_BLESS=1 scripts/pin-env cargo test -p reprisekit` rewrites those
  files; review their diff like code.
- **UI strings.** A string a user reads goes through `tr!("id")` with its
  English text in the crate's `locale/en.ftl`; `cargo test -p reprisekit`
  runs `scripts/i18n-check`, which fails on an id missing from English or
  never used. See [docs/i18n.md](docs/i18n.md).
- **Dependency changes.** After changing a dependency or a feature in any
  `Cargo.toml`, run `scripts/kit-units`. It must report one reprisekit,
  zbus and reprisebus unit; otherwise add the missing features to
  `crates/reprisekit-pins`, or to `crates/repriseapp-pins` when the crate
  is outside reprisekit's own dependency graph.
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

Each program in `crates/apps` is a library with a thin binary, so
`cargo run -p reprise-notepad` runs one alone; packages install them all as
one binary, `reprise-apps`, which runs the program named by the symlink it
was started through.

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
