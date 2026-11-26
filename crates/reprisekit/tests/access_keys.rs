//! No two labels side by side in a form, a menu or a button row share an access key.
//!
//! The test reads every prod source in the workspace. A label is a string literal with an `&`
//! access key, or a `tr!`/`kit_tr!` id, read through its crate's `locale/en.ftl` and through
//! each shipped `data/locale/<lang>` over it. Labels side by side are those in one array or
//! `vec![]`; outside one, those a function adds to one receiver (`menu.row(..)`,
//! `items.push(..)`) or builds in its own statements. Each `if`/`else` branch
//! is apart, and `match` arms are left out: a branch's label replaces another's. Labels with the
//! same text are one, and a menu's rows may share a letter with another menu's.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use reprisekit::text::access_key;

#[test]
fn no_two_labels_side_by_side_share_an_access_key() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    sources(&root.join("crates"), &mut files);
    sources(&root.join("src"), &mut files);
    let mut langs = vec![None];
    for entry in std::fs::read_dir(root.join("data/locale"))
        .unwrap()
        .flatten()
    {
        langs.push(Some(entry.file_name().to_string_lossy().into_owned()));
    }
    let mut clashes = Vec::new();
    for lang in &langs {
        let at = |dir: &Path| locale_strings(&root, dir, lang.as_deref());
        let kit = at(&root.join("crates/reprisekit"));
        let mut own = BTreeMap::new();
        let mut labels = 0;
        for file in &files {
            let text = std::fs::read_to_string(file).unwrap();
            let dir = crate_dir(file);
            let own = own.entry(dir.clone()).or_insert_with(|| at(&dir));
            let (n, found) = shared_keys(&text, own, &kit);
            labels += n;
            let rel = file.strip_prefix(&root).unwrap_or(file).display();
            let lang = lang.as_deref().unwrap_or("en");
            clashes.extend(found.into_iter().map(|c| format!("{lang} {rel}: {c}")));
        }
        assert!(
            labels > 1000,
            "the scan finds the labels in {lang:?}: {labels}"
        );
    }
    assert!(
        clashes.is_empty(),
        "labels side by side share an access key:\n{}",
        clashes.join("\n")
    );
}

#[test]
fn the_scan_finds_a_shared_key_in_one_list_and_keeps_menus_and_branches_apart() {
    let ftl = BTreeMap::from([("props".to_string(), "P&roperties".to_string())]);
    let src = r#"
        fn menus(on: bool) {
            let rows = vec![item(&tr!("props")), item("&Run Now"), item("&Run Now")];
            let bar = vec![("&File", vec![item("&Open")]), ("&Edit", vec![item("&Open")])];
            if on { m.row(row("&Enable")) } else { m.row(row("&Enable all")) }
            let s = match on { true => "&Stop", false => "&Start" };
            m.row(row("&Close"));
            n.row(row("&Copy"));
        }
        #[cfg(test)]
        mod tests { fn t() { let v = vec!["&Ab", "&Ac"]; } }
    "#;
    let (labels, found) = shared_keys(src, &ftl, &BTreeMap::new());
    assert_eq!(labels, 11);
    assert_eq!(found, ["'r' on \"P&roperties\" (3), \"&Run Now\" (3)"]);
}

/// How many labels `text` has, and each access key labels side by side in it share, with their
/// text and lines; `own` and `kit` hold the `tr!` and `kit_tr!` strings.
fn shared_keys(
    text: &str,
    own: &BTreeMap<String, String>,
    kit: &BTreeMap<String, String>,
) -> (usize, Vec<String>) {
    let mut labels = 0;
    let mut groups: BTreeMap<Group, BTreeMap<char, Vec<(String, usize)>>> = BTreeMap::new();
    for l in scan(text) {
        let label = match l.lookup {
            Lookup::Kit => kit.get(&l.text).cloned().unwrap_or(l.text),
            Lookup::Own => own.get(&l.text).cloned().unwrap_or(l.text),
            Lookup::None => l.text,
        };
        let Some(key) = access_key(&label).filter(|c| c.is_alphanumeric()) else {
            continue;
        };
        if is_entity(&label) {
            continue;
        }
        labels += 1;
        let same = groups.entry(l.group).or_default().entry(key).or_default();
        if same.iter().all(|(t, _)| *t != label) {
            same.push((label, l.line));
        }
    }
    let found = groups
        .values()
        .flatten()
        .filter(|(_, same)| same.len() > 1)
        .map(|(key, same)| {
            let all: Vec<String> = same.iter().map(|(t, n)| format!("{t:?} ({n})")).collect();
            format!("'{key}' on {}", all.join(", "))
        })
        .collect();
    (labels, found)
}

