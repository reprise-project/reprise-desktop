//! The login screen in the interface in force.

use crate::interface::Interface;
use crate::logon::{COLUMN, Logon, LogonLayout, Part, Touched};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use crate::{blue, classic, glass};

mod transition;
pub use transition::{Drawn, Transition};

pub fn logon_layout(m: &Logon, theme: &Theme, text: &TextRenderer, output: Rect) -> LogonLayout {
    match m.interface {
        Interface::Classic => classic::logon::layout(m, theme, text, output),
        Interface::Blue => blue::logon::layout(m, theme, text, output),
        Interface::Glass => glass::logon::layout(m, theme, text, output),
    }
}

pub fn draw_logon(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    match m.interface {
        Interface::Classic => classic::logon::draw(c, m, l, theme, text, s),
        Interface::Blue => blue::logon::draw(c, m, l, theme, text, s),
        Interface::Glass => glass::logon::draw(c, m, l, theme, text, s),
    }
}

/// What [`redraw_logon_within`] repaints, design px within the output, to bring a canvas painted
/// for `was` ([`Logon::drawn`]) on `was_l` up to `m` on `l`; None when `m.motion` is to play.
pub fn logon_changed(
    m: &Logon,
    l: &LogonLayout,
    was: &Logon,
    was_l: &LogonLayout,
    theme: &Theme,
) -> Option<Vec<Rect>> {
    if let Some(t) = m.touched_since(was).filter(|_| l.output == was_l.output) {
        if l == was_l {
            return Some(logon_touched(m, l, theme, t));
        }
        let unbadged = |l: &LogonLayout| {
            let parts = l.parts.iter().filter(|(p, _)| *p != Part::Layout);
            (l.dialog, parts.copied().collect::<Vec<_>>())
        };
        if unbadged(l) == unbadged(was_l) {
            let mut rects = logon_touched(m, l, theme, t);
            rects.extend(logon_touched(m, was_l, theme, t));
            return Some(rects);
        }
    }
    m.motion
        .as_ref()
        .map_or_else(|| logon_region(m, l, was, was_l, theme), |_| None)
}

/// Where screens drawn for `was` on `was_l` and for `m` on `l` can differ, design px within the
/// output; None when their backdrops differ (interface, output, picture under the screen).
pub fn logon_region(
    m: &Logon,
    l: &LogonLayout,
    was: &Logon,
    was_l: &LogonLayout,
    theme: &Theme,
) -> Option<Vec<Rect>> {
    let backdrop = |m: &Logon, l: &LogonLayout| (m.interface, m.over_picture, l.output);
    if backdrop(m, l) != backdrop(was, was_l) {
        return None;
    }
    let mut rects = logon_live(was, was_l, theme);
    rects.extend(logon_live(m, l, theme));
    if l.dialog != was_l.dialog {
        // A dialog that moves takes all it draws along.
        rects.extend([was_l.dialog, l.dialog].into_iter().flatten());
    }
    Some(joined(rects))
}

/// Where the interface in force paints from `m`, design px within the output; the rest of the
/// screen is its backdrop, its marks and the frame of a dialog it lays out.
fn logon_live(m: &Logon, l: &LogonLayout, theme: &Theme) -> Vec<Rect> {
    match m.interface {
        Interface::Classic => classic::logon::live(theme, l),
        Interface::Blue => blue::logon::live(m, l, theme),
        Interface::Glass => glass::logon::live(m, l, theme),
    }
}

/// `rects` with every two that meet made one, so no pixel is painted twice.
fn joined(rects: Vec<Rect>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    for mut r in rects.into_iter().filter(|r| !r.is_empty()) {
        while let Some(i) = out.iter().position(|o| o.intersects(r)) {
            r = r.union(&out.swap_remove(i));
        }
        out.push(r);
    }
    out
}

