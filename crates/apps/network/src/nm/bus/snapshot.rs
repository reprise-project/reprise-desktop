use super::*;
use futures_util::future::join_all;
use zbus::{ObjectPath, Proxy};

/// One object's properties, None when gone or refused; a failed read leaves its part out of the
/// snapshot and is logged.
pub(super) async fn get_all(conn: &Connection, path: &str, iface: &str) -> Option<Props> {
    let p = match Proxy::new(conn, NAME, path.to_string(), PROPERTIES).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("no properties proxy for {path}: {e}");
            return None;
        }
    };
    match p.call_method("GetAll", &(iface,)).await {
        Ok(reply) => match reply.body().deserialize() {
            Ok(props) => Some(props),
            Err(e) => {
                log::debug!("GetAll({iface}) on {path} did not deserialize: {e}");
                None
            }
        },
        Err(e) => {
            log::debug!("GetAll({iface}) on {path} failed: {e}");
            None
        }
    }
}

pub(super) fn object(props: &Props, key: &str) -> Option<String> {
    let path = props
        .get(key)
        .and_then(|v| ObjectPath::try_from(v.clone()).ok())?
        .to_string();
    (path != "/").then_some(path)
}

async fn paths(conn: &Connection, path: &str, iface: &str, method: &str) -> Vec<String> {
    let p = match Proxy::new(conn, NAME, path.to_string(), iface).await {
        Ok(p) => p,
        Err(e) => {
            log::debug!("no {iface} proxy for {path}: {e}");
            return vec![];
        }
    };
    match p.call_method(method, &()).await {
        Ok(reply) => match reply.body().deserialize::<Vec<zbus::OwnedObjectPath>>() {
            Ok(v) => v.iter().map(|p| p.as_str().to_string()).collect(),
            Err(e) => {
                log::debug!("{method} on {path} did not deserialize: {e}");
                vec![]
            }
        },
        Err(e) => {
            log::debug!("{method} on {path} failed: {e}");
            vec![]
        }
    }
}

