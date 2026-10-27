//! The tray cells the compositor draws itself: the StickyKeys, MouseKeys and keyboard layout indicators before the StatusNotifier items, and after them the displays cell, the notification bell and the privacy indicators nearest the clock. A new indicator is one more variant here.

use crate::compositor::State;
use crate::input::{BTN_LEFT, BTN_RIGHT};
use crate::media::Value;
use crate::notifylog::Suppress;
use crate::raster::{Canvas, Rect};
use reprisekit::classic::indicator;
use smithay::output::Output;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    StickyKeys,
    /// MouseKeys, while it is on: which button the keys click, and whether it is held.
    MouseKeys,
    Layout,
    /// Every display's brightness and night light: the sun, warm while night light is warming.
    Displays,
    Notifications,
    Sharing,
    Microphone,
    Camera,
    Location,
}

impl Builtin {
    /// Whether the cell sits before the StatusNotifier items rather than after them.
    pub fn leads(self) -> bool {
        matches!(
            self,
            Builtin::StickyKeys | Builtin::MouseKeys | Builtin::Layout
        )
    }
}

pub fn shown(state: &State) -> Vec<Builtin> {
    [
        (
            Builtin::StickyKeys,
            crate::keyfilter::shown(&state.key_input.aids),
        ),
        (
            Builtin::MouseKeys,
            crate::mousekeys::shown(&state.key_input.mouse_keys),
        ),
        (Builtin::Layout, state.key_input.layouts.switchable()),
        (Builtin::Displays, crate::displays::cell_shown(state)),
        (
            Builtin::Notifications,
            state.notify.log.bell_shown() || state.notify.quiet.automatic(),
        ),
        (Builtin::Sharing, state.privacy.sharing_shown()),
        (Builtin::Microphone, state.privacy.mic_shown()),
        (Builtin::Camera, state.privacy.camera_shown()),
        (Builtin::Location, state.location.shown()),
    ]
    .into_iter()
    .filter_map(|(b, on)| on.then_some(b))
    .collect()
}

/// What cell `b` draws, `px` square; None while its state is not here.
pub fn icon(state: &mut State, b: Builtin, px: i32) -> Option<Canvas> {
    match b {
        Builtin::StickyKeys => {
            let f = &state.key_input.aids.filter;
            Some(indicator::draw_sticky(px, f.latched() | f.locked()))
        }
        Builtin::MouseKeys => {
            let m = &state.key_input.mouse_keys.model;
            let pick = match m.pick() {
                crate::input::BTN_MIDDLE => indicator::MouseButton::Middle,
                BTN_RIGHT => indicator::MouseButton::Right,
                _ => indicator::MouseButton::Left,
            };
            Some(indicator::draw_mouse_keys(px, pick, m.held().is_some()))
        }
        Builtin::Layout => {
            let theme = &state.config.theme;
            let size = theme.fonts.size_px * (px / 16).max(1) as u32;
            let layouts = &state.key_input.layouts;
            let short = layouts.active().map_or("", |d| d.short.as_str());
            Some(indicator::draw_layout(
                &state.text,
                &theme.colors,
                short,
                px,
                size,
            ))
        }
        Builtin::Displays => Some(indicator::draw_sun(
            px,
            crate::nightlight::warming(&state.night),
        )),
        Builtin::Sharing => Some(indicator::draw_record(px)),
        Builtin::Microphone => Some(indicator::draw_microphone(px)),
        Builtin::Camera => Some(indicator::draw_camera(px)),
        Builtin::Location => Some(indicator::draw_location(px)),
        Builtin::Notifications => {
            let log = &state.notify.log;
            let quiet = log.dnd || state.notify.quiet.automatic();
            Some(indicator::draw_bell(px, log.unseen, quiet))
        }
    }
}

/// Cell `b`'s tooltip, also its name for a screen reader; None for a layout cell with no active layout.
pub fn label(state: &State, b: Builtin) -> Option<String> {
    match b {
        Builtin::StickyKeys => Some(crate::keyfilter::tip(&state.key_input.aids)),
        Builtin::MouseKeys => Some(crate::mousekeys::tip(&state.key_input.mouse_keys)),
        Builtin::Layout => Some(state.key_input.layouts.active()?.description.clone()),
        Builtin::Displays => Some(crate::displays::tip(state)),
        Builtin::Notifications => {
            let log = &state.notify.log;
            let quiet = Suppress {
                manual: log.dnd,
                ..state.notify.quiet
            };
            Some(bell_tip(log.unseen, quiet))
        }
        Builtin::Sharing => Some(state.privacy.sharing_tip()),
        Builtin::Microphone => Some(state.privacy.mic_tip()),
        Builtin::Camera => Some(crate::privacy::camera_tip(state)),
        Builtin::Location => Some(crate::location::tip(state)),
    }
}

