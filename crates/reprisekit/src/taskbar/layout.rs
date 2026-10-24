use super::*;
use crate::icon::ICON_SIZE;
use crate::raster::Rect;
use crate::theme::Metrics;

/// Quick Launch button width: 16 px icon, 3 px padding, 1 px frame each side. A pager button is as wide.
const QUICK_W: i32 = 23;
/// Width of the » cell ending a Quick Launch band too short for its buttons on a Bottom or Top bar; down a side bar it is a `QUICK_W` cell.
const QUICK_MORE_W: i32 = 14;
const PAGER_GAP: i32 = 4;
const SIDE_TRAY_MARGIN: i32 = 2;

/// Grip handle: 3 px wide, 18 px tall, 4 px before its band.
pub(super) const GRIP_W: i32 = 3;
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
