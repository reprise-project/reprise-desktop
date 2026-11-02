//! Holding Super alone for `[shortcut_overlay] delay_ms` shows the binds in force as a `keysheet`
//! on the output under the pointer, in `layers.overlay`, never under the lock (docs/features.md).

use std::time::Duration;

use reprisekit::controls::keysheet::{self, Binding, Group, KeySheet};
use reprisekit::look::panel;
use reprisescene::{Input, TreeId};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::{SceneOwner, State};
use crate::config::{Action, GlobalShortcut, Keybind};
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::keys::{MOD_ALT, MOD_CTRL, MOD_LOGO, MOD_SHIFT};
use crate::output::upload_scaled;
use crate::raster::Rect;

/// Where a bare Super press stands: waiting out the delay, or the sheet up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hold {
    #[default]
    Idle,
    Waiting,
    Shown,
}

impl Hold {
    /// A bare Super press: wait for the delay when the sheet is wanted. True when it waits.
    pub fn pressed(&mut self, wanted: bool) -> bool {
        *self = if wanted { Hold::Waiting } else { Hold::Idle };
        wanted
    }

    /// The delay ran out with Super still held alone. True when the sheet goes up.
    pub fn elapsed(&mut self) -> bool {
        let show = *self == Hold::Waiting;
        if show {
            *self = Hold::Shown;
        }
        show
    }

    /// Another key or a button went down. True when the sheet was up.
    pub fn cancelled(&mut self) -> bool {
        std::mem::take(self) == Hold::Shown
    }

    /// Super came up alone. True when it was a tap, which opens Start; false
    /// when it ends a hold that showed the sheet.
    pub fn released(&mut self) -> bool {
        std::mem::take(self) != Hold::Shown
    }
}

#[derive(Default)]
pub struct KeySheetView {
    pub hold: Hold,
    timer: Option<RegistrationToken>,
    panel: Option<Panel>,
}

struct Panel {
    tree: TreeId,
    glass_tree: TreeId,
    backdrop: Backdrop,
}

const WINDOWS: &str = "Windows";
const DESKTOPS: &str = "Desktops";
const LAUNCH: &str = "Launch and search";
const SCREENSHOTS: &str = "Screenshots";
const MEDIA: &str = "Media";
const ACCESSIBILITY: &str = "Accessibility";
const SYSTEM: &str = "System";
const PROGRAMS: &str = "Programs";
const GROUPS: [&str; 8] = [
    WINDOWS,
    DESKTOPS,
    LAUNCH,
    SCREENSHOTS,
    MEDIA,
    ACCESSIBILITY,
    SYSTEM,
    PROGRAMS,
];