/// Every saved connection by object path; the `GetSettings` reads go out together so a rebuild
/// waits one round trip.
pub(super) async fn saved(conn: &Connection) -> Vec<(String, HashMap<String, Props>)> {
    let list = paths(conn, SETTINGS_PATH, SETTINGS_IFACE, "ListConnections").await;
    join_all(list.into_iter().map(|path| async move {
        let p = match Proxy::new(conn, NAME, path.clone(), CONNECTION_IFACE).await {
            Ok(p) => p,
            Err(e) => {
                log::debug!("no connection proxy for {path}: {e}");
                return None;
            }
        };
        match p.call_method("GetSettings", &()).await {
            Ok(reply) => match reply.body().deserialize::<HashMap<String, Props>>() {
                Ok(s) => Some((path, s)),
                Err(e) => {
                    log::debug!("the settings of {path} did not deserialize: {e}");
                    None
                }
            },
            Err(e) => {
                log::debug!("GetSettings on {path} failed: {e}");
                None
            }
        }
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

async fn get_all_of(conn: &Connection, paths: Vec<String>, iface: &str) -> Vec<(String, Props)> {
    join_all(paths.into_iter().map(|path| async move {
        let props = get_all(conn, &path, iface).await?;
        Some((path, props))
    }))
    .await
    .into_iter()
    .flatten()
    .collect()
}

pub(super) async fn build(
    conn: &Connection,
    prev: &Snapshot,
    saved: &[(String, HashMap<String, Props>)],
) -> Snapshot {
    let Some(manager_props) = get_all(conn, MANAGER_PATH, NAME).await else {
        log::info!("NetworkManager is not answering");
        return Snapshot::default();
    };
    let active_paths = manager_props
        .get("ActiveConnections")
        .and_then(|v| Vec::<zbus::OwnedObjectPath>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.as_str().to_string())
        .collect();
    let (devices, actives) = futures_util::join!(
        paths(conn, MANAGER_PATH, NAME, "GetDevices"),
        get_all_of(conn, active_paths, ACTIVE_IFACE),
    );
    let (known_ssids, mut vpns) = known_and_vpns(saved);

    let mut active_vpns = HashSet::new();
    let primary_active = object(&manager_props, "PrimaryConnection");
    let mut primary = None;
    let mut profile_of = HashMap::new();
    for (path, props) in actives {
        if let Some(conn_path) = object(&props, "Connection") {
            profile_of.insert(path.clone(), conn_path.clone());
            if uint(&props, "State") == 2 {
                active_vpns.insert(conn_path);
            }
        }
        if primary_active.as_deref() == Some(path.as_str()) {
            primary = props
                .get("Devices")
                .and_then(|v| Vec::<zbus::OwnedObjectPath>::try_from(v.clone()).ok())
                .and_then(|d| d.first().map(|p| p.as_str().to_string()));
        }
    }
    for vpn in &mut vpns {
        vpn.active = active_vpns.contains(&vpn.conn_path);
    }

    let known_ssids = &known_ssids;
    let hotspots = hotspot_profiles(saved);
    let (hotspots, profile_of) = (&hotspots, &profile_of);
    let mut out: Vec<Device> = join_all(devices.into_iter().map(|path| async move {
        let props = get_all(conn, &path, DEVICE_IFACE).await?;
        let kind = uint(&props, "DeviceType");
        if kind != 1 && kind != 2 {
            return None;
        }
        let wifi = kind == 2;
        let link_iface = if wifi { WIRELESS_IFACE } else { WIRED_IFACE };
        let ip4_path = object(&props, "Ip4Config");
        let read_if = |p: Option<String>, iface| async move {
            match p {
                Some(p) => get_all(conn, &p, iface).await,
                None => None,
            }
        };
        let (link, ip4, stats, ap_paths) = futures_util::join!(
            get_all(conn, &path, link_iface),
            read_if(ip4_path, IP4_IFACE),
            get_all(conn, &path, STATS_IFACE),
            async {
                if wifi {
                    paths(conn, &path, WIRELESS_IFACE, "GetAllAccessPoints").await
                } else {
                    vec![]
                }
            },
        );
        let aps = get_all_of(conn, ap_paths, AP_IFACE).await;
        let active_ap = wifi
            .then(|| link.as_ref().and_then(|p| object(p, "ActiveAccessPoint")))
            .flatten();
        let dhcp = object(&props, "Dhcp4Config").is_some();
        let hotspot = object(&props, "ActiveConnection")
            .and_then(|a| profile_of.get(&a))
            .and_then(|c| hotspots.get(c))
            .cloned();
        Some(fold_device(&DeviceProps {
            path: path.clone(),
            device: props,
            wired: (!wifi).then(|| link.clone().unwrap_or_default()),
            wireless: wifi.then(|| link.clone().unwrap_or_default()),
            ip4,
            stats,
            aps,
            known_ssids: known_ssids.clone(),
            active_ap,
            dhcp,
            hotspot,
        }))
    }))
    .await
    .into_iter()
    .flatten()
    .collect();
    for device in &mut out {
        let was = prev.devices.iter().find(|d| d.path == device.path);
        device.connected_since = match (device.state, was) {
            (State::Connected, Some(d)) if d.state == State::Connected => d.connected_since,
            (State::Connected, _) => Some(Instant::now()),
            _ => None,
        };
    }
    let names = shell_names(&out.iter().map(|d| d.kind).collect::<Vec<_>>());
    for (device, name) in out.iter_mut().zip(names) {
        device.shell_name = name;
    }
    Snapshot {
        devices: out,
        vpns,
        primary,
        nm_running: true,
        connectivity: Connectivity::from_code(uint(&manager_props, "Connectivity")),
        check_uri: text(&manager_props, "ConnectivityCheckUri"),
    }
}
