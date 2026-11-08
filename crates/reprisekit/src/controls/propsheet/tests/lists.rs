use super::*;

/// A list sheet of 30 pages under one heading: 527 px of rows in the
/// list's 352 px inside.
fn long_list_sheet() -> PropSheet {
    let labels: Vec<String> = (0..30).map(|i| format!("Page {i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let pages = labels.iter().map(|l| check_page(l)).collect();
    let items: Vec<pagelist::Item> = labels.into_iter().map(pagelist::Item::Page).collect();
    PropSheet::with_list(&[("All", &items)], pages, (398, 424))
}

fn list_top(s: &PropSheet) -> i32 {
    match &s.nav {
        Nav::List(l) => l.top,
        Nav::Tabs => unreachable!(),
    }
}

#[test]
fn a_press_on_a_link_row_comes_back_as_its_link() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = link_sheet();
    grey(&mut s, 1);
    let rows = s.layout(&text, px).list.unwrap().rows;
    let at = |r: Rect| (r.x + 20, r.y + 5);
    let (x, y) = at(rows[4]);
    assert_eq!(s.hit(x, y, &text, px), Some(Hit::Link(0)));
    assert_eq!(s.click(Some(Hit::Link(0))), Event::Link(0));
    assert_eq!(
        (s.tabs.open, s.tabs.focused),
        (0, true),
        "the page stays; the list takes the keyboard"
    );
    assert_eq!(cursor(&s), Some(0));
    assert_eq!(s.click(Some(Hit::Link(1))), Event::None, "a greyed link");
    assert_eq!(cursor(&s), Some(0));
}

/// The filter hides the open page's row: the page stays open with its title, the panel has
/// no list item to name it, and Down from the list opens the first shown page.
#[test]
fn an_open_page_the_filter_hides_stays_open() {
    use crate::access::{ROOT, Tree};
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = link_sheet();
    s.list_mut().unwrap().set_filter(Some(&pagelist::Filter {
        pages: vec![false, true, false],
        links: vec![true, false],
    }));
    let mut tree = Tree::window("T", s.size);
    let (panel, _) = s.access(&mut tree, ROOT, &text, 11);
    let node = tree.get(panel).unwrap();
    assert!(node.labelled_by().is_empty());
    assert_eq!(node.label(), Some("Night Light"));
    s.tabs.focused = true;
    s.open_mut().set_focus(None);
    assert_eq!(s.key(form::Key::Down, None), Event::Redraw);
    assert_eq!(s.tabs.open, 1);
    assert!(sheet(&["General"], (398, 424)).list_mut().is_none());
}

#[test]
fn enter_on_the_link_the_list_rests_on_opens_it() {
    let mut s = link_sheet();
    s.tabs.open = 1;
    s.click(Some(Hit::Tab(1)));
    assert_eq!(s.key(form::Key::Down, None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, cursor(&s)),
        (1, Some(0)),
        "resting on a link opens nothing"
    );
    assert_eq!(s.key(form::Key::Enter, None), Event::Link(0));
    assert_eq!(s.key(form::Key::Down, None), Event::Redraw);
    assert_eq!((s.tabs.open, cursor(&s)), (2, None));
    s.key(form::Key::Up, None);
    assert_eq!(cursor(&s), Some(0));
    s.key(form::Key::Tab { back: false }, None);
    assert_eq!(
        (s.tabs.focused, cursor(&s)),
        (false, None),
        "leaving the list lets go of the link"
    );
}

#[test]
fn a_reader_scrolls_a_page_list_row_in_without_turning_to_it() {
    use crate::access::{Published, ROOT, Tree, accesskit::Role};
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = long_list_sheet();
    let mut tree = Tree::window("T", s.size);
    s.access(&mut tree, ROOT, &text, px);
    let last = tree.find(Role::ListItem, "Page 29").unwrap();
    let mut p = Published::default();
    p.update(tree);
    let r = p.reveal(last).expect("the last page is out of view");
    assert_eq!(r.row, 29);
    let (x, y) = (r.at.0 as i32, r.at.1 as i32);
    assert_eq!(s.reveal(x, y, r.row, &text, px), Event::Redraw);
    assert_eq!(list_top(&s), 175, "the last row's bottom at the inside's");
    assert_eq!(s.tabs.open, 0, "the open page stays");
    assert_eq!(s.reveal(x, y, r.row, &text, px), Event::None);
}

#[test]
fn a_pressed_link_half_out_of_the_list_scrolls_into_view() {
    let labels: Vec<String> = (0..30).map(|i| format!("Page {i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let pages = labels.iter().map(|l| check_page(l)).collect();
    let mut items: Vec<pagelist::Item> = labels.into_iter().map(pagelist::Item::Page).collect();
    items.push(pagelist::Item::Link {
        label: "Printers",
        name: "Printers",
    });
    let mut s = PropSheet::with_list(&[("All", &items)], pages, (398, 424));
    s.list_mut().unwrap().top = 183;
    assert_eq!(s.click(Some(Hit::Link(0))), Event::Link(0));
    assert_eq!(list_top(&s), 192, "the link's bottom at the inside's");
    assert_eq!((s.tabs.focused, cursor(&s)), (true, Some(0)));
}

/// A resized list sheet keeps its buttons in the bottom-right corner
/// and its page and list to the new edges; a page takes a template's
/// rects by id, moved to where the pages sit, and keeps its state.
#[test]
fn a_resized_sheet_moves_its_buttons_and_pages_take_a_layout() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    let before = s.layout(&text, px);
    s.pages[0].set_check("check", true);
    s.resize((s.size.0 + 100, s.size.1 + 60));
    let after = s.layout(&text, px);
    assert_eq!(after.page.width, before.page.width + 100);
    assert_eq!(after.page.height, before.page.height + 60);
    assert_eq!(
        after.list.unwrap().frame.height,
        before.list.unwrap().frame.height + 60
    );
    for (b, a) in [
        (before.ok, after.ok),
        (before.cancel, after.cancel),
        (before.apply, after.apply),
    ] {
        assert_eq!((a.x, a.y), (b.x + 100, b.y + 60));
    }
    assert_eq!(s.pages[1].item(OK).unwrap().rect, after.ok);

    let template = Form::new(vec![Item::check("check", 40, 70, "")]);
    let offset = s.page_offset();
    s.pages[0].take_layout(&template, offset);
    let it = s.pages[0].item("check").unwrap();
    assert_eq!((it.rect.x, it.rect.y), (40 + offset.0, 70 + offset.1));
    assert_eq!(s.pages[0].checked("check"), Some(true), "the state stays");
    assert_eq!(
        s.pages[0].item(OK).unwrap().rect,
        after.ok,
        "no template item"
    );
}

/// The list's down arrow held: the sheet asks for ticks, repeats on
/// them after the pause, lights the arrow under the pointer and stops
/// at the release.
#[test]
fn a_held_list_arrow_repeats_on_the_hosts_ticks() {
    use crate::classic::widgets::BarHit;
    use crate::controls::scrollbar::REPEAT_AFTER;
    let mut s = long_list_sheet();
    let down = Some(Hit::ListBar(BarHit::Forward));
    assert_eq!(s.motion(135, 380, down), Event::Redraw, "lit");
    assert!(!s.ticking());
    assert_eq!(s.click(down), Event::Redraw);
    assert_eq!(list_top(&s), 17);
    assert!(s.ticking());
    let t0 = std::time::Instant::now();
    assert_eq!(s.tick(t0), Event::None, "the pause starts");
    assert_eq!(s.tick(t0 + REPEAT_AFTER), Event::Redraw);
    assert_eq!(list_top(&s), 34);
    assert_eq!(s.release(), Event::Redraw, "pressed back to lit");
    assert!(!s.ticking());
    assert_eq!(s.leave(), Event::Redraw, "lit back to rest");
}

/// The overflowing list scrolls under the wheel and its bar, and a
/// page opened from the keys or by the host comes into view.
#[test]
fn a_long_page_list_scrolls_to_the_open_page() {
    use crate::classic::widgets::BarHit;
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = long_list_sheet();
    let lay = s.layout(&text, px).list.unwrap();
    assert_eq!((lay.total, lay.inner.height), (527, 352));
    assert_eq!(lay.scrollbar, Some(Rect::new(128, 34, 16, 352)));
    assert_eq!(s.hit(30, 54, &text, px), Some(Hit::Tab(0)));
    assert_eq!(s.hit(30, 36, &text, px), Some(Hit::List), "the heading");
    assert_eq!(s.hit(135, 36, &text, px), Some(Hit::ListBar(BarHit::Back)));

    // The keys: End brings the last row's bottom to the inside's.
    assert_eq!(s.click(Some(Hit::Tab(0))), Event::Redraw);
    assert!(s.tabs.focused);
    assert_eq!(s.key(form::Key::End, None), Event::Redraw);
    assert_eq!((s.tabs.open, list_top(&s)), (29, 175));
    assert_eq!(s.hit(30, 380, &text, px), Some(Hit::Tab(29)));
    assert_eq!(s.key(form::Key::Home, None), Event::Redraw);
    assert_eq!(list_top(&s), 0);

    // The wheel over the list scrolls it a row per row; over the
    // page it is the page's.
    assert_eq!(s.wheel(Some(Hit::List), 3), Event::Redraw);
    assert_eq!(list_top(&s), 51);
    assert_eq!(s.wheel(None, 3), Event::None);
    assert_eq!(list_top(&s), 51);
    assert_eq!(s.click(Some(Hit::ListBar(BarHit::Back))), Event::Redraw);
    assert_eq!(list_top(&s), 34);

    // The thumb, grabbed and dragged to the bottom.
    let lay = s.layout(&text, px).list.unwrap();
    let thumb = pagelist::bar_parts(&lay).unwrap().thumb.unwrap();
    let Some(Hit::ListBar(grab)) = s.hit(135, thumb.y + 2, &text, px) else {
        panic!("the thumb is at {thumb:?}");
    };
    assert!(matches!(grab, BarHit::Thumb(_)));
    assert_eq!(
        s.click(Some(Hit::ListBar(grab))),
        Event::Redraw,
        "the thumb draws pressed"
    );
    assert_eq!(s.motion(135, 900, None), Event::Redraw);
    assert_eq!(list_top(&s), 175);
    assert_eq!(s.release(), Event::Redraw, "and at rest once let go");
    assert_eq!(s.motion(135, thumb.y, None), Event::None, "let go");
    assert_eq!(list_top(&s), 175);

    // A host opening a page itself reveals it after.
    s.tabs.open = 0;
    assert!(s.reveal_open());
    assert_eq!(list_top(&s), 0, "the first page brings its heading");
}

/// A list sheet is its pages' client plus the list: 146 px wider, the
/// search field at the strip's top-left, the list 4 px under it down to
/// the page's bottom, the page
/// and its controls 146 px right of where they were built, the
/// buttons in the whole sheet's bottom-right corner.
#[test]
fn a_list_sheets_geometry() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    assert_eq!(s.size, (544, 424));
    assert_eq!(s.fit_tabs(&text, px), 0);
    assert_eq!(s.size, (544, 424), "no tabs to wrap");
    let l = s.layout(&text, px);
    assert_eq!(l.page, Rect::new(152, 28, 386, 360));
    assert_eq!(l.search, Some(Rect::new(6, 8, 140, 20)));
    assert_eq!(
        l.list.as_ref().map(|l| l.frame),
        Some(Rect::new(6, 32, 140, 356))
    );
    assert_eq!(l.title, Rect::new(152, 10, 386, 17));
    assert!(l.tabs.is_empty());
    assert_eq!(
        (l.ok.x, l.cancel.x, l.apply.x, l.ok.y),
        (301, 382, 463, 394)
    );
    assert_eq!(s.page_offset(), (146, 0));
    assert_eq!(
        s.open().item("check").unwrap().rect,
        Rect::new(166, 50, 13, 13)
    );
    s.place((3, 21));
    assert_eq!(s.page_offset(), (146, 0), "the origin is the pages' own");
    assert_eq!(s.open().origin, (3, 21));
    let l = s.layout(&text, px);
    assert_eq!(l.search, Some(Rect::new(9, 29, 140, 20)));
    assert_eq!(l.list.map(|l| l.frame), Some(Rect::new(9, 53, 140, 356)));
    assert_eq!(s.open().rect("check"), Some(Rect::new(169, 71, 13, 13)));
    s.set_page(2, check_page("New"));
    assert_eq!(s.pages[2].rect("check"), Some(Rect::new(169, 71, 13, 13)));
    // A tab sheet's pages have not moved.
    assert_eq!(sheet(&["General"], (398, 424)).page_offset(), (0, 0));
    assert_eq!(
        sheet(&["General"], (398, 424)).layout(&text, px).search,
        None,
        "a tab sheet has none"
    );
}

#[test]
#[should_panic(expected = "one page per tab")]
fn a_list_row_without_a_page_is_refused() {
    PropSheet::with_list(LIST_GROUPS, vec![check_page("Night Light")], (398, 424));
}

/// The list is the tab stop the strip was: Shift+Tab off the page's
/// first control reaches it, Up and Down open the page above or below
/// across a heading and stop at the ends, Home and End open the first
/// and the last, Left and Right do nothing, Ctrl+Tab still wraps, and
/// Tab goes into the open page.
#[test]
fn the_lists_keys_step_the_pages() {
    let mut s = list_sheet();
    let key = |s: &mut PropSheet, k| s.key(k, None);
    assert_eq!(key(&mut s, form::Key::Tab { back: true }), Event::Redraw);
    assert!(s.tabs.focused);
    assert_eq!(key(&mut s, form::Key::Down), Event::Redraw);
    assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
    assert_eq!(s.open().focus(), None, "the list keeps the focus");
    assert_eq!(key(&mut s, form::Key::Down), Event::Redraw);
    assert_eq!(s.tabs.open, 2, "past the Input heading");
    assert_eq!(key(&mut s, form::Key::Down), Event::None, "the last page");
    assert_eq!(key(&mut s, form::Key::Up), Event::Redraw);
    assert_eq!(s.tabs.open, 1);
    // Home and End reach a host as editing keys.
    assert_eq!(
        key(&mut s, form::Key::Edit(EditKey::Home { select: false })),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 0);
    assert_eq!(
        key(&mut s, form::Key::Edit(EditKey::End { select: false })),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 2);
    assert_eq!(key(&mut s, form::Key::Left), Event::None);
    assert_eq!(key(&mut s, form::Key::Right), Event::None);
    assert_eq!(s.tabs.open, 2);
    assert_eq!(
        s.key(form::Key::Tab { back: false }, Some(false)),
        Event::Redraw
    );
    assert_eq!(
        (s.tabs.open, s.tabs.focused),
        (0, true),
        "Ctrl+Tab wraps round"
    );
    assert_eq!(key(&mut s, form::Key::Tab { back: false }), Event::Redraw);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), Some(0), "Tab goes into the page");
    key(&mut s, form::Key::Down);
    assert_eq!(s.tabs.open, 0, "the page's own control takes the arrows");
}

