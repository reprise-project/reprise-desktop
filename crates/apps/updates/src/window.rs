//! Software Updates window: a checked list of updates grouped by source
//! (absent sources have no group), install progress, and Check Now /
//! Install / Close. Unchecked updates stay unchecked across later checks.
//!
//! The [`ListView`] is hosted beside the form; focus runs from the list
//! through the form's tab stops and back.

use std::collections::BTreeSet;

use reprisekit::access::{ROOT, Tree};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, BarHit, LINE_H};
use reprisekit::controls::form::{self, Control, Form, Item};
use reprisekit::controls::listview::{self, Column, Group, ListView, Row};
use reprisekit::controls::metrics::ControlMetrics;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_progress;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::selection::Mods;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

use crate::model::{self, Phase, Restart, Update};

pub const CLIENT: (i32, i32) = (520, 400);

const MARGIN: i32 = 12;
const ICON: Rect = Rect::new(MARGIN, 12, 32, 32);
const MSG_X: i32 = 56;
const MSG_W: i32 = CLIENT.0 - MSG_X - MARGIN;
const MSG_LINES: usize = 3;
pub const LIST: Rect = Rect::new(MARGIN, 60, CLIENT.0 - 2 * MARGIN, 228);
const COLUMNS: [(&str, i32); 3] = [("Name", 214), ("Version", 132), ("Type", 128)];
const ABOUT_Y: i32 = 296;
const PROGRESS_Y: i32 = 318;
const BAR: Rect = Rect::new(MARGIN, 336, CLIENT.0 - 2 * MARGIN, 16);
const BTN_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const CHECK_W: i32 = 90;
const CLOSE_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const ACT_W: i32 = 90;
const ACT_X: i32 = CLOSE_X - BUTTON_GAP - ACT_W;

#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    Check,
    Install,
    Restart,
    Close,
}

/// A key or pointer event as the list reads it; what it doesn't take goes to the form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListInput {
    Press(i32, i32),
    Motion(i32, i32),
    Release(i32, i32),
    Scroll(i32, i32, i32),
}

pub struct Window {
    pub form: Form,
    list: ListView,
    rows: Vec<Update>,
    unchecked: BTreeSet<String>,
    list_focus: bool,
    act: Outcome,
}

fn button(label: &str, enabled: bool, default: bool) -> Control {
    Control::Button {
        label: label.into(),
        enabled,
        default,
    }
}

impl Window {
    pub fn new(icon: Option<Canvas>) -> Window {
        let items = vec![
            Item::picture("icon", ICON, icon),
            Item::new(
                "message",
                Rect::new(MSG_X, 14, MSG_W, LINE_H * MSG_LINES as i32),
                Control::Lines(Vec::new()),
            ),
            Item::label("about", Rect::new(MARGIN, ABOUT_Y, LIST.width, LINE_H), ""),
            Item::label(
                "progress",
                Rect::new(MARGIN, PROGRESS_Y, LIST.width, LINE_H),
                "",
            ),
            Item::new(
                "check",
                Rect::new(MARGIN, BTN_Y, CHECK_W, BUTTON_H),
                button("&Check Now", true, false),
            ),
            Item::new(
                "act",
                Rect::new(ACT_X, BTN_Y, ACT_W, BUTTON_H),
                button("&Install", false, true),
            ),
            Item::new(
                "close",
                Rect::new(CLOSE_X, BTN_Y, BUTTON_W, BUTTON_H),
                button("Close", true, false),
            ),
        ];
        let mut form = Form::new(items);
        form.set_focus(None);
        Window {
            form,
            list: ListView {
                columns: COLUMNS.iter().map(|(t, w)| Column::new(t, *w)).collect(),
                checks: Some(Vec::new()),
                ..ListView::default()
            },
            rows: Vec::new(),
            unchecked: BTreeSet::new(),
            list_focus: true,
            act: Outcome::None,
        }
    }

    pub fn set_icon(&mut self, icon: Canvas) {
        self.form.set_picture("icon", icon);
    }

    fn selected(&self) -> Option<&Update> {
        self.list.selection.focus.and_then(|i| self.rows.get(i))
    }

    pub fn wanted(&self, u: &Update) -> bool {
        !self.unchecked.contains(&u.key())
    }

    #[cfg(test)]
    fn checked(&self) -> Vec<Update> {
        self.rows
            .iter()
            .filter(|u| self.wanted(u))
            .cloned()
            .collect()
    }

