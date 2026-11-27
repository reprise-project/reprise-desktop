//! The clipboard history popup: a flyout panel ([`crate::flyout`]) with a
//! filter field on top and a list box of what was copied, newest first. A
//! row is a one-line text preview or an image's size, its icon slot
//! holding the image's thumbnail. Typing filters the rows with `fuzzy`;
//! Up, Down, Page Up and Page Down move the highlight, Enter picks it,
//! Delete removes it, Escape closes. Pure: the host owns the history and
//! acts on the events, which name a row by the `id` it was given.

use std::time::Instant;

use crate::classic::widgets::{BarHit, ROW_H as FIELD_H, edit_first_visible};
use crate::controls::listbox::{self, ListBox};
use crate::controls::metrics::ControlMetrics;
use crate::controls::scrollbar::RowBar;
use crate::flyout;
use crate::look::widgets::{Interaction, draw_edit_field, draw_edit_text};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey, Preedit, TextEdit};
use crate::theme::Theme;

pub const W: i32 = 360;
pub const VISIBLE: usize = 10;
const PAD: i32 = 4;
pub fn empty() -> String {
    tr!("cliphistory-empty")
}
/// What a reader calls the filter field.
pub fn filter() -> String {
    tr!("cliphistory-filter")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// The host's name for the entry; events carry it back.
    pub id: u64,
    pub label: String,
    /// [`listbox::icon_px`] at the drawing scale, or None for no icon.
    pub icon: Option<Canvas>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ClipHistory {
    /// Private so the list always matches it: every write goes through
    /// `key` or `paste`.
    edit: TextEdit,
    rows: Vec<Row>,
    /// Indices into `rows`, parallel to `list.items`.
    shown: Vec<usize>,
    pub list: ListBox,
    /// Parallel to `list.items`.
    icons: Vec<Option<Canvas>>,
    bar: RowBar,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(EditKey),
    Up,
    Down,
    PageUp,
    PageDown,
    Enter,
    Escape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    /// Put this entry on the clipboard and paste it.
    Pick(u64),
    /// Drop this entry from the history; its row is already gone.
    Remove(u64),
    Close,
}

impl ClipHistory {
    pub fn new(rows: Vec<Row>) -> ClipHistory {
        let mut m = ClipHistory {
            edit: TextEdit::new("", false),
            rows,
            shown: Vec::new(),
            list: ListBox::default(),
            icons: Vec::new(),
            bar: RowBar::default(),
        };
        m.refilter();
        m
    }

    pub fn edit(&self) -> &TextEdit {
        &self.edit
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> Option<u64> {
        let i = *self.shown.get(self.list.selected?)?;
        Some(self.rows[i].id)
    }

    /// Rank the rows for the filter: history order while it is empty, else
    /// the fuzzy matches best first. The highlight goes to the first row.
    fn refilter(&mut self) {
        let query = self.edit.text.as_str();
        self.shown = if query.is_empty() {
            (0..self.rows.len()).collect()
        } else {
            crate::fuzzy::rank(query, self.rows.iter().map(|r| r.label.as_str()))
        };
        let items = self
            .shown
            .iter()
            .map(|&i| self.rows[i].label.clone())
            .collect();
        let selected = (!self.shown.is_empty()).then_some(0);
        self.list = ListBox::new(items, selected);
        self.sync_icons();
    }

    fn sync_icons(&mut self) {
        self.icons = self
            .shown
            .iter()
            .map(|&i| self.rows[i].icon.clone())
            .collect();
    }

    /// An icon that arrived after the popup opened.
    pub fn set_icon(&mut self, id: u64, icon: Canvas) {
        if let Some(r) = self.rows.iter_mut().find(|r| r.id == id) {
            r.icon = Some(icon);
            self.sync_icons();
        }
    }

    pub fn key(&mut self, k: Key) -> Event {
        let page = VISIBLE as i64 - 1;
        match k {
            Key::Char(EditKey::Delete { word: false }) if self.list.selected.is_some() => {
                self.remove_selected()
            }
            Key::Char(ek) => {
                let before = self.edit.text.clone();
                self.edit.key(ek);
                if self.edit.text != before {
                    self.refilter();
                }
                Event::None
            }
            Key::Up => self.step(-1),
            Key::Down => self.step(1),
            Key::PageUp => self.step(-page),
            Key::PageDown => self.step(page),
            Key::Enter => self.selected().map_or(Event::Close, Event::Pick),
            Key::Escape => Event::Close,
        }
    }

    fn step(&mut self, delta: i64) -> Event {
        self.list.step(delta, VISIBLE);
        Event::None
    }

    /// Drop the highlighted row, keeping the highlight at the same place
    /// (on the new last row when the last one went).
    fn remove_selected(&mut self) -> Event {
        let Some(at) = self.list.selected else {
            return Event::None;
        };
        let i = self.shown[at];
        let id = self.rows.remove(i).id;
        let top = self.list.top;
        self.refilter();
        let n = self.shown.len();
        if n > 0 {
            self.list.top = top.min(n.saturating_sub(VISIBLE));
            self.list.select(at.min(n - 1), VISIBLE);
        }
        Event::Remove(id)
    }

    pub fn paste(&mut self, text: &str) {
        let line = text.lines().next().unwrap_or_default();
        if self.edit.paste(line, &[]) {
            self.refilter();
        }
    }

    /// A left click on `hit`: a row picks it, the scroll arrows and track scroll.
    pub fn click(&mut self, hit: Hit) -> Event {
        let (rows, page) = (self.list.items.len(), VISIBLE as i32);
        match hit {
            Hit::Row(i) if i < rows => {
                self.list.select(i, VISIBLE);
                self.selected().map_or(Event::None, Event::Pick)
            }
            Hit::Bar(b) => {
                let rows = self.bar.press(b, page.into());
                self.scroll(rows as i32)
            }
            _ => Event::None,
        }
    }

    fn scroll(&mut self, rows: i32) -> Event {
        self.list.scroll(rows, VISIBLE);
        Event::None
    }

    /// The wheel turned `rows` rows ([`WHEEL_ROWS`](crate::pointer::WHEEL_ROWS)
    /// a notch, down positive) over the panel: the list scrolls and the
    /// highlight stays. True when the view moved.
    pub fn wheel(&mut self, rows: i32) -> bool {
        self.list.scroll(rows, VISIBLE)
    }

    /// A reader asked to see row `i`: the list scrolls it in and the highlight stays. True when
    /// the view moved.
    pub fn reveal(&mut self, i: usize) -> bool {
        self.list.reveal(i, VISIBLE)
    }

    /// The pointer at (`x`, `y`) with the bar held: the list follows a held thumb wherever the
    /// pointer goes, and a held arrow repeats only while the pointer is on it. True when the
    /// list moved.
    pub fn drag(&mut self, x: i32, y: i32, cm: &ControlMetrics) -> bool {
        let ll = listbox::layout(&self.list, layout(self, cm).list, cm);
        self.bar
            .hover(listbox::hit(&self.list, &ll, x, y).and_then(listbox::Hit::bar));
        let Some(grab) = self.bar.grab() else {
            return false;
        };
        let top = listbox::top_at(&self.list, &ll, y, grab);
        let moved = top != self.list.top;
        self.list.top = top;
        moved
    }

    /// Whether the thumb or an arrow is held: the host sends motion, ticks and the release.
    pub fn holding(&self) -> bool {
        self.bar.holding()
    }

    /// The pointer let go. True when the bar was held.
    pub fn release(&mut self) -> bool {
        self.bar.release()
    }

    /// A host tick at `now`: a held arrow steps once it repeats. True when the list moved.
    pub fn tick(&mut self, now: Instant) -> bool {
        let rows = self.bar.tick(now) as i32;
        rows != 0 && self.list.scroll(rows, VISIBLE)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub field: Rect,
    pub list: Rect,
    pub height: i32,
}

/// The panel's width in a look with controls `cm`: [`W`] grown with the text, plus a wider
/// flyout border's extra on each side.
pub fn width(cm: &ControlMetrics) -> i32 {
    cm.text.len(W) + 2 * (cm.flyout_border - flyout::EDGE)
}

/// The list box is as tall as the whole history (one row at least, at most
/// `VISIBLE`), so filtering never moves the panel's edges. A flyout border
/// wider than Classic's edge moves everything in by the difference.
pub fn layout(m: &ClipHistory, cm: &ControlMetrics) -> Layout {
    let pad = PAD + cm.flyout_border - flyout::EDGE;
    let w = width(cm);
    let field = Rect::new(pad, pad, w - 2 * pad, cm.text.len(FIELD_H));
    let rows = m.rows.len().clamp(1, VISIBLE) as i32;
    let list = Rect::new(
        pad,
        field.bottom() + PAD,
        w - 2 * pad,
        rows * cm.list_row + 4,
    );
    Layout {
        field,
        list,
        height: list.bottom() + pad,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// A row, as an index into the list as shown.
    Row(usize),
    Bar(BarHit),
    Field,
    /// Anywhere else on the panel.
    Panel,
}

pub fn hit(m: &ClipHistory, x: i32, y: i32, cm: &ControlMetrics) -> Hit {
    let lay = layout(m, cm);
    if lay.field.contains(x, y) {
        return Hit::Field;
    }
    let ll = listbox::layout(&m.list, lay.list, cm);
    match listbox::hit(&m.list, &ll, x, y) {
        Some(listbox::Hit::Row(i) | listbox::Hit::Check(i)) => Hit::Row(i),
        Some(h) => h.bar().map_or(Hit::Panel, Hit::Bar),
        None => Hit::Panel,
    }
}

/// Draw the panel at `theme.scale` as a flyout; the canvas reaches the
/// look's shadow past the panel. `caret` is the blink's shown half,
/// `preedit` what an input method composes in the filter.
pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    m: &ClipHistory,
    caret: bool,
    preedit: Option<&Preedit>,
) -> Canvas {
    let p = &theme.paint();
    let (cs, s) = (p.cs, p.s);
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let lay = layout(m, &cm);
    let mut c = Canvas::new((width(&cm) * s) as u32, (lay.height * s) as u32);
    c.set_preedit(preedit.cloned());
    let all = c.bounds();
    flyout::draw_panel(
        theme,
        &mut c,
        all,
        cs.button_face,
        flyout::Backdrop::Blurred,
    );
    let inner = draw_edit_field(
        &mut c,
        p,
        lay.field.scaled(s),
        cs.window,
        true,
        Interaction::Idle,
    );
    let first = edit_first_visible(text, &m.edit, inner.width, px, 0);
    draw_edit_text(
        &mut c,
        text,
        &m.edit,
        inner,
        px,
        p,
        Caret::new(true, caret),
        first,
    );
    let ll = listbox::layout(&m.list, lay.list, &cm);
    listbox::draw(&mut c, text, p, &m.list, &ll, &m.icons, None, px);
    if m.rows.is_empty() {
        let r = ll.inner.scaled(s);
        let r = Rect::new(r.x + 3 * s, r.y, r.width - 3 * s, ll.row_h * s);
        text.draw(&mut c, r, &empty(), false, cs.gray_text, Align::Left, px);
    }
    flyout::shadowed(theme, c)
}

/// The panel's `access` part, in panel coordinates. Returns the node with
/// the focus: the highlighted row, else the field.
pub fn access(
    m: &ClipHistory,
    text: &TextRenderer,
    px: u32,
    tree: &mut crate::access::Tree,
    parent: crate::access::accesskit::NodeId,
    cm: &ControlMetrics,
) -> crate::access::accesskit::NodeId {
    let lay = layout(m, cm);
    let ll = listbox::layout(&m.list, lay.list, cm);
    let focus = listbox::access_filtered(
        &m.edit,
        &filter(),
        lay.field,
        &m.list,
        Some(&ll),
        text,
        px,
        tree,
        parent,
    );
    if m.rows.is_empty() {
        let r = Rect::new(ll.inner.x, ll.inner.y, ll.inner.width, ll.row_h);
        tree.push(parent, "empty", crate::access::label(r, &empty(), false));
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    #[test]
    fn at_200_percent_text_the_field_rows_and_width_grow_and_the_panel_pins_in_every_look() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let m = ClipHistory::new(rows(&["cargo build --release", "hello world"]));
        let big = Theme::default().with_text_size(200).controls();
        let lay = layout(&m, &big);
        assert_eq!(width(&big), 2 * W);
        assert_eq!(lay.field.height, 2 * FIELD_H);
        assert_eq!(lay.list.height, 2 * 2 * listbox::ROW_H + 4);
        let mut got = Vec::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = crate::interfaces::theme_for(look)
                    .with_text_size(200)
                    .scaled(scale);
                let text = TextRenderer::load(&theme.fonts);
                let c = draw(&theme, &text, &m, true, None);
                got.push((format!("{look:?}@{scale}"), crate::pins::fnv(&c)));
            }
        }
        crate::pins::check("clip_history_text_200", &got);
    }

    #[test]
    fn the_highlight_has_the_focus_and_an_empty_history_says_so() {
        let text = TextRenderer::load(&FontConfig::default());
        let m = ClipHistory::new(rows(&["newest", "older"]));
        let mut t = crate::access::Tree::window("", (W, 200));
        let focus = access(
            &m,
            &text,
            11,
            &mut t,
            crate::access::ROOT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(t.get(focus).unwrap().label(), Some("newest"));
        let empty = ClipHistory::new(Vec::new());
        let mut t = crate::access::Tree::window("", (W, 200));
        let focus = access(
            &empty,
            &text,
            11,
            &mut t,
            crate::access::ROOT,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(
            t.get(focus).unwrap().role(),
            crate::access::accesskit::Role::TextInput,
            "with no rows the field has the focus"
        );
        assert_eq!(t.get(focus).unwrap().label(), Some(filter().as_str()));
        assert!(
            t.nodes()
                .any(|(_, n)| n.value() == Some(super::empty().as_str())),
            "the empty history's label is there"
        );
    }

    /// The wheel scrolls three rows a notch, clamped to its ends.
    #[test]
    fn the_wheel_scrolls_the_list_and_keeps_the_highlight() {
        use crate::pointer::WHEEL_ROWS;
        let labels: Vec<String> = (0..15).map(|i| format!("entry {i}")).collect();
        let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&labels));
        assert!(!m.wheel(-WHEEL_ROWS), "already at the top");
        assert!(m.wheel(WHEEL_ROWS));
        assert_eq!(m.list.top, 3);
        assert!(m.wheel(WHEEL_ROWS));
        assert_eq!(m.list.top, 5, "15 rows, 10 in view: 5 is the last top");
        assert!(!m.wheel(WHEEL_ROWS), "at the bottom");
        assert!(m.wheel(-3 * WHEEL_ROWS));
        assert_eq!(m.list.top, 0);
        assert_eq!(m.selected(), Some(100), "the highlight stays");
        let mut short = ClipHistory::new(rows(&["a", "b"]));
        assert!(!short.wheel(WHEEL_ROWS), "nothing to scroll");
    }

    fn rows(labels: &[&str]) -> Vec<Row> {
        labels
            .iter()
            .enumerate()
            .map(|(i, l)| Row {
                id: 100 + i as u64,
                label: l.to_string(),
                icon: None,
            })
            .collect()
    }

    fn typed(m: &mut ClipHistory, s: &str) -> Event {
        m.key(Key::Char(EditKey::Insert(s.into())))
    }

    const DEL: Key = Key::Char(EditKey::Delete { word: false });

    #[test]
    fn opens_on_the_newest_row_and_enter_picks_the_highlight() {
        let mut m = ClipHistory::new(rows(&["newest", "older", "oldest"]));
        assert_eq!(m.list.items, vec!["newest", "older", "oldest"]);
        assert_eq!(m.selected(), Some(100));
        assert_eq!(m.key(Key::Down), Event::None);
        assert_eq!(m.key(Key::Down), Event::None);
        assert_eq!(m.key(Key::Down), Event::None);
        assert_eq!(m.key(Key::Enter), Event::Pick(102));
        assert_eq!(m.key(Key::Up), Event::None);
        assert_eq!(m.key(Key::Enter), Event::Pick(101));
        assert_eq!(m.key(Key::Escape), Event::Close);
    }

    #[test]
    fn typing_filters_best_match_first_and_backspace_restores_the_order() {
        let mut m = ClipHistory::new(rows(&["cargo build", "hello world", "git log"]));
        assert_eq!(typed(&mut m, "lo"), Event::None);
        assert_eq!(m.list.items, vec!["git log", "hello world"]);
        assert_eq!(m.selected(), Some(102));
        m.key(Key::Char(EditKey::Backspace { word: false }));
        m.key(Key::Char(EditKey::Backspace { word: false }));
        assert_eq!(m.list.items, vec!["cargo build", "hello world", "git log"]);
        typed(&mut m, "zzz");
        assert!(m.list.items.is_empty());
        assert_eq!(m.key(Key::Enter), Event::Close);
    }

    #[test]
    fn delete_removes_the_highlighted_row_and_keeps_the_place() {
        let mut m = ClipHistory::new(rows(&["a", "b", "c"]));
        m.key(Key::Down);
        assert_eq!(m.key(DEL), Event::Remove(101));
        assert_eq!(m.list.items, vec!["a", "c"]);
        assert_eq!(m.selected(), Some(102));
        assert_eq!(m.key(DEL), Event::Remove(102));
        assert_eq!(m.selected(), Some(100));
        assert_eq!(m.key(DEL), Event::Remove(100));
        assert!(m.rows().is_empty());
        typed(&mut m, "xy");
        m.key(Key::Char(EditKey::Home { select: false }));
        assert_eq!(m.key(DEL), Event::None);
        assert_eq!(m.edit().text, "y");
    }

    #[test]
    fn delete_under_a_filter_removes_the_match_not_the_first_row() {
        let mut m = ClipHistory::new(rows(&["alpha", "beta", "gamma"]));
        typed(&mut m, "gam");
        assert_eq!(m.key(DEL), Event::Remove(102));
        assert!(m.list.items.is_empty());
        assert_eq!(m.edit().text, "gam");
        assert_eq!(m.rows().len(), 2);
    }

    #[test]
    fn a_click_on_a_row_picks_it_and_the_arrows_scroll() {
        let labels: Vec<String> = (0..15).map(|i| format!("entry {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&refs));
        let lay = layout(&m, &ControlMetrics::CLASSIC);
        assert_eq!(
            hit(&m, 40, lay.list.y + 2 + 3, &ControlMetrics::CLASSIC),
            Hit::Row(0)
        );
        assert_eq!(
            hit(
                &m,
                40,
                lay.list.y + 2 + listbox::ROW_H + 3,
                &ControlMetrics::CLASSIC
            ),
            Hit::Row(1)
        );
        assert_eq!(
            hit(&m, 40, lay.field.y + 5, &ControlMetrics::CLASSIC),
            Hit::Field
        );
        assert_eq!(hit(&m, 1, 1, &ControlMetrics::CLASSIC), Hit::Panel);
        let arrow = hit(
            &m,
            lay.list.right() - 6,
            lay.list.y + 5,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(arrow, Hit::Bar(BarHit::Back));
        let down = hit(
            &m,
            lay.list.right() - 6,
            lay.list.bottom() - 5,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(down, Hit::Bar(BarHit::Forward));
        assert_eq!(m.click(down), Event::None);
        assert_eq!(m.list.top, 1);
        assert_eq!(m.click(Hit::Row(3)), Event::Pick(103));
        assert_eq!(m.click(Hit::Row(99)), Event::None);
    }

    #[test]
    fn a_thumb_dragged_to_the_bars_end_shows_the_last_row_and_the_release_lets_go() {
        let cm = ControlMetrics::CLASSIC;
        let labels: Vec<String> = (0..25).map(|i| format!("entry {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&refs));
        let lay = layout(&m, &cm);
        let ll = listbox::layout(&m.list, lay.list, &cm);
        let sb = ll.scrollbar.unwrap();
        let (x, y) = (sb.x + 3, sb.y + sb.width + 2);
        let press = hit(&m, x, y, &cm);
        assert!(matches!(press, Hit::Bar(BarHit::Thumb(_))), "{press:?}");
        assert_eq!(m.click(press), Event::None);
        assert!(m.holding());
        assert!(
            m.drag(x - 300, sb.bottom() + 40, &cm),
            "off the bar it still steers"
        );
        assert_eq!(m.list.top, 25 - VISIBLE, "the last row shows");
        assert!(m.release());
        assert!(!m.drag(x, y, &cm), "released: the list stays");
        assert_eq!(m.list.top, 25 - VISIBLE);
    }

    #[test]
    fn a_held_arrow_steps_on_the_hosts_ticks_while_the_pointer_is_on_it() {
        let cm = ControlMetrics::CLASSIC;
        let labels: Vec<String> = (0..25).map(|i| format!("entry {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&refs));
        let lay = layout(&m, &cm);
        let (x, y) = (lay.list.right() - 6, lay.list.bottom() - 5);
        m.click(hit(&m, x, y, &cm));
        assert_eq!(m.list.top, 1);
        let t0 = Instant::now();
        let later = t0 + crate::controls::scrollbar::REPEAT_AFTER;
        assert!(!m.tick(t0));
        m.drag(1, 1, &cm);
        assert!(!m.tick(later), "off the arrow");
        m.drag(x, y, &cm);
        assert!(m.tick(later + crate::controls::scrollbar::REPEAT_EVERY));
        assert_eq!(m.list.top, 2);
    }

    #[test]
    fn page_down_stops_on_the_last_row() {
        let labels: Vec<String> = (0..25).map(|i| format!("entry {i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&refs));
        m.key(Key::PageDown);
        assert_eq!(m.selected(), Some(109));
        m.key(Key::PageDown);
        m.key(Key::PageDown);
        assert_eq!(m.selected(), Some(124));
        m.key(Key::PageUp);
        assert_eq!(m.selected(), Some(115));
    }

    #[test]
    fn a_paste_filters_on_its_first_line() {
        let mut m = ClipHistory::new(rows(&["one", "two"]));
        m.paste("tw\nsecond line");
        assert_eq!(m.edit().text, "tw");
        assert_eq!(m.list.items, vec!["two"]);
    }

    #[test]
    fn the_list_is_as_tall_as_the_history_up_to_ten_rows() {
        let one = layout(&ClipHistory::new(Vec::new()), &ControlMetrics::CLASSIC);
        assert_eq!(one.field, Rect::new(4, 4, 352, 21));
        assert_eq!(one.list, Rect::new(4, 29, 352, 17 + 4));
        assert_eq!(one.height, 29 + 21 + 4);
        let three = ClipHistory::new(rows(&["a", "b", "c"]));
        assert_eq!(
            layout(&three, &ControlMetrics::CLASSIC).list.height,
            3 * 17 + 4
        );
        assert_eq!(layout(&three, &ControlMetrics::CLASSIC).height, 88);
        let many: Vec<String> = (0..25).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let mut m = ClipHistory::new(rows(&refs));
        assert_eq!(
            layout(&m, &ControlMetrics::CLASSIC).list.height,
            10 * 17 + 4
        );
        typed(&mut m, "24");
        assert_eq!(m.list.items.len(), 1);
        assert_eq!(
            layout(&m, &ControlMetrics::CLASSIC).list.height,
            10 * 17 + 4
        );
    }

    #[test]
    fn the_panel_is_raised_with_a_sunken_field_and_a_highlighted_first_row() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let mut thumb = Canvas::new(16, 16);
        thumb.fill_rect(thumb.bounds(), crate::theme::Color::rgb(0, 128, 0));
        let mut r = rows(&["hello", "Image, 640x480"]);
        r[1].icon = Some(thumb);
        let m = ClipHistory::new(r);
        let c = draw(&theme, &text, &m, true, None);
        let lay = layout(&m, &ControlMetrics::CLASSIC);
        assert_eq!((c.width, c.height), (W as u32, lay.height as u32));
        assert_eq!(c.get(0, 0), cs.button_face.argb());
        assert_eq!(c.get(1, 1), cs.button_hilight.argb());
        assert_eq!(c.get(W - 1, lay.height - 1), cs.button_dk_shadow.argb());
        assert_eq!(c.get(4, 4), cs.button_shadow.argb());
        assert_eq!(c.get(4 + 100, 4 + 10), cs.window.argb());
        // The list box: sunken at (4, 29), rows from (6, 31).
        assert_eq!(c.get(4, 29), cs.button_shadow.argb());
        assert_eq!(c.get(26, 31 + 2), cs.hilight.argb());
        assert_eq!(c.get(26, 31 + 17 + 2), cs.window.argb());
        assert_eq!(
            c.get(9 + 8, 31 + 17 + 8),
            crate::theme::Color::rgb(0, 128, 0).argb()
        );
    }

    #[test]
    fn an_empty_history_says_so_in_grey() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let m = ClipHistory::new(Vec::new());
        let c = draw(&theme, &text, &m, false, None);
        let grey = theme.colors.gray_text.argb();
        let found = (9..200).any(|x| (31..48).any(|y| c.get(x, y) == grey));
        assert!(found, "the empty label draws in gray_text");
    }

    #[test]
    #[ignore]
    fn render_cliphistory_preview() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let m = ClipHistory::new(rows(&[
            "cargo build --release -p reprise",
            "Image, 1920x1080",
            "hello world",
        ]));
        crate::raster::write_preview(
            &draw(&theme, &text, &m, true, None),
            "cliphistory-preview.png",
        );
    }
}
