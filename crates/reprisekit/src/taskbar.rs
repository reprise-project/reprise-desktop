//! Taskbar model, layout, hit test and access part; each interface paints it in its own module.

use crate::icon::ICON_SIZE;
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, Metrics};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

mod access;
mod attention;
mod badge;
mod draw;
mod hit;
mod layout;

pub use access::{ATTENTION, QUICK_MORE_NAME, TaskbarNames, access};
pub use attention::{Attention, FLASH_HALF};
pub use badge::BadgeArt;
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

/// A look's art for a task button's unread badge: the badge alone around `text` at `d` device px
/// per design px, set in the button's top-right corner ([`badge_at`]).
pub type BadgePainter = Arc<dyn Fn(&str, &crate::theme::ColorScheme, i32) -> Canvas + Send + Sync>;

/// From a task button's top and right edges to its badge, in design px.
pub const BADGE_INSET: i32 = 3;
/// The space a badge keeps from the label before it, in design px.
const BADGE_GAP: i32 = 2;

/// The text a badge shows for `count`: the number, `99+` above 99.
pub fn badge_text(count: u32) -> String {
    if count > 99 {
        "99+".into()
    } else {
        count.to_string()
    }
}

/// Where a badge `badge_w` device px wide goes on `button` (device px) at `d`.
pub fn badge_at(button: Rect, badge_w: i32, d: i32) -> (i32, i32) {
    (
        button.right() - BADGE_INSET * d - badge_w,
        button.y + BADGE_INSET * d,
    )
}

/// `label` cut short of a badge at `badge_x` (device px) at `d`.
pub fn label_before_badge(label: Rect, badge_x: i32, d: i32) -> Rect {
    let right = label.right().min(badge_x - BADGE_GAP * d);
    Rect::new(label.x, label.y, (right - label.x).max(0), label.height)
}

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
    /// Per task button, its program's unread count; empty or None draws no badge.
    pub task_badge: Vec<Option<u32>>,
    pub show_desktop_hot: bool,
    pub more: PopupCell,
    pub chevron: PopupCell,
    pub tasks: Vec<TaskEntry>,
    pub tray_icons: Vec<Option<Arc<Canvas>>>,
    pub clock: String,
    /// The network meter's download and upload readings; None draws the meter empty.
    pub meter: Option<[String; 2]>,
}

/// What of a bar drawn for one [`TaskbarState`] a draw for another on the same layout changes:
/// Start's part, and a box round the other cells that change (the whole bar for a change no cell
/// bounds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StateDamage {
    pub start: bool,
    pub cells: Option<Rect>,
}