/// A press on a page's row opens that page and gives the list the
/// focus; a press on a heading opens nothing; a row under the pointer
/// lights nothing.
#[test]
fn a_press_on_a_row_opens_its_page_and_focuses_the_list() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    let rows = s.layout(&text, px).list.unwrap().rows;
    let at = |r: Rect| (r.x + 20, r.y + 5);
    let (x, y) = at(rows[2]);
    let hit = s.hit(x, y, &text, px);
    assert_eq!(hit, Some(Hit::Tab(1)), "row 2 is Effects, page 1");
    assert_eq!(s.click(hit), Event::Redraw);
    assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
    assert_eq!(s.open().focus(), None);
    assert_eq!(s.click(hit), Event::None, "open and focused already");
    let (x, y) = at(rows[3]);
    let heading = s.hit(x, y, &text, px);
    assert_eq!(heading, Some(Hit::List), "the Input heading");
    assert_eq!(s.click(heading), Event::None, "opens nothing");
    assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
    let check = s.hit(170, 55, &text, px);
    s.click(check);
    assert!(!s.tabs.focused, "the page's control takes the focus back");
    let (x, y) = at(rows[4]);
    s.motion(x, y, s.hit(x, y, &text, px));
    assert_eq!(s.tabs.hot, None);
}

