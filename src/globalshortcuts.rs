//! Global shortcuts, the compositor's half: a `reprise_shortcuts_v1` object
//! per portal session, created for one app id. While an app holds one, the
//! chords of its `[[global_shortcuts]]` entries are taken from the focused
//! window (`keyroute`, after `[[keybinds]]`) and reported to every object
//! of that app. reprise-portal writes the entries and relays the events to
//! the program over D-Bus. A shortcut's key coming up sends deactivated
//! (`keyboard`'s `shortcut_keys`).

use smithay::reexports::wayland_server::Resource;

use crate::config::GlobalShortcut;
use crate::inputproto::proto::reprise_shortcuts_v1::RepriseShortcutsV1;
use crate::keyroute::{GlobalBind, GlobalHit};

/// Objects one client may hold; the portal makes one per session. One
/// more is a `too_many` protocol error that ends the client's whole
/// `reprise_input_v1`, so reprise-portal refuses a session past its
/// `inputlink::MAX_SHORTCUTS`, which must stay equal to this (or lower).
pub const MAX_PER_CLIENT: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShortcutsId(u64);

struct Shortcuts {
    id: ShortcutsId,
    resource: RepriseShortcutsV1,
    app: String,
}

#[derive(Default)]
pub struct ShortcutsState {
    objects: Vec<Shortcuts>,
    next_id: u64,
}

impl ShortcutsState {
    pub fn next_id(&mut self) -> ShortcutsId {
        self.next_id += 1;
        ShortcutsId(self.next_id)
    }

    pub fn create(&mut self, id: ShortcutsId, resource: RepriseShortcutsV1, app: String) {
        self.objects.push(Shortcuts { id, resource, app });
    }

    pub fn destroy(&mut self, id: ShortcutsId) {
        self.objects.retain(|o| o.id != id);
    }

    pub fn count_for_client(&self, resource: &impl Resource) -> usize {
        let id = resource.id();
        (self.objects.iter())
            .filter(|o| o.resource.id().same_client_as(&id))
            .count()
    }

    /// The chords of `entries` whose app holds an object.
    pub fn live(&self, entries: &[GlobalShortcut]) -> Vec<GlobalBind> {
        live_binds(entries, |app| self.objects.iter().any(|o| o.app == app))
    }

    /// Send activated (`pressed`) or deactivated for `hit` to every object
    /// of its app.
    pub fn send(&self, hit: &GlobalHit, pressed: bool, time: u32) {
        for o in self.objects.iter().filter(|o| o.app == hit.app) {
            if pressed {
                o.resource.activated(hit.id.clone(), time);
            } else {
                o.resource.deactivated(hit.id.clone(), time);
            }
        }
    }

    /// The config is about to go from `old` to `new`: `changed` to every
    /// object whose app's entries differ.
    pub fn configured(&self, old: &[GlobalShortcut], new: &[GlobalShortcut]) {
        let apps = changed_apps(old, new);
        for o in self.objects.iter().filter(|o| apps.contains(&o.app)) {
            o.resource.changed();
        }
    }
}

/// The chords of `entries` whose app `held` says holds an object; an
/// entry without a chord binds nothing.
fn live_binds(entries: &[GlobalShortcut], held: impl Fn(&str) -> bool) -> Vec<GlobalBind> {
    entries
        .iter()
        .filter(|e| held(&e.app))
        .filter_map(|e| {
            let (mods, keysym) = e.chord?;
            Some(GlobalBind {
                mods,
                keysym,
                app: e.app.clone(),
                id: e.id.clone(),
            })
        })
        .collect()
}

/// Apps whose entries (ids, descriptions, keys, in order) differ between
/// two configs, each once, in the order first seen in `old` then `new`.
pub fn changed_apps(old: &[GlobalShortcut], new: &[GlobalShortcut]) -> Vec<String> {
    let of = |list: &[GlobalShortcut], app: &str| -> Vec<(String, String, String)> {
        list.iter()
            .filter(|e| e.app == app)
            .map(|e| (e.id.clone(), e.description.clone(), e.keys.clone()))
            .collect()
    };
    let mut out: Vec<String> = Vec::new();
    for e in old.iter().chain(new) {
        if !out.contains(&e.app) && of(old, &e.app) != of(new, &e.app) {
            out.push(e.app.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(app: &str, id: &str, keys: &str) -> GlobalShortcut {
        GlobalShortcut {
            app: app.into(),
            id: id.into(),
            description: String::new(),
            keys: keys.into(),
            chord: None,
        }
    }

    #[test]
    fn only_apps_whose_entries_differ_hear_of_a_reload() {
        let old = [g("a", "x", "F9"), g("b", "y", "F10")];
        let new = [g("a", "x", "F9"), g("b", "y", "F11"), g("c", "z", "")];
        assert_eq!(changed_apps(&old, &new), ["b", "c"]);
        assert_eq!(
            changed_apps(&new, &old),
            ["b", "c"],
            "an app whose entries went counts too"
        );
        assert!(changed_apps(&old, &old).is_empty());
    }

    #[test]
    fn only_held_apps_with_a_chord_bind_keys() {
        let chord = |e: GlobalShortcut, c| GlobalShortcut {
            chord: Some(c),
            ..e
        };
        let entries = [
            chord(g("talk", "ptt", "Pause"), (0, 0xff13)),
            g("talk", "mute", ""),
            chord(g("other", "x", "F9"), (0, 0xffc6)),
        ];
        let binds = live_binds(&entries, |app| app == "talk");
        assert_eq!(
            binds,
            [GlobalBind {
                mods: 0,
                keysym: 0xff13,
                app: "talk".into(),
                id: "ptt".into(),
            }]
        );
        assert!(live_binds(&entries, |_| false).is_empty());
    }
}
