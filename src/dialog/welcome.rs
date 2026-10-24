use super::*;
use crate::raster::Canvas;
use crate::text::{Align, split_mnemonic};
use crate::theme::{Color, TextUnits};
use reprisekit::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, draw_label};
use reprisekit::cues::Cues;
use reprisekit::look::Paint;
use reprisekit::look::widgets::draw_button;
use reprisekit::pointer::{Interaction, Lit};
use reprisekit::style::TextSpec;

impl DialogLayout {
    /// The Welcome panel: icon buttons in a row of cells across the
    /// gradient, the rest (Cancel) as small classic buttons from the right
    /// of the bottom band.
    pub(super) fn compute_welcome(d: &Dialog, u: TextUnits) -> DialogLayout {
        let b = WELCOME_BORDER;
        let inner_w = WELCOME_W - 2 * b;
        let caption = Rect::new(b, b, inner_w, WELCOME_TITLE_H);
        let icons = d.buttons.iter().filter(|x| x.icon.is_some()).count();
        let (cell_w, cells): (i32, &[i32]) = if icons == 2 {
            (WELCOME_TWO.0, &WELCOME_TWO.1)
        } else {
            (WELCOME_THREE.0, &WELCOME_THREE.1)
        };
        // Log Off and Turn off computer are the only panels; a cell past
        // the table would sit at the left edge.
        let cell_x = |i: usize| cells.get(i).copied().unwrap_or(b);
        let (cw, ch, cy) = WELCOME_CANCEL;
        let mut next_icon = 0;
        let mut next_plain = 0;
        let buttons: Vec<Rect> = d
            .buttons
            .iter()
            .map(|x| {
                if x.icon.is_some() {
                    let r = Rect::new(
                        cell_x(next_icon),
                        b + WELCOME_CELL_Y,
                        cell_w,
                        WELCOME_CELL_H,
                    );
                    next_icon += 1;
                    r
                } else {
                    let r = Rect::new(
                        WELCOME_W - b - 10 - cw - next_plain * (cw + BUTTON_GAP),
                        b + cy,
                        cw,
                        ch,
                    );
                    next_plain += 1;
                    r
                }
            })
            .collect();
        let buttons = buttons.into_iter().map(|r: Rect| u.rect(r)).collect();
        DialogLayout {
            size: u.size((WELCOME_W, WELCOME_H)),
            caption: u.rect(caption),
            buttons,
            ..Default::default()
        }
    }

    /// The Welcome panel under a look with glass (`GLASS_WELCOME_W` on):
    /// the icon buttons in a centred row, the rest (Cancel) from the
    /// right of the row under them.
    pub(super) fn compute_welcome_glass(d: &Dialog) -> DialogLayout {
        let (x0, y0) = (DIALOG_FRAME, DIALOG_FRAME + CAPTION_H);
        let (cw, ch) = GLASS_WELCOME_CELL;
        let icons = d.buttons.iter().filter(|x| x.icon.is_some()).count() as i32;
        let row = icons * cw + (icons - 1).max(0) * GLASS_WELCOME_GAP;
        let left = x0 + (GLASS_WELCOME_W - row) / 2;
        let plain_y = y0 + GLASS_WELCOME_TOP + ch + GLASS_WELCOME_TOP;
        let mut next_icon = 0;
        let mut next_plain = 0;
        let buttons = d
            .buttons
            .iter()
            .map(|x| {
                if x.icon.is_some() {
                    let r = Rect::new(
                        left + next_icon * (cw + GLASS_WELCOME_GAP),
                        y0 + GLASS_WELCOME_TOP,
                        cw,
                        ch,
                    );
                    next_icon += 1;
                    r
                } else {
                    let r = Rect::new(
                        x0 + GLASS_WELCOME_W
                            - GLASS_WELCOME_MARGIN
                            - BUTTON_W
                            - next_plain * (BUTTON_W + BUTTON_GAP),
                        plain_y,
                        BUTTON_W,
                        BUTTON_H,
                    );
                    next_plain += 1;
                    r
                }
            })
            .collect();
        let client_h = plain_y - y0 + BUTTON_H + GLASS_WELCOME_MARGIN;
        DialogLayout {
            size: (
                GLASS_WELCOME_W + 2 * DIALOG_FRAME,
                client_h + 2 * DIALOG_FRAME + CAPTION_H,
            ),
            buttons,
            ..Default::default()
        }
    }
}

