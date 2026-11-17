//! Boxes that wait their turn. A window root has one modal slot: the box in it is up, and every
//! other box raised for that root waits behind it. A raise never takes the place of the box that
//! is up. A box some owner blocks on (a job held on a failed item, a thread waiting for its answer)
//! is held: it waits ahead of the plain boxes, behind the box that is up and the held boxes raised
//! before it. When the box that is up closes, the next comes up in the other of two windows, so
//! the compositor maps it fresh instead of retitling the window the last one answered in. A box
//! whose owner is gone leaves through [`Queue::withdraw`], whether it is up or waiting.
//!
//! [`Queue`] is the order alone, for a program whose boxes are its own types; [`Asks`] hosts
//! [`MsgBox`]es with a payload each on top of it.

use crate::oneshot::{MsgBox, Outcome};
use crate::{Child, Event, MAIN, WinId};
use reprisekit::access::Tree;
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Theme;
use std::collections::VecDeque;

/// One modal slot and the boxes waiting for it: the front is up.
#[derive(Debug)]
pub struct Queue<B> {
    /// Each box with whether it is held.
    boxes: VecDeque<(B, bool)>,
    turn: bool,
}

impl<B> Default for Queue<B> {
    fn default() -> Queue<B> {
        Queue {
            boxes: VecDeque::new(),
            turn: false,
        }
    }
}

impl<B> Queue<B> {
    pub fn new() -> Queue<B> {
        Queue::default()
    }

    /// Up when nothing is, else behind every box waiting.
    pub fn raise(&mut self, b: B) {
        self.boxes.push_back((b, false));
    }

    /// Up when nothing is, else behind the box up and the held boxes before it.
    pub fn raise_held(&mut self, b: B) {
        let at = (1..self.boxes.len())
            .find(|&i| !self.boxes[i].1)
            .unwrap_or(self.boxes.len());
        self.boxes.insert(at, (b, true));
    }

    pub fn up(&self) -> Option<&B> {
        self.boxes.front().map(|(b, _)| b)
    }

    pub fn up_mut(&mut self) -> Option<&mut B> {
        self.boxes.front_mut().map(|(b, _)| b)
    }

    /// Takes the box up, and the next is up.
    pub fn close(&mut self) -> Option<B> {
        let (b, _) = self.boxes.pop_front()?;
        self.turn = !self.turn;
        Some(b)
    }

    /// [`Queue::close`] when the box up is one `is` picks.
    pub fn close_if(&mut self, is: impl FnOnce(&B) -> bool) -> Option<B> {
        if !is(self.up()?) {
            return None;
        }
        self.close()
    }

    /// Puts `b`, the box [`Queue::close`] took, back up in the window it had: for a handler that
    /// needs the box by value. Boxes raised meanwhile wait behind it.
    pub fn restore(&mut self, b: B) {
        self.boxes.push_front((b, false));
        self.turn = !self.turn;
    }

    /// Takes every box `gone` picks, up or waiting, in order: their owner is gone.
    pub fn withdraw(&mut self, gone: impl Fn(&B) -> bool) -> Vec<B> {
        let mut out = Vec::new();
        let mut kept = VecDeque::with_capacity(self.boxes.len());
        for (i, (b, held)) in self.boxes.drain(..).enumerate() {
            if gone(&b) {
                if i == 0 {
                    self.turn = !self.turn;
                }
                out.push(b);
            } else {
                kept.push_back((b, held));
            }
        }
        self.boxes = kept;
        out
    }

    /// Takes every box, the one up first: the window root is going.
    pub fn drain(&mut self) -> Vec<B> {
        if !self.boxes.is_empty() {
            self.turn = !self.turn;
        }
        self.boxes.drain(..).map(|(b, _)| b).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
    }

    pub fn len(&self) -> usize {
        self.boxes.len()
    }