/// The group `a` is listed under and what its row says.
fn describe(a: &Action) -> (&'static str, String) {
    use crate::desktops::Step;
    use crate::magnifier::ZoomStep;
    use crate::snap::Dir;
    let (group, what) = match a {
        Action::Spawn(cmd) => return (PROGRAMS, format!("Run {cmd}")),
        Action::Exec { app, .. } => return (PROGRAMS, format!("Start {app}")),
        Action::Close => (WINDOWS, "Close the window"),
        Action::Cycle => (WINDOWS, "Switch windows"),
        Action::Flip => (WINDOWS, "Switch windows in 3D"),
        Action::Maximize => (WINDOWS, "Maximize"),
        Action::Minimize => (WINDOWS, "Minimize"),
        Action::WindowMenu => (WINDOWS, "Window menu"),
        Action::ShowDesktop => (WINDOWS, "Show the desktop"),
        Action::ToggleFrame => (WINDOWS, "Hide or show the frame"),
        Action::AlwaysOnTop => (WINDOWS, "Always on top"),
        Action::Center => (WINDOWS, "Centre the window"),
        Action::Tab(false) => (WINDOWS, "Next tab"),
        Action::Tab(true) => (WINDOWS, "Previous tab"),
        Action::Snap(Dir::Left) => (WINDOWS, "Snap left"),
        Action::Snap(Dir::Right) => (WINDOWS, "Snap right"),
        Action::Snap(Dir::Up) => (WINDOWS, "Snap up"),
        Action::Snap(Dir::Down) => (WINDOWS, "Snap down"),
        Action::SnapLayouts => (WINDOWS, "Snap layouts"),
        Action::Overview => (WINDOWS, "Window overview"),
        Action::Desktop(Step::Prev) => (DESKTOPS, "Previous desktop"),
        Action::Desktop(Step::Next) => (DESKTOPS, "Next desktop"),
        Action::CarryToDesktop(Step::Prev) => (DESKTOPS, "Take the window to the previous one"),
        Action::CarryToDesktop(Step::Next) => (DESKTOPS, "Take the window to the next one"),
        Action::AllDesktops => (DESKTOPS, "Show on all desktops"),
        Action::Launcher => (LAUNCH, "Launcher"),
        Action::StartMenu => (LAUNCH, "Start menu"),
        Action::Run => (LAUNCH, "Run"),
        Action::Terminal => (LAUNCH, "Terminal"),
        Action::ClipboardHistory => (LAUNCH, "Clipboard history"),
        Action::EmojiPicker => (LAUNCH, "Emoji picker"),
        Action::ColorPicker => (LAUNCH, "Colour picker"),
        Action::Screenshot => (SCREENSHOTS, "Whole screen"),
        Action::ScreenshotWindow => (SCREENSHOTS, "Window"),
        Action::ScreenshotRegion => (SCREENSHOTS, "Area"),
        Action::ScreenshotContent => (SCREENSHOTS, "Window without its frame"),
        Action::VolumeUp => (MEDIA, "Volume up"),
        Action::VolumeDown => (MEDIA, "Volume down"),
        Action::Mute => (MEDIA, "Mute"),
        Action::MicMute => (MEDIA, "Mute the microphone"),
        Action::BrightnessUp => (MEDIA, "Brighter"),
        Action::BrightnessDown => (MEDIA, "Dimmer"),
        Action::KbdBrightnessUp => (MEDIA, "Keyboard backlight up"),
        Action::KbdBrightnessDown => (MEDIA, "Keyboard backlight down"),
        Action::PlayPause => (MEDIA, "Play or pause"),
        Action::Next => (MEDIA, "Next track"),
        Action::Prev => (MEDIA, "Previous track"),
        Action::Stop => (MEDIA, "Stop"),
        Action::Wifi => (MEDIA, "Wireless on or off"),
        Action::Zoom(ZoomStep::In) => (ACCESSIBILITY, "Zoom in"),
        Action::Zoom(ZoomStep::Out) => (ACCESSIBILITY, "Zoom out"),
        Action::Zoom(ZoomStep::Reset) => (ACCESSIBILITY, "Zoom back"),
        Action::ColorFilter => (ACCESSIBILITY, "Colour filter"),
        Action::Lock => (SYSTEM, "Lock"),
        Action::LogOff => (SYSTEM, "Log off"),
        Action::TurnOff => (SYSTEM, "Turn off"),
        Action::Exit => (SYSTEM, "Quit Reprise"),
        Action::DoNotDisturb => (SYSTEM, "Do not disturb"),
        Action::NightLight => (SYSTEM, "Night light"),
        Action::FocusTaskbar => (SYSTEM, "Go to the taskbar"),
        Action::FocusTray => (SYSTEM, "Go to the notification area"),
        Action::NextLayout => (SYSTEM, "Next keyboard layout"),
    };
    (group, what.to_string())
}

