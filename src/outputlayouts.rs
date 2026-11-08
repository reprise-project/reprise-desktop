//! Saved arrangements per set of connected monitors (`repriseconf::displays`): looked up when the
//! set changes, saved after a reload that changed `[[outputs]]`.

use repriseconf::displays::{self, Layout, Layouts, Monitor, MonitorLayout};
use smithay::output::Output;

use crate::{
    backend::HeadState,
    compositor::State,
    output::{HeadRequest, ModeInfo, pick_mode, transform},
};

pub struct SavedLayouts {
    /// The connected monitors' ids, sorted, as last looked up.
    set: Vec<String>,
    /// The saved layout in force, by connector; None while `[[outputs]]` and automatic placement
    /// decide.
    active: Option<Vec<(String, MonitorLayout)>>,
    /// The `[[outputs]]` fingerprint the file carries when no one else changed it: the config's
    /// at start, then each save's or refusal's.
    seen: Option<String>,
    /// Connectors whose `[[outputs]]` mode was refused: each later `[[outputs]]` change tries them
    /// again and saves no layout while one stays refused.
    refused: Vec<String>,
    /// Where layouts are saved (displays.toml).
    pub path: std::path::PathBuf,
}

impl Default for SavedLayouts {
    fn default() -> SavedLayouts {
        SavedLayouts {
            set: Vec::new(),
            active: None,
            seen: None,
            refused: Vec::new(),
            path: file(),
        }
    }
}

impl SavedLayouts {
    pub fn get(&self, connector: &str) -> Option<&MonitorLayout> {
        let active = self.active.as_ref()?;
        active.iter().find(|(c, _)| c == connector).map(|(_, m)| m)
    }

    pub fn primary(&self) -> Option<&str> {
        let active = self.active.as_ref()?;
        active
            .iter()
            .find(|(_, m)| m.primary)
            .map(|(c, _)| c.as_str())
    }
}

fn file() -> std::path::PathBuf {
    #[cfg(test)]
    return crate::harness::scratch().join("displays.toml");
    #[cfg(not(test))]
    displays::path()
}

/// The ids of `heads`' monitors, in their order.
fn ids_of(heads: &[HeadState]) -> Vec<String> {
    let outputs: Vec<&Output> = heads.iter().map(|h| &h.output).collect();
    monitor_ids(&outputs)
}

/// `outputs`' monitor ids (`displays::monitor_ids`), in their order.
pub fn monitor_ids(outputs: &[&Output]) -> Vec<String> {
    let props: Vec<_> = (outputs.iter())
        .map(|o| (o.name(), o.physical_properties()))
        .collect();
    let monitors: Vec<Monitor> = (props.iter())
        .map(|(name, p)| Monitor {
            connector: name,
            make: &p.make,
            model: &p.model,
            serial: &p.serial_number,
        })
        .collect();
    displays::monitor_ids(&monitors)
}

/// `layout`'s monitors by connector, given the connected `connectors` and their `ids` alike.
pub fn by_connector(
    connectors: &[String],
    ids: &[String],
    layout: &Layout,
) -> Vec<(String, MonitorLayout)> {
    (connectors.iter().zip(ids))
        .filter_map(|(c, id)| {
            let (_, m) = layout.monitors.iter().find(|(i, _)| i == id)?;
            Some((c.clone(), m.clone()))
        })
        .collect()
}

/// What `apply_head` runs for a monitor saved as `m`; a mode is asked only when it differs from
/// `current` or the monitor is not `driven`.
pub fn head_request(
    m: &MonitorLayout,
    modes: &[ModeInfo],
    current: Option<usize>,
    driven: bool,
    adaptive_sync: bool,
) -> HeadRequest {
    let mode = pick_mode(modes, m.mode.as_ref()).filter(|&i| !driven || Some(i) != current);
    HeadRequest {
        enabled: m.enabled,
        mode,
        position: Some(m.position),
        transform: transform(m.transform),
        scale: f64::from(m.scale),
        adaptive_sync,
    }
}

