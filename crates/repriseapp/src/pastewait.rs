//! A paste reads as if it were synchronous. While a paste's source has not answered, the
//! program's keys, pointer, input-method commits, screen reader requests and close requests from
//! the compositor wait here, and once the paste lands they go on in the order they came. Input
//! waits at most [`PASTE_HOLD`] from the ask: then it goes on, and the paste still lands when it
//! comes, within the read's own limit.
//!
//! Only user input waits: drawing, timers, wakes, focus, drags and the compositor's other events
//! go on. A drag is answered live (the compositor asks for an accept on every motion) and its drop
//! re-checks the modal gate when its data arrives. While input waits, a key's repeats are dropped
//! and a pointer motion replaces the motion before it, so a slow source cannot flood the queue.

use crate::{Event, PointerInput, WinId};
use reprisekit::access::Request;
use std::collections::VecDeque;
use std::time::Duration;
use zeroize::Zeroize;

/// How long input waits for a paste, from the ask.
pub const PASTE_HOLD: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PasteId(u64);

/// What waits behind a paste.
#[derive(Debug, PartialEq)]
pub enum Held {
    Event(Event),
    /// A screen reader's request, carried out once the input before it went on.
    Reader(Request),
}

struct Out {
    id: PasteId,
    win: WinId,
    /// Input waits on it: [`PASTE_HOLD`] has not passed since the ask.
    holds: bool,
}

/// The pastes out, those landed, and the input waiting on them. The host holds one; a program's
/// tests may stand one in for the host's.
#[derive(Default)]
pub struct PasteWait {
    out: Vec<Out>,
    landed: Vec<(PasteId, WinId, Event)>,
    held: VecDeque<(WinId, Held)>,
    next: u64,
}

impl PasteWait {
    /// A paste for `win` went out; the host calls [`PasteWait::release`] [`PASTE_HOLD`] later.
    pub fn begin(&mut self, win: WinId) -> PasteId {
        self.next += 1;
        let id = PasteId(self.next);
        self.out.push(Out {
            id,
            win,
            holds: true,
        });
        id
    }

    /// [`PASTE_HOLD`] passed since paste `id`'s ask: input no longer waits on it. True when it
    /// did, so the input held may go on.
    pub fn release(&mut self, id: PasteId) -> bool {
        let o = self.out.iter_mut().find(|o| o.id == id && o.holds);
        o.map(|o| o.holds = false).is_some()
    }

    /// Paste `id`'s answer came: `ev` (None when the read failed) goes to its window from
    /// [`PasteWait::pop`], ahead of the input that waited on it. Dropped when the paste is no
    /// longer out (its window closed, or [`PasteWait::forget`]).
    pub fn land(&mut self, id: PasteId, ev: Option<Event>) {
        let Some(i) = self.out.iter().position(|o| o.id == id) else {
            if let Some(ev) = ev {
                wipe(ev);
            }
            return;
        };
        let o = self.out.remove(i);
        if let Some(ev) = ev {
            self.landed.push((id, o.win, ev));
        }
    }

    /// Drops the pastes for windows `open` says are gone; true when any was.
    pub fn close(&mut self, open: impl Fn(WinId) -> bool) -> bool {
        let before = self.out.len();
        self.out.retain(|o| open(o.win));
        let (gone, kept) = std::mem::take(&mut self.landed)
            .into_iter()
            .partition(|l| !open(l.1));
        self.landed = kept;
        gone.into_iter()
            .for_each(|(_, _, ev): (_, _, Event)| wipe(ev));
        self.out.len() != before
    }

    /// Every paste still out, or landed and not yet handed on, never arrives, and the input that
    /// waited on them is dropped ([`crate::Reply::drop_pastes`]). Key and button releases stay,
    /// so a key or button the program saw go down still comes up.
    pub fn forget(&mut self) {
        self.out.clear();
        self.landed.drain(..).for_each(|(_, _, ev)| wipe(ev));
        let release = |h: &Held| {
            matches!(
                h,
                Held::Event(Event::KeyUp(_) | Event::Pointer(PointerInput::Release { .. }))
            )
        };
        self.held.retain(|(_, h)| release(h));
    }

