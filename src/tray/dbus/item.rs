use super::decoders::{Decoders, Limits};
use super::watcher::MAX_REGISTRATIONS;
use super::{ITEM_IFACE, Icons, Sizes, sizes_now, uncached_proxy};
use crate::tray::{IconFiles, ItemIcon, TrayItem, decode_item_icons, item_pixmaps};
use reprisebus::bounded;
use reprisebus::peer::{AppIdentity, Files, Identities};
use reprisefs::confined::Confined;
use reprisefs::names::shown_line;
use reprisekit::controls::tooltip::TIP_CHARS;
use reprisekit::icon::IconSource;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use zbus::as_variant;
use zbus::message::Body;
use zbus::{Connection, Proxy};
use zbus::{DynamicType, OwnedObjectPath};

async fn prop<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: serde::de::DeserializeOwned + zbus::Type,
{
    bounded::property::<T>(proxy, name).await.ok()
}

pub(super) type Pixmaps<'a> = Vec<(i32, i32, &'a [u8])>;

pub(super) type ToolTip<'a> = (&'a str, Pixmaps<'a>, &'a str, &'a str);

/// One property's reply body, for decoding pixmaps in place with [`as_variant::Deserialize`].
async fn prop_body(proxy: &Proxy<'_>, name: &str) -> Option<Body> {
    bounded::property_body(proxy, name).await.ok()
}

/// How long an item's names may wait for a decoding thread, and then take to read and decode;
/// past either the item keeps the icons it had.
const NAMED_WAIT: Duration = Duration::from_secs(10);
const NAMED_WITHIN: Duration = Duration::from_secs(2);

/// Two threads for tray items' names and notification pictures, so a hung read leaves one; the
/// queue holds one fetch per registered item.
pub(super) static DECODERS: LazyLock<Decoders> = LazyLock::new(|| {
    Decoders::new(Limits {
        threads: 2,
        queue: MAX_REGISTRATIONS,
        per_app: 4,
        wait: NAMED_WAIT,
        within: NAMED_WITHIN,
    })
});

/// Decode the item's icon, attention icon and overlay names at `sizes`, each unless pixmaps
/// stand in, on the shared [`DECODERS`] as the item's app. False when they weren't decoded.
pub(super) async fn decode_names(
    icons: &Icons,
    files: IconFiles,
    item: &mut TrayItem,
    sizes: &[i32],
) -> bool {
    let names = [
        named(&item.icon_name, &item.pixmaps),
        named(&item.attention.name, &item.attention.pixmaps),
        named(&item.overlay.name, &item.overlay.pixmaps),
    ];
    let decoded = match names.iter().all(Option::is_none) {
        true => Default::default(),
        false => {
            let (icons, theme, sizes) =
                (icons.clone(), item.icon_theme_path.clone(), sizes.to_vec());
            let decode = move || {
                let names = names.each_ref().map(Option::as_deref);
                decode_item_icons(&icons, &files, names, theme.as_deref(), &sizes)
            };
            match DECODERS.run(&item.key, decode).await {
                Some(decoded) => decoded,
                None => return false,
            }
        }
    };
    let [main, attention, overlay] = decoded.map(Arc::from);
    item.named = main;
    item.attention.named = attention;
    item.overlay.named = overlay;
    true
}

/// `name` when it is to be decoded: set, and no pixmaps standing in.
pub(super) fn named(name: &Option<String>, pixmaps: &[IconSource]) -> Option<String> {
    name.clone().filter(|_| pixmaps.is_empty())
}

/// The name keys the icon cache, so the `PATH_MAX` cap bounds it.
pub(super) fn icon_ref(name: Option<String>) -> Option<String> {
    name.filter(|s| !s.is_empty() && s.len() <= libc::PATH_MAX as usize)
}

fn pixmaps_of(body: &Option<Body>) -> Option<Pixmaps<'_>> {
    body.as_ref()
        .and_then(|b| b.deserialize::<as_variant::Deserialize<Pixmaps>>().ok())
        .map(|v| v.0)
}

pub(super) fn item_texts(title: Option<String>, tip: Option<ToolTip>) -> (String, String) {
    let title = shown_line(&title.unwrap_or_default(), TIP_CHARS);
    let tooltip = tip
        .map(|t| shown_line(t.2, TIP_CHARS))
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| title.clone());
    (title, tooltip)
}

/// The item at `bus` `path`, and whether its named icons were decoded: when not, it has none.
pub(super) async fn fetch_item(
    conn: &Connection,
    icons: &Icons,
    sizes: &Sizes,
    identities: &Identities,
    bus: &str,
    path: &str,
) -> Option<(TrayItem, bool)> {
    let (mut item, files, sizes) = read_item(conn, sizes, identities, bus, path).await?;
    let decoded = decode_names(icons, files, &mut item, &sizes).await;
    Some((item, decoded))
}

