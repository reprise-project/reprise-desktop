//! Proxies and calls into NetworkManager, and the failures they end in.

use super::*;
use zbus::{ObjectPath, Proxy};

pub(super) fn cmd_device(cmd: &Cmd) -> Option<&str> {
    match cmd {
        Cmd::Connect { device, .. }
        | Cmd::Disconnect(device)
        | Cmd::Disable(device)
        | Cmd::Enable(device)
        | Cmd::Repair(device)
        | Cmd::Rescan(device)
        | Cmd::Hotspot { device, .. }
        | Cmd::Stats { device, .. } => Some(device),
        Cmd::VpnUp(_)
        | Cmd::VpnDown(_)
        | Cmd::LoadVpn(_)
        | Cmd::SaveWireGuard { .. }
        | Cmd::SavePluginVpn { .. }
        | Cmd::DeleteVpn(_)
        | Cmd::Import { .. }
        | Cmd::Quit => None,
    }
}

pub(super) fn failure_title(device: Option<&str>, snapshot: &Snapshot) -> String {
    device
        .and_then(|p| snapshot.devices.iter().find(|d| d.path == p))
        .map(|d| d.shell_name.clone())
        .unwrap_or_else(|| "Network Connections".into())
}

/// Why a command stopped.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    /// The admin password prompt was dismissed: nothing to tell.
    Dismissed,
    Text(String),
}

impl From<String> for Failure {
    fn from(text: String) -> Failure {
        Failure::Text(text)
    }
}

impl From<&str> for Failure {
    fn from(text: &str) -> Failure {
        Failure::Text(text.into())
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Dismissed => f.write_str("the password prompt was dismissed"),
            Failure::Text(text) => f.write_str(text),
        }
    }
}

/// A D-Bus error as the balloon shows it: a sentence for a common NetworkManager error, logged
/// raw, else the text without NM's bus-name prefix.
pub(super) fn clean(e: impl std::fmt::Display) -> Failure {
    let raw = e.to_string();
    match plain(&raw) {
        Some(said) => {
            log::warn!("{raw}");
            Failure::Text(said.into())
        }
        None => Failure::Text(raw.replace("org.freedesktop.NetworkManager.", "")),
    }
}

/// The sentence for a zbus error text (`name: message`) NetworkManager commonly answers.
fn plain(raw: &str) -> Option<&'static str> {
    let (name, msg) = raw.split_once(": ").unwrap_or((raw, ""));
    let short = name.rsplit('.').next().unwrap_or(name);
    match short {
        "PermissionDenied" | "AccessDenied" | "NotAuthorized" => {
            Some(reprisebus::polkit::NOT_ALLOWED)
        }
        _ if msg.to_ascii_lowercase().starts_with("not authorized") => {
            Some(reprisebus::polkit::NOT_ALLOWED)
        }
        "ConnectionNotAvailable" | "UnknownDevice" => Some(NOT_READY),
        _ if msg.contains("because device") => Some(NOT_READY),
        "NoSecrets" => Some("This connection needs a password, and none was given."),
        _ => None,
    }
}

const NOT_READY: &str =
    "The network adapter is not ready. Make sure it is turned on and enabled, then try again.";

/// NetworkManager answers a `PermissionDenied` for a dismissed prompt and a
/// refused password alike; `dismissed` tells them apart.
pub(super) fn auth_failure(e: zbus::Error, dismissed: impl FnOnce() -> bool) -> Failure {
    let refused = |n: &str| n.ends_with(".PermissionDenied");
    match reprisebus::polkit::refusal(&e, refused, dismissed) {
        Some(reprisebus::polkit::Refusal::Dismissed) => Failure::Dismissed,
        Some(reprisebus::polkit::Refusal::TimedOut) => reprisebus::polkit::TIMED_OUT.into(),
        _ => clean(e),
    }
}

pub(super) fn opath(path: &str) -> Result<ObjectPath<'_>, Failure> {
    ObjectPath::try_from(path).map_err(clean)
}

pub(super) async fn proxy<'a>(
    conn: &Connection,
    path: &'a str,
    iface: &'a str,
) -> Result<Proxy<'a>, Failure> {
    proxy_on(conn, NAME, path, iface).await
}

/// A proxy with single `Get`/`Set` calls: a command touches a property once, so caching would only
/// add a match rule and a `GetAll`.
pub(super) async fn proxy_on<'a>(
    conn: &Connection,
    dest: &'a str,
    path: &'a str,
    iface: &'a str,
) -> Result<Proxy<'a>, Failure> {
    let path = opath(path)?;
    zbus::proxy::Builder::<Proxy>::new(conn)
        .destination(dest)
        .path(path)
        .interface(iface)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await
        .map_err(clean)
}

/// A call that may raise the admin password prompt: carries allow-interactive-authorization,
/// without which polkit refuses outright.
pub(super) async fn call_auth<B, R>(
    p: &Proxy<'_>,
    method: &str,
    body: &B,
) -> Result<Option<R>, Failure>
where
    B: zbus::export::serde::Serialize + zbus::DynamicType,
    R: for<'d> zbus::wire::DynamicDeserialize<'d>,
{
    p.call_with_flags(
        method,
        zbus::proxy::MethodFlags::ALLOW_INTERACTIVE_AUTH,
        body,
    )
    .await
    .map_err(|e| auth_failure(e, reprisebus::polkit::dismissed))
}

/// `ActivateConnection(connection, device, specific)`; "/" means "any".
pub(super) async fn activate(
    conn: &Connection,
    connection: &str,
    device: &str,
    ap: &str,
) -> Result<(), Failure> {
    let manager = proxy(conn, MANAGER_PATH, NAME).await?;
    call_auth::<_, zbus::OwnedObjectPath>(
        &manager,
        "ActivateConnection",
        &(opath(connection)?, opath(device)?, opath(ap)?),
    )
    .await
    .map(|_| ())
}
