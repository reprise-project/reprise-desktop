//! The address band: the "Address" label, the editable place combo (icon, path [`TextEdit`], drop button) and Go,
//! in the rebar's third band ([`rebar`](crate::controls::rebar)).

use crate::access::{self, Tree, accesskit};
use crate::controls::combo::{self, Combo, ListItem};
use crate::controls::metrics::ControlMetrics;
use crate::look::Paint;
use crate::look::widgets::toolbar;
use crate::raster::{Canvas, Rect};
use crate::style::{Part, State};
use crate::text::{Align, TextRenderer};
use crate::textedit::{Caret, EditKey, TextEdit};
use accesskit::{NodeId, Role};

pub const LABEL_X: i32 = 6;

/// Where the field's sunken frame starts, from the band's left edge. The
/// label gets the room before it whatever its width, so the field's x is
/// pinned.
pub const FIELD_X: i32 = 49;

/// Classic's Go button width: the gap between the field's outer frame
/// and the rebar's right stripe. A look sets its own in
/// [`AddressMetrics::go_w`](crate::metrics::AddressMetrics::go_w).
pub const GO_W: i32 = crate::metrics::AddressMetrics::CLASSIC.go_w;

pub const RIGHT_PAD: i32 = 2;

const GO_ICON: i32 = 20;
const GO_ICON_X: i32 = 5;

const GO_TEXT_X: i32 = GO_ICON_X + GO_ICON + 4;

/// The band: the places combo (always editable, always with an icon)
/// and the Go button, hot while the pointer is on it.
#[derive(Clone, Debug)]
pub struct AddressBar {
    pub combo: Combo,
    pub go_icon: Option<Canvas>,
    pub go_hot: bool,
}

impl AddressBar {
    /// The places the drop list shows, `selected` the one the field
    /// opens on and takes its text and icon from. The list shows every
    /// place, with no scrollbar.
    pub fn new(items: Vec<ListItem>, selected: usize, go_icon: Option<Canvas>) -> AddressBar {
        let rows = items.len().max(1);
        let mut combo = Combo::with_items(items, selected, rows);
        combo.edit = Some(TextEdit::new(combo.value(), false));
        combo.icon = combo.items.get(selected).and_then(|i| i.icon.clone());
        AddressBar {
            combo,
            go_icon,
            go_hot: false,
        }
    }

    pub fn text(&self) -> &str {
        self.combo.text()
    }

    pub fn set_place(&mut self, text: &str, icon: Option<Canvas>) {
        self.combo.set_text(text);
        self.combo.icon = icon;
    }

    /// An editing key into the field. The open list's keys are [`Combo::step`]
    /// and [`Combo::toggle`]; Enter is [`AddressBar::enter`].
    pub fn key(&mut self, k: EditKey) -> Event {
        match self.combo.edit.as_mut() {
            Some(edit) => {
                edit.key(k);
                Event::Redraw
            }
            None => Event::None,
        }
    }

    /// Enter: takes the open list's highlighted row, else navigates to
    /// whatever the field holds.
    pub fn enter(&mut self) -> Event {
        match self.combo.open {
            Some(i) => self.take(i),
            None => Event::Navigate(self.text().to_string()),
        }
    }

    /// A click on what [`hit`] returned. `x` is the pointer's, for the
    /// caret; `px` and `s` are the ones the band was drawn with.
    pub fn click(
        &mut self,
        hit: Hit,
        x: i32,
        lay: &Layout,
        text: &TextRenderer,
        px: u32,
        s: i32,
    ) -> Event {
        match hit {
            Hit::Combo(combo::Hit::Field) => {
                let i = combo::index_at(&self.combo, &lay.combo, text, px, s, x);
                if let Some(edit) = self.combo.edit.as_mut() {
                    edit.place(i, false);
                }
                Event::Redraw
            }
            Hit::Combo(combo::Hit::Drop) => {
                self.combo.toggle();
                Event::Redraw
            }
            Hit::Combo(combo::Hit::Row(i)) => self.take(i),
            Hit::Combo(combo::Hit::ScrollUp) | Hit::Combo(combo::Hit::PageUp) => {
                self.combo.scroll(-1);
                Event::Redraw
            }
            Hit::Combo(combo::Hit::ScrollDown) | Hit::Combo(combo::Hit::PageDown) => {
                self.combo.scroll(1);
                Event::Redraw
            }
            Hit::Combo(combo::Hit::Thumb(_)) => Event::Redraw,
            Hit::Go => Event::Navigate(self.text().to_string()),
            Hit::Nothing => Event::None,
        }
    }

