//! Power Options Properties: the pure model.
//!
//! One `PropSheet` with the laptop tabs: Power Schemes (power profile, idle timeouts, plugged in against
//! on battery), Alarms, Power Meter and Advanced (tray icon, lid and buttons). Nothing here reads a file
//! or a bus: the host feeds it [`Settings`], [`Power`] and [`Profiles`] snapshots and applies the
//! [`Settings`] and profile it hands back.

use reprisekit::classic::widgets::LINE_H;
use reprisekit::controls::combo;
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::propsheet::{self, PropSheet};
use reprisekit::raster::{Canvas, Rect};
use reprisekit::text::DialogFont;
use reprisekit::trackbar::{self, TrackBar};

use crate::profiles::{self, Profiles};
use crate::settings::Settings;
use crate::tray::clock;
use crate::upower::{Power, State};

/// The sheet's fixed client size: a 404x448 window less the frame.
pub const CLIENT: (i32, i32) = (398, 424);

pub const TABS: [&str; 4] = ["Power Schemes", "Alarms", "Power Meter", "Advanced"];
pub const TAB_SCHEMES: usize = 0;
pub const TAB_ALARMS: usize = 1;
pub const TAB_METER: usize = 2;
pub const TAB_ADVANCED: usize = 3;

pub const MINUTES: [u32; 16] = [
    1, 2, 3, 5, 10, 15, 20, 25, 30, 45, 60, 120, 180, 240, 300, 0,
];

const ALARM_STEP: i64 = 5;
const ALARM_STEPS: usize = 21;

const X: i32 = 20;
const W: i32 = 358;
const GX: i32 = X + 12;
const GW: i32 = W - 24;
const ICON_PX: i32 = 32;

/// The Power Schemes columns: where "Plugged in" and "On batteries" start.
const COL_AC: i32 = 146;
const COL_BAT: i32 = 262;
const COL_W: i32 = 108;

pub fn minutes_label(minutes: u32) -> String {
    match minutes {
        0 => "Never".into(),
        1 => "After 1 min".into(),
        m if m < 60 => format!("After {m} mins"),
        60 => "After 1 hour".into(),
        m => format!("After {} hours", m / 60),
    }
}

pub fn minutes_index(seconds: i64) -> usize {
    if seconds <= 0 {
        return MINUTES.len() - 1;
    }
    let m = seconds as f64 / 60.0;
    MINUTES[..MINUTES.len() - 1]
        .iter()
        .enumerate()
        .min_by(|a, b| {
            (f64::from(*a.1) - m)
                .abs()
                .total_cmp(&(f64::from(*b.1) - m).abs())
        })
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// A combo row's actions: the config name and its label. `ACTIONS` is the compositor's whole set, accepted by the lid and both buttons.
pub const ACTIONS: [(&str, &str); 7] = [
    ("nothing", "Do nothing"),
    ("ask", "Ask me what to do"),
    ("screen_off", "Turn off the screen"),
    ("lock", "Lock computer"),
    ("standby", "Stand by"),
    ("hibernate", "Hibernate"),
    ("turnoff", "Shut down"),
];

pub const CRITICAL_ACTIONS: [(&str, &str); 4] = [
    ("nothing", "Do nothing"),
    ("standby", "Stand by"),
    ("hibernate", "Hibernate"),
    ("turnoff", "Shut down"),
];

/// What the host does after an event. An apply carries the settings to write and the power profile to switch to, if picked.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    None,
    Redraw,
    Apply(Settings, Option<String>),
    Close,
    ApplyAndClose(Settings, Option<String>),
}

pub struct Sheet {
    pub sheet: PropSheet,
    /// What the file holds: what `pending` is compared against.
    pub applied: Settings,
    pub pending: Settings,
    /// The alarm levels the sliders remember while their check is off (an off alarm is `battery_low == 0` in the file).
    low_level: i64,
    critical_level: i64,
    power: Power,
    profiles: Profiles,
    profile: Option<String>,
}

impl Sheet {
    /// `icon` is the theme's power icon at 32 px; a missing one becomes the magenta placeholder.
    pub fn new(
        settings: &Settings,
        power: &Power,
        profiles: &Profiles,
        icon: Option<Canvas>,
    ) -> Sheet {
        let pages = vec![
            schemes_page(icon).set_in(DialogFont::ShellDlg),
            alarms_page(),
            meter_page(),
            advanced_page(),
        ];
        let mut s = Sheet {
            sheet: PropSheet::new(&TABS, pages, CLIENT),
            applied: settings.clone(),
            pending: settings.clone(),
            low_level: remembered(settings.battery_low, 10),
            critical_level: remembered(settings.battery_critical, 3),
            power: power.clone(),
            profiles: profiles.clone(),
            profile: None,
        };
        s.write();
        s
    }

