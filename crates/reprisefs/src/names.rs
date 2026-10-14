//! Naming rules for files the shell creates, copies, shortcuts or renames:
//! numbered duplicates ("Copy of x", "New Folder (2)"), the label that hides a
//! known extension, the zip archive name, and which typed names a rename takes.

use crate::apps::{self, MimeDb};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// `name`, then `name` with `mark(n)` before its extension for n = 2, 3...
/// The extension runs from the last dot unless that dot starts the name or a
/// space follows it: `a.txt` -> `a (2).txt`, `Mr. Smith` -> `Mr. Smith (2)`.
/// Works on bytes, so a non-UTF-8 name keeps them.
pub fn numbered(name: &OsStr, mark: impl Fn(u64) -> String) -> impl Iterator<Item = OsString> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    let (stem, ext) = match bytes.iter().rposition(|&b| b == b'.') {
        Some(dot) if dot > 0 && !bytes[dot..].contains(&b' ') => bytes.split_at(dot),
        _ => (bytes, &b""[..]),
    };
    let stem = OsStr::from_bytes(stem).to_owned();
    let ext = OsStr::from_bytes(ext).to_owned();
    std::iter::once(name.to_owned()).chain((2..).map(move |n| {
        let mut out = stem.clone();
        out.push(mark(n));
        out.push(&ext);
        out
    }))
}

pub fn paren(n: u64) -> String {
    format!(" ({n})")
}

/// The first of `name`, `name (2)`... that `taken` refuses.
pub fn first_free(name: &OsStr, taken: impl Fn(&OsStr) -> bool) -> OsString {
    numbered(name, paren)
        .find(|c| !taken(c))
        .unwrap_or_else(|| name.to_owned())
}

/// [`first_free`] past `existing`: `Copy of a.txt` -> `Copy of a (2).txt`.
pub fn unique_name(base: &str, existing: &[String]) -> String {
    first_free(OsStr::new(base), |c| {
        existing.iter().any(|e| OsStr::new(e) == c)
    })
    .to_string_lossy()
    .into_owned()
}

pub fn copy_name(name: &str, existing: &[String]) -> String {
    unique_name(&format!("Copy of {name}"), existing)
}

/// `existing` holds file names, so a label whose `<label>.desktop` is taken counts as taken.
pub fn shortcut_name(name: &str, existing: &[String]) -> String {
    let base = name.strip_suffix(".desktop").unwrap_or(name);
    let taken: Vec<String> = existing
        .iter()
        .map(|e| e.strip_suffix(".desktop").unwrap_or(e).to_string())
        .collect();
    unique_name(&format!("Shortcut to {base}"), &taken)
}

/// Which extensions desktop labels hide: `desktop_extensions` in config.toml.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Extensions {
    /// Every registered file type's ([`label_for`]).
    #[default]
    Hide,
    /// Only `.desktop`.
    HideLaunchers,
    Show,
}

impl Extensions {
    pub fn parse(s: &str) -> Option<Extensions> {
        Some(match s {
            "hide" => Extensions::Hide,
            "hide_launchers" => Extensions::HideLaunchers,
            "show" => Extensions::Show,
            _ => return None,
        })
    }
}

/// A file's desktop label under `shown`. [`Extensions::Hide`] hides the extension of a registered
/// file type ("New Text Document" for `New Text Document.txt`) and keeps an unknown one
/// (`notes.foo`); a type is known when the shared-mime-info `globs` (`apps::mime_globs`) match.
/// Only the last extension goes (`a.tar.gz` -> `a.tar`), and a name that is all extension
/// (`.txt`) keeps it.
pub fn label_for(file_name: &str, globs: &MimeDb, shown: Extensions) -> String {
    let path = Path::new(file_name);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let hides = match shown {
        Extensions::Hide => apps::mime_of_in(globs, path).is_some(),
        Extensions::HideLaunchers => ext == "desktop",
        Extensions::Show => false,
    };
    if ext.is_empty() || stem.is_empty() || !hides {
        return file_name.to_string();
    }
    stem.to_string()
}

pub fn new_folder_name(existing: &[String]) -> String {
    unique_name("New Folder", existing)
}

pub fn new_text_document_name(existing: &[String]) -> String {
    unique_name("New Text Document.txt", existing)
}

