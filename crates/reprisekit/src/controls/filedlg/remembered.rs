use crate::controls::shellview::{Field, Mode};

/// The Views pick and the Details sort one dialog hands the next: hosts read it with
/// [`FileDlg::remembered`], store it as [`Remembered::text`] and open with
/// [`FileDlg::restore`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Remembered {
    pub mode: Mode,
    pub sort: Field,
    pub ascending: bool,
}

impl Default for Remembered {
    fn default() -> Remembered {
        Remembered {
            mode: Mode::List,
            sort: Field::Name,
            ascending: true,
        }
    }
}

const MODE_KEYS: [(Mode, &str); 5] = [
    (Mode::List, "list"),
    (Mode::Details, "details"),
    (Mode::Thumbnails, "thumbnails"),
    (Mode::Tiles, "tiles"),
    (Mode::Icons, "icons"),
];

const FIELD_KEYS: [(Field, &str); 6] = [
    (Field::Name, "name"),
    (Field::Folder, "folder"),
    (Field::Size, "size"),
    (Field::Kind, "type"),
    (Field::Modified, "modified"),
    (Field::Deleted, "deleted"),
];

impl Remembered {
    /// `view`, `sort` and `ascending` as TOML lines.
    pub fn text(&self) -> String {
        fn key<T: PartialEq>(table: &[(T, &'static str)], v: T) -> &'static str {
            table.iter().find(|(k, _)| *k == v).map_or("", |(_, s)| *s)
        }
        format!(
            "view = \"{}\"\nsort = \"{}\"\nascending = {}\n",
            key(&MODE_KEYS, self.mode),
            key(&FIELD_KEYS, self.sort),
            self.ascending
        )
    }

    /// Reads [`Remembered::text`]'s lines; a missing or unknown value keeps its default.
    pub fn parse(text: &str) -> Remembered {
        let mut r = Remembered::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let word = value.trim_matches('"');
            match key.trim() {
                "view" => {
                    if let Some((m, _)) = MODE_KEYS.iter().find(|(_, s)| *s == word) {
                        r.mode = *m;
                    }
                }
                "sort" => {
                    if let Some((f, _)) = FIELD_KEYS.iter().find(|(_, s)| *s == word) {
                        r.sort = *f;
                    }
                }
                "ascending" => {
                    if let Ok(b) = value.parse() {
                        r.ascending = b;
                    }
                }
                _ => {}
            }
        }
        r
    }
}
