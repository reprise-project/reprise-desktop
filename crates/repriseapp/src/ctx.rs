//! The handle a program gets with each event and draw, and the waker for its threads.

use crate::sizing::at_pick;
use crate::{Slot, heads};
use reprisekit::icontheme::IconCache;
use reprisekit::raster::Rect;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use smithay_client_toolkit::reexports::calloop::ping::{Ping, make_ping};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

pub struct Ctx {
    /// The look's theme. Its text stays at 100% unless the program is
    /// [`Extended::text_sized`](crate::Extended::text_sized): then it is at the text size, and the program
    /// lays its lists, menus and bars out with [`Theme::controls`]. Forms draw with [`Ctx::forms`].
    pub theme: Theme,
    /// `[accessibility] text_size`, in percent: what a program grows its
    /// forms ([`Ctx::forms`]) and its document's text (the edit control) by.
    pub text_size: u32,
    /// The look's UI font at 100%, shared: a program holding a clone for its dialogs takes it
    /// again from here, as a `[fonts]` change replaces it.
    pub text: Rc<TextRenderer>,
    pub icons: IconCache,
    pub caret_on: bool,
    /// The window's client size, logical px.
    pub size: (i32, i32),
    pub maximized: bool,
    /// The window is at the size its program picked, held inside the compositor's bounds:
    /// neither the user nor the compositor sized it since. A program keeps only other sizes.
    pub at_pick: bool,
    /// The global position of the window the event or draw is for: the
    /// output's top-left for a flyout, (0, 0) for a toplevel, which
    /// cannot know its own.
    pub origin: (i32, i32),
    /// Wakes the event loop with `Event::Wake` on `MAIN` from any thread.
    pub wake: Waker,
    pub heads: heads::HeadRequests,
    /// Set while handling an event by a control that steps on its own while held (a sheet's
    /// scrollbar arrow, [`keys::sheet_input`](crate::keys::sheet_input)): the host sends the window
    /// an `Event::Tick`.
    pub want_tick: std::cell::Cell<bool>,
    /// The largest output's logical size; `None` until the compositor names one.
    pub output_size: Option<(i32, i32)>,
}

impl Ctx {
    /// The theme a program's forms, property sheets, wizards and message boxes draw with: `theme`
    /// at the text size. Their 100% layouts grow by its `text`; a window holding one is
    /// `forms().text.size` of its 100% size.
    pub fn forms(&self) -> Theme {
        if self.theme.text.is_one() {
            self.theme.with_form_text(self.text_size)
        } else {
            self.theme.clone()
        }
    }

    /// How [`Ctx::forms`] grows a 100% layout, without building its theme.
    pub fn form_units(&self) -> reprisekit::theme::TextUnits {
        let base = self.base_px();
        reprisekit::theme::TextUnits::new(reprisekit::theme::text_px(base, self.text_size), base)
    }

    /// What a menu shown on popups fits its width to and stays inside: the largest output, or the
    /// window before an output is known. A menu drawn in the window fits the window instead.
    pub fn menu_within(&self) -> Rect {
        let (w, h) = self.output_size.unwrap_or(self.size);
        Rect::new(0, 0, w, h)
    }

    /// Point the window fields at slot `s`'s window, for an event, draw or tree of it.
    pub(crate) fn aim(&mut self, s: &Slot) {
        self.size = s.size;
        self.maximized = s.maximized;
        self.at_pick = at_pick(s.size, s.picked, s.bounds);
        self.origin = s.origin;
    }

    /// The UI font's px at 100% text, what a form's `units().px(..)` grows.
    pub fn base_px(&self) -> u32 {
        self.theme.text.base_px(self.theme.fonts.size_px)
    }

    /// Ask the compositor to set `cfg` on output `name`; returns the number
    /// `Event::HeadsApplied` answers with. Fails without
    /// [`Extended::output_management`](crate::Extended::output_management) or the protocol.
    pub fn configure_head(&mut self, name: &str, cfg: heads::HeadConfig) -> u32 {
        self.heads.configure(name, cfg)
    }

    /// Run `draw` with the look's style built for `scale` device px per
    /// logical px, so parts land crisp on a canvas at `scale`. The theme's
    /// metrics stay at 1: a program lays out in logical px.
    pub fn at_scale<T>(&mut self, scale: i32, draw: impl FnOnce(&Ctx) -> T) -> T {
        let look = self.theme.look.at_scale(scale);
        let rest = std::mem::replace(&mut self.theme.look, look);
        let out = draw(self);
        self.theme.look = rest;
        out
    }

    /// Whether the theme's look draws parts under the pointer apart: a
    /// host redraws for a hover change only then.
    pub fn styled(&self) -> bool {
        self.theme.look.lights_pointer()
    }
}

/// A cheap, cloneable handle that wakes the event loop: `Waker::wake`
/// (callable from any thread, e.g. a `spawn`ed helper's watcher) queues
/// `Event::Wake` on `MAIN`.
#[derive(Clone)]
pub struct Waker(pub(crate) Arc<Ping>);

impl Waker {
    pub fn wake(&self) {
        self.0.ping();
    }

    pub fn noop() -> Waker {
        let (ping, _source) = make_ping().expect("ping");
        Waker(Arc::new(ping))
    }
}

/// A worker thread's end of a channel to the program: each message sent wakes the event loop,
/// which drains the [`Receiver`] on `Event::Wake`.
pub struct Poster<M> {
    tx: Sender<M>,
    waker: Waker,
}

impl<M> Clone for Poster<M> {
    fn clone(&self) -> Self {
        Poster {
            tx: self.tx.clone(),
            waker: self.waker.clone(),
        }
    }
}

impl<M> Poster<M> {
    pub fn channel(waker: Waker) -> (Poster<M>, Receiver<M>) {
        let (tx, rx) = channel();
        (Poster { tx, waker }, rx)
    }

    /// Sends `m` and wakes the loop; a program that has dropped its receiver gets neither.
    pub fn post(&self, m: M) {
        if self.tx.send(m).is_ok() {
            self.waker.wake();
        }
    }
}
