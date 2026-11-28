//! The Font dialog: a family, a style and a size, each typed into its field or picked from the
//! list under it, and a sample in the pick. The host lists the families ([`crate::text::families`])
//! and loads the sample's renderer for each pick; [`follow`] keeps the fields and lists agreeing.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::listbox::ListBox;
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use accesskit::{NodeId, Role};

pub const FAMILY: &str = "family";
pub const FAMILY_LIST: &str = "familylist";
pub const STYLE: &str = "style";
pub const STYLE_LIST: &str = "stylelist";
pub const SIZE: &str = "size";
pub const SIZE_LIST: &str = "sizelist";
pub const SAMPLE: &str = "sample";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";

pub const CLIENT: (i32, i32) = (503, 252);

/// The sizes listed, in points.
pub const SIZES: [u32; 16] = [8, 9, 10, 11, 12, 14, 16, 18, 20, 22, 24, 26, 28, 36, 48, 72];
/// The largest size a typed number may ask for, in points.
pub const MAX_PT: u32 = 400;
pub const SAMPLE_TEXT: &str = "AaBbYyZz";

const MARGIN: i32 = 11;
const LABEL_Y: i32 = 8;
const EDIT_Y: i32 = 24;
const EDIT_H: i32 = 20;
const LIST_Y: i32 = EDIT_Y + EDIT_H;
const LIST_H: i32 = 112;
const GAP: i32 = 10;
/// Wide enough for a family's variant names ("... Nerd Font Mono", "... Nerd Font Propo") to show whole.
const FAMILY_W: i32 = 200;
const STYLE_X: i32 = MARGIN + FAMILY_W + GAP;
const STYLE_W: i32 = 112;
const SIZE_X: i32 = STYLE_X + STYLE_W + GAP;
const SIZE_W: i32 = 64;
const BUTTON_X: i32 = SIZE_X + SIZE_W + GAP;
const GROUP_Y: i32 = LIST_Y + LIST_H + 10;
const GROUP: Rect = Rect::new(
    MARGIN,
    GROUP_Y,
    SIZE_X + SIZE_W - MARGIN,
    CLIENT.1 - MARGIN - GROUP_Y,
);
/// The sample's room inside the group: under its title, clear of its frame.
const SAMPLE_INSET: (i32, i32, i32, i32) = (8, 16, 8, 8);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Style {
    #[default]
    Regular,
    Italic,
    Bold,
    BoldItalic,
}

impl Style {
    pub const ALL: [Style; 4] = [
        Style::Regular,
        Style::Italic,
        Style::Bold,
        Style::BoldItalic,
    ];

    pub fn bold(self) -> bool {
        matches!(self, Style::Bold | Style::BoldItalic)
    }

    pub fn italic(self) -> bool {
        matches!(self, Style::Italic | Style::BoldItalic)
    }

    pub fn label(self) -> String {
        match self {
            Style::Regular => tr!("fontdlg-regular"),
            Style::Italic => tr!("fontdlg-italic"),
            Style::Bold => tr!("fontdlg-bold"),
            Style::BoldItalic => tr!("fontdlg-bold-italic"),
        }
    }
}

/// A font as the dialog picks it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub family: String,
    pub style: Style,
    /// Points; [`px`] gives the pixels.
    pub size: u32,
}

/// `pt` points in pixels at 96 dpi, rounded.
pub fn px(pt: u32) -> u32 {
    (pt * 96 + 36) / 72
}

fn list(id: &'static str, x: i32, w: i32, items: Vec<String>, at: Option<usize>) -> Item {
    Item::new(
        id,
        Rect::new(x, LIST_Y, w, LIST_H),
        Control::ListBox {
            list: ListBox::new(items, at),
            icons: Vec::new(),
        },
    )
}