    /// Refill the list from `updates`, keeping checks and the selection where the update remains.
    fn fill(&mut self, updates: &[Update], cm: &ControlMetrics) {
        let keep = self.selected().map(Update::key);
        self.rows = updates.to_vec();
        let keys: BTreeSet<String> = self.rows.iter().map(Update::key).collect();
        self.unchecked.retain(|k| keys.contains(k));
        self.list.rows = self
            .rows
            .iter()
            .map(|u| Row {
                icon: None,
                cells: vec![u.name.clone(), u.version_cell(), u.type_cell()],
            })
            .collect();
        self.list.checks = Some(
            self.rows
                .iter()
                .map(|u| !self.unchecked.contains(&u.key()))
                .collect(),
        );
        self.list.groups = Vec::new();
        for (i, u) in self.rows.iter().enumerate() {
            let g = u.origin.group();
            if i == 0 || self.rows[i - 1].origin.group() != g {
                self.list.groups.push(Group {
                    title: g.title().into(),
                    first: i,
                });
            }
        }
        let at = keep
            .and_then(|k| self.rows.iter().position(|u| u.key() == k))
            .or((!self.rows.is_empty()).then_some(0));
        let lay = listview::layout(&self.list, LIST, cm);
        self.list.top = self.list.top.min(self.list.slots().saturating_sub(1));
        self.list.select(at, &lay);
    }

    pub fn sync(
        &mut self,
        phase: &Phase,
        updates: &[Update],
        text: &TextRenderer,
        px: u32,
        cm: &ControlMetrics,
    ) {
        if self.rows != updates {
            self.fill(updates, cm);
        }
        let picked = self.rows.iter().filter(|u| self.wanted(u)).count();
        let headline = model::headline(phase, updates, picked);
        let mut lines = reprisekit::text::wrap(text, &headline, MSG_W, px);
        lines.truncate(MSG_LINES);
        self.form.set_lines("message", &lines);
        self.list.focus_ring = self.list_focus;
        let about = self.selected().map(Update::about).unwrap_or_default();
        self.form.set_text("about", &about);
        self.form.set_text("progress", &model::progress_line(phase));

        let busy = phase.busy();
        self.form.set_enabled("check", !busy);
        let (label, act) = match phase {
            Phase::Installed(Restart::System) => ("&Restart Now", Outcome::Restart),
            _ => ("&Install", Outcome::Install),
        };
        let enabled = match act {
            Outcome::Restart => true,
            _ => !busy && picked > 0,
        };
        if let Some(item) = self.form.item_mut("act") {
            item.control = button(label, enabled, true);
        }
        self.act = if enabled { act } else { Outcome::None };
    }

    fn flipped(&mut self, i: usize) {
        let on = self
            .list
            .checks
            .as_ref()
            .and_then(|c| c.get(i))
            .copied()
            .unwrap_or(true);
        if let Some(u) = self.rows.get(i) {
            if on {
                self.unchecked.remove(&u.key());
            } else {
                self.unchecked.insert(u.key());
            }
        }
    }

    fn focus_list(&mut self, on: bool) {
        self.list_focus = on;
        self.list.focus_ring = on;
        if on {
            self.form.set_focus(None);
        }
    }

    /// A pointer event, if the list takes it.
    pub fn list_pointer(&mut self, ev: ListInput, cm: &ControlMetrics) -> Option<Outcome> {
        let lay = listview::layout(&self.list, LIST, cm);
        match ev {
            ListInput::Press(x, y) if LIST.contains(x, y) => {
                self.focus_list(true);
                let hit = self.list.hit(&lay, x, y);
                self.list.press_check(hit);
                match hit {
                    listview::Hit::Check(i) | listview::Hit::Row(i) => {
                        self.list.select(Some(i), &lay)
                    }
                    listview::Hit::VBar(BarHit::Back) => {
                        self.list.scroll(-1, &lay);
                    }
                    listview::Hit::VBar(BarHit::Forward) => {
                        self.list.scroll(1, &lay);
                    }
                    listview::Hit::VBar(BarHit::PageBack) => {
                        self.list.scroll(-(lay.rows as i64), &lay);
                    }
                    listview::Hit::VBar(BarHit::PageForward) => {
                        self.list.scroll(lay.rows as i64, &lay);
                    }
                    _ => {}
                }
                Some(Outcome::Redraw)
            }
            ListInput::Motion(x, y) if self.list.held_check.holding() => {
                let hit = self.list.hit(&lay, x, y);
                Some(if self.list.motion_check(hit) {
                    Outcome::Redraw
                } else {
                    Outcome::None
                })
            }
            ListInput::Release(x, y) if self.list.held_check.holding() => {
                let hit = self.list.hit(&lay, x, y);
                if let Some(i) = self.list.release_check(hit) {
                    self.flipped(i);
                }
                Some(Outcome::Redraw)
            }
            ListInput::Scroll(x, y, rows) if LIST.contains(x, y) => {
                Some(if self.list.scroll(rows as i64, &lay) {
                    Outcome::Redraw
                } else {
                    Outcome::None
                })
            }
            _ => None,
        }
    }