/// The Welcome panel: 314x200 with a 1 px black border, a 43 px title
/// band, icon buttons in cells 64 px tall on the gradient with the icon
/// 8 px into the cell and the label 44 px in, and a 60x20 Cancel button
/// 10 px from the right edge in the bottom band. Three cells are 85 px
/// wide and start at 29, 115 and 200; two are 115 px wide and start at
/// 43 and 158.
pub(super) const WELCOME_W: i32 = 314;

const WELCOME_H: i32 = 200;

pub(super) const WELCOME_BORDER: i32 = 1;

const WELCOME_TITLE_H: i32 = 43;

const WELCOME_CELL_Y: i32 = 72;

const WELCOME_CELL_H: i32 = 64;

pub(super) const WELCOME_ICON_Y: i32 = 8;

const WELCOME_LABEL_Y: i32 = 44;

const WELCOME_THREE: (i32, [i32; 3]) = (85, [29, 115, 200]);

const WELCOME_TWO: (i32, [i32; 2]) = (115, [43, 158]);

const WELCOME_CANCEL: (i32, i32, i32) = (60, 20, 167);

/// The art tables' columns across the panel and the gradient's rows, at 100%.
pub(super) const PANEL_W: i32 = 312;

pub(super) const PANEL_H: i32 = 110;

/// The Welcome panel under a look with glass, a dialog like any other
/// in its frame: a 312 px wide client, the icon buttons 80x64 push
/// buttons 12 px apart, centred, 14 px under the client's top, each with
/// its icon 8 px and its label 44 px into it; Cancel a standard button
/// 11 px in from the client's right and bottom edges, 14 px under them.
pub(super) const GLASS_WELCOME_W: i32 = 312;

pub(super) const GLASS_WELCOME_TOP: i32 = 14;

pub(super) const GLASS_WELCOME_CELL: (i32, i32) = (80, 64);

const GLASS_WELCOME_GAP: i32 = 12;

pub(super) const GLASS_WELCOME_MARGIN: i32 = 11;

/// The Welcome panel's blue colours, a drawing constant rather than
/// theme colours: the panel keeps them whatever the classic scheme says.
pub(super) const PANEL_BAND: Color = Color::rgb(0x00, 0x33, 0x99);

const PANEL_SEPARATOR: [Color; 21] = [
    Color::rgb(0x30, 0x53, 0xb5),
    Color::rgb(0x3c, 0x64, 0xb6),
    Color::rgb(0x4a, 0x6f, 0xba),
    Color::rgb(0x55, 0x79, 0xc0),
    Color::rgb(0x60, 0x83, 0xc5),
    Color::rgb(0x6f, 0x8f, 0xcb),
    Color::rgb(0x80, 0x9d, 0xd2),
    Color::rgb(0x90, 0xaa, 0xd9),
    Color::rgb(0x9d, 0xb5, 0xdf),
    Color::rgb(0xa5, 0xbc, 0xe3),
    Color::rgb(0xad, 0xc2, 0xe6),
    Color::rgb(0xb5, 0xc9, 0xea),
    Color::rgb(0xbc, 0xcf, 0xed),
    Color::rgb(0xc0, 0xd2, 0xef),
    Color::rgb(0xc4, 0xd5, 0xf0),
    Color::rgb(0xc7, 0xd8, 0xf2),
    Color::rgb(0xcb, 0xdc, 0xf4),
    Color::rgb(0xcf, 0xdf, 0xf6),
    Color::rgb(0xd2, 0xe1, 0xf7),
    Color::rgb(0xd5, 0xe4, 0xf8),
    Color::rgb(0xd8, 0xe6, 0xfa),
];