/// Where `t` lies on `l`, design px within the output: the fields, the notes and the badge it
/// touched.
fn logon_touched(m: &Logon, l: &LogonLayout, theme: &Theme, t: Touched) -> Vec<Rect> {
    let notes = match m.interface {
        Interface::Classic => classic::logon::notes(theme, l),
        Interface::Blue => blue::logon::notes(m, l, theme),
        Interface::Glass => glass::logon::notes(m, l, theme),
    };
    let fields = [Part::Name, Part::Password].map(|p| l.rect(p).filter(|_| t.fields));
    (fields.into_iter())
        .chain([
            notes.said.filter(|_| t.said),
            notes.badge.filter(|_| t.badge),
        ])
        .flatten()
        .collect()
}

/// [`draw_logon`] again inside `rects` (design px within the output) alone, over a canvas it
/// filled for a model that differs from `m` only there; the device rects repainted.
pub fn redraw_logon_within(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    (theme, text, s): (&Theme, &TextRenderer, i32),
    rects: &[Rect],
) -> Vec<Rect> {
    let o = l.output;
    let mut drawn = Vec::new();
    for r in rects {
        let r = Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
        let r = r.intersect(&c.bounds());
        if r.is_empty() {
            continue;
        }
        c.with_clip(r, |c| {
            c.clear_rect(r);
            draw_logon(c, m, l, theme, text, s);
        });
        drawn.push(r);
    }
    drawn
}

/// How many tiles the interface in force shows across `output`: the host's [`Logon::column`].
pub fn logon_column(m: &Logon, theme: &Theme, output: Rect) -> usize {
    match m.interface {
        Interface::Classic => COLUMN,
        Interface::Blue => blue::logon::column(theme, output),
        Interface::Glass => glass::logon::column(theme, output),
    }
}

/// The busy ring's device rect on the canvas [`draw_logon`] fills; None while not busy or in an
/// interface without one.
pub fn spinner_rect(
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) -> Option<Rect> {
    match m.interface {
        Interface::Glass => glass::logon::spinner_rect(m, l, theme, text, s),
        _ => None,
    }
}

/// The busy ring alone drawn again at `m.spin` over a canvas [`draw_logon`] filled.
pub fn draw_spinner(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) {
    if m.interface == Interface::Glass {
        glass::logon::draw_spinner(c, m, l, theme, text, s);
    }
}

/// How long the ring takes to turn from step 0 to `step`.
pub fn spin_elapsed(step: u32) -> std::time::Duration {
    std::time::Duration::from_millis(u64::from(step) * u64::from(glass::logon::SPIN_STEP_MS))
}

