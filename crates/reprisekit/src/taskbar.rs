//! Taskbar model, layout, hit test and access part; each interface paints it in its own module.

use crate::icon::ICON_SIZE;
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, Metrics};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod access;
mod attention;
mod draw;
mod hit;
mod layout;

pub use access::{ATTENTION, QUICK_MORE_NAME, TaskbarNames, access};
pub use attention::{Attention, FLASH_HALF};
pub use draw::{
    draw_chevron, draw_clock_lines, draw_more, draw_tray_arrow, draw_tray_arrow_side,
    quick_icon_origin,
};
pub use hit::slot_at;
pub use layout::{RESIZE_W, TASK_GAP, reveal_width, revealed};

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
    pub attention: Attention,
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

impl TaskbarState {
    /// Whether `self` and `other` differ at most in Start's own fields (`start_pressed`,
    /// `start_focus`, `start_hot`), so a bar drawn for one needs only its Start part redrawn
    /// (`look::redraw_taskbar_start`) for the other. Icons count as equal only when shared.
    pub fn same_but_start(&self, other: &Self) -> bool {
        fn icon(a: &Option<Arc<Canvas>>, b: &Option<Arc<Canvas>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        fn icons(a: &[Option<Arc<Canvas>>], b: &[Option<Arc<Canvas>>]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| icon(a, b))
        }
        let Self {
            start_pressed: _,
            start_focus: _,
            start_hot: _,
            focus,
            start_icon,
            orb_glow,
            quick_icons,
            quick_hot,
            quick_pressed,
            quick_drop,
            quick_more,
            pager,
            pager_hot,
            pager_mark,
            task_hot,
            task_pressed,
            task_hot_x,
            task_shift,
            task_float,
            task_progress,
            show_desktop_hot,
            more,
            chevron,
            tasks,
            tray_icons,
            clock,
        } = self;
        let same_task = |a: &TaskEntry, b: &TaskEntry| {
            a.title == b.title
                && a.active == b.active
                && a.attention == b.attention
                && a.glow == b.glow
                && icon(&a.icon, &b.icon)
        };
        *focus == other.focus
            && icon(start_icon, &other.start_icon)
            && *orb_glow == other.orb_glow
            && icons(quick_icons, &other.quick_icons)
            && *quick_hot == other.quick_hot
            && *quick_pressed == other.quick_pressed
            && *quick_drop == other.quick_drop
            && *quick_more == other.quick_more
            && *pager == other.pager
            && *pager_hot == other.pager_hot
            && *pager_mark == other.pager_mark
            && *task_hot == other.task_hot
            && *task_pressed == other.task_pressed
            && *task_hot_x == other.task_hot_x
            && *task_shift == other.task_shift
            && *task_float == other.task_float
            && *task_progress == other.task_progress
            && *show_desktop_hot == other.show_desktop_hot
            && *more == other.more
            && *chevron == other.chevron
            && tasks.len() == other.tasks.len()
            && tasks.iter().zip(&other.tasks).all(|(a, b)| same_task(a, b))
            && icons(tray_icons, &other.tray_icons)
            && *clock == other.clock
    }
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

#[cfg(test)]
mod tests;
