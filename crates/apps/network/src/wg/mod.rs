//! A WireGuard connection as the editor holds it: the interface, its
//! peers, the wg-quick `.conf` reader behind Import..., the checks Save
//! runs, key generation, and the NetworkManager settings it becomes (and
//! is read back from). Nothing here touches the bus.

mod x25519;

use std::collections::HashMap;
use std::net::IpAddr;

use zbus::Variant;

use crate::nm::{Props, Psk};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Peer {
    pub public_key: String,
    pub preshared_key: Psk,
    /// `host:port`, or empty for a peer that only ever calls in.
    pub endpoint: String,
    pub allowed_ips: Vec<String>,
    /// Seconds; `None` sends no keepalives.
    pub keepalive: Option<u16>,
    /// NetworkManager holds a preshared key it did not give out; an empty
    /// `preshared_key` keeps it.
    pub stored_psk: bool,
}

/// One WireGuard connection: the `[Interface]` and every `[Peer]`, plus
/// the profile's name and interface name, which a `.conf` has no line for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    pub name: String,
    pub iface: String,
    pub private_key: Psk,
    /// `10.0.0.2/24`, `fd00::2/64`; a bare address is a host route.
    pub addresses: Vec<String>,
    /// Name servers, and search domains among them as wg-quick allows.
    pub dns: Vec<String>,
    pub listen_port: Option<u16>,
    pub mtu: Option<u32>,
    pub peers: Vec<Peer>,
    /// NetworkManager holds a private key it did not give out; an empty
    /// `private_key` keeps it.
    pub stored_key: bool,
}

/// What a save says about one secret: a new value, the stored one kept, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Secret<'a> {
    New(&'a str),
    Kept,
    None,
}

fn secret(value: &str, stored: bool) -> Secret<'_> {
    match value.trim() {
        "" if stored => Secret::Kept,
        "" => Secret::None,
        v => Secret::New(v),
    }
}

impl Config {
    fn private_secret(&self) -> Secret<'_> {
        secret(&self.private_key, self.stored_key)
    }
}

impl Peer {
    fn psk_secret(&self) -> Secret<'_> {
        secret(&self.preshared_key, self.stored_psk)
    }
}

pub fn join(items: &[String]) -> String {
    items.join(", ")
}

