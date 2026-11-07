use crate::theme::Color;
const TIP_BG: Color = Color::rgb(255, 255, 225);
const TIP_INK: Color = Color::rgb(0, 0, 0);

use super::*;
use crate::theme::FontConfig;

#[test]
#[ignore]
fn render_tooltip() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let c = draw_tip(&theme, &text, "Click here to begin");
    crate::raster::write_preview(&c, "tooltip-preview.png");
    let c = draw_balloon_tip(
        &theme,
        &text,
        "Turn Off",
        "Shuts down Reprise so that you can safely turn off the computer.",
    );
    crate::raster::write_preview(&c, "balloon-tip-preview.png");
    let reply = NotifyReply {
        placeholder: "Write a reply",
        send: "Send",
    };
    let lay = notify_layout(
        &theme,
        &text,
        "Ana",
        "Lunch on Friday?",
        &["Reply"],
        &[],
        Some(reply),
    );
    let typed = TextEdit::new("Sounds good", false);
    let c = draw_notify_balloon(
        &theme,
        &text,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&typed, Caret::Shown, None)),
    );
    crate::raster::write_preview(&c, "notify-reply-preview.png");
}

#[test]
fn balloon_tip_has_a_stem_a_bubble_and_wraps_long_bodies() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let body = "Shuts down the desktop so that you can safely turn off the computer.";
    let (w, h) = balloon_size(&text, &theme, "Turn Off", body);
    assert!(w <= BALLOON_TEXT_W + 2 * BALLOON_PAD_X + 2);
    assert_eq!(
        h,
        STEM_H + BALLOON_BODY_Y + 2 * BALLOON_LINE_H + BALLOON_BOTTOM
    );
    let (_, one) = balloon_size(&text, &theme, "Log Off", "Closes your programs.");
    assert_eq!(one, h - BALLOON_LINE_H);
    let long = format!("{body} {body} {body}");
    let (lw, lh) = balloon_size(&text, &theme, "Turn Off", &long);
    assert!(
        lw <= BALLOON_TEXT_W + 2 * BALLOON_PAD_X + 2,
        "wrapped, not widened"
    );
    assert!(lh >= h + 2 * BALLOON_LINE_H);
    let c = draw_balloon_tip(&theme, &text, "Turn Off", body);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    // The apex: ink at (STEM_X, 0), clear left of it and clear above
    // the bubble elsewhere; the stem's right side reaches STEM_W at the
    // bubble.
    assert_eq!(c.get(STEM_X, 0), TIP_INK.argb());
    assert_eq!(c.get(STEM_X - 1, 0), 0);
    assert_eq!(c.get(STEM_X + 5, 4), 0);
    assert_eq!(c.get(STEM_X + STEM_W - 1, STEM_H - 1), TIP_INK.argb());
    assert_eq!(c.get(STEM_X + 4, STEM_H - 1), TIP_BG.argb());
    // The join is open; the bubble's border runs left of the stem.
    assert_eq!(c.get(STEM_X + 4, STEM_H), TIP_BG.argb());
    assert_eq!(c.get(STEM_X - 4, STEM_H), TIP_INK.argb());
    // Bubble corners are rounded: the corner pixel is clear, the middle
    // of the left border is ink, the inside is yellow.
    assert_eq!(c.get(0, STEM_H), 0);
    assert_eq!(c.get(0, STEM_H + 20), TIP_INK.argb());
    assert_eq!(c.get(w - 1, h - 1), 0);
    assert_eq!(c.get(w / 2, h - 3), TIP_BG.argb());
}

#[test]
fn info_tip_wraps_at_300_px_and_sits_where_asked() {
    crate::pins::fonts();
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let short = "Contains music and other audio files.";
    let (w, h) = info_size(&text, &theme, short);
    assert_eq!(h, INFO_LINE_H + 2 * INFO_PAD + 2);
    assert!(w < INFO_TEXT_W);
    let long = "Opens the settings that change how the desktop looks and behaves, which \
                programs start with it, and how it connects to networks, printers and \
                the other people who share it.";
    let (lw, lh) = info_size(&text, &theme, long);
    assert!(lw <= INFO_TEXT_W + 2 * INFO_PAD + 2);
    assert_eq!(
        lh,
        3 * INFO_LINE_H + 2 * INFO_PAD + 2,
        "this one wraps onto three lines"
    );
    let c = draw_info_tip(&theme, &text, short);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    assert_eq!(c.get(0, 0), TIP_INK.argb());
    assert_eq!(c.get(1, 1), TIP_BG.argb());
    let screen = Rect::new(0, 0, 1920, 1080);
    assert_eq!(place_below((279, 903), (291, 27), screen), (279, 903));
    assert_eq!(
        place_below((1900, 1070), (291, 27), screen),
        (1919 - 291, 1080 - 27)
    );
}

#[test]
fn balloon_hangs_its_apex_below_the_pointer_and_stays_on_screen() {
    let screen = Rect::new(0, 0, 1920, 1080);
    assert_eq!(place_balloon((1017, 392), (289, 63), screen), (1003, 412));
    let (x, y) = place_balloon((1900, 1070), (289, 63), screen);
    assert_eq!((x + 289, y + 63), (1919, 1080));
    assert_eq!(place_balloon((5, 5), (289, 63), screen), (0, 25));
}

#[test]
fn tip_is_text_plus_padding_inside_a_one_pixel_border() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let label = "Click here to begin";
    let (w, h) = tip_size(&text, &theme, label);
    assert_eq!(w, text.measure(label, false, 11) + 6);
    assert_eq!(h, 17);
    let c = draw_tip(&theme, &text, label);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    assert_eq!(c.get(0, 0), TIP_INK.argb());
    assert_eq!(c.get(w - 1, h - 1), TIP_INK.argb());
    assert_eq!(c.get(1, 1), TIP_BG.argb());
    assert_eq!(c.get(w - 2, h - 2), TIP_BG.argb());
    // Ink somewhere in the middle row.
    assert!((1..w - 1).any(|x| c.get(x, h / 2) == TIP_INK.argb()));
}