    /// The file changed under us: take it as the new baseline, keeping what the user has not applied.
    pub fn refresh_settings(&mut self, settings: &Settings) {
        let dirty = self.pending != self.applied;
        self.applied = settings.clone();
        if !dirty {
            self.pending = settings.clone();
            self.low_level = remembered(settings.battery_low, self.low_level);
            self.critical_level = remembered(settings.battery_critical, self.critical_level);
        }
        self.write();
    }

    /// The daemon's profiles changed: the combo follows unless the user picked another and has not applied it.
    pub fn refresh_profiles(&mut self, profiles: &Profiles) {
        self.profiles = profiles.clone();
        if self.profile.as_deref() == Some(profiles.active.as_str())
            || self.profiles.available.is_empty()
        {
            self.profile = None;
        }
        self.write_profiles();
        self.sheet.set_apply_enabled(self.dirty());
    }

    pub fn refresh_power(&mut self, power: &Power) {
        self.power = power.clone();
        self.write_meter();
    }

    pub fn open_tab(&mut self, tab: usize) {
        self.sheet.tabs.open = tab.min(TABS.len() - 1);
    }

    pub fn dirty(&self) -> bool {
        self.pending != self.applied || self.profile.is_some()
    }

    pub fn event(&mut self, ev: propsheet::Event) -> Outcome {
        match ev {
            propsheet::Event::None => Outcome::None,
            propsheet::Event::Redraw => Outcome::Redraw,
            propsheet::Event::Page(_, form::Event::Changed(id)) => {
                self.read(id);
                Outcome::Redraw
            }
            propsheet::Event::Page(..) => Outcome::Redraw,
            propsheet::Event::Apply => {
                if !self.dirty() {
                    return Outcome::Redraw;
                }
                let profile = self.accept();
                Outcome::Apply(self.applied.clone(), profile)
            }
            propsheet::Event::Ok => {
                if !self.dirty() {
                    return Outcome::Close;
                }
                let profile = self.accept();
                Outcome::ApplyAndClose(self.applied.clone(), profile)
            }
            propsheet::Event::Cancel => {
                self.pending = self.applied.clone();
                self.profile = None;
                self.write();
                Outcome::Close
            }
        }
    }

    /// Take `pending` as applied; the picked profile, if any, is taken as on and handed back for the host to switch to.
    fn accept(&mut self) -> Option<String> {
        self.applied = self.pending.clone();
        let profile = self.profile.take();
        if let Some(p) = &profile {
            self.profiles.active = p.clone();
        }
        self.sheet.set_apply_enabled(false);
        profile
    }

    /// The host could not write what `accept` took: the file still holds `applied`, so that is the
    /// baseline again and the user's values stay pending with Apply live.
    pub fn unapplied(&mut self, applied: &Settings) {
        self.applied = applied.clone();
        self.sheet.set_apply_enabled(self.dirty());
    }

    fn read(&mut self, id: &'static str) {
        let page = self.sheet.open();
        let combo = |id| page.combo_selected(id);
        let pos = |id| page.track_pos(id);
        let checked = |id| page.checked(id);
        let seconds = |id| combo(id).map(|i| i64::from(MINUTES[i.min(MINUTES.len() - 1)]) * 60);
        let action = |id, list: &[(&str, &str)]| action_name(page, id, list);
        if id == "profile" {
            if let Some(i) = combo(id)
                && let Some(name) = self.profiles.available.get(i)
            {
                self.profile = (*name != self.profiles.active).then(|| name.clone());
            }
            self.sheet.set_apply_enabled(self.dirty());
            return;
        }
        let p = &mut self.pending;
        match id {
            "ac_monitor" => {
                if let Some(v) = seconds(id) {
                    p.screen_off_after = v;
                }
            }
            "ac_standby" => {
                if let Some(v) = seconds(id) {
                    p.suspend_after = v;
                }
            }
            "bat_monitor" => {
                if let Some(v) = seconds(id) {
                    p.battery_screen_off_after = (v != p.screen_off_after).then_some(v);
                }
            }
            "bat_standby" => {
                if let Some(v) = seconds(id) {
                    p.battery_suspend_after = (v != p.suspend_after).then_some(v);
                }
            }
            "low_check" => {
                if let Some(on) = checked(id) {
                    p.battery_low = if on { self.low_level } else { 0 };
                }
            }
            "low_track" => {
                if let Some(i) = pos(id) {
                    self.low_level = (i as i64 * ALARM_STEP).max(1);
                    if p.battery_low > 0 {
                        p.battery_low = self.low_level;
                    }
                }
            }
            "crit_check" => {
                if let Some(on) = checked(id) {
                    p.battery_critical = if on { self.critical_level } else { 0 };
                }
            }
            "crit_track" => {
                if let Some(i) = pos(id) {
                    self.critical_level = (i as i64 * ALARM_STEP).max(1);
                    if p.battery_critical > 0 {
                        p.battery_critical = self.critical_level;
                    }
                }
            }
            "crit_action" => {
                if let Some(a) = action(id, &CRITICAL_ACTIONS) {
                    p.critical_action = a;
                }
            }
            "show_icon" => {
                if let Some(on) = checked(id) {
                    p.always_show_icon = on;
                }
            }
            "lid" => {
                if let Some(a) = action(id, &ACTIONS) {
                    p.lid_close = a;
                }
            }
            "power_btn" => {
                if let Some(a) = action(id, &ACTIONS) {
                    p.power_button = a;
                }
            }
            "sleep_btn" => {
                if let Some(a) = action(id, &ACTIONS) {
                    p.sleep_button = a;
                }
            }
            _ => {}
        }
        let dirty = self.dirty();
        self.sheet.set_apply_enabled(dirty);
        self.write();
    }