pub fn split(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(44);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A WireGuard key's 32 bytes from its 44-character base64, or None.
pub fn decode_key(s: &str) -> Option<[u8; 32]> {
    let s = s.trim().as_bytes();
    if s.len() != 44 || s[43] != b'=' || s[42] == b'=' {
        return None;
    }
    let mut bits = 0u32;
    let mut n = 0;
    let mut out = [0u8; 32];
    let mut i = 0;
    for &c in &s[..43] {
        let v = B64.iter().position(|&b| b == c)? as u32;
        bits = bits << 6 | v;
        n += 6;
        if n >= 8 {
            n -= 8;
            if i < 32 {
                out[i] = (bits >> n) as u8;
            }
            i += 1;
            bits &= (1 << n) - 1;
        }
    }
    // 43 characters carry 258 bits: the last two must be zero.
    (i == 32 && bits == 0).then_some(out)
}

/// A fresh private key, clamped as `wg genkey` does.
pub fn generate_key() -> Option<String> {
    let mut k = [0u8; 32];
    if let Err(e) = getrandom::fill(&mut k) {
        log::warn!("no random bytes for a WireGuard key: {e}");
        return None;
    }
    k[0] &= 248;
    k[31] = (k[31] & 127) | 64;
    Some(b64_encode(&k))
}

pub fn public_key(private: &str) -> Option<String> {
    decode_key(private).map(|k| b64_encode(&x25519::public_key(&k)))
}

/// A wg-quick `.conf` as a connection named `name`. wg-quick's own lines
/// (Table, PreUp, PostDown, SaveConfig, FwMark) have no place in the
/// profile and are skipped; anything else unknown is an error naming its
/// line.
pub fn parse_conf(text: &str, name: &str) -> Result<Config, String> {
    #[derive(PartialEq)]
    enum Section {
        None,
        Interface,
        Peer,
    }
    let mut cfg = Config {
        name: name.to_string(),
        iface: iface_for(name),
        ..Config::default()
    };
    let mut section = Section::None;
    let mut seen_interface = false;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        let at = |what: &str| format!("Line {}: {what}", n + 1);
        if line.starts_with('[') {
            section = match line.to_ascii_lowercase().as_str() {
                "[interface]" => {
                    seen_interface = true;
                    Section::Interface
                }
                "[peer]" => {
                    cfg.peers.push(Peer::default());
                    Section::Peer
                }
                _ => return Err(at(&format!("{line} is not a WireGuard section."))),
            };
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            return Err(at("expected Key = Variant."));
        };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let number = |what: &str| {
            value
                .parse::<u32>()
                .map_err(|_| at(&format!("{what} must be a number.")))
        };
        match (&section, key.as_str()) {
            (Section::Interface, "privatekey") => cfg.private_key = Psk::new(value),
            (Section::Interface, "address") => cfg.addresses.extend(split(value)),
            (Section::Interface, "dns") => cfg.dns.extend(split(value)),
            (Section::Interface, "listenport") => {
                cfg.listen_port = Some(
                    u16::try_from(number("ListenPort")?)
                        .map_err(|_| at("ListenPort must be a port number."))?,
                )
            }
            (Section::Interface, "mtu") => cfg.mtu = Some(number("MTU")?),
            (
                Section::Interface,
                "table" | "preup" | "postup" | "predown" | "postdown" | "saveconfig" | "fwmark",
            ) => {}
            (Section::Peer, _) => {
                let Some(peer) = cfg.peers.last_mut() else {
                    continue;
                };
                match key.as_str() {
                    "publickey" => peer.public_key = value.to_string(),
                    "presharedkey" => peer.preshared_key = Psk::new(value),
                    "endpoint" => peer.endpoint = value.to_string(),
                    "allowedips" => peer.allowed_ips.extend(split(value)),
                    "persistentkeepalive" if value.eq_ignore_ascii_case("off") => {}
                    "persistentkeepalive" => {
                        peer.keepalive = Some(
                            u16::try_from(number("PersistentKeepalive")?)
                                .map_err(|_| at("PersistentKeepalive is too large."))?,
                        )
                    }
                    _ => return Err(at(&format!("{key} is not a peer setting."))),
                }
            }
            (Section::None, _) => return Err(at("a setting before [Interface].")),
            (Section::Interface, _) => {
                return Err(at(&format!("{key} is not an interface setting.")));
            }
        }
    }
    if !seen_interface {
        return Err("The file has no [Interface] section.".into());
    }
    Ok(cfg)
}

/// An interface name for a connection called `name`: its letters, digits,
/// `-` and `_`, cut to the kernel's 15, or `wg0` when nothing is left.
pub fn iface_for(name: &str) -> String {
    let s: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(15)
        .collect();
    if s.is_empty() { "wg0".into() } else { s }
}

/// An address with an optional prefix length; a bare address takes the
/// whole width (/32, /128).
pub fn parse_cidr(s: &str) -> Option<(IpAddr, u8)> {
    let (ip, prefix) = match s.split_once('/') {
        Some((ip, p)) => (ip.parse::<IpAddr>().ok()?, Some(p.parse::<u8>().ok()?)),
        None => (s.parse::<IpAddr>().ok()?, None),
    };
    let max = if ip.is_ipv4() { 32 } else { 128 };
    match prefix {
        Some(p) if p > max => None,
        Some(p) => Some((ip, p)),
        None => Some((ip, max)),
    }
}