/// In each look: the list exactly as `pagelist::draw` draws it at the
/// sheet's layout, the open page's name over the page in the look's
/// group label ink, the page's edges as `draw_page` draws them with no
/// tab over the top edge, and on Classic OK's black default ring.
#[test]
fn a_list_sheet_draws_in_every_look() {
    use crate::interface::Interface;
    use crate::look::widgets::group_label_ink;
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        let theme = crate::interfaces::theme_for(i);
        let text = TextRenderer::load(&theme.fonts);
        let (p, px) = (theme.paint(), theme.fonts.size_px);
        let mut s = list_sheet();
        s.tabs.open = 1;
        let c = drawn(&s, &theme);
        let l = s.layout(&text, px);
        let lay = l.list.clone().unwrap();
        let Nav::List(list) = &s.nav else {
            unreachable!("a list sheet")
        };
        let mut want = Canvas::new(544, 424);
        want.fill_rect(want.bounds(), theme.colors.button_face);
        pagelist::draw(
            &mut want,
            text.dialog(s.font),
            &p,
            list,
            &s.tabs,
            &lay,
            false,
            px,
        );
        for y in lay.frame.y..lay.frame.bottom() {
            for x in lay.frame.x..lay.frame.right() {
                assert_eq!(c.get(x, y), want.get(x, y), "{i:?}: the list at ({x}, {y})");
            }
        }
        let ink = group_label_ink(&p).argb();
        let t = l.title;
        assert!(
            (t.y..t.bottom()).any(|y| (t.x..t.x + 60).any(|x| c.get(x, y) == ink)),
            "{i:?}: the title in the group label ink"
        );
        assert_eq!(
            c.get(t.right() - 5, t.y + 8),
            theme.colors.button_face.argb()
        );
        let mut bare = Canvas::new(544, 424);
        bare.fill_rect(bare.bounds(), theme.colors.button_face);
        draw_page(&mut bare, &p, l.page);
        let (left, top) = ((l.page.x, l.page.y + 40), (l.page.x + 200, l.page.y));
        assert_eq!(
            c.get(left.0, left.1),
            bare.get(left.0, left.1),
            "{i:?}: the page's left edge"
        );
        assert_eq!(
            c.get(top.0, top.1),
            bare.get(top.0, top.1),
            "{i:?}: its top edge, bare"
        );
        if i == Interface::Classic {
            assert_eq!(
                c.get(301, 394),
                theme.colors.button_text.argb(),
                "OK's default ring"
            );
        }
    }
}

