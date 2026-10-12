//! The pictures the Background page offers, scanned on a thread at startup, and the file a
//! configured `[wallpaper] image` names.

use std::path::{Path, PathBuf};

use crate::backgroundpage::label;

pub const EXTS: [&str; 6] = ["bmp", "gif", "jpg", "jpeg", "dib", "png"];
pub const FILTER: &str = "Background Files (*.bmp;*.gif;*.jpg;*.jpeg;*.dib;*.png)";
const DEPTH: usize = 3;

pub fn is_picture(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

pub fn refusal(path: &Path) -> Option<String> {
    (!is_picture(path)).then(|| {
        let name = path.file_name().unwrap_or(path.as_os_str());
        format!("'{}' is not a picture.", name.to_string_lossy())
    })
}

pub fn dirs() -> Vec<PathBuf> {
    dirs_for(
        reprisefs::places::user_dir("PICTURES", "Pictures"),
        std::env::var_os("HOME").map(PathBuf::from),
    )
}

/// [`dirs`] for a Pictures folder `pictures` and a home `home`. Pictures
/// set to home itself (`XDG_PICTURES_DIR="$HOME/"`, how the user turns
/// the folder off) is left out.
fn dirs_for(pictures: Option<PathBuf>, home: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dirs = vec![
        PathBuf::from("/usr/share/backgrounds"),
        PathBuf::from("/usr/share/wallpapers"),
    ];
    dirs.extend(pictures.filter(|p| home.as_ref() != Some(p)));
    dirs
}

/// The pictures under `dirs` and up to `DEPTH` folders below each,
/// sorted by their row's name without regard to case, then by path. A
/// folder that cannot be read is skipped, and so is every hidden file
/// and folder (a thumbnail cache). A file reached twice, through a
/// symlink, is listed once, by the path that sorts first.
pub fn scan(dirs: &[PathBuf]) -> Vec<PathBuf> {
    scan_to(dirs, DEPTH)
}

/// The pictures straight inside `dir`, in [`scan`]'s order.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    scan_to(&[dir.to_path_buf()], 0)
}

fn scan_to(dirs: &[PathBuf], max: usize) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize, max: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            if e.file_name().as_encoded_bytes().starts_with(b".") {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                if depth < max {
                    walk(&p, out, depth + 1, max);
                }
            } else if is_picture(&p) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    for d in dirs {
        walk(d, &mut out, 0, max);
    }
    out.sort_by_cached_key(|p| (label(p).to_lowercase(), p.clone()));
    let mut seen = std::collections::HashSet::new();
    out.retain(|p| seen.insert(std::fs::canonicalize(p).unwrap_or_else(|_| p.clone())));
    out
}

pub fn merge(mut found: Vec<PathBuf>, had: &[PathBuf]) -> Vec<PathBuf> {
    for p in had {
        place(&mut found, p);
    }
    found
}

pub fn add_missing(mut merged: Vec<PathBuf>, rows: &[PathBuf]) -> Vec<PathBuf> {
    for p in rows {
        if !merged.contains(p) {
            merged.push(p.clone());
        }
    }
    merged
}

pub fn place(list: &mut Vec<PathBuf>, p: &Path) {
    if list.iter().any(|q| q == p) {
        return;
    }
    let same = std::fs::canonicalize(p).ok().and_then(|c| {
        list.iter().position(|q| {
            q.file_name() == c.file_name() && std::fs::canonicalize(q).is_ok_and(|d| d == c)
        })
    });
    match same {
        Some(i) => list[i] = p.to_path_buf(),
        None => list.push(p.to_path_buf()),
    }
}