    /// Pick row `i`: the list closes and the field takes the place's
    /// text and icon.
    fn take(&mut self, i: usize) -> Event {
        self.combo.pick(i);
        let Some(item) = self.combo.items.get(i) else {
            return Event::Redraw;
        };
        let (text, icon) = (item.text.clone(), item.icon.clone());
        self.set_place(&text, icon);
        Event::Pick(i)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub label: Rect,
    pub field: Rect,
    pub combo: combo::Layout,
    pub go: Rect,
}

pub fn layout(
    ab: &AddressBar,
    text: &TextRenderer,
    px: u32,
    r: Rect,
    cm: &ControlMetrics,
) -> Layout {
    let label = Rect::new(
        r.x + LABEL_X,
        r.y,
        text.measure("Address", false, px),
        r.height,
    );
    // The label's and Go's room grows with the text; the band's height is the caller's.
    let go_w = cm.text.len(cm.address.go_w);
    let go = Rect::new(r.right() - RIGHT_PAD - go_w, r.y, go_w, r.height);
    let field_x = r.x + cm.text.len(FIELD_X);
    let field = Rect::new(field_x, r.y, (go.x - field_x).max(0), r.height);
    Layout {
        label,
        field,
        combo: combo::layout(&ab.combo, field, cm),
        go,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Combo(combo::Hit),
    Go,
    Nothing,
}

pub fn hit(ab: &AddressBar, lay: &Layout, x: i32, y: i32) -> Hit {
    if let Some(h) = combo::hit(&ab.combo, &lay.combo, x, y) {
        return Hit::Combo(h);
    }
    if lay.go.contains(x, y) {
        return Hit::Go;
    }
    Hit::Nothing
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Navigate(String),
    Pick(usize),
}

/// The band's closed part. The open list is [`draw_list`], drawn last by
/// the caller so it covers the view under the band.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    ab: &AddressBar,
    lay: &Layout,
    focus: bool,
    caret: Caret,
    px: u32,
) {
    let Paint { cs, s, .. } = *p;
    let label_ink = p
        .look
        .style()
        .and_then(|st| st.text_color(Part::RebarBand, State::Normal))
        .unwrap_or(cs.button_text);
    text.draw(
        c,
        lay.label.scaled(s),
        "Address",
        false,
        label_ink,
        Align::Left,
        px,
    );
    combo::draw(
        c,
        text,
        p,
        &ab.combo,
        &lay.combo,
        focus,
        false,
        caret,
        combo::ComboLit::IDLE,
        px,
    );
    draw_go(c, text, p, ab, lay.go.scaled(s), px);
}

/// Go: a flat toolbar button, its icon and label on the band's face,
/// lit as a toolbar button is only while the pointer is on it, the frame
/// the look's [`AddressMetrics::go_frame_x`] in from each side.
///
/// [`AddressMetrics::go_frame_x`]: crate::metrics::AddressMetrics::go_frame_x
fn draw_go(c: &mut Canvas, text: &TextRenderer, p: &Paint, ab: &AddressBar, go: Rect, px: u32) {
    let Paint { cs, s, .. } = *p;
    if ab.go_hot {
        let dx = p.look.controls().address.go_frame_x * s;
        let frame = Rect::new(go.x + dx, go.y, go.width - 2 * dx, go.height);
        toolbar::draw_button(c, p, frame, None, State::Hot);
    }
    if let Some(icon) = &ab.go_icon {
        c.blit_over(
            go.x + GO_ICON_X * s,
            go.y + (go.height - GO_ICON * s) / 2,
            icon,
        );
    }
    let tx = go.x + GO_TEXT_X * s;
    text.draw(
        c,
        Rect::new(tx, go.y, (go.right() - tx).max(0), go.height),
        "Go",
        false,
        cs.button_text,
        Align::Left,
        px,
    );
}

pub fn draw_list(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    ab: &AddressBar,
    lay: &Layout,
    px: u32,
) {
    if ab.combo.open.is_none() {
        return;
    }
    combo::draw_list(
        c,
        text,
        p,
        &ab.combo.items,
        ab.combo.open,
        ab.combo.top,
        ab.combo.rows,
        lay.combo.list,
        lay.combo.scrollbar,
        None,
        px,
    );
}

/// The address bar's `access` part: the "Address" label, the editable
/// combo it labels and the Go button, under a toolbar. Returns the node
/// that has the focus when the bar does, as [`combo::access`] does.
pub fn access(
    ab: &AddressBar,
    lay: &Layout,
    text: &TextRenderer,
    px: u32,
    tree: &mut Tree,
    parent: NodeId,
) -> NodeId {
    let all = lay.label.union(&lay.field).union(&lay.go);
    let bar = tree.push(parent, "address", access::node(Role::Toolbar, all));
    let label = tree.push(bar, "label", access::label(lay.label, "Address", true));
    let focus = combo::access(&ab.combo, &lay.combo, text, px, tree, bar, "combo");
    if let Some(n) = tree.get_mut(Tree::id_of(bar, "combo")) {
        n.push_labelled_by(label);
    }
    tree.push(bar, "go", access::button(lay.go, "Go", true, false));
    focus
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{Color, FontConfig, Theme};

    const BAND: Rect = Rect::new(0, 59, 792, 22);
    const EXPECTED_LABEL_INK: (i32, i32, i32) = (6, 65, 73);
    const EXPECTED_FIELD: Rect = Rect::new(49, 59, 684, 22);
    const EXPECTED_FIELD_ICON: Rect = Rect::new(52, 62, 16, 16);
    const EXPECTED_FIELD_INK: (i32, i32, i32) = (72, 67, 76);
    const EXPECTED_DROP: Rect = Rect::new(715, 61, 16, 18);
    const EXPECTED_GO: Rect = Rect::new(733, 59, 57, 22);
    const EXPECTED_GO_ICON: Rect = Rect::new(738, 60, 20, 20);
    const EXPECTED_GO_INK: (i32, i32, i32) = (762, 66, 73);
    const EXPECTED_LIST: Rect = Rect::new(49, 81, 684, 178);
    const EXPECTED_ROW0_ICON_X: i32 = 53;
    const EXPECTED_ROW0_INK: (i32, i32, i32) = (73, 86, 96);
    const EXPECTED_ROW1_ICON_X: i32 = 63;
    const EXPECTED_ROW1_INK_X: i32 = 83;
    const EXPECTED_ROW1_HL: Rect = Rect::new(82, 99, 72, 15);

    fn icon(w: u32, h: u32, colour: Color) -> Canvas {
        let mut c = Canvas::new(w, h);
        c.fill_rect(c.bounds(), colour);
        c
    }

    fn places() -> Vec<ListItem> {
        let yellow = Color::rgb(255, 255, 153);
        [
            ("Desktop", 0),
            ("My Documents", 1),
            ("My Computer", 1),
            ("Local Disk (/)", 2),
            ("CD Drive (/media)", 2),
            ("Control Panel", 2),
            ("Public", 2),
            ("user's Documents", 2),
            ("My Network Places", 1),
            ("Recycle Bin", 1),
            ("New Folder", 1),
        ]
        .into_iter()
        .map(|(t, d)| ListItem::shell(t, Some(icon(16, 16, yellow)), d))
        .collect()
    }

    fn bar() -> AddressBar {
        let mut ab = AddressBar::new(places(), 1, Some(icon(20, 20, Color::rgb(0, 128, 0))));
        ab.combo.icon = Some(icon(16, 16, Color::rgb(255, 255, 153)));
        ab
    }

    fn render(ab: &AddressBar, open: bool, caret: bool) -> (Canvas, Theme, Layout) {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let px = theme.fonts.size_px;
        let lay = layout(ab, &text, px, BAND, &theme.look.controls());
        let mut c = Canvas::new(792, 300);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            ab,
            &lay,
            false,
            Caret::new(caret, true),
            px,
        );
        if open {
            draw_list(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                ab,
                &lay,
                px,
            );
        }
        (c, theme, lay)
    }

