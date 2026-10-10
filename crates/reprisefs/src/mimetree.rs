//! MIME type relations from shared-mime-info's `aliases` and `subclasses`
//! files, plus the implied rules: each `text/*` is a `text/plain`, and each
//! type but `inode/*` is an `application/octet-stream`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone, Debug, Default)]
pub struct MimeTree {
    aliases: HashMap<String, String>,
    parents: HashMap<String, Vec<String>>,
}

impl MimeTree {
    /// The user's and the system's database folders; the user's alias wins.
    pub fn load() -> MimeTree {
        MimeTree::load_from(&crate::apps::mime_dirs())
    }

    pub fn load_from(dirs: &[PathBuf]) -> MimeTree {
        let read = |d: &PathBuf, f: &str| std::fs::read_to_string(d.join(f)).unwrap_or_default();
        let aliases: Vec<String> = dirs.iter().map(|d| read(d, "aliases")).collect();
        let subclasses: Vec<String> = dirs.iter().map(|d| read(d, "subclasses")).collect();
        MimeTree::parse(
            aliases.iter().map(String::as_str),
            subclasses.iter().map(String::as_str),
        )
    }

    /// Each text is lines of two types; the first text to name an alias wins.
    pub fn parse<'a>(
        aliases: impl IntoIterator<Item = &'a str>,
        subclasses: impl IntoIterator<Item = &'a str>,
    ) -> MimeTree {
        let mut t = MimeTree::default();
        for (a, b) in aliases.into_iter().flat_map(pairs) {
            t.aliases.entry(a).or_insert(b);
        }
        for (a, b) in subclasses.into_iter().flat_map(pairs) {
            let ps = t.parents.entry(a).or_default();
            if !ps.contains(&b) {
                ps.push(b);
            }
        }
        t
    }

    pub fn canonical(&self, mime: &str) -> String {
        let m = mime.to_ascii_lowercase();
        self.aliases.get(&m).cloned().unwrap_or(m)
    }

    /// `mime` as given, the type its alias names, then that type's other aliases; lower-cased.
    pub fn names(&self, mime: &str) -> Vec<String> {
        let canonical = self.canonical(mime);
        let mut out = vec![mime.to_ascii_lowercase()];
        let mut aliases: Vec<&String> = self
            .aliases
            .iter()
            .filter(|(_, to)| **to == canonical)
            .map(|(alias, _)| alias)
            .collect();
        aliases.sort();
        for m in std::iter::once(&canonical).chain(aliases) {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
        out
    }

    /// Whether `mime` is `base`, a subclass of it at any depth, or, for a
    /// `major/*` base, any type under that major. Aliases count as their types.
    pub fn is_a(&self, mime: &str, base: &str) -> bool {
        let base = self.canonical(base);
        let mut seen = HashSet::new();
        let mut todo = vec![self.canonical(mime)];
        while let Some(m) = todo.pop() {
            if !seen.insert(m.clone()) {
                continue;
            }
            if m == base || implied(&m, &base) {
                return true;
            }
            if let Some(ps) = self.parents.get(&m) {
                todo.extend(ps.iter().map(|p| self.canonical(p)));
            }
        }
        false
    }

    /// `mime` and its kinds, nearest first: as named, its alias target, parents
    /// breadth-first, then `text/plain` for `text/*`. The default program is
    /// looked up along it. `application/octet-stream` is left out so a default for
    /// an unknown file doesn't open every type with no default of its own.
    pub fn lineage(&self, mime: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let push = |out: &mut Vec<String>, m: String| {
            if !out.contains(&m) && m != "application/octet-stream" {
                out.push(m);
            }
        };
        push(&mut out, mime.to_ascii_lowercase());
        push(&mut out, self.canonical(mime));
        let mut i = 0;
        while i < out.len() {
            for p in self.parents.get(&out[i]).cloned().unwrap_or_default() {
                push(&mut out, self.canonical(&p));
            }
            i += 1;
        }
        if out.iter().any(|m| m.starts_with("text/")) {
            push(&mut out, "text/plain".into());
        }
        out
    }
}

fn implied(mime: &str, base: &str) -> bool {
    let major = mime.split('/').next().unwrap_or("");
    match base {
        "text/plain" => major == "text",
        "application/octet-stream" => major != "inode",
        b => b.strip_suffix("/*") == Some(major),
    }
}

