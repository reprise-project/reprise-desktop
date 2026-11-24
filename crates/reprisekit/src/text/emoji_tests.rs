use super::*;

const PX: u32 = 11;
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const BLACK: Color = Color::rgb(0, 0, 0);

fn tahoma() -> TextRenderer {
    crate::pins::fonts();
    TextRenderer::load(&FontConfig::default())
}

fn device_hash(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in c.pixels.iter().flat_map(|p| p.to_le_bytes()) {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    h
}

/// Whether device pixel `p` has hue: its channels differ.
fn hued(p: u32) -> bool {
    let (r, g, b) = ((p >> 16) & 0xFF, (p >> 8) & 0xFF, p & 0xFF);
    r.abs_diff(g) > 24 || g.abs_diff(b) > 24 || r.abs_diff(b) > 24
}

/// The device columns and rows of `c` that are not `paper`, as (x0, x1, y0, y1).
fn ink(c: &Canvas, paper: u32) -> Option<(i32, i32, i32, i32)> {
    let (w, h) = c.device_size();
    let mut out: Option<(i32, i32, i32, i32)> = None;
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            if c.pixels[(y * w as i32 + x) as usize] != paper {
                let (x0, x1, y0, y1) = out.unwrap_or((x, x + 1, y, y + 1));
                out = Some((x0.min(x), x1.max(x + 1), y0.min(y), y1.max(y + 1)));
            }
        }
    }
    out
}

fn on_white(r: &TextRenderer, text: &str, w: i32, s: i32) -> Canvas {
    let mut c = Canvas::with_scale(w as u32, 20, s);
    c.fill_rect(c.bounds(), WHITE);
    r.draw(
        &mut c,
        Rect::new(0, 0, w, 20),
        text,
        false,
        BLACK,
        Align::Left,
        PX,
    );
    c
}

#[test]
fn an_emoji_draws_in_colour_inside_its_advance_and_line_at_scale_1_and_2() {
    let r = tahoma();
    let adv = r.measure("😀", false, PX);
    assert_eq!(adv, r.advance('😀', false, PX));
    assert!((12..=16).contains(&adv), "about an em and a quarter: {adv}");
    assert_eq!(
        r.measure("a😀b", false, PX),
        r.measure("ab", false, PX) + adv
    );
    let (ascent, descent, _) = r.line_metrics(false, PX).unwrap();
    let baseline = r.baseline(Rect::new(0, 0, 40, 20), false, PX).unwrap();
    for s in [1, 2] {
        let c = on_white(&r, "😀", 40, s);
        let (x0, x1, y0, y1) = ink(&c, WHITE.argb()).expect("the emoji inks");
        assert!(c.pixels.iter().any(|&p| hued(p)), "scale {s}: in colour");
        assert!(x0 >= 0 && x1 <= s * adv, "scale {s}: within its advance");
        assert!(
            y0 >= s * (baseline - ascent) && y1 <= s * (baseline - descent),
            "scale {s}: within the line, rows {y0}..{y1}"
        );
    }
}

#[test]
fn a_char_shown_as_text_by_default_draws_in_colour_only_before_the_emoji_selector() {
    let r = tahoma();
    let colour = |text: &str| on_white(&r, text, 40, 1).pixels.iter().any(|&p| hued(p));
    assert!(!colour("\u{2764}"), "a plain heart is a text glyph");
    assert!(colour("\u{2764}\u{FE0F}"), "with U+FE0F it is the emoji");
}

#[test]
fn an_emoji_sequence_draws_as_one_glyph_on_one_advance() {
    let r = tahoma();
    let one = r.measure("😀", false, PX);
    for seq in ["👍🏽", "👨‍👩‍👧", "🇭🇺", "❤️", "#️⃣"] {
        assert_eq!(r.pieces(seq).count(), 1, "{seq} is one piece");
        assert_eq!(r.measure(seq, false, PX), one, "{seq}");
        let text = format!("a{seq}b");
        assert_eq!(
            r.span(&text, 1 + seq.chars().count(), false, PX).0,
            r.measure("a", false, PX) + one,
            "{seq}: the char after it starts past its one advance"
        );
    }
}

#[test]
fn a_caret_and_selection_line_up_with_the_emoji_drawn() {
    let r = tahoma();
    let text = "x😀y";
    let at = |end: usize| r.measure(&text[..end], false, PX);
    let (start, end) = (1, 1 + '😀'.len_utf8());
    let hl = Rect::new(at(start), 0, at(end) - at(start), 20);
    for s in [1, 2] {
        let h = r.highlight(
            hl,
            text,
            start..end,
            0,
            Rect::new(0, 0, 40, 20),
            false,
            PX,
            s,
        );
        let c = on_white(&r, "😀", 40, s);
        let (x0, x1, _, _) = ink(&c, WHITE.argb()).unwrap();
        let pen = s * at(start);
        assert!(
            h.dev.x <= pen + x0 && pen + x1 <= h.dev.right(),
            "scale {s}: the highlight {:?} holds the emoji's ink {}..{}",
            h.dev,
            pen + x0,
            pen + x1
        );
    }
}

