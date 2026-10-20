//! Taskbar model, layout, hit test and access part; each interface paints it in its own module.

use crate::access::accesskit::{HasPopup, NodeId, Orientation, Role};
use crate::access::{self, Tree};
use crate::icon::ICON_SIZE;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Metrics};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A task button's progress, as its program reports it through its launcher entry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaskProgress {
    /// Done, from 0 to 1.
    pub fraction: f64,
    pub paused: bool,
}

/// A look's art for a task button's progress: a canvas `w` x `h` design px at `d` device px each,
/// laid over the button's face under its icon and label.
pub type ProgressPainter =
    Arc<dyn Fn(&TaskProgress, &crate::theme::ColorScheme, i32, i32, i32) -> Canvas + Send + Sync>;

pub struct TaskEntry {
    pub title: String,
    pub active: bool,
    /// Set when an xdg-activation request could not focus the window; drawn like the active button until focused.
    pub attention: bool,
    pub icon: Option<Arc<Canvas>>,
    /// None for an icon without a hue.
    pub glow: Option<Color>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PagerButton {
    pub current: bool,
    pub attention: bool,
}

/// How a look with slot lights marks the current pager cell: a light along its foot or a glass tile behind its number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PagerMark {
    #[default]
    Light,
    Tile,
}

impl PagerMark {
    pub const ALL: [PagerMark; 2] = [PagerMark::Light, PagerMark::Tile];

    pub fn name(self) -> &'static str {
        match self {
            PagerMark::Light => "light",
            PagerMark::Tile => "tile",
        }
    }

    pub fn from_name(s: &str) -> Option<PagerMark> {
        PagerMark::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// The output edge the bar sits on; a side bar is a column. `[taskbar] position` names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Bottom,
    Top,
    Left,
    Right,
}

impl Edge {
    pub const ALL: [Edge; 4] = [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right];

    pub fn name(self) -> &'static str {
        match self {
            Edge::Bottom => "bottom",
            Edge::Top => "top",
            Edge::Left => "left",
            Edge::Right => "right",
        }
    }

    pub fn is_side(self) -> bool {
        matches!(self, Edge::Left | Edge::Right)
    }

    pub fn from_name(s: &str) -> Option<Edge> {
        Edge::ALL.into_iter().find(|e| e.name() == s)
    }

    /// Turn `r` between along/across form (x along the bar, y across from its inner edge) and the bar's own x/y; turning twice returns `r`.
    pub fn to_xy(self, r: Rect, thickness: i32) -> Rect {
        match self {
            Edge::Bottom => r,
            Edge::Top => Rect::new(r.x, thickness - r.y - r.height, r.width, r.height),
            Edge::Left | Edge::Right => Rect::new(r.y, r.x, r.height, r.width),
        }
    }

    /// Draw bar art into `at`, written for a Bottom bar (x along, row 0 on the inner edge) and turned to this edge.
    pub fn draw_art(self, canvas: &mut Canvas, at: Rect, f: impl FnOnce(&mut Canvas, Rect)) {
        match self {
            Edge::Bottom => f(canvas, at),
            Edge::Top => canvas.with_rows_flipped(at, |c| f(c, at)),
            Edge::Left | Edge::Right => {
                let pic = Rect::new(0, 0, at.height, at.width);
                canvas.with_transposed(at, self == Edge::Left, |c| f(c, pic))
            }
        }
    }
}

pub const REVEAL_LINE: i32 = 2;

#[derive(Default)]
pub struct TaskbarState {
    pub start_pressed: bool,
    pub start_focus: bool,
    pub focus: Option<TaskbarHit>,
    pub start_hot: bool,
    pub start_icon: Option<Arc<Canvas>>,
    pub orb_glow: u8,
    pub quick_icons: Vec<Option<Arc<Canvas>>>,
    pub quick_hot: Option<usize>,
    pub quick_pressed: Option<usize>,
    pub quick_drop: Option<usize>,
    pub quick_more: PopupCell,
    pub pager: Vec<PagerButton>,
    pub pager_hot: Option<usize>,
    pub pager_mark: PagerMark,
    pub task_hot: Option<usize>,
    pub task_pressed: Option<usize>,
    pub task_hot_x: Option<i32>,
    /// Per task button, px it is drawn along the bar from its slot while sliding; empty or 0 draws it in place.
    pub task_shift: Vec<i32>,
    /// The dragged task button: its index and px along the bar it is drawn at, over the others; its own slot draws empty.
    pub task_float: Option<(usize, i32)>,
    /// Per task button, its program's progress; empty or None draws none.
    pub task_progress: Vec<Option<TaskProgress>>,
    pub show_desktop_hot: bool,
    pub more: PopupCell,
    pub chevron: PopupCell,
    pub tasks: Vec<TaskEntry>,
    pub tray_icons: Vec<Option<Arc<Canvas>>>,
    pub clock: String,
}

/// A bar cell that opens a popup (» or ^); `open` wins over `hot`; a tray arrow also draws `held`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PopupCell {
    pub hot: bool,
    pub open: bool,
    /// Tray arrow held down; never set on the » cell or the flyout's ^.
    pub held: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskbarHit {
    Start,
    Quick(usize),
    QuickMore,
    Pager(usize),
    Task(usize),
    Chevron,
    Tray(usize),
    Clock,
    ShowDesktop,
    Grip(usize),
    More,
    Resize,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Quick,
    Tasks,
}

/// Which band comes first and how wide it is (None: natural). Persisted in the state file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bands {
    pub tasks_first: bool,
    pub first_width: Option<i32>,
}

impl Bands {
    /// Arrangement after moving from `from` to `to`; a row/column turn drops the first band's width.
    pub fn turned(self, from: Edge, to: Edge) -> Bands {
        if from.is_side() == to.is_side() {
            self
        } else {
            Bands {
                first_width: None,
                ..self
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TaskbarCounts {
    pub quick: usize,
    pub pager: usize,
    pub tasks: usize,
    pub tray: usize,
    pub show_desktop: bool,
    pub chevron: bool,
    /// With a tray arrow ([`TaskbarShape::tray_arrow`]): how many hidden
    /// items the open run can show.
    pub hidden: usize,
    /// Px the run has opened, 0..=[`reveal_width`]; ignored without a tray arrow.
    pub reveal: i32,
}

/// How a look lays its bar out; describes a Bottom bar, other edges turn it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TaskbarShape {
    /// Start's x, y and height (0: as tall as the other buttons). Width comes from `start_button_width`.
    pub start: Rect,
    pub button_top: i32,
    pub button_bottom: i32,
    pub start_gap: i32,
    pub well_full_height: bool,
    pub well_lead: i32,
    pub tray_gap: i32,
    /// Tray icons take the odd px left over, sitting a px low.
    pub tray_low: bool,
    /// Down a side bar without an orb: inset of the tray grid from either side of the button row.
    pub side_tray_inset: i32,
    /// The ^ cell is a box that slides hidden items out in place, not a flyout cell.
    pub tray_arrow: Option<TrayArrow>,
    pub clock: ClockBox,
    pub start_icon: i32,
    pub task_icon: i32,
    /// Icon-only task button width and step to the next; None: square, `TASK_GAP` apart.
    pub icon_button: Option<(i32, i32)>,
    pub show_desktop_width: Option<i32>,
    pub orb: bool,
    /// Orb standing above a Bottom bar's top edge (below a Top bar's foot); None keeps Start inside the bar.
    pub orb_rise: Option<OrbRise>,
    /// Two-line clock baselines (time, date) in px from the top of a [`TWO_LINE_CLOCK_MIN`] bar; None: one line.
    pub two_line_clock: Option<[i32; 2]>,
    pub start_menu: StartMenuDock,
    pub labelled: LabelledButton,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LabelledButton {
    pub gap: i32,
    pub icon_x: i32,
    pub icon_box: i32,
    pub text_x: i32,
}

impl LabelledButton {
    pub const CLASSIC: LabelledButton = LabelledButton {
        gap: TASK_GAP,
        icon_x: 10,
        icon_box: ICON_SIZE,
        text_x: 30,
    };
}

/// Tray arrow box in px at scale 1: `size`, `gap` before the first cell after it, `well_lead` from the well's edge; side bars use the `side_*` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayArrow {
    pub size: (i32, i32),
    pub gap: i32,
    pub well_lead: i32,
    pub side_size: (Option<i32>, i32),
    pub side_gap: i32,
    pub side_lead: i32,
    pub side_inset: i32,
}

impl TrayArrow {
    pub const CLASSIC: TrayArrow = TrayArrow {
        size: (16, 18),
        gap: 1,
        well_lead: 1,
        side_size: (None, 16),
        side_gap: 1,
        side_lead: 1,
        side_inset: 0,
    };
}

/// Clock box: text width plus `pad`, at least `min_width`, ending `right` px before the bar's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockBox {
    pub pad: i32,
    pub min_width: i32,
    pub right: i32,
}

impl ClockBox {
    pub const CLASSIC: ClockBox = ClockBox {
        pad: 16,
        min_width: 0,
        right: 2,
    };
}

impl TaskbarShape {
    pub const CLASSIC: TaskbarShape = TaskbarShape {
        start: Rect::new(2, 4, 0, 0),
        button_top: 4,
        button_bottom: 2,
        start_gap: 3,
        well_full_height: false,
        well_lead: 4,
        tray_gap: 4,
        tray_low: false,
        side_tray_inset: 5,
        tray_arrow: Some(TrayArrow::CLASSIC),
        clock: ClockBox::CLASSIC,
        start_icon: ICON_SIZE,
        task_icon: ICON_SIZE,
        icon_button: None,
        show_desktop_width: None,
        orb: false,
        orb_rise: None,
        two_line_clock: None,
        start_menu: StartMenuDock::Into(4),
        labelled: LabelledButton::CLASSIC,
    };

    /// Two-line baselines for a bar `bar_height` tall, or None for one line.
    pub fn clock_baselines(&self, bar_height: i32) -> Option<[i32; 2]> {
        self.two_line_clock
            .filter(|_| bar_height >= TWO_LINE_CLOCK_MIN)
    }

    /// Narrowest side bar: one icon-only slot.
    pub fn side_slot(&self, m: &Metrics) -> i32 {
        let btn = m.taskbar_height - self.button_top - self.button_bottom;
        self.icon_button.map_or(btn, |(w, _)| w) + self.button_top + self.button_bottom
    }

    /// Side bar width when `[taskbar] side_width` leaves it to the look.
    pub fn side_default(&self, m: &Metrics) -> i32 {
        if self.orb {
            self.side_slot(m)
        } else {
            let (outer, _) = self.start_across(m);
            m.start_button_width + self.start.y + outer
        }
    }

    /// Width of a side bar `wanted` px wide (0: default), at least one slot, at most a third of the output.
    pub fn side_width(&self, m: &Metrics, wanted: i32, output_width: i32) -> i32 {
        let slot = self.side_slot(m);
        let w = if wanted <= 0 {
            self.side_default(m)
        } else {
            wanted
        };
        w.clamp(slot, (output_width / 3).max(slot))
    }

    fn start_across(&self, m: &Metrics) -> (i32, i32) {
        let btn = m.taskbar_height - self.button_top - self.button_bottom;
        let h = if self.start.height == 0 {
            btn
        } else {
            self.start.height.min(m.taskbar_height - self.start.y)
        };
        ((m.taskbar_height - self.start.y - h).max(0), h)
    }
}

/// Where the Start menu meets a bottom bar; other edges turn the dock ([`StartMenuDock::menu_at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartMenuDock {
    /// Px down into the bar; its shadow falls on the bar.
    Into(i32),
    /// On the bar's top edge, casting no shadow on the bar.
    OnTop,
}

impl StartMenuDock {
    /// Start of a Start menu `extent` across the bar, opening from a bar along `edge` whose long edges are `bar.0` and `bar.1`.
    pub fn menu_at(self, edge: Edge, bar: (i32, i32), extent: i32) -> i32 {
        match (edge, self) {
            (Edge::Bottom | Edge::Right, StartMenuDock::Into(n)) => bar.0 + n - extent,
            (Edge::Bottom | Edge::Right, StartMenuDock::OnTop) => bar.0 - extent,
            (Edge::Top | Edge::Left, StartMenuDock::Into(n)) => bar.1 - n,
            (Edge::Top | Edge::Left, StartMenuDock::OnTop) => bar.1,
        }
    }

    /// Px from its top the Start menu's shadow may reach on a bar along `edge`; None: all of it, 0: none.
    pub fn shadow_floor(self, edge: Edge) -> Option<i32> {
        match (edge, self) {
            (Edge::Bottom | Edge::Right, StartMenuDock::OnTop) => Some(0),
            _ => None,
        }
    }

