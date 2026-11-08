use super::call::{Failure, call_auth, clean, proxy, proxy_on};
use super::snapshot::{get_all, saved};
use super::*;
use zbus::Variant;

/// A fresh Wi-Fi profile for `ssid`, secured by `key` when one was typed: `key_mgmt` for a
/// passphrase, `wpa-eap` for an 802.1X login. A `hidden` network is probed for by name, as it
/// beacons none. With an `owner` it is that login's own profile (`settings.modify.own`), else
/// system-wide.
pub(super) fn wifi_settings<'a>(
    ssid: &Ssid,
    key: Option<&'a Key>,
    key_mgmt: &'static str,
    hidden: bool,
    owner: Option<&str>,
) -> HashMap<String, HashMap<String, Variant<'a>>> {
    let mut out: HashMap<String, HashMap<String, Variant<'a>>> = HashMap::new();
    let mut connection = HashMap::from([
        ("id".to_string(), Variant::from(ssid.to_string())),
        ("type".to_string(), Variant::from("802-11-wireless")),
    ]);
    if let Some(owner) = owner {
        connection.insert(
            "permissions".to_string(),
            Variant::from(vec![format!("user:{owner}")]),
        );
    }
    out.insert("connection".into(), connection);
    let mut wireless = HashMap::from([
        ("ssid".to_string(), Variant::from(ssid.0.clone())),
        ("mode".to_string(), Variant::from("infrastructure")),
    ]);
    if hidden {
        wireless.insert("hidden".to_string(), Variant::from(true));
    }
    out.insert("802-11-wireless".into(), wireless);
    if let Some(key) = key {
        secure(&mut out, key, key_mgmt);
    }
    out
}

/// Put `key` into `settings`' security groups: a passphrase under `key_mgmt`, or an 802.1X
/// login as `wpa-eap` with its own `802-1x` group.
fn secure<'a>(
    settings: &mut HashMap<String, HashMap<String, Variant<'a>>>,
    key: &'a Key,
    key_mgmt: &str,
) {
    let security = settings
        .entry("802-11-wireless-security".into())
        .or_default();
    match key {
        Key::Psk(psk) | Key::Sae(psk) => {
            let key_mgmt = if matches!(key, Key::Sae(_)) {
                "sae"
            } else {
                key_mgmt
            };
            security.insert("key-mgmt".into(), Variant::from(key_mgmt.to_string()));
            security.insert("psk".into(), Variant::from(&**psk));
            security.insert("psk-flags".into(), Variant::from(0u32));
        }
        Key::Enterprise(e) => {
            security.insert("key-mgmt".into(), Variant::from("wpa-eap"));
            security.remove("psk");
            security.remove("psk-flags");
            settings.insert("802-1x".into(), dot1x(e));
        }
    }
}

/// The `802-1x` group for `e`. Secrets carry flags 0 so NetworkManager stores them in the
/// profile; an empty private-key password is marked not required (0x4), as an unencrypted key
/// needs none.
fn dot1x(e: &Enterprise) -> HashMap<String, Variant<'_>> {
    let mut g = HashMap::from([
        (
            "eap".to_string(),
            Variant::from(vec![e.eap.label().to_lowercase()]),
        ),
        ("identity".to_string(), Variant::from(e.identity.as_str())),
    ]);
    match &e.ca {
        CaSource::None => {}
        CaSource::File(ca) => {
            g.insert("ca-cert".into(), Variant::from(cert_path(ca)));
        }
        CaSource::System => {
            g.insert("system-ca-certs".into(), Variant::from(true));
        }
    }
    if !e.domain.is_empty() {
        g.insert(
            "domain-suffix-match".into(),
            Variant::from(e.domain.as_str()),
        );
    }
    match e.eap {
        Eap::Peap | Eap::Ttls => {
            if !e.anonymous_identity.is_empty() {
                g.insert(
                    "anonymous-identity".into(),
                    Variant::from(e.anonymous_identity.as_str()),
                );
            }
            // TTLS names an inner EAP method (GTC) apart from its plain ones.
            let phase2 = match (e.eap, e.inner) {
                (Eap::Ttls, Inner::Gtc) => "phase2-autheap",
                _ => "phase2-auth",
            };
            g.insert(phase2.into(), Variant::from(e.inner.label().to_lowercase()));
            g.insert("password".into(), Variant::from(&*e.password));
            g.insert("password-flags".into(), Variant::from(0u32));
        }
        Eap::Tls => {
            if let Some(cert) = &e.client_cert {
                g.insert("client-cert".into(), Variant::from(cert_path(cert)));
            }
            if let Some(key) = &e.private_key {
                g.insert("private-key".into(), Variant::from(cert_path(key)));
            }
            if e.private_key_password.is_empty() {
                g.insert("private-key-password-flags".into(), Variant::from(4u32));
            } else {
                g.insert(
                    "private-key-password".into(),
                    Variant::from(&*e.private_key_password),
                );
                g.insert("private-key-password-flags".into(), Variant::from(0u32));
            }
        }
    }
    g
}

