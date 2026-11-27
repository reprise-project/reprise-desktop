# Contributing

Issues and pull requests are welcome. Open an issue first for anything larger than a bug fix.

## Build

See [docs/install.md](docs/install.md) for requirements and how to build and run Reprise.

## Run checks before opening a pull request

    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace

`cargo nextest` ([cargo-nextest](https://nexte.st) 0.9.149+) reads `.config/nextest.toml` and runs each test in its own process. `cargo test --workspace` runs the same tests without it.

Run `scripts/affected-tests [BASE]` (default `origin/main`) to test only changed crates, their dependents and whole-tree checks.

CI runs these checks as a non-root user in an Arch image from `.github/ci-image/`. Run tests as a normal user too, because root bypasses read-only directory checks. If you edit `.github/ci-image/` or `.github/pacman.sh`, update the image tag in the workflows using `.github/ci-image/tag.sh`.

New crates whose tests touch HOME, XDG folders, D-Bus or a display add `#[cfg(test)] reprisebus::test_session!();` at the crate root. `reprisekit` isolates its environment in `src/testsession.rs`.

## Run pixel and UI checks

Pixel tests pin Wine 9.0 Tahoma, Noto Color Emoji 2.051 and DejaVu. Run them through `scripts/pin-env`:

    scripts/pin-env cargo nextest run --workspace

`scripts/pin-env` sets `REPRISE_TEST_FONTS` (using `scripts/test-fonts.sh`) and `REPRISE_TEST_STRICT=1` under a DejaVu-only fontconfig. Pixel tests fail without `REPRISE_TEST_FONTS` or if they load an unlisted font. Painters that request another family take the font as a parameter so tests can pass Tahoma.

- **D-Bus fakes.** Serve fakes with `Builder::serve_at` or `reprisebus::testbus::serve` instead of `ObjectServer::at` (`crates/reprisebus/tests/served.rs` enforces this). Connect with `testbus::connect` (10 s timeout).
- **Pixel-neutral changes.** Run `scripts/pixels-unchanged.sh [--with-blue] BASE` to confirm no gallery or preview PNG changed.
- **Pinned hashes.** Store hashes in `tests/pins/` via `pins::check` or `pins::check_file`, never inline (`crates/reprisechecks/tests/inline_pins.rs` rejects inline 64-bit literals). Run `REPRISE_BLESS=1 scripts/pin-env cargo test -p <crate>` after intentional visual changes and review the diff.
- **Text at 200%.** `clip_test` checks gallery pages at 200% text against `crates/reprisekit/tests/pins/text_clip_200.txt`. Size rows from `theme.controls()` or `theme.text`.
- **Text without Tahoma.** `fit_test` and app `text_fit` tests compare Tahoma against DejaVu Sans Book (`pins::without_tahoma`) using `tests/pins/text_fit_without_tahoma.txt`. Size controls from measured text (`Form` fit or `ListView::fit_titles`).
- **UI strings.** Wrap user-visible text in `tr!("id")` with English in `locale/en.ftl`. `cargo test -p reprisekit` runs `scripts/i18n-check`. See [docs/i18n.md](docs/i18n.md).
- **Dependency changes.** Run `scripts/kit-units` (must report one `reprisekit`, `zbus` and `reprisebus` unit, fixing splits in `crates/reprisekit-pins` or `crates/repriseapp-pins`) and `scripts/feature-drift` (must print nothing). Update `docs/install.md`, `packaging/arch/` PKGBUILDs and `packaging/deb` (`build-depends` for linked `-dev` libs, `Depends` or `Recommends` in `build.sh` for runtime deps).

## Debug stalls

A main-loop turn over 2 s logs the main thread's backtrace, state and syscall to the journal, repeating at 10 s and every 30 s. Test it with two env vars:

- `REPRISE_STALL_ON_RELOAD=<ms>`: blocks the main loop for `ms` when a file in the config directory changes.
- `REPRISE_COMMIT_DELAY=<connector>:<ms>`: delays commits on that connector (append `:modeset`, such as `HDMI-A-1:5000:modeset`, to delay only modesets).

## Understand the code layout

Every UI element is a control in `crates/reprisekit` with five parts that each take their rect: a pure model (state and keys), a layout, `draw` into a `Canvas`, `hit` for pointer input and `access` for accessibility nodes.

`reprisekit` has no Smithay, seat, timer, clipboard or filesystem code. Hosts (`src/*view.rs` and `crates/repriseapp`) are glue only. Shared behaviour belongs in `reprisekit`, and you should extend existing controls before adding new ones.

Interface-specific code lives in `classic`, `blue` or `glass` (`crates/reprisekit/tests/layers.rs` enforces module boundaries). Controls and hosts draw shared widgets via `reprisekit::look::widgets` with `theme.paint()`. Only Classic painters call `classic::widgets` directly.

Programs in `crates/apps` are libraries compiled into one `reprise-apps` binary, selected by symlink name or first argument (`cargo run -p reprise-apps -- reprise-notepad`).

A control is done when a unit test pins its pixels, `cargo run -p reprisekit --example gallery` draws it (`crates/reprisekit/examples/gallery_*`), and one host displays it.

## Draw art

Draw art in code from shapes and named colours so it follows colour schemes. Only small icons are PNGs, stored in `crates/reprisekit/src/art/icons`.

## Write commits

Use [Conventional Commits](https://www.conventionalcommits.org/): `fix(settings): ...`, `feat(glass): ...`, `chore: ...`. Put one change per commit and state which tests pin it.
