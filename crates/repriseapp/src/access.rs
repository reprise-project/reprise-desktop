//! The screen reader side of a window: one AccessKit adapter per surface, speaking AT-SPI on the
//! session's accessibility bus.
//!
//! `accesskit_unix` calls the handlers from its own D-Bus thread; the shared
//! `reprisekit::access::Handler` only posts a [`Msg`] to the event loop, where the program's state
//! lives. Nothing is built until an assistive tool asks for the tree.

use reprisekit::access::{Gesture, Handler, Published};
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::{BTN_LEFT, BTN_RIGHT, Mods, PointerInput, WinId};

pub(crate) type Msg = reprisekit::access::Msg<WinId>;

pub(crate) struct SlotAccess {
    pub adapter: accesskit_unix::Adapter,
    pub state: AccessState,
}

impl SlotAccess {
    pub fn new(win: WinId, tx: &Sender<Msg>) -> SlotAccess {
        let handler = || {
            let tx = tx.clone();
            Handler::new(win, move |m| {
                let _ = tx.send(m);
            })
        };
        SlotAccess {
            adapter: accesskit_unix::Adapter::new(handler(), handler(), handler()),
            state: AccessState::new(),
        }
    }
}

pub(crate) struct AccessState {
    pub published: Published,
    /// Something may have changed since the last update; the next draw sends one.
    pub dirty: bool,
    /// A screen reader holds this window's tree.
    pub active: bool,
    /// The focus was in a password field at the last update.
    pub secret: bool,
}

impl AccessState {
    pub fn new() -> AccessState {
        AccessState {
            published: Published::default(),
            dirty: true,
            active: false,
            secret: false,
        }
    }

    /// Called after each draw of a window that is `mapped` or not yet. A form learns its look's
    /// control sizes on the first draw, so until the window maps every draw rebuilds the tree.
    pub fn after_draw(&mut self, mapped: bool) {
        if !mapped {
            self.dirty = true;
        }
    }
}

/// The pointer input a user's click or hover would send for gesture `g`; a node's middle is
/// rounded down to whole logical px.
pub(crate) fn reader_input(g: Gesture, time: u32) -> Vec<PointerInput> {
    let click = |(x, y): (f64, f64), button| {
        let (x, y) = (x as i32, y as i32);
        let mods = Mods::default();
        vec![
            PointerInput::Press {
                x,
                y,
                button,
                time,
                mods,
            },
            PointerInput::Release { x, y, button, mods },
        ]
    };
    match g {
        Gesture::Click(at) => click(at, BTN_LEFT),
        Gesture::RightClick(at) => click(at, BTN_RIGHT),
        Gesture::Hover((x, y)) => vec![PointerInput::Motion {
            x: x as i32,
            y: y as i32,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_readers_click_is_a_press_and_release_at_the_nodes_middle() {
        let mods = Mods::default();
        assert_eq!(
            reader_input(Gesture::Click((40.5, 12.0)), 7),
            [
                PointerInput::Press {
                    x: 40,
                    y: 12,
                    button: BTN_LEFT,
                    time: 7,
                    mods
                },
                PointerInput::Release {
                    x: 40,
                    y: 12,
                    button: BTN_LEFT,
                    mods
                },
            ]
        );
        assert!(matches!(
            reader_input(Gesture::RightClick((1.0, 2.0)), 0)[0],
            PointerInput::Press {
                button: BTN_RIGHT,
                ..
            }
        ));
        assert_eq!(
            reader_input(Gesture::Hover((3.0, 4.0)), 0),
            [PointerInput::Motion { x: 3, y: 4 }]
        );
    }

    #[test]
    fn a_draw_before_the_window_is_mapped_rebuilds_the_tree() {
        let mut a = AccessState::new();
        a.dirty = false;
        a.after_draw(false);
        assert!(a.dirty, "an unmapped window's draw marks its tree dirty");
    }

    #[test]
    fn a_draw_of_a_mapped_window_leaves_the_tree_alone() {
        let mut a = AccessState::new();
        a.dirty = false;
        a.after_draw(true);
        assert!(!a.dirty, "a mapped window's draw leaves a clean tree clean");
        a.dirty = true;
        a.after_draw(true);
        assert!(a.dirty, "and a dirty one dirty");
    }
}
