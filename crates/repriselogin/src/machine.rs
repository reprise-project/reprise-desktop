//! The machine settings in `MACHINE_CONFIG`: `[login]` and `[remote_desktop]`.

use std::ops::RangeInclusive;

use crate::Interface;

pub const MACHINE_CONFIG: &str = "/etc/reprise/login.toml";

/// The first line of a save request: `expect ` and the [`file_tag`] of the file the settings
/// were read from, so the helper replaces only that file.
const EXPECT: &str = "expect ";

/// A tag for the bytes of `MACHINE_CONFIG`, `none` while it is missing: FNV-1a, which tells a
/// file changed since it was read and stands against no forger.
pub fn file_tag(bytes: Option<&[u8]>) -> String {
    let Some(bytes) = bytes else {
        return "none".into();
    };
    let hash = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

/// What Settings sends `reprise-login-settings`: the expected tag, then the settings as TOML.
pub fn save_request(tag: &str, toml: &str) -> String {
    format!("{EXPECT}{tag}\n{toml}")
}

/// The tag and the TOML of a [`save_request`]; `None` without the tag line.
pub fn parse_save_request(input: &str) -> Option<(&str, &str)> {
    let (first, toml) = input.split_once('\n')?;
    Some((first.strip_prefix(EXPECT)?, toml))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MachineConfig {
    pub login: LoginConfig,
    pub remote: MachineRemote,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoginConfig {
    /// `None` shows the login screen in the last interface used.
    pub interface: Option<Interface>,
    pub show_users: bool,
    pub autologin_user: Option<String>,
    /// Lets the login screen listen on a Wayland socket, for capture tools while testing.
    pub greeter_socket: bool,
}

impl Default for LoginConfig {
    fn default() -> Self {
        LoginConfig {
            interface: None,
            show_users: true,
            autologin_user: None,
            greeter_socket: false,
        }
    }
}

/// Remote logins to this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MachineRemote {
    pub enabled: bool,
    pub port: u16,
    /// Members of this group may log in remotely.
    pub group: String,
    pub max_sessions: u32,
    /// `None` never ends a disconnected session.
    pub end_disconnected_after_min: Option<u32>,
}

impl Default for MachineRemote {
    fn default() -> Self {
        MachineRemote {
            enabled: false,
            port: 3389,
            group: "reprise-remote".into(),
            max_sessions: 8,
            end_disconnected_after_min: None,
        }
    }
}

impl MachineConfig {
    /// Reads the settings, returning a warning for each value it could not use.
    pub fn parse(text: &str) -> (MachineConfig, Vec<String>) {
        let mut config = MachineConfig::default();
        let mut warnings = Vec::new();
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(e) => {
                warnings.push(one_line(&e.to_string()));
                return (config, warnings);
            }
        };
        if let Some(login) = section(&table, "login", &mut warnings) {
            for (key, value) in login {
                if let Err(w) = config.login.set(key, value) {
                    warnings.push(format!("login.{key}: {w}"));
                }
            }
        }
        if let Some(remote) = section(&table, "remote_desktop", &mut warnings) {
            for (key, value) in remote {
                if let Err(w) = config.remote.set(key, value) {
                    warnings.push(format!("remote_desktop.{key}: {w}"));
                }
            }
        }
        (config, warnings)
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::from("[login]\n");
        let l = &self.login;
        if let Some(i) = l.interface {
            line(&mut out, "interface", i.as_str());
        }
        line(&mut out, "show_users", l.show_users);
        if let Some(user) = &l.autologin_user {
            line(&mut out, "autologin_user", user.as_str());
        }
        line(&mut out, "greeter_socket", l.greeter_socket);
        out.push_str("\n[remote_desktop]\n");
        let r = &self.remote;
        line(&mut out, "enabled", r.enabled);
        line(&mut out, "port", i64::from(r.port));
        line(&mut out, "group", r.group.as_str());
        line(&mut out, "max_sessions", i64::from(r.max_sessions));
        if let Some(min) = r.end_disconnected_after_min {
            line(&mut out, "end_disconnected_after", i64::from(min));
        }
        out
    }
}

fn section<'a>(
    table: &'a toml::Table,
    name: &str,
    warnings: &mut Vec<String>,
) -> Option<&'a toml::Table> {
    match table.get(name)? {
        toml::Value::Table(t) => Some(t),
        _ => {
            warnings.push(format!("{name}: expected a table"));
            None
        }
    }
}

