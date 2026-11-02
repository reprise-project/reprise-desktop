//! Desktop widgets: a clock, a month calendar and a system meter as panels on the desktop grid,
//! glass under a look with glass and a raised face otherwise.

use std::collections::HashSet;
use std::time::Duration;

use crate::access::{self, Tree, accesskit};
use crate::controls::calendar::{self, Calendar};
use crate::controls::iconview::{self, Cell};
use crate::look::widgets::{draw_analog_clock, draw_progress};
use crate::look::{Paint, panel};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::shape::{self, Stops};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Theme};
use accesskit::{NodeId, Role};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Clock,
    Calendar,
    Meter,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Clock, Kind::Calendar, Kind::Meter];

    /// Its name in `desktop_widgets` and the state file.
    pub fn name(self) -> &'static str {
        match self {
            Kind::Clock => "clock",
            Kind::Calendar => "calendar",
            Kind::Meter => "meter",
        }
    }

    pub fn from_name(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }

    /// Its row in menus, with the access key marked.
    pub fn label(self) -> &'static str {
        match self {
            Kind::Clock => "&Clock",
            Kind::Calendar => "C&alendar",
            Kind::Meter => "System &Meter",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Face {
    #[default]
    Analog,
    Digital,
}

impl Face {
    pub fn name(self) -> &'static str {
        match self {
            Face::Analog => "analog",
            Face::Digital => "digital",
        }
    }

    pub fn from_name(name: &str) -> Option<Face> {
        [Face::Analog, Face::Digital]
            .into_iter()
            .find(|f| f.name() == name)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeterStyle {
    #[default]
    Bars,
    Graph,
}

impl MeterStyle {
    pub fn name(self) -> &'static str {
        match self {
            MeterStyle::Bars => "bars",
            MeterStyle::Graph => "graph",
        }
    }

    pub fn from_name(name: &str) -> Option<MeterStyle> {
        [MeterStyle::Bars, MeterStyle::Graph]
            .into_iter()
            .find(|f| f.name() == name)
    }
}

/// What the clock shows. Built by [`Clock::new`], which keeps only what its face draws, so equal
/// clocks draw equal pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clock {
    pub face: Face,
    pub seconds: bool,
    /// Hour 0..23, minute and second: the analog face's hands.
    pub time: (u32, u32, u32),
    /// The digital face's time and the date under it, worded by the host in the locale's format.
    pub text: String,
    pub date: String,
}

impl Clock {
    pub fn new(face: Face, seconds: bool, time: (u32, u32, u32), text: &str, date: &str) -> Clock {
        let (h, m, s) = time;
        let s = if seconds { s.min(59) } else { 0 };
        match face {
            Face::Analog => Clock {
                face,
                seconds,
                time: (h, m, s),
                text: String::new(),
                date: String::new(),
            },
            Face::Digital => Clock {
                face,
                seconds,
                time: (0, 0, 0),
                text: text.into(),
                date: date.into(),
            },
        }
    }
}

/// How long from `since_epoch` until the clock next changes: the next second's start with
/// `seconds`, else the next minute's, a few ms late so the time read then has turned.
pub fn next_tick(seconds: bool, since_epoch: Duration) -> Duration {
    const LATE: Duration = Duration::from_millis(5);
    let period: u128 = if seconds { 1_000 } else { 60_000 };
    let into = since_epoch.as_millis() % period;
    Duration::from_millis((period - into) as u64) + LATE
}

/// The kernel's CPU time counters (`/proc/stat`'s `cpu` line), in clock ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

impl CpuTimes {
    /// The all-CPU line of `/proc/stat`'s text: user, nice, system, idle, iowait, irq, softirq and
    /// steal summed; idle and iowait are the idle part. Guest time is already in user and nice.
    pub fn parse(stat: &str) -> Option<CpuTimes> {
        let line = stat.lines().find(|l| l.starts_with("cpu "))?;
        let fields: Vec<u64> = line
            .split_whitespace()
            .skip(1)
            .take(8)
            .map(str::parse)
            .collect::<Result<_, _>>()
            .ok()?;
        if fields.len() < 4 {
            return None;
        }
        let total: u64 = fields.iter().sum();
        let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
        Some(CpuTimes {
            busy: total - idle,
            total,
        })
    }
}

/// The busy share of the CPU time between two readings, 0..=1; None when no time passed.
pub fn cpu_share(before: CpuTimes, after: CpuTimes) -> Option<f64> {
    let total = after.total.checked_sub(before.total)?;
    let busy = after.busy.saturating_sub(before.busy);
    (total > 0).then(|| (busy as f64 / total as f64).clamp(0.0, 1.0))
}

