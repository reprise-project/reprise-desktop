use std::sync::Arc;

use super::buttons::draw_image_nth;
use super::start::{ORB_MARK_SHARE, start_icon_box};
use super::*;
use crate::interface::Interface;
use crate::interfaces::theme_for;
use crate::raster::Rect;
use crate::style::startbloom::IconKey;
use crate::style::{Part, PartImage};
use crate::taskbar::{Edge, TaskEntry, TaskbarCounts};
use crate::theme::{Color, FontConfig};

mod bar;
mod buttons;
mod edges;
mod glass_buttons;
mod orb;
mod pager;
mod side;

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
                attention: crate::taskbar::Attention::None,
                icon: None,
                glow: None,
            })
            .collect(),
        tray_icons: vec![],
        clock: String::new(),
    }
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

fn solid_icon(c: crate::theme::Color, px: i32) -> Canvas {
    let mut icon = Canvas::new(px as u32, px as u32);
    icon.fill_rect(Rect::new(0, 0, px, px), c);
    icon
}

fn over_grey(px: u32) -> [i32; 3] {
    let a = (px >> 24) as i32;
    [16, 8, 0].map(|sh| ((px >> sh) & 0xFF) as i32 + (255 - a) * 128 / 255)
}

fn over_veil(s: &Style, l: &TaskbarLayout) -> Canvas {
    over_veil_at(s, l, 1)
}

fn label_frame(r: Rect, sc: i32) -> Rect {
    Rect::new(r.x + 3 * sc, r.y, r.width - 6 * sc, r.height)
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

fn lum(px: u32) -> i32 {
    let [r, g, b] = over_grey(px);
    (r * 299 + g * 587 + b * 114) / 1000
}

fn glass_pager(
    mark: crate::taskbar::PagerMark,
    hot: Option<usize>,
    small: bool,
) -> (Canvas, Vec<Rect>, Canvas) {
    let mut theme = theme_for(Interface::Glass);
    if small {
        theme.metrics.taskbar_height = crate::glass::SMALL_TASKBAR_HEIGHT;
        theme.look = crate::paint::Look::Styled(crate::glass::small_bar_style(&Default::default()));
    }
    pager_bar(&theme, mark, hot)
}

fn turned(c: &Canvas, r: Rect, edge: Edge, f: impl FnOnce(&mut Canvas, Rect)) -> Canvas {
    let mut c = c.clone();
    c.with_transposed(r, edge == Edge::Left, |p| {
        f(p, Rect::new(0, 0, r.height, r.width))
    });
    c
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

#[test]
fn a_lit_attention_button_takes_its_attention_frame_and_a_dark_half_its_rest() {
    use crate::taskbar::Attention;
    for interface in [Interface::Blue, Interface::Glass] {
        let theme = theme_for(interface);
        let s = theme.look.style().unwrap();
        let text = TextRenderer::load(&FontConfig::default());
        let start = TextRenderer::load(&s.start_text.font());
        let draw = |attention, active| {
            let mut st = state_with_tasks(&["One", "Two"]);
            st.tasks[0].attention = attention;
            st.tasks[0].active = active;
            let counts = TaskbarCounts {
                tasks: 2,
                ..TaskbarCounts::default()
            };
            let layout = TaskbarLayout::compute(&theme.metrics, &s.taskbar, 1280, true, counts, 60);
            let mut c = Canvas::new(1280, theme.metrics.taskbar_height as u32);
            draw_taskbar(&mut c, s, &theme, &text, &start, &layout, &st);
            c.pixels
        };
        let rest = draw(Attention::None, false);
        let lit = draw(Attention::FlashOn, false);
        assert_eq!(
            draw(Attention::FlashOff, false),
            rest,
            "{interface:?} dark half"
        );
        assert_eq!(
            draw(Attention::Steady, false),
            lit,
            "{interface:?} steady is lit"
        );
        assert_ne!(lit, rest, "{interface:?} lit differs from rest");
        assert_ne!(
            lit,
            draw(Attention::None, true),
            "{interface:?} lit differs from active"
        );
        assert_eq!(
            draw(Attention::Steady, true),
            draw(Attention::None, true),
            "{interface:?} the focused window's button draws active"
        );
    }
}