/// `<stem>.zip` beside `path`, numbered past an existing one. The stem keeps
/// its bytes; `existing` holds display names, compared in lossy form.
pub fn zip_path(path: &Path, existing: &[String]) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut first = path
        .file_stem()
        .unwrap_or(OsStr::new("Archive"))
        .to_os_string();
    first.push(".zip");
    dir.join(first_free(&first, |c| {
        existing.iter().any(|e| *e == c.to_string_lossy())
    }))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Cancel,
    Rename(String),
    Refuse(String),
}

/// `/` and NUL are the only characters a name cannot hold; `.` and `..` are
/// refused; a name already in the folder is refused rather than overwritten.
pub fn verdict(old: &str, typed: &str, existing: &[String]) -> Verdict {
    let typed = typed.trim();
    if typed.is_empty() || typed == old {
        return Verdict::Cancel;
    }
    if typed.contains('/') || typed.contains('\0') {
        return Verdict::Refuse(
            "A file name cannot contain any of the following characters:\n\
             /\n\
             A file name cannot contain the null character."
                .to_string(),
        );
    }
    // `rename` would refuse `.` and `..` with a bare errno, so they get the box's wording.
    if typed == "." || typed == ".." {
        return Verdict::Refuse(format!(
            "Cannot rename {old}: The file name you specified is not valid or too long."
        ));
    }
    if existing.iter().any(|e| e == typed) {
        return Verdict::Refuse(taken_text(old));
    }
    Verdict::Rename(typed.to_string())
}

pub fn taken_text(old: &str) -> String {
    format!(
        "Cannot rename {old}: A file with the name you specified already \
         exists. Specify a different file name."
    )
}

