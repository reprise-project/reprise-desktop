//! New > Shortcut's wizard: what the shortcut opens (a typed location, command line or web
//! address, a file Browse... picked, or a program from the list), then its name. Typing clears the
//! list's pick and a pick clears the field, so Next reads whichever came last.
//!
//! The host lists the programs, answers Browse... ([`Outcome::Browse`]) with
//! [`NewShortcut::set_location`], reads the target on Next ([`Outcome::Next`]) and moves on with
//! [`NewShortcut::named`] (or says why not with [`NewShortcut::refuse`]), and writes the launcher on
//! Finish.

use crate::access::{ROOT, Tree};
use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{self, Control, Item};
use crate::controls::listbox::ListBox;
use crate::controls::wizard::{self, INTERIOR_BODY, Style, Wizard};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

pub const CLIENT: (i32, i32) = wizard::CLIENT;
pub fn title() -> String {
    tr!("newshortcut-title")
}

const LOCATION: &str = "location";
const BROWSE: &str = "browse";
const PROGRAMS: &str = "programs";
const NAME: &str = "name";
const EDIT_H: i32 = 21;
const FIELD_Y: i32 = INTERIOR_BODY.y + LINE_H + 4;
const BROWSE_X: i32 = INTERIOR_BODY.x + INTERIOR_BODY.width - BUTTON_W;
const LIST_LABEL_Y: i32 = FIELD_Y + BUTTON_H + 12;
const LIST_Y: i32 = LIST_LABEL_Y + LINE_H + 4;
const LIST_H: i32 = 116;
/// Two lines under the list for why Next found nothing.
const PROBLEM: &str = "problem";
const PROBLEM_Y: i32 = LIST_Y + LIST_H + 6;
const NAME_PROBLEM_Y: i32 = LIST_LABEL_Y + 2 * LINE_H;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Target,
    Name,
}

/// What the first page points the shortcut at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picked<'a> {
    /// Row `i` of the programs the wizard was given.
    Program(usize),
    /// The location field's text, trimmed.
    Typed(&'a str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Browse... pressed: the host picks a file and hands it to [`NewShortcut::set_location`].
    Browse,
    /// Next on the first page: the host reads [`NewShortcut::picked`], then
    /// [`NewShortcut::named`] with a name for it, or [`NewShortcut::refuse`].
    Next,
    /// Finish: the host writes the shortcut named [`NewShortcut::name`].
    Finish,
    Cancel,
}

pub struct NewShortcut {
    pub page: Page,
    pub wizard: Wizard,
    programs: Vec<String>,
    icons: Vec<Option<Canvas>>,
    location: String,
    program: Option<usize>,
    name: String,
    /// Why the last Next found nothing, under the list until the next edit.
    problem: String,
}

impl NewShortcut {
    /// The first page, its list holding `programs` (labels, in order).
    pub fn new(programs: Vec<String>, forms: &Theme) -> NewShortcut {
        let mut w = NewShortcut {
            page: Page::Target,
            wizard: Wizard::new(
                Style::Exterior {
                    title: String::new(),
                },
                vec![],
                true,
                false,
            ),
            programs,
            icons: Vec::new(),
            location: String::new(),
            program: None,
            name: String::new(),
            problem: String::new(),
        };
        w.go(Page::Target, forms);
        w
    }