#[test]
fn scaled_tip_doubles_border_and_padding() {
    let theme = Theme::default().scaled(2);
    let text = TextRenderer::load(&FontConfig::default());
    let c = draw_tip(&theme, &text, "Hi");
    assert_eq!(c.height as i32, 22 + 12);
    assert_eq!(c.get(1, 1), TIP_INK.argb());
    assert_eq!(c.get(2, 2), TIP_BG.argb());
}

#[test]
fn a_multi_line_tip_stacks_its_lines() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let label = "Local Area Connection 2\nSpeed: 100.0 Mbps\nStatus: Connected";
    let (w, h) = tip_size(&text, &theme, label);
    // A one-line tip's own height (17 px) plus the 13 px line pitch
    // for each further line.
    assert_eq!(h, 17 + 2 * INFO_LINE_H);
    let c = draw_tip(&theme, &text, label);
    assert_eq!((c.width as i32, c.height as i32), (w, h));
    // Line 2 begins exactly one pitch below line 1's own ink row (found
    // rather than assumed, since a font's ascent nudges it down from the
    // padding edge), and line 3 one pitch below that; the row right
    // above each is blank.
    let has_ink = |y: i32| (1..w - 1).any(|x| c.get(x, y) == TIP_INK.argb());
    let line1 = (1..h - 1).find(|&y| has_ink(y)).expect("line 1 has ink");
    assert!(
        !has_ink(line1 + INFO_LINE_H - 1),
        "gap above line 2 is blank"
    );
    assert!(
        has_ink(line1 + INFO_LINE_H),
        "line 2 starts at the line pitch"
    );
    assert!(
        has_ink(line1 + 2 * INFO_LINE_H),
        "line 3 starts two pitches down"
    );
}

#[test]
fn a_grown_tip_pitches_its_lines_with_the_text() {
    let text = TextRenderer::load(&FontConfig::default());
    let label = "Local Area Connection\nStatus: Connected";
    let base = Theme::default();
    let grown = base.with_text_size(200);
    let g = grown.line_growth();
    assert!(g > 0, "a 200% line outgrows the 100% one");
    let (_, h100) = tip_size(&text, &base, label);
    let (w, h) = tip_size(&text, &grown, label);
    let px = (grown.fonts.size_px - base.fonts.size_px) as i32;
    assert_eq!(
        h,
        h100 + px + g,
        "the first line grows with the font, the next by the pitch"
    );
    let c = draw_tip(&grown, &text, label);
    let has_ink = |y: i32| (1..w - 1).any(|x| c.get(x, y) == TIP_INK.argb());
    let line1 = (1..h - 1).find(|&y| has_ink(y)).expect("line 1 has ink");
    let pitch = INFO_LINE_H + g;
    assert!(!has_ink(line1 + pitch - 1), "a blank row above line 2");
    assert!(has_ink(line1 + pitch), "line 2 starts a grown pitch down");

    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let glass200 = glass.with_text_size(200);
    let g = glass200.line_growth();
    let (_, h100) = tip_size(&text, &glass, label);
    let (_, h) = tip_size(&text, &glass200, label);
    assert_eq!(
        h,
        h100 + 2 * g,
        "a look's box grows every line by the pitch"
    );
    let body = "Opens the settings.";
    assert_eq!(
        info_size(&text, &glass200, body).1,
        info_size(&text, &glass, body).1 + g
    );
    let lines = [
        String::from("Type: Text Document"),
        String::from("Size: 1 KB"),
    ];
    assert_eq!(
        crate::controls::infotip::size(&text, &glass200, &lines).1,
        crate::controls::infotip::size(&text, &glass, &lines).1 + 2 * g
    );
}

#[test]
fn a_grown_balloon_tip_grows_its_title_and_lines() {
    let text = TextRenderer::load(&FontConfig::default());
    let body = "Closes your programs.";
    let base = Theme::default();
    let grown = base.with_text_size(200);
    let g = grown.line_growth();
    let (_, h100) = balloon_size(&text, &base, "Log Off", body);
    let (_, h) = balloon_size(&text, &grown, "Log Off", body);
    assert_eq!(h, h100 + 2 * g, "the title's gap and the body line grow");

    let glass = crate::interfaces::theme_for(crate::interface::Interface::Glass);
    let glass200 = glass.with_text_size(200);
    let (b, b200) = (
        balloon_look(&glass).unwrap(),
        balloon_look(&glass200).unwrap(),
    );
    assert_eq!(b200.title_px, 2 * b.title_px);
    assert_eq!(b200.body_y, b.body_y + b.title_px as i32);
    assert_eq!(b200.line_h, b.line_h + glass200.line_growth());
    assert_eq!(
        balloon_look(&glass.with_text_size(100)),
        Some(b),
        "100% keeps the look's own"
    );
}

#[test]
fn a_tip_draws_at_most_tip_lines_of_tip_chars() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let tall = "x\n".repeat(10_000);
    let (_, h) = tip_size(&text, &theme, &tall);
    assert_eq!(h, 17 + (TIP_LINES as i32 - 1) * INFO_LINE_H);
    let wide = "x".repeat(TIP_CHARS + 10_000);
    let (w, _) = tip_size(&text, &theme, &wide);
    assert_eq!(w, text.measure(&wide[..TIP_CHARS], false, 11) + 6);
    let c = draw_tip(&theme, &text, &tall);
    assert_eq!(c.height as i32, h);
}