    fn ink(c: &Canvas, r: Rect, bg: Color) -> (i32, i32, i32) {
        let (mut x0, mut y0, mut y1) = (i32::MAX, i32::MAX, i32::MIN);
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                if c.get(x, y) != bg.argb() {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    y1 = y1.max(y);
                }
            }
        }
        (x0, y0, y1)
    }

    #[test]
    fn blues_go_is_narrower_and_lights_inside_its_rect() {
        const MARK: Color = Color::rgb(1, 2, 3);
        let mut ab = bar();
        let text = TextRenderer::load(&FontConfig::default());
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let band = Rect::new(0, 64, 792, 22);
        let lay = layout(&ab, &text, 11, band, &theme.look.controls());
        assert_eq!(lay.go, Rect::new(735, 64, 55, 22));
        assert_eq!(lay.field, Rect::new(49, 64, 686, 22));
        ab.go_hot = true;
        let mut c = Canvas::new(792, 90);
        c.fill_rect(c.bounds(), MARK);
        draw_go(&mut c, &text, &theme.paint(), &ab, lay.go, 11);
        for (x, lit) in [(736, false), (737, true), (787, true), (788, false)] {
            assert_eq!(c.get(x, 72) != MARK.argb(), lit, "x {x}");
        }
    }

    #[test]
    fn blues_address_label_inks_the_rebar_label_grey() {
        if !crate::text::test_tahoma() {
            return;
        }
        let ab = bar();
        let text = TextRenderer::load(&FontConfig::default());
        use crate::blue::Variant;
        for v in [Variant::Blue, Variant::Olive, Variant::Silver] {
            let theme = crate::interfaces::blue_theme(v);
            let p = theme.paint();
            let lay = layout(&ab, &text, 11, BAND, &theme.look.controls());
            let mut c = Canvas::new(792, 90);
            c.fill_rect(c.bounds(), p.cs.button_face);
            draw(&mut c, &text, &p, &ab, &lay, false, Caret::Unfocused, 11);
            let label = Rect::new(0, 59, 48, 22);
            let inks: std::collections::HashSet<u32> = (label.y..label.bottom())
                .flat_map(|y| (label.x..label.right()).map(move |x| (x, y)))
                .map(|(x, y)| c.get(x, y))
                .filter(|&px| px != p.cs.button_face.argb())
                .collect();
            assert_eq!(
                inks,
                [v.scheme().inks.rebar_label.argb()].into(),
                "{v:?}: the label's only ink"
            );
            assert_eq!(
                ink(&c, label, p.cs.button_face),
                EXPECTED_LABEL_INK,
                "{v:?}: the label keeps its place"
            );
        }
    }

    #[test]
    fn layout_puts_the_label_field_and_go_in_place() {
        let ab = bar();
        let text = TextRenderer::load(&FontConfig::default());
        let lay = layout(&ab, &text, 11, BAND, &ControlMetrics::CLASSIC);
        assert_eq!(lay.label.x, EXPECTED_LABEL_INK.0);
        assert_eq!(lay.field, EXPECTED_FIELD);
        assert_eq!(lay.go, EXPECTED_GO);
        assert_eq!(lay.combo.drop, EXPECTED_DROP);
        assert_eq!(lay.combo.icon, Some(EXPECTED_FIELD_ICON));
        assert_eq!(lay.combo.text.x, EXPECTED_FIELD_INK.0);
        assert_eq!(lay.combo.list, EXPECTED_LIST);
        assert!(lay.combo.scrollbar.is_none(), "11 places, 11 rows");
        assert_eq!(hit(&ab, &lay, 722, 70), Hit::Combo(combo::Hit::Drop));
        assert_eq!(hit(&ab, &lay, 200, 70), Hit::Combo(combo::Hit::Field));
        assert_eq!(hit(&ab, &lay, 745, 70), Hit::Go);
        assert_eq!(hit(&ab, &lay, 20, 70), Hit::Nothing);
        assert_eq!(hit(&ab, &lay, 200, 107), Hit::Nothing);
        let mut open = bar();
        open.combo.toggle();
        assert_eq!(hit(&open, &lay, 200, 107), Hit::Combo(combo::Hit::Row(1)));
        assert_eq!(hit(&open, &lay, 722, 90), Hit::Combo(combo::Hit::Row(0)));
    }

    #[test]
    fn draws_the_band_pixels() {
        if !crate::text::test_tahoma() {
            return;
        }
        let ab = bar();
        let (c, theme, _) = render(&ab, false, false);
        let cs = &theme.colors;
        assert_eq!(
            ink(&c, Rect::new(0, 59, 48, 22), cs.button_face),
            EXPECTED_LABEL_INK,
            "the Address label"
        );
        assert_eq!(c.get(49, 59), cs.button_shadow.argb());
        assert_eq!(c.get(50, 60), cs.button_dk_shadow.argb());
        assert_eq!(c.get(51, 61), cs.window.argb());
        assert_eq!(c.get(731, 78), cs.button_face.argb());
        assert_eq!(c.get(732, 80), cs.button_hilight.argb());
        assert_eq!(c.get(52, 62), Color::rgb(255, 255, 153).argb());
        assert_eq!(c.get(67, 77), Color::rgb(255, 255, 153).argb());
        assert_eq!(c.get(68, 70), cs.window.argb(), "a gap after the icon");
        assert_eq!(
            ink(&c, Rect::new(68, 61, 100, 18), cs.window),
            EXPECTED_FIELD_INK,
            "the path in the field"
        );
        assert_eq!(c.get(715, 61), cs.button_face.argb());
        assert_eq!(c.get(722, 61), cs.button_face.argb());
        assert_eq!(c.get(716, 62), cs.button_hilight.argb());
        assert_eq!(c.get(730, 61), cs.button_dk_shadow.argb());
        assert_eq!(c.get(729, 70), cs.button_shadow.argb());
        assert_eq!(c.get(730, 70), cs.button_dk_shadow.argb());
        assert_eq!(c.get(715, 78), cs.button_dk_shadow.argb());
        assert_eq!(c.get(730, 78), cs.button_dk_shadow.argb());
        assert_eq!(c.get(722, 70), cs.button_text.argb(), "arrow row");
        assert_eq!(c.get(733, 59), cs.button_face.argb(), "no frame");
        assert_eq!(c.get(789, 80), cs.button_face.argb());
        assert_eq!(
            c.get(EXPECTED_GO_ICON.x, EXPECTED_GO_ICON.y),
            Color::rgb(0, 128, 0).argb()
        );
        assert_eq!(
            c.get(EXPECTED_GO_ICON.right() - 1, EXPECTED_GO_ICON.bottom() - 1),
            Color::rgb(0, 128, 0).argb()
        );
        assert_eq!(
            ink(&c, Rect::new(758, 59, 32, 22), cs.button_face),
            EXPECTED_GO_INK,
            "the Go label"
        );
        let mut hot = bar();
        hot.go_hot = true;
        let (c, _, _) = render(&hot, false, false);
        assert_eq!(
            c.get(EXPECTED_GO.x, EXPECTED_GO.y),
            cs.button_hilight.argb()
        );
        assert_eq!(
            c.get(EXPECTED_GO.right() - 1, EXPECTED_GO.bottom() - 1),
            cs.button_shadow.argb()
        );
    }

    #[test]
    fn draws_the_open_list() {
        if !crate::text::test_tahoma() {
            return;
        }
        let mut ab = bar();
        ab.combo.toggle();
        assert_eq!(ab.combo.open, Some(1), "opens on the picked place");
        let (c, theme, _) = render(&ab, true, false);
        let cs = &theme.colors;
        for (x, y) in [
            (EXPECTED_LIST.x, EXPECTED_LIST.y),
            (EXPECTED_LIST.right() - 1, EXPECTED_LIST.y),
            (EXPECTED_LIST.x, EXPECTED_LIST.bottom() - 1),
        ] {
            assert_eq!(c.get(x, y), cs.button_text.argb(), "frame at {x},{y}");
        }
        let yellow = Color::rgb(255, 255, 153).argb();
        assert_eq!(c.get(EXPECTED_ROW0_ICON_X, 82), yellow);
        assert_eq!(c.get(EXPECTED_ROW0_ICON_X - 1, 82), cs.window.argb());
        assert_eq!(c.get(EXPECTED_ROW1_ICON_X, 98), yellow);
        assert_eq!(
            EXPECTED_ROW1_ICON_X - EXPECTED_ROW0_ICON_X,
            combo::INDENT_STEP
        );
        assert_eq!(
            ink(&c, Rect::new(69, 82, 120, 16), cs.window),
            EXPECTED_ROW0_INK,
            "Desktop's label"
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.x + 4, EXPECTED_ROW1_HL.y + 4),
            cs.hilight.argb()
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.x - 1, EXPECTED_ROW1_HL.y + 4),
            cs.window.argb(),
            "white left of the highlight"
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.right(), EXPECTED_ROW1_HL.y + 4),
            cs.window.argb(),
            "white right of it"
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.x, EXPECTED_ROW1_HL.y + 8),
            Color::rgb(245, 219, 149).argb(),
            "the focus rectangle's inverted cell"
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.x, EXPECTED_ROW1_HL.y + 7),
            cs.hilight.argb(),
            "and the cell between two of them"
        );
        assert_eq!(
            c.get(EXPECTED_ROW1_HL.x + 1, EXPECTED_ROW1_HL.y),
            cs.hilight.argb(),
            "no ink on the focus rectangle's top edge"
        );
        assert_eq!(
            ink(&c, Rect::new(79, 99, 4, 14), cs.window).0,
            EXPECTED_ROW1_HL.x,
            "the highlight starts one px before the label"
        );
        assert_eq!(EXPECTED_ROW1_INK_X - EXPECTED_ROW1_HL.x, 1);
        assert_eq!(c.get(400, EXPECTED_LIST.bottom()), cs.button_face.argb());
    }

    #[test]
    fn enter_navigates_what_was_typed_and_a_row_picks_a_place() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut ab = bar();
        assert_eq!(ab.text(), "My Documents");
        ab.key(EditKey::SelectAll);
        ab.key(EditKey::Insert("/tmp".into()));
        assert_eq!(ab.enter(), Event::Navigate("/tmp".into()));
        let lay = layout(&ab, &text, 11, BAND, &ControlMetrics::CLASSIC);
        assert_eq!(
            ab.click(Hit::Go, 745, &lay, &text, 11, 1),
            Event::Navigate("/tmp".into())
        );
        ab.combo.toggle();
        assert_eq!(
            ab.click(Hit::Combo(combo::Hit::Row(4)), 200, &lay, &text, 11, 1),
            Event::Pick(4)
        );
        assert_eq!(ab.text(), "CD Drive (/media)");
        assert_eq!(ab.combo.open, None);
        assert!(ab.combo.icon.is_some(), "the place's icon follows it");
        ab.combo.toggle();
        ab.combo.step(true);
        assert_eq!(ab.enter(), Event::Pick(5));
        assert_eq!(ab.text(), "Control Panel");
        ab.set_place("My Computer", None);
        assert_eq!(ab.text(), "My Computer");
    }

    #[test]
    fn a_click_in_the_field_places_the_caret_after_the_icon() {
        let text = TextRenderer::load(&FontConfig::default());
        let mut ab = bar();
        let lay = layout(&ab, &text, 11, BAND, &ControlMetrics::CLASSIC);
        ab.click(Hit::Combo(combo::Hit::Field), 72, &lay, &text, 11, 1);
        assert_eq!(ab.combo.edit.as_ref().unwrap().caret(), 0);
        let w = text.measure("My Documents", false, 11);
        ab.click(Hit::Combo(combo::Hit::Field), 72 + w, &lay, &text, 11, 1);
        assert_eq!(
            ab.combo.edit.as_ref().unwrap().caret(),
            "My Documents".len()
        );
    }

    #[test]
    #[ignore]
    fn render_addressbar() {
        let theme = Theme::default();
        let text = TextRenderer::load(&FontConfig::default());
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let mut c = Canvas::new(792, 380);
        c.fill_rect(c.bounds(), cs.button_face);
        let ab = bar();
        for (i, caret) in [false, true].into_iter().enumerate() {
            let band = Rect::new(0, 10 + i as i32 * 32, 792, 22);
            let lay = layout(&ab, &text, px, band, &theme.look.controls());
            c.etched_hline_t(0, 792, band.y - 2, cs, 1);
            draw(
                &mut c,
                &text,
                &classic_paint(cs, 1),
                &ab,
                &lay,
                false,
                Caret::new(caret, true),
                px,
            );
        }

        let mut open = bar();
        open.combo.toggle();
        let band = Rect::new(0, 79, 792, 22);
        let lay = layout(&open, &text, px, band, &theme.look.controls());
        c.etched_hline_t(0, 792, band.y - 2, cs, 1);
        draw(
            &mut c,
            &text,
            &classic_paint(cs, 1),
            &open,
            &lay,
            false,
            Caret::Unfocused,
            px,
        );
        draw_list(&mut c, &text, &classic_paint(cs, 1), &open, &lay, px);
        crate::raster::write_preview(&c, "addressbar-preview.png");
    }
}
