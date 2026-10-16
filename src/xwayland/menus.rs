//! An X11 menu closes on a press its X pointer grab sees, but Xwayland sees only input sent to its
//! surfaces: [`dismiss`] presses through Xwayland where Reprise or a Wayland client takes the input.

use smithay::{
    backend::input::{ButtonState, InputTime},
    input::pointer::{ButtonEvent, MotionEvent},
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    xwayland::xwm::{WmWindowType, X11Window},
};

use crate::compositor::State;

/// `BTN_LEFT` from linux/input-event-codes.h.
const BTN_LEFT: u32 = 0x110;

/// Whether an override-redirect window of type `kind` is a menu that holds the X pointer grab
/// while open.
fn is_menu(kind: Option<WmWindowType>) -> bool {
    matches!(
        kind,
        Some(
            WmWindowType::Combo
                | WmWindowType::DropdownMenu
                | WmWindowType::Menu
                | WmWindowType::PopupMenu
        )
    )
}

/// A corner of `area` that no rect of `covered` holds: a press there reaches the X root window,
/// so it only closes what a grab holds open.
fn free_point(
    area: Rectangle<i32, Logical>,
    covered: &[Rectangle<i32, Logical>],
) -> Option<Point<i32, Logical>> {
    let (l, t) = (area.loc.x, area.loc.y);
    let (r, b) = (l + area.size.w - 1, t + area.size.h - 1);
    [(l, b), (r, b), (l, t), (r, t)]
        .into_iter()
        .map(Point::from)
        .find(|&p| !covered.iter().any(|c| c.contains(p)))
}

/// Whether the open `menus` are owed a press: one of them has not had one (`pressed`).
fn due(menus: &[X11Window], pressed: &[X11Window]) -> bool {
    menus.iter().any(|m| !pressed.contains(m))
}

/// Click once for the open X11 menus at a spot no X window covers, then give the pointer back to
/// what lies under it. Not while the pointer is grabbed or the screen is locked.
pub fn dismiss(state: &mut State) {
    let mut menus = Vec::new();
    let mut covered = Vec::new();
    let mut target = None;
    for w in &state.windows {
        let Some(x) = w.x11() else { continue };
        covered.push(x.geometry());
        if w.mapped && w.override_redirect && is_menu(x.window_type()) {
            menus.push(x.window_id());
            let (ox, oy) = w.surface_origin();
            target = Some((w.wl_surface().clone(), Point::from((ox as f64, oy as f64))));
        }
    }
    if !due(&menus, &state.xwayland.pressed) {
        state.xwayland.pressed = menus;
        return;
    }
    let Some((surface, origin)) = target else {
        return;
    };
    let Some(pointer) = state.seat.get_pointer() else {
        return;
    };
    if pointer.is_grabbed() || state.is_locked() {
        return;
    }
    let Some(at) = free_point(state.output_area(), &covered) else {
        return;
    };
    state.xwayland.pressed = menus;
    let pos = state.pointer_location();
    let time = InputTime::now();
    pointer.motion(
        state,
        Some((surface.into(), origin)),
        &MotionEvent {
            location: at.to_f64(),
            serial: SERIAL_COUNTER.next_serial(),
            time,
        },
    );
    for button in [ButtonState::Pressed, ButtonState::Released] {
        pointer.button(
            state,
            &ButtonEvent {
                button: BTN_LEFT,
                state: button,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
    }
    pointer.frame(state);
    state.repoint(pos, time);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn menus_dropdowns_and_combo_lists_are_menus_tooltips_are_not() {
        for t in [
            WmWindowType::Combo,
            WmWindowType::DropdownMenu,
            WmWindowType::Menu,
            WmWindowType::PopupMenu,
        ] {
            assert!(is_menu(Some(t)), "{t:?}");
        }
        for t in [
            WmWindowType::Tooltip,
            WmWindowType::Notification,
            WmWindowType::Dnd,
            WmWindowType::Normal,
        ] {
            assert!(!is_menu(Some(t)), "{t:?}");
        }
        assert!(!is_menu(None));
    }

    #[test]
    fn the_press_goes_to_the_first_corner_no_x_window_covers() {
        let area = rect(0, 0, 1920, 1080);
        assert_eq!(free_point(area, &[]), Some((0, 1079).into()));
        let maximized = rect(0, 0, 1920, 1050);
        let menu = rect(0, 1000, 200, 80);
        assert_eq!(
            free_point(area, &[maximized, menu]),
            Some((1919, 1079).into())
        );
    }

    #[test]
    fn a_screen_an_x_window_fills_has_no_free_point() {
        let area = rect(1920, 0, 1280, 1024);
        assert_eq!(free_point(area, &[rect(1920, 0, 1280, 1024)]), None);
    }

    #[test]
    fn each_menu_is_pressed_for_once() {
        assert!(!due(&[], &[]));
        assert!(due(&[7], &[]));
        assert!(!due(&[7], &[7]));
        assert!(due(&[7, 9], &[7]), "a submenu opened since");
        assert!(due(&[9], &[7]), "a menu opened again");
    }
}
