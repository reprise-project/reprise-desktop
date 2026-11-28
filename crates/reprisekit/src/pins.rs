//! Pinned image hashes in `tests/pins/<name>.txt`, drawn with [`fonts`]. CONTRIBUTING.md covers
//! blessing them, `scripts/pin-env` and `REPRISE_TEST_STRICT`.
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static STRAYS: RefCell<Vec<Box<str>>> = const { RefCell::new(Vec::new()) };
    static CUTS: RefCell<Option<Vec<Cut>>> = const { RefCell::new(None) };
    static SHORTENED: RefCell<Option<Vec<Shortened>>> = const { RefCell::new(None) };
    static UNFIT: RefCell<Option<Unfit>> = const { RefCell::new(None) };
}

/// Per surface, how and text, each line cut short: how many times, and where the first was.
type Unfit = BTreeMap<String, (u32, String)>;

/// Text cut to end in an ellipsis, as [`shortened_text`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortened {
    /// The whole text, before the cut.
    pub text: String,
    /// The width it was cut to, px at the canvas's scale 1.
    pub width: i32,
    pub px: u32,
}

/// Runs `f`, returning with its result every text this thread cut to end in an ellipsis in it
/// ([`crate::text::TextRenderer::ellipsize`]), whether drawn or only measured.
pub fn shortened_text<T>(f: impl FnOnce() -> T) -> (T, Vec<Shortened>) {
    let outer = SHORTENED.replace(Some(Vec::new()));
    let r = f();
    let cut = SHORTENED.replace(outer).unwrap_or_default();
    (r, cut)
}

/// The texts [`shortened_text`] has noted so far in the `f` it is running, handed over and
/// forgotten.
pub fn take_shortened() -> Vec<Shortened> {
    SHORTENED.with_borrow_mut(|c| c.as_mut().map(std::mem::take).unwrap_or_default())
}

pub(crate) fn shortened(text: &str, width: i32, px: u32) {
    SHORTENED.with_borrow_mut(|c| {
        if let Some(c) = c {
            c.push(Shortened {
                text: text.into(),
                width,
                px,
            });
        }
    });
}

/// Runs `f` on a thread of its own whose fonts are those of a machine with no Tahoma and only
/// DejaVu's regular widths, as Debian and Ubuntu install it: the UI text falls back to DejaVu
/// Sans Book, the widest face CI has. A check of what that wider text cuts compares the two.
pub fn without_tahoma<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| {
        s.spawn(|| {
            crate::text::hide_tahoma();
            f()
        })
        .join()
        .unwrap_or_else(|e| std::panic::resume_unwind(e))
    })
}

/// Text drawn with ink cut off by the rect it was laid in, as [`clipped_ink`] lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cut {
    pub text: String,
    /// The rect the line was laid in, logical px.
    pub rect: crate::raster::Rect,
    /// The font size, logical px.
    pub px: u32,
    /// Ink device pixels cut above or below `rect`.
    pub rows: u32,
    /// Ink device pixels cut beside `rect`, within its rows.
    pub cols: u32,
}

/// Runs `f`, returning with its result every line of text this thread drew in it whose ink its
/// rect cut. A line drawn in a rect too short for it shows up here; a line clipped only by a view
/// scrolled past it, or shortened with an ellipsis, does not.
pub fn clipped_ink<T>(f: impl FnOnce() -> T) -> (T, Vec<Cut>) {
    let outer = CUTS.replace(Some(Vec::new()));
    let r = f();
    let cuts = CUTS.replace(outer).unwrap_or_default();
    (r, cuts)
}

/// The lines [`clipped_ink`] has noted so far in the `f` it is running, handed over and forgotten,
/// so a caller can tell which part of `f` cut them.
pub fn take_clipped() -> Vec<Cut> {
    CUTS.with_borrow_mut(|c| c.as_mut().map(std::mem::take).unwrap_or_default())
}

