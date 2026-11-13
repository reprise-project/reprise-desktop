//! Running the commands the windows send back.

use super::call::{Failure, activate, call_auth, clean, opath, proxy};
use super::snapshot::{get_all, object, saved};
use super::wifi::{ap_key_mgmt, connection_for, new_profile_owner, rekey, wifi_settings};
use super::*;
use zbus::Variant;

/// `Device.Disconnect`; Disable and Repair only log its error, since a device already down refuses
/// it.
async fn disconnect(conn: &Connection, device: &str) -> Result<(), Failure> {
    proxy(conn, device, DEVICE_IFACE)
        .await?
        .call_method("Disconnect", &())
        .await
        .map(|_| ())
        .map_err(clean)
}

pub(super) async fn command(conn: &Connection, cmd: &Cmd) -> Result<Option<NmMsg>, Failure> {
    match cmd {
        Cmd::LoadVpn(conn_path) => load_vpn(conn, conn_path).await.map(|e| Some(NmMsg::Vpn(e))),
        cmd => act(conn, cmd).await.map(|()| None),
    }
}

/// Refuse to activate a profile that would wait on a secret agent: NetworkManager fails it
/// without a word. A profile that can't be read is left to `ActivateConnection` to refuse.
async fn needs_no_agent(conn: &Connection, conn_path: &str, why: &str) -> Result<(), Failure> {
    match settings_of(conn, conn_path).await {
        Ok(s) if needs_agent(&s) => Err(why.into()),
        _ => Ok(()),
    }
}

async fn settings_of(
    conn: &Connection,
    conn_path: &str,
) -> Result<HashMap<String, Props>, Failure> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    let reply = p.call_method("GetSettings", &()).await.map_err(clean)?;
    reply.body().deserialize().map_err(clean)
}

/// A saved VPN for its Properties sheet; WireGuard keys come from `GetSecrets`, which may prompt;
/// refused, the key fields are empty.
async fn load_vpn(conn: &Connection, conn_path: &str) -> Result<VpnEdit, Failure> {
    let settings = settings_of(conn, conn_path).await?;
    let kind = settings
        .get("connection")
        .map(|c| text(c, "type"))
        .unwrap_or_default();
    if kind != "wireguard" {
        return Ok(plugin_vpn(conn_path, &settings));
    }
    let secrets = async {
        let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
        call_auth::<_, HashMap<String, Props>>(&p, "GetSecrets", &("wireguard",))
            .await?
            .ok_or_else(|| Failure::from("no secrets in the reply"))
    }
    .await;
    let secrets = secrets
        .inspect_err(|e| log::warn!("the keys of {conn_path} are not readable: {e}"))
        .ok();
    Ok(VpnEdit::WireGuard {
        conn_path: conn_path.to_string(),
        cfg: Box::new(crate::wg::from_settings(&settings, secrets.as_ref())),
    })
}

/// A plugin VPN's saved `vpn.secrets`, which may prompt. A changed secret goes back to `Update`
/// with these, so the ones the sheet left alone are sent as they were.
async fn vpn_secrets(
    conn: &Connection,
    conn_path: &str,
) -> Result<HashMap<String, String>, Failure> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    let all = call_auth::<_, HashMap<String, Props>>(&p, "GetSecrets", &("vpn",))
        .await?
        .unwrap_or_default();
    Ok(all
        .get("vpn")
        .and_then(|v| v.get("secrets"))
        .and_then(|s| HashMap::<String, String>::try_from(s.clone()).ok())
        .unwrap_or_default())
}