#[test]
fn placed_above_the_pointer_and_kept_on_screen() {
    let screen = Rect::new(0, 0, 1920, 1080);
    // Near the bottom-left corner the box starts one pixel left of
    // the pointer, its bottom `ABOVE_POINTER` above it.
    assert_eq!(place((28, 1066), (94, 17), screen, false), (27, 1041));
    // Near the right edge the box is pushed left to end one pixel
    // before the screen's last column.
    assert_eq!(place((1885, 1066), (156, 17), screen, false), (1763, 1041));
    // Never left of, or above, the screen.
    assert_eq!(place((0, 5), (50, 17), screen, false), (0, 0));
    // Follows the output's origin.
    let second = Rect::new(1920, 0, 1920, 1080);
    assert_eq!(place((1925, 1066), (50, 17), second, false), (1924, 1041));
}

#[test]
fn on_a_top_bar_a_tip_opens_under_the_pointer() {
    let screen = Rect::new(0, 0, 1920, 1080);
    // Its top 21 px under the hotspot, one pixel left of the pointer.
    assert_eq!(place((28, 14), (94, 17), screen, true), (27, 35));
    assert_eq!(
        place((1885, 14), (156, 17), screen, true),
        (1763, 35),
        "pushed left off the right edge"
    );
    assert_eq!(
        place((40, 1075), (50, 17), screen, true),
        (39, 1063),
        "kept above the screen's foot"
    );
}

#[test]
fn a_tip_on_a_side_bar_stands_beside_it_level_with_the_pointer() {
    let screen = Rect::new(0, 0, 1920, 1080);
    assert_eq!(
        place_beside((30, 500), 60, true, (120, 20), screen),
        (62, 490),
        "2 px right of a left bar, its middle at the pointer"
    );
    assert_eq!(
        place_beside((1890, 500), 1860, false, (120, 20), screen),
        (1738, 490),
        "2 px left of a right bar"
    );
    assert_eq!(
        place_beside((30, 1075), 60, true, (120, 20), screen).1,
        1060,
        "kept above the output's foot"
    );
    assert_eq!(
        place_beside((30, 500), 60, true, (1900, 20), screen).0,
        19,
        "pushed left to end one pixel before the screen's last column"
    );
}

#[test]
fn task_tip_only_when_the_title_was_cut() {
    let text = TextRenderer::load(&FontConfig::default());
    assert_eq!(task_tip(&text, 11, "Short", false, 160), None);
    let long = "A window title far too long for a taskbar button to show";
    assert_eq!(
        task_tip(&text, 11, long, false, 160),
        Some(long.to_string())
    );
    assert_eq!(task_tip(&text, 11, long, true, 160), Some(long.to_string()));
}

fn balloon(theme: &Theme, text: &TextRenderer, title: &str, body: &str) -> Canvas {
    let lay = notify_layout(theme, text, title, body, &[], &[], None);
    draw_notify_balloon(theme, text, &lay, None, &[], None, None, None)
}

fn notify_height(text: &TextRenderer, theme: &Theme, body: &str) -> u32 {
    balloon(theme, text, "Hi", body).height
}

/// A body of bare newlines would size the balloon by its line count, so
/// the count is capped; past the cap the balloon stops growing.
#[test]
fn notify_body_lines_are_capped() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let at_cap = notify_height(&t, &theme, &"x\n".repeat(NOTIFY_BODY_LINES));
    let past = notify_height(&t, &theme, &"x\n".repeat(100_000));
    assert_eq!(at_cap, past);
}

#[test]
fn a_taller_notify_body_makes_a_taller_balloon() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let long = "one two three four five six seven eight nine ten eleven twelve";
    let short = balloon(&theme, &t, "Hi", "one line");
    let tall = balloon(&theme, &t, "Hi", long);
    assert_eq!((short.width, tall.width), (300, 300));
    assert!(
        tall.height > short.height,
        "{} !> {}",
        tall.height,
        short.height
    );
}

#[test]
fn the_notify_balloon_is_tooltip_yellow_with_clear_corners() {
    let t = TextRenderer::load(&FontConfig::default());
    let c = balloon(&Theme::default(), &t, "Update", "Ready.");
    assert_eq!(c.get(NOTIFY_W / 2, 20), TIP_BG.argb());
    assert_eq!(c.get(0, 0), 0);
}

#[test]
fn a_scale_2_notify_balloon_sits_in_the_same_logical_corner() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let usable = Rect::new(0, 0, 1920, 1050);
    let one = notify_layout(&theme, &t, "Update", "Ready.", &[], &[], None);
    let want = (1920 - 300 - 8, 1050 - one.height - 2);
    assert_eq!(place_notify(usable, &one, 1, Edge::Bottom), want);
    let two = notify_layout(&theme.scaled(2), &t, "Update", "Ready.", &[], &[], None);
    assert_eq!(place_notify(usable, &two, 2, Edge::Bottom), want);
}

#[test]
fn under_a_top_bar_the_notify_balloon_sits_top_right_its_tail_rising() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let usable = Rect::new(0, 30, 1920, 1050);
    let bottom = notify_layout(&theme, &t, "Update", "Ready.", &["Open"], &[], None);
    let top = bottom.clone().on_edge(Edge::Top);
    assert_eq!(
        place_notify(usable, &top, 1, Edge::Top),
        (1612, 32),
        "8 px in from the right, 2 px under the bar"
    );
    assert_eq!(top.height, bottom.height);
    let dy = NOTIFY_TAIL_H;
    assert_eq!(
        top.buttons[0],
        Rect::new(
            bottom.buttons[0].x,
            bottom.buttons[0].y + dy,
            BUTTON_W,
            BUTTON_H
        )
    );
    assert_eq!(top.close.y, bottom.close.y + dy);
    let b = top.buttons[0];
    assert_eq!(notify_hit(&top, b.x + 3, b.y + 3), NotifyHit::Button(0));
    let up = draw_notify_balloon(&theme, &t, &top, None, &[], None, None, None);
    let down = draw_notify_balloon(&theme, &t, &bottom, None, &[], None, None, None);
    // The bubble inside its top and bottom border rows, 12 px lower.
    for y in 1..bottom.bubble_h - 1 {
        for x in 0..NOTIFY_W {
            assert_eq!(up.get(x, y + dy), down.get(x, y), "bubble ({x}, {y})");
        }
    }
    // The tail turned over above the bubble.
    for r in 0..dy {
        for x in 0..NOTIFY_W {
            assert_eq!(
                up.get(x, dy - 1 - r),
                down.get(x, bottom.bubble_h + r),
                "tail row {r} at x {x}"
            );
        }
    }
    let seam = NOTIFY_W - NOTIFY_PAD - NOTIFY_TAIL_W + 5;
    assert_eq!(
        up.get(seam, dy),
        TIP_BG.argb(),
        "the bubble's top border opens where the tail joins"
    );
}

