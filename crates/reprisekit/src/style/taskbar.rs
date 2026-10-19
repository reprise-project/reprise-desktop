//! The taskbar drawn through a [`Style`].

use std::sync::Arc;

use crate::art::logo;
use crate::icon;
use crate::raster::{Canvas, Rect};
use crate::style::startbloom::{Bloom, IconKey};
use crate::style::{Part, PartImage, SpotFit, SpotFrames, State, Style};
use crate::taskbar::{Edge, PagerMark, PopupCell, TaskbarLayout, TaskbarState, quick_icon_origin};
use crate::text::{Align, TextRenderer};
use crate::theme::{Color, Theme};

/// Atlas row of [`Part::Taskbar`] an unlocked bar's grip hairline takes its colour from.
const GRIP_ROW: i32 = 27;

fn draw_start_mark(canvas: &mut Canvas, x: i32, top: i32, box_px: i32, s: i32, ink: Color) {
    // Centred in design pixels, so the mark at scale `s` lands where scale 1 puts it.
    let off = s * ((box_px / s - 16) / 2);
    logo::draw(canvas, x + off, top + off, 16 * s, ink);
}

fn draw_start_pill(
    canvas: &mut Canvas,
    style: &Style,
    start_text: &TextRenderer,
    st: Rect,
    state: &TaskbarState,
    s: i32,
) {
    let flag = style.taskbar.start_icon * s;
    let (flag_x, flag_y) = (st.x + 10 * s, st.y + 5 * s);
    match &state.start_icon {
        Some(ic) => canvas.blit_canvas(
            flag_x + (flag - ic.width as i32) / 2,
            flag_y + (flag - ic.height as i32) / 2,
            ic,
        ),
        None => draw_start_mark(canvas, flag_x, flag_y, flag, s, style.start_text.color),
    }
    let label = Rect::new(st.x + 35 * s, st.y, (st.width - 35 * s).max(0), st.height);
    let spec = &style.start_text;
    let start_size = spec.size_in(start_text) * s as u32;
    let reach = spec.shadow.map_or(0, |sh| {
        sh.dx.abs().max(sh.dy.abs()) + sh.soft.map_or(0, |soft| soft.size / 2)
    }) * s;
    let at = Rect::new(
        label.x - reach,
        label.y - reach,
        label.width + 2 * reach,
        label.height + 2 * reach,
    );
    if at.intersect(&canvas.bounds()).is_empty() {
        return;
    }
    let mut scratch = canvas.scratch(at.width as u32, at.height as u32);
    let inner = Rect::new(reach, reach, label.width, label.height);
    // White ink over clear: its colour channels carry the coverage.
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    start_text.draw(
        &mut scratch,
        inner,
        "start",
        spec.bold,
        white,
        Align::Left,
        start_size,
    );
    let mut coverage: Vec<u8> = scratch.pixels.iter().map(|&p| (p >> 16) as u8).collect();
    spec.weigh(&mut coverage);
    let (w, h) = scratch.device_size();
    let (w, h) = (w as usize, h as usize);
    let cs = canvas.scale();
    let (x, y) = (at.x * cs, at.y * cs);
    let all = canvas.bounds();
    if let Some(sh) = &spec.shadow {
        let alpha = sh.alpha(&coverage, w, h, s * cs);
        canvas.blit_mask_over(x, y, w, h, &alpha, sh.color, all);
    }
    canvas.blit_mask_over(x, y, w, h, &coverage, spec.color, all);
}

fn draw_image_nth(canvas: &mut Canvas, p: &PartImage, r: Rect, state: State, n: usize, s: i32) {
    p.blit(canvas, p.frame_nth(state, n), r, s);
}

fn design_size(p: &PartImage, state: State) -> (i32, i32) {
    let f = p.frame(state);
    (f.width / p.scale, f.height / p.scale)
}

/// [`Part::TaskLabelEdge`] frame indices: the pair beside a preceding or following button.
pub const EDGE_LEFT: usize = 0;
pub const EDGE_RIGHT: usize = 1;
/// At rest: the row's first and last pairs.
pub const EDGE_ROW_START: usize = 2;
pub const EDGE_ROW_END: usize = 3;
/// Lit or held: the next button's left pair and the previous one's right pair.
pub const EDGE_NEXT_LEFT: usize = 2;
pub const EDGE_PREV_RIGHT: usize = 3;

fn is_lit(state: State) -> bool {
    matches!(state, State::Hot | State::CheckedHot | State::Pressed)
}

#[derive(Clone, Copy)]
struct Span {
    x: i32,
    right: i32,
    pointer: Option<i32>,
}

impl Span {
    fn centre(&self, fit: &SpotFit, s: i32) -> f32 {
        let at = self.pointer.map_or((self.right - self.x) / s / 2, |p| {
            (p - self.x).div_euclid(s)
        });
        at as f32 + 0.5 + fit.dx
    }
}

#[derive(Clone, Copy)]
struct Spot<'a> {
    fit: &'a SpotFit,
    span: Span,
    glow: Option<Color>,
}

