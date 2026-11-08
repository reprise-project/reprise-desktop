pub mod account;
pub mod fdpass;
pub mod frame;
pub mod front;
pub mod greeter;
pub mod handover;
pub mod machine;
pub mod session;
pub mod throttle;
pub mod users;

use serde::{Deserialize, Serialize};

pub const GREETER_SOCKET: &str = "/run/reprise-login/greeter.sock";
pub const SESSION_SOCKET: &str = "/run/reprise-login/session.sock";
/// Names the session socket in a user session's environment.
pub const SOCKET_ENV: &str = "REPRISE_LOGIN_SOCKET";
pub const GREETER_USER: &str = "reprise-greeter";

/// Lines of PAM text kept and shown.
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

/// The account a name typed into a Remote Desktop client names: `DOM\anna`, `anna@host` and
/// `anna@` are `anna`. The front and the session's server both use it, so they agree on names.
pub fn account_name(typed: &str) -> &str {
    let name = typed.rsplit('\\').next().unwrap_or(typed);
    name.split('@').next().unwrap_or(name)
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
    fn a_typed_name_means_its_account_without_domain_or_host() {
        for typed in [
            "anna",
            "DOM\\anna",
            ".\\anna",
            "anna@host",
            "anna@",
            "DOM\\anna@host",
        ] {
            assert_eq!(account_name(typed), "anna", "{typed}");
        }
        assert_eq!(account_name("@host"), "");
        assert_eq!(account_name("DOM\\"), "");
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

    #[test]
    fn prompt_lines_fill_up_across_messages() {
        let mut lines = Vec::new();
        add_prompt_lines(&mut lines, "  \n\tThe account\x1b[31m is locked.\r\n");
        assert_eq!(lines, ["The account [31m is locked."]);
        add_prompt_lines(&mut lines, &"x".repeat(1000));
        assert_eq!(lines[1], "x".repeat(PROMPT_CHARS));
        add_prompt_lines(&mut lines, "past the message area");
        assert_eq!(lines.len(), PROMPT_LINES);
    }
}