pub(crate) fn watching_clips() -> bool {
    CUTS.with_borrow(Option::is_some)
}

pub(crate) fn clipped(cut: Cut) {
    CUTS.with_borrow_mut(|c| {
        if let Some(c) = c {
            c.push(cut);
        }
    });
}

/// Hands the lines [`unfit_without_tahoma`]'s `draw` cut or ellipsized since the last call to
/// the surface `name`. Does nothing outside it.
pub fn surface_drawn(name: &str) {
    let cuts = take_clipped();
    let short = take_shortened();
    UNFIT.with_borrow_mut(|found| {
        let Some(found) = found else { return };
        for c in cuts {
            let r = c.rect;
            let at = format!(
                "{} px in {}x{} at ({}, {}): {} px cut above or below, {} beside",
                c.px, r.width, r.height, r.x, r.y, c.rows, c.cols
            );
            let key = format!("{name} cut {:?}", c.text);
            found.entry(key).or_insert((0, at)).0 += 1;
        }
        for s in short {
            let at = format!("{} px into {} px", s.px, s.width);
            let key = format!("{name} ellipsized {:?}", s.text);
            found.entry(key).or_insert((0, at)).0 += 1;
        }
    });
}

fn unfit(draw: &(impl Fn() + Sync)) -> Unfit {
    fonts();
    let outer = UNFIT.replace(Some(Unfit::new()));
    clipped_ink(|| shortened_text(draw));
    UNFIT.replace(outer).unwrap_or_default()
}

/// Calls `draw` with the test fonts' Tahoma and, on a second thread at once, [`without_tahoma`],
/// and lists each line DejaVu Sans cuts by its rect or to an ellipsis more often than Tahoma:
/// `(key, where)`, the key naming the surface (as `draw` hands them to [`surface_drawn`]), how,
/// the text and the extra count. For [`check_list`].
pub fn unfit_without_tahoma(draw: impl Fn() + Sync) -> Vec<(String, String)> {
    let (tahoma, dejavu) = std::thread::scope(|s| {
        let tahoma = s.spawn(|| unfit(&draw));
        let dejavu = without_tahoma(|| unfit(&draw));
        let tahoma = tahoma
            .join()
            .unwrap_or_else(|e| std::panic::resume_unwind(e));
        (tahoma, dejavu)
    });
    dejavu
        .into_iter()
        .filter_map(|(key, (n, at))| {
            let had = tahoma.get(&key).map_or(0, |t| t.0);
            (n > had).then(|| (format!("{key} x{}", n - had), at))
        })
        .collect()
}

