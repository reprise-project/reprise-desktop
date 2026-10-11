//! The About box. Client coordinates throughout: the 413x324 client area.

use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use crate::theme::Color;

pub const BANNER_W: i32 = 413;
pub const BANNER_H: i32 = 77;

/// The client area the box draws into; the compositor draws the frame and caption.
pub const CLIENT: (i32, i32) = (413, 324);

pub const OK: &str = "ok";

// Literals, not scheme colours: the banner is a picture, not a themed surface.
const BASE: Color = Color::rgb(98, 134, 225);
const PEAK: Color = Color::rgb(158, 186, 245);
const EDGE: Color = Color::rgb(112, 146, 230);
const ORANGE: Color = Color::rgb(247, 150, 56);
/// The bottom bar's gradient: blue at both ends, an orange plateau over x 194..226.
const BAR_LEFT: Color = Color::rgb(99, 126, 212);
const BAR_RIGHT: Color = Color::rgb(107, 140, 228);
const ORANGE_X: i32 = 210;

/// The banner's own strings. Every About box paints the same banner, so the wordmark and
/// copyright belong to the box, not `Fields`.
pub const BANNER_MARK: &str = "Reprise";
/// The banner copyright: years, then the organisation; never the string the body row shows.
pub const BANNER_COPYRIGHT: &str = "Copyright (C) 2026\nThe reprise authors";

/// The Reprise mark before the wordmark: its size, and the gap to the wordmark's ink.
const BANNER_LOGO: i32 = 24;
const BANNER_LOGO_GAP: i32 = 8;

const FLAT_X: i32 = 100;
const PEAK_X: i32 = 300;
const ORANGE_TOP: i32 = 72;
/// The copyright's left margin and rect tops that put its ink on banner rows 49 and 60.
const CR_X: i32 = 7;
const CR_Y: [i32; 2] = [47, 58];

/// Paint the banner: blue field with highlight, orange bar, copyright lower left, mark and
/// wordmark right.
pub fn banner(text: &TextRenderer, px: u32) -> Canvas {
    banner_wide(text, px, BANNER_W)
}

/// [`banner`] `w` px wide (at least [`BANNER_W`]): the right-hand falloffs stretch to the new
/// edge; everything left of the peak stays.
pub fn banner_wide(text: &TextRenderer, px: u32, w: i32) -> Canvas {
    let w = w.max(BANNER_W);
    let mut c = Canvas::new(w as u32, BANNER_H as u32);
    c.fill_rect(Rect::new(0, 0, FLAT_X, BANNER_H), BASE);
    c.hgradient(Rect::new(FLAT_X, 0, PEAK_X - FLAT_X, BANNER_H), BASE, PEAK);
    c.hgradient(Rect::new(PEAK_X, 0, w - PEAK_X, BANNER_H), PEAK, EDGE);
    let bar_h = BANNER_H - ORANGE_TOP;
    c.hgradient(
        Rect::new(0, ORANGE_TOP, ORANGE_X + 1, bar_h),
        BAR_LEFT,
        ORANGE,
    );
    c.hgradient(
        Rect::new(ORANGE_X, ORANGE_TOP, w - ORANGE_X, bar_h),
        ORANGE,
        BAR_RIGHT,
    );
    // Copyright, light on blue, two lines split on the newline.
    let white = Color::rgb(255, 255, 255);
    for (i, line) in BANNER_COPYRIGHT.lines().take(2).enumerate() {
        let r = Rect::new(CR_X, CR_Y[i], w - CR_X, px as i32 + 4);
        text.draw(&mut c, r, line, false, white, Align::Left, px);
    }
    let mark = Rect::new(PEAK_X - 120, 24, w - (PEAK_X - 120) - 12, 28);
    text.draw(&mut c, mark, BANNER_MARK, true, white, Align::Right, px * 2);
    // The mark before it, its foot on the wordmark's last inked row; the 24 px drawing inks
    // rows and columns 1..=22.
    let left = mark.right() - text.measure(BANNER_MARK, true, px * 2);
    let foot = text
        .baseline(mark, true, px * 2)
        .map_or(mark.bottom(), |b| b - 1);
    logo_at(
        &mut c,
        left - BANNER_LOGO_GAP - (BANNER_LOGO - 1),
        foot - (BANNER_LOGO - 2),
        white,
    );
    c
}

fn logo_at(c: &mut Canvas, x: i32, y: i32, ink: Color) {
    crate::art::logo::draw(c, x, y, BANNER_LOGO, ink);
}

