//! One file per control, each building its own atlas of parts from Blue's
//! palette data next to the pixels that draw it.

mod button;
pub(super) mod calendar;
mod checkbox;
mod combodrop;
mod explorerbar;
mod field;
mod groupbox;
mod header;
mod help;
mod placesbar;
mod progress;
mod radio;
mod rebar;
mod scrollbar;
mod sizebox;
mod spin;
mod statusbar;
mod tab;
mod taskpane;
pub(super) mod toolbar;
mod tooltip;
mod trackbar;

use crate::blue::Variant;
use crate::metrics::{
    AddressMetrics, BarMetrics, MenuBarMetrics, MenuTitles, PlacesMetrics, RebarMetrics,
    TaskMetrics, ToolbarMetrics,
};
use crate::style::{Part, PartImage, State};
use crate::theme::Color;

/// A control module's parts and the painter that draws them together at a scale.
pub(super) type Group = (&'static [Part], fn(Variant, i32) -> Vec<(Part, PartImage)>);

/// Each control module whose parts are painted at their first use, in file order.
pub(super) const GROUPS: [Group; 19] = [
    (&[Part::PushButton], button::parts),
    (&[Part::CheckBox], checkbox::parts),
    (&[Part::Radio], radio::parts),
    (&[Part::GroupBox], groupbox::parts),
    (
        &[
            Part::ScrollArrowUp,
            Part::ScrollArrowDown,
            Part::ScrollArrowLeft,
            Part::ScrollArrowRight,
            Part::ScrollThumbV,
            Part::ScrollThumbH,
            Part::ScrollGripV,
            Part::ScrollGripH,
            Part::ScrollTrackV,
            Part::ScrollTrackH,
        ],
        scrollbar::parts,
    ),
    (&[Part::ComboDrop], combodrop::parts),
    (&[Part::TabItem, Part::TabPane], tab::parts),
    (&[Part::ProgressBar, Part::ProgressChunk], progress::parts),
    (
        &[
            Part::TrackbarTrack,
            Part::TrackbarThumb,
            Part::TrackbarTrackV,
            Part::TrackbarThumbV,
        ],
        trackbar::parts,
    ),
    (&[Part::SpinUp, Part::SpinDown], spin::parts),
    (&[Part::HelpButton], help::parts),
    (&[Part::SizeBox], sizebox::parts),
    (
        &[
            Part::ToolbarButton,
            Part::ToolbarDrop,
            Part::ToolbarSeparator,
        ],
        toolbar::parts,
    ),
    (&[Part::Rebar, Part::RebarBand], rebar::parts),
    (
        &[Part::HeaderItem, Part::HeaderFiller, Part::HeaderSortArrow],
        header::parts,
    ),
    (&[Part::StatusBar, Part::StatusPane], statusbar::parts),
    (
        &[
            Part::TaskPane,
            Part::TaskGroupHeader,
            Part::TaskGroupBody,
            Part::TaskCollapse,
            Part::TaskExpand,
            Part::TaskCollapseMark,
            Part::TaskExpandMark,
        ],
        taskpane::parts,
    ),
    (
        &[Part::ExplorerBarHeader, Part::ExplorerBarClose],
        explorerbar::parts,
    ),
    (&[Part::PlacesBar], placesbar::parts),
];

/// The ? box `side` design px square at `scale`.
pub(super) fn help_sized(v: Variant, scale: i32, side: i32) -> Vec<(Part, PartImage)> {
    help::parts_sized(v, scale, side)
}

/// The parts cheap enough to paint with the style; which ones depends on the variant.
pub(super) fn eager(v: Variant) -> Vec<(Part, PartImage)> {
    let mut parts = field::parts(v);
    parts.extend(calendar::parts(v));
    parts.extend(tooltip::parts(v));
    parts
}

pub(super) fn inks(v: Variant) -> Vec<(Part, State, Color)> {
    let mut inks = taskpane::inks(v);
    inks.extend(rebar::inks(v));
    inks
}

pub(super) fn task_metrics(v: Variant) -> TaskMetrics {
    taskpane::metrics(v)
}

pub(super) fn bar_metrics() -> BarMetrics {
    explorerbar::metrics()
}

pub(super) fn rebar_metrics(v: Variant) -> RebarMetrics {
    rebar::metrics(v)
}

pub(super) fn toolbar_metrics() -> ToolbarMetrics {
    toolbar::metrics()
}

pub(super) fn places_metrics() -> PlacesMetrics {
    placesbar::metrics()
}

pub(super) fn address_metrics() -> AddressMetrics {
    AddressMetrics {
        go_w: 55,
        go_frame_x: 2,
    }
}

/// Blue's menu bars. A window's own bar is 19 px of titles from its left
/// edge over a row of `window`: 19 px frames from the bar's top, the text
/// 6 px in. A bar in a rebar band (Explorer's) keeps Classic's first
/// frame 2 px in and a pixel down, its frames 19 px tall with the text
/// 8 px in and a pixel below their middle.
pub(super) fn menubar_metrics() -> MenuBarMetrics {
    MenuBarMetrics {
        plain: MenuTitles {
            bar_h: 20,
            first_x: 0,
            frame_y: 0,
            frame_h: 19,
            pad: 6,
            text_dy: 0,
        },
        banded: MenuTitles {
            bar_h: 20,
            first_x: 2,
            frame_y: 1,
            frame_h: 19,
            pad: 8,
            text_dy: 1,
        },
        line: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_group_paints_exactly_the_parts_it_names() {
        for v in Variant::ALL {
            for (names, paint) in GROUPS {
                let painted: Vec<Part> = paint(v, 1).into_iter().map(|(p, _)| p).collect();
                assert_eq!(painted.len(), names.len(), "{v:?} {names:?}");
                assert!(names.iter().all(|p| painted.contains(p)), "{v:?} {names:?}");
            }
        }
    }

    #[test]
    fn a_greyed_label_inks_each_variants_own_grey() {
        use crate::style::State;
        for v in Variant::ALL {
            let st = crate::blue::style_for(v);
            for part in [Part::PushButton, Part::CheckBox, Part::Radio] {
                assert_eq!(
                    st.text_color(part, State::Disabled),
                    Some(v.scheme().inks.greyed_label),
                    "{v:?} {part:?}"
                );
            }
        }
    }
}