fn endpoint_ok(s: &str) -> bool {
    let Some((host, port)) = s.rsplit_once(':') else {
        return false;
    };
    let host_ok = match host.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
        Some(v6) => v6.parse::<std::net::Ipv6Addr>().is_ok(),
        None => !host.is_empty() && !host.contains(':') && !host.contains(char::is_whitespace),
    };
    host_ok && port.parse::<u16>().is_ok_and(|p| p != 0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Iface,
    PrivateKey,
    Addresses,
    Dns,
    PublicKey(usize),
    PresharedKey(usize),
    Endpoint(usize),
    AllowedIps(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub field: Field,
    pub text: String,
}

pub fn check(cfg: &Config) -> Option<Problem> {
    let bad = |field, text: &str| {
        Some(Problem {
            field,
            text: text.to_string(),
        })
    };
    if cfg.name.trim().is_empty() {
        return bad(Field::Name, "Type a name for the connection.");
    }
    let iface = cfg.iface.as_str();
    if iface.is_empty()
        || iface.len() > 15
        || iface == "."
        || iface == ".."
        || iface.contains(['/', ':'])
        || iface.contains(char::is_whitespace)
    {
        return bad(
            Field::Iface,
            "The interface name must be 1 to 15 characters, with no spaces, / or :.",
        );
    }
    if cfg.private_secret() != Secret::Kept && decode_key(&cfg.private_key).is_none() {
        return bad(
            Field::PrivateKey,
            "The private key must be a 44-character WireGuard key. Click Generate for a new one.",
        );
    }
    if let Some(a) = cfg.addresses.iter().find(|a| parse_cidr(a).is_none()) {
        return bad(
            Field::Addresses,
            &format!("{a} is not an address such as 10.0.0.2/24."),
        );
    }
    if let Some(d) = cfg.dns.iter().find(|d| d.contains(char::is_whitespace)) {
        return bad(Field::Dns, &format!("{d} is not a name server."));
    }
    // NetworkManager carries the stored secrets over only into an update
    // that holds none, so keeping one key while changing another loses it.
    let secrets: Vec<(Field, Secret)> = std::iter::once((Field::PrivateKey, cfg.private_secret()))
        .chain(
            cfg.peers
                .iter()
                .enumerate()
                .map(|(i, p)| (Field::PresharedKey(i), p.psk_secret())),
        )
        .collect();
    if let Some(&(field, _)) = secrets.iter().find(|(_, s)| *s == Secret::Kept)
        && secrets.iter().any(|(_, s)| matches!(s, Secret::New(_)))
    {
        return bad(
            field,
            "A saved key is kept only while every key is left as it is. Type this key \
             again too, or clear the keys you changed.",
        );
    }
    for (i, p) in cfg.peers.iter().enumerate() {
        let n = i + 1;
        if decode_key(&p.public_key).is_none() {
            return bad(
                Field::PublicKey(i),
                &format!("Peer {n}'s public key must be a 44-character WireGuard key."),
            );
        }
        if !p.preshared_key.is_empty() && decode_key(&p.preshared_key).is_none() {
            return bad(
                Field::PresharedKey(i),
                &format!(
                    "Peer {n}'s preshared key must be a 44-character WireGuard key, or empty."
                ),
            );
        }
        if !p.endpoint.is_empty() && !endpoint_ok(&p.endpoint) {
            return bad(
                Field::Endpoint(i),
                &format!("Peer {n}'s endpoint must be host:port, such as vpn.example.org:51820."),
            );
        }
        if let Some(a) = p.allowed_ips.iter().find(|a| parse_cidr(a).is_none()) {
            return bad(
                Field::AllowedIps(i),
                &format!("{a} is not a network such as 0.0.0.0/0."),
            );
        }
    }
    None
}

pub type Settings = HashMap<String, HashMap<String, Variant<'static>>>;

/// A secret flag: the secret is never saved, so there is nothing to keep.
const SECRET_NOT_SAVED: u32 = 0x2;
/// `preshared-key-flags` for a peer with no preshared key: not required,
/// so NetworkManager asks no secret agent for one. It is also the flags'
/// value when a peer's settings leave them out.
const SECRET_NOT_REQUIRED: u32 = 0x4;

/// `cfg` as NetworkManager's setting groups. With an `owner` the profile
/// is that login's own (`settings.modify.own`), else system-wide. Every
/// secret goes in with flags 0, so NetworkManager keeps it itself; a kept
/// one goes in as its flags alone.
pub fn settings(cfg: &Config, owner: Option<&str>) -> Settings {
    let mut connection: HashMap<String, Variant<'static>> = HashMap::from([
        ("id".to_string(), Variant::from(cfg.name.trim().to_string())),
        ("type".to_string(), Variant::from("wireguard")),
        (
            "interface-name".to_string(),
            Variant::from(cfg.iface.clone()),
        ),
        ("autoconnect".to_string(), Variant::from(false)),
    ]);
    if let Some(owner) = owner {
        connection.insert(
            "permissions".to_string(),
            Variant::from(vec![format!("user:{owner}")]),
        );
    }
    let peers: Vec<HashMap<String, Variant<'static>>> = cfg
        .peers
        .iter()
        .map(|p| {
            let mut m = HashMap::from([
                (
                    "public-key".to_string(),
                    Variant::from(p.public_key.trim().to_string()),
                ),
                (
                    "allowed-ips".to_string(),
                    Variant::from(p.allowed_ips.clone()),
                ),
            ]);
            if !p.endpoint.is_empty() {
                m.insert("endpoint".into(), Variant::from(p.endpoint.clone()));
            }
            if let Some(k) = p.keepalive {
                m.insert("persistent-keepalive".into(), Variant::from(u32::from(k)));
            }
            match p.psk_secret() {
                Secret::None => {
                    m.insert(
                        "preshared-key-flags".into(),
                        Variant::from(SECRET_NOT_REQUIRED),
                    );
                }
                Secret::Kept => {
                    m.insert("preshared-key-flags".into(), Variant::from(0u32));
                }
                Secret::New(k) => {
                    m.insert("preshared-key".into(), Variant::from(k.to_string()));
                    m.insert("preshared-key-flags".into(), Variant::from(0u32));
                }
            }
            m
        })
        .collect();
    let mut wireguard: HashMap<String, Variant<'static>> = HashMap::from([
        ("private-key-flags".to_string(), Variant::from(0u32)),
        ("peers".to_string(), Variant::from(peers)),
    ]);
    if let Secret::New(k) = cfg.private_secret() {
        wireguard.insert("private-key".into(), Variant::from(k.to_string()));
    }
    if let Some(port) = cfg.listen_port {
        wireguard.insert("listen-port".into(), Variant::from(u32::from(port)));
    }
    if let Some(mtu) = cfg.mtu {
        wireguard.insert("mtu".into(), Variant::from(mtu));
    }
    let (ipv4, ipv6) = ip_groups(cfg);
    HashMap::from([
        ("connection".to_string(), connection),
        ("wireguard".to_string(), wireguard),
        ("ipv4".to_string(), ipv4),
        ("ipv6".to_string(), ipv6),
    ])
}