/// The separator's two rows under the title band, left to right across
/// its 312 columns, as runs of (`PANEL_SEPARATOR` index, columns). The
/// lightest run sits left of centre; the second row is the first with
/// the steps dithered a column either way.
pub(super) const SEPARATOR_RUNS: [&[(u8, u8)]; 2] = [
    &[
        (1, 9),
        (2, 8),
        (3, 6),
        (4, 7),
        (5, 9),
        (6, 9),
        (7, 9),
        (8, 7),
        (9, 5),
        (10, 4),
        (11, 6),
        (12, 5),
        (13, 2),
        (14, 4),
        (15, 3),
        (16, 6),
        (17, 4),
        (18, 3),
        (19, 7),
        (20, 41),
        (19, 19),
        (18, 11),
        (17, 12),
        (16, 16),
        (15, 5),
        (16, 3),
        (15, 3),
        (14, 4),
        (13, 2),
        (12, 5),
        (11, 6),
        (10, 4),
        (9, 5),
        (8, 7),
        (7, 9),
        (6, 9),
        (5, 9),
        (4, 7),
        (3, 6),
        (2, 8),
        (1, 8),
    ],
    &[
        (1, 1),
        (0, 1),
        (1, 6),
        (2, 1),
        (1, 2),
        (2, 4),
        (3, 1),
        (2, 1),
        (3, 4),
        (4, 1),
        (3, 2),
        (4, 6),
        (5, 1),
        (4, 1),
        (5, 5),
        (6, 1),
        (5, 1),
        (6, 1),
        (5, 1),
        (6, 6),
        (7, 1),
        (6, 2),
        (7, 6),
        (8, 1),
        (7, 1),
        (8, 4),
        (9, 1),
        (8, 1),
        (9, 5),
        (10, 5),
        (11, 4),
        (12, 1),
        (11, 1),
        (12, 4),
        (13, 2),
        (14, 4),
        (15, 4),
        (16, 3),
        (17, 1),
        (16, 1),
        (17, 4),
        (18, 3),
        (19, 7),
        (20, 41),
        (19, 19),
        (18, 11),
        (17, 12),
        (16, 1),
        (17, 1),
        (16, 10),
        (15, 1),
        (16, 1),
        (15, 1),
        (16, 1),
        (15, 5),
        (16, 2),
        (15, 4),
        (14, 4),
        (13, 2),
        (12, 4),
        (11, 1),
        (12, 1),
        (11, 4),
        (10, 5),
        (9, 5),
        (8, 1),
        (9, 1),
        (8, 4),
        (7, 1),
        (8, 1),
        (7, 6),
        (6, 2),
        (7, 1),
        (6, 6),
        (5, 1),
        (6, 1),
        (5, 1),
        (6, 1),
        (5, 5),
        (4, 1),
        (5, 1),
        (4, 6),
        (3, 2),
        (4, 1),
        (3, 4),
        (2, 1),
        (3, 1),
        (2, 4),
        (1, 2),
        (2, 1),
        (1, 6),
        (0, 1),
    ],
];

/// The two rows under the gradient, left to right as runs of (colour,
/// columns): a line lighter under the middle, then the bottom band's
/// first row, a shade redder under the middle.
pub(super) const PANEL_LINE_RUNS: [[(Color, i32); 3]; 2] = [
    [
        (Color::rgb(0x0f, 0x40, 0xa4), 87),
        (Color::rgb(0x14, 0x44, 0xa6), 191),
        (Color::rgb(0x0f, 0x40, 0xa4), 34),
    ],
    [
        (PANEL_BAND, 121),
        (Color::rgb(0x01, 0x33, 0x99), 125),
        (PANEL_BAND, 66),
    ],
];

