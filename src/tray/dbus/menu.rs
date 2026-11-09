use super::uncached_proxy;
use crate::tray::{DEFAULT_PROP, MenuNode, TrayItem};
use reprisebus::bounded::{self, Dict, Value};
use serde::de::{self, Deserialize, Deserializer, IgnoredAny, SeqAccess, Visitor};
use zbus::{Connection, Message, Signature, Type, Variant};

const MENU_IFACE: &str = "com.canonical.dbusmenu";

/// A peer's menu is cut to these as it is decoded, so label widths and the menu canvas stay
/// bounded. Only the first level is read, as `GetLayout` asks for; deeper rows are skipped unread.
pub(super) const LABEL_CHARS: usize = 255;
pub(super) const MENU_ROWS: usize = 256;

fn node_of(id: i32, props: &Dict) -> MenuNode {
    let mut node = MenuNode {
        id,
        label: String::new(),
        separator: false,
        enabled: true,
        visible: true,
        checked: None,
        radio: false,
        has_children: false,
        default: false,
    };
    let mut toggle_type: Option<&str> = None;
    let mut toggle_state: Option<i64> = None;
    for (k, v) in props.iter() {
        match (k, v) {
            ("label", Value::Str(s)) => node.label = s.chars().take(LABEL_CHARS).collect(),
            ("enabled", Value::Bool(b)) => node.enabled = *b,
            ("visible", Value::Bool(b)) => node.visible = *b,
            ("type", Value::Str(s)) => node.separator = s == "separator",
            ("toggle-type", Value::Str(s)) => toggle_type = Some(s),
            ("toggle-state", Value::Int(i)) => toggle_state = Some(*i),
            ("children-display", Value::Str(s)) => node.has_children = s == "submenu",
            (DEFAULT_PROP, Value::Bool(b)) => node.default = *b,
            _ => {}
        }
    }
    if matches!(toggle_type, Some("checkmark") | Some("radio")) {
        node.checked = Some(toggle_state == Some(1));
        node.radio = toggle_type == Some("radio");
    }
    node
}

/// A row: `(ia{sv}av)`, its own children skipped unread.
struct Row(MenuNode);

impl Type for Row {
    const SIGNATURE: &'static Signature = <(i32, Dict, Vec<Variant<'_>>)>::SIGNATURE;
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Row;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a menu row")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Row, A::Error> {
                let missing = |n| de::Error::invalid_length(n, &V);
                let id: i32 = seq.next_element()?.ok_or_else(|| missing(0))?;
                let props: Dict = seq.next_element()?.ok_or_else(|| missing(1))?;
                seq.next_element::<IgnoredAny>()?;
                Ok(Row(node_of(id, &props)))
            }
        }
        d.deserialize_tuple(3, V)
    }
}

/// A child of the reply: a variant holding a [`Row`], else nothing.
struct Child(Option<MenuNode>);

impl<'de> Deserialize<'de> for Child {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Child;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a menu child variant")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Child, A::Error> {
                let sig: Signature = seq
                    .next_element()?
                    .ok_or_else(|| de::Error::invalid_length(0, &V))?;
                if sig == *Row::SIGNATURE {
                    Ok(Child(seq.next_element::<Row>()?.map(|r| r.0)))
                } else {
                    seq.next_element::<IgnoredAny>()?;
                    Ok(Child(None))
                }
            }
        }
        d.deserialize_any(V)
    }
}

/// The first [`MENU_ROWS`] well-formed children, in order; the rest are skipped unread.
struct Children(Vec<MenuNode>);

impl Type for Children {
    const SIGNATURE: &'static Signature = <Vec<Variant<'_>>>::SIGNATURE;
}

impl<'de> Deserialize<'de> for Children {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Children;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("menu children")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Children, A::Error> {
                let mut out = Vec::new();
                loop {
                    if out.len() < MENU_ROWS {
                        match seq.next_element::<Child>()? {
                            Some(Child(node)) => out.extend(node),
                            None => break,
                        }
                    } else if seq.next_element::<IgnoredAny>()?.is_none() {
                        break;
                    }
                }
                Ok(Children(out))
            }
        }
        d.deserialize_seq(V)
    }
}

/// The rows of a `GetLayout` reply, `(u(ia{sv}av))`; None for a reply over
/// [`bounded::MAX_BODY`] or of another type.
pub(super) fn parse_layout(reply: &Message) -> Option<Vec<MenuNode>> {
    let (_, (_, _, children)) = bounded::decode::<(u32, (i32, Dict, Children))>(reply).ok()?;
    Some(children.0)
}

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
    parse_layout(&reply)
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
        .call_method("Event", &(node, "clicked", Variant::I32(0), now))
        .await;
}
