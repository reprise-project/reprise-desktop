pub fn split_mnemonic(label: &str) -> (String, Option<usize>) {
    let mut out = String::with_capacity(label.len());
    let mut mnemonic = None;
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.peek() {
                Some('&') => {
                    chars.next();
                    out.push('&');
                }
                Some(_) if mnemonic.is_none() => mnemonic = Some(out.chars().count()),
                _ => {}
            }
        } else {
            out.push(c);
        }
    }
    (out, mnemonic)
}

/// `s` as a label [`split_mnemonic`] reads back unchanged: every `&` doubled,
/// so a file or program name never turns a letter into an access key.
pub fn escape_mnemonic(s: &str) -> String {
    s.replace('&', "&&")
}

pub fn access_key(label: &str) -> Option<char> {
    let (plain, m) = split_mnemonic(label);
    m.and_then(|m| plain.chars().nth(m))
        .and_then(|c| c.to_lowercase().next())
}