const TEXT_X: i32 = 53;
const VALUE_X: i32 = 252;
const ICON_X: i32 = 13;
const ICON_Y: i32 = 91;
const ICON: i32 = 32;
const BANNER_Y: i32 = 1;
const SEP_X0: i32 = 53;
const SEP_X1: i32 = 403;
const SEP_Y: i32 = 240;
const OK_X: i32 = 330;
const OK_Y: i32 = 290;
// Baselines. The top three are about 16.5 px apart, rounding alternately to 17 and 16, so they
// are explicit.
const PRODUCT_BASE: i32 = 100;
const VERSION_BASE: i32 = 117;
const COPYRIGHT_BASE: i32 = 133;
const FACT_BASE: [i32; 3] = [182, 195, 214];
const MEMORY_BASE: i32 = 258;

const FACT_IDS: [&str; 3] = ["fact0", "fact1", "fact2"];

pub struct Fields {
    pub product: String,
    pub version: String,
    pub copyright: String,
    /// Three free rows. An empty entry leaves its row blank, so the separator does not move.
    pub facts: [String; 3],
    pub memory_label: String,
    /// Drawn at its own column, so a longer label cannot shove it.
    pub memory_value: String,
    /// 32x32, left of the product name.
    pub icon: Option<Canvas>,
}

/// A text row's height; `baseline_offset` must use the same one.
fn row_h(px: u32) -> i32 {
    px as i32 + 4
}

/// Row-top offset that puts a row's last inked pixel on its `*_BASE` row (the renderer's
/// baseline minus 1). 0 without a font.
fn baseline_offset(text: &TextRenderer, px: u32) -> i32 {
    text.baseline(Rect::new(0, 0, 200, row_h(px)), false, px)
        .map_or(0, |raw| raw - 1)
}

pub fn form(f: &Fields, text: &TextRenderer, px: u32) -> Form {
    let off = baseline_offset(text, px);
    let row = |base: i32| base - off;
    let line_h = row_h(px);
    let w = CLIENT.0 - TEXT_X;

    let mut items = vec![Item::picture(
        "banner",
        Rect::new(0, BANNER_Y, BANNER_W, BANNER_H),
        Some(banner(text, px)),
    )];
    if let Some(icon) = &f.icon {
        items.push(Item::picture(
            "icon",
            Rect::new(ICON_X, ICON_Y, ICON, ICON),
            Some(icon.clone()),
        ));
    }
    let text_row = |id, base, s: &str| Item::label(id, Rect::new(TEXT_X, row(base), w, line_h), s);
    items.push(text_row("product", PRODUCT_BASE, &f.product));
    items.push(text_row("version", VERSION_BASE, &f.version));
    items.push(text_row("copyright", COPYRIGHT_BASE, &f.copyright));
    for (i, fact) in f.facts.iter().enumerate() {
        items.push(text_row(FACT_IDS[i], FACT_BASE[i], fact));
    }
    items.push(Item::new(
        "sep",
        Rect::new(SEP_X0, SEP_Y, SEP_X1 - SEP_X0 + 1, 2),
        Control::Separator,
    ));
    items.push(Item::label(
        "memlabel",
        Rect::new(TEXT_X, row(MEMORY_BASE), VALUE_X - TEXT_X, line_h),
        &f.memory_label,
    ));
    items.push(Item::label(
        "memvalue",
        Rect::new(VALUE_X, row(MEMORY_BASE), CLIENT.0 - VALUE_X, line_h),
        &f.memory_value,
    ));
    items.push(Item::button(
        OK,
        Rect::new(OK_X, OK_Y, BUTTON_W, BUTTON_H),
        "OK",
        true,
        true,
    ));
    Form::new(items)
}

// The system view: the banner, then the distribution's logo left and the machine as a table of
// groups right, with Copy and OK under a separator. It grows down with the number of lines.

pub const SYSTEM_W: i32 = 500;
pub const COPY: &str = "copy";
pub const LOGO: i32 = 64;
const LOGO_X: i32 = 16;
const TABLE_Y: i32 = BANNER_Y + BANNER_H + 16;
const GROUP_X: i32 = 96;
const LINE_X: i32 = 204;
const RIGHT: i32 = 12;
const PITCH: i32 = 16;
const GROUP_GAP: i32 = 6;
/// The separator's distance under the last row, the buttons' under the separator, and the
/// bottom margin.
const SEP_GAP: i32 = 10;
const BUTTON_GAP: i32 = 12;
const BOTTOM: i32 = 11;
const BUTTON_SPACE: i32 = 6;