/// Fails listing each of `found`'s keys missing from the list at `path`, and each listed key no
/// longer found, or rewrites the list when `REPRISE_BLESS` is set. A listed line may end in
/// `  # why`, which a rewrite keeps.
#[track_caller]
pub fn check_list(path: &Path, found: &[(String, String)], what: &str) {
    use std::fmt::Write as _;
    let listed = std::fs::read_to_string(path).unwrap_or_default();
    let key_of = |l: &str| {
        l.split_once("  #")
            .map_or(l, |(k, _)| k)
            .trim_end()
            .to_string()
    };
    let listed: Vec<(String, &str)> = listed
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| (key_of(l), l))
        .collect();
    if std::env::var_os("REPRISE_BLESS").is_some_and(|v| v != "0") {
        let mut text = String::new();
        for (key, _) in found {
            let kept = listed.iter().find(|(k, _)| k == key).map(|(_, l)| *l);
            writeln!(text, "{}", kept.unwrap_or(key)).unwrap();
        }
        let had = std::fs::read_to_string(path).ok();
        if had.as_deref() != Some(text.as_str()) && (had.is_some() || !text.is_empty()) {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        return;
    }
    let mut msg = String::new();
    for (key, at) in found {
        if !listed.iter().any(|(k, _)| k == key) {
            writeln!(msg, "  not listed: {key}, {at}").unwrap();
        }
    }
    for (key, _) in &listed {
        if !found.iter().any(|(k, _)| k == key) {
            writeln!(msg, "  listed, no longer found: {key}").unwrap();
        }
    }
    assert!(
        msg.is_empty(),
        "{what} differ from {}:\n{msg}REPRISE_BLESS=1 under `scripts/pin-env` rewrites it",
        path.display()
    );
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

/// Whether `px` is a pixel of antialiased text in `ink`: every channel within a quarter of the
/// range of the ink's, so a stem's core counts and its blended edge does not.
pub fn inks(px: u32, ink: crate::theme::Color) -> bool {
    let want = ink.argb();
    [16, 8, 0]
        .into_iter()
        .all(|sh| ((px >> sh) & 0xFF).abs_diff((want >> sh) & 0xFF) <= 0x40)
}

/// Reports a test that cannot run on this machine: panics under `REPRISE_TEST_STRICT`, else
/// prints `reason`. `reprisebus::testbus::skipped` is the same for crates that link reprisebus.
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

/// A pin's value for a drawn canvas: the FNV-1a hash of its scale, device size and every device
/// pixel.
pub fn fnv(c: &crate::raster::Canvas) -> u64 {
    let (w, h) = c.device_size();
    fnv_words(
        [c.scale() as u32, w, h]
            .into_iter()
            .chain(c.pixels.iter().copied()),
    )
}

/// [`fnv`] of the device pixels under logical rect `r` of `c`, 0 for those outside it.
pub fn fnv_in(c: &crate::raster::Canvas, r: crate::raster::Rect) -> u64 {
    let s = c.scale();
    let r = r.scaled(s);
    let (dw, dh) = c.device_size();
    let px = move |x: i32, y: i32| {
        if (0..dw as i32).contains(&x) && (0..dh as i32).contains(&y) {
            c.pixels[y as usize * dw as usize + x as usize]
        } else {
            0
        }
    };
    let words = (r.y..r.bottom()).flat_map(move |y| (r.x..r.right()).map(move |x| px(x, y)));
    fnv_words(
        [s as u32, r.width as u32, r.height as u32]
            .into_iter()
            .chain(words),
    )
}

fn fnv_words(words: impl Iterator<Item = u32>) -> u64 {
    words
        .flat_map(u32::to_le_bytes)
        .fold(0xcbf2_9ce4_8422_2325, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        })
}

/// The calling crate's directory as a `PathBuf`, for its pin files and test fixtures: the one
/// cargo names to the running test or example, else the one the crate was compiled in.
#[macro_export]
macro_rules! crate_dir {
    () => {
        $crate::pins::crate_dir(env!("CARGO_MANIFEST_DIR"))
    };
}

/// [`crate_dir!`](crate::crate_dir) with the caller's compile-time directory as `compiled`.
#[doc(hidden)]
pub fn crate_dir(compiled: &str) -> PathBuf {
    // Read at run time: cargo reuses a unit whose compiled-in CARGO_MANIFEST_DIR names another
    // checkout when target/ was copied from it, and files would be read from and written there.
    std::env::var_os("CARGO_MANIFEST_DIR").map_or_else(|| compiled.into(), PathBuf::from)
}

/// [`check_file`] on `tests/pins/<name>.txt`.
#[cfg(test)]
#[track_caller]
pub(crate) fn check(name: &str, got: &[(String, u64)]) {
    let path = crate_dir(env!("CARGO_MANIFEST_DIR"))
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

    #[test]
    fn scale_2_canvases_differing_in_one_odd_device_pixel_hash_apart() {
        let a = crate::raster::Canvas::with_scale(4, 3, 2);
        let mut b = a.clone();
        let dw = a.device_size().0 as usize;
        b.pixels[3 * dw + 5] = 0xff00_0000;
        assert_ne!(fnv(&a), fnv(&b), "device pixel (5, 3) is in the hash");
    }
}
