//! A program holding off log off through the Inhibit portal (`org.freedesktop.portal.Inhibit`'s
//! Logout flag) while it says why, as a file manager does while a copy runs.

use crate::mutex::lock;
use crate::portal::{self, PORTAL, REQUEST};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use zbus::{Connection, OwnedObjectPath, Variant};

const INHIBIT: &str = "org.freedesktop.portal.Inhibit";
const FLAG_LOGOUT: u32 = 1;
/// How long an Inhibit's Response is waited for.
const ANSWER: Duration = Duration::from_secs(25);

/// Hold off log off because of `reason` (one inhibitor per process, its reason the latest asked
/// for), or let it go with None. Sent from a thread of its own, so the caller never waits.
pub fn hold_logout(reason: Option<&str>) {
    static WORKER: Mutex<Option<Sender<Option<String>>>> = Mutex::new(None);
    let mut worker = lock(&WORKER);
    let mut want = reason.map(String::from);
    if let Some(tx) = worker.as_ref() {
        match tx.send(want) {
            Ok(()) => return,
            Err(e) => want = e.0,
        }
    }
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("logout-inhibit".into())
        .spawn(move || follow(rx, crate::session));
    if let Err(e) = spawned {
        log::warn!("logout inhibitor: no thread: {e}");
        return;
    }
    let _ = tx.send(want);
    *worker = Some(tx);
}

/// Holds what the latest message asks for: a reason that changes takes the new inhibitor before
/// the old one goes, so log off is never let through between them.
fn follow(rx: Receiver<Option<String>>, bus: impl Fn() -> zbus::Result<Connection>) {
    let mut held: Option<(String, OwnedObjectPath)> = None;
    while let Ok(mut want) = rx.recv() {
        want = rx.try_iter().last().unwrap_or(want);
        if held.as_ref().map(|(why, _)| why) == want.as_ref() {
            continue;
        }
        let old = held.take();
        if let Some(why) = want {
            match bus()
                .map_err(portal::Error::from)
                .and_then(|c| inhibit(&c, &why))
            {
                Ok(handle) => held = Some((why, handle)),
                Err(e) => log::info!("logout inhibitor {why:?}: {e}"),
            }
        }
        if let Some((_, handle)) = old
            && let Err(e) = bus().and_then(|conn| close(&conn, &handle))
        {
            log::info!("logout inhibitor {}: close: {e}", handle.as_str());
        }
    }
}

/// The handle of a logout inhibitor for `reason`, returned once the portal's Response says the
/// backend holds it: xdg-desktop-portal hands out the handle before its backend has the request,
/// and a Close sent before then fails and leaves the inhibitor held. With no Response
/// within [`ANSWER`] the handle counts as held, so its Close is still sent.
fn inhibit(conn: &Connection, reason: &str) -> Result<OwnedObjectPath, portal::Error> {
    crate::wait(async {
        let asked = portal::ask(conn, INHIBIT, "Inhibit", |mut options| {
            options.insert("reason", Variant::from(reason.to_string()));
            ("", FLAG_LOGOUT, options)
        })
        .await?;
        let handle: OwnedObjectPath = asked.reply.body().deserialize()?;
        match asked.answer::<portal::Ignored>(Some(ANSWER)).await {
            Ok(_) => Ok(handle),
            Err(portal::Error::NoAnswer(wait)) => {
                log::info!(
                    "logout inhibitor {}: no answer in {wait:?}",
                    handle.as_str()
                );
                Ok(handle)
            }
            Err(e) => Err(e),
        }
    })
}