    /// Whether input waits: a paste holds it, or something that waited has not all gone on yet.
    pub fn holding(&self) -> bool {
        self.out.iter().any(|o| o.holds) || !self.landed.is_empty() || !self.held.is_empty()
    }

    /// User input `ev` for `win`: back to go on now, or kept behind a paste.
    pub fn hold(&mut self, win: WinId, ev: Event) -> Option<Event> {
        let motion = |ev: &Event| matches!(ev, Event::Pointer(PointerInput::Motion { .. }));
        if !self.holding() {
            return Some(ev);
        }
        match self.held.back_mut() {
            Some((w, Held::Event(last))) if motion(&ev) && *w == win && motion(last) => *last = ev,
            _ => self.held.push_back((win, Held::Event(ev))),
        }
        None
    }

    /// A screen reader's `req` for `win`, as [`PasteWait::hold`].
    pub fn hold_reader(&mut self, win: WinId, req: Request) -> Option<Request> {
        if !self.holding() {
            return Some(req);
        }
        self.held.push_back((win, Held::Reader(req)));
        None
    }

    /// What goes on next: a landed paste asked before every paste that still holds input, then,
    /// once none holds it, the input that waited.
    pub fn pop(&mut self) -> Option<(WinId, Held)> {
        let holder = self.out.iter().filter(|o| o.holds).map(|o| o.id).min();
        let first = (self.landed.iter().enumerate())
            .filter(|(_, l)| holder.is_none_or(|h| l.0 < h))
            .min_by_key(|(_, l)| l.0)
            .map(|(i, _)| i);
        if let Some(i) = first {
            let (_, win, ev) = self.landed.remove(i);
            return Some((win, Held::Event(ev)));
        }
        match holder {
            None => self.held.pop_front(),
            Some(_) => None,
        }
    }
}