    /// Same for a submenu whose top is `menu_top`; None: it may fall on the bar.
    pub fn submenu_shadow_floor(self, edge: Edge, bar_top: i32, menu_top: i32) -> Option<i32> {
        match (edge, self) {
            (Edge::Bottom, StartMenuDock::OnTop) => Some((bar_top - menu_top).max(0)),
            _ => None,
        }
    }
}

/// A Start orb whose art reaches past the bar: a disc `diameter` across, `rise` above the bar, art reaching `reach`. A Top bar turns it over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrbRise {
    pub rise: i32,
    pub diameter: i32,
    pub reach: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskRow {
    Shared,
    /// Labelled buttons never narrower than this; the ones that no longer fit are left out.
    AtLeast(i32),
    Icons,
}

/// The shortest bar, at scale 1, that sets its clock on two lines.
pub const TWO_LINE_CLOCK_MIN: i32 = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BarSpan {
    pub edge: Edge,
    pub length: i32,
    pub thickness: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClockText {
    pub time: i32,
    pub date: i32,
}

/// Least room past `text_x` that labels a side bar's buttons; narrower shows icons only.
pub const LABEL_ROOM: i32 = 24;

const SIDE_CLOCK_LINES: [i32; 2] = [17, 34];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskbarLayout {
    pub width: i32,
    pub height: i32,
    /// Top bar rects are the Bottom bar's turned over; a side bar's are its column's. Painters turn the art, not the buttons.
    pub edge: Edge,
    pub start: Rect,
    pub grips: Vec<Rect>,
    /// Quick Launch buttons shown; the hidden rest are the items past them, behind `quick_more`.
    pub quick: Vec<Rect>,
    /// The » cell ending a Quick Launch band too short for its buttons.
    pub quick_more: Option<Rect>,
    pub pager: Vec<Rect>,
    pub tasks: Vec<Rect>,
    /// Windows per task button (2+ draws stacked edges); 0 is a pinned app with no windows, a bare icon.
    pub stacks: Vec<u8>,
    pub icons_only: bool,
    /// With a [`TrayArrow`], the arrow's box before the open run.
    pub chevron: Option<Rect>,
    pub tray: Vec<Rect>,
    /// Leading `tray` cells that are hidden items slid out by an open run.
    pub tray_hidden: usize,
    /// Px a tray arrow's run opens fully: [`reveal_width`] of the hidden
    /// items that fit on the bar, whatever the run's `reveal`.
    pub run_width: i32,
    pub tray_arrow: bool,
    pub clock: Rect,
    pub notification: Rect,
    pub show_desktop: Option<Rect>,
    pub more: Option<Rect>,
    /// Strip down an unlocked side bar's inner edge that drags its width; takes the pointer over what lies under it.
    pub resize: Option<Rect>,
    pub clock_lines: Option<[i32; 2]>,
    /// Rows past the bar's screen-side edge covered by the orb's art, and the disc's box when it reaches past the bar.
    pub start_above: Option<Rect>,
    pub start_disc: Option<Rect>,
    /// Disc rows past the bar's edge; 0 without a disc.
    disc_hang: i32,
    tasks_x: i32,
    tasks_end: i32,
    band_x: i32,
    row: (i32, i32),
    button: i32,
    /// Whether the row may label its buttons: always on Bottom/Top bars, on a side bar once the row is `LABEL_ROOM` past `text_x`.
    labels: bool,
    pub bands: Bands,
}

/// Quick Launch button width: 16 px icon, 3 px padding, 1 px frame each side. A pager button is as wide.
const QUICK_W: i32 = 23;
/// Width of the » cell ending a Quick Launch band too short for its buttons on a Bottom or Top bar; down a side bar it is a `QUICK_W` cell.
const QUICK_MORE_W: i32 = 14;
const PAGER_GAP: i32 = 4;
const SIDE_TRAY_MARGIN: i32 = 2;

/// Grip handle: 3 px wide, 18 px tall, 4 px before its band.
const GRIP_W: i32 = 3;
const GRIP_H: i32 = 18;

pub const RESIZE_W: i32 = 3;
/// Gap between task buttons.
pub const TASK_GAP: i32 = 3;
const GRIP_GAP: i32 = 4;
const GRIP_LEAD: i32 = GRIP_W + GRIP_GAP;
/// After a band's content: locked, the next band starts 4 px on; unlocked, its grip sits 6 px on and its content 13 px on.
const BAND_GAP: i32 = 4;
const BAND_GAP_GRIP: i32 = 6 + GRIP_LEAD;
const TASKS_MIN_W: i32 = 66;

struct Arranged {
    tools: i32,
    /// Length the tools band is laid in: its natural length, or less when shrunk.
    tools_len: i32,
    tasks: i32,
    tasks_end: i32,
    bands: Bands,
    grip: Option<i32>,
}

/// Lay the tools band (`tools`: least, natural length) and the task row from `band_x` to `limit` in `bands` order; the first is `first_width` long, clamped so the second keeps its least length. Without a dragged width the task row leaves the tools band whole.
fn arrange_bands(
    band_x: i32,
    limit: i32,
    tools: (i32, i32),
    task: (i32, i32),
    bands: Bands,
    locked: bool,
    tasks: usize,
) -> Arranged {
    if tools.1 == 0 {
        return Arranged {
            tools: band_x,
            tools_len: 0,
            tasks: band_x,
            tasks_end: limit,
            bands: Bands::default(),
            grip: None,
        };
    }
    let step = if locked { BAND_GAP } else { BAND_GAP_GRIP };
    let (min, natural, second) = if bands.tasks_first {
        let second = if bands.first_width.is_some() {
            tools.0
        } else {
            tools.1
        };
        (task.0, task.1, second)
    } else {
        (tools.0, tools.1, 0)
    };
    let max = (limit - band_x - step - second).max(min);
    let w = bands.first_width.unwrap_or(natural).clamp(min, max);
    let end = band_x + w;
    let grip = (!locked && (tasks > 0 || bands.tasks_first)).then_some(end + 6);
    let laid = Bands {
        tasks_first: bands.tasks_first,
        first_width: Some(w),
    };
    if bands.tasks_first {
        Arranged {
            tools: end + step,
            tools_len: (limit - end - step).clamp(tools.0, tools.1),
            tasks: band_x,
            tasks_end: end.min(limit),
            bands: laid,
            grip,
        }
    } else {
        Arranged {
            tools: band_x,
            tools_len: w,
            tasks: end + step,
            tasks_end: limit,
            bands: laid,
            grip,
        }
    }
}

/// Where the tray row (less its 4 px lead) may start at the earliest so
/// `arrange_bands` still fits the tools band (`tools`: least, natural length)
/// whole, or as dragged when it comes first: `band_x` with no tools, else past
/// their smallest arrangement.
fn bands_floor(band_x: i32, tools: (i32, i32), task_min: i32, bands: Bands, locked: bool) -> i32 {
    if tools.1 == 0 {
        return band_x;
    }
    let step = if locked { BAND_GAP } else { BAND_GAP_GRIP };
    let first = match (bands.tasks_first, bands.first_width) {
        (true, _) => task_min + step + tools.1,
        (false, None) => tools.1 + step,
        (false, Some(w)) => w.clamp(tools.0, tools.1) + step,
    };
    band_x + first + 4
}

impl TaskbarLayout {
    pub fn compute(
        m: &Metrics,
        shape: &TaskbarShape,
        width: i32,
        locked: bool,
        counts: TaskbarCounts,
        clock_text_width: i32,
    ) -> Self {
        Self::compute_with(
            m,
            shape,
            width,
            locked,
            counts,
            clock_text_width,
            Bands::default(),
        )
    }

    pub fn compute_with(
        m: &Metrics,
        shape: &TaskbarShape,
        width: i32,
        locked: bool,
        counts: TaskbarCounts,
        clock_text_width: i32,
        bands: Bands,
    ) -> Self {
        let span = BarSpan {
            edge: Edge::Bottom,
            length: width,
            thickness: m.taskbar_height,
        };
        let clock = ClockText {
            time: clock_text_width,
            date: clock_text_width,
        };
        Self::compute_on(m, shape, span, locked, counts, clock, bands)
    }

    /// Lay the bar out along `span` in `shape`. Locked, the first band starts `start_gap` after Start and the second `BAND_GAP` after the first; unlocked, each band has a grip 7 px before its content. A Top bar is the Bottom bar turned over; a side bar is a column (see `column`).
    pub fn compute_on(
        m: &Metrics,
        shape: &TaskbarShape,
        span: BarSpan,
        locked: bool,
        counts: TaskbarCounts,
        clock: ClockText,
        bands: Bands,
    ) -> Self {
        let TaskbarCounts {
            quick: quick_count,
            pager: pager_count,
            tasks: task_count,
            tray: tray_count,
            show_desktop,
            chevron,
            hidden: counts_hidden,
            reveal,
        } = counts;
        if span.edge.is_side() {
            return Self::column(m, shape, span, locked, counts, clock, bands);
        }
        let (width, height) = (span.length, span.thickness);
        let clock_text_width = if shape.clock_baselines(height).is_some() {
            clock.time.max(clock.date)
        } else {
            clock.time
        };
        let btn_h = height - shape.button_top - shape.button_bottom;
        let strip = show_desktop.then(|| {
            let w = shape.show_desktop_width.unwrap_or(btn_h);
            let margin = if shape.well_full_height { 0 } else { 2 };
            Rect::new(width - margin - w, shape.button_top, w, btn_h)
        });
        let end = strip.map_or(width, |r| r.x);
        let start_h = if shape.start.height == 0 {
            btn_h
        } else {
            shape.start.height.min(height - shape.start.y)
        };
        let start = Rect::new(shape.start.x, shape.start.y, m.start_button_width, start_h);
        let mut grips = Vec::new();
        let grip_y = shape.button_top + (btn_h - GRIP_H) / 2;
        let mut x = start.right() + shape.start_gap;
        if !locked {
            grips.push(Rect::new(x + 2, grip_y, GRIP_W, GRIP_H));
            x += 2 + GRIP_LEAD;
        }
        let band_x = x;
        let cb = shape.clock;
        let clock_w = (clock_text_width + cb.pad).max(cb.min_width);
        let clock_x = (end - cb.right - clock_w).max(band_x);
        let clock = Rect::new(clock_x, shape.button_top, clock_w, btn_h);
        let icon = ICON_SIZE;
        let tray_y = shape.button_top + (btn_h - icon + i32::from(shape.tray_low)) / 2;
        // Right to left from 6 px before the clock: shown cells, then the hidden cells whole inside the open run, then the ^ cell or arrow. Cells that would reach the clock are left out.
        let pitch = icon + shape.tray_gap;
        let arrow = shape.tray_arrow.filter(|_| chevron);
        let shown_n = tray_count as i32;
        let foot = clock.x - 6;
        let cell_x = |k: i32| foot - icon - k * pitch;
        let quick_w = quick_count as i32 * QUICK_W;
        let pager_gap = if quick_count > 0 && pager_count > 0 {
            PAGER_GAP
        } else {
            0
        };
        let tools_w = quick_w + pager_gap + pager_count as i32 * QUICK_W;
        let pager_w = pager_count as i32 * QUICK_W;
        let tools_least = if quick_count > 0 {
            pager_w + pager_gap + QUICK_MORE_W
        } else {
            tools_w
        };
        // A capped run stops short of the tools band, so the arrow is never under it.
        let floor = bands_floor(band_x, (tools_least, tools_w), TASKS_MIN_W, bands, locked);
        let hidden = arrow.map_or(0, |a| {
            let room = cell_x(shown_n - 1) - a.gap - a.size.0 - floor;
            counts_hidden.min((room.max(0) / pitch) as usize)
        });
        let run_width = reveal_width(shape, hidden);
        let open = reveal.clamp(0, run_width);
        let run = revealed(shape, hidden, reveal) as i32;
        let cell = |x: i32| Rect::new(x, tray_y, icon, icon);
        let mut rects: Vec<Rect> = Vec::new();
        if chevron || tray_count > 0 {
            match arrow {
                Some(a) => {
                    // The arrow sits `gap` before the first cell, but shut never past the row's foot.
                    let lead = cell_x(shown_n - 1);
                    let (w, h) = a.size;
                    let y = tray_y + (icon - h).div_euclid(2);
                    let x = (lead - a.gap - w - open).min(foot - w);
                    rects.push(Rect::new(x, y, w, h));
                    rects.extend((0..run).rev().map(|j| cell(lead - (j + 1) * pitch)));
                }
                None if chevron => rects.push(cell(cell_x(shown_n))),
                None => {}
            }
            rects.extend((0..shown_n).rev().map(|k| cell(cell_x(k))));
        }
        let shift = rects.first().map_or(0, |r| (band_x - r.x).max(0));
        let mut tray: Vec<Rect> = Vec::with_capacity(rects.len());
        for r in rects {
            let r = Rect::new(r.x + shift, r.y, r.width, r.height);
            if r.right() > foot {
                break;
            }
            tray.push(r);
        }
        let chevron = if chevron && !tray.is_empty() {
            Some(tray.remove(0))
        } else {
            None
        };
        let tray_hidden = (run as usize).min(tray.len());
        let row_x = chevron.or(tray.first().copied()).map(|r| r.x);
        // 4 px before the tray row, or 6 px before the clock without a tray.
        let limit = match row_x {
            Some(x) => x - 4,
            None => clock.x - 6,
        };
        let a = arrange_bands(
            band_x,
            limit,
            (tools_least, tools_w),
            (TASKS_MIN_W, m.task_button_max_width),
            bands,
            locked,
            task_count,
        );
        if let Some(g) = a.grip {
            grips.push(Rect::new(g, grip_y, GRIP_W, GRIP_H));
        }
        let pager_x = a.tools;
        let pager: Vec<Rect> = (0..pager_count as i32)
            .map(|i| Rect::new(pager_x + i * QUICK_W, shape.button_top, QUICK_W, btn_h))
            .collect();
        let quick_x = pager_x + pager_w + pager_gap;
        let quick_room = a.tools_len - pager_w - pager_gap;
        let shown = if quick_room >= quick_w {
            quick_count
        } else {
            (((quick_room - QUICK_MORE_W).max(0) / QUICK_W) as usize).min(quick_count)
        };
        let quick: Vec<Rect> = (0..shown as i32)
            .map(|i| Rect::new(quick_x + i * QUICK_W, shape.button_top, QUICK_W, btn_h))
            .collect();
        let quick_more = (shown < quick_count).then(|| {
            let x = quick_x + shown as i32 * QUICK_W;
            Rect::new(x, shape.button_top, QUICK_MORE_W, btn_h)
        });
        // One well around the tray and clock, starting `well_lead` left of the ^ cell or first icon; full height in `well_full_height` shapes, else sunken in the button row.
        let lead = match (arrow, chevron) {
            (Some(a), Some(_)) => a.well_lead,
            _ => shape.well_lead,
        };
        let well_x = row_x.map(|x| x - lead).unwrap_or(clock.x);
        let notification = if shape.well_full_height {
            Rect::new(well_x, 0, end - well_x, height)
        } else {
            Rect::new(well_x, clock.y, clock.right() - well_x, clock.height)
        };
        let mut layout = TaskbarLayout {
            width,
            edge: Edge::Bottom,
            height,
            start,
            grips,
            quick,
            quick_more,
            pager,
            tasks: Vec::with_capacity(task_count),
            stacks: Vec::with_capacity(task_count),
            icons_only: false,
            chevron,
            tray,
            tray_hidden,
            run_width,
            tray_arrow: shape.tray_arrow.is_some(),
            tasks_x: a.tasks,
            tasks_end: a.tasks_end,
            band_x,
            row: (shape.button_top, btn_h),
            button: btn_h,
            labels: true,
            more: None,
            resize: None,
            clock_lines: shape.clock_baselines(height),
            bands: a.bands,
            clock,
            notification,
            show_desktop: strip,
            start_above: shape
                .orb_rise
                .map(|o| Rect::new(start.x, -o.reach, start.width, o.reach)),
            start_disc: shape.orb_rise.filter(|o| o.rise > 0).map(|o| {
                Rect::new(
                    start.x + (start.width - o.diameter) / 2,
                    -o.rise,
                    o.diameter,
                    o.diameter,
                )
            }),
            disc_hang: shape.orb_rise.map_or(0, |o| o.rise.max(0)),
        };
        layout.lay_tasks(m, shape, &vec![1; task_count], TaskRow::Shared);
        layout.on_edge(span.edge)
    }

    /// A side bar's column along `span`, in its own x/y: x across from its left, y down. From the top: Start, the bands (Quick Launch and pager in rows of `QUICK_W` cells, then the task row); at the foot the tray grid, the clock and the Show desktop strip. With an orb Start the grids are centred.
    fn column(
        m: &Metrics,
        shape: &TaskbarShape,
        span: BarSpan,
        locked: bool,
        counts: TaskbarCounts,
        clock: ClockText,
        bands: Bands,
    ) -> Self {
        let TaskbarCounts {
            quick: quick_count,
            pager: pager_count,
            tasks: task_count,
            tray: tray_count,
            show_desktop,
            chevron,
            hidden: counts_hidden,
            reveal,
        } = counts;
        let (t, len) = (span.thickness, span.length);
        let btn = m.taskbar_height - shape.button_top - shape.button_bottom;
        let (left, right) = if span.edge == Edge::Left {
            (shape.button_bottom, shape.button_top)
        } else {
            (shape.button_top, shape.button_bottom)
        };
        let room = (t - left - right).max(1);
        let (s_out, s_h) = shape.start_across(m);
        let rise = shape.orb_rise.map_or(0, |o| o.rise.max(0));
        let sw = m.start_button_width.min(t);
        let sx = if shape.orb {
            (t - sw) / 2
        } else {
            let x = if span.edge == Edge::Left {
                s_out
            } else {
                t - s_out - sw
            };
            x.clamp(0, t - sw)
        };
        let start = Rect::new(sx, shape.start.x, sw, s_h + rise);
        let mut grips = Vec::new();
        let gw = GRIP_H.min(room);
        let grip = |y: i32| Rect::new(left + (room - gw) / 2, y, gw, GRIP_W);
        let mut y = start.bottom() + shape.start_gap;
        if !locked {
            grips.push(grip(y + 2));
            y += 2 + GRIP_LEAD;
        }
        let band_y = y;
        let strip = show_desktop.then(|| {
            let h = shape.show_desktop_width.unwrap_or(btn);
            if shape.well_full_height {
                Rect::new(0, len - h, t, h)
            } else {
                Rect::new(left, len - 2 - h, room, h)
            }
        });
        let end = strip.map_or(len, |r| r.y);
        let cb = shape.clock;
        let date = clock.date > 0 && clock.date + cb.pad <= room;
        let clock_h = if date {
            btn.max(TWO_LINE_CLOCK_MIN)
        } else {
            btn
        };
        let clock_r = Rect::new(left, (end - cb.right - clock_h).max(band_y), room, clock_h);
        let clock_lines = date.then(|| shape.two_line_clock.unwrap_or(SIDE_CLOCK_LINES));
        // Start x of a grid: widest row centred with an orb, else on the row's left.
        let grid_x = |n: i32, per: i32, pitch: i32, gap: i32| {
            if shape.orb {
                left + (room - n.min(per) * pitch + gap) / 2
            } else {
                left
            }
        };
        let icon = ICON_SIZE;
        let pitch = icon + shape.tray_gap;
        let margin = if shape.orb {
            SIDE_TRAY_MARGIN
        } else {
            shape.side_tray_inset
        };
        let per_row = ((room - 2 * margin + shape.tray_gap) / pitch).max(1);
        let foot = clock_r.y - 6;
        // An open run lays the hidden cells ahead of the shown ones, so the grid grows up from the clock.
        let arrow = shape.tray_arrow.filter(|_| chevron);
        let per_line = (room / QUICK_W).max(1);
        let lines = |n: usize| n.div_ceil(per_line as usize) as i32 * QUICK_W;
        let pager_gap = if quick_count > 0 && pager_count > 0 {
            PAGER_GAP
        } else {
            0
        };
        let tools = lines(quick_count) + pager_gap + lines(pager_count);
        let tools_least = if quick_count > 0 {
            lines(pager_count) + pager_gap + QUICK_W
        } else {
            tools
        };
        let floor = bands_floor(band_y, (tools_least, tools), btn, bands, locked);
        let fit = arrow.map_or(0, |a| {
            let high = foot - floor - a.side_size.1 - a.side_gap;
            let rows = ((high + shape.tray_gap).max(0) / pitch) as usize;
            counts_hidden.min((rows * per_row as usize).saturating_sub(tray_count))
        });
        let run_width = reveal_width(shape, fit);
        let hidden = if reveal > 0 { fit } else { 0 };
        let (well_x, well_w) = if shape.well_full_height {
            (0, t)
        } else {
            (left, room)
        };
        // A ^ cell leads the grid; a tray arrow heads it `side_gap` over its first row and is left out if it would reach into the clock.
        let cells = (tray_count + hidden + usize::from(chevron && arrow.is_none())) as i32;
        let rows = (cells as usize).div_ceil(per_row as usize) as i32;
        let grid_h = (rows * pitch - shape.tray_gap).max(0);
        let arrow_r = arrow
            .map(|a| {
                let (w, h) = a.side_size;
                let w = w.unwrap_or(well_w - 4);
                let below = if cells > 0 { a.side_gap + grid_h } else { 0 };
                let y = (foot - below - h).max(band_y);
                let inner = if span.edge == Edge::Right {
                    a.side_inset
                } else {
                    0
                };
                Rect::new(well_x + inner + (well_w - a.side_inset - w) / 2, y, w, h)
            })
            .filter(|r| r.bottom() <= foot);
        let mut tray = Vec::with_capacity(cells as usize);
        if cells > 0 {
            let top = match (arrow, arrow_r) {
                (Some(a), Some(r)) => r.bottom() + a.side_gap,
                _ => (foot - grid_h).max(band_y),
            };
            let inset = if shape.orb { 0 } else { shape.side_tray_inset };
            let x0 = grid_x(cells, per_row, pitch, shape.tray_gap) + inset;
            for i in 0..cells {
                let r = Rect::new(
                    x0 + i % per_row * pitch,
                    top + i / per_row * pitch,
                    icon,
                    icon,
                );
                if r.bottom() > foot {
                    break;
                }
                tray.push(r);
            }
        }
        let chevron = if arrow.is_some() {
            arrow_r
        } else if chevron && !tray.is_empty() {
            Some(tray.remove(0))
        } else {
            None
        };
        let tray_hidden = hidden.min(tray.len());
        let row_y = chevron.or(tray.first().copied()).map(|r| r.y);
        let limit = match row_y {
            Some(y) => y - 4,
            None => clock_r.y - 6,
        };
        let lead = arrow.map_or(shape.well_lead, |a| a.side_lead);
        let well_y = row_y.map_or(clock_r.y, |y| y - lead);
        let notification = if shape.well_full_height {
            Rect::new(0, well_y, t, (end - well_y).max(0))
        } else {
            Rect::new(left, well_y, room, clock_r.bottom() - well_y)
        };
        let a = arrange_bands(
            band_y,
            limit,
            (tools_least, tools),
            (btn, btn),
            bands,
            locked,
            task_count,
        );
        if let Some(g) = a.grip {
            grips.push(grip(g));
        }
        let cell = |top: i32, n: usize, i: usize| {
            let i = i as i32;
            Rect::new(
                grid_x(n as i32, per_line, QUICK_W, 0) + i % per_line * QUICK_W,
                top + i / per_line * QUICK_W,
                QUICK_W,
                QUICK_W,
            )
        };
        let pager = (0..pager_count)
            .map(|i| cell(a.tools, pager_count, i))
            .collect();
        let quick_y = a.tools + lines(pager_count) + pager_gap;
        let fits = ((a.tools_len - lines(pager_count) - pager_gap) / QUICK_W).max(1) as usize
            * per_line as usize;
        let shown = if fits >= quick_count {
            quick_count
        } else {
            fits - 1
        };
        let quick = (0..shown).map(|i| cell(quick_y, quick_count, i)).collect();
        let quick_more = (shown < quick_count).then(|| cell(quick_y, quick_count, shown));
        let mut layout = TaskbarLayout {
            width: t,
            height: len,
            edge: span.edge,
            start,
            grips,
            quick,
            quick_more,
            pager,
            tasks: Vec::with_capacity(task_count),
            stacks: Vec::with_capacity(task_count),
            icons_only: false,
            chevron,
            tray,
            tray_hidden,
            run_width,
            tray_arrow: shape.tray_arrow.is_some(),
            clock: clock_r,
            notification,
            show_desktop: strip,
            more: None,
            resize: (!locked).then(|| {
                let x = if span.edge == Edge::Left {
                    t - RESIZE_W
                } else {
                    0
                };
                Rect::new(x, 0, RESIZE_W, len)
            }),
            clock_lines,
            start_above: None,
            start_disc: shape
                .orb_rise
                .filter(|o| o.rise > 0)
                .map(|o| Rect::new(sx + (sw - o.diameter) / 2, start.y, o.diameter, o.diameter)),
            disc_hang: rise,
            tasks_x: a.tasks,
            tasks_end: a.tasks_end,
            band_x: band_y,
            row: (left, room),
            button: btn,
            labels: room >= shape.labelled.text_x + LABEL_ROOM,
            bands: a.bands,
        };
        layout.lay_tasks(m, shape, &vec![1; task_count], TaskRow::Shared);
        layout
    }

    /// `lay_tasks` on a side bar: one button per row, labelled when `labels`. When they do not fit they shrink to icon plus 3 px each side, then the rest wait behind the » cell.
    fn lay_column(&mut self, shape: &TaskbarShape, stacks: &[u8], row: TaskRow) {
        let (x, w) = self.row;
        let full = self.button;
        self.icons_only = self.column_icons_only(row);
        let gap = self.column_gap(shape, row);
        let least = (shape.task_icon + 6).min(full);
        let room = (self.tasks_end - self.tasks_x).max(0);
        let n = stacks.len() as i32;
        let takes = |h: i32| n * h + (n - 1).max(0) * gap;
        let h = if takes(full) <= room {
            full
        } else {
            ((room - (n - 1) * gap) / n).clamp(least, full)
        };
        self.tasks.clear();
        self.stacks.clear();
        self.more = None;
        let mut end = self.tasks_end;
        if takes(h) > room && least <= room {
            let more = Rect::new(x, self.tasks_end - least, w, least);
            end = more.y - gap;
            self.more = Some(more);
        }
        let mut y = self.tasks_x;
        for &stack in stacks {
            if y + h > end {
                break;
            }
            self.tasks.push(Rect::new(x, y, w, h));
            self.stacks.push(stack);
            y += h + gap;
        }
    }

    fn column_icons_only(&self, row: TaskRow) -> bool {
        row == TaskRow::Icons || !self.labels
    }

    /// Gap between a side bar's task buttons: the icon button gap when icons only, else `labelled.gap`.
    pub fn column_gap(&self, shape: &TaskbarShape, row: TaskRow) -> i32 {
        if self.column_icons_only(row) {
            let (w, pitch) = self.icon_step(shape);
            pitch - w
        } else {
            shape.labelled.gap
        }
    }

    /// Lay the task row again, one button per entry of `stacks` (windows each stands for). A stack of 0 (a pinned app) takes an icon button's size; buttons that would overrun the tray or clock are left out.
    pub fn lay_tasks(&mut self, m: &Metrics, shape: &TaskbarShape, stacks: &[u8], row: TaskRow) {
        if self.edge.is_side() {
            self.lay_column(shape, stacks, row);
            return;
        }
        let (icon_w, icon_pitch) = self.icon_step(shape);
        let limit = self.tasks_limit();
        let bare = stacks.iter().filter(|&&s| s == 0).count() as i32;
        let n = stacks.len() as i32 - bare;
        let gap = shape.labelled.gap;
        // A labelled button followed by a bare pinned icon is `icon_gap`
        // from it rather than `gap`.
        let icon_gap = icon_pitch - icon_w;
        let before_bare = stacks.windows(2).filter(|p| p[0] != 0 && p[1] == 0).count() as i32;
        let shared = || {
            if n == 0 {
                return 1;
            }
            let gaps = gap * (n - 1) + (icon_gap - gap) * before_bare;
            ((limit - self.tasks_x - bare * icon_pitch - gaps) / n).min(m.task_button_max_width)
        };
        let (w, pitch) = match row {
            TaskRow::Shared => {
                let w = shared().max(1);
                (w, w + gap)
            }
            TaskRow::AtLeast(min) => {
                let w = shared().max(min).min(m.task_button_max_width).max(1);
                (w, w + gap)
            }
            TaskRow::Icons => (icon_w, icon_pitch),
        };
        self.tasks.clear();
        self.stacks.clear();
        self.icons_only = row == TaskRow::Icons;
        let mut x = self.tasks_x;
        for (i, &stack) in stacks.iter().enumerate() {
            let (w, pitch) = if stack == 0 {
                (icon_w, icon_pitch)
            } else if row != TaskRow::Icons && stacks.get(i + 1) == Some(&0) {
                (w, w + icon_gap)
            } else {
                (w, pitch)
            };
            if x + w > limit {
                break;
            }
            let r = self.to_xy(Rect::new(x, self.row.0, w, self.row.1));
            self.tasks.push(r);
            self.stacks.push(stack);
            x += pitch;
        }
    }

    /// Icon-only task button length and step to the next one.
    pub fn icon_step(&self, shape: &TaskbarShape) -> (i32, i32) {
        let (w, pitch) = shape
            .icon_button
            .unwrap_or((self.button, self.button + TASK_GAP));
        if self.edge.is_side() {
            (self.button, self.button + pitch - w)
        } else {
            (w, pitch)
        }
    }

    /// Least room along the bar a labelled button takes before one app's windows share a button.
    pub fn least_button(&self, row_least: i32) -> i32 {
        if self.edge.is_side() {
            self.button
        } else {
            row_least
        }
    }

    pub fn pinned_idle(&self, i: usize) -> bool {
        self.stacks.get(i) == Some(&0)
    }

    pub fn tasks_x(&self) -> i32 {
        self.tasks_x
    }

    /// Gap a Quick Launch button dragged to bar-local (`x`, `y`) drops into: 0 before the first, `quick.len()` after the last.
    pub fn drop_slot(&self, x: i32, y: i32) -> usize {
        if self.edge.is_side() {
            return self
                .quick
                .iter()
                .filter(|r| r.bottom() <= y || (y >= r.y && x >= r.x + r.width / 2))
                .count();
        }
        slot_at(&self.along_rects(&self.quick), self.along(x, y))
    }

    /// End of the task band when first, else 4 px before the tray row or 6 px before the clock.
    pub fn tasks_limit(&self) -> i32 {
        self.tasks_end
    }

    /// Whether (`x`, `y`) is on the Quick Launch band, so a drop there adds a button.
    pub fn on_quick_band(&self, x: i32, y: i32) -> bool {
        let at = self.along(x, y);
        let along = |r: Rect| self.to_along(r);
        let pager_end = self.pager.last().map(|&p| along(p).right());
        let last = (self.quick_more.or(self.quick.last().copied())).map(|r| along(r).right());
        let end = if self.bands.tasks_first {
            let tray = match self.chevron.or(self.tray.first().copied()) {
                Some(t) => along(t).x - 4,
                None => along(self.clock).x - 6,
            };
            tray.max(last.or(pager_end).unwrap_or(tray))
        } else {
            self.tasks_x
        };
        match (self.quick.first().or(self.quick_more.as_ref()), pager_end) {
            (Some(&first), _) => {
                let start = pager_end.unwrap_or(along(first).x - GRIP_LEAD);
                at >= start && at < end
            }
            (None, Some(p)) => at >= p && at < end,
            (None, None) => at < self.tasks_x,
        }
    }

    pub fn band_at_grip(&self, index: usize) -> Band {
        match (index == 0, self.bands.tasks_first) {
            (true, false) | (false, true) => Band::Quick,
            _ => Band::Tasks,
        }
    }

    /// Arrangement after `band`'s grip is dragged to `grip_at`: the second band's grip sets the first band's width; the first band's grip does nothing until it passes the second's, which swaps them.
    pub fn drag_band(&self, bands: Bands, band: Band, grip_at: i32) -> Bands {
        if self.grips.len() < 2 {
            return bands;
        }
        let width = Some(grip_at - 6 - self.band_x);
        if band != self.band_at_grip(0) {
            Bands {
                tasks_first: bands.tasks_first,
                first_width: width,
            }
        } else if grip_at >= self.to_along(self.grips[1]).x {
            Bands {
                tasks_first: !bands.tasks_first,
                first_width: width,
            }
        } else {
            bands
        }
    }

    pub fn length(&self) -> i32 {
        if self.edge.is_side() {
            self.height
        } else {
            self.width
        }
    }

    pub fn thickness(&self) -> i32 {
        if self.edge.is_side() {
            self.width
        } else {
            self.height
        }
    }

    pub fn to_xy(&self, r: Rect) -> Rect {
        self.edge.to_xy(r, self.thickness())
    }

    pub fn to_along(&self, r: Rect) -> Rect {
        self.edge.to_xy(r, self.thickness())
    }

    pub fn along_rects(&self, v: &[Rect]) -> Vec<Rect> {
        v.iter().map(|&r| self.to_along(r)).collect()
    }

    pub fn along(&self, x: i32, y: i32) -> i32 {
        self.to_along(Rect::new(x, y, 1, 1)).x
    }

    /// Pointer coordinate along the bar's axis, unrounded.
    pub fn along_f(&self, x: f64, y: f64) -> f64 {
        if self.edge.is_side() { y } else { x }
    }

    pub fn quick_drag(&self, dx: f64, dy: f64) -> f64 {
        if self.edge.is_side() {
            dx.abs().max(dy.abs())
        } else {
            dx.abs()
        }
    }

    pub fn moved_along(&self, r: Rect, by: i32) -> Rect {
        let a = self.to_along(r);
        self.to_xy(Rect::new(a.x + by, a.y, a.width, a.height))
    }

    pub fn placed_along(&self, r: Rect, at: i32) -> Rect {
        let a = self.to_along(r);
        self.to_xy(Rect::new(at, a.y, a.width, a.height))
    }

    /// This layout on a bar along `edge`: Bottom to Top turns every rect over, the orb's rows too. A side bar is laid out by `compute_on`; turning to or from one panics.
    pub fn on_edge(self, edge: Edge) -> TaskbarLayout {
        if edge == self.edge {
            return self;
        }
        assert!(
            !edge.is_side() && !self.edge.is_side(),
            "on_edge turns a Bottom bar into a Top one and back; compute_on lays a side bar out"
        );
        let (from, t) = (self.edge, self.thickness());
        let m = move |r: Rect| edge.to_xy(from.to_xy(r, t), t);
        let down = if edge == Edge::Top {
            self.disc_hang
        } else {
            -self.disc_hang
        };
        let all = move |v: Vec<Rect>| -> Vec<Rect> { v.into_iter().map(m).collect() };
        TaskbarLayout {
            edge,
            start: m(self.start),
            grips: all(self.grips),
            quick: all(self.quick),
            quick_more: self.quick_more.map(m),
            pager: all(self.pager),
            tasks: all(self.tasks),
            chevron: self.chevron.map(m),
            tray: all(self.tray),
            clock: m(self.clock),
            notification: m(self.notification),
            show_desktop: self.show_desktop.map(m),
            start_above: self.start_above.map(m),
            start_disc: self
                .start_disc
                .map(|d| Rect::new(d.x, d.y + down, d.width, d.height)),
            ..self
        }
    }

    /// Rows an auto-hidden bar leaves on its output: its inner edge, `REVEAL_LINE` thick.
    pub fn reveal_line(&self) -> Rect {
        let t = self.thickness();
        let h = REVEAL_LINE.min(t).max(0);
        match self.edge {
            Edge::Bottom => Rect::new(0, 0, self.width, h),
            Edge::Top => Rect::new(0, t - h, self.width, h),
            Edge::Left => Rect::new(t - h, 0, h, self.height),
            Edge::Right => Rect::new(0, 0, h, self.height),
        }
    }

    /// The cell under (x, y); a point between the screen edge and a cell, with nothing between them, takes that cell.
    pub fn hit(&self, x: i32, y: i32) -> TaskbarHit {
        let hit = self.hit_drawn(x, y);
        let inside = |x: i32, y: i32| x >= 0 && x < self.width && y >= 0 && y < self.height;
        if hit != TaskbarHit::Nothing || !inside(x, y) {
            return hit;
        }
        let (dx, dy) = match self.edge {
            Edge::Bottom => (0, -1),
            Edge::Top => (0, 1),
            Edge::Left => (1, 0),
            Edge::Right => (-1, 0),
        };
        let (mut x, mut y) = (x + dx, y + dy);
        while inside(x, y) {
            match self.hit_drawn(x, y) {
                TaskbarHit::Nothing => (x, y) = (x + dx, y + dy),
                TaskbarHit::Resize => break,
                hit => return hit,
            }
        }
        TaskbarHit::Nothing
    }

    fn hit_drawn(&self, x: i32, y: i32) -> TaskbarHit {
        if self.disc_past_bar().is_some_and(|d| d.contains(x, y)) && self.on_disc(x, y) {
            return TaskbarHit::Start;
        }
        if y < 0 || y >= self.height || x < 0 || x >= self.width {
            return TaskbarHit::Nothing;
        }
        if self.resize.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::Resize;
        }
        if self.start.contains(x, y) {
            return TaskbarHit::Start;
        }
        // A grip takes 2 px more each way along the bar, and the bar's
        // thickness but 2 px each side across it.
        let p = self.to_along(Rect::new(x, y, 1, 1));
        let t = self.thickness();
        for (i, g) in self.grips.iter().enumerate() {
            let g = self.to_along(*g);
            if p.x >= g.x - 2 && p.x < g.x + GRIP_W + 2 && p.y >= 2 && p.y < t - 2 {
                return TaskbarHit::Grip(i);
            }
        }
        for (i, r) in self.quick.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Quick(i);
            }
        }
        if self.quick_more.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::QuickMore;
        }
        for (i, r) in self.pager.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Pager(i);
            }
        }
        if self.clock.contains(x, y) {
            return TaskbarHit::Clock;
        }
        if self.show_desktop.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::ShowDesktop;
        }
        if self.chevron.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::Chevron;
        }
        for (i, r) in self.tray.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Tray(i);
            }
        }
        for (i, r) in self.tasks.iter().enumerate() {
            if r.contains(x, y) {
                return TaskbarHit::Task(i);
            }
        }
        if self.more.is_some_and(|r| r.contains(x, y)) {
            return TaskbarHit::More;
        }
        TaskbarHit::Nothing
    }

    /// Disc rows past the bar's screen-side edge (`disc_hang`); None for a disc inside the bar.
    pub fn disc_past_bar(&self) -> Option<Rect> {
        let d = self.start_disc?;
        let y = match self.edge {
            Edge::Bottom => -self.disc_hang,
            Edge::Top => self.height,
            Edge::Left | Edge::Right => return None,
        };
        (self.disc_hang > 0).then(|| Rect::new(d.x, y, d.width, self.disc_hang))
    }

    /// Rect of the orb's own buffer: the rows past the bar and, while its menu is open, the Start rect too.
    pub fn start_orb(&self, held: bool) -> Option<Rect> {
        self.start_above
            .map(|a| if held { a.union(&self.start) } else { a })
    }

    pub fn on_disc(&self, x: i32, y: i32) -> bool {
        self.start_disc.is_some_and(|d| {
            // Doubled, so the half-pixel centres stay whole.
            let (dx, dy) = (
                2 * x + 1 - (2 * d.x + d.width),
                2 * y + 1 - (2 * d.y + d.height),
            );
            dx * dx + dy * dy <= d.width * d.width
        })
    }

    pub fn scaled(&self, n: i32) -> TaskbarLayout {
        let n = n.max(1);
        TaskbarLayout {
            width: self.width * n,
            height: self.height * n,
            edge: self.edge,
            start: self.start.scaled(n),
            grips: self.grips.iter().map(|r| r.scaled(n)).collect(),
            quick: self.quick.iter().map(|r| r.scaled(n)).collect(),
            quick_more: self.quick_more.map(|r| r.scaled(n)),
            pager: self.pager.iter().map(|r| r.scaled(n)).collect(),
            tasks: self.tasks.iter().map(|r| r.scaled(n)).collect(),
            stacks: self.stacks.clone(),
            icons_only: self.icons_only,
            chevron: self.chevron.map(|r| r.scaled(n)),
            tray: self.tray.iter().map(|r| r.scaled(n)).collect(),
            tray_hidden: self.tray_hidden,
            run_width: self.run_width * n,
            tray_arrow: self.tray_arrow,
            clock: self.clock.scaled(n),
            notification: self.notification.scaled(n),
            show_desktop: self.show_desktop.map(|r| r.scaled(n)),
            more: self.more.map(|r| r.scaled(n)),
            resize: self.resize.map(|r| r.scaled(n)),
            clock_lines: self.clock_lines,
            labels: self.labels,
            start_above: self.start_above.map(|r| r.scaled(n)),
            start_disc: self.start_disc.map(|r| r.scaled(n)),
            disc_hang: self.disc_hang * n,
            tasks_x: self.tasks_x * n,
            tasks_end: self.tasks_end * n,
            band_x: self.band_x * n,
            row: (self.row.0 * n, self.row.1 * n),
            button: self.button * n,
            bands: Bands {
                tasks_first: self.bands.tasks_first,
                first_width: self.bands.first_width.map(|w| w * n),
            },
        }
    }
}

