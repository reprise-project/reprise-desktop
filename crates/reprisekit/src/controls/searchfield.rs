//! A search box: a single-line edit with a grey "Search" cue, placed at the right end of a
//! toolbar band, or as the nav row's field ([`draw_nav`]): a constant width, an italic cue naming
//! the folder, and a magnifier at its right end that turns into a clear button while it holds
//! text.

use crate::access::{Tree, accesskit};
use crate::classic::widgets::access_edit;
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::{draw_edit_field, draw_edit_text};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey, TextEdit, Took, edit_first_visible, index_at};
use crate::theme::TextUnits;
use accesskit::NodeId;

pub fn cue() -> String {
    tr!("searchfield-cue")
}
/// The field's size at 100% text; both grow with the text.
pub const WIDTH: i32 = 180;
pub const HEIGHT: i32 = 22;
/// The gap from the field's right edge to the band's.
pub const RIGHT_PAD: i32 = 4;
/// The least room kept between the band's other items and the field.
pub const GAP: i32 = 8;
/// The nav row's field at 100% text: a constant width, the address taking what is left.
pub const NAV_WIDTH: i32 = 109;
/// From the nav field's left edge to its text and cue.
pub const NAV_TEXT_X: i32 = 8;
/// The clear button inside the nav field's right edge, the whole height inside its frame.
pub const CLEAR_W: i32 = 21;
pub const MAGNIFIER: i32 = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Field,
    Clear,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchField {
    pub edit: TextEdit,
    /// The nav row's cue, "Search <folder>" ([`SearchField::set_cue`]).
    folder_cue: Option<String>,
    /// The nav field's part under the pointer.
    pub hot: Option<Hit>,
    /// The clear button is held down.
    pub clearing: bool,
}

impl Default for SearchField {
    fn default() -> SearchField {
        let mut edit = TextEdit::new("", false);
        edit.set_cue(cue());
        SearchField {
            edit,
            folder_cue: None,
            hot: None,
            clearing: false,
        }
    }
}

impl SearchField {
    /// The nav field's cue names the folder a search would look in.
    pub fn set_cue(&mut self, folder_name: &str) {
        self.folder_cue = Some(tr!("searchfield-cue-in", name = folder_name));
    }

    /// What the field reads as while empty: the folder's cue once set, else "Search".
    pub fn cue_label(&self) -> String {
        self.folder_cue.clone().unwrap_or_else(cue)
    }

    /// Returns whether the lit part changed.
    pub fn hover(&mut self, hit: Option<Hit>) -> bool {
        std::mem::replace(&mut self.hot, hit) != hit
    }

    /// A press on the clear button holds it; the release over it clears ([`SearchField::release`]).
    pub fn press_clear(&mut self) {
        self.clearing = true;
    }

    /// The button came up over `hit`: true when it empties the field.
    pub fn release(&mut self, hit: Option<Hit>) -> bool {
        let fire = std::mem::take(&mut self.clearing) && hit == Some(Hit::Clear);
        if fire {
            self.clear();
        }
        fire
    }

    pub fn text(&self) -> &str {
        &self.edit.text
    }

    pub fn key(&mut self, k: EditKey) -> Took {
        self.edit.key(k)
    }

    /// Taking the keyboard selects what the field holds, so typing replaces it.
    pub fn focus(&mut self) {
        self.edit.select_all();
    }

    pub fn clear(&mut self) {
        self.edit.select_all();
        self.edit.key(EditKey::Delete { word: false });
    }

    /// A press at `x` in `field` (as [`layout`] gave it, drawn at scale `s`) places the caret.
    pub fn click(&mut self, field: Rect, x: i32, text: &TextRenderer, px: u32, s: i32) {
        let i = self.caret_at(field, x, text, px, s);
        self.edit.place(i, false);
    }

