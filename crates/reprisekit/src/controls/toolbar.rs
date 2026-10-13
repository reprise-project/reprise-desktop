//! Flat icon toolbar strip in two sizes: the file dialog's 23 px icon-only strip and Explorer's 30 px strip (24 px icon with a label beside it; one button can latch down).
//! On Explorer's strip, 1 px in, items chain flush: Back 73 px, Forward 46, Up 30, a 6 px separator, Folders 74, a separator, Views 39.
//! Forward is a labelled drop button with an empty label: `Item::DropButton { label: Some(String::new()), .. }` drops [`LABEL_GAP`] but keeps [`LABEL_PAD`].
//! `layout` and `hit` work in the caller's unscaled coordinates; `draw` scales each rect by its `Paint`'s scale.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{self};
use crate::look::Paint;
use crate::look::widgets::toolbar as look;
use crate::metrics::{ButtonPads, ControlMetrics, ToolbarMetrics};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use accesskit::{NodeId, Role};

pub const BACK: &str = "back";
pub const FORWARD: &str = "forward";
pub const UP: &str = "up";
pub const NEW_FOLDER: &str = "newfolder";
pub const FOLDERS: &str = "folders";
pub const VIEWS: &str = "views";

pub const BUTTON: i32 = widgets::BUTTON_H;

/// Drop arrow width added to a small-strip `DropButton`: 30 px in all.
pub const DROP_W: i32 = 7;

/// Small strip height; a hot button's ring is 22 rows.
pub const HEIGHT: i32 = 22;

/// Small drop arrow: rows of 5, 3 and 1 px, top row 10 px under the button's top, left edge 9 px in from its right.
const ARROW_DY: i32 = 10;
const ARROW_RIGHT: i32 = 9;

/// Small-strip separator width: a centred 1 px etched line in an 8 px gap.
pub const SEP_W: i32 = 8;

pub use crate::classic::toolbar::BIG_SEP_INSET;

/// Classic's big strip height; a look sets its own in [`ToolbarMetrics::height`].
pub const BIG_HEIGHT: i32 = ToolbarMetrics::CLASSIC.height;

/// The big strip's icon box; the art varies within the 24x24 icons, so the box lays out.
pub const BIG_ICON: i32 = 24;

/// A big button's left inset, from its frame to its icon box.
pub const BIG_PAD: i32 = 3;

/// Icon box right edge to the label's pen.
pub const LABEL_GAP: i32 = 5;

/// Label advance end to the frame's right edge: 74 - `BIG_PAD` - `BIG_ICON` - `LABEL_GAP` - `measure("Folders")` (35) = 7.
pub const LABEL_PAD: i32 = 7;

/// Drop arrow width on the big strip: Views' 39 px less its 27 px icon part.
pub const BIG_DROP_W: i32 = 12;

/// Big strip separator slot: 6 px, etched line 2 px in.
pub const BIG_SEP_W: i32 = 6;

/// One item in the strip. `id` names it in reports; `label` is drawn right of the icon in big mode and ignored in small mode.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Button {
        id: &'static str,
        icon: Option<Canvas>,
        label: Option<String>,
        enabled: bool,
    },
    DropButton {
        id: &'static str,
        icon: Option<Canvas>,
        label: Option<String>,
        enabled: bool,
    },
    Separator,
}

/// Which half of a `DropButton` the pointer is on: the icon (invokes) or the arrow (drops the menu).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Main,
    Drop,
}

/// The strip's state: items, `hot`, `pressed`, `latched` (held down while its pane is open: Explorer's Folders), and `big` (the 30 px labelled strip).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolBar {
    pub items: Vec<Item>,
    pub hot: Option<usize>,
    pub pressed: Option<usize>,
    pub latched: Option<usize>,
    pub big: bool,
}

fn label_of(item: &Item) -> Option<&str> {
    match item {
        Item::Button { label, .. } | Item::DropButton { label, .. } => label.as_deref(),
        Item::Separator => None,
    }
}

/// Pads of an unlabelled drop button (Views): no trailing space, its drop part carries one.
const VIEWS_PADS: ButtonPads = ButtonPads {
    inset: BIG_PAD,
    gap: 0,
    trail: 0,
};

/// Arrow offset below the strip's middle for an unlabelled drop button.
const VIEWS_ARROW_DY: i32 = 1;

fn pads(drop: bool, label: Option<&str>, m: &ToolbarMetrics) -> ButtonPads {
    match (drop, label) {
        (true, Some("")) => m.split_empty,
        (true, Some(_)) => m.split,
        (true, None) => VIEWS_PADS,
        (false, Some(_)) => m.labelled,
        (false, None) => m.bare,
    }
}