/// The dialog on `pick`, the families listed from `families` (sorted, as
/// [`crate::text::families`] gives them). Font has the keyboard with its text selected.
pub fn form(families: &[String], pick: &Pick) -> Form {
    let family_at = families.iter().position(|f| *f == pick.family);
    let style_at = Style::ALL.iter().position(|s| *s == pick.style);
    let size_at = SIZES.iter().position(|s| *s == pick.size);
    let styles = Style::ALL.map(Style::label).to_vec();
    let sizes = SIZES.map(|s| s.to_string()).to_vec();
    let labelled = |id, x, w, text: &str| Item::label_at(id, x, LABEL_Y, w, text);
    let edit = |id, x, w, text: &str| Item::edit(id, Rect::new(x, EDIT_Y, w, EDIT_H), text, true);
    let mut f = Form::new(vec![
        labelled("familylabel", MARGIN, FAMILY_W, &tr!("fontdlg-font")),
        edit(FAMILY, MARGIN, FAMILY_W, &pick.family),
        list(FAMILY_LIST, MARGIN, FAMILY_W, families.to_vec(), family_at),
        labelled("stylelabel", STYLE_X, STYLE_W, &tr!("fontdlg-style")),
        edit(STYLE, STYLE_X, STYLE_W, &pick.style.label()),
        list(STYLE_LIST, STYLE_X, STYLE_W, styles, style_at),
        labelled("sizelabel", SIZE_X, SIZE_W, &tr!("fontdlg-size")),
        edit(SIZE, SIZE_X, SIZE_W, &pick.size.to_string()),
        list(SIZE_LIST, SIZE_X, SIZE_W, sizes, size_at),
        Item::button(
            OK,
            Rect::new(BUTTON_X, EDIT_Y, BUTTON_W, BUTTON_H),
            &tr!("ok"),
            true,
            true,
        ),
        Item::button(
            CANCEL,
            Rect::new(BUTTON_X, EDIT_Y + BUTTON_H + 6, BUTTON_W, BUTTON_H),
            &tr!("cancel"),
            true,
            false,
        ),
        Item::group(SAMPLE, GROUP, &tr!("fontdlg-sample")),
    ])
    .set_in(DialogFont::ShellDlg);
    for (id, at) in [
        (FAMILY_LIST, family_at),
        (STYLE_LIST, style_at),
        (SIZE_LIST, size_at),
    ] {
        f.select_list(id, at);
    }
    f.focus_id(FAMILY);
    f.select_edit_text(FAMILY);
    f
}

fn selected(f: &Form, id: &str) -> Option<usize> {
    match f.item(id).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

fn row_text(f: &Form, id: &str) -> Option<String> {
    match f.item(id).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => {
            list.selected.and_then(|i| list.items.get(i)).cloned()
        }
        _ => None,
    }
}

/// The first row of list `id` that `typed` starts, case folded; an exact name wins.
fn row_for(f: &Form, id: &str, typed: &str) -> Option<usize> {
    let Some(Control::ListBox { list, .. }) = f.item(id).map(|i| &i.control) else {
        return None;
    };
    let typed = typed.trim().to_lowercase();
    if typed.is_empty() {
        return None;
    }
    let names = || list.items.iter().map(|n| n.to_lowercase());
    names()
        .position(|n| n == typed)
        .or_else(|| names().position(|n| n.starts_with(&typed)))
}

/// Keeps a field and its list agreeing after the form gave out `ev`: a row picked writes its
/// text into the field, text typed picks the list's first row it starts.
pub fn follow(f: &mut Form, ev: &form::Event) {
    match *ev {
        form::Event::Changed(id @ (FAMILY_LIST | STYLE_LIST | SIZE_LIST)) => {
            let field = match id {
                FAMILY_LIST => FAMILY,
                STYLE_LIST => STYLE,
                _ => SIZE,
            };
            if let Some(text) = row_text(f, id) {
                f.show_edit_text(field, &text);
            }
        }
        form::Event::Changed(id @ (FAMILY | STYLE | SIZE)) => {
            let list = match id {
                FAMILY => FAMILY_LIST,
                STYLE => STYLE_LIST,
                _ => SIZE_LIST,
            };
            let typed = f.edit_text(id).to_string();
            let row = match id {
                SIZE => SIZES.iter().position(|s| s.to_string() == typed.trim()),
                _ => row_for(f, list, &typed),
            };
            if row.is_some() || id == SIZE {
                f.select_list(list, row);
            }
        }
        _ => {}
    }
}