    fn write(&mut self) {
        let p = self.pending.clone();
        let a = self.applied.clone();
        let (low_level, critical_level) = (self.low_level, self.critical_level);
        let dirty = self.dirty();
        {
            let page = &mut self.sheet.pages[TAB_SCHEMES];
            page.set_combo("ac_monitor", minutes_index(p.screen_off_after));
            page.set_combo("ac_standby", minutes_index(p.suspend_after));
            page.set_combo("bat_monitor", minutes_index(p.battery_screen_off()));
            page.set_combo("bat_standby", minutes_index(p.battery_suspend()));
        }
        {
            let page = &mut self.sheet.pages[TAB_ALARMS];
            let low_on = p.battery_low > 0;
            let crit_on = p.battery_critical > 0;
            page.set_check("low_check", low_on);
            set_track(page, "low_track", low_level, low_on);
            set_text(page, "low_pct", &format!("{low_level}%"), low_on);
            page.set_check("crit_check", crit_on);
            set_track(page, "crit_track", critical_level, crit_on);
            set_text(page, "crit_pct", &format!("{critical_level}%"), crit_on);
            set_text(page, "crit_action_label", "&Action:", crit_on);
            set_action(
                page,
                "crit_action",
                &CRITICAL_ACTIONS,
                &p.critical_action,
                &a.critical_action,
            );
            page.set_enabled("crit_action", crit_on);
        }
        {
            let page = &mut self.sheet.pages[TAB_ADVANCED];
            page.set_check("show_icon", p.always_show_icon);
            set_action(page, "lid", &ACTIONS, &p.lid_close, &a.lid_close);
            set_action(
                page,
                "power_btn",
                &ACTIONS,
                &p.power_button,
                &a.power_button,
            );
            set_action(
                page,
                "sleep_btn",
                &ACTIONS,
                &p.sleep_button,
                &a.sleep_button,
            );
        }
        self.write_profiles();
        self.write_meter();
        self.sheet.set_apply_enabled(dirty);
    }

    /// The profile combo: the daemon's profiles with the picked or active one selected; greyed and empty while the daemon is missing.
    fn write_profiles(&mut self) {
        let page = &mut self.sheet.pages[TAB_SCHEMES];
        let labels: Vec<String> = self
            .profiles
            .available
            .iter()
            .map(|p| profiles::label(p))
            .collect();
        if let Some(Control::Combo(c)) = page.item_mut("profile").map(|i| &mut i.control)
            && !c.items.iter().map(|i| &i.text).eq(labels.iter())
        {
            c.items = labels.iter().map(combo::ListItem::new).collect();
        }
        let shown = self.profile.as_ref().unwrap_or(&self.profiles.active);
        let selected = self.profiles.available.iter().position(|p| p == shown);
        page.set_combo("profile", selected.unwrap_or(0));
        page.set_enabled("profile", !labels.is_empty());
    }

