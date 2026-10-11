//! Case-insensitive subsequence matching with a score, for the launcher's
//! rows. Every query char must appear in order; a name prefix scores above a
//! word-boundary start, above a consecutive run, above scattered chars. A
//! shorter candidate wins a tie. `score` keeps the better of two scans.

const PREFIX: u32 = 8;
const WORD_START: u32 = 4;
const RUN: u32 = 2;
const SCATTERED: u32 = 1;

/// Whether `cand[i]` starts a word: it follows a space, `-`, `_` or `.`, or a
/// case step (`MyApp` splits as `My|App`, `KCalc` as `K|Calc`).
pub fn is_word_start(cand: &[char], i: usize) -> bool {
    let Some(prev) = i.checked_sub(1) else {
        return true;
    };
    let (p, c) = (cand[prev], cand[i]);
    matches!(p, ' ' | '-' | '_' | '.')
        || (p.is_lowercase() && c.is_uppercase())
        || (p.is_uppercase()
            && c.is_uppercase()
            && cand.get(i + 1).is_some_and(|n| n.is_lowercase()))
}

fn char_matches(c: char, want: char) -> bool {
    c.to_lowercase().eq(want.to_lowercase())
}

fn scan(query: &[char], cand: &[char], prefer_word_start: bool) -> Option<u32> {
    let mut total: u32 = 0;
    let mut search_from = 0;
    let mut prev_matched: Option<usize> = None;
    for (k, &want) in query.iter().enumerate() {
        let word_start_at = |i: usize| is_word_start(cand, i);
        let found = if prefer_word_start {
            (search_from..cand.len())
                .find(|&i| char_matches(cand[i], want) && word_start_at(i))
                .or_else(|| (search_from..cand.len()).find(|&i| char_matches(cand[i], want)))
        } else {
            (search_from..cand.len()).find(|&i| char_matches(cand[i], want))
        };
        let i = found?;
        total += if i == k {
            PREFIX
        } else if word_start_at(i) {
            WORD_START
        } else if prev_matched == Some(i - 1) {
            RUN
        } else {
            SCATTERED
        };
        prev_matched = Some(i);
        search_from = i + 1;
    }
    Some(total)
}

pub fn score(query: &str, candidate: &str) -> Option<u32> {
    score_lowered(&lower(query), candidate)
}

fn lower(query: &str) -> Vec<char> {
    query.chars().flat_map(char::to_lowercase).collect()
}

fn score_lowered(query: &[char], candidate: &str) -> Option<u32> {
    if query.is_empty() {
        return None;
    }
    let cand: Vec<char> = candidate.chars().collect();
    let total = [scan(query, &cand, false), scan(query, &cand, true)]
        .into_iter()
        .flatten()
        .max()?;
    // Shorter names first among equals: 1000 chars is longer than any name.
    Some(total * 1024 + 1000u32.saturating_sub(cand.len() as u32))
}

/// Matching indices, best first; ties keep input order.
pub fn rank<'a>(query: &str, names: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let query = lower(query);
    let mut hits: Vec<(u32, usize)> = names
        .enumerate()
        .filter_map(|(i, n)| score_lowered(&query, n).map(|s| (s, i)))
        .collect();
    hits.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
    hits.into_iter().map(|(_, i)| i).collect()
}

/// Like `rank`, scored on the file name alone; ties put the shorter path first.
pub fn rank_paths(query: &str, paths: &[std::path::PathBuf]) -> Vec<usize> {
    let mut by_len: Vec<usize> = (0..paths.len()).collect();
    by_len.sort_by_key(|&i| paths[i].as_os_str().len());
    let names: Vec<std::borrow::Cow<'_, str>> = by_len
        .iter()
        .map(|&i| {
            paths[i]
                .file_name()
                .map_or(std::borrow::Cow::Borrowed(""), |n| n.to_string_lossy())
        })
        .collect();
    rank(query, names.iter().map(|n| n.as_ref()))
        .into_iter()
        .map(|k| by_len[k])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_char_is_no_match_and_an_empty_query_matches_nothing() {
        assert_eq!(score("nz", "Notepad"), None);
        assert_eq!(score("", "Notepad"), None);
        assert!(score("npd", "Notepad").is_some());
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(score("NOTE", "notepad"), score("note", "Notepad"));
    }

    #[test]
    fn prefix_beats_word_start_beats_run_beats_scattered() {
        // Per char: prefix 8, word start 4, consecutive 2, scattered 1.
        let prefix = score("no", "Notepad").unwrap(); // 8 + 8
        let word = score("no", "GNU Notes").unwrap(); // 4 + 2
        let run = score("no", "Gnome").unwrap(); // 1 + 2
        let scattered = score("no", "Sunroof").unwrap(); // 1 + 1
        assert!(prefix > word, "{prefix} > {word}");
        assert!(word > run, "{word} > {run}");
        assert!(run > scattered, "{run} > {scattered}");
    }

    #[test]
    fn words_start_after_a_separator_or_a_case_step() {
        let starts = |name: &str| {
            let c: Vec<char> = name.chars().collect();
            (0..c.len())
                .filter(|&i| is_word_start(&c, i))
                .collect::<Vec<_>>()
        };
        assert_eq!(starts("MyApp"), [0, 2]);
        assert_eq!(starts("KCalc"), [0, 1]);
        assert_eq!(starts("GNOME no-op"), [0, 6, 9]);
    }

    #[test]
    fn a_shorter_candidate_wins_a_tie() {
        assert!(score("no", "Notepad") > score("no", "Notepad++"));
    }

    #[test]
    fn rank_orders_best_first_and_keeps_input_order_on_ties() {
        // Gnome Terminal has a consecutive run and beats Sunroof's scattered match;
        // differing lengths keep Notion and Notepad from tying.
        let names = ["Notion", "Gnome Terminal", "Notepad", "Sunroof", "Files"];
        assert_eq!(rank("no", names.iter().copied()), vec![0, 2, 1, 3]);
        assert_eq!(rank("zzz", names.iter().copied()), Vec::<usize>::new());

        // Notion and Notice tie on score and length, so input order decides.
        let tied = ["Notion", "Notice", "Gnome Terminal", "Sunroof"];
        assert_eq!(rank("no", tied.iter().copied()), vec![0, 1, 2, 3]);
    }

    #[test]
    fn rank_paths_scores_the_file_name_and_puts_the_shorter_path_first_on_a_tie() {
        let paths: Vec<std::path::PathBuf> = [
            "/home/u/a/b/c/notes.txt",
            "/home/u/nightowl-test",
            "/home/u/notes.txt",
            "/home/u/notes/readme",
        ]
        .iter()
        .map(Into::into)
        .collect();
        // "notes/readme" matches only through its folder, which is not scored.
        assert_eq!(rank_paths("note", &paths), vec![2, 0, 1]);
    }
}
