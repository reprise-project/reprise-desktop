//! The notification history popup the tray's bell opens: a "Do not disturb"
//! check box, the notifications grouped per program in a list, and a Clear All
//! button, on the look's flyout panel ([`crate::flyout`]). The host fills the
//! entries, says which it can still act on, and decides what a pick does.
//!
//! - **Panel**: 380 px wide, 8 px padding, a `WindowRaised` edge; inside
//!   a wider flyout border the panel grows by the difference each side.
//! - **Check box**: the look's box (13 px) centred in its 17 px row, its
//!   label the look's offset in from the box's left edge (18 px in
//!   Classic, 16 in Blue).
//! - **List**: a [`grouplist`] 6 px under the check row, ten 18 px rows
//!   tall. Each program heads its group: its icon, its name and how many
//!   it sent, an arrow that points down while the group is open and
//!   right while it is folded, and a 44x16 Clear button at the row's
//!   right end. The group's notifications follow, newest first, the time
//!   each came right-aligned in grey. The group with the newest
//!   notification comes first.
//! - **Clear All**: a 75x23 push button 8 px under the list, greyed with
//!   nothing to clear.

use std::collections::BTreeSet;
use std::rc::Rc;

use crate::classic::widgets::{BUTTON_H, BUTTON_W, draw_arrow_h, draw_arrow_v, draw_label};
use crate::controls::grouplist::{self, GroupList, ROW_H, Row, Shown};
use crate::controls::metrics::ControlMetrics;
use crate::flyout;
use crate::look::widgets::{draw_button, draw_checkbox, draw_disabled_label};
use crate::pointer::{Interaction, Pointer};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

