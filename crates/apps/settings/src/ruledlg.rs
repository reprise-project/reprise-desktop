//! The Add Window Rule and Edit Window Rule dialog: which windows the rule matches (a program's app
//! id, picked from the open windows or typed, and an optional title), then what it sets. A ticked
//! row sets its key and an unticked one leaves it out; OK greys while the app id is empty. Screen
//! offers each running output by connector and by the monitor on it, and the rule's own when it
//! is not one of them.

use repriseapp::heads::Head;
use repriseconf::displays::{Monitor, monitor_ids};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::updown::{self, UpDown};
use reprisekit::raster::Rect;
use reprisekit::textedit::TextEdit;

use crate::model::DESKTOPS_MAX;
use crate::rulespage::{STATES, screen};
use crate::windowrules::Rule;

const W: i32 = 360;
const MARGIN: i32 = 12;
const GX: i32 = MARGIN + 10;
const FIELD_X: i32 = GX + 104;
const FIELD_W: i32 = W - MARGIN - 10 - FIELD_X;
/// The second up-down of the Size and Position rows.
const SECOND_X: i32 = FIELD_X + 112;
const MATCH_Y: i32 = 8;
const SETS_Y: i32 = MATCH_Y + 134 + LINE_H;
const SETS_H: i32 = 230;
const BUTTON_Y: i32 = SETS_Y + SETS_H + 10;
pub const CLIENT: (i32, i32) = (W, BUTTON_Y + BUTTON_H + MARGIN);

pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
pub const WINDOWS: &str = "wr_windows";
pub const APP_ID: &str = "wr_app_id";
pub const TITLE: &str = "wr_title";
pub const SIZE: &str = "wr_size";
const WIDTH: &str = "wr_width";
const HEIGHT: &str = "wr_height";
pub const POSITION: &str = "wr_position";
const LEFT: &str = "wr_left";
const TOP: &str = "wr_top";
pub const OUTPUT: &str = "wr_output";
const OUTPUT_NAME: &str = "wr_output_name";
pub const STATE: &str = "wr_state";
pub const DESKTOP: &str = "wr_desktop";
const DESKTOP_N: &str = "wr_desktop_n";
pub const ON_TOP: &str = "wr_on_top";
pub const STICKY: &str = "wr_sticky";
pub const TAB_GROUP: &str = "wr_tab_group";

/// The From a window combo's first row, standing for no pick.
const NO_WINDOW: &str = "(Pick one to fill in its app ID)";

/// Each value row's check box and the fields it lights.
const TICKED: [(&str, &[&str]); 4] = [
    (SIZE, &[WIDTH, "wr_wide", HEIGHT, "wr_high"]),
    (POSITION, &[LEFT, "wr_from_left", TOP, "wr_from_top"]),
    (OUTPUT, &[OUTPUT_NAME]),
    (DESKTOP, &[DESKTOP_N]),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Accept(Rule),
    Cancel,
}

pub struct RuleDialog {
    pub form: Form,
    pub title: &'static str,
    /// The rule it was opened on: the keys the dialog does not show come back from it.
    was: Rule,
    /// The open windows' app ids, in the combo after [`NO_WINDOW`].
    windows: Vec<String>,
    /// The Screen combo's rows, as `output` values.
    outputs: Vec<String>,
}

/// The Screen values for `heads`: each connector, then the monitor id Always Open Here writes for
/// the monitor on it when that is not the connector. Ids are told apart among the enabled heads,
/// the ones the compositor lays out.
pub fn screens(heads: &[Head]) -> Vec<String> {
    let enabled: Vec<&Head> = heads.iter().filter(|h| h.enabled).collect();
    let monitors: Vec<Monitor> = (enabled.iter())
        .map(|h| Monitor {
            connector: &h.name,
            make: &h.make,
            model: &h.model,
            serial: &h.serial,
        })
        .collect();
    let ids = monitor_ids(&monitors);
    let mut out = Vec::new();
    for h in heads {
        out.push(h.name.clone());
        let id = (enabled.iter().position(|e| e.name == h.name)).map(|i| &ids[i]);
        if let Some(id) = id.filter(|id| **id != h.name) {
            out.push(id.clone());
        }
    }
    out
}

