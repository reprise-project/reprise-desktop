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
//! [`charge`] reads and sets UPower's battery charge limit.
//! [`a11y`] reports keys to the accessibility bus's registry for a screen
//! reader. [`fprint`] asks fprintd whether a user has a finger enrolled.
//! [`instance`] keeps a program to one running copy.
//! [`appearance`] follows the compositor's look from a running program.
//! [`filetransfer`] hands dragged files to and from sandboxed programs.
//! [`tray`] asks the compositor to open the tray's Customize dialog, or
//! to turn off for a critical battery, and follows which windows stopped
//! answering. [`polkit`] asks whether a password
//! prompt was dismissed. [`launcher`] shows a program's progress on its
//! task button. [`journal`] starts the logger with journal priorities.
//! The last six share a program's one session bus connection ([`session`]),
//! as do [`units`], which lists the programs the systemd user manager starts
//! with the session, and [`remote`], which reads the Remote Desktop server and
//! turns its user unit on or off. Synchronous callers [`wait`] on it.
//! [`bluez`] lists BlueZ's paired devices and follows whether one is connected.
//! [`bolt`] lists and follows boltd's Thunderbolt devices and authorizes, enrolls or forgets one.
//! [`airplane`] turns every radio off and back on, Bluetooth through [`rfkill`].
//! [`kdeconnect`] follows the phones KDE Connect reaches.
//! [`locale`] reads and sets the system locale and sets a user's language in AccountsService.
//! [`localsearch`] asks the file indexer which files hold some words.
//! [`searchprovider`] asks a program's search provider for results and opens one.
//! [`error`] reads the name and message of an error reply.
//! [`desklock`] follows the compositor's desktop lock on `org.freedesktop.ScreenSaver`.
//! [`peer`] tells which process and which app a session bus peer is, and whether it runs in a
//! Flatpak sandbox.
//! [`mutex`] locks a mutex even when its holder panicked.
//! [`login`] asks the system manager whether reprise-login runs.
//! [`logind`] reads logind's session of a process and its user's display session.
//! [`testsession`] gives a test binary a session of its own.
//! [`secrets`] looks up and stores passwords in the Secret Service.
//! [`eds`] reads a day's events from evolution-data-server's calendars.
//! [`bounded`] decodes what another peer sends within size caps, never as a `Variant` tree.

pub mod a11y;
pub mod airplane;
pub mod appearance;
pub mod bluez;
pub mod bolt;
pub mod bounded;
pub mod charge;
pub mod desklock;
pub mod eds;
pub mod error;
pub mod filetransfer;
pub mod fprint;
pub mod instance;
pub mod journal;
pub mod kdeconnect;
pub mod launcher;
pub mod locale;
pub mod localsearch;
pub mod login;
pub mod logind;
pub mod mutex;
pub mod notify;
pub mod peer;
pub mod permissions;
pub mod polkit;
pub mod remote;
pub mod rfkill;
pub mod searchprovider;
pub mod secrets;
pub mod testsession;
pub mod tray;
pub mod units;

use crate::mutex::lock;
use futures_util::future::ready;
use futures_util::stream::{BoxStream, SelectAll, Stream, StreamExt};
use std::sync::Mutex;
use std::time::Duration;
use zbus::{Connection, Message, MessageStream, OwnedMatchRule};

/// How long a method call on [`session`]'s connection waits for its reply;
/// a hung service never holds a caller's thread past this.
pub const METHOD_TIMEOUT: Duration = Duration::from_secs(5);

/// The program's session bus connection, made by the first call that finds
/// the bus.
pub fn session() -> zbus::Result<Connection> {
    static SESSION: Mutex<Option<Connection>> = Mutex::new(None);
    let mut conn = lock(&SESSION);
    if let Some(c) = conn.as_ref() {
        return Ok(c.clone());
    }
    let c = connect(zbus::connection::Builder::session().method_timeout(METHOD_TIMEOUT))?;
    *conn = Some(c.clone());
    Ok(c)
}

/// The program's system bus connection, made by the first call that finds the bus.
pub fn system() -> zbus::Result<Connection> {
    static SYSTEM: Mutex<Option<Connection>> = Mutex::new(None);
    let mut conn = lock(&SYSTEM);
    if let Some(c) = conn.as_ref() {
        return Ok(c.clone());
    }
    let c = connect(zbus::connection::Builder::system().method_timeout(METHOD_TIMEOUT))?;
    *conn = Some(c.clone());
    Ok(c)
}

/// Builds a connection whose I/O runs on a helper thread of its own, so any
/// thread can [`wait`] on it, an async task included.
pub fn connect(builder: zbus::connection::Builder<'_>) -> zbus::Result<Connection> {
    std::thread::scope(|s| {
        s.spawn(|| zbus::block_on(builder.build()))
            .join()
            .unwrap_or_else(|p| std::panic::resume_unwind(p))
    })
}

/// Runs `future` to completion on the calling thread. Unlike
/// [`zbus::block_on`], it may be called from inside an async task, which
/// it blocks meanwhile; the connection awaited must be one from [`connect`].
pub fn wait<F: Future>(future: F) -> F::Output {
    futures_lite::future::block_on(future)
}

/// The items of `stream`, each [`wait`]ed for, for a thread that follows
/// signals in a plain loop.
pub fn blocking_iter<S: Stream + Unpin>(mut stream: S) -> impl Iterator<Item = S::Item> {
    std::iter::from_fn(move || wait(stream.next()))
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

/// Whether a signal came from `owner`: broadcast, or when `addressed`, also sent to us alone.
fn from_owner(
    sender: Option<&str>,
    destination: Option<&str>,
    owner: Option<&str>,
    addressed: bool,
) -> bool {
    sent_by(sender, owner) && (addressed || destination.is_none())
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
    owner_stream(conn, name, rules, false).await
}

/// [`signals`], plus those the owner sent to this connection alone, as a notification server
/// sends a notification's signals to the program that sent it.
pub async fn owner_signals(
    conn: &Connection,
    name: &'static str,
    rules: Vec<OwnedMatchRule>,
) -> BoxStream<'static, Signal> {
    owner_stream(conn, name, rules, true).await
}

async fn owner_stream(
    conn: &Connection,
    name: &'static str,
    rules: Vec<OwnedMatchRule>,
    addressed: bool,
) -> BoxStream<'static, Signal> {
    let mut items: SelectAll<BoxStream<'static, Item>> = SelectAll::new();
    // Subscribed before the owner is read, so a change in between is seen.
    let owner_rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DBUS)
        .path(DBUS_PATH)
        .interface(DBUS)
        .member("NameOwnerChanged")
        .add_arg(name)
        .build();
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
                        let accepted = from_owner(
                            sender,
                            hdr.destination().map(|d| d.as_str()),
                            owner.as_deref(),
                            addressed,
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
    fn an_owner_signal_may_be_addressed_to_us_only_where_asked() {
        assert!(from_owner(Some(":1.5"), None, Some(":1.5"), false));
        assert!(!from_owner(Some(":1.5"), Some(":1.7"), Some(":1.5"), false));
        assert!(from_owner(Some(":1.5"), Some(":1.7"), Some(":1.5"), true));
        assert!(
            !from_owner(Some(":1.9"), Some(":1.7"), Some(":1.5"), true),
            "addressed to us by another peer"
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
