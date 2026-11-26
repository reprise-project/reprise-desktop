//! The application launcher: a borderless raised panel with a 32 px icon, a
//! single-line field and a list box of matches under it. The field completes the
//! top match inline when it starts with what was typed, keeping the typed
//! prefix's casing, with the completed tail selected so the next key replaces it.
//! Over the app rows, when the text is a sum, the calculator's row with its
//! answer ("= 14"). Open windows hang under a "Windows" heading, above the app
//! rows (then headed "Programs") when the host ranks them first, else under them.
//! Then a greyed "Settings" heading with an etched rule
//! over the settings rows, and each section of found rows the host fills (files by name,
//! by contents, a program's results) under its own heading the same way, the detail dimmed
//! after each name.
//! Arrows step over headings. A query nothing matches shows the Start menu search's greyed
//! [`NO_RESULTS`] line, which takes no highlight.
//! The panel grows with the text size; the bevels and the 32 px icon keep their size, and a row's
//! 16 px icon grows with the text ([`row_icon_px`]). In a work area narrower than the panel the
//! field and list narrow to it; in a shorter one the list shows the lines that fit and scrolls
//! through the rest with the keys and the wheel ([`Launcher::fit`], [`settle`]).
//!
//! Under a look with glass the panel is a piece of glass and the field and rows
//! are the Start menu's search mode's ([`crate::startpanel::search`]): its search
//! field with the italic cue and magnifier (the clear glyph once there is text),
//! under it a light pane of 21 px rows, a 16 px icon grown with the text and the name, the
//! highlighted one in the lit result's box. Settings and each found section hang under
//! group headers with a count and a rule ("Settings (n)").
//!
//! Pure: the host works out the rows and hands them back through `set_calc`,
//! `set_windows`, `set_settings`, `set_rows` and `set_found`.

use crate::access::accesskit;
use crate::classic::widgets::{BarHit, ROW_H as FIELD_H, edit_first_visible};
use crate::completion::Inline;
use crate::controls::listbox;
use crate::controls::listbox::ListBox;
use crate::controls::scrollbar::{RowBar, RowScroll};
use crate::look::panel;
use crate::look::widgets::{Interaction, draw_edit_field, draw_edit_text, draw_scrollbar};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::startpanel::NO_RESULTS;
use crate::startpanel::layout::{GROUP_GAP, HEADER_H, RESULT_H};
use crate::startpanel::search;
use crate::style::{Part, State, Style};
use crate::text::TextRenderer;
use crate::textedit::{Caret, EditKey, Preedit, TextEdit};
use crate::theme::{TextUnits, Theme};

pub const MAX_ROWS: usize = 8;
pub const MAX_SETTINGS: usize = 4;
pub const MAX_FILES: usize = 8;
pub const MAX_WINDOWS: usize = 5;
pub const WINDOWS_HEADING: &str = "Windows";
pub const PROGRAMS_HEADING: &str = "Programs";
pub const SETTINGS_HEADING: &str = "Settings";
/// What a reader calls the field.
pub const SEARCH: &str = "Search";

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub icon: Option<Canvas>,
}

/// A found row: its name, the dimmed detail after it (a file's folder as the host words it,
/// `~/Documents`, or a program result's description), and its 16 px icon.
#[derive(Clone, Debug, PartialEq)]
pub struct FoundRow {
    pub name: String,
    pub detail: String,
    pub icon: Option<Canvas>,
}

/// A heading and the found rows under it, at most [`MAX_FILES`].
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub heading: String,
    pub rows: Vec<FoundRow>,
}