const PANEL_LEFT: Color = Color::rgb(0x5a, 0x7d, 0xde);

const PANEL_PEAK: Color = Color::rgb(0x9b, 0xb9, 0xf7);

pub(super) const PANEL_DISABLED: Color = Color::rgb(0xa0, 0xa0, 0xa0);

/// The gradient's light across its 312 columns and down its 110 rows,
/// as (offset, per mille) stops joined by straight lines. A pixel's
/// colour runs from `PANEL_LEFT` to `PANEL_PEAK` by the product of its
/// column's and its row's light: flat at the left edge, lightest on
/// column 183, a fifth of the way at the right edge; lightest across
/// rows 36-48, darkening towards the bottom, the last row darker again.
const PANEL_ACROSS: [(i32, i32); 12] = [
    (0, 0),
    (9, 0),
    (20, 49),
    (45, 167),
    (100, 562),
    (153, 916),
    (170, 977),
    (183, 1000),
    (196, 970),
    (210, 926),
    (260, 599),
    (311, 224),
];

const PANEL_DOWN: [(i32, i32); 8] = [
    (0, 933),
    (18, 970),
    (36, 994),
    (48, 992),
    (70, 946),
    (90, 884),
    (108, 817),
    (109, 706),
];

/// The per-mille value at `at` along `stops`, joined by straight lines
/// and held past the last stop.
fn stop_value(stops: &[(i32, i32)], at: i32) -> i32 {
    for w in stops.windows(2) {
        let ((x0, y0), (x1, y1)) = (w[0], w[1]);
        if at <= x1 {
            return y0 + ((y1 - y0) * (at - x0)).div_euclid(x1 - x0);
        }
    }
    stops.last().map_or(0, |s| s.1)
}

/// The gradient's colour at (`x`, `y`), unscaled panel px.
pub(super) fn panel_gradient(x: i32, y: i32) -> Color {
    let t = stop_value(&PANEL_ACROSS, x) * stop_value(&PANEL_DOWN, y);
    let mix = |a: u8, b: u8| (a as i32 + ((b as i32 - a as i32) * t + 500_000) / 1_000_000) as u8;
    Color::rgb(
        mix(PANEL_LEFT.r, PANEL_PEAK.r),
        mix(PANEL_LEFT.g, PANEL_PEAK.g),
        mix(PANEL_LEFT.b, PANEL_PEAK.b),
    )
}

/// The Welcome title's face: the first family at 20 px (capitals 14 rows
/// tall), else 18 px, falling back as the Blue Start label does. `bold`
/// takes each family's heaviest face.
pub(super) fn welcome_title_text() -> TextSpec {
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    TextSpec {
        family: vec![
            "Franklin Gothic Medium".into(),
            "Trebuchet MS".into(),
            "DejaVu Sans".into(),
        ],
        size_px: 18,
        family_sizes: vec![("Franklin Gothic Medium".into(), 20)],
        bold: true,
        italic: false,
        antialias: true,
        full_hinting: true,
        color: white,
        inactive: white,
        shadow: None,
        ink_gamma: 1.0,
    }
}

thread_local! {
    pub(super) static WELCOME_TITLE: TextRenderer = TextRenderer::load(&welcome_title_text().font());
}

