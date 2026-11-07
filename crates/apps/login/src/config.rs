use std::path::Path;

use repriselogin::Interface;

pub const CONFIG_PATH: &str = "/etc/reprise/login.toml";

#[derive(Clone, Debug, PartialEq)]
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

impl LoginConfig {
    /// Reads `[login]`, returning a warning for each value it could not use.
    pub fn parse(text: &str) -> (LoginConfig, Vec<String>) {
        let mut config = LoginConfig::default();
        let mut warnings = Vec::new();
        let table = match text.parse::<toml::Table>() {
            Ok(table) => table,
            Err(e) => {
                warnings.push(e.to_string());
                return (config, warnings);
            }
        };
        let login = match table.get("login") {
            None => return (config, warnings),
            Some(toml::Value::Table(login)) => login,
            Some(_) => {
                warnings.push("login: expected a table".into());
                return (config, warnings);
            }
        };
        for (key, value) in login {
            let warning = match (key.as_str(), value) {
                ("interface", toml::Value::String(s)) => match Interface::parse(s) {
                    Some(i) => {
                        config.interface = Some(i);
                        continue;
                    }
                    None => format!("login.interface: \"{s}\" is not classic, blue or glass"),
                },
                ("show_users", toml::Value::Boolean(b)) => {
                    config.show_users = *b;
                    continue;
                }
                ("greeter_socket", toml::Value::Boolean(b)) => {
                    config.greeter_socket = *b;
                    continue;
                }
                ("autologin_user", toml::Value::String(s)) => {
                    config.autologin_user = (!s.is_empty()).then(|| s.clone());
                    continue;
                }
                ("interface" | "autologin_user", _) => format!("login.{key}: expected a string"),
                ("show_users" | "greeter_socket", _) => {
                    format!("login.{key}: expected true or false")
                }
                _ => format!("login.{key}: unknown key"),
            };
            warnings.push(warning);
        }
        (config, warnings)
    }

    /// Reads the file at `path`, logging each warning. A missing file gives the defaults.
    pub fn load(path: &Path) -> LoginConfig {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LoginConfig::default(),
            Err(e) => {
                log::warn!("{}: {e}", path.display());
                return LoginConfig::default();
            }
        };
        let (config, warnings) = LoginConfig::parse(&text);
        for w in warnings {
            log::warn!("{}: {w}", path.display());
        }
        config
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_login_section() {
        let (config, warnings) = LoginConfig::parse(
            "[login]\ninterface = \"glass\"\nshow_users = false\nautologin_user = \"anna\"\n\
             greeter_socket = true\n",
        );
        assert_eq!(
            config,
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
    fn a_missing_file_gives_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let config = LoginConfig::load(&dir.path().join("login.toml"));
        assert_eq!(config, LoginConfig::default());
        assert!(config.show_users);
        assert!(!config.greeter_socket, "the login screen has no socket");
    }

    #[test]
    fn a_bad_interface_warns_and_falls_back() {
        let (config, warnings) =
            LoginConfig::parse("[login]\ninterface = \"aqua\"\nshow_users = false\n");
        assert_eq!(config.interface, None);
        assert!(!config.show_users, "the other keys still apply");
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("aqua"), "{warnings:?}");
    }

    #[test]
    fn other_tables_are_ignored_silently() {
        let (config, warnings) = LoginConfig::parse("[remote_desktop]\nenabled = true\n");
        assert_eq!(config, LoginConfig::default());
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
