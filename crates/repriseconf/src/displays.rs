//! Saved monitor arrangements in `$XDG_STATE_HOME/reprise/displays.toml`, one per set of
//! connected monitors; docs/features.md ("Displays") says when one is saved and when it wins.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::{OutputConfig, OutputMode, Transform};

pub fn path() -> PathBuf {
    crate::xdg_home("XDG_STATE_HOME", ".local/state")
        .join("reprise")
        .join("displays.toml")
}

/// Whether `connector` drives a laptop's or tablet's own panel.
pub fn is_built_in(connector: &str) -> bool {
    ["eDP-", "LVDS-", "DSI-"]
        .iter()
        .any(|p| connector.starts_with(p))
}

/// One connected monitor as its EDID and connector name it.
#[derive(Clone, Copy, Debug)]
pub struct Monitor<'a> {
    pub connector: &'a str,
    pub make: &'a str,
    pub model: &'a str,
    pub serial: &'a str,
}

/// Smithay fills a field the EDID lacks with "Unknown".
fn known(s: &str) -> bool {
    let s = s.trim();
    !s.is_empty() && s != "Unknown"
}

/// Each monitor's id, in `monitors`' order: make, model and serial from the EDID, else the
/// connector name; monitors that would share an id each get their connector appended.
pub fn monitor_ids(monitors: &[Monitor]) -> Vec<String> {
    let base: Vec<String> = monitors
        .iter()
        .map(|m| {
            if [m.make, m.model, m.serial].iter().any(|f| known(f)) {
                let f = |s: &str| {
                    if known(s) {
                        s.trim().to_string()
                    } else {
                        String::new()
                    }
                };
                format!("{}|{}|{}", f(m.make), f(m.model), f(m.serial))
            } else {
                m.connector.to_string()
            }
        })
        .collect();
    base.iter()
        .zip(monitors)
        .map(|(id, m)| {
            if base.iter().filter(|b| *b == id).count() > 1 {
                format!("{id}|{}", m.connector)
            } else {
                id.clone()
            }
        })
        .collect()
}

/// What one monitor of a saved arrangement runs.
#[derive(Clone, Debug, PartialEq)]
pub struct MonitorLayout {
    pub enabled: bool,
    /// None on a disabled monitor.
    pub mode: Option<OutputMode>,
    /// Top-left in the layout, logical pixels.
    pub position: (i32, i32),
    pub scale: f32,
    pub transform: Transform,
    pub primary: bool,
}

/// One set's arrangement: monitor ids with what each runs, sorted by id.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub monitors: Vec<(String, MonitorLayout)>,
}

impl Layout {
    pub fn new(mut monitors: Vec<(String, MonitorLayout)>) -> Layout {
        monitors.sort_by(|a, b| a.0.cmp(&b.0));
        Layout { monitors }
    }

    fn ids(&self) -> Vec<&str> {
        self.monitors.iter().map(|(id, _)| id.as_str()).collect()
    }
}

/// The file: the saved arrangements, and the `[[outputs]]` fingerprint they were saved under.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layouts {
    pub outputs_seen: String,
    pub list: Vec<Layout>,
}

#[derive(Serialize, Deserialize, Default)]
struct FileLayouts {
    #[serde(default)]
    outputs_seen: String,
    #[serde(default)]
    layout: Vec<FileLayout>,
}

#[derive(Serialize, Deserialize)]
struct FileLayout {
    monitor: Vec<FileMonitor>,
}

#[derive(Serialize, Deserialize)]
struct FileMonitor {
    id: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    position: [i32; 2],
    scale: f32,
    transform: String,
    #[serde(default)]
    primary: bool,
}

impl FileMonitor {
    fn parse(self) -> Option<(String, MonitorLayout)> {
        let mode = match self.mode {
            Some(m) => Some(OutputMode::parse(&m).ok()?),
            None => None,
        };
        let layout = MonitorLayout {
            enabled: self.enabled,
            mode,
            position: (self.position[0], self.position[1]),
            scale: crate::config::quarter_scale(self.scale),
            transform: Transform::parse(&self.transform)?,
            primary: self.primary,
        };
        Some((self.id, layout))
    }
}

impl Layouts {
    /// The file's text read; a layout with a monitor that does not parse is left out, and text
    /// that is not TOML reads as empty.
    pub fn parse(text: &str) -> Layouts {
        let file: FileLayouts = toml::from_str(text).unwrap_or_else(|e| {
            log::warn!("displays.toml: {e}; no saved layouts");
            FileLayouts::default()
        });
        let list = file
            .layout
            .into_iter()
            .filter_map(|l| {
                let monitors: Option<Vec<_>> =
                    l.monitor.into_iter().map(FileMonitor::parse).collect();
                monitors.map(Layout::new)
            })
            .collect();
        Layouts {
            outputs_seen: file.outputs_seen,
            list,
        }
    }

