//! The Glass Start menu's pinned programs and recent launches (desktop file ids),
//! persisted in `state.toml`.

use reprisefs::apps::AppEntry;
use std::collections::HashMap;

pub const HISTORY: usize = 100;
pub const FREQUENT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launches {
    pub pins: Vec<String>,
    pub recent: Vec<String>,
}

impl Launches {
    pub fn record(&mut self, id: &str) {
        self.recent.insert(0, id.to_string());
        self.recent.truncate(HISTORY);
    }

    /// Indices into `apps` of the `n` most launched shown programs that
    /// are not pinned: most launches first, ties to the newest.
    pub fn frequent(&self, apps: &[AppEntry], n: usize) -> Vec<usize> {
        let mut count: HashMap<&str, (usize, usize)> = HashMap::new();
        for (age, id) in self.recent.iter().enumerate() {
            let e = count.entry(id.as_str()).or_insert((0, age));
            e.0 += 1;
        }
        let mut ranked: Vec<(&str, usize, usize)> = count
            .into_iter()
            .map(|(id, (c, age))| (id, c, age))
            .collect();
        ranked.sort_by_key(|&(_, c, age)| (std::cmp::Reverse(c), age));
        ranked
            .into_iter()
            .filter(|(id, ..)| !self.is_pinned(id))
            .filter_map(|(id, ..)| apps.iter().position(|a| a.id == id && a.shown))
            .take(n)
            .collect()
    }

    pub fn pinned(&self, apps: &[AppEntry]) -> Vec<usize> {
        self.pins
            .iter()
            .filter_map(|id| apps.iter().position(|a| &a.id == id))
            .collect()
    }

    pub fn pin(&mut self, id: &str) {
        if !self.is_pinned(id) {
            self.pins.push(id.to_string());
        }
    }

    pub fn unpin(&mut self, id: &str) {
        self.pins.retain(|p| p != id);
    }

    pub fn forget(&mut self, id: &str) {
        self.recent.retain(|r| r != id);
    }

    pub fn is_pinned(&self, id: &str) -> bool {
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
            name: id.trim_end_matches(".desktop").into(),
            exec_line: id.into(),
            terminal: false,
            id: id.into(),
            icon: None,
            startup_wm_class: None,
            categories: Vec::new(),
            mime_types: Vec::new(),
            shown: true,
        }
    }

    #[test]
    fn frequent_ranks_by_count_then_recency() {
        let apps = [app("a.desktop"), app("b.desktop"), app("c.desktop")];
        let mut l = Launches::default();
        for id in ["a.desktop", "b.desktop", "b.desktop", "c.desktop"] {
            l.record(id);
        }
        assert_eq!(l.frequent(&apps, 10), [1, 2, 0]);
    }

    #[test]
    fn frequent_skips_pins_and_ids_no_longer_installed() {
        let apps = [app("a.desktop"), app("b.desktop")];
        let mut l = Launches::default();
        for id in ["gone.desktop", "a.desktop", "b.desktop"] {
            l.record(id);
        }
        l.pin("a.desktop");
        assert_eq!(l.frequent(&apps, 10), [1]);
    }

    #[test]
    fn pins_resolve_in_order_and_skip_missing() {
        let apps = [app("a.desktop"), app("b.desktop")];
        let mut l = Launches::default();
        l.pin("b.desktop");
        l.pin("gone.desktop");
        l.pin("a.desktop");
        l.pin("b.desktop");
        assert_eq!(l.pinned(&apps), [1, 0]);
    }

    #[test]
    fn history_keeps_the_newest_hundred_and_forget_drops_every_launch() {
        let mut l = Launches::default();
        for k in 0..120 {
            l.record(&format!("{k}.desktop"));
        }
        assert_eq!(l.recent.len(), HISTORY);
        assert_eq!(l.recent[0], "119.desktop");
        l.record("x.desktop");
        l.record("x.desktop");
        l.forget("x.desktop");
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
