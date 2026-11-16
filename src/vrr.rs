//! Adaptive sync per output. The policy is `adaptive_sync` (top-level or `[[outputs]]`), or what
//! output management last set until the config names the output again. Under `"fullscreen"` an
//! output runs VRR while the focused window is fullscreen on it: the window `restack` lifts above
//! the taskbar. Content type hints are not read.

use smithay::output::Output;

use crate::backend::VrrSet;
use crate::compositor::State;
use crate::config::VrrPolicy;

/// Whether an output under `policy` runs VRR; `fullscreen` says the focused window is fullscreen
/// on it.
pub fn wanted(policy: VrrPolicy, fullscreen: bool) -> bool {
    match policy {
        VrrPolicy::Off => false,
        VrrPolicy::On => true,
        VrrPolicy::Fullscreen => fullscreen,
    }
}

impl State {
    pub fn vrr_policy(&self, name: &str) -> VrrPolicy {
        (self.vrr_policy.get(name).copied())
            .unwrap_or_else(|| self.config.adaptive_sync_for_output_name(name))
    }

    fn vrr_wanted(&self, output: &Output) -> bool {
        let fullscreen = self.fullscreen_on(self.layout_box(Some(output)));
        wanted(self.vrr_policy(&output.name()), fullscreen)
    }

    /// Turn VRR on or off on every driven output that can run it, as [`wanted`] says. A change
    /// the busy display refuses sets `vrr_stale`, and `backend::on_quiet` runs this again.
    pub fn sync_vrr(&mut self) {
        self.vrr_stale = false;
        let outputs = self.scene.outputs().to_vec();
        let mut changed = false;
        for output in outputs {
            let Some((capable, on)) = crate::backend::vrr_of(self, &output) else {
                continue;
            };
            let want = capable && self.vrr_wanted(&output);
            if want == on {
                continue;
            }
            match crate::backend::try_vrr(self, &output, want) {
                VrrSet::Done => changed = true,
                VrrSet::Busy => self.vrr_stale = true,
                VrrSet::Refused => {}
            }
        }
        if changed {
            crate::outputmgmt::publish(self);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::Harness;

    fn under(config: &str) -> Harness {
        let mut h = Harness::with_config(crate::config::Config::parse(config).unwrap());
        h.state.test_vrr.capable = true;
        h.head("DP-1", 1920, 1080);
        h.head("DP-2", 1920, 1080);
        h
    }

    fn set(h: &Harness) -> Vec<(&str, bool)> {
        (h.state.test_vrr.set.iter())
            .map(|(n, on)| (n.as_str(), *on))
            .collect()
    }

    #[test]
    fn fullscreen_turns_vrr_on_only_while_the_focused_window_is_fullscreen_on_the_output() {
        let mut h = under("adaptive_sync = \"fullscreen\"\n");
        let mut c = h.connect();
        let game = h.map_toplevel(&mut c, "game", 300, 200);
        assert_eq!(set(&h), [], "a window that is not fullscreen");
        h.state.set_fullscreen(game.id, true);
        assert_eq!(set(&h), [("DP-1", true)], "only the output it fills");
        h.state.set_fullscreen(game.id, false);
        assert_eq!(set(&h), [("DP-1", true), ("DP-1", false)]);
    }

    #[test]
    fn on_and_off_ignore_fullscreen() {
        for (config, want) in [("adaptive_sync = true\n", true), ("", false)] {
            let mut h = under(config);
            let mut c = h.connect();
            let game = h.map_toplevel(&mut c, "game", 300, 200);
            h.state.set_fullscreen(game.id, true);
            h.state.set_fullscreen(game.id, false);
            let on: Vec<_> = h.state.test_vrr.on.clone();
            assert_eq!(on.len(), if want { 2 } else { 0 }, "{config:?}: {on:?}");
            assert!(
                set(&h).iter().all(|(_, on)| *on),
                "{config:?}: never turned off"
            );
        }
    }

    #[test]
    fn a_change_the_busy_display_refuses_takes_once_it_is_idle() {
        let mut h = under("adaptive_sync = \"fullscreen\"\n");
        let mut c = h.connect();
        let game = h.map_toplevel(&mut c, "game", 300, 200);
        h.state.test_vrr.busy = true;
        h.state.set_fullscreen(game.id, true);
        assert_eq!(set(&h), []);
        assert!(h.state.vrr_stale);
        crate::backend::fake::idle(&mut h.state);
        assert_eq!(set(&h), [("DP-1", true)]);
        assert!(!h.state.vrr_stale);
    }

    #[test]
    fn a_reload_to_fullscreen_turns_an_output_off_until_a_window_is_fullscreen() {
        let mut h = under("adaptive_sync = true\n");
        let mut c = h.connect();
        let game = h.map_toplevel(&mut c, "game", 300, 200);
        assert_eq!(h.state.test_vrr.on.len(), 2);
        let full = crate::config::Config::parse("adaptive_sync = \"fullscreen\"\n").unwrap();
        crate::configwatch::reload(&mut h.state, full);
        assert_eq!(h.state.test_vrr.on, Vec::<String>::new());
        h.state.set_fullscreen(game.id, true);
        assert_eq!(h.state.test_vrr.on, ["DP-1"]);
    }

    #[test]
    fn only_fullscreen_follows_the_focused_window() {
        for fullscreen in [false, true] {
            assert!(!wanted(VrrPolicy::Off, fullscreen));
            assert!(wanted(VrrPolicy::On, fullscreen));
            assert_eq!(wanted(VrrPolicy::Fullscreen, fullscreen), fullscreen);
        }
    }
}
