//! Reads `gio mount`'s prompts off its stdout: the server's message on its own lines, then one
//! field at a time (`User [default]: `, `Password: `) or numbered choices and `Choice: `.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    User,
    Domain,
    Password,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Prompt {
    /// One field. `message` is set on the first field of a new password request.
    Field {
        field: Field,
        default: Option<String>,
        message: Option<String>,
    },
    Choice {
        message: String,
        choices: Vec<String>,
    },
}

#[derive(Default)]
pub struct Conversation {
    buf: Vec<u8>,
}

impl Conversation {
    /// Takes the next bytes gio printed: the prompt they end on, if they end on one.
    pub fn feed(&mut self, bytes: &[u8]) -> Option<Prompt> {
        self.buf.extend_from_slice(bytes);
        let text = String::from_utf8_lossy(&self.buf);
        let (lines, tail) = match text.rfind('\n') {
            Some(i) => (&text[..i], &text[i + 1..]),
            None => ("", text.as_ref()),
        };
        let (name, default) = prompt(tail)?;
        let lines: Vec<&str> = lines.split('\n').collect();
        let out = if name == "Choice" {
            let mut choices = Vec::new();
            let mut message = Vec::new();
            for l in lines {
                match choice(l) {
                    Some(c) => choices.push(c.to_string()),
                    None => message.push(l),
                }
            }
            Prompt::Choice {
                message: joined(&message),
                choices,
            }
        } else {
            let field = match name {
                "User" => Field::User,
                "Domain" => Field::Domain,
                _ => Field::Password,
            };
            let message = joined(&lines);
            Prompt::Field {
                field,
                default: default.map(str::to_string),
                message: (!message.is_empty()).then_some(message),
            }
        };
        self.buf.clear();
        Some(out)
    }
}

/// `tail` as a prompt: its name and the default in brackets.
fn prompt(tail: &str) -> Option<(&str, Option<&str>)> {
    let body = tail.strip_suffix(": ")?;
    let (name, default) = match body.split_once(" [") {
        Some((n, d)) => (n, Some(d.strip_suffix(']')?)),
        None => (body, None),
    };
    ["User", "Domain", "Password", "Choice"]
        .contains(&name)
        .then_some((name, default))
}

/// A choice line, `[2] Cancel Login`: its text.
fn choice(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (n, text) = rest.split_once("] ")?;
    n.parse::<u32>().ok().map(|_| text)
}

/// The message lines, blank ones at either end dropped.
fn joined(lines: &[&str]) -> String {
    let start = lines.iter().position(|l| !l.trim().is_empty());
    let end = lines.iter().rposition(|l| !l.trim().is_empty());
    match (start, end) {
        (Some(s), Some(e)) => lines[s..=e].join("\n"),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_request_asks_each_field_and_the_first_carries_the_message() {
        let mut c = Conversation::default();
        assert_eq!(
            c.feed(b"Authentication Required\nEnter user and pass"),
            None
        );
        assert_eq!(
            c.feed("word for share \u{201c}private\u{201d}:\nUser [user]: ".as_bytes()),
            Some(Prompt::Field {
                field: Field::User,
                default: Some("user".into()),
                message: Some(
                    "Authentication Required\nEnter user and password for share \u{201c}private\u{201d}:"
                        .into()
                ),
            })
        );
        assert_eq!(
            c.feed(b"Domain [WORKGROUP]: "),
            Some(Prompt::Field {
                field: Field::Domain,
                default: Some("WORKGROUP".into()),
                message: None,
            })
        );
        assert_eq!(
            c.feed(b"Password: "),
            Some(Prompt::Field {
                field: Field::Password,
                default: None,
                message: None,
            })
        );
        // gio ends a hidden answer with a newline; a second request starts a new message.
        assert_eq!(c.feed(b"\n"), None);
        assert!(matches!(
            c.feed(b"Authentication Required\nUser [user]: "),
            Some(Prompt::Field {
                message: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn a_question_lists_its_choices_apart_from_its_message() {
        let mut c = Conversation::default();
        let text = "Identity Verification Failed\nVerifying the identity of \u{201c}vm\u{201d} failed.\n\nThe identity sent is \u{201c}x\u{201d}.\n[1] Log In Anyway\n[2] Cancel Login\nChoice: ";
        assert_eq!(
            c.feed(text.as_bytes()),
            Some(Prompt::Choice {
                message: "Identity Verification Failed\nVerifying the identity of \u{201c}vm\u{201d} failed.\n\nThe identity sent is \u{201c}x\u{201d}.".into(),
                choices: vec!["Log In Anyway".into(), "Cancel Login".into()],
            })
        );
    }

    #[test]
    fn a_message_line_ending_in_a_colon_is_no_prompt() {
        let mut c = Conversation::default();
        assert_eq!(c.feed(b"Enter password for share:\n"), None);
        assert_eq!(c.feed(b"Something: "), None);
    }

    #[test]
    fn a_character_split_between_reads_comes_out_whole() {
        let mut c = Conversation::default();
        let text = "\u{201c}nas\u{201d}\nPassword: ".as_bytes();
        assert_eq!(c.feed(&text[..2]), None);
        assert_eq!(
            c.feed(&text[2..]),
            Some(Prompt::Field {
                field: Field::Password,
                default: None,
                message: Some("\u{201c}nas\u{201d}".into()),
            })
        );
    }
}