impl TaskbarState {
    /// Which cells a bar drawn for `self` on `from` redraws to show `other` on `to`. A task
    /// button's change takes its neighbours, whose label edges meet it, and a task row laid out
    /// again takes the buttons and » cell it moves. Any other layout change takes the whole bar
    /// and Start. Icons count as equal only when shared.
    pub fn damage(&self, other: &Self, from: &TaskbarLayout, to: &TaskbarLayout) -> StateDamage {
        fn icon(a: &Option<Arc<Canvas>>, b: &Option<Arc<Canvas>>) -> bool {
            match (a, b) {
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                (None, None) => true,
                _ => false,
            }
        }
        fn moved(a: Option<usize>, b: Option<usize>) -> [Option<usize>; 2] {
            if a == b { [None, None] } else { [a, b] }
        }
        let Self {
            start_pressed,
            start_focus,
            start_hot,
            start_icon,
            orb_glow,
            focus,
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
            task_badge,
            show_desktop_hot,
            more,
            chevron,
            tasks,
            tray_icons,
            clock,
            meter,
        } = self;
        let start = *start_pressed != other.start_pressed
            || *start_focus != other.start_focus
            || *start_hot != other.start_hot
            || !icon(start_icon, &other.start_icon)
            || *orb_glow != other.orb_glow;
        let whole_bar = Some(Rect::new(0, 0, to.width, to.height));
        if from.but_task_row() != to.but_task_row() {
            return StateDamage {
                start: true,
                cells: whole_bar,
            };
        }
        let layout = to;
        let whole = *focus != other.focus
            || *quick_drop != other.quick_drop
            || *pager_mark != other.pager_mark
            || *task_shift != other.task_shift
            || *task_float != other.task_float
            || quick_icons.len() != other.quick_icons.len()
            || pager.len() != other.pager.len()
            || tray_icons.len() != other.tray_icons.len();
        if whole {
            return StateDamage {
                start,
                cells: whole_bar,
            };
        }
        let mut cells: Option<Rect> = None;
        let mut add = |r: Option<&Rect>| {
            if let Some(r) = r {
                cells = Some(cells.map_or(*r, |c| c.union(r)));
            }
        };
        let quick = (0..quick_icons.len())
            .filter(|&i| !icon(&quick_icons[i], &other.quick_icons[i]))
            .chain(moved(*quick_hot, other.quick_hot).into_iter().flatten())
            .chain(
                moved(*quick_pressed, other.quick_pressed)
                    .into_iter()
                    .flatten(),
            );
        for i in quick {
            add(layout.quick.get(i));
        }
        let pagers = (0..pager.len())
            .filter(|&i| pager[i] != other.pager[i])
            .chain(moved(*pager_hot, other.pager_hot).into_iter().flatten());
        for i in pagers {
            add(layout.pager.get(i));
        }
        let same_task = |a: &TaskEntry, b: &TaskEntry| {
            a.title == b.title
                && a.active == b.active
                && a.attention == b.attention
                && a.glow == b.glow
                && icon(&a.icon, &b.icon)
        };
        let lit = if *task_hot_x == other.task_hot_x {
            [None; 4]
        } else {
            [*task_hot, other.task_hot, *task_pressed, other.task_pressed]
        };
        let changed = (0..tasks.len().max(other.tasks.len()))
            .filter(|&i| {
                let same = match (tasks.get(i), other.tasks.get(i)) {
                    (Some(a), Some(b)) => same_task(a, b),
                    _ => false,
                };
                !same
                    || task_progress.get(i) != other.task_progress.get(i)
                    || task_badge.get(i) != other.task_badge.get(i)
                    || from.tasks.get(i) != to.tasks.get(i)
                    || from.stacks.get(i) != to.stacks.get(i)
            })
            .chain(moved(*task_hot, other.task_hot).into_iter().flatten())
            .chain(
                moved(*task_pressed, other.task_pressed)
                    .into_iter()
                    .flatten(),
            )
            .chain(lit.into_iter().flatten());
        for i in changed {
            for k in i.saturating_sub(1)..=i + 1 {
                add(from.tasks.get(k));
                add(to.tasks.get(k));
            }
        }
        if from.more != to.more {
            add(from.more.as_ref());
            add(to.more.as_ref());
        }
        for i in (0..tray_icons.len()).filter(|&i| !icon(&tray_icons[i], &other.tray_icons[i])) {
            add(layout.tray.get(i));
        }
        let cell = |changed: bool, r: Option<Rect>| r.filter(|_| changed);
        let rest = [
            cell(*quick_more != other.quick_more, layout.quick_more),
            cell(
                *show_desktop_hot != other.show_desktop_hot,
                layout.show_desktop,
            ),
            cell(*more != other.more, layout.more),
            cell(*chevron != other.chevron, layout.chevron),
            cell(*clock != other.clock, Some(layout.clock)),
            cell(*meter != other.meter, layout.meter),
        ];
        for r in rest.iter().flatten() {
            add(Some(r));
        }
        StateDamage { start, cells }
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
    Meter,
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
    /// The network meter's text block ([`crate::netmeter::size`]); None: no meter.
    pub meter: Option<(i32, i32)>,
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
    /// The network meter between the tray and the clock.
    pub meter: Option<Rect>,
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
