//! The notification area's network icons, 16x16 (and a 2x `draw32`), from the PNGs in [`icons`]:
//! wired, unplugged, wireless with signal bars, and a VPN padlock over any of them.

use crate::art::icons;
use crate::art::signalbars;
use crate::raster::Canvas;
use crate::theme::Color;

pub const SIZE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NetIcon {
    Wired,
    WiredUnplugged,
    Wireless(u8),
    WirelessOff,
    Connecting,
}

const SCREEN: Color = Color::rgb(20, 140, 140);
const GLINT: Color = Color::rgb(170, 230, 220);
const POWER: Color = Color::rgb(40, 200, 80);
const SCREEN_OFF: Color = Color::rgb(160, 160, 160);
const POWER_OFF: Color = Color::rgb(110, 110, 110);
const CROSS: Color = Color::rgb(0xff, 0, 0);
const BAR_ON: Color = Color::rgb(0, 128, 0);

const LOCK_X: i32 = 10;
const LOCK_Y: i32 = 9;

const BARS: [(i32, i32); 4] = [(9, 10), (11, 8), (13, 6), (15, 4)];
const BAR_BOTTOM: i32 = 11;

pub fn bars(strength: u8) -> u8 {
    signalbars::five(strength).saturating_sub(1)
}

fn draw_monitor(c: &mut Canvas, lit: bool) {
    icons::NET_MONITOR.paint_inked(c, 0, 0, 1, |ink| {
        Some(match ink {
            _ if lit => ink,
            SCREEN | GLINT => SCREEN_OFF,
            POWER => POWER_OFF,
            ink => ink,
        })
    });
}

fn draw_cross(c: &mut Canvas) {
    for i in 0..7 {
        c.put(1 + i, 5 + i, CROSS);
        c.put(7 - i, 5 + i, CROSS);
    }
}

fn draw_bars(c: &mut Canvas, filled: u8) {
    for (i, &(col, top)) in BARS.iter().enumerate() {
        let color = if (i as u8) < filled {
            BAR_ON
        } else {
            SCREEN_OFF
        };
        for y in top..=BAR_BOTTOM {
            c.put(col, y, color);
        }
    }
}

pub fn draw(icon: NetIcon, vpn: bool) -> Canvas {
    let mut c = Canvas::new(SIZE, SIZE);
    match icon {
        NetIcon::Wired | NetIcon::WiredUnplugged | NetIcon::Connecting => {
            icons::NET_PLUG.paint(&mut c, 0, 0, 1);
            draw_monitor(&mut c, icon == NetIcon::Wired);
            if icon == NetIcon::WiredUnplugged {
                draw_cross(&mut c);
            }
        }
        NetIcon::Wireless(n) => {
            draw_monitor(&mut c, true);
            draw_bars(&mut c, n);
        }
        NetIcon::WirelessOff => {
            draw_monitor(&mut c, false);
            draw_bars(&mut c, 0);
            draw_cross(&mut c);
        }
    }
    if vpn {
        icons::NET_LOCK.paint(&mut c, LOCK_X, LOCK_Y, 1);
    }
    c
}

/// Put the limited-connectivity badge over `c`'s bottom right, covering a padlock: no route out
/// matters more than the tunnel.
pub fn badge_limited(c: &mut Canvas) {
    icons::NET_LIMITED.paint(c, 9, 9, 1);
}

