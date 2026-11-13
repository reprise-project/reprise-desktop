//! The session proxy: `environment.d` variables the systemd user manager reads at login. Settings
//! writes the file and the compositor hands its variables to every program it starts.

use std::io;
use std::path::{Path, PathBuf};

pub const FILE: &str = "60-reprise-proxy.conf";

/// Every variable the file may set; a change unsets the ones it stopped setting.
pub const NAMES: [&str; 11] = [
    "http_proxy",
    "HTTP_PROXY",
    "https_proxy",
    "HTTPS_PROXY",
    "ftp_proxy",
    "FTP_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
    "auto_proxy",
];

/// What the ignore list holds before one was written.
pub const DEFAULT_IGNORE: &str = "localhost, 127.0.0.1, ::1";

pub fn path() -> PathBuf {
    crate::autostart::config_home()
        .join("environment.d")
        .join(FILE)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    None,
    Manual,
    /// A proxy auto-config script, written as `auto_proxy`.
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Http,
    Https,
    Ftp,
    /// Written as `all_proxy`: what the other three leave unset goes through it.
    Socks,
}

impl Protocol {
    pub const ALL: [Protocol; 4] = [
        Protocol::Http,
        Protocol::Https,
        Protocol::Ftp,
        Protocol::Socks,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Protocol::Http => "HTTP",
            Protocol::Https => "HTTPS",
            Protocol::Ftp => "FTP",
            Protocol::Socks => "SOCKS",
        }
    }

    /// The lowercase and uppercase names.
    fn names(self) -> [&'static str; 2] {
        let i = 2 * self as usize;
        [NAMES[i], NAMES[i + 1]]
    }

    /// The scheme a host typed without one gets.
    fn scheme(self) -> &'static str {
        match self {
            Protocol::Socks => "socks5://",
            _ => "http://",
        }
    }
}

/// A server as typed: `host` may carry a scheme and credentials, `port` is digits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proxy {
    pub mode: Mode,
    /// In [`Protocol::ALL`] order.
    pub servers: [Server; 4],
    pub ignore: String,
    pub pac: String,
}

impl Default for Proxy {
    fn default() -> Proxy {
        Proxy {
            mode: Mode::None,
            servers: Default::default(),
            ignore: DEFAULT_IGNORE.into(),
            pac: String::new(),
        }
    }
}

/// Characters `environment.d` would expand or a shell-minded reader would split on.
fn unsafe_char(c: char) -> bool {
    c.is_whitespace() || c.is_control() || matches!(c, '$' | '\\' | '"' | '\'' | '`')
}

fn ignore_entries(ignore: &str) -> impl Iterator<Item = &str> {
    ignore
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|e| !e.is_empty())
}

/// `host` without its scheme when it is `scheme`, a trailing slash, and the port off its end.
fn split(value: &str, scheme: &str) -> Server {
    let rest = value.strip_prefix(scheme).unwrap_or(value);
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    match rest.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit())
                && !host.ends_with(':') =>
        {
            Server {
                host: host.into(),
                port: port.into(),
            }
        }
        _ => Server {
            host: rest.into(),
            port: String::new(),
        },
    }
}

impl Proxy {
    /// The variables the file sets, or what to tell the user is wrong.
    pub fn vars(&self) -> Result<Vec<(&'static str, String)>, String> {
        let mut out = Vec::new();
        match self.mode {
            Mode::None => {}
            Mode::Manual => {
                for (p, s) in Protocol::ALL.into_iter().zip(&self.servers) {
                    if let Some(url) = server_url(p, s)? {
                        for n in p.names() {
                            out.push((n, url.clone()));
                        }
                    }
                }
                if out.is_empty() {
                    return Err("Type the address of at least one proxy server.".into());
                }
                if ignore_entries(&self.ignore).any(|e| e.chars().any(unsafe_char)) {
                    return Err("The ignored hosts can't hold quotes, $ or \\.".into());
                }
                let ignore: Vec<&str> = ignore_entries(&self.ignore).collect();
                if !ignore.is_empty() {
                    for n in ["no_proxy", "NO_PROXY"] {
                        out.push((n, ignore.join(",")));
                    }
                }
            }
            Mode::Auto => {
                let pac = self.pac.trim();
                if pac.is_empty() {
                    return Err("Type the address of the configuration script.".into());
                }
                if pac.chars().any(unsafe_char) {
                    return Err(
                        "The configuration script's address can't hold spaces, quotes, $ or \\."
                            .into(),
                    );
                }
                if !pac.contains("://") {
                    return Err("The configuration script's address starts with http://, https:// or file://.".into());
                }
                out.push(("auto_proxy", pac.into()));
            }
        }
        Ok(out)
    }