    fn write_meter(&mut self) {
        let power = self.power.clone();
        let page = &mut self.sheet.pages[TAB_METER];
        let source = if power.on_battery {
            "Batteries"
        } else {
            "AC power"
        };
        set_text(page, "source_value", source, true);
        let (status, charge, remaining) = match &power.battery {
            None => (
                "No battery is present".to_string(),
                "-".to_string(),
                "-".to_string(),
            ),
            Some(b) => {
                let status = match b.state {
                    State::Charging => "Charging",
                    State::Discharging => "Discharging",
                    State::Full => "Fully charged",
                    State::Unknown => "Unknown",
                };
                let remaining = match b.state {
                    State::Discharging if b.time_to_empty > 0 => clock(b.time_to_empty),
                    State::Charging if b.time_to_full > 0 => {
                        format!("{} to full", clock(b.time_to_full))
                    }
                    State::Full => "-".to_string(),
                    _ => "Unknown".to_string(),
                };
                (status.to_string(), format!("{}%", b.percent), remaining)
            }
        };
        set_text(page, "status_value", &status, true);
        set_text(page, "charge_value", &charge, true);
        set_text(page, "remaining_value", &remaining, true);
        if let Some(item) = page.item_mut("battery_icon") {
            let src = crate::icon_for(&power, &self.pending);
            item.control = Control::Picture {
                canvas: Some(upscale(&reprisekit::art::powericon::draw(src), 2)),
                frame: 0,
            };
        }
    }
}

/// An alarm level worth showing on the slider: the file's, or the last one when the alarm is off.
fn remembered(level: i64, last: i64) -> i64 {
    if level > 0 { level } else { last }
}

/// Fill action combo `id` with `list`'s labels, then one row per name in `pending` or `applied` the
/// list lacks, labelled with the name itself, and select `pending`. A name a newer compositor accepts
/// thus survives an Apply of another control.
fn set_action(page: &mut Form, id: &str, list: &[(&str, &str)], pending: &str, applied: &str) {
    let mut rows: Vec<&str> = list.iter().map(|(_, l)| *l).collect();
    let mut names: Vec<&str> = list.iter().map(|(n, _)| *n).collect();
    for extra in [applied, pending] {
        if !names.contains(&extra) {
            rows.push(extra);
            names.push(extra);
        }
    }
    if let Some(Control::Combo(c)) = page.item_mut(id).map(|i| &mut i.control)
        && !c
            .items
            .iter()
            .map(|i| i.text.as_str())
            .eq(rows.iter().copied())
    {
        c.items = rows.iter().map(|&r| combo::ListItem::new(r)).collect();
    }
    page.set_combo(id, names.iter().position(|n| *n == pending).unwrap_or(0));
}

/// The config name behind action combo `id`'s selected row: `list`'s, or the extra row's own text past its end.
fn action_name(page: &Form, id: &str, list: &[(&str, &str)]) -> Option<String> {
    let Some(Control::Combo(c)) = page.item(id).map(|i| &i.control) else {
        return None;
    };
    match list.get(c.selected) {
        Some((name, _)) => Some(name.to_string()),
        None => c.items.get(c.selected).map(|i| i.text.clone()),
    }
}

fn upscale(c: &Canvas, n: u32) -> Canvas {
    let mut out = Canvas::new(c.width * n, c.height * n);
    for y in 0..out.height as i32 {
        for x in 0..out.width as i32 {
            out.put_raw(x, y, c.get(x / n as i32, y / n as i32));
        }
    }
    out
}

fn set_track(page: &mut Form, id: &str, level: i64, enabled: bool) {
    page.set_track(
        id,
        ((level + ALARM_STEP / 2) / ALARM_STEP).clamp(0, ALARM_STEPS as i64 - 1) as usize,
    );
    page.set_enabled(id, enabled);
}

/// A label's text and whether it is greyed together: the alarm rows grey with their check box.
fn set_text(page: &mut Form, id: &str, value: &str, on: bool) {
    page.set_text(id, value);
    page.set_enabled(id, on);
}

fn lines(id: &'static str, x: i32, y: i32, text: &[&str]) -> Item {
    Item::lines(
        id,
        Rect::new(x, y, W - (x - X), LINE_H * text.len() as i32),
        text,
    )
}

fn group(id: &'static str, y: i32, h: i32, title: &str) -> Item {
    Item::group(id, Rect::new(X, y, W, h), title)
}

fn combo(id: &'static str, x: i32, y: i32, w: i32, items: &[&str]) -> Item {
    Item::combo(id, x, y, w, items, 8)
}

fn minutes_combo(id: &'static str, x: i32, y: i32) -> Item {
    let labels: Vec<String> = MINUTES.iter().map(|&m| minutes_label(m)).collect();
    let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
    combo(id, x, y, COL_W, &refs)
}