/// Item ids for group labels and lines: a view shows at most this many of each.
const GROUP_IDS: [&str; 12] = [
    "group0", "group1", "group2", "group3", "group4", "group5", "group6", "group7", "group8",
    "group9", "group10", "group11",
];
const LINE_IDS: [&str; 32] = [
    "line0", "line1", "line2", "line3", "line4", "line5", "line6", "line7", "line8", "line9",
    "line10", "line11", "line12", "line13", "line14", "line15", "line16", "line17", "line18",
    "line19", "line20", "line21", "line22", "line23", "line24", "line25", "line26", "line27",
    "line28", "line29", "line30", "line31",
];

/// One group of the table: its label beside the first of its lines. Strings are set in labels,
/// so a literal `&` is doubled.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub label: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct System {
    pub logo: Option<Canvas>,
    /// In order. A group with no lines, and anything past [`GROUP_IDS`] or [`LINE_IDS`], is
    /// left out.
    pub groups: Vec<Group>,
}

/// Each shown line's group index and row top, and the table's bottom.
fn system_rows(s: &System) -> (Vec<(usize, i32)>, i32) {
    let mut rows = Vec::new();
    let mut y = TABLE_Y;
    let shown = s.groups.iter().filter(|g| !g.lines.is_empty());
    for (gi, g) in shown.take(GROUP_IDS.len()).enumerate() {
        if gi > 0 {
            y += GROUP_GAP;
        }
        for _ in &g.lines {
            if rows.len() == LINE_IDS.len() {
                break;
            }
            rows.push((gi, y));
            y += PITCH;
        }
    }
    (rows, y.max(TABLE_Y + LOGO))
}

fn system_footer(s: &System) -> (i32, i32) {
    let (_, bottom) = system_rows(s);
    let sep = bottom + SEP_GAP;
    (sep, sep + 2 + BUTTON_GAP)
}

/// [`SYSTEM_W`] wide, as tall as its lines.
pub fn system_size(s: &System) -> (i32, i32) {
    let (_, buttons) = system_footer(s);
    (SYSTEM_W, buttons + BUTTON_H + BOTTOM)
}

