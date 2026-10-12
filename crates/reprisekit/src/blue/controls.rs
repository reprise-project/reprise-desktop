//! One file per control, each building its own atlas of parts from Blue's
//! palette data next to the pixels that draw it. `all()` gathers every
//! part built so far in one colour variant, in file order, for
//! [`crate::blue::build`] to hand to [`crate::style::Style::with_part`].

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

pub(super) fn all(v: Variant, scale: i32) -> Vec<(Part, PartImage)> {
    let mut parts = Vec::new();
    parts.extend(button::parts(v, scale));
    parts.extend(checkbox::parts(v, scale));
    parts.extend(radio::parts(v, scale));
    parts.extend(groupbox::parts(v, scale));
    parts.extend(scrollbar::parts(v, scale));
    parts.extend(field::parts(v));
    parts.extend(combodrop::parts(v, scale));
    parts.extend(tab::parts(v, scale));
    parts.extend(progress::parts(v, scale));
    parts.extend(trackbar::parts(v, scale));
    parts.extend(spin::parts(v, scale));
    parts.extend(help::parts(v, scale));
    parts.extend(sizebox::parts(v, scale));
    parts.extend(calendar::parts(v));
    parts.extend(toolbar::parts(v, scale));
    parts.extend(rebar::parts(v, scale));
    parts.extend(header::parts(v, scale));
    parts.extend(statusbar::parts(v, scale));
    parts.extend(tooltip::parts(v));
    parts.extend(taskpane::parts(v, scale));
    parts.extend(explorerbar::parts(v, scale));
    parts.extend(placesbar::parts(v, scale));
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