pub fn reveal_width(shape: &TaskbarShape, hidden: usize) -> i32 {
    hidden as i32 * (ICON_SIZE + shape.tray_gap)
}

pub fn revealed(shape: &TaskbarShape, hidden: usize, reveal: i32) -> usize {
    (reveal.clamp(0, reveal_width(shape, hidden)) / (ICON_SIZE + shape.tray_gap)) as usize
}

/// The ^ glyph for the tray's chevron cell: a 7x4 triangle times `s`.
pub fn draw_chevron(canvas: &mut Canvas, r: Rect, ink: Color, s: i32) {
    let s = s.max(1);
    let cx = r.x + r.width / 2;
    let top = r.y + (r.height - 4 * s) / 2;
    for i in 0..4 {
        canvas.fill_rect(Rect::new(cx - i * s, top + i * s, (2 * i + 1) * s, s), ink);
    }
}

fn fill_rows(canvas: &mut Canvas, rows: &[&str], x: i32, y: i32, ink: Color, s: i32) {
    for (j, row) in rows.iter().enumerate() {
        for (i, ch) in row.chars().enumerate() {
            if ch == '#' {
                let at = Rect::new(x + i as i32 * s, y + j as i32 * s, s, s);
                canvas.fill_rect(at, ink);
            }
        }
    }
}

/// Two chevrons 8x5 times `s`: `«` while the run is shut, `»` while `open`.
pub fn draw_tray_arrow(canvas: &mut Canvas, r: Rect, ink: Color, s: i32, open: bool) {
    const SHUT: [&str; 5] = ["..##..##", ".##..##.", "##..##..", ".##..##.", "..##..##"];
    const OPEN: [&str; 5] = ["##..##..", ".##..##.", "..##..##", ".##..##.", "##..##.."];
    let s = s.max(1);
    let x = r.x + (r.width / s - 8 + 1) / 2 * s;
    let y = r.y + (r.height / s - 5 + 1) / 2 * s;
    fill_rows(canvas, if open { &OPEN } else { &SHUT }, x, y, ink, s);
}

/// Side bar version: two chevrons stacked (5x8 times `s`), pointing up while shut and down while open.
pub fn draw_tray_arrow_side(canvas: &mut Canvas, r: Rect, ink: Color, s: i32, open: bool) {
    const UP: [&str; 8] = [
        "..#..", ".###.", "##.##", "#...#", "..#..", ".###.", "##.##", "#...#",
    ];
    const DOWN: [&str; 8] = [
        "#...#", "##.##", ".###.", "..#..", "#...#", "##.##", ".###.", "..#..",
    ];
    let s = s.max(1);
    let x = r.x + (r.width / s - 5) / 2 * s;
    let (rows, y) = if open { (&DOWN, 6) } else { (&UP, 2) };
    fill_rows(canvas, rows, x, r.y + y * s, ink, s);
}

/// The » glyph on an overflow cell (the task row's or Quick Launch's): two chevrons 4x7 times `s`.
pub fn draw_more(canvas: &mut Canvas, r: Rect, ink: Color, s: i32) {
    let s = s.max(1);
    let (x0, y0) = (r.x + (r.width - 10 * s) / 2, r.y + (r.height - 7 * s) / 2);
    for dx in [0, 6 * s] {
        for i in 0..4 {
            let col = Rect::new(x0 + dx + i * s, y0 + i * s, s, (7 - 2 * i) * s);
            canvas.fill_rect(col, ink);
        }
    }
}

/// The clock on two lines: time over date, each centred across `clock` on its baseline, counted from the top of a [`TWO_LINE_CLOCK_MIN`] box centred in `clock`.
#[expect(clippy::too_many_arguments)]
pub fn draw_clock_lines(
    canvas: &mut Canvas,
    text: &TextRenderer,
    clock: Rect,
    lines: [&str; 2],
    baselines: [i32; 2],
    ink: Color,
    size: u32,
    s: i32,
) {
    let top = clock.y + (clock.height - TWO_LINE_CLOCK_MIN * s) / 2;
    for (line, baseline) in lines.into_iter().zip(baselines) {
        let pen = Align::Center.pen(clock, text.measure(line, false, size));
        text.draw_run(
            canvas,
            pen,
            top + baseline * s,
            clock,
            line,
            false,
            ink,
            size,
        );
    }
}

/// Gap among `rects` that x falls in: 0 before the first, `rects.len()` after the last.
pub fn slot_at(rects: &[Rect], x: i32) -> usize {
    rects.iter().filter(|r| x >= r.x + r.width / 2).count()
}

/// Icon rect in a Quick Launch cell: 3 px in, centred vertically; the drag image is grabbed from here.
pub fn quick_icon_origin(r: Rect, icon_px: i32, s: i32) -> (i32, i32) {
    (r.x + 3 * s, r.y + (r.height - icon_px) / 2)
}

/// What the taskbar's `access` part says, parallel to the layout's rects.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TaskbarNames {
    pub start_open: bool,
    pub quick: Vec<String>,
    pub desktop: usize,
    pub pager_attention: Vec<bool>,
    pub tasks: Vec<(String, bool)>,
    pub attention: Vec<bool>,
    pub tray: Vec<String>,
    /// The flyout or a tray arrow's run is open: the ^ cell reads expanded (a tray arrow is then named Hide).
    pub hidden_open: bool,
    pub more_open: bool,
    pub quick_more_open: bool,
    pub clock: String,
    pub date: String,
    pub focus: Option<TaskbarHit>,
}

pub const ATTENTION: &str = "Wants attention";
/// Name and tip of the Quick Launch band's » cell.
pub const QUICK_MORE_NAME: &str = "Show more Quick Launch buttons";

