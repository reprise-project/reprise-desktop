//! A program's progress on its task button through `com.canonical.Unity.LauncherEntry`'s `Update`
//! signal, with Reprise's own `progress-paused` key beside the convention's.

use crate::mutex::lock;
use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::mpsc::{self, Sender};
use zbus::zvariant::Value;

const IFACE: &str = "com.canonical.Unity.LauncherEntry";
const PATH: &str = "/com/canonical/unity/launcherentry/reprise";

/// An app id and the progress to send for it.
type Job = (String, Option<(f64, bool)>);

/// Progress for the desktop file `app_id` (without `.desktop`): `Some((fraction, paused))` shows
/// it, `None` takes it away. Sent from a thread of its own, in order, so the caller never waits.
pub fn update(app_id: &str, progress: Option<(f64, bool)>) {
    static WORKER: Mutex<Option<Sender<Job>>> = Mutex::new(None);
    let mut worker = lock(&WORKER);
    let mut job = (app_id.to_string(), progress);
    if let Some(tx) = worker.as_ref() {
        match tx.send(job) {
            Ok(()) => return,
            Err(e) => job = e.0,
        }
    }
    let (tx, rx) = mpsc::channel::<Job>();
    let spawned = std::thread::Builder::new()
        .name("launcher-entry".into())
        .spawn(move || {
            for (app, p) in rx {
                if let Err(e) = send(&app, p) {
                    log::info!("launcher entry for {app}: {e}");
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("launcher entry: no thread: {e}");
        return;
    }
    let _ = tx.send(job);
    *worker = Some(tx);
}

fn send(app_id: &str, progress: Option<(f64, bool)>) -> zbus::Result<()> {
    let conn = crate::session()?;
    let (fraction, paused) = progress.unwrap_or_default();
    let props: HashMap<&str, Value<'_>> = HashMap::from([
        ("progress", Value::from(fraction.clamp(0.0, 1.0))),
        ("progress-visible", Value::from(progress.is_some())),
        ("progress-paused", Value::from(paused)),
    ]);
    let uri = format!("application://{app_id}.desktop");
    conn.emit_signal(None::<&str>, PATH, IFACE, "Update", &(uri, props))
}
