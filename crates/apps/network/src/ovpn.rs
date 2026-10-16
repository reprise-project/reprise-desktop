//! An OpenVPN connection as NetworkManager's openvpn plugin keeps it: settings in `vpn.data`,
//! the password and the private key's password in `vpn.secrets`.

use crate::nm::Psk;

/// `connection-type`: how the client proves who it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Auth {
    #[default]
    Tls,
    Password,
    PasswordTls,
    StaticKey,
}

impl Auth {
    pub const ALL: [Auth; 4] = [
        Auth::Tls,
        Auth::Password,
        Auth::PasswordTls,
        Auth::StaticKey,
    ];

    pub fn key(self) -> &'static str {
        match self {
            Auth::Tls => "tls",
            Auth::Password => "password",
            Auth::PasswordTls => "password-tls",
            Auth::StaticKey => "static-key",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Auth::Tls => "Certificates (TLS)",
            Auth::Password => "Password",
            Auth::PasswordTls => "Password with certificates (TLS)",
            Auth::StaticKey => "Static key",
        }
    }

    /// The plugin reads a missing or unknown type as TLS.
    fn from_key(key: &str) -> Auth {
        Auth::ALL
            .into_iter()
            .find(|a| a.key() == key)
            .unwrap_or_default()
    }

    pub fn index(self) -> usize {
        Auth::ALL.iter().position(|&a| a == self).unwrap_or(0)
    }

    pub fn user(self) -> bool {
        matches!(self, Auth::Password | Auth::PasswordTls)
    }

    pub fn ca(self) -> bool {
        self != Auth::StaticKey
    }

    pub fn certs(self) -> bool {
        matches!(self, Auth::Tls | Auth::PasswordTls)
    }
}

/// `static-key-direction`: none, 0 or 1.
pub const DIRECTIONS: [&str; 3] = ["None", "0", "1"];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub name: String,
    pub gateway: String,
    pub auth: Auth,
    pub user: String,
    /// A new password; empty keeps the saved one.
    pub password: Psk,
    pub ca: String,
    pub cert: String,
    pub key: String,
    /// A new password for the private key; empty keeps the saved one.
    pub cert_pass: Psk,
    pub static_key: String,
    /// An index into [`DIRECTIONS`].
    pub direction: usize,
    pub local_ip: String,
    pub remote_ip: String,
    /// Every other `vpn.data` key, kept as loaded.
    pub rest: Vec<(String, String)>,
}

const FORM_KEYS: [&str; 10] = [
    "connection-type",
    "remote",
    "username",
    "ca",
    "cert",
    "key",
    "static-key",
    "static-key-direction",
    "local-ip",
    "remote-ip",
];

pub fn from_data(name: &str, data: &[(String, String)]) -> Config {
    let get = |k: &str| {
        data.iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    Config {
        name: name.to_string(),
        gateway: get("remote"),
        auth: Auth::from_key(&get("connection-type")),
        user: get("username"),
        ca: get("ca"),
        cert: get("cert"),
        key: get("key"),
        static_key: get("static-key"),
        direction: DIRECTIONS
            .iter()
            .skip(1)
            .position(|d| *d == get("static-key-direction"))
            .map_or(0, |i| i + 1),
        local_ip: get("local-ip"),
        remote_ip: get("remote-ip"),
        rest: data
            .iter()
            .filter(|(k, _)| !FORM_KEYS.contains(&k.as_str()))
            .cloned()
            .collect(),
        ..Config::default()
    }
}

/// `vpn.data` for `cfg`: the kept keys, the form's non-empty fields, and a typed secret's flags
/// set to 0 so NetworkManager stores it; with no secret agent, a stored secret is the only one
/// it can connect with.
pub fn to_data(cfg: &Config) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = cfg
        .rest
        .iter()
        .filter(|(k, _)| {
            !(k == "password-flags" && !cfg.password.is_empty()
                || k == "cert-pass-flags" && !cfg.cert_pass.is_empty())
        })
        .cloned()
        .collect();
    let direction = DIRECTIONS[cfg.direction.min(DIRECTIONS.len() - 1)];
    let fields = [
        ("connection-type", cfg.auth.key()),
        ("remote", cfg.gateway.trim()),
        ("username", cfg.user.trim()),
        ("ca", cfg.ca.trim()),
        ("cert", cfg.cert.trim()),
        ("key", cfg.key.trim()),
        ("static-key", cfg.static_key.trim()),
        (
            "static-key-direction",
            if cfg.direction == 0 { "" } else { direction },
        ),
        ("local-ip", cfg.local_ip.trim()),
        ("remote-ip", cfg.remote_ip.trim()),
    ];
    for (k, v) in fields {
        if !v.is_empty() {
            out.push((k.into(), v.into()));
        }
    }
    if !cfg.password.is_empty() {
        out.push(("password-flags".into(), "0".into()));
    }
    if !cfg.cert_pass.is_empty() {
        out.push(("cert-pass-flags".into(), "0".into()));
    }
    out.sort();
    out
}

