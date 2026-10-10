//! Shell globs on names (`*.png`, `IMG_????.jpg`, `*.[ch]`): whole-name, case-insensitive.

pub fn matches(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().flat_map(char::to_lowercase).collect();
    let n: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
    match_at(&p, &n)
}

/// Backtracking over the last `*`: linear for the usual one-star
/// patterns, never exponential.
fn match_at(p: &[char], n: &[char]) -> bool {
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                '*' => {
                    star = Some((pi, ni));
                    pi += 1;
                    continue;
                }
                '?' => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                '[' => {
                    if let Some((hit, end)) = class(&p[pi..], n[ni]) {
                        if hit {
                            pi += end;
                            ni += 1;
                            continue;
                        }
                    } else if n[ni] == '[' {
                        pi += 1;
                        ni += 1;
                        continue;
                    }
                }
                c if c == n[ni] => {
                    pi += 1;
                    ni += 1;
                    continue;
                }
                _ => {}
            }
        }
        match star {
            Some((sp, sn)) => {
                pi = sp + 1;
                ni = sn + 1;
                star = Some((sp, sn + 1));
            }
            None => return false,
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// The class at the start of `p` (which begins with `[`) tested against
/// `c`: whether it matched, and how many pattern chars it spans. None
/// when the class never closes.
fn class(p: &[char], c: char) -> Option<(bool, usize)> {
    let mut i = 1;
    let negate = matches!(p.get(i), Some('!') | Some('^'));
    if negate {
        i += 1;
    }
    let mut hit = false;
    let mut first = true;
    while i < p.len() {
        if p[i] == ']' && !first {
            return Some((hit != negate, i + 1));
        }
        first = false;
        if i + 2 < p.len() && p[i + 1] == '-' && p[i + 2] != ']' {
            hit |= (p[i]..=p[i + 2]).contains(&c);
            i += 3;
        } else {
            hit |= p[i] == c;
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::matches;

    #[test]
    fn a_star_matches_any_run() {
        assert!(matches("*.png", "a.png"));
        assert!(matches("*.png", ".png"));
        assert!(matches("*", ""));
        assert!(!matches("*.png", "a.png.txt"), "the whole name must match");
    }

    #[test]
    fn a_question_mark_is_one_char() {
        assert!(matches("IMG_????.jpg", "IMG_0042.jpg"));
        assert!(!matches("IMG_????.jpg", "IMG_042.jpg"));
        assert!(matches("?", "é"), "one char, not one byte");
    }

    #[test]
    fn classes_ranges_and_negation() {
        assert!(matches("*.[ch]", "x.c"));
        assert!(matches("*.[ch]", "x.h"));
        assert!(!matches("*.[ch]", "x.o"));
        assert!(matches("[a-c]x", "bx"));
        assert!(!matches("[!a-c]x", "bx"));
        assert!(matches("[^a-c]x", "dx"));
    }

    #[test]
    fn case_is_folded() {
        assert!(matches("*.PNG", "a.png"));
        assert!(matches("*.png", "A.PNG"));
        assert!(matches("[A-C]", "b"));
    }

    #[test]
    fn an_unclosed_bracket_is_literal() {
        assert!(matches("a[", "a["));
        assert!(!matches("a[", "ab"));
    }
}