/// The `ipv4` and `ipv6` groups: each family's addresses as `manual`, or
/// `disabled` with none; its name servers; the search domains on ipv4.
fn ip_groups(
    cfg: &Config,
) -> (
    HashMap<String, Variant<'static>>,
    HashMap<String, Variant<'static>>,
) {
    let (mut v4, mut v6) = (Vec::new(), Vec::new());
    for a in &cfg.addresses {
        if let Some((ip, prefix)) = parse_cidr(a) {
            let m = HashMap::from([
                ("address".to_string(), Variant::from(ip.to_string())),
                ("prefix".to_string(), Variant::from(u32::from(prefix))),
            ]);
            if ip.is_ipv4() {
                v4.push(m);
            } else {
                v6.push(m);
            }
        }
    }
    let (mut dns4, mut dns6, mut search) = (Vec::<u32>::new(), Vec::<Vec<u8>>::new(), Vec::new());
    for d in &cfg.dns {
        match d.parse::<IpAddr>() {
            // NetworkManager keeps an IPv4 server as a u32 in network
            // byte order: the octets as they sit in memory.
            Ok(IpAddr::V4(ip)) => dns4.push(u32::from_ne_bytes(ip.octets())),
            Ok(IpAddr::V6(ip)) => dns6.push(ip.octets().to_vec()),
            Err(_) => search.push(d.clone()),
        }
    }
    let group = |addrs: Vec<HashMap<String, Variant<'static>>>| {
        let mut g: HashMap<String, Variant<'static>> = HashMap::new();
        if addrs.is_empty() {
            g.insert("method".into(), Variant::from("disabled"));
        } else {
            g.insert("method".into(), Variant::from("manual"));
            g.insert("address-data".into(), Variant::from(addrs));
        }
        g
    };
    let mut ipv4 = group(v4);
    let mut ipv6 = group(v6);
    ipv4.insert("dns".into(), Variant::from(dns4));
    ipv4.insert("dns-search".into(), Variant::from(search));
    ipv6.insert("dns".into(), Variant::from(dns6));
    (ipv4, ipv6)
}

/// The keys [`settings`] owns in each group. An edit drops these from
/// the profile as it stands and puts the new ones in, so everything else
/// (the UUID, routes, another tool's settings) survives. `addresses` is
/// the old form of `address-data`, which NetworkManager would read in its
/// place if it were left in.
const OWNED: &[(&str, &[&str])] = &[
    (
        "connection",
        &["id", "interface-name", "autoconnect", "permissions"],
    ),
    (
        "wireguard",
        &[
            "private-key",
            "private-key-flags",
            "peers",
            "listen-port",
            "mtu",
        ],
    ),
    (
        "ipv4",
        &[
            "method",
            "addresses",
            "address-data",
            "dns",
            "dns-search",
            "gateway",
        ],
    ),
    (
        "ipv6",
        &["method", "addresses", "address-data", "dns", "gateway"],
    ),
];

