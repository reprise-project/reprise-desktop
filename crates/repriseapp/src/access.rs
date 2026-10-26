//! The screen reader side of a window: one AccessKit adapter per surface, speaking AT-SPI on the
//! session's accessibility bus.
//!
//! `accesskit_unix` calls the handlers from its own D-Bus thread; the shared
//! `reprisekit::access::Handler` only posts a [`Msg`] to the event loop, where the program's state
//! lives. Nothing is built until an assistive tool asks for the tree.

use reprisekit::access::{Gesture, Handler, Published, Reveal, Route, Tree, accesskit};
use smithay_client_toolkit::reexports::calloop::channel::Sender;

use crate::{BTN_LEFT, BTN_RIGHT, DropForm, Host, Mods, PointerInput, WinId};
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

/// A screen reader's action that takes no click ([`crate::Extended::reader`]), at a window point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reader {
    /// Scroll row `row` of the list at `at` into view, picking nothing; the row is counted as
    /// `reprisekit::access::Reveal` counts it.
    Reveal { at: (i32, i32), row: usize },
    /// Give the keyboard to the control at the point, as Tab would, pressing nothing.
    Focus((i32, i32)),
}

/// What a screen reader's Focus and ScrollIntoView reach in a window without a click
/// ([`crate::Extended::reader_form`]): a form, or a property sheet and its open page.
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

impl Host {
    pub(crate) fn publish(&mut self, id: WinId) {
        let Some(i) = self.slots.iter().position(|s| s.id == id) else {
            return;
        };
        let s = &mut self.slots[i];
        if !s.access.state.dirty {
            return;
        }
        s.access.state.dirty = false;
        let title = if s.title.is_empty() {
            self.app.title()
        } else {
            s.title.clone()
        };
        let (size, modal) = (s.size, s.modal);
        self.ctx.aim(s);
        let (app, ctx) = (&mut self.app, &self.ctx);
        let menu = self.edit_menu.menu(id);
        let acc = &mut self.slots[i].access;
        let Some(adapter) = acc.adapter.as_mut() else {
            return;
        };
        adapter.update_if_active(|| {
            let mut tree = Tree::window(&title, size);
            if modal && let Some(root) = tree.get_mut(reprisekit::access::ROOT) {
                root.set_role(accesskit::Role::Dialog);
                root.set_modal();
            }
            app.access(id, &mut tree, ctx);
            if let Some(menu) = menu {
                let node = menu.access(&mut tree, reprisekit::access::ROOT);
                tree.set_focus(node);
            }
            acc.state.secret = tree
                .get(tree.focus())
                .is_some_and(|n| n.role() == accesskit::Role::PasswordInput);
            acc.state.published.update(tree)
        });
    }

    pub(crate) fn access_msg(&mut self, msg: Msg) {
        match msg {
            Msg::Activate(win) => {
                if let Some(s) = self.slot_mut(win) {
                    s.access.state.published.reset();
                    s.access.state.dirty = true;
                    s.access.state.active = true;
                }
                self.key_echo
                    .get_or_insert_with(reprisebus::a11y::KeyEcho::spawn);
                self.publish(win);
            }
            Msg::Deactivate(win) => {
                if let Some(s) = self.slot_mut(win) {
                    s.access.state.published.reset();
                    s.access.state.active = false;
                }
            }
            Msg::Act(win, req) => {
                let target = match self.reader_route(win, req) {
                    Route::Reveal { reveal, then } => {
                        self.reader_reveal(win, reveal);
                        if !then {
                            return;
                        }
                        self.publish(win);
                        match self.reader_route(win, req) {
                            Route::Act(t) => t,
                            _ => return,
                        }
                    }
                    Route::Act(t) => t,
                    Route::None => return,
                };
                let Some(g) = reprisekit::access::gesture(req.act, target) else {
                    return;
                };
                if let Gesture::Focus(at) = g {
                    self.reader_focus(win, at);
                    return;
                }
                let apart = self.ctx.theme.metrics.double_click_ms.max(0) as u32 + 1000;
                self.press_time = self.press_time.wrapping_add(apart);
                for input in reader_input(g, self.press_time) {
                    self.pointer(win, input);
                }
            }
        }
    }

    /// How a reader's `req` in window `win` is carried out, on the tree last published there.
    fn reader_route(&mut self, win: WinId, req: reprisekit::access::Request) -> Route {
        (self.slot_mut(win))
            .filter(|s| s.access.state.active)
            .map_or(Route::None, |s| s.access.state.published.route(req))
    }

    /// Scroll the list in window `win` so the row `r` names shows.
    fn reader_reveal(&mut self, win: WinId, r: Reveal) {
        let at = (r.at.0 as i32, r.at.1 as i32);
        self.reader_act(win, Reader::Reveal { at, row: r.row });
    }

    /// Give the keyboard to the control at `at` in window `win`, pressing nothing.
    fn reader_focus(&mut self, win: WinId, at: (f64, f64)) {
        self.reader_act(win, Reader::Focus((at.0 as i32, at.1 as i32)));
    }

    /// Hand `act` to the program ([`Extended::reader`](crate::Extended::reader)); when it reports a change
    /// the window and its tree are drawn again.
    fn reader_act(&mut self, win: WinId, act: Reader) {
        let Some(s) = self.slots.iter().find(|s| s.id == win) else {
            return;
        };
        self.ctx.aim(s);
        if self.app.reader(win, act, &self.ctx)
            && let Some(s) = self.slot_mut(win)
        {
            s.dirty = true;
            s.access.state.dirty = true;
        }
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
