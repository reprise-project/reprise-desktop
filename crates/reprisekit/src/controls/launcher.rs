//! The application launcher: a borderless raised panel with a 32 px icon, a
//! single-line field and a list box of matches under it. The field completes the
//! top match inline when it starts with what was typed, keeping the typed
//! prefix's casing, with the completed tail selected so the next key replaces it.
//! Over the app rows, when the text is a sum, the calculator's row with its
//! answer ("= 14"). Under them, a greyed "Settings" heading with an etched rule
//! over the settings rows, and once the host has found files, a "Files" heading
//! the same way with the folder dimmed after each name. Arrows step over headings.
//! The panel grows with the text size; the icons and bevels keep their size.
//!
//! Under a look with glass the panel is a piece of glass and the field and rows
//! are the Start menu's search mode's ([`crate::startpanel::search`]): its search
//! field with the italic cue and magnifier (the clear glyph once there is text),
//! under it a light pane of 21 px rows, a 16 px icon and the name, the
//! highlighted one in the lit result's box. Settings and files hang under
//! "Settings (n)" and "Files (n)" group headers with rules.
//!
//! Pure: the host works out the rows and hands them back through `set_calc`,
//! `set_settings`, `set_rows` and `set_files`.

use crate::access::accesskit;
use crate::classic::widgets::{ROW_H as FIELD_H, draw_edit, edit_first_visible};
use crate::controls::listbox;
use crate::controls::listbox::ListBox;
use crate::look::panel;
use crate::look::widgets::{Interaction, draw_edit_field};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::startpanel::layout::{GROUP_GAP, HEADER_H, RESULT_H};
use crate::startpanel::search;
use crate::style::{Part, State, Style};
use crate::text::TextRenderer;
use crate::textedit::{Caret, EditKey, Preedit, TextEdit};
use crate::theme::{TextUnits, Theme};

pub const MAX_ROWS: usize = 8;
pub const MAX_SETTINGS: usize = 4;
pub const MAX_FILES: usize = 8;
pub const SETTINGS_HEADING: &str = "Settings";
pub const FILES_HEADING: &str = "Files";
/// What a reader calls the field.
pub const SEARCH: &str = "Search";

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: String,
    pub icon: Option<Canvas>,
}