fn main_width(
    big: bool,
    drop: bool,
    label: Option<&str>,
    text: &TextRenderer,
    px: u32,
    m: &ToolbarMetrics,
) -> i32 {
    if !big {
        return BUTTON;
    }
    let pads = pads(drop, label, m);
    let label = match label {
        Some(l) if !l.is_empty() => pads.gap + text.measure(l, false, px),
        _ => 0,
    };
    pads.inset + BIG_ICON + label + pads.trail
}

fn drop_width(big: bool, item: &Item, m: &ToolbarMetrics) -> i32 {
    match (big, is_split(item)) {
        (false, _) => DROP_W,
        (true, true) => m.split_drop,
        (true, false) => BIG_DROP_W,
    }
}

pub fn height(big: bool, cm: &ControlMetrics) -> i32 {
    if big { cm.toolbar.height } else { HEIGHT }
}

/// Each item's rect inside `r`, flush left to right from the look's lead on the big strip. All share `r.height`.
pub fn layout(
    tb: &ToolBar,
    text: &TextRenderer,
    px: u32,
    r: Rect,
    cm: &ControlMetrics,
) -> Vec<Rect> {
    let m = &cm.toolbar;
    let mut x = r.x + if tb.big { m.lead } else { 0 };
    tb.items
        .iter()
        .map(|item| {
            let w = match item {
                Item::Button { .. } => main_width(tb.big, false, label_of(item), text, px, m),
                Item::DropButton { .. } => {
                    main_width(tb.big, true, label_of(item), text, px, m)
                        + drop_width(tb.big, item, m)
                }
                Item::Separator => {
                    if tb.big {
                        BIG_SEP_W
                    } else {
                        SEP_W
                    }
                }
            };
            let rect = Rect::new(x, r.y, w, r.height);
            x += w;
            rect
        })
        .collect()
}

/// The item under `(x, y)`, or `None` over a separator, a disabled item or empty space.
pub fn hit(tb: &ToolBar, rects: &[Rect], x: i32, y: i32) -> Option<usize> {
    hit_part(tb, rects, x, y, &ControlMetrics::CLASSIC).map(|(i, _)| i)
}

/// The item under `(x, y)` and which half: the drop part along its right edge is [`Part::Drop`], the rest [`Part::Main`].
pub fn hit_part(
    tb: &ToolBar,
    rects: &[Rect],
    x: i32,
    y: i32,
    cm: &ControlMetrics,
) -> Option<(usize, Part)> {
    tb.items
        .iter()
        .zip(rects)
        .enumerate()
        .find_map(|(i, (item, rect))| {
            if !rect.contains(x, y) {
                return None;
            }
            match item {
                Item::Button { enabled: true, .. } => Some((i, Part::Main)),
                Item::DropButton { enabled: true, .. } => {
                    let part = if x >= rect.right() - drop_width(tb.big, item, &cm.toolbar) {
                        Part::Drop
                    } else {
                        Part::Main
                    };
                    Some((i, part))
                }
                _ => None,
            }
        })
}

/// The icon centred in `r`, shifted `off` (a held button's content moves a pixel), faded to 50% when disabled.
fn draw_icon(c: &mut Canvas, r: Rect, icon: Option<&Canvas>, enabled: bool, off: i32) {
    let Some(icon) = icon else {
        return;
    };
    let x = r.x + (r.width - icon.width as i32) / 2 + off;
    let y = r.y + (r.height - icon.height as i32) / 2 + off;
    if enabled {
        c.blit_over(x, y, icon);
    } else {
        c.blit_over_faded(x, y, icon, 1, 2);
    }
}

/// The big strip's drop arrow, centred in the drop part and `dy` px below the vertical centre.
fn draw_big_arrow(c: &mut Canvas, r: Rect, s: i32, dy: i32, color: crate::theme::Color) {
    let x0 = r.x + (r.width - 5 * s) / 2;
    let y0 = r.y + (r.height - 3 * s) / 2 + dy * s;
    widgets::draw_small_arrow(c, x0, y0, s, color);
}

/// Whether `item` is a split button (a labelled drop button); an unlabelled one keeps its arrow in one frame.
fn is_split(item: &Item) -> bool {
    matches!(item, Item::DropButton { label: Some(_), .. })
}