/// The font the dialog holds: the family and style rows picked, the size typed (or picked).
/// None while no family is picked or the size is not a whole number in `1..=MAX_PT`.
pub fn pick(f: &Form) -> Option<Pick> {
    let family = row_text(f, FAMILY_LIST)?;
    let style = selected(f, STYLE_LIST).map_or(Style::Regular, |i| Style::ALL[i]);
    let size = f
        .edit_text(SIZE)
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|s| (1..=MAX_PT).contains(s))?;
    Some(Pick {
        family,
        style,
        size,
    })
}

/// The sample's renderer and its size in device pixels; `text` is already the italic face for
/// an italic pick.
pub struct Sample<'a> {
    pub text: &'a TextRenderer,
    pub bold: bool,
    pub px: u32,
}

fn sample_rect(f: &Form) -> Option<Rect> {
    let g = f.rect(SAMPLE)?;
    let (l, t, r, b) = SAMPLE_INSET;
    Some(Rect::new(
        g.x + l,
        g.y + t,
        g.width - l - r,
        g.height - t - b,
    ))
}

/// The form, then [`SAMPLE_TEXT`] in `sample` centred in the Sample group, cut to it.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, f: &Form, px: u32, sample: &Sample) {
    form::draw(c, text, p, f, true, px);
    let Some(r) = sample_rect(f).map(|r| r.scaled(p.s)) else {
        return;
    };
    sample.text.draw_clipped(
        c,
        r,
        r,
        SAMPLE_TEXT,
        sample.bold,
        p.cs.window_text,
        Align::Center,
        sample.px,
    );
}