/// The icons [`Launcher::refill_icons`] gives the rows, each list in its rows' order.
#[derive(Clone, Debug, Default)]
pub struct Refill {
    /// The calculator's row's, while it shows.
    pub calc: Option<Canvas>,
    pub windows: Vec<Option<Canvas>>,
    pub apps: Vec<Option<Canvas>>,
    pub settings: Vec<Option<Canvas>>,
    /// Each found section's, in [`Launcher::set_found`]'s order.
    pub found: Vec<Vec<Option<Canvas>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Calc,
    Window(usize),
    App(usize),
    Setting(usize),
    /// Row `.1` of found section `.0`.
    Found(usize, usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Group {
    Windows,
    Programs,
    Settings,
    Found(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Row(Pick),
    Heading(Group),
    /// [`NO_RESULTS`], alone under a query nothing matches.
    NoMatch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Launcher {
    /// Private because `inline`'s tail indexes into its text: every write goes through `key`
    /// or `paste`.
    edit: TextEdit,
    /// The calculator's row, the app names, then the settings and each found section under
    /// its heading ([`Launcher::lines`]); the selection is never on a heading.
    pub list: ListBox,
    pub icons: Vec<Option<Canvas>>,
    calc: Option<Row>,
    windows: Vec<Row>,
    /// The window rows go above the app rows.
    windows_first: bool,
    apps: Vec<Row>,
    settings: Vec<Row>,
    found: Vec<Section>,
    /// The top match's tail shown in `edit`.
    inline: Inline,
    /// The work area the panel fits in (logical width, height), see [`Launcher::fit`].
    room: Option<(i32, i32)>,
    /// The highlight moved since [`settle`] last scrolled it into view.
    reveal: bool,
    /// The pointer's hold on the list's scrollbar ([`bar_press`], [`bar_drag`]).
    bar: RowBar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(EditKey),
    Up,
    Down,
    Enter,
    CtrlEnter,
    Escape,
    /// The menu key or Shift+F10: the highlighted row's context menu.
    Menu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    /// `query()` changed: call `set_calc`, `set_windows`, `set_settings` and `set_rows`,
    /// in that order (and `set_found` when found rows come in).
    QueryChanged,
    Launch(Pick),
    /// Ctrl+Enter on found row `.1` of section `.0`: a file's folder, a result's program.
    OpenContainer(usize, usize),
    /// Open the context menu of a program or file row, highlighted.
    Context(Pick),
    Close,
}

/// Puts `icons` in `slots`, in order. True when one changed.
fn refill<'a>(
    slots: impl Iterator<Item = &'a mut Option<Canvas>>,
    icons: Vec<Option<Canvas>>,
) -> bool {
    let mut changed = false;
    for (slot, icon) in slots.zip(icons) {
        if *slot != icon {
            *slot = icon;
            changed = true;
        }
    }
    changed
}

impl Default for Launcher {
    fn default() -> Self {
        Launcher::new()
    }
}

impl Launcher {
    pub fn new() -> Self {
        Launcher {
            edit: TextEdit::new("", false),
            list: ListBox::new(Vec::new(), None),
            icons: Vec::new(),
            calc: None,
            windows: Vec::new(),
            windows_first: false,
            apps: Vec::new(),
            settings: Vec::new(),
            found: Vec::new(),
            inline: Inline::default(),
            room: None,
            reveal: false,
            bar: RowBar::default(),
        }
    }

    /// Fits the panel in the work area `room` (logical width, height): narrower, the field and
    /// list narrow to it; shorter, the list shows the lines that fit and scrolls through the rest.
    pub fn fit(&mut self, room: (i32, i32)) {
        self.room = Some(room);
    }

    pub fn query(&self) -> &str {
        self.inline.query(&self.edit)
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    pub fn selected(&self) -> usize {
        self.list.selected.unwrap_or(0)
    }

    fn lines(&self) -> Vec<Line> {
        let mut v = Vec::new();
        if self.calc.is_some() {
            v.push(Line::Row(Pick::Calc));
        }
        let windows = |v: &mut Vec<Line>| {
            if !self.windows.is_empty() {
                v.push(Line::Heading(Group::Windows));
                v.extend((0..self.windows.len()).map(|i| Line::Row(Pick::Window(i))));
            }
        };
        let lead = self.windows_lead();
        if lead {
            windows(&mut v);
            if !self.apps.is_empty() {
                v.push(Line::Heading(Group::Programs));
            }
        }
        v.extend((0..self.apps.len()).map(|i| Line::Row(Pick::App(i))));
        if !lead {
            windows(&mut v);
        }
        if !self.settings.is_empty() {
            v.push(Line::Heading(Group::Settings));
            v.extend((0..self.settings.len()).map(|i| Line::Row(Pick::Setting(i))));
        }
        for (s, section) in self.found.iter().enumerate() {
            if !section.rows.is_empty() {
                v.push(Line::Heading(Group::Found(s)));
                v.extend((0..section.rows.len()).map(|i| Line::Row(Pick::Found(s, i))));
            }
        }
        if v.is_empty() && !self.query().is_empty() {
            v.push(Line::NoMatch);
        }
        v
    }

    fn heading(&self, i: usize) -> Option<&str> {
        match self.lines().get(i) {
            Some(Line::Heading(g)) => Some(self.title(*g)),
            _ => None,
        }
    }

    fn title(&self, g: Group) -> &str {
        match g {
            Group::Windows => WINDOWS_HEADING,
            Group::Programs => PROGRAMS_HEADING,
            Group::Settings => SETTINGS_HEADING,
            Group::Found(s) => &self.found[s].heading,
        }
    }

    fn group_rows(&self, g: Group) -> usize {
        match g {
            Group::Windows => self.windows.len(),
            Group::Programs => self.apps.len(),
            Group::Settings => self.settings.len(),
            Group::Found(s) => self.found[s].rows.len(),
        }
    }

    fn group_of(&self, i: usize) -> Option<Group> {
        match self.lines().get(i) {
            Some(Line::Heading(g)) => Some(*g),
            _ => None,
        }
    }

    fn windows_lead(&self) -> bool {
        self.windows_first && !self.windows.is_empty()
    }

    pub fn pick(&self, i: usize) -> Option<Pick> {
        match self.lines().get(i) {
            Some(Line::Row(p)) => Some(*p),
            _ => None,
        }
    }

    pub fn picked(&self) -> Option<Pick> {
        self.list.selected.and_then(|i| self.pick(i))
    }

    /// Up or Down by one row, stepping over a heading.
    fn step(&mut self, delta: i64) {
        let n = self.list.items.len();
        let before = self.list.selected;
        self.list.step(delta, n);
        let on_heading = |l: &Self| l.list.selected.is_some_and(|i| l.pick(i).is_none());
        if on_heading(self) {
            self.list.step(delta, n);
            if on_heading(self) {
                self.list.selected = before;
            }
        }
        self.reveal = true;
    }

    /// Rebuild the list from the rows held, highlighting row `keep` if still a row,
    /// else the first.
    fn rebuild(&mut self, keep: Option<usize>) {
        let lines = self.lines();
        let (mut items, mut icons, mut greyed, mut detail) = (vec![], vec![], vec![], vec![]);
        for line in &lines {
            let (name, icon, dim) = match *line {
                Line::Heading(g) => (self.title(g).to_string(), None, String::new()),
                Line::NoMatch => (NO_RESULTS.to_string(), None, String::new()),
                Line::Row(Pick::Calc) => {
                    let r = self.calc.as_ref().expect("a calc line has its row");
                    (r.name.clone(), r.icon.clone(), String::new())
                }
                Line::Row(Pick::Window(i)) => {
                    let r = &self.windows[i];
                    (r.name.clone(), r.icon.clone(), String::new())
                }
                Line::Row(Pick::App(i)) => {
                    let r = &self.apps[i];
                    (r.name.clone(), r.icon.clone(), String::new())
                }
                Line::Row(Pick::Setting(i)) => {
                    let r = &self.settings[i];
                    (r.name.clone(), r.icon.clone(), String::new())
                }
                Line::Row(Pick::Found(s, i)) => {
                    let f = &self.found[s].rows[i];
                    (f.name.clone(), f.icon.clone(), f.detail.clone())
                }
            };
            items.push(name);
            icons.push(icon);
            greyed.push(!matches!(line, Line::Row(_)));
            detail.push(dim);
        }
        let first = lines.iter().position(|l| matches!(l, Line::Row(_)));
        let selected = keep.filter(|&i| self.pick(i).is_some()).or(first);
        self.list = ListBox::new(items, selected);
        self.list.greyed = greyed;
        self.list.detail = detail;
        self.icons = icons;
        self.reveal = true;
    }

    /// The calculator's row for the current query (its answer, "= 14"), or None.
    /// Call before `set_rows`, which does not complete the field while it is shown.
    pub fn set_calc(&mut self, row: Option<Row>) {
        self.calc = row;
        self.rebuild(None);
    }

    /// The open windows matching the query, best first; `first` puts them above the
    /// app rows, where they take the highlight and the field is not completed. Call
    /// before `set_rows`.
    pub fn set_windows(&mut self, rows: Vec<Row>, first: bool) {
        self.windows = rows.into_iter().take(MAX_WINDOWS).collect();
        self.windows_first = first;
        self.rebuild(None);
    }

    pub fn set_settings(&mut self, rows: Vec<Row>) {
        self.settings = rows.into_iter().take(MAX_SETTINGS).collect();
        self.rebuild(None);
    }

    /// Gives every row the icon the host found for it again (one that landed since), keeping the
    /// list and the highlight as they are. True when a row's icon changed.
    pub fn refill_icons(&mut self, icons: Refill) -> bool {
        let changed = refill(self.calc.iter_mut().map(|r| &mut r.icon), vec![icons.calc])
            | refill(self.windows.iter_mut().map(|r| &mut r.icon), icons.windows)
            | refill(self.apps.iter_mut().map(|r| &mut r.icon), icons.apps)
            | refill(
                self.settings.iter_mut().map(|r| &mut r.icon),
                icons.settings,
            )
            | (self.found.iter_mut().zip(icons.found)).fold(false, |c, (s, icons)| {
                c | refill(s.rows.iter_mut().map(|f| &mut f.icon), icons)
            });
        if !changed {
            return false;
        }
        for (k, line) in self.lines().into_iter().enumerate() {
            let icon = match line {
                Line::Row(Pick::Calc) => self.calc.as_ref().and_then(|r| r.icon.clone()),
                Line::Row(Pick::Window(i)) => self.windows[i].icon.clone(),
                Line::Row(Pick::App(i)) => self.apps[i].icon.clone(),
                Line::Row(Pick::Setting(i)) => self.settings[i].icon.clone(),
                Line::Row(Pick::Found(s, i)) => self.found[s].rows[i].icon.clone(),
                _ => continue,
            };
            if let Some(slot) = self.icons.get_mut(k) {
                *slot = icon;
            }
        }
        true
    }

    /// The host's found sections for the current query, in order, each best first; an empty one
    /// shows no heading. The highlight stays on its row unless that row changed.
    pub fn set_found(&mut self, found: Vec<Section>) {
        let picked = self.picked();
        let row = |f: &[Section], s: usize, i: usize| {
            f.get(s)
                .and_then(|x| x.rows.get(i))
                .map(|r| (r.name.clone(), r.detail.clone()))
        };
        let old = match picked {
            Some(Pick::Found(s, i)) => row(&self.found, s, i),
            _ => None,
        };
        self.found = found
            .into_iter()
            .map(|s| Section {
                rows: s.rows.into_iter().take(MAX_FILES).collect(),
                ..s
            })
            .collect();
        let same = match picked {
            Some(Pick::Found(s, i)) => old.is_some() && row(&self.found, s, i) == old,
            _ => true,
        };
        let keep = picked
            .filter(|_| same)
            .and_then(|p| self.lines().iter().position(|l| *l == Line::Row(p)));
        self.rebuild(keep);
    }

    pub fn key(&mut self, k: Key) -> Event {
        match k {
            Key::Char(ek) => {
                let before = self.query().to_string();
                let tail = self.inline.tail();
                self.inline.key(&mut self.edit, ek);
                if self.query() != before || tail.is_some() {
                    Event::QueryChanged
                } else {
                    Event::None
                }
            }
            Key::Up => {
                self.step(-1);
                Event::None
            }
            Key::Down => {
                self.step(1);
                Event::None
            }
            Key::Enter => self.picked().map_or(Event::Close, Event::Launch),
            Key::CtrlEnter => match self.picked() {
                Some(Pick::Found(s, i)) => Event::OpenContainer(s, i),
                Some(p) => Event::Launch(p),
                None => Event::Close,
            },
            Key::Escape => Event::Close,
            Key::Menu => self.picked().map_or(Event::None, Self::context_of),
        }
    }

    /// A right press on row `p`: it takes the highlight, and a program or
    /// file row opens its context menu.
    pub fn right_press(&mut self, p: Pick) -> Event {
        if !self.highlight(p) {
            return Event::None;
        }
        Self::context_of(p)
    }

    /// Row `p` takes the highlight and nothing starts, as a reader's focus moves it; false
    /// when no such row is shown.
    pub fn highlight(&mut self, p: Pick) -> bool {
        let Some(i) = self.lines().iter().position(|l| *l == Line::Row(p)) else {
            return false;
        };
        self.list.selected = Some(i);
        self.reveal = true;
        true
    }

    fn context_of(p: Pick) -> Event {
        match p {
            Pick::App(_) | Pick::Found(..) => Event::Context(p),
            Pick::Calc | Pick::Window(_) | Pick::Setting(_) => Event::None,
        }
    }

    /// A host's paste: replaces a shown completion as a typed insert would.
    pub fn paste(&mut self, text: &str) -> Event {
        let before = self.query().to_string();
        self.inline.paste(&mut self.edit, text);
        if self.query() != before {
            Event::QueryChanged
        } else {
            Event::None
        }
    }

    /// Text dropped on the field: its first line, as a paste takes it, becomes the query.
    pub fn drop_query(&mut self, text: &str) -> Event {
        let mut edit = TextEdit::new("", false);
        if !edit.paste(text, &[]) {
            return Event::None;
        }
        let before = self.query().to_string();
        self.edit = edit;
        self.inline.reset();
        match self.query() == before {
            true => Event::None,
            false => Event::QueryChanged,
        }
    }

    /// The host's answer to `QueryChanged`: the ranked app rows for `query()`. Found
    /// rows stay until `set_found`. Completes the field from the first app row when
    /// it starts with the query (case-insensitively), keeping the query's casing;
    /// never under the calculator's row.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.apps = rows.into_iter().take(MAX_ROWS).collect();
        self.rebuild(None);
        let query = self.query().to_string();
        let top = self.apps.first().map(|r| r.name.clone());
        let completes = self.inline.completing()
            && self.calc.is_none()
            && !self.windows_lead()
            && !query.is_empty()
            && top.as_deref().is_some_and(|t| {
                t.to_lowercase().starts_with(&query.to_lowercase()) && t.len() > query.len()
            });
        if completes {
            let t = top.unwrap();
            // `query` is a prefix of `t` ignoring case, so its byte length is
            // a char boundary in `t` only when the cases match byte for byte;
            // walk `t` to the same number of chars instead.
            let n = query.chars().count();
            let split = t.char_indices().nth(n).map_or(t.len(), |(i, _)| i);
            self.inline.offer(&mut self.edit, &query, &t[split..]);
        } else {
            // A stale completion with no matching top row: back to the query.
            self.inline.drop_tail(&mut self.edit);
        }
    }
}

pub const W: i32 = 420;
pub const HEAD_H: i32 = 48;
pub const ICON: i32 = 32;

/// A row's icon edge (logical px): 16 px grown with the text, in the list box ([`listbox::icon_px`])
/// and under glass in the Start menu search's rows ([`search::result`] centres it in the grown row).
pub fn row_icon_px(theme: &Theme) -> i32 {
    listbox::icon_px(&controls(theme))
}
const PAD: i32 = 8;
/// Between the heading's label and its rule, and the rule and the list's
/// right edge.
const RULE_GAP: i32 = 6;
/// Under a look with glass: from the pane's edge to the rows' boxes, and
/// from a row's box to a group header, which starts over the icons.
const PANE_PAD: i32 = 4;
const HEADER_IN: i32 = search::at::RESULT_ICON.0;

/// Where the parts go, unscaled, in panel coordinates, laid out for 100% text and
/// grown by [`Theme::text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub icon: Rect,
    pub field: Rect,
    /// The list box frame, under a look with glass the light pane; None
    /// with no rows (the panel is the head alone).
    pub list: Option<Rect>,
    /// Under a look with glass, each list row's rect: an app or found row,
    /// for the heading its group header from the capitals' top, a line
    /// scrolled out an empty rect. Empty otherwise, where the list box lays
    /// its rows out.
    pub rows: Vec<Rect>,
    /// Under a look with glass, the scrollbar down the pane's right while the list is cut to
    /// the room.
    pub bar: Option<Rect>,
    pub width: i32,
    pub height: i32,
}

pub fn layout(l: &Launcher, theme: &Theme) -> Layout {
    let u = theme.text;
    let cut = l.room.map_or(0, |(w, _)| (u.len(W) - w).max(0));
    if panel::glass(theme).is_some() {
        return glass_layout(l, theme, cut);
    }
    let field_x = PAD + ICON + PAD;
    let field = Rect::new(field_x, (HEAD_H - FIELD_H) / 2, W - field_x - PAD, FIELD_H);
    // The list box's bevel keeps its 2 px and its rows their grown height, so it
    // holds every row it shows whole.
    let list = (!l.list.items.is_empty()).then(|| {
        let row_h = controls(theme).list_row;
        let (x, right) = (u.len(PAD), u.len(W - PAD) - cut);
        let h = box_rows(l, theme) as i32 * row_h + 4;
        Rect::new(x, u.len(HEAD_H), right - x, h)
    });
    Layout {
        icon: icon_rect(u),
        field: narrowed(u.rect(field), cut),
        list,
        rows: Vec::new(),
        bar: None,
        width: u.len(W) - cut,
        height: list.map_or(u.len(HEAD_H), |r| r.bottom() + u.len(PAD)),
    }
}

fn narrowed(r: Rect, cut: i32) -> Rect {
    Rect::new(r.x, r.y, r.width - cut, r.height)
}

/// The look's control sizes at the theme's text size, as the list box lays its rows out.
fn controls(theme: &Theme) -> crate::controls::metrics::ControlMetrics {
    theme.look.controls_at_100().grown(theme.text)
}

/// The list box's rows the room holds whole, at least one; every row with no room given.
fn box_rows(l: &Launcher, theme: &Theme) -> usize {
    let n = l.list.items.len();
    let Some((_, h)) = l.room else {
        return n;
    };
    let u = theme.text;
    let fit = (h - u.len(HEAD_H) - u.len(PAD) - 4) / controls(theme).list_row;
    (fit.max(1) as usize).min(n)
}

/// Which lines are headings.
fn headings(l: &Launcher) -> Vec<bool> {
    l.lines()
        .iter()
        .map(|line| matches!(line, Line::Heading(_)))
        .collect()
}

/// Under glass, the lines from `top` the room holds, at least one: a group gap goes over each
/// header but the first shown.
fn glass_shown(heads: &[bool], u: TextUnits, room: Option<(i32, i32)>, top: usize) -> usize {
    let Some((_, room)) = room else {
        return heads.len().saturating_sub(top);
    };
    let mut y = HEAD_H + PANE_PAD;
    let mut k = 0;
    for (i, &head) in heads.iter().enumerate().skip(top) {
        if head && i > top {
            y += GROUP_GAP;
        }
        y += if head { HEADER_H } else { RESULT_H };
        if k > 0 && u.len(y + 1 + PANE_PAD + PAD) > room {
            break;
        }
        k += 1;
    }
    k
}

/// The lines shown from `top`.
fn shown(l: &Launcher, theme: &Theme, heads: &[bool], top: usize) -> usize {
    if panel::glass(theme).is_some() {
        glass_shown(heads, theme.text, l.room, top)
    } else {
        box_rows(l, theme).min(heads.len().saturating_sub(top))
    }
}

/// The furthest the list scrolls: the first top that shows its last line.
fn last_top(l: &Launcher, theme: &Theme, heads: &[bool]) -> usize {
    let n = heads.len();
    (0..n)
        .find(|&t| t + shown(l, theme, heads, t) >= n)
        .unwrap_or(0)
}

/// Scrolls the list cut to the room so the highlight shows after a key, a reader or new rows
/// moved it (a group's first row with its heading), and so no room is left under its last line.
/// The host calls it before laying the panel out.
pub fn settle(l: &mut Launcher, theme: &Theme) {
    let heads = headings(l);
    let mut top = l.list.top.min(last_top(l, theme, &heads));
    if std::mem::take(&mut l.reveal)
        && let Some(s) = l.list.selected
    {
        if s > 0 && heads[s - 1] && s <= top {
            top = s - 1;
        } else if s < top {
            top = s;
        }
        while s >= top + shown(l, theme, &heads, top) {
            top += 1;
        }
    }
    l.list.top = top;
}

/// The wheel over the panel: the list scrolls `rows` lines, the highlight staying. True when
/// it moved.
pub fn wheel(l: &mut Launcher, theme: &Theme, rows: i32) -> bool {
    let heads = headings(l);
    let last = last_top(l, theme, &heads) as i32;
    let top = (l.list.top as i32 + rows).clamp(0, last) as usize;
    let moved = top != l.list.top;
    l.list.top = top;
    moved
}

/// Under glass, the scrollbar's lines: those shown from the top of all.
fn glass_scroll(l: &Launcher, theme: &Theme) -> RowScroll {
    let heads = headings(l);
    RowScroll {
        top: l.list.top,
        visible: shown(l, theme, &heads, l.list.top),
        total: heads.len(),
    }
}

/// A press on the list's scrollbar: an arrow scrolls a line and repeats while held
/// ([`bar_tick`]), the track a page, and the thumb is grabbed ([`bar_drag`]). True when the list
/// moved.
pub fn bar_press(l: &mut Launcher, theme: &Theme, b: BarHit) -> bool {
    let page = shown(l, theme, &headings(l), l.list.top) as i64;
    let rows = l.bar.press(b, page) as i32;
    rows != 0 && wheel(l, theme, rows)
}

/// The pointer at (`x`, `y`) with the scrollbar held: the list follows a held thumb wherever the
/// pointer goes, and a held arrow repeats only while the pointer is on it. True when the list
/// moved.
pub fn bar_drag(l: &mut Launcher, theme: &Theme, x: i32, y: i32) -> bool {
    let lay = layout(l, theme);
    let cm = controls(theme);
    let grab = l.bar.grab();
    let (hit, top) = match (lay.bar, lay.list) {
        (Some(sb), _) => {
            let rows = glass_scroll(l, theme);
            let hit = sb.contains(x, y).then(|| rows.hit(sb, cm.thumb_min, x, y));
            (
                hit.flatten(),
                grab.map(|g| rows.top_at(sb, cm.thumb_min, y, g)),
            )
        }
        (None, Some(frame)) => {
            let ll = listbox::layout(&l.list, frame, &cm);
            let hit = listbox::hit(&l.list, &ll, x, y).and_then(listbox::Hit::bar);
            (hit, grab.map(|g| listbox::top_at(&l.list, &ll, y, g)))
        }
        (None, None) => (None, None),
    };
    l.bar.hover(hit);
    let Some(top) = top else {
        return false;
    };
    let top = top.min(last_top(l, theme, &headings(l)));
    let moved = top != l.list.top;
    l.list.top = top;
    moved
}

/// A host tick at `now` with the scrollbar held: a held arrow steps once it repeats. True when
/// the list moved.
pub fn bar_tick(l: &mut Launcher, theme: &Theme, now: std::time::Instant) -> bool {
    let rows = l.bar.tick(now) as i32;
    rows != 0 && wheel(l, theme, rows)
}

impl Launcher {
    /// Whether the scrollbar's thumb or an arrow is held: the host sends motion, ticks and the
    /// release.
    pub fn bar_held(&self) -> bool {
        self.bar.holding()
    }

    /// The pointer let go. True when the scrollbar was held.
    pub fn bar_release(&mut self) -> bool {
        self.bar.release()
    }
}

/// The 32 px icon on its grown centre.
fn icon_rect(u: TextUnits) -> Rect {
    Rect::new(
        u.len(PAD + ICON / 2) - ICON / 2,
        u.len(HEAD_H / 2) - ICON / 2,
        ICON,
        ICON,
    )
}

/// The glass panel at 100% text with every edge grown by the text size, then `cut` narrower;
/// the 32 px icon keeps its size, its centre moving with the rest. Lines scrolled out get empty
/// rects; a list cut to the room gives the rest of its rows' width to the scrollbar.
fn glass_layout(l: &Launcher, theme: &Theme, cut: i32) -> Layout {
    let u = theme.text;
    let field_x = PAD + ICON + PAD;
    let field_h = search::FIELD.1;
    let field = Rect::new(field_x, (HEAD_H - field_h) / 2, W - field_x - PAD, field_h);
    let heads = headings(l);
    let n = heads.len();
    let top = l.list.top.min(n);
    let end = top + glass_shown(&heads, u, l.room, top);
    let (x, w) = (PAD + PANE_PAD, W - 2 * (PAD + PANE_PAD));
    let mut y = HEAD_H + PANE_PAD;
    let mut rows = vec![Rect::default(); n];
    for (i, row) in rows.iter_mut().enumerate().take(end).skip(top) {
        if heads[i] {
            if i > top {
                y += GROUP_GAP;
            }
            *row = Rect::new(x + HEADER_IN, y, w - 2 * HEADER_IN, HEADER_H);
            y += HEADER_H;
        } else {
            *row = Rect::new(x, y, w, RESULT_H);
            y += RESULT_H;
        }
    }
    // The last lit box runs a row past its 21 px.
    let bottom = y + 1 + PANE_PAD;
    let list = (n > 0).then(|| Rect::new(PAD, HEAD_H, W - 2 * PAD, bottom - HEAD_H));
    let grown = |r: Rect| narrowed(u.rect(r), cut);
    let pane = list.map(grown);
    let bar = pane.filter(|_| end - top < n).map(|pane| {
        let (w, pad) = (controls(theme).scroll, u.len(PANE_PAD));
        Rect::new(
            pane.right() - pad - w,
            pane.y + pad,
            w,
            pane.height - 2 * pad,
        )
    });
    let beside = bar.map_or(0, |b| b.width);
    let row = |r: Rect| match r.width {
        0 => r,
        _ => narrowed(grown(r), beside),
    };
    Layout {
        icon: icon_rect(u),
        field: grown(field),
        list: pane,
        rows: rows.into_iter().map(row).collect(),
        bar,
        width: u.len(W) - cut,
        height: u.len(list.map_or(HEAD_H, |r| r.bottom() + PAD)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(Pick),
    /// The list box's scrollbar ([`bar_press`]).
    Bar(BarHit),
    Field,
    Panel,
}

pub fn hit(l: &Launcher, x: i32, y: i32, theme: &Theme) -> Hit {
    let lay = layout(l, theme);
    if lay.field.contains(x, y) {
        return Hit::Field;
    }
    if let Some(sb) = lay.bar.filter(|sb| sb.contains(x, y)) {
        return glass_scroll(l, theme)
            .hit(sb, controls(theme).thumb_min, x, y)
            .map_or(Hit::Panel, Hit::Bar);
    }
    if !lay.rows.is_empty() {
        let i = lay.rows.iter().position(|r| r.contains(x, y));
        return match i.and_then(|i| l.pick(i)) {
            Some(p) => Hit::Row(p),
            None => Hit::Panel,
        };
    }
    if let Some(frame) = lay.list {
        let ll = listbox::layout(&l.list, frame, &controls(theme));
        match listbox::hit(&l.list, &ll, x, y) {
            Some(listbox::Hit::Row(i)) => return l.pick(i).map_or(Hit::Panel, Hit::Row),
            Some(listbox::Hit::Bar(b)) => return Hit::Bar(b),
            _ => {}
        }
    }
    Hit::Panel
}

/// Line `i`'s rect in panel coordinates, where a menu opened from the keys
/// hangs.
pub fn row_rect(l: &Launcher, i: usize, theme: &Theme) -> Option<Rect> {
    let lay = layout(l, theme);
    if !lay.rows.is_empty() {
        return lay.rows.get(i).copied().filter(|r| r.width > 0);
    }
    let ll = listbox::layout(&l.list, lay.list?, &controls(theme));
    let shown = i.checked_sub(l.list.top)?;
    (i < l.list.items.len() && shown < ll.visible).then(|| {
        Rect::new(
            ll.inner.x,
            ll.inner.y + shown as i32 * ll.row_h,
            ll.inner.width,
            ll.row_h,
        )
    })
}

/// Draw the panel: `icon` is the 32 px icon of the highlighted row (or the
/// host's search icon), `caret` the blink's shown half, `preedit` what an input
/// method composes. Under glass the canvas reaches [`panel::margin`] past the
/// panel for its shadow.
pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    l: &Launcher,
    icon: Option<&Canvas>,
    caret: bool,
    preedit: Option<&Preedit>,
) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let lay = layout(l, theme);
    if let Some((style, spec)) = panel::glass(theme) {
        let g = search::Grow { s, u: theme.text };
        return panel::draw(style, spec, lay.width, lay.height, s, |c| {
            c.set_preedit(preedit.cloned());
            glass_body(c, style, l, &lay, icon, caret, g);
            if let Some(sb) = lay.bar {
                let rows = glass_scroll(l, theme);
                draw_scrollbar(c, p, sb.scaled(s), rows.top, rows.visible, rows.total, None);
            }
        });
    }
    let mut c = Canvas::new((lay.width * s) as u32, (lay.height * s) as u32);
    c.set_preedit(preedit.cloned());
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, theme, text, l, &lay, icon, caret);
    c
}

/// The icon, field and list grown by `g`, as the Start menu draws its search
/// mode, text in `style`'s caption face.
fn glass_body(
    c: &mut Canvas,
    style: &Style,
    l: &Launcher,
    lay: &Layout,
    icon: Option<&Canvas>,
    caret: bool,
    g: search::Grow,
) {
    let s = g.s;
    if let Some(ic) = icon {
        c.blit_over(lay.icon.x * s, lay.icon.y * s, ic);
    }
    let face = crate::look::caption_text(style);
    let cue = search::italic(&style.caption_text.font());
    let field = lay.field.scaled(s);
    search::field(c, style, &face, &cue, field, &l.edit, true, caret, g);
    let Some(pane) = lay.list else {
        return;
    };
    let pane = pane.scaled(s);
    draw_pane(c, style, pane, s);
    for (i, r) in lay.rows.iter().enumerate().filter(|(_, r)| r.width > 0) {
        let r = r.scaled(s);
        if let Some(group) = l.group_of(i) {
            let title = format!("{} ({})", l.title(group), l.group_rows(group));
            search::header(c, style, &face, r, pane, &title, g);
            continue;
        }
        if l.pick(i).is_none() {
            let size = g.px(search::TEXT_PX);
            let x = r.x + (r.width - face.measure(NO_RESULTS, false, size)) / 2;
            let y = r.y + g.len(search::at::RESULT_BASE);
            search::line(c, &face, x, y, r, NO_RESULTS, style.pane_inks.greyed, size);
            continue;
        }
        let (name, lit) = (&l.list.items[i], l.list.selected == Some(i));
        let icon = l.icons.get(i).and_then(Option::as_ref);
        let detail = l.list.detail.get(i).map_or("", String::as_str);
        search::result(c, style, &face, r, icon, name, detail, lit, g);
    }
}

/// The light pane `r` (device pixels): [`Part::StartPane`] without the band the
/// Start menu's search field stands on. The bottom half is the top half turned
/// upside down, the pane's rings being the same along both.
fn draw_pane(c: &mut Canvas, style: &Style, r: Rect, s: i32) {
    let band = style
        .part(Part::StartPane)
        .map_or(0, |img| img.sizing.bottom * s / img.scale);
    let top = Rect::new(r.x, r.y, r.width, r.height - r.height / 2);
    let bottom = Rect::new(r.x, top.bottom(), r.width, r.height / 2);
    for half in [top, bottom] {
        let tall = Rect::new(r.x, half.y, r.width, r.height + band);
        let pane = |c: &mut Canvas| {
            c.with_clip(half, |c| {
                style.draw(c, tall, Part::StartPane, State::Normal, s)
            })
        };
        if half == top {
            pane(c);
        } else {
            c.with_rows_flipped(half, pane);
        }
    }
}

fn body(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    l: &Launcher,
    lay: &Layout,
    icon: Option<&Canvas>,
    caret: bool,
) {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    if let Some(ic) = icon {
        c.blit_over(lay.icon.x * s, lay.icon.y * s, ic);
    }
    // The field always holds the focus.
    let inner = draw_edit_field(
        c,
        p,
        lay.field.scaled(s),
        cs.window,
        true,
        Interaction::Idle,
    );
    let first = edit_first_visible(text, &l.edit, inner.width, px, 0);
    draw_edit_text(
        c,
        text,
        &l.edit,
        inner,
        px,
        p,
        Caret::new(true, caret),
        first,
    );
    if let Some(frame) = lay.list {
        let ll = listbox::layout(&l.list, frame, &controls(theme));
        listbox::draw(c, text, p, &l.list, &ll, &l.icons, None, px);
        let shown = l.list.top..(l.list.top + ll.visible).min(l.list.items.len());
        for (h, label) in shown.filter_map(|i| Some((i, l.heading(i)?))) {
            // From past the heading's label to the list's right edge, on
            // the row's middle.
            let label_w = text.measure(label, false, px) / s;
            let x0 = ll.inner.x + listbox::text_x(&l.list, &ll, &l.icons) + label_w + RULE_GAP;
            let y = ll.inner.y + (h - l.list.top) as i32 * ll.row_h + ll.row_h / 2 - 1;
            c.etched_hline_t(x0 * s, (ll.inner.right() - RULE_GAP) * s, y * s, cs, s);
        }
    }
}

/// The panel's `access` part: field and rows (see `listbox::access_filtered`), headings and the
/// no-match line greyed; under glass each row takes its own rect. Returns the focused node: the
/// highlighted row, else the field.
pub fn access(
    l: &Launcher,
    text: &TextRenderer,
    px: u32,
    tree: &mut crate::access::Tree,
    parent: accesskit::NodeId,
    theme: &Theme,
) -> accesskit::NodeId {
    let lay = layout(l, theme);
    let cm = controls(theme);
    let ll = lay.list.map(|frame| listbox::layout(&l.list, frame, &cm));
    let focus = listbox::access_filtered(
        &l.edit,
        SEARCH,
        lay.field,
        &l.list,
        ll.as_ref(),
        text,
        px,
        tree,
        parent,
    );
    let list = crate::access::Tree::id_of(parent, "rows");
    let rows: Vec<_> = tree
        .get(list)
        .map(|n| n.children().to_vec())
        .unwrap_or_default();
    for (&id, &r) in rows.iter().zip(&lay.rows) {
        if let Some(n) = tree.get_mut(id) {
            n.set_bounds(crate::access::bounds(r));
        }
    }
    for (i, &id) in rows.iter().enumerate() {
        if l.pick(i).is_none()
            && let Some(n) = tree.get_mut(id)
        {
            n.set_disabled();
            n.remove_action(accesskit::Action::Focus);
            n.remove_action(accesskit::Action::Click);
        }
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part_px(style: &Style, part: Part, w: i32, h: i32, x: i32, y: i32) -> u32 {
        let mut c = Canvas::new(w as u32, h as u32);
        let all = c.bounds();
        style.draw(&mut c, all, part, State::Normal, 1);
        c.get(x, y)
    }

    #[test]
    fn under_glass_the_field_rows_and_heading_are_the_start_menus_search_parts() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        set_files(&mut l, files(&[("notes.txt", "~/Documents")]));
        l.key(Key::Down);
        let lay = layout(&l, &theme);
        let c = draw(&theme, &text, &l, None, false, None);
        let m = panel::margin(&theme);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (W + m.left + m.right, lay.height + m.top + m.bottom),
            "the panel and its shadow"
        );
        let at = |x: i32, y: i32| c.get(m.left + x, m.top + y);
        let white = 0xFFFF_FFFF;

        // The field: the search box, 22 tall, centred in the head.
        let f = lay.field;
        assert_eq!(f, Rect::new(48, 13, 364, 22));
        let field = |x, y| part_px(style, Part::StartSearchField, f.width + 2, 24, x + 1, y + 1);
        assert_eq!(at(f.x, f.y + 11), field(0, 11), "the box's left side");
        assert_eq!(at(f.x + 100, f.y), field(100, 0), "its top");
        assert_eq!(at(f.x + 100, f.y + 11), white, "white inside");

        // The pane: white with glass round it, no band at its foot.
        let pane = lay.list.expect("rows");
        assert_eq!(
            pane,
            Rect::new(8, 48, 404, 4 + 2 * 21 + 11 + 21 + 21 + 1 + 4)
        );
        assert_eq!(lay.height, pane.bottom() + 8);
        let a = at(4, pane.y + pane.height / 2) >> 24;
        assert!(a > 0 && a < 0xFF, "glass beside the pane: alpha {a:#x}");
        assert_eq!(
            at(W / 2, pane.bottom() - 4),
            white,
            "the pane's foot is white"
        );

        // Rows: the lit result's box across Notion, none round Notepad.
        assert_eq!(lay.rows[0], Rect::new(12, 52, 396, 21));
        let lit = lay.rows[1];
        assert_eq!(lit, Rect::new(12, 73, 396, 21));
        let lit_px = |x, y| part_px(style, Part::StartLitResult, lit.width, 22, x, y);
        assert_eq!(at(lit.x, lit.y + 11), lit_px(0, 11), "the box's left side");
        assert_eq!(at(lit.right() - 1, lit.y + 11), lit_px(lit.width - 1, 11));
        assert_eq!(at(lit.x + 300, lit.y), lit_px(300, 0), "its top");
        assert_eq!(at(lit.x, lit.y - 10), white, "Notepad is not lit");

        // The Files heading: "Files (1)" a size up, the search rule after it.
        let head = lay.rows[2];
        assert_eq!(head, Rect::new(16, 94 + 11, 388, 21));
        let rule_y = head.y + search::at::HEAD_RULE_Y;
        let label = text_w("Files (1)", search::HEADER_PX);
        let x0 = head.x + label + search::at::HEAD_RULE_GAP;
        let rule = part_px(style, Part::StartSearchRule, 1, 1, 0, 0);
        assert_eq!(at(x0, rule_y), rule, "the rule starts 8 px past the label");
        assert_eq!(at(x0 - 1, rule_y), white);
        assert_eq!(
            at(head.right(), rule_y),
            rule,
            "and runs a pixel past the header"
        );
        assert_eq!(at(head.right() + 1, rule_y), white);
        assert_eq!(at(x0, rule_y + 1), white, "1 px tall");

        // Rows hit by their 21 px rects, the heading by none.
        let mid = |r: Rect| (r.x + 30, r.y + 10);
        let (x, y) = mid(lay.rows[1]);
        assert_eq!(hit(&l, x, y, &theme), Hit::Row(Pick::App(1)));
        let (x, y) = mid(head);
        assert_eq!(hit(&l, x, y, &theme), Hit::Panel);
        let (x, y) = mid(lay.rows[3]);
        assert_eq!(hit(&l, x, y, &theme), Hit::Row(Pick::Found(0, 0)));

        // The reader's rows sit where they are drawn.
        let mut t = crate::access::Tree::window("", (W, 300));
        let focus = access(&l, &text, 11, &mut t, crate::access::ROOT, &theme);
        assert_eq!(t.get(focus).unwrap().label(), Some("Notion"));
        assert_eq!(
            t.get(focus).unwrap().bounds(),
            Some(crate::access::bounds(lit))
        );
    }

