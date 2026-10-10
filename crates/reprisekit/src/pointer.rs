//! Hot and held state for the parts of one surface. Rules: while a part is held nothing else is hot; a held part draws pressed while the pointer is on it and rests when dragged off; a sticky part (a thumb) stays pressed until release; a release acts only on the held part; leaving the surface clears the hot part, never the held one; keys never touch it.

/// Rows one wheel notch scrolls; hosts report the wheel in rows, a control that steps by notches divides by this.
pub const WHEEL_ROWS: i32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nav {
    Back,
    Forward,
}

/// The history step a pointer button (evdev code) stands for: the thumb buttons, whichever pair the mouse reports; `None` for any other.
pub fn nav_button(button: u32) -> Option<Nav> {
    match button {
        0x113 | 0x116 => Some(Nav::Back),
        0x114 | 0x115 => Some(Nav::Forward),
        _ => None,
    }
}

/// What the pointer does to one widget: nothing, rests on it, or holds it down while over it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Interaction {
    #[default]
    Idle,
    Hot,
    Pressed,
}

pub type Lit<P> = Option<(P, Interaction)>;

pub fn lit_of<P: PartialEq>(lit: Lit<P>, part: P) -> Interaction {
    match lit {
        Some((p, how)) if p == part => how,
        _ => Interaction::Idle,
    }
}

/// Hot and held tracking for one surface's parts `P`; the host hands it what the pointer is over on every motion, press and release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pointer<P> {
    hot: Option<P>,
    held: Option<Held<P>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held<P> {
    part: P,
    over: bool,
    sticky: bool,
}

impl Interaction {
    pub fn pressed_if(held: bool) -> Interaction {
        if held {
            Interaction::Pressed
        } else {
            Interaction::Idle
        }
    }
}

impl<P> Default for Pointer<P> {
    fn default() -> Self {
        Pointer {
            hot: None,
            held: None,
        }
    }
}

impl<P: Copy + Eq> Pointer<P> {
    pub fn new() -> Self {
        Self::default()
    }

    /// The pointer moved onto `over` (`None`: no part). True when what draws changed.
    pub fn motion(&mut self, over: Option<P>) -> bool {
        let before = self.lit();
        match &mut self.held {
            Some(h) => h.over = over == Some(h.part),
            None => self.hot = over,
        }
        self.lit() != before
    }

    /// A press on `over`; `None` holds nothing. A `sticky` part stays pressed until release wherever the pointer goes.
    pub fn press(&mut self, over: Option<P>, sticky: bool) -> bool {
        let before = self.lit();
        self.hot = None;
        self.held = over.map(|part| Held {
            part,
            over: true,
            sticky,
        });
        if self.held.is_none() {
            self.hot = over;
        }
        self.lit() != before
    }

    /// The held part when the pointer let go on it, the one that acts; it turns hot, as the pointer still rests there.
    pub fn release(&mut self) -> Option<P> {
        let h = self.held.take()?;
        if h.over {
            self.hot = Some(h.part);
            Some(h.part)
        } else {
            None
        }
    }

    /// The hot part goes cold; a held part stays held.
    pub fn leave(&mut self) -> bool {
        let before = self.lit();
        self.hot = None;
        self.lit() != before
    }

    pub fn clear(&mut self) -> bool {
        let before = self.lit();
        *self = Pointer::default();
        self.lit() != before
    }

    /// Take over what `self` holds onto a rebuilt surface: `remap` turns the old part into the part that means the same there, or `None` when it no longer exists.
    pub fn carry<Q: Copy + Eq>(self, remap: impl Fn(P) -> Option<Q>) -> Pointer<Q> {
        Pointer {
            hot: self.hot.and_then(&remap),
            held: self.held.and_then(|h| {
                Some(Held {
                    part: remap(h.part)?,
                    over: h.over,
                    sticky: h.sticky,
                })
            }),
        }
    }

    pub fn holding(&self) -> bool {
        self.held.is_some()
    }

    pub fn held(&self) -> Option<P> {
        self.held.map(|h| h.part)
    }

    pub fn hot(&self) -> Option<P> {
        self.hot
    }

    pub fn of(&self, part: P) -> Interaction {
        lit_of(self.lit(), part)
    }