/// The profile `current` (`GetSettings`) with `new`'s groups written over
/// it. `connection.autoconnect` and `permissions` keep their current
/// values: the editor shows neither.
pub fn merge(current: HashMap<String, Props>, mut new: Settings) -> Settings {
    let mut out = crate::nm::owned_to_values(current);
    if let Some(c) = new.get_mut("connection") {
        c.remove("autoconnect");
        c.remove("permissions");
    }
    for (group, keys) in OWNED {
        let Some(fresh) = new.remove(*group) else {
            continue;
        };
        let g = out.entry(group.to_string()).or_default();
        for k in *keys {
            if !matches!(*k, "autoconnect" | "permissions") {
                g.remove(*k);
            }
        }
        g.extend(fresh);
    }
    out
}

fn text(props: &Props, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
}

fn uint(props: &Props, key: &str) -> Option<u32> {
    props.get(key).and_then(|v| u32::try_from(v.clone()).ok())
}

fn dicts(props: &Props, key: &str) -> Vec<Props> {
    props
        .get(key)
        .and_then(|v| Vec::<Props>::try_from(v.clone()).ok())
        .unwrap_or_default()
}

fn strings(props: &Props, key: &str) -> Vec<String> {
    props
        .get(key)
        .and_then(|v| Vec::<String>::try_from(v.clone()).ok())
        .unwrap_or_default()
}

