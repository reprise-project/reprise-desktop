//! The ScreenCast portal session the recording reads the screen from. Reprise's portal starts
//! the recorder's session without a dialog and casts every screen (docs/security.md).

use reprisebus::portal::{self, CreateResults, Ignored, Options, PORTAL, PORTAL_PATH};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::Duration;
use zbus::zvariant::{DeserializeDict, DynamicType, Type, Variant};
use zbus::{Connection, OwnedObjectPath};

const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const SESSION: &str = "org.freedesktop.portal.Session";

const MONITOR: u32 = 1;
const CURSOR_EMBEDDED: u32 = 2;
/// How long a request may go unanswered: no dialog is expected, so a silent portal is stuck.
const ANSWER: Duration = Duration::from_secs(30);

/// A started session; dropping it closes the session.
pub struct Cast {
    conn: Connection,
    session: OwnedObjectPath,
    /// The screens cast, in the portal's order.
    pub screens: Vec<Screen>,
}

/// A cast screen: its PipeWire node, the output it shows and its logical size.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub node: u32,
    pub output: Option<String>,
    pub size: (i32, i32),
}

impl Cast {
    /// A session casting every screen with the pointer drawn in.
    pub fn start() -> Result<Cast, String> {
        let conn = zbus::block_on(Connection::session());
        let conn = conn.map_err(|e| format!("no session bus: {e}"))?;
        let token = portal::token();
        let created: CreateResults = request(&conn, "CreateSession", |mut options| {
            options.insert("session_handle_token", Variant::from(token));
            (options,)
        })?;
        let session = created.session_handle.ok_or("CreateSession: no session")?;
        let session = OwnedObjectPath::try_from(session).map_err(|e| e.to_string())?;
        let mut cast = Cast {
            conn,
            session,
            screens: Vec::new(),
        };
        let _: Ignored = request(&cast.conn, "SelectSources", |mut options| {
            options.insert("types", Variant::from(MONITOR));
            options.insert("multiple", Variant::from(true));
            options.insert("cursor_mode", Variant::from(CURSOR_EMBEDDED));
            (&cast.session, options)
        })?;
        let started: StartResults =
            request(&cast.conn, "Start", |options| (&cast.session, "", options))?;
        cast.screens = screens(started.streams.unwrap_or_default());
        Ok(cast)
    }

    /// The screen showing output `name`.
    pub fn screen(&self, name: &str) -> Option<&Screen> {
        self.screens
            .iter()
            .find(|s| s.output.as_deref() == Some(name))
    }

    /// A PipeWire connection that reaches the session's nodes.
    pub fn pipewire(&self) -> Result<OwnedFd, String> {
        let reply = zbus::block_on(self.conn.call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(SCREENCAST),
            "OpenPipeWireRemote",
            &(&self.session, HashMap::<&str, Variant>::new()),
        ))
        .map_err(|e| format!("OpenPipeWireRemote: {e}"))?;
        let fd: zbus::OwnedFd =
            (reply.body().deserialize()).map_err(|e| format!("OpenPipeWireRemote: {e}"))?;
        Ok(fd.into())
    }
}

impl Drop for Cast {
    fn drop(&mut self) {
        let close =
            (self.conn).call_method(Some(PORTAL), &self.session, Some(SESSION), "Close", &());
        let closed = zbus::block_on(close);
        if let Err(e) = closed {
            log::debug!("closing the portal session: {e}");
        }
    }
}

fn screens(streams: Vec<(u32, StreamProps)>) -> Vec<Screen> {
    (streams.into_iter())
        .map(|(node, s)| Screen {
            node,
            output: s.mapping_id,
            size: s.size.unwrap_or_default(),
        })
        .collect()
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StartResults {
    streams: Option<Vec<(u32, StreamProps)>>,
}

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct StreamProps {
    size: Option<(i32, i32)>,
    mapping_id: Option<String>,
}

/// Calls ScreenCast's `method` with the body `body` builds around the request's options, and
/// waits up to [`ANSWER`] for the portal's Response: the results, or the method and what went
/// wrong.
fn request<B, R>(
    conn: &Connection,
    method: &str,
    body: impl FnOnce(Options) -> B,
) -> Result<R, String>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type,
{
    let asked = portal::request(conn, SCREENCAST, method, Some(ANSWER), body);
    zbus::block_on(asked).map_err(|e| format!("{method}: {e}"))
}
