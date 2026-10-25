//! The Start button: the labelled pill and the orb with its bloom.

use std::sync::Arc;

use super::buttons::design_size;
use super::start_state;
use crate::art::logo;
use crate::raster::{Canvas, Rect};
use crate::style::startbloom::{Bloom, IconKey};
use crate::style::{Part, State, Style};
use crate::taskbar::{Edge, TaskbarLayout, TaskbarState};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

fn draw_start_mark(canvas: &mut Canvas, x: i32, top: i32, box_px: i32, s: i32, ink: Color) {
    // Centred in design pixels, so the mark at scale `s` lands where scale 1 puts it.
    let off = s * ((box_px / s - 16) / 2);
    logo::draw(canvas, x + off, top + off, 16 * s, ink);
}

pub(super) fn draw_start_pill(
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
    let scratch = canvas.scratch(at.width as u32, at.height as u32);
    let (w, h) = scratch.device_size();
    let inner = Rect::new(reach, reach, label.width, label.height);
    let mut coverage = scratch.ink_coverage(|c, white| {
        start_text.draw(c, inner, "start", spec.bold, white, Align::Left, start_size);
    });
    spec.weigh(&mut coverage);
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

pub(super) fn draw_orb(
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
pub(super) const ORB_MARK_SHARE: (i32, i32) = (5, 6);

/// Where the orb's icon goes, in the bar's device px: centred on the disc, the icon theme's logo at
/// its own size, else the mark at [`ORB_MARK_SHARE`] of the shape's `start_icon` size.
pub(super) fn start_icon_box(
    style: &Style,
    layout: &TaskbarLayout,
    state: &TaskbarState,
    s: i32,
) -> Rect {
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
