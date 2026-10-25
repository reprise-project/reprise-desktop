//! The taskbar drawn through a [`Style`].

use crate::icon;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State, Style};
use crate::taskbar::{PagerMark, PopupCell, TaskbarLayout, TaskbarState, quick_icon_origin};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

mod buttons;
mod start;

pub use buttons::{
    EDGE_LEFT, EDGE_NEXT_LEFT, EDGE_PREV_RIGHT, EDGE_RIGHT, EDGE_ROW_END, EDGE_ROW_START,
    task_draws_spot,
};
use buttons::{
    PAGER_DIM, Span, Spot, draw_dimmed_number, draw_labelled, draw_slot_edges, draw_slot_light,
    draw_slot_light_foot, draw_spot, draw_task_button, is_lit, label_pair, lights_slots,
    pager_tile_rect, stacked,
};
pub use start::draw_start_above;
use start::{draw_orb, draw_start_pill};
pub(crate) use start::{keeps_orb_bloom, orb_bloom};

/// Atlas row of [`Part::Taskbar`] an unlocked bar's grip hairline takes its colour from.
const GRIP_ROW: i32 = 27;

/// Draws the bar into `canvas` (sized to `layout`) through `style`.
pub fn draw_taskbar(
    canvas: &mut Canvas,
    style: &Style,
    theme: &Theme,
    text: &TextRenderer,
    start_text: &TextRenderer,
    layout: &TaskbarLayout,
    state: &TaskbarState,
) {
    let s = theme.scale.max(1);
    // `theme` is already at scale `s`, so its font size is in device pixels.
    let size = theme.fonts.size_px;
    let icon_px = theme.icon_size();

    let labelled = !layout.icons_only && style.part(Part::TaskLabel).is_some();
    let full = Rect::new(0, 0, layout.width, layout.height);
    layout.edge.draw_art(canvas, full, |canvas, along| {
        style.draw(canvas, along, Part::Taskbar, State::Normal, s);
        if labelled {
            let (x0, x1) = (layout.tasks_x() - 5 * s, layout.tasks_limit() + 7 * s);
            let row = Rect::new(x0, 0, x1 - x0, along.height);
            style.draw(canvas, row, Part::LabelRow, State::Normal, s);
        }
        let well = layout.to_along(layout.notification);
        style.draw(canvas, well, Part::TrayArea, State::Normal, s);
    });

    let st = layout.start;
    if style.taskbar.orb {
        if layout.start_orb(state.start_pressed) == layout.start_above {
            draw_orb(canvas, style, layout, state, 0, 0, s);
        }
    } else {
        style.draw(canvas, st, Part::StartButton, start_state(state), s);
        draw_start_pill(canvas, style, start_text, st, state, s);
    }
    if let Some(bar) = style.part(Part::Taskbar) {
        let f = bar.frame(State::Normal);
        let row = |y: i32| bar.atlas.get(f.x, f.y + y * bar.scale);
        // A translucent bar is one colour from row 2 down, so its grips take the bar's dark and
        // light edge rows instead.
        let inks = if style.glass.is_some() {
            vec![row(0), row(1)]
        } else {
            vec![row(GRIP_ROW)]
        };
        for g in &layout.grips {
            for (k, &ink) in (0..).zip(&inks) {
                let line = if layout.edge.is_side() {
                    Rect::new(g.x, g.y + k * s, g.width, s)
                } else {
                    Rect::new(g.x + k * s, g.y, s, g.height)
                };
                for y in line.y..line.bottom() {
                    for x in line.x..line.right() {
                        canvas.put_raw(x, y, ink);
                    }
                }
            }
        }
    }

    let popup_ink = |lit: Option<State>| {
        lit.and_then(|st| style.text_color(Part::TaskButton, st))
            .unwrap_or(style.bar_text)
    };
    let mut fallback: Option<Canvas> = None;
    for (i, r) in layout.quick.iter().enumerate() {
        if state.quick_pressed == Some(i) {
            style.draw(canvas, *r, Part::TaskButton, State::Checked, s);
        } else if state.quick_hot == Some(i) {
            style.draw(canvas, *r, Part::TaskButton, State::Hot, s);
        }
        let at = quick_icon_origin(*r, icon_px, s);
        let ic = state.quick_icons.get(i).and_then(Option::as_deref);
        blit_icon(canvas, theme, ic, &mut fallback, icon_px, at);
    }
    if let Some(r) = layout.quick_more {
        let lit = popup_state(state.quick_more);
        if let Some(st) = lit {
            style.draw(canvas, r, Part::TaskButton, st, s);
        }
        crate::taskbar::draw_more(canvas, r, popup_ink(lit), s);
    }
    let lights = lights_slots(style);
    for (i, (r, b)) in layout.pager.iter().zip(&state.pager).enumerate() {
        let hot = state.pager_hot == Some(i);
        let label = (i + 1).to_string();
        let (ink, alpha) = match state.pager_mark {
            PagerMark::Light if lights => {
                if b.current {
                    draw_slot_light_foot(canvas, style, *r, State::Hot, s, layout.edge);
                }
                if hot {
                    draw_slot_edges(canvas, style, *r, State::Hot, s, layout.edge);
                }
                let alpha = if b.current { 0xFF } else { PAGER_DIM };
                (style.bar_text, alpha)
            }
            PagerMark::Tile if style.part(Part::PagerTile).is_some() => {
                let tile = match (b.current, hot) {
                    (true, true) => Some(State::CheckedHot),
                    (true, false) => Some(State::Checked),
                    (false, true) => Some(State::Hot),
                    (false, false) => None,
                };
                if let Some(st) = tile {
                    style.draw(canvas, pager_tile_rect(*r, s), Part::PagerTile, st, s);
                }
                (style.bar_text, 0xFF)
            }
            _ => {
                let lit = if b.current {
                    Some(State::Checked)
                } else if hot {
                    Some(State::Hot)
                } else {
                    None
                };
                if let Some(st) = lit {
                    style.draw(canvas, *r, Part::TaskButton, st, s);
                }
                (popup_ink(lit), 0xFF)
            }
        };
        if alpha == 0xFF {
            text.draw(canvas, *r, &label, false, ink, Align::Center, size);
        } else {
            draw_dimmed_number(canvas, text, *r, &label, ink, alpha, size);
        }
    }

    let task_px = style.taskbar.task_icon * s;
    let mut task_fallback: Option<Canvas> = None;
    let count = layout.tasks.len().min(state.tasks.len());
    let float = state.task_float.filter(|&(i, _)| i < count);
    let floated = |i: usize| float.is_some_and(|(f, _)| f == i);
    let shift = |i: usize| state.task_shift.get(i).copied().unwrap_or(0);
    let states: Vec<State> = (0..count).map(|i| task_state(style, state, i)).collect();
    let fronts: Vec<(Rect, Vec<Rect>)> = (0..count)
        .map(|i| {
            let slot = layout.tasks[i];
            let at = match float {
                Some((f, a)) if f == i => layout.placed_along(slot, a * s),
                _ => layout.moved_along(slot, shift(i) * s),
            };
            stacked(style, at, layout.stacks.get(i).copied(), s)
        })
        .collect();
    let home = |i: usize| shift(i) == 0 && !floated(i);
    let abut = |i: usize| {
        labelled
            && i + 1 < count
            && !layout.pinned_idle(i)
            && !layout.pinned_idle(i + 1)
            && home(i)
            && home(i + 1)
            && fronts[i].1.is_empty()
            && fronts[i].0.right() == fronts[i + 1].0.x
    };
    let pointer = |r: Rect| state.task_hot_x.map(|x| r.x + x * s);
    let order = (0..count)
        .filter(|&i| !floated(i))
        .chain(float.map(|(f, _)| f));
    for i in order {
        let (r, edges) = &fronts[i];
        let task = &state.tasks[i];
        let held = state.task_pressed == Some(i);
        if layout.pinned_idle(i) {
            let bare = if held {
                Some(State::Pressed)
            } else if state.task_hot == Some(i) || floated(i) {
                Some(State::Hot)
            } else {
                None
            };
            if let Some(st) = bare
                && !draw_slot_light(canvas, style, *r, st, s, layout.edge)
            {
                let span = Span {
                    x: r.x,
                    right: r.right(),
                    pointer: pointer(*r),
                };
                draw_task_button(canvas, style, task, Part::TaskButton, *r, st, span, s);
            }
            let o = if held { s } else { 0 };
            let at = (
                r.x + (r.width - task_px) / 2 + o,
                r.y + (r.height - task_px) / 2 + o,
            );
            let ic = task.icon.as_deref();
            blit_icon(canvas, theme, ic, &mut task_fallback, task_px, at);
            continue;
        }
        let button_state = states[i];
        let edges_in = edge_state(style, button_state);
        let pair = if labelled {
            label_pair(style, *r, s)
        } else {
            0
        };
        let span = Span {
            x: r.x + pair,
            right: edges.last().map_or(r.right() - pair, |e| e.right()),
            pointer: pointer(*r),
        };
        let lit_edges = style
            .task_spot()
            .filter(|_| is_lit(button_state))
            .and_then(|t| Some(&t.fit).zip(t.edges.as_ref()))
            .filter(|(_, f)| f.base.states.contains(&edges_in));
        for (k, e) in edges.iter().enumerate() {
            match lit_edges {
                Some((fit, f)) => {
                    let spot = Spot {
                        fit,
                        span,
                        glow: task.glow,
                    };
                    draw_spot(canvas, &spot, f, *e, edges_in, k, s);
                }
                None => style.draw_nth(canvas, *e, Part::TaskStack, edges_in, k, s),
            }
        }
        if labelled {
            let prev = (i > 0 && abut(i - 1)).then(|| states[i - 1]);
            let next = abut(i).then(|| states[i + 1]);
            draw_labelled(canvas, style, task, *r, button_state, prev, next, span, s);
        } else {
            let part = Part::TaskButton;
            draw_task_button(canvas, style, task, part, *r, button_state, span, s);
        }
        if let (Some(paint), Some(pr)) = (
            &style.task_progress,
            state.task_progress.get(i).copied().flatten(),
        ) {
            let art = paint(&pr, &theme.colors, r.width / s, r.height / s, s);
            canvas.blit_over(r.x, r.y, &art);
        }
        let pushed = matches!(
            button_state,
            State::Checked | State::CheckedHot | State::Attention
        ) && style.task_nudge;
        let o = if pushed || held { s } else { 0 };
        let lb = style.taskbar.labelled;
        let narrow = r.width < (lb.icon_x + lb.icon_box) * s;
        let icon_x = if narrow || (layout.icons_only && style.taskbar.icon_button.is_some()) {
            r.x + (r.width - task_px) / 2
        } else {
            r.x + lb.icon_x * s + (lb.icon_box * s - task_px) / 2
        };
        let icon_at = (icon_x + o, r.y + (r.height - task_px) / 2 + o);
        if !(narrow && r.width < task_px + 2 * o) {
            let ic = task.icon.as_deref();
            blit_icon(canvas, theme, ic, &mut task_fallback, task_px, icon_at);
        }
        if layout.icons_only {
            continue;
        }
        let label_x = r.x + lb.text_x * s + o;
        let label = Rect::new(
            label_x,
            r.y + o,
            (r.right() - 4 * s - label_x).max(0),
            r.height,
        );
        let title = text.ellipsize(&task.title, label.width, false, size);
        // A look may ink the label per button state (dark on a light face,
        // light on a dark checked one); the bar's own ink otherwise.
        let ink = style
            .text_color(Part::TaskButton, button_state)
            .unwrap_or(style.bar_text);
        text.draw(canvas, label, &title, false, ink, Align::Left, size);
    }

    if let Some(r) = layout.more {
        let lit = popup_state(state.more);
        let st = lit.unwrap_or(State::Normal);
        style.draw(canvas, r, Part::TaskButton, st, s);
        crate::taskbar::draw_more(canvas, r, popup_ink(lit), s);
    }

    for (r, icon) in layout.tray.iter().zip(&state.tray_icons) {
        if let Some(ic) = icon {
            canvas.blit_canvas(r.x, r.y, ic);
        }
    }
    if let Some(r) = layout.chevron {
        if layout.tray_arrow {
            let mut cell = state.chevron;
            cell.hot |= state.focus == Some(crate::taskbar::TaskbarHit::Chevron);
            if let Some(art) = &style.tray_arrow {
                draw_blue_tray_arrow(canvas, r, art, cell, s);
            }
        } else {
            let lit = popup_state(state.chevron);
            if let Some(st) = lit {
                style.draw(canvas, r, Part::TaskButton, st, s);
            }
            let ink = popup_ink(lit);
            layout.edge.draw_art(canvas, r, |c, at| {
                crate::taskbar::draw_chevron(c, at, ink, s)
            });
        }
    }

    match (layout.clock_lines, state.clock.split_once('\n')) {
        (Some(baselines), Some((time, date))) => crate::taskbar::draw_clock_lines(
            canvas,
            text,
            layout.clock,
            [time, date],
            baselines,
            style.bar_text,
            size,
            s,
        ),
        _ => text.draw(
            canvas,
            layout.clock,
            &state.clock,
            false,
            style.bar_text,
            Align::Center,
            size,
        ),
    }
    if let Some(r) = layout.show_desktop {
        let st = if state.show_desktop_hot {
            State::Hot
        } else {
            State::Normal
        };
        if style.part(Part::ShowDesktop).is_some() {
            canvas.clear_rect(r);
            layout.edge.draw_art(canvas, r, |c, at| {
                style.draw(c, at, Part::ShowDesktop, st, s)
            });
        } else {
            style.draw(canvas, r, Part::TaskButton, st, s);
        }
    }
}