    /// Every box, the one up first.
    pub fn iter(&self) -> impl Iterator<Item = &B> {
        self.boxes.iter().map(|(b, _)| b)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut B> {
        self.boxes.iter_mut().map(|(b, _)| b)
    }

    /// Which of two windows, 0 or 1, the box up shows in. It changes each time the box up
    /// leaves, so the box after it maps a window of its own.
    pub fn turn(&self) -> usize {
        self.turn as usize
    }
}

/// What an event did to the box up in an [`Asks`].
#[derive(Debug, PartialEq, Eq)]
pub enum Answer<K> {
    None,
    Redraw,
    /// The box closed on the button at this index, or on `None` for Escape, the close box and a
    /// close request; the next box is up. The host's children changed.
    Closed(K, Option<usize>),
}

/// Message boxes with a payload `K` each, in one modal child window over [`Asks::over`]'s window.
/// The program lists [`Asks::child`] in its children and routes the windows [`Asks::owns`] here.
pub struct Asks<K> {
    /// Each box with its payload and the window it covers when [`Asks::ask_over`] named one.
    queue: Queue<(MsgBox, K, Option<WinId>)>,
    wins: [WinId; 2],
    parent: WinId,
}

/// Boxes in the window named `ask` over [`MAIN`], for a program with one `Asks`.
impl<K> Default for Asks<K> {
    fn default() -> Asks<K> {
        Asks::new(WinId::named("ask"))
    }
}

impl<K> Asks<K> {
    /// Boxes in window `win` and one id derived from it, which they take in turn, over
    /// [`MAIN`]. A program with several window roots gives each root its own, such as
    /// `root.named_in("confirm")`.
    pub fn new(win: WinId) -> Asks<K> {
        Asks {
            queue: Queue::new(),
            wins: [win, win.named_in("next")],
            parent: MAIN,
        }
    }

    /// The boxes cover `parent` instead of [`MAIN`] ([`Asks::parent`]).
    pub fn over(mut self, parent: WinId) -> Asks<K> {
        self.parent = parent;
        self
    }

    /// Raises `b` ([`Queue::raise`]).
    pub fn ask(&mut self, b: MsgBox, k: K) {
        self.queue.raise((b, k, None));
    }

    /// Raises `b` over `parent` in place of the boxes' own parent, such as the sheet it asks
    /// about. A box whose `parent` closes before it comes up maps alone, so the program
    /// withdraws it with that window.
    pub fn ask_over(&mut self, parent: WinId, b: MsgBox, k: K) {
        self.queue.raise((b, k, Some(parent)));
    }

    /// Raises `b` for an owner that blocks on its answer ([`Queue::raise_held`]).
    pub fn ask_held(&mut self, b: MsgBox, k: K) {
        self.queue.raise_held((b, k, None));
    }

    /// Whether `win` is one of the boxes' windows, up or left: route its events here.
    pub fn owns(&self, win: WinId) -> bool {
        self.wins.contains(&win)
    }

    /// The window of the box up.
    pub fn win(&self) -> Option<WinId> {
        (!self.queue.is_empty()).then(|| self.wins[self.queue.turn()])
    }

    /// The window a box window covers, for `Extended::parent`: the one [`Asks::ask_over`] gave
    /// the box up, else [`Asks::over`]'s; `None` when `win` is not a box window.
    pub fn parent(&self, win: WinId) -> Option<WinId> {
        let own = self.queue.up().and_then(|(_, _, p)| *p);
        self.owns(win).then_some(own.unwrap_or(self.parent))
    }

    pub fn up(&self) -> Option<(&MsgBox, &K)> {
        self.queue.up().map(|(b, k, _)| (b, k))
    }

    /// The box up, to give it an icon that decoded after it was raised.
    pub fn up_mut(&mut self) -> Option<&mut MsgBox> {
        self.queue.up_mut().map(|(b, _, _)| b)
    }