pub(super) fn hotspot_settings(
    ssid: &str,
    psk: &str,
    wpa3: bool,
    band: Band,
    owner: Option<&str>,
) -> crate::wg::Settings {
    let mut connection: HashMap<String, Variant<'static>> = HashMap::from([
        ("id".to_string(), Variant::from("Hotspot")),
        ("type".to_string(), Variant::from("802-11-wireless")),
        ("autoconnect".to_string(), Variant::from(false)),
    ]);
    if let Some(owner) = owner {
        connection.insert(
            "permissions".to_string(),
            Variant::from(vec![format!("user:{owner}")]),
        );
    }
    let mut wireless: HashMap<String, Variant<'static>> = HashMap::from([
        ("ssid".to_string(), Variant::from(ssid.as_bytes().to_vec())),
        ("mode".to_string(), Variant::from("ap")),
    ]);
    match band {
        Band::Auto => {}
        Band::Bg => {
            wireless.insert("band".into(), Variant::from("bg"));
        }
        Band::A => {
            wireless.insert("band".into(), Variant::from("a"));
        }
    }
    let security: HashMap<String, Variant<'static>> = HashMap::from([
        (
            "key-mgmt".to_string(),
            Variant::from(if wpa3 { "sae" } else { "wpa-psk" }),
        ),
        ("proto".to_string(), Variant::from(vec!["rsn".to_string()])),
        (
            "pairwise".to_string(),
            Variant::from(vec!["ccmp".to_string()]),
        ),
        ("group".to_string(), Variant::from(vec!["ccmp".to_string()])),
        ("psk".to_string(), Variant::from(psk.to_string())),
        ("psk-flags".to_string(), Variant::from(0u32)),
    ]);
    HashMap::from([
        ("connection".to_string(), connection),
        ("802-11-wireless".to_string(), wireless),
        ("802-11-wireless-security".to_string(), security),
        (
            "ipv4".to_string(),
            HashMap::from([("method".to_string(), Variant::from("shared"))]),
        ),
        (
            "ipv6".to_string(),
            HashMap::from([("method".to_string(), Variant::from("ignore"))]),
        ),
    ])
}

pub(super) fn import_error(stderr: &str) -> String {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("The file could not be imported.");
    line.strip_prefix("Error: ").unwrap_or(line).to_string()
}

/// `nmcli connection import type KIND file PATH`: importers live in libnm and each VPN plugin's
/// editor library, not on the bus. Run on its own thread so the bus executor never waits.
async fn nmcli_import(path: &std::path::Path, kind: &str) -> Result<(), Failure> {
    let (tx, rx) = async_channel::bounded(1);
    let (path, kind) = (path.to_path_buf(), kind.to_string());
    std::thread::Builder::new()
        .name("reprise-network-import".into())
        .spawn(move || {
            let out = std::process::Command::new("nmcli")
                .args(["connection", "import", "type", &kind, "file"])
                .arg(&path)
                .output();
            let _ = tx.send_blocking(match out {
                Ok(o) if o.status.success() => Ok(()),
                Ok(o) => Err(import_error(&String::from_utf8_lossy(&o.stderr))),
                Err(e) => Err(format!("nmcli could not be started: {e}")),
            });
        })
        .map_err(|e| e.to_string())?;
    Ok(rx.recv().await.map_err(|e| e.to_string())??)
}

async fn add_connection(conn: &Connection, settings: crate::wg::Settings) -> Result<(), Failure> {
    let p = proxy(conn, SETTINGS_PATH, SETTINGS_IFACE).await?;
    call_auth::<_, zbus::OwnedObjectPath>(&p, "AddConnection", &(settings,))
        .await
        .map(|_| ())
}

async fn update(
    conn: &Connection,
    conn_path: &str,
    settings: crate::wg::Settings,
) -> Result<(), Failure> {
    let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
    call_auth::<_, ()>(&p, "Update", &(settings,))
        .await
        .map(|_| ())
}