#[test]
fn action_buttons_sit_right_aligned_in_a_row_under_the_body() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let plain = notify_layout(&theme, &t, "Mail", "New message", &[], &[], None);
    let lay = notify_layout(
        &theme,
        &t,
        "Mail",
        "New message",
        &["Open", "Reply"],
        &[],
        None,
    );
    // The row adds its 8 px gap and a 23 px button to the bubble.
    assert_eq!(lay.bubble_h, plain.bubble_h + NOTIFY_ROW_GAP + BUTTON_H);
    assert_eq!(lay.height, lay.bubble_h + NOTIFY_TAIL_H);
    assert_eq!(lay.more, None);
    let [open, reply] = lay.buttons[..] else {
        panic!("two buttons: {:?}", lay.buttons);
    };
    // Short labels get the standard 75x23 button, 6 px apart, the last
    // one 10 px in from the right edge and 10 px up from the bubble's
    // bottom.
    assert_eq!((open.width, open.height), (BUTTON_W, BUTTON_H));
    assert_eq!(reply.right(), NOTIFY_W - NOTIFY_PAD);
    assert_eq!(reply.x - open.right(), BUTTON_GAP);
    assert_eq!(reply.bottom(), lay.bubble_h - NOTIFY_PAD);
    assert_eq!(
        notify_hit(&lay, open.x + 3, open.y + 3),
        NotifyHit::Button(0)
    );
    assert_eq!(
        notify_hit(&lay, reply.x + 3, reply.y + 3),
        NotifyHit::Button(1)
    );
    assert_eq!(notify_hit(&lay, 60, 30), NotifyHit::Body);
    assert_eq!(
        notify_hit(&lay, lay.close.x + 4, lay.close.y + 4),
        NotifyHit::Close
    );

    // The pushed button's outer ring is window-frame black; the other
    // keeps its raised white top edge.
    let c = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        Some(NotifyHit::Button(0)),
        None,
        None,
    );
    let cs = &theme.colors;
    assert_eq!(c.get(open.x + 5, open.y), cs.window_frame.argb());
    assert_eq!(c.get(reply.x + 5, reply.y), cs.button_hilight.argb());
}

/// An opaque 16 px square at `s` device px per px, for an action's icon.
fn swatch(s: i32) -> Canvas {
    let side = (crate::look::widgets::BUTTON_ICON * s) as u32;
    let mut c = Canvas::new(side, side);
    c.fill_rect(c.bounds(), Color::rgb(0xFA, 12, 0));
    c
}

#[test]
fn an_action_with_an_icon_gets_an_icon_button_sized_by_the_icon() {
    let t = TextRenderer::load(&FontConfig::default());
    for s in [1, 2] {
        let theme = Theme::default().scaled(s);
        let acts = ["media-playback-start", "Later"];
        let label = notify_layout(&theme, &t, "Song", "Next up", &acts, &[], None);
        let lay = notify_layout(&theme, &t, "Song", "Next up", &acts, &[true], None);
        let [play, later] = lay.buttons[..] else {
            panic!("two buttons: {:?}", lay.buttons);
        };
        assert_eq!(play.width, ICON_BUTTON_W * s, "the icon button at {s}");
        assert_eq!(
            later.width, label.buttons[1].width,
            "the labelled one at {s}"
        );
        assert_eq!(later.right(), (NOTIFY_W - NOTIFY_PAD) * s);
        assert_eq!(later.x - play.right(), BUTTON_GAP * s);

        let icon = swatch(s);
        let c = draw_notify_balloon(&theme, &t, &lay, None, &[Some(icon)], None, None, None);
        let side = crate::look::widgets::BUTTON_ICON * s;
        let (x, y) = (
            play.x + (play.width - side) / 2,
            play.y + (play.height - side) / 2,
        );
        assert_eq!(c.get(x, y), 0xFFFA_0C00, "the icon's top-left at {s}");
        assert_eq!(
            c.get(x + side - 1, y + side - 1),
            0xFFFA_0C00,
            "its bottom-right at {s}"
        );
        assert_ne!(c.get(x - 1, y), 0xFFFA_0C00, "nothing left of it at {s}");
    }
}

#[test]
fn a_label_wider_than_the_row_takes_a_row_of_its_own_cut_to_it() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let long = "a label long enough to overrun the whole balloon row on its own";
    let acts = ["call-stop", long, "Later"];
    let lay = notify_layout(&theme, &t, "Hi", "Body", &acts, &[true], None);
    let [icon, cut, later] = lay.buttons[..] else {
        panic!("three buttons: {:?}", lay.buttons);
    };
    let room = NOTIFY_W - 2 * NOTIFY_PAD;
    assert_eq!(icon.width, ICON_BUTTON_W);
    assert_eq!(icon.right(), NOTIFY_W - NOTIFY_PAD);
    assert_eq!(
        cut,
        Rect::new(NOTIFY_PAD, icon.bottom() + BUTTON_GAP, room, BUTTON_H)
    );
    assert_eq!(later.y, cut.bottom() + BUTTON_GAP, "Later on a third row");
    assert_eq!(later.right(), NOTIFY_W - NOTIFY_PAD);
    assert_eq!(lay.labels[0], None, "the icon button shows its icon");
    assert!(lay.labels[1].as_ref().is_some_and(|l| l.ends_with("...")));
    assert_eq!(lay.labels[2].as_deref(), Some("Later"));
}