/// The share of memory in use from `/proc/meminfo`'s text: what `MemAvailable` leaves of
/// `MemTotal`, 0..=1.
pub fn memory_share(meminfo: &str) -> Option<f64> {
    let field = |name: &str| -> Option<u64> {
        let line = meminfo.lines().find(|l| l.starts_with(name))?;
        line[name.len()..].split_whitespace().next()?.parse().ok()
    };
    let total = field("MemTotal:")?;
    let available = field("MemAvailable:")?;
    (total > 0).then(|| (1.0 - available as f64 / total as f64).clamp(0.0, 1.0))
}

/// Samples the meter keeps: its graph spans the last minute at one a second.
pub const HISTORY: usize = 60;

/// What the meter shows: the CPU and memory in use, in whole percent, oldest first, the newest
/// last; at most [`HISTORY`] of each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Meter {
    pub style: MeterStyle,
    pub cpu: Vec<u8>,
    pub memory: Vec<u8>,
}

impl Meter {
    pub fn new(style: MeterStyle) -> Meter {
        Meter {
            style,
            ..Meter::default()
        }
    }

    /// Add a sample of each share (0..=1); a missing one repeats the last.
    pub fn push(&mut self, cpu: Option<f64>, memory: Option<f64>) {
        let percent = |v: f64| (v.clamp(0.0, 1.0) * 100.0).round() as u8;
        for (list, v) in [(&mut self.cpu, cpu), (&mut self.memory, memory)] {
            let Some(p) = v.map(percent).or(list.last().copied()) else {
                continue;
            };
            if list.len() == HISTORY {
                list.remove(0);
            }
            list.push(p);
        }
    }

    pub fn cpu_now(&self) -> Option<u8> {
        self.cpu.last().copied()
    }

    pub fn memory_now(&self) -> Option<u8> {
        self.memory.last().copied()
    }
}

/// A widget's content: everything its pixels depend on besides the theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    Clock(Clock),
    /// The month with today; nothing picked.
    Calendar(Calendar),
    Meter(Meter),
}

impl Content {
    pub fn kind(&self) -> Kind {
        match self {
            Content::Clock(_) => Kind::Clock,
            Content::Calendar(_) => Kind::Calendar,
            Content::Meter(_) => Kind::Meter,
        }
    }
}

/// Design px: the room round a panel's content.
const PAD: i32 = 8;
/// The analog face's side, odd so its middle is a pixel's.
const ANALOG: i32 = 113;
/// The digital face: its width, the time's text size and line, and the date's line under it.
const DIGITAL_W: i32 = 200;
const TIME_PX: u32 = 32;
const TIME_H: i32 = 38;
const DATE_H: i32 = 15;
/// The calendar's month title over its grid.
const TITLE_H: i32 = 16;
const TITLE_GAP: i32 = 4;
/// The meter: its width, a row's label column, bar or graph height, and the gap between parts.
const METER_W: i32 = 210;
const LABEL_W: i32 = 48;
const PERCENT_W: i32 = 34;
const BAR_H: i32 = 16;
const ROW_GAP: i32 = 6;
const GRAPH_H: i32 = 40;
const LABEL_H: i32 = 13;
/// The graph's fill under its line, and the well a graph lies in under glass.
const GRAPH_FILL: u8 = 0x50;
const GLASS_WELL: u8 = 0x40;

/// The panel's size at scale 1: the clock's per face, the calendar's grown with the text size
/// like its grid, the meter's per style.
pub fn size(content: &Content, cm: &ControlMetrics) -> (i32, i32) {
    match content {
        Content::Clock(c) => clock_size(c.face),
        Content::Calendar(_) => {
            let (w, h) = calendar::size(cm);
            let title = cm.text.len(TITLE_H);
            (w + 2 * PAD, title + TITLE_GAP + h + 2 * PAD)
        }
        Content::Meter(m) => meter_size(m.style),
    }
}

fn clock_size(face: Face) -> (i32, i32) {
    match face {
        Face::Analog => (ANALOG + 2 * PAD, ANALOG + 2 * PAD),
        Face::Digital => (DIGITAL_W, TIME_H + DATE_H + 2 * PAD),
    }
}

fn meter_size(style: MeterStyle) -> (i32, i32) {
    match style {
        MeterStyle::Bars => (METER_W, 2 * BAR_H + ROW_GAP + 2 * PAD),
        MeterStyle::Graph => (METER_W, LABEL_H + 2 + GRAPH_H + 2 * PAD),
    }
}