impl RuleDialog {
    /// The dialog over `rule`, or for a new rule when `None`; `outputs` are the Screen values
    /// ([`screens`]).
    pub fn new(rule: Option<&Rule>, mut outputs: Vec<String>) -> RuleDialog {
        let (title, r) = match rule {
            Some(r) => ("Edit Window Rule", r.clone()),
            None => ("Add Window Rule", Rule::default()),
        };
        let label = |id, y, text| Item::label_at(id, GX, y + 4, FIELD_X - GX - 4, text);
        let edit = |id, y, text: &Option<String>| {
            Item::edit(
                id,
                Rect::new(FIELD_X, y, FIELD_W, 20),
                text.as_deref().unwrap_or(""),
                true,
            )
        };
        let spin = |id, x, y, v: i64, (lo, hi): (i32, i32)| {
            Item::new(
                id,
                Rect::new(x, y, updown::WIDTH, updown::HEIGHT),
                Control::UpDown(UpDown::new(v as i32, lo, hi)),
            )
        };
        let unit = |id, x, y, text| Item::label_at(id, x + updown::WIDTH + 4, y + 4, 54, text);
        let tick = |id, y, text, on: bool| Item::check(id, GX, y + 3, text).checked(on);
        let [w, h] = r.size.unwrap_or([800, 600]);
        let [x, y] = r.position.unwrap_or([0, 0]);
        let size = (1, 32767);
        let place = (-32768, 32767);
        let row = |i: i32| SETS_Y + 20 + 26 * i;
        let state = (STATES.iter())
            .position(|(v, _)| Some(*v) == r.state.as_deref())
            .unwrap_or(0);
        let states: Vec<&str> = STATES.iter().map(|(_, l)| *l).collect();
        if let Some(o) = &r.output
            && !outputs.contains(o)
        {
            outputs.push(o.clone());
        }
        let output = (r.output.as_ref())
            .and_then(|o| outputs.iter().position(|n| n == o))
            .unwrap_or(0);
        let labels: Vec<String> = outputs.iter().map(|o| screen(o)).collect();
        let screens: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut items = vec![
            Item::group(
                "wr_match",
                Rect::new(MARGIN, MATCH_Y, W - 2 * MARGIN, 126 + LINE_H),
                "Windows",
            ),
            label("wr_windows_label", MATCH_Y + 20, "From a &window:"),
            Item::combo(WINDOWS, FIELD_X, MATCH_Y + 20, FIELD_W, &[NO_WINDOW], 1),
            label("wr_app_id_label", MATCH_Y + 46, "App &ID:"),
            edit(APP_ID, MATCH_Y + 46, &Some(r.app_id.clone())),
            label("wr_title_label", MATCH_Y + 72, "&Title:"),
            edit(TITLE, MATCH_Y + 72, &r.title),
            Item::new(
                "wr_glob_note",
                Rect::new(GX, MATCH_Y + 98, W - 2 * GX, 2 * LINE_H),
                Control::Lines(vec![
                    "The title matches whole, once, as the window opens;".into(),
                    "* stands for any text, ? for one character.".into(),
                ]),
            ),
            Item::group(
                "wr_sets",
                Rect::new(MARGIN, SETS_Y, W - 2 * MARGIN, SETS_H),
                "Open It",
            ),
            tick(SIZE, row(0), "&Size:", r.size.is_some()),
            spin(WIDTH, FIELD_X, row(0), w, size),
            unit("wr_wide", FIELD_X, row(0), "wide"),
            spin(HEIGHT, SECOND_X, row(0), h, size),
            unit("wr_high", SECOND_X, row(0), "high"),
            tick(POSITION, row(1), "&Position:", r.position.is_some()),
            spin(LEFT, FIELD_X, row(1), x, place),
            unit("wr_from_left", FIELD_X, row(1), "from left"),
            spin(TOP, SECOND_X, row(1), y, place),
            unit("wr_from_top", SECOND_X, row(1), "from top"),
            tick(OUTPUT, row(2), "Scr&een:", r.output.is_some()),
            Item::combo(
                OUTPUT_NAME,
                FIELD_X,
                row(2),
                FIELD_W,
                &screens,
                screens.len().clamp(1, 8),
            ),
            label("wr_state_label", row(3), "St&ate:"),
            Item::combo(STATE, FIELD_X, row(3), FIELD_W, &states, states.len()),
            tick(DESKTOP, row(4), "&Desktop:", r.desktop.is_some()),
            spin(
                DESKTOP_N,
                FIELD_X,
                row(4),
                r.desktop.unwrap_or(1),
                (1, DESKTOPS_MAX as i32),
            ),
            Item::check(ON_TOP, GX, row(5) + 3, "Always on t&op")
                .checked(r.always_on_top == Some(true)),
            Item::check(STICKY, GX, row(6) + 3, "On every des&ktop")
                .checked(r.sticky == Some(true)),
            label("wr_tab_group_label", row(7), "Tab &group:"),
            edit(TAB_GROUP, row(7), &r.tab_group),
        ];
        let cancel_x = W - MARGIN - BUTTON_W;
        items.push(Item::button(
            OK,
            Rect::new(
                cancel_x - BUTTON_GAP - BUTTON_W,
                BUTTON_Y,
                BUTTON_W,
                BUTTON_H,
            ),
            "OK",
            true,
            true,
        ));
        items.push(Item::button(
            CANCEL,
            Rect::new(cancel_x, BUTTON_Y, BUTTON_W, BUTTON_H),
            "Cancel",
            true,
            false,
        ));
        let mut form = Form::new(items);
        form.set_combo(STATE, state);
        form.set_combo(OUTPUT_NAME, output);
        form.set_enabled(OUTPUT, !outputs.is_empty());
        form.set_focus(Some(0));
        form.focus_id(APP_ID);
        let mut d = RuleDialog {
            form,
            title,
            was: r,
            windows: Vec::new(),
            outputs,
        };
        d.offer_windows(Vec::new());
        d
    }