/// Prod sources under `dir`: no `tests`, `examples` or `benches` directory, no `*_tests`
/// module, no `target`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            let skip = ["target", "tests", "examples", "benches"].contains(&name.as_str());
            if !skip && !name.ends_with("_tests") {
                sources(&path, out);
            }
        } else if name.ends_with(".rs") && name != "tests.rs" && !name.ends_with("_tests.rs") {
            out.push(path);
        }
    }
}

fn crate_dir(file: &Path) -> PathBuf {
    let mut dir = file.parent().unwrap();
    while !dir.join("Cargo.toml").exists() {
        dir = dir.parent().unwrap();
    }
    dir.to_path_buf()
}

/// The strings of the crate at `dir` in `lang`: its English `locale/en.ftl` with the shipped
/// `data/locale/<lang>/<domain>.ftl` over it, as a missing id falls back to English.
fn locale_strings(root: &Path, dir: &Path, lang: Option<&str>) -> BTreeMap<String, String> {
    let mut all = strings(&dir.join("locale/en.ftl"));
    if let Some(lang) = lang
        && let Some(domain) = domain(dir)
    {
        all.extend(strings(
            &root.join(format!("data/locale/{lang}/{domain}.ftl")),
        ));
    }
    all
}

/// The domain a crate's `strings!("..")` names; reprisekit's own is `reprisekit`.
fn domain(dir: &Path) -> Option<String> {
    if dir.ends_with("reprisekit") {
        return Some("reprisekit".into());
    }
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).ok()?;
    let rest = &lib[lib.find("strings!(\"")? + "strings!(\"".len()..];
    Some(rest[..rest.find('"')?].to_string())
}

