//! The month grid: a framed white box with a band of weekday letters over six rows of day numbers, the picked day on the highlight and today outlined. The Date and Time sheet shows it bare between its month combo and year spinner; the taskbar clock's popup shows it under a title row whose two arrow buttons step the month.
//!
//! The frame is the look's ([`draw_calendar_frame`]). A look with a today box ([`draw_calendar_today`]) puts today's number in it in the look's link ink instead of the outline; one with bare month arrows ([`draw_calendar_arrow`]) draws them in place of buttons.
//!
//! Geometry of the 162x130 box: a 2 px frame, an 18 px band and six 18 px rows across 158 px inside. Column `i` starts `floor(45 i / 2)` px right of the inside's left edge less one (22 or 23 px wide); a weekday letter sits 6 px into its column and a day number is centred 12 px into it. Text boxes are `LINE_H` tall, 3 px below a row's top.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{Arrow, LINE_H, draw_focus_ring};
use crate::flyout;
use crate::look::Paint;
use crate::look::widgets::{
    draw_calendar_arrow, draw_calendar_band, draw_calendar_frame, draw_calendar_today,
};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;
use accesskit::{NodeId, Role};

pub const W: i32 = 162;
pub const H: i32 = 130;
pub const ROW_H: i32 = 18;
pub const ROWS: usize = 6;

