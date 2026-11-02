//! Renaming several items at once: the new names a find and replace or a numbered base name
//! gives, which of them cannot be taken, and the order of renames that applies them.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scheme {
    /// Every match of `find` becomes `with`; an empty `find` changes nothing.
    Replace {
        find: String,
        with: String,
        ignore_case: bool,
    },
    /// `base (1)`, `base (2)`... in the order given.
    Number { base: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub scheme: Scheme,
    /// The extension stays as it was and the scheme works on the rest of the name.
    pub keep_ext: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Empty, `.` or `..`, holding `/` or NUL, or over 255 bytes.
    Invalid,
    /// Another item keeps this name, or another new name is the same.
    Taken,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub old: String,
    pub new: String,
    pub problem: Option<Problem>,
}

impl Row {
    pub fn changes(&self) -> bool {
        self.old != self.new
    }
}

/// The rows for `items` (name and whether it is a folder) in the order given. `others` are the
/// folder's names that are not being renamed.
pub fn plan(items: &[(&str, bool)], others: &[String], opts: &Options) -> Vec<Row> {
    let mut rows: Vec<Row> = items
        .iter()
        .enumerate()
        .map(|(i, &(old, is_dir))| {
            let (stem, ext) = match opts.keep_ext && !is_dir {
                true => split_ext(old),
                false => (old, ""),
            };
            let stem = match &opts.scheme {
                Scheme::Replace {
                    find,
                    with,
                    ignore_case,
                } => replace_all(stem, find, with, *ignore_case),
                Scheme::Number { base } => format!("{base} ({})", i + 1),
            };
            Row {
                old: old.to_string(),
                new: stem + ext,
                problem: None,
            }
        })
        .collect();
    let kept: Vec<&str> = rows
        .iter()
        .filter(|r| !r.changes())
        .map(|r| r.old.as_str())
        .chain(others.iter().map(String::as_str))
        .collect();
    let problems: Vec<Option<Problem>> = rows
        .iter()
        .map(|r| {
            if !r.changes() {
                None
            } else if !valid(&r.new) {
                Some(Problem::Invalid)
            } else if kept.contains(&r.new.as_str())
                || rows.iter().filter(|o| o.new == r.new).count() > 1
            {
                Some(Problem::Taken)
            } else {
                None
            }
        })
        .collect();
    for (r, p) in rows.iter_mut().zip(problems) {
        r.problem = p;
    }
    rows
}

/// Whether `rows` can be applied: no problem, and at least one name changes.
pub fn ready(rows: &[Row]) -> bool {
    rows.iter().all(|r| r.problem.is_none()) && rows.iter().any(Row::changes)
}

/// The renames that apply `rows` to `paths` (parallel to them), in order. When a new name is
/// another item's old one, every item first moves to a temporary name holding `tag`, so no rename
/// lands on a name still in use.
pub fn steps(paths: &[PathBuf], rows: &[Row], tag: &str) -> Vec<(PathBuf, PathBuf)> {
    let moves: Vec<(&PathBuf, &Row)> = paths
        .iter()
        .zip(rows)
        .filter(|(_, r)| r.changes())
        .collect();
    let target = |p: &Path, name: &str| p.with_file_name(name);
    let crossed = moves
        .iter()
        .any(|(_, r)| moves.iter().any(|(_, o)| o.old == r.new));
    if !crossed {
        return moves
            .iter()
            .map(|(p, r)| ((*p).clone(), target(p, &r.new)))
            .collect();
    }
    let temps: Vec<PathBuf> = moves
        .iter()
        .enumerate()
        .map(|(i, (p, _))| target(p, &format!(".{tag}-{i}")))
        .collect();
    let first = moves
        .iter()
        .zip(&temps)
        .map(|((p, _), t)| ((*p).clone(), t.clone()));
    let then = moves
        .iter()
        .zip(&temps)
        .map(|((p, r), t)| (t.clone(), target(p, &r.new)));
    first.chain(then).collect()
}

/// A rename of [`apply`] that failed: its source, the error, and the renames before it that
/// could not be reversed, for the undo stack.
#[derive(Debug)]
pub struct Failed {
    pub from: PathBuf,
    pub error: std::io::Error,
    pub left: Vec<(PathBuf, PathBuf)>,
}

/// Runs `steps` in order, none of them replacing a file. On a failure the renames already done
/// are reversed, last first.
pub fn apply(steps: &[(PathBuf, PathBuf)]) -> Result<(), Failed> {
    for (i, (from, to)) in steps.iter().enumerate() {
        if let Err(error) = crate::hidden::rename(from, to) {
            let mut left = steps[..i].to_vec();
            while let Some((a, b)) = left.last() {
                if let Err(e) = crate::hidden::rename(b, a) {
                    log::warn!("reprisefs: could not rename {} back: {e}", b.display());
                    break;
                }
                left.pop();
            }
            return Err(Failed {
                from: from.clone(),
                error,
                left,
            });
        }
    }
    Ok(())
}

