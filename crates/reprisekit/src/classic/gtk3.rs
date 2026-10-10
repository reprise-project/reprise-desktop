//! Classic's 3D edges for the generated GTK3 theme ([`crate::toolkit`]): `gtk3.css` with its placeholders
//! filled in for a colour scheme, and the pictures it reads (GTK3 reads no `data:` URLs, so they are files).
//! An edge is a stack of 1 px inset box shadows; each ring's bottom-right colour lies over its top-left one.

use crate::classic::widgets::{
    Arrow, CheckState, draw_arrow_h, draw_arrow_v, draw_checkbox_disabled, draw_checkbox_held,
    draw_list_check, draw_radio, draw_radio_disabled,
};
use crate::paint::{Backdrop, Look, Paint};
use crate::raster::{Canvas, EdgeStyle, Rect};
use crate::theme::{Color, ColorScheme};

const RULES: &str = include_str!("gtk3.css");

pub fn bevels(cs: &ColorScheme) -> String {
    let fills = fills(cs);
    let mut out = String::from("\n");
    let mut rest = RULES;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        let name = &rest[start + 1..start + 1 + len];
        out += &rest[..start];
        match fills.iter().find(|(n, _)| *n == name) {
            Some((_, text)) => out += text,
            None => out += &rest[start..start + len + 2],
        }
        rest = &rest[start + len + 2..];
    }
    out += rest;
    out
}

fn fills(cs: &ColorScheme) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("raised", stack(&edge_rings(cs, EdgeStyle::ButtonRaised))),
        (
            "window_raised",
            stack(&edge_rings(cs, EdgeStyle::WindowRaised)),
        ),
        ("sunken", stack(&edge_rings(cs, EdgeStyle::ButtonSunken))),
        ("pushed", stack(&edge_rings(cs, EdgeStyle::ButtonPushed))),
        ("thin_raised", stack(&edge_rings(cs, EdgeStyle::ThinRaised))),
        ("thin_sunken", stack(&edge_rings(cs, EdgeStyle::ThinSunken))),
        ("etched", stack(&etched_rings(cs))),
        (
            "pressed",
            stack(&[
                (cs.window_frame, cs.window_frame),
                (cs.button_shadow, cs.button_shadow),
            ]),
        ),
        // The default button: a text-coloured ring round the raised edge.
        ("default", {
            let mut rings = vec![(cs.button_text, cs.button_text)];
            rings.extend(edge_rings(cs, EdgeStyle::ButtonRaised));
            stack(&rings)
        }),
    ];
    v.extend(ART.iter().map(|&name| {
        let text =
            format!("-gtk-scaled(url(\"{ASSETS}/{name}.png\"), url(\"{ASSETS}/{name}@2.png\"))");
        (name, text)
    }));
    v
}

pub const ASSETS: &str = "assets";

const ART: [&str; 22] = [
    "check_off",
    "check_on",
    "check_mixed",
    "check_off_held",
    "check_on_held",
    "check_off_greyed",
    "check_on_greyed",
    "radio_off",
    "radio_on",
    "radio_off_greyed",
    "radio_on_greyed",
    "list_check_off",
    "list_check_on",
    "arrow_up",
    "arrow_down",
    "arrow_left",
    "arrow_right",
    "arrow_up_greyed",
    "arrow_down_greyed",
    "arrow_left_greyed",
    "arrow_right_greyed",
    "dither",
];

pub fn assets(cs: &ColorScheme) -> Vec<(String, Vec<u8>)> {
    [(1, ""), (2, "@2")]
        .into_iter()
        .flat_map(|(s, tag)| {
            ART.iter()
                .zip(art(cs, s))
                .map(move |(name, c)| (format!("{name}{tag}.png"), rgba_png(&c)))
        })
        .collect()
}

fn edge_rings(cs: &ColorScheme, style: EdgeStyle) -> Vec<(Color, Color)> {
    rings_of(|c, r| c.edge_t(r, style, cs, 1))
}

fn etched_rings(cs: &ColorScheme) -> Vec<(Color, Color)> {
    rings_of(|c, r| c.etched_frame_t(r, cs, 1))
}

fn rings_of(draw: impl FnOnce(&mut Canvas, Rect)) -> Vec<(Color, Color)> {
    let mut c = Canvas::new(6, 6);
    let r = c.bounds();
    draw(&mut c, r);
    let color = |p: u32| Color::rgb((p >> 16) as u8, (p >> 8) as u8, p as u8);
    (0..2)
        .map(|i| (c.get(i, 2), c.get(5 - i, 2)))
        .take_while(|&(tl, _)| tl >> 24 != 0)
        .map(|(tl, br)| (color(tl), color(br)))
        .collect()
}

