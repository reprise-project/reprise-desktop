//! Who sent a D-Bus message, checked against the current owner of a
//! well-known name. The bus stamps every message with its sender's unique
//! name, so that comparison cannot be forged.
//!
//! zbus's own checks are not enough on the system bus. A match rule's
//! well-known `sender` is not checked on our side, and the bus policy lets
//! any local peer send a signal straight to our unique name, which skips
//! the bus-side match. A proxy's signal stream does compare the sender,
//! but it follows the owner through `NameOwnerChanged` messages it takes
//! from any peer.
//!
//! [`a11y`] reports keys to the accessibility bus's registry for a screen
//! reader. [`fprint`] asks fprintd whether a user has a finger enrolled.
//! [`instance`] keeps a program to one running copy.
//! [`appearance`] follows the compositor's look from a running program.
//! [`filetransfer`] hands dragged files to and from sandboxed programs.
//! [`tray`] asks the compositor to open the tray's Customize dialog, or
//! to turn off for a critical battery.
//! The last four share a program's one blocking session bus connection
//! ([`session`]).

pub mod a11y;
pub mod appearance;
pub mod filetransfer;
pub mod fprint;
pub mod instance;
pub mod tray;

use futures_util::future::ready;
use futures_util::stream::{BoxStream, SelectAll, StreamExt};
use std::sync::Mutex;
use std::time::Duration;
use zbus::{Connection, Message, MessageStream, OwnedMatchRule};

/// How long a method call on [`session`]'s connection waits for its reply;
/// a hung service never holds a caller's thread past this.
pub const METHOD_TIMEOUT: Duration = Duration::from_secs(5);

/// The program's blocking session bus connection, made by the first call that
/// finds the bus.
pub fn session() -> zbus::Result<zbus::blocking::Connection> {
    static SESSION: Mutex<Option<zbus::blocking::Connection>> = Mutex::new(None);
    let mut conn = SESSION.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = conn.as_ref() {
        return Ok(c.clone());
    }
    let c = zbus::blocking::connection::Builder::session()?
        .method_timeout(METHOD_TIMEOUT)
        .build()?;
    *conn = Some(c.clone());
    Ok(c)
}

/// The bus itself: the sender of `NameOwnerChanged`, a name no peer can own.
const DBUS: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";

/// Whether `sender` is `owner`. No owner (nothing holds the name) matches
/// no sender.
pub fn sent_by(sender: Option<&str>, owner: Option<&str>) -> bool {
    matches!((sender, owner), (Some(s), Some(o)) if s == o)
}

/// Whether a signal is a broadcast from `owner`. The services this guards
/// broadcast their signals, so one addressed to us came from someone else.
pub fn broadcast_by(sender: Option<&str>, destination: Option<&str>, owner: Option<&str>) -> bool {
    destination.is_none() && sent_by(sender, owner)
}

/// The unique name that owns `name` now, None when nothing does.
pub async fn name_owner(conn: &Connection, name: &str) -> Option<String> {
    conn.call_method(Some(DBUS), DBUS_PATH, Some(DBUS), "GetNameOwner", &(name,))
        .await
        .and_then(|m| m.body().deserialize::<String>())
        .ok()
}

#[derive(Debug)]
pub enum Signal {
    /// The name changed hands, or its owner left the bus.
    OwnerChanged,
    /// A signal matching one of the rules, broadcast by the name's owner.
    Message(Message),
}

enum Item {
    Owner(Option<String>),
    Message(Message),
}

/// The owner a `NameOwnerChanged(name, old, new)` message announces for
/// `name`: Some(None) when it left the bus, None when the message is not a
/// broadcast from the bus or is about another name.
fn owner_change(
    sender: Option<&str>,
    destination: Option<&str>,
    name: &str,
    args: (&str, &str, &str),
) -> Option<Option<String>> {
    let (changed, _old, new) = args;
    if !broadcast_by(sender, destination, Some(DBUS)) || changed != name {
        return None;
    }
    Some((!new.is_empty()).then(|| new.to_string()))
}

fn owner_item(m: Message, name: &str) -> Option<Item> {
    let hdr = m.header();
    let args: (String, String, String) = m.body().deserialize().ok()?;
    owner_change(
        hdr.sender().map(|s| s.as_str()),
        hdr.destination().map(|d| d.as_str()),
        name,
        (&args.0, &args.1, &args.2),
    )
    .map(Item::Owner)
}