    /// The list's row icons, parallel to the programs; drawn from the next layout on.
    pub fn set_icons(&mut self, icons: Vec<Option<Canvas>>) {
        if let Some(Control::ListBox { icons: shown, .. }) =
            self.wizard.form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            shown.clone_from(&icons);
        }
        self.icons = icons;
    }

    fn go(&mut self, page: Page, forms: &Theme) {
        self.page = page;
        let body = INTERIOR_BODY;
        let (style, items) = match page {
            Page::Target => {
                let field_w = BROWSE_X - 8 - body.x;
                let list = ListBox::new(self.programs.clone(), self.program);
                let items = vec![
                    Item::label_at(
                        "location_label",
                        body.x,
                        body.y,
                        body.width,
                        &tr!("newshortcut-type-the-location-of-the-item"),
                    ),
                    Item::edit(
                        LOCATION,
                        Rect::new(body.x, FIELD_Y + 1, field_w, EDIT_H),
                        &self.location,
                        true,
                    ),
                    Item::button(
                        BROWSE,
                        Rect::new(BROWSE_X, FIELD_Y, BUTTON_W, BUTTON_H),
                        "B&rowse...",
                        true,
                        false,
                    ),
                    Item::label_at(
                        "programs_label",
                        body.x,
                        LIST_LABEL_Y,
                        body.width,
                        &tr!("newshortcut-or-pick-a-program"),
                    ),
                    Item::new(
                        PROGRAMS,
                        Rect::new(body.x, LIST_Y, body.width, LIST_H),
                        Control::ListBox {
                            list,
                            icons: self.icons.clone(),
                        },
                    ),
                    Item::lines(
                        PROBLEM,
                        Rect::new(body.x, PROBLEM_Y, body.width, 2 * LINE_H),
                        &[self.problem.as_str()],
                    ),
                ];
                let style = Style::Interior {
                    title: title(),
                    subtitle: tr!("newshortcut-what-do-you-want-the-shortcut"),
                    icon: None,
                };
                (style, items)
            }
            Page::Name => {
                let items = vec![
                    Item::label_at(
                        "name_label",
                        body.x,
                        body.y,
                        body.width,
                        &tr!("newshortcut-type-a-name-for-this-shortcut"),
                    ),
                    Item::edit(
                        NAME,
                        Rect::new(body.x, FIELD_Y + 1, body.width, EDIT_H),
                        &self.name,
                        true,
                    ),
                    Item::lines_at(
                        "finish_note",
                        body.x,
                        LIST_LABEL_Y,
                        body.width,
                        &[&tr!("newshortcut-click-finish-to-create-the-shortcut")],
                    ),
                    Item::lines(
                        PROBLEM,
                        Rect::new(body.x, NAME_PROBLEM_Y, body.width, 2 * LINE_H),
                        &[name_problem(&self.name).as_str()],
                    ),
                ];
                let style = Style::Interior {
                    title: tr!("newshortcut-name-the-shortcut"),
                    subtitle: tr!("newshortcut-what-do-you-want-to-call"),
                    icon: None,
                };
                (style, items)
            }
        };
        let mut wizard = Wizard::new(style, items, page == Page::Target, page == Page::Name);
        let focus = match page {
            Page::Target if self.program.is_some() => PROGRAMS,
            Page::Target => LOCATION,
            Page::Name => NAME,
        };
        wizard.form.focus_id(focus);
        if page == Page::Name {
            wizard.form.select_edit_text(NAME);
        }
        wizard.form.drawn_with(&forms.paint());
        self.wizard = wizard;
        self.gate();
    }

    /// Next and Finish live only with something to point at and a usable name.
    fn gate(&mut self) {
        let on = match self.page {
            Page::Target => self.picked().is_some(),
            Page::Name => valid_name(&self.name),
        };
        self.wizard.set_next_enabled(on);
    }

    /// What the first page points at; None while both the field and the list are empty.
    pub fn picked(&self) -> Option<Picked<'_>> {
        match self.program {
            Some(i) => Some(Picked::Program(i)),
            None => Some(self.location.trim())
                .filter(|t| !t.is_empty())
                .map(Picked::Typed),
        }
    }

    /// The location field set to Browse...'s pick, which clears the list's.
    pub fn set_location(&mut self, path: &str) {
        self.location = path.to_string();
        self.program = None;
        self.wizard.form.show_edit_text(LOCATION, path);
        self.clear_list();
        self.clear_problem();
        self.wizard.form.focus_id(LOCATION);
        self.gate();
    }

    /// Next found nothing to point at: `why` shows under the list.
    pub fn refuse(&mut self, why: &str, forms: &Theme) {
        self.problem = why.to_string();
        self.go(Page::Target, forms);
    }

    /// The name page, its field holding `name` selected.
    pub fn named(&mut self, name: &str, forms: &Theme) {
        self.name = name.to_string();
        self.go(Page::Name, forms);
    }

    pub fn name(&self) -> &str {
        self.name.trim()
    }

    fn clear_problem(&mut self) {
        if !self.problem.is_empty() {
            self.problem.clear();
            self.wizard.form.set_lines(PROBLEM, &[String::new()]);
        }
    }

    fn clear_list(&mut self) {
        if let Some(Control::ListBox { list, .. }) =
            self.wizard.form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            list.selected = None;
        }
    }

    /// Lay the page out again when `forms`' text size is not the one it was built for.
    pub fn follow(&mut self, forms: &Theme) {
        if self.wizard.form.units() != forms.text {
            self.go(self.page, forms);
        }
    }

    pub fn event(&mut self, ev: form::Event, forms: &Theme) -> Outcome {
        match self.wizard.event(ev) {
            wizard::Event::None => Outcome::None,
            wizard::Event::Redraw => Outcome::Redraw,
            wizard::Event::Back => {
                self.go(Page::Target, forms);
                Outcome::Redraw
            }
            wizard::Event::Next => Outcome::Next,
            wizard::Event::Finish => Outcome::Finish,
            wizard::Event::Cancel => Outcome::Cancel,
            wizard::Event::Page(form::Event::Pressed(BROWSE)) => Outcome::Browse,
            wizard::Event::Page(form::Event::Changed(LOCATION)) => {
                self.location = self.wizard.form.edit_text(LOCATION).to_string();
                self.clear_problem();
                if self.program.take().is_some() {
                    self.clear_list();
                }
                self.gate();
                Outcome::Redraw
            }
            wizard::Event::Page(form::Event::Changed(PROGRAMS)) => {
                let picked = match self.wizard.form.item(PROGRAMS).map(|i| &i.control) {
                    Some(Control::ListBox { list, .. }) => list.selected,
                    _ => None,
                };
                self.clear_problem();
                if picked.is_some() && !self.location.is_empty() {
                    self.location.clear();
                    self.wizard.form.show_edit_text(LOCATION, "");
                }
                self.program = picked;
                self.gate();
                Outcome::Redraw
            }
            wizard::Event::Page(form::Event::Changed(NAME)) => {
                self.name = self.wizard.form.edit_text(NAME).to_string();
                (self.wizard.form).set_lines(PROBLEM, &[name_problem(&self.name)]);
                self.gate();
                Outcome::Redraw
            }
            wizard::Event::Page(_) => Outcome::Redraw,
        }
    }

    /// The page's `access` part; the focused node becomes the tree's focus.
    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, px: u32) {
        if let Some(f) = self.wizard.access(tree, ROOT, text, px) {
            tree.set_focus(f);
        }
    }
}