/// The strip: background over `r`, then each item in its `layout` rect.
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    tb: &ToolBar,
    r: Rect,
    rects: &[Rect],
    px: u32,
) {
    let s = p.s;
    let m = &p.look.controls().toolbar;
    look::draw_background(c, p, r.scaled(s));
    for (i, (item, &rect)) in tb.items.iter().zip(rects).enumerate() {
        let sr = rect.scaled(s);
        let (Item::Button {
            icon,
            label,
            enabled,
            ..
        }
        | Item::DropButton {
            icon,
            label,
            enabled,
            ..
        }) = item
        else {
            look::draw_separator(c, p, tb.big, sr);
            continue;
        };
        let (icon, label, enabled) = (icon.as_ref(), label.as_deref(), *enabled);
        let drop = matches!(item, Item::DropButton { .. });
        let state = look::state(
            enabled,
            tb.hot == Some(i),
            tb.pressed == Some(i),
            tb.latched == Some(i),
        );
        let drop_w = drop_width(tb.big, item, m) * s;
        let split = is_split(item).then(|| sr.right() - drop_w);
        look::draw_button(c, p, sr, split, state);
        let off = look::content_shift(p, state);

        let main_w = main_width(tb.big, drop, label, text, px, m) * s;
        let pads = pads(drop, label, m);
        let icon_box = if tb.big {
            Rect::new(
                sr.x + pads.inset * s,
                sr.y + (sr.height - BIG_ICON * s) / 2,
                BIG_ICON * s,
                BIG_ICON * s,
            )
        } else {
            Rect::new(sr.x, sr.y, main_w, sr.height)
        };
        draw_icon(c, icon_box, icon, enabled, off);

        if tb.big
            && let Some(label) = label
        {
            let lx = icon_box.right() + pads.gap * s + off;
            let lr = Rect::new(lx, sr.y + off, sr.right() - lx, sr.height);
            look::draw_label(c, text, p, lr, label, state, px);
        }

        if drop {
            let arrow_r = Rect::new(sr.right() - drop_w + off, sr.y + off, drop_w, sr.height);
            let color = look::arrow_ink(p, enabled);
            if tb.big {
                let dy = if split.is_some() {
                    m.split_arrow_dy
                } else {
                    VIEWS_ARROW_DY
                };
                draw_big_arrow(c, arrow_r, s, dy, color);
            } else {
                let x0 = sr.right() - ARROW_RIGHT * s + off;
                let y0 = sr.y + ARROW_DY * s + off;
                widgets::draw_small_arrow(c, x0, y0, s, color);
            }
        }
    }
}

fn spoken_name(id: &str) -> &str {
    match id {
        BACK => "Back",
        FORWARD => "Forward",
        UP => "Up",
        FOLDERS => "Folders",
        VIEWS => "Views",
        "newfolder" => "Create New Folder",
        other => other,
    }
}

