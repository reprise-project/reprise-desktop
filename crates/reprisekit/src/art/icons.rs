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
        let rgba = to_rgba(&buf[..info.buffer_size()], info.color_type)
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

/// 8-bit pixels of `color_type` (after `EXPAND`) as RGBA.
fn to_rgba(buf: &[u8], color_type: png::ColorType) -> Option<Vec<u8>> {
    use png::ColorType as C;
    Some(match color_type {
        C::Rgba => buf.to_vec(),
        C::Rgb => buf
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        C::GrayscaleAlpha => buf
            .chunks(2)
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        C::Grayscale => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        C::Indexed => return None,
    })
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art::{bticon, devicon, neticon, powericon, printbadge, viewsicon};
    use crate::classic::indicator;
    use crate::controls::iconview;

    fn fnv(c: &Canvas) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |v: u32| {
            for b in v.to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
            }
        };
        eat(c.width);
        eat(c.height);
        eat(c.scale() as u32);
        for &p in &c.pixels {
            eat(p);
        }
        h
    }

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
        }
        out
    }

    /// Every state's FNV hash, at scale 1 and, where the draw takes a scale, 2.
    const PINS: &[(&str, u64)] = &[
        ("devicon Headphones", 0x447f_6e14_74b4_9fb4),
        ("devicon Headset", 0x2ff1_5adc_9601_2839),
        ("devicon Speaker", 0x9751_7f39_b9ef_723d),
        ("devicon Gamepad", 0x70d5_ac4a_e0a0_3abc),
        ("devicon Keyboard", 0x653e_c1fe_768f_8fdc),
        ("devicon Mouse", 0xa8ec_99f5_6a25_bd51),
        ("devicon Phone", 0xcfda_e781_8ad5_3375),
        ("devicon Computer", 0x8346_6371_d222_261c),
        ("devicon Generic", 0x89b8_05ee_a7b2_e2e8),
        ("power ac", 0xf390_dabb_9f78_94f4),
        ("power 0 false false false", 0x31d2_e3be_56b0_0424),
        ("power 50 false false false", 0x4532_6546_9a16_863c),
        ("power 100 false false false", 0xe8b0_8897_08a8_4934),
        ("power 72 true false false", 0x63c0_8a85_3b62_5a7b),
        ("power 8 false true false", 0x0927_6709_1be3_2fd4),
        ("power 2 true true true", 0x2a83_2272_55ab_26e1),
        ("net Wired false", 0x165b_527c_c1b1_1dca),
        ("net32 Wired false", 0x3f25_ef2e_9236_0294),
        ("net Wired true", 0xb128_f537_51cc_b012),
        ("net32 Wired true", 0xeda9_965c_5f2f_de14),
        ("net limited Wired", 0x420c_e13e_a07b_6fc3),
        ("net WiredUnplugged false", 0xa968_79ee_2369_850d),
        ("net32 WiredUnplugged false", 0x534f_161a_61f0_9a24),
        ("net WiredUnplugged true", 0xe658_a478_9d65_5e85),
        ("net32 WiredUnplugged true", 0x53a3_ea27_f1dc_18a4),
        ("net limited WiredUnplugged", 0xe248_86c2_8e3a_d940),
        ("net Connecting false", 0x1419_83d5_6a0a_d478),
        ("net32 Connecting false", 0xe58d_3503_bf0c_c7fc),
        ("net Connecting true", 0x730d_1b0a_34d1_9450),
        ("net32 Connecting true", 0x7166_9bad_43c8_32fc),
        ("net limited Connecting", 0x6df2_642e_2e00_0819),
        ("net Wireless(0) false", 0x9fa8_121d_0cf9_d890),
        ("net32 Wireless(0) false", 0x4e18_5c65_4b24_5e4c),
        ("net Wireless(0) true", 0x1698_e4e7_5747_0004),
        ("net32 Wireless(0) true", 0xee00_6486_7606_8e8c),
        ("net limited Wireless(0)", 0xb4d7_f00c_36b6_b661),
        ("net Wireless(2) false", 0x44fb_7246_bdda_bf50),
        ("net32 Wireless(2) false", 0x2945_e15e_a15a_5a8c),
        ("net Wireless(2) true", 0xbcab_4cb4_adeb_ae44),
        ("net32 Wireless(2) true", 0xe561_7df3_91a5_0f0c),
        ("net limited Wireless(2)", 0xbce2_3261_42ce_2ac1),
        ("net Wireless(4) false", 0x8939_f11b_8f4d_a350),
        ("net32 Wireless(4) false", 0x3270_e0a8_0226_290c),
        ("net Wireless(4) true", 0x8812_a542_de99_4ac4),
        ("net32 Wireless(4) true", 0x7858_bd08_ddb8_6d4c),
        ("net limited Wireless(4)", 0xc2c2_4b0e_cf07_6f21),
        ("net WirelessOff false", 0xdaa2_c1c2_3d37_1e77),
        ("net32 WirelessOff false", 0xf3a7_af6f_8d6c_551c),
        ("net WirelessOff true", 0xfbee_215a_2926_17cb),
        ("net32 WirelessOff true", 0x92dd_cd4b_f7d2_609c),
        ("net limited WirelessOff", 0xa9fc_64d2_183e_cac2),
        ("views 24", 0xc293_f6ee_9b9b_f228),
        ("views 16", 0x3a44_b8ae_188d_f1cc),
        ("views 16 at 2", 0xaaeb_8b01_7371_cff7),
        ("bt false", 0x297d_e668_7983_10cc),
        ("bt32 false", 0x1fab_49c3_6fb5_b0c4),
        ("bt true", 0xf468_7a86_0ab3_986c),
        ("bt32 true", 0x3ae2_8e30_9975_c704),
        ("printbadge Default", 0xe67e_147b_f69b_9b9c),
        ("printbadge Paused", 0xebae_e3d2_4656_aee4),
        ("printbadge Error", 0x4e26_6fe6_5947_2f04),
        ("camera 16", 0x9706_8e69_4ece_088b),
        ("location 16", 0xee80_5878_74ba_e334),
        ("sticky 16 0", 0x1d82_2ddc_dc83_2af4),
        ("sticky 16 1", 0xf3a2_04c9_6887_7874),
        ("sticky 16 4", 0x3634_b4f0_52fa_bcb4),
        ("sticky 16 8", 0x1561_6669_c1c8_28b4),
        ("sticky 16 64", 0x0822_844b_4913_5434),
        ("sticky 16 77", 0x0031_2b56_a50f_b4f4),
        ("mousekeys 16 Left false", 0xa7d4_c484_cce6_d749),
        ("mousekeys 16 Left true", 0x0bb3_8363_368e_1ec9),
        ("mousekeys 16 Middle false", 0x1dca_d7a5_62eb_3064),
        ("mousekeys 16 Middle true", 0xef34_1f8b_368b_0a64),
        ("mousekeys 16 Right false", 0x4561_633e_78ba_2299),
        ("mousekeys 16 Right true", 0xbdf4_4b26_0e73_0919),
        ("sun 16 false", 0x7ca5_e6f5_a0a6_a184),
        ("sun 16 true", 0x45b4_8983_a154_d1e4),
        ("bell 16 0 false", 0xc74b_9556_28ff_d32e),
        ("bell 16 3 false", 0x8090_aa8a_f78d_52c3),
        ("bell 16 12 false", 0x36f1_3da7_34c1_0e3e),
        ("bell 16 0 true", 0x15f6_dbee_3092_7839),
        ("bell 16 5 true", 0x446b_74f6_fc81_bc47),
        ("camera 32", 0x915f_62ab_74ad_fb8c),
        ("location 32", 0xc688_5d70_815f_41c4),
        ("sticky 32 0", 0x0276_e9e8_515b_f004),
        ("sticky 32 1", 0x76a5_4cbe_86d4_0384),
        ("sticky 32 4", 0x1d7a_d644_4b8b_b884),
        ("sticky 32 8", 0x6716_84b5_7b4f_b284),
        ("sticky 32 64", 0x2404_6159_f6d7_9d04),
        ("sticky 32 77", 0x72c3_afd8_c496_7784),
        ("mousekeys 32 Left false", 0x911e_0479_952b_a8e4),
        ("mousekeys 32 Left true", 0x135d_54c7_d4b8_f7e4),
        ("mousekeys 32 Middle false", 0xea5c_d91f_f545_3104),
        ("mousekeys 32 Middle true", 0xb410_a9d9_094f_8504),
        ("mousekeys 32 Right false", 0x7da6_6c70_0c45_6be4),
        ("mousekeys 32 Right true", 0xc320_7565_fe25_89e4),
        ("sun 32 false", 0x8cfc_53a0_94d2_7784),
        ("sun 32 true", 0x026b_8a71_b6b5_f704),
        ("bell 32 0 false", 0xf4d9_d0f9_cc07_bc4c),
        ("bell 32 3 false", 0x0daf_f465_c78b_490c),
        ("bell 32 12 false", 0x31a5_087d_c587_5ffc),
        ("bell 32 0 true", 0x05fa_0943_a292_7a44),
        ("bell 32 5 true", 0xd3ed_1aae_cc74_bfec),
        ("display laptop", 0x02de_19cb_a27f_653d),
        ("display monitor", 0x139f_fe0a_53fa_168d),
        ("display laptop at 2", 0x42da_aa69_2abf_7ba7),
        ("shortcut 1", 0x7c7b_041e_2a2f_c211),
        ("shortcut 2", 0xb3f3_d648_7aa2_e244),
    ];

    #[test]
    fn icons_pin_at_scale_1_and_2() {
        let got = cases();
        let lines: Vec<String> = got
            .iter()
            .map(|(name, c)| format!("(\"{name}\", 0x{:016x}),", fnv(c)))
            .collect();
        let want: Vec<String> = PINS
            .iter()
            .map(|(name, h)| format!("(\"{name}\", 0x{h:016x}),"))
            .collect();
        assert!(lines == want, "the icons moved:\n{}", lines.join("\n"));
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