/// A name a launcher file can take: something past the blanks, and no `/`.
fn valid_name(name: &str) -> bool {
    !name.trim().is_empty() && name_problem(name).is_empty()
}

/// Why the name page greys Finish for `name`, shown under the field; empty for a usable name
/// or a blank one, which the field's label already asks to fill.
fn name_problem(name: &str) -> String {
    match name.trim() {
        n if n.contains('/') => tr!("newshortcut-name-has-a-slash"),
        "." | ".." => tr!("newshortcut-name-is-dots"),
        _ => String::new(),
    }
}

pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, w: &NewShortcut, cues: bool, px: u32) {
    wizard::draw(c, text, p, &w.wizard, cues, px);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;

    fn programs() -> Vec<String> {
        ["Calculator", "Notepad", "Terminal"]
            .map(String::from)
            .into()
    }

    fn type_in(w: &mut NewShortcut, id: &'static str, s: &str, theme: &Theme) -> Outcome {
        w.wizard.form.show_edit_text(id, s);
        w.event(form::Event::Changed(id), theme)
    }

    fn pick(w: &mut NewShortcut, row: usize, theme: &Theme) {
        if let Some(Control::ListBox { list, .. }) =
            w.wizard.form.item_mut(PROGRAMS).map(|i| &mut i.control)
        {
            list.selected = Some(row);
        }
        w.event(form::Event::Changed(PROGRAMS), theme);
    }

    fn next_live(w: &NewShortcut) -> bool {
        matches!(
            w.wizard.form.item(wizard::NEXT).unwrap().control,
            Control::Button { enabled: true, .. }
        )
    }

    #[test]
    fn next_waits_for_a_location_or_a_program_and_the_last_one_wins() {
        let theme = Theme::default();
        let mut w = NewShortcut::new(programs(), &theme);
        assert_eq!(w.picked(), None);
        assert!(!next_live(&w), "nothing to point at");
        type_in(&mut w, LOCATION, "  ~/notes.txt ", &theme);
        assert_eq!(w.picked(), Some(Picked::Typed("~/notes.txt")));
        assert!(next_live(&w));
        pick(&mut w, 1, &theme);
        assert_eq!(w.picked(), Some(Picked::Program(1)));
        assert_eq!(
            w.wizard.form.edit_text(LOCATION),
            "",
            "a pick clears the field"
        );
        type_in(&mut w, LOCATION, "htop", &theme);
        assert_eq!(
            w.picked(),
            Some(Picked::Typed("htop")),
            "typing clears the pick"
        );
        w.set_location("/home/u/Music");
        assert_eq!(w.picked(), Some(Picked::Typed("/home/u/Music")));
        assert_eq!(
            w.event(form::Event::Pressed(BROWSE), &theme),
            Outcome::Browse
        );
    }

    #[test]
    fn a_refused_next_says_why_under_the_list_until_the_next_edit() {
        let theme = Theme::default();
        let mut w = NewShortcut::new(programs(), &theme);
        type_in(&mut w, LOCATION, "htpo", &theme);
        w.refuse("Cannot find 'htpo'.", &theme);
        let lines = |w: &NewShortcut| match &w.wizard.form.item(PROBLEM).unwrap().control {
            Control::Lines(l) => l.join(" "),
            _ => panic!("the problem lines"),
        };
        assert_eq!(lines(&w), "Cannot find 'htpo'.");
        assert_eq!(
            w.wizard.form.edit_text(LOCATION),
            "htpo",
            "the field keeps the text"
        );
        type_in(&mut w, LOCATION, "htop", &theme);
        assert_eq!(lines(&w), "", "an edit clears it");
    }

    #[test]
    fn next_hands_over_to_the_name_page_and_finish_waits_for_a_usable_name() {
        let theme = Theme::default();
        let mut w = NewShortcut::new(programs(), &theme);
        type_in(&mut w, LOCATION, "htop", &theme);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &theme),
            Outcome::Next
        );
        assert_eq!(w.page, Page::Target, "the host moves on");
        w.named("htop", &theme);
        assert_eq!((w.page, w.name()), (Page::Name, "htop"));
        assert!(next_live(&w));
        let problem = |w: &NewShortcut| match &w.wizard.form.item(PROBLEM).unwrap().control {
            Control::Lines(l) => l.join(" "),
            _ => panic!("the problem lines"),
        };
        for (bad, why) in [
            ("", ""),
            ("   ", ""),
            ("a/b", "A name cannot contain a slash (/)."),
            ("..", "A name cannot be . or .. alone."),
        ] {
            type_in(&mut w, NAME, bad, &theme);
            assert!(!next_live(&w), "{bad:?} greys Finish");
            assert_eq!(problem(&w), why, "the line under the field for {bad:?}");
        }
        w.named("https:--example.com/a", &theme);
        assert!(!next_live(&w), "an offered name with a slash greys Finish");
        assert_eq!(
            problem(&w),
            "A name cannot contain a slash (/).",
            "and says why"
        );
        type_in(&mut w, NAME, "Task Viewer", &theme);
        assert_eq!(
            w.event(form::Event::Pressed(wizard::NEXT), &theme),
            Outcome::Finish
        );
        assert_eq!(w.name(), "Task Viewer");
        w.event(form::Event::Pressed(wizard::BACK), &theme);
        assert_eq!(w.page, Page::Target);
        assert_eq!(
            w.wizard.form.edit_text(LOCATION),
            "htop",
            "Back keeps the field"
        );
        assert_eq!(
            w.event(form::Event::Cancel, &theme),
            Outcome::Cancel,
            "Escape cancels"
        );
    }

    #[test]
    fn every_page_fits_its_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, theme, _| {
            let mut w = NewShortcut::new(programs(), theme);
            let mut out = vec![("Target".to_string(), w.wizard.form.clone())];
            w.named("Notepad", theme);
            out.push(("Name".to_string(), w.wizard.form.clone()));
            w.named("a/b", theme);
            out.push(("Name refused".to_string(), w.wizard.form.clone()));
            out
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for v in [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied())
        {
            for b in v.to_le_bytes() {
                h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
            }
        }
        h
    }

    #[test]
    fn the_pages_pin_under_each_interface_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut got = Vec::new();
        for (name, i) in [
            ("Classic", Interface::Classic),
            ("Blue", Interface::Blue),
            ("Glass", Interface::Glass),
        ] {
            for s in [1, 2] {
                let theme = interfaces::theme_for(i).scaled(s);
                let text = TextRenderer::load(&theme.fonts);
                let mut w = NewShortcut::new(programs(), &theme);
                w.refuse(
                    "Cannot find 'htpo'. Check the spelling, then try again.",
                    &theme,
                );
                pick(&mut w, 1, &theme);
                let mut draw_page = |w: &NewShortcut, page: &str| {
                    let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
                    draw(&mut c, &text, &theme.paint(), w, true, theme.fonts.size_px);
                    got.push((format!("{name} {page} {s}"), fnv(&c)));
                };
                draw_page(&w, "Target");
                w.named("Notepad", &theme);
                draw_page(&w, "Name");
                w.named("a/b", &theme);
                draw_page(&w, "Name refused");
            }
        }
        crate::pins::check("newshortcut", &got);
    }

    #[test]
    #[ignore]
    fn render_new_shortcut_pages() {
        let theme = Theme::default();
        let text = TextRenderer::load(&theme.fonts);
        let mut w = NewShortcut::new(programs(), &theme);
        pick(&mut w, 1, &theme);
        for page in ["target", "name"] {
            let mut c = Canvas::new(CLIENT.0 as u32, CLIENT.1 as u32);
            draw(&mut c, &text, &theme.paint(), &w, true, theme.fonts.size_px);
            crate::raster::write_preview(&c, &format!("newshortcut-{page}.png"));
            w.named("Notepad", &theme);
        }
    }
}
