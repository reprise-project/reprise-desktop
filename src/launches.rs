//! The Glass Start menu's pinned programs and recent launches (desktop file ids),
//! persisted in `state.toml`.

use reprisefs::apps::{AppEntry, DesktopId};
use std::collections::HashMap;

pub const HISTORY: usize = 100;
pub const FREQUENT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launches {
    pub pins: Vec<DesktopId>,
    pub recent: Vec<DesktopId>,
}

impl Launches {
    /// The lists `state.toml` holds; a name no desktop file can have is dropped.
    pub fn from_saved(pins: &[String], recent: &[String]) -> Launches {
        let ids = |v: &[String]| {
            v.iter()
                .filter_map(|s| DesktopId::from_file_name(s))
                .collect()
        };
        Launches {
            pins: ids(pins),
            recent: ids(recent),
        }
    }

    pub fn record(&mut self, id: &DesktopId) {
        self.recent.insert(0, id.clone());
        self.recent.truncate(HISTORY);
    }

    /// Indices into `apps` of the `n` most launched shown programs that
    /// are not pinned: most launches first, ties to the newest.
    pub fn frequent(&self, apps: &[AppEntry], n: usize) -> Vec<usize> {
        let mut count: HashMap<&DesktopId, (usize, usize)> = HashMap::new();
        for (age, id) in self.recent.iter().enumerate() {
            let e = count.entry(id).or_insert((0, age));
            e.0 += 1;
        }
        let mut ranked: Vec<(&DesktopId, usize, usize)> = count
            .into_iter()
            .map(|(id, (c, age))| (id, c, age))
            .collect();
        ranked.sort_by_key(|&(_, c, age)| (std::cmp::Reverse(c), age));
        ranked
            .into_iter()
            .filter(|(id, ..)| !self.is_pinned(id))
            .filter_map(|(id, ..)| apps.iter().position(|a| a.id == *id && a.shown))
            .take(n)
            .collect()
    }

    pub fn pinned(&self, apps: &[AppEntry]) -> Vec<usize> {
        self.pins
            .iter()
            .filter_map(|id| apps.iter().position(|a| &a.id == id))
            .collect()
    }

    pub fn pin(&mut self, id: &DesktopId) {
        if !self.is_pinned(id) {
            self.pins.push(id.clone());
        }
    }

    pub fn unpin(&mut self, id: &DesktopId) {
        self.pins.retain(|p| p != id);
    }

    pub fn forget(&mut self, id: &DesktopId) {
        self.recent.retain(|r| r != id);
    }

    pub fn is_pinned(&self, id: &DesktopId) -> bool {
        self.pins.iter().any(|p| p == id)
    }
}

pub fn starters(apps: &[AppEntry]) -> Vec<usize> {
    let mut out: Vec<usize> = [
        "reprise-explorer.desktop",
        "reprise-notepad.desktop",
        "reprise-settings.desktop",
    ]
    .iter()
    .filter_map(|id| apps.iter().position(|a| a.id == *id && a.shown))
    .collect();
    if let Some(t) = apps
        .iter()
        .position(|a| a.shown && a.in_category("TerminalEmulator"))
    {
        out.push(t);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str) -> AppEntry {
        AppEntry {
            name: id.parse::<DesktopId>().unwrap().stem().into(),
            exec_line: id.into(),
            terminal: false,
            id: id.parse().unwrap(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
            path: None,
        }
    }

    #[test]
    fn frequent_ranks_by_count_then_recency() {
        let apps = [app("a.desktop"), app("b.desktop"), app("c.desktop")];
        let mut l = Launches::default();
        for id in ["a.desktop", "b.desktop", "b.desktop", "c.desktop"] {
            l.record(&id.parse().unwrap());
        }
        assert_eq!(l.frequent(&apps, 10), [1, 2, 0]);
    }

    #[test]
    fn frequent_skips_pins_and_ids_no_longer_installed() {
        let apps = [app("a.desktop"), app("b.desktop")];
        let mut l = Launches::default();
        for id in ["gone.desktop", "a.desktop", "b.desktop"] {
            l.record(&id.parse().unwrap());
        }
        l.pin(&"a.desktop".parse().unwrap());
        assert_eq!(l.frequent(&apps, 10), [1]);
    }

    #[test]
    fn pins_resolve_in_order_and_skip_missing() {
        let apps = [app("a.desktop"), app("b.desktop")];
        let mut l = Launches::default();
        l.pin(&"b.desktop".parse().unwrap());
        l.pin(&"gone.desktop".parse().unwrap());
        l.pin(&"a.desktop".parse().unwrap());
        l.pin(&"b.desktop".parse().unwrap());
        assert_eq!(l.pinned(&apps), [1, 0]);
    }

    #[test]
    fn a_saved_name_no_desktop_file_can_have_is_dropped() {
        let saved = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let l = Launches::from_saved(
            &saved(&["a.desktop", "a"]),
            &saved(&[".x.desktop", "b.desktop"]),
        );
        assert_eq!(l.pins, ["a.desktop"]);
        assert_eq!(l.recent, ["b.desktop"]);
    }

    #[test]
    fn history_keeps_the_newest_hundred_and_forget_drops_every_launch() {
        let mut l = Launches::default();
        for k in 0..120 {
            l.record(&format!("{k}.desktop").parse().unwrap());
        }
        assert_eq!(l.recent.len(), HISTORY);
        assert_eq!(l.recent[0], "119.desktop");
        l.record(&"x.desktop".parse().unwrap());
        l.record(&"x.desktop".parse().unwrap());
        l.forget(&"x.desktop".parse().unwrap());
        assert!(!l.recent.iter().any(|i| i == "x.desktop"));
    }

    #[test]
    fn starters_list_the_installed_own_programs_then_a_terminal() {
        let mut term = app("foot.desktop");
        term.categories = vec!["TerminalEmulator".into()];
        let apps = [
            term,
            app("reprise-settings.desktop"),
            app("reprise-explorer.desktop"),
        ];
        assert_eq!(starters(&apps), [2, 1, 0]);
    }
}
