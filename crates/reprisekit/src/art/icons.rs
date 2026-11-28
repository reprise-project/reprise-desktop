//! Reprise's small icons, kept as PNG files under `icons/` and compiled in.
//! Each file decodes once, on first use, and stays for the process.
//!
//! The pixels are opaque or clear. A few files are masks whose inks the
//! draw code swaps at paint time (see [`Icon::paint_inked`]), so one file
//! serves every state of its icon.

use std::sync::OnceLock;

use crate::raster::{Canvas, Rect};
use crate::theme::Color;

pub struct Icon {
    name: &'static str,
    png: &'static [u8],
    decoded: OnceLock<Decoded>,
}

struct Decoded {
    w: u32,
    h: u32,
    px: Box<[Option<Color>]>,
}

impl Icon {
    const fn new(name: &'static str, png: &'static [u8]) -> Icon {
        Icon {
            name,
            png,
            decoded: OnceLock::new(),
        }
    }

    pub fn size(&'static self) -> (u32, u32) {
        let d = self.decoded();
        (d.w, d.h)
    }

    /// Paint the icon with its top-left at (`x`, `y`) of `c`, each pixel a
    /// `s` x `s` cell (nearest neighbour), clear pixels leaving `c` alone.
    pub fn paint(&'static self, c: &mut Canvas, x: i32, y: i32, s: i32) {
        self.paint_inked(c, x, y, s, Some);
    }

    /// [`Icon::paint`] with each pixel's colour passed through `ink`
    /// first; `None` leaves that cell alone. Mask icons hold one ink per
    /// region, which `ink` turns into the state's colour.
    pub fn paint_inked(
        &'static self,
        c: &mut Canvas,
        x: i32,
        y: i32,
        s: i32,
        ink: impl Fn(Color) -> Option<Color>,
    ) {
        let s = s.max(1);
        let d = self.decoded();
        for (i, px) in d.px.iter().enumerate() {
            if let Some(color) = px.and_then(&ink) {
                let (col, row) = ((i as u32 % d.w) as i32, (i as u32 / d.w) as i32);
                c.fill_rect(Rect::new(x + col * s, y + row * s, s, s), color);
            }
        }
    }

    pub fn canvas(&'static self) -> Canvas {
        self.canvas_at(1)
    }

    /// The icon on a canvas at `scale`, each pixel a `scale` x `scale` cell.
    pub fn canvas_at(&'static self, scale: i32) -> Canvas {
        let (w, h) = self.size();
        let mut c = Canvas::with_scale(w, h, scale);
        let s = c.scale();
        c.in_device_px(|c| self.paint(c, 0, 0, s));
        c
    }

    fn decoded(&'static self) -> &'static Decoded {
        self.decoded.get_or_init(|| self.decode())
    }

    /// The bytes are compiled in, so a file that does not decode or has a
    /// half-clear pixel is a build mistake and panics with the file's name.
    /// Any PNG colour type and bit depth reads.
    fn decode(&self) -> Decoded {
        let fail = |why: String| -> ! { panic!("icon {}: {why}", self.name) };
        let mut decoder = png::Decoder::new(std::io::Cursor::new(self.png));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder
            .read_info()
            .unwrap_or_else(|e| fail(format!("bad png header: {e}")));
        let size = reader
            .output_buffer_size()
            .unwrap_or_else(|| fail("png output buffer overflows".into()));
        let mut buf = vec![0u8; size];
        let info = reader
            .next_frame(&mut buf)
            .unwrap_or_else(|e| fail(format!("png data: {e}")));
        let rgba = crate::icontheme::png_rgba(&buf[..info.buffer_size()], info.color_type)
            .unwrap_or_else(|| fail(format!("{:?}: no RGBA reading", info.color_type)));
        let px = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, a]| match a {
                0 => None,
                255 => Some(Color::rgb(r, g, b)),
                _ => fail(format!("alpha {a}: pixels are opaque or clear")),
            })
            .collect();
        Decoded {
            w: info.width,
            h: info.height,
            px,
        }
    }
}

impl PartialEq for Icon {
    fn eq(&self, other: &Icon) -> bool {
        std::ptr::eq(self, other)
    }
}

impl std::fmt::Debug for Icon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name)
    }
}

