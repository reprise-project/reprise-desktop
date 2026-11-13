//! The Secret Service (`org.freedesktop.secrets`): look up and store passwords by attributes. A
//! locked collection raises the service's own prompt, which these calls wait out.

use std::collections::HashMap;

use futures_util::StreamExt;
use zbus::{Connection, MessageStream};
use zbus::{ObjectPath, OwnedObjectPath, OwnedVariant, Variant};
use zeroize::Zeroizing;

const NAME: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const SERVICE: &str = "org.freedesktop.Secret.Service";
const COLLECTION: &str = "org.freedesktop.Secret.Collection";
const ITEM: &str = "org.freedesktop.Secret.Item";
const PROMPT: &str = "org.freedesktop.Secret.Prompt";
const DEFAULT: &str = "/org/freedesktop/secrets/aliases/default";

/// `(session, parameters, value, content type)`, the spec's Secret struct.
type Secret = (OwnedObjectPath, Vec<u8>, Vec<u8>, String);

/// A found item: its attributes and its secret as text.
pub struct Found {
    pub attributes: HashMap<String, String>,
    pub secret: Zeroizing<String>,
}

/// A session with the Secret Service. The secret travels in the clear over the session bus
/// (the spec's `plain` algorithm), which only this user's processes can reach.
pub struct Secrets {
    conn: Connection,
    session: OwnedObjectPath,
}

/// Whether a Secret Service is running or can be started on the session bus.
pub fn available() -> bool {
    let Ok(conn) = crate::session() else {
        return false;
    };
    let call = |method: &str| -> Option<zbus::Message> {
        crate::wait(conn.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            method,
            &(),
        ))
        .ok()
    };
    ["ListNames", "ListActivatableNames"].iter().any(|m| {
        call(m)
            .and_then(|r| r.body().deserialize::<Vec<String>>().ok())
            .is_some_and(|names| names.iter().any(|n| n == NAME))
    })
}

impl Secrets {
    /// A session with the service, started if it is not running. `None` without one.
    pub fn open() -> Option<Secrets> {
        let conn = crate::session().ok()?;
        let reply = crate::wait(conn.call_method(
            Some(NAME),
            PATH,
            Some(SERVICE),
            "OpenSession",
            &("plain", Variant::from("")),
        ));
        match reply.and_then(|r| r.body().deserialize::<(OwnedVariant, OwnedObjectPath)>()) {
            Ok((_, session)) => Some(Secrets { conn, session }),
            Err(e) => {
                log::info!("no Secret Service ({e})");
                None
            }
        }
    }