#[test]
fn a_readers_focus_on_a_page_control_takes_the_keyboard_from_the_list() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = list_sheet();
    s.tabs.focused = true;
    let check = s.open().index_of("check").unwrap();
    let r = s.open().items[check].rect;
    let (x, y) = (r.x + 4, r.y + r.height / 2);
    assert_eq!(s.focus_at(x, y, &text, px), Event::Redraw);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), Some(check));
    assert_eq!(s.focus_at(x, y, &text, px), Event::None, "already there");
    let ok = s.open().index_of(OK).unwrap();
    let r = s.open().items[ok].rect;
    assert_eq!(s.focus_at(r.x + 4, r.y + 4, &text, px), Event::Redraw);
    assert_eq!(s.open().focus(), Some(ok), "OK takes it unpressed");
}

/// The open page's item names the page's panel, which holds its
/// controls; with the list focused, the item has the focus.
#[test]
fn a_list_sheets_open_item_names_the_page_it_holds() {
    use crate::access::{ROOT, Tree, accesskit::Role};
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = list_sheet();
    s.tabs.open = 1;
    let mut tree = Tree::window("T", s.size);
    let (panel, focus) = s.access(&mut tree, ROOT, &text, 11);
    let item = tree.find(Role::ListItem, "Effects").unwrap();
    assert_eq!(tree.get(item).unwrap().is_selected(), Some(true));
    assert_eq!(tree.get(panel).unwrap().role(), Role::Group);
    assert_eq!(tree.get(panel).unwrap().labelled_by(), &[item]);
    let check = tree.find(Role::CheckBox, "Effects").unwrap();
    assert!(tree.get(panel).unwrap().children().contains(&check));
    assert_eq!(focus, Some(check));
    assert!(!tree.nodes().any(|(_, n)| n.role() == Role::TabList));
    s.tabs.focused = true;
    s.pages[1].set_focus(None);
    let mut tree = Tree::window("T", s.size);
    let (_, focus) = s.access(&mut tree, ROOT, &text, 11);
    assert_eq!(focus, tree.find(Role::ListItem, "Effects"));
    let mut s = link_sheet();
    s.tabs.open = 1;
    s.click(Some(Hit::Link(0)));
    let mut tree = Tree::window("T", s.size);
    let (panel, focus) = s.access(&mut tree, ROOT, &text, 11);
    assert_eq!(focus, tree.find(Role::ListItem, "Bluetooth..."));
    let effects = tree.find(Role::ListItem, "Effects").unwrap();
    assert_eq!(tree.get(panel).unwrap().labelled_by(), &[effects]);
}