    /// Every box, the one up first.
    pub fn iter(&self) -> impl Iterator<Item = (&MsgBox, &K)> {
        self.queue.iter().map(|(b, k, _)| (b, k))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&mut MsgBox, &mut K)> {
        self.queue.iter_mut().map(|(b, k, _)| (b, k))
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Takes the box up without an answer, and the next is up.
    pub fn close(&mut self) -> Option<K> {
        self.queue.close().map(|(_, k, _)| k)
    }

    /// The window of the box up.
    pub fn child(&self, text: &TextRenderer, theme: &Theme) -> Option<Child> {
        let (b, _, _) = self.queue.up()?;
        Some(b.child(self.win()?, text, theme))
    }

    /// An event for window `win`. One for a window the box up is not in, the window of a box
    /// that closed, is dropped.
    pub fn event(
        &mut self,
        win: WinId,
        ev: &Event,
        text: &TextRenderer,
        theme: &Theme,
    ) -> Answer<K> {
        if self.win() != Some(win) {
            return Answer::None;
        }
        let Some((b, _, _)) = self.queue.up_mut() else {
            return Answer::None;
        };
        match b.event(ev, text, theme) {
            Outcome::None => Answer::None,
            Outcome::Redraw => Answer::Redraw,
            Outcome::Closed(button) => match self.queue.close() {
                Some((_, k, _)) => Answer::Closed(k, button),
                None => Answer::None,
            },
        }
    }

    pub fn draw(&self, c: &mut Canvas, theme: &Theme, text: &TextRenderer) {
        if let Some((b, _, _)) = self.queue.up() {
            b.draw(c, theme, text);
        }
    }

    pub fn access(&self, tree: &mut Tree, text: &TextRenderer, theme: &Theme) {
        if let Some((b, _, _)) = self.queue.up() {
            b.access(tree, text, theme);
        }
    }

    /// Takes the payloads of every box `gone` picks, up or waiting ([`Queue::withdraw`]).
    pub fn withdraw(&mut self, gone: impl Fn(&K) -> bool) -> Vec<K> {
        let out = self.queue.withdraw(|(_, k, _)| gone(k));
        out.into_iter().map(|(_, k, _)| k).collect()
    }

    /// Takes every box's payload, the one up first ([`Queue::drain`]).
    pub fn drain(&mut self) -> Vec<K> {
        self.queue.drain().into_iter().map(|(_, k, _)| k).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyInput;
    use reprisekit::controls::msgbox::Fields;
    use reprisekit::textedit::keysym;
    use reprisekit::theme::FontConfig;

    fn names(q: &Queue<&'static str>) -> Vec<&'static str> {
        q.iter().copied().collect()
    }

    #[test]
    fn a_raise_waits_behind_the_box_up() {
        let mut q = Queue::new();
        q.raise("a");
        q.raise("b");
        q.raise_held("c");
        assert_eq!(q.up(), Some(&"a"), "neither raise takes a's place");
        assert_eq!(names(&q), ["a", "c", "b"]);
    }

    #[test]
    fn a_held_raise_goes_behind_the_earlier_held_boxes_and_ahead_of_the_plain_ones() {
        let mut q = Queue::new();
        q.raise_held("up");
        q.raise("plain 1");
        q.raise_held("held 1");
        q.raise("plain 2");
        q.raise_held("held 2");
        assert_eq!(names(&q), ["up", "held 1", "held 2", "plain 1", "plain 2"]);
    }

    #[test]
    fn a_held_raise_on_an_empty_queue_is_up() {
        let mut q = Queue::new();
        q.raise_held("a");
        assert_eq!(q.up(), Some(&"a"));
    }

    #[test]
    fn close_takes_the_box_up_and_the_next_is_up() {
        let mut q = Queue::new();
        q.raise("a");
        q.raise("b");
        assert_eq!(q.close(), Some("a"));
        assert_eq!(q.up(), Some(&"b"));
        assert_eq!(q.close(), Some("b"));
        assert_eq!(q.close(), None);
        assert!(q.is_empty());
    }

    #[test]
    fn close_if_leaves_a_box_it_does_not_pick() {
        let mut q = Queue::new();
        q.raise("a");
        assert_eq!(q.close_if(|b| *b == "b"), None);
        assert_eq!(q.close_if(|b| *b == "a"), Some("a"));
    }

    #[test]
    fn withdraw_takes_the_picked_boxes_up_or_waiting() {
        let mut q = Queue::new();
        for b in ["job 1", "other", "job 2"] {
            q.raise(b);
        }
        assert_eq!(q.withdraw(|b| b.starts_with("job")), ["job 1", "job 2"]);
        assert_eq!(names(&q), ["other"]);
        assert_eq!(q.withdraw(|b| *b == "gone"), Vec::<&str>::new());
    }

    #[test]
    fn drain_takes_every_box_up_first() {
        let mut q = Queue::new();
        q.raise("a");
        q.raise("b");
        q.raise_held("c");
        assert_eq!(q.drain(), ["a", "c", "b"]);
        assert!(q.is_empty());
    }

    #[test]
    fn a_restored_box_is_up_again_in_its_window_ahead_of_one_raised_meanwhile() {
        let mut q = Queue::new();
        q.raise("picker");
        q.raise("waiting");
        let turn = q.turn();
        let b = q.close().unwrap();
        q.raise("raised meanwhile");
        q.restore(b);
        assert_eq!(names(&q), ["picker", "waiting", "raised meanwhile"]);
        assert_eq!(q.turn(), turn, "the picker keeps its window");
    }

    #[test]
    fn the_box_after_one_that_left_shows_in_the_other_window() {
        let mut q = Queue::new();
        q.raise("a");
        q.raise("b");
        q.raise("c");
        let first = q.turn();
        q.close();
        assert_ne!(q.turn(), first, "after close");
        let second = q.turn();
        q.withdraw(|b| *b == "b");
        assert_ne!(q.turn(), second, "after a withdraw of the box up");
        let third = q.turn();
        q.raise("d");
        q.withdraw(|b| *b == "d");
        assert_eq!(
            q.turn(),
            third,
            "a waiting box's withdraw leaves the box up where it is"
        );
    }

    fn ok_box(text: &str) -> MsgBox {
        MsgBox::new(
            "T",
            Fields {
                text: text.into(),
                icon: None,
                buttons: vec!["Yes".into(), "No".into()],
                default: 0,
                min_text_h: None,
                min_w: 0,
                max_w: 0,
            },
        )
    }

    fn enter() -> Event {
        Event::Key(KeyInput {
            sym: keysym::RETURN,
            utf8: None,
            mods: Default::default(),
            edit: None,
        })
    }

    #[test]
    fn an_answer_carries_the_payload_with_the_button_or_none_on_a_close_request() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut a = Asks::new(WinId::named("ask"));
        a.ask(ok_box("first"), 1);
        a.ask(ok_box("second"), 2);
        let win = a.win().unwrap();
        assert_eq!(
            a.event(win, &enter(), &text, &theme),
            Answer::Closed(1, Some(0))
        );
        let win = a.win().unwrap();
        assert_eq!(
            a.event(win, &Event::CloseRequest, &text, &theme),
            Answer::Closed(2, None)
        );
        assert!(a.is_empty());
    }

    #[test]
    fn two_boxes_in_a_row_show_in_different_windows() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut a = Asks::new(WinId::named("ask"));
        a.ask(ok_box("first"), ());
        a.ask(ok_box("second"), ());
        let first = a.child(&text, &theme).unwrap().id;
        a.event(first, &enter(), &text, &theme);
        let second = a.child(&text, &theme).unwrap().id;
        assert_ne!(first, second);
        assert!(a.owns(first) && a.owns(second));
        assert_eq!(
            a.event(first, &Event::CloseRequest, &text, &theme),
            Answer::None,
            "the first box's window is gone, and its late events with it"
        );
        assert_eq!(a.up().map(|(b, _)| b.fields.text.as_str()), Some("second"));
    }

    #[test]
    fn boxes_over_a_root_name_it_as_their_parent_and_keep_apart_from_another_roots() {
        let (one, two) = (crate::window_id(1), crate::window_id(2));
        let a: Asks<()> = Asks::new(one.named_in("confirm")).over(one);
        let b: Asks<()> = Asks::new(two.named_in("confirm")).over(two);
        let win = one.named_in("confirm");
        assert_eq!(a.parent(win), Some(one));
        assert_eq!(a.parent(MAIN), None);
        assert!(!b.owns(win));
        assert_eq!(
            Asks::<()>::new(WinId::named("x")).parent(WinId::named("x")),
            Some(MAIN)
        );
    }

    #[test]
    fn a_default_asks_has_windows_of_its_own_over_main() {
        #[derive(Default)]
        struct Program {
            asks: Asks<u8>,
        }
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let mut p = Program::default();
        p.asks.ask(ok_box("question"), 7);
        let win = p.asks.child(&text, &theme).unwrap().id;
        assert_eq!(p.asks.parent(win), Some(MAIN));
        let errors = crate::oneshot::ErrorBox::default();
        for w in p.asks.wins {
            assert!(
                !errors.owns(w) && !crate::filebox::owns(w),
                "apart from the error and file dialog boxes"
            );
        }
        assert_eq!(
            p.asks.event(win, &enter(), &text, &theme),
            Answer::Closed(7, Some(0))
        );
    }

    #[test]
    fn a_box_raised_over_a_window_names_it_and_the_next_box_its_asks_parent() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let (main, sheet) = (crate::window_id(1), crate::window_id(2));
        let mut a = Asks::new(WinId::named("ask")).over(main);
        a.ask_over(sheet, ok_box("about the sheet"), 1);
        a.ask(ok_box("about the program"), 2);
        let first = a.win().unwrap();
        assert_eq!(a.parent(first), Some(sheet));
        a.event(first, &enter(), &text, &theme);
        let second = a.win().unwrap();
        assert_eq!(a.parent(second), Some(main));
    }
}