/// `rings` as a `box-shadow` list, innermost ring first, each ring's bottom-right after its top-left.
fn stack(rings: &[(Color, Color)]) -> String {
    rings
        .iter()
        .enumerate()
        .rev()
        .map(|(i, ring)| (ring, i + 1))
        .map(|(&(tl, br), n)| {
            format!(
                "inset {n}px {n}px {}, inset -{n}px -{n}px {}",
                hex(tl),
                hex(br)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

fn art(cs: &ColorScheme, s: i32) -> Vec<Canvas> {
    let p = Paint {
        cs,
        look: &Look::Classic,
        s,
        back: Backdrop::Face,
        active: true,
    };
    let draw = |w: i32, h: i32, f: &dyn Fn(&mut Canvas, Rect)| {
        let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
        let r = c.bounds();
        f(&mut c, r);
        c
    };
    let check = |state, held| draw(13, 13, &|c, r| draw_checkbox_held(c, &p, r, state, held));
    let arrow = |dir: Arrow, greyed: bool| {
        draw(16, 16, &|c, r| {
            let one = |c: &mut Canvas, r: Rect, ink| match dir {
                Arrow::Up => draw_arrow_v(c, r, s, false, ink),
                Arrow::Down => draw_arrow_v(c, r, s, true, ink),
                Arrow::Left => draw_arrow_h(c, r, s, false, ink),
                Arrow::Right => draw_arrow_h(c, r, s, true, ink),
            };
            if greyed {
                let down_right = Rect::new(r.x + s, r.y + s, r.width, r.height);
                one(c, down_right, cs.button_hilight);
                one(c, r, cs.button_shadow);
            } else {
                one(c, r, cs.button_text);
            }
        })
    };
    let greyed_check = |on| draw(13, 13, &|c, r| draw_checkbox_disabled(c, &p, r, on));
    let radio = |on| draw(12, 12, &|c, r| draw_radio(c, &p, r, on));
    let greyed_radio = |on| draw(12, 12, &|c, r| draw_radio_disabled(c, &p, r, on));
    let list_check = |on| draw(13, 13, &|c, r| draw_list_check(c, &p, r, on));
    vec![
        check(CheckState::Off, false),
        check(CheckState::On, false),
        check(CheckState::Mixed, false),
        check(CheckState::Off, true),
        check(CheckState::On, true),
        greyed_check(false),
        greyed_check(true),
        radio(false),
        radio(true),
        greyed_radio(false),
        greyed_radio(true),
        list_check(false),
        list_check(true),
        arrow(Arrow::Up, false),
        arrow(Arrow::Down, false),
        arrow(Arrow::Left, false),
        arrow(Arrow::Right, false),
        arrow(Arrow::Up, true),
        arrow(Arrow::Down, true),
        arrow(Arrow::Left, true),
        arrow(Arrow::Right, true),
        draw(2, 2, &|c, r| {
            c.dither_rect_t(r, cs.button_face, cs.button_hilight, s)
        }),
    ]
}

fn rgba_png(c: &Canvas) -> Vec<u8> {
    let (w, h) = c.device_size();
    let rgba: Vec<u8> = c
        .pixels
        .iter()
        .flat_map(|&p| {
            let a = p >> 24;
            // Premultiplied to straight; a clear pixel stays zero.
            let un = |v: u32| ((v & 0xff) * 255).checked_div(a).unwrap_or(0) as u8;
            [un(p >> 16), un(p >> 8), un(p), a as u8]
        })
        .collect();
    let mut bytes = Vec::new();
    let mut enc = png::Encoder::new(&mut bytes, w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    // Writing a few hundred bytes into a Vec cannot fail.
    let mut wr = enc.write_header().expect("png header");
    wr.write_image_data(&rgba).expect("png data");
    wr.finish().expect("png end");
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(cs: &ColorScheme) -> String {
        let css = bevels(cs);
        assert!(!css.contains('%'), "a placeholder is left");
        css
    }

    fn rule<'a>(css: &'a str, selector: &str) -> &'a str {
        let at = css
            .find(&format!("\n{selector}"))
            .unwrap_or_else(|| panic!("no rule for {selector}"));
        let end = css[at..].find('}').unwrap();
        &css[at..at + end]
    }

    #[test]
    fn standard_edges() {
        let css = sheet(&ColorScheme::default());
        let raised = "inset 2px 2px #d4d0c8, inset -2px -2px #808080, \
                      inset 1px 1px #ffffff, inset -1px -1px #404040";
        assert!(rule(&css, "button, button:hover").contains(&format!("box-shadow: {raised};")));
        assert!(rule(&css, "entry, entry:focus").contains(
            "box-shadow: inset 2px 2px #404040, inset -2px -2px #d4d0c8, \
             inset 1px 1px #808080, inset -1px -1px #ffffff;"
        ));
        assert!(
            rule(&css, "button:active, button:backdrop:active").contains(
                "box-shadow: inset 2px 2px #808080, inset -2px -2px #808080, \
             inset 1px 1px #000000, inset -1px -1px #000000;"
            )
        );
        assert!(rule(&css, "button.default").contains(
            "box-shadow: inset 3px 3px #d4d0c8, inset -3px -3px #808080, \
             inset 2px 2px #ffffff, inset -2px -2px #404040, \
             inset 1px 1px #000000, inset -1px -1px #000000;"
        ));
        assert!(rule(&css, "scrollbar slider").contains(
            "box-shadow: inset 2px 2px #ffffff, inset -2px -2px #808080, \
             inset 1px 1px #d4d0c8, inset -1px -1px #404040;"
        ));
        assert!(
            rule(&css, "button.flat:hover")
                .contains("box-shadow: inset 1px 1px #ffffff, inset -1px -1px #808080;")
        );
        assert!(rule(&css, "frame > border").contains(
            "box-shadow: inset 2px 2px #ffffff, inset -2px -2px #808080, \
             inset 1px 1px #808080, inset -1px -1px #ffffff;"
        ));
    }

    #[test]
    fn the_sheet_reads_only_defined_colours() {
        let cs = ColorScheme::default();
        let css = crate::toolkit::gtk3_css(&cs, crate::interface::Interface::Classic);
        let defined: Vec<&str> = css
            .lines()
            .filter_map(|l| l.strip_prefix("@define-color "))
            .filter_map(|l| l.split(' ').next())
            .collect();
        for word in RULES.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@')) {
            if let Some(name) = word.strip_prefix('@') {
                assert!(defined.contains(&name), "@{name} is not defined");
            }
        }
        assert!(css.ends_with(&bevels(&cs)), "the edges come last");
    }

    #[test]
    fn a_scheme_brings_its_own_edges() {
        let cs = crate::schemes::scheme("High Contrast Black").unwrap();
        let css = sheet(&cs);
        let want = format!(
            "inset 2px 2px {}, inset -2px -2px {}, inset 1px 1px {}, inset -1px -1px {}",
            hex(cs.button_light),
            hex(cs.button_shadow),
            hex(cs.button_hilight),
            hex(cs.button_dk_shadow),
        );
        assert!(rule(&css, "button, button:hover").contains(&want));
    }

    #[test]
    fn the_art_is_classics_own_drawing() {
        let cs = ColorScheme::default();
        let files = assets(&cs);
        let css = sheet(&cs);
        assert_eq!(files.len(), 2 * ART.len());
        for (name, _) in &files {
            assert!(
                css.contains(&format!("url(\"{ASSETS}/{name}\")")),
                "{name} unused"
            );
        }
        let decode = |name: &str| {
            let bytes = &files.iter().find(|(n, _)| n == name).unwrap().1;
            let mut r = png::Decoder::new(std::io::Cursor::new(bytes))
                .read_info()
                .unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            (info.width, buf)
        };
        let px = |(w, buf): &(u32, Vec<u8>), x: u32, y: u32| {
            let i = ((y * w + x) * 4) as usize;
            [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
        };
        let on = decode("check_on.png");
        assert_eq!(on.0, 13);
        assert_eq!(
            px(&on, 0, 0),
            [0x80, 0x80, 0x80, 255],
            "shadow at the top-left"
        );
        assert_eq!(
            px(&on, 1, 1),
            [0x40, 0x40, 0x40, 255],
            "dark shadow inside it"
        );
        assert_eq!(
            px(&on, 12, 12),
            [255, 255, 255, 255],
            "hilight at the bottom-right"
        );
        assert_eq!(px(&on, 3, 5), [0, 0, 0, 255], "the mark's first stroke");
        assert_eq!(px(&decode("check_off.png"), 3, 5), [255, 255, 255, 255]);
        let on2 = decode("check_on@2.png");
        assert_eq!(on2.0, 26);
        assert_eq!(px(&on2, 7, 11), px(&on, 3, 5));
        assert_eq!(px(&on2, 3, 3), px(&on, 1, 1));
        let radio = decode("radio_on.png");
        assert_eq!(radio.0, 12);
        assert_eq!(px(&radio, 0, 0)[3], 0, "a round radio's corner is clear");
        assert_eq!(px(&radio, 5, 5), [0, 0, 0, 255], "the dot");
        let dither = decode("dither.png");
        assert_eq!(px(&dither, 0, 0)[..3], px(&dither, 1, 1)[..3]);
        assert_ne!(px(&dither, 0, 0)[..3], px(&dither, 1, 0)[..3]);
        let greyed = decode("arrow_up_greyed.png");
        assert_eq!(px(&greyed, 4, 9), [0x80, 0x80, 0x80, 255]);
        assert_eq!(px(&greyed, 4, 10)[3], 0);
        assert_eq!(px(&greyed, 11, 10), [255, 255, 255, 255]);
    }
}
