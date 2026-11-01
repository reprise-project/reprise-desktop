//! The Log Off and Turn off computer panels in three Blue schemes, at rest, with the pointer on a
//! button, pressed and with Shift held, written to `target/reprisekit-gallery-power@<scale>.png`.

use reprisekit::blue::Variant;
use reprisekit::cues::Cues;
use reprisekit::interfaces::draw_power_panel;
use reprisekit::pointer::Interaction;
use reprisekit::powerpanel::{Panel, Part, layout};
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::{Color, TextUnits, Theme};

const GAP: i32 = 10;

pub fn render(scale: i32) -> Canvas {
    let theme = Theme::default().scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let mut shifted = Panel::turn_off(false, true);
    shifted.set_shift(true);
    let mut cancel = Panel::log_off(false);
    cancel.focus = Part::Cancel;
    let cases = [
        (Panel::log_off(false), None, Cues::NONE),
        (
            Panel::log_off(false),
            Some((Part::Big(1), Interaction::Hot)),
            Cues::NONE,
        ),
        (cancel, None, Cues::ALL),
        (Panel::turn_off(false, true), None, Cues::NONE),
        (
            Panel::turn_off(true, true),
            Some((Part::Big(2), Interaction::Pressed)),
            Cues::NONE,
        ),
        (shifted, Some((Part::Big(0), Interaction::Hot)), Cues::ALL),
    ];
    let schemes = [Variant::Blue, Variant::Olive, Variant::Silver];
    let (w, h) = layout(&cases[0].0, TextUnits::ONE).size;
    let cols = cases.len() as i32;
    let rows = schemes.len() as i32;
    let mut c = Canvas::new(
        ((cols * (w + GAP) + GAP) * scale) as u32,
        ((rows * (h + GAP) + GAP) * scale) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x60, 0x60, 0x60));
    for (row, v) in schemes.into_iter().enumerate() {
        for (col, (p, lit, cues)) in cases.iter().enumerate() {
            let lay = layout(p, TextUnits::ONE);
            let mut panel = Canvas::new((w * scale) as u32, (h * scale) as u32);
            draw_power_panel(
                &mut panel,
                text.for_output(scale),
                v,
                p,
                &lay,
                *lit,
                *cues,
                theme.fonts.size_px,
                scale,
            );
            let at = |i: i32, n: i32| (GAP + i * (n + GAP)) * scale;
            c.blit_canvas(at(col as i32, w), at(row as i32, h), &panel);
        }
    }
    c
}