pub fn start_dir() -> PathBuf {
    reprisefs::places::user_dir("PICTURES", "Pictures")
        .filter(|p| p.is_dir())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The file `[wallpaper] image = s` names, as the desktop reads it: a
/// leading `~/` opens at `$HOME` (none when it is unset), empty is none,
/// anything else is taken as written. The config keeps `s` as written.
pub fn expand(s: &str) -> Option<PathBuf> {
    if s.is_empty() {
        return None;
    }
    match s.strip_prefix("~/") {
        Some(rest) => std::env::var_os("HOME").map(|h| Path::new(&h).join(rest)),
        None => Some(PathBuf::from(s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backgroundpage;

    struct Tmp(PathBuf);

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Tmp {
        let p = std::env::temp_dir().join(format!(
            "reprise-settings-wallpapers-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("temp dir");
        Tmp(p)
    }

    fn touch(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).expect("mkdir");
        std::fs::write(&p, b"").expect("write");
    }

    #[test]
    fn the_scan_finds_pictures_three_folders_down_sorted_by_name() {
        let dir = tmp("scan");
        let root = &dir.0;
        for f in [
            "b.PNG",
            "notes.txt",
            "sub/a.jpg",
            "1/2/3/deep.gif",
            "1/2/3/4/too-deep.png",
            "x/C.dib",
        ] {
            touch(root, f);
        }
        let found = scan(std::slice::from_ref(root));
        let names: Vec<String> = found
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .collect();
        assert_eq!(names, ["sub/a.jpg", "b.PNG", "x/C.dib", "1/2/3/deep.gif"]);
        assert!(
            scan(&[root.join("missing")]).is_empty(),
            "a folder that is not there is skipped"
        );
    }

    #[test]
    fn a_folder_lists_only_the_pictures_straight_inside() {
        let dir = tmp("list");
        let root = &dir.0;
        for f in ["b.PNG", "a.jpg", "notes.txt", ".hidden.png", "sub/c.jpg"] {
            touch(root, f);
        }
        let names: Vec<String> = list(root)
            .iter()
            .map(|p| p.strip_prefix(root).unwrap().display().to_string())
            .collect();
        assert_eq!(names, ["a.jpg", "b.PNG"]);
    }

    #[test]
    fn the_scan_skips_hidden_files_and_folders() {
        let dir = tmp("hidden");
        let root = &dir.0;
        for f in [
            "a.png",
            ".b.png",
            ".cache/thumbnails/normal/c.png",
            "sub/.thumbnails/d.png",
        ] {
            touch(root, f);
        }
        let found = scan(std::slice::from_ref(root));
        assert_eq!(found, [root.join("a.png")]);
    }

    #[test]
    fn a_pictures_folder_that_is_home_is_not_scanned() {
        let home = PathBuf::from("/home/u");
        assert_eq!(
            dirs_for(Some(PathBuf::from("/home/u/")), Some(home.clone())),
            [
                PathBuf::from("/usr/share/backgrounds"),
                PathBuf::from("/usr/share/wallpapers")
            ],
            "XDG_PICTURES_DIR=\"$HOME/\" turns Pictures off"
        );
        assert_eq!(
            dirs_for(Some(PathBuf::from("/home/u/Pictures")), Some(home)),
            [
                PathBuf::from("/usr/share/backgrounds"),
                PathBuf::from("/usr/share/wallpapers"),
                PathBuf::from("/home/u/Pictures"),
            ]
        );
    }

    #[test]
    fn a_picture_reached_through_a_symlink_is_one_row() {
        let dir = tmp("symlink");
        let root = &dir.0;
        touch(root, "real/a.png");
        touch(root, "real/b.png");
        std::os::unix::fs::symlink(root.join("real"), root.join("link")).expect("symlink");
        let found = scan(std::slice::from_ref(root));
        assert_eq!(found, [root.join("link/a.png"), root.join("link/b.png")]);
        let had = [root.join("real/b.png"), root.join("real/../real/a.png")];
        assert_eq!(
            merge(found, &had),
            [root.join("real/../real/a.png"), root.join("real/b.png")],
            "a row the config names another way takes the config's spelling"
        );
    }

    #[test]
    fn browse_refuses_a_file_that_is_not_a_picture() {
        assert_eq!(
            refusal(Path::new("/home/u/notes.txt")),
            Some("'notes.txt' is not a picture.".to_string())
        );
        assert_eq!(refusal(Path::new("/home/u/a.PNG")), None);
    }

    #[test]
    fn a_pick_made_while_the_scan_ran_stays_listed() {
        let merged = vec![PathBuf::from("~/w/a.png"), PathBuf::from("/s/b.jpg")];
        let rows = [PathBuf::from("~/w/a.png"), PathBuf::from("/d/picked.png")];
        assert_eq!(
            add_missing(merged, &rows),
            [
                PathBuf::from("~/w/a.png"),
                PathBuf::from("/s/b.jpg"),
                PathBuf::from("/d/picked.png"),
            ]
        );
    }

    #[test]
    fn a_picture_the_scan_missed_stays_listed() {
        let found = vec![PathBuf::from("/s/a.jpg"), PathBuf::from("/s/b.jpg")];
        let had = [
            PathBuf::from("/home/u/Downloads/mine.png"),
            PathBuf::from("/s/a.jpg"),
        ];
        assert_eq!(
            merge(found, &had),
            [
                PathBuf::from("/s/a.jpg"),
                PathBuf::from("/s/b.jpg"),
                PathBuf::from("/home/u/Downloads/mine.png"),
            ]
        );
    }

    #[test]
    fn pictures_are_known_by_their_extension_in_any_case() {
        for ok in ["a.bmp", "a.GIF", "a.jpg", "a.JPEG", "a.dib", "a.png"] {
            assert!(is_picture(Path::new(ok)), "{ok}");
        }
        for no in ["a.webp", "a.txt", "png"] {
            assert!(!is_picture(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn a_leading_tilde_opens_at_home() {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        assert_eq!(expand(""), None, "empty is no wallpaper");
        assert_eq!(expand("/w/a.png"), Some(PathBuf::from("/w/a.png")));
        assert_eq!(expand("w/~/a.png"), Some(PathBuf::from("w/~/a.png")));
        assert_eq!(
            expand("~/Pictures/a.png"),
            home.as_ref().map(|h| h.join("Pictures/a.png"))
        );
        let Some(home) = home else { return };
        let mut f = backgroundpage::page(crate::pages::Area::MIN);
        let paths = [PathBuf::from("/w/b.png"), home.join("Pictures/a.png")];
        backgroundpage::fill(&mut f, &paths, "~/Pictures/a.png", None, &[]);
        let row = backgroundpage::selected(&f).map(backgroundpage::row);
        assert_eq!(row, Some(backgroundpage::Row::Picture(1)));
    }
}