/// The secrets typed into the sheet, by their `vpn.secrets` names.
pub fn secrets(cfg: &Config) -> Vec<(String, Psk)> {
    [("password", &cfg.password), ("cert-pass", &cfg.cert_pass)]
        .into_iter()
        .filter(|(_, s)| !s.is_empty())
        .map(|(k, s)| (k.to_string(), s.clone()))
        .collect()
}

/// Secret flags bit 0 (agent-owned) and bit 1 (not saved): NetworkManager asks a secret agent
/// for the secret at connect time, and this desktop runs none.
const FLAGS_ASKED: u32 = 0x1 | 0x2;

impl Config {
    /// Whether the profile leaves the secret whose flags are `flags_key` to a secret agent, so
    /// NetworkManager has none to connect with until one is typed here.
    pub fn asked(&self, flags_key: &str) -> bool {
        self.rest
            .iter()
            .find(|(k, _)| k == flags_key)
            .and_then(|(_, v)| v.trim().parse::<u32>().ok())
            .is_some_and(|f| f & FLAGS_ASKED != 0)
    }
}

/// The first field the plugin cannot connect without, as its form id and what to say.
pub fn check(cfg: &Config) -> Option<(&'static str, &'static str)> {
    let a = cfg.auth;
    let missing = [
        (
            true,
            cfg.name.as_str(),
            "name",
            "Type a name for the connection.",
        ),
        (
            true,
            cfg.gateway.as_str(),
            "gateway",
            "Type the gateway's address.",
        ),
        (a.user(), cfg.user.as_str(), "user", "Type the user name."),
        (
            a.user() && cfg.asked("password-flags"),
            &*cfg.password,
            "password",
            "Type the password. This connection does not save it.",
        ),
        (
            a.ca(),
            cfg.ca.as_str(),
            "ca",
            "Choose the CA certificate file.",
        ),
        (
            a.certs(),
            cfg.cert.as_str(),
            "cert",
            "Choose your certificate file.",
        ),
        (
            a.certs(),
            cfg.key.as_str(),
            "key",
            "Choose your private key file.",
        ),
        (
            a.certs() && cfg.asked("cert-pass-flags"),
            &*cfg.cert_pass,
            "keypass",
            "Type the private key's password. This connection does not save it.",
        ),
        (
            a == Auth::StaticKey,
            cfg.static_key.as_str(),
            "static",
            "Choose the static key file.",
        ),
    ];
    missing
        .into_iter()
        .find(|(needed, v, _, _)| *needed && v.trim().is_empty())
        .map(|(_, _, id, text)| (id, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn the_plugins_keys_load_into_fields_and_the_rest_rides_along() {
        let data = pairs(&[
            ("ca", "/etc/vpn/ca.crt"),
            ("connection-type", "password-tls"),
            ("dev", "tun"),
            ("password-flags", "1"),
            ("remote", "vpn.example.org:1194"),
            ("username", "u"),
        ]);
        let cfg = from_data("Work", &data);
        assert_eq!(cfg.auth, Auth::PasswordTls);
        assert_eq!(cfg.gateway, "vpn.example.org:1194");
        assert_eq!(cfg.ca, "/etc/vpn/ca.crt");
        assert_eq!(cfg.rest, pairs(&[("dev", "tun"), ("password-flags", "1")]));
        assert_eq!(
            to_data(&cfg),
            data,
            "an untouched form saves what it loaded"
        );
    }

    #[test]
    fn a_typed_password_is_a_secret_with_its_flags_set_to_stored() {
        let mut cfg = from_data(
            "Work",
            &pairs(&[("connection-type", "password"), ("password-flags", "1")]),
        );
        cfg.password = Psk::new("hunter2");
        assert_eq!(
            to_data(&cfg),
            pairs(&[("connection-type", "password"), ("password-flags", "0")])
        );
        assert_eq!(
            secrets(&cfg),
            vec![("password".to_string(), Psk::new("hunter2"))]
        );
    }

    #[test]
    fn an_unknown_type_reads_as_tls_and_a_cleared_field_drops_its_key() {
        let mut cfg = from_data(
            "Work",
            &pairs(&[("connection-type", "odd"), ("cert", "/a.crt")]),
        );
        assert_eq!(cfg.auth, Auth::Tls);
        cfg.cert.clear();
        assert_eq!(to_data(&cfg), pairs(&[("connection-type", "tls")]));
    }

    #[test]
    fn the_static_key_direction_round_trips() {
        let data = pairs(&[
            ("connection-type", "static-key"),
            ("static-key", "/k"),
            ("static-key-direction", "1"),
        ]);
        let cfg = from_data("S", &data);
        assert_eq!(cfg.direction, 2);
        assert_eq!(to_data(&cfg), data);
    }

    #[test]
    fn check_asks_for_a_password_the_profile_leaves_to_an_agent() {
        for flags in ["1", "2", "3"] {
            let mut cfg = from_data(
                "Work",
                &pairs(&[
                    ("connection-type", "password"),
                    ("remote", "gw"),
                    ("ca", "/ca"),
                    ("username", "u"),
                    ("password-flags", flags),
                ]),
            );
            assert_eq!(check(&cfg).map(|p| p.0), Some("password"), "flags {flags}");
            cfg.password = Psk::new("hunter2");
            assert_eq!(check(&cfg), None, "flags {flags} with a typed password");
        }
        for flags in ["0", "4"] {
            let cfg = from_data(
                "Work",
                &pairs(&[
                    ("connection-type", "password"),
                    ("remote", "gw"),
                    ("ca", "/ca"),
                    ("username", "u"),
                    ("password-flags", flags),
                ]),
            );
            assert_eq!(check(&cfg), None, "flags {flags}");
        }
    }

    #[test]
    fn check_asks_for_a_key_password_only_when_the_type_uses_the_key() {
        let mut cfg = from_data(
            "Work",
            &pairs(&[
                ("connection-type", "tls"),
                ("remote", "gw"),
                ("ca", "/ca"),
                ("cert", "/c"),
                ("key", "/k"),
                ("cert-pass-flags", "1"),
            ]),
        );
        assert_eq!(check(&cfg).map(|p| p.0), Some("keypass"));
        cfg.auth = Auth::Password;
        cfg.user = "u".into();
        assert_eq!(check(&cfg), None);
    }

    #[test]
    fn check_names_the_first_missing_field_the_type_needs() {
        let mut cfg = from_data("Work", &pairs(&[("remote", "gw")]));
        assert_eq!(check(&cfg).map(|p| p.0), Some("ca"));
        cfg.auth = Auth::Password;
        assert_eq!(check(&cfg).map(|p| p.0), Some("user"));
        cfg.auth = Auth::StaticKey;
        assert_eq!(check(&cfg).map(|p| p.0), Some("static"));
        cfg.static_key = "/k".into();
        assert_eq!(check(&cfg), None);
    }
}
