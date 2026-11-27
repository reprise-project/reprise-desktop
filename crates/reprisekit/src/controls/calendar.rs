//! The month grid: a framed white box with a band of weekday letters over six rows of day numbers, the picked day on the highlight and today outlined. The Date and Time sheet shows it bare between its month combo and year spinner; the taskbar clock's popup shows it under a title row whose two arrow buttons step the month, and over the picked day's events when the host has a calendar to read ([`agenda`]).
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
    draw_clock_face, draw_link,
};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{TextUnits, Theme};
use accesskit::{NodeId, Role};

pub mod agenda;
pub use agenda::{AGENDA_ROWS, Agenda, DayEvent};

pub const W: i32 = 162;
pub const H: i32 = 130;
pub const ROW_H: i32 = 18;
pub const ROWS: usize = 6;

/// Month and weekday names in the language of the LC_TIME locale, which the host reads
/// (reprisefs's `locale::date_names`). [`DateNames::english`] until a host sets them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DateNames {
    /// January first, each as it stands alone in a title or a list.
    pub months: [String; 12],
    /// Sunday first.
    pub weekdays: [String; 7],
    /// The band's one-letter column heads, Sunday first.
    pub letters: [String; 7],
    /// The text between year and month in a locale that writes the year first (". " gives
    /// "2026. november"); None titles a month as `calendar-title` orders it.
    pub year_first: Option<String>,
    /// The order the locale's short date writes day, month and year in, which a date the
    /// calendar words follows.
    pub order: DateOrder,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DateOrder {
    #[default]
    Mdy,
    Dmy,
    Ymd,
}

impl DateOrder {
    /// The `$order` a date message selects on.
    pub fn key(self) -> &'static str {
        match self {
            DateOrder::Mdy => "mdy",
            DateOrder::Dmy => "dmy",
            DateOrder::Ymd => "ymd",
        }
    }
}

impl DateNames {
    pub fn english() -> &'static DateNames {
        static EN: std::sync::OnceLock<DateNames> = std::sync::OnceLock::new();
        EN.get_or_init(|| DateNames {
            months: [
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
            ]
            .map(String::from),
            weekdays: [
                "Sunday",
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
            ]
            .map(String::from),
            letters: ["S", "M", "T", "W", "T", "F", "S"].map(String::from),
            year_first: None,
            order: DateOrder::Mdy,
        })
    }

    /// What a year-first date puts between the year and the month.
    fn year_sep(&self) -> &str {
        self.year_first.as_deref().unwrap_or(" ")
    }

    /// Month `m` (1 to 12, clamped) by name.
    pub fn month(&self, m: u32) -> &str {
        &self.months[m.clamp(1, 12) as usize - 1]
    }

    /// Weekday `n` (0 Sunday, taken mod 7) by name.
    pub fn weekday(&self, n: u32) -> &str {
        &self.weekdays[n as usize % 7]
    }
}

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
    /// The time of day the popup's analog clock shows: hour 0..23, minute, second.
    pub now: Option<(u32, u32, u32)>,
    /// The popup's time line under the clock, worded by the host in the locale's format.
    pub time: Option<String>,
    /// The popup's date line over the month: today's date as the host words it, longest first.
    /// The line shows the first wording that fits; none hides it.
    pub date: Vec<String>,
    /// The popup's events under the month; none hides the list.
    pub agenda: Option<Agenda>,
    /// The column the week starts in: 0 Sunday .. 6 Saturday.
    pub first_weekday: u32,
    pub names: &'static DateNames,
    pub enabled: bool,
}