#[test]
fn inked_text_draws_an_emoji_as_its_coverage_in_the_text_colour() {
    let r = tahoma();
    let grey = Color::rgb(0x80, 0x80, 0x80);
    let mut c = Canvas::new(40, 20);
    c.fill_rect(c.bounds(), WHITE);
    let ink = r.inked();
    ink.draw(
        &mut c,
        Rect::new(0, 0, 40, 20),
        "😀",
        false,
        grey,
        Align::Left,
        PX,
    );
    assert!(c.pixels.iter().all(|&p| !hued(p)), "no colour");
    assert!(c.pixels.contains(&grey.argb()), "solid grey under the face");
    assert_eq!(ink.measure("😀", false, PX), r.measure("😀", false, PX));
    assert!(std::ptr::eq(ink.inked(), ink));
}

#[test]
fn an_emoji_on_a_translucent_canvas_keeps_its_colour_and_the_text_its_ink() {
    let r = tahoma();
    let mut c = Canvas::new(60, 20);
    let red = Color::rgb(0xFF, 0, 0);
    r.draw_over(
        &mut c,
        Rect::new(0, 0, 60, 20),
        "II😀",
        false,
        red,
        Align::Left,
        PX,
    );
    let emoji_x = r.measure("II", false, PX);
    let (w, _) = c.device_size();
    let at = |x: i32, y: i32| c.pixels[(y * w as i32 + x) as usize];
    let left: Vec<u32> = (0..20)
        .flat_map(|y| (0..emoji_x).map(move |x| (x, y)))
        .map(|(x, y)| at(x, y))
        .filter(|&p| p >> 24 != 0)
        .collect();
    assert!(
        !left.is_empty() && left.iter().all(|&p| p & 0xFFFF == 0),
        "the I's are red"
    );
    let emoji = (0..20).flat_map(|y| (emoji_x..60).map(move |x| (x, y)));
    let yellow = |p: u32| p >> 24 != 0 && hued(p) && (p >> 8) & 0xFF > 0x40;
    assert!(
        emoji.map(|(x, y)| at(x, y)).any(yellow),
        "the face is yellow"
    );
}

#[test]
fn a_font_sample_draws_no_emoji_from_another_face() {
    let dir = crate::pins::fonts();
    let t = TextRenderer::from_file(&dir.join("tahoma.ttf"), 0).expect("Tahoma");
    let mut c = Canvas::new(40, 20);
    c.fill_rect(c.bounds(), WHITE);
    t.draw(
        &mut c,
        Rect::new(0, 0, 40, 20),
        "😀",
        false,
        BLACK,
        Align::Left,
        PX,
    );
    assert!(c.pixels.iter().all(|&p| !hued(p)));
}

#[test]
fn emoji_text_pins_at_scale_1_and_2() {
    let r = tahoma();
    let text = "Hi 😀 👍🏽 🇭🇺 ❤️ ok";
    let (w, h) = (r.measure(text, false, PX) + 4, 18);
    let cs = crate::theme::Theme::default().colors;
    let mut got = Vec::new();
    for s in [1, 2] {
        let draw = |state: &str| {
            let mut c = Canvas::with_scale(w as u32, h as u32, s);
            c.fill_rect(c.bounds(), cs.window);
            let rect = Rect::new(2, 0, w - 2, h);
            match state {
                "plain" => r.draw(&mut c, rect, text, false, cs.window_text, Align::Left, PX),
                "selected" => {
                    let from = 2 + r.measure("Hi ", false, PX);
                    let to = 2 + r.measure("Hi 😀 👍🏽", false, PX);
                    c.fill_rect(Rect::new(from, 1, to - from, h - 2), cs.hilight);
                    let parts = [
                        (2, from, cs.window_text),
                        (from, to, cs.hilight_text),
                        (to, w, cs.window_text),
                    ];
                    for (x0, x1, ink) in parts {
                        let clip = Rect::new(x0, 0, x1 - x0, h);
                        r.draw_clipped(&mut c, rect, clip, text, false, ink, Align::Left, PX);
                    }
                }
                _ => {
                    let lit = Rect::new(3, 1, w - 2, h);
                    r.inked()
                        .draw(&mut c, lit, text, false, cs.button_hilight, Align::Left, PX);
                    r.inked()
                        .draw(&mut c, rect, text, false, cs.gray_text, Align::Left, PX);
                }
            }
            device_hash(&c)
        };
        for state in ["plain", "selected", "greyed"] {
            got.push((format!("{state} {s}"), draw(state)));
        }
    }
    crate::pins::check("emoji_text", &got);
}