    /// At 200% under Classic and Blue the panel, field and list box rows double, the icon keeps 32 px
    /// on its grown centre, the bevel its 2 px, a heading's rule stays on its row's middle and a
    /// name stands twice as tall.
    #[test]
    fn under_classic_and_blue_the_panel_grows_with_the_text_size() {
        use crate::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        set_files(&mut l, files(&[("notes.txt", "~/Documents")]));
        let n = l.list.items.len() as i32;
        let heading = (0..l.list.items.len())
            .find(|&i| l.heading(i).is_some())
            .unwrap();
        let plain = (0..2).find(|&i| l.list.selected != Some(i)).unwrap();
        for iface in [Interface::Classic, Interface::Blue] {
            let base = crate::interfaces::theme_for(iface);
            let big = base.with_text_size(200);
            let small = layout(&l, &base);
            let lay = layout(&l, &big);
            let twice = |r: Rect| Rect::new(2 * r.x, 2 * r.y, 2 * r.width, 2 * r.height);
            assert_eq!(lay.width, 2 * W, "{iface:?}");
            assert_eq!(lay.field, twice(small.field), "{iface:?}");
            assert_eq!(lay.icon, Rect::new(32, 32, ICON, ICON), "{iface:?}");
            let row_h = 2 * listbox::ROW_H;
            let list = lay.list.unwrap();
            assert_eq!(list, Rect::new(16, 2 * HEAD_H, 2 * W - 32, n * row_h + 4));
            assert_eq!(lay.height, list.bottom() + 16, "{iface:?}");
            for i in 0..n as usize {
                let r = row_rect(&l, i, &big).unwrap();
                let want = Rect::new(
                    list.x + 2,
                    list.y + 2 + i as i32 * row_h,
                    list.width - 4,
                    row_h,
                );
                assert_eq!(r, want, "{iface:?} row {i}");
                let at = hit(&l, r.x + 40, r.y + row_h / 2, &big);
                assert_eq!(
                    at,
                    l.pick(i).map_or(Hit::Panel, Hit::Row),
                    "{iface:?} row {i}"
                );
            }

            let c = draw(&big, &text, &l, None, false, None);
            assert_eq!((c.width as i32, c.height as i32), (lay.width, lay.height));
            let rule = row_rect(&l, heading, &big).unwrap();
            let (x, y) = (rule.right() - 20, rule.y + row_h / 2 - 1);
            assert_ne!(
                c.get(x, y),
                c.get(x, y - 4),
                "{iface:?}: the rule on the row's middle"
            );

            let ink_rows = |c: &Canvas, r: Rect| {
                (r.y..r.bottom())
                    .filter(|&y| {
                        (r.x..r.right()).any(|x| {
                            let px = c.get(x, y);
                            [16, 8, 0].iter().all(|&sh| (px >> sh) & 0xFF < 0x60)
                        })
                    })
                    .count() as i32
            };
            let c100 = draw(&base, &text, &l, None, false, None);
            let at_100 = ink_rows(&c100, row_rect(&l, plain, &base).unwrap());
            let at_200 = ink_rows(&c, row_rect(&l, plain, &big).unwrap());
            assert!(
                at_100 > 0 && at_200 >= 2 * at_100 - 2,
                "{iface:?}: {at_200} ink rows at 200%, {at_100} at 100%"
            );
        }
    }