#[test]
fn a_fourth_action_brings_the_more_button() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let acts = ["One", "Two", "Three", "Four", "Five"];
    let lay = notify_layout(&theme, &t, "Hi", "Body", &acts, &[], None);
    assert_eq!(lay.buttons.len(), NOTIFY_BUTTONS);
    let more = lay.more.expect("a ... button");
    assert_eq!((more.width, more.height), (BUTTON_H, BUTTON_H));
    assert_eq!(more.right(), NOTIFY_W - NOTIFY_PAD);
    assert_eq!(notify_hit(&lay, more.x + 2, more.y + 2), NotifyHit::More);
    assert!(lay.buttons.iter().all(|b| b.width == BUTTON_W));
    assert_eq!(more.x - lay.buttons[2].right(), BUTTON_GAP);
}

#[test]
fn flowed_buttons_fill_rows_right_aligned_in_order() {
    let at = |x, width, row| Flowed { x, width, row };
    assert_eq!(flow_buttons(&[], 100, 6), []);
    assert_eq!(
        flow_buttons(&[30, 40], 100, 6),
        [at(24, 30, 0), at(60, 40, 0)]
    );
    assert_eq!(
        flow_buttons(&[30, 40, 50], 100, 6),
        [at(24, 30, 0), at(60, 40, 0), at(50, 50, 1)],
        "the third would overrun the row"
    );
    assert_eq!(
        flow_buttons(&[30, 140, 20], 100, 6),
        [at(70, 30, 0), at(0, 100, 1), at(80, 20, 2)],
        "one wider than the room is cut to it, alone on its row"
    );
}

const LONG_ACTS: [&str; 4] = [
    "Open the disk",
    "Show Disk Usage",
    "Empty Recycle Bin",
    "Remind me tomorrow",
];

/// Each button's rect, `...` last, and the rows they stand on.
fn button_rows(lay: &NotifyLayout) -> (Vec<Rect>, usize) {
    let all: Vec<Rect> = lay.buttons.iter().copied().chain(lay.more).collect();
    let mut ys: Vec<i32> = all.iter().map(|b| b.y).collect();
    ys.dedup();
    (all, ys.len())
}

#[test]
fn long_labels_show_whole_on_wrapped_rows_in_every_balloon_look_size_and_scale() {
    use crate::interface::Interface;
    for i in [Interface::Classic, Interface::Blue] {
        for pct in [100, 125, 150, 200] {
            for s in [1, 2] {
                let theme = crate::interfaces::theme_for(i)
                    .with_text_size(pct)
                    .scaled(s);
                let t = TextRenderer::load(&theme.fonts);
                let (pad, mut rows_before) = (NOTIFY_PAD * s, 0);
                for n in 1..=LONG_ACTS.len() {
                    let acts = &LONG_ACTS[..n];
                    let lay = notify_layout(&theme, &t, "Low Disk Space", "Body", acts, &[], None);
                    let at = format!("{i:?} {pct}% x{s}, {n} actions");
                    let shown = n.min(NOTIFY_BUTTONS);
                    let whole: Vec<Option<String>> =
                        acts[..shown].iter().map(|a| Some(a.to_string())).collect();
                    assert_eq!(lay.labels, whole, "{at}");
                    assert_eq!(lay.more.is_some(), n > NOTIFY_BUTTONS, "{at}");
                    let (all, rows) = button_rows(&lay);
                    for (k, b) in all.iter().enumerate() {
                        assert!(
                            b.x >= pad
                                && b.right() <= lay.bubble_w - pad
                                && b.bottom() <= lay.bubble_h - pad,
                            "{at}: button {k} {b:?} inside the bubble"
                        );
                        assert!(
                            all[k + 1..].iter().all(|o| !b.intersects(*o)),
                            "{at}: button {k} overlaps a later one"
                        );
                    }
                    assert!(rows >= rows_before, "{at}: {rows} rows");
                    rows_before = rows;
                }
                assert!(rows_before >= 2, "{i:?} {pct}% x{s}: the long set wraps");
            }
        }
    }
}

#[test]
fn the_reply_row_takes_the_first_of_several_rows_and_the_size_stays() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let acts = &LONG_ACTS[..3];
    let row = notify_layout(&theme, &t, "Ana", "Lunch?", acts, &[], None);
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", acts, &[], Some(REPLY));
    assert_eq!(button_rows(&row).1, 2);
    assert_eq!(lay.height, row.height);
    assert_eq!(lay.send.map(|b| b.y), Some(row.buttons[0].y));
}

fn fnv(c: &Canvas) -> u64 {
    [c.width, c.height]
        .iter()
        .chain(c.pixels.iter())
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, &p| {
            p.to_le_bytes().iter().fold(h, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
        })
}

const WRAPPED_PINS: [u64; 6] = [
    0x73acc913fd881244,
    0x798b5b5a7a48f6e0,
    0x4ca44806f82bf3a8,
    0x0c239197338ab164,
    0xd5ac7b09480b9329,
    0xf26de3d28d2f147b,
];