/// A file row: its name, the folder as the host words it (`~/Documents`), and its
/// 16 px type icon.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRow {
    pub name: String,
    pub folder: String,
    pub icon: Option<Canvas>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Calc,
    App(usize),
    Setting(usize),
    File(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Line {
    Row(Pick),
    Heading(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Launcher {
    /// Private because `tail` indexes into its text: every write goes through `key`
    /// or `paste`.
    edit: TextEdit,
    /// The calculator's row, the app names, then the settings and files each under
    /// its heading ([`Launcher::lines`]); the selection is never on a heading.
    pub list: ListBox,
    pub icons: Vec<Option<Canvas>>,
    calc: Option<Row>,
    apps: Vec<Row>,
    settings: Vec<Row>,
    files: Vec<FileRow>,
    /// Off after Backspace or Delete until the next typed char, so deleting the tail
    /// does not bring it straight back.
    completing: bool,
    /// The byte index in `edit.text` where a shown completion starts, `None` when
    /// nothing is completed. The only source of truth for "a completion is shown",
    /// never inferred from `edit`'s selection.
    tail: Option<usize>,
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
    /// `query()` changed: call `set_calc`, `set_settings` and `set_rows`,
    /// in that order (and `set_files` when the files come in).
    QueryChanged,
    Launch(Pick),
    OpenFolder(usize),
    /// Open the context menu of a program or file row, highlighted.
    Context(Pick),
    Close,
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
            apps: Vec::new(),
            settings: Vec::new(),
            files: Vec::new(),
            completing: true,
            tail: None,
        }
    }

    pub fn query(&self) -> &str {
        match self.tail {
            Some(i) => &self.edit.text[..i],
            None => &self.edit.text,
        }
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
        v.extend((0..self.apps.len()).map(|i| Line::Row(Pick::App(i))));
        if !self.settings.is_empty() {
            v.push(Line::Heading(SETTINGS_HEADING));
            v.extend((0..self.settings.len()).map(|i| Line::Row(Pick::Setting(i))));
        }
        if !self.files.is_empty() {
            v.push(Line::Heading(FILES_HEADING));
            v.extend((0..self.files.len()).map(|i| Line::Row(Pick::File(i))));
        }
        v
    }

    fn heading(&self, i: usize) -> Option<&'static str> {
        match self.lines().get(i) {
            Some(Line::Heading(h)) => Some(h),
            _ => None,
        }
    }

    fn heading_rows(&self, h: &str) -> usize {
        if h == SETTINGS_HEADING {
            self.settings.len()
        } else {
            self.files.len()
        }
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
        let on_heading = |l: &Self| l.list.selected.is_some_and(|i| l.heading(i).is_some());
        if on_heading(self) {
            self.list.step(delta, n);
            if on_heading(self) {
                self.list.selected = before;
            }
        }
    }

    /// Rebuild the list from the rows held, highlighting row `keep` if still a row,
    /// else the first.
    fn rebuild(&mut self, keep: Option<usize>) {
        let lines = self.lines();
        let (mut items, mut icons, mut greyed, mut detail) = (vec![], vec![], vec![], vec![]);
        for line in &lines {
            let (name, icon, folder) = match *line {
                Line::Heading(h) => (h.to_string(), None, String::new()),
                Line::Row(Pick::Calc) => {
                    let r = self.calc.as_ref().expect("a calc line has its row");
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
                Line::Row(Pick::File(i)) => {
                    let f = &self.files[i];
                    (f.name.clone(), f.icon.clone(), f.folder.clone())
                }
            };
            items.push(name);
            icons.push(icon);
            greyed.push(matches!(line, Line::Heading(_)));
            detail.push(folder);
        }
        let first = lines.iter().position(|l| matches!(l, Line::Row(_)));
        let selected = keep.filter(|&i| self.pick(i).is_some()).or(first);
        self.list = ListBox::new(items, selected);
        self.list.greyed = greyed;
        self.list.detail = detail;
        self.icons = icons;
    }

    /// The calculator's row for the current query (its answer, "= 14"), or None.
    /// Call before `set_rows`, which does not complete the field while it is shown.
    pub fn set_calc(&mut self, row: Option<Row>) {
        self.calc = row;
        self.rebuild(None);
    }

    pub fn set_settings(&mut self, rows: Vec<Row>) {
        self.settings = rows.into_iter().take(MAX_SETTINGS).collect();
        self.rebuild(None);
    }

    /// Gives the app and settings rows, in order, the icons the host found for them again (an
    /// icon file it decoded since), keeping the list as it is.
    pub fn refill_icons(&mut self, apps: Vec<Option<Canvas>>, settings: Vec<Option<Canvas>>) {
        for (r, icon) in self.apps.iter_mut().zip(apps) {
            r.icon = icon;
        }
        for (r, icon) in self.settings.iter_mut().zip(settings) {
            r.icon = icon;
        }
        for (k, line) in self.lines().into_iter().enumerate() {
            let icon = match line {
                Line::Row(Pick::App(i)) => self.apps[i].icon.clone(),
                Line::Row(Pick::Setting(i)) => self.settings[i].icon.clone(),
                _ => continue,
            };
            if let Some(slot) = self.icons.get_mut(k) {
                *slot = icon;
            }
        }
    }

    /// The host's file rows for the current query, best first. The highlight stays
    /// on a row above the files.
    pub fn set_files(&mut self, files: Vec<FileRow>) {
        let files_at = self
            .lines()
            .iter()
            .position(|l| *l == Line::Heading(FILES_HEADING));
        let keep = self
            .list
            .selected
            .filter(|&i| files_at.is_none_or(|f| i < f));
        self.files = files.into_iter().take(MAX_FILES).collect();
        self.rebuild(keep);
    }

    fn drop_tail(&mut self, i: usize) {
        self.edit.text.truncate(i);
        let (anchor, caret) = (self.edit.anchor(), self.edit.caret());
        self.edit.set_selection(anchor, caret);
    }

    pub fn key(&mut self, k: Key) -> Event {
        match k {
            Key::Char(ek) => {
                let before = self.query().to_string();
                let tail = self.tail;
                let is_insert = matches!(ek, EditKey::Insert(_));
                let eats = matches!(
                    ek,
                    EditKey::Backspace { .. } | EditKey::Delete { .. } | EditKey::Cut
                );
                // Keys that neither insert nor delete drop the tail; any
                // non-insert disables completion until the next Insert.
                let drops_tail = tail.is_some() && !is_insert && !eats;
                if let Some(i) = tail {
                    let moves = matches!(
                        ek,
                        EditKey::Left { .. }
                            | EditKey::Right { .. }
                            | EditKey::Home { .. }
                            | EditKey::End { .. }
                            | EditKey::SelectAll
                            | EditKey::Copy
                    );
                    if moves {
                        self.edit.key(ek);
                        self.drop_tail(i);
                    } else {
                        // The completion was never typed, so it goes before any edit lands and
                        // the edit's undo log holds typing alone. Undo, Backspace, Delete and Cut
                        // over it take just the completion.
                        self.drop_tail(i);
                        if !eats && !matches!(ek, EditKey::Undo) {
                            self.edit.key(ek);
                        }
                    }
                    self.tail = None;
                } else {
                    self.edit.key(ek);
                }
                self.completing = if is_insert {
                    true
                } else if eats || drops_tail {
                    false
                } else {
                    self.completing
                };
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
                Some(Pick::File(i)) => Event::OpenFolder(i),
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
        true
    }

    fn context_of(p: Pick) -> Event {
        match p {
            Pick::App(_) | Pick::File(_) => Event::Context(p),
            Pick::Calc | Pick::Setting(_) => Event::None,
        }
    }

    /// A host's paste: replaces a shown completion as a typed insert would.
    pub fn paste(&mut self, text: &str) -> Event {
        let before = self.query().to_string();
        if let Some(i) = self.tail.take() {
            self.drop_tail(i);
        }
        self.edit.paste(text, &[]);
        self.completing = true;
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
        self.tail = None;
        self.completing = true;
        match self.query() == before {
            true => Event::None,
            false => Event::QueryChanged,
        }
    }

    /// The host's answer to `QueryChanged`: the ranked app rows for `query()`. File
    /// rows stay until `set_files`. Completes the field from the first app row when
    /// it starts with the query (case-insensitively), keeping the query's casing;
    /// never under the calculator's row.
    pub fn set_rows(&mut self, rows: Vec<Row>) {
        self.apps = rows.into_iter().take(MAX_ROWS).collect();
        self.rebuild(None);
        let query = self.query().to_string();
        let top = self.apps.first().map(|r| r.name.clone());
        let completes = self.completing
            && self.calc.is_none()
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
            self.edit.text = format!("{query}{}", &t[split..]);
            let end = self.edit.text.len();
            self.edit.set_selection(query.len(), end);
            self.tail = Some(query.len());
        } else if let Some(i) = self.tail {
            // A stale completion with no matching top row: back to the query.
            self.drop_tail(i);
            self.tail = None;
        }
    }
}

pub const W: i32 = 420;
pub const HEAD_H: i32 = 48;
pub const ICON: i32 = 32;
const PAD: i32 = 8;
/// Between the heading's label and its rule, and the rule and the list's
/// right edge.
const RULE_GAP: i32 = 6;
/// Under a look with glass: from the pane's edge to the rows' boxes, and
/// from a row's box to the Files header, which starts over the icons.
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
    /// Under a look with glass, each list row's rect: an app or file row,
    /// for the heading its group header from the capitals' top. Empty
    /// otherwise, where the list box lays its rows out.
    pub rows: Vec<Rect>,
    pub width: i32,
    pub height: i32,
}

pub fn layout(l: &Launcher, theme: &Theme) -> Layout {
    let u = theme.text;
    if panel::glass(theme).is_some() {
        return glass_layout(l, u);
    }
    let field_x = PAD + ICON + PAD;
    let field = Rect::new(field_x, (HEAD_H - FIELD_H) / 2, W - field_x - PAD, FIELD_H);
    // The list box's bevel keeps its 2 px and its rows their grown height, so it
    // holds every row whole.
    let list = (!l.list.items.is_empty()).then(|| {
        let row_h = controls(theme).list_row;
        let (x, right) = (u.len(PAD), u.len(W - PAD));
        let h = l.list.items.len() as i32 * row_h + 4;
        Rect::new(x, u.len(HEAD_H), right - x, h)
    });
    Layout {
        icon: icon_rect(u),
        field: u.rect(field),
        list,
        rows: Vec::new(),
        width: u.len(W),
        height: list.map_or(u.len(HEAD_H), |r| r.bottom() + u.len(PAD)),
    }
}

/// The look's control sizes at the theme's text size, as the list box lays its rows out.
fn controls(theme: &Theme) -> crate::controls::metrics::ControlMetrics {
    theme.look.controls().grown(theme.text)
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

/// The glass panel at 100% text with every edge grown by `u`; the 32 px icon
/// keeps its size, its centre moving with the rest.
fn glass_layout(l: &Launcher, u: TextUnits) -> Layout {
    let field_x = PAD + ICON + PAD;
    let field_h = search::FIELD.1;
    let field = Rect::new(field_x, (HEAD_H - field_h) / 2, W - field_x - PAD, field_h);
    let n = l.list.items.len();
    let (x, w) = (PAD + PANE_PAD, W - 2 * (PAD + PANE_PAD));
    let mut y = HEAD_H + PANE_PAD;
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        if l.heading(i).is_some() {
            if i > 0 {
                y += GROUP_GAP;
            }
            rows.push(Rect::new(x + HEADER_IN, y, w - 2 * HEADER_IN, HEADER_H));
            y += HEADER_H;
        } else {
            rows.push(Rect::new(x, y, w, RESULT_H));
            y += RESULT_H;
        }
    }
    // The last lit box runs a row past its 21 px.
    let bottom = y + 1 + PANE_PAD;
    let list = (n > 0).then(|| Rect::new(PAD, HEAD_H, W - 2 * PAD, bottom - HEAD_H));
    Layout {
        icon: icon_rect(u),
        field: u.rect(field),
        list: list.map(|r| u.rect(r)),
        rows: rows.into_iter().map(|r| u.rect(r)).collect(),
        width: u.len(W),
        height: u.len(list.map_or(HEAD_H, |r| r.bottom() + PAD)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Row(Pick),
    Field,
    Panel,
}

pub fn hit(l: &Launcher, x: i32, y: i32, theme: &Theme) -> Hit {
    let lay = layout(l, theme);
    if lay.field.contains(x, y) {
        return Hit::Field;
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
        if let Some(listbox::Hit::Row(i)) = listbox::hit(&l.list, &ll, x, y)
            && let Some(p) = l.pick(i)
        {
            return Hit::Row(p);
        }
    }
    Hit::Panel
}

/// Line `i`'s rect in panel coordinates, where a menu opened from the keys
/// hangs.
pub fn row_rect(l: &Launcher, i: usize, theme: &Theme) -> Option<Rect> {
    let lay = layout(l, theme);
    if !lay.rows.is_empty() {
        return lay.rows.get(i).copied();
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
            glass_body(c, style, l, &lay, icon, caret, g)
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
    let size = g.px(search::TEXT_PX);
    for (i, r) in lay.rows.iter().enumerate() {
        let r = r.scaled(s);
        if let Some(h) = l.heading(i) {
            let title = format!("{h} ({})", l.heading_rows(h));
            search::header(c, style, &face, r, pane, &title, g);
            continue;
        }
        let (name, lit) = (&l.list.items[i], l.list.selected == Some(i));
        let icon = l.icons.get(i).and_then(Option::as_ref);
        let end = search::result(c, style, &face, r, icon, name, lit, g);
        if let Some(d) = l.list.detail.get(i).filter(|d| !d.is_empty()) {
            let x = end + g.len(listbox::DETAIL_GAP);
            let y = r.y + g.len(search::at::RESULT_BASE);
            let clip = Rect::new(r.x, r.y, r.width - 2 * s, r.height);
            let ink = search::ink(search::INK_GREYED);
            search::line(c, &face, x, y, clip, d, ink, size);
        }
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
    draw_edit(
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
        for (h, label) in (0..l.list.items.len()).filter_map(|i| Some((i, l.heading(i)?))) {
            // From past the heading's label to the list's right edge, on
            // the row's middle.
            let label_w = text.measure(label, false, px) / s;
            let x0 = ll.inner.x + listbox::TEXT_X + label_w + RULE_GAP;
            let y = ll.inner.y + h as i32 * ll.row_h + ll.row_h / 2 - 1;
            c.etched_hline_t(x0 * s, (ll.inner.right() - RULE_GAP) * s, y * s, cs, s);
        }
    }
}

/// The panel's `access` part: field and rows (see `listbox::access_filtered`),
/// headings greyed; under glass each row takes its own rect. Returns the focused
/// node: the highlighted row, else the field.
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
        if l.heading(i).is_some()
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
        l.set_files(files(&[("notes.txt", "~/Documents")]));
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
        assert_eq!(hit(&l, x, y, &theme), Hit::Row(Pick::File(0)));

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
        l.set_files(files(&[("notes.txt", "~/Documents")]));
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
        l.set_files(files(&[("notes.txt", "~/Documents")]));
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
        l.set_files(vec![FileRow {
            name: "notes.txt".into(),
            folder: "~".into(),
            icon: None,
        }]);
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
        let heading = t
            .find(accesskit::Role::ListBoxOption, FILES_HEADING)
            .unwrap();
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
        l.set_settings(rows(&["Display"]));
        l.set_rows(rows(&["Xa", "Xb"]));
        l.key(Key::Down);
        let selected = l.list.selected;
        l.refill_icons(
            vec![None, Some(Canvas::new(16, 16))],
            vec![Some(Canvas::new(8, 8))],
        );
        let widths: Vec<Option<u32>> = l
            .icons
            .iter()
            .map(|c| c.as_ref().map(|c| c.width))
            .collect();
        let at = |name: &str| l.list.items.iter().position(|n| n == name).unwrap();
        assert_eq!(widths[at("Xa")], None);
        assert_eq!(widths[at("Xb")], Some(16));
        assert_eq!(widths[at("Display")], Some(8));
        assert_eq!(l.list.selected, selected);
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
        assert_eq!(l.edit.text, "nx");
        assert_eq!(l.query(), "nx");
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
        l.tail = Some(1);
        assert_eq!(l.drop_query("foot\nbar"), Event::QueryChanged);
        assert_eq!(
            (l.query(), l.edit.text.as_str(), l.tail),
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
        // SelectAll drops the tail like Backspace does, so re-ranking the
        // same query on the next `set_rows` does not complete it right
        // back to "notepad".
        l.key(Key::Char(EditKey::SelectAll));
        assert_eq!(l.edit.text, "no");
        assert_eq!(l.edit.selection(), 0..2);
        l.set_rows(rows(&["Notepad"]));
        assert_eq!(l.edit.text, "no");
        assert_eq!(l.edit.selection(), 0..2);
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
        l.set_files(files(&[("notes.txt", "~/Documents"), ("nothing.md", "~")]));
        crate::raster::write_preview(
            &draw(&theme, &text, &l, None, true, None),
            "launcher-preview.png",
        );
    }

    fn files(rows: &[(&str, &str)]) -> Vec<FileRow> {
        rows.iter()
            .map(|(n, f)| FileRow {
                name: n.to_string(),
                folder: f.to_string(),
                icon: None,
            })
            .collect()
    }

    #[test]
    fn files_hang_under_a_heading_the_arrows_step_over() {
        let mut l = Launcher::new();
        typed(&mut l, "no");
        l.set_rows(rows(&["Notepad"]));
        l.set_files(files(&[("notes.txt", "~/Documents"), ("nothing.md", "~")]));
        assert_eq!(
            l.list.items,
            ["Notepad", FILES_HEADING, "notes.txt", "nothing.md"]
        );
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
            Some(Pick::File(0)),
            "Down steps over the heading"
        );
        assert_eq!(l.key(Key::Enter), Event::Launch(Pick::File(0)));
        assert_eq!(l.key(Key::CtrlEnter), Event::OpenFolder(0));
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::App(0)), "Up steps back over it");
        assert_eq!(l.key(Key::CtrlEnter), Event::Launch(Pick::App(0)));
        assert_eq!(l.pick(1), None, "the heading is no row");
    }

    #[test]
    fn with_no_app_matching_the_first_file_is_highlighted_and_up_stays_on_it() {
        let mut l = Launcher::new();
        typed(&mut l, "zq");
        l.set_rows(Vec::new());
        l.set_files(files(&[("zq.txt", "~")]));
        assert_eq!(l.picked(), Some(Pick::File(0)));
        l.key(Key::Up);
        assert_eq!(
            l.picked(),
            Some(Pick::File(0)),
            "nothing above but the heading"
        );
        l.set_files(Vec::new());
        assert!(l.list.items.is_empty(), "no files, no heading");
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
        l.set_files(files(&[("notes.txt", "~/Documents")]));
        let lay = layout(&l, &theme);
        let list = lay.list.unwrap();
        assert_eq!(list.height, 3 * crate::controls::listbox::ROW_H + 4);
        let c = draw(&theme, &text, &l, None, false, None);
        // Row 1 is the heading: list inner y + 17, its middle at + 8.
        let inner_y = list.y + 2;
        let rule_y = inner_y + crate::controls::listbox::ROW_H + 7;
        let label_w = text.measure(FILES_HEADING, false, theme.fonts.size_px);
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
            Hit::Row(Pick::File(0))
        );
    }

    /// "2+3*4" typed: the answer, one app, a settings page and a file.
    fn with_every_kind() -> Launcher {
        let mut l = Launcher::new();
        typed(&mut l, "2+3*4");
        l.set_calc(rows(&["= 14"]).pop());
        l.set_settings(rows(&["Mouse"]));
        l.set_rows(rows(&["2048"]));
        l.set_files(files(&[("2+3*4.txt", "~")]));
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
                FILES_HEADING,
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
        l.set_files(files(&[("2+3*4.md", "~")]));
        assert_eq!(l.picked(), Some(Pick::Setting(0)), "new files keep it");
        l.key(Key::Down);
        assert_eq!(l.picked(), Some(Pick::File(0)));
        l.key(Key::Up);
        l.key(Key::Up);
        l.key(Key::Up);
        assert_eq!(l.picked(), Some(Pick::Calc));
    }

    #[test]
    fn a_highlighted_row_is_picked_without_starting() {
        let mut l = with_every_kind();
        assert!(l.highlight(Pick::Setting(0)));
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        assert!(!l.highlight(Pick::File(3)), "no such row");
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
    }

    #[test]
    fn program_and_file_rows_open_a_context_menu_from_the_keys_or_a_right_press() {
        let mut l = with_every_kind();
        assert_eq!(l.key(Key::Menu), Event::None, "the answer has no menu");
        l.key(Key::Down);
        assert_eq!(l.key(Key::Menu), Event::Context(Pick::App(0)));
        assert_eq!(l.right_press(Pick::File(0)), Event::Context(Pick::File(0)));
        assert_eq!(l.picked(), Some(Pick::File(0)), "the pressed row is lit");
        assert_eq!(l.right_press(Pick::Setting(0)), Event::None);
        assert_eq!(l.picked(), Some(Pick::Setting(0)));
        assert_eq!(l.right_press(Pick::File(3)), Event::None, "no such row");
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
        assert_eq!(hit_row(5), Hit::Row(Pick::File(0)));
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
        for h in [SETTINGS_HEADING, FILES_HEADING] {
            let n = t.find(accesskit::Role::ListBoxOption, h).unwrap();
            assert!(t.get(n).unwrap().is_disabled(), "{h} is greyed");
        }
    }
}