/// A TOML parse error's heading and message, without its source snippet.
fn one_line(error: &str) -> String {
    let snippet = |l: &str| {
        let l = l
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .trim_start();
        l.is_empty() || l.starts_with('|')
    };
    let lines: Vec<_> = error.lines().filter(|l| !snippet(l)).collect();
    lines.join(": ")
}

fn line(out: &mut String, key: &str, value: impl Into<toml::Value>) {
    out.push_str(&format!("{key} = {}\n", value.into()));
}

/// `value` as an integer in `range`.
fn int<T: TryFrom<i64>>(value: &toml::Value, range: RangeInclusive<i64>) -> Result<T, String> {
    let toml::Value::Integer(n) = *value else {
        return Err("expected a number".into());
    };
    if !range.contains(&n) {
        return Err(format!(
            "{n} is not between {} and {}",
            range.start(),
            range.end()
        ));
    }
    T::try_from(n).map_err(|_| format!("{n} is out of range"))
}

fn boolean(value: &toml::Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| "expected true or false".into())
}

fn string(value: &toml::Value) -> Result<&str, String> {
    value.as_str().ok_or_else(|| "expected a string".into())
}

impl LoginConfig {
    fn set(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "interface" => {
                let s = string(value)?;
                let i = Interface::parse(s)
                    .ok_or_else(|| format!("\"{s}\" is not classic, blue or glass"))?;
                self.interface = Some(i);
            }
            "show_users" => self.show_users = boolean(value)?,
            "greeter_socket" => self.greeter_socket = boolean(value)?,
            "autologin_user" => {
                let s = string(value)?;
                self.autologin_user = (!s.is_empty()).then(|| s.to_owned());
            }
            _ => return Err("unknown key".into()),
        }
        Ok(())
    }
}

