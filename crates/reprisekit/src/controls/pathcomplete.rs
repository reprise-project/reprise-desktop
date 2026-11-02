//! Path completion for a single-line edit: the matches' common prefix inline, its tail selected,
//! and a drop list of the matches. The host routes the edit's keys here and lists each [`Want`].

use crate::access::{self, Tree, accesskit};
use crate::completion::Inline;
use crate::controls::combo::{self, ListItem};
use crate::controls::form;
use crate::controls::hosted::{Anchor, Hosted, Spot};
use crate::look::Paint;
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{EditKey, TextEdit};
use accesskit::{Node, NodeId, Role};

/// The most matches the list holds.
pub const MAX_ROWS: usize = 10;

/// One name in a listed folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// Which text completes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The whole text is a path, relative ones from the host's folder (a file name box).
    Relative,
    /// Only a path from `/` or `~`: the whole text, else its last word (a command line).
    Rooted,
}

/// A folder the host lists for the model: `dir` is the folder part as typed (`~/`, `/usr/`,
/// `sub/`, or empty for the host's folder).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Want {
    pub dir: String,
    era: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Enter,
    Tab,
    Escape,
}

/// The completion's key for a form key, if it has one; Shift+Tab has none.
pub fn nav_key(key: &form::Key) -> Option<Key> {
    Some(match key {
        form::Key::Up => Key::Up,
        form::Key::Down => Key::Down,
        form::Key::Enter => Key::Enter,
        form::Key::Tab { back: false } => Key::Tab,
        form::Key::Escape => Key::Escape,
        _ => return None,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathComplete {
    scope: Scope,
    /// The listed folder part and its entries, folders first, each run by name.
    listing: Option<(String, Vec<Entry>)>,
    /// A folder part to list that the host has not taken yet.
    want: Option<String>,
    /// The folder part the host is listing; no other is asked for until it lands, so a mount
    /// that stopped answering holds one read, not one per key.
    waiting: Option<String>,
    /// Bumped by [`PathComplete::forget`]: a listing asked for before it is dropped.
    era: u32,
    /// The shown matches' labels, folders with their `/`.
    rows: Vec<String>,
    /// The highlighted row; none until Up or Down.
    pub hot: Option<usize>,
    /// The first row in view as the wheel left it; Up and Down bring the highlight back in view.
    first: usize,
    /// Up or Down moved the highlight since the wheel last turned.
    follow: bool,
    inline: Inline,
    /// The edit's text as the model last left it: any other text was set from outside, and a
    /// tail or list over it is stale.
    seen: String,
    /// The last key typed or pasted, so a listing that lands may still complete inline.
    typed: bool,
    /// The list's corner where a host showing it on a popup put it.
    pub list_at: Option<(i32, i32)>,
    /// A host shows the list on a popup: the owner's draw leaves it out.
    pub hosted: bool,
}

impl PathComplete {
    pub fn new(scope: Scope) -> PathComplete {
        PathComplete {
            scope,
            listing: None,
            want: None,
            waiting: None,
            era: 0,
            rows: Vec::new(),
            hot: None,
            first: 0,
            follow: true,
            inline: Inline::default(),
            seen: String::new(),
            typed: false,
            list_at: None,
            hosted: false,
        }
    }

    pub fn is_open(&self) -> bool {
        !self.rows.is_empty()
    }

    pub fn rows(&self) -> &[String] {
        &self.rows
    }

    /// Whether `edit` shows a completed tail.
    pub fn shows_tail(&self) -> bool {
        self.inline.tail().is_some()
    }

    /// The list closes; a shown tail stays as typed text.
    pub fn close(&mut self) {
        self.rows.clear();
        self.hot = None;
        self.first = 0;
        self.follow = true;
        self.list_at = None;
    }

    /// The host's folder changed: listings of relative folder parts are stale.
    pub fn forget(&mut self) {
        self.era += 1;
        self.listing = None;
        self.waiting = None;
        self.want = None;
        self.close();
    }

    /// The edit's text was set from outside: the list closes and nothing is completed.
    pub fn reset(&mut self) {
        self.inline.reset();
        self.typed = false;
        self.close();
    }

    /// The folder the host should list now, if any.
    pub fn take_want(&mut self) -> Option<Want> {
        let dir = self.want.take()?;
        self.waiting = Some(dir.clone());
        Some(Want { dir, era: self.era })
    }

    /// An editing key into `edit`; returns whether the text, the selection or the list changed.
    pub fn edit(&mut self, edit: &mut TextEdit, ek: EditKey) -> bool {
        self.sync(edit);
        let before = (edit.text.clone(), edit.selection(), self.rows.clone());
        self.typed = matches!(ek, EditKey::Insert(_));
        self.inline.key(edit, ek);
        self.refresh(edit);
        before != (edit.text.clone(), edit.selection(), self.rows.clone())
    }

    /// A paste into `edit`, completed as typing is.
    pub fn paste(&mut self, edit: &mut TextEdit, text: &str) {
        self.sync(edit);
        self.inline.paste(edit, text);
        self.typed = true;
        self.refresh(edit);
    }

    /// The host's listing of `want`. Names come in any order; `.` and `..` are left out.
    /// Returns whether the field or the list changed.
    pub fn set_listing(
        &mut self,
        edit: &mut TextEdit,
        want: &Want,
        mut entries: Vec<Entry>,
    ) -> bool {
        if want.era != self.era {
            return false;
        }
        self.sync(edit);
        if self.waiting.as_deref() == Some(want.dir.as_str()) {
            self.waiting = None;
        }
        entries.retain(|e| e.name != "." && e.name != "..");
        entries.sort_by_cached_key(|e| (!e.dir, e.name.to_lowercase(), e.name.clone()));
        self.listing = Some((want.dir.clone(), entries));
        let before = (edit.text.clone(), self.rows.clone());
        self.refresh(edit);
        before != (edit.text.clone(), self.rows.clone())
    }

    /// A key while the field has the focus; true when the completion took it. Tab takes the
    /// highlight or the shown tail; Enter with nothing highlighted closes the list and goes on.
    pub fn nav(&mut self, edit: &mut TextEdit, key: Key) -> bool {
        self.sync(edit);
        let open = self.is_open();
        if open && matches!(key, Key::Up | Key::Down) {
            self.follow = true;
        }
        match key {
            Key::Down if open => {
                let last = self.rows.len() - 1;
                self.hot = Some(self.hot.map_or(0, |i| (i + 1).min(last)));
            }
            Key::Up if open => self.hot = self.hot.and_then(|i| i.checked_sub(1)),
            Key::Enter | Key::Tab if self.hot.is_some() => {
                let i = self.hot.unwrap_or(0);
                self.take(edit, i);
            }
            Key::Tab if self.shows_tail() => {
                self.inline.accept(edit);
                self.typed = false;
                self.refresh(edit);
            }
            Key::Escape if open || self.shows_tail() => {
                self.inline.drop_tail(edit);
                self.close();
            }
            Key::Enter => {
                self.close();
                return false;
            }
            _ => return false,
        }
        self.seen.clone_from(&edit.text);
        true
    }

    /// Row `i` replaces the name being typed; a folder's contents come up next.
    pub fn take(&mut self, edit: &mut TextEdit, i: usize) {
        self.sync(edit);
        let Some(label) = self.rows.get(i).cloned() else {
            return;
        };
        self.inline.drop_tail(edit);
        let Some((_, stem)) = split(&edit.text, self.scope) else {
            return;
        };
        edit.text.truncate(stem);
        edit.text.push_str(&label);
        let end = edit.text.len();
        edit.set_selection(end, end);
        self.typed = false;
        self.refresh(edit);
    }

    /// The first row in view when `rows` show: where the wheel left it, moved as little as
    /// keeping a highlight Up or Down moved in view needs.
    pub fn top(&self, rows: usize) -> usize {
        let rows = rows.max(1);
        let top = self.first.min(self.rows.len().saturating_sub(rows));
        match self.hot.filter(|_| self.follow) {
            Some(h) if h < top => h,
            Some(h) if h >= top + rows => h + 1 - rows,
            _ => top,
        }
    }

    /// The wheel turned `lines` rows (down is positive) while `rows` show: true when the list
    /// scrolls (it has more matches than rows), so the host keeps the wheel from what lies under it.
    pub fn wheel(&mut self, rows: usize, lines: i32) -> bool {
        if !self.is_open() || rows >= self.rows.len() {
            return false;
        }
        let top = self.top(rows) as i64 + i64::from(lines);
        self.first = top.clamp(0, (self.rows.len() - rows) as i64) as usize;
        self.follow = false;
        true
    }

    fn sync(&mut self, edit: &TextEdit) {
        if edit.text != self.seen {
            self.reset();
        }
    }

    /// Lists the matches of the name before the caret and completes it inline after a typed key.
    fn refresh(&mut self, edit: &mut TextEdit) {
        let typed = std::mem::take(&mut self.typed);
        let at = self.list_at;
        self.close();
        self.fill(edit, typed);
        if self.is_open() {
            self.list_at = at;
        }
        self.seen.clone_from(&edit.text);
    }

    fn fill(&mut self, edit: &mut TextEdit, typed: bool) {
        let query = self.inline.query(edit).to_string();
        let at_end = match self.inline.tail() {
            Some(t) => edit.selection() == (t..edit.text.len()),
            None => !edit.has_selection() && edit.caret() == edit.text.len(),
        };
        let Some((dir_at, stem_at)) = split(&query, self.scope).filter(|_| at_end) else {
            self.inline.drop_tail(edit);
            return;
        };
        let (dir, stem) = (&query[dir_at..stem_at], &query[stem_at..]);
        let entries = match &self.listing {
            Some((d, e)) if d == dir => e,
            _ => {
                if self.waiting.is_none() {
                    self.want = Some(dir.to_string());
                }
                // Typed again once the listing lands.
                self.typed = typed;
                self.inline.drop_tail(edit);
                return;
            }
        };
        let show_hidden = stem.starts_with('.');
        let labels: Vec<String> = entries
            .iter()
            .filter(|e| e.name.starts_with(stem) && (show_hidden || !e.name.starts_with('.')))
            .map(|e| match e.dir {
                true => format!("{}/", e.name),
                false => e.name.clone(),
            })
            .collect();
        let common = common_prefix(&labels);
        if typed && self.inline.completing() && !stem.is_empty() && common.len() > stem.len() {
            let rest = common[stem.len()..].to_string();
            self.inline.offer(edit, &query, &rest);
        } else {
            self.inline.drop_tail(edit);
        }
        if labels.iter().all(|l| l == stem) {
            return;
        }
        self.rows = labels.into_iter().take(MAX_ROWS).collect();
    }
}

/// Where the path being completed starts in `text` and where its last name starts, or None when
/// nothing completes: no path in `scope`, quoted names, a pattern, or a bare `~`.
pub fn split(text: &str, scope: Scope) -> Option<(usize, usize)> {
    let rooted = |s: &str| s.starts_with('/') || s.starts_with('~');
    let start = match scope {
        Scope::Relative if text.trim_start().starts_with('"') => return None,
        Scope::Relative => 0,
        Scope::Rooted if rooted(text) => 0,
        Scope::Rooted => {
            let at = text.rfind(char::is_whitespace)? + 1;
            rooted(&text[at..]).then_some(at)?
        }
    };
    let path = &text[start..];
    if path.is_empty() || path == "~" || path.contains(['*', '?']) {
        return None;
    }
    let stem = path.rfind('/').map_or(0, |i| i + 1);
    if stem == 0 && path.starts_with('~') {
        return None;
    }
    Some((start, start + stem))
}

fn common_prefix(labels: &[String]) -> &str {
    let Some(first) = labels.first() else {
        return "";
    };
    let mut end = first.len();
    for l in &labels[1..] {
        end = first
            .char_indices()
            .zip(l.chars())
            .find(|((_, a), b)| a != b)
            .map_or(end.min(l.len()), |((i, _), _)| i.min(end));
    }
    &first[..end]
}

/// The list's place: `rows` in view of `row_h`, inside a 1 px frame, with a scrollbar when more
/// matches than rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub list: Rect,
    pub rows: usize,
    pub row_h: i32,
    pub scrollbar: Option<Rect>,
}

/// The list under `field`, as wide as it. Given `room` (an owner drawing the list itself), it
/// stays inside: over the field when more room is there, as many rows as fit.
pub fn layout(pc: &PathComplete, field: Rect, room: Option<Rect>, cm: &ControlMetrics) -> Layout {
    let row_h = cm.combo_row;
    let n = pc.rows.len();
    let fit = |h: i32| ((h - 2) / row_h).max(0) as usize;
    let (y, rows) = match (pc.list_at, room) {
        (Some((_, y)), _) => (y, n),
        (None, None) => (field.bottom(), n),
        (None, Some(room)) => {
            let below = fit(room.bottom() - field.bottom());
            let above = fit(field.y - room.y);
            if below >= n || below >= above {
                (field.bottom(), n.min(below))
            } else {
                let rows = n.min(above);
                (field.y - rows as i32 * row_h - 2, rows)
            }
        }
    };
    let x = pc.list_at.map_or(field.x, |(x, _)| x);
    let list = Rect::new(x, y, field.width, rows as i32 * row_h + 2);
    let scrollbar = (rows < n).then(|| {
        Rect::new(
            list.right() - 1 - cm.scroll,
            list.y + 1,
            cm.scroll,
            list.height - 2,
        )
    });
    Layout {
        list,
        rows,
        row_h,
        scrollbar,
    }
}

/// The row at (`x`, `y`), when it is on one; a point on the frame or scrollbar is on none.
pub fn hit(pc: &PathComplete, lay: &Layout, x: i32, y: i32) -> Option<usize> {
    if !pc.is_open()
        || !lay.list.inset(1).contains(x, y)
        || lay.scrollbar.is_some_and(|s| s.contains(x, y))
    {
        return None;
    }
    let row = pc.top(lay.rows) + ((y - lay.list.y - 1) / lay.row_h) as usize;
    (row < pc.rows.len()).then_some(row)
}

/// The list in the look's drop list frame, the highlighted row lit; nothing while closed.
pub fn draw(
    pc: &PathComplete,
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    lay: &Layout,
    px: u32,
) {
    if !pc.is_open() || lay.rows == 0 {
        return;
    }
    let items: Vec<ListItem> = pc.rows.iter().map(ListItem::new).collect();
    combo::draw_list_rows(
        c,
        text,
        p,
        &items,
        pc.hot,
        pc.top(lay.rows),
        lay.rows,
        lay.list,
        lay.scrollbar,
        None,
        px,
        lay.row_h,
    );
}

/// A list box of the matches under `parent` while open, the highlighted one selected.
pub fn access(pc: &PathComplete, lay: &Layout, tree: &mut Tree, parent: NodeId) {
    if !pc.is_open() {
        return;
    }
    let rows = lay.list.inset(1);
    let list = tree.push(parent, "completions", access::node(Role::ListBox, rows));
    for (i, label) in pc.rows.iter().enumerate() {
        let mut o = Node::new(Role::ListBoxOption);
        o.set_label(label.as_str());
        let y = rows.y + (i as i32 - pc.top(lay.rows) as i32) * lay.row_h;
        o.set_bounds(access::row_bounds(
            rows,
            Rect::new(rows.x, y, rows.width, lay.row_h),
        ));
        o.set_selected(pc.hot == Some(i));
        tree.push(list, i, o);
    }
}

/// The list for a host that shows it on a popup: `field` and `cm` as the owner lays the field
/// out, `px` its text px.
pub struct Popped<'a> {
    pub pc: &'a mut PathComplete,
    pub field: Rect,
    pub cm: ControlMetrics,
    pub px: u32,
}

/// Dismissing it closes the list.
impl Hosted for Popped<'_> {
    fn spots(&self) -> Vec<Spot> {
        if !self.pc.is_open() {
            return Vec::new();
        }
        vec![Spot {
            rect: layout(self.pc, self.field, None, &self.cm).list,
            anchor: Anchor::Below(self.field),
        }]
    }

    fn place(&mut self, level: usize, origin: (i32, i32)) {
        if level == 0 {
            self.pc.list_at = Some(origin);
        }
    }

    fn shadow(&self, theme: &crate::theme::Theme) -> i32 {
        combo::list_shadow(theme)
    }

    fn draw_level(
        &self,
        level: usize,
        c: &mut Canvas,
        by: (i32, i32),
        theme: &crate::theme::Theme,
        text: &TextRenderer,
    ) {
        if level != 0 {
            return;
        }
        let mut lay = layout(self.pc, self.field, None, &self.cm);
        lay.list = Rect::new(
            lay.list.x + by.0,
            lay.list.y + by.1,
            lay.list.width,
            lay.list.height,
        );
        let p = Paint {
            s: 1,
            ..theme.paint()
        };
        draw(self.pc, c, text, &p, &lay, self.px);
    }

    fn host(&mut self) {
        self.pc.hosted = true;
    }

    fn dismiss(&mut self) {
        self.pc.close();
    }
}

#[cfg(test)]
mod tests;
