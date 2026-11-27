use super::*;
use crate::controls::fingerprints::tests::looks;
use crate::controls::form::Control;
use crate::interfaces;
use crate::theme::Theme;

const A4: (i32, i32) = (21000, 29700);

/// A text page's ink: rows of black bars between 20 mm margins, at 300 dpi.
fn text_page() -> (Canvas, Rect) {
    let (w, h) = (pagesetup::px(17000, 300), pagesetup::px(25700, 300));
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), SHEET);
    for row in 0..40 {
        let len = w * (3 + row % 5) / 8;
        c.fill_rect(Rect::new(0, row * 60, len, 36), Color::rgb(0, 0, 0));
    }
    (c, Rect::new(2000, 2000, 17000, 25700))
}

fn shown(zoom: Zoom, theme: &Theme, s: i32) -> (Preview, Layout) {
    let mut pv = Preview::new(A4);
    pv.laid(3);
    pv.go(1);
    pv.zoom_to(zoom);
    let r = Rect::new(0, 0, CLIENT.0, CLIENT.1);
    let cm = theme.controls();
    let lay = layout(&pv, r, s, TextUnits::ONE, &cm);
    let (page, size) = pv.wanted(&lay).expect("the page is wanted");
    let (drawn, at) = text_page();
    pv.show(page, sheet(&drawn, at, A4, size));
    let lay = layout(&pv, r, s, TextUnits::ONE, &cm);
    (pv, lay)
}

fn hash(theme: &Theme, zoom: Zoom, s: i32) -> u64 {
    let mut theme = theme.clone();
    theme.look = theme.look.at_scale(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let (pv, lay) = shown(zoom, &theme, s);
    let mut c = Canvas::with_scale(CLIENT.0 as u32, CLIENT.1 as u32, s);
    c.fill_rect(c.bounds(), p.cs.button_face);
    draw(&mut c, &text, &p, &pv, &lay, theme.fonts.size_px);
    crate::pins::fnv(&c)
}

#[test]
fn the_preview_pins_its_pixels_whole_and_at_printed_size_in_every_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (state, zoom) in [("whole page", Zoom::Page), ("printed size", Zoom::Actual)] {
            for s in [1, 2] {
                got.push((format!("{look} {state} {s}"), hash(&base, zoom, s)));
            }
        }
    }
    crate::pins::check("controls/printpreview", &got);
}

#[test]
fn the_buttons_fit_their_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| vec![("Preview".into(), toolbar())]);
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
fn the_whole_page_fits_the_box_and_printed_size_is_96_dpi_a_logical_px() {
    let room = Rect::new(0, 35, 620, 465);
    let (w, h) = sheet_size(A4, Zoom::Page, room, 1);
    assert_eq!(
        h as i32,
        465 - 2 * INSET - SHADOW,
        "an upright page fills the height"
    );
    assert_eq!(w, h * 210 / 297, "with A4's shape");
    let (w2, h2) = sheet_size(A4, Zoom::Page, room, 2);
    assert_eq!(h2, h * 2, "scale 2 draws it afresh at twice the px");
    assert!(w2.abs_diff(w * 2) <= 1);
    assert_eq!(sheet_size(A4, Zoom::Actual, room, 1), (794, 1123));
    assert_eq!(sheet_size(A4, Zoom::Actual, room, 2), (1587, 2245));
}

#[test]
fn a_page_is_asked_for_once_per_page_and_size() {
    let theme = Theme::default();
    let cm = theme.controls();
    let r = Rect::new(0, 0, CLIENT.0, CLIENT.1);
    let mut pv = Preview::new(A4);
    let lay = layout(&pv, r, 1, TextUnits::ONE, &cm);
    assert_eq!(
        pv.wanted(&lay),
        None,
        "nothing before the pages are laid out"
    );
    pv.laid(2);
    let first = pv.wanted(&lay).expect("page 1");
    assert_eq!(first.0, 0);
    assert_eq!(pv.wanted(&lay), None, "asked once");
    pv.show(0, Canvas::new(first.1.0, first.1.1));
    assert!(pv.go(1));
    assert_eq!(pv.wanted(&lay).map(|w| w.0), Some(1));
    assert!(pv.go(-1));
    pv.show(1, Canvas::new(first.1.0, first.1.1));
    assert_eq!(
        pv.sheet.as_ref().map(|s| s.0),
        Some(0),
        "a late page 2 is dropped while page 1 shows"
    );
    assert!(pv.go(1));
    pv.zoom_to(Zoom::Actual);
    let lay = layout(&pv, r, 1, TextUnits::ONE, &cm);
    assert_eq!(
        pv.wanted(&lay),
        Some((1, (794, 1123))),
        "printed size asks again"
    );
}

#[test]
fn page_keys_turn_within_the_document_and_grey_the_buttons_at_its_ends() {
    let theme = Theme::default();
    let cm = theme.controls();
    let r = Rect::new(0, 0, CLIENT.0, CLIENT.1);
    let mut pv = Preview::new(A4);
    pv.laid(3);
    let lay = layout(&pv, r, 1, TextUnits::ONE, &cm);
    let enabled = |pv: &Preview, id| {
        matches!(
            pv.form.item(id).map(|i| &i.control),
            Some(Control::Button { enabled: true, .. })
        )
    };
    assert!(!enabled(&pv, PREVIOUS) && enabled(&pv, NEXT));
    assert_eq!(
        pv.key(keysym::PAGE_UP, &lay),
        Some(Action::None),
        "no page before the first"
    );
    pv.form.focus_id(NEXT);
    for _ in 0..3 {
        pv.key(keysym::PAGE_DOWN, &lay);
    }
    assert_eq!(pv.page, 2, "no page after the last");
    assert_eq!(pv.label(), "Page 3 of 3");
    assert!(!enabled(&pv, NEXT) && enabled(&pv, PREVIOUS));
    let focused = pv.form.focus().map(|i| pv.form.items[i].id);
    assert_eq!(
        focused,
        Some(PREVIOUS),
        "the focus leaves the greyed Next Page"
    );
    assert_eq!(pv.key(keysym::ESCAPE, &lay), Some(Action::Close));
    assert_eq!(pv.pressed(ZOOM_IN), Action::Redraw);
    assert!(!enabled(&pv, ZOOM_IN) && enabled(&pv, ZOOM_OUT));
    assert_eq!(pv.pressed(PRINT), Action::Print);
}

#[test]
fn a_sheet_draws_the_page_where_it_goes_on_white_paper() {
    let mut drawn = Canvas::new(10, 10);
    drawn.fill_rect(drawn.bounds(), Color::rgb(0, 0, 0));
    let c = sheet(
        &drawn,
        Rect::new(5000, 10000, 10000, 10000),
        (20000, 40000),
        (40, 80),
    );
    let black = Color::rgb(0, 0, 0).argb();
    assert_eq!(c.get(10, 20), black, "the page's ink from 5 mm, 10 mm in");
    assert_eq!(c.get(29, 39), black);
    assert_eq!(c.get(9, 20), SHEET.argb(), "white paper around it");
    assert_eq!(c.get(30, 40), SHEET.argb());
}
