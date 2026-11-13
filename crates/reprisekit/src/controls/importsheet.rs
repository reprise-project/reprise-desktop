//! The import sheet: another desktop's settings, a checked row per change (its value now and the one
//! it takes) and greyed rows for what has no setting. The list sits beside the form.

use crate::access::{ROOT, Tree, accesskit::NodeId};
use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, BarHit};
use crate::controls::form::{self, Form, Item};
use crate::controls::listview::{self, Column, Group, ListView, Rows};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::selection::Mods;
use crate::text::{TextRenderer, escape_mnemonic, wrap};
use crate::theme::{TextUnits, Theme};
use std::borrow::Cow;

pub const CLIENT: (i32, i32) = (520, 392);
pub const IMPORT: &str = "import";
pub const CANCEL: &str = "cancel";
pub const CLOSE: &str = "close";
const SOURCE: &str = "source";
const SOURCE_LABEL: &str = "source_label";
const HEAD: &str = "head";
const ABOUT: &str = "about";

const MARGIN: i32 = 12;
const WIDE: i32 = CLIENT.0 - 2 * MARGIN;
const HEAD_Y: i32 = 12;
const HEAD_LINES: usize = 2;
const SOURCE_Y: i32 = 44;
const SOURCE_LABEL_W: i32 = 80;
const SOURCE_W: i32 = 140;
const LIST: Rect = Rect::new(MARGIN, 72, WIDE, 236);
const ABOUT_Y: i32 = LIST.y + LIST.height + 8;
const ABOUT_LINES: usize = 2;
const BUTTON_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const COLUMNS: [(&str, i32); 3] = [("Setting", 168), ("Now", 132), ("New", 170)];

/// One row: a change Reprise can make, or with `greyed` a setting it found but can't take, `now`
/// then saying why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub what: String,
    pub now: String,
    pub new: String,
    pub greyed: bool,
}

impl Rows for Vec<Line> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn icon(&self, _row: usize) -> Option<&Canvas> {
        None
    }
    fn cell(&self, row: usize, col: usize) -> Cow<'_, str> {
        let Some(l) = self.get(row) else {
            return Cow::Borrowed("");
        };
        Cow::Borrowed(match col {
            0 => &l.what,
            1 => &l.now,
            2 => &l.new,
            _ => "",
        })
    }
    fn greyed(&self, row: usize) -> bool {
        self.get(row).is_some_and(|l| l.greyed)
    }
}

