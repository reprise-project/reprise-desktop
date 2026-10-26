//! Pinned image hashes kept in `tests/pins/<name>.txt`, one `key hash` line per image.
//! `REPRISE_BLESS=1 cargo test -p reprisekit` rewrites the files from what the tests draw.
//! A test that cannot run here reports it through [`skipped`]; `REPRISE_TEST_STRICT=1` (CI) turns
//! that into a failure.

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

/// Fails listing every key whose hash differs from `tests/pins/<name>.txt`, or rewrites
/// the file with `got` when `REPRISE_BLESS` is set.
#[cfg(test)]
#[track_caller]
pub(crate) fn check(name: &str, got: &[(String, u64)]) {
    use std::fmt::Write as _;
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/pins")
        .join(format!("{name}.txt"));
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
        if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            std::fs::write(&path, text).unwrap();
        }
        return;
    }
    let pinned: Vec<(String, u64)> = std::fs::read_to_string(&path)
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
        "tests/pins/{name}.txt differs:\n{msg}REPRISE_BLESS=1 cargo test -p reprisekit rewrites it"
    );
}