    /// Offer `app_ids`, the open windows' programs, in the From a window combo, each once; greyed
    /// while there are none.
    pub fn offer_windows(&mut self, app_ids: Vec<String>) {
        let mut ids: Vec<String> = Vec::new();
        for id in app_ids {
            if !id.is_empty() && !ids.contains(&id) {
                ids.push(id);
            }
        }
        let rows: Vec<String> = std::iter::once(NO_WINDOW.to_string())
            .chain(ids.iter().cloned())
            .collect();
        self.form.set_combo_rows(WINDOWS, &rows, 0, 10);
        self.form.set_enabled(WINDOWS, !ids.is_empty());
        self.form.set_enabled("wr_windows_label", !ids.is_empty());
        self.windows = ids;
        self.sync();
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        match ev {
            form::Event::Changed(WINDOWS) => {
                let pick = self.form.combo_selected(WINDOWS).unwrap_or(0);
                if let Some(id) = pick
                    .checked_sub(1)
                    .and_then(|i| self.windows.get(i))
                    .cloned()
                    && let Some(Control::Edit { edit, first, .. }) =
                        self.form.item_mut(APP_ID).map(|i| &mut i.control)
                {
                    *edit = TextEdit::new(id, false);
                    first.set(0);
                }
                self.sync();
                Outcome::Redraw
            }
            form::Event::Changed(_) => {
                self.sync();
                Outcome::Redraw
            }
            form::Event::Pressed(OK) | form::Event::Default => match self.valid() {
                true => Outcome::Accept(self.value()),
                false => Outcome::None,
            },
            form::Event::Pressed(CANCEL) | form::Event::Cancel => Outcome::Cancel,
            form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// A rule needs a program to match.
    pub fn valid(&self) -> bool {
        !self.form.edit_text(APP_ID).trim().is_empty()
    }

    /// The rule the dialog shows. An unticked Always on top or On every desktop leaves its key out,
    /// unless the rule held it false. An Always Open Here rule changed here is no longer the
    /// window menu's, so the menu neither replaces nor removes it.
    pub fn value(&self) -> Rule {
        let f = &self.form;
        let on = |id| f.checked(id) == Some(true);
        let text = |id| Some(f.edit_text(id).trim().to_string()).filter(|t| !t.is_empty());
        let num = |id| match f.item(id).map(|i| &i.control) {
            Some(Control::UpDown(u)) => i64::from(u.value()),
            _ => 0,
        };
        let flag = |id, was: Option<bool>| match on(id) {
            true => Some(true),
            false => was.filter(|v| !v),
        };
        let state = STATES[f.combo_selected(STATE).unwrap_or(0).min(STATES.len() - 1)].0;
        let output = (f.combo_selected(OUTPUT_NAME)).and_then(|i| self.outputs.get(i));
        let title = f.edit_text(TITLE);
        let mut rule = Rule {
            app_id: f.edit_text(APP_ID).trim().to_string(),
            title: (!title.trim().is_empty()).then(|| title.to_string()),
            size: on(SIZE).then(|| [num(WIDTH), num(HEIGHT)]),
            position: on(POSITION).then(|| [num(LEFT), num(TOP)]),
            output: output.filter(|_| on(OUTPUT)).cloned(),
            state: Some(state.to_string()).filter(|s| !s.is_empty()),
            desktop: on(DESKTOP).then(|| num(DESKTOP_N)),
            always_on_top: flag(ON_TOP, self.was.always_on_top),
            sticky: flag(STICKY, self.was.sticky),
            tab_group: text(TAB_GROUP),
            from_menu: self.was.from_menu,
        };
        if rule != self.was {
            rule.from_menu = false;
        }
        rule
    }

    /// Light each ticked row's fields, and OK while the rule is [`valid`](Self::valid).
    fn sync(&mut self) {
        for (check, fields) in TICKED {
            let on = self.form.checked(check) == Some(true);
            for id in fields {
                self.form.set_enabled(id, on);
            }
        }
        let valid = self.valid();
        self.form.set_enabled(OK, valid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_into(d: &mut RuleDialog, id: &str, text: &str) {
        if let Some(Control::Edit { edit, .. }) = d.form.item_mut(id).map(|i| &mut i.control) {
            *edit = TextEdit::new(text, false);
        }
        d.event(form::Event::Changed(APP_ID));
    }

    fn lit(d: &RuleDialog, id: &str) -> bool {
        match &d.form.item(id).unwrap().control {
            Control::Button { enabled, .. } | Control::Edit { enabled, .. } => *enabled,
            Control::Combo(c) => c.enabled,
            Control::UpDown(u) => u.enabled,
            _ => panic!("{id}"),
        }
    }

    pub(crate) fn full() -> Rule {
        Rule {
            app_id: "foot".into(),
            title: Some("*build*".into()),
            size: Some([1024, 768]),
            position: Some([-40, 40]),
            output: Some("DP-1".into()),
            state: Some("maximized".into()),
            desktop: Some(3),
            always_on_top: Some(true),
            sticky: Some(true),
            tab_group: Some("shells".into()),
            from_menu: true,
        }
    }

    #[test]
    fn an_empty_app_id_is_refused() {
        let mut d = RuleDialog::new(None, Vec::new());
        assert_eq!(d.title, "Add Window Rule");
        assert!(!lit(&d, OK), "OK greys with no app id");
        assert_eq!(d.event(form::Event::Default), Outcome::None);
        assert_eq!(d.event(form::Event::Pressed(OK)), Outcome::None);
        type_into(&mut d, APP_ID, "   ");
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::None,
            "blanks are empty"
        );
        type_into(&mut d, APP_ID, " foot ");
        assert!(lit(&d, OK));
        assert_eq!(
            d.event(form::Event::Default),
            Outcome::Accept(Rule {
                app_id: "foot".into(),
                ..Rule::default()
            })
        );
        assert_eq!(d.event(form::Event::Cancel), Outcome::Cancel);
    }

    #[test]
    fn a_rule_shows_in_the_dialog_and_comes_back_out() {
        let d = RuleDialog::new(Some(&full()), Vec::new());
        assert_eq!(d.title, "Edit Window Rule");
        assert_eq!(d.value(), full());
        let plain = Rule {
            always_on_top: Some(false),
            ..Rule {
                app_id: "kitty".into(),
                ..Rule::default()
            }
        };
        assert_eq!(
            RuleDialog::new(Some(&plain), Vec::new()).value(),
            plain,
            "a false stays false"
        );
    }

    #[test]
    fn a_row_ticked_off_leaves_its_key_out_and_greys_its_fields() {
        let mut d = RuleDialog::new(Some(&full()), Vec::new());
        for id in [SIZE, POSITION, OUTPUT, DESKTOP, ON_TOP, STICKY] {
            d.form.set_check(id, false);
        }
        d.form.set_combo(STATE, 0);
        d.event(form::Event::Changed(SIZE));
        for id in [WIDTH, HEIGHT, LEFT, TOP, OUTPUT_NAME, DESKTOP_N] {
            assert!(!lit(&d, id), "{id} greys");
        }
        assert_eq!(
            d.value(),
            Rule {
                app_id: "foot".into(),
                title: Some("*build*".into()),
                tab_group: Some("shells".into()),
                ..Rule::default()
            }
        );
    }

    #[test]
    fn an_always_open_here_rule_changed_here_is_no_longer_the_window_menus() {
        let menu = Rule {
            size: Some([800, 600]),
            from_menu: true,
            ..Rule {
                app_id: "firefox".into(),
                ..Rule::default()
            }
        };
        let mut d = RuleDialog::new(Some(&menu), Vec::new());
        assert_eq!(d.value(), menu, "OK on it as it was keeps it the menu's");
        d.form.set_check(DESKTOP, true);
        d.event(form::Event::Changed(DESKTOP));
        assert_eq!(
            d.value(),
            Rule {
                desktop: Some(1),
                from_menu: false,
                ..menu
            }
        );
    }

    #[test]
    fn screen_offers_the_running_outputs_and_the_rules_own() {
        let screens = |d: &RuleDialog| match d.form.item(OUTPUT_NAME).map(|i| &i.control) {
            Some(Control::Combo(c)) => c.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
            _ => panic!("Screen is a combo"),
        };
        let running = || vec!["DP-1".to_string(), "HDMI-A-1".to_string()];
        let mut d = RuleDialog::new(None, running());
        assert_eq!(screens(&d), ["DP-1", "HDMI-A-1"]);
        assert!(!lit(&d, OUTPUT_NAME), "greyed until Screen is ticked");
        d.form.set_check(OUTPUT, true);
        d.form.set_combo(OUTPUT_NAME, 1);
        type_into(&mut d, APP_ID, "foot");
        assert_eq!(d.value().output.as_deref(), Some("HDMI-A-1"));

        let d = RuleDialog::new(Some(&full()), running());
        assert_eq!(screens(&d), ["DP-1", "HDMI-A-1"]);
        assert_eq!(d.value(), full());
        let gone = Rule {
            output: Some("eDP-1".into()),
            ..full()
        };
        let d = RuleDialog::new(Some(&gone), running());
        assert_eq!(screens(&d), ["DP-1", "HDMI-A-1", "eDP-1"]);
        assert_eq!(d.value(), gone, "an output not running stays picked");
        let d = RuleDialog::new(None, Vec::new());
        assert!(
            matches!(
                d.form.item(OUTPUT).map(|i| &i.control),
                Some(Control::Check { enabled: false, .. })
            ),
            "no outputs, no Screen"
        );
    }

    #[test]
    fn screen_offers_each_monitor_by_the_id_always_open_here_writes() {
        use crate::displaypage::test_head;
        let serial = |name: &str, serial: &str| {
            let mut h = test_head(name);
            h.serial = serial.into();
            h
        };
        let mut bare = test_head("eDP-1");
        (bare.make, bare.model) = (String::new(), String::new());
        let mut off = serial("HDMI-A-1", "XYZ");
        off.enabled = false;
        let heads = [serial("DP-1", "ABC"), bare, off];
        assert_eq!(
            screens(&heads),
            ["DP-1", "Dell Inc.|DELL U2412M|ABC", "eDP-1", "HDMI-A-1"],
            "a monitor without an EDID and a disabled one are offered by connector only"
        );
        let twins = [test_head("DP-1"), test_head("DP-2")];
        assert_eq!(
            screens(&twins),
            [
                "DP-1",
                "Dell Inc.|DELL U2412M||DP-1",
                "DP-2",
                "Dell Inc.|DELL U2412M||DP-2"
            ]
        );

        let mut d = RuleDialog::new(None, screens(&heads));
        let shown = match d.form.item(OUTPUT_NAME).map(|i| &i.control) {
            Some(Control::Combo(c)) => c.items.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
            _ => panic!("Screen is a combo"),
        };
        assert_eq!(
            shown,
            ["DP-1", "Dell Inc. DELL U2412M (ABC)", "eDP-1", "HDMI-A-1"]
        );
        d.form.set_check(OUTPUT, true);
        d.form.set_combo(OUTPUT_NAME, 1);
        type_into(&mut d, APP_ID, "foot");
        assert_eq!(
            d.value().output.as_deref(),
            Some("Dell Inc.|DELL U2412M|ABC")
        );
        let menu = Rule {
            output: Some("Dell Inc.|DELL U2412M|ABC".into()),
            ..full()
        };
        let d = RuleDialog::new(Some(&menu), screens(&heads));
        assert_eq!(d.form.combo_selected(OUTPUT_NAME), Some(1));
        assert_eq!(
            d.value(),
            menu,
            "an Always Open Here rule stays on its monitor"
        );
    }

    #[test]
    fn a_title_keeps_its_spaces() {
        let spaced = Rule {
            title: Some("*build* ".into()),
            ..full()
        };
        assert_eq!(RuleDialog::new(Some(&spaced), Vec::new()).value(), spaced);
    }

    #[test]
    fn picking_an_open_window_fills_in_its_app_id() {
        let mut d = RuleDialog::new(None, Vec::new());
        assert!(!lit(&d, WINDOWS), "greyed until the windows are in");
        d.offer_windows(vec![
            "foot".into(),
            "".into(),
            "reprise.notepad".into(),
            "foot".into(),
        ]);
        assert!(lit(&d, WINDOWS));
        let Some(Control::Combo(c)) = d.form.item(WINDOWS).map(|i| &i.control) else {
            panic!("the combo is there");
        };
        let rows: Vec<&str> = c.items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(rows, [NO_WINDOW, "foot", "reprise.notepad"]);
        d.form.set_combo(WINDOWS, 2);
        assert_eq!(d.event(form::Event::Changed(WINDOWS)), Outcome::Redraw);
        assert_eq!(d.form.edit_text(APP_ID), "reprise.notepad");
        assert!(lit(&d, OK));
    }

    #[test]
    fn no_two_controls_share_an_access_key_and_everything_sits_inside() {
        let d = RuleDialog::new(Some(&full()), Vec::new());
        let mut seen: Vec<(char, &str)> = Vec::new();
        for item in &d.form.items {
            let text = match &item.control {
                Control::Label { text, .. }
                | Control::Check { label: text, .. }
                | Control::Button { label: text, .. } => text,
                _ => continue,
            };
            if let Some(k) = reprisekit::text::access_key(text) {
                assert!(
                    !seen.iter().any(|(c, _)| *c == k),
                    "{} and {seen:?} share Alt+{k}",
                    item.id
                );
                seen.push((k, item.id));
            }
            let r = item.rect;
            assert!(
                r.x >= 0 && r.y >= 0 && r.right() <= CLIENT.0 && r.bottom() <= CLIENT.1,
                "{} at {r:?}",
                item.id
            );
        }
        assert_eq!(seen.len(), 11);
    }
}