/// The `id = value` lines of a Fluent file.
fn strings(ftl: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(ftl).unwrap_or_default();
    text.lines()
        .filter(|l| l.starts_with(|c: char| c.is_ascii_lowercase()))
        .filter_map(|l| l.split_once(" = "))
        .map(|(id, value)| (id.trim().to_string(), value.to_string()))
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Open {
    Paren,
    Bracket,
    Brace,
    Branch,
    Match,
    Fn,
    Test,
}

/// Where a label sits: the offset of its array or block, and the receiver its statement adds to.
type Group = (usize, Option<String>);

enum Lookup {
    None,
    Own,
    Kit,
}

struct Label {
    text: String,
    lookup: Lookup,
    group: Group,
    line: usize,
}

/// The labels of a Rust source, outside `#[cfg(test)]` items.
fn scan(text: &str) -> Vec<Label> {
    let b = text.as_bytes();
    let mut stack: Vec<(Open, usize)> = Vec::new();
    let mut out = Vec::new();
    // The start of the current statement: just past the last `;`, `{` or `}`.
    let mut stmt = 0;
    let mut test_item = false;
    let mut i = 0;
    while i < b.len() {
        let rest = &text[i..];
        if rest.starts_with("//") {
            i += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            i += rest.find("*/").map_or(rest.len(), |e| e + 2);
        } else if rest.starts_with("#[cfg(test)]") {
            test_item = true;
            i += "#[cfg(test)]".len();
        } else if let Some((value, len)) = string(rest, i > 0 && is_ident(b[i - 1])) {
            if !stack.iter().any(|(o, _)| *o == Open::Test)
                && let Some(group) = group(&stack, &text[stmt..i])
            {
                let before = text[stmt..i].trim_end();
                let lookup = if before.ends_with("kit_tr!(") {
                    Lookup::Kit
                } else if before.ends_with("tr!(") {
                    Lookup::Own
                } else {
                    Lookup::None
                };
                out.push(Label {
                    text: value,
                    lookup,
                    group,
                    line: text[..i].matches('\n').count() + 1,
                });
            }
            i += len;
        } else if b[i] == b'\'' {
            i += char_literal(rest).unwrap_or(1);
        } else {
            match b[i] {
                b'(' => stack.push((Open::Paren, i)),
                b'[' => stack.push((Open::Bracket, i)),
                b'{' => {
                    let open = if std::mem::take(&mut test_item) {
                        Open::Test
                    } else {
                        block(&text[stmt..i])
                    };
                    stack.push((open, i));
                    stmt = i + 1;
                }
                b')' | b']' => {
                    stack.pop();
                }
                b'}' => {
                    stack.pop();
                    stmt = i + 1;
                }
                b';' => {
                    test_item = false;
                    stmt = i + 1;
                }
                _ => {}
            }
            i += 1;
        }
    }
    out
}

/// An XML entity (`&amp;`) the code writes, which is no label.
fn is_entity(s: &str) -> bool {
    s.split('&').skip(1).any(|rest| {
        rest.split_once(';').is_some_and(|(name, _)| {
            !name.is_empty() && name.bytes().all(|c| c.is_ascii_lowercase())
        })
    })
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn has_word(s: &str, word: &str) -> bool {
    s.split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|w| w == word)
}

/// What a `{` after `head` (its statement so far) opens.
fn block(head: &str) -> Open {
    let head = head.trim_end();
    if has_word(head, "fn") {
        Open::Fn
    } else if has_word(head, "match") {
        Open::Match
    } else if has_word(head, "if") || head.ends_with("else") || head.ends_with("=>") {
        Open::Branch
    } else {
        Open::Brace
    }
}

/// The group of a label at the top of `stack`, `head` being its statement so far; `None` in a
/// `match` arm or outside any function.
fn group(stack: &[(Open, usize)], head: &str) -> Option<Group> {
    for &(open, at) in stack.iter().rev() {
        match open {
            Open::Bracket => return Some((at, None)),
            Open::Branch | Open::Fn => return Some((at, receiver(head))),
            Open::Match => return None,
            Open::Paren | Open::Brace | Open::Test => {}
        }
    }
    None
}

/// The receiver a statement calls a method on: `file` in `file.row(..`, `self.items` in
/// `self.items.push(..`.
fn receiver(head: &str) -> Option<String> {
    let head = head
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .collect::<Vec<_>>()
        .join(" ");
    let call = head.find('(')?;
    let path: Vec<&str> = head[..call].trim().split('.').collect();
    let ok = path.len() > 1
        && path
            .iter()
            .all(|s| !s.is_empty() && s.bytes().all(is_ident));
    ok.then(|| path[..path.len() - 1].join("."))
}

/// The value and source length of the string literal `s` starts with, if it starts one.
/// `after_ident` says an identifier character comes right before (so `r"` is no raw string).
fn string(s: &str, after_ident: bool) -> Option<(String, usize)> {
    let raw = (!after_ident && s.starts_with('r'))
        .then(|| {
            let hashes = s[1..].bytes().take_while(|&c| c == b'#').count();
            (s.as_bytes().get(1 + hashes) == Some(&b'"')).then_some(hashes)
        })
        .flatten();
    if let Some(hashes) = raw {
        let start = 2 + hashes;
        let end = s[start..].find(&format!("\"{}", "#".repeat(hashes)))? + start;
        return Some((s[start..end].to_string(), end + 1 + hashes));
    }
    let start = match s.as_bytes() {
        [b'"', ..] => 1,
        [b'b', b'"', ..] if !after_ident => 2,
        _ => return None,
    };
    let mut value = String::new();
    let mut chars = s[start..].char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return Some((value, start + at + 1)),
            '\\' => {
                chars.next();
            }
            c => value.push(c),
        }
    }
    None
}

/// The length of the char literal `s` starts with (`'a'`, `'\n'`, `'\u{2026}'`), or `None`
/// for a lifetime.
fn char_literal(s: &str) -> Option<usize> {
    let body = &s[1..];
    let len = if let Some(esc) = body.strip_prefix('\\') {
        2 + esc.get(1..)?.find('\'')?
    } else {
        body.chars().next()?.len_utf8()
    };
    (body[len..].starts_with('\'')).then_some(len + 2)
}
