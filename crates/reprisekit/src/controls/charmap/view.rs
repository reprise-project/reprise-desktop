//! Character Map's window: the font and block pickers and the search over the grid, the
//! picked character large with its name and code under it, and the characters to copy. The
//! grid and the large character draw over the form's blank items.

use super::{CharMap, Layout, code, layout, name};
use crate::access::Tree;
use crate::access::accesskit::NodeId;
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::combo::ListItem;
use crate::controls::form::{Control, Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::EditKey;

pub const FONT: &str = "font";
pub const BLOCK: &str = "block";
pub const SEARCH: &str = "search";
pub const GRID: &str = "grid";
pub const PREVIEW: &str = "preview";
pub const NAME: &str = "name";
pub const CODE: &str = "code";
pub const TO_COPY: &str = "tocopy";
pub const SELECT: &str = "select";
pub const COPY: &str = "copy";

pub const CLIENT: (i32, i32) = (480, 442);

const MARGIN: i32 = 11;
const LABEL_W: i32 = 52;
const FIELD_X: i32 = MARGIN + LABEL_W;
const ROW2_Y: i32 = 40;
const SEARCH_LABEL_X: i32 = 262;
const SEARCH_X: i32 = 322;
const EDIT_H: i32 = 21;
const GRID_Y: i32 = 70;
const GRID_H: i32 = 268;
const PREVIEW_Y: i32 = GRID_Y + GRID_H + 8;
const PREVIEW_W: i32 = 52;
const COPY_Y: i32 = CLIENT.1 - MARGIN - BUTTON_H;
const COPY_LABEL_W: i32 = 112;
const GAP: i32 = 6;

/// The window with `fonts` to pick from (`font` picked) over `map`'s characters.
pub fn form(fonts: &[String], font: usize, map: &CharMap) -> Form {
    let right = CLIENT.0 - MARGIN;
    let fonts: Vec<&str> = fonts.iter().map(String::as_str).collect();
    let mut font_combo = Item::combo(FONT, FIELD_X, 11, 260, &fonts, 14);
    if let Control::Combo(c) = &mut font_combo.control {
        c.selected = font.min(fonts.len().saturating_sub(1));
    }
    let mut items = vec![
        Item::label_at("fontlabel", MARGIN, 14, LABEL_W - 4, &tr!("charmap-font")),
        font_combo,
        Item::label_at(
            "blocklabel",
            MARGIN,
            ROW2_Y + 3,
            LABEL_W - 4,
            &tr!("charmap-block"),
        ),
        Item::combo(
            BLOCK,
            FIELD_X,
            ROW2_Y,
            SEARCH_LABEL_X - GAP - FIELD_X,
            &[],
            14,
        ),
        Item::label_at(
            "searchlabel",
            SEARCH_LABEL_X,
            ROW2_Y + 3,
            SEARCH_X - SEARCH_LABEL_X - 4,
            &tr!("charmap-search"),
        ),
        Item::edit(
            SEARCH,
            Rect::new(SEARCH_X, ROW2_Y, right - SEARCH_X, EDIT_H),
            &map.query,
            true,
        ),
        Item::new(
            GRID,
            Rect::new(MARGIN, GRID_Y, right - MARGIN, GRID_H),
            Control::Blank,
        ),
        Item::new(
            PREVIEW,
            Rect::new(MARGIN, PREVIEW_Y, PREVIEW_W, PREVIEW_W),
            Control::Blank,
        ),
        Item::label_at(NAME, MARGIN + PREVIEW_W + 10, PREVIEW_Y + 8, 400, ""),
        Item::label_at(CODE, MARGIN + PREVIEW_W + 10, PREVIEW_Y + 28, 400, ""),
        Item::label_at(
            "tocopylabel",
            MARGIN,
            COPY_Y + 5,
            COPY_LABEL_W - 4,
            &tr!("charmap-to-copy"),
        ),
    ];
    let copy_x = right - BUTTON_W;
    let select_x = copy_x - GAP - BUTTON_W;
    let edit_x = MARGIN + COPY_LABEL_W;
    items.push(Item::edit(
        TO_COPY,
        Rect::new(edit_x, COPY_Y + 1, select_x - GAP - edit_x, EDIT_H),
        "",
        true,
    ));
    items.push(Item::button(
        SELECT,
        Rect::new(select_x, COPY_Y, BUTTON_W, BUTTON_H),
        &tr!("charmap-select"),
        true,
        false,
    ));
    items.push(Item::button(
        COPY,
        Rect::new(copy_x, COPY_Y, BUTTON_W, BUTTON_H),
        &tr!("charmap-copy"),
        true,
        false,
    ));
    let mut f = Form::new(items);
    f.set_focus(None);
    follow(&mut f, map);
    f
}

/// Brings the block list, the name and the code to `map`'s.
pub fn follow(f: &mut Form, map: &CharMap) {
    let mut blocks = vec![tr!("charmap-all-blocks")];
    blocks.extend(map.blocks.iter().map(|b| b.name.to_string()));
    if let Some(Control::Combo(c)) = f.item_mut(BLOCK).map(|i| &mut i.control) {
        if c.items.iter().map(|i| &i.text).ne(blocks.iter()) {
            c.items = blocks.into_iter().map(ListItem::new).collect();
            c.top = 0;
        }
        c.selected = map.block.map_or(0, |b| b + 1);
    }
    let (n, k) = match map.picked() {
        Some(ch) => (name(ch).unwrap_or_default(), code(ch)),
        None => (String::new(), String::new()),
    };
    f.set_text(NAME, &n);
    f.set_text(CODE, &k);
}

/// The block the block list picks: `None` for all of them.
pub fn picked_block(f: &Form) -> Option<usize> {
    f.combo_selected(BLOCK)?.checked_sub(1)
}

/// Adds `ch` to the end of the characters to copy.
pub fn append(f: &mut Form, ch: char) {
    if let Some(Control::Edit { edit, .. }) = f.item_mut(TO_COPY).map(|i| &mut i.control) {
        edit.key(EditKey::End { select: false });
        edit.key(EditKey::Insert(ch.to_string()));
    }
}

/// The grid's layout where the form placed it.
pub fn grid_layout(f: &Form, cm: &crate::metrics::ControlMetrics) -> Option<Layout> {
    f.rect(GRID).map(|r| layout(r, cm))
}

/// Draws the form, the grid in `glyphs` and the picked character large.
#[expect(clippy::too_many_arguments)]
pub fn draw_window(
    c: &mut Canvas,
    text: &TextRenderer,
    glyphs: &TextRenderer,
    p: &Paint,
    f: &Form,
    map: &CharMap,
    grid_focused: bool,
    px: u32,
) {
    crate::controls::form::draw(c, text, p, f, true, px);
    let cm = p.look.controls().grown(p.text);
    if let Some(lay) = grid_layout(f, &cm) {
        super::draw(c, glyphs, p, map, &lay, grid_focused);
    }
    if let Some(r) = f.rect(PREVIEW) {
        super::draw_preview(c, glyphs, p, r, map.picked());
    }
    crate::controls::form::draw_popups(c, text, p, f, px);
}

/// The window's access part; returns the focused node.
#[expect(clippy::too_many_arguments)]
pub fn access_window(
    f: &Form,
    map: &CharMap,
    grid_focused: bool,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
    cm: &crate::metrics::ControlMetrics,
) -> Option<NodeId> {
    let focus = f.access(tree, parent, text, px);
    let grid = grid_layout(f, cm).map(|lay| super::access(map, &lay, tree, parent));
    if grid_focused { grid } else { focus }
}