fn picture(id: &'static str, x: i32, y: i32, px: i32, canvas: Option<Canvas>) -> Item {
    Item::picture(
        id,
        Rect::new(x, y, px, px),
        Some(canvas.unwrap_or_else(|| reprisekit::classic::widgets::placeholder(px, px))),
    )
}

fn schemes_page(icon: Option<Canvas>) -> Form {
    let mut items = vec![
        picture("intro_icon", X, 48, ICON_PX, icon),
        lines(
            "intro",
            X + 46,
            50,
            &[
                "Select the power-saving settings you want to use.",
                "The second column applies while the computer runs",
                "on its batteries.",
            ],
        ),
        group("schemes_g", 106, 52, "Power schemes"),
        combo("profile", GX, 126, GW, &[]),
        group("settings_g", 170, 130, "Settings"),
        Item::label_at("col_ac", COL_AC, 190, COL_W, "Plugged in"),
        Item::label_at("col_bat", COL_BAT, 190, COL_W, "On batteries"),
        Item::label_at("monitor_l", GX, 218, 110, "Turn off &monitor:"),
        minutes_combo("ac_monitor", COL_AC, 214),
        minutes_combo("bat_monitor", COL_BAT, 214),
        Item::label_at("standby_l", GX, 250, 110, "System &stand by:"),
        minutes_combo("ac_standby", COL_AC, 246),
        minutes_combo("bat_standby", COL_BAT, 246),
    ];
    items.push(Item::label_at(
        "schemes_note",
        X,
        314,
        W,
        "Blank and lock times are on Reprise Settings' Power page.",
    ));
    Form::new(items)
}

fn alarms_page() -> Form {
    let actions: Vec<&str> = CRITICAL_ACTIONS.iter().map(|(_, l)| *l).collect();
    Form::new(vec![
        group("low_g", 48, 96, "Low battery alarm"),
        Item::check(
            "low_check",
            GX,
            68,
            "Activate &low battery alarm when power level reaches:",
        ),
        Item::new(
            "low_track",
            Rect::new(GX, 90, 240, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(2, ALARM_STEPS)),
        ),
        Item::label_at("low_pct", GX + 252, 96, 50, "10%"),
        group("crit_g", 156, 130, "Critical battery alarm"),
        Item::check(
            "crit_check",
            GX,
            176,
            "Activate &critical battery alarm when power level reaches:",
        ),
        Item::new(
            "crit_track",
            Rect::new(GX, 198, 240, trackbar::HEIGHT),
            Control::TrackBar(TrackBar::new(1, ALARM_STEPS)),
        ),
        Item::label_at("crit_pct", GX + 252, 204, 50, "3%"),
        Item::label_at("crit_action_label", GX, 246, 60, "&Action:"),
        combo("crit_action", GX + 60, 242, 160, &actions),
    ])
}

fn meter_page() -> Form {
    Form::new(vec![
        Item::label_at("source_l", X, 50, 150, "Current power source:"),
        Item::label_at("source_value", X + 150, 50, 200, ""),
        group("battery_g", 76, 110, "Battery"),
        picture("battery_icon", GX, 100, ICON_PX, None),
        Item::label_at("status_l", GX + 50, 100, 80, "Status:"),
        Item::label_at("status_value", GX + 130, 100, 190, ""),
        Item::label_at("charge_l", GX + 50, 122, 80, "Charge:"),
        Item::label_at("charge_value", GX + 130, 122, 190, ""),
        Item::label_at("remaining_l", GX + 50, 144, 80, "Remaining:"),
        Item::label_at("remaining_value", GX + 130, 144, 190, ""),
    ])
}