fn valid(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\0'])
        && name.len() <= 255
}

/// The stem and extension as [`crate::names::numbered`] splits them: from the last dot, unless
/// it starts the name or a space follows it.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && !name[dot..].contains(' ') => name.split_at(dot),
        _ => (name, ""),
    }
}

fn replace_all(hay: &str, find: &str, with: &str, ignore_case: bool) -> String {
    if find.is_empty() {
        return hay.to_string();
    }
    if !ignore_case {
        return hay.replace(find, with);
    }
    let needle: Vec<char> = find.chars().flat_map(char::to_lowercase).collect();
    let mut out = String::with_capacity(hay.len());
    let mut rest = hay;
    while let Some(c) = rest.chars().next() {
        match folded_prefix(rest, &needle) {
            Some(len) => {
                out.push_str(with);
                rest = &rest[len..];
            }
            None => {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    out
}

/// The byte length of the start of `s` whose lowercase is `needle`.
fn folded_prefix(s: &str, needle: &[char]) -> Option<usize> {
    let mut want = needle.iter();
    let mut len = 0;
    for c in s.chars() {
        for l in c.to_lowercase() {
            if want.next() != Some(&l) {
                return None;
            }
        }
        len += c.len_utf8();
        if want.len() == 0 {
            return Some(len);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replace(find: &str, with: &str, ignore_case: bool, keep_ext: bool) -> Options {
        Options {
            scheme: Scheme::Replace {
                find: find.into(),
                with: with.into(),
                ignore_case,
            },
            keep_ext,
        }
    }

    fn number(base: &str, keep_ext: bool) -> Options {
        Options {
            scheme: Scheme::Number { base: base.into() },
            keep_ext,
        }
    }

    fn news(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.new.as_str()).collect()
    }

    #[test]
    fn numbering_follows_the_order_given_and_keeps_extensions() {
        let items = [("b.jpg", false), ("a.png", false), ("Notes", true)];
        let rows = plan(&items, &[], &number("Photo", true));
        assert_eq!(news(&rows), ["Photo (1).jpg", "Photo (2).png", "Photo (3)"]);
        assert!(ready(&rows));
        let rows = plan(&items, &[], &number("Photo", false));
        assert_eq!(news(&rows), ["Photo (1)", "Photo (2)", "Photo (3)"]);
    }

    #[test]
    fn a_folder_keeps_its_dots_and_a_dot_file_has_no_extension() {
        let items = [("v1.2", true), (".bashrc", false), ("Mr. Smith", false)];
        let rows = plan(&items, &[], &replace(".", "_", false, true));
        assert_eq!(news(&rows), ["v1_2", "_bashrc", "Mr_ Smith"]);
    }

    #[test]
    fn find_and_replace_works_on_the_stem_unless_extensions_are_off() {
        let items = [("draft.txt", false), ("txt notes.txt", false)];
        let rows = plan(&items, &[], &replace("txt", "md", false, true));
        assert_eq!(news(&rows), ["draft.txt", "md notes.txt"]);
        let rows = plan(&items, &[], &replace("txt", "md", false, false));
        assert_eq!(news(&rows), ["draft.md", "md notes.md"]);
    }

    #[test]
    fn ignoring_case_matches_any_case_and_keeps_the_rest() {
        let items = [
            ("IMG_001.JPG", false),
            ("img_002.jpg", false),
            ("Straße img", false),
        ];
        let rows = plan(&items, &[], &replace("img_", "Holiday ", true, true));
        assert_eq!(
            news(&rows),
            ["Holiday 001.JPG", "Holiday 002.jpg", "Straße img"]
        );
        let rows = plan(&items, &[], &replace("img_", "Holiday ", false, true));
        assert_eq!(
            news(&rows),
            ["IMG_001.JPG", "Holiday 002.jpg", "Straße img"]
        );
        let rows = plan(
            &[("STRASSE", false)],
            &[],
            &replace("straße", "x", true, true),
        );
        assert_eq!(
            news(&rows),
            ["STRASSE"],
            "no match across a folded expansion"
        );
    }

    #[test]
    fn an_empty_find_changes_nothing_and_is_not_ready() {
        let rows = plan(
            &[("a", false), ("b", false)],
            &[],
            &replace("", "x", false, true),
        );
        assert_eq!(news(&rows), ["a", "b"]);
        assert!(rows.iter().all(|r| r.problem.is_none()));
        assert!(!ready(&rows), "nothing to rename");
    }

    #[test]
    fn a_name_kept_elsewhere_or_given_twice_is_taken() {
        let others = vec!["Photo (2).jpg".to_string()];
        let items = [("a.jpg", false), ("b.jpg", false)];
        let rows = plan(&items, &others, &number("Photo", true));
        assert_eq!(rows[0].problem, None);
        assert_eq!(rows[1].problem, Some(Problem::Taken));
        assert!(!ready(&rows));

        let items = [("a1.txt", false), ("a2.txt", false), ("a", false)];
        let rows = plan(&items, &[], &replace("1", "2", false, true));
        assert_eq!(rows[0].problem, Some(Problem::Taken), "a2.txt stays");
        assert_eq!(rows[1].problem, None, "unchanged");
        let rows = plan(&items, &[], &replace("a", "", false, false));
        assert_eq!(news(&rows), ["1.txt", "2.txt", ""]);
        assert_eq!(rows[2].problem, Some(Problem::Invalid));

        let items = [("x.a", false), ("x.b", false)];
        let rows = plan(&items, &[], &replace("x", "y", false, false));
        assert!(ready(&rows));
        let rows = plan(&items, &[], &replace(".a", ".b", false, false));
        assert_eq!(rows[0].problem, Some(Problem::Taken), "x.b keeps its name");
    }

    #[test]
    fn slashes_dots_and_long_names_are_not_valid() {
        let items = [("a", false), ("b", false), ("c", false)];
        for with in ["x/y", ".", "..", &"n".repeat(256)] {
            let rows = plan(&items[..1], &[], &replace("a", with, false, true));
            assert_eq!(rows[0].problem, Some(Problem::Invalid), "{with:?}");
        }
        let rows = plan(&items, &[], &replace("b", "\0", false, true));
        assert_eq!(rows[1].problem, Some(Problem::Invalid));
    }

    #[test]
    fn names_taken_by_each_other_go_through_temporary_names() {
        let dir = PathBuf::from("/home/u/Pictures");
        let paths = vec![dir.join("a"), dir.join("b"), dir.join("c")];
        let items = [("a", false), ("b", false), ("c", false)];
        let rows = plan(&items, &[], &number("x", true));
        assert_eq!(
            steps(&paths, &rows, "t"),
            vec![
                (dir.join("a"), dir.join("x (1)")),
                (dir.join("b"), dir.join("x (2)")),
                (dir.join("c"), dir.join("x (3)")),
            ]
        );
        let rows = plan(&items[..2], &[], &replace("", "", false, true));
        assert!(steps(&paths[..2], &rows, "t").is_empty());
        let rows = vec![
            Row {
                old: "a".into(),
                new: "b".into(),
                problem: None,
            },
            Row {
                old: "b".into(),
                new: "a".into(),
                problem: None,
            },
        ];
        assert_eq!(
            steps(&paths[..2], &rows, "t"),
            vec![
                (dir.join("a"), dir.join(".t-0")),
                (dir.join("b"), dir.join(".t-1")),
                (dir.join(".t-0"), dir.join("b")),
                (dir.join(".t-1"), dir.join("a")),
            ]
        );
    }

    #[test]
    fn apply_swaps_names_and_a_failure_puts_every_name_back() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let d = dir.path();
        std::fs::write(d.join("a"), "A").expect("write");
        std::fs::write(d.join("b"), "B").expect("write");
        let paths = vec![d.join("a"), d.join("b")];
        let rows = vec![
            Row {
                old: "a".into(),
                new: "b".into(),
                problem: None,
            },
            Row {
                old: "b".into(),
                new: "a".into(),
                problem: None,
            },
        ];
        apply(&steps(&paths, &rows, "t")).expect("the swap applies");
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "B");
        assert_eq!(std::fs::read_to_string(d.join("b")).unwrap(), "A");

        std::fs::write(d.join("c"), "C").expect("write");
        let s = vec![(d.join("a"), d.join("x")), (d.join("b"), d.join("c"))];
        let failed = apply(&s).expect_err("c is taken");
        assert_eq!(failed.from, d.join("b"));
        assert!(failed.left.is_empty(), "the first rename was reversed");
        assert_eq!(std::fs::read_to_string(d.join("a")).unwrap(), "B");
        assert!(!d.join("x").exists());
    }

    #[test]
    fn a_rotation_among_the_items_is_not_a_clash() {
        let items = [("Photo (1)", false), ("Photo (2)", false)];
        let rows = plan(&items, &[], &replace("Photo", "Photo", false, true));
        assert!(!ready(&rows), "nothing changes");
        let items = [("Photo (2)", false), ("Photo (1)", false)];
        let rows = plan(&items, &[], &number("Photo", true));
        assert_eq!(news(&rows), ["Photo (1)", "Photo (2)"]);
        assert!(ready(&rows), "each name is freed by the other's rename");
    }
}