    /// At 200% the glass panel is laid out at 100% and grown: every rect doubles, the icon keeps
    /// 32 px on its grown centre, the lit box runs down its grown row and a name stands twice as tall.
    #[test]
    fn under_glass_the_panel_grows_with_the_text_size() {
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let big = glass.with_text_size(200);
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        set_files(&mut l, files(&[("notes.txt", "~/Documents")]));
        l.key(Key::Down);
        let small = layout(&l, &glass);
        let lay = layout(&l, &big);
        let twice = |r: Rect| Rect::new(2 * r.x, 2 * r.y, 2 * r.width, 2 * r.height);
        assert_eq!((lay.width, lay.height), (2 * W, 2 * small.height));
        assert_eq!(lay.field, twice(small.field));
        assert_eq!(lay.list, small.list.map(twice));
        let doubled: Vec<_> = small.rows.iter().map(|&r| twice(r)).collect();
        assert_eq!(lay.rows, doubled);
        assert_eq!(lay.icon, Rect::new(32, 32, ICON, ICON));

        let c = draw(&big, &text, &l, None, false, None);
        let m = panel::margin(&big);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (lay.width + m.left + m.right, lay.height + m.top + m.bottom)
        );
        let at = |x: i32, y: i32| c.get(m.left + x, m.top + y);
        let white = 0xFFFF_FFFF;
        let lit = lay.rows[1];
        assert_ne!(
            at(lit.x + 1, lit.bottom() - 4),
            white,
            "the box runs down the row"
        );
        assert_eq!(
            at(lit.x + 1, lay.rows[0].bottom() - 4),
            white,
            "Notepad is not lit"
        );

