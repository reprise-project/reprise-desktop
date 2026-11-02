use super::*;
use crate::access::accesskit::{HasPopup, Orientation, Role, Toggled};
use crate::access::{self, Tree};

mod access_tree;
mod buttons;
mod column;
mod edges;
mod row;
mod tray;

fn classic_full(edge: Edge) -> TaskbarLayout {
    let c = TaskbarCounts {
        quick: 2,
        pager: 2,
        tasks: 2,
        tray: 2,
        show_desktop: true,
        chevron: true,
        ..TaskbarCounts::default()
    };
    TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        800,
        false,
        c,
        30,
    )
    .on_edge(edge)
}

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

fn classic_row(n: usize) -> TaskbarLayout {
    TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        800,
        true,
        counts(0, n, 0),
        30,
    )
}

fn classic_column(edge: Edge, t: i32, c: TaskbarCounts, locked: bool) -> TaskbarLayout {
    let span = BarSpan {
        edge,
        length: 600,
        thickness: t,
    };
    let clock = ClockText { time: 30, date: 0 };
    TaskbarLayout::compute_on(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        span,
        locked,
        c,
        clock,
        Bands::default(),
    )
}

fn look_column(
    interface: crate::interface::Interface,
    edge: Edge,
    (t, length): (i32, i32),
    c: TaskbarCounts,
    clock: ClockText,
    bands: Bands,
) -> TaskbarLayout {
    let theme = crate::interfaces::theme_for(interface);
    let span = BarSpan {
        edge,
        length,
        thickness: t,
    };
    TaskbarLayout::compute_on(
        &theme.metrics,
        &theme.taskbar_shape(),
        span,
        true,
        c,
        clock,
        bands,
    )
}

fn arrow_counts(tray: usize, hidden: usize, reveal: i32) -> TaskbarCounts {
    TaskbarCounts {
        tray,
        hidden,
        reveal,
        chevron: true,
        ..TaskbarCounts::default()
    }
}

#[test]
fn a_badge_shows_the_count_up_to_99_then_99_plus() {
    use crate::taskbar::badge_text;
    assert_eq!(badge_text(1), "1");
    assert_eq!(badge_text(99), "99");
    assert_eq!(badge_text(100), "99+");
    assert_eq!(badge_text(u32::MAX), "99+");
}

#[test]
fn a_badge_sits_in_the_buttons_top_right_corner_and_the_label_stops_before_it() {
    use crate::taskbar::{badge_at, label_before_badge};
    let button = Rect::new(100, 10, 160, 24);
    assert_eq!(badge_at(button, 11, 1), (246, 13));
    assert_eq!(badge_at(button.scaled(2), 22, 2), (492, 26));
    let label = Rect::new(124, 10, 132, 24);
    assert_eq!(
        label_before_badge(label, 246, 1),
        Rect::new(124, 10, 120, 24)
    );
    assert_eq!(
        label_before_badge(label, 100, 1).width,
        0,
        "a badge over the whole label leaves none"
    );
}
