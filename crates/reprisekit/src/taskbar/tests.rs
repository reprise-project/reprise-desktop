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