    pub fn render(&self) -> String {
        let file = FileLayouts {
            outputs_seen: self.outputs_seen.clone(),
            layout: (self.list.iter())
                .map(|l| FileLayout {
                    monitor: (l.monitors.iter())
                        .map(|(id, m)| FileMonitor {
                            id: id.clone(),
                            enabled: m.enabled,
                            mode: m.mode.map(|m| m.to_string()),
                            position: [m.position.0, m.position.1],
                            scale: m.scale,
                            transform: m.transform.name().to_string(),
                            primary: m.primary,
                        })
                        .collect(),
                })
                .collect(),
        };
        toml::to_string(&file).unwrap_or_default()
    }

    /// The arrangement saved for exactly the monitors `ids` (any order), when the file was saved
    /// under `outputs_seen` and the arrangement turns at least one monitor on.
    pub fn find(&self, ids: &[String], outputs_seen: &str) -> Option<&Layout> {
        if self.outputs_seen != outputs_seen {
            return None;
        }
        let mut want: Vec<&str> = ids.iter().map(String::as_str).collect();
        want.sort_unstable();
        self.list
            .iter()
            .find(|l| l.ids() == want)
            .filter(|l| l.monitors.iter().any(|(_, m)| m.enabled))
    }

    /// `layout` in place of the one for its set, the file now under `outputs_seen`. When the file
    /// was saved under another fingerprint than `expected`, its other layouts are dropped.
    pub fn put(&mut self, layout: Layout, expected: &str, outputs_seen: &str) {
        if self.outputs_seen != expected {
            self.list.clear();
        }
        self.list.retain(|l| l.ids() != layout.ids());
        self.list.push(layout);
        self.outputs_seen = outputs_seen.to_string();
    }
}