/// Three long buttons on two rows: Classic and Blue at scale 1 and 2, and at 200% text.
#[test]
fn wrapped_button_rows_pins() {
    use crate::interface::Interface;
    crate::pins::fonts();
    let mut got = Vec::new();
    for i in [Interface::Classic, Interface::Blue] {
        for (pct, s) in [(100, 1), (100, 2), (200, 1)] {
            let theme = crate::interfaces::theme_for(i)
                .with_text_size(pct)
                .scaled(s);
            let t = TextRenderer::load(&theme.fonts);
            let lay = notify_layout(
                &theme,
                &t,
                "Low Disk Space",
                "The disk of / has only 812 MB free of 20.0 GB.",
                &LONG_ACTS[..3],
                &[],
                None,
            );
            let hot = Some(NotifyHit::Button(2));
            let c = draw_notify_balloon(&theme, &t, &lay, None, &[], None, hot, None);
            got.push((format!("{i:?} {pct}% x{s}"), fnv(&c)));
        }
    }
    let now: Vec<String> = got.iter().map(|(_, h)| format!("{h:#018x},")).collect();
    for (i, (case, h)) in got.iter().enumerate() {
        assert_eq!(*h, WRAPPED_PINS[i], "{case}; now:\n{}", now.join("\n"));
    }
}

/// Blue's tip, in every variant, is a black 1 px frame round info_bk
/// with its ink from 3 px in and 4 px down.
#[test]
fn a_blue_tip_is_black_round_its_info_colour_in_every_variant() {
    use crate::theme::Color;
    for v in crate::blue::Variant::ALL {
        let theme = crate::interfaces::blue_theme(v);
        let text = TextRenderer::load(&theme.fonts);
        let c = draw_tip(&theme, &text, "Tuesday");
        let (w, h) = (c.width as i32, c.height as i32);
        let black = Color::rgb(0, 0, 0).argb();
        for (x, y) in [(0, 0), (w - 1, h - 1), (0, h / 2), (w / 2, 0)] {
            assert_eq!(c.get(x, y), black, "{v:?}: the frame at {x},{y}");
        }
        assert_eq!(c.get(1, 1), theme.colors.info_bk.argb(), "{v:?}: the fill");
        crate::pins::fonts();
        assert_eq!(h, 17, "{v:?}: 17 px tall");
        let first = (1..w - 1).find(|&x| (1..h - 1).any(|y| c.get(x, y) == black));
        let top = (1..h - 1).find(|&y| (1..w - 1).any(|x| c.get(x, y) == black));
        assert_eq!(
            (first, top),
            (Some(3), Some(4)),
            "{v:?}: the ink starts 3 px in, 4 down"
        );
    }
}

#[test]
fn beside_a_side_bar_the_balloon_sits_in_the_foot_corner_by_the_tray() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = notify_layout(&theme, &t, "Update", "Ready.", &["Open"], &[], None);
    let left = lay.clone().on_edge(Edge::Left);
    assert_eq!(
        (left.bubble_y, left.close, left.buttons.clone()),
        (lay.bubble_y, lay.close, lay.buttons.clone()),
        "the tail hangs under the bubble, as over a bottom bar"
    );
    let usable = Rect::new(60, 0, 1860, 1080);
    assert_eq!(
        place_notify(usable, &left, 1, Edge::Left),
        (68, 1080 - lay.height - 2),
        "8 px off the bar, 2 px up from the foot"
    );
    let usable = Rect::new(0, 0, 1860, 1080);
    assert_eq!(
        place_notify(usable, &lay, 1, Edge::Right),
        (1860 - lay.width - 8, 1080 - lay.height - 2)
    );
}
const REPLY: NotifyReply = NotifyReply {
    placeholder: "Write a reply",
    send: "Send",
};

/// The expanded reply row takes the button row's place at its height:
/// a 21 px field from the left padding to 6 px short of a 75x23 Send
/// button that ends 10 px in from the right.
#[test]
fn the_reply_row_stands_where_the_buttons_stood() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let acts = ["Reply", "Mark as read"];
    let row = notify_layout(&theme, &t, "Ana", "Lunch?", &acts, &[], None);
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &acts, &[], Some(REPLY));
    assert_eq!(lay.height, row.height, "same size either way");
    assert!(lay.buttons.is_empty() && lay.more.is_none());
    let send = lay.send.expect("a send button");
    assert_eq!(
        send,
        Rect::new(
            NOTIFY_W - NOTIFY_PAD - BUTTON_W,
            row.buttons[0].y,
            BUTTON_W,
            BUTTON_H
        )
    );
    let field = lay.field.expect("a field");
    assert_eq!(
        field,
        Rect::new(NOTIFY_PAD, send.y + 1, send.x - BUTTON_GAP - NOTIFY_PAD, 21)
    );
    assert_eq!(notify_hit(&lay, field.x + 5, field.y + 5), NotifyHit::Field);
    assert_eq!(notify_hit(&lay, send.x + 5, send.y + 5), NotifyHit::Send);
    let top = lay.clone().on_edge(Edge::Top);
    assert_eq!(top.field.map(|f| f.y), Some(field.y + NOTIFY_TAIL_H));
    assert_eq!(top.send.map(|b| b.y), Some(send.y + NOTIFY_TAIL_H));
}

/// An empty field shows the grey hint and a greyed Send; typed text
/// replaces the hint in button-text ink, Send comes alive, and a
/// focused field draws its caret in the blink's shown half only.
#[test]
fn the_reply_field_shows_its_hint_until_there_is_text() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let cs = &theme.colors;
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
    let (field, send) = (lay.field.unwrap(), lay.send.unwrap());
    let inks = |c: &Canvas, r: Rect, ink: crate::theme::Color| {
        (r.x..r.right()).any(|x| (r.y..r.bottom()).any(|y| c.get(x, y) == ink.argb()))
    };
    let inside = field.inset(3);
    let empty = TextEdit::default();
    let c = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&empty, Caret::Unfocused, None)),
    );
    assert!(inks(&c, inside, cs.gray_text), "the hint is grey");
    assert!(
        !inks(&c, inside, cs.button_text),
        "and nothing else inks it"
    );
    assert_eq!(
        c.get(field.x + 2, field.y + 2),
        cs.window.argb(),
        "a white field"
    );
    assert!(!inks(&c, send, cs.button_text), "Send is greyed");

    let typed = TextEdit::new("Sure", false);
    let c = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&typed, Caret::Shown, None)),
    );
    assert!(inks(&c, inside, cs.button_text), "the text inks the field");
    assert!(!inks(&c, inside, cs.gray_text), "the hint has gone");
    assert!(inks(&c, send, cs.button_text), "Send is live");
    // The caret: a 1 px bar right after the text.
    let w = t.measure("Sure", false, theme.fonts.size_px);
    let cx = field.x + 3 + w;
    let bar = |c: &Canvas| (field.y..field.bottom()).any(|y| c.get(cx, y) == cs.button_text.argb());
    assert!(bar(&c), "the caret shows");
    let hidden = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&typed, Caret::Hidden, None)),
    );
    assert!(!bar(&hidden), "the blink's hidden half");
}

