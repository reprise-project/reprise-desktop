//! The `Host` names of `~/.ssh/config` that the new-tab menu offers.

use std::path::{Path, PathBuf};

/// The names on `text`'s `Host` lines, skipping patterns (`*`, `?`) and negations (`!`).
pub fn parse(text: &str) -> Vec<String> {
    scan(text).0
}

/// The hosts of `dir/config` and of the files its `Include` lines name (relative to `dir`, `~/`
/// from its parent, globs within one directory); included files' `Include` lines are not followed.
pub fn read(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join("config")) else {
        return Vec::new();
    };
    let (mut hosts, includes) = scan(&text);
    for path in includes.iter().flat_map(|i| expand(dir, i)) {
        if let Ok(text) = std::fs::read_to_string(path) {
            hosts.extend(parse(&text));
        }
    }
    let mut seen = std::collections::HashSet::new();
    hosts.retain(|h| seen.insert(h.clone()));
    hosts
}

/// `text`'s host names and `Include` arguments.
fn scan(text: &str) -> (Vec<String>, Vec<String>) {
    let (mut hosts, mut includes) = (Vec::new(), Vec::new());
    for line in text.lines().map(str::trim) {
        if line.starts_with('#') {
            continue;
        }
        let end = line
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(line.len());
        let (key, rest) = line.split_at(end);
        let words = rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .split_whitespace();
        if key.eq_ignore_ascii_case("host") {
            hosts.extend(
                words
                    .filter(|w| !w.contains(['*', '?']) && !w.starts_with('!'))
                    .map(str::to_owned),
            );
        } else if key.eq_ignore_ascii_case("include") {
            includes.extend(words.map(str::to_owned));
        }
    }
    (hosts, includes)
}

/// The files an `Include` argument names, a pattern's in name order.
fn expand(dir: &Path, pattern: &str) -> Vec<PathBuf> {
    let path = match pattern.strip_prefix("~/") {
        Some(rest) => dir.parent().unwrap_or(dir).join(rest),
        None => dir.join(pattern),
    };
    let (Some(parent), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str()))
    else {
        return Vec::new();
    };
    if !name.contains(['*', '?']) {
        return vec![path.clone()];
    }
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| glob(name, n)))
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

/// Whether `name` matches `pat`, whose `*` stands for any run of characters and `?` for one.
fn glob(pat: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pat.chars().collect(), name.chars().collect());
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ni));
                pi += 1;
            }
            Some(&c) if c == '?' || c == n[ni] => {
                pi += 1;
                ni += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    pi = sp + 1;
                    ni = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_hosts_skip_wildcards() {
        let text = "Host a b *.lan !x\n  User me\nHost ?ab\nMatch host c exec true\n  Port 2\n# Host d\nhost=e\n";
        assert_eq!(parse(text), ["a", "b", "e"]);
    }

    #[test]
    fn ssh_hosts_follow_include_one_level() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join(".ssh");
        std::fs::create_dir_all(d.join("config.d")).unwrap();
        std::fs::write(d.join("config"), "Include config.d/*\nHost top\n").unwrap();
        std::fs::write(d.join("config.d/1-work"), "Host work\nInclude deeper\n").unwrap();
        std::fs::write(d.join("config.d/2-home"), "Host home top\n").unwrap();
        std::fs::write(d.join("deeper"), "Host never\n").unwrap();
        assert_eq!(read(&d), ["top", "work", "home"]);
    }

    #[test]
    fn ssh_hosts_include_from_home_and_absolute() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join(".ssh");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(dir.path().join("extra"), "Host fromhome\n").unwrap();
        let abs = dir.path().join("abs");
        std::fs::write(&abs, "Host fromabs\n").unwrap();
        let config = format!("Include ~/extra {}\n", abs.display());
        std::fs::write(d.join("config"), config).unwrap();
        assert_eq!(read(&d), ["fromhome", "fromabs"]);
    }

    #[test]
    fn ssh_hosts_missing_file_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read(&dir.path().join(".ssh")).is_empty());
    }
}