/// A name free when [`verdict`] looked can be taken by the time the write
/// lands; `move_to_free` refuses it there, with the same wording as a listed name.
pub fn failed_text(old: &str, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::AlreadyExists {
        taken_text(old)
    } else {
        format!("Cannot rename {old}: {e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_and_shortcut_names_number_past_existing_ones() {
        let none: Vec<String> = vec![];
        assert_eq!(copy_name("a.txt", &none), "Copy of a.txt");
        assert_eq!(
            copy_name("a.txt", &["Copy of a.txt".into()]),
            "Copy of a (2).txt"
        );
        assert_eq!(
            copy_name("Reports", &["Copy of Reports".into()]),
            "Copy of Reports (2)"
        );
        assert_eq!(shortcut_name("Firefox", &none), "Shortcut to Firefox");
        assert_eq!(
            shortcut_name("firefox.desktop", &none),
            "Shortcut to firefox"
        );
        assert_eq!(
            shortcut_name("a.txt", &["Shortcut to a.txt.desktop".into()]),
            "Shortcut to a (2).txt"
        );
    }

    #[test]
    fn label_hides_a_known_extension() {
        let globs = MimeDb::parse(["50:text/plain:*.txt\n50:application/gzip:*.gz\n"]);
        let label = |name| label_for(name, &globs, Extensions::Hide);
        assert_eq!(label("New Text Document.txt"), "New Text Document");
        assert_eq!(label("a.tar.gz"), "a.tar");
        assert_eq!(label("notes.foo"), "notes.foo");
        assert_eq!(label("README"), "README");
        assert_eq!(label("foo."), "foo.");
        assert_eq!(label(".txt"), ".txt");
    }

    #[test]
    fn hide_launchers_hides_only_desktop_and_show_hides_nothing() {
        let globs = MimeDb::parse([
            "50:text/plain:*.txt\n50:application/gzip:*.gz\n50:application/x-desktop:*.desktop\n",
        ]);
        for (name, launchers, show) in [
            ("report.txt", "report.txt", "report.txt"),
            ("archive.tar.gz", "archive.tar.gz", "archive.tar.gz"),
            ("app.desktop", "app", "app.desktop"),
            ("app.tar.desktop", "app.tar", "app.tar.desktop"),
            (".desktop", ".desktop", ".desktop"),
            (".bashrc", ".bashrc", ".bashrc"),
            ("README", "README", "README"),
        ] {
            assert_eq!(
                label_for(name, &globs, Extensions::HideLaunchers),
                launchers,
                "hide_launchers: {name}"
            );
            assert_eq!(
                label_for(name, &globs, Extensions::Show),
                show,
                "show: {name}"
            );
        }
    }

    #[test]
    fn extensions_parse_the_config_spellings() {
        assert_eq!(Extensions::parse("hide"), Some(Extensions::Hide));
        assert_eq!(
            Extensions::parse("hide_launchers"),
            Some(Extensions::HideLaunchers)
        );
        assert_eq!(Extensions::parse("show"), Some(Extensions::Show));
        assert_eq!(Extensions::parse("Show"), None);
        assert_eq!(Extensions::default(), Extensions::Hide);
    }

    #[test]
    fn new_folder_name_numbers_duplicates() {
        assert_eq!(new_folder_name(&[]), "New Folder");
        assert_eq!(
            new_folder_name(&["New Folder".to_string()]),
            "New Folder (2)"
        );
        let taken = vec![
            "New Folder".to_string(),
            "New Folder (2)".to_string(),
            "Other".to_string(),
        ];
        assert_eq!(new_folder_name(&taken), "New Folder (3)");
    }

    #[test]
    fn new_text_document_name_numbers_duplicates() {
        assert_eq!(new_text_document_name(&[]), "New Text Document.txt");
        assert_eq!(
            new_text_document_name(&["New Text Document.txt".to_string()]),
            "New Text Document (2).txt"
        );
    }

    #[test]
    fn zip_path_is_numbered_past_existing_archives() {
        let p = Path::new("/home/u/Desktop/Reports");
        assert_eq!(
            zip_path(p, &[]),
            PathBuf::from("/home/u/Desktop/Reports.zip")
        );
        assert_eq!(
            zip_path(p, &["Reports.zip".into(), "Reports (2).zip".into()]),
            PathBuf::from("/home/u/Desktop/Reports (3).zip")
        );
    }

    #[test]
    fn zip_path_keeps_a_stem_that_is_not_utf8() {
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(OsStr::from_bytes(b"/d/a\xffb.txt"));
        assert_eq!(
            zip_path(p, &[]),
            Path::new(OsStr::from_bytes(b"/d/a\xffb.zip"))
        );
        assert_eq!(
            zip_path(p, &["a\u{FFFD}b.zip".into()]),
            Path::new(OsStr::from_bytes(b"/d/a\xffb (2).zip")),
            "a listed name matches its lossy form"
        );
    }

    #[test]
    fn empty_and_unchanged_names_just_close_the_box() {
        let none: Vec<String> = vec![];
        assert_eq!(verdict("a.txt", "", &none), Verdict::Cancel);
        assert_eq!(verdict("a.txt", "   ", &none), Verdict::Cancel);
        assert_eq!(verdict("a.txt", "a.txt", &none), Verdict::Cancel);
        // The trim runs before the comparison, so trailing space is not a change.
        assert_eq!(verdict("a.txt", "a.txt  ", &none), Verdict::Cancel);
    }

    #[test]
    fn a_separator_or_a_null_is_refused_naming_the_separator() {
        let none: Vec<String> = vec![];
        for bad in ["a/b.txt", "a\0b.txt"] {
            match verdict("a.txt", bad, &none) {
                Verdict::Refuse(text) => {
                    assert!(text.starts_with("A file name cannot contain"), "{text}");
                    assert!(text.contains('/'), "{text}");
                }
                other => panic!("{bad}: {other:?}"),
            }
        }
    }

    #[test]
    fn the_dot_entries_are_refused_rather_than_handed_to_rename() {
        let existing = vec!["a.txt".to_string()];
        for bad in [".", "..", "  ..  "] {
            assert_eq!(
                verdict("a.txt", bad, &existing),
                Verdict::Refuse(
                    "Cannot rename a.txt: The file name you specified is not valid \
                     or too long."
                        .to_string()
                ),
                "{bad}"
            );
        }
        assert_eq!(
            verdict("a.txt", ".hidden", &existing),
            Verdict::Rename(".hidden".to_string())
        );
        assert_eq!(
            verdict("a.txt", "...", &existing),
            Verdict::Rename("...".to_string())
        );
    }

    #[test]
    fn a_name_the_folder_already_holds_is_refused() {
        let existing = vec!["a.txt".to_string(), "b.txt".to_string()];
        assert_eq!(
            verdict("a.txt", "b.txt", &existing),
            Verdict::Refuse(
                "Cannot rename a.txt: A file with the name you specified already \
                 exists. Specify a different file name."
                    .to_string()
            )
        );
        assert_eq!(
            verdict("a.txt", "c.txt", &existing),
            Verdict::Rename("c.txt".to_string())
        );
    }
}
