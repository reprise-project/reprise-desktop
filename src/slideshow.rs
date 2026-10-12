//! Which picture of a slideshow folder comes next. The wallpaper slideshow
//! (`[wallpaper] slideshow`) and the Photos screen saver both pick through
//! it, each on its own thread, from `reprisefs::pictures::in_folder`.

use std::path::{Path, PathBuf};

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
