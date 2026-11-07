//! Pinned image hashes in `tests/pins/<name>.txt`, drawn with [`fonts`]. CONTRIBUTING.md covers
//! blessing them, `scripts/pin-env` and `REPRISE_TEST_STRICT`.
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static STRAYS: RefCell<Vec<Box<str>>> = const { RefCell::new(Vec::new()) };
}

const STRAYS_KEPT: usize = 16;

/// The `scripts/test-fonts.sh` directory pins draw with. Panics without it and on any glyph this
/// thread draws from a font outside the set: drawn before the call, or where drawn after it.
#[track_caller]
pub fn fonts() -> PathBuf {
    arm(crate::text::test_fonts())
}

#[track_caller]
fn arm(test_fonts: Result<PathBuf, Option<PathBuf>>) -> PathBuf {
    let dir = match test_fonts {
        Ok(dir) => dir,
        Err(None) => panic!(
            "pixel pins draw only with the test fonts: run under `scripts/pin-env`, or set \
             REPRISE_TEST_FONTS=$(scripts/test-fonts.sh)"
        ),
        Err(Some(dir)) => panic!(
            "REPRISE_TEST_FONTS={} lacks the test fonts (no Tahoma): point it at \
             $(scripts/test-fonts.sh), or run under `scripts/pin-env`",
            dir.display()
        ),
    };
    let strays = STRAYS.take();
    assert!(
        strays.is_empty(),
        "a pin drew with fonts outside the test set (run it under `scripts/pin-env`):\n  {}",
        strays.join("\n  ")
    );
    ARMED.set(true);
    dir
}

/// Notes a glyph drawn from `face`, a font outside the test set: a panic once [`fonts`] ran on
/// this thread, else kept for it.
pub(crate) fn stray(face: &str) {
    assert!(
        !ARMED.get(),
        "a pin drew with a font outside the test set (run it under `scripts/pin-env`): {face}"
    );
    STRAYS.with_borrow_mut(|s| {
        if s.len() < STRAYS_KEPT && !s.iter().any(|f| **f == *face) {
            s.push(face.into());
        }
    });
}

/// Reports a test that cannot run on this machine: panics under `REPRISE_TEST_STRICT`, else
/// prints `reason`.
#[track_caller]
pub fn skipped(reason: &str) {
    let strict = std::env::var_os("REPRISE_TEST_STRICT").is_some_and(|v| v != "0");
    assert!(
        !strict,
        "REPRISE_TEST_STRICT is set, so this test may not skip: {reason}"
    );
    eprintln!("skipped: {reason}");
}

/// Returns from the calling test after [`skipped`]. `skip!(face "Segoe UI")` is for a font CI does
/// not install: it prints and returns even under `REPRISE_TEST_STRICT`.
#[cfg(test)]
macro_rules! skip {
    (face $face:expr) => {{
        eprintln!("skipped: no {} installed", $face);
        return;
    }};
    ($reason:expr) => {{
        $crate::pins::skipped($reason);
        return;
    }};
}
#[cfg(test)]
pub(crate) use skip;

/// [`check_file`] on `tests/pins/<name>.txt`.
#[cfg(test)]
#[track_caller]
pub(crate) fn check(name: &str, got: &[(String, u64)]) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pins")
        .join(format!("{name}.txt"));
    check_file(&path, got);
}

/// Calls [`fonts`], then fails listing every key whose hash differs from the pin file at `path`, or
/// rewrites the file with `got` when `REPRISE_BLESS` is set.
#[track_caller]
pub fn check_file(path: &Path, got: &[(String, u64)]) {
    use std::fmt::Write as _;
    fonts();
    let name = path.file_stem().unwrap_or_default().display();
    let mut text = String::new();
    for (i, (key, hash)) in got.iter().enumerate() {
        assert!(
            got[..i].iter().all(|(k, _)| k != key),
            "{name}: key {key:?} is drawn twice"
        );
        writeln!(text, "{key} {hash:016x}").unwrap();
    }
    if std::env::var_os("REPRISE_BLESS").is_some_and(|v| v != "0") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if std::fs::read_to_string(path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(path, text).unwrap();
        }
        return;
    }
    let pinned: Vec<(String, u64)> = std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (key, hash) = l.rsplit_once(' ')?;
            Some((key.to_string(), u64::from_str_radix(hash, 16).ok()?))
        })
        .collect();
    let pinned_of = |key: &str| pinned.iter().find(|(k, _)| k == key).map(|p| p.1);
    let mut msg = String::new();
    for (key, hash) in got {
        match pinned_of(key) {
            Some(want) if want == *hash => {}
            Some(want) => writeln!(msg, "  {key}: pinned {want:016x}, drawn {hash:016x}").unwrap(),
            None => writeln!(msg, "  {key}: not pinned, drawn {hash:016x}").unwrap(),
        }
    }
    for (key, _) in &pinned {
        if !got.iter().any(|(k, _)| k == key) {
            writeln!(msg, "  {key}: pinned, no longer drawn").unwrap();
        }
    }
    assert!(
        msg.is_empty(),
        "{} differs:\n{msg}REPRISE_BLESS=1 under `scripts/pin-env` rewrites it",
        path.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "REPRISE_TEST_FONTS=$(scripts/test-fonts.sh)")]
    fn a_pin_without_the_test_fonts_refuses() {
        arm(Err(None));
    }

    #[test]
    #[should_panic(expected = "REPRISE_TEST_FONTS=/nowhere lacks the test fonts")]
    fn a_pin_with_a_test_fonts_dir_lacking_tahoma_refuses() {
        arm(Err(Some(PathBuf::from("/nowhere"))));
    }

    #[test]
    #[should_panic(expected = "a pin drew with fonts outside the test set")]
    fn a_stray_glyph_before_the_pin_fails_it() {
        stray("Trebuchet MS (/usr/share/fonts/trebuc.ttf) for [\"Trebuchet MS\"]");
        check("never_written", &[]);
    }
}