/// Blits `image` at `at`, or the theme's chrome fallback at `px`, built once into `fallback`.
fn blit_icon(
    canvas: &mut Canvas,
    theme: &Theme,
    image: Option<&Canvas>,
    fallback: &mut Option<Canvas>,
    px: i32,
    at: (i32, i32),
) {
    let ic =
        image.unwrap_or_else(|| fallback.get_or_insert_with(|| icon::chrome_fallback(theme, px)));
    canvas.blit_canvas(at.0, at.1, ic);
}

fn edge_state(style: &Style, button: State) -> State {
    let has = |st| {
        style
            .part(Part::TaskStack)
            .is_some_and(|p| p.states.contains(&st))
    };
    let order: &[State] = match button {
        State::Pressed => &[State::Pressed, State::CheckedHot, State::Checked],
        State::CheckedHot => &[State::CheckedHot, State::Checked],
        State::Checked | State::Attention => &[State::Checked],
        State::Hot => &[State::Hot],
        _ => &[],
    };
    order
        .iter()
        .copied()
        .find(|&st| has(st))
        .unwrap_or(State::Normal)
}

fn task_state(style: &Style, state: &TaskbarState, i: usize) -> State {
    let hot = state.task_hot == Some(i) || state.task_float.is_some_and(|(f, _)| f == i);
    let task = &state.tasks[i];
    if state.task_pressed == Some(i) {
        State::Pressed
    } else if task.active && hot {
        State::CheckedHot
    } else if task.active {
        State::Checked
    } else if task.attention.lit() {
        attention_state(style)
    } else if hot {
        State::Hot
    } else {
        State::Normal
    }
}

