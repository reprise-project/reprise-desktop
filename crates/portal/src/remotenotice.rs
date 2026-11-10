//! The balloon a RemoteDesktop session started from a remembered grant shows, since no dialog
//! asked; its button ends the session and deletes the grant from xdg-desktop-portal's store.

use crate::{SessionData, Sessions};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use zbus::{OwnedVariant, Variant};

const SERVER: &str = "org.freedesktop.Notifications";
const SERVER_PATH: &str = "/org/freedesktop/Notifications";
const SENDER: &str = "Reprise";
const FORGET_KEY: &str = "forget";
const FORGET_LABEL: &str = "Stop and forget";
const BODY: &str = "You allowed this before and ticked Remember this choice.";
/// How long the balloon stays up before it waits in the history.
const TIMEOUT_MS: i32 = 30_000;
const STORE: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_PATH: &str = "/org/freedesktop/impl/portal/PermissionStore";
/// The store table xdg-desktop-portal keeps RemoteDesktop restore tokens in.
const TABLE: &str = "remote-desktop";

/// The balloons shown and the apps whose grant was forgotten.
#[derive(Clone, Default)]
pub(crate) struct Notices(Arc<Mutex<Book>>);

#[derive(Default)]
struct Book {
    /// The server's id to the app it tells of and that app's restored sessions.
    shown: HashMap<u32, (String, HashSet<String>)>,
    /// Apps whose grant was forgotten. A token the frontend holds only in memory (persist mode 1)
    /// never reached the store, so it restores nothing until the dialog is allowed with Remember.
    forgotten: HashSet<String>,
}

impl Notices {
    pub(crate) fn forgotten(&self, app_id: &str) -> bool {
        self.0.lock().unwrap().forgotten.contains(app_id)
    }

    /// The dialog allowed `app_id` with Remember: its new token restores again.
    pub(crate) fn remembered(&self, app_id: &str) {
        self.0.lock().unwrap().forgotten.remove(app_id);
    }

    /// Tell the user that `app_id`'s session `key` started from a remembered grant, in the
    /// app's balloon still open if it has one.
    pub(crate) async fn show(
        self,
        conn: zbus::Connection,
        sessions: Sessions,
        key: String,
        app_id: String,
    ) {
        let id = app_id.clone();
        let (name, icon) = blocking::unblock(move || face(&id)).await;
        let summary = match name.is_empty() {
            true => "An application is controlling this computer".to_string(),
            false => format!("{name} is controlling this computer"),
        };
        // From Reprise and critical, so neither the app's own mute nor Do Not Disturb holds it,
        // and the history keeps it open until dismissed or the session ends.
        let mut hints: HashMap<&str, Variant<'_>> = HashMap::new();
        hints.insert("urgency", Variant::U8(2));
        if !icon.is_empty() {
            hints.insert("image-path", Variant::from(icon.as_str()));
        }
        let replaces = (self.0.lock().unwrap().shown.iter())
            .find(|(_, (app, _))| *app == app_id)
            .map_or(0, |(id, _)| *id);
        let body = (
            SENDER,
            replaces,
            "",
            summary.as_str(),
            BODY,
            vec![FORGET_KEY, FORGET_LABEL],
            hints,
            TIMEOUT_MS,
        );
        let reply = conn
            .call_method(Some(SERVER), SERVER_PATH, Some(SERVER), "Notify", &body)
            .await;
        match reply.and_then(|r| r.body().deserialize::<u32>()) {
            Ok(id) => {
                log::info!("RemoteDesktop {key}: restored for {app_id:?}; balloon {id}");
                let mut book = self.0.lock().unwrap();
                let (_, keys) = book.shown.entry(id).or_insert((app_id, HashSet::new()));
                keys.insert(key.clone());
                drop(book);
                let shown = Shown {
                    id,
                    key: key.clone(),
                    notices: self,
                    conn,
                };
                let mut map = sessions.lock().unwrap();
                if let Some(s) = map.get_mut(&key).and_then(SessionData::remote_mut) {
                    s.notice = Some(shown);
                }
            }
            Err(e) => log::warn!("RemoteDesktop {key}: restored for {app_id:?}; no balloon: {e}"),
        }
    }

    /// Balloon `server_id`'s Stop and forget: its app, now forgotten. Taken as the click comes
    /// in, since the server's `NotificationClosed` for it follows at once.
    fn forget(&self, server_id: u32) -> Option<String> {
        let mut book = self.0.lock().unwrap();
        let (app_id, _) = book.shown.remove(&server_id)?;
        book.forgotten.insert(app_id.clone());
        Some(app_id)
    }
}

/// Session `key`'s hold on balloon `id`: the balloon closes once the last session of its app goes.
pub(crate) struct Shown {
    id: u32,
    key: String,
    notices: Notices,
    conn: zbus::Connection,
}

