//! The taskbar drawn in the Classic look.

use crate::art::logo;
use crate::icon;
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::taskbar::{TaskbarHit, TaskbarLayout, TaskbarState, quick_icon_origin};
use crate::text::{Align, TextRenderer};
use crate::theme::Theme;

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
        let tw = text.measure("Start", true, size);
        let ring = Rect::new(label.x, st.y + 5 * s, tw + 4 * s, st.height - 6 * s);
        for x in (ring.x..ring.right()).step_by(2 * s as usize) {
            canvas.fill_rect(Rect::new(x, ring.y, s, s), cs.button_text);
            canvas.fill_rect(Rect::new(x, ring.bottom() - s, s, s), cs.button_text);
        }
        for y in (ring.y..ring.bottom()).step_by(2 * s as usize) {
            canvas.fill_rect(Rect::new(ring.x, y, s, s), cs.button_text);
            canvas.fill_rect(Rect::new(ring.right() - s, y, s, s), cs.button_text);
        }
    }

    for g in &layout.grips {
        canvas.edge_t(*g, EdgeStyle::ThinRaised, cs, s);
    }

    let mut fallback: Option<Canvas> = None;
    for (i, r) in layout.quick.iter().enumerate() {
        let pressed = state.quick_pressed == Some(i);
        if pressed {
            canvas.edge_t(*r, EdgeStyle::ThinSunken, cs, s);
        } else if state.quick_hot == Some(i) {
            canvas.edge_t(*r, EdgeStyle::ThinRaised, cs, s);
        }
        let o = if pressed { s } else { 0 };
        let (ix, iy) = quick_icon_origin(*r, icon_px, s);
        let at = (ix + o, iy + o);
        match state.quick_icons.get(i) {
            Some(Some(ic)) => canvas.blit_canvas(at.0, at.1, ic),
            _ => {
                let fb = fallback.get_or_insert_with(|| icon::fallback(theme, icon_px));
                canvas.blit_canvas(at.0, at.1, fb)
            }
        }
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
            match &task.icon {
                Some(ic) => canvas.blit_canvas(at.0, at.1, ic),
                None => {
                    let fb = fallback.get_or_insert_with(|| icon::fallback(theme, icon_px));
                    canvas.blit_canvas(at.0, at.1, fb)
                }
            }
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
        let pressed = task.active || task.attention;
        let o = if pressed { s } else { 0 };
        let icon_at = (r.x + 4 * s + o, r.y + (r.height - icon_px) / 2 + o);
        match &task.icon {
            Some(ic) => canvas.blit_canvas(icon_at.0, icon_at.1, ic),
            None => {
                let fb = fallback.get_or_insert_with(|| icon::fallback(theme, icon_px));
                canvas.blit_canvas(icon_at.0, icon_at.1, fb)
            }
        }
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
            let o = if state.chevron.open {
                canvas.edge_t(r, EdgeStyle::ThinSunken, cs, s);
                s
            } else {
                if state.chevron.hot {
                    canvas.edge_t(r, EdgeStyle::ThinRaised, cs, s);
                }
                0
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taskbar::{Edge, PagerButton, TaskEntry, TaskbarCounts, TaskbarShape};
    use crate::theme::{Color, FontConfig, Metrics};

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
    fn drawing_basics() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 2, 0),
            text.measure("00:00", false, theme.fonts.size_px),
        );
        let state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![
                TaskEntry {
                    title: "One".into(),
                    active: true,
                    attention: false,
                    icon: None,
                    glow: None,
                },
                TaskEntry {
                    title: "Two".into(),
                    active: false,
                    attention: false,
                    icon: None,
                    glow: None,
                },
            ],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        assert_eq!(c.get(400, 0), cs.button_face.argb());
        assert_eq!(c.get(400, 1), cs.button_hilight.argb());
        assert_eq!(c.get(400, 15), cs.button_face.argb());
        assert_eq!(c.get(l.start.x, l.start.y + 10), cs.button_hilight.argb());
        let t0 = l.tasks[0];
        assert_eq!(c.get(t0.x, t0.y + 10), cs.button_dk_shadow.argb());
        assert_eq!(c.get(t0.x + 1, t0.y + 10), cs.button_shadow.argb());
        let interior = Rect::new(t0.x + 22, t0.y + 2, 4, 4);
        let has_hilight = (interior.x..interior.right()).any(|x| {
            (interior.y..interior.bottom()).any(|y| c.get(x, y) == cs.button_hilight.argb())
        });
        assert!(has_hilight);
        for x in [t0.x + 30, t0.x + 31] {
            let y = t0.y + 2;
            let want = if (x + y) % 2 == 0 {
                cs.button_hilight
            } else {
                cs.button_face
            };
            assert_eq!(c.get(x, y), want.argb(), "dither at {x},{y}");
        }
        let icon_y = t0.y + (t0.height - theme.icon_size()) / 2;
        assert_eq!(c.get(t0.x + 5, icon_y + 1), cs.button_dk_shadow.argb());
        let t1 = l.tasks[1];
        assert_eq!(c.get(t1.x, t1.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(l.clock.x, l.clock.y + 10), cs.button_shadow.argb());
    }

    #[test]
    fn a_150_percent_bar_grows_around_its_titles_and_clock() {
        let theme = Theme::default().with_text_size(150);
        let cs = &theme.colors;
        assert_eq!(theme.metrics.taskbar_height, 35);
        if !crate::text::test_tahoma() {
            return;
        }
        let text = TextRenderer::load(&theme.fonts);
        let size = theme.fonts.size_px;
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 1, 0),
            text.measure("12:00 PM", false, size),
        );
        let state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![TaskEntry {
                title: "Notepad".into(),
                active: false,
                attention: false,
                icon: None,
                glow: None,
            }],
            tray_icons: vec![],
            clock: "12:00 PM".into(),
        };
        let mut c = Canvas::new(800, 35);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let ink = cs.button_text.argb();
        let ink_rows = |r: Rect, from: i32| -> Vec<i32> {
            (r.y + 2..r.bottom() - 2)
                .filter(|&y| (r.x + from..r.right() - 3).any(|x| c.get(x, y) == ink))
                .map(|y| y - r.y)
                .collect()
        };
        let t = l.tasks[0];
        assert_eq!(t.height, 29, "the button keeps 3 px above and below");
        let title = ink_rows(t, 24);
        assert_eq!(
            (title[0], title[title.len() - 1]),
            (7, 24),
            "the title's ink rows"
        );
        let clock = ink_rows(l.clock, 2);
        assert_eq!(
            (clock[0], clock[clock.len() - 1]),
            (8, 20),
            "the clock's ink rows"
        );
        assert!(
            text.measure("12:00 PM", false, size) < l.clock.width - 4,
            "the clock is wide enough for the large font"
        );
    }

    #[test]
    fn drawing_at_scale_two_is_twice_the_pixels() {
        let theme = Theme::default().scaled(2);
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let logical = TaskbarLayout::compute(
            &Theme::default().metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 1, 0),
            text.measure("00:00", false, 11),
        );
        let l = logical.scaled(2);
        let state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![TaskEntry {
                title: "One".into(),
                active: true,
                attention: false,
                icon: None,
                glow: None,
            }],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(1600, 60);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        assert_eq!(c.get(400, 1), cs.button_face.argb());
        assert_eq!(c.get(400, 2), cs.button_hilight.argb());
        assert_eq!(c.get(400, 3), cs.button_hilight.argb());
        assert_eq!(c.get(400, 4), cs.button_face.argb());
        assert_eq!(c.get(l.start.x, l.start.y + 20), cs.button_hilight.argb());
        assert_eq!(
            c.get(l.start.x + 1, l.start.y + 20),
            cs.button_hilight.argb()
        );
        assert_eq!(c.get(l.start.x + 2, l.start.y + 20), cs.button_light.argb());
        let t0 = l.tasks[0];
        let icon_y = t0.y + (t0.height - 32) / 2 + 2;
        assert_eq!(c.get(t0.x + 10, icon_y), cs.button_dk_shadow.argb());
        assert_eq!(c.get(t0.x + 11, icon_y + 1), cs.button_dk_shadow.argb());
        let fy = l.start.y + (l.start.height - 32) / 2;
        crate::art::logo::assert_drawn(&c, l.start.x + 4, fy, 32, cs.active_title);
        assert_eq!(
            c.get(l.start.x + 4, fy),
            cs.button_face.argb(),
            "its clear corner"
        );
    }

    #[test]
    fn a_pinned_app_without_windows_is_a_bare_icon() {
        use crate::taskbar::TaskRow;
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 2, 0),
            30,
        );
        l.lay_tasks(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            &[0, 1],
            TaskRow::Shared,
        );
        let entry = |title: &str| TaskEntry {
            title: title.into(),
            active: false,
            attention: false,
            icon: None,
            glow: None,
        };
        let mut state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![entry(""), entry("Two")],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let draw = |state: &TaskbarState| {
            let mut c = Canvas::new(800, 30);
            draw_taskbar(&mut c, &theme, &text, &l, state);
            c
        };
        let c = draw(&state);
        let p = l.tasks[0];
        assert_eq!(p, Rect::new(59, 4, 22, 22));
        for (x, y) in [(p.x, p.y + 10), (p.right() - 1, p.y + 10), (p.x + 10, p.y)] {
            assert_eq!(c.get(x, y), cs.button_face.argb(), "face at {x},{y}");
        }
        assert_eq!(c.get(62, 8), cs.button_dk_shadow.argb());
        assert_eq!(c.get(l.tasks[1].x, 14), cs.button_hilight.argb());
        state.task_hot = Some(0);
        let c = draw(&state);
        assert_eq!(c.get(p.x, p.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(p.right() - 1, p.y + 10), cs.button_shadow.argb());
    }

    #[test]
    fn attention_button_is_sunken_without_dither() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let clock_w = text.measure("00:00", false, theme.fonts.size_px);
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 1, 0),
            clock_w,
        );
        let state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![TaskEntry {
                title: "Ping".into(),
                active: false,
                attention: true,
                icon: None,
                glow: None,
            }],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let t0 = l.tasks[0];
        assert_eq!(c.get(t0.x, t0.y + 10), cs.button_dk_shadow.argb());
        let interior = Rect::new(t0.x + 22, t0.y + 2, 4, 4);
        for x in interior.x..interior.right() {
            for y in interior.y..interior.bottom() {
                assert_eq!(c.get(x, y), cs.button_face.argb(), "dither at {x},{y}");
            }
        }
    }

    #[test]
    fn tray_icons_are_drawn_at_their_rects() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 0, 1),
            text.measure("00:00", false, 11),
        );
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(0, 0, 16, 16), Color::rgb(1, 2, 3));
        let state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![Some(icon.into())],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let r = l.tray[0];
        assert_eq!(c.get(r.x + 8, r.y + 8), Color::rgb(1, 2, 3).argb());
        assert_eq!(c.get(r.x - 1, r.y + 8), theme.colors.button_face.argb());
    }

    #[test]
    fn pager_pushes_the_current_desktop_and_marks_attention() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let counts = TaskbarCounts {
            quick: 0,
            pager: 3,
            tasks: 0,
            tray: 0,
            show_desktop: false,
            chevron: false,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts,
            30,
        );
        let state = TaskbarState {
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
            pager: vec![
                PagerButton {
                    current: true,
                    attention: false,
                },
                PagerButton {
                    current: false,
                    attention: true,
                },
                PagerButton::default(),
            ],
            pager_hot: Some(2),
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let has = |r: Rect, color: crate::theme::Color| {
            (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == color.argb()))
        };
        let p0 = l.pager[0];
        assert_eq!(c.get(p0.x, p0.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(p0.right() - 1, p0.y + 10), cs.button_hilight.argb());
        for x in [p0.x + 2, p0.x + 3] {
            let y = p0.y + 2;
            let want = if (x + y) % 2 == 0 {
                cs.button_hilight
            } else {
                cs.button_face
            };
            assert_eq!(c.get(x, y), want.argb(), "dither at {x},{y}");
        }
        assert!(has(p0.inset(2), cs.button_text));
        let p1 = l.pager[1];
        assert_eq!(c.get(p1.x + 1, p1.y + 1), cs.hilight.argb());
        assert_eq!(c.get(p1.x, p1.y + 10), cs.button_face.argb());
        assert!(has(p1.inset(2), cs.hilight_text));
        let p2 = l.pager[2];
        assert_eq!(c.get(p2.x, p2.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(p2.right() - 1, p2.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(p2.x + 1, p2.y + 1), cs.button_face.argb());
        assert!(has(p2.inset(2), cs.button_text));
    }

    #[test]
    fn keyboard_opened_start_menu_rings_the_start_label() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 0, 0),
            30,
        );
        let state = TaskbarState {
            start_pressed: true,
            start_focus: true,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let cs = &theme.colors;
        let st = l.start;
        let (x0, y0) = (st.x + 20, st.y + 5);
        assert_eq!(c.get(x0, y0), cs.button_text.argb());
        assert_ne!(c.get(x0 + 1, y0), cs.button_text.argb());
        assert_eq!(c.get(x0 + 2, y0), cs.button_text.argb());
        assert_eq!(c.get(x0, y0 + 2), cs.button_text.argb());
        assert_eq!(c.get(x0, st.y + 5 + 15), cs.button_text.argb());
        let plain = TaskbarState {
            start_focus: false,
            focus: None,
            start_icon: None,
            orb_glow: 100,
            ..state
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &plain);
        assert_ne!(c.get(x0, y0), cs.button_text.argb());
    }

    #[test]
    fn start_button_shows_the_theme_icon_in_place_of_the_mark() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &Metrics::default(),
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(0, 0, 0),
            30,
        );
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(0, 0, 16, 16), Color::rgb(1, 2, 3));
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: Some(icon.into()),
            orb_glow: 100,
            quick_icons: vec![],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let st = l.start;
        let icon_px = Color::rgb(1, 2, 3).argb();
        assert_eq!(c.get(st.x + 2, st.y + 3), icon_px);
        assert_eq!(c.get(st.x + 17, st.y + 18), icon_px);
        assert_ne!(c.get(st.x + 1, st.y + 3), icon_px);
        assert_ne!(c.get(st.x + 18, st.y + 18), icon_px);
        assert_ne!(c.get(st.x + 2, st.y + 2), icon_px);
        assert_ne!(c.get(st.x + 2, st.y + 19), icon_px);
        let plain = TaskbarState {
            start_icon: None,
            orb_glow: 100,
            ..state
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &plain);
        let cs = &theme.colors;
        assert_eq!(c.get(st.x + 4, st.y + 6), cs.active_title.argb());
        let primaries = [(255, 0, 0), (0, 128, 0), (0, 0, 255), (255, 255, 0)]
            .map(|(r, g, b)| Color::rgb(r, g, b).argb());
        for y in st.y..st.bottom() {
            for x in st.x..st.right() {
                assert!(!primaries.contains(&c.get(x, y)), "({x}, {y})");
            }
        }
    }

    #[test]
    fn pressed_quick_launch_button_is_sunken_and_a_drag_marks_its_slot() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            false,
            counts(2, 0, 0),
            30,
        );
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![None, None],
            quick_hot: None,
            quick_pressed: Some(0),
            quick_drop: Some(2),
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let q0 = l.quick[0];
        assert_eq!(c.get(q0.x, q0.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(q0.right() - 1, q0.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(q0.x + 3, q0.y + 3), cs.button_face.argb());
        assert_eq!(c.get(q0.x + 4, q0.y + 4), cs.button_dk_shadow.argb());
        let end = l.quick[1].right();
        let ink = cs.button_dk_shadow.argb();
        assert_eq!(c.get(end - 1, 10), cs.button_face.argb());
        assert_eq!(c.get(end, 10), ink);
        assert_eq!(c.get(end + 1, 10), ink);
        assert_eq!(c.get(end + 2, 10), cs.button_face.argb());
        assert_eq!(c.get(end, 3), cs.button_face.argb());
        assert_eq!(c.get(end - 2, 4), ink);
        assert_eq!(c.get(end + 3, 4), ink);
        assert_eq!(c.get(end - 2, 5), cs.button_face.argb());
        assert_eq!(c.get(end - 1, 5), ink);
        assert_eq!(c.get(end - 1, 24), ink);
        assert_eq!(c.get(end + 3, 25), ink);
        assert_eq!(c.get(end, 26), cs.button_face.argb());
        let g = l.grips[0];
        assert_eq!(c.get(g.x, g.y + 5), cs.button_hilight.argb());
        assert_eq!(c.get(g.x + 1, g.y + 5), cs.button_face.argb());
        assert_eq!(c.get(g.x + 2, g.y + 5), cs.button_shadow.argb());
    }

    #[test]
    fn hot_quick_launch_button_gets_a_thin_raised_frame() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts(2, 0, 0),
            30,
        );
        let mut icon = Canvas::new(16, 16);
        icon.fill_rect(Rect::new(0, 0, 16, 16), Color::rgb(1, 2, 3));
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![Some(icon.into()), None],
            quick_hot: Some(1),
            quick_pressed: None,
            quick_drop: None,
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![],
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 30);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        let (q0, q1) = (l.quick[0], l.quick[1]);
        assert_eq!(c.get(q0.x + 3, q0.y + 3), Color::rgb(1, 2, 3).argb());
        assert_eq!(c.get(q0.x + 18, q0.y + 18), Color::rgb(1, 2, 3).argb());
        assert_eq!(c.get(q0.x, q0.y + 10), cs.button_face.argb());
        assert_eq!(c.get(q1.x, q1.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(q1.x + 10, q1.y), cs.button_hilight.argb());
        assert_eq!(c.get(q1.right() - 1, q1.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(q1.x + 10, q1.bottom() - 1), cs.button_shadow.argb());
        assert_eq!(c.get(q1.x + 1, q1.y + 1), cs.button_face.argb());
        assert_eq!(c.get(q1.x + 3, q1.y + 3), cs.button_dk_shadow.argb());
    }

    #[test]
    fn the_show_desktop_strip_is_a_thin_raised_button_after_the_clock() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = TextRenderer::load(&FontConfig::default());
        let c = |show_desktop| TaskbarCounts {
            tasks: 1,
            show_desktop,
            ..TaskbarCounts::default()
        };
        let draw = |show_desktop| {
            let l = TaskbarLayout::compute(
                &theme.metrics,
                &TaskbarShape::CLASSIC,
                1920,
                true,
                c(show_desktop),
                text.measure("00:00", false, theme.fonts.size_px),
            );
            let state = TaskbarState {
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
                pager: vec![],
                pager_hot: None,
                pager_mark: Default::default(),
                task_hot: None,
                task_pressed: None,
                task_hot_x: None,
                task_shift: Vec::new(),
                task_float: None,
                show_desktop_hot: false,
                more: Default::default(),
                chevron: Default::default(),
                tasks: vec![TaskEntry {
                    title: "One".into(),
                    active: false,
                    attention: false,
                    icon: None,
                    glow: None,
                }],
                tray_icons: vec![],
                clock: "00:00".into(),
            };
            let mut canvas = Canvas::new(1920, 28);
            draw_taskbar(&mut canvas, &theme, &text, &l, &state);
            (canvas, l)
        };
        let (c, l) = draw(true);
        let r = l.show_desktop.unwrap();
        assert_eq!(r, Rect::new(1896, 4, 22, 22));
        assert_eq!(c.get(r.x, r.y + 10), cs.button_hilight.argb());
        assert_eq!(c.get(r.x + 10, r.y), cs.button_hilight.argb());
        assert_eq!(c.get(r.right() - 1, r.y + 10), cs.button_shadow.argb());
        assert_eq!(c.get(r.x + 10, r.bottom() - 1), cs.button_shadow.argb());
        assert_eq!(c.get(r.x + 10, r.y + 10), cs.button_face.argb());
        assert_eq!(c.get(1918, 10), cs.button_face.argb());
        assert_eq!(l.notification.right(), r.x - 2);
        let (off, _) = draw(false);
        assert_eq!(
            off.get(r.x, r.y + 10),
            cs.button_face.argb(),
            "no strip, no frame"
        );
    }

    fn classic_row(
        titles: &[&str],
        edit: impl FnOnce(&mut TaskbarState, &mut TaskbarLayout),
    ) -> (Canvas, TaskbarLayout) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let mut l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            1920,
            true,
            counts(0, titles.len(), 0),
            text.measure("00:00", false, theme.fonts.size_px),
        );
        let mut state = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: titles
                .iter()
                .map(|t| TaskEntry {
                    title: (*t).into(),
                    active: false,
                    attention: false,
                    icon: None,
                    glow: None,
                })
                .collect(),
            tray_icons: vec![],
            clock: "00:00".into(),
        };
        edit(&mut state, &mut l);
        let mut c = Canvas::new(1920, 28);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        (c, l)
    }

    fn pixels(c: &Canvas, r: Rect) -> Vec<u32> {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| c.get(x, y)))
            .collect()
    }

    #[test]
    fn a_sliding_button_draws_off_its_slot() {
        let (c, _) = classic_row(&["One", "Two"], |st, _| st.task_shift = vec![0, 25]);
        let (want, _) = classic_row(&["One", "Two"], |_, l| l.tasks[1].x += 25);
        assert!(c.pixels == want.pixels, "Two draws 25 px right");
    }

    #[test]
    fn a_floated_button_draws_over_its_neighbour_and_leaves_its_slot_bare() {
        let face = Theme::default().colors.button_face.argb();
        let (rest, l) = classic_row(&["One", "Two"], |_, _| {});
        let (t0, t1) = (l.tasks[0], l.tasks[1]);
        let (c, _) = classic_row(&["One", "Two"], |st, _| st.task_float = Some((0, t1.x)));
        assert_eq!(pixels(&c, t1), pixels(&rest, t0), "One, whole, over Two");
        assert!(
            pixels(&c, t0).iter().all(|&p| p == face),
            "One's slot is bare face"
        );
    }

    fn classic_bar_on(edge: Edge, chevron: bool) -> (Canvas, TaskbarLayout) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let counts = TaskbarCounts {
            quick: 1,
            pager: 2,
            tasks: 2,
            tray: 1,
            show_desktop: true,
            chevron,
            ..TaskbarCounts::default()
        };
        let l = TaskbarLayout::compute(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            800,
            true,
            counts,
            text.measure("00:00", false, theme.fonts.size_px),
        )
        .on_edge(edge);
        let entry = |title: &str, active| TaskEntry {
            title: title.into(),
            active,
            attention: false,
            icon: None,
            glow: None,
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![None],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: None,
            pager: vec![
                PagerButton {
                    current: true,
                    attention: false,
                },
                PagerButton::default(),
            ],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: vec![entry("One", true), entry("Two", false)],
            tray_icons: vec![None],
            clock: "00:00".into(),
        };
        let mut c = Canvas::new(800, 28);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        (c, l)
    }

    #[test]
    fn a_top_classic_bar_has_its_hilight_line_at_its_foot_and_its_buttons_upright() {
        let cs = Theme::default().colors;
        let (top, tl) = classic_bar_on(Edge::Top, false);
        let (bottom, bl) = classic_bar_on(Edge::Bottom, false);
        assert_eq!(tl.start, Rect::new(2, 2, 54, 22), "Start 2 px from the top");
        assert_eq!(
            top.get(500, 26),
            cs.button_hilight.argb(),
            "the hilight line on row 26"
        );
        assert_eq!(
            top.get(500, 27),
            cs.button_face.argb(),
            "face on the last row"
        );
        assert_eq!(
            top.get(500, 1),
            cs.button_face.argb(),
            "no line along the top"
        );
        for y in 0..28 {
            assert_eq!(
                top.get(500, y),
                bottom.get(500, 27 - y),
                "beside the buttons row {y} is the Bottom bar's row {}",
                27 - y
            );
        }
        let rects = |l: &TaskbarLayout| {
            let mut v = vec![l.start, l.notification];
            v.extend(&l.quick);
            v.extend(&l.pager);
            v.extend(&l.tasks);
            v.extend(l.show_desktop);
            v
        };
        for (t, b) in rects(&tl).into_iter().zip(rects(&bl)) {
            assert_eq!(
                pixels(&top, t),
                pixels(&bottom, b),
                "{t:?} holds the Bottom bar's {b:?}, upright"
            );
        }
    }

    #[test]
    fn a_top_classic_bar_draws_its_tray_arrow_turned_over() {
        let (top, tl) = classic_bar_on(Edge::Top, true);
        let (bottom, bl) = classic_bar_on(Edge::Bottom, true);
        let (t, b) = (tl.chevron.unwrap(), bl.chevron.unwrap());
        assert_eq!(
            (t.x, t.y, b.y),
            (b.x, 4, 6),
            "the cell 4 px from the top, 6 on a Bottom bar"
        );
        for y in 0..18 {
            assert_eq!(
                pixels(&top, Rect::new(t.x, t.y + y, 16, 1)),
                pixels(&bottom, Rect::new(b.x, b.y + 17 - y, 16, 1)),
                "the cell's row {y} is the Bottom cell's row {}",
                17 - y
            );
        }
    }

    fn arrow_rows(c: &Canvas, r: Rect) -> Vec<String> {
        (r.y..r.bottom())
            .map(|y| {
                (r.x..r.right())
                    .map(|x| match c.get(x, y) {
                        0xffffffff => 'W',
                        0xff808080 => 'g',
                        0xffd4d0c8 => '.',
                        0xff000000 => '#',
                        _ => '?',
                    })
                    .collect()
            })
            .collect()
    }

    fn arrow_cell(
        w: usize,
        h: usize,
        frame: bool,
        glyph: &[&str],
        x: usize,
        y: usize,
    ) -> Vec<String> {
        (0..h)
            .map(|j| {
                let mut line: Vec<char> = (0..w)
                    .map(|i| match (frame, i, j) {
                        (false, ..) => '.',
                        (true, i, j) if i == w - 1 || j == h - 1 => 'g',
                        (true, i, j) if i == 0 || j == 0 => 'W',
                        _ => '.',
                    })
                    .collect();
                if let Some(row) = j.checked_sub(y).and_then(|k| glyph.get(k)) {
                    for (i, ch) in row.chars().enumerate() {
                        if ch == '#' {
                            line[x + i] = '#';
                        }
                    }
                }
                line.into_iter().collect()
            })
            .collect()
    }

    #[test]
    fn a_classic_tray_arrow_draws_each_state() {
        use crate::taskbar::{PopupCell, TaskbarHit};
        let shut = ["..##..##", ".##..##.", "##..##..", ".##..##.", "..##..##"];
        let open = ["##..##..", ".##..##.", "..##..##", ".##..##.", "##..##.."];
        for (hot, expanded, focus, held) in [
            (false, false, false, false),
            (true, false, false, false),
            (false, false, true, false),
            (false, true, false, false),
            (true, true, false, false),
            (true, false, false, true),
            (false, false, false, true),
            (true, true, false, true),
        ] {
            let cell = PopupCell {
                hot,
                open: expanded,
                held,
            };
            let (c, _, l) = classic_popup_cells(Edge::Bottom, 1, true, |st| {
                st.chevron = cell;
                st.focus = focus.then_some(TaskbarHit::Chevron);
            });
            let r = l.chevron.unwrap();
            assert_eq!((r.width, r.height), (16, 18));
            let glyph = if expanded { &open } else { &shut };
            assert_eq!(
                arrow_rows(&c, r),
                arrow_cell(16, 18, hot || focus || held, glyph, 4, 7),
                "hot {hot}, open {expanded}, focus {focus}, held {held}: framed only when lit or \
                 held, the glyph at (4, 7)"
            );
        }
    }

    #[test]
    fn a_left_classic_bar_turns_its_tray_arrow() {
        use crate::taskbar::PopupCell;
        let up = ["..#..", ".###.", "##.##", "#...#"];
        let down = ["#...#", "##.##", ".###.", "..#.."];
        for (hot, expanded) in [(false, false), (true, false), (false, true)] {
            let cell = PopupCell {
                hot,
                open: expanded,
                held: false,
            };
            let (c, _, l) = classic_popup_cells(Edge::Left, 1, true, |st| st.chevron = cell);
            let r = l.chevron.unwrap();
            assert_eq!(
                (r.x, r.width, r.height),
                (4, 50, 16),
                "across the well's top"
            );
            let (rows, y) = if expanded { (down, 6) } else { (up, 2) };
            let glyph: Vec<&str> = rows.iter().chain(&rows).copied().collect();
            assert_eq!(
                arrow_rows(&c, r),
                arrow_cell(50, 16, true, &glyph, 22, y),
                "hot {hot}, open {expanded}: framed in every state, the glyph centred, {y} px down"
            );
        }
    }

    #[test]
    fn a_hidden_classic_bar_leaves_its_face_and_hilight_rows_on_screen() {
        let cs = Theme::default().colors;
        let (face, hilight) = (cs.button_face.argb(), cs.button_hilight.argb());
        for (edge, rows) in [
            (Edge::Bottom, [face, hilight]),
            (Edge::Top, [hilight, face]),
        ] {
            let (c, l) = classic_bar_on(edge, false);
            let line = c.copy_rect(l.reveal_line());
            assert_eq!((line.width, line.height), (800, 2), "{edge:?}");
            let end = l.show_desktop.map_or(l.width, |r| r.x);
            for x in 0..end {
                for (y, want) in rows.iter().enumerate() {
                    assert_eq!(
                        line.get(x, y as i32),
                        *want,
                        "{edge:?}: column {x}, line row {y}"
                    );
                }
            }
        }
    }

    fn classic_side(
        edge: Edge,
        t: i32,
        drop: Option<usize>,
        clock: &str,
    ) -> (Canvas, TaskbarLayout) {
        classic_side_with(edge, t, drop, clock, 2)
    }

    fn classic_side_with(
        edge: Edge,
        t: i32,
        drop: Option<usize>,
        clock: &str,
        tasks: usize,
    ) -> (Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let (time, date) = clock.split_once('\n').unwrap_or((clock, ""));
        let counts = TaskbarCounts {
            quick: 3,
            pager: 2,
            tasks,
            tray: 1,
            show_desktop: true,
            chevron: false,
            ..TaskbarCounts::default()
        };
        let span = BarSpan {
            edge,
            length: 600,
            thickness: t,
        };
        let clock_text = ClockText {
            time: text.measure(time, false, px),
            date: text.measure(date, false, px),
        };
        let l = TaskbarLayout::compute_on(
            &theme.metrics,
            &TaskbarShape::CLASSIC,
            span,
            true,
            counts,
            clock_text,
            Bands::default(),
        );
        let entry = |title: &str, active| TaskEntry {
            title: title.into(),
            active,
            attention: false,
            icon: None,
            glow: None,
        };
        let state = TaskbarState {
            start_pressed: false,
            start_focus: false,
            focus: None,
            start_hot: false,
            start_icon: None,
            orb_glow: 100,
            quick_icons: vec![None; 3],
            quick_hot: None,
            quick_pressed: None,
            quick_drop: drop,
            pager: vec![
                PagerButton {
                    current: true,
                    attention: false,
                },
                PagerButton::default(),
            ],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: (0..tasks)
                .map(|i| entry(["One", "Two"].get(i).unwrap_or(&"Three"), i == 0))
                .collect(),
            tray_icons: vec![None],
            clock: clock.into(),
        };
        let mut c = Canvas::new(l.width as u32, l.height as u32);
        draw_taskbar(&mut c, &theme, &text, &l, &state);
        (c, l)
    }

    #[test]
    fn a_side_classic_bar_runs_its_hilight_down_its_inner_edge_and_keeps_its_buttons_upright() {
        let cs = Theme::default().colors;
        let (face, hilight) = (cs.button_face.argb(), cs.button_hilight.argb());
        let (bottom, bl) = classic_bar_on(Edge::Bottom, false);
        for (edge, inner) in [(Edge::Left, [59, 58]), (Edge::Right, [0, 1])] {
            let (c, l) = classic_side(edge, 60, None, "00:00");
            let y = l.start.bottom() + 1;
            assert_eq!(
                [c.get(inner[0], y), c.get(inner[1], y)],
                [face, hilight],
                "{edge:?}: a face column, then the hilight, on the inner edge"
            );
            let outer = if edge == Edge::Left { 0 } else { 59 };
            assert_eq!(c.get(outer, y), face, "{edge:?}: no line on the outer edge");
            assert_eq!(
                pixels(&c, l.start),
                pixels(&bottom, bl.start),
                "{edge:?}: Start upright, as on a bottom bar"
            );
            let t = l.tasks[1];
            assert_eq!(
                c.get(t.x, t.y + 10),
                hilight,
                "{edge:?}: a raised button's left column"
            );
        }
    }

    #[test]
    fn a_side_classic_bar_under_60_px_draws_no_title_on_its_buttons() {
        let ink = Theme::default().colors.button_text.argb();
        let (c, l) = classic_side(Edge::Left, 50, None, "00:00");
        assert!(l.icons_only, "a 44 px button row: icons only");
        for r in &l.tasks {
            let inked = (r.x + 22..r.right() - 2)
                .any(|x| (r.y + 2..r.bottom() - 2).any(|y| c.get(x, y) == ink));
            assert!(!inked, "{r:?}: no title after the icon");
        }
    }

    #[test]
    fn on_a_side_bar_the_quick_launch_drop_mark_lies_across_the_cell() {
        let ink = Theme::default().colors.button_dk_shadow.argb();
        let (c, l) = classic_side(Edge::Left, 60, Some(1), "00:00");
        let r = l.quick[1];
        for x in r.x..r.right() {
            assert_eq!(
                [c.get(x, r.y), c.get(x, r.y + 1)],
                [ink, ink],
                "a 2 px bar across the cell's top at x {x}"
            );
        }
        let rows = |x: i32| {
            (r.y - 3..r.y + 8)
                .filter(|&y| c.get(x, y) == ink)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(r.x),
            (r.y - 2..r.y + 4).collect::<Vec<_>>(),
            "a 6 px serif at the left end"
        );
        assert_eq!(
            rows(r.x + 1),
            (r.y - 1..r.y + 3).collect::<Vec<_>>(),
            "then a 4 px one"
        );
        assert_eq!(
            rows(r.right() - 1),
            (r.y - 2..r.y + 4).collect::<Vec<_>>(),
            "and 6 px at the right end"
        );
    }

    #[test]
    fn on_a_side_bar_a_drop_past_the_last_cell_marks_under_it() {
        let ink = Theme::default().colors.button_dk_shadow.argb();
        let (c, l) = classic_side(Edge::Left, 60, Some(3), "00:00");
        let r = *l.quick.last().unwrap();
        let rows = |x: i32| {
            (r.bottom() - 3..r.bottom() + 4)
                .filter(|&y| c.get(x, y) == ink)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rows(r.x + 10),
            vec![r.bottom(), r.bottom() + 1],
            "a 2 px bar along the last cell's foot"
        );
        assert_eq!(
            rows(r.x),
            (r.bottom() - 2..r.bottom() + 4).collect::<Vec<_>>(),
            "a 6 px serif at the left end"
        );
        assert_eq!(
            rows(r.x + 1),
            (r.bottom() - 1..r.bottom() + 3).collect::<Vec<_>>(),
            "then a 4 px one"
        );
        assert_eq!(
            rows(r.right() - 1),
            (r.bottom() - 2..r.bottom() + 4).collect::<Vec<_>>(),
            "and 6 px at the right end"
        );
    }

    #[test]
    fn a_side_classic_bars_more_cell_is_a_raised_button_with_the_double_chevron() {
        let cs = Theme::default().colors;
        let (c, l) = classic_side_with(Edge::Left, 60, None, "00:00", 20);
        let r = l
            .more
            .expect("twenty 22 px buttons overrun a 600 px column");
        assert_eq!(r, Rect::new(2, 504, 54, 22));
        let mut want = Canvas::new(r.width as u32, r.height as u32);
        let at = Rect::new(0, 0, r.width, r.height);
        want.fill_rect(at, cs.button_face);
        want.edge_t(at, EdgeStyle::ButtonRaised, &cs, 1);
        crate::taskbar::draw_more(&mut want, at, cs.button_text, 1);
        assert_eq!(pixels(&c, r), pixels(&want, at));
        assert_eq!(
            c.get(r.x + 22, r.y + 10),
            cs.button_text.argb(),
            "the first >'s back, centred"
        );
    }

    fn classic_popup_cells(
        edge: Edge,
        tasks: usize,
        arrow: bool,
        cells: impl Fn(&mut TaskbarState),
    ) -> (Canvas, Canvas, TaskbarLayout) {
        use crate::taskbar::{Bands, BarSpan, ClockText};
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let span = BarSpan {
            edge,
            length: if edge.is_side() { 600 } else { 800 },
            thickness: if edge.is_side() { 60 } else { 28 },
        };
        let counts = TaskbarCounts {
            chevron: true,
            ..counts(0, tasks, 1)
        };
        let mut shape = TaskbarShape::CLASSIC;
        if !arrow {
            shape.tray_arrow = None;
        }
        let l = TaskbarLayout::compute_on(
            &theme.metrics,
            &shape,
            span,
            true,
            counts,
            ClockText::default(),
            Bands::default(),
        );
        let mut st = TaskbarState {
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
            pager: vec![],
            pager_hot: None,
            pager_mark: Default::default(),
            task_hot: None,
            task_pressed: None,
            task_hot_x: None,
            task_shift: Vec::new(),
            task_float: None,
            show_desktop_hot: false,
            more: Default::default(),
            chevron: Default::default(),
            tasks: (0..tasks)
                .map(|_| TaskEntry {
                    title: String::new(),
                    active: false,
                    attention: false,
                    icon: None,
                    glow: None,
                })
                .collect(),
            tray_icons: vec![None],
            clock: String::new(),
        };
        let draw = |l: &TaskbarLayout, st: &TaskbarState| {
            let mut c = Canvas::new(l.width as u32, l.height as u32);
            draw_taskbar(&mut c, &theme, &text, l, st);
            c
        };
        let mut bare = l.clone();
        bare.more = None;
        bare.chevron = None;
        let under = draw(&bare, &st);
        cells(&mut st);
        (draw(&l, &st), under, l)
    }

    #[test]
    fn a_classic_more_cell_stays_raised_under_the_pointer_and_pushes_in_while_its_menu_is_open() {
        use crate::taskbar::PopupCell;
        let cs = Theme::default().colors;
        for edge in [Edge::Left, Edge::Right] {
            for cell in [
                PopupCell {
                    hot: true,
                    open: false,
                    held: false,
                },
                PopupCell {
                    hot: false,
                    open: true,
                    held: false,
                },
                PopupCell {
                    hot: true,
                    open: true,
                    held: false,
                },
            ] {
                let (c, mut want, l) = classic_popup_cells(edge, 30, true, |st| st.more = cell);
                let r = l.more.expect("thirty buttons overrun a 600 px column");
                let (frame, o) = if cell.open {
                    (EdgeStyle::ButtonPushed, 1)
                } else {
                    (EdgeStyle::ButtonRaised, 0)
                };
                want.fill_rect(r, cs.button_face);
                want.edge_t(r, frame, &cs, 1);
                let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
                crate::taskbar::draw_more(&mut want, at, cs.button_text, 1);
                assert_eq!(pixels(&c, r), pixels(&want, r), "{edge:?} {cell:?}");
            }
        }
    }

    #[test]
    fn a_classic_chevron_cell_without_an_arrow_rises_under_the_pointer_and_sinks_while_its_flyout_is_open()
     {
        use crate::taskbar::PopupCell;
        let cs = Theme::default().colors;
        for edge in [Edge::Bottom, Edge::Top, Edge::Left, Edge::Right] {
            for cell in [
                PopupCell {
                    hot: true,
                    open: false,
                    held: false,
                },
                PopupCell {
                    hot: false,
                    open: true,
                    held: false,
                },
                PopupCell {
                    hot: true,
                    open: true,
                    held: false,
                },
            ] {
                let (c, mut want, l) = classic_popup_cells(edge, 1, false, |st| st.chevron = cell);
                let r = l.chevron.unwrap();
                let (frame, o) = if cell.open {
                    (EdgeStyle::ThinSunken, 1)
                } else {
                    (EdgeStyle::ThinRaised, 0)
                };
                want.edge_t(r, frame, &cs, 1);
                let at = Rect::new(r.x + o, r.y + o, r.width, r.height);
                edge.draw_art(&mut want, at, |w, at| {
                    crate::taskbar::draw_chevron(w, at, cs.button_text, 1)
                });
                assert_eq!(pixels(&c, r), pixels(&want, r), "{edge:?} {cell:?}");
            }
        }
    }

    #[test]
    fn a_side_classic_buttons_title_reads_upright_as_on_a_bottom_bar() {
        let (side, l) = classic_side(Edge::Right, 60, None, "00:00");
        assert!(!l.icons_only, "labelled at 60 px");
        let r = l.tasks[1];
        let at = Rect::new(300, 4, r.width, r.height);
        let (bottom, _) = classic_row(&["Two"], |_, l| l.tasks[0] = at);
        assert_eq!(
            pixels(&side, r),
            pixels(&bottom, at),
            "Two's button and title as a bottom bar draws them"
        );
    }

    #[test]
    fn a_wide_side_classic_bar_sets_the_date_under_the_time() {
        let face = Theme::default().colors.button_face.argb();
        let (c, l) = classic_side(Edge::Right, 140, None, "3:45 PM\n10/04/2026");
        assert!(l.clock_lines.is_some(), "the date fits across 134 px");
        let k = l.clock.inset(2);
        let inked: Vec<i32> = (k.y..k.bottom())
            .filter(|&y| (k.x..k.right()).any(|x| c.get(x, y) != face))
            .collect();
        let mid = l.clock.y + l.clock.height / 2;
        assert!(
            inked.iter().any(|&y| y < mid) && inked.iter().any(|&y| y >= mid),
            "ink over and under the clock's middle: {inked:?}"
        );
        let (_, narrow) = classic_side(Edge::Right, 60, None, "3:45 PM");
        assert_eq!(narrow.clock_lines, None, "one line at the default width");
    }

    #[test]
    fn a_wide_side_classic_bar_sets_each_clock_line_on_its_own_baseline() {
        let face = Theme::default().colors.button_face.argb();
        let (c, l) = classic_side(Edge::Right, 140, None, "8\n8");
        assert_eq!((l.clock.height, l.clock_lines), (40, Some([17, 34])));
        let k = l.clock.inset(2);
        let inked: Vec<i32> = (k.y..k.bottom())
            .filter(|&y| (k.x..k.right()).any(|x| c.get(x, y) != face))
            .collect();
        let mut runs: Vec<(i32, i32)> = Vec::new();
        for &y in &inked {
            match runs.last_mut() {
                Some((_, end)) if *end + 1 == y => *end = y,
                _ => runs.push((y, y)),
            }
        }
        assert_eq!(runs.len(), 2, "two lines of ink: {inked:?}");
        let ((first, first_end), (second, _)) = (runs[0], runs[1]);
        assert_eq!(
            second - first,
            17,
            "the date's 8 sits 17 rows under the time's"
        );
        assert_eq!(
            first_end,
            l.clock.y + 17 - 1,
            "the time's 8 stands on baseline 17"
        );
    }

    #[test]
    fn a_hidden_side_classic_bar_leaves_its_hilight_and_face_columns_on_screen() {
        let cs = Theme::default().colors;
        let (face, hilight) = (cs.button_face.argb(), cs.button_hilight.argb());
        for (edge, cols) in [
            (Edge::Left, [hilight, face]),
            (Edge::Right, [face, hilight]),
        ] {
            let (c, l) = classic_side(edge, 60, None, "00:00");
            let line = c.copy_rect(l.reveal_line());
            assert_eq!((line.width, line.height), (2, 600), "{edge:?}");
            for y in [l.start.bottom() + 1, l.tasks[1].bottom() + 1] {
                for (x, want) in cols.iter().enumerate() {
                    assert_eq!(
                        line.get(x as i32, y),
                        *want,
                        "{edge:?}: line column {x}, row {y}"
                    );
                }
            }
        }
    }
}