fn advanced_page() -> Form {
    let actions: Vec<&str> = ACTIONS.iter().map(|(_, l)| *l).collect();
    Form::new(vec![
        group("options_g", 48, 52, "Options"),
        Item::check("show_icon", GX, 70, "&Always show icon on the taskbar"),
        group("buttons_g", 112, 196, "Power buttons"),
        Item::label_at(
            "lid_l",
            GX,
            132,
            GW,
            "When I close the lid of my portable computer:",
        ),
        combo("lid", GX, 150, GW, &actions),
        Item::label_at(
            "power_l",
            GX,
            190,
            GW,
            "When I press the power button on my computer:",
        ),
        combo("power_btn", GX, 208, GW, &actions),
        Item::label_at(
            "sleep_l",
            GX,
            248,
            GW,
            "When I press the sleep button on my computer:",
        ),
        combo("sleep_btn", GX, 266, GW, &actions),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upower::Battery;

    fn sheet() -> Sheet {
        Sheet::new(
            &Settings::default(),
            &Power::default(),
            &Profiles::default(),
            None,
        )
    }

    /// Every page's text fits in Tahoma and in DejaVu Sans, which is wider.
    #[test]
    fn every_page_fits_its_text() {
        let s = sheet();
        let mut over = Vec::new();
        for family in ["Tahoma", "DejaVu Sans"] {
            let text = reprisekit::text::TextRenderer::load(&reprisekit::theme::FontConfig {
                family: vec![family.into()],
                ..Default::default()
            });
            if !text.has_face(family) {
                continue;
            }
            for page in &s.sheet.pages {
                page.drawn_with(&reprisekit::theme::Theme::default().paint());
                over.extend(
                    page.overflows(&text, 11)
                        .into_iter()
                        .map(|o| format!("{family}: {o:?}")),
                );
            }
        }
        assert!(over.is_empty(), "{over:?}");
    }

    fn changed(s: &mut Sheet, id: &'static str) -> Outcome {
        s.event(propsheet::Event::Page(
            s.sheet.tabs.open,
            form::Event::Changed(id),
        ))
    }

    fn apply_on(s: &Sheet) -> bool {
        match &s.sheet.open().item(propsheet::APPLY).unwrap().control {
            Control::Button { enabled, .. } => *enabled,
            _ => panic!(),
        }
    }

    /// The whole sheet, every page and its tabs, is set in the second dialog font.
    #[test]
    fn the_sheet_is_set_in_the_second_dialog_font() {
        let s = sheet();
        assert_eq!(s.sheet.font, DialogFont::ShellDlg);
        assert!(s.sheet.pages.iter().all(|p| p.font == DialogFont::ShellDlg));
    }

    /// Every action the compositor accepts, and one it may accept later, opens the sheet clean, and an
    /// Apply of an alarm level leaves the action keys as the file has them.
    #[test]
    fn every_action_name_survives_an_unrelated_apply() {
        let buttons = ACTIONS.iter().map(|(n, _)| *n).chain(["hybrid_sleep"]);
        let critical = CRITICAL_ACTIONS
            .iter()
            .map(|(n, _)| *n)
            .chain(["hybrid_sleep"]);
        for (button, critical) in buttons.zip(critical.cycle()) {
            let text = format!(
                "[power]\nlid_close = \"{button}\"\npower_button = \"{button}\"\n\
                 sleep_button = \"{button}\"\ncritical_action = \"{critical}\"\n"
            );
            let base = crate::settings::load(&text);
            let mut s = Sheet::new(&base, &Power::default(), &Profiles::default(), None);
            s.open_tab(TAB_ADVANCED);
            assert!(!s.dirty(), "{button}/{critical}: opens clean");
            assert!(!apply_on(&s), "{button}/{critical}: Apply is greyed");
            s.open_tab(TAB_ALARMS);
            set_track(&mut s.sheet.pages[TAB_ALARMS], "low_track", 25, true);
            changed(&mut s, "low_track");
            let Outcome::Apply(want, None) = s.event(propsheet::Event::Apply) else {
                panic!("{button}/{critical}: Apply writes");
            };
            let saved = crate::settings::load(&crate::settings::save(&text, &want, &base).unwrap());
            assert_eq!(saved.battery_low, 25);
            assert_eq!(saved.lid_close, button);
            assert_eq!(saved.power_button, button);
            assert_eq!(saved.sleep_button, button);
            assert_eq!(saved.critical_action, critical);
        }
    }

    /// A name the list lacks is an extra row the user can leave and pick again before applying.
    #[test]
    fn an_unknown_action_name_is_an_extra_row() {
        let settings = Settings {
            lid_close: "hybrid_sleep".into(),
            ..Settings::default()
        };
        let mut s = Sheet::new(&settings, &Power::default(), &Profiles::default(), None);
        s.open_tab(TAB_ADVANCED);
        let page = &s.sheet.pages[TAB_ADVANCED];
        assert_eq!(page.combo_selected("lid"), Some(ACTIONS.len()));
        s.sheet.pages[TAB_ADVANCED].set_combo("lid", 0);
        changed(&mut s, "lid");
        assert_eq!(s.pending.lid_close, "nothing");
        s.sheet.pages[TAB_ADVANCED].set_combo("lid", ACTIONS.len());
        changed(&mut s, "lid");
        assert_eq!(s.pending.lid_close, "hybrid_sleep");
        assert!(!s.dirty());
    }

    fn profiles(active: &str) -> Profiles {
        Profiles {
            available: vec![
                "power-saver".into(),
                "balanced".into(),
                "performance".into(),
            ],
            active: active.into(),
        }
    }

    fn profile_combo(s: &Sheet) -> &combo::Combo {
        match &s.sheet.pages[TAB_SCHEMES].item("profile").unwrap().control {
            Control::Combo(c) => c,
            _ => panic!("the profile row is a combo"),
        }
    }

    #[test]
    fn the_profile_combo_lists_the_daemons_profiles() {
        let s = Sheet::new(
            &Settings::default(),
            &Power::default(),
            &profiles("balanced"),
            None,
        );
        let c = profile_combo(&s);
        let rows: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(rows, ["Power saver", "Balanced", "Performance"]);
        assert_eq!(c.selected, 1, "the active profile is selected");
        assert!(c.enabled);
        assert!(!s.dirty(), "opens clean");

        let s = sheet();
        let c = profile_combo(&s);
        assert!(c.items.is_empty(), "no daemon, no rows");
        assert!(!c.enabled, "no daemon, the combo is greyed");
    }

    #[test]
    fn a_picked_profile_is_applied_and_an_outside_change_follows() {
        let mut s = Sheet::new(
            &Settings::default(),
            &Power::default(),
            &profiles("balanced"),
            None,
        );
        s.open_tab(TAB_SCHEMES);
        s.refresh_profiles(&profiles("performance"));
        assert_eq!(profile_combo(&s).selected, 2);
        assert!(!s.dirty());
        s.sheet.pages[TAB_SCHEMES].set_combo("profile", 0);
        changed(&mut s, "profile");
        assert!(s.dirty());
        assert!(apply_on(&s));
        s.refresh_profiles(&profiles("balanced"));
        assert_eq!(profile_combo(&s).selected, 0, "the pick stays");
        assert_eq!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(Settings::default(), Some("power-saver".into()))
        );
        assert!(!s.dirty());
        assert_eq!(profile_combo(&s).selected, 0);
        s.sheet.pages[TAB_SCHEMES].set_combo("profile", 0);
        changed(&mut s, "profile");
        assert!(!s.dirty());
        s.sheet.pages[TAB_SCHEMES].set_combo("profile", 2);
        changed(&mut s, "profile");
        assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
        assert_eq!(profile_combo(&s).selected, 0);
    }

    #[test]
    fn minutes_round_trip() {
        assert_eq!(minutes_label(1), "After 1 min");
        assert_eq!(minutes_label(20), "After 20 mins");
        assert_eq!(minutes_label(60), "After 1 hour");
        assert_eq!(minutes_label(120), "After 2 hours");
        assert_eq!(minutes_label(0), "Never");
        assert_eq!(MINUTES[minutes_index(900)], 15);
        assert_eq!(MINUTES[minutes_index(0)], 0);
        assert_eq!(MINUTES[minutes_index(70 * 60)], 60, "nearest wins");
    }

    #[test]
    fn every_control_sits_inside_its_page() {
        let s = sheet();
        let page_bottom = CLIENT.1 - 36;
        for (t, page) in s.sheet.pages.iter().enumerate() {
            for item in &page.items {
                if [propsheet::OK, propsheet::CANCEL, propsheet::APPLY].contains(&item.id) {
                    continue;
                }
                let r = item.rect;
                assert!(
                    r.x >= 6 && r.right() <= CLIENT.0 - 6,
                    "{t}/{}: {r:?}",
                    item.id
                );
                assert!(
                    r.y >= 28 && r.bottom() <= page_bottom,
                    "{t}/{}: {r:?}",
                    item.id
                );
            }
        }
        let p = &s.sheet.pages[TAB_SCHEMES];
        assert_eq!(p.item("profile").unwrap().rect, Rect::new(32, 126, 334, 21));
        assert_eq!(
            p.item("ac_monitor").unwrap().rect,
            Rect::new(146, 214, 108, 21)
        );
        assert_eq!(
            p.item("bat_standby").unwrap().rect,
            Rect::new(262, 246, 108, 21)
        );
    }

    #[test]
    fn the_battery_column_inherits_until_it_differs() {
        let mut s = sheet();
        assert!(!s.dirty());
        s.open_tab(TAB_SCHEMES);
        s.sheet.pages[TAB_SCHEMES].set_combo("ac_monitor", 3);
        changed(&mut s, "ac_monitor");
        assert_eq!(s.pending.screen_off_after, 300);
        assert_eq!(s.pending.battery_screen_off_after, None);
        match &s.sheet.pages[TAB_SCHEMES]
            .item("bat_monitor")
            .unwrap()
            .control
        {
            Control::Combo(c) => assert_eq!(MINUTES[c.selected], 5),
            _ => panic!(),
        }
        s.sheet.pages[TAB_SCHEMES].set_combo("bat_monitor", 0);
        changed(&mut s, "bat_monitor");
        assert_eq!(s.pending.battery_screen_off_after, Some(60));
        s.sheet.pages[TAB_SCHEMES].set_combo("bat_monitor", 3);
        changed(&mut s, "bat_monitor");
        assert_eq!(s.pending.battery_screen_off_after, None);
        assert!(s.dirty());
        match s.event(propsheet::Event::Apply) {
            Outcome::Apply(w, None) => assert_eq!(w.screen_off_after, 300),
            other => panic!("{other:?}"),
        }
        assert!(!s.dirty());
    }

    #[test]
    fn an_alarm_check_off_writes_zero_and_keeps_its_level() {
        let mut s = sheet();
        s.open_tab(TAB_ALARMS);
        s.sheet.pages[TAB_ALARMS].set_check("low_check", false);
        changed(&mut s, "low_check");
        assert_eq!(s.pending.battery_low, 0);
        match &s.sheet.pages[TAB_ALARMS].item("low_track").unwrap().control {
            Control::TrackBar(t) => {
                assert!(!t.enabled);
                assert_eq!(t.pos, 2, "10% stays on the slider");
            }
            _ => panic!(),
        }
        s.sheet.pages[TAB_ALARMS].set_check("low_check", true);
        changed(&mut s, "low_check");
        assert_eq!(s.pending.battery_low, 10);
        set_track(&mut s.sheet.pages[TAB_ALARMS], "low_track", 25, true);
        changed(&mut s, "low_track");
        assert_eq!(s.pending.battery_low, 25);
    }

    #[test]
    fn advanced_combos_map_to_config_names() {
        let mut s = sheet();
        s.open_tab(TAB_ADVANCED);
        s.sheet.pages[TAB_ADVANCED].set_combo("lid", 1);
        changed(&mut s, "lid");
        assert_eq!(s.pending.lid_close, "ask");
        s.sheet.pages[TAB_ADVANCED].set_combo("sleep_btn", 2);
        changed(&mut s, "sleep_btn");
        assert_eq!(s.pending.sleep_button, "screen_off");
        s.sheet.pages[TAB_ADVANCED].set_combo("power_btn", 6);
        changed(&mut s, "power_btn");
        assert_eq!(s.pending.power_button, "turnoff");
        s.sheet.pages[TAB_ADVANCED].set_check("show_icon", true);
        changed(&mut s, "show_icon");
        assert!(s.pending.always_show_icon);
        match s.event(propsheet::Event::Ok) {
            Outcome::ApplyAndClose(w, None) => assert!(w.always_show_icon),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_failed_apply_leaves_the_values_pending() {
        let mut s = sheet();
        s.open_tab(TAB_ADVANCED);
        s.sheet.pages[TAB_ADVANCED].set_check("show_icon", true);
        changed(&mut s, "show_icon");
        let before = s.applied.clone();
        assert!(matches!(
            s.event(propsheet::Event::Apply),
            Outcome::Apply(..)
        ));
        assert!(!s.dirty());
        assert!(!apply_on(&s));
        s.unapplied(&before);
        assert!(s.dirty(), "the edit is still pending");
        assert!(apply_on(&s));
        assert!(s.pending.always_show_icon, "and it kept its value");
        assert_eq!(s.applied, before);
    }

    #[test]
    fn cancel_puts_the_file_back() {
        let mut s = sheet();
        s.open_tab(TAB_ADVANCED);
        s.sheet.pages[TAB_ADVANCED].set_check("show_icon", true);
        changed(&mut s, "show_icon");
        assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
        assert!(!s.dirty());
        assert!(!s.pending.always_show_icon);
    }

    #[test]
    fn the_meter_reads_the_snapshot() {
        let mut s = sheet();
        s.refresh_power(&Power {
            on_battery: true,
            battery: Some(Battery {
                state: State::Discharging,
                percent: 45,
                time_to_empty: 7800,
                time_to_full: 0,
            }),
        });
        let p = &s.sheet.pages[TAB_METER];
        let text = |id| match &p.item(id).unwrap().control {
            Control::Label { text, .. } => text.clone(),
            _ => panic!(),
        };
        assert_eq!(text("source_value"), "Batteries");
        assert_eq!(text("status_value"), "Discharging");
        assert_eq!(text("charge_value"), "45%");
        assert_eq!(text("remaining_value"), "2:10");
        assert_eq!(text("status_value"), "Discharging");
    }
}