/// Build the system view over its [`system_size`]. A line too long for its column ends in an
/// ellipsis.
pub fn system_form(s: &System, text: &TextRenderer, px: u32) -> Form {
    let (w, _) = system_size(s);
    let mut items = vec![Item::picture(
        "banner",
        Rect::new(0, BANNER_Y, w, BANNER_H),
        Some(banner_wide(text, px, w)),
    )];
    if let Some(logo) = &s.logo {
        items.push(Item::picture(
            "logo",
            Rect::new(LOGO_X, TABLE_Y, LOGO, LOGO),
            Some(logo.clone()),
        ));
    }
    let (rows, _) = system_rows(s);
    let groups: Vec<&Group> = s.groups.iter().filter(|g| !g.lines.is_empty()).collect();
    let line_w = w - RIGHT - LINE_X;
    let fit = |line: &str| {
        let shown = crate::text::split_mnemonic(line).0;
        let cut = text.ellipsize(&shown, line_w, false, px);
        if cut == shown {
            line.to_string()
        } else {
            cut.replace('&', "&&")
        }
    };
    let mut lines = groups.iter().flat_map(|g| g.lines.iter());
    let mut last_group = None;
    for (i, &(gi, y)) in rows.iter().enumerate() {
        if last_group != Some(gi) {
            last_group = Some(gi);
            items.push(Item::label(
                GROUP_IDS[gi],
                Rect::new(GROUP_X, y, LINE_X - GROUP_X, row_h(px)),
                &groups[gi].label,
            ));
        }
        let line = lines.next().map(|l| fit(l)).unwrap_or_default();
        items.push(Item::label(
            LINE_IDS[i],
            Rect::new(LINE_X, y, line_w, row_h(px)),
            &line,
        ));
    }
    let (sep, by) = system_footer(s);
    items.push(Item::new(
        "sep",
        Rect::new(RIGHT, sep, w - 2 * RIGHT, 2),
        Control::Separator,
    ));
    let ok_x = w - RIGHT - BUTTON_W;
    items.push(Item::button(
        COPY,
        Rect::new(ok_x - BUTTON_SPACE - BUTTON_W, by, BUTTON_W, BUTTON_H),
        "&Copy",
        true,
        false,
    ));
    items.push(Item::button(
        OK,
        Rect::new(ok_x, by, BUTTON_W, BUTTON_H),
        "OK",
        true,
        true,
    ));
    let mut form = Form::new(items);
    form.focus_id(OK);
    form
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::text::TextRenderer;
    use crate::theme::{Color, FontConfig, Theme};

    #[test]
    fn banner_is_413x77() {
        let text = TextRenderer::load(&FontConfig::default());
        let c = banner(&text, 11);
        assert_eq!(c.width, BANNER_W as u32);
        assert_eq!(c.height, BANNER_H as u32);
    }

    /// The Reprise mark stands white before the wordmark, its foot on the wordmark's last inked
    /// row, 8 px between them.
    #[test]
    fn the_mark_stands_before_the_wordmark_on_its_foot() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = 11;
        let c = banner(&text, px);
        let mark = Rect::new(PEAK_X - 120, 24, BANNER_W - (PEAK_X - 120) - 12, 28);
        let left = mark.right() - text.measure(BANNER_MARK, true, px * 2);
        let foot = text.baseline(mark, true, px * 2).unwrap() - 1;
        let (x, y) = (
            left - BANNER_LOGO_GAP - (BANNER_LOGO - 1),
            foot - (BANNER_LOGO - 2),
        );
        crate::art::logo::assert_drawn(&c, x, y, BANNER_LOGO, Color::rgb(255, 255, 255));
        assert_ne!(c.get(x + 3, foot + 1), 0xFFFF_FFFF, "banner under the stem");
    }

    /// The bottom bar occupies banner rows 72..76 and is a gradient: blue at both ends, orange
    /// over the plateau. Both ends are checked because the middle alone cannot tell it from
    /// flat orange.
    #[test]
    fn the_bottom_bar_is_a_gradient_over_the_bottom_five_rows() {
        const BAR_TOP: i32 = 72;
        const EXPECTED_LEFT: Color = Color::rgb(99, 126, 212);
        const EXPECTED_ORANGE: Color = Color::rgb(247, 150, 56);
        const EXPECTED_RIGHT: Color = Color::rgb(107, 140, 228);
        const PLATEAU_MID: i32 = 210;
        let text = TextRenderer::load(&FontConfig::default());
        let c = banner(&text, 11);
        for y in BAR_TOP..BANNER_H {
            assert_eq!(c.get(0, y), EXPECTED_LEFT.argb(), "row {y} is blue at x0");
            assert_eq!(
                c.get(PLATEAU_MID, y),
                EXPECTED_ORANGE.argb(),
                "row {y} is orange at the plateau"
            );
            assert_eq!(
                c.get(BANNER_W - 1, y),
                EXPECTED_RIGHT.argb(),
                "row {y} is blue at x412"
            );
        }
        // The blue field above differs from the bar's left end, so the bar starts on row 72.
        assert_ne!(c.get(0, BAR_TOP - 1), EXPECTED_LEFT.argb());
        // It ramps rather than steps: halfway to the plateau the red channel is far from orange.
        let red = |x: i32, y: i32| (c.get(x, y) >> 16) & 0xff;
        assert!(red(100, 74) < 200, "the bar ramps into the orange");
    }

    /// The three colour anchors across the banner; the ramp between them is not pinned.
    #[test]
    fn the_gradient_hits_its_anchor_colours() {
        const EXPECTED_BASE: Color = Color::rgb(98, 134, 225);
        const EXPECTED_PEAK: Color = Color::rgb(158, 186, 245);
        const EXPECTED_EDGE: Color = Color::rgb(112, 146, 230);
        let text = TextRenderer::load(&FontConfig::default());
        let c = banner(&text, 11);
        // Row 5 is above the text and below the top edge.
        assert_eq!(c.get(0, 5), EXPECTED_BASE.argb());
        assert_eq!(c.get(50, 5), EXPECTED_BASE.argb());
        assert_eq!(c.get(300, 5), EXPECTED_PEAK.argb());
        assert_eq!(c.get(BANNER_W - 1, 5), EXPECTED_EDGE.argb());
    }

    /// The copyright's two ink tops, banner rows 49 and 60, found by scanning the x7..45 band
    /// for the first inked row. A single column could land mid-glyph rather than on a capital's
    /// top.
    ///
    /// Each line is scanned on its own canvas: in the UI face line 1's descenders and line 2's
    /// ascenders touch, so no scan of the finished picture says where one stops. Line 1's top is
    /// also checked on the banner.
    #[test]
    fn copyright_ink_tops_land_on_rows_49_and_60() {
        if !crate::text::test_tahoma() {
            return;
        }
        const INK_TOP: [i32; 2] = [49, 60];
        const BLUE_BASE: Color = Color::rgb(98, 134, 225);
        let px = 11;
        let text = TextRenderer::load(&FontConfig::default());
        // The copyright's columns, all left of where the highlight starts.
        let first_ink =
            |c: &Canvas| (40..72).find(|&y| (7..46).any(|x| c.get(x, y) != BLUE_BASE.argb()));
        for (i, line) in BANNER_COPYRIGHT.lines().enumerate() {
            let mut c = Canvas::new(BANNER_W as u32, BANNER_H as u32);
            c.fill_rect(c.bounds(), BLUE_BASE);
            let r = Rect::new(CR_X, CR_Y[i], BANNER_W - CR_X, px + 4);
            let white = Color::rgb(255, 255, 255);
            text.draw(&mut c, r, line, false, white, Align::Left, px as u32);
            assert_eq!(
                first_ink(&c),
                Some(INK_TOP[i]),
                "copyright line {} starts on its expected row",
                i + 1
            );
        }
        assert_eq!(
            first_ink(&banner(&text, px as u32)),
            Some(INK_TOP[0]),
            "and the banner itself inks first on row 49"
        );
    }

    /// The two-line copyright spans rows 49..66 and must not reach the bar at row 72. The bar
    /// is a gradient, so check for ink: no white pixel in the copyright's columns over the
    /// bar's rows.
    #[test]
    fn the_copyright_does_not_bleed_into_the_bottom_bar() {
        const BAR_TOP: i32 = 72;
        let white = Color::rgb(255, 255, 255).argb();
        let text = TextRenderer::load(&FontConfig::default());
        let c = banner(&text, 11);
        assert_eq!(
            BANNER_COPYRIGHT.lines().count(),
            2,
            "the banner copyright is the two-line string this test is about"
        );
        for x in CR_X..=45 {
            for y in BAR_TOP..BANNER_H {
                assert_ne!(c.get(x, y), white, "no copyright ink at ({x}, {y})");
            }
        }
    }

    /// Every row lands on its baseline. Compares against literals, not the constants `form` uses.
    #[test]
    fn rows_land_on_their_baselines() {
        const EXPECTED_PRODUCT: i32 = 100;
        const EXPECTED_VERSION: i32 = 117;
        const EXPECTED_COPYRIGHT: i32 = 133;
        const EXPECTED_FACTS: [i32; 3] = [182, 195, 214];
        const EXPECTED_MEMORY: i32 = 258;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let off = baseline_offset(&text, px);
        let f = sample();
        let form = form(&f, &text, px);
        let top = |id: &str| form.item(id).expect("item exists").rect.y;
        assert_eq!(top("product") + off, EXPECTED_PRODUCT);
        assert_eq!(top("version") + off, EXPECTED_VERSION);
        assert_eq!(top("copyright") + off, EXPECTED_COPYRIGHT);
        for (i, want) in EXPECTED_FACTS.iter().enumerate() {
            assert_eq!(top(&format!("fact{i}")) + off, *want);
        }
        assert_eq!(top("memlabel") + off, EXPECTED_MEMORY);
        assert_eq!(top("memvalue") + off, EXPECTED_MEMORY);
    }

    /// Every text row starts at x53, the memory value at x252.
    #[test]
    fn text_columns_start_at_53_and_252() {
        const EXPECTED_TEXT_X: i32 = 53;
        const EXPECTED_VALUE_X: i32 = 252;
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let f = sample();
        let form = form(&f, &text, px);
        let x = |id: &str| form.item(id).expect("item exists").rect.x;
        for id in [
            "product",
            "version",
            "copyright",
            "fact0",
            "fact1",
            "fact2",
            "memlabel",
        ] {
            assert_eq!(x(id), EXPECTED_TEXT_X, "{id} starts at x53");
        }
        assert_eq!(x("memvalue"), EXPECTED_VALUE_X);
    }

    /// The OK button's rect, and the separator's span and etched row order. The order is
    /// asserted on a render: rect arithmetic would not notice a bottom-etched separator or
    /// swapped colours.
    #[test]
    fn the_button_and_separator_sit_in_place() {
        const EXPECTED_OK: (i32, i32, i32, i32) = (330, 290, 75, 23);
        const EXPECTED_SEP_X0: i32 = 53;
        const EXPECTED_SEP_Y: i32 = 240;
        const HILIGHT_Y: i32 = 241;
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let cs = &theme.colors;
        let built = form(&sample(), &text, px);
        let ok = built.item(OK).expect("ok exists").rect;
        assert_eq!((ok.x, ok.y, ok.width, ok.height), EXPECTED_OK);
        assert_eq!(ok.width, crate::classic::widgets::BUTTON_W);
        assert_eq!(ok.height, crate::classic::widgets::BUTTON_H);
        let sep = built.item("sep").expect("sep exists").rect;
        assert_eq!(sep.x, EXPECTED_SEP_X0);
        assert_eq!(sep.y, EXPECTED_SEP_Y);
        assert_eq!(sep.right(), 404, "the separator ends at x403 inclusive");
        let (w, h) = CLIENT;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &text, &classic_paint(cs, 1), &built, false, px);
        assert_eq!(
            c.get(200, EXPECTED_SEP_Y),
            cs.button_shadow.argb(),
            "the separator's upper row is the shadow"
        );
        assert_eq!(
            c.get(200, HILIGHT_Y),
            cs.button_hilight.argb(),
            "the hilight is the row under it"
        );
    }

    #[test]
    fn the_icon_does_not_move_the_text_column() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let without = form(&sample(), &text, px);
        let mut with_icon = sample();
        with_icon.icon = Some(Canvas::new(32, 32));
        let with = form(&with_icon, &text, px);
        let x = |fm: &Form, id: &str| fm.item(id).expect("item exists").rect.x;
        for id in ["product", "version", "copyright", "memlabel"] {
            assert_eq!(x(&without, id), x(&with, id), "{id} column is stable");
        }
        assert!(without.item("icon").is_none());
        assert!(with.item("icon").is_some());
    }

    /// An empty fact leaves its row blank, so the separator and everything below stay put.
    #[test]
    fn an_empty_fact_does_not_move_the_separator() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let full = form(&sample(), &text, px);
        let mut sparse = sample();
        sparse.facts[1] = String::new();
        let thin = form(&sparse, &text, px);
        let y = |fm: &Form, id: &str| fm.item(id).expect("item exists").rect.y;
        assert_eq!(y(&full, "sep"), y(&thin, "sep"));
        assert_eq!(y(&full, "memlabel"), y(&thin, "memlabel"));
        assert_eq!(y(&full, OK), y(&thin, OK));
        assert_eq!(y(&full, "fact2"), y(&thin, "fact2"));
    }

    /// `form` subtracts a single offset from a baseline to get a rect top, which holds only if
    /// the renderer's baseline tracks the rect's y one-for-one.
    #[test]
    fn the_baseline_tracks_the_rect() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let h = row_h(px);
        let at = |y: i32| {
            text.baseline(Rect::new(0, y, 200, h), false, px)
                .expect("the renderer reports a baseline")
        };
        assert_eq!(at(100) - at(0), 100);
        assert_eq!(at(250) - at(0), 250);
    }

    /// A render that checks where the ink lands. `rows_land_on_their_baselines` cannot: `row`
    /// is `BASE - off`, so the offset cancels and it would pass with a wrong `baseline_offset`.
    ///
    /// Every field is capital-only text without ascenders, descenders or digits, so a full-row
    /// scan cannot pick up a taller neighbour. The last inked row must land on the `*_BASE`
    /// constants.
    #[test]
    fn the_ink_lands_on_the_base_rows() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let cs = &theme.colors;
        let caps = Fields {
            product: "ACE".into(),
            version: "ACE".into(),
            copyright: "ACE".into(),
            facts: ["ACE".into(), "ACE".into(), "ACE".into()],
            memory_label: "ACE".into(),
            memory_value: "ACE".into(),
            icon: None,
        };
        let built = form(&caps, &text, px);
        let (w, h) = CLIENT;
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::form::draw(&mut c, &text, &classic_paint(cs, 1), &built, false, px);
        let bg = cs.button_face.argb();
        // The last inked row in a narrow window at the row's text start (TEXT_X..TEXT_X+15
        // covers the "A" only).
        let last_ink_near = |base: i32| -> Option<i32> {
            (base - 12..=base + 2)
                .rev()
                .find(|&y| (TEXT_X..TEXT_X + 15).any(|x| c.get(x, y) != bg))
        };
        assert_eq!(
            last_ink_near(PRODUCT_BASE),
            Some(PRODUCT_BASE),
            "product ink ends on its base row"
        );
        assert_eq!(
            last_ink_near(VERSION_BASE),
            Some(VERSION_BASE),
            "version ink ends on its base row"
        );
        assert_eq!(
            last_ink_near(COPYRIGHT_BASE),
            Some(COPYRIGHT_BASE),
            "copyright ink ends on its base row"
        );
        for base in FACT_BASE {
            assert_eq!(
                last_ink_near(base),
                Some(base),
                "fact ink ends on its base row"
            );
        }
        assert_eq!(
            last_ink_near(MEMORY_BASE),
            Some(MEMORY_BASE),
            "memlabel ink ends on its base row"
        );
    }

    /// The banner is the same in every box: `Fields` differing only in `product` paint a
    /// byte-identical banner.
    #[test]
    fn the_product_does_not_change_the_banner() {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        let mut other = sample();
        other.product = "Notepad".into();
        let pic = |f: &Fields| match &form(f, &text, px)
            .item("banner")
            .expect("the banner item exists")
            .control
        {
            Control::Picture {
                canvas: Some(c), ..
            } => c.clone(),
            _ => panic!("the banner is a Picture carrying a canvas"),
        };
        assert_eq!(
            pic(&sample()),
            pic(&other),
            "the banner does not vary with `product`"
        );
        // Not vacuous: the product row does change.
        let row = |f: &Fields| match &form(f, &text, px)
            .item("product")
            .expect("the product item exists")
            .control
        {
            Control::Label { text, .. } => text.clone(),
            _ => panic!("the product row is a Label"),
        };
        assert_eq!(row(&sample()), "Reprise");
        assert_eq!(row(&other), "Notepad");
    }

    /// With no font loaded the box still builds and draws, without ink.
    #[test]
    fn a_fontless_renderer_still_builds_and_draws_the_box() {
        let text = TextRenderer::fontless();
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let built = form(&sample(), &text, px);
        assert!(built.item("product").is_some(), "the product row exists");
        let (w, h) = CLIENT;
        let mut c = Canvas::new(w as u32, h as u32);
        crate::controls::form::draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &built,
            false,
            px,
        );
    }

    #[test]
    fn the_wide_banner_at_413_is_the_banner() {
        let text = TextRenderer::load(&FontConfig::default());
        assert_eq!(banner_wide(&text, 11, BANNER_W), banner(&text, 11));
        assert_eq!(banner_wide(&text, 11, 200).width, BANNER_W as u32);
    }

    /// Three groups of 2, 1 and 3 lines, a 64 px one-colour logo, and a line too long for its
    /// column.
    fn system_sample() -> System {
        let mut logo = Canvas::new(LOGO as u32, LOGO as u32);
        logo.fill_rect(logo.bounds(), Color::rgb(200, 30, 40));
        let g = |label: &str, lines: &[&str]| Group {
            label: label.into(),
            lines: lines.iter().map(|s| s.to_string()).collect(),
        };
        System {
            logo: Some(logo),
            groups: vec![
                g(
                    "Reprise:",
                    &["Version 0.1.0 (build a1b2c3d)", "Classic interface"],
                ),
                g("Empty:", &[]),
                g("Memory:", &["15,728,640 KB"]),
                g(
                    "Graphics:",
                    &[
                        "AMD Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M and a name far too long to fit",
                        "Ampersand && Co",
                        "virtio-pci",
                    ],
                ),
            ],
        }
    }

    fn render_system(s: &System) -> (Canvas, Form, Theme) {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let px = theme.fonts.size_px;
        let built = system_form(s, &text, px);
        let (w, h) = system_size(s);
        let mut c = Canvas::new(w as u32, h as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::form::draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &built,
            false,
            px,
        );
        (c, built, theme)
    }

    /// Pinned layout of the sample: 6 lines in 3 groups make the table 6 * 16 + 2 * 6 = 108 px
    /// from row 94, so the separator is on row 212, the buttons on 226, the client 500x260. The
    /// empty group takes no room.
    #[test]
    fn the_system_view_grows_with_its_lines() {
        const EXPECTED_SIZE: (i32, i32) = (500, 260);
        let (_, built, _) = render_system(&system_sample());
        assert_eq!(system_size(&system_sample()), EXPECTED_SIZE);
        let rect = |id: &str| built.item(id).expect("item exists").rect;
        let top = |id: &str| rect(id).y;
        assert_eq!(top("group0"), 94);
        assert_eq!(top("line1"), 110);
        assert_eq!(top("group1"), 132, "a group starts 6 px further down");
        assert_eq!(top("group2"), 154);
        assert_eq!(top("line5"), 186);
        assert!(built.item("group3").is_none());
        assert!(built.item("line6").is_none());
        assert_eq!(rect("group0").x, 96);
        assert_eq!(rect("line0").x, 204);
        assert_eq!(rect("line0").right(), 488);
        assert_eq!(top("sep"), 212);
        let ok = rect(OK);
        assert_eq!((ok.x, ok.y, ok.width, ok.height), (413, 226, 75, 23));
        let copy = rect(COPY);
        assert_eq!((copy.x, copy.y), (332, 226));
        assert_eq!(
            built.focus().map(|i| built.items[i].id),
            Some(OK),
            "OK has the focus"
        );
    }

    /// The table is at least the logo's height, so one line still leaves the logo clear of the
    /// separator.
    #[test]
    fn a_short_table_still_fits_the_logo() {
        let s = System {
            logo: None,
            groups: vec![Group {
                label: "Memory:".into(),
                lines: vec!["1.0 GB".into()],
            }],
        };
        let (sep, _) = system_footer(&s);
        assert_eq!(sep, TABLE_Y + LOGO + SEP_GAP);
    }

    /// A render of the sample: logo at (16, 94), banner stretched to 500 px, ink in every
    /// column where it belongs, the long line cut before the margin, and the separator.
    #[test]
    fn the_system_view_draws_in_place() {
        let (c, built, theme) = render_system(&system_sample());
        let cs = &theme.colors;
        let logo = Color::rgb(200, 30, 40).argb();
        assert_eq!(c.get(16, 94), logo);
        assert_eq!(c.get(79, 157), logo);
        assert_ne!(c.get(80, 94), logo);
        assert_ne!(c.get(16, 158), logo);
        assert_eq!(c.get(499, 6), EDGE.argb(), "the banner reaches x499");
        assert_eq!(c.get(499, 75), BAR_RIGHT.argb());
        assert_eq!(c.get(0, 6), BASE.argb());
        let bg = cs.button_face.argb();
        let ink_in =
            |x0: i32, x1: i32, y: i32| (x0..x1).find(|&x| (y..y + 14).any(|yy| c.get(x, yy) != bg));
        for (label, line) in [
            ("group0", "line0"),
            ("group1", "line2"),
            ("group2", "line3"),
        ] {
            let y = built.item(label).expect("label").rect.y;
            let gx = ink_in(90, 204, y).expect("the label inks");
            assert!(
                (96..99).contains(&gx),
                "{label} ink starts at x96, not {gx}"
            );
            let lx =
                ink_in(200, 500, built.item(line).expect("line").rect.y).expect("the line inks");
            assert!(
                (204..207).contains(&lx),
                "{line} ink starts at x204, not {lx}"
            );
        }
        let long_y = built.item("line3").expect("line").rect.y;
        let last = (204..500)
            .rev()
            .find(|&x| (long_y..long_y + 14).any(|y| c.get(x, y) != bg))
            .expect("the long line inks");
        assert!(
            last < 488,
            "the long line stops before the margin, at {last}"
        );
        assert_eq!(c.get(250, 212), cs.button_shadow.argb());
        assert_eq!(c.get(250, 213), cs.button_hilight.argb());
        assert_eq!(c.get(11, 212), bg, "the separator starts at x12");
    }

    /// A long line is cut with an ellipsis, and a doubled `&` still draws as one.
    #[test]
    fn long_lines_end_in_an_ellipsis_and_ampersands_survive() {
        let (_, built, _) = render_system(&system_sample());
        let label = |id: &str| match &built.item(id).expect("item").control {
            Control::Label { text, .. } => text.clone(),
            _ => panic!("{id} is a label"),
        };
        assert!(label("line3").ends_with("..."), "{}", label("line3"));
        assert_eq!(label("line4"), "Ampersand && Co");
    }

    fn sample() -> Fields {
        Fields {
            product: "Reprise".into(),
            version: "Version 0.1.0 (Build a1b2c3d : 6.9.0-generic)".into(),
            copyright: "Copyright (C) 2026".into(),
            facts: [
                "Example CPU @ 2.00GHz".into(),
                "Video:  amdgpu".into(),
                "Free space on /:  38.2 GB".into(),
            ],
            memory_label: "Physical memory available to Reprise:".into(),
            memory_value: "16,000,000 KB".into(),
            icon: None,
        }
    }
}