    /// What `vars` (as [`read`] gives them) stand for; fields they leave out keep their defaults.
    pub fn from_vars(vars: &[(String, String)]) -> Proxy {
        let get = |name: &str| {
            vars.iter()
                .find(|(k, v)| k == name && !v.is_empty())
                .map(|(_, v)| v.as_str())
        };
        let mut proxy = Proxy::default();
        if let Some(pac) = get("auto_proxy") {
            proxy.mode = Mode::Auto;
            proxy.pac = pac.into();
            return proxy;
        }
        for (p, s) in Protocol::ALL.into_iter().zip(&mut proxy.servers) {
            if let Some(v) = p.names().into_iter().find_map(get) {
                *s = split(v, p.scheme());
                proxy.mode = Mode::Manual;
            }
        }
        if proxy.mode == Mode::Manual {
            proxy.ignore = ["no_proxy", "NO_PROXY"]
                .into_iter()
                .find_map(get)
                .map(|v| ignore_entries(v).collect::<Vec<_>>().join(", "))
                .unwrap_or_default();
        }
        proxy
    }
}

fn server_url(p: Protocol, s: &Server) -> Result<Option<String>, String> {
    let host = s.host.trim();
    let port = s.port.trim();
    if host.is_empty() {
        return Ok(None);
    }
    let label = p.label();
    if host.chars().any(unsafe_char) {
        return Err(format!(
            "The {label} proxy's address can't hold spaces, quotes, $ or \\."
        ));
    }
    // A host typed with its port ("proxy:3128") and an empty Port field is taken as typed.
    let typed = split(host, p.scheme());
    let (host, port) = match port.is_empty() {
        true if !typed.port.is_empty() => (typed.host, typed.port),
        _ => (host.to_string(), port.to_string()),
    };
    if !port.parse::<u16>().is_ok_and(|n| n > 0) {
        return Err(format!(
            "Type the {label} proxy's port, a number from 1 to 65535."
        ));
    }
    let scheme = if host.contains("://") { "" } else { p.scheme() };
    Ok(Some(format!("{scheme}{host}:{port}")))
}

/// `KEY=VALUE` lines of the file's own variables ([`NAMES`]); comments and others are skipped.
pub fn parse(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .filter(|(k, _)| NAMES.contains(&k.trim()))
        .map(|(k, v)| (k.trim().into(), v.trim().into()))
        .collect()
}

/// The file's variables; none when it is missing or unreadable.
pub fn read(path: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(path)
        .map(|t| parse(&t))
        .unwrap_or_default()
}

pub fn text(vars: &[(&str, String)]) -> String {
    let mut out = String::from("# Reprise Settings > Network writes this file.\n");
    for (k, v) in vars {
        out.push_str(&format!("{k}={v}\n"));
    }
    out
}

/// Write `vars` to `path`, or remove the file when there are none.
pub fn write(path: &Path, vars: &[(&str, String)]) -> io::Result<()> {
    if vars.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            r => r,
        };
    }
    crate::ops::write_atomic(path, text(vars).as_bytes())
}