impl State {
    /// When the connected set changed and has a saved layout, run it on every monitor and lay
    /// out. False otherwise: the caller places a new monitor from `[[outputs]]`.
    pub fn monitors_changed(&mut self) -> bool {
        let heads = crate::backend::heads(self);
        let ids = ids_of(&heads);
        let mut set = ids.clone();
        set.sort();
        if set == self.saved_layouts.set {
            return false;
        }
        self.saved_layouts.set = set;
        let seen = (self.saved_layouts.seen)
            .get_or_insert_with(|| displays::outputs_fingerprint(&self.config.outputs))
            .clone();
        let layouts =
            Layouts::parse(&std::fs::read_to_string(&self.saved_layouts.path).unwrap_or_default());
        let connectors: Vec<String> = heads.iter().map(|h| h.output.name()).collect();
        let active = (layouts.find(&ids, &seen)).map(|l| by_connector(&connectors, &ids, l));
        self.saved_layouts.active = active.clone();
        let Some(active) = active else {
            log::info!("monitors {connectors:?}: no saved layout");
            return false;
        };
        log::info!("monitors {connectors:?}: their saved layout");
        for head in &heads {
            let name = head.output.name();
            let Some((_, m)) = active.iter().find(|(c, _)| *c == name) else {
                continue;
            };
            let vrr = self.config.adaptive_sync_for_output_name(&name);
            let req = head_request(m, &head.modes, head.current, head.enabled, vrr);
            if !self.apply_head(&head.output, &req) {
                log::warn!("output {name}: the saved layout was refused");
            }
        }
        for head in &heads {
            let on = self
                .saved_layouts
                .get(&head.output.name())
                .is_some_and(|m| m.enabled);
            if on && !self.scene.outputs().contains(&head.output) {
                self.set_in_layout(&head.output, true);
            }
        }
        self.update_layout();
        true
    }

    /// The saved layout's settings for `output`, when one is in force.
    pub fn saved_layout_of(&self, output: &Output) -> Option<&MonitorLayout> {
        self.saved_layouts.get(&output.name())
    }

    pub fn saved_primary(&self) -> Option<&str> {
        self.saved_layouts.primary()
    }

    /// `[[outputs]]` changed and was applied: it decides until the arrangement it leaves is saved
    /// for the connected monitors, once the reload has laid out.
    pub fn outputs_reloaded(&mut self) {
        self.saved_layouts.active = None;
        self.saved_layouts.refused.clear();
        self.handle.insert_idle(|state| state.save_layout());
    }

    /// As `outputs_reloaded`, but the mode `[[outputs]]` pins for `connectors` was refused. The
    /// connected set's layout is dropped instead of saved, so the next start tries the pin; the
    /// other sets' layouts move to the new `[[outputs]]` as a save would move them.
    pub fn outputs_refused(&mut self, connectors: Vec<String>) {
        self.saved_layouts.active = None;
        self.saved_layouts.refused = connectors;
        let mut set = ids_of(&crate::backend::heads(self));
        set.sort();
        let now = displays::outputs_fingerprint(&self.config.outputs);
        let expected =
            (self.saved_layouts.seen.replace(now.clone())).unwrap_or_else(|| now.clone());
        let path = self.saved_layouts.path.clone();
        crate::state::run_in_order(move || {
            let saved = repriseconf::edit_or_reset(&path, |old| {
                let mut l = Layouts::parse(old);
                if l.outputs_seen != expected {
                    l.list.clear();
                }
                l.list
                    .retain(|layout| !layout.monitors.iter().map(|(id, _)| id).eq(set.iter()));
                l.outputs_seen = now;
                Ok::<_, std::convert::Infallible>(l.render())
            });
            if let Err(e) = saved {
                log::warn!("{}: {e}", path.display());
            }
        });
    }

    /// Whether the last `[[outputs]]` change had `connector`'s mode refused; the next one tries
    /// it again.
    pub fn mode_refused(&self, connector: &str) -> bool {
        self.saved_layouts.refused.iter().any(|c| c == connector)
    }

