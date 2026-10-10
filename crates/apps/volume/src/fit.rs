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
    statusbar::height(&theme.look) - 1
}

/// The mixer body's size for `m` under `theme` on an output `room` wide (unknown: as wide as the columns want).
pub fn body(m: &Mixer, theme: &Theme, room: Option<i32>) -> (i32, i32) {
    let (w, h) = m.size();
    let frame = 2 * theme.metrics.dialog_frame_width;
    match room {
        Some(room) if w + frame > room => (
            (room - frame).max(mixer::MIN_W),
            h + theme.look.controls().scroll,
        ),
        _ => (w, h),
    }
}

pub fn window(m: &Mixer, theme: &Theme, room: Option<i32>) -> (i32, i32) {
    let (w, h) = body(m, theme, room);
    (w, menubar::BAR_H + h + status_h(theme))
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
        assert_eq!(body(&m, &theme, Some(4000)), m.size(), "room for all");
    }
}
