//! Snap groups across a restart, matched to restored windows as they map (`State::snap_on_map`);
//! docs/features.md has the matching rule.

use crate::session::SavedApp;
use reprisekit::snaplayouts::ZoneLayout;
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// A snap group the session ended with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSnap {
    /// The monitor id (`displays::monitor_ids`) of the output it was on.
    pub monitor: String,
    pub desktop: usize,
    /// Its layout (`ZoneLayout`).
    pub columns: bool,
    pub bands: Vec<(f64, u32)>,
    pub members: Vec<SavedZone>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedZone {
    pub zone: usize,
    pub app: String,
    pub title: String,
}

impl SavedSnap {
    pub fn layout(&self) -> ZoneLayout {
        ZoneLayout {
            columns: self.columns,
            bands: self.bands.clone(),
        }
    }
}

/// The snap groups to save from `groups`: members of apps not in `saved` are left out, and so
/// is a group left with none.
pub fn pick_snaps(groups: &[SavedSnap], saved: &[SavedApp]) -> Vec<SavedSnap> {
    let keep = |a: &str| saved.iter().any(|s| s.app_id == a);
    (groups.iter())
        .map(|g| SavedSnap {
            members: (g.members.iter())
                .filter(|m| keep(&m.app))
                .cloned()
                .collect(),
            ..g.clone()
        })
        .filter(|g| !g.members.is_empty())
        .collect()
}

/// How a window was matched to its slot, for the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matched {
    Title,
    Order,
}

/// Where a mapped window goes back to.
#[derive(Clone, Debug, PartialEq)]
pub struct Resnap<T> {
    pub monitor: String,
    pub desktop: usize,
    pub layout: ZoneLayout,
    pub zone: usize,
    /// A member of its group already back, whose group it joins.
    pub with: Option<T>,
    pub matched: Matched,
}

#[derive(Debug)]
struct Slot<T> {
    zone: usize,
    app: String,
    title: String,
    window: Option<T>,
    /// Its window closed: the slot stays empty.
    gone: bool,
}

#[derive(Debug)]
struct Group<T> {
    monitor: String,
    desktop: usize,
    layout: ZoneLayout,
    slots: Vec<Slot<T>>,
}

/// The saved snap groups waiting for their windows, while the login is recent.
#[derive(Debug)]
pub struct PendingSnaps<T> {
    groups: Vec<Group<T>>,
    /// Restored apps' windows outside every group, as (app id, title).
    loose: Vec<(String, String)>,
    until: Option<Instant>,
}

impl<T> Default for PendingSnaps<T> {
    fn default() -> Self {
        PendingSnaps {
            groups: Vec::new(),
            loose: Vec::new(),
            until: None,
        }
    }
}

impl<T: Copy + Eq> PendingSnaps<T> {
    pub fn new(snaps: &[SavedSnap], apps: &[SavedApp], now: Instant) -> PendingSnaps<T> {
        let groups: Vec<Group<T>> = (snaps.iter())
            .map(|g| Group {
                monitor: g.monitor.clone(),
                desktop: g.desktop,
                layout: g.layout(),
                slots: (g.members.iter())
                    .map(|m| Slot {
                        zone: m.zone,
                        app: m.app.clone(),
                        title: m.title.clone(),
                        window: None,
                        gone: false,
                    })
                    .collect(),
            })
            .collect();
        let snapped = |a: &str, t: &str| {
            (groups.iter().flat_map(|g| &g.slots)).any(|s| s.app == a && s.title == t)
        };
        let loose = (apps.iter())
            .flat_map(|a| a.loose.iter().map(|t| (a.app_id.clone(), t.clone())))
            .filter(|(a, t)| !snapped(a, t))
            .collect();
        PendingSnaps {
            groups,
            loose,
            until: Some(now + crate::session::DESKTOP_WINDOW),
        }
    }

    fn open(&self, now: Instant) -> bool {
        self.until.is_some_and(|u| now <= u)
    }

    fn free(&self) -> impl Iterator<Item = (usize, usize, &Slot<T>)> {
        (self.groups.iter().enumerate()).flat_map(|(g, r)| {
            (r.slots.iter().enumerate())
                .filter(|(_, s)| s.window.is_none() && !s.gone)
                .map(move |(i, s)| (g, i, s))
        })
    }

    fn slot_for(&self, app: &str, title: &str, now: Instant) -> Option<(usize, usize, Matched)> {
        if !self.open(now) {
            return None;
        }
        let titled = (!title.is_empty())
            .then(|| {
                self.free()
                    .find(|(_, _, s)| s.app == app && s.title == title)
            })
            .flatten();
        if let Some((g, i, _)) = titled {
            return Some((g, i, Matched::Title));
        }
        if self.loose.iter().any(|(a, t)| a == app && t == title) {
            return None;
        }
        let (g, i, _) = self.free().find(|(_, _, s)| s.app == app)?;
        Some((g, i, Matched::Order))
    }