    /// The byte a press at `x` in `field` puts the caret at, as [`SearchField::click`] reads it.
    pub fn caret_at(&self, field: Rect, x: i32, text: &TextRenderer, px: u32, s: i32) -> usize {
        let inner = inner(field.scaled(s), s);
        let first = edit_first_visible(text, &self.edit, inner.width, px, 0);
        first + index_at(text, &self.edit.text[first..], x * s - inner.x, px)
    }
}

fn inner(r: Rect, s: i32) -> Rect {
    r.inset(crate::classic::widgets::FIELD_PAD * s)
}

/// The field at the right end of `band`, vertically centred; `None` when the band's items,
/// ending at `items_right`, leave it no room.
pub fn layout(band: Rect, items_right: i32, cm: &ControlMetrics) -> Option<Rect> {
    let (w, h) = (cm.text.len(WIDTH), cm.text.len(HEIGHT).min(band.height));
    let x = band.right() - cm.text.len(RIGHT_PAD) - w;
    (x - items_right >= cm.text.len(GAP)).then(|| {
        let y = band.y + (band.height - h) / 2;
        Rect::new(x, y, w, h)
    })
}

pub fn hit(field: Rect, x: i32, y: i32) -> bool {
    field.contains(x, y)
}

/// The nav row's field at the right end of `row`, centred down it, `h` tall.
pub fn nav_layout(row: Rect, h: i32, u: TextUnits) -> Rect {
    let w = u.len(NAV_WIDTH).min(row.width.max(0));
    Rect::new(row.right() - w, row.y + (row.height - h) / 2, w, h)
}

/// The nav field's clear button, which is also where its magnifier sits.
pub fn clear_rect(field: Rect) -> Rect {
    Rect::new(
        field.right() - 1 - CLEAR_W,
        field.y + 1,
        CLEAR_W,
        field.height - 2,
    )
}

/// The nav field's text: from its cue's place to its button.
fn nav_text(field: Rect) -> Rect {
    let b = clear_rect(field);
    Rect::new(
        field.x + NAV_TEXT_X,
        field.y + 2,
        b.x - field.x - NAV_TEXT_X,
        field.height - 4,
    )
}

/// The clear button only while the field holds text; the rest of it is the field.
pub fn nav_hit(sf: &SearchField, field: Rect, x: i32, y: i32) -> Option<Hit> {
    if !field.contains(x, y) {
        return None;
    }
    let clear = !sf.text().is_empty() && clear_rect(field).contains(x, y);
    Some(if clear { Hit::Clear } else { Hit::Field })
}

impl SearchField {
    /// [`SearchField::click`] for the nav field.
    pub fn nav_click(&mut self, field: Rect, x: i32, text: &TextRenderer, px: u32, s: i32) {
        let inner = nav_text(field).scaled(s);
        let first = edit_first_visible(text, &self.edit, inner.width, px, 0);
        let i = first + index_at(text, &self.edit.text[first..], x * s - inner.x, px);
        self.edit.place(i, false);
    }
}