impl Drop for Shown {
    fn drop(&mut self) {
        let mut book = self.notices.0.lock().unwrap();
        let Some((_, keys)) = book.shown.get_mut(&self.id) else {
            return;
        };
        if !keys.remove(&self.key) || !keys.is_empty() {
            return;
        }
        book.shown.remove(&self.id);
        let (conn, id) = (self.conn.clone(), self.id);
        zruntime::spawn(async move {
            let body = (id,);
            let close = "CloseNotification";
            let sent = conn.call_method(Some(SERVER), SERVER_PATH, Some(SERVER), close, &body);
            if let Err(e) = sent.await {
                log::warn!("RemoteDesktop: balloon {id} stays: {e}");
            }
        })
        .detach();
    }
}

/// End every RemoteDesktop session of `app_id` and delete the app's grants.
async fn stop_and_forget(conn: zbus::Connection, sessions: Sessions, app_id: String) {
    let gone: Vec<(String, SessionData)> = (sessions.lock().unwrap())
        .extract_if(|_, s| s.remote_mut().is_some_and(|r| r.app_id == app_id))
        .collect();
    for (key, data) in gone {
        log::info!("RemoteDesktop {key}: stopped from its balloon");
        crate::end_session(&conn, &key, Some(data)).await;
    }
    match forget_grants(&conn, &app_id).await {
        Ok(n) => log::info!("RemoteDesktop: forgot {n} grant(s) of {app_id:?}"),
        Err(e) => log::warn!("RemoteDesktop: could not forget the grants of {app_id:?}: {e}"),
    }
}

/// The app's name and icon as its desktop entry gives them; the app id and no icon without one.
fn face(app_id: &str) -> (String, String) {
    match crate::remotedesktop::app_entry(app_id) {
        Some(a) => (a.name, a.icon.unwrap_or_default()),
        None => (app_id.to_string(), String::new()),
    }
}

/// Delete every RemoteDesktop token in the store issued to `app_id`; how many went.
async fn forget_grants(conn: &zbus::Connection, app_id: &str) -> zbus::Result<usize> {
    let ids: Vec<String> = store(conn, "List", &(TABLE,)).await?.body().deserialize()?;
    let mut gone = 0;
    for id in ids {
        let Ok(found) = store(conn, "Lookup", &(TABLE, &id)).await else {
            continue;
        };
        let Ok((_, data)) = found
            .body()
            .deserialize::<(HashMap<String, Vec<String>>, OwnedVariant)>()
        else {
            continue;
        };
        if remote_grant_for(&data, app_id) {
            store(conn, "Delete", &(TABLE, &id)).await?;
            gone += 1;
        }
    }
    Ok(gone)
}

async fn store<B>(conn: &zbus::Connection, method: &str, body: &B) -> zbus::Result<zbus::Message>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    conn.call_method(Some(STORE), STORE_PATH, Some(STORE), method, body)
        .await
}

/// Whether a store entry's data is one of our RemoteDesktop tokens issued to `app_id`.
fn remote_grant_for(data: &Variant<'_>, app_id: &str) -> bool {
    if let Variant::Variant(inner) = data {
        return remote_grant_for(inner, app_id);
    }
    let Ok(token) = data
        .try_to_owned()
        .and_then(<(String, u32, OwnedVariant)>::try_from)
    else {
        return false;
    };
    crate::restored_grant(&token, app_id)
        .and_then(crate::remotedesktop::parse_grant)
        .is_some()
}

/// Follow the server's `ActionInvoked` and `NotificationClosed` for the balloons in `notices`.
/// Runs for as long as the connection does.
pub(crate) async fn watch(conn: zbus::Connection, sessions: Sessions, notices: Notices) {
    use futures_lite::StreamExt;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(SERVER)
        .path(SERVER_PATH)
        .build()
        .map(Into::into);
    let rules = match rule {
        Ok(r) => vec![r],
        Err(e) => {
            log::warn!("RemoteDesktop balloons: {e}");
            return;
        }
    };
    let mut signals = reprisebus::owner_signals(&conn, SERVER, rules).await;
    while let Some(sig) = signals.next().await {
        let m = match sig {
            reprisebus::Signal::Message(m) => m,
            // A new server numbers its notifications afresh.
            reprisebus::Signal::OwnerChanged => {
                notices.0.lock().unwrap().shown.clear();
                continue;
            }
        };
        let member = m.header().member().map(|m| m.to_string());
        match member.as_deref() {
            Some("ActionInvoked") => {
                if let Ok((id, key)) = m.body().deserialize::<(u32, String)>()
                    && key == FORGET_KEY
                    && let Some(app_id) = notices.forget(id)
                {
                    let stop = stop_and_forget(conn.clone(), sessions.clone(), app_id);
                    zruntime::spawn(stop).detach();
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _)) = m.body().deserialize::<(u32, u32)>() {
                    notices.0.lock().unwrap().shown.remove(&id);
                }
            }
            _ => {}
        }
    }
}
