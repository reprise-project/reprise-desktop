//! Finding the URL a Ctrl+click lands on in a row of text.

const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];
/// Ends a URL: whitespace and the marks that commonly wrap one in prose.
const STOPS: &str = "<>\"'`{}|\\^";
/// Left off a URL's end: punctuation of the sentence around it.
const TRAILING: &str = ").,;:!?]";

/// The URL covering char `col` of `line`, if any.
pub fn at(line: &str, col: usize) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let ends = |c: char| c.is_whitespace() || c.is_control() || STOPS.contains(c);
    let mut start = col.min(chars.len());
    while start > 0 && !ends(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && !ends(chars[end]) {
        end += 1;
    }
    if col >= end {
        return None;
    }
    let word: String = chars[start..end].iter().collect();
    let (offset, scheme) = SCHEMES
        .iter()
        .filter_map(|s| word.find(s).map(|i| (i, *s)))
        .min_by_key(|&(i, _)| i)?;
    let url = word[offset..].trim_end_matches(|c| TRAILING.contains(c));
    let first = start + word[..offset].chars().count();
    let last = first + url.chars().count();
    (url.len() > scheme.len() && (first..last).contains(&col)).then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_under_ctrl_click() {
        let line = "see (https://x.y/a?b=c).,  done";
        let url = Some("https://x.y/a?b=c".to_owned());
        assert_eq!(at(line, 5), url, "its first char");
        assert_eq!(at(line, 21), url, "its last char");
        assert_eq!(at(line, 22), None, "the closing paren past it");
        assert_eq!(at(line, 4), None, "the opening paren before it");
        assert_eq!(at(line, 1), None, "a plain word");
        assert_eq!(at(line, 26), None, "a blank");
        assert_eq!(at("http://", 2), None, "a scheme alone");
        assert_eq!(
            at("go to file:///tmp/a.txt.", 10),
            Some("file:///tmp/a.txt".into())
        );
    }
}
