use super::*;

#[test]
fn restore_defaults_sits_under_the_pages_left_edge_on_the_button_row() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    s.tabs.open = 1;
    s.set_restore(1, true);
    let l = s.layout(&text, px);
    let at = Rect::new(l.page.x, l.ok.y, RESTORE_W, BUTTON_H);
    assert_eq!(at, Rect::new(152, 394, 96, 23));
    assert_eq!(s.pages[1].item(RESTORE).unwrap().rect, at);
    assert!(at.right() < l.ok.x, "clear of OK");
    assert!(s.pages[0].item(RESTORE).is_none(), "only the page given it");
    let order: Vec<&str> = s.pages[1].items.iter().map(|it| it.id).collect();
    assert_eq!(order, ["check", RESTORE, OK, CANCEL, APPLY], "before OK");

    s.pages[1].set_enabled(RESTORE, false);
    s.set_page(1, check_page("Effects"));
    let kept = s.pages[1].item(RESTORE).map(|it| it.control.clone());
    assert!(
        matches!(kept, Some(Control::Button { enabled: false, .. })),
        "a rebuilt page keeps it, greyed as it was"
    );
    s.resize((s.size.0 + 100, s.size.1 + 60));
    let r = s.pages[1].item(RESTORE).unwrap().rect;
    assert_eq!((r.x, r.y), (at.x, at.y + 60), "it rides the row down");

    s.open_mut().focus_id(OK);
    s.set_restore(1, false);
    assert!(s.pages[1].item(RESTORE).is_none());
    let focused = s.open().focus().map(|i| s.open().items[i].id);
    assert_eq!(focused, Some(OK), "the focus stays on OK");
}

#[test]
fn pressing_restore_defaults_is_the_pages_press() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    s.tabs.open = 2;
    s.set_restore(2, true);
    let r = s.pages[2].item(RESTORE).unwrap().rect;
    let hit = s.hit(r.x + 5, r.y + 5, &text, px);
    assert_eq!(s.click(hit), Event::Redraw, "the button goes down");
    assert_eq!(s.release(), Event::Page(2, form::Event::Pressed(RESTORE)));
    s.open_mut().focus_id(RESTORE);
    assert_eq!(
        s.key(form::Key::Enter, None),
        Event::Page(2, form::Event::Pressed(RESTORE)),
        "Enter on it"
    );
}

#[test]
fn restore_defaults_draws_in_every_look() {
    use crate::interface::Interface;
    let mut hashes = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let mut s = list_sheet();
        let plain = drawn(&s, &theme);
        s.set_restore(0, true);
        let c = drawn(&s, &theme);
        let r = s.pages[0].item(RESTORE).unwrap().rect;
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut changed = false;
        for y in 0..s.size.1 {
            for x in 0..s.size.0 {
                let inside = r.contains(x, y);
                if !inside {
                    assert_eq!(c.get(x, y), plain.get(x, y), "{i:?}: ({x}, {y})");
                    continue;
                }
                changed |= c.get(x, y) != plain.get(x, y);
                h = (h ^ u64::from(c.get(x, y))).wrapping_mul(0x100_0000_01b3);
            }
        }
        assert!(changed, "{i:?}: the button is drawn");
        hashes.push(h);
    }
    crate::pins::fonts();
    assert_eq!(
        hashes,
        [
            11_219_644_271_039_405_423,
            11_406_097_270_237_410_466,
            15_751_631_490_463_045_461
        ],
        "Classic, Blue, Glass"
    );
}