fn draw_spot(
    canvas: &mut Canvas,
    spot: &Spot,
    frames: &SpotFrames,
    r: Rect,
    state: State,
    n: usize,
    s: i32,
) {
    draw_image_nth(canvas, &frames.base, r, state, n, s);
    let mut gain = Canvas::new(r.width.max(0) as u32, r.height.max(0) as u32);
    let at = Rect::new(0, 0, r.width, r.height);
    draw_image_nth(&mut gain, &frames.gain, at, state, n, s);
    let span = spot.span;
    let (w, h) = (((span.right - span.x) / s) as f32, (r.height / s) as f32);
    let cx = span.centre(spot.fit, s);
    let light = spot.fit.in_colour(spot.glow);
    let (n, per) = (frames.base.scale, frames.base.per_atlas(s));
    // Each atlas pixel's middle in design px: design-size frames give the scale-1 spot
    // magnified, frames painted at `s` a smooth one.
    let mid = |v: i32| (v.div_euclid(per) as f32 + 0.5) / n as f32;
    for y in 0..r.height {
        let py = mid(y);
        for x in 0..r.width {
            let g = (gain.get(x, y) & 0xFF) as f32 / 32.0;
            if g == 0.0 {
                continue;
            }
            let px = mid(r.x + x - span.x);
            let under = canvas.get(r.x + x, r.y + y);
            let lit = light.light(under, g, px - cx, py, w, h);
            canvas.put_raw(r.x + x, r.y + y, lit);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_task_button(
    canvas: &mut Canvas,
    style: &Style,
    task: &crate::taskbar::TaskEntry,
    part: Part,
    r: Rect,
    state: State,
    span: Span,
    s: i32,
) {
    let lit = style.task_spot().filter(|_| is_lit(state));
    let frames = lit.and_then(|t| match part {
        Part::TaskLabel => t.labelled.as_ref(),
        _ => Some(&t.button),
    });
    match lit.zip(frames) {
        Some((t, f)) => {
            let spot = Spot {
                fit: &t.fit,
                span,
                glow: task.glow,
            };
            draw_spot(canvas, &spot, f, r, state, 0, s);
        }
        None => style.draw(canvas, r, part, state, s),
    }
}

fn label_pair(style: &Style, r: Rect, s: i32) -> i32 {
    (style
        .part(Part::TaskLabelEdge)
        .map_or(0, |p| design_size(p, State::Normal).0)
        * s)
        .min(r.width / 2)
}

#[allow(clippy::too_many_arguments)]
fn draw_labelled(
    canvas: &mut Canvas,
    style: &Style,
    task: &crate::taskbar::TaskEntry,
    r: Rect,
    state: State,
    prev: Option<State>,
    next: Option<State>,
    span: Span,
    s: i32,
) {
    let pair = label_pair(style, r, s);
    let frame = Rect::new(r.x + pair, r.y, (r.width - 2 * pair).max(0), r.height);
    let part = Part::TaskLabel;
    draw_task_button(canvas, style, task, part, frame, state, span, s);
    let rest = state == State::Normal;
    let left = match prev {
        None => (State::Normal, EDGE_ROW_START),
        Some(p) if rest && is_lit(p) => (p, EDGE_NEXT_LEFT),
        Some(_) => (state, EDGE_LEFT),
    };
    let right = match next {
        None => (State::Normal, EDGE_ROW_END),
        Some(n) if rest && is_lit(n) => (n, EDGE_PREV_RIGHT),
        Some(_) => (state, EDGE_RIGHT),
    };
    let glowing = (style.task_glow.and_then(|g| g.label_edges))
        .zip(style.part(Part::TaskLabelEdge))
        .zip(task.glow)
        .filter(|_| is_lit(state));
    for ((st, n), x) in [(left, r.x), (right, r.right() - pair)] {
        let at = Rect::new(x, r.y, pair, r.height);
        match glowing.filter(|_| st == state) {
            Some(((f, edges), colour)) => {
                draw_image_nth(canvas, &f(edges, st, n, colour), at, st, 0, s)
            }
            None => style.draw_nth(canvas, at, Part::TaskLabelEdge, st, n, s),
        }
    }
}

fn lights_slots(style: &Style) -> bool {
    style.has_parts(&[Part::SlotEdge, Part::SlotLight])
}

/// Whether a lit or held task button draws a spot at the pointer: the style spots its buttons and
/// the button is not an `idle` pinned slot lit by its slot light. A host redraws on pointer motion
/// only over such a button.
pub fn task_draws_spot(style: &Style, idle: bool) -> bool {
    style.task_spot.is_some() && !(idle && lights_slots(style))
}

fn draw_slot_light(
    canvas: &mut Canvas,
    style: &Style,
    r: Rect,
    state: State,
    s: i32,
    edge: Edge,
) -> bool {
    if !lights_slots(style) {
        return false;
    }
    if state == State::Pressed {
        draw_slot_band(canvas, style, r, s, edge);
    }
    draw_slot_edges(canvas, style, r, state, s, edge);
    draw_slot_light_foot(canvas, style, r, state, s, edge);
    true
}

fn slot_edge_width(style: &Style, s: i32) -> i32 {
    style
        .part(Part::SlotEdge)
        .map_or(0, |p| design_size(p, State::Hot).0 * s)
}

fn draw_slot_edges(canvas: &mut Canvas, style: &Style, r: Rect, state: State, s: i32, edge: Edge) {
    let w = slot_edge_width(style, s);
    canvas.with_clip(r, |c| {
        slot_art(edge).draw_art(c, r, |c, slot| {
            for x in [slot.x, slot.right() - w] {
                let at = Rect::new(x, slot.y, w, slot.height);
                style.draw(c, at, Part::SlotEdge, state, s);
            }
        })
    });
}

fn slot_art(edge: Edge) -> Edge {
    if edge.is_side() { edge } else { Edge::Bottom }
}

fn draw_slot_light_foot(
    canvas: &mut Canvas,
    style: &Style,
    r: Rect,
    state: State,
    s: i32,
    edge: Edge,
) {
    let Some(p) = style.part(Part::SlotLight) else {
        return;
    };
    let (fw, fh) = design_size(p, state);
    let (w, h) = (fw * s, fh * s);
    canvas.with_clip(r, |c| {
        edge.draw_art(c, r, |c, slot| {
            let x = slot.x + s * (slot.width / s / 2 - fw / 2);
            let at = Rect::new(x, slot.bottom() - h, w, h);
            style.draw(c, at, Part::SlotLight, state, s)
        })
    });
}

fn draw_slot_band(canvas: &mut Canvas, style: &Style, r: Rect, s: i32, edge: Edge) {
    let e = slot_edge_width(style, s);
    slot_art(edge).draw_art(canvas, r, |c, slot| {
        let band = Rect::new(slot.x + e, slot.y, (slot.width - 2 * e).max(0), slot.height);
        style.draw(c, band, Part::SlotBand, State::Pressed, s)
    });
}

fn pager_tile_rect(r: Rect, s: i32) -> Rect {
    let h = if r.height >= 40 * s { 24 } else { 20 } * s;
    Rect::new(r.x + s, r.y + (r.height - h) / 2, 21 * s, h)
}

/// Ink alpha (70%) of a pager number for a desktop other than the current one.
const PAGER_DIM: u32 = 0xB3;

fn draw_dimmed_number(
    canvas: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    label: &str,
    ink: Color,
    alpha: u32,
    size: u32,
) {
    let mut scratch = canvas.scratch(r.width.max(0) as u32, r.height.max(0) as u32);
    // White ink over clear: its red channel is the coverage.
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let local = Rect::new(0, 0, r.width, r.height);
    text.draw(
        &mut scratch,
        local,
        label,
        false,
        white,
        Align::Center,
        size,
    );
    let coverage: Vec<u8> = scratch
        .pixels
        .iter()
        .map(|&p| (((p >> 16) & 0xFF) * alpha / 255) as u8)
        .collect();
    let (w, h) = scratch.device_size();
    let cs = canvas.scale();
    let all = canvas.bounds();
    canvas.blit_mask_over(
        r.x * cs,
        r.y * cs,
        w as usize,
        h as usize,
        &coverage,
        ink,
        all,
    );
}

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

    let mut fallback: Option<Canvas> = None;
    for (i, r) in layout.quick.iter().enumerate() {
        if state.quick_pressed == Some(i) {
            style.draw(canvas, *r, Part::TaskButton, State::Checked, s);
        } else if state.quick_hot == Some(i) {
            style.draw(canvas, *r, Part::TaskButton, State::Hot, s);
        }
        let (ix, iy) = quick_icon_origin(*r, icon_px, s);
        match state.quick_icons.get(i) {
            Some(Some(ic)) => canvas.blit_canvas(ix, iy, ic),
            _ => {
                let fb = fallback.get_or_insert_with(|| icon::chrome_fallback(theme, icon_px));
                canvas.blit_canvas(ix, iy, fb)
            }
        }
    }
    if let Some(r) = layout.quick_more {
        let lit = popup_state(state.quick_more);
        if let Some(st) = lit {
            style.draw(canvas, r, Part::TaskButton, st, s);
        }
        let ink = lit
            .and_then(|st| style.text_color(Part::TaskButton, st))
            .unwrap_or(style.bar_text);
        crate::taskbar::draw_more(canvas, r, ink, s);
    }
    let lights = style.has_parts(&[Part::SlotEdge, Part::SlotLight]);
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
                let ink = lit
                    .and_then(|st| style.text_color(Part::TaskButton, st))
                    .unwrap_or(style.bar_text);
                (ink, 0xFF)
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
    let states: Vec<State> = (0..count).map(|i| task_state(state, i)).collect();
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
            match &task.icon {
                Some(ic) => canvas.blit_canvas(at.0, at.1, ic),
                None => {
                    let fb =
                        task_fallback.get_or_insert_with(|| icon::chrome_fallback(theme, task_px));
                    canvas.blit_canvas(at.0, at.1, fb)
                }
            }
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
        let pushed = matches!(button_state, State::Checked | State::CheckedHot) && style.task_nudge;
        let o = if pushed || held { s } else { 0 };
        let lb = style.taskbar.labelled;
        let narrow = r.width < (lb.icon_x + lb.icon_box) * s;
        let icon_x = if narrow || (layout.icons_only && style.taskbar.icon_button.is_some()) {
            r.x + (r.width - task_px) / 2
        } else {
            r.x + lb.icon_x * s + (lb.icon_box * s - task_px) / 2
        };
        let icon_at = (icon_x + o, r.y + (r.height - task_px) / 2 + o);
        match &task.icon {
            _ if narrow && r.width < task_px + 2 * o => {}
            Some(ic) => canvas.blit_canvas(icon_at.0, icon_at.1, ic),
            None => {
                let fb = task_fallback.get_or_insert_with(|| icon::chrome_fallback(theme, task_px));
                canvas.blit_canvas(icon_at.0, icon_at.1, fb)
            }
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

    let popup_ink = |lit: Option<State>| {
        lit.and_then(|st| style.text_color(Part::TaskButton, st))
            .unwrap_or(style.bar_text)
    };
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

fn edge_state(style: &Style, button: State) -> State {
    let has = |st| {
        style
            .part(Part::TaskStack)
            .is_some_and(|p| p.states.contains(&st))
    };
    let order: &[State] = match button {
        State::Pressed => &[State::Pressed, State::CheckedHot, State::Checked],
        State::CheckedHot => &[State::CheckedHot, State::Checked],
        State::Checked => &[State::Checked],
        State::Hot => &[State::Hot],
        _ => &[],
    };
    order
        .iter()
        .copied()
        .find(|&st| has(st))
        .unwrap_or(State::Normal)
}

fn task_state(state: &TaskbarState, i: usize) -> State {
    let hot = state.task_hot == Some(i) || state.task_float.is_some_and(|(f, _)| f == i);
    let task = &state.tasks[i];
    if state.task_pressed == Some(i) {
        State::Pressed
    } else if task.active && hot {
        State::CheckedHot
    } else if task.active || task.attention {
        State::Checked
    } else if hot {
        State::Hot
    } else {
        State::Normal
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

fn draw_orb(
    canvas: &mut Canvas,
    style: &Style,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    dx: i32,
    dy: i32,
    s: i32,
) {
    let st = layout.start;
    let at = |r: Rect| Rect::new(r.x + dx, r.y + dy, r.width, r.height);
    let look = start_state(state);
    match (layout.edge, layout.start_above) {
        (Edge::Top, Some(past)) => {
            let h = layout.height;
            let hang = layout.disc_past_bar().map_or(0, |d| d.height);
            let reach = past.height;
            let above = Rect::new(st.x, hang - reach, st.width, reach);
            style.draw(canvas, at(above), Part::StartAbove, look, s);
            let button = Rect::new(st.x, st.y + hang, st.width, st.height);
            style.draw(canvas, at(button), Part::StartButton, look, s);
            let glow = at(Rect::new(st.x, h + hang, st.width, reach - hang));
            canvas.with_rows_flipped(glow, |c| {
                c.with_clip(glow, |c| {
                    let r = at(Rect::new(st.x, h + hang, st.width, reach));
                    style.draw(c, r, Part::StartAbove, look, s)
                })
            });
            let whole = at(above).union(&glow);
            draw_bloom(canvas, style, layout, state, dx, dy, s, whole);
        }
        (Edge::Left | Edge::Right, _) => {
            let size = |p: Part| {
                style.part(p).map(|i| {
                    let (w, h) = design_size(i, look);
                    (w * s, h * s)
                })
            };
            let (w, h) = size(Part::StartButton).unwrap_or((st.width, st.height));
            let x = st.x + (st.width - w) / 2;
            let button = Rect::new(x, st.bottom() - h, w, h);
            canvas.with_clip(at(st), |c| {
                if let Some((_, reach)) = size(Part::StartAbove) {
                    let above = Rect::new(x, button.y - reach, w, reach);
                    style.draw(c, at(above), Part::StartAbove, look, s);
                }
                style.draw(c, at(button), Part::StartButton, look, s);
            });
            draw_bloom(canvas, style, layout, state, dx, dy, s, at(st));
        }
        _ => {
            style.draw(canvas, at(st), Part::StartButton, look, s);
            draw_bloom(canvas, style, layout, state, dx, dy, s, at(st));
        }
    }
    let i = start_icon_box(style, layout, state, s);
    match &state.start_icon {
        Some(ic) => canvas.blit_canvas(i.x + dx, i.y + dy, ic),
        None => logo::draw_path(canvas, i.x + dx, i.y + dy, i.width, START_MARK),
    }
}

const START_MARK: Color = Color::rgb(0xff, 0xff, 0xff);
const ORB_MARK_SHARE: (i32, i32) = (5, 6);

/// Where the orb's icon goes, in the bar's device px: centred on the disc, the icon theme's logo at
/// its own size, else the mark at [`ORB_MARK_SHARE`] of the shape's `start_icon` size.
fn start_icon_box(style: &Style, layout: &TaskbarLayout, state: &TaskbarState, s: i32) -> Rect {
    let d = layout.start_disc.unwrap_or(layout.start);
    let (w, h) = match &state.start_icon {
        Some(ic) => (ic.width as i32, ic.height as i32),
        None => (mark_px(style, s), mark_px(style, s)),
    };
    Rect::new(d.x + (d.width - w) / 2, d.y + (d.height - h) / 2, w, h)
}

/// The side of the orb's mark at scale `s`, device px.
fn mark_px(style: &Style, s: i32) -> i32 {
    style.taskbar.start_icon * s * ORB_MARK_SHARE.0 / ORB_MARK_SHARE.1
}

fn mark_icon(px: i32) -> Canvas {
    let mut c = Canvas::new(px as u32, px as u32);
    logo::draw_path(&mut c, 0, 0, px, START_MARK);
    c
}

/// The bloom round the orb's `icon` (the mark when None) at scale `s` in `state` at `strength`,
/// painted unless kept.
pub(crate) fn orb_bloom(
    style: &Style,
    s: i32,
    icon: Option<&Canvas>,
    state: State,
    strength: u8,
) -> Option<Arc<Bloom>> {
    let blooms = style.start_bloom.as_ref()?;
    Some(match icon {
        Some(ic) => blooms.get(IconKey::of(ic), s, state, strength, || ic.clone()),
        None => {
            let px = mark_px(style, s);
            blooms.get(IconKey::Mark(px), s, state, strength, || mark_icon(px))
        }
    })
}

/// Whether `style` keeps [`orb_bloom`]'s bloom.
pub(crate) fn keeps_orb_bloom(
    style: &Style,
    s: i32,
    icon: Option<&Canvas>,
    state: State,
    strength: u8,
) -> bool {
    let key = match icon {
        Some(ic) => IconKey::of(ic),
        None => IconKey::Mark(mark_px(style, s)),
    };
    (style.start_bloom.as_ref()).is_some_and(|b| b.keeps(key, s, state, strength))
}

/// The glow is painted once per icon, scale, state and strength; here it is only laid on.
#[allow(clippy::too_many_arguments)]
fn draw_bloom(
    canvas: &mut Canvas,
    style: &Style,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    dx: i32,
    dy: i32,
    s: i32,
    clip: Rect,
) {
    if state.orb_glow == 0 {
        return;
    }
    let icon = state.start_icon.as_deref();
    let Some(bloom) = orb_bloom(style, s, icon, start_state(state), state.orb_glow) else {
        return;
    };
    let i = start_icon_box(style, layout, state, s);
    canvas.blit_screen_clipped(i.x + dx + bloom.x, i.y + dy + bloom.y, &bloom.canvas, clip);
}

/// Draws the orb's own buffer (sized to `layout.start_orb(..)`): the rows reaching past the bar,
/// and while the menu is open the whole orb, which `draw_taskbar` omits so the host can stand it
/// over the menu.
pub fn draw_start_above(
    canvas: &mut Canvas,
    style: &Style,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    s: i32,
) {
    let (Some(a), Some(r)) = (layout.start_above, layout.start_orb(state.start_pressed)) else {
        return;
    };
    if layout.edge == Edge::Top {
        draw_orb(canvas, style, layout, state, -r.x, -r.y, s);
        return;
    }
    let at = Rect::new(a.x - r.x, a.y - r.y, a.width, a.height);
    style.draw(canvas, at, Part::StartAbove, start_state(state), s);
    draw_bloom(canvas, style, layout, state, -r.x, -r.y, s, at);
    if r != a {
        draw_orb(canvas, style, layout, state, -r.x, -r.y, s);
    }
}

fn stacked(style: &Style, slot: Rect, stack: Option<u8>, s: i32) -> (Rect, Vec<Rect>) {
    let Some(p) = style.part(Part::TaskStack) else {
        return (slot, Vec::new());
    };
    let n = (i32::from(stack.unwrap_or(1)) - 1).clamp(0, 2);
    let w = design_size(p, State::Normal).0 * s;
    let front = Rect::new(slot.x, slot.y, slot.width - n * w, slot.height);
    let edges = (0..n)
        .map(|k| Rect::new(front.right() + k * w, slot.y, w, slot.height))
        .collect();
    (front, edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces::theme_for;
    use crate::raster::Rect;
    use crate::style::Part;
    use crate::taskbar::{TaskEntry, TaskbarCounts};
    use crate::theme::FontConfig;

    fn draw_image(canvas: &mut Canvas, p: &PartImage, r: Rect, state: State, s: i32) {
        draw_image_nth(canvas, p, r, state, 0, s);
    }

    fn clear_start_logo() -> std::sync::Arc<Canvas> {
        Canvas::new(24, 24).into()
    }

    fn state_with_tasks(titles: &[&str]) -> TaskbarState {
        TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            quick_more: Default::default(),
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            task_progress: Vec::new(),
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: titles
                .iter()
                .map(|t| TaskEntry {
                    title: (*t).to_string(),
                    active: false,
                    attention: false,
                    icon: None,
                    glow: None,
                })
                .collect(),
            tray_icons: vec![],
            clock: String::new(),
        }
    }

    /// A task's progress lands inside its own button, under the label, in Blue and Glass.
    #[test]
    fn a_tasks_progress_draws_inside_its_button() {
        use crate::taskbar::{TaskProgress, TaskRow};
        for iface in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(iface);
            let s = theme.look.style().unwrap();
            let counts = TaskbarCounts {
                tasks: 2,
                ..TaskbarCounts::default()
            };
            let mut layout =
                TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
            layout.lay_tasks(&theme.metrics, &s.taskbar, &[0, 1], TaskRow::Shared);
            let h = layout.height as u32;
            let text = TextRenderer::load(&FontConfig::default());
            let start = TextRenderer::load(&s.start_text.font());
            let draw = |st: &TaskbarState| {
                let mut c = Canvas::new(800, h);
                draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
                c
            };
            let mut st = state_with_tasks(&["One", "Two"]);
            let plain = draw(&st);
            st.task_progress = vec![
                None,
                Some(TaskProgress {
                    fraction: 0.5,
                    paused: false,
                }),
            ];
            let shown = draw(&st);
            let changed: Vec<(i32, i32)> = (0..h as i32)
                .flat_map(|y| (0..800).map(move |x| (x, y)))
                .filter(|&(x, y)| plain.get(x, y) != shown.get(x, y))
                .collect();
            let b = layout.tasks[1];
            assert!(!changed.is_empty(), "{iface:?} draws the progress");
            assert!(
                changed.iter().all(|&(x, y)| b.contains(x, y)),
                "{iface:?} keeps it inside the second button"
            );
            if iface == Interface::Glass {
                assert!(
                    changed.iter().all(|&(x, _)| x < b.x + b.width * 3 / 4),
                    "Glass fills only the done half"
                );
            }
        }
    }

    #[test]
    fn a_pinned_app_without_windows_draws_no_button_until_hot() {
        use crate::taskbar::TaskRow;
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        layout.lay_tasks(&theme.metrics, &s.taskbar, &[0, 1], TaskRow::Shared);
        let h = layout.height as u32;
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |st: &TaskbarState| {
            let mut c = Canvas::new(800, h);
            draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
            c
        };
        let part_px = |state, r: Rect, x, y| {
            let mut w = Canvas::new(800, h);
            s.draw(
                &mut w,
                Rect::new(0, 0, 800, h as i32),
                Part::Taskbar,
                State::Normal,
                1,
            );
            s.draw(&mut w, r, Part::TaskButton, state, 1);
            w.get(x, y)
        };
        let p = layout.tasks[0];
        let mut st = state_with_tasks(&["", "Two"]);
        let (x, y) = (p.x + 1, p.y + 2);
        let mut bar = Canvas::new(800, h);
        s.draw(
            &mut bar,
            Rect::new(0, 0, 800, h as i32),
            Part::Taskbar,
            State::Normal,
            1,
        );
        assert_eq!(draw(&st).get(x, y), bar.get(x, y), "the bar shows through");
        st.task_hot = Some(0);
        let hot = part_px(State::Hot, p, x, y);
        assert_ne!(hot, bar.get(x, y), "a Hot button shows");
        assert_eq!(draw(&st).get(x, y), hot);
        st.task_hot = None;
        st.task_pressed = Some(0);
        let held = part_px(State::Pressed, p, x, y);
        assert_ne!(held, hot, "a Pressed button differs from the Hot one");
        assert_eq!(draw(&st).get(x, y), held, "a Pressed button shows held");
    }

    #[test]
    fn blue_bar_paints_start_tasks_and_the_tray_area() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 2,
                tray: 1,
                show_desktop: false,
                chevron: false,
                ..TaskbarCounts::default()
            },
            40,
        );
        let mut st = state_with_tasks(&["One", "Two"]);
        st.tasks[0].active = true;
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        let part_px = |part, state, r: Rect, x, y| {
            let mut w = Canvas::new(800, 30);
            s.draw(&mut w, r, part, state, 1);
            w.get(x, y)
        };
        let t0 = layout.tasks[0];
        let t1 = layout.tasks[1];
        assert_eq!(
            c.get(t0.x + 2, t0.y + 2),
            part_px(Part::TaskButton, State::Checked, t0, t0.x + 2, t0.y + 2)
        );
        assert_eq!(
            c.get(t1.x + 2, t1.y + 2),
            part_px(Part::TaskButton, State::Normal, t1, t1.x + 2, t1.y + 2)
        );
        let n = layout.notification;
        assert_eq!(
            c.get(n.x + 1, 1),
            part_px(Part::TrayArea, State::Normal, n, n.x + 1, 1)
        );
        assert_eq!(
            c.get(3, 15),
            part_px(Part::StartButton, State::Normal, layout.start, 3, 15)
        );
    }

    #[test]
    fn the_start_label_draws_at_its_own_font_size() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let start = TextRenderer::load(&s.start_text.font());
        if !start.has_face("Franklin Gothic Medium") && !start.has_face("Trebuchet MS") {
            return;
        }
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts::default(),
            40,
        );
        let text = TextRenderer::load(&FontConfig::default());
        let mut c = Canvas::new(800, 30);
        draw_taskbar(
            &mut c,
            s,
            &theme,
            &text,
            &start,
            &layout,
            &state_with_tasks(&[]),
        );

        let scale = theme.scale.max(1);
        let st = layout.start;
        let spec = &s.start_text;
        let label = Rect::new(st.x + 28 * scale, st.y, st.right() - 28 * scale, st.height);
        let drawn = ink_span(&c, label, spec.color.argb());

        let mut alone = Canvas::new(100, 30);
        start.draw(
            &mut alone,
            Rect::new(0, 0, 100, 30),
            "start",
            spec.bold,
            spec.color,
            Align::Left,
            spec.size_in(&start),
        );
        let want = ink_span(&alone, Rect::new(0, 0, 100, 30), spec.color.argb());
        assert_eq!(
            drawn, want,
            "the label's ink spans {drawn} px, not the {want} px its own font draws it at"
        );
    }

    fn ink_span(c: &Canvas, r: Rect, ink: u32) -> i32 {
        let xs = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .map(|(x, _)| x);
        let (lo, hi) = xs.fold((i32::MAX, i32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)));
        if lo > hi { 0 } else { hi - lo + 1 }
    }

    #[test]
    fn a_task_buttons_icon_and_title_sit_10_and_30_px_in_a_pixel_further_when_pushed() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: 2,
                tray: 0,
                show_desktop: false,
                chevron: false,
                ..TaskbarCounts::default()
            },
            40,
        );
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(
            Rect::new(0, 0, 16, 16),
            crate::theme::Color::rgb(0xFF, 0, 0xFF),
        );
        let magenta = icon.get(0, 0);
        let mut st = state_with_tasks(&["Notes", "Notes"]);
        st.tasks[1].active = true;
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
        }
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);

        let white = s.bar_text.argb();
        let mut alone = Canvas::new(60, 25);
        text.draw(
            &mut alone,
            Rect::new(0, 0, 60, 25),
            "Notes",
            false,
            s.bar_text,
            Align::Left,
            theme.fonts.size_px,
        );
        let lead = |c: &Canvas, r: Rect| {
            (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) == white))
        };
        let bearing = lead(&alone, Rect::new(0, 0, 60, 25));

        for (i, o) in [(0, 0), (1, 1)] {
            let r = layout.tasks[i];
            assert_eq!(
                c.get(r.x + 10 + o, r.y + 4 + o),
                magenta,
                "task {i} icon top-left"
            );
            assert_ne!(
                c.get(r.x + 9 + o, r.y + 4 + o),
                magenta,
                "task {i} left of its icon"
            );
            let title = Rect::new(r.x + 27, r.y, r.width - 27, r.height);
            assert_eq!(
                lead(&c, title),
                bearing.map(|b| r.x + 30 + o + b),
                "task {i} title's first ink"
            );
        }
    }

    #[test]
    fn the_start_flag_sits_10_px_in_and_5_down_and_the_label_35_px_in() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts::default(),
            40,
        );
        assert_eq!(s.taskbar.start_icon, 18);
        let mut flag = Canvas::new(18, 18);
        flag.fill_rect(
            Rect::new(0, 0, 18, 18),
            crate::theme::Color::rgb(0xFF, 0, 0xFF),
        );
        let magenta = flag.get(0, 0);
        let mut st = state_with_tasks(&[]);
        st.start_icon = Some(flag.into());
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        let b = layout.start;
        assert_eq!(c.get(b.x + 10, b.y + 5), magenta, "the flag's top-left");
        assert_eq!(
            c.get(b.x + 27, b.y + 22),
            magenta,
            "the flag's bottom-right"
        );
        assert_ne!(c.get(b.x + 9, b.y + 5), magenta);
        assert_ne!(c.get(b.x + 10, b.y + 4), magenta);

        let spec = &s.start_text;
        let white = spec.color.argb();
        let lead = |c: &Canvas, r: Rect| {
            (r.x..r.right()).find(|&x| (r.y..r.bottom()).any(|y| c.get(x, y) == white))
        };
        let mut alone = Canvas::new(100, 30);
        start.draw(
            &mut alone,
            Rect::new(0, 0, 100, 30),
            "start",
            spec.bold,
            spec.color,
            Align::Left,
            spec.size_in(&start),
        );
        let bearing = lead(&alone, Rect::new(0, 0, 100, 30));
        let label = Rect::new(b.x + 28, b.y, b.width - 28, b.height);
        assert_eq!(lead(&c, label), bearing.map(|x| b.x + 35 + x));
    }

    #[test]
    fn the_start_label_is_inked_heavier_over_a_soft_shadow_from_1_px_up_left_to_2_down_right() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let start = TextRenderer::load(&s.start_text.font());
        if !start.has_face("Franklin Gothic Medium") && !start.has_face("Trebuchet MS") {
            return;
        }
        let text = TextRenderer::load(&FontConfig::default());
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts::default(),
            40,
        );
        let mut c = Canvas::new(800, 30);
        draw_taskbar(
            &mut c,
            s,
            &theme,
            &text,
            &start,
            &layout,
            &state_with_tasks(&[]),
        );
        let b = layout.start;
        let mut bare = Canvas::new(800, 30);
        s.draw(
            &mut bare,
            Rect::new(0, 0, 800, 30),
            Part::Taskbar,
            State::Normal,
            1,
        );
        s.draw(&mut bare, b, Part::StartButton, State::Normal, 1);

        let spec = &s.start_text;
        let mut cov = Canvas::new(800, 30);
        let label = Rect::new(b.x + 35, b.y, b.width - 35, b.height);
        let white = crate::theme::Color::rgb(0xFF, 0xFF, 0xFF);
        let size = spec.size_in(&start);
        start.draw(
            &mut cov,
            label,
            "start",
            spec.bold,
            white,
            Align::Left,
            size,
        );
        let at = |x: i32, y: i32| (cov.get(x, y) >> 16) & 0xFF;
        let inked: Vec<(i32, i32)> = (0..30)
            .flat_map(|y| (0..800).map(move |x| (x, y)))
            .filter(|&(x, y)| at(x, y) != 0)
            .collect();
        let x0 = inked.iter().map(|p| p.0).min().unwrap();
        let x1 = inked.iter().map(|p| p.0).max().unwrap();
        let y0 = inked.iter().map(|p| p.1).min().unwrap();
        let y1 = inked.iter().map(|p| p.1).max().unwrap();

        let plain = {
            let mut p = bare.clone();
            start.draw(&mut p, label, "start", spec.bold, white, Align::Left, size);
            p
        };
        let luma = |px: u32| (px >> 16 & 0xFF) + (px >> 8 & 0xFF) + (px & 0xFF);
        let &(hx, hy) = inked
            .iter()
            .find(|&&(x, y)| (96..160).contains(&at(x, y)))
            .expect("a half-covered pixel");
        assert!(
            luma(c.get(hx, hy)) > luma(plain.get(hx, hy)),
            "({hx}, {hy}) at coverage {} inks heavier than a plain draw",
            at(hx, hy)
        );

        let differs = |x: i32, y: i32| c.get(x, y) != bare.get(x, y);
        let within =
            |x: i32, y: i32| (x0 - 1..=x1 + 2).contains(&x) && (y0 - 1..=y1 + 2).contains(&y);
        for y in 0..30 {
            for x in b.x + 28..b.right() {
                if !within(x, y) {
                    assert!(!differs(x, y), "({x}, {y}) is the bare pill");
                }
            }
        }
        assert!(
            (y0..=y1).any(|y| differs(x1 + 2, y)),
            "2 px right of the ink"
        );
        assert!((x0..=x1).any(|x| differs(x, y1 + 2)), "2 px below the ink");
        assert!(
            (y0..=y1).any(|y| differs(x0 - 1, y)),
            "1 px left of the ink"
        );
        assert!((x0..=x1).any(|x| differs(x, y0 - 1)), "1 px above the ink");
        assert_eq!(
            spec.shadow.map(|sh| sh.color),
            Some(crate::blue::Variant::Blue.scheme().inks.start_shadow),
            "a dark olive shadow"
        );
    }

    fn glass_bar(clock: &str, start_icon: Option<Canvas>) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        glass_bar_in(theme.look.style().unwrap(), 40, clock, start_icon)
    }

    fn glass_bar_in(
        s: &Style,
        height: i32,
        clock: &str,
        start_icon: Option<Canvas>,
    ) -> (Canvas, TaskbarLayout) {
        let mut st = state_with_tasks(&["One", "Two", "Three"]);
        st.tasks[1].active = true;
        st.task_hot = Some(2);
        st.clock = clock.to_string();
        st.start_icon = start_icon.map(Into::into);
        draw_glass(s, height, &st)
    }

    fn draw_glass(s: &Style, height: i32, st: &TaskbarState) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: height,
            ..theme.metrics.clone()
        };
        let layout = TaskbarLayout::compute(
            &metrics,
            &s.taskbar,
            1920,
            true,
            TaskbarCounts {
                quick: 0,
                pager: 0,
                tasks: st.tasks.len(),
                tray: 0,
                chevron: false,
                show_desktop: false,
                ..TaskbarCounts::default()
            },
            60,
        );
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
        (c, layout)
    }

    #[test]
    fn a_window_without_an_icon_shows_a_light_glyph_on_the_dark_glass_bar() {
        let settings = crate::glass::Settings {
            mode: crate::glass::Mode::Dark,
            ..crate::glass::Settings::default()
        };
        let theme = crate::interfaces::theme_with(
            Interface::Glass,
            crate::blue::Variant::default(),
            &settings,
        );
        let glyph = icon::chrome_fallback(&theme, 16);
        assert_eq!(glyph.get(8, 10) & 0xFF_FFFF, 0xFF_FFFF, "the white body");
        assert_eq!(glyph.get(1, 8) & 0xFF_FFFF, 0xEF_EEEF, "the EFEEEF frame");
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tasks: 1,
            ..TaskbarCounts::default()
        };
        let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        let h = layout.height as u32;
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |st: &TaskbarState| {
            let mut c = Canvas::new(800, h);
            draw_taskbar(&mut c, s, &theme, &text, &start, &layout, st);
            c
        };
        let bare = state_with_tasks(&["One"]);
        let mut given = state_with_tasks(&["One"]);
        given.tasks[0].icon = Some(icon::chrome_fallback(&theme, s.taskbar.task_icon).into());
        assert!(
            draw(&bare).pixels == draw(&given).pixels,
            "the bar draws the light glyph"
        );
    }

    fn solid_icon(c: crate::theme::Color, px: i32) -> Canvas {
        let mut icon = Canvas::new(px as u32, px as u32);
        icon.fill_rect(Rect::new(0, 0, px, px), c);
        icon
    }

    fn over_grey(px: u32) -> [i32; 3] {
        let a = (px >> 24) as i32;
        [16, 8, 0].map(|sh| ((px >> sh) & 0xFF) as i32 + (255 - a) * 128 / 255)
    }

    fn glass_stacks(s: &Style, height: i32) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: height,
            ..theme.metrics.clone()
        };
        let counts = TaskbarCounts {
            tasks: 3,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &metrics,
            &s.taskbar,
            &[1, 2, 3],
            crate::taskbar::TaskRow::Icons,
        );
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&["", "", ""]);
        for t in &mut st.tasks {
            t.icon = Some(Canvas::new(16, 16).into());
        }
        let mut c = Canvas::new(1920, height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        (c, layout)
    }

    #[test]
    fn a_glass_button_for_several_windows_ends_in_stacked_edges() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, l) = glass_stacks(s, 40);
        for (i, edges) in [0, 1, 2].into_iter().enumerate() {
            let r = l.tasks[i];
            assert_eq!(r.width, 60, "slot {i} keeps its 60 px");
            let mut want = Canvas::new(1920, 40);
            s.draw(
                &mut want,
                Rect::new(0, 0, 1920, 40),
                Part::Taskbar,
                State::Normal,
                1,
            );
            let front = Rect::new(r.x, r.y, 60 - 4 * edges, r.height);
            s.draw(&mut want, front, Part::TaskButton, State::Normal, 1);
            for k in 0..edges {
                let e = Rect::new(front.right() + 4 * k, r.y, 4, r.height);
                s.draw_nth(&mut want, e, Part::TaskStack, State::Normal, k as usize, 1);
            }
            for y in 0..40 {
                for x in r.x..r.right() {
                    assert_eq!(c.get(x, y), want.get(x, y), "{} windows: ({x}, {y})", i + 1);
                }
            }
        }
        let at = |i: usize, dx: i32| over_grey(c.get(l.tasks[i].right() - dx, 20));
        assert_eq!(
            at(0, 2),
            [180, 187, 197],
            "one window: the button's light line"
        );
        assert_eq!(at(0, 1), [42, 57, 77], "one window: the button's dark line");
        assert_eq!(
            at(1, 5),
            [42, 57, 77],
            "two windows: the button's dark line, 4 px in"
        );
        assert_eq!(
            at(1, 2),
            [143, 154, 168],
            "two windows: the edge's light line"
        );
        assert_eq!(at(1, 1), [67, 82, 102], "two windows: the edge's dark line");
        assert_eq!(
            at(2, 6),
            [143, 154, 168],
            "three windows: the nearer light line"
        );
        assert_eq!(
            at(2, 2),
            [118, 131, 149],
            "three windows: the farther light line"
        );
        assert_eq!(
            at(2, 1),
            [84, 99, 119],
            "three windows: the farther dark line"
        );
    }

    fn glass_stacks_in(
        active: [bool; 3],
        hot: Option<usize>,
        held: Option<usize>,
    ) -> (Canvas, TaskbarLayout) {
        glass_stacks_glowing(active, hot, held, None, 1)
    }

    fn glass_stacks_glowing(
        active: [bool; 3],
        hot: Option<usize>,
        held: Option<usize>,
        glow: Option<crate::theme::Color>,
        scale: i32,
    ) -> (Canvas, TaskbarLayout) {
        glass_row(&[1, 2, 3], active, hot, held, glow, scale)
    }

    fn glass_row(
        stacks: &[u8],
        active: [bool; 3],
        hot: Option<usize>,
        held: Option<usize>,
        glow: Option<crate::theme::Color>,
        scale: i32,
    ) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        glass_row_in(s, stacks, active, (hot, held, None), glow, scale)
    }

    fn glass_row_in(
        s: &Style,
        stacks: &[u8],
        active: [bool; 3],
        lit: (Option<usize>, Option<usize>, Option<i32>),
        glow: Option<crate::theme::Color>,
        scale: i32,
    ) -> (Canvas, TaskbarLayout) {
        let (hot, held, x) = lit;
        let theme = theme_for(Interface::Glass).scaled(scale);
        let counts = TaskbarCounts {
            tasks: 3,
            ..TaskbarCounts::default()
        };
        let base = theme_for(Interface::Glass);
        let mut layout = TaskbarLayout::compute(&base.metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &base.metrics,
            &s.taskbar,
            stacks,
            crate::taskbar::TaskRow::Icons,
        );
        let layout = layout.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&["", "", ""]);
        let px = (s.taskbar.task_icon * scale) as u32;
        let mut white = Canvas::new(px, px);
        white.fill_rect(white.bounds(), crate::theme::Color::rgb(0xFF, 0xFF, 0xFF));
        for (t, a) in st.tasks.iter_mut().zip(active) {
            t.icon = Some(white.clone().into());
            t.active = a;
            t.glow = glow;
        }
        st.task_hot = hot;
        st.task_pressed = held;
        st.task_hot_x = x;
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        (c, layout)
    }

    fn glass_unlit_spot() -> Style {
        glass_unlit_spot_at(1)
    }

    fn glass_unlit_spot_at(sc: i32) -> Style {
        let mut s = crate::glass::build(&Default::default());
        if sc > 1 {
            let at = crate::glass::style(&Default::default()).at_scale(sc);
            for part in [
                Part::Taskbar,
                Part::TrayArea,
                Part::LabelRow,
                Part::TaskButton,
                Part::TaskStack,
                Part::TaskLabel,
                Part::TaskLabelEdge,
                Part::SlotEdge,
                Part::SlotLight,
                Part::SlotBand,
                Part::PagerTile,
                Part::ShowDesktop,
            ] {
                s = s.with_part(part, at.part(part).unwrap().clone());
            }
            s.task_spot = at.task_spot.clone();
        }
        let mut spot = s.task_spot().expect("Glass buttons light a spot").clone();
        spot.fit.halo.peak = 0.0;
        spot.fit.core.peak = 0.0;
        s.with_task_spot(Some(spot))
    }

    fn brightest(lit: &Canvas, bare: &Canvas, x0: i32, cols: std::ops::Range<i32>, y: i32) -> i32 {
        let sum = |c: &Canvas, x: i32| over_grey(c.get(x0 + x, y)).iter().sum::<i32>();
        let lift: Vec<(i32, i32)> = cols.map(|x| (x, sum(lit, x) - sum(bare, x))).collect();
        let top = lift.iter().map(|&(_, l)| l).max().expect("some columns");
        let tied: Vec<i32> = lift
            .iter()
            .filter(|&&(_, l)| l == top)
            .map(|&(x, _)| x)
            .collect();
        tied[tied.len() / 2]
    }

    fn over_veil(s: &Style, l: &TaskbarLayout) -> Canvas {
        over_veil_at(s, l, 1)
    }

    fn over_veil_at(s: &Style, l: &TaskbarLayout, sc: i32) -> Canvas {
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        let bar = Rect::new(0, 0, l.width, l.height);
        s.draw(&mut c, bar, Part::Taskbar, State::Normal, sc);
        let (x0, x1) = (l.tasks_x() - 5 * sc, l.tasks_limit() + 7 * sc);
        let row = Rect::new(x0, 0, x1 - x0, l.height);
        s.draw(&mut c, row, Part::LabelRow, State::Normal, sc);
        c
    }

    fn label_frame(r: Rect, sc: i32) -> Rect {
        Rect::new(r.x + 3 * sc, r.y, r.width - 6 * sc, r.height)
    }

    fn glass_labelled(
        stacks: &[u8],
        active: &[bool],
        hot: Option<usize>,
        held: Option<usize>,
        glow: Option<crate::theme::Color>,
        scale: i32,
    ) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        glass_labelled_in(s, stacks, active, (hot, held, None), glow, scale)
    }

    fn glass_labelled_in(
        s: &Style,
        stacks: &[u8],
        active: &[bool],
        lit: (Option<usize>, Option<usize>, Option<i32>),
        glow: Option<crate::theme::Color>,
        scale: i32,
    ) -> (Canvas, TaskbarLayout) {
        let (hot, held, x) = lit;
        labelled_row(s, stacks, scale, |st, _| {
            for (t, &a) in st.tasks.iter_mut().zip(active) {
                t.active = a;
                t.glow = glow;
            }
            st.task_hot = hot;
            st.task_pressed = held;
            st.task_hot_x = x;
        })
    }

    fn labelled_row(
        s: &Style,
        stacks: &[u8],
        scale: i32,
        edit: impl FnOnce(&mut TaskbarState, &mut TaskbarLayout),
    ) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass).scaled(scale);
        let base = theme_for(Interface::Glass);
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&base.metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &base.metrics,
            &s.taskbar,
            stacks,
            crate::taskbar::TaskRow::AtLeast(62),
        );
        let mut layout = layout.scaled(scale);
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&vec![""; stacks.len()]);
        let px = 16 * scale as u32;
        for t in st.tasks.iter_mut() {
            t.icon = Some(Canvas::new(px, px).into());
        }
        edit(&mut st, &mut layout);
        let mut c = Canvas::new(layout.width as u32, layout.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        (c, layout)
    }

    #[test]
    fn a_sliding_glass_button_draws_off_its_slot_with_its_stacked_edges() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        for scale in [1, 2] {
            let (c, _) = labelled_row(s, &[1, 2], scale, |st, _| st.task_shift = vec![0, 30]);
            let (want, _) = labelled_row(s, &[1, 2], scale, |_, l| l.tasks[1].x += 30 * scale);
            assert_eq!(
                same_columns(&c, &want, 0..c.width as i32),
                Ok(()),
                "scale {scale}: the group draws 30 px right, as in a slot there"
            );
        }
    }

    #[test]
    fn a_floated_glass_button_draws_lit_over_the_row_and_leaves_its_slot_empty() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (_, l) = labelled_row(s, &[1, 1, 1], 1, |_, _| {});
        let (x1, x2) = (l.tasks[1].x, l.tasks[2].x);
        let (c, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| st.task_float = Some((2, x1)));
        let (want, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| {
            st.task_shift = vec![0, 0, x1 - x2];
            st.task_hot = Some(2);
        });
        assert_eq!(same_columns(&c, &want, 0..1920), Ok(()));
        let veil = over_veil(s, &l);
        let slot = l.tasks[2];
        assert_eq!(
            same_columns(&c, &veil, slot.x..slot.right()),
            Ok(()),
            "the floated button's slot is empty"
        );
        let (first, _) = labelled_row(s, &[1, 1, 1], 1, |st, _| st.task_float = Some((0, x1)));
        let slot = l.tasks[0];
        assert_eq!(same_columns(&first, &veil, slot.x..slot.right()), Ok(()));
        let r = label_frame(l.tasks[1], 1);
        assert_eq!(same_columns(&first, &c, r.x..r.right()), Ok(()));
    }

    type Slot = (State, (State, usize), (State, usize));

    fn labelled_want(s: &Style, l: &TaskbarLayout, slots: &[Slot], sc: i32) -> Canvas {
        let mut c = over_veil_at(s, l, sc);
        for (r, &(st, left, right)) in l.tasks.iter().zip(slots) {
            label_in(&mut c, s, label_frame(*r, sc), st, sc);
            for ((est, n), x) in [(left, r.x), (right, r.right() - 3 * sc)] {
                let at = Rect::new(x, r.y, 3 * sc, r.height);
                s.draw_nth(&mut c, at, Part::TaskLabelEdge, est, n, sc);
            }
        }
        c
    }

    fn label_in(c: &mut Canvas, s: &Style, frame: Rect, state: State, sc: i32) {
        let lit = s.task_spot().filter(|_| is_lit(state));
        match lit.and_then(|t| Some(&t.fit).zip(t.labelled.as_ref())) {
            Some((fit, f)) => {
                let span = Span {
                    x: frame.x,
                    right: frame.right(),
                    pointer: None,
                };
                let spot = Spot {
                    fit,
                    span,
                    glow: None,
                };
                draw_spot(c, &spot, f, frame, state, 0, sc);
            }
            None => s.draw(c, frame, Part::TaskLabel, state, sc),
        }
    }

    fn pixels(c: &Canvas, r: Rect) -> Vec<u32> {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect()
    }

    fn same_columns(a: &Canvas, b: &Canvas, xs: std::ops::Range<i32>) -> Result<(), (i32, i32)> {
        for y in 0..a.height as i32 {
            for x in xs.clone() {
                if a.get(x, y) != b.get(x, y) {
                    return Err((x, y));
                }
            }
        }
        Ok(())
    }

    #[test]
    fn a_labelled_glass_row_draws_each_frame_between_its_pairs() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let n = State::Normal;
        let rest = |st: State| (st, (st, EDGE_LEFT), (st, EDGE_RIGHT));
        let first = (n, (n, EDGE_ROW_START), (n, EDGE_RIGHT));
        let last = (n, (n, EDGE_LEFT), (n, EDGE_ROW_END));
        let beside = |st: State| {
            [
                (n, (n, EDGE_ROW_START), (st, EDGE_PREV_RIGHT)),
                rest(st),
                (n, (st, EDGE_NEXT_LEFT), (n, EDGE_ROW_END)),
            ]
        };
        let c = State::Checked;
        let [before, lit, after] = beside(State::Hot);
        let [_, held, held_after] = beside(State::Pressed);
        type Case = ([bool; 3], Option<usize>, Option<usize>, [Slot; 3]);
        let cases: [Case; 9] = [
            (
                [true, false, false],
                Some(1),
                None,
                [(c, (n, EDGE_ROW_START), (c, EDGE_RIGHT)), lit, after],
            ),
            (
                [false, false, true],
                Some(1),
                None,
                [before, lit, (c, (c, EDGE_LEFT), (n, EDGE_ROW_END))],
            ),
            (
                [true, false, false],
                None,
                Some(1),
                [(c, (n, EDGE_ROW_START), (c, EDGE_RIGHT)), held, held_after],
            ),
            ([false; 3], None, None, [first, rest(n), last]),
            ([false; 3], Some(1), None, beside(State::Hot)),
            (
                [false, true, false],
                None,
                None,
                [first, rest(State::Checked), last],
            ),
            (
                [false, true, false],
                Some(1),
                None,
                beside(State::CheckedHot),
            ),
            ([false; 3], None, Some(1), beside(State::Pressed)),
            (
                [false, true, false],
                Some(1),
                Some(1),
                beside(State::Pressed),
            ),
        ];
        let (s1, s2) = glass_at_1_and_2();
        for (active, hot, held, slots) in cases {
            for (sc, st) in [(1, &s1), (2, &s2)] {
                let lit = (hot, held, None);
                let (c, l) = glass_labelled_in(st, &[1, 1, 1], &active, lit, None, sc);
                let want = labelled_want(st, &l, &slots, sc);
                let row = l.tasks[0].x..l.tasks[2].right();
                assert_eq!(
                    same_columns(&c, &want, row),
                    Ok(()),
                    "scale {sc}: active {active:?} hot {hot:?} held {held:?}"
                );
            }
        }
        let edge = s.part(Part::TaskLabelEdge).unwrap();
        let label = s.part(Part::TaskLabel).unwrap();
        for (a, b) in [(State::Pressed, State::CheckedHot)] {
            let (fa, fb) = (label.frame(a), label.frame(b));
            assert_eq!(
                pixels(&label.atlas, fa),
                pixels(&label.atlas, fb),
                "a held frame is the active lit one"
            );
            for k in 0..4 {
                let (ea, eb) = (edge.frame_nth(a, k), edge.frame_nth(b, k));
                assert_eq!(
                    pixels(&edge.atlas, ea),
                    pixels(&edge.atlas, eb),
                    "held pair {k}"
                );
            }
        }
    }

    #[test]
    fn a_labelled_glass_row_ends_at_a_pinned_slot_and_at_stacked_edges() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let n = State::Normal;
        let (start, end) = ((n, EDGE_ROW_START), (n, EDGE_ROW_END));
        let (c, l) = glass_labelled(&[1, 0, 1, 1], &[false; 4], None, None, None, 1);
        let want = labelled_want(
            s,
            &l,
            &[
                (n, start, end),
                (n, start, start),
                (n, start, (n, EDGE_RIGHT)),
                (n, (n, EDGE_LEFT), end),
            ],
            1,
        );
        for i in [0, 2, 3] {
            let r = l.tasks[i];
            assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "slot {i}");
        }
        let (c, l) = glass_labelled(&[2, 1], &[false; 2], None, None, None, 1);
        let (front, next) = (l.tasks[0], l.tasks[1]);
        let mut want = over_veil(s, &l);
        let f = Rect::new(front.x, front.y, front.width - 4, front.height);
        s.draw(&mut want, label_frame(f, 1), Part::TaskLabel, n, 1);
        s.draw_nth(
            &mut want,
            Rect::new(f.right() - 3, 0, 3, 40),
            Part::TaskLabelEdge,
            n,
            EDGE_ROW_END,
            1,
        );
        s.draw_nth(
            &mut want,
            Rect::new(next.x, 0, 3, 40),
            Part::TaskLabelEdge,
            n,
            EDGE_ROW_START,
            1,
        );
        assert_eq!(
            same_columns(&c, &want, f.right() - 3..f.right()),
            Ok(()),
            "the group's front ends the run"
        );
        assert_eq!(
            same_columns(&c, &want, next.x..next.x + 3),
            Ok(()),
            "the next button starts one"
        );
    }

    #[test]
    fn a_lit_labelled_glass_button_glows_in_its_icons_colour() {
        let cyan = crate::theme::Color::rgb(113, 164, 168);
        let unlit = glass_unlit_spot();
        let base = &unlit.task_spot().unwrap().labelled.as_ref().unwrap().base;
        for (active, hot, held, state) in [
            ([false; 3], Some(1), None, State::Hot),
            ([false, true, false], Some(1), None, State::CheckedHot),
            ([false; 3], None, Some(1), State::Pressed),
        ] {
            let (c, l) = glass_labelled_in(
                &unlit,
                &[1, 1, 1],
                &active,
                (hot, held, None),
                Some(cyan),
                1,
            );
            let r = label_frame(l.tasks[1], 1);
            let mut want = over_veil(&unlit, &l);
            draw_image(&mut want, base, r, state, 1);
            assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "{state:?}");
            let (c, _) = glass_labelled(&[1, 1, 1], &active, hot, held, Some(cyan), 1);
            let (plain, _) = glass_labelled(&[1, 1, 1], &active, hot, held, None, 1);
            let [red, _, blue] = over_grey(c.get(r.x + 80, 36));
            let [pred, _, pblue] = over_grey(plain.get(r.x + 80, 36));
            assert!(
                blue - red > pblue - pred,
                "{state:?}: the foot glow leans to cyan ({red} {blue} against {pred} {pblue})"
            );
        }
    }

    #[test]
    fn a_lit_labelled_glass_buttons_pairs_glow_with_it() {
        let cyan = crate::theme::Color::rgb(113, 164, 168);
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (glowing, edges) = (
            s.task_glow.unwrap().label_edges.unwrap(),
            s.part(Part::TaskLabelEdge).unwrap(),
        );
        for (active, hot, held, state) in [
            ([false; 3], Some(1), None, State::Hot),
            ([false, true, false], Some(1), None, State::CheckedHot),
            ([false; 3], None, Some(1), State::Pressed),
        ] {
            let (c, l) = glass_labelled(&[1, 1, 1], &active, hot, held, Some(cyan), 1);
            let (plain, _) = glass_labelled(&[1, 1, 1], &active, hot, held, None, 1);
            let (before, lit, after) = (l.tasks[0], l.tasks[1], l.tasks[2]);
            let own = [(lit.x, EDGE_LEFT), (lit.right() - 3, EDGE_RIGHT)];
            let given = [
                (before.right() - 3, EDGE_PREV_RIGHT),
                (after.x, EDGE_NEXT_LEFT),
            ];
            let pairs = [own[0], own[1], given[0], given[1]];
            let mut want = over_veil(s, &l);
            for (x, n) in own {
                let one = glowing(edges, state, n, cyan);
                draw_image_nth(&mut want, &one, Rect::new(x, 0, 3, 40), state, 0, 1);
            }
            for (x, n) in given {
                s.draw_nth(
                    &mut want,
                    Rect::new(x, 0, 3, 40),
                    Part::TaskLabelEdge,
                    state,
                    n,
                    1,
                );
            }
            let lean = |c: &Canvas| -> i32 {
                let mut sum = 0;
                for (x, _) in pairs {
                    for y in 0..40 {
                        for dx in 0..3 {
                            let [red, _, blue] = over_grey(c.get(x + dx, y));
                            sum += blue - red;
                        }
                    }
                }
                sum
            };
            for (x, n) in pairs {
                assert_eq!(
                    same_columns(&c, &want, x..x + 3),
                    Ok(()),
                    "{state:?} pair {n} at {x}"
                );
            }
            assert!(
                lean(&c) > lean(&plain),
                "{state:?}: the pairs lean to cyan ({} against {})",
                lean(&c),
                lean(&plain)
            );
        }
    }

    #[test]
    fn labelled_glass_parts_keep_their_top_and_bottom_rows_on_a_bar_of_another_height() {
        let cyan = crate::theme::Color::rgb(113, 164, 168);
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let lit = s.task_spot().unwrap().labelled.as_ref().unwrap();
        let (base, gain, glowing, edges) = (
            &lit.base,
            &lit.gain,
            s.task_glow.unwrap().label_edges.unwrap(),
            s.part(Part::TaskLabelEdge).unwrap(),
        );
        let all = [
            State::Normal,
            State::Hot,
            State::Checked,
            State::CheckedHot,
            State::Pressed,
        ];
        type Piece<'a> = (String, Box<dyn Fn(&mut Canvas, i32) + 'a>);
        let mut pieces: Vec<Piece> = vec![(
            "veil".into(),
            Box::new(|c, h| s.draw(c, Rect::new(0, 0, 160, h), Part::LabelRow, State::Normal, 1)),
        )];
        for st in all {
            pieces.push((
                format!("{st:?} frame"),
                Box::new(move |c, h| s.draw(c, Rect::new(0, 0, 154, h), Part::TaskLabel, st, 1)),
            ));
            for n in 0..4 {
                pieces.push((
                    format!("{st:?} pair {n}"),
                    Box::new(move |c, h| {
                        s.draw_nth(c, Rect::new(0, 0, 3, h), Part::TaskLabelEdge, st, n, 1)
                    }),
                ));
            }
        }
        for st in [State::Hot, State::CheckedHot, State::Pressed] {
            pieces.push((
                format!("{st:?} frame under the spot"),
                Box::new(move |c, h| draw_image(c, base, Rect::new(0, 0, 154, h), st, 1)),
            ));
            pieces.push((
                format!("{st:?} spot's gain"),
                Box::new(move |c, h| draw_image(c, gain, Rect::new(0, 0, 154, h), st, 1)),
            ));
            for n in [EDGE_LEFT, EDGE_RIGHT] {
                let one = glowing(edges, st, n, cyan);
                pieces.push((
                    format!("glowing {st:?} pair {n}"),
                    Box::new(move |c, h| draw_image(c, &one, Rect::new(0, 0, 3, h), st, 1)),
                ));
            }
        }
        for (what, draw) in &pieces {
            let mut at40 = Canvas::new(160, 40);
            draw(&mut at40, 40);
            for h in [22, 34, 48] {
                let mut c = Canvas::new(160, h as u32);
                draw(&mut c, h);
                let rows = [
                    (0, 0),
                    (1, 1),
                    (2, 2),
                    (h - 3, 37),
                    (h - 2, 38),
                    (h - 1, 39),
                ];
                for (y, y40) in rows {
                    for x in 0..160 {
                        assert_eq!(
                            c.get(x, y),
                            at40.get(x, y40),
                            "{what} at {h} px: ({x}, {y}) is its row {y40} at 40 px"
                        );
                    }
                }
            }
        }
    }

    fn glass_at_1_and_2() -> (std::sync::Arc<Style>, std::sync::Arc<Style>) {
        let one = crate::glass::style(&Default::default());
        let two = one.at_scale(2);
        (one, two)
    }

    #[test]
    fn labelled_glass_buttons_over_grey_land_on_their_colours() {
        let (c, l) = glass_labelled(&[1, 1, 1], &[false, true, false], None, None, None, 1);
        let (lit, _) = glass_labelled(&[1, 1, 1], &[false; 3], Some(1), None, None, 1);
        let bar = over_veil(theme_for(Interface::Glass).look.style().unwrap(), &l);
        let at = |c: &Canvas, i: usize, x: i32, y: i32| over_grey(c.get(l.tasks[i].x + x, y));
        let got = [
            ("veil", over_grey(bar.get(1000, 20))),
            ("veil's top lines", over_grey(bar.get(1000, 1))),
            ("first button's dark line", at(&c, 0, 0, 20)),
            ("first button's light line", at(&c, 0, 1, 20)),
            ("sheen", at(&c, 0, 20, 5)),
            ("plain glass", at(&c, 0, 150, 20)),
            ("the line two buttons share", at(&c, 0, 159, 20)),
            ("active frame", at(&c, 1, 150, 20)),
            ("active foot", at(&c, 1, 80, 38)),
            ("lit foot", at(&lit, 1, 80, 36)),
            ("beside the lit one", at(&lit, 0, 158, 20)),
            ("last button's light line", at(&c, 2, 158, 20)),
            ("last button's dark line", at(&c, 2, 159, 20)),
        ];
        let want: [[i32; 3]; 13] = [
            [75, 88, 103],
            [166, 177, 188],
            [34, 51, 70],
            [130, 142, 154],
            [190, 196, 203],
            [99, 114, 129],
            [80, 96, 112],
            [156, 165, 175],
            [219, 222, 226],
            [224, 224, 225],
            [228, 230, 233],
            [132, 144, 156],
            [35, 52, 71],
        ];
        for ((what, rgb), want) in got.into_iter().zip(want) {
            assert!(
                (0..3).all(|i| (rgb[i] - want[i]).abs() <= 5),
                "{what} over grey is {want:?} within 5, got {rgb:?}"
            );
        }
    }

    #[test]
    fn a_held_glass_button_is_its_lit_active_frame_its_icon_a_pixel_in() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (lit, l) = glass_stacks_in([true, false, false], Some(0), None);
        let r = l.tasks[0];
        for active in [true, false] {
            let (held, _) = glass_stacks_in([active, false, false], None, Some(0));
            for (x, y) in [(2, 1), (30, 2), (55, 20), (30, 37)] {
                assert_eq!(
                    held.get(r.x + x, r.y + y),
                    lit.get(r.x + x, r.y + y),
                    "active {active}: the frame at ({x}, {y})"
                );
            }
            let white = 0xFFFF_FFFF;
            assert_eq!(lit.get(r.x + 14, r.y + 4), white);
            assert_ne!(held.get(r.x + 14, r.y + 4), white, "active {active}");
            assert_eq!(held.get(r.x + 15, r.y + 5), white, "active {active}");
            assert_eq!(held.get(r.x + 46, r.y + 36), white, "active {active}");
        }
        assert!(
            s.part(Part::TaskButton)
                .unwrap()
                .states
                .contains(&State::Pressed),
            "the button has a held frame"
        );
    }

    #[test]
    fn stacked_glass_edges_take_the_front_buttons_state() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let edges = |c: &Canvas, l: &TaskbarLayout| {
            let r = l.tasks[2];
            (0..8)
                .flat_map(|x| (0..40).map(move |y| (x, y)))
                .map(|(x, y)| c.get(r.x + 52 + x, r.y + y))
                .collect::<Vec<u32>>()
        };
        let want = |state: State| {
            let (_, l) = glass_stacks_in([false; 3], None, None);
            let r = l.tasks[2];
            let mut bar = Canvas::new(1920, 40);
            s.draw(
                &mut bar,
                Rect::new(0, 0, 1920, 40),
                Part::Taskbar,
                State::Normal,
                1,
            );
            for k in 0..2 {
                let e = Rect::new(r.x + 52 + 4 * k, r.y, 4, 40);
                s.draw_nth(&mut bar, e, Part::TaskStack, state, k as usize, 1);
            }
            edges(&bar, &l)
        };
        let cases = [
            ([false; 3], None, None, State::Normal),
            ([false; 3], Some(2), None, State::Hot),
            ([false, false, true], None, None, State::Checked),
            ([false, false, true], Some(2), None, State::CheckedHot),
            ([false; 3], None, Some(2), State::Pressed),
        ];
        let plain = crate::glass::build(&Default::default()).with_task_spot(None);
        let mut seen = Vec::new();
        for (active, hot, held, state) in cases {
            let (c, l) = glass_row_in(&plain, &[1, 2, 3], active, (hot, held, None), None, 1);
            let got = edges(&c, &l);
            assert_eq!(got, want(state), "{state:?} edges");
            seen.push(got);
        }
        assert_ne!(seen[0], seen[1], "lit edges differ from resting ones");
        assert_ne!(seen[0], seen[2], "active edges differ from resting ones");
        assert_eq!(seen[3], seen[4], "held edges are the active, lit pair");
    }

    #[test]
    fn an_idle_pin_draws_no_spot_where_its_slot_lights() {
        let glass = || crate::glass::build(&Default::default());
        assert!(task_draws_spot(&glass(), false), "a running button");
        assert!(
            !task_draws_spot(&glass(), true),
            "an idle pin draws its slot light"
        );
        let unlit = glass().without_part(Part::SlotLight);
        assert!(
            task_draws_spot(&unlit, true),
            "no slot light: the idle pin's button lights"
        );
        let plain = glass().with_task_spot(None);
        assert!(!task_draws_spot(&plain, false), "a style without the spot");
    }

    #[test]
    fn lit_and_held_glass_edges_draw_their_pairs_under_the_spot() {
        let (_, s2) = glass_at_1_and_2();
        let one = crate::glass::build(&Default::default());
        for (sc, s) in [(1, &one), (2, &*s2)] {
            lit_edges_at(s, sc);
        }
    }

    fn lit_edges_at(s: &Style, sc: i32) {
        let cyan = crate::theme::Color::rgb(113, 164, 168);
        let unlit = glass_unlit_spot_at(sc);
        let pairs = &unlit.task_spot().unwrap().edges.as_ref().unwrap().base;
        let mut bar = Canvas::new(1920 * sc as u32, 40 * sc as u32);
        s.draw(
            &mut bar,
            Rect::new(0, 0, 1920 * sc, 40 * sc),
            Part::Taskbar,
            State::Normal,
            sc,
        );
        let cases = [
            ([false; 3], Some(2), None, State::Hot),
            ([false, false, true], Some(2), None, State::CheckedHot),
            ([false; 3], None, Some(2), State::Pressed),
        ];
        for (active, hot, held, state) in cases {
            let lit = (hot, held, None);
            let (c, l) = glass_row_in(&unlit, &[1, 2, 3], active, lit, Some(cyan), sc);
            let r = l.tasks[2];
            for k in 0..2 {
                let f = pairs.frame_nth(state, k);
                for (x, y) in [(0, 5), (1, 20), (2, 30), (3, 38)] {
                    for (i, j) in (0..sc).flat_map(|i| (0..sc).map(move |j| (i, j))) {
                        let (dx, dy) = (sc * x + i, sc * y + j);
                        let at = r.x + sc * (52 + 4 * k as i32) + dx;
                        let mut want = Canvas::new(1, 1);
                        want.put_raw(0, 0, bar.get(at, r.y + dy));
                        let mut edge = Canvas::new(1, 1);
                        edge.put_raw(0, 0, pairs.atlas.get(f.x + dx, f.y + dy));
                        want.blit_over(0, 0, &edge);
                        assert_eq!(
                            c.get(at, r.y + dy),
                            want.get(0, 0),
                            "scale {sc}, {state:?} edge {k} at ({dx}, {dy}): the pair over the bar"
                        );
                    }
                }
            }
            let foot = |x: i32| {
                let lit = (hot, held, Some(x));
                let (c, l) = glass_row_in(s, &[1, 2, 3], active, lit, Some(cyan), sc);
                let r = l.tasks[2];
                (52 * sc..60 * sc)
                    .map(|dx| over_grey(c.get(r.x + dx, 34 * sc)).iter().sum::<i32>())
                    .sum::<i32>()
            };
            assert!(
                foot(59) > foot(0),
                "scale {sc}, {state:?}: the edges light under the pointer"
            );
        }
    }

    #[derive(Clone, Copy, Debug)]
    enum Hover {
        None,
        Hot,
        Held,
    }

    impl Hover {
        fn hot_held(self) -> (Option<usize>, Option<usize>) {
            match self {
                Hover::None => (None, None),
                Hover::Hot => (Some(0), None),
                Hover::Held => (None, Some(0)),
            }
        }
    }

    fn lum(px: u32) -> i32 {
        let [r, g, b] = over_grey(px);
        (r * 299 + g * 587 + b * 114) / 1000
    }

    fn glass_bar_with_idle_pin(hover: Hover, scale: i32) -> (Canvas, Rect) {
        let (hot, held) = hover.hot_held();
        let (c, l) = glass_row(&[0, 1, 1], [false; 3], hot, held, None, scale);
        (c, l.tasks[0])
    }

    /// What an idle pinned slot `r` lit in `state` draws at scale `sc` on a `w` x `h` bar through
    /// `s`: the bar, the band (if held) between the 2 px edges, the edges at both ends, the `light`
    /// (design px) foot light, then a `px` white icon at `icon`.
    #[allow(clippy::too_many_arguments)]
    fn idle_pin_want(
        s: &Style,
        (w, h): (i32, i32),
        r: Rect,
        state: State,
        light: (i32, i32),
        px: i32,
        icon: (i32, i32),
        sc: i32,
    ) -> Canvas {
        let mut c = Canvas::new(w as u32, h as u32);
        s.draw(
            &mut c,
            Rect::new(0, 0, w, h),
            Part::Taskbar,
            State::Normal,
            sc,
        );
        let e = 2 * sc;
        if state == State::Pressed {
            let band = Rect::new(r.x + e, r.y, r.width - 2 * e, r.height);
            s.draw(&mut c, band, Part::SlotBand, State::Pressed, sc);
        }
        for x in [r.x, r.right() - e] {
            s.draw(
                &mut c,
                Rect::new(x, r.y, e, r.height),
                Part::SlotEdge,
                state,
                sc,
            );
        }
        let foot = Rect::new(
            r.x + sc * ((r.width / sc - light.0) / 2),
            r.bottom() - light.1 * sc,
            light.0 * sc,
            light.1 * sc,
        );
        s.draw(&mut c, foot, Part::SlotLight, state, sc);
        c.fill_rect(
            Rect::new(icon.0, icon.1, px, px),
            crate::theme::Color::rgb(0xFF, 0xFF, 0xFF),
        );
        c
    }

    #[test]
    fn a_hot_idle_glass_pin_draws_its_edges_and_foot_light_only() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (canvas, r) = glass_bar_with_idle_pin(Hover::Hot, 1);
        let rest = glass_bar_with_idle_pin(Hover::None, 1).0;
        let mid = r.y + r.height / 2;
        assert_ne!(canvas.get(r.x, mid), rest.get(r.x, mid), "left edge");
        assert_ne!(
            canvas.get(r.right() - 1, mid),
            rest.get(r.right() - 1, mid),
            "right edge"
        );
        assert_eq!(
            canvas.get(r.x + 4, r.y + 2),
            rest.get(r.x + 4, r.y + 2),
            "no frame"
        );
        let foot = (r.x + r.width / 2, r.bottom() - 2);
        assert!(
            lum(canvas.get(foot.0, foot.1)) > lum(rest.get(foot.0, foot.1)) + 20,
            "foot light"
        );
        let want = idle_pin_want(
            s,
            (1920, 40),
            r,
            State::Hot,
            (60, 7),
            32,
            (r.x + 14, r.y + 4),
            1,
        );
        assert_eq!(same_columns(&canvas, &want, r.x..r.right()), Ok(()));
    }

    #[test]
    fn a_lit_idle_glass_pin_at_scale_2_draws_its_pieces_painted_at_2() {
        let (_, s2) = glass_at_1_and_2();
        for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
            let (hot, held) = hover.hot_held();
            let (c, l) = glass_row_in(&s2, &[0, 1, 1], [false; 3], (hot, held, None), None, 2);
            let r = l.tasks[0];
            let o = if state == State::Pressed { 2 } else { 0 };
            let icon = (r.x + 28 + o, r.y + 8 + o);
            let want = idle_pin_want(&s2, (3840, 80), r, state, (60, 7), 64, icon, 2);
            assert_eq!(same_columns(&c, &want, r.x..r.right()), Ok(()), "{hover:?}");
        }
    }

    #[test]
    fn a_held_idle_glass_pin_lights_its_slot_and_nudges_its_icon() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (held, r) = glass_bar_with_idle_pin(Hover::Held, 1);
        let hot = glass_bar_with_idle_pin(Hover::Hot, 1).0;
        let rest = glass_bar_with_idle_pin(Hover::None, 1).0;
        let (x, y) = (r.x + 7, r.y + r.height / 2);
        assert!(
            lum(held.get(x, y)) < lum(rest.get(x, y)) - 20,
            "the band darkens the slot"
        );
        let white = 0xFFFF_FFFF;
        assert_eq!(hot.get(r.x + 14, r.y + 4), white, "hot icon's top left");
        assert_ne!(held.get(r.x + 14, r.y + 4), white, "held icon moved off it");
        for (dx, dy) in [(14, 4), (45, 4), (14, 35), (45, 35)] {
            assert_eq!(
                held.get(r.x + dx + 1, r.y + dy + 1),
                hot.get(r.x + dx, r.y + dy),
                "icon corner ({dx}, {dy}) a pixel right and down"
            );
        }
        let want = idle_pin_want(
            s,
            (1920, 40),
            r,
            State::Pressed,
            (60, 7),
            32,
            (r.x + 15, r.y + 5),
            1,
        );
        assert_eq!(same_columns(&held, &want, r.x..r.right()), Ok(()));
        let cyan = crate::theme::Color::rgb(113, 164, 168);
        let (glowing, _) = glass_row(&[0, 1, 1], [false; 3], None, Some(0), Some(cyan), 1);
        assert_eq!(same_columns(&glowing, &held, r.x..r.right()), Ok(()));
    }

    #[test]
    fn a_lit_idle_glass_pin_on_the_small_bar_draws_the_small_pieces() {
        let small = crate::glass::small_bar_style(&Default::default());
        for sc in [1, 2] {
            small_idle_pin_at(&small.at_scale(sc), sc);
        }
    }

    fn small_idle_pin_at(s: &Style, sc: i32) {
        let theme = theme_for(Interface::Glass);
        let h = crate::glass::SMALL_TASKBAR_HEIGHT;
        let metrics = crate::theme::Metrics {
            taskbar_height: h,
            ..theme.metrics.clone()
        };
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &metrics,
            &s.taskbar,
            &[0, 1],
            crate::taskbar::TaskRow::Icons,
        );
        assert_eq!(layout.tasks[0].width, 44, "the small bar's slot");
        let layout = layout.scaled(sc);
        let theme = theme.scaled(sc);
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let px = s.taskbar.task_icon * sc;
        let r = layout.tasks[0];
        let (w, h) = (layout.width, layout.height);
        for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
            let mut st = state_with_tasks(&["", ""]);
            for t in &mut st.tasks {
                t.icon = Some(solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), px).into());
            }
            (st.task_hot, st.task_pressed) = hover.hot_held();
            let mut c = Canvas::new(w as u32, h as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
            let o = sc * i32::from(state == State::Pressed);
            let icon = (r.x + (r.width - px) / 2 + o, r.y + (h - px) / 2 + o);
            let want = idle_pin_want(s, (w, h), r, state, (44, 7), px, icon, sc);
            assert_eq!(
                same_columns(&c, &want, r.x..r.right()),
                Ok(()),
                "scale {sc}: {hover:?}"
            );
        }
    }

    fn small_bar_in(state: &str) -> Canvas {
        let s = crate::glass::small_bar_style(&Default::default());
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
            ..theme.metrics.clone()
        };
        let counts = TaskbarCounts {
            tasks: 5,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &metrics,
            &s.taskbar,
            &[0, 1, 0, 3, 1],
            crate::taskbar::TaskRow::Icons,
        );
        let mut st = state_with_tasks(&["", "", "", "", ""]);
        st.start_icon = Some(clear_start_logo());
        for t in &mut st.tasks {
            t.icon = Some(Canvas::new(16, 16).into());
        }
        let calc_active = matches!(state, "active_calc" | "hot_active_calc");
        st.tasks[3].active = !calc_active;
        st.tasks[4].active = calc_active;
        st.start_hot = state == "orb_hot";
        st.start_pressed = state == "orb_pressed";
        st.show_desktop_hot = state == "strip_hot";
        st.task_hot = match state {
            "hot_explorer" => Some(1),
            "hot_active_calc" => Some(4),
            _ => None,
        };
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, 30);
        draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
        let r = layout.start_orb(st.start_pressed).unwrap();
        let mut orb = Canvas::new(r.width as u32, r.height as u32);
        draw_start_above(&mut orb, &s, &layout, &st, 1);
        c.blit_over(r.x, r.y, &orb);
        c
    }

    #[test]
    fn the_small_glass_bar_lights_its_buttons_under_the_spot() {
        let spot: [(&str, i32, i32, [u8; 3]); 6] = [
            ("hot_explorer", 107, 2, [214, 217, 221]),
            ("hot_explorer", 127, 27, [233, 233, 233]),
            ("hot_explorer", 146, 15, [202, 205, 209]),
            ("hot_active_calc", 245, 2, [242, 242, 243]),
            ("hot_active_calc", 265, 27, [242, 242, 242]),
            ("hot_active_calc", 284, 15, [218, 219, 221]),
        ];
        let points = spot.map(|(state, x, y, want)| (state, x, y, want, 1));
        let mut drawn: Vec<(&str, Canvas)> = Vec::new();
        for (state, x, y, want, within) in points {
            if !drawn.iter().any(|(s, _)| *s == state) {
                drawn.push((state, small_bar_in(state)));
            }
            let c = &drawn.iter().find(|(s, _)| *s == state).unwrap().1;
            let got = over_grey(c.get(x, y));
            for (g, w) in got.iter().zip(want) {
                assert!(
                    (g - i32::from(w)).abs() <= within,
                    "{state}: ({x}, {y}) over grey is {want:?} within {within}, got {got:?}"
                );
            }
        }
        assert_eq!(drawn.len(), 2, "both lit states have points");
    }

    #[test]
    fn the_mark_stands_20_px_on_the_orb_centred_and_a_theme_logo_keeps_its_size() {
        let mut s = (*crate::glass::style(&Default::default())).clone();
        s.start_bloom = None;
        let mut st = state_with_tasks(&[]);
        let (with_mark, layout) = draw_glass(&s, 40, &st);
        st.start_icon = Some(Canvas::new(24, 24).into());
        let (without, _) = draw_glass(&s, 40, &st);
        let disc = layout.start_disc.unwrap_or(layout.start);
        let (cx, cy) = (disc.x + disc.width / 2, disc.y + disc.height / 2);
        let ink: Vec<(i32, i32)> = (0..40)
            .flat_map(|y| (disc.x..disc.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| with_mark.get(x, y) != without.get(x, y))
            .collect();
        let span = |f: fn(&(i32, i32)) -> i32| {
            let v = ink.iter().map(f);
            (v.clone().min().unwrap(), v.max().unwrap())
        };
        let ((x0, x1), (y0, y1)) = (span(|p| p.0), span(|p| p.1));
        assert!(
            x0 >= cx - 10 && x1 < cx + 10,
            "across {x0}..={x1} round {cx}"
        );
        assert!(y0 >= cy - 10 && y1 < cy + 10, "down {y0}..={y1} round {cy}");
        assert!(x1 - x0 >= 15 && y1 - y0 >= 15, "{x0}..={x1}, {y0}..={y1}");
        for d in [1, 2] {
            let l = layout.scaled(d);
            let mut st = state_with_tasks(&[]);
            let i = start_icon_box(&s, &l, &st, d);
            assert_eq!((i.width, i.height), (20 * d, 20 * d), "the mark at {d}");
            let disc = l.start_disc.unwrap_or(l.start);
            assert_eq!(2 * (i.x - disc.x) + i.width, disc.width, "centred at {d}");
            st.start_icon = Some(Canvas::new(24 * d as u32, 24 * d as u32).into());
            let i = start_icon_box(&s, &l, &st, d);
            assert_eq!(i.width, 24 * d, "a theme logo at its own size at {d}");
        }
    }

    #[test]
    fn a_lit_orb_glows_onto_the_bar_from_a_glow_built_once_per_state() {
        let s = crate::glass::build(&Default::default());
        let mut bare = s.clone();
        bare.start_bloom = None;
        let mut st = state_with_tasks(&[]);
        st.start_hot = true;
        let (lit, layout) = draw_glass(&s, 40, &st);
        let (plain, _) = draw_glass(&bare, 40, &st);
        // Left of the disc, on the bar: lighter with the glow, never darker.
        // The disc fills x 9..45 of Start, centred on row 20.
        let (x, y) = (layout.start.x + 8, layout.start.y + 20);
        let ch = |p: u32| [p >> 16, p >> 8, p].map(|c| c & 0xFF);
        let (a, b) = (ch(lit.get(x, y)), ch(plain.get(x, y)));
        assert!(a.iter().zip(&b).all(|(a, b)| a > b), "{a:?} over {b:?}");
        st.orb_glow = 0;
        assert_eq!(draw_glass(&s, 40, &st).0.pixels, plain.pixels, "orb_glow 0");
        st.orb_glow = 100;
        for (hot, pressed, look) in [
            (false, false, State::Normal),
            (true, false, State::Hot),
            (true, true, State::Pressed),
        ] {
            st.start_hot = hot;
            st.start_pressed = pressed;
            draw_glass(&s, 40, &st);
            let r = layout.start_orb(pressed).unwrap();
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            draw_start_above(&mut orb, &s, &layout, &st, 1);
            let kept = s.start_bloom.as_ref().unwrap();
            let px = start_icon_box(&s, &layout, &st, 1).width;
            kept.get(IconKey::Mark(px), 1, look, 100, || {
                panic!("{look:?} built again")
            });
        }
    }

    #[test]
    fn a_warm_paints_the_orb_glow_drawn_at_scale_1_for_scale_2() {
        let d = crate::glass::Settings::default();
        let s = Arc::new(crate::glass::build(&d).with_rescale(move |_| crate::glass::build(&d)));
        let mut st = state_with_tasks(&[]);
        st.start_hot = true;
        draw_glass(&s, 40, &st);
        let mut logo = Canvas::new(48, 48);
        logo.fill_rect(Rect::new(8, 8, 32, 32), Color::rgb(0xFF, 0x80, 0));
        for icon in [None, Some(Arc::new(logo))] {
            st.start_icon = icon.clone();
            draw_glass(&s, 40, &st);
            let warm = (s.warm(2, icon.clone())).expect("the scale 1 glow is unpainted at scale 2");
            std::thread::spawn(move || warm.paint()).join().unwrap();
            let two = s.at_scale(2);
            let icon = icon.as_deref();
            assert!(
                keeps_orb_bloom(&two, 2, icon, State::Hot, 100),
                "the scale 2 glow round the logo ({}) is painted before the first draw",
                icon.is_some()
            );
            assert!(
                !keeps_orb_bloom(&two, 2, icon, State::Normal, 100),
                "a state not drawn at scale 1 waits for its draw"
            );
        }
    }

    #[test]
    fn the_small_orb_centres_its_icon_on_the_disc_and_draws_its_rows_above() {
        let mut s = (*crate::glass::small_bar_style(&Default::default())).clone();
        s.start_bloom = None;
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
            ..theme.metrics.clone()
        };
        let layout = TaskbarLayout::compute(
            &metrics,
            &s.taskbar,
            1920,
            true,
            TaskbarCounts::default(),
            60,
        );
        let magenta = crate::theme::Color::rgb(0xFF, 0, 0xFF);
        let mut icon = Canvas::new(24, 24);
        icon.fill_rect(Rect::new(0, 0, 24, 24), magenta);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let bare = {
            let mut c = Canvas::new(1920, 30);
            s.draw(
                &mut c,
                Rect::new(0, 0, 1920, 30),
                Part::Taskbar,
                State::Normal,
                1,
            );
            c
        };
        for (hot, pressed, state) in [
            (false, false, State::Normal),
            (true, false, State::Hot),
            (false, true, State::Pressed),
        ] {
            let mut st = state_with_tasks(&[]);
            st.start_icon = Some(icon.clone().into());
            st.start_hot = hot;
            st.start_pressed = pressed;
            let mut bar = Canvas::new(1920, 30);
            draw_taskbar(&mut bar, &s, &theme, &text, &start, &layout, &st);
            assert_eq!(
                bar.get(26, 13) == bare.get(26, 13),
                pressed,
                "{state:?}: the bar draws the orb unless held"
            );
            let r = layout.start_orb(pressed).unwrap();
            let want = if pressed {
                (0, -13, 54, 43)
            } else {
                (0, -13, 54, 13)
            };
            assert_eq!((r.x, r.y, r.width, r.height), want, "{state:?}: its buffer");
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            draw_start_above(&mut orb, &s, &layout, &st, 1);
            let mut c = Canvas::new(1920, 43);
            c.blit_canvas(0, 13, &bar);
            c.blit_over(r.x, r.y + 13, &orb);
            let ink = magenta.argb();
            let at = |x, y| c.get(x, y + 13);
            assert_eq!(
                at(15, 1),
                ink,
                "{state:?}: the icon's top-left is at (15, 1)"
            );
            assert_eq!(
                at(38, 24),
                ink,
                "{state:?}: its bottom-right is at (38, 24)"
            );
            assert_ne!(
                at(15, 25),
                ink,
                "{state:?}: (15, 25), under the icon, is not its ink"
            );
            assert_ne!(
                at(15, 0),
                ink,
                "{state:?}: (15, 0), over the icon, is not its ink"
            );

            let p = s.part(Part::StartAbove).unwrap();
            let f = p.frame(state);
            for (x, y) in [(26, 8), (20, 12), (8, 11), (17, 7), (45, 12)] {
                assert_eq!(
                    orb.get(x, y),
                    p.atlas.get(f.x + x, f.y + y),
                    "{state:?}: ({x}, {y}) is the part's pixel there"
                );
            }
        }

        let full = crate::glass::style(&Default::default());
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &full.taskbar,
            1920,
            true,
            TaskbarCounts::default(),
            60,
        );
        assert_eq!(
            l.start_above,
            Some(Rect::new(0, -6, 54, 6)),
            "54x6 above the 40 px bar"
        );
        for (hot, pressed, lit) in [
            (false, false, false),
            (true, false, true),
            (false, true, true),
        ] {
            let mut st = state_with_tasks(&[]);
            st.start_hot = hot;
            st.start_pressed = pressed;
            let r = l.start_orb(pressed).unwrap();
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            draw_start_above(&mut orb, &full, &l, &st, 1);
            assert_eq!(
                orb.get(17, 4) >> 24 != 0,
                lit,
                "hot {hot}, held {pressed}: a glow at (17, -2) is {lit}"
            );
        }
    }

    #[test]
    fn under_a_top_bar_the_small_orb_hangs_upright_with_its_glow_below() {
        let mut s = (*crate::glass::small_bar_style(&Default::default())).clone();
        s.start_bloom = None;
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: crate::glass::SMALL_TASKBAR_HEIGHT,
            ..theme.metrics.clone()
        };
        let layout = TaskbarLayout::compute(
            &metrics,
            &s.taskbar,
            1920,
            true,
            TaskbarCounts::default(),
            60,
        )
        .on_edge(Edge::Top);
        let magenta = crate::theme::Color::rgb(0xFF, 0, 0xFF);
        let icon = solid_icon(magenta, 24);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let button = s.part(Part::StartButton).unwrap();
        let above = s.part(Part::StartAbove).unwrap();
        for (hot, pressed, state) in [
            (false, false, State::Normal),
            (true, false, State::Hot),
            (false, true, State::Pressed),
        ] {
            let mut st = state_with_tasks(&[]);
            st.start_icon = Some(icon.clone().into());
            st.start_hot = hot;
            st.start_pressed = pressed;
            let mut bar = Canvas::new(1920, 30);
            draw_taskbar(&mut bar, &s, &theme, &text, &start, &layout, &st);
            let r = layout.start_orb(pressed).unwrap();
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            draw_start_above(&mut orb, &s, &layout, &st, 1);
            let mut c = Canvas::new(1920, 43);
            c.blit_over(0, 0, &bar);
            c.blit_over(r.x, r.y, &orb);

            let mut want = Canvas::new(1920, 43);
            let full = Rect::new(0, 0, 1920, 30);
            want.with_rows_flipped(full, |w| s.draw(w, full, Part::Taskbar, State::Normal, 1));
            s.draw(
                &mut want,
                Rect::new(0, -8, 54, 13),
                Part::StartAbove,
                state,
                1,
            );
            s.draw(
                &mut want,
                Rect::new(0, 5, 54, 30),
                Part::StartButton,
                state,
                1,
            );
            want.blit_canvas(15, 6, &icon);
            let (bf, af) = (button.frame(state), above.frame(state));
            for x in 0..54 {
                for y in 0..35 {
                    assert_eq!(
                        c.get(x, y),
                        want.get(x, y),
                        "{state:?}: ({x}, {y}) is the orb drawn upright 5 rows lower"
                    );
                }
                for y in 30..35 {
                    assert_eq!(
                        orb.get(x, y - r.y),
                        button.atlas.get(bf.x + x, bf.y + y - 5),
                        "{state:?}: ({x}, {y}) is the button's row {}",
                        y - 5
                    );
                }
                for y in 35..43 {
                    assert_eq!(
                        orb.get(x, y - r.y),
                        above.atlas.get(af.x + x, af.y + 42 - y),
                        "{state:?}: ({x}, {y}) is the row {} above turned over",
                        42 - y
                    );
                }
            }
            let ink = magenta.argb();
            assert_eq!(
                c.get(15, 6),
                ink,
                "{state:?}: the icon's top-left at (15, 6)"
            );
            assert_eq!(
                c.get(38, 29),
                ink,
                "{state:?}: its bottom-right at (38, 29)"
            );
            assert_ne!(c.get(15, 5), ink, "{state:?}: (15, 5) is over the icon");
            assert_ne!(c.get(15, 30), ink, "{state:?}: (15, 30) is under the icon");
        }

        let mut full = (*crate::glass::style(&Default::default())).clone();
        full.start_bloom = None;
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &full.taskbar,
            1920,
            true,
            TaskbarCounts::default(),
            60,
        )
        .on_edge(Edge::Top);
        let above = full.part(Part::StartAbove).unwrap();
        for (hot, pressed, state) in [
            (false, false, State::Normal),
            (true, false, State::Hot),
            (false, true, State::Pressed),
        ] {
            let mut st = state_with_tasks(&[]);
            st.start_hot = hot;
            st.start_pressed = pressed;
            let r = l.start_orb(pressed).unwrap();
            let mut orb = Canvas::new(r.width as u32, r.height as u32);
            draw_start_above(&mut orb, &full, &l, &st, 1);
            let f = above.frame(state);
            for x in 0..54 {
                for y in 40..46 {
                    assert_eq!(
                        orb.get(x, y - r.y),
                        above.atlas.get(f.x + x, f.y + 45 - y),
                        "{state:?}: ({x}, {y}) is the row {} above turned over",
                        45 - y
                    );
                }
            }
        }
    }

    #[test]
    fn an_icon_only_glass_button_centres_its_icon() {
        for (s, height, px) in [
            (crate::glass::style(&Default::default()), 40, 32),
            (crate::glass::small_bar_style(&Default::default()), 30, 16),
        ] {
            assert_eq!(s.taskbar.task_icon, px, "{height} px bar's icon size");
            let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), px);
            let ink = magenta.get(0, 0);
            let theme = theme_for(Interface::Glass);
            let metrics = crate::theme::Metrics {
                taskbar_height: height,
                ..theme.metrics.clone()
            };
            let counts = TaskbarCounts {
                tasks: 2,
                ..TaskbarCounts::default()
            };
            let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
            layout.lay_tasks(
                &metrics,
                &s.taskbar,
                &[1, 3],
                crate::taskbar::TaskRow::Icons,
            );
            let mut st = state_with_tasks(&["", ""]);
            for t in &mut st.tasks {
                t.icon = Some(magenta.clone().into());
            }
            let text = TextRenderer::load(&FontConfig::default());
            let start = TextRenderer::load(&s.start_text.font());
            let mut c = Canvas::new(1920, height as u32);
            draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
            for (r, front) in [(layout.tasks[0], 0), (layout.tasks[1], 8)] {
                let w = r.width - front;
                let (x, y) = (r.x + (w - px) / 2, r.y + (height - px) / 2);
                assert_eq!(c.get(x, y), ink, "{height} px bar: top-left at ({x}, {y})");
                let far = px - 1;
                assert_eq!(
                    c.get(x + far, y + far),
                    ink,
                    "{height} px bar: bottom-right"
                );
                assert_ne!(c.get(x - 1, y), ink, "{height} px bar: left of the icon");
                assert_ne!(c.get(x + px, y), ink, "{height} px bar: right of the icon");
            }
        }
    }

    #[test]
    fn a_labelled_glass_buttons_icon_centres_in_its_32_px_box_10_px_in() {
        let s = crate::glass::style(&Default::default());
        let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
        let ink = magenta.get(0, 0);
        let theme = theme_for(Interface::Glass);
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &theme.metrics,
            &s.taskbar,
            &[1, 1],
            crate::taskbar::TaskRow::AtLeast(62),
        );
        let mut st = state_with_tasks(&["", "Notes"]);
        for t in &mut st.tasks {
            t.icon = Some(magenta.clone().into());
        }
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, 40);
        draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
        for (i, r) in layout.tasks.iter().enumerate() {
            assert!(r.width > 100, "button {i} is a labelled one");
            let (x, y) = (r.x + 10, r.y + 4);
            assert_eq!(
                c.get(x, y),
                ink,
                "button {i}: icon's top-left at ({x}, {y})"
            );
            assert_ne!(c.get(x - 1, y), ink, "button {i}: left of the icon");
        }
    }

    #[test]
    fn a_narrow_labelled_glass_button_keeps_its_icon_inside_it() {
        let s = crate::glass::style(&Default::default());
        let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
        let ink = magenta.get(0, 0);
        let theme = theme_for(Interface::Glass);
        let counts = TaskbarCounts {
            tasks: 4,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &theme.metrics,
            &s.taskbar,
            &[1, 1, 1, 1],
            crate::taskbar::TaskRow::Shared,
        );
        layout.tasks = [(100, 36), (140, 12), (156, 48), (220, 2)]
            .map(|(x, w)| Rect::new(x, 0, w, 40))
            .to_vec();
        let mut st = state_with_tasks(&["", "", "", ""]);
        for t in &mut st.tasks {
            t.icon = Some(magenta.clone().into());
        }
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, 40);
        draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
        let inked = |r: &Rect| pixels(&c, *r).iter().filter(|&&p| p == ink).count();
        let counts: Vec<usize> = layout.tasks.iter().map(inked).collect();
        assert_eq!(
            counts,
            [1024, 0, 1024, 0],
            "whole icons, each inside its button"
        );
        assert_eq!(c.get(102, 4), ink, "the narrow button's icon is centred");
        assert_ne!(c.get(101, 4), ink, "left of the centred icon");
        assert_eq!(c.get(156 + 10, 4), ink, "the wide one's sits in its box");
        let bar = over_veil(&s, &layout);
        for x in [219, 222] {
            assert_eq!(
                same_columns(&c, &bar, x..x + 1),
                Ok(()),
                "a 2 px button's pairs stay inside it"
            );
        }
    }

    #[test]
    fn a_squeezed_small_bar_button_centres_its_icon() {
        let magenta = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 16);
        let ink = magenta.get(0, 0);
        let s = crate::glass::small_bar_style(&Default::default());
        let theme = theme_for(Interface::Glass);
        let metrics = crate::theme::Metrics {
            taskbar_height: 30,
            ..theme.metrics.clone()
        };
        let counts = TaskbarCounts {
            tasks: 3,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&metrics, &s.taskbar, 1920, true, counts, 60);
        layout.lay_tasks(
            &metrics,
            &s.taskbar,
            &[1, 1, 1],
            crate::taskbar::TaskRow::Shared,
        );
        layout.tasks = [(100, 20), (120, 12), (132, 48)]
            .map(|(x, w)| Rect::new(x, 0, w, 30))
            .to_vec();
        let mut st = state_with_tasks(&["", "", ""]);
        for t in &mut st.tasks {
            t.icon = Some(magenta.clone().into());
        }
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(1920, 30);
        draw_taskbar(&mut c, &s, &theme, &text, &start, &layout, &st);
        let inked = |r: &Rect| pixels(&c, *r).iter().filter(|&&p| p == ink).count();
        let counts: Vec<usize> = layout.tasks.iter().map(inked).collect();
        assert_eq!(counts, [256, 0, 256], "whole icons, each inside its button");
        assert_eq!(c.get(102, 7), ink, "the 20 px button's icon is centred");
        assert_ne!(c.get(101, 7), ink, "left of the centred icon");
        let lb = s.taskbar.labelled;
        let x = 132 + lb.icon_x + (lb.icon_box - 16) / 2;
        assert_eq!(c.get(x, 7), ink, "the wide one's sits in its box");
    }

    #[test]
    fn classic_and_blue_have_no_stacked_edges() {
        for i in [Interface::Classic, Interface::Blue] {
            let theme = theme_for(i);
            if let Some(s) = theme.look.style() {
                assert!(s.part(Part::TaskStack).is_none(), "{i:?} stacks nothing");
            }
        }
    }

    #[test]
    fn a_hot_glass_button_glows_in_its_icons_colour() {
        use crate::theme::Color;
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let glow = s.task_glow.expect("Glass buttons glow in colour");
        let yellow = solid_icon(Color::rgb(0xF0, 0xC8, 0x40), 32);
        let blue = solid_icon(Color::rgb(0x30, 0x70, 0xD0), 32);
        let hot = |icon: &Canvas, with_glow: bool| {
            let mut st = state_with_tasks(&["One", "Two"]);
            st.tasks[1].active = true;
            for t in &mut st.tasks {
                t.icon = Some(icon.clone().into());
                t.glow = if with_glow { (glow.colour)(icon) } else { None };
            }
            st.task_hot_x = Some(100);
            let (a, l) = {
                st.task_hot = Some(0);
                draw_glass(s, 40, &st)
            };
            st.task_hot = Some(1);
            let (b, _) = draw_glass(s, 40, &st);
            let at = |c: &Canvas, r: Rect| over_grey(c.get(r.x + 100, r.y + 34));
            (at(&a, l.tasks[0]), at(&b, l.tasks[1]))
        };
        let grey = hot(&yellow, false);
        let (y_hot, y_checked) = hot(&yellow, true);
        let (b_hot, b_checked) = hot(&blue, true);
        for ([r, g, b], w) in [(y_hot, grey.0), (y_checked, grey.1)] {
            assert!(
                r > b + 20 && g > b + 10,
                "yellow icon: ({r}, {g}, {b}), no colour {w:?}"
            );
        }
        for ([r, g, b], w) in [(b_hot, grey.0), (b_checked, grey.1)] {
            assert!(
                b > g && b - r > w[2] - w[0] + 15,
                "blue icon: ({r}, {g}, {b}), no colour {w:?}"
            );
        }
    }

    #[test]
    fn the_spot_follows_the_pointer() {
        // The third button, clear of the denser glass under the orb, so
        // the bar under it is even. At scale 2, over the frames painted at
        // 2, it lies where it does at 1, in device px twice over.
        let (_, s2) = glass_at_1_and_2();
        let one = crate::glass::build(&Default::default());
        let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
        for (sc, s) in [(1, &one), (2, &*s2)] {
            let unlit = glass_unlit_spot_at(sc);
            for x in [6, 30, 54] {
                let lit = (Some(2), None, Some(x));
                let (c, l) = glass_row_in(s, &[1, 1, 1], [false; 3], lit, Some(yellow), sc);
                let (bare, _) = glass_row_in(&unlit, &[1, 1, 1], [false; 3], lit, Some(yellow), sc);
                let got = brightest(&c, &bare, l.tasks[2].x, 3 * sc..57 * sc, 36 * sc);
                assert!(
                    (got / sc - x.clamp(3, 56)).abs() <= 3,
                    "scale {sc}, the pointer at {x}: the foot is brightest at {got}"
                );
            }
        }
    }

    #[test]
    fn the_spot_stays_inside_the_frame() {
        let s = crate::glass::build(&Default::default());
        let unlit = glass_unlit_spot();
        let spot = s.task_spot().unwrap();
        let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
        let states = [
            (State::Hot, false, None),
            (State::CheckedHot, true, None),
            (State::Pressed, false, Some(0)),
        ];
        for scale in [1, 2] {
            for (state, active, held) in states {
                for (labelled, stacks) in
                    [(false, [1, 1, 1]), (false, [2, 1, 1]), (true, [1, 1, 1])]
                {
                    for x in [0, 59] {
                        let lit = (Some(0), held, Some(x));
                        let act = [active, false, false];
                        let row = |st: &Style| {
                            if labelled {
                                glass_labelled_in(st, &stacks, &act, lit, Some(yellow), scale)
                            } else {
                                glass_row_in(st, &stacks, act, lit, Some(yellow), scale)
                            }
                        };
                        let (c, l) = row(&s);
                        let (bare, _) = row(&unlit);
                        let what = format!(
                            "{state:?}, labelled {labelled}, scale {scale}, stacks {stacks:?}, pointer at {x}"
                        );
                        let mut frame = Canvas::new(l.width as u32, l.height as u32);
                        let mut edges = Vec::new();
                        if labelled {
                            let f = spot.labelled.as_ref().unwrap();
                            draw_image(
                                &mut frame,
                                &f.base,
                                label_frame(l.tasks[0], scale),
                                state,
                                scale,
                            );
                        } else {
                            let front;
                            (front, edges) =
                                stacked(&s, l.tasks[0], l.stacks.first().copied(), scale);
                            draw_image(&mut frame, &spot.button.base, front, state, scale);
                            let pairs = &spot.edges.as_ref().unwrap().base;
                            for (k, e) in edges.iter().enumerate() {
                                draw_image_nth(&mut frame, pairs, *e, state, k, scale);
                            }
                        }
                        let mut lit_px = 0;
                        for y in 0..l.height {
                            for px in 0..l.width {
                                if c.get(px, y) != bare.get(px, y) {
                                    lit_px += 1;
                                    assert_ne!(
                                        frame.get(px, y) >> 24,
                                        0,
                                        "{what}: ({px}, {y}) lies outside the frame"
                                    );
                                }
                            }
                        }
                        assert!(lit_px > 0, "{what}: the spot shows");
                        if stacks[0] == 2 {
                            let e = edges[0];
                            let edge_lit = (e.x..e.right())
                                .any(|px| (0..l.height).any(|y| c.get(px, y) != bare.get(px, y)));
                            assert!(edge_lit, "{what}: the spot lights the stacked edge");
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_spot_at_the_middle_lands_on_its_colours() {
        let s = crate::glass::build(&Default::default());
        let glow = crate::theme::Color::rgb(191, 200, 150);
        type Points = [((i32, i32), [i32; 3]); 6];
        let want: [(bool, Points); 2] = [
            (
                false,
                [
                    ((30, 36), [236, 233, 206]),
                    ((2, 37), [228, 226, 193]),
                    ((57, 11), [198, 202, 189]),
                    ((10, 34), [216, 214, 177]),
                    ((30, 2), [194, 199, 196]),
                    ((50, 38), [246, 243, 227]),
                ],
            ),
            (
                true,
                [
                    ((30, 36), [245, 241, 225]),
                    ((2, 37), [240, 236, 205]),
                    ((57, 11), [226, 226, 205]),
                    ((10, 34), [232, 229, 194]),
                    ((30, 2), [229, 231, 228]),
                    ((50, 38), [254, 254, 254]),
                ],
            ),
        ];
        for (active, points) in want {
            let lit = (Some(0), None, Some(30));
            let active = [active, false, false];
            let (c, l) = glass_row_in(&s, &[1, 1, 1], active, lit, Some(glow), 1);
            let r = l.tasks[0];
            for ((x, y), rgb) in points {
                let got = over_grey(c.get(r.x + x, r.y + y));
                assert!(
                    (0..3).all(|i| (got[i] - rgb[i]).abs() <= 1),
                    "active {}: ({x}, {y}) is {got:?}, want {rgb:?}",
                    active[0]
                );
            }
            let middle = (Some(0), None, None);
            let (centred, _) = glass_row_in(&s, &[1, 1, 1], active, middle, Some(glow), 1);
            assert_eq!(
                same_columns(&c, &centred, r.x..r.right()),
                Ok(()),
                "a lit button with no pointer x draws the spot at its middle"
            );
        }
    }

    #[test]
    fn a_labelled_spot_follows_across_the_label() {
        let s = crate::glass::build(&Default::default());
        let unlit = glass_unlit_spot();
        let yellow = crate::theme::Color::rgb(0xF0, 0xC8, 0x40);
        let draw = |s: &Style, x: i32| {
            let lit = (Some(1), None, Some(x));
            glass_labelled_in(s, &[1, 1, 1], &[false; 3], lit, Some(yellow), 1)
        };
        let ((left, l), (right, _)) = (draw(&s, 20), draw(&s, 140));
        let r = l.tasks[1];
        assert_eq!(r.width, 160, "a 160 px labelled button");
        let foot = |c: &Canvas, cols: std::ops::Range<i32>| -> i32 {
            cols.flat_map(|x| (33..37).map(move |y| (x, y)))
                .map(|(x, y)| over_grey(c.get(r.x + x, r.y + y)).iter().sum::<i32>())
                .sum()
        };
        assert!(
            foot(&left, 6..46) > foot(&right, 6..46),
            "the left end lights under the pointer"
        );
        assert!(
            foot(&right, 114..154) > foot(&left, 114..154),
            "the right end lights under it"
        );
        let (bare, _) = draw(&unlit, 140);
        let got = brightest(&right, &bare, r.x, 48..154, 22);
        assert!(
            (got - 140).abs() <= 3,
            "the pointer at 140: the middle row is brightest at {got}"
        );
    }

    #[test]
    fn a_grey_icon_lights_a_neutral_spot() {
        use crate::style::{Layout, SpotFrames};
        use crate::theme::Color;
        let s = crate::glass::build(&Default::default());
        let fit = s.task_spot().unwrap().fit;
        assert_eq!(
            fit.halo.colour(None),
            [136.0; 3],
            "the halo: its white's luma"
        );
        assert_eq!(
            fit.core.colour(None),
            [250.0; 3],
            "the core: its white's luma"
        );
        let grey = Some(Color::rgb(0x77, 0x77, 0x77));
        assert_eq!(fit.halo.colour(grey), [136.0; 3], "a grey glow has no hue");
        let mut gain = Canvas::new(60, 40);
        gain.fill_rect(gain.bounds(), Color::rgb(0, 0, 32));
        let frames = SpotFrames {
            base: PartImage::new(Canvas::new(60, 40), &[State::Hot], Layout::Vertical),
            gain: PartImage::new(gain, &[State::Hot], Layout::Vertical),
        };
        let r = Rect::new(0, 0, 60, 40);
        let lit = |glow: Option<Color>| {
            let span = Span {
                x: 0,
                right: 60,
                pointer: None,
            };
            let spot = Spot {
                fit: &fit,
                span,
                glow,
            };
            let mut c = Canvas::new(60, 40);
            draw_spot(&mut c, &spot, &frames, r, State::Hot, 0, 1);
            c
        };
        let c = lit(None);
        for y in 0..40 {
            for x in 0..60 {
                let px = c.get(x, y);
                let [r, g, b] = [16, 8, 0].map(|sh| (px >> sh) & 0xFF);
                assert!(r == g && g == b, "({x}, {y}) is grey: {px:08x}");
            }
        }
        assert!(c.get(32, 34) >> 24 > 200, "the foot is lit");
        let warm = lit(Some(Color::rgb(0xF0, 0xC8, 0x40))).get(32, 34);
        assert!(
            (warm >> 16 & 0xFF) > (warm & 0xFF) + 20,
            "a yellow icon's spot is warm: {warm:08x}"
        );
    }

    #[test]
    fn a_checked_glass_buttons_icon_and_title_are_not_pushed_in() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        assert!(!s.task_nudge);
        let icon = solid_icon(crate::theme::Color::rgb(0xFF, 0, 0xFF), 32);
        let magenta = icon.get(0, 0);
        let mut st = state_with_tasks(&["Notes", "Notes"]);
        st.tasks[1].active = true;
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
        }
        let (c, l) = draw_glass(s, 40, &st);
        let icon_px = s.taskbar.task_icon;
        for (i, r) in l.tasks.iter().enumerate() {
            let (x, y) = (r.x + 10, r.y + (r.height - icon_px) / 2);
            assert_eq!(c.get(x, y), magenta, "task {i}'s icon at ({x}, {y})");
            assert_ne!(c.get(x - 1, y), magenta, "task {i} left of its icon");
            assert_ne!(c.get(x, y - 1), magenta, "task {i} above its icon");
        }
        let ink = |r: Rect| {
            let white = s.bar_text.argb();
            let inked: Vec<(i32, i32)> = (r.y + 5..r.bottom() - 5)
                .flat_map(|y| (r.x + 30..r.right()).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == white)
                .collect();
            let left = inked.iter().map(|&(x, _)| x - r.x).min();
            let top = inked.iter().map(|&(_, y)| y - r.y).min();
            (left, top)
        };
        assert!(ink(l.tasks[0]).0.is_some(), "the title inks");
        assert_eq!(ink(l.tasks[0]), ink(l.tasks[1]));
    }

    fn clock_rows(c: &Canvas, bare: &Canvas, k: Rect) -> Vec<i32> {
        (k.y..k.bottom())
            .filter(|&y| (k.x..k.right()).any(|x| c.get(x, y) != bare.get(x, y)))
            .collect()
    }

    #[test]
    fn a_taller_bar_centres_the_two_clock_lines() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let clock = "12:34\n27.09.2026";
        let at = |h: i32| {
            let (bare, l) = glass_bar_in(s, h, "", None);
            let (c, _) = glass_bar_in(s, h, clock, None);
            clock_rows(&c, &bare, l.clock)
        };
        let (forty, sixty) = (at(40), at(60));
        assert!(!forty.is_empty(), "the clock inks");
        let moved: Vec<i32> = forty.iter().map(|y| y + 10).collect();
        assert_eq!(sixty, moved, "on a 60 px bar the pair sits 10 px lower");
    }

    #[test]
    fn a_glass_start_is_the_orb_in_any_colour_and_intensity() {
        let other = crate::glass::Settings {
            color: crate::theme::Color::rgb(206, 15, 15),
            intensity: 80,
            ..crate::glass::Settings::default()
        };
        let s = crate::glass::style(&other);
        assert!(s.taskbar.orb, "an orb at intensity 80");
        let (c, _) = glass_bar_in(&s, 40, "", None);
        let (d, _) = glass_bar("", None);
        assert_ne!(
            c.get(26, 25),
            d.get(26, 25),
            "the orb takes the glass colour"
        );
        assert_ne!(c.get(200, 20), d.get(200, 20), "the bar's glass does not");
    }

    #[test]
    fn a_glass_bar_draws_its_bar_orb_and_buttons_from_the_art() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, l) = glass_bar("", Some(Canvas::new(24, 24)));
        let part = |part, state, r: Rect, x, y| {
            let mut w = Canvas::new(1920, 40);
            s.draw(
                &mut w,
                Rect::new(0, 0, 1920, 40),
                Part::Taskbar,
                State::Normal,
                1,
            );
            s.draw(&mut w, r, part, state, 1);
            w.get(x, y)
        };
        let mut alone = Canvas::new(1920, 40);
        s.draw(
            &mut alone,
            Rect::new(0, 0, 1920, 40),
            Part::Taskbar,
            State::Normal,
            1,
        );
        let veiled = over_veil(s, &l);
        assert_eq!(c.get(1000, 20), veiled.get(1000, 20), "the bar's glass");
        assert_ne!(c.get(1000, 20), alone.get(1000, 20), "veiled under the row");
        assert_eq!(
            c.get(26, 19),
            part(Part::StartButton, State::Normal, l.start, 26, 19),
            "the orb, with no label on it"
        );
        for (i, state) in [State::Normal, State::Checked, State::Hot]
            .into_iter()
            .enumerate()
        {
            let r = l.tasks[i];
            let mut want = over_veil(s, &l);
            label_in(&mut want, s, label_frame(r, 1), state, 1);
            assert_eq!(
                c.get(r.x + 60, r.y + 1),
                want.get(r.x + 60, r.y + 1),
                "button {i} is {state:?}"
            );
        }
    }

    #[test]
    fn the_orbs_icon_sits_centred_on_it() {
        let mut icon = Canvas::new(24, 24);
        icon.fill_rect(
            Rect::new(0, 0, 24, 24),
            crate::theme::Color::rgb(0xFF, 0, 0xFF),
        );
        let magenta = icon.get(0, 0);
        let (c, _) = glass_bar("", Some(icon));
        assert_eq!(c.get(15, 8), magenta, "top-left at (15, 8)");
        assert_eq!(c.get(38, 31), magenta, "bottom-right at (38, 31)");
        assert_ne!(c.get(14, 8), magenta);
        assert_ne!(c.get(15, 7), magenta);
    }

    #[test]
    fn a_two_line_clock_sets_the_time_over_the_date() {
        let (bare, l) = glass_bar("", None);
        let (c, _) = glass_bar("12:34\n27.09.2026", None);
        let k = l.clock;
        let ink: Vec<(i32, i32)> = (k.y..k.bottom())
            .flat_map(|y| (k.x..k.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) != bare.get(x, y))
            .collect();
        let rows = |lo: i32, hi: i32| -> Vec<(i32, i32)> {
            ink.iter()
                .copied()
                .filter(|&(_, y)| y >= lo && y < hi)
                .collect()
        };
        let (time, date) = (rows(0, 20), rows(20, 40));
        assert!(!time.is_empty() && !date.is_empty(), "both lines ink");
        let bottom = |v: &[(i32, i32)]| v.iter().map(|&(_, y)| y).max().unwrap();
        assert!(
            (15..=17).contains(&bottom(&time)),
            "the time sits on row 17"
        );
        assert!(
            (32..=34).contains(&bottom(&date)),
            "the date sits on row 34"
        );
        let centre = |v: &[(i32, i32)]| {
            let lo = v.iter().map(|&(x, _)| x).min().unwrap();
            let hi = v.iter().map(|&(x, _)| x).max().unwrap();
            lo + hi
        };
        let mid = 2 * k.x + k.width;
        assert!((centre(&time) - mid).abs() <= 3, "the time is centred");
        assert!((centre(&date) - mid).abs() <= 3, "the date is centred");
    }

    #[test]
    fn an_unlocked_glass_bars_grips_stand_out_from_the_bar_beside_them() {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        let text = TextRenderer::load(&FontConfig::default());
        for small in [false, true] {
            let mut theme = theme_for(Interface::Glass);
            if small {
                crate::interfaces::small_bar(&mut theme, &Default::default());
            }
            let s = theme.look.style().unwrap();
            let start = TextRenderer::load(&s.start_text.font());
            for edge in [Edge::Bottom, Edge::Left] {
                let span = BarSpan {
                    edge,
                    length: if edge.is_side() { 600 } else { 800 },
                    thickness: if edge.is_side() {
                        s.taskbar.side_default(&theme.metrics)
                    } else {
                        theme.metrics.taskbar_height
                    },
                };
                let counts = TaskbarCounts {
                    quick: 2,
                    tasks: 1,
                    ..TaskbarCounts::default()
                };
                let l = TaskbarLayout::compute_on(
                    &theme.metrics,
                    &s.taskbar,
                    span,
                    false,
                    counts,
                    ClockText::default(),
                    Bands::default(),
                );
                let mut c = Canvas::new(l.width as u32, l.height as u32);
                let st = state_with_tasks(&[""]);
                draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
                assert_eq!(l.grips.len(), 2, "small {small} {edge:?}: two bands");
                for (i, g) in l.grips.iter().enumerate() {
                    let shows = |a: u32, b: u32| {
                        let (a, b) = (over_grey(a), over_grey(b));
                        (0..3).any(|ch| (a[ch] - b[ch]).abs() >= 24)
                    };
                    for k in 0..2 {
                        let (line, beside): (Vec<_>, Vec<_>) = if edge.is_side() {
                            (g.x..g.right())
                                .map(|x| (c.get(x, g.y + k), c.get(x, g.y - 1)))
                                .unzip()
                        } else {
                            (g.y..g.bottom())
                                .map(|y| (c.get(g.x + k, y), c.get(g.x - 1, y)))
                                .unzip()
                        };
                        assert!(
                            line.iter().zip(&beside).all(|(&a, &b)| shows(a, b)),
                            "small {small} {edge:?}: grip {i}'s line {k} {line:08x?} \
                             stands out from the bar beside it {beside:08x?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn an_unlocked_bar_marks_each_grip_with_the_taskbar_atlas_row_27_colour() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            false,
            TaskbarCounts::default(),
            40,
        );
        let g = layout.grips[0];
        let bar = s.part(Part::Taskbar).unwrap();
        let f = bar.frame(State::Normal);
        let want = bar.atlas.get(f.x, f.y + GRIP_ROW);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(800, 30);
        draw_taskbar(
            &mut c,
            s,
            &theme,
            &text,
            &start,
            &layout,
            &state_with_tasks(&[]),
        );
        assert_eq!(c.get(g.x, g.y + 1), want);
    }

    fn bar_with_strip(i: Interface, hot: bool) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(i);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tasks: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 40);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&["One"]);
        st.show_desktop_hot = hot;
        let mut c = Canvas::new(1920, layout.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        (c, layout)
    }

    #[test]
    fn the_glass_strip_replaces_the_bar_under_it() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, layout) = bar_with_strip(Interface::Glass, false);
        let r = layout.show_desktop.unwrap();
        let mut alone = Canvas::new(1920, 40);
        s.draw(&mut alone, r, Part::ShowDesktop, State::Normal, 1);
        for (x, y) in [(0, 0), (0, 1), (7, 1), (7, 20), (14, 39)] {
            assert_eq!(
                c.get(r.x + x, y),
                alone.get(r.x + x, y),
                "({x}, {y}) is the strip's own pixel, no bar under it"
            );
        }
    }

    #[test]
    fn blue_draws_its_strip_as_a_task_button_lit_under_the_pointer() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        for (hot, state) in [(false, State::Normal), (true, State::Hot)] {
            let (c, layout) = bar_with_strip(Interface::Blue, hot);
            let r = layout.show_desktop.unwrap();
            assert_eq!(r, Rect::new(1895, 3, 25, 25));
            let mut base = Canvas::new(1920, 30);
            s.draw(
                &mut base,
                Rect::new(0, 0, 1920, 30),
                Part::Taskbar,
                State::Normal,
                1,
            );
            s.draw(&mut base, r, Part::TaskButton, state, 1);
            for (x, y) in [(1, 1), (2, 12), (12, 12), (23, 23)] {
                assert_eq!(
                    c.get(r.x + x, r.y + y),
                    base.get(r.x + x, r.y + y),
                    "hot {hot}: ({x}, {y}) is the {state:?} task button over the bar"
                );
            }
        }
        let (idle, l) = bar_with_strip(Interface::Blue, false);
        let (lit, _) = bar_with_strip(Interface::Blue, true);
        let r = l.show_desktop.unwrap();
        assert_ne!(idle.get(r.x + 12, r.y + 12), lit.get(r.x + 12, r.y + 12));
    }

    fn doubles_outside(one: &Canvas, two: &Canvas, skip: std::ops::Range<i32>) -> bool {
        (0..two.height as i32).all(|y| {
            (0..two.width as i32)
                .filter(|x| !skip.contains(&(x / 2)))
                .all(|x| two.get(x, y) == one.get(x / 2, y / 2))
        })
    }

    fn blue_bar_at_1_and_2(flag: bool) -> (Canvas, Canvas, std::ops::Range<i32>) {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            ..TaskbarCounts::default()
        };
        let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        let draw = |theme: &Theme, layout: &TaskbarLayout| {
            let text = TextRenderer::load(&theme.fonts);
            let start = TextRenderer::load(&s.start_text.font());
            let px = s.taskbar.start_icon * theme.scale;
            let mut ic = Canvas::new(px as u32, px as u32);
            ic.fill_rect(Rect::new(0, 0, px, px), crate::theme::Color::rgb(9, 99, 9));
            let st = TaskbarState {
                start_icon: flag.then_some(ic.into()),
                orb_glow: 100,
                ..state_with_tasks(&[])
            };
            let mut c = Canvas::new(layout.width as u32, layout.height as u32);
            draw_taskbar(&mut c, s, theme, &text, &start, layout, &st);
            c
        };
        let one = draw(&theme, &layout);
        let two = draw(&theme.scaled(2), &layout.scaled(2));
        let label = layout.start.x + 34..layout.start.right() + 2;
        (one, two, label)
    }

    #[test]
    fn a_blue_bar_at_scale_2_doubles_the_scale_1_bar_outside_the_start_label() {
        let (one, two, label) = blue_bar_at_1_and_2(true);
        assert!(
            doubles_outside(&one, &two, label),
            "the bar, the Start pill and flag and the tray area at scale 2 are the scale-1 pixels doubled"
        );
    }

    #[test]
    fn blue_s_start_mark_is_the_16_px_drawing_and_the_32_px_one_at_scale_2() {
        let (one, two, label) = blue_bar_at_1_and_2(false);
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        let flag = l.start.x + 10..label.end;
        assert!(
            doubles_outside(&one, &two, flag),
            "the bar around the mark doubles"
        );
        let ink = s.start_text.color;
        let (x, y) = (l.start.x + 11, l.start.y + 6);
        crate::art::logo::assert_drawn(&one, x, y, 16, ink);
        crate::art::logo::assert_drawn(&two, 2 * x, 2 * y, 32, ink);
    }

    #[test]
    fn the_glass_orb_carries_the_mark_in_white_without_a_theme_logo() {
        let (c, l) = glass_bar("", None);
        let s = theme_for(Interface::Glass)
            .look
            .style()
            .unwrap()
            .taskbar
            .start_icon
            * ORB_MARK_SHARE.0
            / ORB_MARK_SHARE.1;
        let d = l.start_disc.unwrap_or(l.start);
        crate::art::logo::assert_path_drawn(
            &c,
            d.x + (d.width - s) / 2,
            d.y + (d.height - s) / 2,
            s,
            crate::theme::Color::rgb(0xff, 0xff, 0xff),
        );
    }

    fn ink_size(c: &Canvas, r: Rect, ink: u32) -> (i32, i32) {
        let hits: Vec<(i32, i32)> = (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == ink)
            .collect();
        let xs = || hits.iter().map(|p| p.0);
        let ys = || hits.iter().map(|p| p.1);
        (
            xs().max().unwrap() - xs().min().unwrap() + 1,
            ys().max().unwrap() - ys().min().unwrap() + 1,
        )
    }

    fn blue_clock_bar(clock: &str) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let counts = TaskbarCounts {
            tray: 2,
            ..TaskbarCounts::default()
        };
        let w = text.measure(clock, false, theme.fonts.size_px);
        let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, w);
        let st = TaskbarState {
            clock: clock.into(),
            tray_icons: vec![None, None],
            ..state_with_tasks(&[])
        };
        let mut c = Canvas::new(1920, 30);
        draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
        (c, layout)
    }

    #[test]
    fn blue_sets_the_clock_in_its_48_px_box_12_px_from_the_end() {
        if !crate::text::test_tahoma() {
            return;
        }
        for (clock, x0, x1) in [
            ("11:00 PM", 1862, 1904),
            ("12:19 PM", 1862, 1904),
            ("10:28 AM", 1862, 1905),
            ("7:46 AM", 1864, 1902),
            ("8:29 PM", 1864, 1901),
            ("9:12 PM", 1864, 1901),
            ("1:35 PM", 1865, 1901),
        ] {
            let (c, layout) = blue_clock_bar(clock);
            assert_eq!(layout.clock, Rect::new(1860, 3, 48, 25), "{clock}: the box");
            let white = crate::theme::Color::rgb(0xFF, 0xFF, 0xFF).argb();
            let ink: Vec<(i32, i32)> = (0..30)
                .flat_map(|y| (1840..1920).map(move |x| (x, y)))
                .filter(|&(x, y)| c.get(x, y) == white)
                .collect();
            let xs = || ink.iter().map(|p| p.0);
            let ys = || ink.iter().map(|p| p.1);
            assert_eq!(
                (xs().min(), xs().max()),
                (Some(x0), Some(x1)),
                "{clock}: its first and last inked columns"
            );
            assert_eq!(
                (ys().min(), ys().max()),
                (Some(11), Some(18)),
                "{clock}: its rows"
            );
        }
    }

    #[test]
    fn every_variant_sets_the_start_label_and_the_clock_as_blue_does() {
        use crate::blue::{Variant, style_for};
        use crate::theme::Color;
        let blue = style_for(Variant::Blue);
        for v in Variant::ALL {
            let s = style_for(v);
            let mut want = blue.start_text.clone();
            if let Some(sh) = &mut want.shadow {
                sh.color = v.scheme().inks.start_shadow;
            }
            assert_eq!(s.start_text, want, "{v:?}: the Start label");
            assert_eq!(s.taskbar, blue.taskbar, "{v:?}: the clock box and tray");
        }
        assert_eq!(
            style_for(Variant::Ember)
                .start_text
                .shadow
                .map(|sh| sh.color),
            Some(Color::rgb(0x00, 0x00, 0x00)),
            "ember's label casts a black shadow"
        );
    }

    #[test]
    fn blues_tray_icons_sit_2_px_apart_6_px_before_the_clock_in_a_well_from_11_px_before_them() {
        // The box holds any 12-hour time, so the well stays put.
        for w in [39, 45, 46] {
            let theme = theme_for(Interface::Blue);
            let s = theme.look.style().unwrap();
            let counts = TaskbarCounts {
                tray: 2,
                ..TaskbarCounts::default()
            };
            let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, w);
            assert_eq!(l.clock, Rect::new(1860, 3, 48, 25), "{w} px of text");
            assert_eq!(
                l.tray,
                [Rect::new(1820, 8, 16, 16), Rect::new(1838, 8, 16, 16)]
            );
            assert_eq!(l.notification, Rect::new(1809, 0, 111, 30));
        }
    }

    #[test]
    fn a_styled_clock_at_scale_2_is_set_at_twice_the_scale_1_font_size() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let layout = TaskbarLayout::compute(
            &theme.metrics,
            &s.taskbar,
            800,
            true,
            TaskbarCounts::default(),
            40,
        );
        let draw = |theme: &Theme, layout: &TaskbarLayout| {
            let text = TextRenderer::load(&theme.fonts);
            let start = TextRenderer::load(&s.start_text.font());
            let st = TaskbarState {
                clock: "12:34".into(),
                ..state_with_tasks(&[])
            };
            let mut c = Canvas::new(layout.width as u32, layout.height as u32);
            draw_taskbar(&mut c, s, theme, &text, &start, layout, &st);
            ink_size(&c, layout.clock, s.bar_text.argb())
        };
        let (w1, h1) = draw(&theme, &layout);
        let (w2, h2) = draw(&theme.scaled(2), &layout.scaled(2));
        assert!(
            (w2 - 2 * w1).abs() <= 2 && (h2 - 2 * h1).abs() <= 2,
            "the scale-2 clock inks {w2}x{h2}, expected {}x{} +- 2",
            2 * w1,
            2 * h1
        );
    }

    fn glass_bar_at(
        theme: &Theme,
        s: &Style,
        (edge, t): (Edge, i32),
        strip_hot: bool,
        sc: i32,
    ) -> (Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText, TaskRow};
        let shape = s.taskbar;
        let counts = TaskbarCounts {
            tasks: 3,
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length: 600,
            thickness: t,
        };
        let mut l = TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        l.lay_tasks(&theme.metrics, &shape, &[1, 2, 3], TaskRow::Icons);
        let (theme, l) = (theme.scaled(sc), l.scaled(sc));
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&["", "", ""]);
        let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), shape.task_icon * sc);
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
        }
        st.tasks[1].active = true;
        st.show_desktop_hot = strip_hot;
        st.tray_icons = vec![None];
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
        (c, l)
    }

    #[test]
    fn a_glass_bar_at_scale_2_draws_its_parts_painted_at_2_where_scale_1_puts_them() {
        let glass = theme_for(Interface::Glass);
        let mut small = theme_for(Interface::Glass);
        crate::interfaces::small_bar(&mut small, &Default::default());
        for (theme, h) in [(&glass, 40), (&small, 30)] {
            let s2 = theme.scaled(2);
            let s = s2.look.style().unwrap();
            assert_eq!(s.part(Part::TaskButton).unwrap().scale, 2);
            let side = s.taskbar.side_default(&theme.metrics);
            for (edge, t) in [(Edge::Bottom, h), (Edge::Left, side)] {
                for hot in [false, true] {
                    let (c, l) = glass_bar_at(theme, s, (edge, t), hot, 2);
                    let (w, ht) = (l.width, l.height);
                    let mut want = Canvas::new(w as u32, ht as u32);
                    l.edge
                        .draw_art(&mut want, Rect::new(0, 0, w, ht), |a, along| {
                            s.draw(a, along, Part::Taskbar, State::Normal, 2);
                            let well = l.to_along(l.notification);
                            s.draw(a, well, Part::TrayArea, State::Normal, 2);
                        });
                    let px = s.taskbar.task_icon * 2;
                    let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
                    for (i, &slot) in l.tasks.iter().enumerate() {
                        let st = if i == 1 {
                            State::Checked
                        } else {
                            State::Normal
                        };
                        let (front, edges) = stacked(s, slot, l.stacks.get(i).copied(), 2);
                        for (k, e) in edges.iter().enumerate() {
                            s.draw_nth(&mut want, *e, Part::TaskStack, st, k, 2);
                        }
                        s.draw(&mut want, front, Part::TaskButton, st, 2);
                        let (x, y) = (
                            front.x + (front.width - px) / 2,
                            front.y + (front.height - px) / 2,
                        );
                        want.blit_canvas(x, y, &icon);
                    }
                    let strip = l.show_desktop.unwrap();
                    want.clear_rect(strip);
                    let lit = if hot { State::Hot } else { State::Normal };
                    l.edge.draw_art(&mut want, strip, |a, at| {
                        s.draw(a, at, Part::ShowDesktop, lit, 2)
                    });
                    let what = format!("{h} px bar along {edge:?}, strip lit {hot}");
                    let first = l.tasks[0];
                    let rects = l.tasks.iter().copied().chain([strip, l.notification]);
                    for r in rects {
                        for y in r.y..r.bottom() {
                            for x in r.x..r.right() {
                                assert_eq!(c.get(x, y), want.get(x, y), "{what}: ({x}, {y})");
                            }
                        }
                    }
                    let gap = if edge.is_side() {
                        Rect::new(0, l.start.bottom(), w, first.y - l.start.bottom())
                    } else {
                        Rect::new(l.start.right(), 0, first.x - l.start.right(), ht)
                    };
                    for y in gap.y..gap.bottom() {
                        for x in gap.x..gap.right() {
                            assert_eq!(c.get(x, y), want.get(x, y), "{what}: bar ({x}, {y})");
                        }
                    }
                }
            }
        }
    }

    fn glass_pager(
        mark: crate::taskbar::PagerMark,
        hot: Option<usize>,
        small: bool,
    ) -> (Canvas, Vec<Rect>, Canvas) {
        let mut theme = theme_for(Interface::Glass);
        if small {
            theme.metrics.taskbar_height = crate::glass::SMALL_TASKBAR_HEIGHT;
            theme.look =
                crate::paint::Look::Styled(crate::glass::small_bar_style(&Default::default()));
        }
        pager_bar(&theme, mark, hot)
    }

    fn pager_bar(
        theme: &Theme,
        mark: crate::taskbar::PagerMark,
        hot: Option<usize>,
    ) -> (Canvas, Vec<Rect>, Canvas) {
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            pager: 4,
            ..TaskbarCounts::default()
        };
        let mut layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 60);
        layout.lay_tasks(
            &theme.metrics,
            &s.taskbar,
            &[],
            crate::taskbar::TaskRow::Icons,
        );
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&[]);
        st.pager = (0..4)
            .map(|i| crate::taskbar::PagerButton {
                current: i == 1,
                attention: false,
            })
            .collect();
        st.pager_hot = hot;
        st.pager_mark = mark;
        let h = layout.height;
        let mut c = Canvas::new(800, h as u32);
        draw_taskbar(&mut c, s, theme, &text, &start, &layout, &st);
        let mut bar = Canvas::new(800, h as u32);
        s.draw(
            &mut bar,
            Rect::new(0, 0, 800, h),
            Part::Taskbar,
            State::Normal,
            1,
        );
        (c, layout.pager.clone(), bar)
    }

    fn peak_lum(c: &Canvas, xs: std::ops::Range<i32>, ys: std::ops::Range<i32>) -> i32 {
        ys.flat_map(|y| xs.clone().map(move |x| (x, y)))
            .map(|(x, y)| lum(c.get(x, y)))
            .max()
            .unwrap_or(0)
    }

    #[test]
    fn a_glass_pager_lights_the_current_desktop_at_its_foot() {
        use crate::taskbar::PagerMark;
        for small in [false, true] {
            let (c, cells, bar) = glass_pager(PagerMark::Light, None, small);
            let (r0, r1) = (cells[0], cells[1]);
            assert_eq!(r1.width, 23, "a pager cell is 23 px");
            let foot = r1.bottom() - 1;
            assert!(
                lum(c.get(r1.x + 11, foot)) > lum(c.get(r0.x + 11, foot)) + 20,
                "small {small}: the current cell's foot is lit"
            );
            let peak = (r1.x..r1.right())
                .max_by_key(|&x| lum(c.get(x, foot)))
                .unwrap();
            assert_eq!(
                peak,
                r1.x + 11,
                "small {small}: the light peaks at the cell's middle column"
            );
            for (x, y) in (0..3).flat_map(|x| (0..3).map(move |y| (x, y))) {
                assert_eq!(
                    c.get(r1.x + x, r1.y + y),
                    bar.get(r1.x + x, r1.y + y),
                    "small {small}: no frame at the current cell's top left ({x}, {y})"
                );
            }
            let rows = r1.y..r1.bottom() - 7;
            assert!(
                peak_lum(&c, r0.x..r0.right(), rows.clone()) < peak_lum(&c, r1.x..r1.right(), rows),
                "small {small}: another desktop's number is dimmer than the current one's"
            );
        }
    }

    #[test]
    fn a_glass_pager_tile_is_short_and_centred() {
        use crate::taskbar::PagerMark;
        for (small, top, h) in [(false, 8, 24), (true, 5, 20)] {
            for hot in [None, Some(1), Some(2)] {
                let (c, cells, bar) = glass_pager(PagerMark::Tile, hot, small);
                let mut lit = vec![cells[1]];
                lit.extend(hot.map(|i| cells[i]));
                for r in lit {
                    for y in (0..top).chain(top + h..r.height) {
                        for x in r.x..r.right() {
                            assert_eq!(
                                c.get(x, r.y + y),
                                bar.get(x, r.y + y),
                                "small {small} hot {hot:?}: no tile at ({x}, {y}) outside rows {top}..{}",
                                top + h
                            );
                        }
                    }
                    let mid = r.y + top + h / 2;
                    assert_ne!(
                        c.get(r.x + 1, mid),
                        bar.get(r.x + 1, mid),
                        "small {small} hot {hot:?}: the tile starts 1 px in"
                    );
                    assert_ne!(
                        c.get(r.x + 21, mid),
                        bar.get(r.x + 21, mid),
                        "small {small} hot {hot:?}: the tile ends 1 px before the cell does"
                    );
                    for x in [r.x, r.x + 22] {
                        assert_eq!(
                            c.get(x, mid),
                            bar.get(x, mid),
                            "small {small} hot {hot:?}: the tile is 21 px wide, column {x} is bar"
                        );
                    }
                }
                let mid = cells[0].y + top + h / 2;
                assert_eq!(
                    c.get(cells[0].x + 1, mid),
                    bar.get(cells[0].x + 1, mid),
                    "small {small} hot {hot:?}: another desktop at rest has no tile"
                );
            }
        }
    }

    #[test]
    fn a_hot_glass_pager_cell_lights_its_edges() {
        use crate::taskbar::PagerMark;
        let (c, cells, bar) = glass_pager(PagerMark::Light, Some(2), false);
        let r = cells[2];
        let mid = r.y + r.height / 2;
        for x in [r.x, r.x + 1, r.right() - 2, r.right() - 1] {
            assert_ne!(c.get(x, mid), bar.get(x, mid), "edge column {x} is lit");
        }
        let foot = r.bottom() - 1;
        assert_eq!(
            c.get(r.x + 11, foot),
            bar.get(r.x + 11, foot),
            "no foot light"
        );
        let (hot, cells, _) = glass_pager(PagerMark::Light, Some(1), false);
        let (rest, _, _) = glass_pager(PagerMark::Light, None, false);
        let r = cells[1];
        for x in [r.x, r.right() - 1] {
            assert_ne!(
                hot.get(x, mid),
                bar.get(x, mid),
                "current: edge column {x} is lit"
            );
        }
        assert_eq!(
            hot.get(r.x + 11, foot),
            rest.get(r.x + 11, foot),
            "current: the foot light stays"
        );
    }

    #[test]
    fn blue_ignores_the_pager_mark() {
        use crate::taskbar::PagerMark;
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let (light, cells, bar) = pager_bar(&theme, PagerMark::Light, None);
        let (tile, _, _) = pager_bar(&theme, PagerMark::Tile, None);
        assert_eq!(light.pixels, tile.pixels, "both marks draw alike");
        let r = cells[1];
        let mut want = bar.clone();
        s.draw(&mut want, r, Part::TaskButton, State::Checked, 1);
        for (x, y) in [
            (r.x + 1, r.y + 2),
            (r.x, r.y),
            (r.right() - 1, r.bottom() - 1),
        ] {
            assert_eq!(
                light.get(x, y),
                want.get(x, y),
                "the current cell is a Checked button at ({x}, {y})"
            );
        }
    }

    fn glass_top_row(stacks: &[u8], hot: Option<usize>) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let shape = s.taskbar;
        let counts = TaskbarCounts {
            tasks: stacks.len(),
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&theme.metrics, &shape, 1920, true, counts, 60)
            .on_edge(Edge::Top);
        l.lay_tasks(
            &theme.metrics,
            &shape,
            stacks,
            crate::taskbar::TaskRow::Icons,
        );
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&vec![""; stacks.len()]);
        st.start_icon = Some(clear_start_logo());
        let white = solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), 32);
        for t in &mut st.tasks {
            t.icon = Some(white.clone().into());
        }
        st.task_hot = hot;
        let mut c = Canvas::new(1920, 40);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
        (c, l)
    }

    #[test]
    fn a_top_glass_bar_turns_its_bar_well_and_strip_over_under_upright_buttons() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, l) = glass_top_row(&[1, 1, 1], None);
        assert_eq!(l.start, Rect::new(0, 0, 54, 40), "the orb fills the bar");
        let full = Rect::new(0, 0, 1920, 40);
        let strip = l.show_desktop.unwrap();
        let mut want = Canvas::new(1920, 40);
        want.with_rows_flipped(full, |w| {
            s.draw(w, full, Part::Taskbar, State::Normal, 1);
            s.draw(w, l.notification, Part::TrayArea, State::Normal, 1);
        });
        s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
        let white = solid_icon(crate::theme::Color::rgb(0xFF, 0xFF, 0xFF), 32);
        for r in &l.tasks {
            s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
            want.blit_canvas(r.x + 14, r.y + 4, &white);
        }
        want.clear_rect(strip);
        want.with_rows_flipped(strip, |w| {
            s.draw(w, strip, Part::ShowDesktop, State::Normal, 1)
        });
        assert_eq!(
            same_columns(&c, &want, 0..1920),
            Ok(()),
            "the bar, the well and the strip turned over; the orb and the buttons upright"
        );
    }

    #[test]
    fn a_top_glass_bar_turns_the_labelled_rows_veil_over_with_the_bar() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let st = state_with_tasks(&["One", "Two", "Three"]);
        let (bottom, _) = draw_glass(s, 40, &st);
        let counts = TaskbarCounts {
            tasks: 3,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1920, true, counts, 60)
            .on_edge(Edge::Top);
        assert!(
            !l.icons_only && l.tasks_limit() > 1000,
            "a labelled row, its veil past x 1000"
        );
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut top = Canvas::new(1920, 40);
        draw_taskbar(&mut top, s, &theme, &text, &start, &l, &st);
        for y in 0..40 {
            assert_eq!(
                top.get(1000, y),
                bottom.get(1000, 39 - y),
                "row {y} is the Bottom bar's row {}",
                39 - y
            );
        }
    }

    #[test]
    fn a_top_blue_bar_turns_its_bar_and_tray_over_under_upright_buttons() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tasks: 2,
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40)
            .on_edge(Edge::Top);
        assert_eq!(
            (l.tasks[0].y, l.tasks[0].height),
            (2, 25),
            "the buttons 2 px from the top, 3 from the foot"
        );
        let px = s.taskbar.task_icon;
        let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
        let mut st = state_with_tasks(&["", ""]);
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
        }
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
        let full = Rect::new(0, 0, 800, 30);
        let mut want = Canvas::new(800, 30);
        want.with_rows_flipped(full, |w| {
            s.draw(w, full, Part::Taskbar, State::Normal, 1);
            s.draw(w, l.notification, Part::TrayArea, State::Normal, 1);
        });
        s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
        draw_start_pill(&mut want, s, &start, l.start, &st, 1);
        for r in &l.tasks {
            s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
            want.blit_canvas(r.x + 10 + (16 - px) / 2, r.y + (r.height - px) / 2, &icon);
        }
        s.draw(
            &mut want,
            l.show_desktop.unwrap(),
            Part::TaskButton,
            State::Normal,
            1,
        );
        assert_eq!(
            same_columns(&c, &want, 0..800),
            Ok(()),
            "the bar and the tray turned over; Start, the buttons and the strip upright"
        );
    }

    #[test]
    fn a_blue_bar_draws_the_round_arrow_and_a_glass_bar_keeps_its_caret() {
        for (interface, round) in [(Interface::Blue, true), (Interface::Glass, false)] {
            let theme = theme_for(interface);
            let s = theme.look.style().unwrap();
            let counts = TaskbarCounts {
                tray: 1,
                chevron: true,
                ..TaskbarCounts::default()
            };
            let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
            assert_eq!(l.tray_arrow, round, "{interface:?}");
            let text = TextRenderer::load(&FontConfig::default());
            let start = TextRenderer::load(&s.start_text.font());
            let mut c = Canvas::new(800, l.height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
            let r = l.chevron.unwrap();
            let mut bare = l.clone();
            bare.chevron = None;
            let mut want = Canvas::new(800, l.height as u32);
            draw_taskbar(
                &mut want,
                s,
                &theme,
                &text,
                &start,
                &bare,
                &state_with_tasks(&[]),
            );
            if round {
                let art = s.tray_arrow.as_deref().unwrap();
                draw_blue_tray_arrow(&mut want, r, art, PopupCell::default(), 1);
                assert_eq!(
                    pixels(&c, r),
                    pixels(&want, r),
                    "the cell holds the round button"
                );
            } else {
                crate::taskbar::draw_chevron(&mut want, r, s.bar_text, 1);
                assert_eq!(pixels(&c, r), pixels(&want, r), "the cell holds the ^");
            }
        }
    }

    #[test]
    fn a_held_blue_arrow_draws_its_pressed_disc() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let r = l.chevron.unwrap();
        for open in [false, true] {
            let cell = PopupCell {
                hot: true,
                open,
                held: true,
            };
            let mut st = state_with_tasks(&[]);
            st.chevron = cell;
            let mut c = Canvas::new(800, l.height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &l, &st);
            let mut bare = l.clone();
            bare.chevron = None;
            let mut want = Canvas::new(800, l.height as u32);
            draw_taskbar(
                &mut want,
                s,
                &theme,
                &text,
                &start,
                &bare,
                &state_with_tasks(&[]),
            );
            let art = s.tray_arrow.as_deref().unwrap();
            draw_blue_tray_arrow(&mut want, r, art, cell, 1);
            assert_eq!(
                pixels(&c, r),
                pixels(&want, r),
                "open {open}: the pressed disc"
            );
        }
    }

    #[test]
    fn a_top_bar_points_its_caret_down_in_glass() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let draw = |edge: Edge| {
            let l = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40)
                .on_edge(edge);
            let text = TextRenderer::load(&FontConfig::default());
            let start = TextRenderer::load(&s.start_text.font());
            let mut c = Canvas::new(800, l.height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
            (c, l.chevron.unwrap())
        };
        let ((top, t), (bottom, b)) = (draw(Edge::Top), draw(Edge::Bottom));
        assert_eq!(t, b, "the cell is centred down the bar and keeps its rect");
        for y in 0..16 {
            assert_eq!(
                pixels(&top, Rect::new(t.x, t.y + y, 16, 1)),
                pixels(&bottom, Rect::new(b.x, b.y + 15 - y, 16, 1)),
                "the cell's row {y} is the Bottom cell's row {}",
                15 - y
            );
        }
    }

    #[test]
    fn on_a_top_bar_a_lit_idle_glass_pin_has_its_light_turned_over_at_its_top() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, l) = glass_top_row(&[0, 1, 1], Some(0));
        let r = l.tasks[0];
        let full = Rect::new(0, 0, 1920, 40);
        let mut want = Canvas::new(1920, 40);
        want.with_rows_flipped(full, |w| s.draw(w, full, Part::Taskbar, State::Normal, 1));
        for x in [r.x, r.right() - 2] {
            s.draw(
                &mut want,
                Rect::new(x, r.y, 2, r.height),
                Part::SlotEdge,
                State::Hot,
                1,
            );
        }
        let light = Rect::new(r.x, r.y, 60, 7);
        want.with_rows_flipped(light, |w| s.draw(w, light, Part::SlotLight, State::Hot, 1));
        want.fill_rect(
            Rect::new(r.x + 14, r.y + 4, 32, 32),
            crate::theme::Color::rgb(0xFF, 0xFF, 0xFF),
        );
        assert_eq!(
            same_columns(&c, &want, r.x..r.right()),
            Ok(()),
            "the bar turned over, the edges upright, the 60x7 light turned over on the top rows, the icon"
        );
        let rest = glass_top_row(&[0, 1, 1], None).0;
        let at = (r.x + r.width / 2, r.y);
        assert!(
            lum(c.get(at.0, at.1)) > lum(rest.get(at.0, at.1)) + 20,
            "the slot's top row is lit"
        );
    }

    #[test]
    fn on_a_top_bar_the_glass_pager_lights_the_current_desktop_at_its_top() {
        use crate::taskbar::{PagerButton, PagerMark, TaskRow};
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let shape = s.taskbar;
        let counts = TaskbarCounts {
            pager: 4,
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&theme.metrics, &shape, 800, true, counts, 60)
            .on_edge(Edge::Top);
        l.lay_tasks(&theme.metrics, &shape, &[], TaskRow::Icons);
        let text = TextRenderer::load(&theme.fonts);
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&[]);
        st.pager = (0..4)
            .map(|i| PagerButton {
                current: i == 1,
                attention: false,
            })
            .collect();
        st.pager_mark = PagerMark::Light;
        let mut top = Canvas::new(800, 40);
        draw_taskbar(&mut top, s, &theme, &text, &start, &l, &st);
        let (bottom, cells, _) = glass_pager(PagerMark::Light, None, false);
        let r1 = l.pager[1];
        assert_eq!(r1, cells[1], "a full-height cell keeps its rect");
        for y in 0..7 {
            for x in r1.x..r1.right() {
                assert_eq!(
                    top.get(x, y),
                    bottom.get(x, 39 - y),
                    "({x}, {y}) is the Bottom cell's foot row {} turned over",
                    39 - y
                );
            }
        }
        assert!(
            lum(top.get(r1.x + 11, 0)) > lum(top.get(l.pager[0].x + 11, 0)) + 20,
            "the current cell's top row is lit"
        );
    }

    fn hidden_line(interface: Interface, edge: Edge) -> (Canvas, Canvas, std::ops::Range<i32>) {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        let counts = TaskbarCounts {
            tray: 1,
            ..TaskbarCounts::default()
        };
        let l =
            TaskbarLayout::compute(&theme.metrics, &s.taskbar, 800, true, counts, 40).on_edge(edge);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let h = l.height;
        let mut c = Canvas::new(800, h as u32);
        draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
        let full = Rect::new(0, 0, 800, h);
        let mut art = Canvas::new(800, h as u32);
        if edge == Edge::Bottom {
            s.draw(&mut art, full, Part::Taskbar, State::Normal, 1)
        } else {
            art.with_rows_flipped(full, |a| s.draw(a, full, Part::Taskbar, State::Normal, 1))
        }
        let line = l.reveal_line();
        let inner = if edge == Edge::Bottom {
            Rect::new(0, 0, 800, 2)
        } else {
            Rect::new(0, h - 2, 800, 2)
        };
        assert_eq!(
            line, inner,
            "{interface:?} {edge:?}: the reveal line's rows"
        );
        (
            c.copy_rect(line),
            art.copy_rect(inner),
            l.start.right() + 2..l.notification.x - 2,
        )
    }

    #[test]
    fn a_hidden_blue_bar_leaves_its_two_inner_bar_rows_on_screen() {
        for edge in [Edge::Bottom, Edge::Top] {
            let (got, want, xs) = hidden_line(Interface::Blue, edge);
            assert_eq!((got.width, got.height), (800, 2), "{edge:?}");
            assert_eq!(
                same_columns(&got, &want, xs.clone()),
                Ok(()),
                "{edge:?}: the line is the bar's own gradient rows"
            );
            assert_ne!(
                got.get(xs.start, 0),
                got.get(xs.start, 1),
                "{edge:?}: two rows of the gradient"
            );
        }
    }

    #[test]
    fn a_hidden_glass_bar_leaves_its_dark_and_light_lines_on_screen() {
        for edge in [Edge::Bottom, Edge::Top] {
            let (got, want, xs) = hidden_line(Interface::Glass, edge);
            assert_eq!((got.width, got.height), (800, 2), "{edge:?}");
            assert_eq!(
                same_columns(&got, &want, xs.clone()),
                Ok(()),
                "{edge:?}: the line is the bar's own two edge lines"
            );
            assert_ne!(got.get(xs.start, 0) >> 24, 0, "{edge:?}: the line shows");
            assert_ne!(
                got.get(xs.start, 0),
                got.get(xs.start, 1),
                "{edge:?}: a dark line and a light one"
            );
        }
    }

    fn side_bar_in(
        theme: &Theme,
        s: &Style,
        edge: Edge,
        stacks: &[u8],
        hot: Option<usize>,
    ) -> (Canvas, TaskbarLayout) {
        let t = s.taskbar.side_default(&theme.metrics);
        side_bar_titled(theme, s, edge, t, stacks, hot, "")
    }

    fn side_bar_titled(
        theme: &Theme,
        s: &Style,
        edge: Edge,
        t: i32,
        stacks: &[u8],
        hot: Option<usize>,
        title: &str,
    ) -> (Canvas, TaskbarLayout) {
        side_bar_with(theme, s, edge, t, 0, stacks, title, |st| st.task_hot = hot)
    }

    #[allow(clippy::too_many_arguments)]
    fn side_bar_with(
        theme: &Theme,
        s: &Style,
        edge: Edge,
        t: i32,
        pager: usize,
        stacks: &[u8],
        title: &str,
        tweak: impl FnOnce(&mut TaskbarState),
    ) -> (Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText, TaskRow};
        let shape = s.taskbar;
        let counts = TaskbarCounts {
            pager,
            tasks: stacks.len(),
            tray: 1,
            show_desktop: true,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length: 600,
            thickness: t,
        };
        let mut l = TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        l.lay_tasks(&theme.metrics, &shape, stacks, TaskRow::Shared);
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut st = state_with_tasks(&vec![title; stacks.len()]);
        st.start_icon = Some(clear_start_logo());
        let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), shape.task_icon);
        for t in &mut st.tasks {
            t.icon = Some(icon.clone().into());
        }
        st.pager = vec![crate::taskbar::PagerButton::default(); pager];
        tweak(&mut st);
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_taskbar(&mut c, s, theme, &text, &start, &l, &st);
        (c, l)
    }

    fn side_bar(
        interface: Interface,
        edge: Edge,
        stacks: &[u8],
        hot: Option<usize>,
    ) -> (Canvas, TaskbarLayout) {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        side_bar_in(&theme, s, edge, stacks, hot)
    }

    #[test]
    fn a_side_blue_bar_lays_its_bar_and_well_on_their_side_under_upright_buttons() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let start = TextRenderer::load(&s.start_text.font());
        let px = s.taskbar.task_icon;
        let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), px);
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar(Interface::Blue, edge, &[1, 1], None);
            let (w, h) = (l.width, l.height);
            assert_eq!(
                l.notification.width, w,
                "{edge:?}: the well a block across the column"
            );
            assert!(!l.icons_only, "{edge:?}: labelled at Start's width");
            let mut want = Canvas::new(w as u32, h as u32);
            want.with_transposed(Rect::new(0, 0, w, h), edge == Edge::Left, |a| {
                s.draw(a, Rect::new(0, 0, h, w), Part::Taskbar, State::Normal, 1);
                s.draw(
                    a,
                    l.to_along(l.notification),
                    Part::TrayArea,
                    State::Normal,
                    1,
                );
            });
            s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
            let mut st = state_with_tasks(&["", ""]);
            st.start_icon = Some(clear_start_logo());
            draw_start_pill(&mut want, s, &start, l.start, &st, 1);
            for r in &l.tasks {
                s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
                want.blit_canvas(r.x + 10 + (16 - px) / 2, r.y + (r.height - px) / 2, &icon);
            }
            s.draw(
                &mut want,
                l.show_desktop.unwrap(),
                Part::TaskButton,
                State::Normal,
                1,
            );
            assert_eq!(
                same_columns(&c, &want, 0..w),
                Ok(()),
                "{edge:?}: the bar and the well on their side; Start, the buttons and the strip upright"
            );
        }
    }

    #[test]
    fn a_side_glass_bar_lays_its_bar_well_and_strip_on_their_side_under_upright_buttons() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let icon = solid_icon(crate::theme::Color::rgb(1, 2, 3), 32);
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar(Interface::Glass, edge, &[1, 1, 1], None);
            let (w, h) = (l.width, l.height);
            let left = edge == Edge::Left;
            assert!(l.icons_only, "{edge:?}: one slot, icons only");
            let strip = l.show_desktop.unwrap();
            assert_eq!(
                strip,
                Rect::new(0, 585, w, 15),
                "{edge:?}: 15 px across the column, last"
            );
            let mut want = Canvas::new(w as u32, h as u32);
            want.with_transposed(Rect::new(0, 0, w, h), left, |a| {
                s.draw(a, Rect::new(0, 0, h, w), Part::Taskbar, State::Normal, 1);
                s.draw(
                    a,
                    l.to_along(l.notification),
                    Part::TrayArea,
                    State::Normal,
                    1,
                );
            });
            s.draw(&mut want, l.start, Part::StartButton, State::Normal, 1);
            for r in &l.tasks {
                s.draw(&mut want, *r, Part::TaskButton, State::Normal, 1);
                want.blit_canvas(r.x + 14, r.y + 4, &icon);
            }
            want.clear_rect(strip);
            want.with_transposed(strip, left, |a| {
                let at = Rect::new(0, 0, strip.height, strip.width);
                s.draw(a, at, Part::ShowDesktop, State::Normal, 1)
            });
            assert_eq!(
                same_columns(&c, &want, 0..w),
                Ok(()),
                "{edge:?}: the bar, the well and the strip on their side; the orb and the buttons upright"
            );
        }
    }

    #[test]
    fn a_glass_column_draws_its_buttons_icon_only_below_70_px() {
        let glass = theme_for(Interface::Glass);
        let mut small = theme_for(Interface::Glass);
        crate::interfaces::small_bar(&mut small, &Default::default());
        let title = "Untitled - Notepad";
        for (theme, t) in [(&glass, 60), (&small, 44)] {
            let s = theme.look.style().unwrap();
            for edge in [Edge::Left, Edge::Right] {
                let (c, l) = side_bar_titled(theme, s, edge, t, &[1, 1], None, title);
                let (bare, _) = side_bar_titled(theme, s, edge, t, &[1, 1], None, "");
                assert!(l.icons_only, "{edge:?} at {t} px: icons only");
                assert_eq!(
                    same_columns(&c, &bare, 0..l.width),
                    Ok(()),
                    "{edge:?} at {t} px: the buttons as if untitled, no title on them"
                );
            }
        }
        let s = glass.look.style().unwrap();
        let (c, l) = side_bar_titled(&glass, s, Edge::Left, 70, &[1, 1], None, title);
        let (bare, _) = side_bar_titled(&glass, s, Edge::Left, 70, &[1, 1], None, "");
        assert!(!l.icons_only, "70 px: labelled");
        assert_ne!(
            same_columns(&c, &bare, 0..l.width),
            Ok(()),
            "70 px: the title on the buttons"
        );
        let counts = TaskbarCounts {
            tasks: 2,
            ..TaskbarCounts::default()
        };
        let mut l = TaskbarLayout::compute(&glass.metrics, &s.taskbar, 800, true, counts, 60);
        l.lay_tasks(
            &glass.metrics,
            &s.taskbar,
            &[1, 1],
            crate::taskbar::TaskRow::Icons,
        );
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |title: &str| {
            let mut c = Canvas::new(800, l.height as u32);
            let st = state_with_tasks(&[title, title]);
            draw_taskbar(&mut c, s, &glass, &text, &start, &l, &st);
            c
        };
        assert_eq!(
            same_columns(&draw(title), &draw(""), 0..800),
            Ok(()),
            "a combined row: no title on its buttons"
        );
    }

    #[test]
    fn on_a_side_bar_a_lit_idle_glass_pin_has_its_light_down_its_outer_side() {
        for edge in [Edge::Left, Edge::Right] {
            let (lit, l) = side_bar(Interface::Glass, edge, &[0, 1], Some(0));
            let (rest, _) = side_bar(Interface::Glass, edge, &[0, 1], None);
            let r = l.tasks[0];
            let (outer, inner) = if edge == Edge::Left {
                (r.x + 2, r.right() - 5)
            } else {
                (r.right() - 3, r.x + 4)
            };
            let y = r.y + r.height / 2;
            assert!(
                lum(lit.get(outer, y)) > lum(rest.get(outer, y)) + 20,
                "{edge:?}: lit down its outer side"
            );
            assert_eq!(
                lit.get(inner, y),
                rest.get(inner, y),
                "{edge:?}: not on its inner side"
            );
        }
    }

    fn turned(c: &Canvas, r: Rect, edge: Edge, f: impl FnOnce(&mut Canvas, Rect)) -> Canvas {
        let mut c = c.clone();
        c.with_transposed(r, edge == Edge::Left, |p| {
            f(p, Rect::new(0, 0, r.height, r.width))
        });
        c
    }

    fn slot_ends(s: &Style, c: &mut Canvas, slot: Rect, state: State) {
        for x in [slot.x, slot.right() - 2] {
            let at = Rect::new(x, slot.y, 2, slot.height);
            s.draw(c, at, Part::SlotEdge, state, 1);
        }
    }

    #[test]
    fn on_a_side_bar_a_lit_idle_glass_pin_turns_its_edges_and_band_across_its_ends() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let t = s.taskbar.side_default(&theme.metrics);
        let bar = |edge: Edge, hover: Hover| {
            let (hot, held) = hover.hot_held();
            side_bar_with(&theme, s, edge, t, 0, &[0, 1], "", |st| {
                st.task_hot = hot;
                st.task_pressed = held;
            })
        };
        for edge in [Edge::Left, Edge::Right] {
            let (rest, l) = bar(edge, Hover::None);
            let r = l.tasks[0];
            let px = s.taskbar.task_icon;
            let icon = Rect::new(
                r.x + (r.width - px) / 2 - 1,
                r.y + (r.height - px) / 2 - 1,
                px + 3,
                px + 3,
            );
            for (hover, state) in [(Hover::Hot, State::Hot), (Hover::Held, State::Pressed)] {
                let got = bar(edge, hover).0;
                let mut want = rest.clone();
                if state == State::Pressed {
                    want = turned(&want, r, edge, |c, p| {
                        let band = Rect::new(2, 0, p.width - 4, p.height);
                        s.draw(c, band, Part::SlotBand, State::Pressed, 1)
                    });
                }
                want = turned(&want, r, edge, |c, p| slot_ends(s, c, p, state));
                want = turned(&want, r, edge, |c, p| {
                    let foot = Rect::new(p.width / 2 - 30, p.height - 7, 60, 7);
                    s.draw(c, foot, Part::SlotLight, state, 1)
                });
                for y in r.y..r.bottom() {
                    for x in r.x..r.right() {
                        if icon.contains(x, y) {
                            continue;
                        }
                        assert_eq!(
                            got.get(x, y),
                            want.get(x, y),
                            "{edge:?} {state:?} ({x}, {y}): the band, the edges and the light turned"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn on_a_side_bar_the_hot_glass_pager_cell_turns_its_edges_across_its_ends() {
        use crate::taskbar::PagerMark;
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let t = s.taskbar.side_default(&theme.metrics);
        for edge in [Edge::Left, Edge::Right] {
            let bar = |hot: Option<usize>| {
                side_bar_with(&theme, s, edge, t, 4, &[], "", |st| {
                    st.pager_mark = PagerMark::Light;
                    st.pager[0].current = true;
                    st.pager_hot = hot;
                })
            };
            let (rest, l) = bar(None);
            let (got, _) = bar(Some(1));
            let r = l.pager[1];
            let want = turned(&rest, r, edge, |c, p| slot_ends(s, c, p, State::Hot));
            assert_eq!(
                pixels(&got, r),
                pixels(&want, r),
                "{edge:?}: the cell's edges across its top and foot"
            );
        }
    }

    #[test]
    fn on_a_side_bar_the_glass_pager_light_lies_whole_inside_the_column() {
        use crate::taskbar::PagerMark;
        let glass = theme_for(Interface::Glass);
        let mut small = theme_for(Interface::Glass);
        crate::interfaces::small_bar(&mut small, &Default::default());
        for (theme, t) in [(&glass, 60), (&small, 44), (&glass, 140)] {
            let s = theme.look.style().unwrap();
            for edge in [Edge::Left, Edge::Right] {
                let bar = |current: bool| {
                    side_bar_with(theme, s, edge, t, 4, &[], "", |st| {
                        st.pager_mark = PagerMark::Light;
                        st.pager[0].current = current;
                    })
                };
                let ((lit, l), (dark, _)) = (bar(true), bar(false));
                let r = l.pager[0];
                let xs: Vec<i32> = (r.x..r.right())
                    .filter(|&x| {
                        (r.y..r.bottom()).any(|y| lum(lit.get(x, y)) > lum(dark.get(x, y)) + 20)
                    })
                    .collect();
                assert!(!xs.is_empty(), "{edge:?} at {t} px: the cell lit");
                let (first, last) = (xs[0], *xs.last().unwrap());
                assert!(
                    first >= 2 && last < l.width - 2,
                    "{edge:?} at {t} px: the light (x {first}..={last}) clear of the screen edge and the 2 px inner edge"
                );
            }
        }
    }

    #[test]
    fn an_unlocked_side_bar_marks_each_grip_with_a_line_across_the_column() {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        for interface in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(interface);
            let s = theme.look.style().unwrap();
            let bar = s.part(Part::Taskbar).unwrap();
            let f = bar.frame(State::Normal);
            let row = |y: i32| bar.atlas.get(f.x, f.y + y);
            let inks = match interface {
                Interface::Glass => vec![row(0), row(1)],
                _ => vec![row(GRIP_ROW)],
            };
            let text = TextRenderer::load(&FontConfig::default());
            let start = TextRenderer::load(&s.start_text.font());
            for edge in [Edge::Left, Edge::Right] {
                let span = BarSpan {
                    edge,
                    length: 600,
                    thickness: s.taskbar.side_default(&theme.metrics),
                };
                let counts = TaskbarCounts {
                    tasks: 1,
                    ..TaskbarCounts::default()
                };
                let l = TaskbarLayout::compute_on(
                    &theme.metrics,
                    &s.taskbar,
                    span,
                    false,
                    counts,
                    ClockText::default(),
                    Bands::default(),
                );
                let mut c = Canvas::new(l.width as u32, l.height as u32);
                draw_taskbar(
                    &mut c,
                    s,
                    &theme,
                    &text,
                    &start,
                    &l,
                    &state_with_tasks(&[""]),
                );
                for g in &l.grips {
                    assert!(
                        g.width > g.height,
                        "{interface:?} {edge:?}: the grip lies across"
                    );
                    for (k, &ink) in (0..).zip(&inks) {
                        let line: Vec<u32> = (g.x..g.right()).map(|x| c.get(x, g.y + k)).collect();
                        assert_eq!(
                            line,
                            vec![ink; g.width as usize],
                            "{interface:?} {edge:?}: the grip's row {k} in {ink:08x}"
                        );
                    }
                    let n = inks.len() as i32;
                    assert!(
                        !inks.contains(&c.get(g.x + g.width / 2, g.y + n)),
                        "{interface:?} {edge:?}: {n} rows, hairlines"
                    );
                }
            }
        }
    }

    #[test]
    fn a_side_bar_points_its_chevron_at_the_screen() {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let ink = s.bar_text.argb();
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        for edge in [Edge::Left, Edge::Right] {
            let counts = TaskbarCounts {
                tray: 1,
                chevron: true,
                ..TaskbarCounts::default()
            };
            let span = BarSpan {
                edge,
                length: 600,
                thickness: s.taskbar.side_default(&theme.metrics),
            };
            let l = TaskbarLayout::compute_on(
                &theme.metrics,
                &s.taskbar,
                span,
                true,
                counts,
                ClockText::default(),
                Bands::default(),
            );
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &l, &state_with_tasks(&[]));
            let r = l.chevron.unwrap();
            let count = |x: i32| (0..16).filter(|&y| c.get(r.x + x, r.y + y) == ink).count();
            let (tip, base) = if edge == Edge::Left { (9, 6) } else { (6, 9) };
            assert_eq!(
                (count(tip), count(base)),
                (1, 7),
                "{edge:?}: the tip toward the screen"
            );
        }
    }

    #[test]
    fn a_side_bar_draws_its_more_cell_with_the_double_chevron() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let (c, l) = side_bar(Interface::Glass, Edge::Left, &[1; 14], None);
        let r = l
            .more
            .expect("fourteen 40 px buttons overrun a 600 px column");
        let ink = s.bar_text.argb();
        let (x0, mid) = (r.x + (r.width - 10) / 2, r.y + r.height / 2);
        assert_eq!(c.get(x0, mid), ink, "the first >'s back");
        assert_eq!(c.get(x0 + 6, mid), ink, "the second's");
        assert_ne!(c.get(x0 + 5, mid), ink, "the gap between them");
    }

    #[test]
    fn a_side_blue_bars_more_cell_is_a_task_button_at_rest_with_the_double_chevron() {
        let theme = theme_for(Interface::Blue);
        let s = theme.look.style().unwrap();
        let (c, l) = side_bar(Interface::Blue, Edge::Left, &[1; 30], None);
        let r = l
            .more
            .expect("thirty 25 px buttons overrun a 600 px column");
        let mut bare = l.clone();
        bare.more = None;
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let mut want = Canvas::new(l.width as u32, l.height as u32);
        let st = state_with_tasks(&[""; 30]);
        draw_taskbar(&mut want, s, &theme, &text, &start, &bare, &st);
        s.draw(&mut want, r, Part::TaskButton, State::Normal, 1);
        crate::taskbar::draw_more(&mut want, r, s.bar_text, 1);
        let cell = |c: &Canvas| {
            (r.y..r.bottom())
                .flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)))
                .collect::<Vec<_>>()
        };
        assert_eq!(cell(&c), cell(&want));
        assert_eq!(
            c.get(r.x + (r.width - 10) / 2, r.y + r.height / 2),
            s.bar_text.argb(),
            "the first >'s back in the bar's ink"
        );
    }

    fn bar_with_popup_cells(
        interface: Interface,
        edge: Edge,
        tasks: usize,
        cells: impl Fn(&mut TaskbarState),
    ) -> (Canvas, Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        let span = BarSpan {
            edge,
            length: if edge.is_side() { 600 } else { 800 },
            thickness: if edge.is_side() {
                s.taskbar.side_default(&theme.metrics)
            } else {
                theme.metrics.taskbar_height
            },
        };
        let counts = TaskbarCounts {
            tasks,
            tray: 1,
            chevron: true,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute_on(
            &theme.metrics,
            &s.taskbar,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |l: &TaskbarLayout, st: &TaskbarState| {
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, l, st);
            c
        };
        let mut st = state_with_tasks(&vec![""; tasks]);
        let mut bare = l.clone();
        bare.more = None;
        bare.chevron = None;
        let under = draw(&bare, &st);
        cells(&mut st);
        (draw(&l, &st), under, l)
    }

    #[test]
    fn the_more_cell_lights_under_the_pointer_and_is_checked_while_its_menu_is_open() {
        use crate::taskbar::PopupCell;
        let cases = [
            (
                PopupCell {
                    hot: true,
                    open: false,
                    held: false,
                },
                State::Hot,
            ),
            (
                PopupCell {
                    hot: false,
                    open: true,
                    held: false,
                },
                State::Checked,
            ),
            (
                PopupCell {
                    hot: true,
                    open: true,
                    held: false,
                },
                State::Checked,
            ),
        ];
        for interface in [Interface::Blue, Interface::Glass] {
            let theme = theme_for(interface);
            let s = theme.look.style().unwrap();
            for edge in [Edge::Left, Edge::Right] {
                for (cell, look) in cases {
                    let (c, mut want, l) =
                        bar_with_popup_cells(interface, edge, 30, |st| st.more = cell);
                    let r = l.more.expect("thirty buttons overrun a 600 px column");
                    s.draw(&mut want, r, Part::TaskButton, look, 1);
                    let ink = s.text_color(Part::TaskButton, look).unwrap_or(s.bar_text);
                    crate::taskbar::draw_more(&mut want, r, ink, 1);
                    assert_eq!(
                        pixels(&c, r),
                        pixels(&want, r),
                        "{interface:?} {edge:?} {cell:?}: a {look:?} task button under the »"
                    );
                }
            }
        }
    }

    #[test]
    fn the_chevron_cell_lights_under_the_pointer_and_is_checked_while_its_flyout_is_open() {
        use crate::taskbar::PopupCell;
        let cases = [
            (
                PopupCell {
                    hot: true,
                    open: false,
                    held: false,
                },
                State::Hot,
            ),
            (
                PopupCell {
                    hot: false,
                    open: true,
                    held: false,
                },
                State::Checked,
            ),
            (
                PopupCell {
                    hot: true,
                    open: true,
                    held: false,
                },
                State::Checked,
            ),
        ];
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            for (cell, look) in cases {
                let (c, mut want, l) =
                    bar_with_popup_cells(Interface::Glass, edge, 1, |st| st.chevron = cell);
                let r = l.chevron.unwrap();
                s.draw(&mut want, r, Part::TaskButton, look, 1);
                let ink = s.text_color(Part::TaskButton, look).unwrap_or(s.bar_text);
                edge.draw_art(&mut want, r, |w, at| {
                    crate::taskbar::draw_chevron(w, at, ink, 1)
                });
                assert_eq!(
                    pixels(&c, r),
                    pixels(&want, r),
                    "{edge:?} {cell:?}: a {look:?} task button under the ^"
                );
            }
        }
    }

    #[test]
    fn a_side_glass_bars_end_cap_tops_the_column_and_its_lines_run_down_the_inner_edge() {
        let theme = theme_for(Interface::Glass);
        let s = theme.look.style().unwrap();
        let mut bottom = Canvas::new(600, 40);
        s.draw(
            &mut bottom,
            Rect::new(0, 0, 600, 40),
            Part::Taskbar,
            State::Normal,
            1,
        );
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar(Interface::Glass, edge, &[1], None);
            let w = l.width;
            let col = |k: i32| if edge == Edge::Left { w - 1 - k } else { k };
            assert!(
                l.start.bottom() < 104,
                "the end cap's rows are among those checked"
            );
            let rows =
                (l.start.bottom()..l.tasks[0].y).chain(l.tasks[0].bottom()..l.notification.y);
            for y in rows {
                for k in 0..2 {
                    assert_eq!(
                        c.get(col(k), y),
                        bottom.get(y, k),
                        "{edge:?}: row {y}, the inner edge's column {k} is the bottom bar's row {k} at {y} px along"
                    );
                }
            }
        }
    }

    #[test]
    fn a_small_glass_side_bar_draws_its_whole_disc_inside_the_column() {
        let mut small = theme_for(Interface::Glass);
        crate::interfaces::small_bar(&mut small, &Default::default());
        let s = small.look.style().unwrap();
        let size = |p: Part| {
            let f = s.part(p).unwrap().frame(State::Normal);
            (f.width, f.height)
        };
        assert_eq!(
            (size(Part::StartButton), size(Part::StartAbove)),
            ((54, 30), (54, 13)),
            "the small orb's button art and the 13 rows over it"
        );
        for edge in [Edge::Left, Edge::Right] {
            let (c, l) = side_bar_in(&small, s, edge, &[1], None);
            let st = l.start;
            assert_eq!(st, Rect::new(0, 0, 44, 35), "{edge:?}");
            let bar = || {
                let mut b = Canvas::new(l.width as u32, l.height as u32);
                b.with_transposed(
                    Rect::new(0, 0, l.width, l.height),
                    edge == Edge::Left,
                    |a| {
                        s.draw(
                            a,
                            Rect::new(0, 0, l.height, l.width),
                            Part::Taskbar,
                            State::Normal,
                            1,
                        )
                    },
                );
                b
            };
            let mut want = bar();
            want.with_clip(st, |w| {
                s.draw(
                    w,
                    Rect::new(-5, -8, 54, 13),
                    Part::StartAbove,
                    State::Normal,
                    1,
                );
                s.draw(
                    w,
                    Rect::new(-5, 5, 54, 30),
                    Part::StartButton,
                    State::Normal,
                    1,
                );
            });
            assert_eq!(
                pixels(&c, st),
                pixels(&want, st),
                "{edge:?}: the orb at its own size, not stretched over Start"
            );
            let bare = bar();
            let disc = (0..5).filter(|&y| c.get(22, y) != bare.get(22, y)).count();
            assert_eq!(
                disc, 5,
                "{edge:?}: the disc's top five rows stand above the button art"
            );
            assert_eq!(
                c.get(1, 1),
                bare.get(1, 1),
                "{edge:?}: beside the disc, the bar alone"
            );
        }
    }

    #[test]
    fn a_hidden_side_bar_leaves_its_inner_two_columns_on_screen() {
        for i in [Interface::Blue, Interface::Glass] {
            for edge in [Edge::Left, Edge::Right] {
                let (c, l) = side_bar(i, edge, &[1], None);
                let inner = if edge == Edge::Left {
                    Rect::new(l.width - 2, 0, 2, 600)
                } else {
                    Rect::new(0, 0, 2, 600)
                };
                assert_eq!(l.reveal_line(), inner, "{i:?} {edge:?}");
                let got = c.copy_rect(inner);
                let y = l.start.bottom() + 1;
                assert_ne!(got.get(0, y) >> 24, 0, "{i:?} {edge:?}: the line shows");
                assert_ne!(got.get(1, y) >> 24, 0, "{i:?} {edge:?}");
                if i == Interface::Glass {
                    assert_ne!(
                        got.get(0, y),
                        got.get(1, y),
                        "{edge:?}: a dark line and a light one"
                    );
                }
            }
        }
    }
}
