//! Window frames and the taskbar in the column's variant.

use reprisekit::classic::frame::{Chrome, FrameGeometry, FrameState, Glow};
use reprisekit::classic::indicator;
use reprisekit::look;
use reprisekit::raster::{Canvas, Rect};
use reprisekit::taskbar::{TaskEntry, TaskbarCounts, TaskbarLayout, TaskbarState};
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;

pub const HEIGHT: i32 = 150;

pub fn draw(c: &mut Canvas, theme: &Theme, text: &TextRenderer, r: Rect) {
    let style = theme.look.style().expect("a Blue theme is styled");
    let caption_text = TextRenderer::load(&style.caption_text.font());
    c.fill_rect(r, theme.colors.background);
    for (i, active) in [true, false].into_iter().enumerate() {
        let (x, y) = (r.x + 10 + i as i32 * 310, r.y + 8);
        let g = FrameGeometry::layout(&theme.metrics, 290, 60, false, Chrome::WINDOW)
            .translated(-x, -y);
        look::draw_frame(
            c,
            theme,
            text,
            &caption_text,
            &g,
            &FrameState {
                title: "Notes",
                active,
                maximized: false,
                pressed: None,
                glow: Glow::lit(None),
                icon: None,
                tabbed: false,
            },
        );
        c.fill_rect(g.client, theme.colors.window);
    }
    let start_text = TextRenderer::load(&style.start_text.font());
    let clock = "4:02 PM";
    let layout = TaskbarLayout::compute(
        &theme.metrics,
        &style.taskbar,
        r.width,
        true,
        TaskbarCounts {
            quick: 0,
            pager: 0,
            tasks: 3,
            tray: 1,
            chevron: false,
            show_desktop: false,
            ..TaskbarCounts::default()
        },
        text.measure(clock, false, theme.fonts.size_px),
    );
    let entry = |title: &str, active: bool| TaskEntry {
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
        quick_icons: vec![],
        quick_hot: None,
        quick_pressed: None,
        quick_drop: None,
        quick_more: Default::default(),
        pager: vec![],
        pager_hot: None,
        pager_mark: Default::default(),
        task_hot: Some(2),
        task_pressed: None,
        task_hot_x: None,
        task_shift: Vec::new(),
        task_float: None,
        task_progress: Vec::new(),
        show_desktop_hot: false,
        more: Default::default(),
        chevron: Default::default(),
        tasks: vec![
            entry("Notes", false),
            entry("p3", true),
            entry("Calculator", false),
        ],
        tray_icons: vec![Some(indicator::draw_record(16).into())],
        clock: clock.into(),
    };
    let mut bar = Canvas::new(r.width as u32, layout.height as u32);
    look::draw_taskbar(&mut bar, theme, text, &start_text, &layout, &state);
    c.blit_canvas(r.x, r.y + r.height - layout.height - 4, &bar);
}