/// A desktop whose settings were found, its changes before its greyed rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Source {
    pub name: String,
    pub lines: Vec<Line>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stage {
    Asking,
    /// Written; the line says what.
    Done(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    None,
    Redraw,
    /// Write [`Sheet::checked`] of [`Sheet::source`], then [`Sheet::done`].
    Import,
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

pub struct Sheet {
    pub sources: Vec<Source>,
    /// The source the list shows.
    pub source: usize,
    pub stage: Stage,
    pub form: Form,
    pub list: ListView<Vec<Line>>,
    list_focus: bool,
}

impl Sheet {
    /// The sheet over `sources`, the first one shown, laid out at `forms`' text size.
    pub fn new(sources: Vec<Source>, text: &TextRenderer, forms: &Theme) -> Sheet {
        let mut s = Sheet {
            sources,
            source: 0,
            stage: Stage::Asking,
            form: Form::new(Vec::new()),
            list: ListView {
                columns: COLUMNS.iter().map(|(t, w)| Column::new(t, *w)).collect(),
                ..ListView::default()
            },
            list_focus: true,
        };
        s.fill(&forms.controls());
        s.lay_out(text, forms);
        s
    }

    fn shown(&self) -> Option<&Source> {
        self.sources.get(self.source)
    }

    /// The indices into the shown source's lines of the changes checked.
    pub fn checked(&self) -> Vec<usize> {
        let checks = self.list.checks.as_deref().unwrap_or_default();
        (self.list.rows.iter().enumerate())
            .filter(|(i, l)| !l.greyed && checks.get(*i).copied().unwrap_or(false))
            .map(|(i, _)| i)
            .collect()
    }

    /// The list shows the picked source, every change checked.
    fn fill(&mut self, cm: &ControlMetrics) {
        let lines = self.shown().map(|s| s.lines.clone()).unwrap_or_default();
        let changes = lines.iter().take_while(|l| !l.greyed).count();
        self.list.groups = Vec::new();
        if changes > 0 {
            self.list.groups.push(Group {
                title: "Changes".into(),
                first: 0,
            });
        }
        if changes < lines.len() {
            self.list.groups.push(Group {
                title: "Found, but Reprise has no setting for it".into(),
                first: changes,
            });
        }
        self.list.checks = Some(lines.iter().map(|l| !l.greyed).collect());
        self.list.rows = lines;
        self.list.top = 0;
        let lay = listview::layout(&self.list, cm.text.rect(LIST), cm);
        let first = (!self.list.rows.is_empty()).then_some(0);
        self.list.select(first, &lay);
    }

    fn headline(&self) -> String {
        let Some(src) = self.shown() else {
            return "Reprise found no GNOME or Plasma settings in this account.".into();
        };
        match &self.stage {
            Stage::Done(line) => line.clone(),
            Stage::Asking if src.lines.iter().any(|l| !l.greyed) => format!(
                "Reprise found these {} settings. Check the ones to bring over; nothing changes \
                 until you click Import.",
                src.name
            ),
            Stage::Asking => format!(
                "Reprise found these {} settings. Each one is already set here or has no \
                 Reprise setting.",
                src.name
            ),
        }
    }

    /// The selected row in full, which its cells may cut short.
    fn about(&self) -> String {
        let Some(l) = self
            .list
            .selection
            .focus
            .and_then(|i| self.list.rows.get(i))
        else {
            return String::new();
        };
        match (l.greyed, l.now.is_empty()) {
            (true, _) => format!("{}: {} ({})", l.what, l.new, l.now),
            (false, true) => format!("{}: {}", l.what, l.new),
            (false, false) => format!("{}: {} (now {})", l.what, l.new, l.now),
        }
    }

    fn ended(&self) -> bool {
        self.sources.is_empty() || matches!(self.stage, Stage::Done(_))
    }

    /// Build the form for the stage at `forms`' text size, keeping the focused item.
    fn lay_out(&mut self, text: &TextRenderer, forms: &Theme) {
        let (u, px) = (forms.text, forms.fonts.size_px);
        let lines = |body: &str, max: usize| -> Vec<String> {
            let mut l = wrap(text, body, u.len(WIDE), px);
            l.truncate(max);
            l.iter().map(|l| escape_mnemonic(l)).collect()
        };
        let head = lines(&self.headline(), HEAD_LINES);
        let head: Vec<&str> = head.iter().map(String::as_str).collect();
        let mut items = vec![Item::lines_at(HEAD, MARGIN, HEAD_Y, WIDE, &head)];
        if self.sources.len() > 1 && !self.ended() {
            let names: Vec<&str> = self.sources.iter().map(|s| s.name.as_str()).collect();
            items.push(Item::label_at(
                SOURCE_LABEL,
                MARGIN,
                SOURCE_Y + 3,
                SOURCE_LABEL_W,
                "Import &from:",
            ));
            items.push(Item::combo(
                SOURCE,
                MARGIN + SOURCE_LABEL_W,
                SOURCE_Y,
                SOURCE_W,
                &names,
                names.len(),
            ));
        }
        let about = lines(&self.about(), ABOUT_LINES);
        let about: Vec<&str> = about.iter().map(String::as_str).collect();
        items.push(Item::lines_at(ABOUT, MARGIN, ABOUT_Y, WIDE, &about));
        let right = Rect::new(CLIENT.0 - MARGIN - BUTTON_W, BUTTON_Y, BUTTON_W, BUTTON_H);
        if self.ended() {
            items.push(Item::button(CLOSE, right, "Close", true, true));
        } else {
            let left = Rect::new(
                right.x - BUTTON_GAP - BUTTON_W,
                BUTTON_Y,
                BUTTON_W,
                BUTTON_H,
            );
            let any = !self.checked().is_empty();
            items.push(Item::button(IMPORT, left, "&Import", any, any));
            items.push(Item::button(CANCEL, right, "Cancel", true, !any));
        }
        let focus = self.form.focus().map(|i| self.form.items[i].id);
        self.form = Form::new(items);
        self.form.set_combo(SOURCE, self.source);
        match focus {
            Some(id) if self.form.can_focus(id) => self.form.focus_id(id),
            _ if self.list_focus && !self.ended() => self.form.set_focus(None),
            _ => {
                self.list_focus = false;
                self.form
                    .focus_id(if self.ended() { CLOSE } else { CANCEL });
            }
        }
        self.form.drawn_with(&forms.paint());
    }

    /// Lay the sheet out again when `forms`' text size is not the one it was built for.
    pub fn follow(&mut self, text: &TextRenderer, forms: &Theme) {
        if self.form.units() != forms.text {
            self.lay_out(text, forms);
        }
    }

    /// The import ended: `line` says how, and only Close is left. When it `wrote`, the list
    /// keeps just the rows written.
    pub fn done(&mut self, line: String, wrote: bool, text: &TextRenderer, forms: &Theme) {
        if wrote {
            let rows: Vec<Line> = (self.checked().into_iter())
                .filter_map(|i| self.list.rows.get(i).cloned())
                .collect();
            self.list.groups = vec![Group {
                title: "Imported".into(),
                first: 0,
            }];
            self.list.checks = None;
            self.list.rows = rows;
            self.list.top = 0;
            self.list.selection = Default::default();
        }
        self.stage = Stage::Done(line);
        self.list_focus = false;
        self.form.set_focus(None);
        self.lay_out(text, forms);
    }

    fn focus_list(&mut self, on: bool) {
        self.list_focus = on;
        self.list.focus_ring = on;
        self.list.inactive = !on;
        if on {
            self.form.set_focus(None);
        }
    }

    fn list_rect(&self, cm: &ControlMetrics) -> Rect {
        cm.text.rect(LIST)
    }

    /// A form event; the host hands Import and Close on.
    pub fn event(&mut self, ev: form::Event, text: &TextRenderer, forms: &Theme) -> Outcome {
        if self.form.focus().is_some() {
            self.focus_list(false);
        }
        match ev {
            form::Event::Pressed(IMPORT) => Outcome::Import,
            form::Event::Default if !self.ended() && !self.checked().is_empty() => Outcome::Import,
            form::Event::Pressed(CANCEL | CLOSE) | form::Event::Default | form::Event::Cancel => {
                Outcome::Close
            }
            form::Event::Changed(SOURCE) => {
                let pick = self.form.combo_selected(SOURCE).unwrap_or(0);
                if pick != self.source {
                    self.source = pick;
                    self.fill(&forms.controls());
                    self.lay_out(text, forms);
                }
                Outcome::Redraw
            }
            form::Event::Changed(_) | form::Event::Redraw => Outcome::Redraw,
            _ => Outcome::None,
        }
    }

    /// The list's checks or selection moved: Import and the line under the list follow.
    fn moved(&mut self, text: &TextRenderer, forms: &Theme) -> Outcome {
        self.lay_out(text, forms);
        Outcome::Redraw
    }

    /// A pointer event, if the list takes it: none while the source box's list is open over it.
    pub fn list_pointer(
        &mut self,
        ev: ListInput,
        text: &TextRenderer,
        forms: &Theme,
    ) -> Option<Outcome> {
        if self.form.popup_open() {
            return None;
        }
        let cm = forms.controls();
        let list = self.list_rect(&cm);
        let lay = listview::layout(&self.list, list, &cm);
        match ev {
            ListInput::Press(x, y) if list.contains(x, y) => {
                self.focus_list(true);
                let hit = self.list.hit(&lay, x, y);
                if !self.ended() {
                    self.list.press_check(hit);
                }
                match hit {
                    listview::Hit::Check(i) | listview::Hit::Row(i) => {
                        self.list.select(Some(i), &lay)
                    }
                    listview::Hit::VBar(BarHit::Back) => _ = self.list.scroll(-1, &lay),
                    listview::Hit::VBar(BarHit::Forward) => _ = self.list.scroll(1, &lay),
                    listview::Hit::VBar(BarHit::PageBack) => {
                        _ = self.list.scroll(-(lay.rows as i64), &lay)
                    }
                    listview::Hit::VBar(BarHit::PageForward) => {
                        _ = self.list.scroll(lay.rows as i64, &lay)
                    }
                    _ => {}
                }
                Some(self.moved(text, forms))
            }
            ListInput::Motion(x, y) if self.list.held_check.holding() => {
                let hit = self.list.hit(&lay, x, y);
                Some(match self.list.motion_check(hit) {
                    true => Outcome::Redraw,
                    false => Outcome::None,
                })
            }
            ListInput::Release(x, y) if self.list.held_check.holding() => {
                let hit = self.list.hit(&lay, x, y);
                self.list.release_check(hit);
                Some(self.moved(text, forms))
            }
            ListInput::Scroll(x, y, rows) if list.contains(x, y) => {
                Some(match self.list.scroll(rows as i64, &lay) {
                    true => Outcome::Redraw,
                    false => Outcome::None,
                })
            }
            _ => None,
        }
    }

    /// A key, if the list takes it: Tab moves between the list and the form, and in the list the
    /// arrows move the selection and Space flips its check.
    pub fn list_key(
        &mut self,
        key: &form::Key,
        text: &TextRenderer,
        forms: &Theme,
    ) -> Option<Outcome> {
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
        let cm = forms.controls();
        let lay = listview::layout(&self.list, self.list_rect(&cm), &cm);
        let n = self.list.rows.len() as i64;
        let mods = Mods::default();
        let moved = match flat {
            form::Key::Up => self.list.step(-1, &lay, mods),
            form::Key::Down => self.list.step(1, &lay, mods),
            form::Key::Home => self.list.step(-n, &lay, mods),
            form::Key::End => self.list.step(n, &lay, mods),
            form::Key::PageUp => self.list.step(-(lay.rows as i64), &lay, mods),
            form::Key::PageDown => self.list.step(lay.rows as i64, &lay, mods),
            form::Key::Space if !self.ended() => {
                let i = self.list.selection.focus?;
                self.list.toggle(i)
            }
            _ => return None,
        };
        Some(match moved {
            true => self.moved(text, forms),
            false => Outcome::None,
        })
    }

    /// The screen reader nodes: the form's and the list's. Returns the focused node.
    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, forms: &Theme) -> Option<NodeId> {
        let focus = self.form.access(tree, ROOT, text, forms.fonts.size_px);
        let cm = forms.controls();
        let lay = listview::layout(&self.list, self.list_rect(&cm), &cm);
        let row = self.list.access(&lay, tree, ROOT, "settings");
        if self.list_focus { Some(row) } else { focus }
    }

    /// A screen reader's ScrollIntoView on row `row`; true when the list moved.
    pub fn reveal(&mut self, row: usize, forms: &Theme) -> bool {
        let cm = forms.controls();
        let lay = listview::layout(&self.list, self.list_rect(&cm), &cm);
        self.list.reveal(row, &lay)
    }

    /// A screen reader's Focus at `x`, `y`: a form control there takes the keyboard from the list.
    pub fn focus_at(&mut self, x: i32, y: i32, text: &TextRenderer, px: u32) -> bool {
        let moved = self.form.focus_at(x, y, text, px) != form::Event::None;
        if moved {
            self.focus_list(false);
        }
        moved
    }

    /// The window's size at `u`.
    pub fn size(u: TextUnits) -> (i32, i32) {
        u.size(CLIENT)
    }
}

/// Paint `s` on `c`, whose top-left is the client's, laid out at `p.text`.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, s: &Sheet, px: u32) {
    c.fill_rect(c.bounds(), p.cs.button_face);
    form::draw(c, text, p, &s.form, true, px);
    let cm = p.look.controls().grown(p.text);
    let lay = listview::layout(&s.list, cm.text.rect(LIST), &cm);
    listview::draw(c, text, p, &s.list, &lay, None, px);
    form::draw_popups(c, text, p, &s.form, px);
}

#[cfg(test)]
#[path = "importsheet_tests.rs"]
mod tests;
