//! Wallpaper pictures: which files count and the order a slideshow folder
//! takes them in. The compositor's slideshow and the Settings monitor both
//! list a folder through [`in_folder`], so they agree on its first picture.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// The extensions of the formats the compositor decodes a wallpaper from.
pub const EXTS: [&str; 5] = ["bmp", "dib", "jpg", "jpeg", "png"];

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