    fn call<B>(
        &self,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message>
    where
        B: serde::ser::Serialize + zbus::DynamicType,
    {
        crate::wait(
            self.conn
                .call_method(Some(NAME), path, Some(iface), method, body),
        )
    }

    /// The first item whose attributes include every one of `attributes`, unlocked first when
    /// its collection is locked. `None` when there is none or the unlock prompt was dismissed.
    pub fn lookup(&self, attributes: &HashMap<&str, &str>) -> Option<Found> {
        let reply = self
            .call(PATH, SERVICE, "SearchItems", &(attributes,))
            .ok()?;
        let (unlocked, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) =
            reply.body().deserialize().ok()?;
        let item = match unlocked.into_iter().next() {
            Some(item) => item,
            None => {
                let item = locked.into_iter().next()?;
                self.unlock(&item)?;
                item
            }
        };
        let reply = self
            .call(PATH, SERVICE, "GetSecrets", &(vec![&item], &self.session))
            .ok()?;
        let mut secrets: HashMap<OwnedObjectPath, Secret> = reply.body().deserialize().ok()?;
        let (_, _, value, _) = secrets.remove(&item)?;
        let value = Zeroizing::new(value);
        let secret = Zeroizing::new(String::from_utf8(value.to_vec()).ok()?);
        let attrs = self
            .call(
                item.as_str(),
                "org.freedesktop.DBus.Properties",
                "Get",
                &(ITEM, "Attributes"),
            )
            .ok()?;
        let attrs: OwnedVariant = attrs.body().deserialize().ok()?;
        let attributes = HashMap::<String, String>::try_from(attrs).ok()?;
        Some(Found { attributes, secret })
    }

    /// Stores `secret` labelled `label` under `attributes` in the default collection,
    /// replacing an item with the same attributes. The collection is made when there is none.
    pub fn store(
        &self,
        label: &str,
        attributes: &HashMap<&str, &str>,
        secret: &str,
    ) -> Result<(), String> {
        let collection = match self.default_collection() {
            Some(c) => c,
            None => self.create_default()?,
        };
        let props: HashMap<&str, Variant> = HashMap::from([
            ("org.freedesktop.Secret.Item.Label", Variant::from(label)),
            (
                "org.freedesktop.Secret.Item.Attributes",
                Variant::from(attributes.clone()),
            ),
        ]);
        let value = Zeroizing::new(secret.as_bytes().to_vec());
        let secret = (
            &self.session,
            Vec::<u8>::new(),
            value.as_slice(),
            "text/plain",
        );
        let reply = self
            .call(
                collection.as_str(),
                COLLECTION,
                "CreateItem",
                &(props, secret, true),
            )
            .map_err(|e| e.to_string())?;
        let (_, prompt): (OwnedObjectPath, OwnedObjectPath) =
            reply.body().deserialize().map_err(|e| e.to_string())?;
        match self.prompt(&prompt) {
            Some(_) => Ok(()),
            None => Err("The keyring was not unlocked.".into()),
        }
    }

    fn default_collection(&self) -> Option<OwnedObjectPath> {
        let reply = self.call(PATH, SERVICE, "ReadAlias", &("default",)).ok()?;
        let path: OwnedObjectPath = reply.body().deserialize().ok()?;
        (path.as_str() != "/").then(|| ObjectPath::from_static_str_unchecked(DEFAULT).into())
    }

    fn create_default(&self) -> Result<OwnedObjectPath, String> {
        let props: HashMap<&str, Variant> = HashMap::from([(
            "org.freedesktop.Secret.Collection.Label",
            Variant::from("Default keyring"),
        )]);
        let reply = self
            .call(PATH, SERVICE, "CreateCollection", &(props, "default"))
            .map_err(|e| e.to_string())?;
        let (made, prompt): (OwnedObjectPath, OwnedObjectPath) =
            reply.body().deserialize().map_err(|e| e.to_string())?;
        if made.as_str() != "/" {
            return Ok(made);
        }
        let result = self
            .prompt(&prompt)
            .ok_or_else(|| "No keyring was made.".to_string())?;
        OwnedObjectPath::try_from(result).map_err(|e| e.to_string())
    }

    fn unlock(&self, item: &OwnedObjectPath) -> Option<()> {
        let reply = self.call(PATH, SERVICE, "Unlock", &(vec![item],)).ok()?;
        let (done, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) =
            reply.body().deserialize().ok()?;
        if done.contains(item) {
            return Some(());
        }
        self.prompt(&prompt).map(|_| ())
    }

    /// Runs the service's prompt at `prompt` ("/" is none) and waits for the person's answer:
    /// its result, or `None` when it was dismissed.
    fn prompt(&self, prompt: &OwnedObjectPath) -> Option<OwnedVariant> {
        if prompt.as_str() == "/" {
            return Some(OwnedVariant::from(true));
        }
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .path(prompt.as_str())
            .interface(PROMPT)
            .member("Completed")
            .build()
            .ok()?;
        let mut stream = crate::wait(MessageStream::for_match_rule(rule, &self.conn, None)).ok()?;
        self.call(prompt.as_str(), PROMPT, "Prompt", &("",)).ok()?;
        let msg = crate::wait(stream.next())?.ok()?;
        let owner = crate::wait(crate::name_owner(&self.conn, NAME));
        let sender = msg.header().sender().map(|s| s.to_string());
        if !crate::sent_by(sender.as_deref(), owner.as_deref()) {
            return None;
        }
        let (dismissed, result): (bool, OwnedVariant) = msg.body().deserialize().ok()?;
        (!dismissed).then_some(result)
    }
}
