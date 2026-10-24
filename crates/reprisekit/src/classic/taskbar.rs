//! The taskbar drawn in the Classic look.

use crate::art::logo;
use crate::icon;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::taskbar::{TaskbarHit, TaskbarLayout, TaskbarState, quick_icon_origin};
use crate::text::{Align, TextRenderer};
use crate::theme::{ColorScheme, Theme};

use super::widgets::draw_focus_ring;

/// The dotted ring of a focused Start; it may reach past Start's cell.
pub(crate) fn start_ring(
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskbarLayout,
    pressed: bool,
) -> Rect {
    let s = theme.scale.max(1);
    let st = layout.start;
    let o = if pressed { s } else { 0 };
    let tw = text.measure("Start", true, theme.fonts.size_px);
    Rect::new(
        st.x + 19 * s + o,
        st.y + 5 * s,
        tw + 4 * s,
        st.height - 6 * s,
    )
}

/// Draw into `canvas` sized to `layout` (already scaled to `theme.scale`). The
/// hilight line and ^ are turned to `layout.edge`; the rest draws upright at the layout's rects.
pub fn draw_taskbar(
    canvas: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    layout: &TaskbarLayout,
    state: &TaskbarState,
) {
    let cs = &theme.colors;
    let s = theme.scale.max(1);
    let size = theme.fonts.size_px;
    let icon_px = theme.icon_size();
    let full = Rect::new(0, 0, layout.width, layout.height);
    canvas.fill_rect(full, cs.button_face);
    layout.edge.draw_art(canvas, full, |c, at| {
        c.fill_rect(Rect::new(at.x, at.y + s, at.width, s), cs.button_hilight)
    });

    let st = layout.start;
    let style = if state.start_pressed {
        EdgeStyle::ButtonPushed
    } else {
        EdgeStyle::ButtonRaised
    };
    canvas.edge_t(st, style, cs, s);
    let o = if state.start_pressed { s } else { 0 };
    match &state.start_icon {
        Some(icon) => canvas.blit_over(
            st.x + 2 * s + o,
            st.y + (st.height - icon.height as i32) / 2 + o,
            icon,
        ),
        None => logo::draw(
            canvas,
            st.x + 2 * s + o,
            st.y + (st.height - 16 * s) / 2 + o,
            16 * s,
            cs.active_title,
        ),
    }
    let label = Rect::new(
        st.x + 19 * s + o,
        st.y + o,
        (st.width - 21 * s).max(0),
        st.height,
    );
    text.draw(
        canvas,
        label,
        "Start",
        true,
        cs.button_text,
        Align::Left,
        size,
    );
    if state.start_focus || state.focus == Some(crate::taskbar::TaskbarHit::Start) {
        let ring = start_ring(theme, text, layout, state.start_pressed);
        draw_focus_ring(canvas, ring, s, cs.button_text);
    }

    for g in &layout.grips {
        canvas.edge_t(*g, EdgeStyle::ThinRaised, cs, s);
    }

    let mut fallback: Option<Canvas> = None;
    for (i, r) in layout.quick.iter().enumerate() {
        let o = thin_cell(
            canvas,
            cs,
            *r,
            state.quick_pressed == Some(i),
            state.quick_hot == Some(i),
            s,
        );
        let (ix, iy) = quick_icon_origin(*r, icon_px, s);
        let icon = state.quick_icons.get(i).and_then(|ic| ic.as_deref());
        blit_icon(canvas, theme, &mut fallback, (ix + o, iy + o), icon);
    }
    if let Some(r) = layout.quick_more {
        let cell = state.quick_more;
        let o = thin_cell(canvas, cs, r, cell.open, cell.hot, s);
        let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
        crate::taskbar::draw_more(canvas, at, cs.button_text, s);
    }
    if let (Some(slot), Some(first)) = (state.quick_drop, layout.quick.first()) {
        let ink = cs.button_dk_shadow;
        let last = layout.quick.last().unwrap_or(first);
        if layout.edge.is_side() {
            let (x, y, w) = match layout.quick.get(slot) {
                Some(r) => (r.x, r.y, r.width),
                None => (last.x, last.bottom(), last.width),
            };
            canvas.fill_rect(Rect::new(x, y, w, 2 * s), ink);
            for (dx, h) in [(0, 6), (s, 4), (w - 2 * s, 4), (w - s, 6)] {
                canvas.fill_rect(Rect::new(x + dx, y + s - h * s / 2, s, h * s), ink);
            }
        } else {
            let x = match layout.quick.get(slot) {
                Some(r) => r.x,
                None => last.right(),
            };
            let (y, h) = (first.y, first.height);
            canvas.fill_rect(Rect::new(x, y, 2 * s, h), ink);
            for (dy, w) in [(0, 6), (s, 4), (h - 2 * s, 4), (h - s, 6)] {
                canvas.fill_rect(Rect::new(x + s - w * s / 2, y + dy, w * s, s), ink);
            }
        }
    }

    for (i, (r, b)) in layout.pager.iter().zip(&state.pager).enumerate() {
        let inner = r.inset(s);
        if b.current {
            canvas.dither_rect_t(inner, cs.button_hilight, cs.button_face, s);
            canvas.edge_t(*r, EdgeStyle::ThinSunken, cs, s);
        } else if state.pager_hot == Some(i) {
            canvas.edge_t(*r, EdgeStyle::ThinRaised, cs, s);
        }
        if b.attention && !b.current {
            canvas.fill_rect(inner, cs.hilight);
        }
        let o = if b.current { s } else { 0 };
        let ink = if b.attention && !b.current {
            cs.hilight_text
        } else {
            cs.button_text
        };
        let label = Rect::new(r.x + o, r.y + o, r.width, r.height);
        text.draw(
            canvas,
            label,
            &(i + 1).to_string(),
            false,
            ink,
            Align::Center,
            size,
        );
    }

    let count = layout.tasks.len().min(state.tasks.len());
    let float = state.task_float.filter(|&(i, _)| i < count);
    let floated = |i: usize| float.is_some_and(|(f, _)| f == i);
    let order = (0..count)
        .filter(|&i| !floated(i))
        .chain(float.map(|(f, _)| f));
    for i in order {
        let task = &state.tasks[i];
        let slot = layout.tasks[i];
        let r = &match float {
            Some((f, a)) if f == i => layout.placed_along(slot, a * s),
            _ => layout.moved_along(slot, state.task_shift.get(i).copied().unwrap_or(0) * s),
        };
        canvas.fill_rect(*r, cs.button_face);
        if layout.pinned_idle(i) {
            if state.task_hot == Some(i) || floated(i) {
                canvas.edge_t(*r, EdgeStyle::ThinRaised, cs, s);
            }
            let at = (
                r.x + (r.width - icon_px) / 2,
                r.y + (r.height - icon_px) / 2,
            );
            blit_icon(canvas, theme, &mut fallback, at, task.icon.as_deref());
            continue;
        }
        if task.active {
            canvas.dither_rect_t(r.inset(2 * s), cs.button_hilight, cs.button_face, s);
            canvas.edge_t(*r, EdgeStyle::ButtonPushed, cs, s);
        } else if task.attention {
            canvas.edge_t(*r, EdgeStyle::ButtonPushed, cs, s);
        } else {
            canvas.edge_t(*r, EdgeStyle::ButtonRaised, cs, s);
        }
        if let Some(pr) = state.task_progress.get(i).copied().flatten() {
            let art = crate::classic::taskprogress::paint(&pr, cs, r.width / s, r.height / s, s);
            canvas.blit_over(r.x, r.y, &art);
        }
        let pressed = task.active || task.attention;
        let o = if pressed { s } else { 0 };
        let icon_at = (r.x + 4 * s + o, r.y + (r.height - icon_px) / 2 + o);
        blit_icon(canvas, theme, &mut fallback, icon_at, task.icon.as_deref());
        if layout.icons_only {
            continue;
        }
        let label = Rect::new(
            r.x + 24 * s + o,
            r.y + o,
            (r.width - 28 * s).max(0),
            r.height,
        );
        let title = text.ellipsize(&task.title, label.width, pressed, size);
        text.draw(
            canvas,
            label,
            &title,
            pressed,
            cs.button_text,
            Align::Left,
            size,
        );
    }

    if let Some(r) = layout.more {
        let (frame, o) = if state.more.open {
            (EdgeStyle::ButtonPushed, s)
        } else {
            (EdgeStyle::ButtonRaised, 0)
        };
        canvas.fill_rect(r, cs.button_face);
        canvas.edge_t(r, frame, cs, s);
        let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
        crate::taskbar::draw_more(canvas, at, cs.button_text, s);
    }

    canvas.edge_t(layout.notification, EdgeStyle::ThinSunken, cs, s);

    for (r, icon) in layout.tray.iter().zip(&state.tray_icons) {
        if let Some(ic) = icon {
            canvas.blit_canvas(r.x, r.y, ic);
        }
    }
    if let Some(r) = layout.chevron {
        let ink = cs.button_text;
        if layout.tray_arrow {
            let side = layout.edge.is_side();
            let lit =
                state.chevron.hot || state.chevron.held || state.focus == Some(TaskbarHit::Chevron);
            if side || lit {
                canvas.edge_t(r, EdgeStyle::ThinRaised, cs, s);
            }
            let open = state.chevron.open;
            if side {
                crate::taskbar::draw_tray_arrow_side(canvas, r, ink, s, open);
            } else {
                layout.edge.draw_art(canvas, r, |c, at| {
                    crate::taskbar::draw_tray_arrow(c, at, ink, s, open)
                });
            }
        } else {
            let o = thin_cell(canvas, cs, r, state.chevron.open, state.chevron.hot, s);
            let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
            layout.edge.draw_art(canvas, at, |c, at| {
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
            cs.button_text,
            size,
            s,
        ),
        _ => text.draw(
            canvas,
            layout.clock.inset(s),
            &state.clock,
            false,
            cs.button_text,
            Align::Center,
            size,
        ),
    }
    if let Some(r) = layout.show_desktop {
        canvas.edge_t(r, EdgeStyle::ThinRaised, cs, s);
    }
}

/// A thin quick-launch or tray cell: sunken when `down`, else raised when `hot`. Returns how far
/// its content moves right and down.
fn thin_cell(c: &mut Canvas, cs: &ColorScheme, r: Rect, down: bool, hot: bool, s: i32) -> i32 {
    if down {
        c.edge_t(r, EdgeStyle::ThinSunken, cs, s);
        s
    } else {
        if hot {
            c.edge_t(r, EdgeStyle::ThinRaised, cs, s);
        }
        0
    }
}

fn blit_icon(
    c: &mut Canvas,
    theme: &Theme,
    fallback: &mut Option<Canvas>,
    at: (i32, i32),
    icon: Option<&Canvas>,
) {
    let icon = icon.unwrap_or_else(|| {
        fallback.get_or_insert_with(|| icon::fallback(theme, theme.icon_size()))
    });
    c.blit_canvas(at.0, at.1, icon);
}

#[cfg(test)]
mod tests;