/// Every signal matching `rules` that `name`'s current owner broadcast,
/// plus [`Signal::OwnerChanged`] each time the name changes hands. The
/// owner is read once and then followed through the bus's
/// `NameOwnerChanged`; while nothing owns the name, every signal is
/// dropped. A rule that cannot be added is logged and left out, and so
/// are stream errors, which carry no sender to check.
///
/// Each rule gets its own zbus queue, which blocks the connection's reader
/// when full: poll the stream continuously.
pub async fn signals(
    conn: &Connection,
    name: &'static str,
    rules: Vec<OwnedMatchRule>,
) -> BoxStream<'static, Signal> {
    let mut items: SelectAll<BoxStream<'static, Item>> = SelectAll::new();
    // Subscribed before the owner is read, so a change in between is seen.
    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DBUS)
        .and_then(|b| b.path(DBUS_PATH))
        .and_then(|b| b.interface(DBUS))
        .and_then(|b| b.member("NameOwnerChanged"))
        .and_then(|b| b.add_arg(name))
        .map(|b| b.build());
    let owner_stream = match owner_rule {
        Ok(rule) => MessageStream::for_match_rule(rule, conn, None).await,
        Err(e) => Err(e),
    };
    match owner_stream {
        Ok(s) => items.push(
            s.filter_map(move |m| ready(m.ok().and_then(|m| owner_item(m, name))))
                .boxed(),
        ),
        Err(e) => log::warn!("cannot follow the owner of {name}: {e}"),
    }
    let owner = name_owner(conn, name).await;
    for rule in rules {
        let shown = rule.to_string();
        match MessageStream::for_match_rule(rule, conn, None).await {
            Ok(s) => items.push(s.filter_map(|m| ready(m.ok().map(Item::Message))).boxed()),
            Err(e) => log::warn!("cannot watch {shown}: {e}"),
        }
    }
    items
        .scan(owner, move |owner, item| {
            let out = match item {
                Item::Owner(new) => {
                    *owner = new;
                    Some(Signal::OwnerChanged)
                }
                Item::Message(m) => {
                    let accepted = {
                        let hdr = m.header();
                        let sender = hdr.sender().map(|s| s.as_str());
                        let accepted = broadcast_by(
                            sender,
                            hdr.destination().map(|d| d.as_str()),
                            owner.as_deref(),
                        );
                        if !accepted {
                            log::debug!("dropping a signal from {sender:?}, which is not {name}");
                        }
                        accepted
                    };
                    accepted.then_some(Signal::Message(m))
                }
            };
            ready(Some(out))
        })
        .filter_map(ready)
        .boxed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_owner_is_the_sender() {
        assert!(sent_by(Some(":1.5"), Some(":1.5")));
        assert!(!sent_by(Some(":1.9"), Some(":1.5")), "another peer");
        assert!(!sent_by(Some(":1.5"), None), "nothing owns the name");
        assert!(!sent_by(None, Some(":1.5")), "no sender header");
    }

    #[test]
    fn a_signal_counts_only_as_the_owners_broadcast() {
        assert!(broadcast_by(Some(":1.5"), None, Some(":1.5")));
        assert!(
            !broadcast_by(Some(":1.9"), None, Some(":1.5")),
            "spoofed sender"
        );
        assert!(
            !broadcast_by(Some(":1.5"), Some(":1.7"), Some(":1.5")),
            "addressed to us"
        );
        assert!(
            !broadcast_by(Some(":1.5"), None, None),
            "nothing owns the name"
        );
    }

    #[test]
    fn the_owner_follows_only_the_bus() {
        let nm = "org.freedesktop.NetworkManager";
        assert_eq!(
            owner_change(Some(DBUS), None, nm, (nm, ":1.5", ":1.6")),
            Some(Some(":1.6".into()))
        );
        assert_eq!(
            owner_change(Some(DBUS), None, nm, (nm, ":1.5", "")),
            Some(None),
            "the owner left the bus"
        );
        assert_eq!(
            owner_change(Some(":1.9"), None, nm, (nm, ":1.5", ":1.9")),
            None,
            "a peer claiming to be the bus"
        );
        assert_eq!(
            owner_change(Some(DBUS), Some(":1.7"), nm, (nm, ":1.5", ":1.9")),
            None,
            "addressed to us"
        );
        assert_eq!(
            owner_change(Some(DBUS), None, nm, ("org.example", "", ":1.9")),
            None,
            "another name"
        );
    }
}