impl Calendar {
    pub fn new(year: i32, month: u32, day: Option<u32>, first_weekday: u32) -> Calendar {
        let mut c = Calendar {
            year,
            month: month.clamp(1, 12),
            day,
            today: None,
            now: None,
            time: None,
            date: Vec::new(),
            agenda: None,
            first_weekday: first_weekday % 7,
            names: DateNames::english(),
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

    /// Day `d` of the month shown, worded in the locale's date order.
    pub fn date_text(&self, d: u32) -> String {
        tr!(
            "calendar-date",
            order = self.names.order.key(),
            month = self.names.month(self.month),
            day = d as i64,
            year = self.year as i64,
            sep = self.names.year_sep()
        )
    }

    pub fn title(&self) -> String {
        let month = self.names.month(self.month);
        match &self.names.year_first {
            Some(sep) => format!("{}{sep}{month}", self.year),
            None => tr!("calendar-title", month = month, year = self.year as i64),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub frame: Rect,
    pub inner: Rect,
    pub band: Rect,
    /// How the grid's 100% columns, rows and text boxes grow with the text; the frame and the
    /// today ring keep their px.
    pub text: TextUnits,
}

impl Layout {
    /// Where column `i` starts, `dx` 100% px further right.
    fn col_at(&self, i: usize, dx: i32) -> i32 {
        self.inner.x - 1 + self.text.len(45 * i as i32 / 2 + dx)
    }

    pub fn col_x(&self, i: usize) -> i32 {
        self.col_at(i, 0)
    }

    fn centre_x(&self, i: usize) -> i32 {
        self.col_at(i, 12)
    }

    pub fn row_y(&self, r: usize) -> i32 {
        self.inner.y + self.text.len(ROW_H * (r as i32 + 1))
    }

    pub fn cell(&self, k: usize) -> Rect {
        let (col, row) = (k % 7, k / 7);
        let x = self.col_x(col);
        let right = if col == 6 {
            self.inner.right()
        } else {
            self.col_x(col + 1)
        };
        let y = self.row_y(row);
        Rect::new(x, y, right - x, self.row_y(row + 1) - y)
    }

    /// The text box day number `label` takes in cell `k`: its advance
    /// width by a line, centred on the column's centre point.
    pub fn number_box(&self, k: usize, label_w: i32) -> Rect {
        let x = self.centre_x(k % 7) - label_w / 2;
        let y = self.row_y(k / 7) + self.text.len(3);
        Rect::new(x, y, label_w, self.text.len(LINE_H))
    }

    /// Weekday letter `i`'s text box in the band.
    fn letter_box(&self, i: usize) -> Rect {
        let x = self.col_at(i, 6);
        let y = self.band.y + self.text.len(3);
        Rect::new(x, y, self.col_at(i, 18) - x, self.text.len(LINE_H))
    }
}

/// The grid in `r`, its rows and columns grown by `cm.text`.
pub fn layout(r: Rect, cm: &ControlMetrics) -> Layout {
    let inner = r.inset(2);
    Layout {
        frame: r,
        inner,
        band: Rect::new(inner.x, inner.y, inner.width, cm.text.len(ROW_H)),
        text: cm.text,
    }
}

/// The grid's size at the text size `cm.text`.
pub fn size(cm: &ControlMetrics) -> (i32, i32) {
    cm.text.size((W, H))
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
        let letter = &cal.names.letters[(i as u32 + cal.first_weekday) as usize % 7];
        text.draw(
            c,
            lay.letter_box(i).scaled(s),
            letter,
            false,
            letters,
            Align::Left,
            px,
        );
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
/// The date line's row over the title row, and the settings link's at the foot.
const DATE_H: i32 = 16;
const LINK_H: i32 = 16;
/// The title row: two 16x16 arrow buttons and the month between them.
const TITLE_H: i32 = 16;
/// The link at the popup's foot, which opens Date and Time Properties.
pub fn settings() -> String {
    tr!("calendar-settings")
}
const ARROW_W: i32 = 16;

/// From the grid's right edge to the analog clock's.
const CLOCK_GAP: i32 = 12;
/// The time line under the clock: its row and its text, a size up from the title.
const TIME_H: i32 = 24;
pub const TIME_PX: u32 = 20;

/// The analog clock's side at `cm.text`, kept odd so its middle is a pixel's; 0 in a look
/// without one.
fn clock_side(cm: &ControlMetrics) -> i32 {
    match cm.clock_face {
        0 => 0,
        side => cm.text.len(side) | 1,
    }
}

/// The popup panel's size in a look with controls `cm`: 174x202 inside
/// Classic's 2 px edge (the date line, the title row, the grid and the link),
/// 12 px more each way inside an 8 px border, in a look with an analog clock
/// wider by the clock and its gap and taller by the time line, and with an
/// `agenda` taller by its heading and rows. Everything but the border grows
/// with `cm.text`.
pub fn popup_size(cm: &ControlMetrics, agenda: bool) -> (i32, i32) {
    let e = cm.flyout_border;
    let (w, h) = size(cm);
    let (clock, time) = match clock_side(cm) {
        0 => (0, 0),
        side => (cm.text.len(CLOCK_GAP) + side, cm.text.len(TIME_H) + PAD),
    };
    let list = match agenda {
        true => PAD + agenda::height(cm),
        false => 0,
    };
    let rows = cm.text.len(DATE_H) + PAD + cm.text.len(TITLE_H) + PAD + cm.text.len(LINK_H);
    (
        w + clock + 2 * (e + PAD),
        rows + h + time + list + 2 * (e + PAD) + PAD,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PopupLayout {
    /// Today's date across the panel's top.
    pub date: Rect,
    pub prev: Rect,
    pub next: Rect,
    pub title: Rect,
    pub grid: Layout,
    /// The analog clock's square right of the grid, centred on it; none in a look without one.
    pub clock: Option<Rect>,
    /// The time line's row under the grid, as wide as the clock and centred under it.
    pub time: Option<Rect>,
    /// The event list across the panel under the month, when the popup has one.
    pub agenda: Option<agenda::Layout>,
    /// The [`SETTINGS`] link's row across the panel's foot.
    pub settings: Rect,
}

pub fn popup_layout(cm: &ControlMetrics, with_agenda: bool) -> PopupLayout {
    let x = cm.flyout_border + PAD;
    let date_h = cm.text.len(DATE_H);
    let y = x + date_h + PAD;
    let (w, h) = size(cm);
    let title_h = cm.text.len(TITLE_H);
    let grid = Rect::new(x, y + title_h + PAD, w, h);
    let side = clock_side(cm);
    let clock = (side > 0).then(|| {
        Rect::new(
            grid.right() + cm.text.len(CLOCK_GAP),
            grid.y + (h - side) / 2,
            side,
            side,
        )
    });
    let time = clock.map(|r| Rect::new(r.x, grid.bottom() + PAD, r.width, cm.text.len(TIME_H)));
    let top = time.map_or(grid.bottom(), |t| t.bottom()) + PAD;
    let width = popup_size(cm, false).0 - 2 * x;
    let foot = match with_agenda {
        true => top + agenda::height(cm),
        false => top - PAD,
    };
    PopupLayout {
        date: Rect::new(x, x, width, date_h),
        prev: Rect::new(x, y, ARROW_W, title_h),
        next: Rect::new(x + w - ARROW_W, y, ARROW_W, title_h),
        title: Rect::new(x + ARROW_W, y, w - 2 * ARROW_W, title_h),
        grid: layout(grid, cm),
        clock,
        time,
        agenda: with_agenda.then(|| agenda::layout(Rect::new(x, top, width, 0), cm)),
        settings: Rect::new(x, foot + PAD, width, cm.text.len(LINK_H)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupHit {
    Prev,
    Next,
    /// A day of the month shown, which picks it for the event list; only with an agenda.
    Day(u32),
    /// Row `n` of the event list, an event or its "more" row; only when a click opens them.
    Event(usize),
    /// The [`SETTINGS`] link's row.
    Settings,
    Panel,
}

/// What point (`x`, `y`) of the popup showing `cal` lands on.
pub fn popup_hit(cm: &ControlMetrics, cal: &Calendar, x: i32, y: i32) -> PopupHit {
    let lay = popup_layout(cm, cal.agenda.is_some());
    if lay.prev.contains(x, y) {
        return PopupHit::Prev;
    }
    if lay.next.contains(x, y) {
        return PopupHit::Next;
    }
    if lay.settings.contains(x, y) {
        return PopupHit::Settings;
    }
    let (Some(a), Some(list)) = (&cal.agenda, &lay.agenda) else {
        return PopupHit::Panel;
    };
    if let Some(Hit::Day(d)) = hit(cal, &lay.grid, x, y) {
        return PopupHit::Day(d);
    }
    agenda::hit(a, list, x, y).map_or(PopupHit::Panel, PopupHit::Event)
}

/// The popup panel at `theme`'s scale: the flyout's panel
/// ([`crate::flyout`]) round the date line of `cal.date`, the title row, the grid
/// and, in a look with one, the analog clock at `cal.now` over the time line
/// `cal.time`, then the event list of `cal.agenda` and the centred
/// [`SETTINGS`] link, with its shadow round it when the look has one. Today is
/// outlined; a listed day other than today is picked. The arrow or event row
/// `hot` names is drawn lit where the look lights it; the link underlines
/// while hot. Under a look that frames its flyouts as windows the title and
/// the list's heading are regular weight.
pub fn draw_popup(theme: &Theme, text: &TextRenderer, cal: &Calendar, hot: PopupHit) -> Canvas {
    let mut c = draw_popup_base(theme, text, cal, hot);
    if let Some(now) = cal.now {
        let base = c.clone();
        draw_popup_clock(theme, &mut c, &base, now);
    }
    c
}

/// The analog clock's square in a canvas [`draw_popup`] drew at `theme`'s scale, device px;
/// none in a look without one.
pub fn popup_clock(theme: &Theme) -> Option<Rect> {
    let s = theme.scale.max(1);
    let m = flyout::margin(theme).scaled(s);
    let r = popup_layout(&theme.controls(), false).clock?.scaled(s);
    Some(Rect::new(r.x + m.left, r.y + m.top, r.width, r.height))
}

/// Moves the analog clock in `c`, a popup drawn from `base` ([`draw_popup_base`]), to `now`:
/// only the pixels inside [`popup_clock`] change.
pub fn draw_popup_clock(theme: &Theme, c: &mut Canvas, base: &Canvas, now: (u32, u32, u32)) {
    if let Some(r) = popup_clock(theme) {
        c.clear_rect(r);
        c.blit_over(r.x, r.y, &base.copy_rect(r));
        draw_clock_face(c, &theme.paint(), r, now);
    }
}

/// [`draw_popup`] without the analog clock, for [`draw_popup_clock`] to draw over.
pub fn draw_popup_base(
    theme: &Theme,
    text: &TextRenderer,
    cal: &Calendar,
    hot: PopupHit,
) -> Canvas {
    let p = &theme.paint();
    let cs = p.cs;
    let s = p.s;
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let lay = popup_layout(&cm, cal.agenda.is_some());
    let (w, h) = popup_size(&cm, cal.agenda.is_some());
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
    let date_row = lay.date.scaled(s);
    let fits = |d: &&String| text.measure(d, false, px) <= date_row.width;
    if let Some(date) = cal.date.iter().find(fits).or(cal.date.last()) {
        let date = text.ellipsize(date, date_row.width, false, px);
        text.draw(
            &mut c,
            date_row,
            &date,
            false,
            cs.button_text,
            Align::Center,
            px,
        );
    }
    let link = lay.settings.scaled(s);
    let lw = text.measure(&settings(), false, px);
    let at = Rect::new(
        link.x + (link.width - lw).max(0) / 2,
        link.y,
        lw,
        link.height,
    );
    draw_link(
        &mut c,
        text,
        p,
        at,
        &settings(),
        hot == PopupHit::Settings,
        px,
    );
    draw(&mut c, text, p, &picked(cal), &lay.grid, false, px);
    if let (Some(a), Some(list)) = (&cal.agenda, &lay.agenda) {
        let bold = flyout::look(theme).is_none();
        agenda::draw(&mut c, text, p, a, cal, list, hot, bold, px);
    }
    if let (Some(r), Some(time)) = (lay.time, &cal.time) {
        let size = cm.text.px(TIME_PX) * s as u32;
        text.draw(
            &mut c,
            r.scaled(s),
            time,
            false,
            cs.button_text,
            Align::Center,
            size,
        );
    }
    flyout::shadowed(theme, c)
}

/// `cal` as the popup's grid shows it: the agenda's day picked when it is in the month shown and is
/// not today, which stays outlined.
fn picked(cal: &Calendar) -> Calendar {
    let mut shown = cal.clone();
    shown.day = match &cal.agenda {
        Some(a) if (a.day.0, a.day.1) == (cal.year, cal.month) && Some(a.day) != cal.today => {
            Some(a.day.2)
        }
        _ => None,
    };
    shown
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
    n.set_read_only();
    let id = tree.push(parent, key, n);
    let value = match cal.day {
        Some(d) => cal.date_text(d),
        None => cal.title(),
    };
    access::value_run(tree, id, lay.band, &value, 0..0, text, px);
    id
}

/// The popup's `access` part, in popup coordinates: the date line, the two month
/// buttons, which a reader can click, the analog clock where the look has one, the grid, which
/// reads as the listed day's date (else today's) while it is in the month shown, the event
/// list and the [`SETTINGS`] link. Returns the grid's node, which has the focus.
pub fn popup_access(
    cal: &Calendar,
    cm: &ControlMetrics,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let lay = popup_layout(cm, cal.agenda.is_some());
    if let Some(date) = cal.date.first() {
        tree.push(parent, "date", access::named(Role::Label, lay.date, date));
    }
    for (key, r, name) in [
        ("prev", lay.prev, tr!("calendar-previous-month")),
        ("next", lay.next, tr!("calendar-next-month")),
    ] {
        let mut n = access::named(Role::Button, r, &name);
        n.add_action(accesskit::Action::Click);
        tree.push(parent, key, n);
    }
    if let Some(r) = lay.clock {
        tree.push(
            parent,
            "clock",
            access::named(Role::Image, r, &tr!("calendar-clock")),
        );
    }
    if let (Some(r), Some(time)) = (lay.time, &cal.time) {
        tree.push(parent, "time", access::named(Role::Label, r, time));
    }
    let mut shown = cal.clone();
    let day = cal.agenda.as_ref().map(|a| a.day).or(cal.today);
    shown.day = match day {
        Some((y, m, d)) if y == cal.year && m == cal.month => Some(d),
        _ => None,
    };
    let grid = access(&shown, &lay.grid, text, px, tree, parent, "grid");
    if let (Some(a), Some(list)) = (&cal.agenda, &lay.agenda) {
        agenda::access(a, cal, list, tree, parent);
    }
    let mut link = access::named(Role::Link, lay.settings, &settings());
    link.add_action(accesskit::Action::Click);
    tree.push(parent, "settings", link);
    grid
}

#[cfg(test)]
mod tests {
    #[test]
    fn month_and_date_texts_read_from_the_names_and_messages() {
        let en = DateNames::english();
        assert_eq!(en.month(1), "January");
        assert_eq!(en.month(12), "December");
        assert_eq!(en.month(0), "January", "clamped");
        assert_eq!(en.weekday(7), "Sunday", "mod 7");
        assert_eq!(
            Calendar::new(2026, 3, None, 0).date_text(5),
            "March 5, 2026",
            "no digit grouping in a year"
        );
    }

    use super::*;
    use crate::look::classic_paint;
    use crate::theme::FontConfig;

    fn bare() -> Calendar {
        Calendar::new(2026, 9, None, 0)
    }

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
        let lay = layout(Rect::new(27, 90, W, H), &ControlMetrics::CLASSIC);
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
        crate::pins::fonts();
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let text = TextRenderer::load(&FontConfig::default());
        let p = theme.paint();
        let lay = layout(Rect::new(27, 90, W, H), &ControlMetrics::CLASSIC);
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
            let mut t = Tree::window("", popup_size(&cm, false));
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
    fn a_reader_can_click_the_month_buttons() {
        let text = TextRenderer::load(&FontConfig::default());
        let cm = ControlMetrics::CLASSIC;
        let mut t = Tree::window("", popup_size(&cm, false));
        popup_access(
            &Calendar::new(2026, 9, None, 0),
            &cm,
            &text,
            11,
            &mut t,
            access::ROOT,
        );
        for name in ["Previous month", "Next month"] {
            let id = t.find(Role::Button, name).unwrap();
            assert!(
                t.get(id).unwrap().supports_action(accesskit::Action::Click),
                "{name} offers Click"
            );
        }
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
        let l = layout(Rect::new(27, 87, W, H), &ControlMetrics::CLASSIC);
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
        let l = layout(Rect::new(27, 87, W, H), &ControlMetrics::CLASSIC);
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
    fn a_year_first_locale_titles_the_month_after_the_year() {
        let names: &'static DateNames = Box::leak(Box::new(DateNames {
            months: std::array::from_fn(|m| format!("m{}", m + 1)),
            year_first: Some(". ".into()),
            ..DateNames::english().clone()
        }));
        let cal = Calendar {
            names,
            ..Calendar::new(2026, 11, None, 1)
        };
        assert_eq!(cal.title(), "2026. m11");
    }

    #[test]
    fn the_agenda_heading_and_the_picked_date_follow_the_locales_date_order() {
        let with = |order, year_first: Option<&str>| -> &'static DateNames {
            Box::leak(Box::new(DateNames {
                order,
                year_first: year_first.map(String::from),
                ..DateNames::english().clone()
            }))
        };
        let today = Some((2026, 11, 13));
        let (sat, next_year) = (
            Agenda::new((2026, 11, 14), true),
            Agenda::new((2027, 1, 1), true),
        );
        let picked = |names| {
            Calendar {
                names,
                ..Calendar::new(2026, 11, None, 0)
            }
            .date_text(14)
        };

        let mdy = with(DateOrder::Mdy, None);
        assert_eq!(sat.heading(today, mdy), "Saturday, November 14");
        assert_eq!(next_year.heading(today, mdy), "Friday, January 1, 2027");
        assert_eq!(picked(mdy), "November 14, 2026");

        let dmy = with(DateOrder::Dmy, None);
        assert_eq!(sat.heading(today, dmy), "Saturday 14 November");
        assert_eq!(next_year.heading(today, dmy), "Friday 1 January 2027");
        assert_eq!(picked(dmy), "14 November 2026");

        let ymd = with(DateOrder::Ymd, Some(". "));
        assert_eq!(sat.heading(today, ymd), "Saturday, November 14");
        assert_eq!(next_year.heading(today, ymd), "Friday, 2027. January 1");
        assert_eq!(picked(ymd), "2026. November 14");
    }

    #[test]
    fn the_band_title_and_heading_take_the_names_the_host_gives() {
        let names: &'static DateNames = Box::leak(Box::new(DateNames {
            months: std::array::from_fn(|m| format!("m{}", m + 1)),
            weekdays: std::array::from_fn(|d| format!("w{d}")),
            letters: ["V", "H", "K", "S", "C", "P", "S"].map(String::from),
            year_first: None,
            order: DateOrder::Mdy,
        }));
        let en = Calendar::new(2026, 11, Some(27), 0);
        let local = Calendar {
            names,
            ..en.clone()
        };
        assert_eq!(local.title(), "m11 2026");
        let a = Agenda::new((2026, 11, 28), true);
        assert_eq!(a.heading(None, names), "w6, m11 28");

        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let l = layout(Rect::new(0, 0, W, H), &ControlMetrics::CLASSIC);
        let drawn = |cal: &Calendar| {
            let mut c = Canvas::new(W as u32, H as u32);
            draw(&mut c, &text, &classic_paint(cs, 1), cal, &l, false, 11);
            c
        };
        let (en, local) = (drawn(&en), drawn(&local));
        let letter = |c: &Canvas, i: usize| {
            let r = l.letter_box(i);
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)))
                .collect::<Vec<_>>()
        };
        assert_ne!(letter(&en, 1), letter(&local, 1), "Monday's M becomes H");
        assert_eq!(letter(&en, 6), letter(&local, 6), "Saturday's S stays");
        assert!(
            (l.band.bottom()..H).all(|y| (0..W).all(|x| en.get(x, y) == local.get(x, y))),
            "the days draw the same"
        );
    }

    /// At 150% and 200% the grid's columns, rows and number boxes grow with the text, the 2 px
    /// sunken edge keeps its px, and the picked day's highlight is the grown number box.
    #[test]
    fn a_grown_grid_grows_its_cells_and_keeps_its_edge() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        for (pct, px) in [(100, 11), (150, 17), (200, 22)] {
            let u = crate::theme::TextUnits::new(px, 11);
            let cm = ControlMetrics::CLASSIC.grown(u);
            let frame = u.rect(Rect::new(27, 87, W, H));
            let l = layout(frame, &cm);
            assert_eq!(l.band.height, u.len(ROW_H), "{pct}%");
            assert_eq!(l.row_y(0), l.inner.y + u.len(18), "{pct}%");
            assert_eq!(l.cell(1).height, u.len(36) - u.len(18), "{pct}%");
            assert_eq!(l.col_x(3), l.inner.x - 1 + u.len(67), "{pct}%");
            assert_eq!(l.cell(6).right(), l.inner.right(), "{pct}%");
            let mut cal = Calendar::new(2026, 9, Some(24), 0);
            cal.today = Some((2026, 9, 3));
            let mut c = Canvas::new(500, 500);
            c.fill_rect(c.bounds(), cs.button_face);
            let p = Paint {
                text: u,
                ..classic_paint(cs, 1)
            };
            draw(&mut c, &text, &p, &cal, &l, false, px);
            assert_eq!(c.get(frame.x, frame.y), cs.button_shadow.argb());
            assert_eq!(c.get(frame.x + 1, frame.y + 1), cs.button_dk_shadow.argb());
            assert_eq!(
                c.get(l.inner.x + 1, l.band.bottom() - 1),
                cs.button_shadow.argb()
            );
            assert_eq!(c.get(l.inner.x + 1, l.band.bottom()), cs.window.argb());
            let b = l.number_box(cal.cell_of(24), text.measure("24", false, px));
            assert_eq!(b.height, u.len(LINE_H), "{pct}%");
            assert_eq!(c.get(b.x, b.y), cs.hilight.argb(), "{pct}%");
            assert_eq!(
                c.get(b.right() - 1, b.bottom() - 1),
                cs.hilight.argb(),
                "{pct}%"
            );
            assert_eq!(c.get(b.x, b.bottom()), cs.window.argb(), "{pct}%");
            let cell = l.cell(cal.cell_of(24));
            assert_eq!(
                hit(&cal, &l, cell.x + 2, cell.bottom() - 1),
                Some(Hit::Day(24))
            );
        }
    }