/// The ring's step `busy_for` after the screen first showed busy, and when the next step is due.
pub fn spin_step(busy_for: std::time::Duration) -> (u32, std::time::Duration) {
    let step_ms = u64::from(glass::logon::SPIN_STEP_MS);
    let next = (busy_for.as_millis() as u64 / step_ms + 1) * step_ms;
    (
        glass::logon::spin_step(busy_for),
        std::time::Duration::from_millis(next),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::userpic;
    use crate::logon::{LogonKey, LogonUser};
    use crate::pointer::Interaction;
    use crate::textedit::EditKey;

    #[test]
    fn the_ring_turned_for_a_steps_time_stands_at_that_step() {
        for step in 0..crate::glass::logon::SPIN_STEPS {
            assert_eq!(super::spin_step(super::spin_elapsed(step)).0, step);
        }
    }

    fn users() -> Vec<LogonUser> {
        (0..2)
            .map(|i| LogonUser {
                name: format!("user{i}"),
                real_name: String::new(),
                picture: Some(userpic::draw(userpic::ALL[i], userpic::SAVED_PX)),
                note: None,
                stock: i,
            })
            .collect()
    }

    /// Each interface's screens with a field focused: the login screen typed and on a tile, and
    /// the lock.
    fn on_a_field() -> Vec<(String, Logon)> {
        let mut out = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let typed = Logon::new(i, users(), false, Some("user0"));
            let mut name = typed.clone();
            name.name = crate::textedit::TextEdit::new(String::new(), false);
            name.focus = Part::Name;
            let mut tile = Logon::new(i, users(), true, Some("user0"));
            if i == Interface::Blue {
                tile.click(Part::Tile(0));
            }
            let locked = Logon::locked(i, users().remove(1), true, true);
            for (what, mut m) in [
                ("typed", typed),
                ("name", name),
                ("tile", tile),
                ("locked", locked),
            ] {
                m.take_motion();
                out.push((format!("{i:?} {what}"), m));
            }
        }
        out
    }

    fn edit(k: EditKey) -> LogonKey<'static> {
        LogonKey::Edit(k)
    }

    /// What lays out and draws a screen over a 1280x800 output at scale `s`.
    struct Bench {
        s: i32,
        theme: Theme,
        text: TextRenderer,
        output: Rect,
    }

    impl Bench {
        fn at(i: Interface, s: i32) -> Bench {
            let theme = crate::interfaces::theme_for(i).scaled(s);
            let text = TextRenderer::load(&theme.fonts);
            let output = Rect::new(0, 0, 1280, 800);
            Bench {
                s,
                theme,
                text,
                output,
            }
        }

        fn laid(&self, m: &mut Logon) -> LogonLayout {
            m.column = logon_column(m, &self.theme, self.output);
            logon_layout(m, &self.theme, &self.text, self.output)
        }

        fn whole(&self, m: &Logon, l: &LogonLayout) -> Canvas {
            let s = self.s;
            let mut c = Canvas::new((1280 * s) as u32, (800 * s) as u32);
            draw_logon(&mut c, m, l, &self.theme, self.text.for_output(s), s);
            c
        }

        /// `c`, drawn whole for `was` on `was_l`, brought up to `m` on `l` by repainting what
        /// [`logon_changed`] names; None when only a whole draw will do.
        fn within(
            &self,
            c: &mut Canvas,
            (m, l): (&Logon, &LogonLayout),
            (was, was_l): (&Logon, &LogonLayout),
        ) -> Option<Vec<Rect>> {
            let rects = logon_changed(m, l, was, was_l, &self.theme)?;
            let s = self.s;
            let drawn = (&self.theme, self.text.for_output(s), s);
            Some(redraw_logon_within(c, m, l, drawn, &rects))
        }
    }

    #[test]
    fn redrawing_the_fields_alone_leaves_the_screen_a_whole_draw_draws() {
        let keys = [
            LogonKey::Text("a"),
            LogonKey::Text("bc"),
            edit(EditKey::Left {
                select: false,
                word: false,
            }),
            LogonKey::Text("d"),
            edit(EditKey::Home { select: true }),
            LogonKey::Text("e"),
            LogonKey::Text("a password long enough to run past the end of any field"),
            edit(EditKey::Home { select: false }),
            edit(EditKey::End { select: false }),
            edit(EditKey::Backspace { word: true }),
            edit(EditKey::SelectAll),
            edit(EditKey::Backspace { word: false }),
        ];
        for s in [1, 2] {
            for (what, mut m) in on_a_field() {
                let screen = Bench::at(m.interface, s);
                let mut l = screen.laid(&mut m);
                let mut c = screen.whole(&m, &l);
                for k in &keys {
                    let was = m.drawn();
                    m.key(k.clone());
                    let now = screen.laid(&mut m);
                    let touched = m.touched_since(&was);
                    assert!(
                        touched.is_some_and(|t| !t.said && !t.badge) && now == l,
                        "{what} at {s}: {k:?} changes the fields alone"
                    );
                    let r = screen.within(&mut c, (&m, &now), (&was, &l)).unwrap();
                    l = now;
                    let full = screen.whole(&m, &l);
                    assert!(!r.is_empty(), "{what} at {s}: a field is repainted");
                    assert!(
                        c.pixels == full.pixels,
                        "{what} at {s}: after {k:?} the screen differs from a whole draw in {:?}",
                        c.differing_rect(&full)
                    );
                }
            }
        }
    }

    #[test]
    fn redrawing_the_notes_alone_leaves_the_screen_a_whole_draw_draws() {
        const LONG: &str = "The login service did not answer in time, so nothing was checked. \
                            Try again in a moment, or ask whoever looks after this computer.";
        let steps: [Step; 16] = [
            ("caps on", |m| m.caps_lock = true),
            ("message", |m| m.info(crate::logon::text::incorrect())),
            ("caps off", |m| m.caps_lock = false),
            ("hint", |m| {
                m.hint = Some("Place a finger on the reader".into())
            }),
            ("message cleared", |m| m.message = None),
            ("caps on over the hint", |m| m.caps_lock = true),
            ("badge", |m| m.layout = Some("HU".into())),
            ("hint cleared", |m| m.hint = None),
            ("long message", |m| m.info(LONG.into())),
            ("caps off under it", |m| m.caps_lock = false),
            ("message cleared again", |m| m.message = None),
            ("badge back", |m| m.layout = Some("EN".into())),
            ("badge gone", |m| m.layout = None),
            ("caps on without the badge", |m| m.caps_lock = true),
            ("badge shown", |m| m.layout = Some("HU".into())),
            ("caps off beside the badge", |m| m.caps_lock = false),
        ];
        let mut screens = on_a_field();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            screens.push((format!("{i:?} listed"), Logon::new(i, users(), true, None)));
            let mut admin = Logon::new(i, users(), true, None);
            admin.admin(crate::logon::PowerAction::PowerOff);
            screens.push((format!("{i:?} admin"), admin));
        }
        let mut error = Logon::new(Interface::Glass, users(), true, Some("user0"));
        error.fail(crate::logon::text::failed());
        screens.push(("Glass error".into(), error));
        let mut relaid = Vec::new();
        for s in [1, 2] {
            for (what, mut m) in screens.clone() {
                m.layout = Some("EN".into());
                let screen = Bench::at(m.interface, s);
                let mut l = screen.laid(&mut m);
                let mut c = screen.whole(&m, &l);
                for (step, change) in steps {
                    let was = m.drawn();
                    change(&mut m);
                    let now = screen.laid(&mut m);
                    let touched = m.touched_since(&was);
                    assert!(
                        touched.is_some_and(|t| !t.fields),
                        "{what} at {s}: {step} changes the notes alone"
                    );
                    if now != l {
                        relaid.push(format!("{what} at {s}: {step}"));
                    }
                    let r = screen.within(&mut c, (&m, &now), (&was, &l));
                    assert!(r.is_some(), "{what} at {s}: {step} repaints a part");
                    l = now;
                    let full = screen.whole(&m, &l);
                    assert!(
                        c.pixels == full.pixels,
                        "{what} at {s}: after {step} the screen differs from a whole draw in {:?}",
                        c.differing_rect(&full)
                    );
                }
            }
        }
        for s in [1, 2] {
            let mut laid_again = vec![format!("Classic admin at {s}: caps on")];
            for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
                let badge =
                    ["badge gone", "badge shown"].map(|b| format!("{i:?} typed at {s}: {b}"));
                laid_again.extend(badge);
            }
            for step in laid_again {
                assert!(relaid.contains(&step), "{step} lays the screen out again");
            }
        }
    }

    type Step = (&'static str, fn(&mut Logon));

    /// Changes a login or lock goes through, from a check to the administrator's form.
    const LIVED: [Step; 20] = [
        ("busy", |m| m.set_busy(true)),
        ("ring turned", |m| m.spin = 7),
        ("failed", |m| m.fail(crate::logon::text::incorrect())),
        ("enter", |m| {
            m.key(LogonKey::Enter);
        }),
        ("power focused", |m| m.focus = Part::Power),
        ("power hot", |m| {
            m.lit = Some((Part::Power, Interaction::Hot))
        }),
        ("power pressed", |m| {
            m.lit = Some((Part::Power, Interaction::Pressed))
        }),
        ("unlit", |m| m.lit = None),
        ("tab", |m| {
            m.key(LogonKey::Tab { back: false });
        }),
        ("asked", |m| {
            m.ask(crate::logon::Prompt {
                text: "Token:".into(),
                echo: true,
            })
        }),
        ("answered", |m| {
            m.key(LogonKey::Text("42"));
        }),
        ("busy again", |m| m.set_busy(true)),
        ("idle", |m| m.set_busy(false)),
        ("escape", |m| {
            m.key(LogonKey::Escape);
        }),
        ("down", |m| {
            m.key(LogonKey::Down);
        }),
        ("opened", |m| {
            m.key(LogonKey::Enter);
        }),
        ("admin", |m| m.admin(crate::logon::PowerAction::PowerOff)),
        ("admin busy", |m| m.set_busy(true)),
        ("going down", |m| m.going_down()),
        ("admin failed", |m| {
            m.fail(crate::logon::text::power_failed())
        }),
    ];

    /// Each interface's screens to start [`LIVED`] from: those of [`on_a_field`], the listed
    /// users, and the typed screen over a picture, each with its badge.
    fn lived_from() -> Vec<(String, Logon)> {
        let mut screens = on_a_field();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            screens.push((format!("{i:?} listed"), Logon::new(i, users(), true, None)));
            let mut over = Logon::new(i, users(), false, Some("user0"));
            over.over_picture = true;
            screens.push((format!("{i:?} over a picture"), over));
        }
        for (_, m) in &mut screens {
            m.layout = Some("EN".into());
        }
        screens
    }

    #[test]
    fn any_change_repaints_where_either_screen_paints_and_leaves_the_screen_a_whole_draw_draws() {
        for s in [1, 2] {
            for (what, mut m) in lived_from() {
                let screen = Bench::at(m.interface, s);
                let mut l = screen.laid(&mut m);
                let mut c = screen.whole(&m, &l);
                for (step, change) in LIVED {
                    let was = m.drawn();
                    change(&mut m);
                    let now = screen.laid(&mut m);
                    let r = screen.within(&mut c, (&m, &now), (&was, &l));
                    assert!(r.is_some(), "{what} at {s}: {step} repaints a part");
                    l = now;
                    let full = screen.whole(&m, &l);
                    assert!(
                        c.pixels == full.pixels,
                        "{what} at {s}: after {step} the screen differs from a whole draw in {:?}",
                        c.differing_rect(&full)
                    );
                }
            }
        }
    }

    #[test]
    fn a_failed_login_on_classic_repaints_part_of_its_dialog() {
        for s in [1, 2] {
            for (what, mut m) in lived_from() {
                if m.interface != Interface::Classic {
                    continue;
                }
                m.set_busy(true);
                let screen = Bench::at(m.interface, s);
                let was_l = screen.laid(&mut m);
                let was = m.drawn();
                m.fail(crate::logon::text::incorrect());
                let l = screen.laid(&mut m);
                let rects = logon_changed(&m, &l, &was, &was_l, &screen.theme).unwrap();
                let dialog = l.dialog.unwrap();
                let area: i32 = rects.iter().map(|r| r.width * r.height).sum();
                assert!(
                    rects.iter().all(|r| r.intersect(&dialog) == *r),
                    "{what} at {s}: {rects:?} lie in {dialog:?}"
                );
                assert!(
                    area < dialog.width * dialog.height / 2,
                    "{what} at {s}: {area}"
                );
            }
        }
    }

    /// Changes that play from a screen at rest, from a check to a reopened screen.
    const MOVING: [Step; 5] = [
        ("busy", |m| m.set_busy(true)),
        ("failed", |m| m.fail(crate::logon::text::incorrect())),
        ("enter", |m| {
            m.key(LogonKey::Enter);
        }),
        ("escape", |m| {
            m.key(LogonKey::Escape);
        }),
        ("opened", |m| {
            m.key(LogonKey::Enter);
        }),
    ];

    /// From a screen at rest, Glass's frame flies back to the tiles, and each next change lands
    /// while the one before flies.
    const FLIGHTS: [Step; 4] = [
        ("at rest", |_| {}),
        ("back", |m| {
            m.key(LogonKey::Escape);
        }),
        ("opened mid-flight", |m| {
            m.key(LogonKey::Enter);
        }),
        ("back mid-flight", |m| {
            m.key(LogonKey::Escape);
        }),
    ];

    impl Bench {
        fn drawn<'a>(&'a self, m: &'a Logon, l: &'a LogonLayout) -> Drawn<'a> {
            Drawn {
                m,
                l,
                theme: &self.theme,
                text: &self.text,
                s: self.s,
            }
        }
    }

    /// `whole` and `within` give the same rect and the same frames over the canvases they play on.
    fn same_frames(
        at: &str,
        (whole, a): (&Transition, &Canvas),
        (within, b): (&Transition, &Canvas),
        m: &Logon,
    ) {
        assert_eq!(within.rect(), whole.rect(), "{at}: the same rect");
        for ms in [0, 40, 100, 160, 240, 400] {
            let (mut a, mut b) = (a.clone(), b.clone());
            whole.frame(&mut a, m, ms);
            within.frame(&mut b, m, ms);
            assert!(a.pixels == b.pixels, "{at}: the frame at {ms} ms");
        }
    }

    #[test]
    fn a_change_begun_within_its_region_plays_as_one_begun_from_whole_draws() {
        let (mut played, mut flown) = (0, 0);
        for s in [1, 2] {
            for (what, mut m) in lived_from() {
                m.moves = crate::logon::Moves::Full;
                let screen = Bench::at(m.interface, s);
                let mut l = screen.laid(&mut m);
                let mut c = screen.whole(&m, &l);
                for (step, change) in MOVING {
                    let was = m.drawn();
                    change(&mut m);
                    let now = screen.laid(&mut m);
                    let Some(motion) = m.take_motion() else {
                        l = now;
                        c = screen.whole(&m, &l);
                        continue;
                    };
                    let rects = logon_region(&m, &now, &was, &l, &screen.theme).unwrap();
                    let d = screen.drawn(&m, &now);
                    let at = format!("{what} at {s}: {step}");
                    let whole = Transition::begin_over(&motion, &d, Some(c.clone()));
                    let mut b = c.clone();
                    let (within, r) = Transition::begin_within(&motion, &d, &rects, &mut b);
                    let first = match &whole {
                        Some(t) => {
                            let mut first = c.clone();
                            let drawn = t.frame(&mut first, &m, 0);
                            assert_eq!(r, [drawn], "{at}: the rect drawn");
                            first
                        }
                        None => screen.whole(&m, &now),
                    };
                    assert!(first.pixels == b.pixels, "{at}: the first frame drawn");
                    let (whole, within) = match (whole, within) {
                        (Some(whole), Some(within)) => (whole, within),
                        (None, None) => {
                            l = now;
                            c = screen.whole(&m, &l);
                            continue;
                        }
                        _ => panic!("{at}: both play or neither"),
                    };
                    played += 1;
                    if within.carried_at(0).is_some() {
                        flown += 1;
                        assert!(
                            within.held().intersect(&b.bounds()) != b.bounds(),
                            "{at}: the flight holds less than the screen"
                        );
                    }
                    same_frames(&at, (&whole, &c), (&within, &b), &m);
                    within.settle(&mut b, &m);
                    l = now;
                    c = b;
                    let full = screen.whole(&m, &l);
                    assert!(
                        c.pixels == full.pixels,
                        "{at}: settled, the screen differs from a whole draw in {:?}",
                        c.differing_rect(&full)
                    );
                }
            }
        }
        assert!(played > 20, "{played} changes played");
        assert!(flown >= 8, "{flown} frames flew");
    }

    #[test]
    fn a_change_landing_while_another_plays_carries_on_within_its_region_as_from_whole_draws() {
        const LANDS_AT: u32 = 120;
        const TYPED_AT: u32 = 60;
        let (mut carried, mut refreshed, mut flown_on) = (0, 0, 0);
        for s in [1, 2] {
            for (what, mut m) in lived_from() {
                m.moves = crate::logon::Moves::Full;
                let screen = Bench::at(m.interface, s);
                let mut l = screen.laid(&mut m);
                // The canvases the frames draw into and what plays on them: drawn whole, and
                // within the region.
                let (mut a, mut b) = (screen.whole(&m, &l), screen.whole(&m, &l));
                let mut playing: Option<(Transition, Transition)> = None;
                for &(step, change) in MOVING.iter().chain(&FLIGHTS) {
                    let at = format!("{what} at {s}: {step}");
                    if let Some((whole, within)) = &mut playing {
                        let was = m.drawn();
                        m.caps_lock = !m.caps_lock;
                        let now = screen.laid(&mut m);
                        let rects = logon_changed(&m, &now, &was, &l, &screen.theme).unwrap();
                        let d = screen.drawn(&m, &now);
                        whole.frame(&mut a, &m, TYPED_AT);
                        whole.refresh_now(&d, &a);
                        within.frame(&mut b, &m, TYPED_AT);
                        within.refresh_within(&mut b, &d, &rects);
                        within.frame(&mut b, &m, TYPED_AT);
                        a = whole.rest(&m);
                        whole.frame(&mut a, &m, TYPED_AT);
                        let changed = a.differing_rect(&b);
                        assert!(
                            changed.is_empty(),
                            "{at}: Caps Lock typed, the frames differ in {changed:?}"
                        );
                        same_frames(
                            &format!("{at}, Caps Lock typed"),
                            (whole, &a),
                            (within, &b),
                            &m,
                        );
                        l = now;
                        refreshed += 1;
                    }
                    let was = m.drawn();
                    change(&mut m);
                    let now = screen.laid(&mut m);
                    let Some(motion) = m.take_motion() else {
                        if let Some((_, within)) = playing.take() {
                            within.settle(&mut b, &m);
                        }
                        l = now;
                        (a, b) = (screen.whole(&m, &l), screen.whole(&m, &l));
                        continue;
                    };
                    let rects = logon_region(&m, &now, &was, &l, &screen.theme).unwrap();
                    let d = screen.drawn(&m, &now);
                    let (whole, within) = match playing.take() {
                        Some((pw, pv)) => {
                            pw.frame(&mut a, &m, LANDS_AT - 16);
                            pv.frame(&mut b, &m, LANDS_AT - 16);
                            let whole = Transition::then(&pw, LANDS_AT, &motion, &d, a.clone());
                            let (within, _) =
                                Transition::then_within(&pv, LANDS_AT, &motion, &d, &rects, &mut b);
                            match &whole {
                                Some(t) => {
                                    t.frame(&mut a, &m, 0);
                                }
                                None => a = screen.whole(&m, &now),
                            }
                            carried += 1;
                            if pv.carried_at(0).is_some() {
                                flown_on += 1;
                            }
                            (whole, within)
                        }
                        None => {
                            let whole = Transition::begin_over(&motion, &d, Some(a.clone()));
                            let (within, _) = Transition::begin_within(&motion, &d, &rects, &mut b);
                            match &whole {
                                Some(t) => {
                                    t.frame(&mut a, &m, 0);
                                }
                                None => a = screen.whole(&m, &now),
                            }
                            (whole, within)
                        }
                    };
                    l = now;
                    let changed = a.differing_rect(&b);
                    assert!(
                        changed.is_empty(),
                        "{at}: the first frames differ in {changed:?}"
                    );
                    playing = match (whole, within) {
                        (Some(whole), Some(within)) => {
                            same_frames(&at, (&whole, &a), (&within, &b), &m);
                            Some((whole, within))
                        }
                        (None, None) => None,
                        _ => panic!("{at}: both play or neither"),
                    };
                }
                if let Some((_, within)) = playing {
                    within.settle(&mut b, &m);
                }
                let full = screen.whole(&m, &l);
                assert!(
                    b.pixels == full.pixels,
                    "{what} at {s}: settled, the screen differs from a whole draw in {:?}",
                    b.differing_rect(&full)
                );
            }
        }
        assert!(
            carried > 10 && refreshed > 10 && flown_on >= 4,
            "{carried} carried on, {refreshed} refreshed, {flown_on} landed on a flight"
        );
    }
}