impl MachineRemote {
    fn set(&mut self, key: &str, value: &toml::Value) -> Result<(), String> {
        match key {
            "enabled" => self.enabled = boolean(value)?,
            "port" => self.port = int(value, 1..=65535)?,
            "group" => {
                let s = string(value)?;
                if s.is_empty() {
                    return Err("expected a group name".into());
                }
                self.group = s.to_owned();
            }
            "max_sessions" => self.max_sessions = int(value, 1..=50)?,
            "end_disconnected_after" => {
                let min: u32 = int(value, 0..=10080)?;
                self.end_disconnected_after_min = (min > 0).then_some(min);
            }
            _ => return Err("unknown key".into()),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_login_section() {
        let (config, warnings) = MachineConfig::parse(
            "[login]\ninterface = \"glass\"\nshow_users = false\nautologin_user = \"anna\"\n\
             greeter_socket = true\n",
        );
        assert_eq!(
            config.login,
            LoginConfig {
                interface: Some(Interface::Glass),
                show_users: false,
                autologin_user: Some("anna".into()),
                greeter_socket: true,
            }
        );
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn a_parse_error_is_one_line() {
        let (_, warnings) = MachineConfig::parse("[login]\nshow_users = \n");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        let line = &warnings[0];
        assert!(!line.contains('\n'), "{line}");
        assert!(line.starts_with("TOML parse error at line 2"), "{line}");
        assert!(!line.contains('|'), "{line}");
    }

    #[test]
    fn a_bad_interface_warns_and_falls_back() {
        let (config, warnings) =
            MachineConfig::parse("[login]\ninterface = \"aqua\"\nshow_users = false\n");
        assert_eq!(config.login.interface, None);
        assert!(!config.login.show_users, "the other keys still apply");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("aqua"), "{warnings:?}");
    }

    #[test]
    fn other_tables_are_ignored_silently() {
        let (config, warnings) = MachineConfig::parse("[appearance]\ninterface = \"glass\"\n");
        assert_eq!(config, MachineConfig::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn parses_every_key_and_defaults_the_rest() {
        let (config, warnings) = MachineConfig::parse(
            "[remote_desktop]\nenabled = true\nport = 3390\ngroup = \"rdp\"\nmax_sessions = 3\n\
             end_disconnected_after = 60\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            config.remote,
            MachineRemote {
                enabled: true,
                port: 3390,
                group: "rdp".into(),
                max_sessions: 3,
                end_disconnected_after_min: Some(60),
            }
        );
        assert_eq!(config.login, LoginConfig::default());

        let (config, warnings) = MachineConfig::parse("");
        assert!(warnings.is_empty(), "{warnings:?}");
        let r = config.remote;
        assert_eq!(
            (r.enabled, r.port, r.group.as_str(), r.max_sessions),
            (false, 3389, "reprise-remote", 8)
        );
        assert_eq!(r.end_disconnected_after_min, None);
    }

    #[test]
    fn out_of_range_values_warn_and_keep_the_default() {
        let (config, warnings) = MachineConfig::parse(
            "[remote_desktop]\nport = 0\nmax_sessions = 0\nend_disconnected_after = -5\n",
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(config.remote, MachineRemote::default());

        let (config, warnings) = MachineConfig::parse(
            "[remote_desktop]\nport = 65536\nmax_sessions = 51\nend_disconnected_after = 10081\n",
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(config.remote, MachineRemote::default());

        let (config, warnings) = MachineConfig::parse(
            "[remote_desktop]\nport = 65535\nmax_sessions = 50\nend_disconnected_after = 10080\n",
        );
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            (config.remote.port, config.remote.max_sessions),
            (65535, 50)
        );
        assert_eq!(config.remote.end_disconnected_after_min, Some(10080));
    }

    #[test]
    fn zero_minutes_never_ends_a_disconnected_session() {
        let (config, warnings) =
            MachineConfig::parse("[remote_desktop]\nend_disconnected_after = 0\n");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(config.remote.end_disconnected_after_min, None);
    }

    #[test]
    fn wrong_types_and_unknown_keys_warn() {
        let (config, warnings) = MachineConfig::parse(
            "[remote_desktop]\nenabled = \"yes\"\nport = \"3390\"\ngroup = \"\"\nmode = 1\n",
        );
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert_eq!(config.remote, MachineRemote::default());
        let (_, warnings) = MachineConfig::parse("remote_desktop = 1\n");
        assert_eq!(warnings, ["remote_desktop: expected a table"]);
    }

    #[test]
    fn to_toml_round_trips() {
        let c = MachineConfig {
            login: LoginConfig {
                interface: Some(Interface::Blue),
                show_users: false,
                autologin_user: Some("anna \"the admin\"".into()),
                greeter_socket: true,
            },
            remote: MachineRemote {
                enabled: true,
                port: 3390,
                group: "rdp users".into(),
                max_sessions: 2,
                end_disconnected_after_min: Some(90),
            },
        };
        assert_eq!(MachineConfig::parse(&c.to_toml()), (c, vec![]));
        let d = MachineConfig::default();
        assert_eq!(MachineConfig::parse(&d.to_toml()), (d, vec![]));
    }

    #[test]
    fn to_toml_writes_login_then_remote_desktop_without_unset_values() {
        let text = MachineConfig::default().to_toml();
        let login = text.find("[login]").expect("a [login] table");
        let remote = text
            .find("[remote_desktop]")
            .expect("a [remote_desktop] table");
        assert!(login < remote, "{text}");
        for unset in ["interface", "autologin_user", "end_disconnected_after"] {
            assert!(!text.contains(unset), "{text}");
        }
    }
}
