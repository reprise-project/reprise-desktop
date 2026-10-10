//! Account pictures: loading one for the list and the sheet, cutting a
//! picture file to a centred square, and saving the chosen one as the PNG
//! AccountsService copies in.

use std::path::{Path, PathBuf};

use repriseapp::decode::Decoders;
use reprisekit::art::userpic::{self, Picture};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::theme::Color;

/// Files bigger than this are not taken as pictures.
const MAX_BYTES: u64 = 16 << 20;
/// The longest side a file is decoded at before it is cut.
const DECODE_MAX: u32 = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Stock(Picture),
    File(PathBuf),
}

/// The centred square of `c`, its shorter side long, laid over white so
/// a transparent picture saves opaque.
pub fn square(c: &Canvas) -> Canvas {
    let side = c.width.min(c.height);
    let r = Rect::new(
        (c.width - side) as i32 / 2,
        (c.height - side) as i32 / 2,
        side as i32,
        side as i32,
    );
    let cut = c.copy_rect(r);
    let mut out = Canvas::new(side, side);
    out.fill_rect(out.bounds(), Color::rgb(0xff, 0xff, 0xff));
    out.blit_over(0, 0, &cut);
    out
}

pub fn load_file(path: &Path, px: u32) -> Option<Canvas> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES || meta.len() == 0 {
        return None;
    }
    let d = Decoders::default_set().decode_file(path, (DECODE_MAX, DECODE_MAX))?;
    Some(square(&d.canvas).resample_box(px, px))
}

pub fn load(src: &Source, px: u32) -> Option<Canvas> {
    match src {
        Source::Stock(p) => Some(userpic::draw(*p, px)),
        Source::File(path) => load_file(path, px),
    }
}

/// `src` saved at [`userpic::SAVED_PX`] as a PNG in the runtime folder,
/// for AccountsService to copy. Each save gets a file of its own.
pub fn save(src: &Source) -> Result<PathBuf, String> {
    let c = load(src, userpic::SAVED_PX).ok_or("The file is not a picture Reprise can read.")?;
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("reprise-accounts");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let path = dir.join(format!("picture-{}-{stamp}.png", std::process::id()));
    std::fs::write(&path, reprisekit::raster::to_png(&c)).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The picture a user's list row and sheet show: your `~/.face` first, as
/// the Start panel reads it, then AccountsService's file.
pub fn of_user(icon: &str, you: bool, px: u32) -> Option<Canvas> {
    let face = you
        .then(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".face")))
        .flatten();
    face.iter()
        .map(PathBuf::as_path)
        .chain((!icon.is_empty()).then(|| Path::new(icon)))
        .find_map(|p| load_file(p, px))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_picture_is_cut_to_its_centre() {
        let mut c = Canvas::new(30, 10);
        c.fill_rect(c.bounds(), Color::rgb(255, 0, 0));
        c.fill_rect(Rect::new(10, 0, 10, 10), Color::rgb(0, 0, 255));
        let s = square(&c);
        assert_eq!((s.width, s.height), (10, 10));
        for (x, y) in [(0, 0), (9, 9), (5, 5)] {
            assert_eq!(s.get(x, y), Color::rgb(0, 0, 255).argb(), "{x},{y}");
        }
    }

    #[test]
    fn a_transparent_picture_squares_onto_white() {
        let c = Canvas::new(4, 6);
        let s = square(&c);
        assert_eq!((s.width, s.height), (4, 4));
        assert_eq!(s.get(1, 1), Color::rgb(255, 255, 255).argb());
    }

    #[test]
    fn a_saved_stock_picture_reads_back_square() {
        let dir =
            std::env::temp_dir().join(format!("reprise-accounts-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: the test sets the variable before anything reads it, and
        // no other test in this crate reads XDG_RUNTIME_DIR.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", &dir) };
        let path = save(&Source::Stock(Picture::Planet)).unwrap();
        assert!(path.starts_with(&dir));
        let back = load_file(&path, 48).unwrap();
        assert_eq!((back.width, back.height), (48, 48));
        assert_eq!(
            back.get(2, 2),
            userpic::draw(Picture::Planet, 48).get(2, 2),
            "the dark sky survives the round trip"
        );
        let _ = std::fs::remove_dir_all(&dir);
        assert!(load_file(Path::new("/nonexistent/picture.png"), 48).is_none());
    }
}