pub const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const LETTERS: [&str; 7] = ["S", "M", "T", "W", "T", "F", "S"];

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        2 if is_leap(y) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The weekday of `y-m-d`, 0 for Sunday (Sakamoto's method).
pub fn weekday(y: i32, m: u32, d: u32) -> u32 {
    const T: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m < 3 { y - 1 } else { y };
    let w =
        y + y.div_euclid(4) - y.div_euclid(100) + y.div_euclid(400) + T[m as usize - 1] + d as i32;
    w.rem_euclid(7) as u32
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Calendar {
    pub year: i32,
    pub month: u32,
    /// The picked day, or none (the popup picks nothing).
    pub day: Option<u32>,
    pub today: Option<(i32, u32, u32)>,
    /// The column the week starts in: 0 Sunday .. 6 Saturday.
    pub first_weekday: u32,
    pub enabled: bool,
}

impl Calendar {
    pub fn new(year: i32, month: u32, day: Option<u32>, first_weekday: u32) -> Calendar {
        let mut c = Calendar {
            year,
            month: month.clamp(1, 12),
            day,
            today: None,
            first_weekday: first_weekday % 7,
            enabled: true,
        };
        c.clamp_day();
        c
    }

    fn clamp_day(&mut self) {
        let last = days_in_month(self.year, self.month);
        if let Some(d) = &mut self.day {
            *d = (*d).clamp(1, last);
        }
    }

    /// Show month `m` of the same year; the picked day stays, clamped to
    /// the month's last. Returns whether anything changed.
    pub fn set_month(&mut self, m: u32) -> bool {
        let before = (self.month, self.day);
        self.month = m.clamp(1, 12);
        self.clamp_day();
        before != (self.month, self.day)
    }

    pub fn set_year(&mut self, y: i32) -> bool {
        let before = (self.year, self.day);
        self.year = y;
        self.clamp_day();
        before != (self.year, self.day)
    }

    pub fn step_month(&mut self, delta: i32) {
        let k = self.year * 12 + self.month as i32 - 1 + delta;
        self.year = k.div_euclid(12);
        self.month = k.rem_euclid(12) as u32 + 1;
        self.clamp_day();
    }

    pub fn step_day(&mut self, delta: i32) -> bool {
        let Some(d) = self.day else {
            return false;
        };
        let last = days_in_month(self.year, self.month) as i32;
        let next = (d as i32 + delta).clamp(1, last) as u32;
        self.day = Some(next);
        next != d
    }

    pub fn pick(&mut self, d: u32) -> bool {
        let changed = self.day != Some(d);
        self.day = Some(d);
        self.clamp_day();
        changed
    }

    fn first_cell(&self) -> usize {
        ((weekday(self.year, self.month, 1) + 7 - self.first_weekday) % 7) as usize
    }

    pub fn day_at(&self, k: usize) -> Option<u32> {
        let d = k.checked_sub(self.first_cell())? as u32 + 1;
        (d <= days_in_month(self.year, self.month)).then_some(d)
    }

    pub fn cell_of(&self, d: u32) -> usize {
        self.first_cell() + d as usize - 1
    }

    pub fn title(&self) -> String {
        format!("{} {}", MONTHS[self.month as usize - 1], self.year)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub inner: Rect,
    pub band: Rect,
}

impl Layout {
    pub fn col_x(&self, i: usize) -> i32 {
        self.inner.x - 1 + 45 * i as i32 / 2
    }

    fn centre_x(&self, i: usize) -> i32 {
        self.col_x(i) + 12
    }

    pub fn row_y(&self, r: usize) -> i32 {
        self.band.bottom() + ROW_H * r as i32
    }

    pub fn cell(&self, k: usize) -> Rect {
        let (col, row) = (k % 7, k / 7);
        let x = self.col_x(col);
        let right = if col == 6 {
            self.inner.right()
        } else {
            self.col_x(col + 1)
        };
        Rect::new(x, self.row_y(row), right - x, ROW_H)
    }

    /// The text box day number `label` takes in cell `k`: its advance
    /// width by `LINE_H`, centred on the column's centre point.
    pub fn number_box(&self, k: usize, label_w: i32) -> Rect {
        let x = self.centre_x(k % 7) - label_w / 2;
        Rect::new(x, self.row_y(k / 7) + 3, label_w, LINE_H)
    }
}

pub fn layout(r: Rect) -> Layout {
    let inner = r.inset(2);
    Layout {
        frame: r,
        inner,
        band: Rect::new(inner.x, inner.y, inner.width, ROW_H),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Day(u32),
    Body,
}

pub fn hit(cal: &Calendar, lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    if !lay.frame.contains(x, y) {
        return None;
    }
    let day = (0..7 * ROWS)
        .find(|&k| lay.cell(k).contains(x, y))
        .and_then(|k| cal.day_at(k));
    Some(day.map_or(Hit::Body, Hit::Day))
}

pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    cal: &Calendar,
    lay: &Layout,
    focused: bool,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    // The look's frame, round `window` in use and round what sits behind
    // the box once greyed.
    let frame = lay.frame.scaled(s);
    if cal.enabled {
        c.fill_rect(frame, cs.window);
    } else {
        p.fill_back(c, frame);
    }
    draw_calendar_frame(c, p, frame, cal.enabled);
    let letters = draw_calendar_band(c, p, lay.band.scaled(s), cal.enabled);
    for i in 0..7 {
        let letter = LETTERS[(i as u32 + cal.first_weekday) as usize % 7];
        let r = Rect::new(lay.col_x(i) + 6, lay.band.y + 3, 12, LINE_H);
        text.draw(c, r.scaled(s), letter, false, letters, Align::Left, px);
    }
    let ink = if cal.enabled {
        cs.window_text
    } else {
        cs.gray_text
    };
    let today = cal
        .today
        .filter(|&(y, m, _)| (y, m) == (cal.year, cal.month))
        .map(|(_, _, d)| d);
    for d in 1..=days_in_month(cal.year, cal.month) {
        let k = cal.cell_of(d);
        let label = d.to_string();
        let b = lay.number_box(k, text.measure(&label, false, px) / s.max(1));
        let picked = cal.day == Some(d);
        let bs = b.scaled(s);
        let boxed = (today == Some(d) && !picked && cal.enabled)
            .then(|| draw_calendar_today(c, p, bs))
            .flatten();
        if let Some(today_ink) = boxed {
            text.draw(c, bs, &label, false, today_ink, Align::Left, px);
        } else if picked && cal.enabled {
            c.fill_rect(bs, cs.hilight);
            text.draw(c, bs, &label, false, cs.hilight_text, Align::Left, px);
            if focused {
                draw_focus_ring(c, bs, s, cs.hilight.inverted());
            }
        } else {
            text.draw(c, bs, &label, false, ink, Align::Left, px);
        }
        if today == Some(d) && !picked && boxed.is_none() {
            let ring = Rect::new(b.x - 1, b.y - 1, b.width + 2, b.height + 2).scaled(s);
            c.ring_t(ring, cs.hilight, cs.hilight, s);
        }
    }
}

/// The gap inside the popup's border ([`crate::flyout`]).
const PAD: i32 = 4;
/// The title row: two 16x16 arrow buttons and the month between them.
const TITLE_H: i32 = 16;
const ARROW_W: i32 = 16;

/// The popup panel's size in a look with controls `cm`: 174x162 inside
/// Classic's 2 px edge, 12 px more each way inside an 8 px border.
pub fn popup_size(cm: &ControlMetrics) -> (i32, i32) {
    let e = cm.flyout_border;
    (W + 2 * (e + PAD), TITLE_H + H + 2 * (e + PAD) + PAD)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupLayout {
    pub prev: Rect,
    pub next: Rect,
    pub title: Rect,
    pub grid: Layout,
}

pub fn popup_layout(cm: &ControlMetrics) -> PopupLayout {
    let x = cm.flyout_border + PAD;
    let y = x;
    PopupLayout {
        prev: Rect::new(x, y, ARROW_W, TITLE_H),
        next: Rect::new(x + W - ARROW_W, y, ARROW_W, TITLE_H),
        title: Rect::new(x + ARROW_W, y, W - 2 * ARROW_W, TITLE_H),
        grid: layout(Rect::new(x, y + TITLE_H + PAD, W, H)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupHit {
    Prev,
    Next,
    Panel,
}

pub fn popup_hit(cm: &ControlMetrics, x: i32, y: i32) -> PopupHit {
    let lay = popup_layout(cm);
    if lay.prev.contains(x, y) {
        PopupHit::Prev
    } else if lay.next.contains(x, y) {
        PopupHit::Next
    } else {
        PopupHit::Panel
    }
}

/// The popup panel at `theme`'s scale: the flyout's panel
/// ([`crate::flyout`]) round the title row and the grid, with its shadow
/// round it when the look has one. Nothing is picked; today is outlined.
/// The arrow `hot` names is drawn lit where the look lights it. Under a
/// look that frames its flyouts as windows the title is regular weight.
pub fn draw_popup(theme: &Theme, text: &TextRenderer, cal: &Calendar, hot: PopupHit) -> Canvas {
    let p = &theme.paint();
    let cs = p.cs;
    let s = p.s;
    let px = theme.fonts.size_px;
    let cm = theme.look.controls();
    let lay = popup_layout(&cm);
    let (w, h) = popup_size(&cm);
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    let all = c.bounds();
    flyout::draw_panel(
        theme,
        &mut c,
        all,
        cs.button_face,
        flyout::Backdrop::Blurred,
    );
    draw_calendar_arrow(
        &mut c,
        p,
        lay.prev.scaled(s),
        Arrow::Left,
        hot == PopupHit::Prev,
    );
    draw_calendar_arrow(
        &mut c,
        p,
        lay.next.scaled(s),
        Arrow::Right,
        hot == PopupHit::Next,
    );
    text.draw(
        &mut c,
        lay.title.scaled(s),
        &cal.title(),
        flyout::look(theme).is_none(),
        cs.button_text,
        Align::Center,
        px,
    );
    draw(&mut c, text, p, cal, &lay.grid, false, px);
    flyout::shadowed(theme, c)
}

/// The month grid's `access` part: a read-only date input whose text is
/// the picked day, "September 25, 2026", or the month alone while no day
/// is picked.
pub fn access(
    cal: &Calendar,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let mut n = access::node(Role::DateInput, lay.frame);
    access::focusable(&mut n, cal.enabled);
    let id = tree.push(parent, key, n);
    let month = MONTHS[(cal.month.clamp(1, 12) - 1) as usize];
    let value = match cal.day {
        Some(d) => format!("{month} {d}, {}", cal.year),
        None => cal.title(),
    };
    access::value_run(tree, id, lay.band, &value, 0..0, text, px);
    id
}

/// The popup's `access` part, in popup coordinates: the two month
/// buttons and the grid, which reads as today's date while today is in
/// the month shown. Returns the grid's node, which has the focus.
pub fn popup_access(
    cal: &Calendar,
    cm: &ControlMetrics,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let lay = popup_layout(cm);
    for (key, r, name) in [
        ("prev", lay.prev, "Previous month"),
        ("next", lay.next, "Next month"),
    ] {
        tree.push(parent, key, access::named(Role::Button, r, name));
    }
    let mut shown = cal.clone();
    shown.day = match cal.today {
        Some((y, m, d)) if y == cal.year && m == cal.month => Some(d),
        _ => None,
    };
    access(&shown, &lay.grid, text, px, tree, parent, "grid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::FontConfig;

    /// Blue's calendar box keeps the 2 px sunken edge in Blue's colours,
    /// in use and greyed: #ACA899 then #716F64 along the top and left,
    /// #F1EFE2 then white along the bottom and right. In use it is white
    /// inside; greyed on the Date and Time sheet's page, the body left of
    /// the day numbers shows the page, as it shows the face on the dialog.
    #[test]
    fn a_blue_calendar_box_has_the_sunken_edge_in_use_and_greyed() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let cs = theme.colors.clone();
        let face = cs.button_face;
        let p = theme.paint();
        let sheet = crate::controls::propsheet::PropSheet::new(
            &["Date & Time"],
            vec![crate::controls::form::Form::new(Vec::new())],
            (398, 317),
        );
        let page = sheet.frame().1;
        let lay = layout(Rect::new(27, 90, W, H));
        let f = lay.frame;
        let body = (lay.inner.x, lay.band.bottom() + 20);
        let mut bare = Canvas::new(398, 317);
        bare.fill_rect(bare.bounds(), face);
        crate::look::widgets::draw_page(&mut bare, &p, page);
        for enabled in [true, false] {
            let mut cal = Calendar::new(2026, 9, Some(28), 0);
            cal.enabled = enabled;
            let mut on_page = bare.clone();
            draw(&mut on_page, &text, &p.on_page(page), &cal, &lay, false, 11);
            for (x, y, argb) in [
                (f.x + 20, f.y, cs.button_shadow.argb()),
                (f.x, f.y + 60, cs.button_shadow.argb()),
                (f.x + 20, f.y + 1, cs.button_dk_shadow.argb()),
                (f.x + 1, f.y + 60, cs.button_dk_shadow.argb()),
                (f.x + 20, f.bottom() - 2, cs.button_light.argb()),
                (f.right() - 2, f.y + 60, cs.button_light.argb()),
                (f.x + 20, f.bottom() - 1, 0xFFFF_FFFF),
                (f.right() - 1, f.y + 60, 0xFFFF_FFFF),
            ] {
                assert_eq!(on_page.get(x, y), argb, "({x}, {y}), enabled {enabled}");
            }
            let inside = if enabled {
                0xFFFF_FFFF
            } else {
                bare.get(body.0, body.1)
            };
            assert_eq!(
                on_page.get(body.0, body.1),
                inside,
                "inside the edge, enabled {enabled}"
            );
        }
        let mut cal = Calendar::new(2026, 9, Some(28), 0);
        cal.enabled = false;
        let mut on_dialog = Canvas::new(398, 317);
        on_dialog.fill_rect(on_dialog.bounds(), face);
        draw(&mut on_dialog, &text, &p, &cal, &lay, false, 11);
        assert_eq!(
            on_dialog.get(body.0, body.1),
            face.argb(),
            "greyed on the dialog: the face"
        );
    }

    /// Blue's weekday band is the scheme's inactive title colour edge to edge under its letters, in use and greyed alike; the "S" starts 5 px into the band, 6 rows down.
    #[test]
    fn a_blue_calendar_band_is_blue_with_pale_letters_greyed_or_not() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        let lay = layout(Rect::new(27, 90, W, H));
        let b = lay.band;
        let letters = crate::blue::Variant::Blue
            .scheme()
            .inks
            .calendar_letters
            .argb();
        for enabled in [true, false] {
            let mut cal = Calendar::new(2026, 9, Some(29), 0);
            cal.enabled = enabled;
            let mut c = Canvas::new(200, 230);
            c.fill_rect(c.bounds(), theme.colors.button_face);
            draw(&mut c, &text, &p, &cal, &lay, false, 11);
            for (x, y) in [
                (b.x, b.y),
                (b.right() - 1, b.y),
                (b.x, b.bottom() - 1),
                (b.right() - 1, b.bottom() - 1),
                (b.x + 2, b.y + 6),
            ] {
                assert_eq!(
                    c.get(x, y),
                    theme.colors.inactive_title.argb(),
                    "({x}, {y}) is the band, {enabled}"
                );
            }
            let inked: Vec<(i32, i32)> = (b.y..b.bottom())
                .flat_map(|y| (b.x..b.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == letters)
                .collect();
            let top = inked.iter().map(|&(_, y)| y).min();
            let left = inked.iter().map(|&(x, _)| x).min();
            assert_eq!(top, Some(b.y + 6), "the letters' top row, {enabled}");
            assert_eq!(left, Some(b.x + 5), "the Sunday S's left edge, {enabled}");
        }
    }

    #[test]
    fn the_popup_reads_today_in_its_own_month_only() {
        let text = TextRenderer::load(&FontConfig::default());
        let value = |cal: &Calendar| {
            let cm = ControlMetrics::CLASSIC;
            let mut t = Tree::window("", popup_size(&cm));
            let grid = popup_access(cal, &cm, &text, 11, &mut t, access::ROOT);
            let run = t.get(grid).unwrap().children()[0];
            t.get(run).unwrap().value().map(str::to_owned)
        };
        let mut cal = Calendar::new(2026, 9, None, 0);
        cal.today = Some((2026, 9, 25));
        assert_eq!(value(&cal).as_deref(), Some("September 25, 2026"));
        cal.step_month(1);
        assert_eq!(value(&cal).as_deref(), Some(cal.title().as_str()));
    }

    #[test]
    fn weekdays_and_month_lengths() {
        assert_eq!(weekday(2026, 9, 1), 2, "1 September 2026 is a Tuesday");
        assert_eq!(weekday(2000, 1, 1), 6, "1 January 2000 is a Saturday");
        assert_eq!(weekday(2024, 2, 29), 4);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(2026, 9), 30);
    }

    #[test]
    fn the_first_weekday_shifts_the_grid() {
        let sun = Calendar::new(2026, 9, Some(24), 0);
        assert_eq!(sun.cell_of(1), 2, "Tuesday, third column from Sunday");
        assert_eq!(sun.cell_of(24), 25);
        let mon = Calendar::new(2026, 9, Some(24), 1);
        assert_eq!(mon.cell_of(1), 1);
        assert_eq!(mon.day_at(0), None);
        assert_eq!(mon.day_at(1), Some(1));
        assert_eq!(mon.day_at(30), Some(30));
        assert_eq!(mon.day_at(31), None);
    }

    #[test]
    fn stepping_keeps_the_day_in_its_month() {
        let mut c = Calendar::new(2026, 1, Some(31), 0);
        c.step_month(1);
        assert_eq!((c.year, c.month, c.day), (2026, 2, Some(28)));
        c.step_month(-2);
        assert_eq!((c.year, c.month), (2025, 12));
        assert!(c.set_year(2024) && c.set_month(2));
        assert_eq!(c.day, Some(28));
        assert!(!c.step_day(-100) || c.day == Some(1));
        assert_eq!(c.day, Some(1));
        assert!(!c.step_day(-1), "the first stays the first");
        assert!(c.step_day(7));
        assert_eq!(c.day, Some(8));
    }

    #[test]
    fn layout_and_hits() {
        let l = layout(Rect::new(27, 87, W, H));
        assert_eq!(l.inner, Rect::new(29, 89, 158, 126));
        assert_eq!(l.band, Rect::new(29, 89, 158, 18));
        let lefts: Vec<i32> = (0..7).map(|i| l.col_x(i)).collect();
        assert_eq!(lefts, [28, 50, 73, 95, 118, 140, 163]);
        assert_eq!(l.row_y(0), 107);
        assert_eq!(l.row_y(3), 161);
        assert_eq!(l.cell(6), Rect::new(163, 107, 24, 18));
        // "24" is 12 px wide: its box runs from x 124 on row 3, 3 px down.
        assert_eq!(l.number_box(25, 12), Rect::new(124, 164, 12, 13));
        let cal = Calendar::new(2026, 9, Some(24), 0);
        assert_eq!(hit(&cal, &l, 130, 170), Some(Hit::Day(24)));
        assert_eq!(hit(&cal, &l, 35, 110), Some(Hit::Body), "before the 1st");
        assert_eq!(hit(&cal, &l, 35, 95), Some(Hit::Body), "the band");
        assert_eq!(hit(&cal, &l, 20, 95), None);
    }

    #[test]
    fn draws_the_band_the_pick_and_today() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(Rect::new(27, 87, W, H));
        let mut cal = Calendar::new(2026, 9, Some(24), 0);
        cal.today = Some((2026, 9, 3));
        let mut c = Canvas::new(200, 230);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &cal, &l, false, 11);
        // Sunken edge: grey outer, dark inner at the top-left; white outer
        // at the bottom-right.
        assert_eq!(c.get(27, 87), cs.button_shadow.argb());
        assert_eq!(c.get(28, 88), cs.button_dk_shadow.argb());
        assert_eq!(c.get(188, 216), cs.button_hilight.argb());
        // The band is grey across rows 89..=106, white from 107.
        assert_eq!(c.get(30, 89), cs.button_shadow.argb());
        assert_eq!(c.get(30, 106), cs.button_shadow.argb());
        assert_eq!(c.get(30, 107), cs.window.argb());
        // The letters are face coloured inside the band.
        assert!((95..103).any(|y| (29..45).any(|x| c.get(x, y) == cs.button_face.argb())));
        // The 24th is on the highlight, rows 164..=176, as wide as the
        // font draws "24".
        let p = l.number_box(cal.cell_of(24), text.measure("24", false, 11));
        assert_eq!((p.y, p.bottom()), (164, 177));
        assert_eq!(c.get(p.x, 164), cs.hilight.argb());
        assert_eq!(c.get(p.right() - 1, 176), cs.hilight.argb());
        assert_eq!(c.get(p.x - 1, 164), cs.window.argb());
        assert_eq!(c.get(p.right(), 164), cs.window.argb());
        assert_eq!(c.get(p.x, 177), cs.window.argb());
        // Today, the 3rd, has a one-pixel ring round its number box.
        let b = l.number_box(cal.cell_of(3), text.measure("3", false, 11));
        assert_eq!(c.get(b.x - 1, b.y - 1), cs.hilight.argb());
        assert_eq!(c.get(b.x, b.y), cs.window.argb());
    }

    #[test]
    fn the_popup_steps_by_its_arrows() {
        let cm = ControlMetrics::CLASSIC;
        let lay = popup_layout(&cm);
        assert_eq!(popup_size(&cm), (174, 162));
        assert_eq!(
            popup_hit(&cm, lay.prev.x + 2, lay.prev.y + 2),
            PopupHit::Prev
        );
        assert_eq!(
            popup_hit(&cm, lay.next.x + 2, lay.next.y + 2),
            PopupHit::Next
        );
        assert_eq!(popup_hit(&cm, 80, 100), PopupHit::Panel);
        // Inside an 8 px border everything moves 6 px in and the panel
        // grows by 12 each way.
        let glass = ControlMetrics {
            flyout_border: 8,
            ..cm
        };
        assert_eq!(popup_size(&glass), (186, 174));
        assert_eq!(popup_layout(&glass).prev, Rect::new(12, 12, 16, 16));
        assert_eq!(popup_hit(&glass, 13, 13), PopupHit::Prev);
        assert_eq!(Calendar::new(2026, 9, None, 0).title(), "September 2026");
    }

    #[test]
    #[ignore]
    fn render_popup() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut cal = Calendar::new(2026, 9, None, 0);
        cal.today = Some((2026, 9, 24));
        let c = draw_popup(&theme, &text, &cal, PopupHit::Panel);
        crate::raster::write_preview(&c, "calendar-popup-preview.png");
    }
}