pub const W: i32 = 380;
const PAD: i32 = 8;
const CHECK_ROW: i32 = 17;
const LIST_GAP: i32 = 6;
pub const ROWS: i32 = 10;
/// A group's Clear button: 44x16, 2 px in from the row's right end and
/// 1 px down; the fold arrow's 11 px cell 3 px left of it.
const GROUP_CLEAR_W: i32 = 44;
const GROUP_CLEAR_H: i32 = ROW_H - 2;
const GROUP_CLEAR_INSET: i32 = 2;
const ARROW_CELL: i32 = 11;
const ARROW_GAP: i32 = 3;
const GROUP_TRAIL: i32 = GROUP_CLEAR_INSET + GROUP_CLEAR_W + ARROW_GAP + ARROW_CELL + 4;
/// A notification's time: right-aligned 4 px in from the row's end, in a
/// 60 px column its message stays clear of.
const TIME_W: i32 = 60;
const TIME_INSET: i32 = 4;
const DND_LABEL: &str = "Do not disturb";
const CLEAR_LABEL: &str = "Clear All";
const GROUP_CLEAR_LABEL: &str = "Clear";
const EMPTY_LABEL: &str = "No notifications";
/// A group's name when the program gave none.
const NO_APP: &str = "Other";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: u32,
    pub app: String,
    /// The newest one's heads its group.
    pub icon: Option<Rc<Canvas>>,
    pub message: String,
    pub time: String,
    pub live: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Meta {
    Header { app: String, open: bool },
    Note { id: u32, time: String, live: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct History {
    pub dnd: bool,
    pub list: GroupList<u32>,
    meta: Vec<Meta>,
    icons: Vec<Option<Rc<Canvas>>>,
    entries: Vec<Entry>,
    folded: BTreeSet<String>,
    /// The check box, Clear All or a group's Clear the pointer holds down.
    pointer: Pointer<Held>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Held {
    Dnd,
    Clear,
    GroupClear(usize),
}

impl Held {
    fn of(hit: Hit) -> Option<Held> {
        match hit {
            Hit::Dnd => Some(Held::Dnd),
            Hit::Clear => Some(Held::Clear),
            Hit::GroupClear(i) => Some(Held::GroupClear(i)),
            _ => None,
        }
    }
}

impl Default for History {
    fn default() -> Self {
        History::new()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub check: Rect,
    pub label: Rect,
    pub list: Rect,
    pub clear: Rect,
    pub height: i32,
}

/// The panel's width in a look with controls `cm`: [`W`], and as much again as a
/// wider flyout border takes on each side.
pub fn width(cm: &ControlMetrics) -> i32 {
    W + 2 * (cm.flyout_border - flyout::EDGE)
}

/// The panel's parts at control sizes `cm`; inside a wider flyout border
/// everything moves in by the difference.
pub fn layout(cm: &ControlMetrics) -> Layout {
    let pad = PAD + cm.flyout_border - flyout::EDGE;
    let w = width(cm);
    let check = Rect::new(pad, pad + (CHECK_ROW - cm.check) / 2, cm.check, cm.check);
    let label_x = check.x + cm.check_label;
    let label = Rect::new(label_x, pad, 100, CHECK_ROW);
    let list_h = 2 + ROWS * ROW_H + 2;
    let list = Rect::new(pad, pad + CHECK_ROW + LIST_GAP, w - 2 * pad, list_h);
    let clear = Rect::new(w - pad - BUTTON_W, list.bottom() + PAD, BUTTON_W, BUTTON_H);
    Layout {
        check,
        label,
        list,
        clear,
        height: clear.bottom() + pad,
    }
}

fn header_parts(row: Rect) -> (Rect, Rect) {
    let clear = Rect::new(
        row.right() - GROUP_CLEAR_INSET - GROUP_CLEAR_W,
        row.y + 1,
        GROUP_CLEAR_W,
        GROUP_CLEAR_H,
    );
    let arrow = Rect::new(clear.x - ARROW_GAP - ARROW_CELL, row.y, ARROW_CELL, ROW_H);
    (clear, arrow)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Dnd,
    List(grouplist::Hit),
    GroupClear(usize),
    Clear,
    Panel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    /// Left folds the group, from its header or any of its rows; Right
    /// opens a folded one.
    Left,
    Right,
    Enter,
    Delete,
    Escape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Dnd,
    Clear,
    /// A group's Clear was released over, or Delete pressed on its
    /// header: the host dismisses every notification from that program.
    ClearGroup(String),
    Dismiss(u32),
    Invoke(u32),
    Close,
}

impl History {
    pub fn new() -> History {
        History {
            dnd: false,
            list: GroupList::new(Vec::new()),
            meta: Vec::new(),
            icons: Vec::new(),
            entries: Vec::new(),
            folded: BTreeSet::new(),
            pointer: Pointer::new(),
        }
    }

    /// New entries, newest first. The selection stays on its header or notification
    /// while it is still there, the scroll inside the new rows, folded groups folded.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.entries = entries;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let picked = self.list.selected.and_then(|i| self.meta.get(i).cloned());
        let mut apps: Vec<&str> = Vec::new();
        for e in &self.entries {
            if !apps.contains(&e.app.as_str()) {
                apps.push(&e.app);
            }
        }
        let (mut rows, mut meta, mut icons, mut trail) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for app in apps {
            let notes: Vec<&Entry> = self.entries.iter().filter(|e| e.app == app).collect();
            let open = !self.folded.contains(app);
            let name = if app.is_empty() { NO_APP } else { app };
            rows.push(Row::Group(format!("{name} ({})", notes.len())));
            meta.push(Meta::Header {
                app: app.to_string(),
                open,
            });
            icons.push(notes.first().and_then(|e| e.icon.clone()));
            trail.push(GROUP_TRAIL);
            if !open {
                continue;
            }
            for e in notes {
                rows.push(Row::Item {
                    name: e.message.clone(),
                    data: e.id,
                });
                meta.push(Meta::Note {
                    id: e.id,
                    time: e.time.clone(),
                    live: e.live,
                });
                icons.push(None);
                trail.push(TIME_W);
            }
        }
        let n = rows.len();
        let top = self.list.top;
        let old = self.list.selected;
        self.list = GroupList::new(rows);
        self.list.trail = trail;
        self.meta = meta;
        self.icons = icons;
        let same = |m: &Meta| match (&picked, m) {
            (Some(Meta::Header { app: a, .. }), Meta::Header { app: b, .. }) => a == b,
            (Some(Meta::Note { id: a, .. }), Meta::Note { id: b, .. }) => a == b,
            _ => false,
        };
        self.list.selected = self
            .meta
            .iter()
            .position(same)
            .or_else(|| old.map(|i| i.min(n.saturating_sub(1))))
            .filter(|_| n > 0);
        self.list.top = top.min(n.saturating_sub(ROWS as usize));
        // A rebuilt list may have moved a held group's Clear.
        if matches!(self.pointer.held(), Some(Held::GroupClear(_))) {
            self.pointer.clear();
        }
    }

    pub fn note_at(&self, i: usize) -> Option<u32> {
        match self.meta.get(i)? {
            Meta::Note { id, .. } => Some(*id),
            Meta::Header { .. } => None,
        }
    }

    fn app_of(&self, i: usize) -> Option<&str> {
        self.meta[..=i.min(self.meta.len().checked_sub(1)?)]
            .iter()
            .rev()
            .find_map(|m| match m {
                Meta::Header { app, .. } => Some(app.as_str()),
                Meta::Note { .. } => None,
            })
    }

    pub fn is_live(&self, i: usize) -> bool {
        matches!(self.meta.get(i), Some(Meta::Note { live: true, .. }))
    }

    fn set_open(&mut self, app: &str, open: bool) -> Event {
        let changed = if open {
            self.folded.remove(app)
        } else {
            self.folded.insert(app.to_string())
        };
        let header = self
            .meta
            .iter()
            .position(|m| matches!(m, Meta::Header { app: a, .. } if a == app));
        if let Some(h) = header {
            self.list.selected = Some(h);
        }
        if changed {
            self.rebuild();
        }
        if let Some(i) = self.list.selected {
            self.list.select(i, ROWS as usize);
        }
        Event::Redraw
    }

    /// Open `app`'s group and select its newest notification, scrolled
    /// so the header shows above it: where a click on its "new from"
    /// balloon lands.
    pub fn reveal(&mut self, app: &str) {
        self.folded.remove(app);
        self.rebuild();
        let header = self
            .meta
            .iter()
            .position(|m| matches!(m, Meta::Header { app: a, .. } if a == app));
        if let Some(h) = header {
            let first = if self.note_at(h + 1).is_some() {
                h + 1
            } else {
                h
            };
            self.list.top = h.min(self.list.rows.len().saturating_sub(ROWS as usize));
            self.list.select(first, ROWS as usize);
        }
    }

    pub fn key(&mut self, key: Key) -> Event {
        let page = ROWS as i64;
        let step = match key {
            Key::Up => -1,
            Key::Down => 1,
            Key::PageUp => -page,
            Key::PageDown => page,
            Key::Home => i64::MIN / 2,
            Key::End => i64::MAX / 2,
            Key::Escape => return Event::Close,
            Key::Enter | Key::Left | Key::Right | Key::Delete => {
                let Some(i) = self.list.selected else {
                    return Event::None;
                };
                return self.act(i, key);
            }
        };
        if self.list.step(step, ROWS as usize) {
            Event::Redraw
        } else {
            Event::None
        }
    }

    fn act(&mut self, i: usize, key: Key) -> Event {
        let Some(app) = self.app_of(i).map(str::to_string) else {
            return Event::None;
        };
        match (self.meta[i].clone(), key) {
            (Meta::Header { open, .. }, Key::Enter) => self.set_open(&app, !open),
            (Meta::Header { open: false, .. }, Key::Right) => self.set_open(&app, true),
            (_, Key::Left) => self.set_open(&app, false),
            (Meta::Header { .. }, Key::Delete) => Event::ClearGroup(app),
            (Meta::Note { id, live: true, .. }, Key::Enter) => Event::Invoke(id),
            (Meta::Note { id, .. }, Key::Delete) => Event::Dismiss(id),
            _ => Event::None,
        }
    }

    /// A left press at `hit`. The check box, Clear All and a group's
    /// Clear only hold down here; they act on a release over them
    /// (`release`). A press on a header folds or opens its group.
    pub fn press(&mut self, hit: Hit) -> Event {
        match hit {
            Hit::Dnd => self.hold(Held::Dnd),
            Hit::Clear if !self.list.rows.is_empty() => self.hold(Held::Clear),
            Hit::GroupClear(i) => self.hold(Held::GroupClear(i)),
            Hit::List(grouplist::Hit::Row(i)) => match self.meta.get(i).cloned() {
                Some(Meta::Header { app, open }) => self.set_open(&app, !open),
                Some(Meta::Note { .. }) => {
                    self.list.select(i, ROWS as usize);
                    Event::Redraw
                }
                None => Event::None,
            },
            Hit::List(bar) => {
                let by = match bar {
                    grouplist::Hit::ScrollUp => -1,
                    grouplist::Hit::ScrollDown => 1,
                    grouplist::Hit::PageUp => -ROWS,
                    grouplist::Hit::PageDown => ROWS,
                    _ => 0,
                };
                if self.list.scroll(by, ROWS as usize) {
                    Event::Redraw
                } else {
                    Event::None
                }
            }
            _ => Event::None,
        }
    }

    fn hold(&mut self, part: Held) -> Event {
        self.pointer.press(Some(part), false);
        Event::Redraw
    }

    pub fn holding(&self) -> bool {
        self.pointer.holding()
    }

    /// The pointer moved onto `hit` while holding a part, which draws
    /// pressed only while the pointer is on it. True when that changed.
    pub fn motion(&mut self, hit: Hit) -> bool {
        self.pointer.holding() && self.pointer.motion(Held::of(hit))
    }

    /// The pointer let go over `hit`: the held check box flips, or a
    /// Clear clears, when the release lands on it; off it, it pops back
    /// up.
    pub fn release(&mut self, hit: Hit) -> Event {
        if !self.pointer.holding() {
            return Event::None;
        }
        self.pointer.motion(Held::of(hit));
        let acted = self.pointer.release();
        self.pointer.leave();
        match acted {
            Some(Held::Dnd) => {
                self.dnd = !self.dnd;
                Event::Dnd
            }
            Some(Held::Clear) => Event::Clear,
            Some(Held::GroupClear(i)) => match self.meta.get(i) {
                Some(Meta::Header { app, .. }) => Event::ClearGroup(app.clone()),
                _ => Event::Redraw,
            },
            None => Event::Redraw,
        }
    }

    pub fn cancel_press(&mut self) -> bool {
        self.pointer.clear()
    }

    pub fn scroll(&mut self, rows: i64) -> bool {
        let rows = rows.clamp(i32::MIN.into(), i32::MAX.into()) as i32;
        self.list.scroll(rows, ROWS as usize)
    }

    fn list_layout(&self, cm: &ControlMetrics) -> grouplist::Layout {
        grouplist::layout(&self.list, layout(cm).list, cm)
    }

    pub fn row_at(&self, x: i32, y: i32, cm: &ControlMetrics) -> Option<usize> {
        match hit(self, x, y, cm) {
            Hit::List(grouplist::Hit::Row(i)) => Some(i),
            _ => None,
        }
    }
}

pub fn hit(h: &History, x: i32, y: i32, cm: &ControlMetrics) -> Hit {
    let lay = layout(cm);
    let check_row = Rect::new(
        lay.check.x,
        lay.label.y,
        lay.label.right() - lay.check.x,
        CHECK_ROW,
    );
    if check_row.contains(x, y) {
        return Hit::Dnd;
    }
    if lay.clear.contains(x, y) {
        return Hit::Clear;
    }
    let ll = h.list_layout(cm);
    match grouplist::hit(&h.list, &ll, x, y) {
        Some(grouplist::Hit::Row(i)) => {
            let on_clear = matches!(h.meta.get(i), Some(Meta::Header { .. }))
                && grouplist::row_rect(&h.list, &ll, i)
                    .is_some_and(|r| header_parts(r).0.contains(x, y));
            if on_clear {
                Hit::GroupClear(i)
            } else {
                Hit::List(grouplist::Hit::Row(i))
            }
        }
        Some(part) => Hit::List(part),
        // The list's frame, the strip under its rows, or off the list.
        None => Hit::Panel,
    }
}

/// Draw the panel at `theme`'s scale, as a flyout ([`crate::flyout`]); the
/// canvas reaches the look's shadow past the panel.
pub fn draw(theme: &Theme, text: &TextRenderer, h: &History) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let cm = theme.look.controls();
    let lay = layout(&cm);
    let mut c = Canvas::new((width(&cm) * s) as u32, (lay.height * s) as u32);
    let all = c.bounds();
    flyout::draw_panel(
        theme,
        &mut c,
        all,
        cs.button_face,
        flyout::Backdrop::Blurred,
    );
    let dnd = h.pointer.of(Held::Dnd);
    draw_checkbox(&mut c, p, lay.check.scaled(s), h.dnd, dnd);
    draw_label(
        &mut c,
        text,
        lay.label.scaled(s),
        DND_LABEL,
        false,
        cs.button_text,
        Align::Left,
        px,
        s,
    );
    let ll = h.list_layout(&cm);
    let icons: Vec<Option<&Canvas>> = h.icons.iter().map(|i| i.as_deref()).collect();
    grouplist::draw(&mut c, text, p, &h.list, &ll, &icons, Shown::Focused, px);
    for (i, m) in h.meta.iter().enumerate() {
        let Some(row) = grouplist::row_rect(&h.list, &ll, i) else {
            continue;
        };
        match m {
            Meta::Header { open, .. } => {
                let (clear, arrow) = header_parts(row);
                let arrow = arrow.scaled(s);
                if *open {
                    draw_arrow_v(&mut c, arrow, s, true, cs.button_text);
                } else {
                    draw_arrow_h(&mut c, arrow, s, true, cs.button_text);
                }
                draw_button(
                    &mut c,
                    text,
                    p,
                    clear.scaled(s),
                    GROUP_CLEAR_LABEL,
                    false,
                    h.pointer.of(Held::GroupClear(i)),
                    true,
                    false,
                    px,
                );
            }
            Meta::Note { time, .. } => {
                let r = Rect::new(row.right() - TIME_W, row.y, TIME_W - TIME_INSET, ROW_H);
                text.draw(
                    &mut c,
                    r.scaled(s),
                    time,
                    false,
                    cs.gray_text,
                    Align::Right,
                    px,
                );
            }
        }
    }
    if h.list.rows.is_empty() {
        let first = Rect::new(ll.inner.x, ll.inner.y + 4, ll.inner.width, ROW_H);
        draw_disabled_label(
            &mut c,
            text,
            p,
            first.scaled(s),
            EMPTY_LABEL,
            Align::Center,
            px,
            None,
        );
    }
    let enabled = !h.list.rows.is_empty();
    draw_button(
        &mut c,
        text,
        p,
        lay.clear.scaled(s),
        CLEAR_LABEL,
        false,
        Interaction::pressed_if(h.pointer.of(Held::Clear) == Interaction::Pressed && enabled),
        enabled,
        false,
        px,
    );
    flyout::shadowed(theme, c)
}

/// Four notifications from three programs, Calendar's folded, the newest Mail
/// one selected and Do not disturb on: what the gallery and preview test draw.
pub fn sample() -> History {
    let entry = |id, app: &str, message: &str, time: &str, live| Entry {
        id,
        app: app.into(),
        icon: None,
        message: message.into(),
        time: time.into(),
        live,
    };
    let mut h = History::new();
    h.folded.insert("Calendar".into());
    h.set_entries(vec![
        entry(4, "Mail", "3 new messages", "4:12 PM", true),
        entry(3, "Updates", "Updates are ready", "3:58 PM", true),
        entry(2, "Mail", "Lunch on Friday?", "1:05 PM", true),
        entry(1, "Calendar", "Stand-up in 5 minutes", "9:55 AM", false),
    ]);
    h.list.select(1, ROWS as usize);
    h.dnd = true;
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    fn labels(h: &History) -> Vec<&str> {
        h.list.rows.iter().map(Row::label).collect()
    }

    #[test]
    fn the_parts_stack_down_the_panel() {
        let lay = layout(&ControlMetrics::CLASSIC);
        assert_eq!(lay.check, Rect::new(8, 10, 13, 13));
        assert_eq!(lay.label.x, 8 + 13 + 5);
        // Two px of frame each side of ten 18 px rows.
        assert_eq!(lay.list, Rect::new(8, 31, 364, 4 + 10 * 18));
        assert_eq!(
            lay.clear,
            Rect::new(380 - 8 - 75, lay.list.bottom() + 8, 75, 23)
        );
        assert_eq!(lay.height, lay.clear.bottom() + 8);
    }

    /// A header per program, the program with the newest notification
    /// first, each saying how many it holds; a folded group shows its
    /// header alone.
    #[test]
    fn notifications_group_under_their_program_newest_first() {
        let h = sample();
        assert_eq!(
            labels(&h),
            [
                "Mail (2)",
                "3 new messages",
                "Lunch on Friday?",
                "Updates (1)",
                "Updates are ready",
                "Calendar (1)",
            ]
        );
        assert_eq!(h.note_at(1), Some(4));
        assert_eq!(h.note_at(0), None);
        assert_eq!(h.list.trail[0], GROUP_TRAIL);
        assert_eq!(h.list.trail[1], TIME_W);
    }

    #[test]
    fn a_program_with_no_name_groups_as_other() {
        let mut h = History::new();
        h.set_entries(vec![Entry {
            id: 1,
            app: String::new(),
            icon: None,
            message: "Hi".into(),
            time: "1:00 PM".into(),
            live: true,
        }]);
        assert_eq!(labels(&h), ["Other (1)", "Hi"]);
    }

    /// A press on a header folds its group and another opens it; Left
    /// folds from any row in it, Right opens, and the selection rests on
    /// the header.
    #[test]
    fn headers_fold_and_open_their_groups() {
        let mut h = sample();
        assert_eq!(h.press(Hit::List(grouplist::Hit::Row(0))), Event::Redraw);
        assert_eq!(
            labels(&h)[..3],
            ["Mail (2)", "Updates (1)", "Updates are ready"]
        );
        assert_eq!(h.list.selected, Some(0));
        h.press(Hit::List(grouplist::Hit::Row(0)));
        assert_eq!(labels(&h).len(), 6, "open again");
        h.list.select(2, ROWS as usize);
        assert_eq!(h.key(Key::Left), Event::Redraw);
        assert_eq!((labels(&h).len(), h.list.selected), (4, Some(0)));
        assert_eq!(h.key(Key::Right), Event::Redraw);
        assert_eq!(labels(&h).len(), 6);
        h.list.select(5, ROWS as usize);
        assert_eq!(h.key(Key::Enter), Event::Redraw, "Enter opens Calendar");
        assert_eq!(labels(&h).last(), Some(&"Stand-up in 5 minutes"));
    }

    #[test]
    fn enter_runs_a_live_notification_and_delete_dismisses() {
        let mut h = sample();
        assert_eq!(h.key(Key::Enter), Event::Invoke(4));
        assert_eq!(h.key(Key::Delete), Event::Dismiss(4));
        assert_eq!(h.key(Key::Up), Event::Redraw);
        assert_eq!(h.key(Key::Delete), Event::ClearGroup("Mail".into()));
        let mut dead = sample();
        dead.reveal("Calendar");
        assert_eq!(dead.list.selected, Some(6));
        assert_eq!(dead.key(Key::Enter), Event::None, "not live");
        assert_eq!(dead.key(Key::Escape), Event::Close);
    }

    /// A group's Clear holds down on the press and clears that program
    /// on the release over it; dragged off, it does nothing.
    #[test]
    fn a_groups_clear_acts_on_the_release_over_it() {
        let cm = ControlMetrics::CLASSIC;
        let mut h = sample();
        let ll = h.list_layout(&cm);
        let row = grouplist::row_rect(&h.list, &ll, 3).unwrap();
        let (clear, _) = header_parts(row);
        assert_eq!(clear, Rect::new(row.right() - 46, row.y + 1, 44, 16));
        let at = hit(&h, clear.x + 5, clear.y + 5, &cm);
        assert_eq!(at, Hit::GroupClear(3));
        assert_eq!(h.press(at), Event::Redraw);
        assert!(h.holding());
        assert_eq!(h.release(at), Event::ClearGroup("Updates".into()));
        h.press(at);
        assert!(h.motion(Hit::Panel));
        assert_eq!(h.release(Hit::Panel), Event::Redraw, "released off it");
        assert_eq!(
            hit(&h, row.x + 30, row.y + 5, &cm),
            Hit::List(grouplist::Hit::Row(3)),
            "the header's name is its row"
        );
    }

    #[test]
    fn new_entries_keep_the_selection_on_its_notification() {
        let mut h = sample();
        h.list.select(2, ROWS as usize);
        let mut entries = h.entries.clone();
        entries.insert(
            0,
            Entry {
                id: 5,
                app: "Chat".into(),
                icon: None,
                message: "Hey".into(),
                time: "4:20 PM".into(),
                live: true,
            },
        );
        h.set_entries(entries);
        assert_eq!(h.note_at(h.list.selected.unwrap()), Some(2));
        h.set_entries(Vec::new());
        assert_eq!(h.list.selected, None);
    }

    #[test]
    fn reveal_opens_the_group_and_picks_its_newest() {
        let mut h = sample();
        h.reveal("Calendar");
        assert_eq!(h.note_at(h.list.selected.unwrap()), Some(1));
        assert!(h.list.top <= 5, "its header in view");
    }

    #[test]
    fn clear_all_only_holds_down_with_rows_to_clear() {
        let mut h = sample();
        assert_eq!(h.press(Hit::Clear), Event::Redraw);
        assert!(h.holding());
        assert_eq!(h.release(Hit::Clear), Event::Clear);
        let mut empty = History::new();
        assert_eq!(empty.press(Hit::Clear), Event::None);
        assert!(!empty.holding());
    }

    /// The check box flips on the release over it, as Clear All acts: a
    /// release after dragging off leaves it, and back on it arms again.
    #[test]
    fn do_not_disturb_flips_on_the_release_over_it() {
        let mut h = History::new();
        assert_eq!(h.press(Hit::Dnd), Event::Redraw, "the press holds it");
        assert_eq!(h.pointer.of(Held::Dnd), Interaction::Pressed);
        assert!(!h.dnd, "the press waits for the release");
        assert_eq!(h.release(Hit::Dnd), Event::Dnd);
        assert!(h.dnd);
        assert_eq!(h.pointer.of(Held::Dnd), Interaction::Idle, "nothing lit");
        h.press(Hit::Dnd);
        assert!(h.motion(Hit::Panel), "dragged off");
        assert_eq!(h.release(Hit::Panel), Event::Redraw);
        assert!(h.dnd, "released off it, unchanged");
        h.press(Hit::Dnd);
        assert!(h.cancel_press());
        assert_eq!(h.release(Hit::Dnd), Event::None, "a cancelled press");
        assert!(h.dnd);
    }

    /// Under Blue the Do not disturb box stays 13 px and its label starts
    /// 16 px in from the box's left edge.
    #[test]
    fn a_blue_do_not_disturb_label_starts_16_px_in() {
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let cm = blue.look.controls();
        let lay = layout(&cm);
        assert_eq!(lay.check, Rect::new(8, 10, 13, 13));
        assert_eq!(lay.label.x, 8 + 16);
    }

    /// The panel's pixels: a ticked box, an open group's down arrow and a
    /// folded one's right arrow in button text, each header's Clear a
    /// raised button, a grey time right-aligned on a notification's row
    /// and the selected notification's highlight.
    #[test]
    fn the_panel_draws_its_groups() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let h = sample();
        let c = draw(&theme, &text, &h);
        let lay = layout(&ControlMetrics::CLASSIC);
        let cs = &theme.colors;
        assert_eq!((c.width as i32, c.height as i32), (W, lay.height));
        assert_eq!(c.get(5, 1), cs.button_hilight.argb(), "the raised edge");
        let b = lay.check;
        assert!((b.x + 3..b.x + 10).any(|x| c.get(x, b.y + 7) == cs.window_text.argb()));
        let ll = h.list_layout(&ControlMetrics::CLASSIC);
        let row = |i| grouplist::row_rect(&h.list, &ll, i).unwrap();
        let ink = cs.button_text.argb();
        // Mail's arrow points down: rows of 7, 5, 3, 1 from 7 px up top.
        let (clear, arrow) = header_parts(row(0));
        let (ax, ay) = (arrow.x + 2, arrow.y + 7);
        assert!(
            (0..7).all(|i| c.get(ax + i, ay) == ink),
            "the down arrow's top row"
        );
        assert_eq!(c.get(ax + 3, ay + 3), ink);
        assert_ne!(c.get(ax, ay + 3), ink);
        // Calendar's, folded, points right: its first column 7 tall.
        let (_, folded) = header_parts(row(5));
        let fx = folded.x + (ARROW_CELL - 4) / 2;
        let fy = folded.y + (ROW_H - 7) / 2;
        assert!(
            (0..7).all(|i| c.get(fx, fy + i) == ink),
            "the right arrow's first column"
        );
        assert_ne!(c.get(fx + 3, fy), ink);
        // The Clear button: a white top edge, its label's ink inside.
        assert_eq!(c.get(clear.x + 5, clear.y), cs.button_hilight.argb());
        assert!(
            (clear.x..clear.right()).any(|x| (clear.y..clear.bottom()).any(|y| c.get(x, y) == ink))
        );
        // The selected notification (row 1) wears the highlight; its time
        // stays grey on white to the right.
        let r1 = row(1);
        assert_eq!(c.get(r1.x + 44, r1.y + 2), cs.hilight.argb());
        let time = Rect::new(r1.right() - TIME_W, r1.y, TIME_W - TIME_INSET, ROW_H);
        let grey = cs.gray_text.argb();
        let xs: Vec<i32> = (time.x..r1.right())
            .filter(|&x| (time.y..time.bottom()).any(|y| c.get(x, y) == grey))
            .collect();
        // The advance ends at the column's right edge; the last glyph's
        // side bearing leaves a pixel or two.
        let last = xs.last().copied().unwrap_or(0);
        assert!(
            (time.right() - 3..time.right()).contains(&last),
            "right-aligned: {last}"
        );
        assert_eq!(c.get(r1.right() - 2, r1.y + 2), cs.window.argb());
    }

    /// Blue and Glass draw the panel through their own button and check
    /// box parts; the rows sit where Classic's do.
    #[test]
    fn every_interface_draws_the_panel() {
        use crate::interface::Interface;
        let text = TextRenderer::load(&FontConfig::default());
        for iface in [Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(iface);
            let c = draw(&theme, &text, &sample());
            let cm = theme.look.controls();
            let lay = layout(&cm);
            let m = flyout::margin(&theme);
            assert_eq!(
                (c.width as i32, c.height as i32),
                (width(&cm) + m.left + m.right, lay.height + m.top + m.bottom),
                "{iface:?}"
            );
            let h = sample();
            let ll = h.list_layout(&cm);
            let r1 = grouplist::row_rect(&h.list, &ll, 1).unwrap();
            assert_eq!(
                c.get(m.left + r1.x + 44, m.top + r1.y + 2),
                theme.colors.hilight.argb(),
                "{iface:?}: the selected notification"
            );
        }
    }

    #[test]
    fn under_glass_the_panel_is_a_flyout_with_its_contents_6_px_further_in() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let cm = theme.look.controls();
        let c = draw(&theme, &text, &sample());
        let lay = layout(&cm);
        let classic = layout(&ControlMetrics {
            flyout_border: 2,
            ..cm
        });
        assert_eq!(width(&cm), 392);
        assert_eq!(lay.height, classic.height + 12);
        assert_eq!(lay.list.x, classic.list.x + 6);
        assert_eq!(lay.list.width, classic.list.width);
        assert_eq!(lay.clear.x, classic.clear.x + 6);
        let m = crate::flyout::margin(&theme);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (392 + m.left + m.right, lay.height + m.top + m.bottom),
            "the panel and its shadow"
        );
        // White inside the border, between it and the list.
        assert_eq!(c.get(m.left + 9, m.top + lay.list.y + 20), 0xFFFF_FFFF);
        assert!(
            c.get(m.left + 3, m.top + 100) >> 24 < 0xFF,
            "the glass border"
        );
    }

    /// Writes target/notifyhistory-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_notify_history() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        crate::raster::write_preview(&draw(&theme, &text, &sample()), "notifyhistory-preview.png");
        crate::raster::write_preview(
            &draw(&theme, &text, &History::new()),
            "notifyhistory-empty-preview.png",
        );
    }
}
