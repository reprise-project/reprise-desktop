use crate::registry::Page;
use crate::{SettingsApp, notifypage, registry, take};
use repriseapp::Reply;

impl SettingsApp {
    /// Ask the compositor which programs sent notifications each time the Notifications page opens, and
    /// keep the list in step with the pending settings. True when the list was refilled.
    pub(crate) fn ensure_notify_programs(&mut self) -> bool {
        if registry::open(&self.sheet) != Page::Notifications {
            self.notify_fresh = false;
            return false;
        }
        if !self.notify_fresh {
            self.notify_fresh = true;
            // Tests leave the session bus alone.
            if !cfg!(test) {
                self.notify_reading = Some(self.spawn(|| {
                    reprisebus::tray::notification_programs().unwrap_or_else(|e| {
                        log::warn!("notification programs: {e}");
                        Vec::new()
                    })
                }));
            }
            self.name_unseen();
        }
        let names = &self.notify_names;
        let programs =
            notifypage::programs(&self.notify_seen, &self.want, |k| names.get(k).cloned());
        let changed = programs != self.notify_programs;
        self.notify_programs = programs;
        let page = &mut self.sheet.pages[Page::Notifications.index()];
        notifypage::fill(page, &self.notify_programs, &self.want);
        changed
    }

    /// The compositor said which programs sent notifications; the next event lists them.
    pub(crate) fn notify_seen_done(&mut self) -> bool {
        let Some(rx) = &self.notify_reading else {
            return false;
        };
        self.notify_seen = match take(rx) {
            Ok(None) => return false,
            Ok(Some(seen)) => seen,
            Err(()) => Vec::new(),
        };
        self.notify_reading = None;
        self.name_unseen();
        true
    }

    /// Name the programs the config lists that this session has not seen.
    fn name_unseen(&mut self) {
        let w = &self.want;
        self.notify_names = (w.notify_no_balloons.iter().chain(&w.notify_no_sounds))
            .filter(|k| !self.notify_seen.iter().any(|(s, _)| s == *k))
            .filter_map(|k| Some((k.clone(), entry_name(k)?)))
            .collect();
    }

    pub(crate) fn notify_changed(&mut self, id: &str) -> Reply {
        let page = &self.sheet.pages[Page::Notifications.index()];
        if notifypage::read(page, &self.notify_programs, id, &mut self.want) {
            self.sync_apply();
        }
        let page = &mut self.sheet.pages[Page::Notifications.index()];
        notifypage::fill(page, &self.notify_programs, &self.want);
        Reply::redraw()
    }
}

/// The name of the desktop entry `key` names, when one is installed.
fn entry_name(key: &str) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let (path, id) = (reprisefs::apps::DesktopId::for_app_id(key).into_iter())
        .find_map(|id| Some((reprisefs::apps::find_file(id.as_str())?, id)))?;
    let text = reprisefs::apps::read_desktop_file(&path).ok()?;
    Some(reprisefs::apps::parse_desktop_any(&text, id.as_str())?.name)
}
