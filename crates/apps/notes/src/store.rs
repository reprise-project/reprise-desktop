//! Each note as two files in the notes folder: `<id>.txt`, its text as typed, and `<id>.note`,
//! its colour and size as `key = value` lines. Ids are the creation time in milliseconds, so the
//! folder lists the notes in the order they were made.

use reprisekit::controls::stickynote::Colour;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Saved {
    pub id: String,
    pub text: String,
    pub colour: Colour,
    /// The client size it was last left at, logical px.
    pub size: Option<(i32, i32)>,
}

/// `$XDG_DATA_HOME/reprise/notes`, `~/.local/share/reprise/notes` when it is unset or relative.
pub fn dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
    Some(base.join("reprise").join("notes"))
}

/// Every note in `dir`, oldest first. A `.note` without its text is left out; a text without its
/// `.note` is a yellow note at the default size.
pub fn load(dir: &Path) -> Vec<Saved> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<Saved> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let id = path.file_stem()?.to_str()?.to_string();
            if path.extension()? != "txt" {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            let meta = std::fs::read_to_string(path.with_extension("note")).unwrap_or_default();
            let (colour, size) = parse_meta(&meta);
            Some(Saved {
                id,
                text,
                colour,
                size,
            })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

/// Writes `note`'s two files, each replaced whole.
pub fn write(dir: &Path, note: &Saved) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("{}.txt", note.id));
    reprisefs::ops::write_atomic(&path, note.text.as_bytes())?;
    reprisefs::ops::write_atomic(
        &path.with_extension("note"),
        meta_text(note.colour, note.size).as_bytes(),
    )
}

pub fn remove(dir: &Path, id: &str) {
    for ext in ["txt", "note"] {
        let path = dir.join(format!("{id}.{ext}"));
        if let Err(e) = std::fs::remove_file(&path)
            && e.kind() != io::ErrorKind::NotFound
        {
            log::warn!("could not delete {}: {e}", path.display());
        }
    }
}

/// A fresh id at `now_ms` (milliseconds since the epoch), one later for each `taken` one.
pub fn new_id(now_ms: u128, taken: impl Fn(&str) -> bool) -> String {
    (now_ms..)
        .map(|n| format!("{n:016}"))
        .find(|id| !taken(id))
        .unwrap_or_default()
}

fn meta_text(colour: Colour, size: Option<(i32, i32)>) -> String {
    let mut out = format!("colour = {}\n", colour.slug());
    if let Some((w, h)) = size {
        out.push_str(&format!("width = {w}\nheight = {h}\n"));
    }
    out
}

fn parse_meta(text: &str) -> (Colour, Option<(i32, i32)>) {
    let mut colour = Colour::default();
    let (mut w, mut h) = (None, None);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "colour" => colour = Colour::from_slug(value).unwrap_or_default(),
            "width" => w = value.parse::<i32>().ok().filter(|v| *v > 0),
            "height" => h = value.parse::<i32>().ok().filter(|v| *v > 0),
            _ => {}
        }
    }
    (colour, w.zip(h))
}

/// When a changed note is written: [`Autosave::DELAY`] after the last of a run of changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Autosave {
    since: Option<Instant>,
}

impl Autosave {
    pub const DELAY: Duration = Duration::from_millis(600);

    pub fn changed(&mut self, now: Instant) {
        self.since = Some(now);
    }

    pub fn pending(&self) -> bool {
        self.since.is_some()
    }

    /// Whether the write is due at `now`; true once per run of changes.
    pub fn due(&mut self, now: Instant) -> bool {
        match self.since {
            Some(t) if now.duration_since(t) >= Autosave::DELAY => {
                self.since = None;
                true
            }
            _ => false,
        }
    }

    /// Whether a write was waiting; it is due now.
    pub fn flush(&mut self) -> bool {
        self.since.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_reads_back_as_it_was_written() {
        let dir = tempfile::tempdir().unwrap();
        let notes = [
            Saved {
                id: new_id(1_700_000_000_000, |_| false),
                text: "Milk\n\tEggs\nüber".into(),
                colour: Colour::Pink,
                size: Some((240, 180)),
            },
            Saved {
                id: new_id(1_700_000_000_001, |_| false),
                text: String::new(),
                colour: Colour::Yellow,
                size: None,
            },
        ];
        for n in notes.iter().rev() {
            write(dir.path(), n).unwrap();
        }
        assert_eq!(load(dir.path()), notes, "oldest first");
        let meta = std::fs::read_to_string(dir.path().join("0001700000000000.note")).unwrap();
        assert_eq!(meta, "colour = pink\nwidth = 240\nheight = 180\n");
        remove(dir.path(), &notes[0].id);
        assert_eq!(load(dir.path()), notes[1..]);
    }

    #[test]
    fn a_text_without_its_note_file_is_a_yellow_note_and_a_stray_note_file_is_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("1.txt"), "hello").unwrap();
        std::fs::write(dir.path().join("2.note"), "colour = blue\n").unwrap();
        std::fs::write(dir.path().join("3.txt"), "x").unwrap();
        std::fs::write(dir.path().join("3.note"), "colour = teal\nwidth = -4\n").unwrap();
        let got = load(dir.path());
        assert_eq!(got.len(), 2);
        assert_eq!((got[0].colour, got[0].size), (Colour::Yellow, None));
        assert_eq!(
            (got[1].colour, got[1].size),
            (Colour::Yellow, None),
            "an unknown colour and a bad size fall back"
        );
    }

    #[test]
    fn a_new_id_steps_past_the_taken_ones() {
        let taken = ["0000000000000005", "0000000000000006"];
        assert_eq!(new_id(5, |id| taken.contains(&id)), "0000000000000007");
    }

    #[test]
    fn the_write_waits_for_a_pause_in_the_changes() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut a = Autosave::default();
        assert!(!a.due(ms(10_000)), "nothing changed");
        a.changed(t0);
        assert!(!a.due(ms(500)));
        a.changed(ms(500));
        assert!(!a.due(ms(1000)), "the second change restarted the wait");
        assert!(a.due(ms(1100)));
        assert!(!a.due(ms(5000)), "written once");
        a.changed(ms(6000));
        assert!(a.flush(), "a quit writes it at once");
        assert!(!a.pending());
    }
}
