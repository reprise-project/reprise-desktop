# Contributing

Issues and pull requests are welcome. For anything bigger than a fix, open
an issue first so we can agree on the shape before you write it.

## Build

[docs/install.md](docs/install.md) lists the requirements and how to build
and run Reprise, from a TTY or a login manager.

## Before you open a pull request

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

`cargo nextest` ([cargo-nextest](https://nexte.st), 0.9.149 or newer) runs
every test in its own process across all test binaries at once and reads
`.config/nextest.toml`. Without it, `cargo test --workspace` runs the same
tests.
`scripts/affected-tests [BASE]` builds and runs only the tests of the crates
changed since BASE (default `origin/main`) and the crates that depend on them,
plus the tests that read the whole tree (string ids, window ids, licence
citations, packaging), which run on any crate or `packaging/` change.

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

Pixel tests are pinned to Wine 9.0's Tahoma, Noto Color Emoji 2.051 and
DejaVu, the CI image's one system font. `scripts/pin-env` runs a command in
CI's font environment. It sets `REPRISE_TEST_FONTS` to the fonts that
`scripts/test-fonts.sh` downloads into your cache and `REPRISE_TEST_STRICT=1`, under a fontconfig
that sees only DejaVu. It drops your session's `REPRISE_*` variables and
keeps the test and gallery ones such as `REPRISE_BLESS`:

    scripts/pin-env cargo nextest run --workspace

A pixel test without `REPRISE_TEST_FONTS` fails, as does one that draws a
glyph from a font outside that set; a painter that asks for another family
takes its font as a parameter so its test can pass Tahoma. Under
`REPRISE_TEST_STRICT=1` a test that skips for want of `dbus-daemon`, an EGL
device or a font of the set fails instead (`reprisebus::testbus::skipped`,
`reprisekit::pins::skipped`). Still skipping under it: the tests for
Trebuchet MS, Franklin Gothic Medium, Segoe UI and Wine's `sserife.fon`
where those fonts aren't installed, dma-buf uploads with no render node,
kernel TLS without `REPRISE_KTLS_TESTS=1`, the icon theme check with no icon
theme installed, and the zombie test when another test's reaper wins.

- **D-Bus fakes.** A test serves a fake service with `Builder::serve_at` or
  `reprisebus::testbus::serve`, which returns once the connection answers
  calls; zbus drops a call that reaches an interface added with
  `ObjectServer::at` before its object server is up. Callers connect with
  `testbus::connect`, whose calls give up after 10 s.
  `crates/reprisebus/tests/served.rs` fails on `ObjectServer::at` in tests.
- **Pixel-neutral changes.** If a change shouldn't move a pixel, prove it
  with `scripts/pixels-unchanged.sh [--with-blue] BASE`. It renders the
  gallery and the preview tests on BASE and on your tree and lists every
  PNG that differs.
- **Pinned hashes.** Tests that call `pins::check` keep their image hashes
  in `crates/reprisekit/tests/pins/`, one `key hash` line per image. After a
  deliberate drawing change, or a new Blue scheme,
  `REPRISE_BLESS=1 scripts/pin-env cargo test -p reprisekit` rewrites those
  files; review their diff like code.
- **Text at 200%.** The gallery's `clip_test` draws every gallery page with
  its themes at 200% text and lists each line whose rect cuts its ink in
  `crates/reprisekit/tests/pins/text_clip_200.txt`. A line that newly clips
  fails it: lay its row out from `theme.controls()` or `theme.text` instead
  of a 100% size. Bless the list as above once a fix takes a line off it.
- **Text without Tahoma.** Debian and Ubuntu have no Tahoma, so the UI text
  falls back to DejaVu Sans Book, which is wider. The gallery's `fit_test`
  and the Settings, Network Status and Explorer `text_fit` tests draw their
  surfaces with Tahoma and again under `pins::without_tahoma`, and list each
  line DejaVu cuts or ellipsizes more often than Tahoma in the crate's
  `tests/pins/text_fit_without_tahoma.txt` (a line may end in `  # why`). A
  new line fails: size the control from its measured text (a form item
  gets room from `Form`'s fit, a list column from `ListView::fit_titles`).
  A program's property sheets get the same test through
  `interfaces::sheets_unfit_without_tahoma`.
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
  docs/install.md, the PKGBUILDs under `packaging/arch/`, and
  `packaging/deb`: its Ubuntu `-dev` package into `build-depends` when the
  build links it, its runtime package into `Depends` in `build.sh` (or
  `Recommends`, for an optional one) when it's used without linking (a
  daemon, a program, a library loaded at run time). Pull requests build
  only the Arch package, so a missing `.deb` entry shows first in the
  Nightly build.

## Debugging

A main-loop turn that runs for 2 s writes the main thread's backtrace,
state and system call to the journal, again at 10 s and every 30 s after,
and how long it took once it ends. Nothing is stopped or restarted. Two
variables in Reprise's environment provoke a stall to try it:

- `REPRISE_STALL_ON_RELOAD=<ms>` holds the main loop `ms` each time a file
  in the config folder is written (`touch` one).
- `REPRISE_COMMIT_DELAY=<connector>:<ms>` makes every commit on that
  connector wait `ms` first; `HDMI-A-1:5000:modeset` delays only its
  modesets. Each monitor commits from its own thread, so input, programs,
  the lock screen and VT switches carry on while one is held.

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
reach which. Controls and hosts draw buttons, edges and scrollbars through
`reprisekit::look::widgets` with the `look::Paint` from `theme.paint()`, so
each interface draws its own; only Classic's painters call
`classic::widgets` directly. layers.rs doesn't catch that one: a control
calling `classic::widgets` passes and draws Classic bevels under Blue and
Glass.

Each program in `crates/apps` is a library, and one binary, `reprise-apps`,
holds them all. It runs the program named by the symlink it was started
through or by its first argument, so
`cargo run -p reprise-apps -- reprise-notepad` runs one alone. Packages
install `reprise-apps` with a symlink per program.

A control is done when a unit test pins its pixels,
`cargo run -p reprisekit --example gallery` draws it on a page of one of the
`crates/reprisekit/examples/gallery_*` modules, and one host (a
`src/*view.rs` or a program) shows it.

## Art

Art is drawn in code from shapes and named colours, so it follows the
colour scheme. The exception is the small icons, which are
PNGs in `crates/reprisekit/src/art/icons`.

## Commits

[Conventional commits](https://www.conventionalcommits.org/):
`fix(settings): ...`, `feat(glass): ...`, `chore: ...`. Say what changed
and which tests pin it. One change per commit.