    /// The popup's grid and title row grow with the text in a theme at 200%; its border and arrow
    /// buttons' widths keep their px.
    #[test]
    fn a_grown_popup_grows_its_grid_and_title_row() {
        let u = crate::theme::TextUnits::new(22, 11);
        let cm = ControlMetrics::CLASSIC.grown(u);
        let lay = popup_layout(&cm, false);
        assert_eq!(
            popup_size(&cm, false),
            (2 * W + 12, 2 * 16 + 2 * H + 16 + 2 * (32 + 4))
        );
        assert_eq!(lay.date, Rect::new(6, 6, 2 * W, 32));
        assert_eq!(lay.prev, Rect::new(6, 42, 16, 32));
        assert_eq!(lay.grid.frame, Rect::new(6, 78, 2 * W, 2 * H));
        assert_eq!(lay.settings.height, 32);
        assert_eq!(
            popup_hit(&cm, &bare(), lay.next.x + 2, lay.next.y + 30),
            PopupHit::Next
        );
    }

    #[test]
    fn the_popup_steps_by_its_arrows() {
        let cm = ControlMetrics::CLASSIC;
        let lay = popup_layout(&cm, false);
        assert_eq!(popup_size(&cm, false), (174, 202));
        assert_eq!(
            popup_hit(&cm, &bare(), lay.prev.x + 2, lay.prev.y + 2),
            PopupHit::Prev
        );
        assert_eq!(
            popup_hit(&cm, &bare(), lay.next.x + 2, lay.next.y + 2),
            PopupHit::Next
        );
        assert_eq!(popup_hit(&cm, &bare(), 80, 100), PopupHit::Panel);
        // Inside an 8 px border everything moves 6 px in and the panel
        // grows by 12 each way.
        let glass = ControlMetrics {
            flyout_border: 8,
            ..cm
        };
        assert_eq!(popup_size(&glass, false), (186, 214));
        assert_eq!(popup_layout(&glass, false).prev, Rect::new(12, 32, 16, 16));
        assert_eq!(popup_hit(&glass, &bare(), 13, 33), PopupHit::Prev);
        assert_eq!(Calendar::new(2026, 9, None, 0).title(), "September 2026");
    }

