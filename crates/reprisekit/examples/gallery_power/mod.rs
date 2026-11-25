//! Log Off and Turn off panels in three Blue schemes and with Classic's Cancel: at rest, hot,
//! pressed, Shift held, offering Disconnect, offering Restart Into... with the focus, with the
//! Install updates check box, and with both. Writes `target/reprisekit-gallery-power@<scale>.png`.

use reprisekit::blue::Variant;
use reprisekit::cues::Cues;
use reprisekit::interfaces::draw_power_panel;
use reprisekit::pointer::Interaction;
use reprisekit::powerpanel::{First, Leave, Panel, Part, layout};
use reprisekit::raster::Canvas;
use reprisekit::text::TextRenderer;
use reprisekit::theme::Color;

const GAP: i32 = 10;

pub fn render(scale: i32) -> Canvas {
    let theme = crate::interfaces::classic().scaled(scale);
    let text = TextRenderer::load(&theme.fonts);
    let mut shifted = Panel::turn_off(First::StandBy {
        live: false,
        hibernate: true,
    });
    shifted.set_shift(true);
    let mut cancel = Panel::log_off(Leave::SwitchUser { live: false });
    cancel.focus = Part::Cancel;
    let mut restart_into = Panel::turn_off(First::StandBy {
        live: true,
        hibernate: false,
    });
    restart_into.restart_into = true;
    restart_into.focus = Part::RestartInto;
    let mut updates = Panel::turn_off(First::StandBy {
        live: true,
        hibernate: false,
    });
    updates.offer_updates(true, true);
    let mut unticked = updates.clone();
    unticked.toggle_updates();
    let mut both = updates.clone();
    both.restart_into = true;
    let cases = [
        (
            Panel::log_off(Leave::SwitchUser { live: false }),
            None,
            Cues::NONE,
        ),
        (
            Panel::log_off(Leave::SwitchUser { live: false }),
            Some((Part::Big(1), Interaction::Hot)),
            Cues::NONE,
        ),
        (cancel, None, Cues::ALL),
        (
            Panel::turn_off(First::StandBy {
                live: false,
                hibernate: true,
            }),
            None,
            Cues::NONE,
        ),
        (
            Panel::turn_off(First::StandBy {
                live: true,
                hibernate: true,
            }),
            Some((Part::Big(2), Interaction::Pressed)),
            Cues::NONE,
        ),
        (shifted, Some((Part::Big(0), Interaction::Hot)), Cues::ALL),
        (Panel::log_off(Leave::Disconnect), None, Cues::ALL),
        (Panel::turn_off(First::Disconnect), None, Cues::ALL),
        (restart_into, None, Cues::ALL),
        (updates, Some((Part::Updates, Interaction::Hot)), Cues::NONE),
        (unticked, None, Cues::ALL),
        (both, None, Cues::NONE),
    ];
    let classic = theme.paint();
    let schemes = [
        (Variant::Blue, None),
        (Variant::Olive, None),
        (Variant::Silver, None),
        (Variant::Blue, Some(&classic)),
    ];
    let (w, h) = layout(&cases[0].0, theme.text).size;
    let cols = cases.len() as i32;
    let rows = schemes.len() as i32;
    let mut c = Canvas::new(
        ((cols * (w + GAP) + GAP) * scale) as u32,
        ((rows * (h + GAP) + GAP) * scale) as u32,
    );
    c.fill_rect(c.bounds(), Color::rgb(0x60, 0x60, 0x60));
    for (row, (v, cancel)) in schemes.into_iter().enumerate() {
        for (col, (p, lit, cues)) in cases.iter().enumerate() {
            let lay = layout(p, theme.text);
            let mut panel = Canvas::new((w * scale) as u32, (h * scale) as u32);
            draw_power_panel(
                &mut panel,
                text.for_output(scale),
                v,
                cancel,
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
