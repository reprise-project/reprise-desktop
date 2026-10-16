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
/// `BTN_FORWARD`, which Xwayland sends as X button 10: a press programs leave unbound, though a
/// menu's grab still closes on it.
const BTN_FORWARD: u32 = 0x115;

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

/// A point of `area` that no rect of `covered` holds, bottom corners first. Every uncovered region
/// has a point on an edge of `area` or just outside a rect on each axis, so only those are tried.
fn free_point(
    area: Rectangle<i32, Logical>,
    covered: &[Rectangle<i32, Logical>],
) -> Option<Point<i32, Logical>> {
    let (l, t) = (area.loc.x, area.loc.y);
    let (r, b) = (l + area.size.w - 1, t + area.size.h - 1);
    let xs: Vec<i32> = [l, r]
        .into_iter()
        .chain(
            covered
                .iter()
                .flat_map(|c| [c.loc.x - 1, c.loc.x + c.size.w]),
        )
        .filter(|x| (l..=r).contains(x))
        .collect();
    let ys = [b, t]
        .into_iter()
        .chain(
            covered
                .iter()
                .flat_map(|c| [c.loc.y + c.size.h, c.loc.y - 1]),
        )
        .filter(|y| (t..=b).contains(y));
    ys.flat_map(|y| xs.iter().map(move |&x| Point::from((x, y))))
        .find(|&p| !covered.iter().any(|c| c.contains(p)))
}

/// Where to press and with which button. A left press outside every X window (`windows`) reaches
/// the root window, so it only closes what a grab holds open. With none free, the grab takes a
/// `BTN_FORWARD` press anywhere off the override-redirect windows (`popups`).
fn press_at(
    area: Rectangle<i32, Logical>,
    windows: &[Rectangle<i32, Logical>],
    popups: &[Rectangle<i32, Logical>],
) -> Option<(Point<i32, Logical>, u32)> {
    free_point(area, windows)
        .map(|p| (p, BTN_LEFT))
        .or_else(|| free_point(area, popups).map(|p| (p, BTN_FORWARD)))
}

/// Whether the open `menus` are owed a press: one of them has not had one (`pressed`).
fn due(menus: &[X11Window], pressed: &[X11Window]) -> bool {
    menus.iter().any(|m| !pressed.contains(m))
}

/// Press once for the open X11 menus at `press_at`, then give the pointer back to what lies under
/// it. Not while the pointer is grabbed or the screen is locked. A menu is a typed one or an
/// untyped override-redirect window that showed during an X pointer grab.
pub fn dismiss(state: &mut State) {
    let mut menus = Vec::new();
    let mut windows = Vec::new();
    let mut popups = Vec::new();
    let mut target = None;
    for w in &state.windows {
        let Some(x) = w.x11() else { continue };
        windows.push(x.geometry());
        if w.override_redirect {
            popups.push(x.geometry());
        }
        let grabbing = state.xwayland.grabbing.contains(&x.window_id());
        if w.mapped && w.override_redirect && (is_menu(x.window_type()) || grabbing) {
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
    let Some((at, btn)) = press_at(state.output_area(), &windows, &popups) else {
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
                button: btn,
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
    fn a_gap_between_windows_that_cover_every_corner_is_free() {
        let area = rect(0, 0, 1920, 1080);
        let left = rect(0, 0, 900, 1080);
        let right = rect(1000, 0, 920, 1080);
        assert_eq!(free_point(area, &[left, right]), Some((900, 1079).into()));
        let top = rect(0, 0, 1920, 500);
        let bottom = rect(0, 600, 1920, 480);
        assert_eq!(free_point(area, &[top, bottom]), Some((0, 500).into()));
    }

    #[test]
    fn a_free_corner_takes_a_left_press() {
        let area = rect(0, 0, 1920, 1080);
        let window = rect(100, 100, 800, 600);
        let menu = rect(150, 130, 200, 300);
        assert_eq!(
            press_at(area, &[window, menu], &[menu]),
            Some(((0, 1079).into(), BTN_LEFT))
        );
    }

    #[test]
    fn a_fullscreen_window_takes_a_forward_press_off_the_menus() {
        let area = rect(0, 0, 1920, 1080);
        let fullscreen = rect(0, 0, 1920, 1080);
        let menu = rect(0, 900, 300, 180);
        assert_eq!(
            press_at(area, &[fullscreen, menu], &[menu]),
            Some(((1919, 1079).into(), BTN_FORWARD))
        );
        let everywhere = rect(0, 0, 1920, 1080);
        assert_eq!(
            press_at(area, &[fullscreen, everywhere], &[everywhere]),
            None
        );
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