fn pairs(text: &str) -> impl Iterator<Item = (String, String)> + '_ {
    text.lines().filter_map(|l| {
        let mut w = l.split_whitespace();
        match (w.next(), w.next()) {
            (Some(a), Some(b)) if !a.starts_with('#') => {
                Some((a.to_ascii_lowercase(), b.to_ascii_lowercase()))
            }
            _ => None,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> MimeTree {
        MimeTree::parse(
            [
                "application/x-pdf application/pdf\nimage/jpg image/jpeg\n",
                "application/x-pdf application/other\n",
            ],
            ["application/x-shellscript application/x-executable\n\
                 application/x-shellscript text/plain\n\
                 image/svg+xml application/xml\n\
                 application/xml text/plain\n\
                 application/vnd.oasis.opendocument.text application/zip\n\
                 # comment line\n"],
        )
    }

    #[test]
    fn a_subclass_is_its_parents_and_their_parents() {
        let t = tree();
        assert!(t.is_a("image/svg+xml", "application/xml"));
        assert!(t.is_a("image/svg+xml", "text/plain"), "two steps up");
        assert!(t.is_a("application/x-shellscript", "application/x-executable"));
        assert!(!t.is_a("application/xml", "image/svg+xml"), "not downwards");
        assert!(!t.is_a("application/zip", "application/pdf"));
    }

    #[test]
    fn aliases_count_as_the_types_they_name() {
        let t = tree();
        assert!(t.is_a("application/pdf", "application/x-pdf"));
        assert!(t.is_a("application/x-pdf", "application/pdf"));
        assert!(t.is_a("image/jpeg", "IMAGE/JPG"));
        assert_eq!(
            t.canonical("application/x-pdf"),
            "application/pdf",
            "the first file's alias wins"
        );
    }

    #[test]
    fn text_is_plain_text_and_everything_is_a_stream() {
        let t = MimeTree::default();
        assert!(t.is_a("text/x-csrc", "text/plain"));
        assert!(!t.is_a("image/png", "text/plain"));
        assert!(t.is_a("image/png", "application/octet-stream"));
        assert!(!t.is_a("inode/directory", "application/octet-stream"));
    }

    #[test]
    fn a_wildcard_takes_the_major_type_and_its_subclasses() {
        let t = tree();
        assert!(t.is_a("image/png", "image/*"));
        assert!(!t.is_a("text/plain", "image/*"));
        assert!(t.is_a("image/svg+xml", "text/*"), "svg is xml is text");
        assert!(t.is_a("application/vnd.oasis.opendocument.text", "application/*"));
    }

    #[test]
    fn a_lineage_runs_from_the_type_to_its_furthest_parent() {
        let t = MimeTree::parse(
            ["text/x-py text/x-python\n"],
            ["text/x-python3 text/x-python\n\
              text/x-python application/x-executable\n\
              image/svg+xml application/xml\n\
              application/xml text/plain\n\
              application/x-executable application/octet-stream\n"],
        );
        assert_eq!(
            t.lineage("text/x-python3"),
            [
                "text/x-python3",
                "text/x-python",
                "application/x-executable",
                "text/plain"
            ]
        );
        assert_eq!(
            t.lineage("text/x-py"),
            [
                "text/x-py",
                "text/x-python",
                "application/x-executable",
                "text/plain"
            ],
            "an alias walks the type it names"
        );
        assert_eq!(
            t.lineage("image/svg+xml"),
            ["image/svg+xml", "application/xml", "text/plain"]
        );
        assert_eq!(t.lineage("image/png"), ["image/png"]);
        assert_eq!(
            MimeTree::parse([], ["a/x b/y\nb/y a/x\n"]).lineage("a/x"),
            ["a/x", "b/y"]
        );
    }

    #[test]
    fn a_type_s_names_are_itself_then_its_aliases() {
        let t = MimeTree::parse(
            ["application/x-pdf application/pdf\napplication/acrobat application/pdf\n"],
            [],
        );
        let all = [
            "application/pdf",
            "application/acrobat",
            "application/x-pdf",
        ];
        assert_eq!(t.names("application/pdf"), all);
        assert_eq!(
            t.names("Application/X-PDF"),
            [
                "application/x-pdf",
                "application/pdf",
                "application/acrobat"
            ]
        );
        assert_eq!(t.names("image/png"), ["image/png"]);
    }

    #[test]
    fn a_cycle_in_the_files_ends() {
        let t = MimeTree::parse([], ["a/x b/y\nb/y a/x\n"]);
        assert!(!t.is_a("a/x", "c/z"));
    }

    #[test]
    fn the_database_folders_are_read() {
        let d = std::env::temp_dir().join(format!("reprisefs-mimetree-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("aliases"), "image/x-foo image/foo\n").unwrap();
        std::fs::write(d.join("subclasses"), "image/foo application/xml\n").unwrap();
        let t = MimeTree::load_from(&[d.clone(), d.join("missing")]);
        assert!(t.is_a("image/x-foo", "application/xml"));
        std::fs::remove_dir_all(&d).ok();
    }
}