/// The toolbar's `access` part over `rects` (from [`layout`]); icon-only buttons are named by what their id stands for.
pub fn access(
    tb: &ToolBar,
    r: Rect,
    rects: &[Rect],
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let bar = tree.push(parent, key, access::node(Role::Toolbar, r));
    for (i, (item, rect)) in tb.items.iter().zip(rects).enumerate() {
        let (id, label, enabled, drop) = match item {
            Item::Button {
                id, label, enabled, ..
            } => (*id, label, *enabled, false),
            Item::DropButton {
                id, label, enabled, ..
            } => (*id, label, *enabled, true),
            Item::Separator => continue,
        };
        let label = label
            .as_deref()
            .filter(|l| !l.is_empty())
            .unwrap_or_else(|| spoken_name(id));
        let mut n = access::button(*rect, label, enabled, false);
        if drop {
            n.set_has_popup(accesskit::HasPopup::Menu);
        }
        if tb.latched == Some(i) {
            n.set_toggled(accesskit::Toggled::True);
        }
        tree.push(bar, id, n);
    }
    bar
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{FontConfig, Theme};

    const FOLDERS_W: i32 = 74;
    const VIEWS_W: i32 = 39;
    const FORWARD_W: i32 = 46;

    fn renderer() -> TextRenderer {
        TextRenderer::load(&FontConfig::default())
    }

    fn button(id: &'static str, label: Option<&str>, enabled: bool) -> Item {
        Item::Button {
            id,
            icon: None,
            label: label.map(str::to_string),
            enabled,
        }
    }

    fn drop_button(id: &'static str, label: Option<&str>, enabled: bool) -> Item {
        Item::DropButton {
            id,
            icon: None,
            label: label.map(str::to_string),
            enabled,
        }
    }

    fn small(items: Vec<Item>) -> ToolBar {
        ToolBar {
            items,
            hot: None,
            pressed: None,
            latched: None,
            big: false,
        }
    }

    fn explorer_strip() -> ToolBar {
        ToolBar {
            items: vec![
                drop_button(BACK, Some("Back"), true),
                drop_button(FORWARD, Some(""), false),
                button(UP, None, true),
                Item::Separator,
                button(FOLDERS, Some("Folders"), true),
                Item::Separator,
                drop_button(VIEWS, None, true),
            ],
            hot: None,
            pressed: None,
            latched: Some(4),
            big: true,
        }
    }

    #[test]
    fn layout_places_the_four_buttons_on_the_small_grid() {
        const EXPECTED_BUTTON: i32 = 23;
        const EXPECTED_DROP: i32 = 7;
        let text = renderer();
        let tb = small(vec![
            drop_button(BACK, None, false),
            button(UP, None, true),
            button(NEW_FOLDER, None, true),
            drop_button(VIEWS, None, true),
        ]);
        let r = Rect::new(0, 0, 200, HEIGHT);
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        assert_eq!(rects.len(), 4);
        assert_eq!(rects[0].width, EXPECTED_BUTTON + EXPECTED_DROP);
        assert_eq!(rects[1].width, EXPECTED_BUTTON);
        assert_eq!(rects[1].height, HEIGHT);
        assert_eq!(rects[1].x, rects[0].right());
    }

    #[test]
    fn a_small_button_ignores_its_label() {
        let text = renderer();
        let plain = small(vec![button(UP, None, true)]);
        let labelled = small(vec![button(UP, Some("Up One Level"), true)]);
        let r = Rect::new(0, 0, 200, HEIGHT);
        assert_eq!(
            layout(&plain, &text, 11, r, &ControlMetrics::CLASSIC)[0].width,
            layout(&labelled, &text, 11, r, &ControlMetrics::CLASSIC)[0].width
        );
    }

    #[test]
    fn big_layout_places_explorers_strip() {
        if !crate::text::test_tahoma() {
            return;
        }
        let text = renderer();
        let tb = explorer_strip();
        let r = Rect::new(0, 27, 400, BIG_HEIGHT);
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        let edges: Vec<(i32, i32)> = rects.iter().map(|b| (b.x, b.width)).collect();
        assert_eq!(
            edges,
            vec![
                (1, 73),          // Back, labelled drop button
                (74, FORWARD_W),  // Forward, empty label
                (120, 30),        // Up
                (150, BIG_SEP_W), // separator, line at 152/153
                (156, FOLDERS_W), // Folders, x 156..229
                (230, BIG_SEP_W), // separator, line at 232/233
                (236, VIEWS_W),   // Views, x 236..274
            ]
        );
        assert!(rects.iter().all(|b| b.height == BIG_HEIGHT));
    }

    #[test]
    fn blues_big_strip_lays_out_wider_history_buttons() {
        if !crate::text::test_tahoma() {
            return;
        }
        let text = renderer();
        let tb = explorer_strip();
        let cm = crate::interfaces::blue_theme(crate::blue::Variant::Blue)
            .look
            .controls();
        let r = Rect::new(0, 24, 792, 38);
        let rects = layout(&tb, &text, 11, r, &cm);
        let edges: Vec<(i32, i32)> = rects.iter().map(|b| (b.x, b.width)).collect();
        assert_eq!(
            edges,
            vec![
                (2, 77),
                (79, 51),
                (130, 32),
                (162, BIG_SEP_W),
                (168, FOLDERS_W),
                (242, BIG_SEP_W),
                (248, VIEWS_W),
            ]
        );
        assert_eq!(drop_width(true, &tb.items[0], &cm.toolbar), 14);
        assert_eq!(drop_width(true, &tb.items[6], &cm.toolbar), BIG_DROP_W);
        let back = pads(true, Some("Back"), &cm.toolbar);
        assert_eq!((back.inset, back.gap), (6, 5));
        let folders = pads(false, Some("Folders"), &cm.toolbar);
        assert_eq!((folders.inset, folders.gap), (3, 4));
        let b = rects[0];
        assert_eq!(
            hit_part(&tb, &rects, b.right() - 14, 30, &cm),
            Some((0, Part::Drop))
        );
        assert_eq!(
            hit_part(&tb, &rects, b.right() - 15, 30, &cm),
            Some((0, Part::Main))
        );
    }

    #[test]
    fn an_empty_label_reserves_its_trailing_pad() {
        let text = renderer();
        let r = Rect::new(0, 0, 400, BIG_HEIGHT);
        let width = |label: Option<&str>| {
            let tb = ToolBar {
                items: vec![drop_button(FORWARD, label, true)],
                hot: None,
                pressed: None,
                latched: None,
                big: true,
            };
            layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC)[0].width
        };
        assert_eq!(width(Some("")), FORWARD_W, "Forward 74..119");
        assert_eq!(width(Some("")), BIG_PAD + BIG_ICON + LABEL_PAD + BIG_DROP_W);
        assert_eq!(width(None), VIEWS_W, "Views 308..346");
    }

    #[test]
    fn a_big_labelled_button_places_its_icon_and_label() {
        if !crate::text::test_tahoma() {
            return;
        }
        const FRAME: i32 = 228;
        const ICON_BOX: i32 = 231;
        const LABEL_PEN: i32 = 260;
        assert_eq!(FRAME + BIG_PAD, ICON_BOX);
        assert_eq!(ICON_BOX + BIG_ICON + LABEL_GAP, LABEL_PEN);
        let text = renderer();
        assert_eq!(
            BIG_PAD + BIG_ICON + LABEL_GAP + text.measure("Folders", false, 11) + LABEL_PAD,
            FOLDERS_W
        );
    }

    #[test]
    fn a_big_drop_arrow_inks_at_338() {
        const FRAME: i32 = 308;
        const ARROW_X: i32 = 338;
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let tb = ToolBar {
            items: vec![drop_button(VIEWS, None, true)],
            hot: None,
            pressed: None,
            latched: None,
            big: true,
        };
        let r = Rect::new(FRAME, 0, VIEWS_W, BIG_HEIGHT);
        let mut c = Canvas::new((FRAME + 60) as u32, BIG_HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &tb, r, &[r], 11);
        let inked: Vec<i32> = (FRAME..FRAME + VIEWS_W)
            .filter(|&x| (0..BIG_HEIGHT).any(|y| c.get(x, y) == cs.button_text.argb()))
            .collect();
        assert_eq!(inked.first().copied(), Some(ARROW_X));
        assert_eq!(inked.len(), 5, "the arrow is 5 px at its widest row");
    }

    /// The big strip grows with its labels' line, 13 px at 100%; the 24 px icons centre down it.
    #[test]
    fn a_big_strip_grows_to_fit_its_grown_labels() {
        const EXPECTED: [(u32, i32); 3] = [(100, 30), (150, 37), (200, 43)];
        for (pct, h) in EXPECTED {
            let theme = Theme::default().with_text_size(pct);
            let cm = theme.controls();
            assert_eq!(height(true, &cm), h, "{pct}%");
            assert_eq!(
                height(false, &cm),
                HEIGHT,
                "{pct}%: the small strip keeps its height"
            );
            let text = TextRenderer::load(&theme.fonts);
            let px = theme.fonts.size_px;
            let mut icon = Canvas::new(BIG_ICON as u32, BIG_ICON as u32);
            let red = crate::theme::Color::rgb(255, 0, 0);
            icon.fill_rect(icon.bounds(), red);
            let tb = ToolBar {
                items: vec![Item::Button {
                    id: FOLDERS,
                    icon: Some(icon),
                    label: Some("Folders".into()),
                    enabled: true,
                }],
                ..explorer_strip()
            };
            let r = Rect::new(0, 0, 200, h);
            let rects = layout(&tb, &text, px, r, &cm);
            assert_eq!(rects[0].height, h);
            let mut c = Canvas::new(200, h as u32);
            draw(&mut c, &text, &theme.paint(), &tb, r, &rects, px);
            let top = (h - BIG_ICON) / 2;
            let x = rects[0].x + BIG_PAD + 1;
            assert_eq!(c.get(x, top), red.argb(), "{pct}%");
            assert_eq!(c.get(x, top + BIG_ICON - 1), red.argb(), "{pct}%");
            assert_ne!(c.get(x, top - 1), red.argb(), "{pct}%");
            let ink = theme.colors.button_text.argb();
            let rows: Vec<i32> = (0..h)
                .filter(|&y| (x + BIG_ICON..200).any(|x| c.get(x, y) == ink))
                .collect();
            let (first, last) = (rows[0], *rows.last().unwrap());
            assert!(
                first > 1 && last < h - 2,
                "{pct}%: the label's ink {first}..={last} sits inside the strip"
            );
        }
    }

    #[test]
    fn hit_finds_the_item_under_the_pointer() {
        let text = renderer();
        let tb = small(vec![
            button(UP, None, true),
            Item::Separator,
            button(NEW_FOLDER, None, true),
        ]);
        let r = Rect::new(0, 0, 200, HEIGHT);
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        assert_eq!(hit(&tb, &rects, rects[0].x + 2, r.y + 2), Some(0));
        assert_eq!(
            hit(&tb, &rects, rects[1].x + 1, r.y + 2),
            None,
            "a separator is not hittable"
        );
        assert_eq!(hit(&tb, &rects, r.x + 199, r.y + 2), None);
    }

    #[test]
    fn hit_part_splits_a_drop_button() {
        let text = renderer();
        let tb = ToolBar {
            items: vec![drop_button(VIEWS, None, true), button(UP, None, true)],
            hot: None,
            pressed: None,
            latched: None,
            big: true,
        };
        let r = Rect::new(0, 0, 200, BIG_HEIGHT);
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        let b = rects[0];
        assert_eq!(
            hit_part(&tb, &rects, b.x + 2, 4, &ControlMetrics::CLASSIC),
            Some((0, Part::Main))
        );
        assert_eq!(
            hit_part(
                &tb,
                &rects,
                b.right() - BIG_DROP_W,
                4,
                &ControlMetrics::CLASSIC
            ),
            Some((0, Part::Drop))
        );
        assert_eq!(
            hit_part(
                &tb,
                &rects,
                b.right() - BIG_DROP_W - 1,
                4,
                &ControlMetrics::CLASSIC
            ),
            Some((0, Part::Main))
        );
        assert_eq!(
            hit_part(&tb, &rects, rects[1].x + 2, 4, &ControlMetrics::CLASSIC),
            Some((1, Part::Main))
        );
    }

    #[test]
    fn a_disabled_button_is_not_hittable() {
        let text = renderer();
        let tb = small(vec![drop_button(BACK, None, false)]);
        let r = Rect::new(0, 0, 200, HEIGHT);
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        assert_eq!(hit(&tb, &rects, rects[0].x + 2, r.y + 2), None);
    }

    #[test]
    fn a_hot_button_gets_the_thin_raised_border() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let tb = ToolBar {
            hot: Some(0),
            ..small(vec![button(UP, None, true)])
        };
        let r = Rect::new(0, 0, 40, HEIGHT);
        let mut c = Canvas::new(40, HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &tb,
            r,
            &layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC),
            11,
        );
        let rects = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC);
        let b = rects[0];
        assert_eq!(c.get(b.x, b.y), cs.button_hilight.argb());
        assert_eq!(
            c.get(b.right() - 1, b.bottom() - 1),
            cs.button_shadow.argb()
        );

        let cold = ToolBar { hot: None, ..tb };
        let mut c2 = Canvas::new(40, HEIGHT as u32);
        c2.fill_rect(c2.bounds(), cs.button_face);
        draw(
            &mut c2,
            &text,
            &classic_paint(cs, 1),
            &cold,
            r,
            &layout(&cold, &text, 11, r, &ControlMetrics::CLASSIC),
            11,
        );
        assert_eq!(c2.get(b.x, b.y), cs.button_face.argb());
    }

    #[test]
    fn a_pressed_button_gets_the_inverted_bevel() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let tb = ToolBar {
            pressed: Some(0),
            ..small(vec![button(UP, None, true)])
        };
        let r = Rect::new(0, 0, 40, HEIGHT);
        let mut c = Canvas::new(40, HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &tb,
            r,
            &layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC),
            11,
        );
        let b = layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC)[0];
        assert_eq!(c.get(b.x, b.y), cs.button_shadow.argb());
        assert_eq!(
            c.get(b.right() - 1, b.bottom() - 1),
            cs.button_hilight.argb()
        );
    }

    #[test]
    fn a_latched_button_sinks_over_a_plain_face_even_when_hot() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let tb = ToolBar {
            items: vec![button(FOLDERS, Some("Folders"), true)],
            hot: Some(0),
            pressed: None,
            latched: Some(0),
            big: true,
        };
        let r = Rect::new(0, 0, FOLDERS_W, BIG_HEIGHT);
        let mut c = Canvas::new(FOLDERS_W as u32, BIG_HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &tb, r, &[r], 11);
        assert_eq!(c.get(0, 5), cs.button_shadow.argb(), "left edge");
        assert_eq!(
            c.get(FOLDERS_W - 1, 5),
            cs.button_hilight.argb(),
            "right edge"
        );
        for (x, y) in [(22, 5), (23, 5), (22, 6)] {
            assert_eq!(c.get(x, y), cs.button_face.argb(), "no dither at {x},{y}");
        }
    }

    #[test]
    fn a_big_button_gets_the_hot_border_and_the_pressed_shift() {
        if !crate::text::test_tahoma() {
            return;
        }
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let render = |hot: Option<usize>, pressed: Option<usize>| {
            let tb = ToolBar {
                items: vec![button(FOLDERS, Some("Folders"), true)],
                hot,
                pressed,
                latched: None,
                big: true,
            };
            let r = Rect::new(0, 0, FOLDERS_W, BIG_HEIGHT);
            let mut c = Canvas::new(FOLDERS_W as u32, BIG_HEIGHT as u32);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(&mut c, &text, &classic_paint(cs, 1), &tb, r, &[r], 11);
            c
        };
        let label_pen = |c: &Canvas| {
            (0..FOLDERS_W)
                .find(|&x| (0..BIG_HEIGHT).any(|y| c.get(x, y) == cs.button_text.argb()))
                .expect("the label inks something")
        };

        let hot = render(Some(0), None);
        assert_eq!(hot.get(0, 0), cs.button_hilight.argb());
        assert_eq!(
            hot.get(FOLDERS_W - 1, BIG_HEIGHT - 1),
            cs.button_shadow.argb()
        );
        assert_eq!(label_pen(&hot), BIG_PAD + BIG_ICON + LABEL_GAP);

        let pressed = render(None, Some(0));
        assert_eq!(pressed.get(0, 0), cs.button_shadow.argb());
        assert_eq!(
            pressed.get(FOLDERS_W - 1, BIG_HEIGHT - 1),
            cs.button_hilight.argb()
        );
        assert_eq!(
            label_pen(&pressed),
            BIG_PAD + BIG_ICON + LABEL_GAP + 1,
            "held content shifts one px right"
        );
    }

    #[test]
    fn a_big_separator_etches_its_inset_line() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let tb = ToolBar {
            items: vec![Item::Separator],
            hot: None,
            pressed: None,
            latched: None,
            big: true,
        };
        let r = Rect::new(0, 0, BIG_SEP_W, BIG_HEIGHT);
        let mut c = Canvas::new(BIG_SEP_W as u32, BIG_HEIGHT as u32);
        c.fill_rect(c.bounds(), cs.button_face);
        draw(&mut c, &text, &classic_paint(cs, 1), &tb, r, &[r], 11);
        for y in BIG_SEP_INSET..BIG_HEIGHT - BIG_SEP_INSET {
            assert_eq!(c.get(BIG_SEP_INSET, y), cs.button_shadow.argb(), "y {y}");
            assert_eq!(
                c.get(BIG_SEP_INSET + 1, y),
                cs.button_hilight.argb(),
                "y {y}"
            );
        }
        assert_eq!(c.get(BIG_SEP_INSET, 0), cs.button_face.argb());
        assert_eq!(c.get(BIG_SEP_INSET, BIG_HEIGHT - 1), cs.button_face.argb());
        assert_eq!(c.get(0, BIG_HEIGHT / 2), cs.button_face.argb());
    }

    #[test]
    fn a_label_greys_when_its_button_is_disabled() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let ink = |enabled: bool| {
            let tb = ToolBar {
                items: vec![button(FOLDERS, Some("Folders"), enabled)],
                hot: None,
                pressed: None,
                latched: None,
                big: true,
            };
            let r = Rect::new(0, 0, FOLDERS_W, BIG_HEIGHT);
            let mut c = Canvas::new(FOLDERS_W as u32, BIG_HEIGHT as u32);
            c.fill_rect(c.bounds(), cs.button_face);
            draw(
                &mut c,
                &text,
                &classic_paint(cs, 1),
                &tb,
                r,
                &layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC),
                11,
            );
            let mut seen = std::collections::BTreeSet::new();
            for x in BIG_PAD + BIG_ICON..FOLDERS_W {
                for y in 0..BIG_HEIGHT {
                    let p = c.get(x, y);
                    if p != cs.button_face.argb() {
                        seen.insert(p);
                    }
                }
            }
            seen
        };
        assert!(ink(true).contains(&cs.button_text.argb()));
        assert!(ink(false).contains(&cs.gray_text.argb()));
        assert!(!ink(false).contains(&cs.button_text.argb()));
    }

    fn blue_folders(
        v: crate::blue::Variant,
        enabled: bool,
        hot: bool,
        pressed: bool,
        latched: bool,
    ) -> Canvas {
        let theme = crate::interfaces::blue_theme(v);
        let text = renderer();
        let tb = ToolBar {
            items: vec![button(FOLDERS, Some("Folders"), enabled)],
            hot: hot.then_some(0),
            pressed: pressed.then_some(0),
            latched: latched.then_some(0),
            big: true,
        };
        let r = Rect::new(0, 0, FOLDERS_W, BIG_HEIGHT);
        let mut c = Canvas::new(FOLDERS_W as u32, BIG_HEIGHT as u32);
        c.fill_rect(c.bounds(), crate::theme::Color::rgb(1, 2, 3));
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &tb,
            r,
            &layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC),
            11,
        );
        c
    }

    fn label_inks(c: &Canvas) -> std::collections::BTreeSet<u32> {
        (BIG_PAD + BIG_ICON..FOLDERS_W - 4)
            .flat_map(|x| (4..BIG_HEIGHT - 4).map(move |y| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .collect()
    }

    #[test]
    fn a_blue_button_at_rest_leaves_the_band_to_the_rebar() {
        let c = blue_folders(crate::blue::Variant::Blue, true, false, false, false);
        for (x, y) in [(0, 0), (FOLDERS_W - 1, BIG_HEIGHT - 1), (2, 15)] {
            assert_eq!(c.get(x, y), 0xFF010203, "at {x},{y}");
        }
    }

    #[test]
    fn blue_labels_take_the_held_and_greyed_inks() {
        use crate::blue::Variant;
        let held = label_inks(&blue_folders(Variant::Blue, true, true, true, false));
        assert!(held.contains(&0xFFFFFFFF), "a held label is white");
        assert!(!held.contains(&0xFF000000), "and not black");
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let grey = v.scheme().inks.toolbar_disabled.argb();
            let greyed = label_inks(&blue_folders(v, false, false, false, false));
            assert!(greyed.contains(&grey), "{v:?}: the greyed ink");
            assert!(greyed.contains(&0xFFFFFFFF), "{v:?}: the white etch");
            assert!(!greyed.contains(&0xFF000000), "{v:?}: no black");
        }
        let latched = label_inks(&blue_folders(Variant::Blue, true, false, false, true));
        assert!(latched.contains(&0xFF000000), "a latched label stays black");
        for v in [Variant::Slate, Variant::Sapphire, Variant::Obsidian] {
            let held = label_inks(&blue_folders(v, true, true, true, false));
            assert!(held.contains(&0xFF000000), "{v:?}: a held label is black");
        }
    }

    #[test]
    fn a_blue_latched_label_moves_a_pixel() {
        use crate::blue::Variant;
        let pen = |c: &Canvas| {
            (BIG_PAD + BIG_ICON..FOLDERS_W)
                .find(|&x| (0..BIG_HEIGHT).any(|y| c.get(x, y) == 0xFF000000))
                .expect("the label inks black")
        };
        let rest = pen(&blue_folders(Variant::Blue, true, false, false, false));
        let latched = pen(&blue_folders(Variant::Blue, true, false, false, true));
        assert_eq!(latched, rest + 1);
    }

    #[test]
    fn a_blue_greyed_arrow_is_flat_grey() {
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let text = renderer();
        let tb = ToolBar {
            items: vec![drop_button(VIEWS, None, false)],
            hot: None,
            pressed: None,
            latched: None,
            big: true,
        };
        let r = Rect::new(0, 0, VIEWS_W, BIG_HEIGHT);
        let mut c = Canvas::new(VIEWS_W as u32, BIG_HEIGHT as u32);
        c.fill_rect(c.bounds(), crate::theme::Color::rgb(1, 2, 3));
        draw(
            &mut c,
            &text,
            &theme.paint(),
            &tb,
            r,
            &layout(&tb, &text, 11, r, &ControlMetrics::CLASSIC),
            11,
        );
        let inked: Vec<u32> = (0..VIEWS_W)
            .flat_map(|x| (0..BIG_HEIGHT).map(move |y| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .filter(|&p| p != 0xFF010203)
            .collect();
        assert_eq!(inked.len(), 9, "a 5-3-1 arrow");
        let grey = crate::blue::Variant::Blue
            .scheme()
            .inks
            .toolbar_drop_disabled
            .argb();
        assert!(inked.iter().all(|&p| p == grey));
    }

    /// Writes `target/toolbar-preview.png` for a visual check.
    #[test]
    #[ignore]
    fn render_toolbar() {
        let theme = Theme::default();
        let cs = &theme.colors;
        let text = renderer();
        let rb = crate::controls::rebar::Rebar {
            bands: vec![
                crate::controls::rebar::Band { height: 22 },
                crate::controls::rebar::Band { height: BIG_HEIGHT },
                crate::controls::rebar::Band { height: 22 },
            ],
        };
        let r = Rect::new(
            0,
            1,
            400,
            crate::controls::rebar::height(&rb, &crate::metrics::RebarMetrics::CLASSIC),
        );
        let mut c = Canvas::new(
            400,
            (crate::controls::rebar::height(&rb, &crate::metrics::RebarMetrics::CLASSIC) + 1)
                as u32,
        );
        c.fill_rect(c.bounds(), cs.button_face);
        crate::controls::rebar::draw(&mut c, &classic_paint(cs, 1), &rb, r);
        let tb = explorer_strip();
        let band =
            crate::controls::rebar::layout(&rb, r, &crate::metrics::RebarMetrics::CLASSIC)[1];
        let rects = layout(&tb, &text, 11, band, &ControlMetrics::CLASSIC);
        assert_eq!(rects[0].x, 1, "the first item's frame starts at x 1");
        draw(&mut c, &text, &classic_paint(cs, 1), &tb, band, &rects, 11);
        crate::raster::write_preview(&c, "toolbar-preview.png");
    }
}
