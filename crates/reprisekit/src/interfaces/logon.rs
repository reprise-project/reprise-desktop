//! The login screen in the interface in force.

use crate::interface::Interface;
use crate::logon::{COLUMN, Logon, LogonLayout};
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

/// The ring's step `busy_for` after the screen first showed busy, and when the next step is due.
pub fn spin_step(busy_for: std::time::Duration) -> (u32, std::time::Duration) {
    let step_ms = u64::from(glass::logon::SPIN_STEP_MS);
    let next = (busy_for.as_millis() as u64 / step_ms + 1) * step_ms;
    (
        glass::logon::spin_step(busy_for),
        std::time::Duration::from_millis(next),
    )
}