        let ink_rows = |c: &Canvas, m: crate::raster::Insets, r: Rect| {
            (r.y..r.bottom())
                .filter(|&y| {
                    (r.x..r.right()).any(|x| {
                        let px = c.get(m.left + x, m.top + y);
                        [16, 8, 0].iter().all(|&sh| (px >> sh) & 0xFF < 0x60)
                    })
                })
                .count() as i32
        };
        let c100 = draw(&glass, &text, &l, None, false, None);
        let at_100 = ink_rows(&c100, panel::margin(&glass), small.rows[0]);
        let at_200 = ink_rows(&c, m, lay.rows[0]);
        assert!(
            at_100 > 0 && at_200 >= 2 * at_100 - 2,
            "{at_200} ink rows at 200%, {at_100} at 100%"
        );
    }

    fn text_w(s: &str, px: u32) -> i32 {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        crate::look::caption_text(theme.look.style().unwrap()).measure(s, false, px)
    }

    #[test]
    fn the_highlighted_row_has_the_focus_and_the_heading_is_greyed() {
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let mut l = Launcher::new();
        let mut t = crate::access::Tree::window("", (W, 100));
        let focus = access(
            &l,
            &text,
            11,
            &mut t,
            crate::access::ROOT,
            &Theme::default(),
        );
        assert_eq!(
            t.get(focus).unwrap().role(),
            accesskit::Role::TextInput,
            "with no rows the field has the focus"
        );
        assert_eq!(t.get(focus).unwrap().label(), Some(SEARCH));
        typed(&mut l, "n");
        l.set_rows(rows(&["Notepad", "Notion"]));
        set_files(
            &mut l,
            vec![FoundRow {
                name: "notes.txt".into(),
                detail: "~".into(),
                icon: None,
            }],
        );
        let mut t = crate::access::Tree::window("", (W, 300));
        let focus = access(
            &l,
            &text,
            11,
            &mut t,
            crate::access::ROOT,
            &Theme::default(),
        );
        assert_eq!(t.get(focus).unwrap().label(), Some("Notepad"));
        let heading = t.find(accesskit::Role::ListBoxOption, FILES).unwrap();
        assert!(
            t.get(heading).unwrap().is_disabled(),
            "the heading is greyed"
        );
        let h = t.get(heading).unwrap();
        assert!(
            !h.supports_action(accesskit::Action::Click)
                && !h.supports_action(accesskit::Action::Focus),
            "the greyed heading offers no click or focus"
        );
    }
    use crate::raster::Rect;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    fn rows(names: &[&str]) -> Vec<Row> {
        names
            .iter()
            .map(|n| Row {
                name: n.to_string(),
                icon: None,
            })
            .collect()
    }

    fn typed(l: &mut Launcher, s: &str) -> Event {
        l.key(Key::Char(EditKey::Insert(s.into())))
    }

    #[test]
    fn refilled_icons_land_on_their_rows_and_keep_the_highlight() {
        let mut l = Launcher::new();
        typed(&mut l, "x");
        l.set_calc(rows(&["= 1"]).pop());
        l.set_settings(rows(&["Display"]));
        l.set_rows(rows(&["Xa", "Xb"]));
        l.key(Key::Down);
        let selected = l.list.selected;
        set_files(
            &mut l,
            vec![FoundRow {
                name: "x.txt".into(),
                detail: "~".into(),
                icon: None,
            }],
        );
        let refill = || Refill {
            calc: Some(Canvas::new(2, 2)),
            apps: vec![None, Some(Canvas::new(16, 16))],
            settings: vec![Some(Canvas::new(8, 8))],
            found: vec![vec![Some(Canvas::new(4, 4))]],
            ..Refill::default()
        };
        assert!(l.refill_icons(refill()));
        let widths: Vec<Option<u32>> = l
            .icons
            .iter()
            .map(|c| c.as_ref().map(|c| c.width))
            .collect();
        let at = |name: &str| l.list.items.iter().position(|n| n == name).unwrap();
        assert_eq!(widths[at("Xa")], None);
        assert_eq!(widths[at("Xb")], Some(16));
        assert_eq!(widths[at("Display")], Some(8));
        assert_eq!(widths[at("x.txt")], Some(4));
        assert_eq!(widths[at("= 1")], Some(2));
        assert_eq!(l.list.selected, selected);
        assert!(!l.refill_icons(refill()), "the same icons change nothing");
    }

    #[test]
    fn typing_completes_the_top_prefix_match_with_the_tail_selected() {
        let mut l = Launcher::new();
        assert_eq!(typed(&mut l, "n"), Event::QueryChanged);
        assert_eq!(l.query(), "n");
        l.set_rows(rows(&["Notepad", "Notion"]));
        assert_eq!(l.edit.text, "notepad");
        assert_eq!(l.edit.selection(), 1..7);
        assert_eq!(l.query(), "n");
        // Typing over the tail replaces it (the tail is the selection);
        // the query keeps growing in the casing actually typed.
        assert_eq!(typed(&mut l, "o"), Event::QueryChanged);
        assert_eq!(l.query(), "no");
        assert_eq!(l.edit.text, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        assert_eq!(l.edit.text, "notepad");
        assert_eq!(l.edit.selection(), 2..7);
        assert_eq!(l.selected(), 0);
    }

    #[test]
    fn a_fuzzy_only_top_row_is_listed_but_not_inlined() {
        let mut l = Launcher::new();
        typed(&mut l, "np");
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "np");
        assert!(!l.edit.has_selection());
        assert_eq!(l.list.items, vec!["Notepad".to_string()]);
    }

    #[test]
    fn backspace_eats_the_tail_and_holds_completion_off_until_the_next_char() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(
            l.key(Key::Char(EditKey::Backspace { word: false })),
            Event::QueryChanged
        );
        assert_eq!(l.edit.text, "no");
        assert_eq!(l.query(), "no");
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "no");
        typed(&mut l, "t");
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "notepad");
        assert_eq!(l.edit.selection(), 3..7);
    }

    #[test]
    fn down_and_up_move_the_highlight_without_touching_the_field() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        assert_eq!(l.key(Key::Down), Event::None);
        assert_eq!(l.selected(), 1);
        assert_eq!(l.edit.text, "notepad");
        assert_eq!(l.key(Key::Down), Event::None);
        assert_eq!(l.selected(), 1);
        assert_eq!(l.key(Key::Up), Event::None);
        assert_eq!(l.selected(), 0);
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::App(0)));
    }

    #[test]
    fn enter_launches_the_highlight_and_closes_on_no_rows() {
        let mut l = Launcher::new();
        assert_eq!(l.key(Key::Enter), Event::Close);
        assert_eq!(l.key(Key::Escape), Event::Close);
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        l.key(Key::Down);
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::App(1)));
    }

    /// What the host hands back for a query nothing matches.
    fn nothing_found(l: &mut Launcher) {
        l.set_calc(None);
        l.set_windows(Vec::new(), false);
        l.set_settings(Vec::new());
        l.set_rows(Vec::new());
        l.set_found(Vec::new());
    }

    #[test]
    fn a_query_with_no_match_shows_a_greyed_line_that_takes_no_highlight() {
        let mut l = Launcher::new();
        nothing_found(&mut l);
        assert!(l.list.items.is_empty(), "an empty field shows no list");
        typed(&mut l, "zzqx");
        nothing_found(&mut l);
        assert_eq!(l.list.items, [NO_RESULTS]);
        assert_eq!(l.list.greyed, [true]);
        assert_eq!(l.list.selected, None);
        l.key(Key::Down);
        assert_eq!(l.list.selected, None, "the arrows skip it");
        assert_eq!(l.pick(0), None);
        assert_eq!(l.key(Key::Menu), Event::None);
        assert_eq!(l.key(Key::Enter), Event::Close);
        l.set_found(vec![Section {
            heading: "Files".into(),
            rows: vec![FoundRow {
                name: "zzqx.txt".into(),
                detail: "~".into(),
                icon: None,
            }],
        }]);
        assert_eq!(l.list.items, ["Files", "zzqx.txt"], "found rows replace it");
    }

    #[test]
    fn the_no_match_line_pins_in_each_interface_at_scale_1_and_2() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "zzqx");
        nothing_found(&mut l);
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = Theme {
                    scale,
                    ..crate::interfaces::theme_for(i)
                };
                assert!(layout(&l, &theme).list.is_some(), "{i:?} shows the list");
                assert_eq!(hit_rows(&l, &theme), 0, "{i:?}: the line is not a row");
                let c = draw(&theme, &text, &l, None, false, None);
                got.push((format!("{i:?} {scale}"), fnv(&c)));
            }
        }
        crate::pins::check("controls/launcher_no_match", &got);
    }

    /// How many points down the list's middle hit a row.
    fn hit_rows(l: &Launcher, theme: &Theme) -> usize {
        let lay = layout(l, theme);
        let list = lay.list.expect("a list");
        (list.y..list.bottom())
            .filter(|&y| matches!(hit(l, list.x + list.width / 2, y, theme), Hit::Row(_)))
            .count()
    }

    #[test]
    fn new_rows_reset_the_highlight_and_cap_at_max_rows() {
        let mut l = Launcher::new();
        typed(&mut l, "a");
        let many: Vec<String> = (0..12).map(|i| format!("App {i}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        l.set_rows(rows(&refs));
        l.key(Key::Down);
        l.key(Key::Down);
        assert_eq!(l.selected(), 2);
        l.set_rows(rows(&["App 1"]));
        assert_eq!(l.selected(), 0);
        assert_eq!(l.list.items.len(), 1);
    }

    #[test]
    fn an_arrow_off_a_tail_lands_on_its_edge_instead_of_moving_further() {
        let mut l = Launcher::new();
        typed(&mut l, "n");
        l.set_rows(rows(&["Notepad"]));
        l.key(Key::Char(EditKey::Left {
            select: false,
            word: false,
        }));
        assert_eq!(typed(&mut l, "x"), Event::QueryChanged);
        assert_eq!(l.edit.text, "nxotepad");
        assert_eq!(l.query(), "nxotepad");
    }

    #[test]
    fn backspace_then_home_holds_completion_off_for_the_next_set_rows() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        l.key(Key::Char(EditKey::Backspace { word: false }));
        l.key(Key::Char(EditKey::Home { select: false }));
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "no");
        assert!(!l.edit.has_selection());
    }

    #[test]
    fn backspace_over_a_plain_selection_is_not_mistaken_for_a_completion_tail() {
        let mut l = Launcher::new();
        typed(&mut l, "note");
        l.key(Key::Char(EditKey::Left {
            select: false,
            word: false,
        }));
        l.key(Key::Char(EditKey::Left {
            select: false,
            word: false,
        }));
        assert_eq!(l.edit.caret(), 2);
        l.key(Key::Char(EditKey::End { select: true }));
        assert_eq!(l.edit.selection(), 2..4);
        assert_eq!(
            l.key(Key::Char(EditKey::Backspace { word: false })),
            Event::QueryChanged
        );
        assert_eq!(l.edit.text, "no");
    }

    #[test]
    fn paste_replaces_the_selection_and_updates_the_query() {
        let mut l = Launcher::new();
        typed(&mut l, "n");
        assert_eq!(l.paste("ot"), Event::QueryChanged);
        assert_eq!(l.query(), "not");
    }

    #[test]
    fn a_drop_replaces_the_query_and_a_shown_completion() {
        let mut l = Launcher::new();
        typed(&mut l, "n");
        l.inline.offer(&mut l.edit, "n", "");
        assert_eq!(l.drop_query("foot\nbar"), Event::QueryChanged);
        assert_eq!(
            (l.query(), l.edit.text.as_str(), l.inline.tail()),
            ("foot", "foot", None)
        );
        assert_eq!(l.drop_query("foot"), Event::None, "the same query");
        assert_eq!(l.drop_query(""), Event::None, "nothing to drop");
        assert_eq!(l.query(), "foot");
    }

    #[test]
    fn undo_over_a_shown_completion_drops_just_the_tail() {
        let mut l = Launcher::new();
        // A multi-byte char (Utf-8 'É', 2 bytes) straddles the byte offset
        // the tail is later dropped at, so a truncation that used the
        // wrong string for that offset would panic instead of just
        // misbehaving.
        typed(&mut l, "hxxxxÉyo");
        l.key(Key::Char(EditKey::Home { select: false }));
        for _ in 0..4 {
            l.key(Key::Char(EditKey::Right {
                select: true,
                word: false,
            }));
        }
        // Checkpoints the 9-byte "hxxxxÉyo" for Undo, then replaces the
        // 4-byte selection with "i".
        typed(&mut l, "i");
        assert_eq!(l.edit.text, "ixÉyo");
        l.set_rows(rows(&["ixÉyoZZZ"]));
        assert_eq!(l.edit.text, "ixÉyoZZZ");
        assert_eq!(l.key(Key::Char(EditKey::Undo)), Event::QueryChanged);
        assert_eq!(l.edit.text, "ixÉyo");
        assert!(!l.edit.has_selection());
        // Undo dropping the tail also holds completion off, same as
        // Backspace: the same rows come back unchanged instead of
        // re-completing to "ixÉyoZZZ".
        l.set_rows(rows(&["ixÉyoZZZ"]));
        assert_eq!(l.edit.text, "ixÉyo");
        assert!(!l.edit.has_selection());
    }

    #[test]
    fn undo_after_typing_over_completions_walks_back_the_typing_alone() {
        let mut l = Launcher::new();
        typed(&mut l, "f");
        l.set_rows(rows(&["Firefox"]));
        typed(&mut l, "i");
        l.set_rows(rows(&["Firefox"]));
        assert_eq!(l.edit.text, "firefox");
        l.key(Key::Char(EditKey::Undo));
        assert_eq!(l.edit.text, "fi", "the first undo drops the completion");
        l.key(Key::Char(EditKey::Undo));
        assert_eq!(l.edit.text, "", "the typing run is one step");
        l.key(Key::Char(EditKey::Redo));
        assert_eq!(l.edit.text, "fi");
    }

    #[test]
    fn a_navigation_key_off_a_tail_holds_completion_off_until_the_next_char() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "notepad");
        l.key(Key::Char(EditKey::SelectAll));
        assert_eq!(l.edit.text, "notepad", "the tail stays as typed text");
        assert_eq!(l.edit.selection(), 0..7);
        l.set_rows(rows(&["Notepad Plus"]));
        assert_eq!(l.edit.text, "notepad", "nothing completes on its own");
        assert_eq!(typed(&mut l, "x"), Event::QueryChanged);
        assert_eq!(l.edit.text, "x");
    }

    #[test]
    fn the_panel_is_a_raised_face_box_that_grows_a_list_with_the_rows() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        let c = draw(&theme, &text, &l, None, true, None);
        assert_eq!((c.width, c.height), (W as u32, HEAD_H as u32));
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(W - 1, HEAD_H - 1), cs.button_dk_shadow.argb());
        let lay = layout(&l, &theme);
        assert_eq!(lay.icon, Rect::new(8, 8, 32, 32));
        // The field: a sunken box (shadow at its top-left) filled window
        // colour, caret at the start of an empty field.
        assert_eq!(c.get(lay.field.x, lay.field.y), cs.button_shadow.argb());
        assert_eq!(c.get(lay.field.x + 10, lay.field.y + 10), cs.window.argb());
        assert_eq!(lay.list, None);
        assert_eq!(hit(&l, 100, 20, &Theme::default()), Hit::Field);
        assert_eq!(hit(&l, 4, 4, &Theme::default()), Hit::Panel);

        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion"]));
        let lay = layout(&l, &theme);
        let list = lay.list.unwrap();
        assert_eq!(
            list,
            Rect::new(8, HEAD_H, W - 16, 2 * crate::controls::listbox::ROW_H + 4)
        );
        assert_eq!(lay.height, HEAD_H + list.height + 8);
        let c = draw(&theme, &text, &l, None, false, None);
        assert_eq!(c.height, lay.height as u32);
        // The first row is highlighted at its text, the second is not.
        let row0 = Rect::new(
            list.x + 2 + 22,
            list.y + 2,
            10,
            crate::controls::listbox::ROW_H,
        );
        assert_eq!(c.get(row0.x, row0.y + 2), cs.hilight.argb());
        assert_eq!(
            c.get(row0.x, row0.y + crate::controls::listbox::ROW_H + 2),
            cs.window.argb()
        );
        assert_eq!(
            hit(&l, list.x + 30, list.y + 3, &Theme::default()),
            Hit::Row(Pick::App(0))
        );
        assert_eq!(
            hit(
                &l,
                list.x + 30,
                list.y + 3 + crate::controls::listbox::ROW_H,
                &Theme::default()
            ),
            Hit::Row(Pick::App(1))
        );
        // The completed tail sits on the highlight in the field.
        let c = draw(&theme, &text, &l, None, true, None);
        let tail_x = lay.field.x + 3 + text.measure("no", false, theme.fonts.size_px) + 2;
        assert_eq!(c.get(tail_x, lay.field.y + 10), cs.hilight.argb());
        // The raised edge closes under the list.
        assert_eq!(c.get(W - 1, lay.height - 1), cs.button_dk_shadow.argb());
    }

    #[test]
    #[ignore]
    fn render_launcher_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad", "Notion", "Gnome Terminal"]));
        set_files(
            &mut l,
            files(&[("notes.txt", "~/Documents"), ("nothing.md", "~")]),
        );
        crate::raster::write_preview(
            &draw(&theme, &text, &l, None, true, None),
            "launcher-preview.png",
        );
    }

    const FILES: &str = "Files";

    fn set_files(l: &mut Launcher, rows: Vec<FoundRow>) {
        l.set_found(vec![Section {
            heading: FILES.into(),
            rows,
        }]);
    }

    fn files(rows: &[(&str, &str)]) -> Vec<FoundRow> {
        rows.iter()
            .map(|(n, f)| FoundRow {
                name: n.to_string(),
                detail: f.to_string(),
                icon: None,
            })
            .collect()
    }

    #[test]
    fn files_hang_under_a_heading_the_arrows_step_over() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        set_files(
            &mut l,
            files(&[("notes.txt", "~/Documents"), ("nothing.md", "~")]),
        );
        assert_eq!(l.list.items, ["Notepad", FILES, "notes.txt", "nothing.md"]);
        assert_eq!(l.list.greyed, [false, true, false, false]);
        assert_eq!(l.list.detail[2], "~/Documents");
        assert_eq!(
            l.picked(),
            Some(Pick::App(0)),
            "files keep the app highlight"
        );
        l.key(Key::Down);
        assert_eq!(
            l.picked(),
            Some(Pick::Found(0, 0)),
            "Down steps over the heading"
        );
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::Found(0, 0)));
        assert_eq!(l.key(Key::CtrlEnter), Event::OpenContainer(0, 0));
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::App(0)), "Up steps back over it");
        assert_eq!(l.key(Key::CtrlEnter), Event::Launch(Pick::App(0)));
        assert_eq!(l.pick(1), None, "the heading is no row");
    }

    const CONTENTS: &str = "Contents";

    /// "no" typed: one program, a file found by name and two by their contents.
    fn with_two_sections() -> Launcher {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        l.set_found(vec![
            Section {
                heading: FILES.into(),
                rows: files(&[("notes.txt", "~/Documents")]),
            },
            Section {
                heading: CONTENTS.into(),
                rows: files(&[("minutes.pdf", "~/Documents"), ("plan.odt", "~")]),
            },
        ]);
        l
    }

    #[test]
    fn each_found_section_hangs_under_its_heading_and_its_rows_pick_by_section() {
        let mut l = with_two_sections();
        assert_eq!(
            l.list.items,
            [
                "Notepad",
                FILES,
                "notes.txt",
                CONTENTS,
                "minutes.pdf",
                "plan.odt"
            ]
        );
        assert_eq!(l.list.greyed, [false, true, false, true, false, false]);
        assert_eq!(l.list.detail[4], "~/Documents");
        l.key(Key::Down);
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::Found(1, 0)), "over both headings");
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::Found(1, 0)));
        assert_eq!(l.key(Key::CtrlEnter), Event::OpenContainer(1, 0));
        assert_eq!(l.key(Key::Menu), Event::Context(Pick::Found(1, 0)));
        let empty_names = |l: &mut Launcher| {
            l.set_found(vec![
                Section {
                    heading: FILES.into(),
                    rows: Vec::new(),
                },
                Section {
                    heading: CONTENTS.into(),
                    rows: files(&[("minutes.pdf", "~/Documents")]),
                },
            ])
        };
        empty_names(&mut l);
        assert_eq!(
            l.list.items,
            ["Notepad", CONTENTS, "minutes.pdf"],
            "an empty section shows no heading"
        );
        assert_eq!(
            l.pick(2),
            Some(Pick::Found(1, 0)),
            "the next keeps its number"
        );
    }

    #[test]
    fn a_section_landing_keeps_the_highlight_unless_its_own_row_changed() {
        let mut l = with_two_sections();
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::Found(0, 0)));
        let mut contents = with_two_sections().found;
        contents[1].rows.truncate(1);
        l.set_found(contents.clone());
        assert_eq!(
            l.picked(),
            Some(Pick::Found(0, 0)),
            "the contents changed, the file row stays lit"
        );
        contents[0].rows = files(&[("nothing.md", "~")]);
        l.set_found(contents);
        assert_eq!(
            l.picked(),
            Some(Pick::App(0)),
            "its row changed: back to the top"
        );
    }

    #[test]
    fn two_found_sections_pin_in_each_interface_at_scale_1_and_2() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = with_two_sections();
        l.key(Key::Down);
        l.key(Key::Down);
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = Theme {
                    scale,
                    ..crate::interfaces::theme_for(i)
                };
                let c = draw(&theme, &text, &l, None, false, None);
                let lay = layout(&l, &theme);
                let m = panel::margin(&theme);
                assert_eq!(
                    (c.width as i32, c.height as i32),
                    (
                        (lay.width + m.left + m.right) * scale,
                        (lay.height + m.top + m.bottom) * scale
                    ),
                    "{i:?} @{scale}"
                );
                got.push((format!("{i:?} {scale}"), fnv(&c)));
            }
        }
        crate::pins::check("controls/launcher_sections", &got);
    }

    /// Dark Glass's launcher at scale 1 and 2: the pane is dark Glass's window with no white on
    /// it, the lit row's box is blue, and its name is in dark Glass's text.
    #[test]
    fn under_dark_glass_the_pane_and_its_rows_go_dark() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let g = crate::glass::Settings {
            mode: crate::glass::Mode::Dark,
            ..crate::glass::Settings::default()
        };
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = with_two_sections();
        l.key(Key::Down);
        let lit = l.list.selected.expect("a lit row");
        let ink = crate::glass::DARK_TEXT;
        let mut got = Vec::new();
        for scale in [1, 2] {
            let theme = Theme {
                scale,
                ..crate::interfaces::theme_with(Interface::Glass, Default::default(), &g)
            };
            let window = theme.look.style().unwrap().colors.window.argb();
            let c = draw(&theme, &text, &l, None, false, None);
            let lay = layout(&l, &theme);
            let m = panel::margin(&theme);
            let w = c.width as i32;
            let dev = |r: Rect| {
                let r = Rect::new(m.left + r.x, m.top + r.y, r.width, r.height).scaled(scale);
                (r.y..r.bottom()).flat_map(move |y| (r.x..r.right()).map(move |x| (x, y)))
            };
            let px = |(x, y): (i32, i32)| c.pixels[(y * w + x) as usize];
            let pane = lay.list.expect("rows");
            let foot = Rect::new(pane.x + pane.width / 2, pane.bottom() - 4, 1, 1);
            assert_eq!(
                px(dev(foot).next().unwrap()),
                window,
                "the pane's foot at {scale}"
            );
            assert_eq!(
                dev(pane).filter(|&p| px(p) == 0xFFFF_FFFF).count(),
                0,
                "no white on the pane at {scale}"
            );
            let row = lay.rows[lit];
            let v = px(dev(Rect::new(row.right() - 4, row.y + 11, 1, 1))
                .next()
                .unwrap());
            let fill = crate::theme::Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            assert!(
                fill.b >= fill.r + 0x30,
                "the lit row is blue at {scale}, got {fill:?}"
            );
            let near = |v: u32| {
                [16, 8, 0]
                    .iter()
                    .all(|&sh| ((v >> sh) as u8).abs_diff((ink.argb() >> sh) as u8) <= 2)
            };
            assert!(
                dev(row).any(|p| near(px(p))),
                "the lit row's name in dark Glass's text at {scale}"
            );
            got.push((format!("{scale}"), fnv(&c)));
        }
        crate::pins::check("controls/launcher_dark", &got);
    }

    /// A program's section: its results' descriptions dim after their names, as a file's folder.
    #[test]
    fn a_programs_section_pins_in_each_interface_at_scale_1_and_2() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = with_two_sections();
        let mut found = l.found.clone();
        found.push(Section {
            heading: "Calculator".into(),
            rows: files(&[("14", "2 + 3 * 4")]),
        });
        l.set_found(found);
        assert_eq!(l.list.items[6..], ["Calculator", "14"]);
        assert_eq!(l.list.detail[7], "2 + 3 * 4");
        for _ in 0..5 {
            l.key(Key::Down);
        }
        assert_eq!(l.picked(), Some(Pick::Found(2, 0)));
        assert_eq!(l.key(Key::CtrlEnter), Event::OpenContainer(2, 0));
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = Theme {
                    scale,
                    ..crate::interfaces::theme_for(i)
                };
                let c = draw(&theme, &text, &l, None, false, None);
                got.push((format!("{i:?} {scale}"), fnv(&c)));
            }
        }
        crate::pins::check("controls/launcher_program_section", &got);
    }

    /// Eight programs, four settings and eight files: 22 lines.
    fn with_every_section() -> Launcher {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        let names: Vec<String> = (1..=8).map(|i| format!("Notes {i}")).collect();
        l.set_settings(rows(&["Notifications", "Network", "Mouse", "Sounds"]));
        l.set_rows(rows(&names.iter().map(String::as_str).collect::<Vec<_>>()));
        let names: Vec<String> = (1..=8).map(|i| format!("note{i}.txt")).collect();
        let found: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "~")).collect();
        set_files(&mut l, files(&found));
        assert_eq!(l.list.items.len(), 22);
        l
    }

    /// At 200% in the work areas of 720x450 and 960x540 screens, the panel narrows to the room
    /// and its list shows the lines that fit: the keys scroll the highlight into view (a group's
    /// first row with its heading), the wheel scrolls without moving it, and under Classic and
    /// Blue the list box's scrollbar arrow scrolls a line.
    #[test]
    fn in_a_short_work_area_the_panel_narrows_and_its_list_scrolls() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let big = crate::interfaces::theme_for(i).with_text_size(200);
            for room in [(720, 420), (960, 510)] {
                let at = format!("{i:?} {room:?}");
                let mut l = with_every_section();
                l.fit(room);
                settle(&mut l, &big);
                let lay = layout(&l, &big);
                assert_eq!(lay.width, (2 * W).min(room.0), "{at}");
                assert!(lay.height <= room.1, "{at}: {} tall", lay.height);
                let shown: Vec<usize> = (0..22)
                    .filter(|&k| row_rect(&l, k, &big).is_some())
                    .collect();
                let n = shown.len();
                assert!((6..22).contains(&n), "{at}: {n} lines show");
                assert_eq!(shown.first(), Some(&0), "{at}: from the top");
                let in_panel = |l: &Launcher, k: usize| {
                    let lay = layout(l, &big);
                    row_rect(l, k, &big).is_some_and(|r| r.y >= 0 && r.bottom() <= lay.height)
                };
                for _ in 0..19 {
                    l.key(Key::Down);
                }
                settle(&mut l, &big);
                assert_eq!(l.picked(), Some(Pick::Found(0, 7)), "{at}");
                assert!(in_panel(&l, 21), "{at}: the last row scrolled in");
                assert!(!in_panel(&l, 0), "{at}: the first scrolled out");
                if room == (720, 420) {
                    for scale in [1, 2] {
                        let theme = Theme {
                            scale,
                            ..big.clone()
                        };
                        let c = draw(&theme, &text, &l, None, false, None);
                        let m = panel::margin(&theme);
                        let size = (lay.width + m.left + m.right, lay.height + m.top + m.bottom);
                        let want = (size.0 * scale, size.1 * scale);
                        assert_eq!((c.width as i32, c.height as i32), want, "{at} @{scale}");
                        got.push((format!("{i:?} {scale}"), fnv(&c)));
                    }
                }
                let top = l.list.top;
                assert!(wheel(&mut l, &big, -1), "{at}");
                assert_eq!(l.list.top, top - 1, "{at}: a line up");
                assert!(wheel(&mut l, &big, 1), "{at}");
                assert_eq!(l.list.top, top, "{at}");
                assert!(
                    !wheel(&mut l, &big, 1),
                    "{at}: no further than the last line"
                );
                assert!(wheel(&mut l, &big, -100), "{at}");
                assert_eq!(l.list.top, 0, "{at}");
                settle(&mut l, &big);
                assert_eq!(
                    (l.list.top, l.picked()),
                    (0, Some(Pick::Found(0, 7))),
                    "{at}: kept"
                );
                l.key(Key::Down);
                settle(&mut l, &big);
                assert!(in_panel(&l, 21), "{at}: a key brings the highlight back");
                for _ in 0..7 {
                    l.key(Key::Up);
                }
                settle(&mut l, &big);
                assert_eq!(l.picked(), Some(Pick::Found(0, 0)), "{at}");
                assert!(
                    in_panel(&l, 13),
                    "{at}: the Files heading over its first row"
                );
                if i != Interface::Glass {
                    let list = layout(&l, &big).list.unwrap();
                    let (x, y) = (list.right() - 4, list.y + 6);
                    assert_eq!(hit(&l, x, y, &big), Hit::Bar(BarHit::Back), "{at}");
                    let top = l.list.top;
                    assert!(bar_press(&mut l, &big, BarHit::Back), "{at}");
                    assert_eq!(l.list.top, top - 1, "{at}");
                }
            }
        }
        crate::pins::check("controls/launcher_short_room", &got);
    }

    #[test]
    fn a_thumb_dragged_down_its_bar_scrolls_the_list_and_the_release_lets_go() {
        use crate::interface::Interface;
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let big = crate::interfaces::theme_for(i).with_text_size(200);
            for room in [(720, 420), (960, 510)] {
                let at = format!("{i:?} {room:?}");
                let mut l = with_every_section();
                l.fit(room);
                settle(&mut l, &big);
                let lay = layout(&l, &big);
                let sb = match lay.bar {
                    Some(sb) => sb,
                    None => {
                        let ll = listbox::layout(&l.list, lay.list.unwrap(), &controls(&big));
                        ll.scrollbar.expect("the list box's scrollbar")
                    }
                };
                let (x, y) = (sb.x + sb.width / 2, sb.y + sb.width + 2);
                let press = hit(&l, x, y, &big);
                assert!(
                    matches!(press, Hit::Bar(BarHit::Thumb(_))),
                    "{at}: {press:?}"
                );
                let Hit::Bar(b) = press else { unreachable!() };
                assert!(!bar_press(&mut l, &big, b), "{at}: the grab moves nothing");
                assert!(l.bar_held(), "{at}");
                assert!(
                    bar_drag(&mut l, &big, x - 300, sb.bottom() + 40),
                    "{at}: off the bar it still steers"
                );
                let last = last_top(&l, &big, &headings(&l));
                assert_eq!(l.list.top, last, "{at}: the last line shows");
                assert!(bar_drag(&mut l, &big, x, y), "{at}: back up");
                assert_eq!(l.list.top, 0, "{at}");
                assert!(l.bar_release(), "{at}");
                assert!(!bar_drag(&mut l, &big, x, sb.bottom()), "{at}: released");
                assert_eq!(l.list.top, 0, "{at}");
            }
        }
    }

    #[test]
    fn a_held_arrow_steps_on_the_hosts_ticks_while_the_pointer_is_on_it() {
        use crate::controls::scrollbar::{REPEAT_AFTER, REPEAT_EVERY};
        let big =
            crate::interfaces::theme_for(crate::interface::Interface::Classic).with_text_size(200);
        let mut l = with_every_section();
        l.fit((720, 420));
        settle(&mut l, &big);
        let list = layout(&l, &big).list.unwrap();
        let (x, y) = (list.right() - 4, list.bottom() - 6);
        assert_eq!(hit(&l, x, y, &big), Hit::Bar(BarHit::Forward));
        assert!(bar_press(&mut l, &big, BarHit::Forward));
        assert_eq!(l.list.top, 1);
        let t0 = std::time::Instant::now();
        assert!(!bar_tick(&mut l, &big, t0));
        bar_drag(&mut l, &big, 1, 1);
        assert!(!bar_tick(&mut l, &big, t0 + REPEAT_AFTER), "off the arrow");
        bar_drag(&mut l, &big, x, y);
        assert!(bar_tick(&mut l, &big, t0 + REPEAT_AFTER + REPEAT_EVERY));
        assert_eq!(l.list.top, 2);
        assert!(l.bar_release());
        assert!(!bar_tick(
            &mut l,
            &big,
            t0 + REPEAT_AFTER + 3 * REPEAT_EVERY
        ));
    }

    fn fnv(c: &Canvas) -> u64 {
        (0..c.height as i32)
            .flat_map(|y| (0..c.width as i32).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .fold(0xcbf2_9ce4_8422_2325, |h, w| {
                w.to_le_bytes()
                    .iter()
                    .fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
            })
    }

    #[test]
    fn with_no_app_matching_the_first_file_is_highlighted_and_up_stays_on_it() {
        let mut l = Launcher::new();
        typed(&mut l, "zq");
        l.set_rows(Vec::new());
        set_files(&mut l, files(&[("zq.txt", "~")]));
        assert_eq!(l.picked(), Some(Pick::Found(0, 0)));
        l.key(Key::Up);
        assert_eq!(
            l.picked(),
            Some(Pick::Found(0, 0)),
            "nothing above but the heading"
        );
        set_files(&mut l, Vec::new());
        assert_eq!(l.list.items, [NO_RESULTS], "no files, no heading");
        assert_eq!(l.key(Key::Enter), Event::Close);
    }

    #[test]
    fn the_heading_is_greyed_with_an_etched_rule_and_a_file_row_hits_as_a_file() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        set_files(&mut l, files(&[("notes.txt", "~/Documents")]));
        let lay = layout(&l, &theme);
        let list = lay.list.unwrap();
        assert_eq!(list.height, 3 * crate::controls::listbox::ROW_H + 4);
        let c = draw(&theme, &text, &l, None, false, None);
        // Row 1 is the heading: list inner y + 17, its middle at + 8.
        let inner_y = list.y + 2;
        let rule_y = inner_y + crate::controls::listbox::ROW_H + 7;
        let label_w = text.measure(FILES, false, theme.fonts.size_px);
        let x0 = list.x + 2 + crate::controls::listbox::TEXT_X + label_w + 6;
        assert_eq!(
            c.get(x0, rule_y),
            cs.button_shadow.argb(),
            "the rule's shadow"
        );
        assert_eq!(
            c.get(x0, rule_y + 1),
            cs.button_hilight.argb(),
            "over its hilight"
        );
        assert_eq!(
            c.get(x0 - 1, rule_y),
            cs.window.argb(),
            "starting 6 px past the label"
        );
        let heading = Rect::new(
            list.x + 2 + crate::controls::listbox::TEXT_X,
            inner_y + crate::controls::listbox::ROW_H,
            label_w,
            crate::controls::listbox::ROW_H,
        );
        let inks = (heading.y..heading.bottom())
            .any(|y| (heading.x..heading.right()).any(|x| c.get(x, y) == cs.gray_text.argb()));
        assert!(inks, "the heading inks in gray_text");
        assert_eq!(
            hit(&l, list.x + 30, heading.y + 3, &Theme::default()),
            Hit::Panel
        );
        assert_eq!(
            hit(
                &l,
                list.x + 30,
                heading.y + 3 + crate::controls::listbox::ROW_H,
                &Theme::default()
            ),
            Hit::Row(Pick::Found(0, 0))
        );
    }

    /// "2+3*4" typed: the answer, one app, a settings page and a file.
    fn with_every_kind() -> Launcher {
        let mut l = Launcher::new();
        typed(&mut l, "2+3*4");
        l.set_calc(rows(&["= 14"]).pop());
        l.set_settings(rows(&["Mouse"]));
        l.set_rows(rows(&["2048"]));
        set_files(&mut l, files(&[("2+3*4.txt", "~")]));
        l
    }

    #[test]
    fn the_answer_tops_the_list_and_settings_hang_over_the_files() {
        let mut l = with_every_kind();
        assert_eq!(
            l.list.items,
            [
                "= 14",
                "2048",
                SETTINGS_HEADING,
                "Mouse",
                FILES,
                "2+3*4.txt"
            ]
        );
        assert_eq!(l.list.greyed, [false, false, true, false, true, false]);
        assert_eq!(l.picked(), Some(Pick::Calc), "the answer is highlighted");
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::Calc));
        assert_eq!(l.edit.text, "2+3*4", "no completion under the answer");
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::App(0)));
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::Setting(0)), "over the heading");
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::Setting(0)));
        set_files(&mut l, files(&[("2+3*4.md", "~")]));
        assert_eq!(l.picked(), Some(Pick::Setting(0)), "new files keep it");
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::Found(0, 0)));
        l.key(Key::Up);
        l.key(Key::Up);
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::Calc));
    }

    #[test]
    fn windows_ranked_first_lead_under_their_heading_and_programs_get_one() {
        let mut l = Launcher::new();
        typed(&mut l, "fire");
        l.set_calc(None);
        l.set_windows(rows(&["Mozilla Firefox"]), true);
        l.set_settings(Vec::new());
        l.set_rows(rows(&["Firefox"]));
        assert_eq!(
            l.list.items,
            [
                WINDOWS_HEADING,
                "Mozilla Firefox",
                PROGRAMS_HEADING,
                "Firefox"
            ]
        );
        assert_eq!(l.list.greyed, [true, false, true, false]);
        assert_eq!(l.picked(), Some(Pick::Window(0)));
        assert_eq!(l.edit.text, "fire", "no completion while a window leads");
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::Window(0)));
        assert_eq!(l.key(Key::Menu), Event::None, "a window row has no menu");
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::App(0)), "over the Programs heading");
        l.key(Key::Up);
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::Window(0)), "the heading above stays");
    }

    #[test]
    fn windows_ranked_after_the_programs_hang_under_them_and_the_field_completes() {
        let mut l = Launcher::new();
        typed(&mut l, "te");
        l.set_calc(None);
        l.set_windows(rows(&["notes.txt - Editor"]), false);
        l.set_settings(rows(&["Keyboard"]));
        l.set_rows(rows(&["Terminal"]));
        assert_eq!(
            l.list.items,
            [
                "Terminal",
                WINDOWS_HEADING,
                "notes.txt - Editor",
                SETTINGS_HEADING,
                "Keyboard"
            ]
        );
        assert_eq!(l.picked(), Some(Pick::App(0)));
        assert_eq!(l.edit.text, "terminal", "completed in the typed casing");
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::Window(0)));
        l.set_windows(Vec::new(), true);
        assert!(
            !l.list
                .items
                .iter()
                .any(|i| i == WINDOWS_HEADING || i == PROGRAMS_HEADING),
            "no window rows, no headings for them"
        );
    }

    #[test]
    fn a_highlighted_row_is_picked_without_starting() {
        let mut l = with_every_kind();
        assert!(l.highlight(Pick::Setting(0)));
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        assert!(!l.highlight(Pick::Found(0, 3)), "no such row");
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
    }

    #[test]
    fn program_and_file_rows_open_a_context_menu_from_the_keys_or_a_right_press() {
        let mut l = with_every_kind();
        assert_eq!(l.key(Key::Menu), Event::None, "the answer has no menu");
        l.key(Key::Down);
        assert_eq!(l.key(Key::Menu), Event::Context(Pick::App(0)));
        assert_eq!(
            l.right_press(Pick::Found(0, 0)),
            Event::Context(Pick::Found(0, 0))
        );
        assert_eq!(
            l.picked(),
            Some(Pick::Found(0, 0)),
            "the pressed row is lit"
        );
        assert_eq!(l.right_press(Pick::Setting(0)), Event::None);
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        assert_eq!(l.right_press(Pick::Found(0, 3)), Event::None, "no such row");
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        assert_eq!(Launcher::new().key(Key::Menu), Event::None, "no rows");
    }

    #[test]
    fn a_rows_rect_is_where_it_hits() {
        for theme in [
            Theme::default(),
            crate::interfaces::theme_for(crate::interface::Interface::Glass),
        ] {
            let l = with_every_kind();
            for i in [0, 1, 3, 5] {
                let r = row_rect(&l, i, &theme).expect("a shown row");
                let hit = hit(&l, r.x + 30, r.y + r.height / 2, &theme);
                assert_eq!(hit, Hit::Row(l.pick(i).unwrap()), "line {i}");
            }
            assert_eq!(row_rect(&l, 6, &theme), None, "past the last line");
        }
    }

    #[test]
    fn settings_alone_highlight_their_first_row() {
        let mut l = Launcher::new();
        typed(&mut l, "wifi");
        l.set_calc(None);
        l.set_settings(rows(&["Network Connections"]));
        l.set_rows(Vec::new());
        assert_eq!(l.list.items, [SETTINGS_HEADING, "Network Connections"]);
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::Setting(0)), "only the heading above");
        assert_eq!(l.key(Key::CtrlEnter), Event::Launch(Pick::Setting(0)));
    }

    /// The list of `with_every_kind` in `theme`'s list box: the answer
    /// highlighted at its text, the Settings heading greyed with the etched
    /// rule after it, the rows hit as what they are.
    fn check_list_rows(theme: &Theme) {
        let p = theme.paint();
        let cs = p.cs;
        let text = TextRenderer::load(&FontConfig::default());
        let l = with_every_kind();
        let lay = layout(&l, theme);
        let list = lay.list.unwrap();
        let row_h = crate::controls::listbox::ROW_H;
        assert_eq!(list.height, 6 * row_h + 4);
        let c = draw(theme, &text, &l, None, false, None);
        let inner = (list.x + 2, list.y + 2);
        let text_x = inner.0 + crate::controls::listbox::TEXT_X;
        assert_eq!(
            c.get(text_x, inner.1 + 2),
            cs.hilight.argb(),
            "the answer is highlighted"
        );
        assert_eq!(c.get(text_x, inner.1 + row_h + 2), cs.window.argb());
        let rule_y = inner.1 + 2 * row_h + 7;
        let label_w = text.measure(SETTINGS_HEADING, false, theme.fonts.size_px);
        let x0 = text_x + label_w + 6;
        assert_eq!(c.get(x0, rule_y), cs.button_shadow.argb(), "the rule");
        assert_eq!(c.get(x0, rule_y + 1), cs.button_hilight.argb());
        assert_eq!(
            c.get(x0 - 1, rule_y),
            cs.window.argb(),
            "6 px past the label"
        );
        let heading = Rect::new(text_x, inner.1 + 2 * row_h, label_w, row_h);
        let inks = (heading.y..heading.bottom())
            .any(|y| (heading.x..heading.right()).any(|x| c.get(x, y) == cs.gray_text.argb()));
        assert!(inks, "the heading inks in gray_text");
        let hit_row = |k: i32| hit(&l, list.x + 30, inner.1 + k * row_h + 3, theme);
        assert_eq!(hit_row(0), Hit::Row(Pick::Calc));
        assert_eq!(hit_row(2), Hit::Panel, "the heading");
        assert_eq!(hit_row(3), Hit::Row(Pick::Setting(0)));
        assert_eq!(hit_row(5), Hit::Row(Pick::Found(0, 0)));
    }

    #[test]
    fn classic_draws_the_answer_and_the_settings_heading_as_list_rows() {
        check_list_rows(&Theme::default());
    }

    #[test]
    fn blue_draws_the_answer_and_the_settings_heading_as_list_rows() {
        check_list_rows(&crate::interfaces::theme_for(
            crate::interface::Interface::Blue,
        ));
    }

    #[test]
    fn under_glass_the_answer_is_a_result_and_settings_get_a_header() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let style = theme.look.style().unwrap();
        let text = TextRenderer::load(&FontConfig::default());
        let l = with_every_kind();
        let lay = layout(&l, &theme);
        let c = draw(&theme, &text, &l, None, false, None);
        let m = panel::margin(&theme);
        let at = |x: i32, y: i32| c.get(m.left + x, m.top + y);
        let white = 0xFFFF_FFFF;

        // The answer, then the app, each a 21 px result; the answer lit.
        assert_eq!(lay.rows[0], Rect::new(12, 52, 396, 21));
        assert_eq!(lay.rows[1], Rect::new(12, 73, 396, 21));
        let lit = lay.rows[0];
        let lit_px = |x, y| part_px(style, Part::StartLitResult, lit.width, 22, x, y);
        assert_eq!(at(lit.x, lit.y + 11), lit_px(0, 11), "the answer is lit");
        assert_eq!(at(lit.x + 300, lit.y), lit_px(300, 0));
        assert_eq!(at(lit.x, lay.rows[1].y + 11), white, "the app is not");

        // "Settings (1)" a group gap under the app, its rule after it.
        let head = lay.rows[2];
        assert_eq!(head, Rect::new(16, 94 + 11, 388, 21));
        let rule_y = head.y + search::at::HEAD_RULE_Y;
        let x0 = head.x + text_w("Settings (1)", search::HEADER_PX) + search::at::HEAD_RULE_GAP;
        let rule = part_px(style, Part::StartSearchRule, 1, 1, 0, 0);
        assert_eq!(at(x0, rule_y), rule, "the rule starts 8 px past the label");
        assert_eq!(at(x0 - 1, rule_y), white);
        assert_eq!(lay.rows[3], Rect::new(12, head.bottom(), 396, 21));

        // "Files (1)" a group gap under the settings row.
        let files_head = lay.rows[4];
        assert_eq!(files_head.y, lay.rows[3].bottom() + 11);
        let x0 = files_head.x + text_w("Files (1)", search::HEADER_PX) + search::at::HEAD_RULE_GAP;
        assert_eq!(at(x0, files_head.y + search::at::HEAD_RULE_Y), rule);

        let mid = |r: Rect| (r.x + 30, r.y + 10);
        let (x, y) = mid(lay.rows[3]);
        assert_eq!(hit(&l, x, y, &theme), Hit::Row(Pick::Setting(0)));
        let (x, y) = mid(head);
        assert_eq!(hit(&l, x, y, &theme), Hit::Panel);

        let mut t = crate::access::Tree::window("", (W, 300));
        let focus = access(&l, &text, 11, &mut t, crate::access::ROOT, &theme);
        assert_eq!(t.get(focus).unwrap().label(), Some("= 14"));
        for h in [SETTINGS_HEADING, FILES] {
            let n = t.find(accesskit::Role::ListBoxOption, h).unwrap();
            assert!(t.get(n).unwrap().is_disabled(), "{h} is greyed");
        }
    }
}
