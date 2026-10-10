//! The pairing agent: `org.bluez.Agent1` served at [`PATH`] and made
//! BlueZ's default agent. A call that needs the user becomes an
//! [`AgentRequest`] for the windows and waits for the answer; zbus runs
//! each call on its own task, so `Cancel` gets through while one waits.
//! The display calls (a passkey to type on the device) return at once.
//! Only bluetoothd may call it.

use super::bus::{NAME, Poster};
use super::{AgentRequest, Answer, Ask, BzMsg};
use zbus::message::Header;
use zbus::zvariant::{ObjectPath, OwnedObjectPath};

pub const PATH: &str = "/reprise/bluetooth/agent";
/// A keyboard and a display: BlueZ may use any pairing method.
const CAPABILITY: &str = "KeyboardDisplay";
const MANAGER_PATH: &str = "/org/bluez";
const MANAGER_IFACE: &str = "org.bluez.AgentManager1";

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.bluez.Error")]
enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    Rejected(String),
    Canceled(String),
}

struct Agent {
    poster: Poster,
}

/// The windows' answer as the agent's reply: Reject refuses, and a
/// request dropped unanswered (its window closed, or BlueZ's `Cancel`
/// closed it) is Canceled.
fn reply(got: Result<Answer, async_channel::RecvError>) -> Result<Answer, Error> {
    match got {
        Ok(Answer::Reject) => Err(Error::Rejected("refused by the user".into())),
        Ok(a) => Ok(a),
        Err(_) => Err(Error::Canceled("closed without an answer".into())),
    }
}

impl Agent {
    async fn ask(&self, device: &OwnedObjectPath, ask: Ask) -> Result<Answer, Error> {
        let (done, wait) = async_channel::bounded(1);
        self.poster.post(BzMsg::Agent(AgentRequest {
            device: device.to_string(),
            ask,
            done: Some(done),
        }));
        reply(wait.recv().await)
    }

    fn show(&self, device: &OwnedObjectPath, ask: Ask) {
        self.poster.post(BzMsg::Agent(AgentRequest {
            device: device.to_string(),
            ask,
            done: None,
        }));
    }
}

/// Refuse a call that bluetoothd did not send. The owner is looked up per
/// call, which follows a restart.
async fn check_caller(conn: &zbus::Connection, hdr: &Header<'_>) -> Result<(), Error> {
    let owner = reprisebus::name_owner(conn, NAME).await;
    let sender = hdr.sender().map(|s| s.as_str());
    if reprisebus::sent_by(sender, owner.as_deref()) {
        return Ok(());
    }
    log::warn!("refusing an agent call from {sender:?}, which is not {NAME}");
    Err(Error::Rejected(format!("only {NAME} may call the agent")))
}

#[zbus::interface(name = "org.bluez.Agent1")]
impl Agent {
    async fn release(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        log::info!("BlueZ released the pairing agent");
        Ok(())
    }

    async fn request_pin_code(
        &self,
        device: OwnedObjectPath,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<String, Error> {
        check_caller(conn, &hdr).await?;
        match self.ask(&device, Ask::PinCode).await? {
            Answer::Pin(p) => Ok(p),
            _ => Err(Error::Rejected("no PIN".into())),
        }
    }

    async fn display_pin_code(
        &self,
        device: OwnedObjectPath,
        pincode: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        self.show(&device, Ask::ShowPin(pincode));
        Ok(())
    }

    async fn request_passkey(
        &self,
        device: OwnedObjectPath,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<u32, Error> {
        check_caller(conn, &hdr).await?;
        match self.ask(&device, Ask::Passkey).await? {
            Answer::Passkey(n) => Ok(n),
            _ => Err(Error::Rejected("no passkey".into())),
        }
    }

    /// Called again as each digit is typed on the device; only the first
    /// call opens the prompt.
    async fn display_passkey(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        entered: u16,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        if entered == 0 {
            self.show(&device, Ask::ShowPasskey(passkey));
        }
        Ok(())
    }

    async fn request_confirmation(
        &self,
        device: OwnedObjectPath,
        passkey: u32,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        match self.ask(&device, Ask::Confirm(passkey)).await? {
            Answer::Accept => Ok(()),
            _ => Err(Error::Rejected("the passkeys differ".into())),
        }
    }

    async fn request_authorization(
        &self,
        device: OwnedObjectPath,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        match self.ask(&device, Ask::Authorize).await? {
            Answer::Accept => Ok(()),
            _ => Err(Error::Rejected("not allowed".into())),
        }
    }

    /// BlueZ calls this only for an untrusted device, which may not connect a service on its own: the user connects it from the Devices window or ticks "Connect to this device automatically".
    async fn authorize_service(
        &self,
        device: OwnedObjectPath,
        uuid: String,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        log::info!("refusing {uuid} for {device}: it is not trusted");
        Err(Error::Rejected("the device is not trusted".into()))
    }

    async fn cancel(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        self.poster.post(BzMsg::AgentCancel);
        Ok(())
    }
}

pub(super) async fn serve(conn: &zbus::Connection, poster: Poster) -> zbus::Result<bool> {
    conn.object_server().at(PATH, Agent { poster }).await
}

/// Register with BlueZ and ask to be the default agent, so a pairing a
/// device starts reaches this one too.
pub(super) async fn register(conn: &zbus::Connection) -> zbus::Result<()> {
    let path = ObjectPath::try_from(PATH)?;
    conn.call_method(
        Some(NAME),
        MANAGER_PATH,
        Some(MANAGER_IFACE),
        "RegisterAgent",
        &(&path, CAPABILITY),
    )
    .await?;
    conn.call_method(
        Some(NAME),
        MANAGER_PATH,
        Some(MANAGER_IFACE),
        "RequestDefaultAgent",
        &(&path,),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_map_to_bluez_errors() {
        assert!(matches!(reply(Ok(Answer::Accept)), Ok(Answer::Accept)));
        assert!(matches!(reply(Ok(Answer::Reject)), Err(Error::Rejected(_))));
        assert!(matches!(
            reply(Err(async_channel::RecvError)),
            Err(Error::Canceled(_))
        ));
    }

    #[test]
    fn error_names_use_the_bluez_prefix() {
        use zbus::DBusError;
        assert_eq!(
            Error::Rejected(String::new()).name().as_str(),
            "org.bluez.Error.Rejected"
        );
        assert_eq!(
            Error::Canceled(String::new()).name().as_str(),
            "org.bluez.Error.Canceled"
        );
    }
}
