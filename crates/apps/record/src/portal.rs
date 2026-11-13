//! The ScreenCast portal session the recording reads the screen from. Reprise's portal starts
//! the recorder's session without a dialog and casts every screen (docs/security.md).

use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::time::Duration;
use zbus::zvariant::{DeserializeDict, DynamicType, Type, Variant};
use zbus::{Connection, MessageStream, OwnedObjectPath, OwnedVariant};

const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SCREENCAST: &str = "org.freedesktop.portal.ScreenCast";
const REQUEST: &str = "org.freedesktop.portal.Request";
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
        let token = "reprise_record_session";
        let created: CreateResults = request(&conn, "CreateSession", 0, |mut options| {
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
        let _: Ignored = request(&cast.conn, "SelectSources", 1, |mut options| {
            options.insert("types", Variant::from(MONITOR));
            options.insert("multiple", Variant::from(true));
            options.insert("cursor_mode", Variant::from(CURSOR_EMBEDDED));
            (&cast.session, options)
        })?;
        let started: StartResults = request(&cast.conn, "Start", 2, |options| {
            (&cast.session, "", options)
        })?;
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

type Options = HashMap<&'static str, Variant<'static>>;
type Ignored = HashMap<String, OwnedVariant>;

#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
struct CreateResults {
    session_handle: Option<String>,
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

/// The request object the portal answers on, derived from our unique name and the handle
/// token, so the Response can be subscribed before the call.
fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

/// Call ScreenCast's `method` with the body `body` builds around options carrying handle token
/// `serial`, and wait for its Response: the results on success, the method and code otherwise.
fn request<B, R>(
    conn: &Connection,
    method: &str,
    serial: u32,
    body: impl FnOnce(Options) -> B,
) -> Result<R, String>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type + Send + 'static,
{
    let failed = |e: zbus::Error| format!("{method}: {e}");
    let token = format!("reprise_record_{serial}");
    let unique = conn
        .unique_name()
        .map(|n| n.to_string())
        .unwrap_or_default();
    let path = request_path(&unique, &token);
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(REQUEST)
        .member("Response")
        .path(path.as_str())
        .build()
        .map_err(failed)?;
    let mut responses =
        zbus::block_on(MessageStream::for_match_rule(rule, conn, None)).map_err(failed)?;
    let options = Options::from([("handle_token", Variant::from(token))]);
    let body = body(options);
    let call = conn.call_method(Some(PORTAL), PORTAL_PATH, Some(SCREENCAST), method, &body);
    zbus::block_on(call).map_err(failed)?;
    let (tx, rx) = mpsc::channel();
    let name = method.to_string();
    std::thread::spawn(move || {
        let answer = match zbus::block_on(responses.next()) {
            Some(Ok(m)) => (m.body().deserialize::<(u32, R)>()).map_err(|e| format!("{name}: {e}")),
            Some(Err(e)) => Err(format!("{name}: {e}")),
            None => Err(format!("{name}: the bus closed")),
        };
        let _ = tx.send(answer);
    });
    let (code, results) = (rx.recv_timeout(ANSWER))
        .map_err(|_| format!("{method}: no answer in {} s", ANSWER.as_secs()))??;
    match code {
        0 => Ok(results),
        1 => Err(format!("{method}: refused")),
        _ => Err(format!("{method}: failed")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_response_is_awaited_on_the_path_the_token_names() {
        assert_eq!(
            request_path(":1.42", "reprise_record_1"),
            "/org/freedesktop/portal/desktop/request/1_42/reprise_record_1"
        );
    }
}