/// The item's properties as a [`TrayItem`] without its named icons, where those may be opened
/// from, and the sizes read for. Every reply body is dropped on return.
async fn read_item(
    conn: &Connection,
    sizes: &Sizes,
    identities: &Identities,
    bus: &str,
    path: &str,
) -> Option<(TrayItem, IconFiles, Vec<i32>)> {
    let proxy = uncached_proxy(conn, bus, path, ITEM_IFACE).await.ok()?;
    // Concurrent, so a silent peer costs one METHOD_TIMEOUT, not one per property.
    let (
        sni_id,
        title,
        icon_name,
        theme_path,
        pixmaps,
        menu,
        status,
        tip,
        pid,
        attention_name,
        attention_pixmaps,
        overlay_name,
        overlay_pixmaps,
        identity,
    ) = futures_util::join!(
        prop::<String>(&proxy, "Id"),
        prop::<String>(&proxy, "Title"),
        prop::<String>(&proxy, "IconName"),
        prop::<String>(&proxy, "IconThemePath"),
        prop_body(&proxy, "IconPixmap"),
        prop::<OwnedObjectPath>(&proxy, "Menu"),
        prop::<String>(&proxy, "Status"),
        prop_body(&proxy, "ToolTip"),
        owner_pid(conn, bus),
        prop::<String>(&proxy, "AttentionIconName"),
        prop_body(&proxy, "AttentionIconPixmap"),
        prop::<String>(&proxy, "OverlayIconName"),
        prop_body(&proxy, "OverlayIconPixmap"),
        identities.of(conn, bus),
    );
    let pixmaps = pixmaps_of(&pixmaps);
    let tip = tip
        .as_ref()
        .and_then(|b| b.deserialize::<as_variant::Deserialize<ToolTip>>().ok())
        .map(|v| v.0);
    // Not one property answered: the peer is gone, or never was an SNI.
    // `IconThemePath` is left out, plenty of well-behaved items omit it.
    if title.is_none() && icon_name.is_none() && pixmaps.is_none() && menu.is_none() {
        log::warn!("tray: {bus}{path} answered none of Title/IconName/IconPixmap/Menu");
        return None;
    }
    // Read after the properties, so a size change during the read is decoded by it.
    let sizes = sizes_now(sizes);
    let pixmaps: Arc<[IconSource]> = item_pixmaps(&pixmaps.unwrap_or_default(), &sizes).into();
    let attention = ItemIcon {
        name: icon_ref(attention_name),
        pixmaps: item_pixmaps(&pixmaps_of(&attention_pixmaps).unwrap_or_default(), &sizes).into(),
        named: Arc::from([]),
    };
    let overlay = ItemIcon {
        name: icon_ref(overlay_name),
        pixmaps: item_pixmaps(&pixmaps_of(&overlay_pixmaps).unwrap_or_default(), &sizes).into(),
        named: Arc::from([]),
    };
    let icon_name = icon_ref(icon_name);
    let has_names = [
        named(&icon_name, &pixmaps),
        named(&attention.name, &attention.pixmaps),
        named(&overlay.name, &overlay.pixmaps),
    ]
    .iter()
    .any(Option::is_some);
    // A sandboxed app's item is kept and listed under its own app, whatever `Id` and `Title` say.
    let (key, title, files, saved_icon) = match &identity {
        AppIdentity::Host { .. } => {
            let program = pid.and_then(program_name);
            let key = item_key(sni_id.as_deref(), program.as_deref(), bus);
            (key, title, IconFiles::Host, icon_name.clone())
        }
        AppIdentity::Sandboxed { app_id } => {
            let entry = super::notifications::desktop_entry(app_id);
            let files = match has_names {
                true => sandbox_files(identities.files(conn, bus).await, app_id),
                false => IconFiles::Refused,
            };
            let (name, icon) = entry.map_or((app_id.clone(), None), |e| (e.name, e.icon));
            (app_id.clone(), Some(name), files, icon)
        }
        AppIdentity::Unknown => (item_key(None, None, bus), None, IconFiles::Refused, None),
    };
    let (title, tooltip) = item_texts(title, tip);
    let item = TrayItem {
        id: format!("{bus}{path}"),
        key,
        service: bus.to_string(),
        path: path.to_string(),
        title,
        tooltip,
        icon_name,
        icon_theme_path: icon_ref(theme_path).map(PathBuf::from),
        saved_icon,
        pixmaps,
        named: Arc::from([]),
        menu_path: menu.map(|p| p.as_str().to_string()).filter(|p| p != "/"),
        passive: status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("Passive")),
        pid,
        needs_attention: status.is_some_and(|s| s.eq_ignore_ascii_case("NeedsAttention")),
        attention,
        overlay,
    };
    Some((item, files, sizes))
}

/// Where a sandboxed item of `app_id` opens files from: only as that same app, read again now.
fn sandbox_files(files: Files, app_id: &str) -> IconFiles {
    match files {
        Files::Flatpak { app_id: a, root } if a == app_id => {
            IconFiles::Confined(Confined::Root(root))
        }
        Files::Snap { app_id: a, snap } if a == app_id => {
            IconFiles::Confined(Confined::snap(&snap))
        }
        _ => IconFiles::Refused,
    }
}

fn item_key(sni_id: Option<&str>, program: Option<&str>, bus: &str) -> String {
    let key = [sni_id, program]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|s| !s.is_empty())
        .unwrap_or(bus);
    key.chars().take(TIP_CHARS).collect()
}

fn program_name(pid: i32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_string()).filter(|s| !s.is_empty())
}

pub(super) async fn owner_pid(conn: &Connection, bus: &str) -> Option<i32> {
    conn.unique_name()?;
    let dbus = zbus::fdo::DBusProxy::new(conn).await.ok()?;
    let name = zbus::names::BusName::try_from(bus).ok()?;
    let pid = dbus.get_connection_unix_process_id(name).await.ok()?;
    i32::try_from(pid).ok()
}

pub(super) async fn call_item<B>(conn: &Connection, item: &TrayItem, method: &str, body: &B)
where
    B: serde::Serialize + DynamicType,
{
    if let Ok(proxy) = uncached_proxy(conn, &item.service, &item.path, ITEM_IFACE).await
        && let Err(e) = proxy.call_method(method, body).await
    {
        log::debug!("tray: {method} on {} failed: {e}", item.id);
    }
}

/// Hand the item an xdg-activation token so the window its next `Activate`
/// or menu row shows or raises can take focus. Items without the method ignore it.
pub(super) async fn provide_token(conn: &Connection, item: &TrayItem, token: &str) {
    call_item(conn, item, "ProvideXdgActivationToken", &(token,)).await;
}