    /// Output management or a saved layout set `connector`'s mode: no longer retried as refused.
    pub fn mode_set(&mut self, connector: &str) {
        self.saved_layouts.refused.retain(|c| c != connector);
    }

    fn save_layout(&mut self) {
        let heads = crate::backend::heads(self);
        if !heads.iter().any(|h| h.enabled) {
            return;
        }
        let ids = ids_of(&heads);
        let primary = self.console_primary();
        let shown: Vec<_> = (self.scene.outputs().iter())
            .filter(|o| !crate::virtualout::is_remote(o))
            .filter_map(reprisescene::output_rect)
            .collect();
        let right = (shown.iter())
            .max_by_key(|r| r.loc.x + r.size.w)
            .map_or((0, 0), |r| (r.loc.x + r.size.w, r.loc.y));
        let monitors: Vec<(String, MonitorLayout)> = (heads.iter().zip(&ids))
            .map(|(h, id)| {
                let o = &h.output;
                let name = o.name();
                // A panel the closed lid holds out comes back when it opens: saved on, where it
                // returns to.
                let lid_held =
                    !h.enabled && self.power.lid_closed && crate::power::is_internal_output(&name);
                let on = h.enabled || lid_held;
                let mode = (h.current.and_then(|i| h.modes.get(i)))
                    .filter(|_| on)
                    .map(|m| repriseconf::config::OutputMode {
                        width: m.width,
                        height: m.height,
                        refresh_mhz: Some(m.refresh_mhz),
                    });
                let at = o.current_location();
                let position = if h.enabled {
                    (at.x, at.y)
                } else if lid_held {
                    (self.output_places.back(&name, None, &shown)).unwrap_or(right)
                } else {
                    (0, 0)
                };
                let m = MonitorLayout {
                    enabled: on,
                    mode,
                    position,
                    scale: o.current_scale().fractional_scale() as f32,
                    transform: crate::output::config_transform(o.current_transform()),
                    primary: h.enabled && primary.as_ref() == Some(o),
                };
                (id.clone(), m)
            })
            .collect();
        let layout = Layout::new(monitors);
        let connectors: Vec<String> = heads.iter().map(|h| h.output.name()).collect();
        let expected = (self.saved_layouts.seen.take())
            .unwrap_or_else(|| displays::outputs_fingerprint(&self.config.outputs));
        let now = displays::outputs_fingerprint(&self.config.outputs);
        self.saved_layouts.seen = Some(now.clone());
        self.saved_layouts.active = Some(by_connector(&connectors, &ids, &layout));
        self.saved_layouts.set = {
            let mut s = ids;
            s.sort();
            s
        };
        log::info!("monitors {connectors:?}: layout saved");
        let path = self.saved_layouts.path.clone();
        crate::state::run_in_order(move || {
            let saved = repriseconf::edit_or_reset(&path, |old| {
                let mut l = Layouts::parse(old);
                l.put(layout, &expected, &now);
                Ok::<_, std::convert::Infallible>(l.render())
            });
            if let Err(e) = saved {
                log::warn!("{}: {e}", path.display());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{OutputMode, Transform as T};
    use smithay::utils::Transform;

    fn mode(width: i32, height: i32, refresh_mhz: i32, preferred: bool) -> ModeInfo {
        ModeInfo {
            width,
            height,
            refresh_mhz,
            preferred,
        }
    }

    fn modes() -> Vec<ModeInfo> {
        vec![
            mode(2560, 1440, 59_951, true),
            mode(2560, 1440, 143_912, false),
            mode(1920, 1080, 60_000, false),
        ]
    }

    fn saved(m: &str, x: i32) -> MonitorLayout {
        MonitorLayout {
            enabled: true,
            mode: Some(OutputMode::parse(m).unwrap()),
            position: (x, 0),
            scale: 1.25,
            transform: T::Rotate90,
            primary: false,
        }
    }

    #[test]
    fn a_saved_monitor_asks_its_mode_place_scale_and_turn() {
        let r = head_request(
            &saved("2560x1440@143.912", 1920),
            &modes(),
            Some(0),
            true,
            true,
        );
        assert_eq!(
            r,
            HeadRequest {
                enabled: true,
                mode: Some(1),
                position: Some((1920, 0)),
                transform: Transform::_90,
                scale: 1.25,
                adaptive_sync: true,
            }
        );
    }

    #[test]
    fn a_driven_monitor_already_in_the_saved_mode_asks_no_modeset() {
        let r = head_request(
            &saved("2560x1440@59.951", 0),
            &modes(),
            Some(0),
            true,
            false,
        );
        assert_eq!(r.mode, None);
    }

    #[test]
    fn a_parked_monitor_always_asks_a_mode() {
        let r = head_request(
            &saved("2560x1440@59.951", 0),
            &modes(),
            Some(0),
            false,
            false,
        );
        assert_eq!(r.mode, Some(0));
    }

    #[test]
    fn a_saved_mode_the_connector_lacks_takes_the_preferred_one() {
        let r = head_request(&saved("3840x2160@60", 0), &modes(), Some(2), true, false);
        assert_eq!(r.mode, Some(0));
    }

    #[test]
    fn a_saved_off_monitor_asks_to_leave() {
        let off = MonitorLayout {
            enabled: false,
            mode: None,
            ..saved("1920x1080", 0)
        };
        assert!(!head_request(&off, &modes(), Some(0), true, false).enabled);
    }

    #[test]
    fn the_saved_layout_maps_to_the_connectors_the_monitors_are_on_now() {
        let layout = Layout::new(vec![
            ("Dell|U2720Q|A".into(), saved("1920x1080", 0)),
            ("BOE|0x0BCA|".into(), saved("1920x1080", 1920)),
        ]);
        let got = by_connector(
            &["eDP-1".into(), "DP-7".into()],
            &["BOE|0x0BCA|".into(), "Dell|U2720Q|A".into()],
            &layout,
        );
        let at: Vec<(&str, (i32, i32))> =
            got.iter().map(|(c, m)| (c.as_str(), m.position)).collect();
        assert_eq!(at, [("eDP-1", (1920, 0)), ("DP-7", (0, 0))]);
    }

    /// A harness with head DP-9 running 1280x800 that also lists 1920x1080.
    fn one_head() -> (crate::harness::Harness, Output) {
        let mut h = crate::harness::Harness::new();
        let out = h.head("DP-9", 1280, 800);
        out.add_mode(smithay::output::Mode {
            size: (1920, 1080).into(),
            refresh: 60_000,
        });
        assert!(!h.state.monitors_changed(), "nothing saved yet");
        (h, out)
    }

    /// Reload `[[outputs]]` pinning DP-9 to 1920x1080, which is refused (the harness has no
    /// device to set it on).
    fn refuse(h: &mut crate::harness::Harness) {
        let old = h.state.config.clone();
        h.state.config.outputs.push(crate::config::OutputConfig {
            name: "DP-9".into(),
            scale: 1.0,
            mode: Some(OutputMode::parse("1920x1080@60").unwrap()),
            adaptive_sync: None,
            transform: Default::default(),
            position: None,
            primary: false,
        });
        assert!(!h.state.apply_output_config(&old), "the mode is refused");
        h.dispatch();
        crate::state::flush();
    }

    /// [`one_head`] after [`refuse`].
    fn refused() -> (crate::harness::Harness, Output) {
        let (mut h, out) = one_head();
        refuse(&mut h);
        (h, out)
    }

    /// Whether displays.toml holds a layout for `h`'s monitors under its `[[outputs]]`.
    fn saved_under_outputs(h: &crate::harness::Harness) -> bool {
        let ids = ids_of(&crate::backend::heads(&h.state));
        let now = displays::outputs_fingerprint(&h.state.config.outputs);
        let text = std::fs::read_to_string(&h.state.saved_layouts.path).unwrap_or_default();
        Layouts::parse(&text).find(&ids, &now).is_some()
    }

    #[test]
    fn a_reload_whose_mode_is_refused_saves_no_layout() {
        let (h, out) = refused();
        assert_eq!(h.state.saved_layout_of(&out), None);
        assert!(
            !saved_under_outputs(&h),
            "no layout under the new [[outputs]]"
        );
    }

    #[test]
    fn a_replug_after_a_refused_mode_leaves_the_monitors_to_outputs() {
        let (mut h, out) = refused();
        let ids = ids_of(&crate::backend::heads(&h.state));
        let before = displays::outputs_fingerprint(&[]);
        let mut layouts = Layouts::parse("");
        let m = saved("1280x800@60", 500);
        layouts.put(Layout::new(vec![(ids[0].clone(), m)]), &before, &before);
        std::fs::write(&h.state.saved_layouts.path, layouts.render()).unwrap();
        h.state.saved_layouts.set.clear();
        assert!(
            !h.state.monitors_changed(),
            "the layout from before [[outputs]] changed is not taken"
        );
        assert_eq!(h.state.saved_layout_of(&out), None);
    }

    #[test]
    fn a_later_reload_saves_nothing_while_the_pinned_mode_stays_refused() {
        let (mut h, out) = refused();
        let old = h.state.config.clone();
        h.state.config.outputs[0].position = Some((10, 0));
        h.state.apply_output_config(&old);
        h.dispatch();
        crate::state::flush();
        assert_eq!(h.state.saved_layout_of(&out), None);
        assert!(
            !saved_under_outputs(&h),
            "no layout under the newest [[outputs]]"
        );
    }

    #[test]
    fn a_reload_that_drops_a_refused_mode_saves_the_layout() {
        let (mut h, out) = refused();
        let old = h.state.config.clone();
        h.state.config.outputs[0].mode = None;
        h.state.apply_output_config(&old);
        h.dispatch();
        crate::state::flush();
        assert!(h.state.saved_layout_of(&out).is_some());
        assert!(
            saved_under_outputs(&h),
            "the layout under the newest [[outputs]]"
        );
    }

    #[test]
    fn a_mode_output_management_sets_is_not_replaced_by_the_refused_pin() {
        let (mut h, out) = refused();
        let req = |enabled| HeadRequest {
            enabled,
            mode: Some(0),
            position: None,
            transform: Transform::Normal,
            scale: 1.0,
            adaptive_sync: false,
        };
        assert!(h.state.apply_head(&out, &req(false)), "turned off");
        assert!(
            h.state.apply_head(&out, &req(true)),
            "turned on in 1280x800"
        );
        assert!(!h.state.mode_refused("DP-9"));
    }

    #[test]
    fn a_refused_mode_keeps_the_layouts_of_other_monitor_sets() {
        let (mut h, _) = one_head();
        let start = displays::outputs_fingerprint(&[]);
        let dock = displays::monitor_ids(&["DP-9", "DP-5"].map(|connector| Monitor {
            connector,
            make: "Reprise",
            model: "Test",
            serial: "0",
        }));
        let mut layouts = Layouts::parse("");
        let docked = (dock.iter().zip([0, 1280]))
            .map(|(id, x)| (id.clone(), saved("1280x800@60", x)))
            .collect();
        layouts.put(Layout::new(docked), &start, &start);
        std::fs::write(&h.state.saved_layouts.path, layouts.render()).unwrap();
        refuse(&mut h);
        let old = h.state.config.clone();
        h.state.config.outputs[0].mode = None;
        h.state.apply_output_config(&old);
        h.dispatch();
        crate::state::flush();
        assert!(saved_under_outputs(&h), "DP-9 alone saved");
        let dp5 = h.head("DP-5", 1280, 800);
        assert!(h.state.monitors_changed(), "the docked layout is restored");
        assert_eq!(
            h.state.saved_layout_of(&dp5).map(|m| m.position),
            Some((1280, 0))
        );
    }
}