#[test]
fn ctrl_tab_and_ctrl_page_keys_step_the_pages() {
    use crate::textedit::keysym;
    assert_eq!(page_step(keysym::TAB, true, false), Some(false));
    assert_eq!(page_step(keysym::TAB, true, true), Some(true));
    assert_eq!(page_step(keysym::ISO_LEFT_TAB, true, true), Some(true));
    assert_eq!(page_step(keysym::PAGE_DOWN, true, false), Some(false));
    assert_eq!(page_step(keysym::KP_PAGE_DOWN, true, false), Some(false));
    assert_eq!(page_step(keysym::PAGE_UP, true, false), Some(true));
    assert_eq!(page_step(keysym::KP_PAGE_UP, true, true), Some(true));
    assert_eq!(
        page_step(keysym::PAGE_DOWN, false, false),
        None,
        "the page's own"
    );
    assert_eq!(page_step(keysym::TAB, false, false), None);
    assert_eq!(page_step(u32::from(b'a'), true, false), None);
}

/// A Blue sheet hands its page to the controls on it: the blank that
/// breaks the Recycle Bin Global page's group line (156x2 at 30,70 on
/// a 338x376 sheet) shows the page there, and the group's line runs
/// on past it.
#[test]
fn a_blank_on_a_blue_page_shows_the_page() {
    let theme = crate::interfaces::theme_for(crate::interface::Interface::Blue);
    let text = TextRenderer::load(&theme.fonts);
    let px = theme.fonts.size_px;
    let blank = Rect::new(30, 70, 156, 2);
    let form = Form::new(vec![
        Item::group("group", Rect::new(21, 70, 296, 161), ""),
        Item::new("gap", blank, Control::Blank),
    ]);
    let sheet = PropSheet::new(&["Global"], vec![form], (338, 376));
    let p = theme.paint();
    let face = theme.colors.button_face;
    let mut c = Canvas::new(338, 376);
    c.fill_rect(c.bounds(), face);
    let mut bare = c.clone();
    draw(&mut c, &text, &p, &sheet, false, px);
    draw_page(&mut bare, &p, sheet.layout(&text, px).page);
    for y in blank.y..blank.bottom() {
        for x in blank.x..blank.right() {
            assert_eq!(c.get(x, y), bare.get(x, y), "({x}, {y}) is the page");
        }
    }
    assert_eq!(c.get(40, 70), 0xFFFA_FCFA, "the page's row 70");
    let line = crate::blue::Variant::Blue
        .scheme()
        .group_box
        .normal
        .rim
        .argb();
    assert_eq!(c.get(200, 70), line, "the group line past the blank");
}