/// The taskbar's `access` part, in taskbar coordinates; titles and tips are taken as they are (an `&` is text).
pub fn access(layout: &TaskbarLayout, names: &TaskbarNames, tree: &mut Tree, parent: NodeId) {
    let button = |r: Rect, name: &str| {
        let mut n = access::node(Role::Button, r);
        if !name.is_empty() {
            n.set_label(name);
        }
        access::focusable(&mut n, true);
        n
    };
    let mut focus = None;
    let mut mark = |hit: TaskbarHit, id: NodeId| {
        if names.focus == Some(hit) {
            focus = Some(id);
        }
    };
    let start_rect = layout
        .disc_past_bar()
        .map_or(layout.start, |d| layout.start.union(&d));
    let mut start = button(start_rect, "Start");
    start.set_has_popup(HasPopup::Menu);
    start.set_expanded(names.start_open);
    mark(TaskbarHit::Start, tree.push(parent, "start", start));
    let vertical = layout.edge.is_side();
    let bar = |tree: &mut Tree, key: &str, rects: &[Rect], name: &str| {
        let all = rects.iter().skip(1).fold(rects[0], |u, r| u.union(r));
        let mut n = access::named(Role::Toolbar, all, name);
        if vertical {
            n.set_orientation(Orientation::Vertical);
        }
        tree.push(parent, key, n)
    };
    let wants = |n: &mut crate::access::accesskit::Node, on: Option<&bool>| {
        if on == Some(&true) {
            n.set_description(ATTENTION);
        }
    };
    let quick_cells: Vec<Rect> = layout
        .quick
        .iter()
        .copied()
        .chain(layout.quick_more)
        .collect();
    if !quick_cells.is_empty() {
        let id = bar(tree, "quick", &quick_cells, "Quick Launch");
        for (i, (r, name)) in layout.quick.iter().zip(&names.quick).enumerate() {
            mark(TaskbarHit::Quick(i), tree.push(id, i, button(*r, name)));
        }
        if let Some(r) = layout.quick_more {
            let mut n = button(r, QUICK_MORE_NAME);
            n.set_has_popup(HasPopup::Menu);
            n.set_expanded(names.quick_more_open);
            mark(TaskbarHit::QuickMore, tree.push(id, "more", n));
        }
    }
    if !layout.pager.is_empty() {
        let id = bar(tree, "pager", &layout.pager, "Desktops");
        for (i, r) in layout.pager.iter().enumerate() {
            let mut n = button(*r, &format!("Desktop {}", i + 1));
            n.set_toggled((i == names.desktop).into());
            wants(&mut n, names.pager_attention.get(i));
            mark(TaskbarHit::Pager(i), tree.push(id, i, n));
        }
    }
    if !layout.tasks.is_empty() {
        let id = bar(tree, "tasks", &layout.tasks, "Running Applications");
        for (i, (r, (title, active))) in layout.tasks.iter().zip(&names.tasks).enumerate() {
            let mut n = button(*r, title);
            n.set_toggled((*active).into());
            wants(&mut n, names.attention.get(i));
            mark(TaskbarHit::Task(i), tree.push(id, i, n));
        }
    }
    if let Some(r) = layout.more {
        let mut n = button(r, "Show more windows");
        n.set_has_popup(HasPopup::Menu);
        n.set_expanded(names.more_open);
        mark(TaskbarHit::More, tree.push(parent, "more", n));
    }
    if let Some(r) = layout.chevron {
        // A tray arrow opens the run in place: no popup, and named for
        // what pressing it does.
        let mut n = if !layout.tray_arrow {
            let mut n = button(r, "Show hidden icons");
            n.set_has_popup(HasPopup::Dialog);
            n
        } else if names.hidden_open {
            button(r, "Hide")
        } else {
            button(r, "Show hidden icons")
        };
        n.set_expanded(names.hidden_open);
        mark(TaskbarHit::Chevron, tree.push(parent, "chevron", n));
    }
    if !layout.tray.is_empty() {
        let id = bar(tree, "tray", &layout.tray, "Notification Area");
        for (i, (r, name)) in layout.tray.iter().zip(&names.tray).enumerate() {
            mark(TaskbarHit::Tray(i), tree.push(id, i, button(*r, name)));
        }
    }
    let mut clock = button(layout.clock, &names.clock);
    if !names.date.is_empty() {
        clock.set_description(names.date.as_str());
    }
    mark(TaskbarHit::Clock, tree.push(parent, "clock", clock));
    if let Some(r) = layout.show_desktop {
        let id = tree.push(parent, "show_desktop", button(r, "Show desktop"));
        mark(TaskbarHit::ShowDesktop, id);
    }
    if let Some(id) = focus {
        tree.set_focus(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::access::accesskit::Toggled;

    fn classic_full(edge: Edge) -> TaskbarLayout {
        let c = TaskbarCounts {
            quick: 2,
            pager: 2,
            tasks: 2,
            tray: 2,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            false,
            c,
            30,
        )
        .on_edge(edge)
    }

    #[test]
    fn a_top_bar_mirrors_every_rect_across_its_height() {
        let bottom = classic_full(Edge::Bottom);
        let top = classic_full(Edge::Top);
        let flip = |r: Rect| Rect::new(r.x, 28 - r.y - r.height, r.width, r.height);
        let all = |v: &[Rect]| v.iter().copied().map(flip).collect::<Vec<_>>();
        assert_eq!(top.edge, Edge::Top);
        assert_eq!(
            top.start,
            Rect::new(2, 2, 54, 22),
            "Start 2 px from the top"
        );
        assert_eq!(top.start, flip(bottom.start));
        assert_eq!(top.grips, all(&bottom.grips));
        assert_eq!(top.quick, all(&bottom.quick));
        assert_eq!(top.pager, all(&bottom.pager));
        assert_eq!(top.tasks, all(&bottom.tasks));
        assert_eq!(top.tray, all(&bottom.tray));
        assert_eq!(top.chevron, bottom.chevron.map(flip));
        assert_eq!(top.clock, flip(bottom.clock));
        assert_eq!(top.notification, flip(bottom.notification));
        assert_eq!(top.show_desktop, bottom.show_desktop.map(flip));
        assert_eq!(
            (
                top.width,
                top.height,
                top.tasks_x(),
                top.tasks_limit(),
                top.bands
            ),
            (
                bottom.width,
                bottom.height,
                bottom.tasks_x(),
                bottom.tasks_limit(),
                bottom.bands
            ),
            "along the bar nothing moves"
        );
        assert_eq!(
            top.to_along(top.start),
            bottom.start,
            "to_along maps a Top rect back"
        );
        assert_eq!(
            bottom.to_along(bottom.start),
            bottom.start,
            "a Bottom bar keeps its rects"
        );
        assert_eq!(
            top.on_edge(Edge::Bottom),
            bottom,
            "back on the foot it is the Bottom bar"
        );
    }

    #[test]
    fn a_top_bar_hits_what_the_bottom_bar_hits_on_the_mirrored_row() {
        let bottom = classic_full(Edge::Bottom);
        let top = classic_full(Edge::Top);
        for y in 0..28 {
            for x in 0..800 {
                assert_eq!(top.hit(x, 27 - y), bottom.hit(x, y), "({x}, {y})");
            }
        }
        assert_eq!(top.hit(10, -1), TaskbarHit::Nothing, "above the bar");
        assert_eq!(top.hit(10, 28), TaskbarHit::Nothing, "below the bar");
    }

    #[test]
    fn lay_tasks_on_a_top_bar_keeps_the_row_mirrored() {
        let m = Metrics::default();
        let mut top = classic_row(3).on_edge(Edge::Top);
        top.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 0, 2], TaskRow::Icons);
        assert_eq!(
            top.tasks,
            vec![
                Rect::new(59, 2, 22, 22),
                Rect::new(84, 2, 22, 22),
                Rect::new(109, 2, 22, 22),
            ],
            "22 px squares 2 px from the top"
        );
        assert_eq!(top.stacks, vec![1, 0, 2]);
        top.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1, 1], TaskRow::Shared);
        assert_eq!(
            top.tasks,
            classic_row(3).on_edge(Edge::Top).tasks,
            "a shared row lays out again the same"
        );
        assert!(top.tasks.iter().all(|r| r.y == 2));
    }

    #[test]
    fn a_scaled_top_bar_stays_on_the_top() {
        let top = classic_row(2).on_edge(Edge::Top);
        let s = top.scaled(2);
        assert_eq!(s.edge, Edge::Top);
        assert_eq!(
            s.start,
            Rect::new(4, 4, 108, 44),
            "Start 4 device px from the top at scale 2"
        );
        assert_eq!(s.tasks[1], top.tasks[1].scaled(2));
        assert_eq!(s.to_along(s.start), classic_row(2).scaled(2).start);
    }

    fn counts(quick: usize, tasks: usize, tray: usize) -> TaskbarCounts {
        TaskbarCounts {
            quick,
            pager: 0,
            tasks,
            tray,
            show_desktop: false,
            chevron: false,
            ..TaskbarCounts::default()
        }
    }

    #[test]
    fn classic_shape_reproduces_the_default_layout() {
        let m = Metrics::default();
        let a = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 1), 40);
        assert_eq!(a.start, Rect::new(2, 4, 54, 22));
        assert_eq!(a.tasks[0].y, 4);
        assert_eq!(a.tasks[0].height, 22);
    }

    #[test]
    fn a_full_height_shape_lays_start_and_the_well_edge_to_edge() {
        let m = Metrics {
            taskbar_height: 30,
            start_button_width: 97,
            ..Metrics::default()
        };
        let shape = TaskbarShape {
            start: Rect::new(0, 0, 0, 30),
            button_top: 3,
            button_bottom: 2,
            start_gap: 7,
            well_full_height: true,
            well_lead: 4,
            tray_gap: 4,
            tray_low: false,
            side_tray_inset: 0,
            tray_arrow: None,
            clock: ClockBox::CLASSIC,
            start_icon: 18,
            task_icon: ICON_SIZE,
            icon_button: None,
            show_desktop_width: None,
            orb: false,
            orb_rise: None,
            two_line_clock: None,
            start_menu: StartMenuDock::Into(4),
            labelled: LabelledButton::CLASSIC,
        };
        let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 1, 2), 40);
        assert_eq!(l.start, Rect::new(0, 0, 97, 30));
        assert_eq!(
            (l.tasks[0].x, l.tasks[0].y, l.tasks[0].height),
            (104, 3, 25)
        );
        assert_eq!(l.notification.y, 0);
        assert_eq!(l.notification.height, 30);
        assert_eq!(l.notification.right(), 1920);
    }

    #[test]
    fn access_names_the_buttons_and_presses_the_active_task() {
        let l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(1, 2, 1),
            40,
        );
        let names = TaskbarNames {
            start_open: true,
            quick: vec!["Show Desktop".into()],
            tasks: vec![("One".into(), true), ("A & B".into(), false)],
            tray: vec!["Volume".into()],
            clock: "12:00".into(),
            date: "Friday".into(),
            ..TaskbarNames::default()
        };
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        let start = t.find(Role::Button, "Start").unwrap();
        assert_eq!(t.get(start).unwrap().is_expanded(), Some(true));
        let one = t.find(Role::Button, "One").unwrap();
        assert_eq!(
            t.get(one).unwrap().toggled(),
            Some(Toggled::True),
            "the active window's button is pressed"
        );
        assert!(
            t.find(Role::Button, "A & B").is_some(),
            "an & in a title stays"
        );
        assert!(t.find(Role::Button, "Show Desktop").is_some());
        assert!(t.find(Role::Button, "Volume").is_some());
        let clock = t.find(Role::Button, "12:00").unwrap();
        assert_eq!(t.get(clock).unwrap().description(), Some("Friday"));
        assert_eq!(t.focus(), access::ROOT, "the bar has no keyboard");
    }

    #[test]
    fn access_focuses_the_keyboards_item_and_says_who_wants_attention() {
        let l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 2, 1),
            40,
        );
        let names = TaskbarNames {
            tasks: vec![("One".into(), true), ("Two".into(), false)],
            attention: vec![false, true],
            tray: vec!["Volume".into()],
            focus: Some(TaskbarHit::Task(1)),
            ..TaskbarNames::default()
        };
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        let two = t.find(Role::Button, "Two").unwrap();
        assert_eq!(t.focus(), two, "the keyboard is on the second task");
        assert_eq!(t.get(two).unwrap().description(), Some(ATTENTION));
        let one = t.find(Role::Button, "One").unwrap();
        assert_eq!(t.get(one).unwrap().description(), None);
        assert!(
            t.get(two)
                .unwrap()
                .supports_action(crate::access::accesskit::Action::Click),
            "a reader can click it"
        );
        let names = TaskbarNames {
            tray: vec!["Volume".into()],
            focus: Some(TaskbarHit::Tray(0)),
            ..names
        };
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        assert_eq!(Some(t.focus()), t.find(Role::Button, "Volume"));
    }

    #[test]
    fn layout_with_default_metrics() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
        assert_eq!(l.height, 28);
        assert_eq!(l.start, Rect::new(2, 4, 54, 22));
        assert_eq!(l.clock, Rect::new(800 - 2 - 46, 4, 46, 22));
        assert_eq!(l.tasks.len(), 2);
        assert_eq!(l.tasks[0], Rect::new(59, 4, 160, 22));
        assert_eq!(l.tasks[1], Rect::new(222, 4, 160, 22));
        assert!(l.grips.is_empty());
    }

    #[test]
    fn tasks_shrink_to_fit() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 5, 0), 30);
        let avail = l.clock.x - 6 - l.tasks_x();
        let w = (avail - 2 * 4) / 5;
        assert_eq!(l.tasks[0].width, w);
        assert!(l.tasks[4].right() <= l.clock.x - 6);
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 0, 0), 30);
        assert!(l.tasks.is_empty());
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 40, 0), 30);
        assert!(l.tasks[0].width >= 1);
    }

    #[test]
    fn tasks_never_overrun_clock() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 200, true, counts(0, 50, 0), 30);
        for r in &l.tasks {
            assert!(r.right() <= l.clock.x - 6);
        }
        assert!(l.tasks.len() < 50);
    }

    #[test]
    fn narrow_width_clamps_clock_and_drops_tasks() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 10, true, counts(0, 3, 0), 30);
        assert!(l.clock.x >= l.tasks_x());
        assert!(l.tasks.is_empty());
    }

    #[test]
    fn hit_testing() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
        assert_eq!(l.hit(10, 10), TaskbarHit::Start);
        assert_eq!(l.hit(70, 10), TaskbarHit::Task(0));
        assert_eq!(l.hit(230, 10), TaskbarHit::Task(1));
        assert_eq!(l.hit(780, 10), TaskbarHit::Clock);
        assert_eq!(l.hit(500, 10), TaskbarHit::Nothing);
        assert_eq!(l.hit(10, 40), TaskbarHit::Nothing);
    }

    fn every_cell(l: &TaskbarLayout) -> Vec<(TaskbarHit, Rect)> {
        let mut cells = vec![(TaskbarHit::Start, l.start), (TaskbarHit::Clock, l.clock)];
        let each = |hit: fn(usize) -> TaskbarHit, v: &[Rect]| {
            v.iter()
                .enumerate()
                .map(move |(i, &r)| (hit(i), r))
                .collect::<Vec<_>>()
        };
        cells.extend(each(TaskbarHit::Grip, &l.grips));
        cells.extend(each(TaskbarHit::Quick, &l.quick));
        cells.extend(each(TaskbarHit::Pager, &l.pager));
        cells.extend(each(TaskbarHit::Task, &l.tasks));
        cells.extend(each(TaskbarHit::Tray, &l.tray));
        cells.extend(l.quick_more.map(|r| (TaskbarHit::QuickMore, r)));
        cells.extend(l.chevron.map(|r| (TaskbarHit::Chevron, r)));
        cells.extend(l.show_desktop.map(|r| (TaskbarHit::ShowDesktop, r)));
        cells.extend(l.more.map(|r| (TaskbarHit::More, r)));
        cells
    }

    /// The screen-edge pixel across from the middle of `r`, and the strip between it and `r`.
    fn edge_pixel(l: &TaskbarLayout, r: Rect) -> ((i32, i32), Rect) {
        let (cx, cy) = (r.x + r.width / 2, r.y + r.height / 2);
        match l.edge {
            Edge::Bottom => (
                (cx, l.height - 1),
                Rect::new(cx, r.bottom(), 1, l.height - r.bottom()),
            ),
            Edge::Top => ((cx, 0), Rect::new(cx, 0, 1, r.y)),
            Edge::Left => ((0, cy), Rect::new(0, cy, r.x, 1)),
            Edge::Right => (
                (l.width - 1, cy),
                Rect::new(r.right(), cy, l.width - r.right(), 1),
            ),
        }
    }

    #[test]
    fn every_cell_takes_the_screen_edge_pixel_across_from_it() {
        use crate::interface::Interface;
        let counts = [
            TaskbarCounts {
                quick: 6,
                pager: 2,
                tasks: 3,
                tray: 3,
                show_desktop: true,
                chevron: true,
                ..TaskbarCounts::default()
            },
            TaskbarCounts {
                quick: 2,
                tasks: 60,
                tray: 1,
                ..TaskbarCounts::default()
            },
        ];
        let bands = Bands {
            tasks_first: false,
            first_width: Some(60),
        };
        let mut seen = std::collections::HashSet::new();
        for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let theme = crate::interfaces::theme_for(look);
            let shape = theme.taskbar_shape();
            for edge in Edge::ALL {
                let thickness = if edge.is_side() {
                    100
                } else {
                    theme.metrics.taskbar_height
                };
                let span = BarSpan {
                    edge,
                    length: if edge.is_side() { 700 } else { 1000 },
                    thickness,
                };
                for (c, locked) in counts.iter().zip([false, true]) {
                    let l = TaskbarLayout::compute_on(
                        &theme.metrics,
                        &shape,
                        span,
                        locked,
                        *c,
                        ClockText { time: 30, date: 30 },
                        bands,
                    );
                    let cells = every_cell(&l);
                    for &(hit, r) in &cells {
                        let ((x, y), between) = edge_pixel(&l, r);
                        let shielded = cells
                            .iter()
                            .any(|&(h, o)| h != hit && !o.intersect(&between).is_empty());
                        if shielded {
                            continue;
                        }
                        assert_eq!(
                            l.hit(x, y),
                            hit,
                            "{look:?} {edge:?}: {hit:?} at {r:?} takes ({x}, {y})"
                        );
                        seen.insert(std::mem::discriminant(&hit));
                    }
                }
            }
        }
        assert_eq!(seen.len(), 11, "every kind of cell checked");
    }

    #[test]
    fn scaled_layout_doubles_every_rect() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
        let s = l.scaled(2);
        assert_eq!(s.width, 1600);
        assert_eq!(s.height, 56);
        assert_eq!(s.start, l.start.scaled(2));
        assert_eq!(s.clock, l.clock.scaled(2));
        assert_eq!(s.tasks, vec![l.tasks[0].scaled(2), l.tasks[1].scaled(2)]);
        // hit-testing stays on the logical layout; the scaled one is for drawing only
        assert_eq!(l.hit(10, 10), TaskbarHit::Start);
    }

    #[test]
    fn tray_sits_left_of_clock_and_narrows_tasks() {
        let m = Metrics::default();
        let none =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
        let two =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
        assert_eq!(two.tray.len(), 2);
        assert_eq!(two.tray[1].right(), two.clock.x - 6);
        assert_eq!(two.tray[1].x - two.tray[0].right(), 4);
        assert_eq!(two.tray[0].width, crate::icon::ICON_SIZE);
        assert_eq!(two.tray[0].height, crate::icon::ICON_SIZE);
        assert_eq!(
            two.tray[0].y,
            4 + (m.taskbar_height - 6 - crate::icon::ICON_SIZE) / 2
        );
        // A single task button is capped at task_button_max_width either way,
        // so compare the available span instead: two icons cost 40 px.
        let avail = |l: &TaskbarLayout| l.tasks_limit() - l.tasks_x();
        assert_eq!(avail(&none) - avail(&two), 40);
        assert_eq!(two.hit(two.tray[1].x + 3, 15), TaskbarHit::Tray(1));
        assert_eq!(two.hit(two.tray[0].x + 3, 15), TaskbarHit::Tray(0));
        assert_eq!(none.tasks_limit(), none.clock.x - 6);
    }

    #[test]
    fn the_chevron_leads_the_tray_one_cell_left_of_its_icons() {
        let m = Metrics::default();
        let shape = TaskbarShape {
            tray_arrow: None,
            ..TaskbarShape::CLASSIC
        };
        let bare = TaskbarLayout::compute(&m, &shape, 800, true, counts(0, 1, 2), 30);
        let with = TaskbarLayout::compute(
            &m,
            &shape,
            800,
            true,
            TaskbarCounts {
                chevron: true,
                ..counts(0, 1, 2)
            },
            30,
        );
        assert_eq!(bare.chevron, None);
        assert_eq!(with.tray, bare.tray);
        let c = with.chevron.unwrap();
        assert_eq!(c, Rect::new(bare.tray[0].x - 20, bare.tray[0].y, 16, 16));
        assert_eq!(with.notification.x, c.x - 4);
        assert_eq!(bare.tasks_limit() - with.tasks_limit(), 20);
        assert_eq!(with.hit(c.x + 8, c.y + 8), TaskbarHit::Chevron);
        assert_eq!(with.hit(with.tray[0].x + 3, 15), TaskbarHit::Tray(0));
        assert_eq!(with.scaled(2).chevron, Some(c.scaled(2)));
    }

    #[test]
    fn with_every_item_hidden_the_chevron_sits_before_the_clock() {
        let m = Metrics::default();
        let counts = TaskbarCounts {
            chevron: true,
            ..counts(0, 1, 0)
        };
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts, 30);
        let c = l.chevron.unwrap();
        assert!(l.tray.is_empty());
        assert_eq!(c.right(), l.clock.x - 6);
        assert_eq!(l.tasks_limit(), c.x - 4);
    }

    #[test]
    fn the_chevron_is_a_centred_up_triangle() {
        let mut c = Canvas::new(16, 16);
        let ink = Color::rgb(255, 255, 255);
        draw_chevron(&mut c, Rect::new(0, 0, 16, 16), ink, 1);
        let row = |y: i32| {
            (0..16)
                .filter(|&x| c.get(x, y) == ink.argb())
                .collect::<Vec<_>>()
        };
        assert!(row(5).is_empty());
        assert_eq!(row(6), vec![8]);
        assert_eq!(row(7), vec![7, 8, 9]);
        assert_eq!(row(8), (6..=10).collect::<Vec<_>>());
        assert_eq!(row(9), (5..=11).collect::<Vec<_>>());
        assert!(row(10).is_empty());
    }

    #[test]
    fn the_chevron_is_a_button_only_when_laid_out() {
        let m = Metrics::default();
        let names = TaskbarNames {
            hidden_open: true,
            ..TaskbarNames::default()
        };
        let tree = |chevron| {
            let counts = TaskbarCounts {
                chevron,
                ..counts(0, 0, 0)
            };
            let shape = TaskbarShape {
                tray_arrow: None,
                ..TaskbarShape::CLASSIC
            };
            let l = TaskbarLayout::compute(&m, &shape, 800, true, counts, 30);
            let mut t = Tree::window("Taskbar", (l.width, l.height));
            access(&l, &names, &mut t, access::ROOT);
            t
        };
        let t = tree(true);
        let n = t.find(Role::Button, "Show hidden icons").unwrap();
        assert_eq!(t.get(n).unwrap().is_expanded(), Some(true));
        assert_eq!(t.get(n).unwrap().has_popup(), Some(HasPopup::Dialog));
        assert!(
            tree(false)
                .find(Role::Button, "Show hidden icons")
                .is_none()
        );
    }

    fn arrow_counts(tray: usize, hidden: usize, reveal: i32) -> TaskbarCounts {
        TaskbarCounts {
            tray,
            hidden,
            reveal,
            chevron: true,
            ..TaskbarCounts::default()
        }
    }

    #[test]
    fn an_open_run_lays_the_hidden_cells_between_the_arrow_and_the_icons() {
        let m = Metrics::default();
        let lay = |reveal| {
            TaskbarLayout::compute(
                &m,
                &TaskbarShape::CLASSIC,
                800,
                true,
                arrow_counts(2, 3, reveal),
                30,
            )
        };
        let shut = lay(0);
        let arrow = shut.chevron.unwrap();
        assert_eq!((shut.tray.len(), shut.tray_hidden), (2, 0));
        assert_eq!(reveal_width(&TaskbarShape::CLASSIC, 3), 60);
        let open = lay(60);
        assert_eq!(
            open.chevron.unwrap().x,
            arrow.x - 60,
            "the arrow leads the run"
        );
        assert_eq!(open.tray_hidden, 3);
        let first = arrow.right() + 1;
        assert_eq!(first, shut.tray[0].x);
        let xs: Vec<i32> = open.tray.iter().map(|r| r.x).collect();
        assert_eq!(
            xs,
            vec![first - 60, first - 40, first - 20, first, first + 20]
        );
        assert_eq!(
            open.notification.x,
            shut.notification.x - 60,
            "the well widens"
        );
        assert_eq!(
            open.tasks_end,
            shut.tasks_end - 60,
            "the task row gives way"
        );
    }

    #[test]
    fn a_part_open_run_shows_only_whole_cells_and_moves_the_arrow_by_the_pixel() {
        let m = Metrics::default();
        let shut = TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            arrow_counts(2, 3, 0),
            30,
        );
        let half = TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            arrow_counts(2, 3, 30),
            30,
        );
        let arrow = shut.chevron.unwrap().x;
        assert_eq!(half.chevron.unwrap().x, arrow - 30);
        assert_eq!(half.tray_hidden, 1, "one whole cell in 30 px");
        assert_eq!(
            half.tray[0].x,
            shut.tray[0].x - 20,
            "the hidden cell nearest the icons comes out first"
        );
    }

    #[test]
    fn an_open_run_never_reaches_the_clock_on_a_narrow_bar() {
        let m = Metrics::default();
        let lay = |reveal| {
            TaskbarLayout::compute(
                &m,
                &TaskbarShape::CLASSIC,
                240,
                true,
                arrow_counts(6, 8, reveal),
                30,
            )
        };
        let l = lay(160);
        assert!(l.chevron.unwrap().x >= l.band_x);
        for r in &l.tray {
            assert!(
                r.right() <= l.clock.x - 6,
                "cell at {} ends 6 px before the clock",
                r.x
            );
        }
        // Six shown cells already fill the row: the run of eight opens
        // none, and the row stays as it is shut.
        assert_eq!(l.tray_hidden, 0, "no room for a hidden cell");
        let shut = lay(0);
        assert_eq!((l.chevron, &l.tray), (shut.chevron, &shut.tray));
    }

    #[test]
    fn only_classic_and_blue_have_a_tray_arrow() {
        use crate::interface::Interface;
        use crate::interfaces::theme_for;
        assert!(TaskbarShape::CLASSIC.tray_arrow.is_some());
        assert!(
            theme_for(Interface::Blue)
                .taskbar_shape()
                .tray_arrow
                .is_some()
        );
        assert!(
            theme_for(Interface::Glass)
                .taskbar_shape()
                .tray_arrow
                .is_none()
        );
    }

    #[test]
    fn a_shape_without_an_arrow_ignores_the_reveal() {
        let m = Metrics::default();
        let mut shape = TaskbarShape::CLASSIC;
        shape.tray_arrow = None;
        let a = TaskbarLayout::compute(&m, &shape, 800, true, arrow_counts(2, 3, 0), 30);
        let b = TaskbarLayout::compute(&m, &shape, 800, true, arrow_counts(2, 3, 60), 30);
        assert_eq!(
            (a.chevron, a.tray.clone(), b.tray_hidden),
            (b.chevron, b.tray.clone(), 0)
        );
    }

    #[test]
    fn a_classic_tray_arrow_is_a_cell_flush_in_the_well_1_px_before_the_first_icon() {
        let m = Metrics::default();
        let lay = |reveal| {
            TaskbarLayout::compute(
                &m,
                &TaskbarShape::CLASSIC,
                800,
                true,
                arrow_counts(2, 3, reveal),
                30,
            )
        };
        let (shut, open) = (lay(0), lay(60));
        let icon = shut.tray[0];
        let a = shut.chevron.unwrap();
        assert_eq!(a, Rect::new(icon.x - 17, icon.y - 1, 16, 18));
        assert_eq!(
            (shut.notification.x, shut.notification.y),
            (a.x - 1, a.y - 2),
            "the cell inside the well's frame: flush left, 1 px clear above"
        );
        assert_eq!(shut.notification.bottom(), a.bottom() + 2, "and below");
        let b = open.chevron.unwrap();
        assert_eq!(b, Rect::new(a.x - 60, a.y, 16, 18), "out by the whole run");
        assert_eq!(
            b.right() + 1,
            open.tray[0].x,
            "the first hidden icon 1 px after it"
        );
        assert_eq!(open.tray[3..], shut.tray[..], "the shown icons stay");
        assert_eq!(open.notification.x, b.x - 1);
    }

    #[test]
    fn a_blue_tray_arrow_is_a_19_px_button_across_the_wells_left_edge() {
        use crate::interface::Interface;
        let blue = crate::interfaces::theme_for(Interface::Blue);
        let shape = blue.taskbar_shape();
        assert_eq!(reveal_width(&shape, 3), 54, "18 px a cell");
        let lay = |reveal| {
            TaskbarLayout::compute(
                &blue.metrics,
                &shape,
                800,
                true,
                arrow_counts(2, 3, reveal),
                30,
            )
        };
        let (shut, open) = (lay(0), lay(54));
        let icon = shut.tray[0];
        let a = shut.chevron.unwrap();
        assert_eq!(a, Rect::new(icon.x - 20, icon.y - 2, 19, 19));
        assert_eq!(
            shut.notification.x,
            a.x + 9,
            "the well's left edge on the button's middle column"
        );
        assert_eq!(shut.notification.x, icon.x - 11, "11 px before the icon");
        let b = open.chevron.unwrap();
        assert_eq!(b, Rect::new(a.x - 54, a.y, 19, 19));
        assert_eq!(b.right() + 1, open.tray[0].x);
        assert_eq!(open.tray[3..], shut.tray[..], "the shown icons stay");
        assert_eq!(open.notification.x, b.x + 9);
    }

    #[test]
    fn a_classic_side_tray_arrow_spans_the_wells_top() {
        let shut = classic_column(Edge::Left, 60, arrow_counts(2, 3, 0), true);
        let open = classic_column(Edge::Left, 60, arrow_counts(2, 3, 1), true);
        for l in [&shut, &open] {
            let (a, well) = (l.chevron.unwrap(), l.notification);
            assert_eq!(
                a,
                Rect::new(well.x + 2, well.y + 1, well.width - 4, 16),
                "16 tall, flush under the well's frame, 1 px clear of its sides"
            );
            assert_eq!(l.tray[0].y, a.y + 17, "the first icon row 1 px under it");
        }
        assert_eq!(open.tray_hidden, 3);
        assert_eq!(
            open.tray[0].x, shut.tray[0].x,
            "the hidden cells lead the grid"
        );
    }

    #[test]
    fn a_blue_side_tray_arrow_is_centred_on_the_wells_top_edge() {
        use crate::interface::Interface;
        let blue = crate::interfaces::theme_for(Interface::Blue);
        let lay = |edge, thickness| {
            let span = BarSpan {
                edge,
                length: 700,
                thickness,
            };
            TaskbarLayout::compute_on(
                &blue.metrics,
                &blue.taskbar_shape(),
                span,
                true,
                arrow_counts(2, 3, 0),
                ClockText::default(),
                Bands::default(),
            )
        };
        for (edge, t, x) in [
            (Edge::Left, 104, 41),
            (Edge::Left, 80, 29),
            (Edge::Right, 104, 44),
        ] {
            let l = lay(edge, t);
            let a = l.chevron.unwrap();
            assert_eq!(
                (a.x, a.width, a.height),
                (x, 19, 19),
                "{edge:?} at {t} px: centred across"
            );
            assert_eq!(
                l.notification.y,
                a.y + 8,
                "its middle row 1 px under the well's top edge"
            );
            assert_eq!(l.tray[0].y, a.y + 21, "the first icon row 2 px under it");
        }
    }

    #[test]
    fn blue_row_tray_icons_sit_8_px_down_a_30_px_bar_and_the_arrow_6() {
        use crate::interface::Interface;
        let blue = crate::interfaces::theme_for(Interface::Blue);
        assert_eq!(blue.metrics.taskbar_height, 30);
        let l = TaskbarLayout::compute(
            &blue.metrics,
            &blue.taskbar_shape(),
            800,
            true,
            arrow_counts(2, 3, 0),
            30,
        );
        assert!(l.tray.iter().all(|r| r.y == 8), "{:?}", l.tray);
        assert_eq!(l.chevron.unwrap().y, 6);
    }

    #[test]
    fn classic_side_tray_icons_start_4_px_inside_the_wells_frame() {
        let c = |hidden, reveal| TaskbarCounts {
            tray: 6,
            ..arrow_counts(6, hidden, reveal)
        };
        for (edge, frame) in [(Edge::Left, 2), (Edge::Right, 4)] {
            for counts in [c(0, 0), c(3, 1)] {
                let l = classic_column(edge, 104, counts, true);
                let well = l.notification;
                assert_eq!(well.x, frame, "{edge:?}: the well's frame column");
                assert_eq!(l.tray[0].x, frame + 5, "{edge:?}: 4 px inside the frame");
                let xs: Vec<i32> = l.tray.iter().map(|r| r.x).collect();
                assert_eq!(xs[..4], [0, 20, 40, 60].map(|d| frame + 5 + d), "{edge:?}");
                assert_eq!(xs[4], frame + 5, "{edge:?}: the fifth wraps");
                assert!(l.tray.iter().all(|r| r.right() <= well.right() - 5));
            }
        }
    }

    #[test]
    fn with_no_shown_icon_an_open_run_ends_where_a_shown_icon_would() {
        use crate::interface::Interface;
        let blue = crate::interfaces::theme_for(Interface::Blue);
        let looks = [
            (Metrics::default(), TaskbarShape::CLASSIC),
            (blue.metrics.clone(), blue.taskbar_shape()),
        ];
        for (m, shape) in looks {
            let lay = |tray, hidden, reveal| {
                TaskbarLayout::compute(
                    &m,
                    &shape,
                    800,
                    true,
                    arrow_counts(tray, hidden, reveal),
                    30,
                )
            };
            let full = reveal_width(&shape, 2);
            let (shut, open) = (lay(0, 2, 0), lay(0, 2, full));
            let one = lay(1, 0, 0);
            assert_eq!(
                shut.chevron.unwrap().right(),
                shut.clock.x - 6,
                "shut, the arrow ends 6 px before the clock"
            );
            assert_eq!(
                open.tray.last(),
                one.tray.first(),
                "the nearest hidden cell where a lone shown icon sits"
            );
            let (a, w) = (open.chevron.unwrap(), shape.tray_arrow.unwrap());
            assert_eq!(a.right() + w.gap, open.tray[0].x, "the arrow leads the run");
        }
    }

    fn ink_rows(c: &Canvas, r: Rect, ink: Color) -> Vec<String> {
        (r.y..r.bottom())
            .map(|y| {
                (r.x..r.right())
                    .map(|x| if c.get(x, y) == ink.argb() { '#' } else { '.' })
                    .collect()
            })
            .collect()
    }

    fn glyph_at(rows: &[&str], x: usize, y: usize, w: usize, h: usize) -> Vec<String> {
        (0..h)
            .map(|j| {
                let mut line = vec!['.'; w];
                if let Some(row) = j.checked_sub(y).and_then(|k| rows.get(k)) {
                    for (i, ch) in row.chars().enumerate() {
                        line[x + i] = ch;
                    }
                }
                line.into_iter().collect()
            })
            .collect()
    }

    #[test]
    fn the_tray_arrow_points_back_while_shut_and_on_while_open() {
        let ink = Color::rgb(0, 0, 0);
        let r = Rect::new(0, 0, 16, 18);
        for (open, rows) in [
            (
                false,
                ["..##..##", ".##..##.", "##..##..", ".##..##.", "..##..##"],
            ),
            (
                true,
                ["##..##..", ".##..##.", "..##..##", ".##..##.", "##..##.."],
            ),
        ] {
            let mut c = Canvas::new(16, 18);
            draw_tray_arrow(&mut c, r, ink, 1, open);
            assert_eq!(
                ink_rows(&c, r, ink),
                glyph_at(&rows, 4, 7, 16, 18),
                "open {open}: 8x5 at (4, 7)"
            );
        }
    }

    #[test]
    fn the_side_tray_arrow_points_up_while_shut_and_down_while_open() {
        let ink = Color::rgb(0, 0, 0);
        let r = Rect::new(0, 0, 95, 16);
        let up = ["..#..", ".###.", "##.##", "#...#"];
        let down = ["#...#", "##.##", ".###.", "..#.."];
        for (open, rows, y) in [(false, up, 2), (true, down, 6)] {
            let mut c = Canvas::new(95, 16);
            draw_tray_arrow_side(&mut c, r, ink, 1, open);
            let twice: Vec<&str> = rows.iter().chain(&rows).copied().collect();
            assert_eq!(
                ink_rows(&c, r, ink),
                glyph_at(&twice, 45, y, 95, 16),
                "open {open}: 5x8 centred across, {y} px down"
            );
        }
    }

    #[test]
    fn the_tray_arrows_double_at_scale_two() {
        let ink = Color::rgb(0, 0, 0);
        type Draw = fn(&mut Canvas, Rect, Color, i32, bool);
        let glyphs: [(Draw, i32, i32); 2] =
            [(draw_tray_arrow, 16, 18), (draw_tray_arrow_side, 95, 16)];
        for (draw, w, h) in glyphs {
            for open in [false, true] {
                let mut one = Canvas::new(w as u32, h as u32);
                draw(&mut one, Rect::new(0, 0, w, h), ink, 1, open);
                let mut two = Canvas::new(2 * w as u32, 2 * h as u32);
                draw(&mut two, Rect::new(0, 0, 2 * w, 2 * h), ink, 2, open);
                for y in 0..2 * h {
                    for x in 0..2 * w {
                        assert_eq!(two.get(x, y), one.get(x / 2, y / 2), "({x}, {y})");
                    }
                }
            }
        }
    }

    #[test]
    fn the_tray_arrow_is_a_button_reading_expanded_while_open() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            arrow_counts(1, 2, 0),
            30,
        );
        for (open, name) in [(false, "Show hidden icons"), (true, "Hide")] {
            let names = TaskbarNames {
                hidden_open: open,
                ..TaskbarNames::default()
            };
            let mut t = Tree::window("Taskbar", (l.width, l.height));
            access(&l, &names, &mut t, access::ROOT);
            let n = t.find(Role::Button, name).expect(name);
            let node = t.get(n).unwrap();
            assert_eq!(node.is_expanded(), Some(open), "{name}");
            assert_eq!(node.has_popup(), None, "{name}: it opens no popup");
        }
    }

    #[test]
    fn tray_cells_that_reach_the_clock_are_dropped() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 400, true, counts(0, 1, 16), 30);
        assert!(!l.tray.is_empty() && l.tray.len() < 16);
        assert_eq!(l.tray[0].x, l.band_x);
        for r in &l.tray {
            assert!(
                r.right() <= l.clock.x - 6,
                "cell at {} ends 6 px before the clock",
                r.x
            );
        }
    }

    #[test]
    fn notification_well_wraps_the_tray_and_clock() {
        let m = Metrics::default();
        let bare =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
        assert_eq!(bare.notification, bare.clock);
        let two =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
        assert_eq!(two.notification.x, two.tray[0].x - 4);
        assert_eq!(two.notification.right(), two.clock.right());
        assert!(two.notification.x < two.tray[0].x);
        assert_eq!(two.notification.y, two.clock.y);
        assert_eq!(two.notification.height, two.clock.height);
    }

    #[test]
    fn quick_launch_sits_between_start_and_the_tasks() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(2, 1, 0), 30);
        assert_eq!(
            l.quick,
            vec![Rect::new(59, 4, 23, 22), Rect::new(82, 4, 23, 22)]
        );
        assert_eq!(l.tasks_x(), 109);
        assert_eq!(l.tasks[0].x, 109);
        assert_eq!(l.hit(70, 10), TaskbarHit::Quick(0));
        assert_eq!(l.hit(100, 10), TaskbarHit::Quick(1));
        assert_eq!(l.hit(120, 10), TaskbarHit::Task(0));
        let none =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 0), 30);
        assert!(none.quick.is_empty());
        assert_eq!(none.tasks_x(), 59);
        assert_eq!(l.scaled(2).quick[1], l.quick[1].scaled(2));
        assert_eq!(l.scaled(2).tasks_x(), 218);
    }

    #[test]
    fn pager_leads_quick_launch_in_its_band() {
        let m = Metrics::default();
        let c = |quick, pager| TaskbarCounts {
            quick,
            pager,
            tasks: 1,
            tray: 0,
            show_desktop: false,
            chevron: false,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, c(2, 4), 30);
        assert_eq!(l.pager[0], Rect::new(59, 4, 23, 22));
        assert_eq!(l.pager[3], Rect::new(128, 4, 23, 22));
        assert_eq!(l.quick[0].x, 155);
        assert_eq!(l.quick[1].x, 178);
        assert_eq!(l.tasks_x(), 205);
        assert_eq!(l.hit(60, 10), TaskbarHit::Pager(0));
        assert_eq!(l.hit(150, 10), TaskbarHit::Pager(3));
        assert_eq!(l.hit(152, 10), TaskbarHit::Nothing);
        assert!(l.on_quick_band(152, 10) && !l.on_quick_band(150, 10));
        let alone = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, c(0, 2), 30);
        assert_eq!(alone.pager[0], Rect::new(59, 4, 23, 22));
        assert_eq!(alone.tasks_x(), 109);
        assert!(alone.on_quick_band(106, 10) && !alone.on_quick_band(104, 10));
        assert!(!alone.on_quick_band(57, 10) && !alone.on_quick_band(109, 10));
        let un = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, c(2, 4), 30);
        assert_eq!(un.pager[0].x, 68);
        assert_eq!(un.quick[0].x, 164);
        assert_eq!(un.bands.first_width, Some(142));
        assert_eq!(un.grips[1].x, 216);
        assert_eq!(l.scaled(2).pager[1], l.pager[1].scaled(2));
    }

    #[test]
    fn unlocked_taskbar_puts_a_grip_before_each_band() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 1, 0), 30);
        assert_eq!(
            l.grips,
            vec![Rect::new(61, 6, 3, 18), Rect::new(120, 6, 3, 18)]
        );
        assert_eq!(l.quick[0], Rect::new(68, 4, 23, 22));
        assert_eq!(l.tasks[0].x, 127);
        assert_eq!(l.hit(62, 10), TaskbarHit::Grip(0));
        assert_eq!(l.hit(119, 10), TaskbarHit::Grip(1));
        assert_eq!(l.hit(126, 10), TaskbarHit::Nothing);
        assert_eq!(
            l.bands,
            Bands {
                tasks_first: false,
                first_width: Some(46)
            }
        );
        assert!(l.on_quick_band(66, 10) && l.on_quick_band(118, 10));
        assert!(!l.on_quick_band(127, 10));
        let idle =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 0, 0), 30);
        assert_eq!(idle.grips, vec![Rect::new(61, 6, 3, 18)]);
        assert_eq!(idle.tasks_x(), 127);
        let bare =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(0, 1, 0), 30);
        assert_eq!(bare.grips, vec![Rect::new(61, 6, 3, 18)]);
        assert_eq!(bare.tasks[0].x, 68);
        assert_eq!(l.scaled(2).grips[0], l.grips[0].scaled(2));
    }

    #[test]
    fn first_band_width_moves_the_second_band() {
        let m = Metrics::default();
        let wide = |w| Bands {
            tasks_first: false,
            first_width: Some(w),
        };
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            wide(247),
        );
        assert_eq!(l.grips[1], Rect::new(321, 6, 3, 18));
        assert_eq!(l.tasks[0].x, 328);
        assert_eq!(l.quick[1].x, 91);
        assert!(l.on_quick_band(300, 10) && !l.on_quick_band(328, 10));
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            wide(10),
        );
        assert_eq!(
            l.grips[1].x,
            68 + 14 + 6,
            "the Quick Launch band shrinks to its »"
        );
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(2, 1, 0),
            30,
            wide(247),
        );
        assert!(l.grips.is_empty());
        assert_eq!(l.tasks_x(), 59 + 247 + 4);
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            400,
            false,
            counts(2, 1, 2),
            30,
            wide(1000),
        );
        assert_eq!(l.tasks_x(), l.tasks_limit());
    }

    #[test]
    fn tasks_first_puts_quick_launch_after_the_task_band() {
        let m = Metrics::default();
        let b = Bands {
            tasks_first: true,
            first_width: Some(425),
        };
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            b,
        );
        assert_eq!(
            l.grips,
            vec![Rect::new(61, 6, 3, 18), Rect::new(499, 6, 3, 18)]
        );
        assert_eq!(l.tasks[0], Rect::new(68, 4, 160, 22));
        assert_eq!(l.tasks_limit(), 493);
        assert_eq!(l.quick[0].x, 506);
        assert_eq!(l.band_at_grip(0), Band::Tasks);
        assert_eq!(l.band_at_grip(1), Band::Quick);
        assert!(l.on_quick_band(500, 10) && l.on_quick_band(560, 10) && !l.on_quick_band(300, 10));
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            Bands {
                tasks_first: true,
                first_width: None,
            },
        );
        assert_eq!(l.grips[1].x, 68 + 160 + 6);
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            Bands {
                tasks_first: true,
                first_width: Some(0),
            },
        );
        assert_eq!(l.grips[1].x, 68 + 66 + 6);
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(0, 1, 0),
            30,
            b,
        );
        assert_eq!(l.grips.len(), 1);
        assert_eq!(l.tasks[0].x, 68);
        assert_eq!(l.bands, Bands::default());
    }

    #[test]
    fn an_open_tray_run_reaches_a_quick_band_dragged_short() {
        let m = Metrics::default();
        let lay = |bands: Bands| {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 800,
                thickness: m.taskbar_height,
            };
            let counts = TaskbarCounts {
                quick: 12,
                reveal: i32::MAX,
                ..arrow_counts(2, 30, 0)
            };
            let clock = ClockText { time: 30, date: 30 };
            TaskbarLayout::compute_on(
                &m,
                &TaskbarShape::CLASSIC,
                span,
                false,
                counts,
                clock,
                bands,
            )
        };
        let whole = lay(Bands::default());
        let short = lay(Bands {
            tasks_first: false,
            first_width: Some(60),
        });
        let more = short.quick_more.expect("a » cell");
        let arrow = short.chevron.expect("the arrow");
        assert_eq!(
            (whole.tray_hidden, short.tray_hidden),
            (16, 27),
            "the run reaches the shortened band"
        );
        assert!(arrow.x > more.right(), "the run stops short of the band");
    }

    #[test]
    fn a_quick_band_shorter_than_its_buttons_ends_in_a_more_cell() {
        let m = Metrics::default();
        let lay = |bands: Bands, edge: Edge| {
            let span = BarSpan {
                edge,
                length: 800,
                thickness: m.taskbar_height,
            };
            let clock = ClockText { time: 30, date: 30 };
            TaskbarLayout::compute_on(
                &m,
                &TaskbarShape::CLASSIC,
                span,
                false,
                counts(5, 1, 0),
                clock,
                bands,
            )
        };
        let wide = |w| Bands {
            tasks_first: false,
            first_width: Some(w),
        };
        let whole = lay(Bands::default(), Edge::Bottom);
        assert_eq!((whole.quick.len(), whole.quick_more), (5, None));
        let x0 = whole.quick[0].x;
        let three = lay(wide(3 * 23 + 14), Edge::Bottom);
        assert_eq!(three.quick, whole.quick[..3].to_vec());
        let r = three.quick_more.expect("a band 83 px long shows a »");
        assert_eq!(r, Rect::new(x0 + 69, 4, 14, 22));
        assert_eq!(three.hit(r.x + 7, r.y + 11), TaskbarHit::QuickMore);
        assert!(three.on_quick_band(r.right() - 1, 10));
        let two = lay(wide(3 * 23 + 13), Edge::Bottom);
        assert_eq!(two.quick.len(), 2, "a px short of three buttons and the »");
        assert_eq!(two.quick_more.map(|r| r.x), Some(x0 + 46));
        let least = lay(wide(0), Edge::Bottom);
        assert_eq!(least.quick, vec![]);
        assert_eq!(least.quick_more, Some(Rect::new(x0, 4, 14, 22)));
        assert_eq!(
            least.grips[1].x,
            x0 + 14 + 6,
            "the band keeps room for the » alone"
        );

        let top = lay(wide(83), Edge::Top);
        assert_eq!(top.quick_more, Some(Rect::new(x0 + 69, 2, 14, 22)));
        assert_eq!(three.scaled(2).quick_more, Some(r.scaled(2)));

        let after = lay(
            Bands {
                tasks_first: true,
                first_width: Some(2000),
            },
            Edge::Bottom,
        );
        let r = after
            .quick_more
            .expect("a task band dragged over the tools leaves the »");
        assert_eq!(after.quick, vec![]);
        assert_eq!(r.right(), after.tasks_limit() + 13 + 14);
    }

    #[test]
    fn a_short_column_quick_band_gives_its_last_cell_to_the_more_cell() {
        let span = BarSpan {
            edge: Edge::Left,
            length: 600,
            thickness: 60,
        };
        let lay = |first_width| {
            TaskbarLayout::compute_on(
                &Metrics::default(),
                &TaskbarShape::CLASSIC,
                span,
                false,
                counts(7, 1, 0),
                ClockText { time: 30, date: 0 },
                Bands {
                    tasks_first: false,
                    first_width,
                },
            )
        };
        let whole = lay(None);
        assert_eq!((whole.quick.len(), whole.quick_more), (7, None));
        let two_rows = lay(Some(2 * 23));
        assert_eq!(two_rows.quick, whole.quick[..3].to_vec());
        assert_eq!(two_rows.quick_more, Some(whole.quick[3]));
        assert_eq!(
            two_rows.hit(30, whole.quick[3].y + 5),
            TaskbarHit::QuickMore
        );
        let least = lay(Some(0));
        assert_eq!(least.quick, vec![whole.quick[0]]);
        assert_eq!(least.quick_more, Some(whole.quick[1]));
    }

    #[test]
    fn dragging_a_grip_resizes_or_swaps_the_bands() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(2, 1, 0), 30);
        let d = Bands::default();
        assert_eq!(
            l.drag_band(d, Band::Tasks, 315),
            Bands {
                tasks_first: false,
                first_width: Some(241)
            }
        );
        let back = l.drag_band(d, Band::Tasks, 50);
        assert!(!back.tasks_first);
        let l2 = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            back,
        );
        assert_eq!(l2.grips[1].x, 68 + 14 + 6);
        assert_eq!(l.drag_band(d, Band::Quick, 100), d);
        assert_eq!(
            l.drag_band(d, Band::Quick, 499),
            Bands {
                tasks_first: true,
                first_width: Some(425)
            }
        );
        let t = Bands {
            tasks_first: true,
            first_width: Some(425),
        };
        let l = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            t,
        );
        assert_eq!(
            l.drag_band(t, Band::Quick, 200),
            Bands {
                tasks_first: true,
                first_width: Some(126)
            }
        );
        let back = l.drag_band(t, Band::Quick, 40);
        assert!(back.tasks_first);
        let l2 = TaskbarLayout::compute_with(
            &m,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 1, 0),
            30,
            back,
        );
        assert_eq!(l2.grips[1].x, 140);
        assert_eq!(l2.tasks[0].width, 66);
        assert_eq!(l.drag_band(t, Band::Tasks, 300), t);
        let one =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, counts(0, 1, 0), 30);
        assert_eq!(one.drag_band(d, Band::Tasks, 300), d);
    }

    #[test]
    fn drop_slot_counts_the_buttons_left_of_the_pointer() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(3, 0, 0), 30);
        assert_eq!(l.drop_slot(0, 10), 0);
        assert_eq!(l.drop_slot(l.quick[0].x + 11, 10), 1);
        assert_eq!(l.drop_slot(l.quick[1].x + 5, 10), 1);
        assert_eq!(l.drop_slot(l.quick[2].x + 20, 10), 3);
        assert_eq!(l.drop_slot(700, 10), 3);
    }

    fn classic_row(n: usize) -> TaskbarLayout {
        TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, n, 0),
            30,
        )
    }

    #[test]
    fn a_shared_row_is_the_labelled_layout_with_its_stacks() {
        let mut l = classic_row(3);
        let before = l.tasks.clone();
        assert_eq!(l.stacks, vec![1, 1, 1], "compute lays single windows");
        l.lay_tasks(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            &[1, 4, 1],
            TaskRow::Shared,
        );
        assert_eq!(l.tasks, before);
        assert_eq!(l.stacks, vec![1, 4, 1]);
    }

    #[test]
    fn classic_icon_buttons_are_square_three_px_apart() {
        let mut l = classic_row(3);
        l.lay_tasks(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            &[1, 2, 3],
            TaskRow::Icons,
        );
        assert_eq!(
            l.tasks,
            vec![
                Rect::new(59, 4, 22, 22),
                Rect::new(84, 4, 22, 22),
                Rect::new(109, 4, 22, 22),
            ]
        );
        assert_eq!(l.stacks, vec![1, 2, 3]);
        assert_eq!(l.hit(90, 10), TaskbarHit::Task(1));
    }

    #[test]
    fn glass_icon_buttons_are_60_by_40_at_a_62_px_pitch() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let (m, shape) = (&theme.metrics, theme.taskbar_shape());
        let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 2, 0), 30);
        l.lay_tasks(m, &shape, &[5, 2], TaskRow::Icons);
        let x = l.tasks_x();
        assert_eq!(
            l.tasks,
            vec![Rect::new(x, 0, 60, 40), Rect::new(x + 62, 0, 60, 40)]
        );
    }

    #[test]
    fn small_icons_shrink_glass_to_a_30_px_bar_at_a_46_px_pitch() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let shape = crate::glass::small_bar_style(&Default::default()).taskbar;

        let mut l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
        l.lay_tasks(&m, &shape, &[3, 1], TaskRow::Icons);
        let x = l.tasks_x();
        assert_eq!(
            l.tasks,
            vec![Rect::new(x, 0, 44, 30), Rect::new(x + 46, 0, 44, 30)]
        );

        let mut l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
        l.lay_tasks(&m, &shape, &[1, 1], TaskRow::Shared);
        assert!(l.tasks.iter().all(|r| r.height == 30 && r.width <= 160));

        let mut l = TaskbarLayout::compute(&m, &shape, 200, true, counts(0, 6, 0), 30);
        l.lay_tasks(&m, &shape, &[1; 6], TaskRow::AtLeast(62));
        assert!(l.tasks.iter().all(|r| r.height == 30 && r.width == 62));
    }

    #[test]
    fn the_small_glass_orb_stands_5_px_above_the_bar_and_its_disc_is_start() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
        let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
        assert_eq!(l.start, Rect::new(0, 0, 54, 30), "Start is 54x30 at (0, 0)");
        assert_eq!(
            l.start_above,
            Some(Rect::new(0, -13, 54, 13)),
            "the rows above the bar are 54x13 at (0, -13)"
        );
        assert_eq!(
            (l.start_orb(false), l.start_orb(true)),
            (
                Some(Rect::new(0, -13, 54, 13)),
                Some(Rect::new(0, -13, 54, 43))
            ),
            "the orb's own buffer: the rows above, held with Start's 30 rows too"
        );
        assert_eq!(
            l.start_disc,
            Some(Rect::new(9, -5, 36, 36)),
            "the disc is 36x36 at (9, -5)"
        );
        let across = |y: i32| {
            (0..54)
                .filter(|&x| l.hit(x, y) == TaskbarHit::Start)
                .count()
        };
        assert_eq!(across(-5), 8, "the disc's top row is 8 px of Start");
        assert_eq!(across(-1), 24, "the row against the bar is 24 px of Start");
        assert_eq!(across(-6), 0, "no Start above the disc, 0 px");
        assert_eq!(l.hit(26, -5), TaskbarHit::Start, "(26, -5) is Start");
        assert_eq!(
            l.hit(10, -1),
            TaskbarHit::Nothing,
            "(10, -1), beside the disc, is Nothing"
        );
        assert_eq!(
            l.hit(26, -8),
            TaskbarHit::Nothing,
            "(26, -8), on the lit glow's rows, is Nothing"
        );
        assert_eq!(
            l.hit(100, -1),
            TaskbarHit::Nothing,
            "(100, -1), above the bar past Start, is Nothing"
        );
        let s2 = l.scaled(2);
        assert_eq!(
            s2.start_above,
            Some(Rect::new(0, -26, 108, 26)),
            "at scale 2 the rows above the bar are 108x26 at (0, -26)"
        );
        assert_eq!(
            s2.start_disc,
            Some(Rect::new(18, -10, 72, 72)),
            "at scale 2 the disc is 72x72 at (18, -10)"
        );
        assert!(
            s2.on_disc(53, -10),
            "at scale 2 the disc's top row holds (53, -10)"
        );
        assert!(
            !s2.on_disc(53, -11),
            "at scale 2 (53, -11) lies above the disc"
        );

        let full = TaskbarLayout::compute(
            &theme.metrics,
            &theme.taskbar_shape(),
            1920,
            true,
            counts(0, 2, 0),
            30,
        );
        assert_eq!(
            (full.start_above, full.start_disc),
            (Some(Rect::new(0, -6, 54, 6)), None),
            "the 40 px bar has 54x6 rows above it and no disc above it"
        );
        assert_eq!(
            full.start_orb(true),
            Some(Rect::new(0, -6, 54, 46)),
            "held, the 40 px bar's orb buffer takes in Start: 54x46 at (0, -6)"
        );
        assert_eq!(
            full.hit(26, -1),
            TaskbarHit::Nothing,
            "(26, -1) above the 40 px bar is Nothing"
        );
        let classic = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            1920,
            true,
            counts(0, 2, 0),
            30,
        );
        assert_eq!(classic.start_above, None, "Classic has no rows above: None");
    }

    #[test]
    fn a_small_glass_orbs_access_bounds_take_in_its_disc_above_the_bar() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
        let bounds = |l: &TaskbarLayout| {
            let mut t = Tree::window("Taskbar", (l.width, l.height));
            access(l, &TaskbarNames::default(), &mut t, access::ROOT);
            let b = t
                .get(t.find(Role::Button, "Start").unwrap())
                .unwrap()
                .bounds()
                .unwrap();
            (b.x0, b.y0, b.x1, b.y1)
        };
        let l = TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30);
        assert_eq!(
            bounds(&l),
            (0.0, -5.0, 54.0, 30.0),
            "Start reaches from (0, -5) to (54, 30)"
        );
        let full = TaskbarLayout::compute(
            &theme.metrics,
            &theme.taskbar_shape(),
            1920,
            true,
            counts(0, 2, 0),
            30,
        );
        assert_eq!(
            bounds(&full),
            (0.0, 0.0, 54.0, 40.0),
            "on the 40 px bar Start is its button, (0, 0) to (54, 40)"
        );
    }

    #[test]
    fn icon_buttons_past_the_row_are_left_out() {
        let mut l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            200,
            true,
            counts(0, 10, 0),
            30,
        );
        l.lay_tasks(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            &[1; 10],
            TaskRow::Icons,
        );
        assert_eq!(l.tasks_limit(), 146);
        assert_eq!(l.tasks.len(), 3);
        assert_eq!(l.stacks.len(), 3, "a stack for every laid-out button");
    }

    #[test]
    fn an_at_least_row_stops_shrinking_and_leaves_the_rest_out() {
        let m = Metrics::default();
        let mut l = classic_row(20);
        let avail = l.tasks_limit() - l.tasks_x();
        assert!(l.tasks[0].width < 62, "20 shared buttons are narrower");
        l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1; 20], TaskRow::AtLeast(62));
        assert!(l.tasks.iter().all(|r| r.width == 62));
        assert_eq!(l.tasks.len() as i32, (avail + TASK_GAP) / (62 + TASK_GAP));
        assert!(l.tasks.last().unwrap().right() <= l.tasks_limit());
        let mut roomy = classic_row(2);
        roomy.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::AtLeast(62));
        assert_eq!(roomy.tasks, classic_row(2).tasks);
    }

    #[test]
    fn an_at_least_row_never_passes_the_widest_button() {
        let m = Metrics {
            task_button_max_width: 40,
            ..Metrics::default()
        };
        let mut l =
            TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 2, 0), 30);
        l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::AtLeast(62));
        assert!(l.tasks.iter().all(|r| r.width == 40), "{:?}", l.tasks);
    }

    #[test]
    fn a_pinned_app_without_windows_takes_an_icon_slot_and_labels_share_the_rest() {
        let m = Metrics::default();
        let mut l = classic_row(3);
        l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[0, 1, 1], TaskRow::Shared);
        assert_eq!(
            l.tasks,
            vec![
                Rect::new(59, 4, 22, 22),
                Rect::new(84, 4, 160, 22),
                Rect::new(247, 4, 160, 22),
            ]
        );
        assert!(l.pinned_idle(0) && !l.pinned_idle(1) && !l.pinned_idle(2));
        let mut l = classic_row(8);
        let avail = l.tasks_limit() - l.tasks_x();
        l.lay_tasks(
            &m,
            &TaskbarShape::CLASSIC,
            &[1, 0, 1, 1, 1, 1, 1, 1],
            TaskRow::Shared,
        );
        assert_eq!(l.tasks[1].width, 22);
        assert_eq!(l.tasks[0].width, (avail - 25 - TASK_GAP * 6) / 7);
        assert_eq!(l.tasks[2].x, l.tasks[1].x + 25);
    }

    #[test]
    fn a_glass_pinned_slot_is_an_icon_button() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let (m, shape) = (&theme.metrics, theme.taskbar_shape());
        let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 3, 0), 30);
        l.lay_tasks(m, &shape, &[0, 1, 0], TaskRow::Shared);
        let x = l.tasks_x();
        assert_eq!(l.tasks[0], Rect::new(x, 0, 60, 40));
        assert_eq!(l.tasks[1].x, x + 62);
        // Glass's labelled buttons abut each other, but a pinned icon
        // after one keeps the icon buttons' 2 px gap.
        assert_eq!(l.tasks[2], Rect::new(l.tasks[1].right() + 2, 0, 60, 40));
        assert_eq!(l.icon_step(&shape), (60, 62));
    }

    #[test]
    fn glass_labelled_buttons_abut_160_px_wide_after_the_pinned_slots() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let (m, shape) = (&theme.metrics, theme.taskbar_shape());
        let mut l = TaskbarLayout::compute(m, &shape, 1920, true, counts(0, 6, 0), 30);
        l.lay_tasks(m, &shape, &[0, 0, 0, 1, 1, 1], TaskRow::AtLeast(62));
        let xs: Vec<i32> = l.tasks.iter().map(|r| r.x).collect();
        assert_eq!(xs, [59, 121, 183, 245, 405, 565]);
        for r in &l.tasks[3..] {
            assert_eq!((r.width, r.height), (160, 40), "a labelled slot at {}", r.x);
        }
    }

    #[test]
    fn a_labelled_button_keeps_the_icon_gap_before_an_idle_pin() {
        use crate::interface::Interface;
        for (i, icon_gap) in [(Interface::Glass, 2), (Interface::Classic, 3)] {
            let theme = crate::interfaces::theme_for(i);
            let (m, shape) = (&theme.metrics, theme.taskbar_shape());
            for (width, row) in [(1920, TaskRow::AtLeast(62)), (700, TaskRow::Shared)] {
                let mut l = TaskbarLayout::compute(m, &shape, width, true, counts(0, 5, 0), 30);
                l.lay_tasks(m, &shape, &[1, 0, 1, 1, 0], row);
                assert_eq!(l.tasks.len(), 5, "{i:?} at {width}: every slot fits");
                let gaps: Vec<i32> = l.tasks.windows(2).map(|p| p[1].x - p[0].right()).collect();
                let labelled = shape.labelled.gap;
                assert_eq!(
                    gaps,
                    [icon_gap, icon_gap, labelled, icon_gap],
                    "{i:?} at {width}: a pinned icon is {icon_gap} px from its neighbours"
                );
                assert!(l.tasks[4].right() <= l.tasks_limit(), "{i:?} at {width}");
            }
        }
    }

    #[test]
    fn scaled_layout_keeps_the_stacks() {
        let mut l = classic_row(2);
        l.lay_tasks(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            &[3, 1],
            TaskRow::Icons,
        );
        let s = l.scaled(2);
        assert_eq!(s.tasks[0], l.tasks[0].scaled(2));
        assert_eq!(s.stacks, vec![3, 1]);
    }

    fn bar_1920(i: crate::interface::Interface, strip: bool) -> TaskbarLayout {
        let theme = crate::interfaces::theme_for(i);
        let c = TaskbarCounts {
            tasks: 2,
            tray: 2,
            show_desktop: strip,
            ..TaskbarCounts::default()
        };
        TaskbarLayout::compute(&theme.metrics, &theme.taskbar_shape(), 1920, true, c, 40)
    }

    #[test]
    fn each_look_ends_its_bar_in_its_own_show_desktop_strip() {
        use crate::interface::Interface;
        for (i, strip, gap) in [
            (Interface::Classic, Rect::new(1896, 4, 22, 22), 2),
            (Interface::Blue, Rect::new(1895, 3, 25, 25), 12),
            (Interface::Glass, Rect::new(1905, 0, 15, 40), 2),
        ] {
            let (off, on) = (bar_1920(i, false), bar_1920(i, true));
            assert_eq!(off.show_desktop, None, "{i:?}: no strip with the key off");
            assert_eq!(
                off.clock.right(),
                1920 - gap,
                "{i:?}: the clock ends {gap} px in"
            );
            assert_eq!(on.show_desktop, Some(strip), "{i:?}: the strip's rect");
            assert_eq!(
                on.clock.right(),
                strip.x - gap,
                "{i:?}: the clock {gap} px before it"
            );
            let dx = off.clock.x - on.clock.x;
            assert_eq!(dx, 1920 - strip.x, "{i:?}: the clock moves by the strip");
            for (a, b) in off.tray.iter().zip(&on.tray) {
                assert_eq!(a.x - b.x, dx, "{i:?}: the tray moves with the clock");
            }
            assert_eq!(off.tasks_limit() - on.tasks_limit(), dx, "{i:?}: the row");
            assert!(
                on.notification.right() <= strip.x,
                "{i:?}: the well stops short"
            );
            assert_eq!(on.width, 1920, "{i:?}: the bar keeps its width");
            assert_eq!(on.scaled(2).show_desktop, Some(strip.scaled(2)));
        }
        let glass = bar_1920(Interface::Glass, true);
        assert_eq!(
            glass.notification.right(),
            1905,
            "Glass's well runs to the strip"
        );
    }

    #[test]
    fn the_show_desktop_strip_takes_its_own_clicks() {
        use crate::interface::Interface;
        let l = bar_1920(Interface::Glass, true);
        assert_eq!(l.hit(1905, 0), TaskbarHit::ShowDesktop);
        assert_eq!(l.hit(1919, 39), TaskbarHit::ShowDesktop);
        assert_eq!(
            l.hit(1903, 20),
            TaskbarHit::Nothing,
            "the gap after the clock"
        );
        assert_eq!(l.hit(l.clock.x + 2, 20), TaskbarHit::Clock);
        let off = bar_1920(Interface::Glass, false);
        assert_eq!(off.hit(1919, 20), TaskbarHit::Nothing, "no strip, the well");
        let classic = bar_1920(Interface::Classic, true);
        assert_eq!(classic.hit(1900, 10), TaskbarHit::ShowDesktop);
        assert_eq!(
            classic.hit(1918, 10),
            TaskbarHit::Nothing,
            "the 2 px margin"
        );
    }

    #[test]
    fn access_names_the_show_desktop_strip() {
        use crate::interface::Interface;
        let tree_of = |l: &TaskbarLayout| {
            let mut t = Tree::window("Taskbar", (l.width, l.height));
            access(l, &TaskbarNames::default(), &mut t, access::ROOT);
            t
        };
        let l = bar_1920(Interface::Glass, true);
        let t = tree_of(&l);
        let id = t
            .find(Role::Button, "Show desktop")
            .expect("a Show desktop button");
        assert_eq!(
            t.get(id).unwrap().bounds().map(|b| b.x0),
            Some(1905.0),
            "at the strip"
        );
        let t = tree_of(&bar_1920(Interface::Glass, false));
        assert!(
            t.find(Role::Button, "Show desktop").is_none(),
            "none without it"
        );
    }

    #[test]
    fn scaled_layout_scales_tray_too() {
        let m = Metrics::default();
        let l = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, true, counts(0, 1, 2), 30);
        let s = l.scaled(2);
        assert_eq!(s.tray[0], l.tray[0].scaled(2));
        assert_eq!(s.tray.len(), 2);
    }

    #[test]
    fn an_edge_goes_by_its_config_name() {
        assert_eq!(
            Edge::default(),
            Edge::Bottom,
            "a bar runs along the foot by default"
        );
        assert_eq!(
            Edge::ALL.map(Edge::name),
            ["bottom", "top", "left", "right"]
        );
        assert!(Edge::Left.is_side() && Edge::Right.is_side());
        assert!(!Edge::Bottom.is_side() && !Edge::Top.is_side());
        for e in Edge::ALL {
            assert_eq!(Edge::from_name(e.name()), Some(e), "{e:?} reads back");
        }
        assert_eq!(Edge::from_name("sideways"), None);
    }

    #[test]
    fn a_top_glass_bar_hangs_its_orb_below_the_bar() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let small = crate::glass::small_bar_style(&Default::default()).taskbar;
        let bottom = TaskbarLayout::compute(&m, &small, 1920, true, counts(0, 2, 0), 30);
        let l = bottom.clone().on_edge(Edge::Top);
        assert_eq!(l.start, Rect::new(0, 0, 54, 30), "Start is 54x30 at (0, 0)");
        assert_eq!(
            l.start_above,
            Some(Rect::new(0, 30, 54, 13)),
            "the rows below the bar are 54x13 at (0, 30)"
        );
        assert_eq!(
            l.start_disc,
            Some(Rect::new(9, 0, 36, 36)),
            "the disc is 36x36 at (9, 0), lowered by its 5-row hang"
        );
        assert_eq!(
            l.disc_past_bar(),
            Some(Rect::new(9, 30, 36, 5)),
            "the disc's art hangs 36x5 at (9, 30)"
        );
        assert_eq!(
            (l.start_orb(false), l.start_orb(true)),
            (
                Some(Rect::new(0, 30, 54, 13)),
                Some(Rect::new(0, 0, 54, 43))
            ),
            "the orb's own buffer: the rows below, held with Start's 30 rows too"
        );
        // The art is upright and lowered by 5 rows, so each row below the
        // bar is the disc's row 5 rows higher over a Bottom bar: 26 px of
        // Start against the bar, 14 in the art's last row. The box's 36th
        // row, which runs off a Bottom bar's foot, has no art and no Start.
        let across = |y: i32| {
            (0..54)
                .filter(|&x| l.hit(x, y) == TaskbarHit::Start)
                .count()
        };
        for y in 30..35 {
            let upright = (0..54).filter(|&x| bottom.on_disc(x, y - 5)).count();
            assert_eq!(across(y), upright, "row {y} is as wide as row {}", y - 5);
        }
        assert_eq!(across(30), 26, "the row against the bar is 26 px of Start");
        assert_eq!(across(34), 14, "the art's last row is 14 px of Start");
        assert_eq!(across(35), 0, "no Start below the art, 0 px");
        assert_eq!(
            l.hit(10, 30),
            TaskbarHit::Nothing,
            "(10, 30), beside the disc, is Nothing"
        );
        assert_eq!(
            l.hit(26, 38),
            TaskbarHit::Nothing,
            "(26, 38), on the lit glow's rows, is Nothing"
        );
        assert_eq!(
            l.hit(26, -1),
            TaskbarHit::Nothing,
            "(26, -1), past the screen's top edge, is Nothing"
        );
        assert_eq!(l.on_edge(Edge::Bottom), bottom, "moved back it rises");

        let full = TaskbarLayout::compute(
            &theme.metrics,
            &theme.taskbar_shape(),
            1920,
            true,
            counts(0, 2, 0),
            30,
        )
        .on_edge(Edge::Top);
        assert_eq!(
            (full.start_above, full.start_disc),
            (Some(Rect::new(0, 40, 54, 6)), None),
            "the 40 px bar has 54x6 rows below it and no disc below it"
        );
        assert_eq!(
            full.hit(26, 40),
            TaskbarHit::Nothing,
            "(26, 40) below the 40 px bar is Nothing"
        );
    }

    #[test]
    fn a_top_small_glass_orbs_access_bounds_take_in_its_disc_below_the_bar() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let shape = crate::glass::small_bar_style(&Default::default()).taskbar;
        let l =
            TaskbarLayout::compute(&m, &shape, 1920, true, counts(0, 2, 0), 30).on_edge(Edge::Top);
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &TaskbarNames::default(), &mut t, access::ROOT);
        let b = t
            .get(t.find(Role::Button, "Start").unwrap())
            .unwrap()
            .bounds()
            .unwrap();
        assert_eq!(
            (b.x0, b.y0, b.x1, b.y1),
            (0.0, 0.0, 54.0, 35.0),
            "Start reaches from (0, 0) to (54, 35)"
        );
    }

    #[test]
    fn the_reveal_line_is_the_bars_inner_two_rows() {
        let l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            TaskbarCounts::default(),
            40,
        );
        let h = l.height;
        assert_eq!(
            l.reveal_line(),
            Rect::new(0, 0, 800, REVEAL_LINE),
            "a Bottom bar's top rows"
        );
        let top = l.on_edge(Edge::Top);
        assert_eq!(
            top.reveal_line(),
            Rect::new(0, h - REVEAL_LINE, 800, REVEAL_LINE),
            "a Top bar's foot rows"
        );
        let thin = TaskbarLayout { height: 1, ..top };
        assert_eq!(
            thin.reveal_line(),
            Rect::new(0, 0, 800, 1),
            "a 1 px bar is all line"
        );
    }

    #[test]
    fn an_edge_turns_a_rect_between_along_across_and_x_y_and_back() {
        let r = Rect::new(59, 4, 160, 22);
        assert_eq!(
            Edge::Bottom.to_xy(r, 28),
            r,
            "a Bottom bar is its own along/across form"
        );
        assert_eq!(
            Edge::Top.to_xy(r, 28),
            Rect::new(59, 2, 160, 22),
            "a Top bar mirrors it across its 28 rows"
        );
        for e in Edge::ALL {
            assert_eq!(
                e.to_xy(e.to_xy(r, 28), 28),
                r,
                "{e:?}: twice is where it started"
            );
        }
        assert_eq!(
            Edge::Left.to_xy(r, 60),
            Rect::new(4, 59, 22, 160),
            "a side bar swaps x and y: along runs down the column"
        );
        assert_eq!(Edge::Right.to_xy(r, 60), Edge::Left.to_xy(r, 60));
    }

    #[test]
    fn along_a_top_bar_is_along_x_as_on_a_bottom_one() {
        for edge in [Edge::Bottom, Edge::Top] {
            let l = classic_row(3).on_edge(edge);
            let r = l.tasks[1];
            assert_eq!((l.length(), l.thickness()), (800, 28), "{edge:?}");
            assert_eq!(l.along(r.x + 5, r.y + 3), r.x + 5, "{edge:?}");
            assert_eq!(l.along_f(12.5, 3.0), 12.5, "{edge:?}");
            assert_eq!(
                l.quick_drag(5.0, -40.0),
                5.0,
                "{edge:?}: only along the bar"
            );
            assert_eq!(
                l.moved_along(r, 7),
                Rect::new(r.x + 7, r.y, r.width, r.height),
                "{edge:?}"
            );
            assert_eq!(
                l.placed_along(r, 300),
                Rect::new(300, r.y, r.width, r.height),
                "{edge:?}"
            );
            assert_eq!(l.along_rects(&l.tasks)[1].x, r.x, "{edge:?}");
            assert_eq!(l.to_xy(l.to_along(r)), r, "{edge:?}: there and back");
        }
    }

    #[test]
    fn a_top_span_lays_the_bottom_bar_turned_over() {
        let m = Metrics::default();
        let c = TaskbarCounts {
            quick: 2,
            pager: 2,
            tasks: 3,
            tray: 2,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge: Edge::Top,
            length: 800,
            thickness: 28,
        };
        let clock = ClockText { time: 30, date: 30 };
        let top = TaskbarLayout::compute_on(
            &m,
            &TaskbarShape::CLASSIC,
            span,
            false,
            c,
            clock,
            Bands::default(),
        );
        let bottom = TaskbarLayout::compute(&m, &TaskbarShape::CLASSIC, 800, false, c, 30);
        assert_eq!(top, bottom.on_edge(Edge::Top));
    }

    #[test]
    fn a_two_line_clock_is_as_wide_as_its_wider_line() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let shape = theme.taskbar_shape();
        let lay = |thickness| {
            let span = BarSpan {
                edge: Edge::Bottom,
                length: 1920,
                thickness,
            };
            let clock = ClockText { time: 40, date: 70 };
            TaskbarLayout::compute_on(
                &theme.metrics,
                &shape,
                span,
                true,
                TaskbarCounts::default(),
                clock,
                Bands::default(),
            )
        };
        assert_eq!(
            lay(40).clock.width,
            70 + shape.clock.pad,
            "two lines on the 40 px bar: the date's width"
        );
        assert_eq!(
            lay(30).clock.width,
            40 + shape.clock.pad,
            "one line on a 30 px bar: the time's"
        );
    }

    #[test]
    fn a_top_bar_drops_a_quick_launch_button_where_a_bottom_one_does() {
        let bottom = classic_full(Edge::Bottom);
        let top = classic_full(Edge::Top);
        for x in [0, 66, 70, 80, 95, 118, 300] {
            assert_eq!(top.drop_slot(x, 20), bottom.drop_slot(x, 7), "x {x}");
            assert_eq!(
                top.on_quick_band(x, 20),
                bottom.on_quick_band(x, 7),
                "x {x}"
            );
        }
    }

    #[test]
    fn draw_art_turns_a_bottom_picture_to_each_edge() {
        let ink = Color::rgb(255, 0, 0);
        let px = ink.argb();
        let art = |c: &mut Canvas, at: Rect| c.fill_rect(Rect::new(at.x, at.y, 3, 1), ink);
        let mut b = Canvas::new(6, 4);
        Edge::Bottom.draw_art(&mut b, Rect::new(0, 0, 6, 4), art);
        assert_eq!(
            (b.get(2, 0), b.get(3, 0), b.get(0, 1)),
            (px, 0, 0),
            "Bottom: as drawn"
        );
        let mut t = Canvas::new(6, 4);
        Edge::Top.draw_art(&mut t, Rect::new(0, 0, 6, 4), art);
        assert_eq!((t.get(0, 3), t.get(0, 0)), (px, 0), "Top: on the foot row");
        let mut l = Canvas::new(4, 6);
        Edge::Left.draw_art(&mut l, Rect::new(0, 0, 4, 6), art);
        assert_eq!(
            (l.get(3, 0), l.get(3, 2), l.get(3, 3), l.get(0, 0)),
            (px, px, 0, 0),
            "Left: down the right column, from the top"
        );
        let mut r = Canvas::new(4, 6);
        Edge::Right.draw_art(&mut r, Rect::new(0, 0, 4, 6), art);
        assert_eq!(
            (r.get(0, 0), r.get(0, 2), r.get(0, 3), r.get(3, 0)),
            (px, px, 0, 0),
            "Right: down the left column"
        );
    }

    #[test]
    fn a_side_bars_reveal_line_is_its_inner_two_columns() {
        let row = classic_row(1);
        let left = TaskbarLayout {
            edge: Edge::Left,
            width: 60,
            height: 600,
            ..row.clone()
        };
        assert_eq!(
            left.reveal_line(),
            Rect::new(58, 0, 2, 600),
            "a Left bar's right columns"
        );
        let right = TaskbarLayout {
            edge: Edge::Right,
            ..left.clone()
        };
        assert_eq!(
            right.reveal_line(),
            Rect::new(0, 0, 2, 600),
            "a Right bar's left columns"
        );
        assert_eq!((left.length(), left.thickness()), (600, 60));
    }

    #[test]
    fn a_side_bar_is_starts_width_or_one_slot_by_default_and_clamped() {
        use crate::interface::Interface;
        let m = Metrics::default();
        let c = TaskbarShape::CLASSIC;
        assert_eq!((c.side_slot(&m), c.side_default(&m)), (28, 60), "Classic");
        let blue = crate::interfaces::theme_for(Interface::Blue);
        let b = blue.taskbar_shape();
        assert_eq!(
            (b.side_slot(&blue.metrics), b.side_default(&blue.metrics)),
            (30, blue.metrics.start_button_width),
            "Blue: Start fills its bar's height, so its width is the column's"
        );
        let glass = crate::interfaces::theme_for(Interface::Glass);
        let g = glass.taskbar_shape();
        assert_eq!(
            (g.side_slot(&glass.metrics), g.side_default(&glass.metrics)),
            (60, 60),
            "Glass: one 60 px slot"
        );
        let small_m = Metrics {
            taskbar_height: 30,
            ..glass.metrics.clone()
        };
        let small = crate::glass::small_bar_style(&Default::default()).taskbar;
        assert_eq!(
            (small.side_slot(&small_m), small.side_default(&small_m)),
            (44, 44),
            "Glass's small bar: one 44 px slot"
        );
        assert_eq!(c.side_width(&m, 0, 1920), 60, "0: the default");
        assert_eq!(c.side_width(&m, 10, 1920), 28, "never under a slot");
        assert_eq!(c.side_width(&m, 200, 1920), 200);
        assert_eq!(
            c.side_width(&m, 2000, 1920),
            640,
            "never past a third of the output"
        );
        assert_eq!(
            c.side_width(&m, 0, 60),
            28,
            "an output too narrow for the default: a slot"
        );
    }

    fn classic_column(edge: Edge, t: i32, c: TaskbarCounts, locked: bool) -> TaskbarLayout {
        let span = BarSpan {
            edge,
            length: 600,
            thickness: t,
        };
        let clock = ClockText { time: 30, date: 0 };
        TaskbarLayout::compute_on(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            span,
            locked,
            c,
            clock,
            Bands::default(),
        )
    }

    #[test]
    fn a_left_classic_column_stacks_start_its_bands_tray_clock_and_strip() {
        let c = TaskbarCounts {
            quick: 3,
            pager: 2,
            tasks: 2,
            tray: 2,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let l = classic_column(Edge::Left, 60, c, true);
        assert_eq!((l.width, l.height), (60, 600), "a column 60 wide");
        assert_eq!(
            l.start,
            Rect::new(2, 2, 54, 22),
            "Start 2 px from the screen's edge and its top, 4 from the inner edge"
        );
        assert_eq!(
            l.pager,
            vec![Rect::new(2, 27, 23, 23), Rect::new(25, 27, 23, 23)],
            "the pager's own row of 23 px cells, 3 px under Start"
        );
        assert_eq!(
            l.quick,
            vec![
                Rect::new(2, 54, 23, 23),
                Rect::new(25, 54, 23, 23),
                Rect::new(2, 77, 23, 23),
            ],
            "two cells a row, 4 px under the pager"
        );
        assert_eq!(
            l.tasks,
            vec![Rect::new(2, 104, 54, 22), Rect::new(2, 129, 54, 22)],
            "one 22 px button a row, 3 px apart, labelled"
        );
        assert!(!l.icons_only);
        assert_eq!(
            l.chevron,
            Some(Rect::new(4, 513, 50, 16)),
            "the arrow across the well's top"
        );
        assert_eq!(
            l.tray,
            vec![Rect::new(7, 530, 16, 16), Rect::new(27, 530, 16, 16)],
            "the icons under it, two a row, 4 px inside the well's frame"
        );
        assert_eq!(l.clock, Rect::new(2, 552, 54, 22));
        assert_eq!(l.clock_lines, None, "no room across for the date");
        assert_eq!(l.notification, Rect::new(2, 512, 54, 62));
        assert_eq!(
            l.show_desktop,
            Some(Rect::new(2, 576, 54, 22)),
            "last, 2 px off the foot"
        );
        assert_eq!((l.tasks_x(), l.tasks_limit()), (104, 509));
        assert_eq!(l.more, None);
        let r = classic_column(Edge::Right, 60, c, true);
        assert_eq!(r.start, Rect::new(4, 2, 54, 22), "against the right edge");
        assert_eq!(r.quick[1], Rect::new(27, 54, 23, 23));
        assert_eq!(r.tasks[0], Rect::new(4, 104, 54, 22));
        assert_eq!(r.chevron, Some(Rect::new(6, 513, 50, 16)));
    }

    #[test]
    fn along_down_a_side_bar_is_along_y() {
        let l = classic_column(Edge::Left, 60, counts(0, 2, 0), true);
        let r = l.tasks[1];
        assert_eq!((l.length(), l.thickness()), (600, 60));
        assert_eq!(l.along(r.x + 5, r.y + 3), r.y + 3);
        assert_eq!(l.along_f(12.0, 300.5), 300.5);
        assert_eq!(l.quick_drag(5.0, -2.0), 5.0, "either way over the grid");
        assert_eq!(
            l.moved_along(r, 7),
            Rect::new(r.x, r.y + 7, r.width, r.height)
        );
        assert_eq!(
            l.placed_along(r, 300),
            Rect::new(r.x, 300, r.width, r.height)
        );
        assert_eq!(
            l.along_rects(&l.tasks)[1],
            Rect::new(r.y, r.x, r.height, r.width)
        );
    }

    #[test]
    fn a_column_labels_its_buttons_once_it_is_wide_enough() {
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let shape = glass.taskbar_shape();
        let lay = |m: &Metrics, s: &TaskbarShape, t: i32| {
            let span = BarSpan {
                edge: Edge::Left,
                length: 800,
                thickness: t,
            };
            TaskbarLayout::compute_on(
                m,
                s,
                span,
                true,
                counts(0, 2, 0),
                ClockText::default(),
                Bands::default(),
            )
        };
        assert!(
            lay(&glass.metrics, &shape, 60).icons_only,
            "Glass's 60 px slot: icons only"
        );
        assert!(lay(&glass.metrics, &shape, 69).icons_only);
        assert!(
            !lay(&glass.metrics, &shape, 70).icons_only,
            "70 px: 24 px of title after the icon's 46"
        );
        let m = Metrics::default();
        assert!(
            lay(&m, &TaskbarShape::CLASSIC, 59).icons_only,
            "a 53 px button row"
        );
        assert!(
            !lay(&m, &TaskbarShape::CLASSIC, 60).icons_only,
            "Classic's default: labelled"
        );
        let mut l = lay(&m, &TaskbarShape::CLASSIC, 60);
        l.lay_tasks(&m, &TaskbarShape::CLASSIC, &[1, 1], TaskRow::Icons);
        assert!(l.icons_only, "Combine stays icons only however wide");
        assert_eq!(l.tasks[0].height, 22, "still one button a row");
    }

    #[test]
    fn an_open_column_adds_every_hidden_cell_to_its_grid() {
        let shut = classic_column(Edge::Left, 60, arrow_counts(5, 3, 0), true);
        let open = classic_column(Edge::Left, 60, arrow_counts(5, 3, 1), true);
        assert_eq!((shut.tray.len(), shut.tray_hidden), (5, 0));
        assert_eq!(
            (open.tray.len(), open.tray_hidden),
            (8, 3),
            "any open px shows the whole run"
        );
        // The grid grows up from the clock: its last row stays where it was.
        // Cells are numbered from the top-left, so the shown ones may change
        // column.
        assert_eq!(open.tray.last().map(|r| r.y), shut.tray.last().map(|r| r.y));
        assert!(open.chevron.unwrap().y < shut.chevron.unwrap().y);
    }

    #[test]
    fn a_column_wraps_its_tray_into_rows_above_the_clock() {
        let c = TaskbarCounts {
            tray: 5,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let l = classic_column(Edge::Left, 60, c, true);
        assert_eq!(
            l.clock,
            Rect::new(2, 576, 54, 22),
            "no strip: 2 px off the foot"
        );
        assert_eq!(l.chevron, Some(Rect::new(4, 497, 50, 16)));
        assert_eq!(
            l.tray,
            vec![
                Rect::new(7, 514, 16, 16),
                Rect::new(27, 514, 16, 16),
                Rect::new(7, 534, 16, 16),
                Rect::new(27, 534, 16, 16),
                Rect::new(7, 554, 16, 16),
            ],
            "two a row, the last row 6 px over the clock"
        );
        assert_eq!(
            l.notification,
            Rect::new(2, 496, 54, 102),
            "from 1 px over the arrow to the clock's foot"
        );
        assert_eq!(l.tasks_limit(), 493);
    }

    fn glass_column(edge: Edge, t: i32, small: bool, counts: TaskbarCounts) -> TaskbarLayout {
        let mut glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        if small {
            crate::interfaces::small_bar(&mut glass, &Default::default());
        }
        let span = BarSpan {
            edge,
            length: 700,
            thickness: t,
        };
        TaskbarLayout::compute_on(
            &glass.metrics,
            &glass.taskbar_shape(),
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        )
    }

    #[test]
    fn an_orb_column_keeps_its_tray_grid_2_px_off_both_sides() {
        let c = TaskbarCounts {
            tray: 7,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for (t, small, per_row) in [(60, false, 3), (44, true, 2), (140, false, 7)] {
            for edge in [Edge::Left, Edge::Right] {
                let l = glass_column(edge, t, small, c);
                let cells: Vec<Rect> = l.chevron.into_iter().chain(l.tray.clone()).collect();
                assert_eq!(cells.len(), 8, "{edge:?} at {t} px: every cell laid");
                for r in &cells {
                    assert!(
                        r.x >= 2 && r.right() <= t - 2,
                        "{edge:?} at {t} px: {r:?} 2 px clear of the screen edge and the inner edge"
                    );
                }
                let (x0, x1) = (cells[0].x, cells[per_row - 1].right());
                assert_eq!(cells[per_row].x, x0, "{edge:?} at {t} px: {per_row} a row");
                assert_eq!(
                    x0,
                    t - x1,
                    "{edge:?} at {t} px: the grid centred across the column"
                );
            }
        }
    }

    #[test]
    fn an_orb_column_centres_its_quick_launch_and_pager_grids() {
        let c = TaskbarCounts {
            quick: 1,
            pager: 4,
            ..TaskbarCounts::default()
        };
        for edge in [Edge::Left, Edge::Right] {
            let l = glass_column(edge, 60, false, c);
            assert_eq!(l.quick[0].x, 18, "{edge:?}: one 23 px cell, centred");
            let xs: Vec<i32> = l.pager.iter().map(|r| r.x).collect();
            assert_eq!(xs, [7, 30, 7, 30], "{edge:?}: two cells a row, centred");
            let wide = glass_column(edge, 140, false, c);
            let xs: Vec<i32> = wide.pager.iter().map(|r| r.x).collect();
            assert_eq!(
                xs,
                [24, 47, 70, 93],
                "{edge:?} at 140 px: the four cells on one row, centred"
            );
        }
    }

    #[test]
    fn a_column_wraps_the_pager_and_quick_launch_into_rows_of_its_width() {
        let blue = crate::interfaces::theme_for(crate::interface::Interface::Blue);
        let shape = blue.taskbar_shape();
        let t = shape.side_default(&blue.metrics);
        let span = BarSpan {
            edge: Edge::Left,
            length: 700,
            thickness: t,
        };
        let c = TaskbarCounts {
            quick: 5,
            pager: 6,
            tasks: 1,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute_on(
            &blue.metrics,
            &shape,
            span,
            true,
            c,
            ClockText::default(),
            Bands::default(),
        );
        assert_eq!(
            l.start,
            Rect::new(0, 0, t, 30),
            "Start across the whole column"
        );
        let xs: Vec<i32> = l.pager.iter().map(|r| r.x).collect();
        assert_eq!(
            xs,
            [2, 25, 48, 71, 2, 25],
            "four 23 px cells a row, from the left"
        );
        assert_eq!(l.pager[0].y, 37, "7 px under Start");
        assert_eq!(l.pager[4], Rect::new(2, l.pager[0].y + 23, 23, 23));
        assert_eq!(
            l.quick[0].y,
            l.pager[5].bottom() + 4,
            "Quick Launch 4 px under the pager"
        );
        assert_eq!(l.quick[4].y, l.quick[0].y + 23);
        assert_eq!(l.tasks_x(), l.quick[4].bottom() + 4);
    }

    #[test]
    fn a_column_sets_the_date_under_the_time_once_it_fits_across() {
        let lay = |t: i32| {
            let span = BarSpan {
                edge: Edge::Right,
                length: 600,
                thickness: t,
            };
            let clock = ClockText { time: 40, date: 60 };
            TaskbarLayout::compute_on(
                &Metrics::default(),
                &TaskbarShape::CLASSIC,
                span,
                true,
                counts(0, 1, 0),
                clock,
                Bands::default(),
            )
        };
        let narrow = lay(60);
        assert_eq!(
            narrow.clock_lines, None,
            "the date needs 76 px, the row has 54"
        );
        assert_eq!(narrow.clock, Rect::new(4, 576, 54, 22));
        let wide = lay(82);
        assert_eq!(wide.clock_lines, Some([17, 34]));
        assert_eq!(
            wide.clock,
            Rect::new(4, 558, 76, 40),
            "two lines take 40 rows"
        );
        assert_eq!(
            classic_row(1).clock_lines,
            None,
            "a 28 px bottom bar keeps one line"
        );
    }

    fn look_column(
        interface: crate::interface::Interface,
        edge: Edge,
        (t, length): (i32, i32),
        c: TaskbarCounts,
        clock: ClockText,
        bands: Bands,
    ) -> TaskbarLayout {
        let theme = crate::interfaces::theme_for(interface);
        let span = BarSpan {
            edge,
            length,
            thickness: t,
        };
        TaskbarLayout::compute_on(
            &theme.metrics,
            &theme.taskbar_shape(),
            span,
            true,
            c,
            clock,
            bands,
        )
    }

    #[test]
    fn a_blue_or_glass_column_runs_its_well_and_strip_across_the_column() {
        use crate::interface::Interface;
        let c = TaskbarCounts {
            tasks: 1,
            tray: 2,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let clock = ClockText { time: 40, date: 0 };
        let lay = |i, edge, t| look_column(i, edge, (t, 600), c, clock, Bands::default());
        let blue = lay(Interface::Blue, Edge::Left, 100);
        assert_eq!(
            blue.show_desktop,
            Some(Rect::new(0, 575, 100, 25)),
            "Blue: a button's height across the column, last"
        );
        assert_eq!(
            blue.clock,
            Rect::new(2, 538, 95, 25),
            "12 px over the strip"
        );
        assert_eq!(
            blue.tray,
            vec![Rect::new(2, 516, 16, 16), Rect::new(20, 516, 16, 16)],
            "2 px apart, 6 px over the clock"
        );
        assert_eq!(
            blue.notification,
            Rect::new(0, 505, 100, 70),
            "from 11 px over the tray to the strip, across the column"
        );
        let right = lay(Interface::Blue, Edge::Right, 100);
        assert_eq!(right.notification, blue.notification);
        assert_eq!(right.tray[0], Rect::new(3, 516, 16, 16));
        let glass = lay(Interface::Glass, Edge::Left, 60);
        assert_eq!(
            glass.show_desktop,
            Some(Rect::new(0, 585, 60, 15)),
            "Glass: 15 px across the column, last"
        );
        assert_eq!(glass.clock, Rect::new(0, 543, 60, 40));
        assert_eq!(
            glass.tray,
            vec![Rect::new(12, 521, 16, 16), Rect::new(32, 521, 16, 16)],
            "4 px apart, centred across the column"
        );
        assert_eq!(
            glass.notification,
            Rect::new(0, 513, 60, 72),
            "from 8 px over the tray to the strip"
        );
    }

    #[test]
    fn a_blue_or_glass_column_sets_the_date_under_the_time_once_it_fits_across() {
        use crate::interface::Interface;
        let clock = ClockText { time: 40, date: 60 };
        let lay = |i, t| {
            look_column(
                i,
                Edge::Left,
                (t, 600),
                counts(0, 1, 0),
                clock,
                Bands::default(),
            )
        };
        let blue = lay(Interface::Blue, 100);
        assert_eq!(blue.clock_lines, Some([17, 34]), "Blue: 62 px in 95");
        assert_eq!(
            blue.clock,
            Rect::new(2, 548, 95, 40),
            "two lines take 40 rows"
        );
        assert_eq!(
            lay(Interface::Blue, 60).clock_lines,
            None,
            "62 px in 55: one line"
        );
        let glass = lay(Interface::Glass, 120);
        assert_eq!(glass.clock_lines, Some([17, 34]), "Glass: 76 px in 120");
        assert_eq!(glass.clock, Rect::new(0, 558, 120, 40));
        assert_eq!(lay(Interface::Glass, 60).clock_lines, None, "76 px in 60");
    }

    #[test]
    fn a_column_with_the_tasks_first_lays_them_above_quick_launch() {
        use crate::interface::Interface;
        let lay = |tasks, first_width| {
            let bands = Bands {
                tasks_first: true,
                first_width,
            };
            let clock = ClockText { time: 30, date: 0 };
            look_column(
                Interface::Classic,
                Edge::Left,
                (60, 600),
                counts(3, tasks, 0),
                clock,
                bands,
            )
        };
        let l = lay(2, Some(100));
        assert_eq!(l.tasks_x(), 27, "the task row first, 3 px under Start");
        assert_eq!(
            l.tasks,
            vec![Rect::new(2, 27, 54, 22), Rect::new(2, 52, 54, 22)]
        );
        assert_eq!(l.tasks_limit(), 127, "the band 100 px long");
        assert_eq!(
            l.quick,
            vec![
                Rect::new(2, 131, 23, 23),
                Rect::new(25, 131, 23, 23),
                Rect::new(2, 154, 23, 23),
            ],
            "Quick Launch 4 px after it, two cells a row"
        );
        let natural = lay(1, None);
        assert_eq!(
            (natural.tasks_x(), natural.tasks_limit()),
            (27, 49),
            "unsized, the band is one full button long"
        );
        assert_eq!(natural.quick[0].y, 53);
    }

    #[test]
    fn a_column_a_third_of_a_1920_output_wide_labels_its_buttons_across_it() {
        use crate::interface::Interface;
        let m = Metrics::default();
        let t = TaskbarShape::CLASSIC.side_width(&m, 2000, 1920);
        assert_eq!(t, 640);
        let l = look_column(
            Interface::Classic,
            Edge::Left,
            (t, 1080),
            counts(0, 2, 3),
            ClockText { time: 40, date: 60 },
            Bands::default(),
        );
        assert_eq!((l.width, l.height), (640, 1080));
        assert_eq!(l.start, Rect::new(2, 2, 54, 22), "Start keeps its width");
        assert!(!l.icons_only);
        assert_eq!(l.tasks[0], Rect::new(2, 27, 634, 22), "across the row");
        assert_eq!(l.clock, Rect::new(2, 1038, 634, 40));
        assert!(l.clock_lines.is_some(), "the date fits across");
        assert!(
            l.tray.iter().all(|r| r.y == l.tray[0].y),
            "the tray on one row: {:?}",
            l.tray
        );
    }

    #[test]
    fn a_side_bars_grips_lie_across_the_column() {
        let l = classic_column(Edge::Left, 60, counts(2, 1, 0), false);
        assert_eq!(
            l.grips,
            vec![Rect::new(20, 29, 18, 3), Rect::new(20, 65, 18, 3)],
            "3 px bars across the button row, 2 px under Start and 6 px after the band"
        );
        assert_eq!(l.quick[0].y, 36);
        assert_eq!(l.tasks_x(), 72);
        assert_eq!(l.hit(25, 30), TaskbarHit::Grip(0));
        assert_eq!(l.hit(25, 67), TaskbarHit::Grip(1));
        assert_eq!(
            l.drag_band(Bands::default(), Band::Tasks, 300),
            Bands {
                tasks_first: false,
                first_width: Some(300 - 6 - 36)
            },
            "dragged down the column"
        );
    }

    #[test]
    fn a_column_drops_a_quick_launch_button_in_reading_order() {
        let l = classic_column(Edge::Left, 60, counts(3, 0, 0), true);
        assert_eq!(
            l.quick,
            vec![
                Rect::new(2, 27, 23, 23),
                Rect::new(25, 27, 23, 23),
                Rect::new(2, 50, 23, 23),
            ]
        );
        assert_eq!(l.drop_slot(10, 30), 0, "left of the first cell's middle");
        assert_eq!(l.drop_slot(20, 30), 1);
        assert_eq!(l.drop_slot(40, 30), 2, "past the second");
        assert_eq!(
            l.drop_slot(5, 55),
            2,
            "the next row, before its first cell's middle"
        );
        assert_eq!(l.drop_slot(30, 60), 3);
        assert_eq!(l.drop_slot(5, 300), 3, "under the band");
        assert!(l.on_quick_band(30, 60) && !l.on_quick_band(30, l.tasks_x()));
    }

    #[test]
    fn a_side_glass_bar_keeps_its_orb_inside_the_column() {
        let theme = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let small_m = Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let small = crate::glass::small_bar_style(&Default::default()).taskbar;
        let full = theme.taskbar_shape();
        let lay = |m: &Metrics, s: &TaskbarShape, edge| {
            let span = BarSpan {
                edge,
                length: 600,
                thickness: s.side_default(m),
            };
            TaskbarLayout::compute_on(
                m,
                s,
                span,
                true,
                counts(0, 1, 0),
                ClockText::default(),
                Bands::default(),
            )
        };
        for edge in [Edge::Left, Edge::Right] {
            let l = lay(&theme.metrics, &full, edge);
            assert_eq!(
                l.start,
                Rect::new(3, 0, 54, 40),
                "{edge:?}: the orb centred in 60 px"
            );
            assert_eq!(
                (l.start_above, l.start_disc, l.disc_past_bar()),
                (None, None, None)
            );
            let s = lay(&small_m, &small, edge);
            assert_eq!(
                s.start,
                Rect::new(0, 0, 44, 35),
                "{edge:?}: the small orb's 5 px rise inside its 44 px column"
            );
            assert_eq!(s.start_disc, Some(Rect::new(4, 0, 36, 36)));
            assert_eq!(
                s.disc_past_bar(),
                None,
                "{edge:?}: the disc stays inside the column"
            );
            assert_eq!(s.start_orb(true), None, "{edge:?}: no buffer of its own");
            assert_eq!(s.hit(22, 2), TaskbarHit::Start);
            assert_eq!(s.hit(22, -1), TaskbarHit::Nothing);
        }
    }

    #[test]
    fn every_edges_rects_take_their_own_hits() {
        let c = TaskbarCounts {
            quick: 2,
            pager: 2,
            tasks: 2,
            tray: 2,
            show_desktop: true,
            chevron: true,
            ..TaskbarCounts::default()
        };
        for edge in Edge::ALL {
            let l = if edge.is_side() {
                classic_column(edge, 60, c, false)
            } else {
                classic_full(edge)
            };
            let mut want = vec![(l.start, TaskbarHit::Start), (l.clock, TaskbarHit::Clock)];
            want.extend(
                l.grips
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (*r, TaskbarHit::Grip(i))),
            );
            want.extend(
                l.quick
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (*r, TaskbarHit::Quick(i))),
            );
            want.extend(
                l.pager
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (*r, TaskbarHit::Pager(i))),
            );
            want.extend(
                l.tasks
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (*r, TaskbarHit::Task(i))),
            );
            want.extend(
                l.tray
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (*r, TaskbarHit::Tray(i))),
            );
            want.extend(l.chevron.map(|r| (r, TaskbarHit::Chevron)));
            want.extend(l.show_desktop.map(|r| (r, TaskbarHit::ShowDesktop)));
            for (r, hit) in want {
                let corners = [
                    (r.x, r.y),
                    (r.x + r.width / 2, r.y + r.height / 2),
                    (r.right() - 1, r.bottom() - 1),
                ];
                for (x, y) in corners {
                    assert_eq!(l.hit(x, y), hit, "{edge:?}: ({x}, {y}) in {r:?}");
                }
            }
            assert_eq!(l.hit(-1, 5), TaskbarHit::Nothing, "{edge:?}: off the bar");
        }
    }

    #[test]
    fn past_what_fits_a_column_shrinks_its_buttons_then_hides_the_rest_behind_more() {
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let (m, shape) = (&glass.metrics, glass.taskbar_shape());
        let lay = |n: usize| {
            let span = BarSpan {
                edge: Edge::Left,
                length: 400,
                thickness: 60,
            };
            TaskbarLayout::compute_on(
                m,
                &shape,
                span,
                true,
                counts(0, n, 0),
                ClockText::default(),
                Bands::default(),
            )
        };
        let seven = lay(7);
        assert_eq!((seven.tasks_x(), seven.tasks_limit()), (45, 352));
        assert_eq!(seven.tasks.len(), 7);
        assert!(seven.tasks.iter().all(|r| r.width == 60 && r.height == 40));
        assert_eq!(seven.tasks[1].y, 87);
        assert_eq!(seven.more, None);
        let eight = lay(8);
        assert_eq!(eight.more, Some(Rect::new(0, 314, 60, 38)));
        assert_eq!((eight.tasks.len(), eight.stacks.len()), (6, 6));
        assert!(eight.tasks.iter().all(|r| r.height == 38));
        assert_eq!(eight.tasks[5], Rect::new(0, 245, 60, 38));
        assert_eq!(eight.hit(30, 330), TaskbarHit::More);
        assert_eq!(eight.scaled(2).more, Some(Rect::new(0, 628, 120, 76)));
        assert_eq!(
            eight.least_button(62),
            40,
            "a full button decides grouping on a side bar"
        );
        assert_eq!(
            classic_row(1).least_button(62),
            62,
            "the row's own least on a bottom bar"
        );
        assert_eq!(
            eight.icon_step(&shape),
            (40, 42),
            "an icon button is a full row"
        );
    }

    #[test]
    fn a_column_too_short_for_its_tray_lays_no_buttons_and_no_more() {
        let span = BarSpan {
            edge: Edge::Left,
            length: 120,
            thickness: 60,
        };
        let c = TaskbarCounts {
            tray: 12,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute_on(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            span,
            true,
            c,
            ClockText::default(),
            Bands::default(),
        );
        assert!(
            l.tasks_limit() < l.tasks_x(),
            "the tray reaches past the bands"
        );
        assert_eq!((l.tasks.len(), l.more), (0, None));
        let mut l = l;
        l.lay_tasks(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            &[1, 1],
            TaskRow::Shared,
        );
        assert_eq!((l.tasks.len(), l.more), (0, None), "no room for a » either");
    }

    #[test]
    fn a_column_too_short_for_its_clock_has_an_empty_well() {
        let c = TaskbarCounts {
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let l = look_column(
            crate::interface::Interface::Glass,
            Edge::Left,
            (60, 50),
            c,
            ClockText::default(),
            Bands::default(),
        );
        assert!(
            l.clock.y > l.show_desktop.unwrap().y,
            "the clock pushed past the strip"
        );
        assert_eq!(
            l.notification,
            Rect::new(0, l.clock.y, 60, 0),
            "no rows, not a negative height"
        );
    }

    #[test]
    fn the_more_cell_is_two_centred_right_chevrons() {
        let mut c = Canvas::new(60, 22);
        let ink = Color::rgb(255, 255, 255);
        draw_more(&mut c, Rect::new(0, 0, 60, 22), ink, 1);
        let col = |x: i32| {
            (0..22)
                .filter(|&y| c.get(x, y) == ink.argb())
                .collect::<Vec<_>>()
        };
        assert_eq!(col(25), (7..14).collect::<Vec<_>>());
        assert_eq!(col(28), vec![10]);
        assert!(
            col(29).is_empty() && col(30).is_empty(),
            "2 px between them"
        );
        assert_eq!(col(31), (7..14).collect::<Vec<_>>());
        assert_eq!(col(34), vec![10]);
        assert!(col(35).is_empty());
    }

    fn left_side_bar() -> TaskbarLayout {
        let span = BarSpan {
            edge: Edge::Left,
            length: 600,
            thickness: 60,
        };
        TaskbarLayout::compute_on(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            span,
            true,
            TaskbarCounts::default(),
            ClockText::default(),
            Bands::default(),
        )
    }

    #[test]
    #[should_panic(expected = "compute_on lays a side bar out")]
    fn turning_a_side_bar_to_the_other_side_panics() {
        left_side_bar().on_edge(Edge::Right);
    }

    #[test]
    #[should_panic(expected = "compute_on lays a side bar out")]
    fn turning_a_side_bar_to_the_bottom_panics() {
        left_side_bar().on_edge(Edge::Bottom);
    }

    #[test]
    fn beside_a_right_bar_a_start_menu_on_its_edge_casts_no_shadow() {
        assert_eq!(
            StartMenuDock::OnTop.shadow_floor(Edge::Right),
            Some(0),
            "its shadow would fall on the bar"
        );
        assert_eq!(
            StartMenuDock::OnTop.shadow_floor(Edge::Left),
            None,
            "it falls away from a left bar"
        );
        assert_eq!(
            StartMenuDock::Into(4).shadow_floor(Edge::Right),
            None,
            "a menu into the bar casts its shadow on it"
        );
    }

    #[test]
    fn a_side_bars_toolbars_are_vertical_and_its_more_cell_opens_a_menu() {
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let shape = glass.taskbar_shape();
        let span = BarSpan {
            edge: Edge::Left,
            length: 400,
            thickness: 60,
        };
        let c = TaskbarCounts {
            quick: 2,
            tasks: 9,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute_on(
            &glass.metrics,
            &shape,
            span,
            true,
            c,
            ClockText::default(),
            Bands::default(),
        );
        assert!(l.more.is_some(), "nine 40 px buttons overrun the column");
        let names = TaskbarNames {
            quick: vec!["A".into(), "B".into()],
            tasks: (0..9).map(|i| (format!("T{i}"), false)).collect(),
            more_open: true,
            ..TaskbarNames::default()
        };
        let mut t = Tree::window("Taskbar", (l.width, l.height));
        access(&l, &names, &mut t, access::ROOT);
        let quick = t.find(Role::Toolbar, "Quick Launch").unwrap();
        assert_eq!(
            t.get(quick).unwrap().orientation(),
            Some(Orientation::Vertical)
        );
        let tasks = t.find(Role::Toolbar, "Running Applications").unwrap();
        assert_eq!(
            t.get(tasks).unwrap().orientation(),
            Some(Orientation::Vertical)
        );
        let more = t.find(Role::Button, "Show more windows").unwrap();
        assert_eq!(t.get(more).unwrap().has_popup(), Some(HasPopup::Menu));
        assert_eq!(
            t.get(more).unwrap().is_expanded(),
            Some(true),
            "its menu is open"
        );
        let row = classic_full(Edge::Bottom);
        let mut b = Tree::window("Taskbar", (row.width, row.height));
        let names = TaskbarNames {
            quick: vec!["A".into(), "B".into()],
            ..TaskbarNames::default()
        };
        access(&row, &names, &mut b, access::ROOT);
        let q = b.find(Role::Toolbar, "Quick Launch").unwrap();
        assert_eq!(
            b.get(q).unwrap().orientation(),
            None,
            "a bottom bar's toolbars stay as they were"
        );
        assert!(
            b.find(Role::Button, "Show more windows").is_none(),
            "a bottom bar has no » cell"
        );
    }

    #[test]
    fn an_unlocked_side_bar_takes_width_drags_on_its_inner_edge_over_its_buttons() {
        let c = TaskbarCounts {
            quick: 2,
            pager: 0,
            tasks: 3,
            tray: 2,
            show_desktop: true,
            chevron: false,
            ..TaskbarCounts::default()
        };
        let left = classic_column(Edge::Left, 60, c, false);
        assert_eq!(
            left.resize,
            Some(Rect::new(57, 0, RESIZE_W, 600)),
            "down the right edge"
        );
        let right = classic_column(Edge::Right, 60, c, false);
        assert_eq!(
            right.resize,
            Some(Rect::new(0, 0, RESIZE_W, 600)),
            "down the left edge"
        );
        let t = left.tasks[0];
        assert_eq!(left.hit(59, t.y + 2), TaskbarHit::Resize);
        assert_eq!(left.hit(t.x + 1, t.y + 2), TaskbarHit::Task(0));
        assert_eq!(right.hit(0, right.tasks[0].y + 2), TaskbarHit::Resize);
        assert_eq!(left.scaled(2).resize, Some(Rect::new(114, 0, 6, 1200)));
        // Glass's strip spans the whole column: the edge wins over it.
        let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
        let (m, shape) = (&glass.metrics, glass.taskbar_shape());
        let span = BarSpan {
            edge: Edge::Left,
            length: 600,
            thickness: 60,
        };
        let clock = ClockText { time: 30, date: 0 };
        let g = TaskbarLayout::compute_on(m, &shape, span, false, c, clock, Bands::default());
        let sd = g.show_desktop.expect("a strip");
        assert_eq!(sd.right(), 60, "Glass's strip runs to the inner edge");
        assert_eq!(g.hit(59, sd.y + 2), TaskbarHit::Resize);
        assert_eq!(g.hit(56, sd.y + 2), TaskbarHit::ShowDesktop);
        let locked = classic_column(Edge::Left, 60, c, true);
        assert_eq!(locked.resize, None);
        assert_ne!(locked.hit(59, t.y + 2), TaskbarHit::Resize);
        for edge in [Edge::Bottom, Edge::Top] {
            let row = TaskbarLayout::compute(
                &Metrics::default(),
                &TaskbarShape::CLASSIC,
                800,
                false,
                c,
                30,
            )
            .on_edge(edge);
            assert_eq!(row.resize, None, "{edge:?}: no height drag");
        }
    }

    #[test]
    fn a_turn_between_a_row_and_a_column_drops_the_first_bands_width() {
        let b = Bands {
            tasks_first: true,
            first_width: Some(300),
        };
        assert_eq!(
            b.turned(Edge::Bottom, Edge::Top),
            b,
            "the same axis keeps it"
        );
        assert_eq!(b.turned(Edge::Left, Edge::Right), b);
        assert_eq!(b.turned(Edge::Left, Edge::Left), b);
        let natural = Bands {
            tasks_first: true,
            first_width: None,
        };
        assert_eq!(b.turned(Edge::Bottom, Edge::Left), natural);
        assert_eq!(b.turned(Edge::Right, Edge::Top), natural);
    }

    #[test]
    fn a_column_too_short_for_its_tray_keeps_the_arrow_and_cells_off_the_clock() {
        use crate::interface::Interface;
        for interface in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for len in (40..240).step_by(4) {
                for (tray, hidden, reveal) in [(12, 3, 0), (12, 3, 1), (2, 3, 1), (0, 3, 1)] {
                    let c = arrow_counts(tray, hidden, reveal);
                    let l = look_column(
                        interface,
                        Edge::Left,
                        (60, len),
                        c,
                        ClockText::default(),
                        Bands::default(),
                    );
                    for r in l.tray.iter().chain(&l.chevron) {
                        assert!(
                            r.bottom() <= l.clock.y - 6,
                            "{interface:?} {len} px long, {tray} shown, {hidden} hidden, \
                             reveal {reveal}: {r:?} ends 6 px or more above the clock {:?}",
                            l.clock
                        );
                    }
                }
            }
        }
        let l = look_column(
            Interface::Classic,
            Edge::Left,
            (60, 60),
            arrow_counts(2, 3, 0),
            ClockText::default(),
            Bands::default(),
        );
        assert_eq!(l.chevron, None, "no room for the arrow: it is left out");
    }

    #[test]
    fn an_open_run_never_pushes_the_shown_cells_off_a_row() {
        let m = Metrics::default();
        let s = TaskbarShape::CLASSIC;
        let lay =
            |reveal| TaskbarLayout::compute(&m, &s, 400, true, arrow_counts(4, 30, reveal), 30);
        let l = lay(reveal_width(&s, 30));
        let shut = lay(0);
        assert_eq!(
            l.tray.len() - l.tray_hidden,
            4,
            "every shown cell stays on the bar"
        );
        assert_eq!(
            l.tray[l.tray_hidden..],
            shut.tray[..],
            "the shown cells stay put"
        );
        let arrow = l.chevron.unwrap();
        assert!(
            arrow.x >= l.band_x,
            "the arrow {arrow:?} stops at the bands' start {}",
            l.band_x
        );
        assert!(
            arrow.x < l.band_x + 20,
            "the run fills the room up to the bands' start: {arrow:?}"
        );
        assert!(l.tray_hidden < 30);
    }

    #[test]
    fn a_capped_run_opens_fully_at_its_run_width() {
        let m = Metrics::default();
        let s = TaskbarShape::CLASSIC;
        let lay =
            |reveal| TaskbarLayout::compute(&m, &s, 400, true, arrow_counts(4, 30, reveal), 30);
        let full = lay(reveal_width(&s, 30));
        let run = lay(0).run_width;
        assert!(run > 0 && run < reveal_width(&s, 30), "capped: {run}");
        assert_eq!(full.run_width, run, "the cap does not move with the run");
        assert_eq!(run, reveal_width(&s, full.tray_hidden));
        let at = lay(run);
        assert_eq!((at.chevron, &at.tray), (full.chevron, &full.tray));
        assert_eq!(lay(run - 1).tray_hidden, full.tray_hidden - 1);
    }

    fn tools_and_arrow(hidden: usize, reveal: i32) -> TaskbarCounts {
        TaskbarCounts {
            quick: 3,
            pager: 4,
            tasks: 2,
            ..arrow_counts(4, hidden, reveal)
        }
    }

    /// The tools band's rects end before `arrow` on the bar's long axis.
    fn clear_of_tools(l: &TaskbarLayout, arrow: Rect, side: bool) -> bool {
        (l.quick.iter().chain(&l.pager)).all(|r| {
            if side {
                r.bottom() <= arrow.y
            } else {
                r.right() <= arrow.x
            }
        })
    }

    #[test]
    fn a_capped_run_leaves_its_arrow_clear_of_the_tools_band() {
        let m = Metrics::default();
        let s = TaskbarShape::CLASSIC;
        for locked in [true, false] {
            for tasks_first in [false, true] {
                let bands = Bands {
                    tasks_first,
                    first_width: None,
                };
                let lay = |reveal| {
                    let span = BarSpan {
                        edge: Edge::Bottom,
                        length: 800,
                        thickness: m.taskbar_height,
                    };
                    let clock = ClockText { time: 30, date: 30 };
                    TaskbarLayout::compute_on(
                        &m,
                        &s,
                        span,
                        locked,
                        tools_and_arrow(60, reveal),
                        clock,
                        bands,
                    )
                };
                let open = lay(reveal_width(&s, 60));
                let arrow = open.chevron.unwrap();
                assert!(open.tray_hidden > 0, "the run still opens");
                assert!(
                    clear_of_tools(&open, arrow, false),
                    "locked {locked}, tasks first {tasks_first}: arrow {arrow:?} under {:?} {:?}",
                    open.quick,
                    open.pager
                );
                assert_eq!(open.run_width, reveal_width(&s, open.tray_hidden));
            }
        }
    }

    #[test]
    fn a_capped_column_run_leaves_its_arrow_clear_of_the_tools_band() {
        let open = classic_column(Edge::Left, 60, tools_and_arrow(60, 1), true);
        let arrow = open.chevron.unwrap();
        assert!(open.tray_hidden > 0, "the run still opens");
        assert!(
            clear_of_tools(&open, arrow, true),
            "arrow {arrow:?} under {:?} {:?}",
            open.quick,
            open.pager
        );
    }

    #[test]
    fn a_shut_columns_run_width_is_what_its_open_run_shows() {
        let shut = classic_column(Edge::Left, 60, arrow_counts(4, 60, 0), true);
        let open = classic_column(Edge::Left, 60, arrow_counts(4, 60, 1), true);
        assert_eq!(shut.tray_hidden, 0);
        assert_eq!(
            shut.run_width,
            reveal_width(&TaskbarShape::CLASSIC, open.tray_hidden)
        );
        assert!(open.tray_hidden > 0 && open.tray_hidden < 60);
    }

    #[test]
    fn an_open_run_never_pushes_the_shown_cells_off_a_column() {
        let l = classic_column(Edge::Left, 60, arrow_counts(4, 60, 1), true);
        let shut = classic_column(Edge::Left, 60, arrow_counts(4, 60, 0), true);
        assert!(l.tray_hidden > 0 && l.tray_hidden < 60);
        assert_eq!(
            l.tray.len() - l.tray_hidden,
            4,
            "every shown cell stays on the bar"
        );
        let last = |l: &TaskbarLayout| *l.tray.last().unwrap();
        assert!(last(&l).bottom() <= l.clock.y - 6);
        assert_eq!(last(&l).y, last(&shut).y, "the grid still ends at the foot");
    }

    #[test]
    fn revealed_counts_the_whole_cells_inside_the_run() {
        let s = TaskbarShape::CLASSIC;
        assert_eq!(revealed(&s, 3, -5), 0, "a negative run shows none");
        assert_eq!(revealed(&s, 3, 19), 0, "a cell shows once it is whole");
        assert_eq!(revealed(&s, 3, 20), 1);
        assert_eq!(revealed(&s, 3, 59), 2);
        assert_eq!(revealed(&s, 3, 60), 3);
        assert_eq!(revealed(&s, 3, 500), 3, "never more than are hidden");
        assert_eq!(revealed(&s, 0, 60), 0);
    }
}