fn bell_tip(unseen: usize, quiet: Suppress) -> String {
    let count = match unseen {
        0 => "Notifications".to_string(),
        1 => "1 new notification".to_string(),
        n => format!("{n} new notifications"),
    };
    let why = if quiet.manual {
        ""
    } else if quiet.fullscreen_focus {
        " while a full-screen window has focus"
    } else if quiet.sharing {
        " while the screen is shared"
    } else if quiet.scheduled {
        " during quiet hours"
    } else {
        return count;
    };
    format!("{count}\nDo not disturb is on{why}")
}

/// A pointer button on cell `b`, which sits at `cell` (layout coordinates) on `output`'s taskbar.
pub fn click(state: &mut State, output: &Output, b: Builtin, cell: Rect, button: u32) {
    match b {
        Builtin::Layout if button == BTN_LEFT => state.open_layout_menu(output, cell),
        Builtin::Displays if button == BTN_LEFT => crate::displaysview::toggle(state, output, cell),
        Builtin::Displays if button == BTN_RIGHT => {
            crate::displaysview::close(state);
            crate::nightlight::open_menu(state, output, cell)
        }
        Builtin::Notifications if matches!(button, BTN_LEFT | BTN_RIGHT) => {
            crate::notifycenter::toggle(state, output, cell)
        }
        Builtin::StickyKeys if button == BTN_LEFT => {
            state.spawn_argv_activated(&crate::keyfilter::Aid::StickyKeys.settings_argv())
        }
        Builtin::MouseKeys if button == BTN_LEFT => {
            state.spawn_argv_activated(&crate::keyfilter::Aid::MouseKeys.settings_argv())
        }
        // The other indicators only tell; a click on them does nothing.
        Builtin::StickyKeys
        | Builtin::MouseKeys
        | Builtin::Layout
        | Builtin::Displays
        | Builtin::Notifications
        | Builtin::Sharing
        | Builtin::Microphone
        | Builtin::Camera
        | Builtin::Location => {}
    }
}

/// A wheel step on cell `b`, with the axis event's `discrete` and `continuous` (see [`crate::displays::wheel_steps`]): the displays cell steps every display's brightness up or down.
pub fn scroll(state: &mut State, b: Builtin, horizontal: bool, discrete: i32, continuous: f64) {
    if b != Builtin::Displays || horizontal {
        return;
    }
    let steps = crate::displays::wheel_steps(discrete, continuous, &mut state.displays.wheel);
    if steps != 0 {
        crate::displays::step(state, None, Value::Brightness, steps);
        state.show_display_osd(Value::Brightness);
    }
}

/// Whether the open menu is cell `b`'s own, so a click on the cell closes it.
pub fn menu_open(state: &State, b: Builtin) -> bool {
    use crate::menuview::MenuKind;
    matches!(
        (b, state.menus.first().map(|m| &m.kind)),
        (Builtin::Layout, Some(MenuKind::Layouts))
            | (Builtin::Displays, Some(MenuKind::NightLight(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_tip_counts_what_is_new() {
        let none = Suppress::default();
        assert_eq!(bell_tip(0, none), "Notifications");
        assert_eq!(bell_tip(1, none), "1 new notification");
        let manual = Suppress {
            manual: true,
            sharing: true,
            ..none
        };
        assert_eq!(
            bell_tip(4, manual),
            "4 new notifications\nDo not disturb is on"
        );
    }

    #[test]
    fn the_bell_tip_says_why_do_not_disturb_turned_itself_on() {
        let none = Suppress::default();
        let tip = |s| {
            bell_tip(0, s)
                .lines()
                .nth(1)
                .unwrap_or_default()
                .to_string()
        };
        let full = Suppress {
            fullscreen_focus: true,
            sharing: true,
            ..none
        };
        assert_eq!(
            tip(full),
            "Do not disturb is on while a full-screen window has focus"
        );
        let shared = Suppress {
            sharing: true,
            ..none
        };
        assert_eq!(
            tip(shared),
            "Do not disturb is on while the screen is shared"
        );
        let quiet = Suppress {
            scheduled: true,
            ..none
        };
        assert_eq!(tip(quiet), "Do not disturb is on during quiet hours");
    }
}
