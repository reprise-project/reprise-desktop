use super::TextRenderer;

pub fn wrap(text: &TextRenderer, body: &str, max_w: i32, size: u32) -> Vec<String> {
    wrap_bold(text, body, max_w, size, false)
}

pub fn wrap_bold(
    text: &TextRenderer,
    body: &str,
    max_w: i32,
    size: u32,
    bold: bool,
) -> Vec<String> {
    let space = text.advance(' ', bold, size);
    wrap_by(body, max_w, space, |w| text.measure(w, bold, size))
}

/// `body` wrapped as [`wrap_bold`] does, at most `max` lines, the last kept ending in "...".
pub fn wrap_capped(
    text: &TextRenderer,
    body: &str,
    max_w: i32,
    size: u32,
    bold: bool,
    max: usize,
) -> Vec<String> {
    let mut lines = wrap_bold(text, body, max_w, size, bold);
    if lines.len() > max {
        lines.truncate(max);
        if let Some(last) = lines.last_mut() {
            while !last.is_empty() && text.measure(&format!("{last}..."), bold, size) > max_w {
                last.pop();
            }
            *last = format!("{}...", last.trim_end());
        }
    }
    lines
}

pub fn wrap_chars(body: &str, cols: usize) -> Vec<String> {
    wrap_by(body, cols as i32, 1, |w| w.chars().count() as i32)
}

fn wrap_by(body: &str, max_w: i32, space: i32, measure: impl Fn(&str) -> i32) -> Vec<String> {
    let mut lines = Vec::new();
    for para in body.split('\n') {
        let mut line = String::new();
        let mut line_w = 0;
        for word in para.split(breaks).filter(|w| !w.is_empty()) {
            let word_w = measure(word);
            if !line.is_empty() && line_w + space + word_w <= max_w {
                line.push(' ');
                line.push_str(word);
                line_w += space + word_w;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            let mut rest = word;
            let mut rest_w = word_w;
            while rest_w > max_w {
                let cut = word_break(rest, max_w, &measure);
                if cut == rest.len() {
                    break;
                }
                lines.push(rest[..cut].to_string());
                rest = &rest[cut..];
                rest_w = measure(rest);
            }
            line.push_str(rest);
            line_w = rest_w;
        }
        lines.push(line);
    }
    if lines.iter().all(|l| l.is_empty()) {
        lines.clear();
    }
    if lines.len() > 1 && crate::textlog::on() {
        crate::textlog::wrapped(&lines);
    }
    lines
}

/// Whitespace a line may break at: all but the no-break spaces.
fn breaks(c: char) -> bool {
    c.is_whitespace() && !matches!(c, '\u{a0}' | '\u{2007}' | '\u{202f}')
}

/// The byte length of the head of `word` that goes on a line `max_w` wide:
/// through the last `/`, `\`, `-`, `_` or `.` past its first char that fits,
/// else the widest prefix, never less than one char.
fn word_break(word: &str, max_w: i32, measure: impl Fn(&str) -> i32) -> usize {
    let mut fits = 0;
    let mut after_sep = 0;
    for (i, c) in word.char_indices() {
        let end = i + c.len_utf8();
        if fits > 0 && measure(&word[..end]) > max_w {
            break;
        }
        fits = end;
        if i > 0 && matches!(c, '/' | '\\' | '-' | '_' | '.') {
            after_sep = end;
        }
    }
    if after_sep > 0 { after_sep } else { fits }
}
