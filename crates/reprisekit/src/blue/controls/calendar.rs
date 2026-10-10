//! The month calendar's weekday band, [`Part::CalendarBand`]: a flat fill in the scheme's inactive
//! title colour, one pixel stretched across the band, lettered in the scheme's `calendar_letters` ink
//! (registered as the part's text colour). A greyed calendar keeps the band and letters as they are, so
//! Disabled repeats Normal; only the day numbers grey.

use crate::blue::Variant;
use crate::raster::Canvas;
use crate::style::{Layout, Part, PartImage, State};

pub(super) fn parts(v: Variant) -> Vec<(Part, PartImage)> {
    let states = [State::Normal, State::Disabled];
    let mut a = Canvas::new(1, states.len() as u32);
    a.fill_rect(a.bounds(), v.scheme().system.inactive_title);
    vec![(
        Part::CalendarBand,
        PartImage::new(a, &states, Layout::Vertical),
    )]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::widgets::draw_calendar_band;
    use crate::raster::Rect;

    /// Each variant's band fills with its scheme's inactive title colour and letters in its calendar ink, in use and greyed.
    #[test]
    fn every_variant_band_takes_its_inactive_title_colours() {
        for v in Variant::ALL {
            let theme = crate::interfaces::blue_theme(v);
            for enabled in [true, false] {
                let mut c = Canvas::new(40, 20);
                let ink =
                    draw_calendar_band(&mut c, &theme.paint(), Rect::new(2, 1, 36, 18), enabled);
                assert_eq!(
                    ink,
                    v.scheme().inks.calendar_letters,
                    "{v:?} letters, {enabled}"
                );
                for (x, y) in [(2, 1), (37, 18), (20, 10)] {
                    assert_eq!(
                        c.get(x, y),
                        v.scheme().system.inactive_title.argb(),
                        "{v:?} band at ({x}, {y}), {enabled}"
                    );
                }
            }
        }
    }

    #[test]
    fn every_variant_band_at_paint_scale_2_doubles_every_pixel() {
        for v in Variant::ALL {
            let draw = |s: i32| {
                let mut theme = crate::interfaces::blue_theme(v);
                theme.scale = s;
                let mut c = Canvas::new((34 * s) as u32, (22 * s) as u32);
                let r = Rect::new(2, 2, 30, 18).scaled(s);
                draw_calendar_band(&mut c, &theme.paint(), r, true);
                c
            };
            assert!(
                crate::look::styled::fixture::doubles_at_scale_1(&draw(1), &draw(2)),
                "{v:?}'s band at s = 2 is its s = 1 pixels doubled"
            );
        }
    }
}
