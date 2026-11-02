//! The pictures of the open picture's folder, in name order, and which one shows.

use std::path::{Path, PathBuf};

/// Where a step goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
    First,
    Last,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    pub pictures: Vec<PathBuf>,
    pub at: usize,
}

impl Folder {
    /// `path` alone, until its folder's listing lands.
    pub fn of(path: PathBuf) -> Folder {
        Folder {
            pictures: vec![path],
            at: 0,
        }
    }

    pub fn current(&self) -> Option<&Path> {
        self.pictures.get(self.at).map(PathBuf::as_path)
    }

    /// The folder listed as `pictures`, still on the picture that shows; one the listing missed
    /// (renamed, or not a picture by name) keeps its place in name order.
    pub fn listed(&mut self, mut pictures: Vec<PathBuf>) {
        let Some(current) = self.current().map(Path::to_path_buf) else {
            self.pictures = pictures;
            self.at = 0;
            return;
        };
        let at = match pictures.iter().position(|p| *p == current) {
            Some(i) => i,
            None => {
                let i = pictures.partition_point(|p| by_name(p, &current).is_lt());
                pictures.insert(i, current);
                i
            }
        };
        self.pictures = pictures;
        self.at = at;
    }

    /// Move along the folder; wraps from the last to the first and back, as a slideshow does.
    /// True when another picture is current.
    pub fn step(&mut self, step: Step) -> bool {
        let n = self.pictures.len();
        if n == 0 {
            return false;
        }
        let to = match step {
            Step::Previous => (self.at + n - 1) % n,
            Step::Next => (self.at + 1) % n,
            Step::First => 0,
            Step::Last => n - 1,
        };
        std::mem::replace(&mut self.at, to) != to
    }

    /// The current picture is gone: the next one shows, or the one before when it was the last.
    pub fn remove_current(&mut self) {
        if self.at < self.pictures.len() {
            self.pictures.remove(self.at);
        }
        self.at = self.at.min(self.pictures.len().saturating_sub(1));
    }

    /// "3 of 12".
    pub fn position(&self) -> String {
        format!("{} of {}", self.at + 1, self.pictures.len())
    }
}

fn name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn by_name(a: &Path, b: &Path) -> std::cmp::Ordering {
    reprisefs::pictures::natural_cmp(&name(a), &name(b)).then_with(|| a.cmp(b))
}

/// The pictures straight inside `dir` that the viewer decodes, hidden files left out, in natural
/// name order ("img9" before "img10", case ignored); none when it cannot be read.
pub fn list(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| !e.file_name().as_encoded_bytes().starts_with(b"."))
        .map(|e| e.path())
        .filter(|p| repriseapp::decode::is_image(p) && p.is_file())
        .collect();
    out.sort_by(|a, b| by_name(a, b));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(names: &[&str]) -> Vec<PathBuf> {
        names
            .iter()
            .map(|n| PathBuf::from(format!("/home/u/Pictures/{n}")))
            .collect()
    }

    #[test]
    fn a_folder_lists_its_pictures_in_natural_name_order() {
        let dir = tempfile::tempdir().unwrap();
        for n in [
            "img10.png",
            "Img9.JPG",
            "beach.webp",
            "notes.txt",
            ".hidden.png",
            "img2.gif",
        ] {
            std::fs::write(dir.path().join(n), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub.png")).unwrap();
        let names: Vec<String> = list(dir.path()).iter().map(|p| name(p)).collect();
        assert_eq!(names, ["beach.webp", "img2.gif", "Img9.JPG", "img10.png"]);
    }

    #[test]
    fn the_listing_keeps_the_picture_shown_current() {
        let mut f = Folder::of(PathBuf::from("/home/u/Pictures/b.png"));
        f.listed(paths(&["a.png", "b.png", "c.png"]));
        assert_eq!((f.at, f.position()), (1, "2 of 3".to_string()));
        let mut f = Folder::of(PathBuf::from("/home/u/Pictures/b.svg"));
        f.listed(paths(&["a.png", "c.png"]));
        assert_eq!(f.pictures, paths(&["a.png", "b.svg", "c.png"]));
        assert_eq!(f.at, 1);
    }

    #[test]
    fn steps_wrap_at_the_ends_and_home_and_end_jump_there() {
        let mut f = Folder {
            pictures: paths(&["a", "b", "c"]),
            at: 0,
        };
        assert!(f.step(Step::Previous));
        assert_eq!(f.at, 2);
        assert!(f.step(Step::Next));
        assert_eq!(f.at, 0);
        assert!(!f.step(Step::First));
        assert!(f.step(Step::Last));
        assert_eq!(f.at, 2);
        let mut one = Folder::of(PathBuf::from("/home/u/a.png"));
        assert!(!one.step(Step::Next));
    }

    #[test]
    fn removing_the_last_picture_shows_the_one_before() {
        let mut f = Folder {
            pictures: paths(&["a", "b", "c"]),
            at: 1,
        };
        f.remove_current();
        assert_eq!((f.current(), f.at), (Some(paths(&["c"])[0].as_path()), 1));
        f.remove_current();
        assert_eq!(f.current(), Some(paths(&["a"])[0].as_path()));
        f.remove_current();
        assert_eq!(f.current(), None);
    }
}