/// A certificate or key by path as NetworkManager's D-Bus API takes it: the bytes of
/// `file://` and the absolute path, ending in a NUL.
fn cert_path(path: &std::path::Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    let mut out = b"file://".to_vec();
    out.extend_from_slice(path.as_os_str().as_bytes());
    out.push(0);
    out
}

/// The `key-mgmt` for a passphrase profile: "sae" when the AP offers SAE and no PSK (NM will not
/// put wpa-psk on a WPA3-only network), else "wpa-psk".
pub(super) fn key_mgmt_for(wpa: u32, rsn: u32) -> &'static str {
    if rsn & 0x400 != 0 && (wpa | rsn) & 0x100 == 0 {
        "sae"
    } else {
        "wpa-psk"
    }
}

pub(super) async fn ap_key_mgmt(conn: &Connection, ap: &str) -> &'static str {
    if ap == "/" {
        return "wpa-psk";
    }
    get_all(conn, ap, AP_IFACE).await.map_or("wpa-psk", |p| {
        key_mgmt_for(uint(&p, "WpaFlags"), uint(&p, "RsnFlags"))
    })
}

pub(super) fn rekey_mgmt<'a>(current: &'a str, offered: &'a str) -> &'a str {
    match current {
        "wpa-psk" | "sae" => current,
        _ => offered,
    }
}

/// Who a new profile belongs to: nobody (system-wide) when NM allows `settings.modify.system`, else
/// `login`; a system-wide profile would cost everyone else an admin password on every join.
pub(super) fn profile_owner(
    perms: &HashMap<String, String>,
    login: Option<String>,
) -> Option<String> {
    if perms.get(MODIFY_SYSTEM).is_some_and(|r| r == "yes") {
        None
    } else {
        login
    }
}

pub(super) async fn new_profile_owner(conn: &Connection) -> Option<String> {
    let perms = async {
        let manager = proxy(conn, MANAGER_PATH, NAME).await?;
        let reply = manager
            .call_method("GetPermissions", &())
            .await
            .map_err(clean)?;
        reply
            .body()
            .deserialize::<HashMap<String, String>>()
            .map_err(clean)
    }
    .await
    .unwrap_or_else(|e| {
        log::debug!("GetPermissions: {e}");
        HashMap::new()
    });
    let login = match proxy_on(
        conn,
        LOGIND,
        "/org/freedesktop/login1/user/self",
        "org.freedesktop.login1.User",
    )
    .await
    {
        Ok(user) => user.get_property::<String>("Name").await.ok(),
        Err(_) => None,
    };
    if login.is_none() {
        log::warn!("logind has no name for this user; a new profile is system-wide");
    }
    profile_owner(&perms, login)
}

pub(super) async fn connection_for(conn: &Connection, ssid: &Ssid) -> Option<String> {
    if ssid.is_empty() {
        return None;
    }
    saved(conn)
        .await
        .into_iter()
        .find(|(_, s)| {
            s.get("802-11-wireless")
                .is_some_and(|w| ssid_of(w, "ssid") == *ssid)
        })
        .map(|(path, _)| path)
}

/// Write a new key into a saved connection. `GetSettings` omits secrets and `Update` replaces
/// the whole profile, so the map sent back is everything else plus the key.
pub(super) async fn rekey(
    conn: &Connection,
    conn_path: &str,
    key: &Key,
    offered: &'static str,
) -> Result<(), Failure> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    let reply = p.call_method("GetSettings", &()).await.map_err(clean)?;
    let current: HashMap<String, Props> = reply.body().deserialize().map_err(clean)?;
    call_auth::<_, ()>(&p, "Update", &(rekeyed(current, key, offered),))
        .await
        .map(|_| ())
}

/// `current` with `key` in place of its key. A passphrase keeps a PSK profile's own
/// `key-mgmt`, else takes `offered`; an 802.1X login replaces the whole `802-1x` group.
pub(super) fn rekeyed<'a>(
    current: HashMap<String, Props>,
    key: &'a Key,
    offered: &'static str,
) -> HashMap<String, HashMap<String, Variant<'a>>> {
    let key_mgmt = current
        .get("802-11-wireless-security")
        .map(|s| text(s, "key-mgmt"))
        .unwrap_or_default();
    let key_mgmt = rekey_mgmt(&key_mgmt, offered).to_string();
    let mut settings: HashMap<String, HashMap<String, Variant<'a>>> = current
        .into_iter()
        .map(|(group, entries)| {
            let entries = entries
                .into_iter()
                .map(|(k, v)| (k, Variant::from(v)))
                .collect();
            (group, entries)
        })
        .collect();
    secure(&mut settings, key, &key_mgmt);
    settings
}
