//! A folder of pictures taking turns: which files count and which one
//! comes next. The wallpaper slideshow (`[wallpaper] slideshow`) and the
//! Photos screen saver both pick through it, each on its own thread.

use std::path::{Path, PathBuf};

/// The picture files the compositor decodes.
const EXTS: [&str; 5] = ["jpg", "jpeg", "png", "bmp", "dib"];

/// The pictures straight inside `folder`, by name; none when it cannot
/// be read.
pub fn pictures(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
                && p.is_file()
        })
        .collect();
    out.sort();
    out
}

/// The picture after `current` in `list`: the next by name (back to the
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

    #[test]
    fn only_picture_files_count() {
        let dir = std::env::temp_dir().join(format!("reprise-slides-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub.jpg")).unwrap();
        for f in ["b.JPG", "a.png", "notes.txt", "c.gif"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let found = pictures(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(found, vec![dir.join("a.png"), dir.join("b.JPG")]);
    }
}