/// The nav row's field: the look's frame (white while it holds the keyboard), the italic cue
/// while empty and unfocused, then the text, and the magnifier or, with text, the clear button.
#[expect(clippy::too_many_arguments)]
pub fn draw_nav(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sf: &SearchField,
    field: Rect,
    focused: bool,
    caret: Caret,
    px: u32,
) {
    let s = p.s;
    let r = field.scaled(s);
    let parts = [Part::SearchField, Part::SearchGlass, Part::SearchClear];
    let st = p.look.style().filter(|st| st.has_parts(&parts));
    match st {
        Some(st) => {
            let state = match (focused, sf.hot.is_some()) {
                (true, _) => State::Focused,
                (false, true) => State::Hot,
                (false, false) => State::Normal,
            };
            st.draw(c, r, Part::SearchField, state, s);
        }
        None => {
            let inner = draw_edit_field(c, p, r, p.cs.window, focused, Interaction::Idle);
            c.fill_rect(inner, p.cs.window);
        }
    }
    let inner = nav_text(field).scaled(s);
    if sf.text().is_empty() && !focused {
        let (it, cue) = (text.italic(), sf.cue_label());
        let shown = it.ellipsize(&cue, inner.width, false, px);
        it.draw(c, inner, &shown, false, p.cs.gray_text, Align::Left, px);
    } else {
        let mut edit = sf.edit.clone();
        edit.set_cue("");
        let first = edit_first_visible(text, &edit, inner.width, px, 0);
        draw_edit_text(c, text, &edit, inner, px, p, caret, first);
    }
    let b = clear_rect(field);
    match st {
        Some(st) if sf.text().is_empty() => {
            let m = Rect::new(
                b.x + (b.width - MAGNIFIER) / 2,
                b.y + (b.height - MAGNIFIER) / 2,
                MAGNIFIER,
                MAGNIFIER,
            );
            st.draw(c, m.scaled(s), Part::SearchGlass, State::Normal, s);
        }
        Some(st) => {
            let state = match (sf.clearing, sf.hot) {
                (true, Some(Hit::Clear)) => State::Pressed,
                (_, Some(Hit::Clear)) => State::Hot,
                _ => State::Normal,
            };
            st.draw(c, b.scaled(s), Part::SearchClear, state, s);
        }
        None => {}
    }
}

#[expect(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    sf: &SearchField,
    field: Rect,
    focused: bool,
    caret: Caret,
    px: u32,
) {
    let r = field.scaled(p.s);
    let inner = draw_edit_field(c, p, r, p.cs.window, focused, Interaction::Idle);
    c.fill_rect(inner, p.cs.window);
    let first = edit_first_visible(text, &sf.edit, inner.width, px, 0);
    draw_edit_text(c, text, &sf.edit, inner, px, p, caret, first);
}

pub fn access(
    sf: &SearchField,
    field: Rect,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
    key: impl std::hash::Hash,
) -> NodeId {
    let id = access_edit(&sf.edit, 0, false, field, true, text, px, tree, parent, key);
    if let Some(n) = tree.get_mut(id) {
        n.set_label(sf.cue_label());
    }
    id
}

