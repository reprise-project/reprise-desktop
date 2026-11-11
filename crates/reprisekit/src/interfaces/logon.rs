//! The login screen in the interface in force.

use crate::interface::Interface;
use crate::logon::{COLUMN, Logon, LogonLayout, Part};
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

/// [`draw_logon`] again inside the name and password fields alone, over a canvas it filled for a
/// model that differs from `m` only in what they hold; the device rect repainted.
pub fn redraw_logon_fields(
    c: &mut Canvas,
    m: &Logon,
    l: &LogonLayout,
    theme: &Theme,
    text: &TextRenderer,
    s: i32,
) -> Rect {
    let o = l.output;
    let mut all = Rect::default();
    for part in [Part::Name, Part::Password] {
        let Some(r) = l.rect(part) else { continue };
        let r = Rect::new(r.x - o.x, r.y - o.y, r.width, r.height).scaled(s);
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
                let theme = crate::interfaces::theme_for(m.interface).scaled(s);
                let unscaled = TextRenderer::load(&theme.fonts);
                let text = unscaled.for_output(s);
                let output = Rect::new(0, 0, 1280, 800);
                let laid = |m: &mut Logon| {
                    m.column = logon_column(m, &theme, output);
                    logon_layout(m, &theme, &unscaled, output)
                };
                let whole = |m: &Logon, l: &LogonLayout| {
                    let mut c = Canvas::new((1280 * s) as u32, (800 * s) as u32);
                    draw_logon(&mut c, m, l, &theme, text, s);
                    c
                };
                let mut l = laid(&mut m);
                let mut c = whole(&m, &l);
                for k in &keys {
                    let was = m.outside_fields();
                    m.key(k.clone());
                    let now = laid(&mut m);
                    assert!(
                        m.outside_fields() == was && now == l,
                        "{what} at {s}: {k:?} changes the fields alone"
                    );
                    l = now;
                    let r = redraw_logon_fields(&mut c, &m, &l, &theme, text, s);
                    let full = whole(&m, &l);
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
}