fn close(conn: &Connection, handle: &OwnedObjectPath) -> zbus::Result<()> {
    crate::wait(conn.call_method(Some(PORTAL), handle.as_str(), Some(REQUEST), "Close", &()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal::PORTAL_PATH;
    use crate::testbus::{Bus, bus, connect, until};
    use std::collections::HashMap;
    use std::sync::Arc;
    use zbus::zvariant::OwnedVariant;

    type Heard = Arc<Mutex<Vec<String>>>;
    /// The request handles the frontend gave out, in turn.
    type Handles = Arc<Mutex<Vec<String>>>;

    /// When the frontend's Response comes: ahead of its method reply, or once the test serves the
    /// Request the way a backend does after a slow Inhibit.
    #[derive(Clone, Copy, Debug)]
    enum Answer {
        BeforeReply,
        Later,
    }

    /// The frontend's Inhibit. Its Request objects stand in for the backend's: a Close that
    /// reaches one before it is served gets UnknownObject, as xdg-desktop-portal passes on.
    struct FakeInhibit(Heard, Answer, Handles);

    #[zbus::interface(name = "org.freedesktop.portal.Inhibit")]
    impl FakeInhibit {
        async fn inhibit(
            &self,
            window: String,
            flags: u32,
            options: HashMap<String, OwnedVariant>,
            #[zbus(header)] hdr: zbus::message::Header<'_>,
            #[zbus(connection)] conn: &Connection,
        ) -> OwnedObjectPath {
            let reason = String::try_from(options["reason"].try_clone().unwrap()).unwrap();
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let handle = portal::request_path(hdr.sender().unwrap(), &token);
            lock(&self.2).push(handle.clone());
            let n = {
                let mut heard = lock(&self.0);
                heard.push(format!("inhibit {window:?} {flags} {reason}"));
                heard.iter().filter(|h| h.starts_with("inhibit")).count() - 1
            };
            if let Answer::BeforeReply = self.1 {
                hold(conn, &handle, &self.0, n).await;
            }
            OwnedObjectPath::try_from(handle).unwrap()
        }
    }

    /// The backend takes inhibitor `n` at `handle`: its Request is served, then the Response sent.
    async fn hold(conn: &Connection, handle: &str, heard: &Heard, n: usize) {
        crate::testbus::serve(conn, handle, FakeRequest(heard.clone(), n)).await;
        let results: HashMap<&str, Variant<'_>> = HashMap::new();
        (conn.emit_signal(None::<&str>, handle, REQUEST, "Response", &(0u32, results)))
            .await
            .unwrap();
    }

    struct FakeRequest(Heard, usize);

    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl FakeRequest {
        fn close(&self) {
            lock(&self.0).push(format!("close {}", self.1));
        }
    }

    /// A frontend on a bus of its own answering as `answer` says, and a client connection to it.
    fn frontend(answer: Answer, heard: &Heard, handles: &Handles) -> (Bus, Connection, Connection) {
        let bus = bus().expect("a dbus-daemon");
        let (portal, client) = zbus::block_on(async {
            let c = connect(&bus.1).await;
            let fake = FakeInhibit(heard.clone(), answer, handles.clone());
            crate::testbus::serve(&c, PORTAL_PATH, fake).await;
            c.request_name(PORTAL).await.unwrap();
            (c, connect(&bus.1).await)
        });
        (bus, portal, client)
    }

    fn says(heard: &Heard, want: &[&str]) {
        until(&format!("the portal hears {want:?}"), || {
            *lock(heard) == want
        });
    }

    #[test]
    fn a_new_reason_is_held_before_the_old_goes_and_none_lets_go() {
        if bus().is_none() {
            return;
        }
        let heard = Heard::default();
        let (_bus, portal, client) = frontend(Answer::BeforeReply, &heard, &Handles::default());
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || follow(rx, move || Ok(client.clone())));
        tx.send(Some("Copying files".into())).unwrap();
        says(&heard, &["inhibit \"\" 1 Copying files"]);
        tx.send(Some("Copying files".into())).unwrap();
        tx.send(Some("Deleting files".into())).unwrap();
        says(
            &heard,
            &[
                "inhibit \"\" 1 Copying files",
                "inhibit \"\" 1 Deleting files",
                "close 0",
            ],
        );
        tx.send(None).unwrap();
        drop(tx);
        worker.join().unwrap();
        says(
            &heard,
            &[
                "inhibit \"\" 1 Copying files",
                "inhibit \"\" 1 Deleting files",
                "close 0",
                "close 1",
            ],
        );
        drop(portal);
    }

    #[test]
    fn a_close_right_after_the_ask_lets_go_whichever_answer_comes_first() {
        if bus().is_none() {
            return;
        }
        for answer in [Answer::BeforeReply, Answer::Later] {
            let (heard, handles) = (Heard::default(), Handles::default());
            let (_bus, portal, client) = frontend(answer, &heard, &handles);
            let (tx, rx) = mpsc::channel();
            let worker = std::thread::spawn(move || follow(rx, move || Ok(client.clone())));
            tx.send(Some("Copying files".into())).unwrap();
            until("the portal hears the ask", || !lock(&handles).is_empty());
            tx.send(None).unwrap();
            drop(tx);
            if let Answer::Later = answer {
                // Long enough for a Close sent at once to reach the unserved Request.
                std::thread::sleep(Duration::from_millis(300));
                let handle = lock(&handles)[0].clone();
                zbus::block_on(hold(&portal, &handle, &heard, 0));
            }
            worker.join().unwrap();
            says(&heard, &["inhibit \"\" 1 Copying files", "close 0"]);
        }
    }
}