/// Keysym `sym` as a key cap names it: a letter in capitals, a typed sign as
/// itself, a named key in words ("Page Down", "Print").
pub fn key_text(sym: u32) -> String {
    if let Some(c) = char::from_u32(sym).filter(|_| (0x21..=0x7e).contains(&sym)) {
        return c.to_ascii_uppercase().to_string();
    }
    let name = repriseconf::keybind::key_name(sym).unwrap_or_else(|| format!("{sym:#x}"));
    let fixed = match name.as_str() {
        "Return" | "return" => "Enter",
        "space" => "Space",
        "Escape" | "escape" => "Esc",
        "Sys_Req" | "sys_req" => "SysRq",
        "Prior" => "Page Up",
        "Next" => "Page Down",
        "Super_L" | "super_l" => "Left Super",
        "Super_R" | "super_r" => "Right Super",
        "Alt_L" | "alt_l" => "Left Alt",
        "Alt_R" | "alt_r" => "Right Alt",
        "Shift_L" | "shift_l" => "Left Shift",
        "Shift_R" | "shift_r" => "Right Shift",
        "Control_L" | "control_l" => "Left Ctrl",
        "Control_R" | "control_r" => "Right Ctrl",
        _ => "",
    };
    if !fixed.is_empty() {
        return fixed.to_string();
    }
    let name = name.strip_prefix("XF86").unwrap_or(&name);
    // Underscores and case steps split words: "Page_Down", "AudioRaiseVolume".
    let mut out = String::new();
    let mut prev: Option<char> = None;
    for c in name.chars() {
        if c == '_' {
            out.push(' ');
        } else {
            if c.is_uppercase() && prev.is_some_and(char::is_lowercase) {
                out.push(' ');
            }
            out.push(c);
        }
        prev = Some(c);
    }
    let mut chars = out.chars();
    chars
        .next()
        .map(|f| f.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

/// A chord as the sheet lists it: "Super+Ctrl+Alt+Shift+" then the key.
pub fn chord_text(mods: u32, sym: u32) -> String {
    let mut out = String::new();
    for (bit, name) in [
        (MOD_LOGO, "Super"),
        (MOD_CTRL, "Ctrl"),
        (MOD_ALT, "Alt"),
        (MOD_SHIFT, "Shift"),
    ] {
        if mods & bit != 0 {
            out.push_str(name);
            out.push('+');
        }
    }
    out.push_str(&key_text(sym));
    out
}

/// The sheet for `binds` and the programs' `globals` (each with its program's name): every bound
/// action once, its chords joined, under its group, groups in [`GROUPS`] order; the Super tap
/// heads Launch and search.
pub fn sheet(binds: &[Keybind], globals: &[(&GlobalShortcut, String)]) -> KeySheet {
    let mut rows: Vec<(&'static str, String, Vec<String>)> =
        vec![(LAUNCH, "Start menu".to_string(), vec!["Super".to_string()])];
    let mut add = |group: &'static str, what: String, keys: String| match rows
        .iter_mut()
        .find(|r| r.0 == group && r.1 == what)
    {
        Some(r) if !r.2.contains(&keys) => r.2.push(keys),
        Some(_) => {}
        None => rows.push((group, what, vec![keys])),
    };
    for b in binds {
        let (group, what) = describe(&b.action);
        add(group, what, chord_text(b.mods, b.keysym));
    }
    for (g, name) in globals {
        let Some((mods, sym)) = g.chord else {
            continue;
        };
        let what = if g.description.is_empty() {
            &g.id
        } else {
            &g.description
        };
        add(PROGRAMS, format!("{name}: {what}"), chord_text(mods, sym));
    }
    let groups = GROUPS
        .iter()
        .filter_map(|&title| {
            let rows: Vec<Binding> = (rows.iter())
                .filter(|r| r.0 == title)
                .map(|r| Binding {
                    keys: r.2.join(", "),
                    what: r.1.clone(),
                })
                .collect();
            (!rows.is_empty()).then(|| Group {
                title: title.to_string(),
                rows,
            })
        })
        .collect();
    KeySheet { groups }
}

/// Where a sheet of `size` goes in `usable`: centred, top-left kept inside.
fn sheet_origin(usable: Rect, size: (i32, i32)) -> (i32, i32) {
    let x = usable.x + (usable.width - size.0) / 2;
    let y = usable.y + (usable.height - size.1) / 2;
    (x.max(usable.x), y.max(usable.y))
}

impl State {
    /// A bare Super press: start waiting out the delay, if the sheet is on and the screen unlocked.
    pub fn super_hold_pressed(&mut self) {
        self.stop_key_sheet_timer();
        let cfg = &self.config.shortcut_overlay;
        let wanted = cfg.enabled && !self.is_locked();
        let delay = Duration::from_millis(cfg.delay_ms as u64);
        if !self.key_sheet.hold.pressed(wanted) {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(delay), |_, _, st| {
                st.key_sheet.timer = None;
                if st.key_input.super_tap_armed && !st.is_locked() && st.key_sheet.hold.elapsed() {
                    st.show_key_sheet();
                }
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.key_sheet.timer = Some(t),
            Err(e) => log::warn!("key sheet: no timer: {}", e.error),
        }
    }

    /// Super is no longer held alone: another key or a button went down.
    pub fn super_hold_cancelled(&mut self) {
        self.stop_key_sheet_timer();
        if self.key_sheet.hold.cancelled() {
            self.hide_key_sheet();
        }
    }

    /// Super came up alone; true when it was a tap, which toggles Start.
    pub fn super_hold_released(&mut self) -> bool {
        self.stop_key_sheet_timer();
        let tap = self.key_sheet.hold.released();
        if !tap {
            self.hide_key_sheet();
        }
        tap
    }

    fn stop_key_sheet_timer(&mut self) {
        if let Some(t) = self.key_sheet.timer.take() {
            self.handle.remove(t);
        }
    }

    #[cfg(test)]
    pub fn key_sheet_shown(&self) -> bool {
        self.key_sheet.panel.is_some()
    }

    fn show_key_sheet(&mut self) {
        self.drop_key_sheet();
        let pos = self.pointer_location();
        let Some(out) = self
            .output_at(pos.x, pos.y)
            .or_else(|| self.scene.outputs().first().cloned())
        else {
            return;
        };
        let usable = self.usable_area(Some(&out));
        let scale = self.draw_scale_at(Some(&out)).max(1);
        let theme = self.config.theme.scaled(scale);
        let globals: Vec<(&GlobalShortcut, String)> = (self.config.global_shortcuts.iter())
            .map(|g| {
                let name = reprisefs::apps::entry_for_app_id(&self.app_index, &g.app, |_| true)
                    .map_or_else(|| g.app.clone(), |e| e.name.clone());
                (g, name)
            })
            .collect();
        let k = sheet(&self.config.keybinds, &globals);
        let text = self.text.for_output(scale);
        let m = panel::margin(&self.config.theme);
        let room = (
            usable.width - m.left - m.right,
            usable.height - m.top - m.bottom,
        );
        let lay = keysheet::layout(&k, &theme, text, room);
        let canvas = keysheet::draw(&theme, text, &k, &lay);
        let size = (lay.width, lay.height);
        let at = sheet_origin(usable, size);
        let pane = match self.config.theme.glass() {
            Some(g) if crate::glass::wanted(&self.config) => Some(Pane {
                shapes: vec![PaneShape {
                    rect: Rect::new(0, 0, size.0, size.1),
                    corner: g.panel.corner,
                    square_bottom: false,
                    square_top: false,
                }],
                holes: Vec::new(),
            }),
            _ => None,
        };
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.overlay);
        scene.set_data(tree, Some(SceneOwner::KeySheet));
        crate::glass::leave_out(scene, tree);
        scene.set_position(tree, at.0, at.1);
        let glass_tree = scene.tree_create(tree);
        let mut backdrop = Backdrop::new(&self.glass);
        backdrop.set(scene, &self.glass, glass_tree, pane);
        let buffer = scene.buffer_create(tree);
        scene.set_position(buffer, -m.left, -m.top);
        scene.set_input(buffer, Input::Passthrough);
        upload_scaled(scene, buffer, &canvas, scale);
        self.key_sheet.panel = Some(Panel {
            tree,
            glass_tree,
            backdrop,
        });
        self.redraw_nodes(Vec::new(), &[tree.into()]);
    }

    /// Take the sheet down and stop waiting for it, as the lock does.
    pub fn hide_key_sheet(&mut self) {
        self.stop_key_sheet_timer();
        self.key_sheet.hold = Hold::Idle;
        self.drop_key_sheet();
    }

    fn drop_key_sheet(&mut self) {
        let Some(mut p) = self.key_sheet.panel.take() else {
            return;
        };
        p.backdrop
            .set(&mut self.scene, &self.glass, p.glass_tree, None);
        self.destroy_redrawn(p.tree);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys;

    const KEY_A: u32 = 30;
    const KEY_LEFTMETA: u32 = 125;

    #[test]
    fn a_hold_waits_shows_and_ends_without_a_tap() {
        let mut h = Hold::default();
        assert!(h.pressed(true));
        assert_eq!(h, Hold::Waiting);
        assert!(h.elapsed(), "the delay ran out");
        assert!(!h.elapsed(), "once");
        assert!(!h.released(), "the release after the sheet is no tap");
        assert_eq!(h, Hold::Idle);
    }

    #[test]
    fn a_release_before_the_delay_is_a_tap_and_another_key_cancels() {
        let mut h = Hold::default();
        h.pressed(true);
        assert!(h.released(), "Start opens as before");
        assert!(!h.elapsed(), "a late timer shows nothing");
        h.pressed(true);
        assert!(!h.cancelled(), "nothing was up");
        assert!(!h.elapsed());
        h.pressed(true);
        h.elapsed();
        assert!(h.cancelled(), "the sheet goes");
        assert!(!h.pressed(false), "turned off: no wait");
        assert!(!h.elapsed());
        assert!(h.released());
    }

    #[test]
    fn keys_read_as_their_caps() {
        assert_eq!(
            chord_text(MOD_LOGO | MOD_SHIFT, keys::KEY_b),
            "Super+Shift+B"
        );
        assert_eq!(chord_text(MOD_ALT, keys::KEY_F4), "Alt+F4");
        assert_eq!(chord_text(MOD_LOGO, keys::KEY_Return), "Super+Enter");
        assert_eq!(chord_text(MOD_ALT, keys::KEY_space), "Alt+Space");
        assert_eq!(chord_text(MOD_LOGO, 0x60), "Super+`", "grave");
        assert_eq!(chord_text(MOD_LOGO, 0x2e), "Super+.", "period");
        assert_eq!(key_text(0xff56), "Page Down");
        assert_eq!(key_text(0xff61), "Print");
        assert_eq!(key_text(0xff15), "SysRq");
        assert_eq!(key_text(0xffe9), "Left Alt");
        assert_eq!(key_text(0x1008ff13), "Audio Raise Volume");
        assert_eq!(
            chord_text(MOD_CTRL | MOD_LOGO | MOD_ALT, keys::KEY_Left),
            "Super+Ctrl+Alt+Left"
        );
    }

    #[test]
    fn the_sheet_groups_the_binds_and_joins_an_actions_chords() {
        let binds = crate::config::Config::default().keybinds;
        let shortcut = GlobalShortcut {
            app: "org.example.Player".into(),
            id: "toggle".into(),
            description: "Play or pause".into(),
            keys: "Super+F9".into(),
            chord: Some((MOD_LOGO, 0xffc6)),
        };
        let unbound = GlobalShortcut {
            chord: None,
            id: "other".into(),
            ..shortcut.clone()
        };
        let k = sheet(
            &binds,
            &[(&shortcut, "Player".into()), (&unbound, "Player".into())],
        );
        let titles: Vec<&str> = k.groups.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                WINDOWS,
                DESKTOPS,
                LAUNCH,
                SCREENSHOTS,
                ACCESSIBILITY,
                SYSTEM,
                PROGRAMS
            ]
        );
        let row = |group: &str, what: &str| {
            let g = k.groups.iter().find(|g| g.title == group).unwrap();
            g.rows
                .iter()
                .find(|r| r.what == what)
                .map(|r| r.keys.clone())
        };
        assert_eq!(
            row(WINDOWS, "Close the window").as_deref(),
            Some("Super+Q, Alt+F4")
        );
        assert_eq!(
            row(LAUNCH, "Start menu").as_deref(),
            Some("Super, Ctrl+Esc")
        );
        assert_eq!(row(LAUNCH, "Launcher").as_deref(), Some("Super+Space"));
        assert_eq!(
            row(SYSTEM, "Next keyboard layout").as_deref(),
            Some("Shift+Left Alt")
        );
        assert_eq!(
            row(PROGRAMS, "Player: Play or pause").as_deref(),
            Some("Super+F9")
        );
        let programs = k.groups.iter().find(|g| g.title == PROGRAMS).unwrap();
        assert_eq!(programs.rows.len(), 1, "an unassigned shortcut is left out");
    }

    fn harness(enabled: bool) -> crate::harness::Harness {
        let mut h = crate::harness::Harness::with_config(crate::config::Config {
            base: repriseconf::config::Config {
                shortcut_overlay: repriseconf::config::ShortcutOverlay {
                    enabled,
                    delay_ms: 30,
                },
                ..Default::default()
            },
            ..Default::default()
        });
        h.output("TEST-1", 1024, 768);
        h
    }

    #[test]
    fn holding_super_shows_the_sheet_and_the_release_opens_no_start_menu() {
        let mut h = harness(true);
        h.key(KEY_LEFTMETA, true);
        assert!(!h.state.key_sheet_shown(), "not before the delay");
        h.run_for(80);
        assert!(h.state.key_sheet_shown());
        h.key(KEY_LEFTMETA, false);
        assert!(!h.state.key_sheet_shown(), "the release takes it down");
        assert!(h.state.menus.is_empty(), "and opens no Start menu");
    }

    #[test]
    fn another_key_takes_the_sheet_down_and_its_bind_runs() {
        let mut h = harness(true);
        h.key(KEY_LEFTMETA, true);
        h.run_for(80);
        assert!(h.state.key_sheet_shown());
        h.key(KEY_A, true);
        assert!(!h.state.key_sheet_shown());
        h.key(KEY_A, false);
        h.key(KEY_LEFTMETA, false);
        assert!(h.state.menus.is_empty(), "Super+A is no tap");
    }

    #[test]
    fn a_super_tap_still_opens_start_and_shows_no_sheet() {
        let mut h = harness(true);
        h.key(KEY_LEFTMETA, true);
        h.key(KEY_LEFTMETA, false);
        assert_eq!(h.state.menus.len(), 1, "the Start menu");
        h.run_for(80);
        assert!(!h.state.key_sheet_shown(), "the timer is gone");
    }

    #[test]
    fn turned_off_or_locked_holding_super_shows_nothing() {
        let mut h = harness(false);
        h.key(KEY_LEFTMETA, true);
        h.run_for(80);
        assert!(!h.state.key_sheet_shown(), "turned off");
        h.key(KEY_LEFTMETA, false);

        let mut h = harness(true);
        crate::lockview::lock(&mut h.state);
        h.key(KEY_LEFTMETA, true);
        h.run_for(80);
        assert!(!h.state.key_sheet_shown(), "locked");
    }
}