/// A widget's place on the desktop grid: its top-left cell and how many cells it covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub cell: (i32, i32),
    pub span: (i32, i32),
}

impl Spot {
    pub fn cells(self) -> impl Iterator<Item = (i32, i32)> {
        let (c, r) = self.cell;
        (c..c + self.span.0).flat_map(move |x| (r..r + self.span.1).map(move |y| (x, y)))
    }

    pub fn overlaps(self, o: Spot) -> bool {
        self.cell.0 < o.cell.0 + o.span.0
            && o.cell.0 < self.cell.0 + self.span.0
            && self.cell.1 < o.cell.1 + o.span.1
            && o.cell.1 < self.cell.1 + self.span.1
    }

    /// The cells' rect on a grid of `grid` cells over `area`.
    pub fn rect(self, grid: Cell, area: Rect) -> Rect {
        let at = iconview::cell_rect(grid, area, self.cell.0, self.cell.1);
        Rect::new(at.x, at.y, grid.w * self.span.0, grid.h * self.span.1)
    }

    /// Where a panel of `size` stands: centred in [`Spot::rect`].
    pub fn panel(self, grid: Cell, area: Rect, size: (i32, i32)) -> Rect {
        let r = self.rect(grid, area);
        Rect::new(
            r.x + (r.width - size.0) / 2,
            r.y + (r.height - size.1) / 2,
            size.0,
            size.1,
        )
    }
}

/// The cells a panel of `size` covers on a grid of `grid` cells.
pub fn span(grid: Cell, size: (i32, i32)) -> (i32, i32) {
    let up = |n: i32, d: i32| ((n + d - 1) / d.max(1)).max(1);
    (up(size.0, grid.w), up(size.1, grid.h))
}

/// The grid's columns and rows over `area`.
pub fn dims(grid: Cell, area: Rect) -> (i32, i32) {
    (
        iconview::columns(grid, area) as i32,
        iconview::rows_per_column(grid, area) as i32,
    )
}

/// The top-left cell nearest `want` for a widget covering `span` on a grid of `dims`: inside it,
/// off `others`, and off `avoid` where it can be; with no room, `want` pulled inside the grid.
pub fn place(
    dims: (i32, i32),
    span: (i32, i32),
    want: (i32, i32),
    others: &[Spot],
    avoid: &HashSet<(i32, i32)>,
) -> (i32, i32) {
    let last = ((dims.0 - span.0).max(0), (dims.1 - span.1).max(0));
    let inside = (want.0.clamp(0, last.0), want.1.clamp(0, last.1));
    let distance = |(c, r): (i32, i32)| (c - want.0).pow(2) + (r - want.1).pow(2);
    let free: Vec<Spot> = (0..=last.0)
        .flat_map(|c| (0..=last.1).map(move |r| Spot { cell: (c, r), span }))
        .filter(|s| others.iter().all(|o| !s.overlaps(*o)))
        .collect();
    let nearest = |spots: &mut dyn Iterator<Item = &Spot>| {
        spots
            .min_by_key(|s| (distance(s.cell), s.cell.1, -s.cell.0))
            .map(|s| s.cell)
    };
    nearest(
        &mut free
            .iter()
            .filter(|s| s.cells().all(|c| !avoid.contains(&c))),
    )
    .or_else(|| nearest(&mut free.iter()))
    .unwrap_or(inside)
}

/// Whether the panel of `size` (scale 1, panel coordinates) holds (`x`, `y`): a widget takes
/// every press on its panel, none on its shadow.
pub fn hit(size: (i32, i32), x: i32, y: i32) -> bool {
    Rect::new(0, 0, size.0, size.1).contains(x, y)
}

/// The widget at `theme`'s scale: [`size`] plus [`panel::margin`] on each side.
pub fn draw(theme: &Theme, text: &TextRenderer, content: &Content) -> Canvas {
    let p = &theme.paint();
    let cs = p.cs;
    let s = p.s;
    let (w, h) = size(content, &theme.controls());
    if let Some((style, spec)) = panel::glass(theme) {
        return panel::draw(style, spec, w, h, s, |c| {
            body(c, theme, p, text, content, true)
        });
    }
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    c.edge_t(c.bounds(), EdgeStyle::WindowRaised, cs, s);
    body(&mut c, theme, p, text, content, false);
    c
}