macro_rules! icons {
    ($($(#[$doc:meta])* $id:ident = $file:literal;)*) => {
        $($(#[$doc])* pub static $id: Icon = Icon::new($file, include_bytes!(concat!("icons/", $file)));)*
        #[cfg(test)]
        static ALL: &[&Icon] = &[$(&$id),*];
    };
}

icons! {
    /// Headphones: a headband over two ear cups, cushions toward the head.
    HEADPHONES = "device-headphones.png";
    /// A headset: one cup, a slim pad and a boom to a microphone capsule.
    HEADSET = "device-headset.png";
    /// A speaker cabinet with a tweeter over a woofer.
    SPEAKER = "device-speaker.png";
    /// A game controller: d-pad left, four buttons right, grips below.
    GAMEPAD = "device-gamepad.png";
    /// A keyboard slab with three rows of keys and a space bar.
    KEYBOARD = "device-keyboard.png";
    /// A mouse with split buttons and a cord.
    MOUSE = "device-mouse.png";
    /// A mobile phone: a dark handset, blue screen, 3x4 keypad.
    PHONE = "device-phone.png";
    /// A monitor with blue glass on a neck and base.
    COMPUTER = "device-computer.png";
    /// The battery's outline and empty body, 16x16.
    BATTERY = "power-battery.png";
    /// The charging bolt laid over the battery, 16x16.
    BOLT = "power-bolt.png";
    /// The mains plug, 16x16.
    AC = "power-ac.png";
    /// The network monitor, 16x16, in its connected inks.
    NET_MONITOR = "net-monitor.png";
    /// The network plug and its cable into the monitor's side, 16x16.
    NET_PLUG = "net-plug.png";
    /// The VPN padlock, 6x7.
    NET_LOCK = "net-lock.png";
    /// The limited-connectivity triangle, 7x7.
    NET_LIMITED = "net-limited.png";
    /// The Views window with four tiles, 24x24.
    VIEWS_24 = "views-24.png";
    /// The Views window with four tiles, 16x16.
    VIEWS_16 = "views-16.png";
    /// The Bluetooth badge in its on inks, 16x16.
    BLUETOOTH = "bluetooth.png";
    /// The default printer's check disc, 8x8.
    PRINT_DEFAULT = "printbadge-default.png";
    /// The paused printer's bars disc, 8x8.
    PRINT_PAUSED = "printbadge-paused.png";
    /// The stopped printer's cross disc, 8x8.
    PRINT_ERROR = "printbadge-error.png";
    /// The webcam indicator, 16x16.
    CAMERA = "camera.png";
    /// The location pin indicator, 16x16.
    LOCATION = "location.png";
    /// The notification bell, 16x16.
    BELL = "bell.png";
    /// The StickyKeys mask, 16x16: black outline, the key insides in
    /// greys (Shift 64, Ctrl 96, Super 128, Alt 160).
    STICKY_KEYS = "sticky-keys.png";
    /// The MouseKeys mask, 16x16: black outline, the button insides in
    /// greys (left 64, middle 96, right 128).
    MOUSE_KEYS = "mouse-keys.png";
    /// The sun line sketch, 16x16: a one-ink mask in black.
    SUN = "sun.png";
    /// A laptop: an open lid with blue glass over a flat base, 16x16.
    LAPTOP = "display-laptop.png";
    /// A monitor with blue glass on a neck and foot, 16x16.
    MONITOR = "display-monitor.png";
    /// The shortcut arrow box, 11x11.
    SHORTCUT = "shortcut.png";
    /// The red exclamation disc for Confirm Delete, 12x12.
    DELETE_BADGE = "delete-badge.png";
    /// A set of files: two pages, the front one dog-eared, 32x32.
    SET = "set.png";
    /// An open folder holding a dog-eared page, 32x32.
    FOLDER_PAGE = "folder-page.png";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{bticon, devicon, neticon, powericon, printbadge, viewsicon};
    use crate::classic::indicator;
    use crate::controls::iconview;

    use crate::pins::fnv;

    fn cases() -> Vec<(String, Canvas)> {
        let mut out: Vec<(String, Canvas)> = Vec::new();
        for d in devicon::ALL {
            out.push((format!("devicon {d:?}"), devicon::draw(d)));
        }
        out.push(("power ac".into(), powericon::draw(powericon::Source::Ac)));
        for (percent, charging, low, critical) in [
            (0, false, false, false),
            (50, false, false, false),
            (100, false, false, false),
            (72, true, false, false),
            (8, false, true, false),
            (2, true, true, true),
        ] {
            out.push((
                format!("power {percent} {charging} {low} {critical}"),
                powericon::draw(powericon::Source::Battery {
                    percent,
                    charging,
                    low,
                    critical,
                }),
            ));
        }
        let nets = [
            neticon::NetIcon::Wired,
            neticon::NetIcon::WiredUnplugged,
            neticon::NetIcon::Connecting,
            neticon::NetIcon::Wireless(0),
            neticon::NetIcon::Wireless(2),
            neticon::NetIcon::Wireless(4),
            neticon::NetIcon::WirelessOff,
        ];
        for n in nets {
            for vpn in [false, true] {
                out.push((format!("net {n:?} {vpn}"), neticon::draw(n, vpn)));
                out.push((format!("net32 {n:?} {vpn}"), neticon::draw32(n, vpn)));
            }
            let mut c = neticon::draw(n, true);
            neticon::badge_limited(&mut c);
            out.push((format!("net limited {n:?}"), c));
        }
        out.push(("views 24".into(), viewsicon::draw24()));
        out.push(("views 16".into(), viewsicon::draw16()));
        out.push(("views 16 at 2".into(), viewsicon::draw16_at(2)));
        for on in [false, true] {
            out.push((format!("bt {on}"), bticon::draw(on)));
            out.push((format!("bt32 {on}"), bticon::draw32(on)));
        }
        for b in printbadge::ALL {
            out.push((format!("printbadge {b:?}"), printbadge::draw(b)));
        }
        for px in [16, 32] {
            out.push((format!("camera {px}"), indicator::draw_camera(px)));
            out.push((format!("location {px}"), indicator::draw_location(px)));
            for held in [0, 1, 4, 8, 64, 77] {
                out.push((
                    format!("sticky {px} {held}"),
                    indicator::draw_sticky(px, held),
                ));
            }
            for pick in [
                indicator::MouseButton::Left,
                indicator::MouseButton::Middle,
                indicator::MouseButton::Right,
            ] {
                for held in [false, true] {
                    out.push((
                        format!("mousekeys {px} {pick:?} {held}"),
                        indicator::draw_mouse_keys(px, pick, held),
                    ));
                }
            }
            for warm in [false, true] {
                out.push((format!("sun {px} {warm}"), indicator::draw_sun(px, warm)));
            }
            for (unseen, dnd) in [(0, false), (3, false), (12, false), (0, true), (5, true)] {
                out.push((
                    format!("bell {px} {unseen} {dnd}"),
                    indicator::draw_bell(px, unseen, dnd),
                ));
            }
        }
        out.push(("display laptop".into(), LAPTOP.canvas()));
        out.push(("display monitor".into(), MONITOR.canvas()));
        out.push(("display laptop at 2".into(), LAPTOP.canvas_at(2)));
        for s in [1, 2] {
            out.push((format!("shortcut {s}"), iconview::shortcut_overlay(s)));
            out.push((format!("set {s}"), SET.canvas_at(s)));
            out.push((format!("folder page {s}"), FOLDER_PAGE.canvas_at(s)));
        }
        out
    }

    #[test]
    fn icons_pin_at_scale_1_and_2() {
        let got: Vec<_> = cases()
            .into_iter()
            .map(|(name, c)| (name, fnv(&c)))
            .collect();
        crate::pins::check("art/icons", &got);
    }

    #[test]
    fn every_icon_decodes_opaque_or_clear() {
        for icon in ALL {
            let (w, h) = icon.size();
            assert!(w > 0 && h > 0, "{} has pixels", icon.name);
            let c = icon.canvas();
            assert!(
                c.pixels.iter().all(|p| p >> 24 == 0 || p >> 24 == 255),
                "{}",
                icon.name
            );
        }
    }

    #[test]
    fn paint_scales_each_pixel_to_a_cell() {
        let one = SHORTCUT.canvas();
        let mut two = Canvas::new(22, 22);
        SHORTCUT.paint(&mut two, 0, 0, 2);
        for y in 0..11 {
            for x in 0..11 {
                for (dx, dy) in [(0, 0), (1, 1)] {
                    assert_eq!(two.get(2 * x + dx, 2 * y + dy), one.get(x, y), "({x}, {y})");
                }
            }
        }
    }
}
