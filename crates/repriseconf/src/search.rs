//! Word-start matching shared by the Settings search and the compositor's Start and launcher search.

/// Whether a word of `name` (split by `fuzzy::is_word_start`) starts with
/// `query`, the rest of `query` running on from there ("sticky no" fits
/// "Sticky Notes"). The Start menu lists only such names.
pub fn word_starts(query: &str, name: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    let name: Vec<char> = name.chars().collect();
    (0..name.len()).any(|i| {
        reprisekit::fuzzy::is_word_start(&name, i)
            && name[i..]
                .iter()
                .flat_map(|c| c.to_lowercase())
                .collect::<String>()
                .starts_with(&query)
    })
}

/// Whether every whitespace-separated word of `query` word-starts at least one of
/// `haystacks`. A query with no words matches nothing.
pub fn all_words(query: &str, haystacks: &[&str]) -> bool {
    let mut words = query.split_whitespace().peekable();
    words.peek().is_some() && words.all(|w| haystacks.iter().any(|h| word_starts(w, h)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_of_the_name_starts_with_the_text() {
        for name in [
            "Notepad",
            "Sticky Notes",
            "GNOME no-op",
            "file_notes",
            "a.Note",
        ] {
            assert!(word_starts("no", name), "{name}");
        }
        for name in ["Network Connections", "Print Editor", "Piano"] {
            assert!(!word_starts("no", name), "{name}");
        }
        assert!(word_starts("NO", "Notepad"), "case-insensitive");
        assert!(word_starts("sticky no", "Sticky Notes"));
        assert!(
            word_starts("calc", "KCalc"),
            "an uppercase run ends before its last capital"
        );
        assert!(!word_starts("", "Notepad"));
    }

    #[test]
    fn every_query_word_starts_a_word_of_some_haystack() {
        let hay = ["Mouse", "pointer speed", "touchpad"];
        assert!(all_words("mouse speed", &hay));
        assert!(
            all_words("Speed  MOUSE", &hay),
            "any order, any case, runs of spaces"
        );
        assert!(all_words("touch", &hay));
        assert!(
            !all_words("mouse zzz", &hay),
            "one word matching nothing fails the query"
        );
        assert!(!all_words("peed", &hay), "a word's start, not its middle");
        assert!(!all_words("", &hay));
        assert!(!all_words("   ", &hay));
        assert!(!all_words("mouse", &[]));
    }
}