/// Text in `r` (device px) in `ink`: run over a clear canvas under glass, drawn plainly on a face.
#[expect(clippy::too_many_arguments)]
fn put_text(
    c: &mut Canvas,
    text: &TextRenderer,
    glass: bool,
    r: Rect,
    words: &str,
    bold: bool,
    ink: Color,
    align: Align,
    px: u32,
) {
    if !glass {
        return text.draw(c, r, words, bold, ink, align, px);
    }
    let w = text.measure(words, bold, px);
    let x = match align {
        Align::Center => r.x + (r.width - w) / 2,
        Align::Right => r.right() - w,
        Align::Left => r.x,
    };
    let baseline = text.baseline(r, bold, px).unwrap_or(r.bottom());
    text.draw_run_over(c, x, baseline, r, words, bold, ink, px);
}

fn body(
    c: &mut Canvas,
    theme: &Theme,
    p: &Paint,
    text: &TextRenderer,
    content: &Content,
    glass: bool,
) {
    let s = p.s;
    let ink = if glass { panel::INK } else { p.cs.button_text };
    let px = theme.fonts.size_px;
    let cm = theme.controls();
    let (w, h) = size(content, &cm);
    let inner = Rect::new(PAD, PAD, w - 2 * PAD, h - 2 * PAD);
    match content {
        Content::Clock(clock) => match clock.face {
            Face::Analog => {
                let face = Rect::new(PAD, PAD, ANALOG, ANALOG).scaled(s);
                draw_analog_clock(c, p, face, clock.time, clock.seconds);
            }
            Face::Digital => {
                let time = Rect::new(inner.x, inner.y, inner.width, TIME_H).scaled(s);
                let size = cm.text.px(TIME_PX) * s as u32;
                put_text(
                    c,
                    text,
                    glass,
                    time,
                    &clock.text,
                    false,
                    ink,
                    Align::Center,
                    size,
                );
                let date = Rect::new(inner.x, inner.y + TIME_H, inner.width, DATE_H).scaled(s);
                let date_text = text.ellipsize(&clock.date, date.width, false, px);
                put_text(
                    c,
                    text,
                    glass,
                    date,
                    &date_text,
                    false,
                    ink,
                    Align::Center,
                    px,
                );
            }
        },
        Content::Calendar(cal) => {
            let title_h = cm.text.len(TITLE_H);
            let title = Rect::new(inner.x, inner.y, inner.width, title_h).scaled(s);
            put_text(
                c,
                text,
                glass,
                title,
                &cal.title(),
                true,
                ink,
                Align::Center,
                px,
            );
            let (gw, gh) = calendar::size(&cm);
            let grid = Rect::new(inner.x, inner.y + title_h + TITLE_GAP, gw, gh);
            calendar::draw(c, text, p, cal, &calendar::layout(grid, &cm), false, px);
        }
        Content::Meter(m) => {
            let rows = [("CPU", &m.cpu), ("Memory", &m.memory)];
            match m.style {
                MeterStyle::Bars => {
                    for (i, (label, list)) in rows.into_iter().enumerate() {
                        let y = inner.y + i as i32 * (BAR_H + ROW_GAP);
                        let l = Rect::new(inner.x, y, LABEL_W, BAR_H).scaled(s);
                        put_text(c, text, glass, l, label, false, ink, Align::Left, px);
                        let bar_w = inner.width - LABEL_W - PERCENT_W;
                        let bar = Rect::new(inner.x + LABEL_W, y, bar_w, BAR_H);
                        let now = list.last().copied();
                        let share = f64::from(now.unwrap_or(0)) / 100.0;
                        draw_progress(c, p, bar, share, true);
                        let r = Rect::new(bar.right(), y, PERCENT_W, BAR_H).scaled(s);
                        put_text(
                            c,
                            text,
                            glass,
                            r,
                            &percent(now),
                            false,
                            ink,
                            Align::Right,
                            px,
                        );
                    }
                }
                MeterStyle::Graph => {
                    let gw = (inner.width - PAD) / 2;
                    for (i, (label, list)) in rows.into_iter().enumerate() {
                        let x = inner.x + i as i32 * (gw + PAD);
                        let words = format!("{label} {}", percent(list.last().copied()));
                        let l = Rect::new(x, inner.y, gw, LABEL_H).scaled(s);
                        put_text(c, text, glass, l, &words, false, ink, Align::Left, px);
                        let well = Rect::new(x, inner.y + LABEL_H + 2, gw, GRAPH_H);
                        draw_graph(c, p, well, list, glass);
                    }
                }
            }
        }
    }
}