    /// A key, if the list takes it.
    pub fn list_key(&mut self, key: &form::Key, cm: &ControlMetrics) -> Option<Outcome> {
        if let form::Key::Tab { back } = *key {
            if self.list_focus {
                self.focus_list(false);
                self.form.focus_edge(back);
            } else if !self.form.tab_no_wrap(back) {
                self.focus_list(true);
            }
            return Some(Outcome::Redraw);
        }
        if !self.list_focus {
            return None;
        }
        let flat = match key {
            form::Key::Edit(e) => form::flat_key(e)?,
            other => other.clone(),
        };
        let lay = listview::layout(&self.list, LIST, cm);
        let n = self.list.rows.len() as i64;
        let mods = Mods::default();
        let moved = match flat {
            form::Key::Up => self.list.step(-1, &lay, mods),
            form::Key::Down => self.list.step(1, &lay, mods),
            form::Key::Home => self.list.step(-n, &lay, mods),
            form::Key::End => self.list.step(n, &lay, mods),
            form::Key::PageUp => self.list.step(-(lay.rows as i64), &lay, mods),
            form::Key::PageDown => self.list.step(lay.rows as i64, &lay, mods),
            form::Key::Space => {
                let i = self.list.selection.focus?;
                let t = self.list.toggle(i);
                if t {
                    self.flipped(i);
                }
                t
            }
            _ => return None,
        };
        Some(if moved {
            Outcome::Redraw
        } else {
            Outcome::None
        })
    }