/// The `systemctl --user` argument lists that take the user manager from `old` (the file as it
/// was) to `new`: set what `new` sets, unset what only `old` set.
pub fn manager_args(old: &[(String, String)], new: &[(&str, String)]) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    if !new.is_empty() {
        let mut set = vec!["set-environment".to_string()];
        set.extend(new.iter().map(|(k, v)| format!("{k}={v}")));
        out.push(set);
    }
    let gone: Vec<String> = NAMES
        .into_iter()
        .filter(|n| old.iter().any(|(k, _)| k == n) && !new.iter().any(|(k, _)| k == n))
        .map(String::from)
        .collect();
    if !gone.is_empty() {
        let mut unset = vec!["unset-environment".to_string()];
        unset.extend(gone);
        out.push(unset);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manual(servers: [(&str, &str); 4], ignore: &str) -> Proxy {
        Proxy {
            mode: Mode::Manual,
            servers: servers.map(|(h, p)| Server {
                host: h.into(),
                port: p.into(),
            }),
            ignore: ignore.into(),
            pac: String::new(),
        }
    }

    fn owned(vars: &[(&str, String)]) -> Vec<(String, String)> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn manual_servers_set_both_cases_and_the_ignore_list() {
        let p = manual(
            [
                ("proxy.lan", "3128"),
                ("https://secure.lan", "443"),
                ("", ""),
                ("socks.lan", "1080"),
            ],
            "localhost,  127.0.0.1 ,::1",
        );
        let vars = p.vars().unwrap();
        let want = [
            ("http_proxy", "http://proxy.lan:3128"),
            ("HTTP_PROXY", "http://proxy.lan:3128"),
            ("https_proxy", "https://secure.lan:443"),
            ("HTTPS_PROXY", "https://secure.lan:443"),
            ("all_proxy", "socks5://socks.lan:1080"),
            ("ALL_PROXY", "socks5://socks.lan:1080"),
            ("no_proxy", "localhost,127.0.0.1,::1"),
            ("NO_PROXY", "localhost,127.0.0.1,::1"),
        ];
        assert_eq!(
            vars,
            want.map(|(k, v)| (k, v.to_string())).to_vec(),
            "an unset FTP proxy writes nothing"
        );
    }

    #[test]
    fn a_written_proxy_reads_back_as_typed() {
        let p = manual(
            [
                ("user@proxy.lan", "3128"),
                ("https://secure.lan", "443"),
                ("[fd00::1]", "21"),
                ("socks.lan", "1080"),
            ],
            "localhost, .corp",
        );
        let file = parse(&text(&p.vars().unwrap()));
        assert_eq!(Proxy::from_vars(&file), p);
        let auto = Proxy {
            mode: Mode::Auto,
            pac: "http://wpad.lan/wpad.dat".into(),
            ..Proxy::default()
        };
        assert_eq!(Proxy::from_vars(&parse(&text(&auto.vars().unwrap()))), auto);
        assert_eq!(Proxy::from_vars(&[]), Proxy::default(), "no file: no proxy");
    }

    #[test]
    fn a_port_typed_into_the_host_field_counts_when_port_is_empty() {
        let p = manual([("proxy.lan:8080", ""), ("", ""), ("", ""), ("", "")], "");
        assert_eq!(
            p.vars().unwrap(),
            vec![
                ("http_proxy", "http://proxy.lan:8080".to_string()),
                ("HTTP_PROXY", "http://proxy.lan:8080".to_string()),
            ],
            "an empty ignore list writes no no_proxy"
        );
    }

    #[test]
    fn wrong_fields_say_what_is_wrong() {
        let err = |servers, ignore| manual(servers, ignore).vars().unwrap_err();
        let none = ("", "");
        assert_eq!(
            err([none; 4], ""),
            "Type the address of at least one proxy server."
        );
        assert_eq!(
            err([none, ("secure.lan", ""), none, none], ""),
            "Type the HTTPS proxy's port, a number from 1 to 65535."
        );
        assert_eq!(
            err([("proxy.lan", "70000"), none, none, none], ""),
            "Type the HTTP proxy's port, a number from 1 to 65535."
        );
        assert_eq!(
            err([none, none, none, ("$HOST", "1080")], ""),
            "The SOCKS proxy's address can't hold spaces, quotes, $ or \\."
        );
        assert_eq!(
            err([("proxy.lan", "3128"), none, none, none], "a,\"b\""),
            "The ignored hosts can't hold quotes, $ or \\."
        );
        let auto = |pac: &str| {
            Proxy {
                mode: Mode::Auto,
                pac: pac.into(),
                ..Proxy::default()
            }
            .vars()
        };
        assert!(auto("").is_err());
        assert!(auto("wpad.lan/wpad.dat").is_err(), "a scheme is required");
        assert_eq!(
            auto("file:///etc/proxy.pac").unwrap(),
            vec![("auto_proxy", "file:///etc/proxy.pac".to_string())]
        );
    }

    #[test]
    fn the_manager_sets_the_new_and_unsets_what_only_the_old_file_set() {
        let old = owned(
            &manual([("a", "1"), ("b", "2"), ("", ""), ("", "")], "x")
                .vars()
                .unwrap(),
        );
        let new = manual([("a", "1"), ("", ""), ("", ""), ("", "")], "")
            .vars()
            .unwrap();
        assert_eq!(
            manager_args(&old, &new),
            vec![
                vec![
                    "set-environment".to_string(),
                    "http_proxy=http://a:1".into(),
                    "HTTP_PROXY=http://a:1".into(),
                ],
                [
                    "unset-environment",
                    "https_proxy",
                    "HTTPS_PROXY",
                    "no_proxy",
                    "NO_PROXY"
                ]
                .map(String::from)
                .to_vec(),
            ]
        );
        assert_eq!(
            manager_args(&old, &[])[0][..2],
            ["unset-environment", "http_proxy"].map(String::from),
            "No proxy unsets every variable the file had"
        );
        assert!(manager_args(&[], &[]).is_empty());
    }

    #[test]
    fn write_removes_the_file_for_no_proxy_and_parse_skips_strangers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("environment.d").join(FILE);
        write(&path, &[("http_proxy", "http://a:1".into())]).unwrap();
        assert_eq!(
            read(&path),
            vec![("http_proxy".into(), "http://a:1".into())]
        );
        write(&path, &[]).unwrap();
        assert!(!path.exists());
        write(&path, &[]).unwrap();
        assert_eq!(
            parse("# c\nPATH=/bin\n no_proxy = a,b \n"),
            vec![("no_proxy".into(), "a,b".into())]
        );
    }
}
