//! The screen reader side of a window: one AccessKit adapter per surface, speaking AT-SPI on the
//! session's accessibility bus.
//!
//! `accesskit_unix` calls the handlers from its own D-Bus thread; the shared
//! `reprisekit::access::Handler` only posts a [`Msg`] to the event loop, where the program's state
//! lives. Nothing is built until an assistive tool asks for the tree.

use reprisekit::access::{Gesture, Handler, Published};
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::{BTN_LEFT, BTN_RIGHT, DropForm, Mods, PointerInput, WinId};
use reprisekit::controls::{
    form,
    propsheet::{self, PropSheet},
};
use reprisekit::text::TextRenderer;

pub(crate) type Msg = reprisekit::access::Msg<WinId>;

pub(crate) struct SlotAccess {
    /// None until the accessibility bus answers (`reprisebus::a11y::when_bus_ready`).
    pub adapter: Option<accesskit_unix::Adapter>,
    pub state: AccessState,
}

impl SlotAccess {
    /// `tx` is None while the accessibility bus has not answered.
    pub fn new(win: WinId, tx: Option<&Sender<Msg>>) -> SlotAccess {
        SlotAccess {
            adapter: tx.map(|tx| adapter(win, tx)),
            state: AccessState::new(),
        }
    }

    /// Makes the adapter of a window that opened before the accessibility bus answered.
    pub fn connect(&mut self, win: WinId, tx: &Sender<Msg>, focused: bool) {
        if self.adapter.is_none() {
            let mut a = adapter(win, tx);
            if focused {
                a.update_window_focus_state(true);
            }
            self.adapter = Some(a);
        }
    }
}

fn adapter(win: WinId, tx: &Sender<Msg>) -> accesskit_unix::Adapter {
    let handler = || {
        let tx = tx.clone();
        Handler::new(win, move |m| {
            let _ = tx.send(m);
        })
    };
    accesskit_unix::Adapter::new(handler(), handler(), handler())
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

/// A screen reader's action that takes no click ([`crate::App::reader`]), at a window point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// Scroll row `row` of the list at `at` into view, picking nothing; the row is counted as
    /// `reprisekit::access::Reveal` counts it.
    Reveal { at: (i32, i32), row: usize },
    /// Give the keyboard to the control at the point, as Tab would, pressing nothing.
    Focus((i32, i32)),
}

/// What a screen reader's Focus and ScrollIntoView reach in a window without a click
/// ([`crate::App::reader_form`]): a form, or a property sheet and its open page.
pub enum ReaderForm<'a> {
    Form(DropForm<'a>),
    Sheet(&'a mut PropSheet),
}

impl ReaderForm<'_> {
    /// Carry out `act` at the font size the form is drawn at, from the theme's `base`; true
    /// when the form changed.
    pub fn act(self, act: Reader, text: &TextRenderer, base: u32) -> bool {
        match (self, act) {
            (ReaderForm::Form(d), Reader::Reveal { at: (x, y), row }) => {
                let px = d.px.unwrap_or_else(|| d.form.units().px(base));
                d.form.reveal(x, y, row, text, px) != form::Event::None
            }
            (ReaderForm::Form(d), Reader::Focus((x, y))) => {
                let px = d.px.unwrap_or_else(|| d.form.units().px(base));
                d.form.focus_at(x, y, text, px) != form::Event::None
            }
            (ReaderForm::Sheet(s), Reader::Reveal { at: (x, y), row }) => {
                let px = s.open().units().px(base);
                s.reveal(x, y, row, text, px) != propsheet::Event::None
            }
            (ReaderForm::Sheet(s), Reader::Focus((x, y))) => {
                let px = s.open().units().px(base);
                s.focus_at(x, y, text, px) != propsheet::Event::None
            }
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
        Gesture::Focus(_) => Vec::new(),
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