    /// The desktop and monitor the next window of `app` titled `title` goes back to.
    pub fn place(&self, app: &str, title: &str, now: Instant) -> Option<(usize, &str)> {
        let (g, _, _) = self.slot_for(app, title, now)?;
        let g = &self.groups[g];
        Some((g.desktop, &g.monitor))
    }

    /// Window `w` of `app` titled `title` has mapped: the slot it takes, if any.
    pub fn claim(&mut self, app: &str, title: &str, w: T, now: Instant) -> Option<Resnap<T>> {
        let (g, i, matched) = self.slot_for(app, title, now)?;
        let group = &mut self.groups[g];
        group.slots[i].window = Some(w);
        let with = (group.slots.iter())
            .enumerate()
            .find_map(|(j, s)| s.window.filter(|_| j != i));
        Some(Resnap {
            monitor: group.monitor.clone(),
            desktop: group.desktop,
            layout: group.layout.clone(),
            zone: group.slots[i].zone,
            with,
            matched,
        })
    }

    /// `w` is gone: its slot stays empty for good.
    pub fn forget(&mut self, w: T) {
        for s in self.groups.iter_mut().flat_map(|g| &mut g.slots) {
            if s.window == Some(w) {
                s.window = None;
                s.gone = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(zone: usize, app: &str, title: &str) -> SavedZone {
        SavedZone {
            zone,
            app: app.into(),
            title: title.into(),
        }
    }

    fn pair(members: Vec<SavedZone>) -> SavedSnap {
        SavedSnap {
            monitor: "Dell|U2720Q|1".into(),
            desktop: 1,
            columns: true,
            bands: vec![(1.0, 1), (1.0, 1)],
            members,
        }
    }

    fn app(id: &str, loose: &[&str]) -> SavedApp {
        SavedApp {
            app_id: id.into(),
            entry: format!("{id}.desktop"),
            desktop: 0,
            loose: loose.iter().map(|t| t.to_string()).collect(),
        }
    }

    #[test]
    fn pick_snaps_leaves_out_unsaved_apps_and_emptied_groups() {
        let saved = [app("foot", &[])];
        let groups = [
            pair(vec![zone(0, "foot", "a"), zone(1, "gimp", "b")]),
            pair(vec![zone(0, "gimp", "c")]),
        ];
        assert_eq!(
            pick_snaps(&groups, &saved),
            [pair(vec![zone(0, "foot", "a")])]
        );
    }

    #[test]
    fn a_window_takes_the_slot_saved_with_its_title_first() {
        let t = Instant::now();
        let saved = [pair(vec![
            zone(0, "foot", "left"),
            zone(1, "foot", "right"),
        ])];
        let mut p = PendingSnaps::new(&saved, &[app("foot", &["left", "right"])], t);
        let r = p.claim("foot", "right", 1, t).expect("the right slot");
        assert_eq!((r.zone, r.with, r.matched), (1, None, Matched::Title));
        assert_eq!((r.desktop, r.monitor.as_str()), (1, "Dell|U2720Q|1"));
        let l = p.claim("foot", "", 2, t).expect("the left slot by order");
        assert_eq!((l.zone, l.with, l.matched), (0, Some(1), Matched::Order));
        assert_eq!(p.claim("foot", "", 3, t), None, "both slots are full");
    }

    #[test]
    fn a_window_titled_as_one_outside_the_groups_takes_no_slot() {
        let t = Instant::now();
        let saved = [pair(vec![zone(0, "ed", "a.txt")])];
        let mut p = PendingSnaps::new(&saved, &[app("ed", &["a.txt", "notes.txt"])], t);
        assert_eq!(p.place("ed", "notes.txt", t), None);
        assert_eq!(p.claim("ed", "notes.txt", 1, t), None);
        assert_eq!(p.claim("other", "", 2, t), None, "an app with no slot");
        assert!(p.claim("ed", "untitled", 3, t).is_some());
    }

    #[test]
    fn a_closed_windows_slot_stays_empty_and_the_wait_ends() {
        let t = Instant::now();
        let saved = [pair(vec![zone(0, "foot", ""), zone(1, "foot", "")])];
        let mut p = PendingSnaps::new(&saved, &[], t);
        p.claim("foot", "", 1, t);
        p.forget(1);
        let r = p.claim("foot", "", 2, t).expect("the second slot");
        assert_eq!((r.zone, r.with), (1, None), "the first is gone");
        assert_eq!(p.claim("foot", "", 3, t), None);
        let mut p = PendingSnaps::<u32>::new(&saved, &[], t);
        let late = t + crate::session::DESKTOP_WINDOW + std::time::Duration::from_secs(1);
        assert_eq!(p.claim("foot", "", 1, late), None);
    }
}