pub(super) async fn act(conn: &Connection, cmd: &Cmd) -> Result<(), Failure> {
    match cmd {
        Cmd::Connect {
            device,
            ap,
            ssid,
            key,
            hidden,
        } => {
            let known = connection_for(conn, ssid).await;
            match connect_plan(known.as_deref(), key.as_ref()) {
                ConnectPlan::Activate(connection) => {
                    needs_no_agent(conn, &connection, WIFI_NEEDS_AGENT).await?;
                    activate(conn, &connection, device, ap).await
                }
                ConnectPlan::Rekey { conn_path, key } => {
                    let offered = ap_key_mgmt(conn, ap).await;
                    rekey(conn, &conn_path, key, offered).await?;
                    activate(conn, &conn_path, device, ap).await
                }
                ConnectPlan::Add { key } => {
                    let owner = new_profile_owner(conn).await;
                    let key_mgmt = ap_key_mgmt(conn, ap).await;
                    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
                    call_auth::<_, (zbus::OwnedObjectPath, zbus::OwnedObjectPath)>(
                        &manager,
                        "AddAndActivateConnection",
                        &(
                            wifi_settings(ssid, key, key_mgmt, *hidden, owner.as_deref()),
                            opath(device)?,
                            opath(ap)?,
                        ),
                    )
                    .await
                    .map(|_| ())
                }
            }
        }
        Cmd::Disconnect(device) => disconnect(conn, device).await,
        Cmd::Disable(device) => {
            if let Err(e) = disconnect(conn, device).await {
                log::debug!("disconnecting {device} before disabling it: {e}");
            }
            proxy(conn, device, DEVICE_IFACE)
                .await?
                .set_property("Managed", false)
                .await
                .map_err(clean)
        }
        Cmd::Enable(device) => proxy(conn, device, DEVICE_IFACE)
            .await?
            .set_property("Managed", true)
            .await
            .map_err(clean),
        Cmd::Repair(device) => {
            if let Err(e) = disconnect(conn, device).await {
                log::debug!("disconnecting {device} before repairing it: {e}");
            }
            activate(conn, "/", device, "/").await
        }
        Cmd::VpnUp(connection) => {
            needs_no_agent(conn, connection, VPN_NEEDS_AGENT).await?;
            activate(conn, connection, "/", "/").await
        }
        Cmd::VpnDown(connection) => {
            let manager = proxy(conn, MANAGER_PATH, NAME).await?;
            let actives = manager
                .get_property::<Vec<zbus::OwnedObjectPath>>("ActiveConnections")
                .await
                .map_err(clean)?;
            for active in actives {
                let active = active.as_str().to_string();
                let Some(props) = get_all(conn, &active, ACTIVE_IFACE).await else {
                    continue;
                };
                if object(&props, "Connection").as_deref() != Some(connection.as_str()) {
                    continue;
                }
                return manager
                    .call_method("DeactivateConnection", &(opath(&active)?,))
                    .await
                    .map(|_| ())
                    .map_err(clean);
            }
            Err("That connection is not active.".into())
        }
        Cmd::SaveWireGuard { conn_path, cfg } => {
            let owner = new_profile_owner(conn).await;
            let fresh = crate::wg::settings(cfg, owner.as_deref());
            match conn_path {
                Some(path) => {
                    let current = settings_of(conn, path).await?;
                    update(conn, path, crate::wg::merge(current, fresh)).await
                }
                None => add_connection(conn, fresh).await,
            }
        }
        Cmd::SavePluginVpn {
            conn_path,
            name,
            data,
            secrets,
        } => {
            let current = settings_of(conn, conn_path).await?;
            let secrets = if secrets.is_empty() {
                HashMap::new()
            } else {
                let mut all = vpn_secrets(conn, conn_path).await?;
                all.extend(secrets.iter().map(|(k, v)| (k.clone(), v.to_string())));
                all
            };
            update(conn, conn_path, plugin_update(current, name, data, secrets)).await
        }
        Cmd::DeleteVpn(conn_path) => {
            let p = proxy(conn, conn_path, CONNECTION_IFACE).await?;
            call_auth::<_, ()>(&p, "Delete", &()).await.map(|_| ())
        }
        Cmd::Import { path, kinds } => {
            let mut last = Err("No VPN plugin for NetworkManager is installed.".into());
            for kind in kinds {
                last = nmcli_import(path, kind).await;
                if last.is_ok() {
                    break;
                }
            }
            last
        }
        Cmd::Hotspot {
            device,
            ssid,
            psk,
            wpa3,
            band,
        } => {
            let owner = new_profile_owner(conn).await;
            let fresh = hotspot_settings(ssid, psk, *wpa3, *band, owner.as_deref());
            let existing = hotspot_profiles(&saved(conn).await).into_keys().min();
            match existing {
                Some(path) => {
                    let current = settings_of(conn, &path).await?;
                    update(conn, &path, merge_groups(current, fresh)).await?;
                    activate(conn, &path, device, "/").await
                }
                None => {
                    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
                    call_auth::<_, (zbus::OwnedObjectPath, zbus::OwnedObjectPath)>(
                        &manager,
                        "AddAndActivateConnection",
                        &(fresh, opath(device)?, opath("/")?),
                    )
                    .await
                    .map(|_| ())
                }
            }
        }
        Cmd::Rescan(device) => {
            let empty: HashMap<&str, Variant<'_>> = HashMap::new();
            proxy(conn, device, WIRELESS_IFACE)
                .await?
                .call_method("RequestScan", &(empty,))
                .await
                .map(|_| ())
                .map_err(clean)
        }
        Cmd::Stats { device, on } => proxy(conn, device, STATS_IFACE)
            .await?
            .set_property("RefreshRateMs", if *on { STATS_MS } else { 0 })
            .await
            .map_err(clean),
        Cmd::LoadVpn(_) => unreachable!("`command` answers LoadVpn itself"),
        Cmd::Airplane(on) => reprisebus::airplane::set(conn, *on)
            .await
            .map_err(Failure::from),
        Cmd::Quit => unreachable!("`commands` turns Quit into Wake::Quit"),
    }
}
