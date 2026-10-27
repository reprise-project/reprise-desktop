//! `org.freedesktop.impl.portal.Background`: which apps run with and
//! without a window, and the "running in the background" prompt.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;
use zbus::object_server::SignalEmitter;
use zbus::{ObjectPath, OwnedVariant};

const STATE_BACKGROUND: u32 = 0;
const STATE_RUNNING: u32 = 1;

const RESULT_FORBID: u32 = 0;
const RESULT_ALLOW: u32 = 1;
const RESULT_ALLOW_ONCE: u32 = 2;

/// How long after a change to the list of sandboxed apps with a window the
/// list is read again for RunningApplicationsChanged, so a window swapped for
/// another within it sends nothing.
const SETTLE: Duration = Duration::from_millis(500);

pub struct Background {
    pub frontend: crate::Frontend,
    pub link: Option<crate::inputlink::Link>,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Background")]
impl Background {
    #[zbus(property, name = "version")]
    fn version(&self) -> u32 {
        1
    }

    async fn get_app_state(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedVariant>> {
        self.frontend.check(&hdr).await?;
        // With either list missing, every app with a window would read as in the background, and
        // xdg-desktop-portal kills a background app the user forbade; an error skips the round.
        let sandboxed = (self.link.as_ref())
            .and_then(crate::inputlink::Link::sandboxed)
            .ok_or_else(|| zbus::fdo::Error::Failed("no list of sandboxed windows".into()))?;
        let states = blocking::unblock(move || {
            let wl = crate::Wayland::connect().map_err(|e| {
                log::warn!("GetAppState: {e}");
                zbus::fdo::Error::Failed(e)
            })?;
            let windowed: Vec<String> = wl.toplevels().into_iter().map(|t| t.app_id).collect();
            Ok::<_, zbus::fdo::Error>(app_states(&running_flatpaks(), &windowed, &sandboxed))
        })
        .await?;
        Ok(states
            .into_iter()
            .map(|(id, s)| (id, OwnedVariant::from(s)))
            .collect())
    }

    #[zbus(signal)]
    async fn running_applications_changed(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    async fn notify_background(
        &self,
        #[zbus(header)] hdr: zbus::message::Header<'_>,
        handle: ObjectPath<'_>,
        app_id: String,
        name: String,
        #[zbus(object_server)] server: &zbus::ObjectServer,
    ) -> zbus::fdo::Result<(u32, HashMap<String, OwnedVariant>)> {
        self.frontend.check(&hdr).await?;
        let cancel = Arc::new(AtomicBool::new(false));
        crate::export_request(server, &handle, cancel.clone(), &self.frontend).await?;
        let id = app_id.clone();
        let answer = blocking::unblock(move || {
            let app = if name.trim().is_empty() {
                crate::remotedesktop::display_name(&id)
            } else {
                name
            };
            let input = crate::permit::Input {
                parent: None,
                app,
                offer_remember: true,
                items: Vec::new(),
                preview: None,
            };
            crate::permit::ask_with(crate::permit::Kind::Background, input, &cancel)
        })
        .await;
        let _ = server.remove::<crate::Request, _>(handle.clone()).await;
        let result = background_result(answer);
        log::info!("NotifyBackground for {app_id:?}: {result}");
        let mut r = HashMap::new();
        r.insert("result".to_string(), OwnedVariant::from(result));
        Ok((crate::RESPONSE_OK, r))
    }
}

/// Send RunningApplicationsChanged each time the compositor's list of
/// sandboxed apps with a window settles on a new set.
pub async fn announce(conn: zbus::Connection, link: crate::inputlink::Link) {
    let Ok(emitter) = SignalEmitter::new(&conn, crate::PATH) else {
        return;
    };
    let changes = link.sandboxed_changes();
    let mut emitted = link.sandboxed().unwrap_or_default();
    while changes.recv().await.is_ok() {
        async_io::Timer::after(SETTLE).await;
        while changes.try_recv().is_ok() {}
        if settled(&mut emitted, link.sandboxed().unwrap_or_default())
            && let Err(e) = Background::running_applications_changed(&emitter).await
        {
            log::warn!("RunningApplicationsChanged: {e}");
        }
    }
}

/// Whether `now` differs from the set last announced, taking it as the
/// new `emitted` when it does.
fn settled(emitted: &mut BTreeSet<String>, now: BTreeSet<String>) -> bool {
    if *emitted == now {
        return false;
    }
    *emitted = now;
    true
}

/// The frontend's answer for the prompt: Allow with Remember allows for
/// good, Deny forbids, and Allow alone or no answer allows this run.
fn background_result(answer: crate::permit::Answer) -> u32 {
    use crate::permit::Answer;
    match answer {
        Answer::Allow { remember: true } => RESULT_ALLOW,
        Answer::Deny => RESULT_FORBID,
        Answer::Allow { remember: false } | Answer::Dismissed => RESULT_ALLOW_ONCE,
    }
}

/// Every app id with a window as running, and every other app in
/// `running` as in the background. `windowed` holds the windows' own app
/// ids, `sandboxed` the sandbox app ids of the clients with a window.
fn app_states(
    running: &[String],
    windowed: &[String],
    sandboxed: &BTreeSet<String>,
) -> HashMap<String, u32> {
    let mut out: HashMap<String, u32> = running
        .iter()
        .filter(|id| !id.is_empty())
        .map(|id| (id.clone(), STATE_BACKGROUND))
        .collect();
    for id in windowed.iter().chain(sandboxed) {
        if !id.is_empty() && !crate::HIDDEN_APPS.contains(&id.as_str()) {
            out.insert(id.clone(), STATE_RUNNING);
        }
    }
    out
}

/// The app ids of the live Flatpak instances under
/// `$XDG_RUNTIME_DIR/.flatpak`.
fn running_flatpaks() -> Vec<String> {
    let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") else {
        return Vec::new();
    };
    let Ok(dirs) = std::fs::read_dir(Path::new(&runtime).join(".flatpak")) else {
        return Vec::new();
    };
    let ids: BTreeSet<String> = dirs
        .flatten()
        .filter_map(|d| instance_app(&d.path()))
        .collect();
    ids.into_iter().collect()
}

/// The app id of the instance in `dir`, if its process still runs.
fn instance_app(dir: &Path) -> Option<String> {
    let pid = std::fs::read_to_string(dir.join("pid")).ok()?;
    let pid: u32 = pid.trim().parse().ok()?;
    if !Path::new(&format!("/proc/{pid}")).exists() {
        return None;
    }
    reprisefs::flatpak::app_of_info(&std::fs::read_to_string(dir.join("info")).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn an_app_with_a_window_runs_and_one_without_is_in_the_background() {
        let states = app_states(
            &strings(&["org.example.Chat", "org.example.Sync", ""]),
            &strings(&["org.example.Chat", "foot", "reprise.portal", ""]),
            &BTreeSet::new(),
        );
        assert_eq!(states.len(), 3, "{states:?}");
        assert_eq!(states["org.example.Chat"], STATE_RUNNING);
        assert_eq!(states["foot"], STATE_RUNNING);
        assert_eq!(states["org.example.Sync"], STATE_BACKGROUND);
    }

    #[test]
    fn an_app_whose_sandbox_has_a_window_runs_whatever_the_window_calls_itself() {
        let states = app_states(
            &strings(&["com.valvesoftware.Steam", "org.example.Sync"]),
            &strings(&["steam"]),
            &BTreeSet::from(["com.valvesoftware.Steam".to_string()]),
        );
        assert_eq!(states["com.valvesoftware.Steam"], STATE_RUNNING);
        assert_eq!(states["org.example.Sync"], STATE_BACKGROUND);
    }

    #[test]
    fn a_change_is_announced_once_and_a_set_back_where_it_was_not_at_all() {
        let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<BTreeSet<_>>();
        let mut emitted = set(&["org.example.Chat"]);
        assert!(!settled(&mut emitted, set(&["org.example.Chat"])));
        assert!(settled(
            &mut emitted,
            set(&["org.example.Chat", "org.example.Sync"])
        ));
        assert!(!settled(
            &mut emitted,
            set(&["org.example.Chat", "org.example.Sync"])
        ));
        assert!(settled(&mut emitted, set(&[])));
        assert_eq!(emitted, set(&[]));
    }

    #[test]
    fn allow_with_remember_allows_for_good_and_no_answer_allows_once() {
        use crate::permit::Answer;
        assert_eq!(
            background_result(Answer::Allow { remember: true }),
            RESULT_ALLOW
        );
        assert_eq!(
            background_result(Answer::Allow { remember: false }),
            RESULT_ALLOW_ONCE
        );
        assert_eq!(background_result(Answer::Dismissed), RESULT_ALLOW_ONCE);
        assert_eq!(background_result(Answer::Deny), RESULT_FORBID);
    }
}
