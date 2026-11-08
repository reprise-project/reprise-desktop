use super::*;

/// The open tab is the selected one and names the page's panel, which
/// holds the page's controls; switching tabs replaces the panel.
#[test]
fn the_open_tab_names_the_panel_it_holds() {
    use crate::access::{ROOT, Tree, accesskit::Role};
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = sheet(&["General", "Advanced"], (398, 424));
    let access = |s: &PropSheet| {
        let mut tree = Tree::window("T", s.size);
        let (panel, focus) = s.access(&mut tree, ROOT, &text, 11);
        (tree, panel, focus)
    };
    let (tree, panel, focus) = access(&s);
    let tab = tree.find(Role::Tab, "General").unwrap();
    assert_eq!(tree.get(tab).unwrap().is_selected(), Some(true));
    assert_eq!(tree.get(panel).unwrap().labelled_by(), &[tab]);
    let check = tree.find(Role::CheckBox, "General").unwrap();
    assert!(tree.get(panel).unwrap().children().contains(&check));
    assert_eq!(focus, Some(check));
    s.tabs.open = 1;
    let (_, other, _) = access(&s);
    assert_ne!(panel, other);
}

/// Under Blue a closed tab lights under the pointer and draws its Hot
/// frame; the open tab never lights, and a leave clears the lit tab.
#[test]
fn a_blue_sheet_lights_the_closed_tab_under_the_pointer() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
    let mut s = sheet(&["General", "Advanced"], (398, 424));
    drawn(&s, &theme);
    let t = s.layout(&text, px).tabs[1];
    let (x, y) = (t.x + 5, t.y + 5);
    assert_eq!(s.motion(x, y, s.hit(x, y, &text, px)), Event::Redraw);
    assert_eq!(s.tabs.hot, Some(1));
    let c = drawn(&s, &theme);
    let hot = &crate::blue::Variant::Blue.scheme().tab_item.hot;
    assert_eq!(
        c.get(t.x + 20, t.y),
        hot.mark.unwrap().argb(),
        "the hot tab's lit outline"
    );
    assert_eq!(
        c.get(t.x + 20, t.y + 1),
        hot.down[0].1.argb(),
        "the hot tab's light band"
    );
    let open = s.layout(&text, px).tabs[0];
    let (ox, oy) = (open.x + 5, open.y + 5);
    assert_eq!(s.motion(ox, oy, s.hit(ox, oy, &text, px)), Event::Redraw);
    assert_eq!(s.tabs.hot, None, "the open tab does not light");
    s.motion(x, y, s.hit(x, y, &text, px));
    assert_eq!(s.leave(), Event::Redraw);
    assert_eq!(s.tabs.hot, None);
    assert_eq!(s.leave(), Event::None);
}

/// A motion over a page's control lights it, a leave clears it, and a
/// switch to another tab leaves the old page and the tab unlit.
#[test]
fn a_blue_sheet_lights_its_page_and_a_tab_switch_leaves_it() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
    let mut s = sheet(&["General", "Advanced"], (398, 424));
    drawn(&s, &theme);
    let lit = |s: &PropSheet| s.pages[0].lit(0, form::Light::Body);
    assert_eq!(s.motion(25, 55, s.hit(25, 55, &text, px)), Event::Redraw);
    assert_eq!(lit(&s), crate::pointer::Interaction::Hot);
    assert_eq!(s.leave(), Event::Redraw);
    assert_eq!(lit(&s), crate::pointer::Interaction::Idle);
    s.motion(25, 55, s.hit(25, 55, &text, px));
    let t = s.layout(&text, px).tabs[1];
    let hit = s.hit(t.x + 5, t.y + 5, &text, px);
    assert_eq!(s.click(hit), Event::Redraw);
    assert_eq!(s.tabs.open, 1);
    assert_eq!(s.tabs.hot, None, "the tab just opened is not lit");
    assert_eq!(
        lit(&s),
        crate::pointer::Interaction::Idle,
        "the page left behind"
    );
    // Ctrl+Tab with a button held lets the page left behind go of it.
    let ok = s.pages[1].index_of(OK).unwrap();
    let r = s.pages[1].items[ok].rect;
    s.click(s.hit(r.x + 5, r.y + 5, &text, px));
    assert!(s.pages[1].holding());
    s.key(form::Key::Tab { back: false }, Some(false));
    assert!(!s.pages[1].holding(), "the button is let go");
}

/// Classic has no hot tab: the strip tracks the pointer but asks for no
/// redraw.
#[test]
fn a_classic_sheet_asks_no_redraw_for_a_tab_under_the_pointer() {
    let theme = Theme::default();
    let (text, px) = (TextRenderer::load(&theme.fonts), theme.fonts.size_px);
    let mut s = sheet(&["General", "Advanced"], (398, 424));
    let rest = drawn(&s, &theme);
    let t = s.layout(&text, px).tabs[1];
    let (x, y) = (t.x + 5, t.y + 5);
    assert_eq!(s.motion(x, y, s.hit(x, y, &text, px)), Event::None);
    assert!(drawn(&s, &theme) == rest, "a Classic tab draws the same");
}
