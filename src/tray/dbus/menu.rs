use super::uncached_proxy;
use crate::tray::{DEFAULT_PROP, MenuNode, TrayItem};
use std::collections::HashMap;
use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// A peer's menu is cut to these as it is parsed, so label widths and the menu canvas stay
/// bounded.
pub(super) const LABEL_CHARS: usize = 255;
pub(super) const MENU_ROWS: usize = 256;

fn parse_node(v: &OwnedValue) -> Option<MenuNode> {
    let Value::Structure(s) = &**v else {
        return None;
    };
    let fields = s.fields();
    let Value::I32(id) = fields.first()? else {
        return None;
    };
    let Value::Dict(props) = fields.get(1)? else {
        return None;
    };
    let mut node = MenuNode {
        id: *id,
        label: String::new(),
        separator: false,
        enabled: true,
        visible: true,
        checked: None,
        radio: false,
        has_children: false,
        default: false,
    };
    let mut toggle_type: Option<String> = None;
    let mut toggle_state: Option<i32> = None;
    for (k, v) in props.iter() {
        let Value::Str(k) = k else { continue };
        let v = match v {
            Value::Value(inner) => &**inner,
            other => other,
        };
        match (k.as_str(), v) {
            ("label", Value::Str(s)) => node.label = s.chars().take(LABEL_CHARS).collect(),
            ("enabled", Value::Bool(b)) => node.enabled = *b,
            ("visible", Value::Bool(b)) => node.visible = *b,
            ("type", Value::Str(s)) => node.separator = s.as_str() == "separator",
            ("toggle-type", Value::Str(s)) => toggle_type = Some(s.as_str().to_string()),
            ("toggle-state", Value::I32(i)) => toggle_state = Some(*i),
            ("children-display", Value::Str(s)) => node.has_children = s.as_str() == "submenu",
            (DEFAULT_PROP, Value::Bool(b)) => node.default = *b,
            _ => {}
        }
    }
    if matches!(toggle_type.as_deref(), Some("checkmark") | Some("radio")) {
        node.checked = Some(toggle_state == Some(1));
        node.radio = toggle_type.as_deref() == Some("radio");
    }
    Some(node)
}

/// The first `MENU_ROWS` well-formed children of a `GetLayout` reply, in order; malformed children
/// are skipped.
pub(super) fn parse_layout(children: &[OwnedValue]) -> Vec<MenuNode> {
    children
        .iter()
        .filter_map(parse_node)
        .take(MENU_ROWS)
        .collect()
}

type LayoutReply = (u32, (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>));

pub(super) async fn get_layout(
    conn: &Connection,
    item: &TrayItem,
    parent: i32,
) -> Option<Vec<MenuNode>> {
    let menu_path = item.menu_path.as_deref()?;
    let proxy = uncached_proxy(conn, &item.service, menu_path, MENU_IFACE)
        .await
        .ok()?;
    // Lets the app refresh the menu; its answer (whether it changed) is moot
    // since we fetch the layout right after either way.
    let _ = proxy.call_method("AboutToShow", &(parent,)).await;
    let props = [
        "label",
        "enabled",
        "visible",
        "type",
        "toggle-type",
        "toggle-state",
        "children-display",
        DEFAULT_PROP,
    ];
    let reply = proxy
        .call_method("GetLayout", &(parent, 1i32, props.as_slice()))
        .await
        .ok()?;
    let (_, (_, _, children)): LayoutReply = reply.body().deserialize().ok()?;
    Some(parse_layout(&children))
}

pub(super) async fn send_menu_event(conn: &Connection, item: &TrayItem, node: i32) {
    let Some(menu_path) = item.menu_path.as_deref() else {
        return;
    };
    let Ok(proxy) = uncached_proxy(conn, &item.service, menu_path, MENU_IFACE).await else {
        return;
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as u32)
        .unwrap_or(0);
    let _ = proxy
        .call_method("Event", &(node, "clicked", Value::I32(0), now))
        .await;
}
