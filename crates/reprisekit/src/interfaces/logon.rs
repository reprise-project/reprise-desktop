//! The login screen in the interface in force.

use crate::interface::Interface;
use crate::logon::{COLUMN, Logon, LogonLayout};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use crate::{blue, classic, glass};

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