/// The Welcome panel: black border, title band with the logo, the icon
/// buttons on the gradient (`panel_gradient`), Cancel in the bottom
/// band. `button_icons` is parallel to `d.buttons`. The focus
/// sits on the default button: an icon button shows it by drawing its
/// icon hot, as the one `lit` names under the pointer does, and Cancel
/// by its black default frame, with the dotted rectangle while `cues`
/// shows it. The labels underline their access keys only while `cues`
/// shows them.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_welcome(
    theme: &Theme,
    text: &TextRenderer,
    layout: &DialogLayout,
    button_icons: &[Option<Canvas>],
    logo: Option<&Canvas>,
    d: &Dialog,
    lit: Lit<usize>,
    cues: Cues,
) -> Canvas {
    let s = theme.scale.max(1);
    let px = theme.fonts.size_px;
    let white = Color::rgb(255, 255, 255);
    let (w, h) = (layout.size.0 * s, layout.size.1 * s);
    let mut c = Canvas::new(w.max(1) as u32, h.max(1) as u32);
    c.fill_rect(Rect::new(0, 0, w, h), Color::rgb(0, 0, 0));
    let b = WELCOME_BORDER * s;
    let inner = Rect::new(b, b, w - 2 * b, h - 2 * b);
    c.fill_rect(inner, PANEL_BAND);
    // The bands are drawn for 312 columns and a 110-row gradient at 100%; grown with the text
    // they stretch, each logical px taking the colour of the 100% one it maps back to.
    let u = theme.text;
    let caption = layout.caption;
    let title = caption.scaled(s);
    let band_top = caption.bottom() + 2;
    let band = Rect::new(caption.x, band_top, caption.width, u.len(156) - band_top);
    let across = |n: i32| n * band.width / PANEL_W;
    let runs_row = |c: &mut Canvas, y: i32, runs: &mut dyn Iterator<Item = (Color, i32)>| {
        let mut at = 0;
        for (ink, n) in runs {
            let (x0, x1) = (across(at), across(at + n));
            c.fill_rect(Rect::new(band.x + x0, y, x1 - x0, 1).scaled(s), ink);
            at += n;
        }
    };
    for (row, runs) in SEPARATOR_RUNS.iter().enumerate() {
        let mut inks = runs
            .iter()
            .map(|&(i, n)| (PANEL_SEPARATOR[i as usize], n as i32));
        runs_row(&mut c, caption.bottom() + row as i32, &mut inks);
    }
    for y in 0..band.height {
        for x in 0..band.width {
            let at = Rect::new(band.x + x, band.y + y, 1, 1).scaled(s);
            let ink = panel_gradient(x * PANEL_W / band.width, y * PANEL_H / band.height);
            c.fill_rect(at, ink);
        }
    }
    for (row, runs) in PANEL_LINE_RUNS.iter().enumerate() {
        runs_row(
            &mut c,
            band.bottom() + row as i32,
            &mut runs.iter().copied(),
        );
    }
    let spec = welcome_title_text();
    WELCOME_TITLE.with(|t| {
        t.for_output(s).draw(
            &mut c,
            Rect::new(
                title.x + 12 * s,
                title.y,
                title.width - 60 * s,
                title.height,
            ),
            &d.title,
            spec.bold,
            spec.color,
            Align::Left,
            u.px(spec.size_in(t)) * s as u32,
        )
    });
    if let Some(logo) = logo {
        let dy = 6 + (caption.height - WELCOME_TITLE_H) / 2;
        c.blit_over(title.right() - 43 * s, title.y + dy * s, logo);
    }
    let mut fallback: Option<Canvas> = None;
    for (i, (r, btn)) in layout.buttons.iter().zip(&d.buttons).enumerate() {
        let r = r.scaled(s);
        if btn.icon.is_some() {
            let icon_px = ICON_PX * s;
            let (icon_y, label) =
                welcome_cell(r, WELCOME_CELL_H, WELCOME_CELL_H - WELCOME_LABEL_Y, u, s);
            let at = (r.x + (r.width - icon_px) / 2, icon_y);
            let icon = match button_icons.get(i).and_then(|o| o.as_ref()) {
                Some(icon) => icon,
                None => {
                    &*fallback.get_or_insert_with(|| reprisekit::icon::fallback(theme, icon_px))
                }
            };
            let hot = lit.is_some_and(|(j, _)| j == i) || btn.default;
            if btn.enabled && hot {
                c.blit_over(at.0, at.1, &reprisekit::icon::hot(icon));
            } else if btn.enabled {
                c.blit_over(at.0, at.1, icon);
            } else {
                let mut faded = icon.clone();
                faded.fade(1, 3);
                c.blit_over(at.0, at.1, &faded);
            }
            let color = if btn.enabled { white } else { PANEL_DISABLED };
            if cues.keys {
                draw_label(
                    &mut c,
                    text,
                    label,
                    &btn.label,
                    true,
                    color,
                    Align::Center,
                    px,
                    s,
                );
            } else {
                let plain = split_mnemonic(&btn.label).0;
                text.draw(&mut c, label, &plain, true, color, Align::Center, px);
            }
        } else {
            draw_button(
                &mut c,
                text,
                &Paint {
                    access_keys: cues.keys,
                    ..theme.paint()
                },
                r,
                &btn.label,
                btn.default,
                Interaction::Idle,
                true,
                btn.default && cues.focus,
                px,
            );
        }
    }
    c
}