    pub fn lit(&self) -> Lit<P> {
        match self.held {
            Some(h) if h.over || h.sticky => Some((h.part, Interaction::Pressed)),
            Some(_) => None,
            None => self.hot.map(|p| (p, Interaction::Hot)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Vertical,
    Horizontal,
}

/// What a scrollbar has at a point. A bar with no thumb still reports its arrows; a disabled bar's callers ignore them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollHit {
    Back,
    Forward,
    PageBack,
    PageForward,
    Thumb,
}

impl Pointer<(Axis, ScrollHit)> {
    /// A thumb stays held wherever the pointer goes until the release.
    pub fn press_bar(&mut self, over: Option<(Axis, ScrollHit)>) -> bool {
        let thumb = matches!(over, Some((_, ScrollHit::Thumb)));
        self.press(over, thumb)
    }

    /// The held part lets go and turns hot if the pointer is still on it.
    pub fn release_bar(&mut self) -> bool {
        let before = self.lit();
        self.release();
        self.lit() != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Interaction::{Hot, Idle, Pressed};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Part {
        A,
        B,
        Thumb,
    }
    use Part::{A, B, Thumb};

    #[test]
    fn a_bare_motion_makes_the_part_under_it_hot_and_only_that_part() {
        let mut p = Pointer::new();
        assert!(p.motion(Some(A)), "A turning hot is a change");
        assert_eq!((p.of(A), p.of(B)), (Hot, Idle));
        assert!(!p.motion(Some(A)), "a move within A is no change");
        assert!(p.motion(Some(B)));
        assert_eq!((p.of(A), p.of(B)), (Idle, Hot));
        assert!(p.motion(None));
        assert_eq!(p.lit(), None);
    }

    #[test]
    fn a_press_holds_the_part_and_nothing_else_turns_hot_until_the_release() {
        let mut p = Pointer::new();
        p.motion(Some(A));
        assert!(p.press(Some(A), false), "hot to pressed is a change");
        assert_eq!(p.lit(), Some((A, Pressed)));
        assert!(p.holding());
        p.motion(Some(B));
        assert_eq!(p.of(B), Idle, "B stays cold while A is held");
        assert_eq!(p.hot(), None);
    }

    #[test]
    fn a_held_part_dragged_off_draws_idle_and_pressed_again_back_on() {
        let mut p = Pointer::new();
        p.press(Some(A), false);
        assert!(p.motion(Some(B)), "dragging off A is a change");
        assert_eq!((p.of(A), p.of(B)), (Idle, Idle));
        assert!(!p.motion(None), "still off A: no change");
        assert!(p.motion(Some(A)), "back on A is a change");
        assert_eq!(p.of(A), Pressed);
    }

    #[test]
    fn a_release_over_the_held_part_acts_and_leaves_it_hot() {
        let mut p = Pointer::new();
        p.press(Some(A), false);
        assert_eq!(p.release(), Some(A));
        assert!(!p.holding());
        assert_eq!(p.of(A), Hot, "the pointer still rests on A");
    }

    #[test]
    fn a_release_off_the_held_part_acts_on_nothing_and_leaves_nothing_hot() {
        let mut p = Pointer::new();
        p.press(Some(A), false);
        p.motion(Some(B));
        assert_eq!(p.release(), None);
        assert!(!p.holding());
        assert_eq!(p.lit(), None, "B turns hot only on the next motion");
        p.motion(Some(B));
        assert_eq!(p.of(B), Hot);
    }

    #[test]
    fn a_press_on_no_part_holds_nothing_and_release_acts_on_nothing() {
        let mut p: Pointer<Part> = Pointer::new();
        assert!(!p.press(None, false));
        assert!(!p.holding());
        assert_eq!(p.release(), None);
    }

    #[test]
    fn a_leave_clears_the_hot_part_but_never_the_held_one() {
        let mut p = Pointer::new();
        p.motion(Some(A));
        assert!(p.leave(), "A going cold is a change");
        assert_eq!(p.lit(), None);
        assert!(!p.leave(), "nothing hot: no change");
        p.press(Some(B), false);
        assert!(!p.leave(), "a held part stays held");
        assert_eq!(p.of(B), Pressed);
        assert_eq!(p.release(), Some(B));
    }

    #[test]
    fn a_sticky_thumb_stays_pressed_off_it_until_the_release() {
        let mut p = Pointer::new();
        p.motion(Some(Thumb));
        assert!(p.press(Some(Thumb), true));
        assert!(
            !p.motion(Some(A)),
            "a sticky thumb off the pointer is no change"
        );
        assert_eq!((p.of(Thumb), p.of(A)), (Pressed, Idle));
        assert!(!p.motion(None));
        assert_eq!(p.of(Thumb), Pressed);
        assert_eq!(p.release(), None, "released off the thumb");
        assert_eq!(p.lit(), None);
    }

    #[test]
    fn lit_of_reads_one_part_out_of_a_lit_pair() {
        assert_eq!(lit_of(Some((A, Pressed)), A), Pressed);
        assert_eq!(lit_of(Some((A, Pressed)), B), Idle);
        assert_eq!(lit_of(None, A), Idle);
    }

    #[test]
    fn carry_remaps_a_held_or_hot_part_onto_the_rebuilt_surface() {
        let mut p = Pointer::new();
        p.press(Some(A), false);
        let carried = p.carry(|part| (part == A).then_some(B));
        assert_eq!(carried.lit(), Some((B, Pressed)));
        assert!(carried.holding());

        let mut hot = Pointer::new();
        hot.motion(Some(A));
        let carried = hot.carry(|part| (part == A).then_some(B));
        assert_eq!(carried.of(B), Hot);
    }

    #[test]
    fn carry_drops_a_part_the_rebuild_no_longer_offers() {
        let mut p = Pointer::new();
        p.press(Some(A), false);
        let carried: Pointer<Part> = p.carry(|_| None);
        assert_eq!(
            carried.lit(),
            None,
            "the held control is gone: nothing held"
        );
        assert!(!carried.holding());
    }

    #[test]
    fn carry_keeps_a_sticky_thumbs_drag_off_state() {
        let mut p = Pointer::new();
        p.press(Some(Thumb), true);
        p.motion(Some(A));
        assert_eq!(p.of(Thumb), Pressed, "sticky while dragged off");
        let mut carried = p.carry(Some);
        assert_eq!(carried.of(Thumb), Pressed, "carried over still sticky");
        assert_eq!(
            carried.release(),
            None,
            "release off the thumb still does nothing"
        );
    }
}
