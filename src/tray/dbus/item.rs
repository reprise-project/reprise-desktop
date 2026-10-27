use super::{ITEM_IFACE, Icons, Sizes, lock, sizes_now, uncached_proxy};
use crate::tray::{ItemIcon, TrayItem, clip, decode_named_at, item_pixmaps};
use reprisekit::controls::tooltip::TIP_CHARS;
use reprisekit::icon::IconSource;
use std::path::PathBuf;
use std::sync::Arc;
use zbus::as_variant;
use zbus::message::Body;
use zbus::{Connection, Proxy};
use zbus::{DynamicType, OwnedObjectPath};

async fn prop<T>(proxy: &Proxy<'_>, name: &str) -> Option<T>
where
    T: serde::de::DeserializeOwned + zbus::Type,
{
    proxy.get_property::<T>(name).await.ok()
}

pub(super) type Pixmaps<'a> = Vec<(i32, i32, &'a [u8])>;

pub(super) type ToolTip<'a> = (&'a str, Pixmaps<'a>, &'a str, &'a str);

/// One property's `Properties.Get` reply body, for decoding in place with [`as_variant::Deserialize`].
/// [`prop`]'s `OwnedVariant` holds each `ay` byte as its own `Variant`: gigabytes for a 128 MiB pixmap.
async fn prop_body(proxy: &Proxy<'_>, name: &str) -> Option<Body> {
    let reply = proxy
        .connection()
        .call_method(
            Some(proxy.destination()),
            proxy.path(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(proxy.interface(), name),
        )
        .await
        .ok()?;
    Some(reply.body())
}

/// `name` from the theme (and the item's `IconThemePath`) at `sizes`, unless `pixmaps` stand in.
fn named(
    icons: &Icons,
    name: Option<&str>,
    pixmaps: &[IconSource],
    theme_path: Option<&PathBuf>,
    sizes: &[i32],
) -> Arc<[IconSource]> {
    match name {
        Some(name) if pixmaps.is_empty() => {
            let extra: Vec<PathBuf> = theme_path.into_iter().cloned().collect();
            decode_named_at(&mut lock(icons), name, &extra, sizes).into()
        }
        _ => Arc::from([]),
    }
}

/// Decode the item's icon, attention icon and overlay names at `sizes`; true when any changed.
pub(super) fn decode_names(icons: &Icons, item: &mut TrayItem, sizes: &[i32]) -> bool {
    let theme = item.icon_theme_path.clone();
    let main = named(
        icons,
        item.icon_name.as_deref(),
        &item.pixmaps,
        theme.as_ref(),
        sizes,
    );
    let mut changed = main != item.named;
    item.named = main;
    for icon in [&mut item.attention, &mut item.overlay] {
        let n = named(
            icons,
            icon.name.as_deref(),
            &icon.pixmaps,
            theme.as_ref(),
            sizes,
        );
        changed |= n != icon.named;
        icon.named = n;
    }
    changed
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
    let title = clip(title.unwrap_or_default(), TIP_CHARS);
    let tooltip = tip
        .map(|t| t.2)
        .filter(|s| !s.trim().is_empty())
        .map_or_else(|| title.clone(), |s| s.chars().take(TIP_CHARS).collect());
    (title, tooltip)
}

pub(super) async fn fetch_item(
    conn: &Connection,
    icons: &Icons,
    sizes: &Sizes,
    bus: &str,
    path: &str,
) -> Option<TrayItem> {
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
    let (title, tooltip) = item_texts(title, tip);
    // Read after the properties, so a size change during the read is decoded by it.
    let sizes = sizes_now(sizes);
    let key = item_key(
        sni_id.as_deref(),
        pid.and_then(program_name).as_deref(),
        bus,
    );
    let mut item = TrayItem {
        id: format!("{bus}{path}"),
        key,
        service: bus.to_string(),
        path: path.to_string(),
        title,
        tooltip,
        icon_name: icon_ref(icon_name),
        icon_theme_path: icon_ref(theme_path).map(PathBuf::from),
        pixmaps: item_pixmaps(&pixmaps.unwrap_or_default(), &sizes).into(),
        named: Arc::from([]),
        menu_path: menu.map(|p| p.as_str().to_string()).filter(|p| p != "/"),
        passive: status
            .as_deref()
            .is_some_and(|s| s.eq_ignore_ascii_case("Passive")),
        pid,
        needs_attention: status.is_some_and(|s| s.eq_ignore_ascii_case("NeedsAttention")),
        attention: ItemIcon {
            name: icon_ref(attention_name),
            pixmaps: item_pixmaps(&pixmaps_of(&attention_pixmaps).unwrap_or_default(), &sizes)
                .into(),
            named: Arc::from([]),
        },
        overlay: ItemIcon {
            name: icon_ref(overlay_name),
            pixmaps: item_pixmaps(&pixmaps_of(&overlay_pixmaps).unwrap_or_default(), &sizes).into(),
            named: Arc::from([]),
        },
    };
    decode_names(icons, &mut item, &sizes);
    Some(item)
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