/// An input method's preedit draws underlined after the typed text,
/// and the focused field reports its spot for the candidate popup;
/// an unfocused one shows no preedit and reports nothing.
#[test]
fn the_reply_field_shows_the_preedit_and_reports_its_spot() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let ink = theme.colors.button_text.argb();
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
    let field = lay.field.unwrap();
    let typed = TextEdit::new("ok", false);
    let preedit = Preedit::new("nihao".into(), 5, 5);
    let draw = |caret| {
        draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            &[],
            None,
            None,
            Some((&typed, caret, preedit.as_ref())),
        )
    };
    let mut c = draw(Caret::Hidden);
    let spot = c.take_spot().expect("a focused field reports its spot");
    let px = theme.fonts.size_px;
    let (ok, all) = (t.measure("ok", false, px), t.measure("oknihao", false, px));
    assert_eq!(
        spot.caret.x,
        field.x + 3 + all,
        "the caret ends the preedit"
    );
    assert!(field.contains(spot.caret.x, spot.caret.y));
    let underline = (field.y..field.bottom())
        .any(|y| (field.x + 3 + ok..field.x + 3 + all).all(|x| c.get(x, y) == ink));
    assert!(underline, "the preedit is underlined");

    let mut idle = draw(Caret::Unfocused);
    assert_eq!(idle.take_spot(), None, "an unfocused field reports nothing");
    let plain = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&typed, Caret::Unfocused, None)),
    );
    assert!(idle.pixels == plain.pixels, "and draws no preedit");

    let empty = TextEdit::default();
    let first = draw_notify_balloon(
        &theme,
        &t,
        &lay,
        None,
        &[],
        None,
        None,
        Some((&empty, Caret::Shown, preedit.as_ref())),
    );
    let inside = field.inset(3);
    let grey = theme.colors.gray_text.argb();
    let hint = (inside.x..inside.right())
        .any(|x| (inside.y..inside.bottom()).any(|y| first.get(x, y) == grey));
    assert!(!hint, "the hint goes while a preedit is composed");
}

/// Blue and Glass draw the reply row through their own field and
/// button parts at the same places: the field's inside is the window
/// colour and the send button covers the balloon's yellow.
#[test]
fn every_interface_draws_the_reply_row() {
    use crate::interface::Interface;
    for iface in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(iface);
        let t = TextRenderer::load(&theme.fonts);
        let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
        let field = lay.field.unwrap();
        let typed = TextEdit::default();
        let c = draw_notify_balloon(
            &theme,
            &t,
            &lay,
            None,
            &[],
            None,
            None,
            Some((&typed, Caret::Shown, None)),
        );
        let mid = (field.x + field.width / 2, field.y + field.height / 2);
        assert_eq!(
            c.get(mid.0, mid.1),
            theme.colors.window.argb(),
            "{iface:?}: the field's inside"
        );
        let send = lay.send.unwrap();
        assert_ne!(
            c.get(send.x + send.width / 2, send.y + 3),
            theme.colors.info_bk.argb(),
            "{iface:?}: the send button covers the balloon"
        );
    }
}

#[test]
fn enter_sends_the_trimmed_reply_and_escape_collapses() {
    let mut f = ReplyField::default();
    assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::None, "nothing typed");
    f.key(ReplyKey::Edit(EditKey::Insert("  ".into())));
    assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::None, "only blanks");
    assert!(f.paste("on my way\nsecond line"));
    assert_eq!(f.edit.text, "  on my way");
    assert_eq!(f.key(ReplyKey::Enter), ReplyEvent::Send("on my way".into()));
    assert_eq!(f.key(ReplyKey::Escape), ReplyEvent::Collapse);
}

/// A click focuses the field and puts the caret at the boundary
/// nearest the pointer: before the text left of it, after it right.
#[test]
fn a_click_on_the_field_focuses_it_and_places_the_caret() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
    let field = lay.field.unwrap();
    let mut f = ReplyField {
        edit: TextEdit::new("Sure thing", false),
        focused: false,
    };
    f.click(&theme, &t, &lay, field.x + 1);
    assert!(f.focused);
    assert_eq!(f.edit.caret(), 0);
    f.click(&theme, &t, &lay, field.right() - 2);
    assert_eq!(f.edit.caret(), "Sure thing".len());
    let w = t.measure("Sure", false, theme.fonts.size_px);
    f.click(&theme, &t, &lay, field.x + FIELD_PAD + w);
    assert_eq!(f.edit.caret(), 4);
}

#[test]
fn a_right_press_keeps_the_selection_under_it_and_moves_the_caret_elsewhere() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
    let field = lay.field.unwrap();
    let mut f = ReplyField {
        edit: TextEdit::new("Sure thing", false),
        focused: false,
    };
    f.edit.set_selection(0, 4);
    let w = t.measure("Su", false, theme.fonts.size_px);
    f.press_for_menu(|f| f.click(&theme, &t, &lay, field.x + FIELD_PAD + w));
    assert!(f.focused);
    assert_eq!(f.edit.selection(), 0..4, "a press on \"Sure\" keeps it");
    f.press_for_menu(|f| f.click(&theme, &t, &lay, field.right() - 2));
    assert_eq!(
        (f.edit.selection(), f.edit.caret()),
        (10..10, 10),
        "a press past it moves the caret to the end"
    );
}