/// A lit attention button's state: [`State::Attention`] where the look draws one, else checked.
fn attention_state(style: &Style) -> State {
    let has = style
        .part(Part::TaskButton)
        .is_some_and(|p| p.states.contains(&State::Attention));
    if has {
        State::Attention
    } else {
        State::Checked
    }
}

/// A round tray arrow in cell `r`: the look's button frame (19 x 19 px times `s`) centred in the
/// cell, ‹ while the hidden icons are folded away and › while shown.
fn draw_blue_tray_arrow(
    canvas: &mut Canvas,
    r: Rect,
    art: &crate::style::trayarrow::TrayArrowArt,
    cell: PopupCell,
    s: i32,
) {
    use crate::style::trayarrow::SIZE;
    let s = s.max(1);
    let frame = match (cell.open, cell.held, cell.hot) {
        (false, true, _) => &art.pressed,
        (false, false, true) => &art.hot,
        (false, false, false) => &art.rest,
        (true, true, _) => &art.open_pressed,
        (true, false, true) => &art.open_hot,
        (true, false, false) => &art.open_rest,
    };
    let n = SIZE * s;
    let (x0, y0) = (r.x + (r.width - n) / 2, r.y + (r.height - n) / 2);
    if art.scale == s {
        canvas.blit_over(x0, y0, frame);
    } else {
        canvas.blit_over(x0, y0, &frame.resample_box(n as u32, n as u32));
    }
}

fn popup_state(cell: PopupCell) -> Option<State> {
    if cell.open {
        Some(State::Checked)
    } else if cell.hot {
        Some(State::Hot)
    } else {
        None
    }
}

fn start_state(state: &TaskbarState) -> State {
    if state.start_pressed {
        State::Pressed
    } else if state.start_hot {
        State::Hot
    } else {
        State::Normal
    }
}

#[cfg(test)]
mod tests;