/// A Welcome icon cell `r` (scaled) laid out `cell_h` tall at 100% with a `label_h` label line
/// `WELCOME_LABEL_Y` down: the icon's top and the label's box at the text size `u`. The label
/// keeps its 100% gap to the cell's bottom, grown; the icon keeps its px and moves down half
/// as far as the label did.
fn welcome_cell(r: Rect, cell_h: i32, label_h: i32, u: TextUnits, s: i32) -> (i32, Rect) {
    let h = r.height / s;
    let label_y = (h - u.len(cell_h - WELCOME_LABEL_Y)).max(WELCOME_LABEL_Y);
    let icon_y = WELCOME_ICON_Y + (label_y - WELCOME_LABEL_Y) / 2;
    let label = Rect::new(r.x, r.y + label_y * s, r.width, u.len(label_h) * s);
    (r.y + icon_y * s, label)
}

/// How a framed Welcome panel's icon button shows: wearing the default
/// ring, what the pointer does to it, and the dotted focus rectangle.
#[derive(Clone, Copy)]
pub(super) struct IconButton {
    pub(super) default: bool,
    pub(super) ptr: Interaction,
    pub(super) focus: bool,
}

/// A framed Welcome panel's icon button `b` across `r` (scaled): a push
/// button with `icon` (the fallback glyph when None) `WELCOME_ICON_Y`
/// into it, faded when greyed, and the label `WELCOME_LABEL_Y` in,
/// centred, its access key underlined while `cues` shows the keys.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_icon_button(
    c: &mut Canvas,
    theme: &Theme,
    text: &TextRenderer,
    p: &Paint,
    r: Rect,
    b: &DialogButton,
    icon: Option<&Canvas>,
    look: IconButton,
    cues: Cues,
) {
    let s = p.s;
    let px = theme.fonts.size_px;
    let icon_px = ICON_PX * s;
    draw_button(
        c,
        text,
        p,
        r,
        "",
        look.default,
        look.ptr,
        b.enabled,
        look.focus,
        px,
    );
    let fallback;
    let icon = match icon {
        Some(icon) => icon,
        None => {
            fallback = reprisekit::icon::fallback(theme, icon_px);
            &fallback
        }
    };
    let cell_h = GLASS_WELCOME_CELL.1;
    let (icon_y, label) = welcome_cell(r, cell_h, cell_h - WELCOME_LABEL_Y - 4, theme.text, s);
    let at = (r.x + (r.width - icon_px) / 2, icon_y);
    if b.enabled {
        c.blit_over(at.0, at.1, icon);
    } else {
        let mut faded = icon.clone();
        faded.fade(1, 3);
        c.blit_over(at.0, at.1, &faded);
    }
    let ink = if b.enabled {
        p.cs.button_text
    } else {
        p.cs.gray_text
    };
    if cues.keys {
        draw_label(c, text, label, &b.label, false, ink, Align::Center, px, s);
    } else {
        let plain = split_mnemonic(&b.label).0;
        text.draw(c, label, &plain, false, ink, Align::Center, px);
    }
}