    /// Under Glass the clock stands right of the grid, level with it, and widens the popup; the
    /// time line's row under it makes the popup 28 px taller. Classic has neither.
    #[test]
    fn a_glass_popup_shows_the_clock_beside_the_month() {
        use crate::interface::Interface;
        let glass = crate::interfaces::theme_for(Interface::Glass);
        let cm = glass.controls();
        assert_eq!(popup_size(&cm, false), (327, 214 + 28));
        let lay = popup_layout(&cm, false);
        assert_eq!(lay.clock, Some(Rect::new(186, 52, 129, 129)));
        assert_eq!(lay.time, Some(Rect::new(186, 186, 129, 24)));
        assert_eq!(lay.grid.frame.y, 52);
        assert_eq!(popup_layout(&ControlMetrics::CLASSIC, false).clock, None);
        assert_eq!(popup_layout(&ControlMetrics::CLASSIC, false).time, None);
        assert_eq!(popup_size(&ControlMetrics::CLASSIC, false), (174, 202));
        let text = TextRenderer::load(&FontConfig::default());
        let mut cal = Calendar::new(2026, 10, None, 0);
        let m = crate::flyout::margin(&glass);
        let clock = lay.clock.unwrap();
        let mid = (m.left + clock.x + 64, m.top + clock.y + 64 - 30);
        let bare = draw_popup(&glass, &text, &cal, PopupHit::Panel);
        cal.now = Some((0, 0, 0));
        let ticking = draw_popup(&glass, &text, &cal, PopupHit::Panel);
        assert_eq!(
            bare.get(mid.0, mid.1),
            bare.get(mid.0, mid.1 - 20),
            "no clock without a time"
        );
        assert_ne!(
            ticking.get(mid.0, mid.1),
            bare.get(mid.0, mid.1),
            "the hands at 12 cross the face above its middle"
        );
        let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 327, 202)));
        popup_access(&cal, &cm, &text, 11, &mut tree, crate::access::ROOT);
        assert!(
            tree.nodes().any(|(_, n)| n.label() == Some("Clock")),
            "the clock is named"
        );
    }

    /// The time line stands centred under the clock in the title's ink, a size up; without a time
    /// the row is bare.
    #[test]
    fn a_glass_popup_writes_the_time_under_the_clock() {
        use crate::interface::Interface;
        let glass = crate::interfaces::theme_for(Interface::Glass);
        let text = TextRenderer::load(&FontConfig::default());
        let m = crate::flyout::margin(&glass);
        let r = popup_layout(&glass.controls(), false).time.unwrap();
        let row = Rect::new(r.x + m.left, r.y + m.top, r.width, r.height);
        let mut cal = Calendar::new(2026, 10, None, 0);
        let bare = draw_popup(&glass, &text, &cal, PopupHit::Panel);
        cal.time = Some("14:05".into());
        let timed = draw_popup(&glass, &text, &cal, PopupHit::Panel);
        let inked = bare.differing_rect(&timed);
        assert!(!inked.is_empty(), "the time is written");
        assert_eq!(inked.intersect(&row), inked, "inside its row");
        let w = text.measure("14:05", false, TIME_PX);
        assert!(
            inked.width <= w && inked.width >= w * 3 / 4,
            "{inked:?} for {w} px of text"
        );
        let (left, right) = (inked.x - row.x, row.right() - inked.right());
        assert!((left - right).abs() <= 2, "centred: {left} and {right}");
        assert!(
            inked.height >= 12,
            "a size up from the title: {} rows",
            inked.height
        );
        let ink = glass.colors.button_text.argb();
        assert!(
            timed.pixels.iter().filter(|&&px| px == ink).count()
                > bare.pixels.iter().filter(|&&px| px == ink).count(),
            "in the title's ink"
        );
        let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 327, 202)));
        popup_access(
            &cal,
            &glass.controls(),
            &text,
            11,
            &mut tree,
            crate::access::ROOT,
        );
        assert!(tree.nodes().any(|(_, n)| n.label() == Some("14:05")));
    }

    /// At 200% under Glass the clock, its gap and the time line grow with the grid: the clock
    /// keeps an odd side, still level with the grid, and the time row stays under it.
    #[test]
    fn a_grown_glass_popup_grows_its_clock_and_time_line() {
        use crate::interface::Interface;
        let glass = crate::interfaces::theme_for(Interface::Glass).with_text_size(200);
        let cm = glass.controls();
        let lay = popup_layout(&cm, false);
        let grid = lay.grid.frame;
        assert_eq!((grid.width, grid.height), (2 * W, 2 * H));
        let clock = lay.clock.unwrap();
        assert_eq!(clock, Rect::new(grid.right() + 24, grid.y, 259, 259));
        assert_eq!(
            lay.time,
            Some(Rect::new(clock.x, grid.bottom() + 4, 259, 48))
        );
        assert_eq!(
            popup_size(&cm, false),
            (
                2 * W + 24 + 259 + 24,
                2 * 16 + 2 * H + 48 + 4 + 28 + 2 * (32 + 4)
            )
        );
        let text = TextRenderer::load(&FontConfig::default());
        let mut cal = Calendar::new(2026, 10, None, 0);
        cal.now = Some((10, 9, 30));
        cal.time = Some("10:09".into());
        let c = draw_popup(&glass, &text, &cal, PopupHit::Panel);
        let m = crate::flyout::margin(&glass);
        let (w, h) = popup_size(&cm, false);
        assert_eq!(
            (c.width as i32, c.height as i32),
            (w + m.left + m.right, h + m.top + m.bottom)
        );
        let mut small = Calendar::new(2026, 10, None, 0);
        small.time = cal.time.clone();
        let g100 = crate::interfaces::theme_for(Interface::Glass);
        let ink_h = |theme: &Theme, cal: &Calendar| {
            let mut bare = cal.clone();
            bare.time = None;
            let a = draw_popup(theme, &text, &bare, PopupHit::Panel);
            let b = draw_popup(theme, &text, cal, PopupHit::Panel);
            a.differing_rect(&b).height
        };
        let (at_100, at_200) = (ink_h(&g100, &small), ink_h(&glass, &small));
        assert!(
            at_100 > 0 && at_200 >= 2 * at_100 - 2,
            "{at_200} rows at 200%, {at_100} at 100%"
        );
    }

    /// A tick redraws the clock's square over the drawn popup, and the result is the popup drawn
    /// whole at the new time.
    #[test]
    fn a_tick_changes_only_the_clock_and_matches_a_whole_redraw() {
        use crate::interface::Interface;
        let glass = crate::interfaces::theme_for(Interface::Glass);
        let m = crate::flyout::margin(&glass);
        assert_eq!(
            popup_clock(&glass),
            Some(Rect::new(m.left + 186, m.top + 52, 129, 129))
        );
        assert_eq!(popup_clock(&Theme::default()), None);
        let text = TextRenderer::load(&FontConfig::default());
        for s in [1, 2] {
            let theme = glass.scaled(s);
            let clock = popup_clock(&theme).unwrap();
            let mut cal = Calendar::new(2026, 10, None, 0);
            cal.today = Some((2026, 10, 16));
            let base = draw_popup_base(&theme, &text, &cal, PopupHit::Panel);
            cal.now = Some((10, 9, 30));
            let mut shown = draw_popup(&theme, &text, &cal, PopupHit::Panel);
            let before = shown.clone();
            cal.now = Some((10, 9, 31));
            draw_popup_clock(&theme, &mut shown, &base, (10, 9, 31));
            let changed = before.differing_rect(&shown);
            assert!(!changed.is_empty(), "scale {s}: the second hand moves");
            assert_eq!(
                changed.intersect(&clock),
                changed,
                "scale {s}: only the clock's square changes"
            );
            assert!(
                shown.pixels == draw_popup(&theme, &text, &cal, PopupHit::Panel).pixels,
                "scale {s}: the same pixels as the popup drawn at 10:09:31"
            );
        }
    }

    /// Each look's popup with its date line and the link lit, at scale 1 and 2.
    #[test]
    fn the_popup_with_its_date_and_lit_link_pins_in_each_look_at_scale_1_and_2() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let text = TextRenderer::load(&FontConfig::default());
        let mut cal = Calendar::new(2026, 11, None, 0);
        cal.today = Some((2026, 11, 26));
        cal.date = vec!["Thursday, November 26, 2026".into(), "11/26/2026".into()];
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for s in [1, 2] {
                let theme = crate::interfaces::theme_for(i).scaled(s);
                let c = draw_popup(&theme, &text, &cal, PopupHit::Settings);
                got.push((format!("{i:?} {s}"), crate::pins::fnv(&c)));
            }
        }
        crate::pins::check("controls/calendar_popup", &got);
    }

    /// The date line heads the panel over the title row and the link ends it, centred, underlined
    /// while hot; a click on its row and a reader's link both reach it.
    #[test]
    fn the_popup_heads_with_the_date_and_ends_with_the_settings_link() {
        let cm = ControlMetrics::CLASSIC;
        let lay = popup_layout(&cm, false);
        assert_eq!(lay.date, Rect::new(6, 6, 162, 16));
        assert_eq!(lay.prev.y, 26, "the title row sits under the date line");
        assert_eq!(lay.settings, Rect::new(6, 180, 162, 16));
        assert_eq!(lay.settings.bottom() + 6, popup_size(&cm, false).1);
        assert_eq!(popup_hit(&cm, &bare(), 20, 185), PopupHit::Settings);
        assert_eq!(popup_hit(&cm, &bare(), 20, 10), PopupHit::Panel);
        let with_list = popup_layout(&cm, true);
        assert_eq!(
            with_list.settings.y,
            with_list.agenda.unwrap().rows[AGENDA_ROWS - 1].bottom() + 4,
            "under the event list"
        );
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut cal = bare();
        cal.date = vec!["Thursday, November 26, 2026".into(), "11/26/2026".into()];
        let plain = draw_popup(&theme, &text, &cal, PopupHit::Panel);
        let hot = draw_popup(&theme, &text, &cal, PopupHit::Settings);
        let under = plain.differing_rect(&hot);
        assert!(!under.is_empty(), "the hot link underlines");
        assert_eq!(under.height, 1, "a one-row underline");
        assert!(lay.settings.contains(under.x, under.y));
        assert!(
            under.x > lay.settings.x && under.right() < lay.settings.right(),
            "centred in its row"
        );
        let wording = |date: &[&str]| {
            let mut cal = bare();
            cal.date = date.iter().map(|d| d.to_string()).collect();
            draw_popup(&theme, &text, &cal, PopupHit::Panel).pixels
        };
        assert!(
            wording(&[&"W".repeat(40), "11/26/2026"]) == wording(&["11/26/2026"]),
            "a wording too wide gives way to the next"
        );
        let undated = draw_popup(&theme, &text, &bare(), PopupHit::Panel);
        let date = plain.differing_rect(&undated);
        assert!(
            lay.date.contains(date.x, date.y),
            "the date line draws in its row"
        );
        let mut tree = Tree::new(access::node(Role::Window, Rect::new(0, 0, 174, 202)));
        popup_access(&cal, &cm, &text, 11, &mut tree, crate::access::ROOT);
        let link = tree.find(Role::Link, &settings()).expect("the link's node");
        assert!(
            tree.get(link)
                .unwrap()
                .supports_action(accesskit::Action::Click)
        );
        assert!(
            tree.nodes()
                .any(|(_, n)| n.label() == Some("Thursday, November 26, 2026")),
            "the date line reads"
        );
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
