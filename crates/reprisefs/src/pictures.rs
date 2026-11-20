//! Wallpaper pictures: which files count and the order a slideshow folder
//! takes them in. The compositor's slideshow and the Settings monitor both
//! list a folder through [`in_folder`], so they agree on its first picture.
//! The wallpaper slideshow and the Photos saver, full screen and in
//! Settings' preview, pick the next one with [`next`].

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// The extensions of the formats the compositor decodes a wallpaper from.
pub const EXTS: [&str; 7] = ["bmp", "dib", "jpg", "jpeg", "png", "jxl", "avif"];

/// A format the compositor decodes a wallpaper from, told by content: BMP, JPEG and PNG in the
/// image crate, JPEG XL and AVIF in reprise-decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bmp,
    Jpeg,
    Png,
    Jxl,
    Avif,
}

impl Format {
    /// The format `head`, a file's first bytes, starts; None for one the compositor cannot show.
    pub fn of(head: &[u8]) -> Option<Format> {
        use crate::imagesize::{is_avif, is_jxl};
        if is_jxl(head) {
            Some(Format::Jxl)
        } else if is_avif(head) {
            Some(Format::Avif)
        } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some(Format::Png)
        } else if head.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Format::Jpeg)
        } else if head.starts_with(b"BM") {
            Some(Format::Bmp)
        } else {
            None
        }
    }
}

/// Whether `p` has one of [`EXTS`], in any case.
pub fn is_picture(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The pictures straight inside `folder`, hidden files left out, in
/// [`natural_cmp`] order of their names; none when it cannot be read.
pub fn in_folder(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| !e.file_name().as_encoded_bytes().starts_with(b"."))
        .map(|e| e.path())
        .filter(|p| is_picture(p) && p.is_file())
        .collect();
    out.sort_by(|a, b| natural_cmp(&name(a), &name(b)).then_with(|| a.cmp(b)));
    out
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Names compared ignoring case, with each run of digits taken as a
/// number ("img9" before "img10"); names equal that way fall back to
/// byte order, so the order is total.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (la, lb) = (a.to_lowercase(), b.to_lowercase());
    let (mut x, mut y) = (la.as_str(), lb.as_str());
    while let (Some(c), Some(d)) = (x.chars().next(), y.chars().next()) {
        if c.is_ascii_digit() && d.is_ascii_digit() {
            let (nx, rx) = split_digits(x);
            let (ny, ry) = split_digits(y);
            let (tx, ty) = (nx.trim_start_matches('0'), ny.trim_start_matches('0'));
            let o = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
            if o != Ordering::Equal {
                return o;
            }
            (x, y) = (rx, ry);
        } else {
            if c != d {
                return c.cmp(&d);
            }
            (x, y) = (&x[c.len_utf8()..], &y[d.len_utf8()..]);
        }
    }
    x.len().cmp(&y.len()).then_with(|| a.cmp(b))
}

fn split_digits(s: &str) -> (&str, &str) {
    let n = s.bytes().take_while(u8::is_ascii_digit).count();
    s.split_at(n)
}

/// The picture after `current` in `list`: the next in list order (back to the
/// first after the last, or the first when `current` is not there), or
/// with `shuffle` any other one, picked by `roll`. None for an empty list.
pub fn next(list: &[PathBuf], current: Option<&Path>, shuffle: bool, roll: u32) -> Option<PathBuf> {
    if list.is_empty() {
        return None;
    }
    let at = current.and_then(|c| list.iter().position(|p| p == c));
    let i = match (shuffle, at) {
        (true, Some(at)) if list.len() > 1 => {
            // Any index but `at`, evenly.
            let i = roll as usize % (list.len() - 1);
            if i >= at { i + 1 } else { i }
        }
        (true, _) => roll as usize % list.len(),
        (false, Some(at)) => (at + 1) % list.len(),
        (false, None) => 0,
    };
    Some(list[i].clone())
}

/// A roll for [`next`] that differs from call to call.
pub fn roll() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    // A few rounds of xorshift over the clock's nanoseconds.
    let mut x = (t.subsec_nanos() ^ t.as_secs() as u32) | 1;
    for _ in 0..4 {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wallpaper_format_is_told_by_content() {
        let avif = b"\0\0\0\x1cftypavif\0\0\0\0avifmif1miaf";
        for (head, want) in [
            (&b"\x89PNG\r\n\x1a\n\0"[..], Some(Format::Png)),
            (&[0xff, 0xd8, 0xff, 0xe0][..], Some(Format::Jpeg)),
            (b"BM\0\0", Some(Format::Bmp)),
            (&[0xff, 0x0a, 0, 0][..], Some(Format::Jxl)),
            (crate::imagesize::JXL_CONTAINER, Some(Format::Jxl)),
            (avif, Some(Format::Avif)),
            (b"GIF89a\0\0", None),
            (b"RIFF\0\0\0\0WEBPVP8 ", None),
            (b"II*\0", None),
            (b"", None),
        ] {
            assert_eq!(Format::of(head), want, "{head:?}");
        }
    }

    #[test]
    fn names_sort_ignoring_case_with_numbers_by_value() {
        let mut names = vec![
            "img10.jpg",
            "Img2.jpg",
            "beach.png",
            "img2.jpg",
            "Zoo.bmp",
            "img002.jpg",
            "apple.jpg",
        ];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "apple.jpg",
                "beach.png",
                "Img2.jpg",
                "img002.jpg",
                "img2.jpg",
                "img10.jpg",
                "Zoo.bmp"
            ]
        );
    }

    #[test]
    fn a_folder_lists_the_decodable_pictures_in_name_order() {
        let dir = std::env::temp_dir().join(format!("reprisefs-pictures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub.jpg")).unwrap();
        for f in [
            "b10.JPG",
            "B2.png",
            "a.gif",
            "notes.txt",
            ".hidden.png",
            "c.dib",
        ] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let found = in_folder(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            found,
            ["B2.png", "b10.JPG", "c.dib"].map(|f| dir.join(f)),
            "no .gif, folder or hidden file; B2 before b10"
        );
    }

    fn list() -> Vec<PathBuf> {
        ["a.jpg", "b.png", "c.bmp"].map(PathBuf::from).to_vec()
    }

    #[test]
    fn in_order_the_next_by_name_wraps_around() {
        let l = list();
        assert_eq!(next(&l, None, false, 0), Some("a.jpg".into()));
        assert_eq!(
            next(&l, Some(Path::new("a.jpg")), false, 0),
            Some("b.png".into())
        );
        assert_eq!(
            next(&l, Some(Path::new("c.bmp")), false, 0),
            Some("a.jpg".into())
        );
        assert_eq!(
            next(&l, Some(Path::new("gone.jpg")), false, 0),
            Some("a.jpg".into())
        );
        assert_eq!(next(&[], None, false, 0), None);
    }

    #[test]
    fn shuffled_never_repeats_the_current_picture() {
        let l = list();
        for roll in 0..20 {
            let n = next(&l, Some(Path::new("b.png")), true, roll).unwrap();
            assert_ne!(n, PathBuf::from("b.png"), "roll {roll}");
        }
        let one = vec![PathBuf::from("a.jpg")];
        assert_eq!(
            next(&one, Some(Path::new("a.jpg")), true, 5),
            Some("a.jpg".into())
        );
    }
}