pub fn draw32(icon: NetIcon, vpn: bool) -> Canvas {
    draw(icon, vpn).resample_box(SIZE * 2, SIZE * 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: Color = Color::rgb(255, 216, 0);
    const WARN_OUTLINE: Color = Color::rgb(128, 96, 0);
    const WARN_FILL: Color = Color::rgb(255, 204, 0);
    const WARN_MARK: Color = Color::rgb(0, 0, 0);

    fn px(c: &Canvas, x: i32, y: i32) -> Option<Color> {
        let v = c.get(x, y);
        (v >> 24 != 0).then(|| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
    }

    #[test]
    fn tray_bars_are_the_list_bars_less_one() {
        for (strength, tray) in [
            (0, 0),
            (20, 0),
            (21, 1),
            (40, 1),
            (41, 2),
            (80, 3),
            (81, 4),
            (100, 4),
        ] {
            assert_eq!(
                bars(strength),
                tray,
                "strength {strength} is {tray} tray bars"
            );
        }
    }

    #[test]
    fn wired_has_a_teal_screen_unplugged_has_red_x() {
        let wired = draw(NetIcon::Wired, false);
        let unplugged = draw(NetIcon::WiredUnplugged, false);
        assert_eq!(px(&wired, 2, 5), Some(SCREEN), "wired screen is teal");
        assert_eq!(
            px(&unplugged, 2, 5),
            Some(SCREEN_OFF),
            "unplugged screen is grey"
        );
        assert_eq!(
            px(&unplugged, 1, 5),
            Some(CROSS),
            "the X's top-left diagonal pixel"
        );
        assert_ne!(
            px(&wired, 1, 5),
            Some(CROSS),
            "wired has no cross at that pixel"
        );
    }

    #[test]
    fn wired_runs_a_cable_to_a_plug() {
        let cable = Color::rgb(48, 96, 176);
        let wired = draw(NetIcon::Wired, false);
        let connecting = draw(NetIcon::Connecting, false);
        for c in [&wired, &connecting] {
            assert_eq!(px(c, 13, 7), Some(cable), "the cable's rise");
            assert_eq!(px(c, 9, 8), Some(cable), "the cable at the monitor");
            assert_eq!(px(c, 12, 2), Some(Color::rgb(212, 168, 48)), "the contacts");
        }
        assert_eq!(px(&wired, 6, 10), Some(POWER), "the power light");
        assert_eq!(px(&connecting, 6, 10), Some(POWER_OFF));
        assert_eq!(px(&connecting, 2, 4), Some(SCREEN_OFF), "the glint greys");
        assert_ne!(px(&connecting, 1, 5), Some(CROSS));
        assert_eq!(px(&draw(NetIcon::Wireless(4), false), 13, 7), Some(BAR_ON));
        assert_eq!(px(&draw(NetIcon::Wireless(0), false), 12, 2), None);
    }

    #[test]
    fn wireless_bars_fill_to_strength() {
        let two = draw(NetIcon::Wireless(2), false);
        let one = draw(NetIcon::Wireless(1), false);
        assert_eq!(px(&two, 11, 10), Some(BAR_ON), "second bar lit at 2");
        assert_eq!(px(&one, 11, 10), Some(SCREEN_OFF), "second bar dark at 1");
    }

    #[test]
    fn vpn_padlock_is_only_drawn_when_asked() {
        let with_vpn = draw(NetIcon::Wired, true);
        let without = draw(NetIcon::Wired, false);
        assert_eq!(px(&with_vpn, 12, 12), Some(LOCK));
        assert_eq!(
            px(&without, 12, 12),
            None,
            "no lock and nothing else drawn there either"
        );
    }

    #[test]
    fn the_limited_badge_is_a_triangle_with_a_mark() {
        let mut c = draw(NetIcon::Wired, true);
        badge_limited(&mut c);
        assert_eq!(px(&c, 12, 9), Some(WARN_OUTLINE), "the apex");
        assert_eq!(px(&c, 11, 9), Some(LOCK), "the shackle beside the apex");
        assert_eq!(px(&c, 12, 10), Some(WARN_FILL));
        for y in [11, 12, 14] {
            assert_eq!(px(&c, 12, y), Some(WARN_MARK), "the mark at row {y}");
        }
        assert_eq!(px(&c, 12, 13), Some(WARN_FILL), "the gap above the dot");
        for x in 9..16 {
            assert_eq!(px(&c, x, 15), Some(WARN_OUTLINE), "the base at column {x}");
        }
        assert_eq!(px(&c, 10, 12), Some(WARN_OUTLINE), "the left side");
        assert_eq!(px(&c, 15, 14), Some(WARN_OUTLINE), "the right side");
        assert_eq!(
            px(&c, 15, 10),
            Some(LOCK),
            "the padlock's corner outside the triangle stays"
        );
    }

    #[test]
    fn draw32_is_a_2x_scale() {
        let icon = NetIcon::Wireless(3);
        let small = draw(icon, true);
        let big = draw32(icon, true);
        assert_eq!((big.width, big.height), (32, 32));
        for y in 0..SIZE as i32 {
            for x in 0..SIZE as i32 {
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    assert_eq!(
                        big.get(2 * x + dx, 2 * y + dy),
                        small.get(x, y),
                        "block of ({x}, {y}) is that pixel at +({dx}, {dy})"
                    );
                }
            }
        }
    }
}
