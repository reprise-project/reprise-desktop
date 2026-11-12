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

/// Where `t` lies on `l`, design px within the output: the fields, the notes and the badge it
/// touched.
pub fn logon_touched(m: &Logon, l: &LogonLayout, theme: &Theme, t: Touched) -> Vec<Rect> {
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
/// filled for a model that differs from `m` only there; the device rect repainted.
pub fn redraw_logon_within(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    (theme, text, s): (&Theme, &TextRenderer, i32),
    rects: &[Rect],
) -> Rect {
    let o = l.output;
    let mut all = Rect::default();
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
        all = if all.is_empty() { r } else { all.union(&r) };
    }
    all
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

        /// `c`, drawn whole for `l`, brought up to `m` by repainting what `touched` names.
        fn within(&self, c: &mut Canvas, m: &Logon, l: &LogonLayout, touched: Touched) -> Rect {
            let rects = logon_touched(m, l, &self.theme, touched);
            let s = self.s;
            redraw_logon_within(c, m, l, (&self.theme, self.text.for_output(s), s), &rects)
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
                    l = now;
                    let r = screen.within(&mut c, &m, &l, touched.unwrap());
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
        type Step = (&'static str, fn(&mut Logon));
        let steps: [Step; 12] = [
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
                        l = now;
                        c = screen.whole(&m, &l);
                        continue;
                    }
                    screen.within(&mut c, &m, &l, touched.unwrap());
                    let full = screen.whole(&m, &l);
                    assert!(
                        c.pixels == full.pixels,
                        "{what} at {s}: after {step} the screen differs from a whole draw in {:?}",
                        c.differing_rect(&full)
                    );
                }
            }
        }
        let caps = [
            "caps on",
            "caps off",
            "caps on over the hint",
            "caps off under it",
        ];
        let heading_grows =
            [1, 2].map(|s| caps.map(|step| format!("Classic admin at {s}: {step}")));
        assert_eq!(
            relaid,
            heading_grows.concat(),
            "only Caps Lock beside Classic's wrapped heading moves the dialog"
        );
    }
}