fn percent(v: Option<u8>) -> String {
    v.map_or_else(|| "--".into(), |v| format!("{v}%"))
}

/// A history graph in the well `r` (scale 1): a sunken white well on a face, a dark one under
/// glass; the samples run right to left from the newest, a line over a lighter fill.
fn draw_graph(c: &mut Canvas, p: &Paint, r: Rect, values: &[u8], glass: bool) {
    let (cs, s) = (p.cs, p.s);
    let line = if glass { panel::INK } else { cs.hilight };
    let dev = r.scaled(s);
    let well = if glass {
        shape::draw(r.width, r.height, s, |x, y| {
            let edge = shape::rounded(
                x,
                y,
                f64::from(dev.width),
                f64::from(dev.height),
                2.0 * f64::from(s),
            );
            shape::scale(shape::premul(Color::rgb(0, 0, 0), GLASS_WELL), edge)
        })
    } else {
        let mut w = Canvas::new(dev.width as u32, dev.height as u32);
        w.fill_rect(w.bounds(), cs.window);
        w.edge_t(w.bounds(), EdgeStyle::ButtonSunken, cs, s);
        w
    };
    c.blit_over(dev.x, dev.y, &well);
    let edge = if glass { 0 } else { 2 };
    let inside = Rect::new(edge, edge, r.width - 2 * edge, r.height - 2 * edge);
    if values.is_empty() || inside.width <= 0 || inside.height <= 0 {
        return;
    }
    let k = f64::from(s);
    let (iw, ih) = (f64::from(inside.width) * k, f64::from(inside.height) * k);
    let step = iw / (HISTORY - 1) as f64;
    let pts: Vec<(f64, f64)> = values
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &v)| {
            let x = iw - i as f64 * step;
            let y = ih - f64::from(v) / 100.0 * (ih - k) - k / 2.0;
            (x, y)
        })
        .collect();
    let mut area = pts.clone();
    if let (Some(&(x0, _)), Some(&(x1, _))) = (pts.first(), pts.last()) {
        area.extend([(x1, ih), (x0, ih)]);
    }
    let lit = shape::premul(line, 255);
    let fill_stops = [
        (0.0, shape::premul(line, GRAPH_FILL)),
        (1.0, shape::premul(line, GRAPH_FILL / 2)),
    ];
    let art = shape::draw(inside.width, inside.height, s, |x, y| {
        let under = shape::scale(
            Stops(&fill_stops).at((y + 0.5) / ih),
            shape::polygon(x, y, &area),
        );
        let on = if pts.len() > 1 {
            shape::stroke(x, y, &pts, k)
        } else {
            0.0
        };
        shape::over(shape::scale(lit, on), under)
    });
    c.blit_over(dev.x + edge * s, dev.y + edge * s, &art);
}

/// The widget's `access` part under `parent`: a group named for its kind holding what it reads
/// out: the time, the month grid, or the two meters.
#[expect(clippy::too_many_arguments)]
pub fn access(
    content: &Content,
    r: Rect,
    cm: &ControlMetrics,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let name = crate::text::split_mnemonic(content.kind().label()).0;
    let id = tree.push(parent, key, access::titled(Role::Group, r, &name));
    let inner = Rect::new(r.x + PAD, r.y + PAD, r.width - 2 * PAD, r.height - 2 * PAD);
    match content {
        Content::Clock(c) => {
            let said = match c.face {
                Face::Analog if c.seconds => {
                    format!("{}:{:02}:{:02}", c.time.0, c.time.1, c.time.2)
                }
                Face::Analog => format!("{}:{:02}", c.time.0, c.time.1),
                Face::Digital => format!("{}, {}", c.text, c.date),
            };
            tree.push(id, "time", access::label(inner, &said, true));
        }
        Content::Calendar(cal) => {
            let title_h = cm.text.len(TITLE_H);
            let (gw, gh) = calendar::size(cm);
            let grid = Rect::new(inner.x, inner.y + title_h + TITLE_GAP, gw, gh);
            calendar::access(
                cal,
                &calendar::layout(grid, cm),
                text,
                px,
                tree,
                id,
                "month",
            );
        }
        Content::Meter(m) => {
            for (label, now) in [("CPU", m.cpu_now()), ("Memory", m.memory_now())] {
                let mut n = access::range(
                    Role::ProgressIndicator,
                    inner,
                    f64::from(now.unwrap_or(0)),
                    0.0,
                    100.0,
                    true,
                );
                n.set_label(label);
                tree.push(id, label, n);
            }
        }
    }
    id
}

#[cfg(test)]
mod tests;
