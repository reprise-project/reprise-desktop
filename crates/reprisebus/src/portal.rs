//! A desktop portal call that answers through a Request object's Response signal. The Response
//! counts only from the portal's own unique name, on the path the request's token names.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use futures_util::StreamExt;
use serde::Serialize;
use serde::de::DeserializeOwned;
use zbus::names::{BusName, WellKnownName};
use zbus::zvariant::{DeserializeDict, DynamicType, OwnedVariant, Type, Variant};
use zbus::{Connection, MatchRule, MessageStream};

pub const PORTAL: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const REQUEST: &str = "org.freedesktop.portal.Request";

/// A request's options; [`request`] adds its `handle_token`.
pub type Options = HashMap<&'static str, Variant<'static>>;
/// The results of a request whose answer carries nothing the caller reads.
pub type Ignored = HashMap<String, OwnedVariant>;

/// A `CreateSession` request's results.
#[derive(DeserializeDict, Type)]
#[zvariant(signature = "a{sv}")]
pub struct CreateResults {
    pub session_handle: Option<String>,
}

#[derive(Debug)]
pub enum Error {
    Bus(zbus::Error),
    /// No Response within the wait [`request`] was given.
    NoAnswer(Duration),
    /// The connection closed before the Response came.
    Closed,
    /// Response code 1: the user cancelled the dialog.
    Cancelled,
    /// Response code 2, or one the spec doesn't name.
    Failed,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Bus(e) => write!(f, "{e}"),
            Error::NoAnswer(wait) => write!(f, "no answer in {} s", wait.as_secs()),
            Error::Closed => f.write_str("the bus closed"),
            Error::Cancelled => f.write_str("refused"),
            Error::Failed => f.write_str("failed"),
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(e: zbus::Error) -> Error {
        Error::Bus(e)
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(e: zbus::fdo::Error) -> Error {
        Error::Bus(e.into())
    }
}

/// A handle token no other request or session of this process uses.
pub fn token() -> String {
    static SERIAL: AtomicU32 = AtomicU32::new(0);
    format!("reprise_{}", SERIAL.fetch_add(1, Ordering::Relaxed))
}

/// The request object the portal answers on: the spec derives it from the caller's unique name
/// and the handle token, so the Response is subscribed before the call.
pub fn request_path(unique: &str, token: &str) -> String {
    let sender = unique.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

/// The Response signals `owner` sends on request `path`.
fn response_rule(path: &str, owner: &str) -> zbus::Result<MatchRule<'static>> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(owner)
        .interface(REQUEST)
        .member("Response")
        .path(path)
        .build()?;
    Ok(rule.into_owned())
}

/// The portal's unique name, started if need be.
async fn owner(conn: &Connection) -> Result<String, Error> {
    let bus = zbus::fdo::DBusProxy::new(conn).await?;
    // A bus with no activation file for the portal may answer ServiceUnknown even while the
    // name has an owner; GetNameOwner decides.
    let _ = bus
        .start_service_by_name(WellKnownName::from_static_str_unchecked(PORTAL), 0)
        .await;
    let owner = bus
        .get_name_owner(BusName::WellKnown(
            WellKnownName::from_static_str_unchecked(PORTAL),
        ))
        .await?;
    Ok(owner.to_string())
}

/// Calls the portal's `iface.method` with the body `body` builds around options carrying a
/// fresh handle token, and waits for its Response, up to `wait` (`None` for a dialog the user
/// may leave open): the results on code 0.
pub async fn request<B, R>(
    conn: &Connection,
    iface: &str,
    method: &str,
    wait: Option<Duration>,
    body: impl FnOnce(Options) -> B,
) -> Result<R, Error>
where
    B: Serialize + DynamicType,
    R: DeserializeOwned + Type,
{
    ask(conn, iface, method, body).await?.answer(wait).await
}

/// A request the portal took: its method reply, and its Response stream, subscribed before the
/// call so a Response sent ahead of the reply is kept.
pub struct Asked {
    pub reply: zbus::Message,
    responses: MessageStream,
}

/// [`request`]'s call, without waiting for the Response.
pub async fn ask<B>(
    conn: &Connection,
    iface: &str,
    method: &str,
    body: impl FnOnce(Options) -> B,
) -> Result<Asked, Error>
where
    B: Serialize + DynamicType,
{
    let owner = owner(conn).await?;
    let token = token();
    let unique = conn.unique_name().map(|n| n.as_str()).unwrap_or_default();
    let rule = response_rule(&request_path(unique, &token), &owner)?;
    let responses = MessageStream::for_match_rule(rule, conn, None).await?;
    let options = Options::from([("handle_token", Variant::from(token))]);
    let body = body(options);
    let reply = (conn.call_method(Some(PORTAL), PORTAL_PATH, Some(iface), method, &body)).await?;
    Ok(Asked { reply, responses })
}

impl Asked {
    /// The Response's results on code 0, waiting up to `wait` (`None`: as long as it takes).
    pub async fn answer<R: DeserializeOwned + Type>(
        mut self,
        wait: Option<Duration>,
    ) -> Result<R, Error> {
        let next = self.responses.next();
        let answer = match wait {
            Some(wait) => futures_lite::future::or(async { Some(next.await) }, async {
                async_io::Timer::after(wait).await;
                None
            })
            .await
            .ok_or(Error::NoAnswer(wait))?,
            None => next.await,
        };
        let (code, results): (u32, R) = answer.ok_or(Error::Closed)??.body().deserialize()?;
        match code {
            0 => Ok(results),
            1 => Err(Error::Cancelled),
            _ => Err(Error::Failed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_response_is_awaited_from_the_portal_on_the_path_the_token_names() {
        let path = request_path(":1.42", "reprise_1");
        assert_eq!(
            path,
            "/org/freedesktop/portal/desktop/request/1_42/reprise_1"
        );
        let rule = response_rule(&path, ":1.7").unwrap();
        assert_eq!(rule.sender().map(|s| s.as_str()), Some(":1.7"));
        assert_eq!(rule.member().map(|m| m.as_str()), Some("Response"));
        assert_eq!(
            rule.path_spec(),
            Some(&zbus::match_rule::PathSpec::Path(
                path.as_str().try_into().unwrap()
            ))
        );
        assert_ne!(
            rule,
            response_rule(&path, ":1.8").unwrap(),
            "another peer's Response does not match"
        );
    }

    use crate::testbus::bus;

    async fn connect(address: &str) -> Connection {
        zbus::connection::Builder::address(address)
            .build()
            .await
            .expect("connect")
    }

    type Asked = HashMap<String, OwnedVariant>;

    /// A portal whose `Ask` has another peer, `forger`, answer first, broadcast and sent to the
    /// caller alone, before it answers itself; `Ignore` never answers.
    struct Fake {
        forger: Connection,
    }

    fn who(said: &str) -> (u32, HashMap<&str, Variant<'_>>) {
        (0, HashMap::from([("who", Variant::from(said))]))
    }

    #[zbus::interface(name = "org.reprise.Test")]
    impl Fake {
        async fn ask(
            &self,
            options: Asked,
            #[zbus(connection)] conn: &Connection,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
        ) -> zbus::fdo::Result<()> {
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let caller = hdr.sender().unwrap().to_string();
            let path = request_path(&caller, &token);
            let forged = who("forger");
            for to in [None, Some(caller.as_str())] {
                (self.forger)
                    .emit_signal(to, path.as_str(), REQUEST, "Response", &forged)
                    .await?;
            }
            // The bus handles the forger's messages in order, so its signals are delivered
            // before this reply.
            zbus::fdo::DBusProxy::new(&self.forger)
                .await?
                .get_id()
                .await?;
            (conn.emit_signal(
                None::<&str>,
                path.as_str(),
                REQUEST,
                "Response",
                &who("portal"),
            ))
            .await?;
            Ok(())
        }

        async fn ignore(&self, _options: Asked) {}
    }

    #[test]
    fn only_the_portals_own_response_answers_a_request() {
        let Some(bus) = bus() else {
            return;
        };
        zbus::block_on(async {
            let fake = Fake {
                forger: connect(&bus.1).await,
            };
            let _portal = zbus::connection::Builder::address(bus.1.as_str())
                .name(PORTAL)
                .serve_at(PORTAL_PATH, fake)
                .build()
                .await
                .unwrap();
            let conn = connect(&bus.1).await;
            let wait = Some(Duration::from_secs(5));
            let results: Asked = request(&conn, "org.reprise.Test", "Ask", wait, |o| (o,))
                .await
                .unwrap();
            assert_eq!(
                String::try_from(results["who"].try_clone().unwrap()).unwrap(),
                "portal"
            );
            let short = Duration::from_millis(200);
            let silent =
                request::<_, Asked>(&conn, "org.reprise.Test", "Ignore", Some(short), |o| (o,))
                    .await;
            assert!(
                matches!(silent, Err(Error::NoAnswer(d)) if d == short),
                "{silent:?}"
            );
        });
    }
}
