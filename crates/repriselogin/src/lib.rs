pub mod frame;
pub mod greeter;
pub mod session;

use serde::{Deserialize, Serialize};

pub const GREETER_SOCKET: &str = "/run/reprise-login/greeter.sock";
pub const SESSION_SOCKET: &str = "/run/reprise-login/session.sock";
/// Names the session socket in a user session's environment.
pub const SOCKET_ENV: &str = "REPRISE_LOGIN_SOCKET";
pub const GREETER_USER: &str = "reprise-greeter";

/// Lines of PAM text one message keeps.
pub const PROMPT_LINES: usize = 2;
/// Characters kept of one PAM text line.
pub const PROMPT_CHARS: usize = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Interface {
    Classic,
    Blue,
    Glass,
}

impl Interface {
    pub fn parse(s: &str) -> Option<Interface> {
        match s {
            "classic" => Some(Interface::Classic),
            "blue" => Some(Interface::Blue),
            "glass" => Some(Interface::Glass),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Interface::Classic => "classic",
            Interface::Blue => "blue",
            Interface::Glass => "glass",
        }
    }
}

/// Appends PAM text to `lines` fit for the screen: control characters become
/// spaces, blank lines go, and lines past `PROMPT_LINES` or `PROMPT_CHARS` are cut.
pub fn add_prompt_lines(lines: &mut Vec<String>, raw: &str) {
    for line in raw.lines() {
        if lines.len() >= PROMPT_LINES {
            return;
        }
        let line: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let line = line.trim();
        if !line.is_empty() {
            lines.push(line.chars().take(PROMPT_CHARS).collect());
        }
    }
}

pub fn clean_prompt(raw: &str) -> Vec<String> {
    let mut v = Vec::new();
    add_prompt_lines(&mut v, raw);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interface_names_round_trip() {
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            assert_eq!(Interface::parse(i.as_str()), Some(i));
            assert_eq!(
                serde_json::to_string(&i).unwrap(),
                format!("\"{}\"", i.as_str())
            );
        }
        assert_eq!(Interface::parse("Glass"), None);
    }

    #[test]
    fn prompt_text_loses_control_characters_and_blank_lines() {
        assert_eq!(
            clean_prompt("\n  Password\x1b[1m:\t \r\n\n"),
            vec!["Password [1m:".to_string()]
        );
    }

    #[test]
    fn prompt_text_keeps_two_lines_of_200_characters() {
        let long = "x".repeat(250);
        let lines = clean_prompt(&format!("{long}\nsecond\nthird"));
        assert_eq!(lines, vec!["x".repeat(200), "second".to_string()]);

        let mut lines = vec!["first".to_string()];
        add_prompt_lines(&mut lines, "second\nthird");
        assert_eq!(lines, vec!["first".to_string(), "second".to_string()]);
    }
}