use crate::access::ROOT;
use crate::access::accesskit::{Action, Live, Role};

fn parts(tree: &crate::access::Tree, id: crate::access::accesskit::NodeId) -> Vec<(Role, String)> {
    let n = tree.get(id).unwrap();
    n.children()
        .iter()
        .map(|&c| {
            let n = tree.get(c).unwrap();
            let name = n.label().or(n.value()).unwrap_or_default();
            (n.role(), name.to_string())
        })
        .collect()
}

#[test]
fn the_balloon_is_a_live_alert_naming_its_parts() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let acts = ["Open", "Save & Archive", "Later", "Snooze"];
    let body = "Lunch at noon? The usual place, unless it rains again today.";
    let lay = notify_layout(&theme, &t, "Ana", body, &acts, &[], None);
    let says = NotifySays {
        app: "Chat",
        title: "Ana",
        body,
        actions: &acts,
        opens: true,
        urgent: false,
    };
    let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
    let px = theme.fonts.size_px;
    let alert = access_notify(1, &lay, &says, None, &t, px, &mut tree, ROOT);
    let n = tree.get(alert).unwrap();
    assert_eq!(n.role(), Role::Alert);
    assert_eq!(n.label(), Some(format!("Chat: Ana. {body}").as_str()));
    assert_eq!(n.live(), Some(Live::Polite));
    let named = |r: Role, s: &str| (r, s.to_string());
    assert_eq!(
        parts(&tree, alert),
        [
            named(Role::Label, "Ana"),
            named(Role::Label, body),
            named(Role::Button, "Open"),
            named(Role::Button, "Save & Archive"),
            named(Role::Button, "Later"),
            named(Role::Button, "More actions"),
            named(Role::Button, "Close"),
        ]
    );
    let kids = n.children();
    let body_node = tree.get(kids[1]).unwrap();
    assert!(body_node.supports_action(Action::Click), "the body opens");
    let open = tree.get(kids[2]).unwrap();
    assert_eq!(open.bounds(), Some(crate::access::bounds(lay.buttons[0])));
    assert!(open.supports_action(Action::Click));
    let close = tree.get(kids[6]).unwrap();
    assert_eq!(close.bounds(), Some(crate::access::bounds(lay.close)));
    assert!(
        kids.iter()
            .all(|&k| tree.get(k).unwrap().live() == Some(Live::Off)),
        "only the alert is announced, not each part again"
    );
}

#[test]
fn an_urgent_balloon_is_assertive_and_follows_the_bubble() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = notify_layout(&theme, &t, "Battery low", "", &[], &[], None).on_edge(Edge::Top);
    let says = NotifySays {
        app: "",
        title: "Battery low",
        body: "",
        actions: &[],
        opens: false,
        urgent: true,
    };
    let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
    let alert = access_notify(1, &lay, &says, None, &t, 11, &mut tree, ROOT);
    let n = tree.get(alert).unwrap();
    assert_eq!(n.live(), Some(Live::Assertive));
    assert_eq!(n.label(), Some("Battery low"));
    assert_eq!(n.bounds().map(|b| b.y0), Some(f64::from(NOTIFY_TAIL_H)));
    assert_eq!(
        parts(&tree, alert),
        [
            (Role::Label, "Battery low".to_string()),
            (Role::Button, "Close".to_string())
        ]
    );
    let only_app = NotifySays {
        app: "Mail",
        title: " ",
        ..says
    };
    assert_eq!(only_app.spoken(), "Mail");
}

#[test]
fn the_reply_row_reads_as_a_field_and_send() {
    let t = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let lay = notify_layout(&theme, &t, "Ana", "Lunch?", &["Reply"], &[], Some(REPLY));
    let says = NotifySays {
        app: "Chat",
        title: "Ana",
        body: "Lunch?",
        actions: &["Reply"],
        opens: false,
        urgent: false,
    };
    let px = theme.fonts.size_px;
    let mut field = ReplyField::default();
    let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
    let alert = access_notify(1, &lay, &says, Some(&field), &t, px, &mut tree, ROOT);
    assert_eq!(
        tree.get(alert).unwrap().role(),
        Role::Alert,
        "not focused yet"
    );
    let kids = tree.get(alert).unwrap().children().to_vec();
    let input = tree.get(kids[2]).unwrap();
    assert_eq!(input.role(), Role::TextInput);
    assert_eq!(input.placeholder(), Some("Write a reply"));
    assert_eq!(
        input.bounds(),
        Some(crate::access::bounds(lay.field.unwrap()))
    );
    let send = tree.get(kids[3]).unwrap();
    assert_eq!((send.role(), send.label()), (Role::Button, Some("Send")));
    assert!(send.is_disabled(), "nothing to send");
    assert!(
        !tree.get(kids[1]).unwrap().supports_action(Action::Click),
        "a body with no default action does not offer a click"
    );

    field.focused = true;
    field.key(ReplyKey::Edit(EditKey::Insert("on my way".into())));
    let mut tree = crate::access::Tree::window("", (lay.width, lay.height));
    let focus = access_notify(1, &lay, &says, Some(&field), &t, px, &mut tree, ROOT);
    let input = tree.get(focus).unwrap();
    assert_eq!(input.role(), Role::TextInput);
    let run = tree.get(input.children()[0]).unwrap();
    assert_eq!(run.value(), Some("on my way"));
    let kids = tree.get(alert).unwrap().children().to_vec();
    assert!(
        !tree.get(kids[3]).unwrap().is_disabled(),
        "Send comes alive"
    );
}