/// A fingerprint of the `[[outputs]]` entries, to tell whether they changed since a save.
pub fn outputs_fingerprint(outputs: &[OutputConfig]) -> String {
    let mut text = String::new();
    for o in outputs {
        let mode = o.mode.map(|m| m.to_string()).unwrap_or_default();
        text += &format!(
            "{}|{}|{mode}|{:?}|{}|{:?}|{}\n",
            o.name,
            o.scale,
            o.adaptive_sync,
            o.transform.name(),
            o.position,
            o.primary
        );
    }
    // FNV-1a: stable across Rust releases, unlike `DefaultHasher`.
    let hash = text.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    });
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon<'a>(connector: &'a str, make: &'a str, model: &'a str, serial: &'a str) -> Monitor<'a> {
        Monitor {
            connector,
            make,
            model,
            serial,
        }
    }

    fn on(x: i32, primary: bool) -> MonitorLayout {
        MonitorLayout {
            enabled: true,
            mode: Some(OutputMode::parse("1920x1080@60").unwrap()),
            position: (x, 0),
            scale: 1.0,
            transform: Transform::Normal,
            primary,
        }
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_monitor_id_is_its_edid_and_the_connector_only_without_one() {
        let got = monitor_ids(&[
            mon("eDP-1", "BOE", "0x0BCA", "Unknown"),
            mon("DP-3", "Dell Inc.", "DELL U2720Q", "ABC123"),
            mon("HDMI-A-1", "Unknown", "Unknown", "Unknown"),
            mon("DP-4", "", "", ""),
        ]);
        assert_eq!(
            got,
            [
                "BOE|0x0BCA|",
                "Dell Inc.|DELL U2720Q|ABC123",
                "HDMI-A-1",
                "DP-4"
            ]
        );
    }

    #[test]
    fn the_same_monitor_on_another_connector_keeps_its_id() {
        let a = monitor_ids(&[mon("DP-3", "Dell Inc.", "U2720Q", "ABC")]);
        let b = monitor_ids(&[mon("DP-7", "Dell Inc.", "U2720Q", "ABC")]);
        assert_eq!(a, b);
    }

    #[test]
    fn two_monitors_of_a_model_without_serials_get_their_connectors() {
        let got = monitor_ids(&[
            mon("DP-1", "Acme", "X1", "Unknown"),
            mon("DP-2", "Acme", "X1", "Unknown"),
            mon("DP-3", "Acme", "X2", "Unknown"),
        ]);
        assert_eq!(got, ["Acme|X1||DP-1", "Acme|X1||DP-2", "Acme|X2|"]);
    }

    #[test]
    fn a_layout_is_found_for_exactly_its_set_in_any_order() {
        let mut l = Layouts::default();
        let both = Layout::new(vec![
            ("b".into(), on(0, true)),
            ("a".into(), on(1920, false)),
        ]);
        l.put(both.clone(), "", "f");
        l.put(Layout::new(vec![("a".into(), on(0, true))]), "f", "f");
        assert_eq!(l.find(&ids(&["a", "b"]), "f"), Some(&both));
        assert_eq!(l.find(&ids(&["b", "a"]), "f"), Some(&both));
        assert_eq!(
            l.find(&ids(&["a"]), "f").unwrap().monitors[0].1.position,
            (0, 0)
        );
        assert_eq!(l.find(&ids(&["b"]), "f"), None);
        assert_eq!(l.find(&ids(&["a", "b", "c"]), "f"), None);
    }

    #[test]
    fn layouts_saved_under_other_outputs_entries_are_not_found() {
        let mut l = Layouts::default();
        l.put(Layout::new(vec![("a".into(), on(0, true))]), "", "f1");
        assert!(l.find(&ids(&["a"]), "f1").is_some());
        assert_eq!(l.find(&ids(&["a"]), "f2"), None);
    }

    #[test]
    fn a_layout_with_every_monitor_off_is_not_found() {
        let mut l = Layouts::default();
        let off = MonitorLayout {
            enabled: false,
            mode: None,
            ..on(0, false)
        };
        l.put(Layout::new(vec![("a".into(), off)]), "", "f");
        assert_eq!(l.find(&ids(&["a"]), "f"), None);
    }

    #[test]
    fn saving_replaces_the_sets_layout_and_keeps_the_others() {
        let mut l = Layouts::default();
        l.put(Layout::new(vec![("a".into(), on(0, true))]), "", "f");
        l.put(
            Layout::new(vec![
                ("a".into(), on(0, true)),
                ("b".into(), on(1920, false)),
            ]),
            "f",
            "f",
        );
        l.put(Layout::new(vec![("a".into(), on(500, true))]), "f", "g");
        assert_eq!(l.list.len(), 2);
        assert_eq!(
            l.find(&ids(&["a"]), "g").unwrap().monitors[0].1.position,
            (500, 0)
        );
        assert!(l.find(&ids(&["a", "b"]), "g").is_some());
    }

    #[test]
    fn saving_over_a_file_from_other_outputs_entries_drops_its_layouts() {
        let mut l = Layouts::default();
        l.put(
            Layout::new(vec![
                ("a".into(), on(0, true)),
                ("b".into(), on(1920, false)),
            ]),
            "",
            "old",
        );
        l.put(Layout::new(vec![("a".into(), on(0, true))]), "new", "new");
        assert_eq!(l.list.len(), 1);
        assert_eq!(l.find(&ids(&["a", "b"]), "new"), None);
    }

    #[test]
    fn the_file_round_trips() {
        let mut l = Layouts::default();
        let rotated = MonitorLayout {
            scale: 1.5,
            transform: Transform::Rotate90,
            position: (-1080, 200),
            ..on(0, false)
        };
        let off = MonitorLayout {
            enabled: false,
            mode: None,
            ..on(0, false)
        };
        l.put(
            Layout::new(vec![
                ("a".into(), on(0, true)),
                ("b".into(), rotated),
                ("c".into(), off),
            ]),
            "",
            "f",
        );
        assert_eq!(Layouts::parse(&l.render()), l);
    }

    #[test]
    fn a_layout_with_a_bad_monitor_is_left_out() {
        let text = r#"
outputs_seen = "f"
[[layout]]
[[layout.monitor]]
id = "a"
enabled = true
mode = "1920x1080"
position = [0, 0]
scale = 1.0
transform = "normal"
[[layout]]
[[layout.monitor]]
id = "b"
enabled = true
mode = "wide"
position = [0, 0]
scale = 1.0
transform = "normal"
"#;
        let l = Layouts::parse(text);
        assert_eq!(l.list.len(), 1);
        assert!(l.find(&ids(&["a"]), "f").is_some());
        assert_eq!(Layouts::parse("not toml ["), Layouts::default());
    }

    #[test]
    fn the_outputs_fingerprint_changes_with_any_key() {
        let base = OutputConfig {
            name: "DP-1".into(),
            scale: 1.0,
            mode: None,
            adaptive_sync: None,
            transform: Transform::Normal,
            position: None,
            primary: false,
            icc_profile: None,
        };
        let f = outputs_fingerprint(std::slice::from_ref(&base));
        assert_eq!(f, outputs_fingerprint(std::slice::from_ref(&base.clone())));
        let moved = OutputConfig {
            position: Some((1920, 0)),
            ..base.clone()
        };
        assert_ne!(f, outputs_fingerprint(&[moved]));
        assert_ne!(f, outputs_fingerprint(&[]));
    }
}