/// Drops `ev`, wiping a paste's text.
fn wipe(ev: Event) {
    if let Event::Pasted(mut t) | Event::PastedPrimary(mut t) = ev {
        t.zeroize();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{KeyInput, MAIN, Mods};
    use reprisekit::access::{Act, ROOT};
    use reprisekit::textedit::{EditKey, keysym};

    const BOX: WinId = WinId::named("box");

    fn key(ch: char, ctrl: bool) -> KeyInput {
        let edit = (ctrl && ch == 'v').then_some(EditKey::Paste);
        KeyInput {
            sym: ch as u32,
            utf8: (!ctrl).then(|| ch.to_string()),
            mods: Mods {
                ctrl,
                ..Mods::default()
            },
            edit,
        }
    }

    fn escape() -> KeyInput {
        KeyInput {
            sym: keysym::ESCAPE,
            utf8: None,
            mods: Mods::default(),
            edit: None,
        }
    }

    fn name(k: &KeyInput) -> String {
        match k.sym {
            keysym::ESCAPE => "Escape".into(),
            sym => char::from_u32(sym).map_or("?".into(), String::from),
        }
    }

    /// A program whose Ctrl+V asks for the clipboard and whose Ctrl+O raises a box; it notes
    /// what it handled, and a paste landing while its box is up.
    #[derive(Default)]
    struct Prog {
        handled: Vec<String>,
        box_up: bool,
    }

    impl Prog {
        /// `input` for `win`; true when it asks for the clipboard.
        fn input(&mut self, win: WinId, input: Held) -> bool {
            let note = match &input {
                Held::Reader(_) => "reader".to_string(),
                Held::Event(Event::Key(k)) if k.edit == Some(EditKey::Paste) => "Ctrl+V".into(),
                Held::Event(Event::Key(k)) if k.mods.ctrl && k.sym == 'o' as u32 => {
                    self.box_up = true;
                    "Ctrl+O".into()
                }
                Held::Event(Event::Key(k)) => name(k),
                Held::Event(Event::KeyUp(k)) => format!("{} up", name(k)),
                Held::Event(Event::Pasted(t)) if self.box_up => format!("{t} under the box"),
                Held::Event(Event::Pasted(t)) => t.clone(),
                Held::Event(other) => format!("{other:?}"),
            };
            let win = if win == MAIN { "MAIN" } else { "BOX" };
            self.handled.push(format!("{win} {note}"));
            matches!(input, Held::Event(Event::Key(k)) if k.edit == Some(EditKey::Paste))
        }
    }

    /// The host's part: input through `hold`, a paste out for each ask, what goes on next after
    /// a landing or a release.
    struct Seat {
        wait: PasteWait,
        prog: Prog,
        asked: Vec<PasteId>,
    }

    impl Seat {
        fn new() -> Seat {
            Seat {
                wait: PasteWait::default(),
                prog: Prog::default(),
                asked: Vec::new(),
            }
        }

        fn send(&mut self, win: WinId, input: Held) {
            if self.prog.input(win, input) {
                self.asked.push(self.wait.begin(win));
            }
        }

        fn key(&mut self, win: WinId, k: KeyInput) {
            if let Some(ev) = self.wait.hold(win, Event::Key(k)) {
                self.send(win, Held::Event(ev));
            }
        }

        fn go_on(&mut self) {
            while let Some((win, input)) = self.wait.pop() {
                self.send(win, input);
            }
        }

        /// The `n`th paste asked answers `text`.
        fn answer(&mut self, n: usize, text: &str) {
            self.wait
                .land(self.asked[n], Some(Event::Pasted(text.into())));
            self.go_on();
        }

        /// [`PASTE_HOLD`] passed since the `n`th paste was asked.
        fn hold_passes(&mut self, n: usize) {
            if self.wait.release(self.asked[n]) {
                self.go_on();
            }
        }

        fn handled(&self) -> Vec<&str> {
            self.prog.handled.iter().map(|s| s.as_str()).collect()
        }
    }

    #[test]
    fn a_key_typed_after_ctrl_v_is_handled_after_the_paste_lands() {
        let mut s = Seat::new();
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('o', true));
        assert_eq!(s.handled(), ["MAIN Ctrl+V"], "Ctrl+O waits for the paste");
        s.answer(0, "pasted");
        assert_eq!(s.handled(), ["MAIN Ctrl+V", "MAIN pasted", "MAIN Ctrl+O"]);
        assert!(!s.wait.holding());
        assert!(
            !s.wait.release(s.asked[0]),
            "its hold has nothing to let go"
        );
    }

    #[test]
    fn a_second_ctrl_v_waits_for_the_first_paste_and_the_keys_after_it_for_the_second() {
        let mut s = Seat::new();
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('x', false));
        s.answer(0, "one");
        assert_eq!(s.handled(), ["MAIN Ctrl+V", "MAIN one", "MAIN Ctrl+V"]);
        s.key(MAIN, key('y', false));
        s.answer(1, "two");
        assert_eq!(
            s.handled(),
            [
                "MAIN Ctrl+V",
                "MAIN one",
                "MAIN Ctrl+V",
                "MAIN two",
                "MAIN x",
                "MAIN y"
            ]
        );
    }

    #[test]
    fn input_goes_on_once_the_hold_passes_and_the_paste_still_lands_after_it() {
        let mut s = Seat::new();
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('o', true));
        s.hold_passes(0);
        assert_eq!(s.handled(), ["MAIN Ctrl+V", "MAIN Ctrl+O"]);
        s.key(MAIN, key('a', false));
        assert_eq!(
            s.handled().last(),
            Some(&"MAIN a"),
            "input goes straight on"
        );
        s.answer(0, "late");
        assert_eq!(
            s.handled(),
            [
                "MAIN Ctrl+V",
                "MAIN Ctrl+O",
                "MAIN a",
                "MAIN late under the box"
            ]
        );
    }

    #[test]
    fn a_paste_past_its_hold_lands_ahead_of_the_input_held_for_a_later_paste() {
        let mut s = Seat::new();
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('x', false));
        s.hold_passes(0);
        assert_eq!(
            s.handled(),
            ["MAIN Ctrl+V", "MAIN Ctrl+V"],
            "x waits on the second"
        );
        s.answer(0, "one");
        assert_eq!(s.handled().last(), Some(&"MAIN one"));
        s.answer(1, "two");
        assert_eq!(
            s.handled(),
            [
                "MAIN Ctrl+V",
                "MAIN Ctrl+V",
                "MAIN one",
                "MAIN two",
                "MAIN x"
            ]
        );
    }

    #[test]
    fn a_forgotten_paste_never_arrives_and_only_releases_of_its_held_input_go_on() {
        let mut s = Seat::new();
        s.key(MAIN, key('v', true));
        s.key(MAIN, key('a', false));
        assert!(s.wait.hold(MAIN, Event::KeyUp(key('a', false))).is_none());
        s.key(MAIN, escape());
        s.wait.forget();
        s.go_on();
        assert_eq!(s.handled(), ["MAIN Ctrl+V", "MAIN a up"]);
        s.answer(0, "hunter2");
        assert_eq!(s.handled(), ["MAIN Ctrl+V", "MAIN a up"]);
        assert!(!s.wait.holding());
    }

    #[test]
    fn a_later_paste_landing_first_waits_for_the_earlier_one_and_held_input_for_both() {
        let mut w = PasteWait::default();
        let a = w.begin(MAIN);
        let b = w.begin(BOX);
        assert!(w.hold(MAIN, Event::Key(key('x', false))).is_none());
        w.land(b, Some(Event::Pasted("b".into())));
        assert_eq!(w.pop(), None, "b waits for a");
        w.land(a, Some(Event::Pasted("a".into())));
        let replayed: Vec<_> = std::iter::from_fn(|| w.pop()).collect();
        assert_eq!(
            replayed,
            [
                (MAIN, Held::Event(Event::Pasted("a".into()))),
                (BOX, Held::Event(Event::Pasted("b".into()))),
                (MAIN, Held::Event(Event::Key(key('x', false)))),
            ]
        );
    }

    #[test]
    fn a_screen_readers_request_waits_with_the_keys() {
        let mut s = Seat::new();
        let req = Request {
            node: ROOT,
            act: Act::Focus,
        };
        s.key(MAIN, key('v', true));
        assert_eq!(s.wait.hold_reader(MAIN, req), None);
        s.key(MAIN, key('x', false));
        s.answer(0, "pasted");
        assert_eq!(
            s.handled(),
            ["MAIN Ctrl+V", "MAIN pasted", "MAIN reader", "MAIN x"]
        );
        assert_eq!(s.wait.hold_reader(MAIN, req), Some(req), "nothing out");
    }

    #[test]
    fn motion_while_input_waits_keeps_only_the_newest_between_presses() {
        let mut w = PasteWait::default();
        let id = w.begin(MAIN);
        let motion = |x| Event::Pointer(PointerInput::Motion { x, y: 0 });
        let press = Event::Pointer(PointerInput::Press {
            x: 2,
            y: 0,
            button: crate::BTN_LEFT,
            time: 0,
            mods: Mods::default(),
        });
        for ev in [motion(1), motion(2), press.clone(), motion(3), motion(4)] {
            assert!(w.hold(MAIN, ev).is_none());
        }
        assert!(w.hold(BOX, motion(5)).is_none(), "another window's motion");
        w.land(id, None);
        let replayed: Vec<_> = std::iter::from_fn(|| w.pop()).collect();
        let held = |win, ev| (win, Held::Event(ev));
        assert_eq!(
            replayed,
            [
                held(MAIN, motion(2)),
                held(MAIN, press),
                held(MAIN, motion(4)),
                held(BOX, motion(5))
            ]
        );
    }

    #[test]
    fn a_closed_windows_paste_is_dropped_and_input_aimed_elsewhere_goes_on() {
        let mut s = Seat::new();
        s.key(BOX, key('v', true));
        s.key(MAIN, key('a', false));
        assert!(!s.wait.close(|_| true), "both windows open");
        assert!(s.wait.close(|w| w != BOX));
        s.go_on();
        assert_eq!(s.handled(), ["BOX Ctrl+V", "MAIN a"]);
        s.answer(0, "late");
        assert_eq!(
            s.handled(),
            ["BOX Ctrl+V", "MAIN a"],
            "the paste is not delivered"
        );
    }
}