/// The form's nodes and the sample as an image titled with its text; the focused node, as
/// [`Form::access`] gives it.
pub fn access(
    f: &Form,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
) -> Option<NodeId> {
    let focus = f.access(tree, parent, text, px);
    if let Some(r) = sample_rect(f) {
        tree.push(
            parent,
            SAMPLE_TEXT,
            access::titled(Role::Image, r, SAMPLE_TEXT),
        );
    }
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::theme::Theme;

    fn families() -> Vec<String> {
        [
            "Arial",
            "DejaVu Sans",
            "Lucida Console",
            "MesloLGL Nerd Font",
            "MesloLGL Nerd Font Mono",
            "MesloLGL Nerd Font Propo",
            "Tahoma",
            "Verdana",
        ]
        .map(String::from)
        .to_vec()
    }

    fn tahoma(style: Style, size: u32) -> Pick {
        Pick {
            family: "Tahoma".into(),
            style,
            size,
        }
    }

    #[test]
    fn the_form_reads_back_the_pick_it_was_built_on() {
        let pick = tahoma(Style::BoldItalic, 12);
        let f = form(&families(), &pick);
        assert_eq!(super::pick(&f), Some(pick));
        assert_eq!(f.edit_text(FAMILY), "Tahoma");
        assert_eq!(f.edit_text(STYLE), "Bold Italic");
        assert_eq!(f.edit_text(SIZE), "12");
    }

    #[test]
    fn typing_picks_the_first_row_it_starts_and_a_row_picked_fills_the_field() {
        let mut f = form(&families(), &tahoma(Style::Regular, 10));
        let family = |f: &Form| super::pick(f).map(|p| p.family);
        f.show_edit_text(FAMILY, "luc");
        follow(&mut f, &form::Event::Changed(FAMILY));
        assert_eq!(family(&f).as_deref(), Some("Lucida Console"));
        f.show_edit_text(FAMILY, "zzz");
        follow(&mut f, &form::Event::Changed(FAMILY));
        assert_eq!(
            family(&f).as_deref(),
            Some("Lucida Console"),
            "a name no row starts keeps the row"
        );

        f.select_list(STYLE_LIST, Some(2));
        follow(&mut f, &form::Event::Changed(STYLE_LIST));
        assert_eq!(f.edit_text(STYLE), "Bold");

        f.show_edit_text(SIZE, "13");
        follow(&mut f, &form::Event::Changed(SIZE));
        assert_eq!(
            super::pick(&f).map(|p| p.size),
            Some(13),
            "a size not listed"
        );
        assert_eq!(selected(&f, SIZE_LIST), None);
        f.show_edit_text(SIZE, "0");
        follow(&mut f, &form::Event::Changed(SIZE));
        assert_eq!(super::pick(&f), None, "no font is 0 points");
    }

    #[test]
    fn ten_points_is_thirteen_pixels() {
        assert_eq!(px(10), 13);
        assert_eq!(px(12), 16);
        assert_eq!(px(72), 96);
    }

    #[test]
    fn the_dialog_fits_its_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, _, _| {
            vec![(
                "Font".into(),
                form(&families(), &tahoma(Style::BoldItalic, 10)),
            )]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    use crate::pins::fnv;

    fn looks() -> Vec<(String, Theme)> {
        let mut out = vec![("Classic".to_string(), Theme::default())];
        out.push((
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ));
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let g = crate::glass::Settings {
                mode,
                ..Default::default()
            };
            let t = interfaces::theme_with(Interface::Glass, Default::default(), false, &g);
            out.push((format!("Glass {mode:?}"), t));
        }
        out
    }

    fn drawn(theme: &Theme, pick: &Pick, s: i32) -> Canvas {
        let theme = theme.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let sample_text = match pick.style.italic() {
            true => text.italic(),
            false => &text,
        };
        let p = theme.paint();
        let f = form(&families(), pick);
        let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
        c.fill_rect(c.bounds(), p.cs.button_face);
        let sample = Sample {
            text: sample_text,
            bold: pick.style.bold(),
            px: px(pick.size) * s as u32,
        };
        draw(&mut c, &text, &p, &f, theme.fonts.size_px, &sample);
        c
    }

    #[test]
    fn a_bold_sample_is_set_wider_than_a_regular_one() {
        crate::pins::fonts();
        let classic = Theme::default();
        let face = classic.paint().cs.button_face.argb();
        let (l, t, r, b) = SAMPLE_INSET;
        let room = Rect::new(
            GROUP.x + l,
            GROUP.y + t,
            GROUP.width - l - r,
            GROUP.height - t - b,
        );
        // The leftmost and rightmost columns holding ink in the sample's room.
        let span = |c: &Canvas| {
            let inked = |x: i32| {
                (room.y..room.y + room.height)
                    .any(|y| c.pixels[(y * c.width as i32 + x) as usize] != face)
            };
            let cols: Vec<i32> = (room.x..room.x + room.width)
                .filter(|&x| inked(x))
                .collect();
            cols.last().copied().unwrap_or(0) - cols.first().copied().unwrap_or(0)
        };
        let regular = span(&drawn(&classic, &tahoma(Style::Regular, 24), 1));
        let bold = span(&drawn(&classic, &tahoma(Style::Bold, 24), 1));
        assert!(regular > 0, "the sample is drawn");
        assert!(bold > regular, "{bold} > {regular}");
    }

    #[test]
    fn variant_family_names_show_whole() {
        crate::pins::fonts();
        let (_, cut) = crate::pins::shortened_text(|| {
            drawn(&Theme::default(), &tahoma(Style::Regular, 10), 1)
        });
        let cut: Vec<_> = cut.iter().map(|c| c.text.as_str()).collect();
        assert!(cut.is_empty(), "nothing ends in dots: {cut:?}");
    }

    #[test]
    fn font_dialog_pins_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut got = Vec::new();
        for (name, theme) in looks() {
            for pick in [tahoma(Style::Regular, 10), tahoma(Style::Bold, 24)] {
                for s in [1, 2] {
                    let state = format!("{:?} {}", pick.style, pick.size);
                    got.push((format!("{name} {state} {s}"), fnv(&drawn(&theme, &pick, s))));
                }
            }
        }
        crate::pins::check("font_dialog", &got);
    }
}
