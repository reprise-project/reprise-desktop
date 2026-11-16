//! `org.freedesktop.FileManager1`: another program's "Show in folder" and "Properties" for files.
//! [`serve`] owns the name and hands each call on as a [`Request`]; the file manager acts on it.

use crate::instance::Handoff;
use crate::peer::HostOnly;
use std::sync::mpsc::{Receiver, Sender, channel};
use zbus::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::Header;

pub const NAME: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    /// `ShowFolders`: each URI is a folder to open.
    Folders,
    /// `ShowItems`: each URI's folder opens with it selected.
    Items,
    /// `ShowItemProperties`: each URI's Properties sheet.
    ItemProperties,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub show: Show,
    pub uris: Vec<String>,
    /// The call's `startup_id` as the activation token, when it carried one.
    pub handoff: Handoff,
}

/// Serves [`NAME`] on the session bus and owns it; each call arrives on the receiver. Err when
/// there is no session bus or another process owns the name.
pub fn serve() -> zbus::Result<(Connection, Receiver<Request>)> {
    let conn = crate::session()?;
    let (tx, rx) = channel();
    let fm = FileManager {
        tx,
        host_only: HostOnly::default(),
    };
    crate::wait(conn.object_server().at(PATH, fm))?;
    match crate::wait(conn.request_name_with_flags(NAME, RequestNameFlags::DO_NOT_QUEUE))? {
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner => Ok((conn, rx)),
        _ => Err(zbus::Error::NameTaken),
    }
}

/// A sandboxed caller names paths inside its sandbox and is refused; xdg-desktop-portal's
/// `OpenDirectory` calls `ShowItems` for it with the host path.
struct FileManager {
    tx: Sender<Request>,
    host_only: HostOnly,
}

impl FileManager {
    async fn take(
        &self,
        conn: &Connection,
        header: &Header<'_>,
        show: Show,
        uris: Vec<String>,
        startup_id: String,
    ) -> zbus::fdo::Result<()> {
        self.host_only.check(conn, header).await?;
        let handoff = Handoff {
            token: Some(startup_id).filter(|t| !t.is_empty()),
            by_key: false,
        };
        let _ = self.tx.send(Request {
            show,
            uris,
            handoff,
        });
        Ok(())
    }
}

#[zbus::interface(name = "org.freedesktop.FileManager1")]
impl FileManager {
    async fn show_folders(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: Header<'_>,
        uris: Vec<String>,
        startup_id: String,
    ) -> zbus::fdo::Result<()> {
        self.take(conn, &header, Show::Folders, uris, startup_id)
            .await
    }

    async fn show_items(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: Header<'_>,
        uris: Vec<String>,
        startup_id: String,
    ) -> zbus::fdo::Result<()> {
        self.take(conn, &header, Show::Items, uris, startup_id)
            .await
    }

    async fn show_item_properties(
        &self,
        #[zbus(connection)] conn: &Connection,
        #[zbus(header)] header: Header<'_>,
        uris: Vec<String>,
        startup_id: String,
    ) -> zbus::fdo::Result<()> {
        self.take(conn, &header, Show::ItemProperties, uris, startup_id)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a call of `member` with `uris` and `startup_id` from another peer on `bus` gets back,
    /// and the requests a [`FileManager`] reading processes under `proc` handed on.
    fn called(
        bus: &crate::peer::tests::Bus,
        proc: &std::path::Path,
        member: &str,
        uris: &[&str],
        startup_id: &str,
    ) -> (zbus::Result<()>, Vec<Request>) {
        let connect = || zbus::block_on(crate::testbus::connect(&bus.1));
        let (server, caller) = (connect(), connect());
        let (tx, rx) = channel();
        let fm = FileManager {
            tx,
            host_only: HostOnly::under(proc),
        };
        let to = server.unique_name().unwrap().to_string();
        let uris: Vec<String> = uris.iter().map(|u| u.to_string()).collect();
        let reply = zbus::block_on(async {
            crate::testbus::serve(&server, PATH, fm).await;
            caller
                .call_method(
                    Some(to.as_str()),
                    PATH,
                    Some(NAME),
                    member,
                    &(uris, startup_id),
                )
                .await
                .map(drop)
        });
        (reply, rx.try_iter().collect())
    }

    #[test]
    fn each_method_hands_its_uris_and_startup_id_on() {
        let Some(bus) = crate::peer::tests::bus() else {
            return;
        };
        let host = std::path::Path::new("/proc");
        for (member, show) in [
            ("ShowFolders", Show::Folders),
            ("ShowItems", Show::Items),
            ("ShowItemProperties", Show::ItemProperties),
        ] {
            let (reply, requests) = called(&bus, host, member, &["file:///a", "file:///b"], "t-1");
            assert!(reply.is_ok(), "{member}: {reply:?}");
            assert_eq!(
                requests,
                [Request {
                    show,
                    uris: vec!["file:///a".into(), "file:///b".into()],
                    handoff: Handoff {
                        token: Some("t-1".into()),
                        by_key: false,
                    },
                }],
                "{member}"
            );
        }
        let (_, requests) = called(&bus, host, "ShowItems", &["file:///a"], "");
        assert_eq!(
            requests[0].handoff.token, None,
            "an empty startup_id carries no token"
        );
    }

    #[test]
    fn a_sandboxed_peer_is_refused_every_method() {
        let Some(bus) = crate::peer::tests::bus() else {
            return;
        };
        let sandboxed = crate::peer::tests::sandboxed_proc();
        for member in ["ShowFolders", "ShowItems", "ShowItemProperties"] {
            let (reply, requests) = called(&bus, sandboxed.path(), member, &["file:///a"], "");
            let refusal = reply.as_ref().err().and_then(crate::error::name);
            assert_eq!(
                refusal.as_deref(),
                Some("org.freedesktop.DBus.Error.AccessDenied"),
                "{member} answers AccessDenied"
            );
            assert_eq!(requests, [], "{member} hands nothing on");
        }
    }
}
