//! The tray cells the compositor draws itself: the StickyKeys, MouseKeys and keyboard layout indicators before the StatusNotifier items, and after them the night light, the notification bell and the privacy indicators nearest the clock. A new indicator is one more variant here.

use crate::compositor::State;
use crate::input::{BTN_LEFT, BTN_RIGHT};
use crate::raster::{Canvas, Rect};
use reprisekit::classic::indicator;
use smithay::output::Output;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Builtin {
    StickyKeys,
    /// MouseKeys, while it is on: which button the keys click, and whether it is held.
    MouseKeys,
    Layout,
    NightLight,
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
        (Builtin::NightLight, crate::nightlight::shown(&state.night)),
        (Builtin::Notifications, state.notify.log.bell_shown()),
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
        Builtin::NightLight => Some(crate::nightlight::icon(state, px)),
        Builtin::Sharing => Some(indicator::draw_record(px)),
        Builtin::Microphone => Some(indicator::draw_microphone(px)),
        Builtin::Camera => Some(indicator::draw_camera(px)),
        Builtin::Location => Some(indicator::draw_location(px)),
        Builtin::Notifications => {
            let log = &state.notify.log;
            Some(indicator::draw_bell(px, log.unseen, log.dnd))
        }
    }
}

/// Cell `b`'s tooltip, also its name for a screen reader; None for a layout cell with no active layout.
pub fn label(state: &State, b: Builtin) -> Option<String> {
    match b {
        Builtin::StickyKeys => Some(crate::keyfilter::tip(&state.key_input.aids)),
        Builtin::MouseKeys => Some(crate::mousekeys::tip(&state.key_input.mouse_keys)),
        Builtin::Layout => Some(state.key_input.layouts.active()?.description.clone()),
        Builtin::NightLight => Some(crate::nightlight::tip(&state.night)),
        Builtin::Notifications => Some(bell_tip(state.notify.log.unseen, state.notify.log.dnd)),
        Builtin::Sharing => Some(state.privacy.sharing_tip()),
        Builtin::Microphone => Some(state.privacy.mic_tip()),
        Builtin::Camera => Some(crate::privacy::camera_tip(state)),
        Builtin::Location => Some(crate::location::tip(state)),
    }
}

fn bell_tip(unseen: usize, dnd: bool) -> String {
    let count = match unseen {
        0 => "Notifications".to_string(),
        1 => "1 new notification".to_string(),
        n => format!("{n} new notifications"),
    };
    if dnd {
        format!("{count}\nDo not disturb is on")
    } else {
        count
    }
}

/// A pointer button on cell `b`, which sits at `cell` (layout coordinates) on `output`'s taskbar.
pub fn click(state: &mut State, output: &Output, b: Builtin, cell: Rect, button: u32) {
    match b {
        Builtin::Layout if button == BTN_LEFT => state.open_layout_menu(output, cell),
        Builtin::NightLight if button == BTN_LEFT => crate::nightlight::toggle_pause(state),
        Builtin::NightLight if button == BTN_RIGHT => {
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
        | Builtin::NightLight
        | Builtin::Notifications
        | Builtin::Sharing
        | Builtin::Microphone
        | Builtin::Camera
        | Builtin::Location => {}
    }
}

/// Whether the open menu is cell `b`'s own, so a click on the cell closes it.
pub fn menu_open(state: &State, b: Builtin) -> bool {
    use crate::menuview::MenuKind;
    matches!(
        (b, state.menus.first().map(|m| &m.kind)),
        (Builtin::Layout, Some(MenuKind::Layouts))
            | (Builtin::NightLight, Some(MenuKind::NightLight(_)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bell_tip_counts_what_is_new() {
        assert_eq!(bell_tip(0, false), "Notifications");
        assert_eq!(bell_tip(1, false), "1 new notification");
        assert_eq!(
            bell_tip(4, true),
            "4 new notifications\nDo not disturb is on"
        );
    }
}
