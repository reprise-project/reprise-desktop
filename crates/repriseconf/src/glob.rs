//! Shell-style globs over whole names: window rules' app ids, the
//! session's excluded apps, tablet pad names.

/// `pattern` against the whole of `text`: `*` is any run of characters,
/// `?` any one character, anything else itself, case included.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0, 0);
    // The last `*` seen: the pattern index after it and the text index it
    // has swallowed up to, so a mismatch retries with one character more.
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if p.get(pi) == Some(&'*') {
            star = Some((pi + 1, ti));
            pi += 1;
        } else if p.get(pi).is_some_and(|&c| c == '?' || c == t[ti]) {
            pi += 1;
            ti += 1;
        } else if let Some((after, from)) = star {
            pi = after;
            ti = from + 1;
            star = Some((after, from + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_matches_stars_and_question_marks_over_the_whole_text() {
        assert!(glob("foot", "foot"));
        assert!(!glob("foot", "footclient"));
        assert!(glob("foot*", "footclient"));
        assert!(glob("*", ""));
        assert!(glob("org.*.Nautilus", "org.gnome.Nautilus"));
        assert!(glob("*a", "*ba"), "a star in the text is an ordinary char");
        assert!(glob("a*b*c", "axxbyybc"));
        assert!(!glob("a*b*c", "axxbyyb"));
        assert!(glob("f??t", "foot"));
        assert!(!glob("f?t", "foot"));
        assert!(!glob("Foot", "foot"), "case counts");
        assert!(glob("Übersicht*", "Übersicht – Datei"));
        assert!(!glob("", "x"));
    }
}
