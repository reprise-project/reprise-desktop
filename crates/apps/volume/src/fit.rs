//! The window's size. It does not resize by hand: as wide as its columns, as
//! tall as menu bar, mixer body and status line, re-fitted whenever a line comes
//! or goes. On an output too narrow for every column it takes the width left and
//! the body gets a scrollbar under the columns.

use reprisekit::controls::menubar;
use reprisekit::controls::mixer::{self, Mixer};
use reprisekit::controls::statusbar;
use reprisekit::theme::Theme;

/// The status line's height: the look's status bar.
pub fn status_h(theme: &Theme) -> i32 {
    statusbar::height(&theme.controls()) - 1
}

/// The mixer body's size for `m` under `theme` on an output `room` wide (unknown: as wide as the columns want).
pub fn body(m: &Mixer, theme: &Theme, room: Option<i32>) -> (i32, i32) {
    let (w, h) = m.size(theme.text);
    let frame = 2 * theme.metrics.dialog_frame_width;
    match room {
        Some(room) if w + frame > room => (
            (room - frame).max(mixer::min_width(theme.text)),
            h + theme.look.controls().scroll,
        ),
        _ => (w, h),
    }
}

pub fn window(m: &Mixer, theme: &Theme, room: Option<i32>) -> (i32, i32) {
    let (w, h) = body(m, theme, room);
    (w, menubar::bar_h(&theme.controls()) + h + status_h(theme))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reprisekit::controls::mixer::{Check, Line};

    fn line(key: &str, check: Check) -> Line {
        Line {
            key: key.into(),
            title: key.into(),
            volume: 0,
            balance: None,
            check,
            checked: false,
            live: true,
        }
    }

    #[test]
    fn the_window_fits_its_columns_the_menu_bar_and_the_status_line() {
        let theme = Theme::default();
        let lines = [
            line("out", Check::MuteAll),
            line("a", Check::Mute),
            line("b", Check::Mute),
            line("c", Check::Mute),
        ];
        let m = Mixer::new(&lines, false);
        assert_eq!(window(&m, &theme, None), (450, 20 + 225 + 21));
        assert_eq!(window(&Mixer::new(&lines, true), &theme, None).1, 293);
    }

    /// At 200% the menu bar, the mixer and the status line all grow with the text.
    #[test]
    fn at_200_percent_the_window_fits_the_grown_bars_and_columns() {
        let theme = Theme::default().with_text_size(200);
        let m = Mixer::new(
            &[line("out", Check::MuteAll), line("a", Check::Mute)],
            false,
        );
        let (w, h) = m.size(theme.text);
        assert_eq!((w, h), (2 * 135 + 5 + 2 * 100, 2 * 225));
        let bars = menubar::bar_h(&theme.controls()) + status_h(&theme);
        assert_eq!(bars, 40 + 43);
        assert_eq!(window(&m, &theme, None), (w, h + bars));
    }

    #[test]
    fn a_narrow_output_takes_a_scrollbar() {
        let theme = Theme::default();
        let lines: Vec<Line> = (0..20)
            .map(|i| line(&format!("l{i}"), Check::Mute))
            .collect();
        let m = Mixer::new(&lines, false);
        let (w, h) = body(&m, &theme, Some(1024));
        assert_eq!(w, 1024 - 2 * theme.metrics.dialog_frame_width);
        assert_eq!(h, 225 + theme.look.controls().scroll);
        assert_eq!(
            body(&m, &theme, Some(4000)),
            m.size(theme.text),
            "room for all"
        );
    }
}
