//! Task buttons with their spots, stacked edges and slot lights, and the pager's tiles.

use crate::raster::{Canvas, Rect};
use crate::style::{Part, PartImage, SpotFit, SpotFrames, State, Style};
use crate::taskbar::Edge;
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

pub(super) fn draw_image_nth(
    canvas: &mut Canvas,
    p: &PartImage,
    r: Rect,
    state: State,
    n: usize,
    s: i32,
) {
    p.blit(canvas, p.frame_nth(state, n), r, s);
}

pub(super) fn design_size(p: &PartImage, state: State) -> (i32, i32) {
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

pub(super) fn is_lit(state: State) -> bool {
    matches!(state, State::Hot | State::CheckedHot | State::Pressed)
}

#[derive(Clone, Copy)]
pub(super) struct Span {
    pub(super) x: i32,
    pub(super) right: i32,
    pub(super) pointer: Option<i32>,
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
pub(super) struct Spot<'a> {
    pub(super) fit: &'a SpotFit,
    pub(super) span: Span,
    pub(super) glow: Option<Color>,
}

pub(super) fn draw_spot(
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
pub(super) fn draw_task_button(
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

pub(super) fn label_pair(style: &Style, r: Rect, s: i32) -> i32 {
    (style
        .part(Part::TaskLabelEdge)
        .map_or(0, |p| design_size(p, State::Normal).0)
        * s)
        .min(r.width / 2)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_labelled(
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
    let edges_have = |st| {
        style
            .part(Part::TaskLabelEdge)
            .is_some_and(|p| p.states.contains(&st))
    };
    let own = if state == State::Attention && !edges_have(state) {
        State::Checked
    } else {
        state
    };
    let left = match prev {
        None => (State::Normal, EDGE_ROW_START),
        Some(p) if rest && is_lit(p) => (p, EDGE_NEXT_LEFT),
        Some(_) => (own, EDGE_LEFT),
    };
    let right = match next {
        None => (State::Normal, EDGE_ROW_END),
        Some(n) if rest && is_lit(n) => (n, EDGE_PREV_RIGHT),
        Some(_) => (own, EDGE_RIGHT),
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

pub(super) fn lights_slots(style: &Style) -> bool {
    style.has_parts(&[Part::SlotEdge, Part::SlotLight])
}

/// Whether a lit or held task button draws a spot at the pointer: the style spots its buttons and
/// the button is not an `idle` pinned slot lit by its slot light. A host redraws on pointer motion
/// only over such a button.
pub fn task_draws_spot(style: &Style, idle: bool) -> bool {
    style.task_spot.is_some() && !(idle && lights_slots(style))
}

pub(super) fn draw_slot_light(
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

pub(super) fn draw_slot_edges(
    canvas: &mut Canvas,
    style: &Style,
    r: Rect,
    state: State,
    s: i32,
    edge: Edge,
) {
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

pub(super) fn draw_slot_light_foot(
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

pub(super) fn pager_tile_rect(r: Rect, s: i32) -> Rect {
    let h = if r.height >= 40 * s { 24 } else { 20 } * s;
    Rect::new(r.x + s, r.y + (r.height - h) / 2, 21 * s, h)
}

/// Ink alpha (70%) of a pager number for a desktop other than the current one.
pub(super) const PAGER_DIM: u32 = 0xB3;

pub(super) fn draw_dimmed_number(
    canvas: &mut Canvas,
    text: &TextRenderer,
    r: Rect,
    label: &str,
    ink: Color,
    alpha: u32,
    size: u32,
) {
    let scratch = canvas.scratch(r.width.max(0) as u32, r.height.max(0) as u32);
    let (w, h) = scratch.device_size();
    let local = Rect::new(0, 0, r.width, r.height);
    let coverage: Vec<u8> = scratch
        .ink_coverage(|c, white| {
            text.draw(c, local, label, false, white, Align::Center, size);
        })
        .into_iter()
        .map(|c| (u32::from(c) * alpha / 255) as u8)
        .collect();
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

pub(super) fn stacked(style: &Style, slot: Rect, stack: Option<u8>, s: i32) -> (Rect, Vec<Rect>) {
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