/// A saved WireGuard profile read back: `settings` is `GetSettings`,
/// `secrets` the `wireguard` group's `GetSecrets` when NetworkManager gave
/// them out. Without secrets the key fields come back empty, marked stored
/// where the flags say NetworkManager keeps one.
pub fn from_settings(
    settings: &HashMap<String, Props>,
    secrets: Option<&HashMap<String, Props>>,
) -> Config {
    let empty = Props::new();
    let group = |name: &str| settings.get(name).unwrap_or(&empty);
    let conn = group("connection");
    let wg = group("wireguard");
    let secret = secrets.and_then(|s| s.get("wireguard"));
    let secret_peers: HashMap<String, String> = secret
        .map(|s| {
            dicts(s, "peers")
                .iter()
                .map(|p| (text(p, "public-key"), text(p, "preshared-key")))
                .collect()
        })
        .unwrap_or_default();
    let peers = dicts(wg, "peers")
        .iter()
        .map(|p| {
            let public_key = text(p, "public-key");
            let flags = uint(p, "preshared-key-flags").unwrap_or(SECRET_NOT_REQUIRED);
            Peer {
                stored_psk: secrets.is_none()
                    && flags & (SECRET_NOT_SAVED | SECRET_NOT_REQUIRED) == 0,
                preshared_key: Psk::new(
                    secret_peers
                        .get(&public_key)
                        .map(String::as_str)
                        .unwrap_or_default(),
                ),
                public_key,
                endpoint: text(p, "endpoint"),
                allowed_ips: strings(p, "allowed-ips"),
                keepalive: uint(p, "persistent-keepalive")
                    .filter(|&k| k != 0)
                    .and_then(|k| u16::try_from(k).ok()),
            }
        })
        .collect();
    let mut addresses = Vec::new();
    let mut dns = Vec::new();
    for fam in ["ipv4", "ipv6"] {
        let g = group(fam);
        for a in dicts(g, "address-data") {
            addresses.push(format!(
                "{}/{}",
                text(&a, "address"),
                uint(&a, "prefix").unwrap_or(0)
            ));
        }
        if let Some(v) = g.get("dns") {
            if let Ok(v4) = Vec::<u32>::try_from(v.clone()) {
                dns.extend(
                    v4.iter()
                        .map(|n| std::net::Ipv4Addr::from(n.to_ne_bytes()).to_string()),
                );
            } else if let Ok(v6) = Vec::<Vec<u8>>::try_from(v.clone()) {
                dns.extend(v6.iter().filter_map(|b| {
                    <[u8; 16]>::try_from(b.as_slice())
                        .ok()
                        .map(|o| std::net::Ipv6Addr::from(o).to_string())
                }));
            }
        }
    }
    dns.extend(strings(group("ipv4"), "dns-search"));
    Config {
        name: text(conn, "id"),
        iface: text(conn, "interface-name"),
        private_key: Psk::new(&secret.map(|s| text(s, "private-key")).unwrap_or_default()),
        addresses,
        dns,
        listen_port: uint(wg, "listen-port")
            .filter(|&p| p != 0)
            .and_then(|p| u16::try_from(p).ok()),
        mtu: uint(wg, "mtu").filter(|&m| m != 0),
        peers,
        stored_key: secrets.is_none()
            && uint(wg, "private-key-flags").unwrap_or(0) & SECRET_NOT_SAVED == 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=";
    const KEY_B: &str = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=";

    #[test]
    fn a_key_round_trips_through_base64() {
        let k = decode_key(KEY_A).expect("a valid key");
        assert_eq!(b64_encode(&k), KEY_A);
        assert_eq!(decode_key("short"), None);
        assert_eq!(decode_key(&KEY_A.replace('=', "A")), None, "no padding");
        assert_eq!(
            decode_key("yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmn="),
            None,
            "the last character carries bits past the 32 bytes"
        );
    }

    #[test]
    fn the_public_key_follows_from_the_private_one() {
        assert_eq!(
            public_key(KEY_A).as_deref(),
            Some("HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=")
        );
        assert_eq!(public_key("not a key"), None);
    }

    #[test]
    fn a_generated_key_is_clamped_and_unique() {
        let a = generate_key().unwrap();
        let b = generate_key().unwrap();
        assert_ne!(a, b);
        let k = decode_key(&a).unwrap();
        assert_eq!(k[0] & 7, 0, "the low three bits are clear");
        assert_eq!(k[31] & 0xc0, 0x40, "bit 254 set, bit 255 clear");
        assert!(public_key(&a).is_some());
    }

    const CONF: &str = "\
# home tunnel
[Interface]
PrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=
Address = 10.192.122.3/24, fd00::3/64
Address = 10.10.0.1/16
DNS = 10.192.122.1, home.lan
ListenPort = 51820
PostUp = iptables -A FORWARD -i %i -j ACCEPT

[Peer]
PublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=
Endpoint = 192.95.5.67:1234
AllowedIPs = 10.192.122.3/32, 10.192.124.1/24
PersistentKeepalive = 25

[Peer]
PublicKey = TrMvSoP4jYQlY6RIzBgbssQqY3vxI2Pi+y71lOWWXX0=
PresharedKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=
AllowedIPs = 0.0.0.0/0
";

    #[test]
    fn a_conf_file_reads_into_a_connection() {
        let c = parse_conf(CONF, "home tunnel").unwrap();
        assert_eq!(c.name, "home tunnel");
        assert_eq!(c.iface, "hometunnel");
        assert_eq!(&*c.private_key, KEY_A);
        assert_eq!(
            c.addresses,
            ["10.192.122.3/24", "fd00::3/64", "10.10.0.1/16"]
        );
        assert_eq!(c.dns, ["10.192.122.1", "home.lan"]);
        assert_eq!(c.listen_port, Some(51820));
        assert_eq!(c.peers.len(), 2);
        assert_eq!(c.peers[0].public_key, KEY_B);
        assert_eq!(c.peers[0].endpoint, "192.95.5.67:1234");
        assert_eq!(
            c.peers[0].allowed_ips,
            ["10.192.122.3/32", "10.192.124.1/24"]
        );
        assert_eq!(c.peers[0].keepalive, Some(25));
        assert_eq!(&*c.peers[1].preshared_key, KEY_A);
        assert_eq!(c.peers[1].endpoint, "");
        assert_eq!(check(&c), None);
    }

    #[test]
    fn a_broken_conf_names_its_line() {
        assert_eq!(
            parse_conf("[Interface]\nPrivateKey\n", "x"),
            Err("Line 2: expected Key = Variant.".into())
        );
        assert_eq!(
            parse_conf("[Interface]\nFoo = 1\n", "x"),
            Err("Line 2: foo is not an interface setting.".into())
        );
        assert_eq!(
            parse_conf("[Peer]\nPublicKey = a\n", "x"),
            Err("The file has no [Interface] section.".into())
        );
        assert_eq!(
            parse_conf("client\ndev tun\n", "x"),
            Err("Line 1: expected Key = Variant.".into()),
            "an OpenVPN file is no WireGuard one"
        );
    }

    fn good() -> Config {
        Config {
            name: "Away".into(),
            iface: "wg0".into(),
            private_key: Psk::new(KEY_A),
            addresses: vec!["10.0.0.2/24".into()],
            peers: vec![Peer {
                public_key: KEY_B.into(),
                endpoint: "vpn.example.org:51820".into(),
                allowed_ips: vec!["0.0.0.0/0".into(), "::/0".into()],
                ..Peer::default()
            }],
            ..Config::default()
        }
    }

    #[test]
    fn check_names_the_first_bad_field() {
        assert_eq!(check(&good()), None);
        let field = |f: fn(&mut Config)| {
            let mut c = good();
            f(&mut c);
            check(&c).map(|p| p.field)
        };
        assert_eq!(field(|c| c.name.clear()), Some(Field::Name));
        assert_eq!(field(|c| c.iface = "a/b".into()), Some(Field::Iface));
        assert_eq!(
            field(|c| c.iface = "sixteen-chars-xx".into()),
            Some(Field::Iface)
        );
        assert_eq!(
            field(|c| c.private_key = Psk::new("")),
            Some(Field::PrivateKey)
        );
        assert_eq!(
            field(|c| c.addresses = vec!["10.0.0.300/24".into()]),
            Some(Field::Addresses)
        );
        assert_eq!(
            field(|c| c.addresses = vec!["10.0.0.1/33".into()]),
            Some(Field::Addresses)
        );
        assert_eq!(
            field(|c| c.peers[0].public_key = "nope".into()),
            Some(Field::PublicKey(0))
        );
        assert_eq!(
            field(|c| c.peers[0].endpoint = "vpn.example.org".into()),
            Some(Field::Endpoint(0))
        );
        assert_eq!(
            field(|c| c.peers[0].endpoint = "[fd00::1]:51820".into()),
            None
        );
        assert_eq!(
            field(|c| c.peers[0].allowed_ips = vec!["everything".into()]),
            Some(Field::AllowedIps(0))
        );
    }

    #[test]
    fn the_settings_carry_every_field() {
        let mut c = good();
        c.dns = vec!["1.1.1.1".into(), "2606:4700::1111".into(), "corp".into()];
        c.addresses.push("fd00::2/64".into());
        c.listen_port = Some(51820);
        c.peers[0].keepalive = Some(25);
        let s = settings(&c, Some("alice"));
        assert_eq!(s["connection"]["type"], Variant::from("wireguard"));
        assert_eq!(s["connection"]["interface-name"], Variant::from("wg0"));
        assert_eq!(
            s["connection"]["permissions"],
            Variant::from(vec!["user:alice".to_string()])
        );
        assert_eq!(s["wireguard"]["private-key"], Variant::from(KEY_A));
        assert_eq!(s["wireguard"]["listen-port"], Variant::from(51820u32));
        assert_eq!(s["ipv4"]["method"], Variant::from("manual"));
        assert_eq!(s["ipv6"]["method"], Variant::from("manual"));
        assert_eq!(
            s["ipv4"]["dns"],
            Variant::from(vec![u32::from_ne_bytes([1, 1, 1, 1])]),
            "1.1.1.1 in network byte order"
        );
        assert_eq!(
            s["ipv4"]["dns-search"],
            Variant::from(vec!["corp".to_string()])
        );
        let owned = |s: &Settings| -> HashMap<String, Props> {
            s.iter()
                .map(|(g, m)| {
                    let m = m
                        .iter()
                        .map(|(k, v)| (k.clone(), v.try_to_owned().unwrap()))
                        .collect();
                    (g.clone(), m)
                })
                .collect()
        };
        let all = owned(&s);
        let back = from_settings(&all, Some(&all));
        assert_eq!(back, c);
        let keyless = from_settings(&all, None);
        assert_eq!(&*keyless.private_key, "");
        assert_eq!(keyless.peers[0].public_key, KEY_B);
    }

    #[test]
    fn a_peer_without_a_preshared_key_needs_none() {
        let s = settings(&good(), None);
        assert!(!s["connection"].contains_key("permissions"));
        let peers =
            Vec::<HashMap<String, Variant>>::try_from(s["wireguard"]["peers"].clone()).unwrap();
        assert_eq!(peers[0]["preshared-key-flags"], Variant::from(4u32));
        assert!(!peers[0].contains_key("preshared-key"));
        assert_eq!(
            settings(
                &Config {
                    addresses: vec![],
                    ..good()
                },
                None
            )["ipv4"]["method"],
            Variant::from("disabled")
        );
    }

    fn kept() -> Config {
        let mut c = good();
        c.private_key = Psk::new("");
        c.stored_key = true;
        c.peers[0].stored_psk = true;
        c
    }

    #[test]
    fn kept_keys_go_out_as_flags_alone() {
        assert_eq!(check(&kept()), None);
        let s = settings(&kept(), None);
        assert!(!s["wireguard"].contains_key("private-key"));
        assert_eq!(s["wireguard"]["private-key-flags"], Variant::from(0u32));
        let peers =
            Vec::<HashMap<String, Variant>>::try_from(s["wireguard"]["peers"].clone()).unwrap();
        assert!(!peers[0].contains_key("preshared-key"));
        assert_eq!(
            peers[0]["preshared-key-flags"],
            Variant::from(0u32),
            "a kept preshared key stays required"
        );
    }

    #[test]
    fn a_kept_key_beside_a_changed_one_is_a_problem() {
        let mut c = kept();
        c.peers[0].preshared_key = Psk::new(KEY_A);
        assert_eq!(check(&c).map(|p| p.field), Some(Field::PrivateKey));
        let mut c = kept();
        c.private_key = Psk::new(KEY_A);
        assert_eq!(check(&c).map(|p| p.field), Some(Field::PresharedKey(0)));
        c.peers[0].preshared_key = Psk::new(KEY_B);
        assert_eq!(check(&c), None, "every key typed again");
        let mut c = kept();
        c.peers[0].stored_psk = false;
        c.private_key = Psk::new(KEY_A);
        assert_eq!(check(&c), None, "a peer with no key keeps nothing");
    }

    #[test]
    fn refused_secrets_read_back_as_stored_keys() {
        let v = |x: Variant<'static>| x.try_to_owned().unwrap();
        let peer = |flags: Option<u32>| {
            let mut p = HashMap::from([("public-key".to_string(), v(Variant::from(KEY_B)))]);
            if let Some(f) = flags {
                p.insert("preshared-key-flags".into(), v(Variant::from(f)));
            }
            p
        };
        let profile = |key_flags: Option<u32>| {
            let mut wg = HashMap::from([(
                "peers".to_string(),
                v(Variant::from(vec![
                    peer(Some(0)),
                    peer(None),
                    peer(Some(2)),
                ])),
            )]);
            if let Some(f) = key_flags {
                wg.insert("private-key-flags".into(), v(Variant::from(f)));
            }
            HashMap::from([("wireguard".to_string(), wg)])
        };
        let c = from_settings(&profile(None), None);
        assert!(c.stored_key);
        let stored: Vec<bool> = c.peers.iter().map(|p| p.stored_psk).collect();
        assert_eq!(
            stored,
            [true, false, false],
            "flags 0 store a key; none (not required) and not-saved do not"
        );
        assert!(!from_settings(&profile(Some(2)), None).stored_key);
        let given = from_settings(&profile(None), Some(&HashMap::new()));
        assert!(!given.stored_key, "secrets given out leave nothing hidden");
        assert!(given.peers.iter().all(|p| !p.stored_psk));
    }

    #[test]
    fn an_edit_keeps_what_the_editor_does_not_show() {
        let v = |x: Variant<'static>| x.try_to_owned().unwrap();
        let current = HashMap::from([
            (
                "connection".to_string(),
                HashMap::from([
                    ("id".to_string(), v(Variant::from("Old"))),
                    ("uuid".to_string(), v(Variant::from("1234"))),
                    ("autoconnect".to_string(), v(Variant::from(true))),
                ]),
            ),
            (
                "ipv4".to_string(),
                HashMap::from([
                    (
                        "addresses".to_string(),
                        v(Variant::from(vec![vec![1u32, 24, 0]])),
                    ),
                    ("route-metric".to_string(), v(Variant::from(50i64))),
                ]),
            ),
        ]);
        let m = merge(current, settings(&good(), Some("alice")));
        assert_eq!(m["connection"]["id"], Variant::from("Away"));
        assert_eq!(m["connection"]["uuid"], Variant::from("1234"));
        assert_eq!(
            m["connection"]["autoconnect"],
            Variant::from(true),
            "autoconnect stays as it was"
        );
        assert!(!m["connection"].contains_key("permissions"));
        assert!(!m["ipv4"].contains_key("addresses"), "the old form goes");
        assert_eq!(m["ipv4"]["route-metric"], Variant::from(50i64));
        assert!(m["ipv4"].contains_key("address-data"));
    }
}
