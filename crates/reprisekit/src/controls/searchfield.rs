//! A search box: a single-line edit with a grey "Search" cue, placed at the right end of a
//! toolbar band.

use crate::access::{Tree, accesskit};
use crate::classic::widgets::access_edit;
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::{draw_edit_field, draw_edit_text};
use crate::pointer::Interaction;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::textedit::{Caret, EditKey, TextEdit, edit_first_visible, index_at};
use accesskit::NodeId;

pub const CUE: &str = "Search";
/// The field's size at 100% text; both grow with the text.
pub const WIDTH: i32 = 180;
pub const HEIGHT: i32 = 22;
/// The gap from the field's right edge to the band's.
pub const RIGHT_PAD: i32 = 4;
/// The least room kept between the band's other items and the field.
pub const GAP: i32 = 8;

#[derive(Clone, Debug)]
pub struct SearchField {
    pub edit: TextEdit,
}

impl Default for SearchField {
    fn default() -> SearchField {
        let mut edit = TextEdit::new("", false);
        edit.set_cue(CUE);
        SearchField { edit }
    }
}

impl SearchField {
    pub fn text(&self) -> &str {
        &self.edit.text
    }

    /// An editing key; true when the field changed.
    pub fn key(&mut self, k: EditKey) -> bool {
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
        let inner = inner(field.scaled(s), s);
        let first = edit_first_visible(text, &self.edit, inner.width, px, 0);
        let i = first + index_at(text, &self.edit.text[first..], x * s - inner.x, px);
        self.edit.place(i, false);
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
        n.set_label(CUE);
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
        if crate::text::test_tahoma() {
            assert!(cue.contains(&cs.gray_text.argb()));
        }

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
        assert_eq!(n.label(), Some(CUE));
    }
}
