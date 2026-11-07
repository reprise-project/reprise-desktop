//! The login screen in the interface in force. Every interface draws Classic's dialog until it
//! has its own.

use crate::classic;
use crate::interface::Interface;
use crate::logon::{Logon, LogonLayout};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;

pub fn logon_layout(m: &Logon, theme: &Theme, text: &TextRenderer, output: Rect) -> LogonLayout {
    match m.interface {
        Interface::Classic | Interface::Blue | Interface::Glass => {
            classic::logon::layout(m, theme, text, output)
        }
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
        Interface::Classic | Interface::Blue | Interface::Glass => {
            classic::logon::draw(c, m, l, theme, text, s)
        }
    }
}