/// [`access`] for the nav field, with its clear button while it holds text.
pub fn access_nav(
    sf: &SearchField,
    field: Rect,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let id = access(sf, field, text, px, tree, parent, "search");
    if !sf.text().is_empty() {
        let b = crate::access::button(clear_rect(field), &tr!("searchfield-clear"), true, false);
        tree.push(parent, "search clear", b);
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{Color, FontConfig, Theme};

    const BAND: Rect = Rect::new(0, 27, 792, 30);

    fn render(sf: &SearchField, focused: bool) -> (Canvas, Theme, Rect) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let field = layout(BAND, 300, &ControlMetrics::CLASSIC).expect("room");
        let mut c = Canvas::new(792, 80);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        let p = classic_paint(&theme.colors, 1);
        let caret = Caret::new(focused, true);
        draw(&mut c, &text, &p, sf, field, focused, caret, 11);
        (c, theme, field)
    }

    fn inks(c: &Canvas, r: Rect, bg: Color) -> std::collections::HashSet<u32> {
        (r.y..r.bottom())
            .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
            .map(|(x, y)| c.get(x, y))
            .filter(|&px| px != bg.argb())
            .collect()
    }

    #[test]
    fn the_field_sits_at_the_bands_right_end_and_gives_way_to_its_items() {
        let cm = ControlMetrics::CLASSIC;
        assert_eq!(
            layout(BAND, 300, &cm),
            Some(Rect::new(608, 31, 180, 22)),
            "180 x 22, 4 px in from the right, centred in the 30 px band"
        );
        assert_eq!(layout(BAND, 600, &cm), Some(Rect::new(608, 31, 180, 22)));
        assert_eq!(layout(BAND, 601, &cm), None, "8 px clear of the items");
        let big = ControlMetrics::CLASSIC.grown(crate::theme::TextUnits::new(22, 11));
        assert_eq!(
            layout(Rect::new(0, 47, 792, 43), 100, &big),
            Some(Rect::new(424, 47, 360, 43)),
            "twice the text, twice the field, never taller than the band"
        );
        assert!(hit(Rect::new(608, 31, 180, 22), 608, 31));
        assert!(!hit(Rect::new(608, 31, 180, 22), 607, 31));
    }

    #[test]
    fn draws_a_sunken_white_field_with_the_grey_cue_until_text_is_typed() {
        let (c, theme, f) = render(&SearchField::default(), false);
        let cs = &theme.colors;
        assert_eq!(c.get(f.x, f.y), cs.button_shadow.argb());
        assert_eq!(c.get(f.x + 1, f.y + 1), cs.button_dk_shadow.argb());
        assert_eq!(
            c.get(f.right() - 1, f.bottom() - 1),
            cs.button_hilight.argb()
        );
        assert_eq!(c.get(f.x + 2, f.y + 2), cs.window.argb());
        assert_eq!(
            c.get(f.x - 1, f.y),
            cs.button_face.argb(),
            "nothing left of it"
        );
        let body = f.inset(2);
        let cue = inks(&c, body, cs.window);
        assert!(!cue.is_empty(), "the cue is drawn");
        assert!(
            !cue.contains(&cs.button_text.argb()),
            "the cue is grey, never the text's ink"
        );
        crate::pins::fonts();
        assert!(cue.contains(&cs.gray_text.argb()));

        let mut typed = SearchField::default();
        typed.key(EditKey::Insert("notes".into()));
        let (c, ..) = render(&typed, false);
        assert!(
            inks(&c, body, cs.window).contains(&cs.button_text.argb()),
            "typed text takes the text's ink"
        );
    }

    #[test]
    fn a_focused_field_shows_its_caret_and_selects_all_on_focus() {
        let mut sf = SearchField::default();
        sf.key(EditKey::Insert("abc".into()));
        sf.focus();
        assert!(sf.edit.all_selected());
        sf.key(EditKey::Insert("x".into()));
        assert_eq!(sf.text(), "x", "typing replaces the selected query");
        let empty = SearchField::default();
        let (focused, theme, f) = render(&empty, true);
        let (idle, ..) = render(&empty, false);
        let ink = theme.colors.button_text.argb();
        let body = f.inset(2);
        assert!(
            inks(&focused, body, theme.colors.window).contains(&ink),
            "a focused field shows its caret"
        );
        assert!(!inks(&idle, body, theme.colors.window).contains(&ink));
        sf.clear();
        assert_eq!(sf.text(), "");
    }

    #[test]
    fn an_unfocused_field_hides_its_selection() {
        let mut sf = SearchField::default();
        sf.key(EditKey::Insert("report".into()));
        sf.focus();
        let lit = |focused: bool| {
            let (c, theme, f) = render(&sf, focused);
            inks(&c, f.inset(2), theme.colors.window).contains(&theme.colors.hilight.argb())
        };
        assert!(lit(true), "the focused field shows its selected query");
        assert!(!lit(false), "without the focus no highlight");
        assert!(sf.edit.all_selected(), "the selection itself is kept");
    }

    #[test]
    fn a_click_places_the_caret() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut sf = SearchField::default();
        sf.key(EditKey::Insert("report".into()));
        let f = Rect::new(608, 31, 180, 22);
        sf.click(f, f.x + 2, &text, 11, 1);
        assert_eq!(sf.edit.caret(), 0);
        sf.click(f, f.right() - 3, &text, 11, 1);
        assert_eq!(sf.edit.caret(), "report".len());
    }

    #[test]
    fn a_reader_hears_a_text_input_named_search() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut t = Tree::window("Explorer", (792, 80));
        let id = access(
            &SearchField::default(),
            Rect::new(608, 31, 180, 22),
            &text,
            11,
            &mut t,
            crate::access::ROOT,
            "search",
        );
        let n = t.get(id).expect("the node");
        assert_eq!(n.role(), accesskit::Role::TextInput);
        assert_eq!(n.label(), Some(cue().as_str()));
    }
}
