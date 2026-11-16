//! A program holding off log off through the Inhibit portal (`org.freedesktop.portal.Inhibit`'s
//! Logout flag) while it says why, as a file manager does while a copy runs.

use crate::mutex::lock;
use crate::portal::{Options, PORTAL, PORTAL_PATH, REQUEST};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use zbus::{Connection, OwnedObjectPath, Variant};

const INHIBIT: &str = "org.freedesktop.portal.Inhibit";
const FLAG_LOGOUT: u32 = 1;

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
            match bus().and_then(|conn| inhibit(&conn, &why)) {
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

fn inhibit(conn: &Connection, reason: &str) -> zbus::Result<OwnedObjectPath> {
    let options: Options = Options::from([
        ("handle_token", Variant::from(crate::portal::token())),
        ("reason", Variant::from(reason.to_string())),
    ]);
    let reply = crate::wait(conn.call_method(
        Some(PORTAL),
        PORTAL_PATH,
        Some(INHIBIT),
        "Inhibit",
        &("", FLAG_LOGOUT, options),
    ))?;
    reply.body().deserialize()
}

fn close(conn: &Connection, handle: &OwnedObjectPath) -> zbus::Result<()> {
    crate::wait(conn.call_method(Some(PORTAL), handle.as_str(), Some(REQUEST), "Close", &()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testbus::{bus, connect, until};
    use std::sync::Arc;

    type Heard = Arc<Mutex<Vec<String>>>;

    const REQUESTS: [&str; 2] = [
        "/org/freedesktop/portal/desktop/request/1_1/a",
        "/org/freedesktop/portal/desktop/request/1_1/b",
    ];

    /// The frontend's Inhibit, handing out [`REQUESTS`] in turn.
    struct FakeInhibit(Heard);

    #[zbus::interface(name = "org.freedesktop.portal.Inhibit")]
    impl FakeInhibit {
        fn inhibit(
            &self,
            window: String,
            flags: u32,
            options: std::collections::HashMap<String, zbus::OwnedVariant>,
        ) -> OwnedObjectPath {
            let reason = String::try_from(options["reason"].clone()).unwrap();
            let mut heard = lock(&self.0);
            heard.push(format!("inhibit {window:?} {flags} {reason}"));
            let n = heard.iter().filter(|h| h.starts_with("inhibit")).count();
            OwnedObjectPath::try_from(REQUESTS[n - 1]).unwrap()
        }
    }

    struct FakeRequest(Heard, usize);

    #[zbus::interface(name = "org.freedesktop.portal.Request")]
    impl FakeRequest {
        fn close(&self) {
            lock(&self.0).push(format!("close {}", self.1));
        }
    }

    #[test]
    fn a_new_reason_is_held_before_the_old_goes_and_none_lets_go() {
        let Some(bus) = bus() else {
            return;
        };
        let heard = Heard::default();
        let portal = zbus::block_on(async {
            let c = connect(&bus.1).await;
            crate::testbus::serve(&c, PORTAL_PATH, FakeInhibit(heard.clone())).await;
            for (n, path) in REQUESTS.iter().enumerate() {
                crate::testbus::serve(&c, *path, FakeRequest(heard.clone(), n)).await;
            }
            c.request_name(PORTAL).await.unwrap();
            c
        });
        let client = zbus::block_on(connect(&bus.1));
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || follow(rx, move || Ok(client.clone())));
        let says = |want: &[&str]| {
            until(&format!("the portal hears {want:?}"), || {
                *lock(&heard) == want
            });
        };
        tx.send(Some("Copying files".into())).unwrap();
        says(&["inhibit \"\" 1 Copying files"]);
        tx.send(Some("Copying files".into())).unwrap();
        tx.send(Some("Deleting files".into())).unwrap();
        says(&[
            "inhibit \"\" 1 Copying files",
            "inhibit \"\" 1 Deleting files",
            "close 0",
        ]);
        tx.send(None).unwrap();
        drop(tx);
        worker.join().unwrap();
        says(&[
            "inhibit \"\" 1 Copying files",
            "inhibit \"\" 1 Deleting files",
            "close 0",
            "close 1",
        ]);
        drop(portal);
    }
}