    pub fn event(&mut self, ev: form::Event) -> Outcome {
        if self.form.focus().is_some() {
            self.focus_list(false);
        }
        match ev {
            form::Event::Pressed("check") => Outcome::Check,
            form::Event::Pressed("act") | form::Event::Default => {
                std::mem::replace(&mut self.act, Outcome::None)
            }
            form::Event::Pressed("close") | form::Event::Cancel => Outcome::Close,
            form::Event::Changed(_) | form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32, cm: &ControlMetrics) {
        let focus = self.form.access(tree, ROOT, text, px);
        let lay = listview::layout(&self.list, LIST, cm);
        let row = self.list.access(&lay, tree, ROOT, "updates");
        if let Some(f) = if self.list_focus { Some(row) } else { focus } {
            tree.set_focus(f);
        }
    }

    pub fn draw(&self, c: &mut Canvas, text: &TextRenderer, theme: &Theme, px: u32, phase: &Phase) {
        let paint = Paint {
            s: 1,
            ..theme.paint()
        };
        c.fill_rect(c.bounds(), theme.colors.button_face);
        form::draw(c, text, &paint, &self.form, true, px);
        let lay = listview::layout(&self.list, LIST, &theme.look.controls());
        listview::draw(c, text, &paint, &self.list, &lay, None, px);
        if let Phase::Installing { fraction, .. } = phase {
            draw_progress(c, &paint, BAR, fraction.unwrap_or(0.0), true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Install, Kind, Origin};
    use reprisekit::theme::FontConfig;

    fn update(name: &str) -> Update {
        Update {
            id: format!("{name};2;x86_64;core"),
            name: name.into(),
            current: String::new(),
            version: "2".into(),
            summary: format!("the {name} package"),
            kind: Kind::Normal,
            origin: Origin::Package,
        }
    }

    fn label(w: &Window, id: &str) -> String {
        match w.form.item(id).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            Some(Control::Button { label, .. }) => label.clone(),
            other => panic!("{id} is {other:?}"),
        }
    }

    fn message(w: &Window) -> String {
        match w.form.item("message").map(|i| &i.control) {
            Some(Control::Lines(lines)) => lines.join(" "),
            other => panic!("message is {other:?}"),
        }
    }

    fn enabled(w: &Window, id: &str) -> bool {
        matches!(
            w.form.item(id).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    }

    struct Env {
        text: TextRenderer,
        theme: Theme,
    }

    impl Env {
        fn new() -> Env {
            Env {
                text: TextRenderer::load(&FontConfig::default()),
                theme: Theme::default(),
            }
        }
        fn sync(&self, w: &mut Window, phase: &Phase, ups: &[Update]) {
            let cm = self.theme.look.controls();
            w.sync(phase, ups, &self.text, self.theme.fonts.size_px, &cm);
        }
    }

    /// Two packages, a Flatpak app and runtime, a device's firmware.
    fn mixed() -> Vec<Update> {
        let mut ups = vec![
            update("bash"),
            update("zlib"),
            Update {
                id: "app/org.example.Editor/x86_64/stable".into(),
                name: "Editor".into(),
                current: "2.1".into(),
                version: "2.2".into(),
                summary: "from flathub".into(),
                origin: Origin::App(Install::User),
                ..update("Editor")
            },
            Update {
                id: "runtime/org.example.Platform/x86_64/24".into(),
                name: "Platform".into(),
                version: "24".into(),
                summary: "from flathub".into(),
                origin: Origin::Runtime(Install::System),
                ..update("Platform")
            },
            Update {
                id: "08d460be".into(),
                name: "Integrated Webcam".into(),
                current: "1.2.2".into(),
                version: "1.2.4".into(),
                summary: "Fixes the picture going dark in low light.".into(),
                origin: Origin::Firmware,
                ..update("cam")
            },
        ];
        model::sort(&mut ups);
        ups
    }

    #[test]
    fn install_is_there_only_with_checked_updates_and_nothing_running() {
        let env = Env::new();
        let cm = env.theme.look.controls();
        let mut w = Window::new(None);
        env.sync(&mut w, &Phase::Ready, &[]);
        assert!(!enabled(&w, "act"));
        assert_eq!(w.event(form::Event::Default), Outcome::None);

        let ups = [update("bash"), update("zlib")];
        env.sync(&mut w, &Phase::Ready, &ups);
        assert!(enabled(&w, "act"));
        assert_eq!(label(&w, "about"), "Update: the bash package");
        assert_eq!(w.event(form::Event::Pressed("act")), Outcome::Install);

        assert!(message(&w).starts_with("2 updates are available"));

        w.list_key(&form::Key::Space, &cm);
        env.sync(&mut w, &Phase::Ready, &ups);
        assert!(message(&w).starts_with("1 of 2 updates selected."));
        assert!(enabled(&w, "act"));
        w.list_key(&form::Key::Down, &cm);
        w.list_key(&form::Key::Space, &cm);
        env.sync(&mut w, &Phase::Ready, &ups);
        assert!(w.checked().is_empty());
        assert!(message(&w).starts_with("0 of 2 updates selected."));
        assert!(!enabled(&w, "act"));

        let installing = Phase::Installing {
            fraction: Some(0.5),
            doing: "Installing bash".into(),
        };
        env.sync(&mut w, &installing, &ups);
        assert!(!enabled(&w, "act"));
        assert!(!enabled(&w, "check"));
        assert_eq!(label(&w, "progress"), "Installing bash (50%)");
    }

    #[test]
    fn a_restart_turns_install_into_restart_now() {
        let env = Env::new();
        let mut w = Window::new(None);
        env.sync(&mut w, &Phase::Installed(Restart::System), &[]);
        assert_eq!(label(&w, "act"), "&Restart Now");
        assert_eq!(w.event(form::Event::Default), Outcome::Restart);
    }

    #[test]
    fn the_selection_follows_its_update_when_the_list_changes() {
        let env = Env::new();
        let cm = env.theme.look.controls();
        let mut w = Window::new(None);
        env.sync(&mut w, &Phase::Ready, &[update("a"), update("b")]);
        w.list_key(&form::Key::Down, &cm);
        env.sync(&mut w, &Phase::Ready, &[update("b"), update("c")]);
        assert_eq!(w.selected().map(|u| u.name.as_str()), Some("b"));
    }

    #[test]
    fn each_source_gets_a_heading_and_an_unchecked_update_stays_unchecked() {
        let env = Env::new();
        let cm = env.theme.look.controls();
        let mut w = Window::new(None);
        let ups = mixed();
        env.sync(&mut w, &Phase::Ready, &ups);
        let heads: Vec<(&str, usize)> = w
            .list
            .groups
            .iter()
            .map(|g| (g.title.as_str(), g.first))
            .collect();
        assert_eq!(
            heads,
            [("System packages", 0), ("Flatpak", 2), ("Firmware", 4)]
        );
        assert_eq!(
            w.list.rows[4].cells,
            ["Integrated Webcam", "1.2.2 → 1.2.4", "Update"]
        );
        assert_eq!(w.list.rows[2].cells[2], "Application (user)");
        assert_eq!(w.list.rows[3].cells[2], "Runtime (system)");

        let lay = listview::layout(&w.list, LIST, &cm);
        let b = w.list.check_rect(&lay, 3);
        let (x, y) = (b.x + b.width / 2, b.y + b.height / 2);
        w.list_pointer(ListInput::Press(x, y), &cm);
        w.list_pointer(ListInput::Release(x, y), &cm);
        assert_eq!(w.checked().len(), 4);
        assert!(w.checked().iter().all(|u| u.name != "Platform"));

        let mut more = ups.clone();
        more.push(update("curl"));
        model::sort(&mut more);
        env.sync(&mut w, &Phase::Ready, &more);
        let names: Vec<String> = w.checked().into_iter().map(|u| u.name).collect();
        assert_eq!(
            names,
            ["bash", "curl", "zlib", "Editor", "Integrated Webcam"]
        );
    }

    #[test]
    fn tab_runs_from_the_list_through_the_buttons_and_back() {
        let env = Env::new();
        let cm = env.theme.look.controls();
        let mut w = Window::new(None);
        env.sync(&mut w, &Phase::Ready, &[update("a")]);
        assert!(w.list_focus);
        w.list_key(&form::Key::Tab { back: false }, &cm);
        assert!(!w.list_focus);
        assert_eq!(w.form.focus(), w.form.index_of("check"));
        assert_eq!(w.list_key(&form::Key::Space, &cm), None);
        w.list_key(&form::Key::Tab { back: true }, &cm);
        assert!(w.list_focus);
        assert_eq!(w.form.focus(), None);
    }

    #[test]
    fn the_grouped_list_draws_its_headings_and_boxes() {
        let env = Env::new();
        let cm = env.theme.look.controls();
        let mut w = Window::new(None);
        let ups = mixed();
        let phase = Phase::Installing {
            fraction: Some(0.5),
            doing: "Updating Editor".into(),
        };
        env.sync(&mut w, &phase, &ups);
        w.list_key(&form::Key::Down, &cm);
        w.list_key(&form::Key::Space, &cm);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        w.draw(
            &mut c,
            &env.text,
            &env.theme,
            env.theme.fonts.size_px,
            &phase,
        );
        let lay = listview::layout(&w.list, LIST, &cm);
        let rule = reprisekit::theme::Color::rgb(166, 202, 240).argb();
        // Each heading's rule starts at the body's left edge, 18 rows into its slot.
        let row_h = listview::ROW_H;
        let slot = listview::GROUP_H;
        let starts = [0, slot + 2 * row_h, 2 * slot + 4 * row_h];
        for (i, s) in starts.iter().enumerate() {
            let y = lay.body.y + s + listview::GROUP_RULE_Y;
            assert_eq!(
                c.get(lay.body.x + 2, y),
                rule,
                "heading {i}'s rule at y {y}"
            );
        }
        // The first row's box holds a check mark; the second, unchecked, is window colour.
        let ink = |row: usize| {
            let b = w.list.check_rect(&lay, row);
            c.get(b.x + b.width / 2, b.y + b.height / 2)
        };
        let window = env.theme.colors.window.argb();
        assert_ne!(ink(0), window, "the first box is checked");
        assert_eq!(ink(1), window, "the second box is unchecked");
        let hl = env.theme.colors.hilight.argb();
        let mid = BAR.y + BAR.height / 2;
        let lit = |from: i32, to: i32| (from..to).any(|x| c.get(x, mid) == hl);
        assert!(lit(BAR.x, BAR.x + 20));
        assert!(lit(BAR.x + BAR.width / 2 - 20, BAR.x + BAR.width / 2));
        assert!(!lit(BAR.x + BAR.width / 2 + 10, BAR.right()));
    }

    #[test]
    #[ignore]
    fn render_updates() {
        let env = Env::new();
        let mut ups = mixed();
        ups[0].kind = Kind::Security;
        ups[0].summary = "Standalone web browser from mozilla.org".into();
        model::sort(&mut ups);
        let phase = Phase::Installing {
            fraction: Some(0.42),
            doing: "Updating Editor".into(),
        };
        let mut w = Window::new(None);
        env.sync(&mut w, &phase, &ups);
        let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
        w.draw(
            &mut c,
            &env.text,
            &env.theme,
            env.theme.fonts.size_px,
            &phase,
        );
        reprisekit::raster::write_preview(&c, "reprise-updates.png");
    }
}
