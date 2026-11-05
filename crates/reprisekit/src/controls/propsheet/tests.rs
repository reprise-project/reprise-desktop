use super::*;
use crate::classic::widgets::{BUTTON_H, BUTTON_W, BarHit, LINE_H};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::{pagelist, tabstrip};
use crate::look::Paint;
use crate::look::classic_paint;
use crate::look::widgets::draw_page;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, DialogFont, TextRenderer};
use crate::textedit::EditKey;
use crate::theme::TextUnits;
use crate::theme::{FontConfig, Theme};

/// A 398x424 sheet whose page's controls reach its bottom.
fn tall_sheet() -> PropSheet {
    let page = Form::new(vec![
        Item::check("top", 20, 50, "Top"),
        Item::check("low", 20, 424 - PAGE_BOTTOM - PAGE_EDGE - 13, "Low"),
    ]);
    PropSheet::new(&["General"], vec![page], (398, 424))
}

fn sheet(labels: &[&str], size: (i32, i32)) -> PropSheet {
    let pages = labels
        .iter()
        .map(|l| {
            Form::new(vec![Item::new(
                "check",
                Rect::new(20, 50, 13, 13),
                Control::Check {
                    label: (*l).into(),
                    checked: false,
                    enabled: true,
                    mixed: None,
                },
            )])
        })
        .collect();
    PropSheet::new(labels, pages, size)
}

fn check_page(label: &str) -> Form {
    Form::new(vec![Item::new(
        "check",
        Rect::new(20, 50, 13, 13),
        Control::Check {
            label: label.into(),
            checked: false,
            enabled: true,
            mixed: None,
        },
    )])
}

const LIST_GROUPS: &[(&str, &[pagelist::Item])] = &[
    (
        "Look",
        &[
            pagelist::Item::Page("Night Light"),
            pagelist::Item::Page("Effects"),
        ],
    ),
    ("Input", &[pagelist::Item::Page("Mouse")]),
];

fn list_sheet() -> PropSheet {
    let pages = ["Night Light", "Effects", "Mouse"]
        .into_iter()
        .map(check_page)
        .collect();
    PropSheet::with_list(LIST_GROUPS, pages, (398, 424))
}

/// A list sheet of 30 pages under one heading: 527 px of rows in the
/// list's 352 px inside.
fn long_list_sheet() -> PropSheet {
    let labels: Vec<String> = (0..30).map(|i| format!("Page {i}")).collect();
    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
    let pages = labels.iter().map(|l| check_page(l)).collect();
    let items: Vec<pagelist::Item> = labels.into_iter().map(pagelist::Item::Page).collect();
    PropSheet::with_list(&[("All", &items)], pages, (398, 424))
}

const LINK_GROUPS: &[(&str, &[pagelist::Item])] = &[
    (
        "Look",
        &[
            pagelist::Item::Page("Night Light"),
            pagelist::Item::Page("Effects"),
        ],
    ),
    (
        "Devices",
        &[
            pagelist::Item::Link {
                label: "Bluetooth",
                name: "Bluetooth",
            },
            pagelist::Item::Page("Mouse"),
            pagelist::Item::Link {
                label: "Printers",
                name: "Printers",
            },
        ],
    ),
];

/// Two pages under Look; Bluetooth, Mouse and Printers under Devices, the first and last links.
fn link_sheet() -> PropSheet {
    let pages = ["Night Light", "Effects", "Mouse"]
        .into_iter()
        .map(check_page)
        .collect();
    PropSheet::with_list(LINK_GROUPS, pages, (398, 424))
}

fn list_top(s: &PropSheet) -> i32 {
    match &s.nav {
        Nav::List(l) => l.top,
        Nav::Tabs => unreachable!(),
    }
}

fn cursor(s: &PropSheet) -> Option<usize> {
    match &s.nav {
        Nav::List(l) => l.cursor,
        Nav::Tabs => unreachable!(),
    }
}

fn grey(s: &mut PropSheet, link: usize) {
    match &mut s.nav {
        Nav::List(l) => l.set_link_enabled(link, false),
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

/// Tabs on two rows grow the sheet by one row: the page keeps its size
/// 18 px lower, and the controls and buttons move down with it.
#[test]
fn a_second_row_of_tabs_grows_the_sheet_by_a_row() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let labels = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut s = sheet(&labels, (361, 424));
    let before = s.layout(&text, px);
    assert_eq!(s.fit_tabs(&text, px), 18);
    assert_eq!(s.size, (361, 442));
    let l = s.layout(&text, px);
    assert_eq!(l.page, Rect::new(6, 46, 349, 360));
    assert_eq!(l.ok, Rect::new(before.ok.x, before.ok.y + 18, 75, 23));
    assert_eq!(
        s.open().item("check").unwrap().rect,
        Rect::new(20, 68, 13, 13)
    );
    let mut one = sheet(&["Themes", "Desktop"], (398, 424));
    assert_eq!(one.fit_tabs(&text, px), 0);
    assert_eq!(one.size, (398, 424));
}

/// Tabs on one row across the sheet as built wrap onto two in a client cut narrower: the
/// page starts a row lower, the buttons keep to the client's bottom, and a client as wide
/// as the sheet takes the row back.
#[test]
fn a_narrow_client_wraps_the_tabs_to_its_width() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let labels = [
        "General",
        "Sharing",
        "Customize",
        "Security",
        "Previous Versions",
        "Summary",
        "Compatibility",
    ];
    let mut s = sheet(&labels, (700, 424));
    s.drawn_with(&theme.paint());
    assert_eq!(s.fit_tabs(&text, px), 0);
    s.fit((361, 424));
    assert_eq!(s.fit_tabs(&text, px), 18);
    assert_eq!(s.fit_tabs(&text, px), 0, "idempotent");
    assert_eq!(s.client(), (361, 424), "the client keeps its size");
    let l = s.layout(&text, px);
    assert!(
        l.tabs.iter().all(|t| t.right() <= 361 - MARGIN),
        "every tab shows: {:?}",
        l.tabs
    );
    assert_eq!(l.page.y, 46, "the page starts under the second row");
    assert_eq!(l.ok.bottom(), 424 - BUTTON_BOTTOM);
    assert_eq!(s.open().rect("check").unwrap().y, 68);

    s.fit((700, 424));
    assert_eq!(s.fit_tabs(&text, px), -18);
    assert_eq!(s.size, (700, 424));
    assert_eq!(s.page_offset(), (0, 0));
    assert_eq!(s.layout(&text, px).ok.bottom(), 424 - BUTTON_BOTTOM);
}

/// A page cut short above where its last control ends shows whole: no scrollbar, and
/// the wheel moves nothing.
#[test]
fn a_page_cut_short_scrolls_no_further_than_its_last_control() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let px = theme.fonts.size_px;
    let mut s = sheet(&["General"], (398, 424));
    s.drawn_with(&theme.paint());
    assert!(s.fit((398, 200)));
    assert_eq!(s.layout(&text, px).bar, None, "Check shows whole");
    assert_eq!(s.wheel(None, 3), Event::None);
    assert_eq!(s.page_offset(), (0, 0));
}

/// A sheet is set in its first page's font, every page and the tabs
/// with it, and a page put in later takes it too.
#[test]
fn a_sheet_takes_its_first_pages_font() {
    let text =
        TextRenderer::load(&FontConfig::default()).with_shell_dlg(TextRenderer::stub(11, |_| 9));
    let second = text.dialog(DialogFont::ShellDlg);
    let pages = vec![
        Form::new(vec![]).set_in(DialogFont::ShellDlg),
        Form::new(vec![]),
    ];
    let mut s = PropSheet::new(&["General", "Web"], pages, (371, 424));
    assert_eq!(s.font, DialogFont::ShellDlg);
    s.set_page(1, Form::new(vec![]));
    assert!(s.pages.iter().all(|p| p.font == DialogFont::ShellDlg));
    let l = s.layout(&text, 11);
    let want = tabstrip::layout(
        second,
        11,
        &s.tabs,
        l.strip.0,
        l.strip.1,
        l.page.width,
        TextUnits::ONE,
    );
    assert_eq!(l.tabs, want);
    let ui = sheet(&["General", "Web"], (371, 424));
    assert_eq!(ui.font, DialogFont::Ui);
    assert_ne!(ui.layout(&text, 11).tabs, want, "the UI font's tabs");
}

#[test]
fn display_properties_geometry() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let s = sheet(&["Themes", "Desktop", "Screen Saver"], (398, 424));
    let l = s.layout(&text, px);
    // The sheet's margins at a 398x424 client.
    assert_eq!(l.strip, (6, 8));
    assert_eq!(l.page, Rect::new(6, 28, 386, 360));
    assert_eq!(l.ok, Rect::new(155, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.cancel, Rect::new(236, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.apply, Rect::new(317, 394, BUTTON_W, BUTTON_H));
    // The open tab's bottom runs into the page's top edge.
    assert_eq!(l.tabs[0].y, 8);
    assert_eq!(l.tabs[0].bottom(), l.page.y);
}

#[test]
fn the_audio_sheets_geometry() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
    let l = s.layout(&text, px);
    assert_eq!(l.page, Rect::new(6, 28, 349, 228));
    assert_eq!(l.ok, Rect::new(118, 262, BUTTON_W, BUTTON_H));
    assert_eq!(l.cancel, Rect::new(199, 262, BUTTON_W, BUTTON_H));
    assert_eq!(l.apply, Rect::new(280, 262, BUTTON_W, BUTTON_H));
}

#[test]
fn every_page_gets_the_buttons_and_apply_greys_together() {
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    let (ok, _, apply) = buttons(s.size);
    for page in &s.pages {
        assert_eq!(page.item(OK).map(|i| i.rect), Some(ok));
        assert_eq!(page.item(APPLY).map(|i| i.rect), Some(apply));
        assert_eq!(
            page.item(APPLY).map(|i| i.control.clone()),
            Some(Control::Button {
                label: "&Apply".into(),
                enabled: false,
                default: false,
            }),
            "a sheet opens with nothing to apply"
        );
    }
    s.set_apply_enabled(true);
    assert!(s.pages.iter().all(|p| matches!(
        p.item(APPLY).map(|i| &i.control),
        Some(Control::Button { enabled: true, .. })
    )));
}

#[test]
fn close_only_lays_out_one_button() {
    let pages = vec![Form::new(vec![]), Form::new(vec![])];
    let s = close_only(&["General", "Support"], pages, (361, 419));
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    for page in &s.pages {
        let buttons: Vec<&Item> = page
            .items
            .iter()
            .filter(|i| matches!(i.control, Control::Button { .. }))
            .collect();
        assert_eq!(buttons.len(), 1, "one button per page");
        assert_eq!(buttons[0].id, OK);
        assert_eq!(
            buttons[0].control,
            Control::Button {
                label: "Close".into(),
                enabled: true,
                default: true,
            }
        );
    }
    // At a 361x419 client, the Apply slot flush with the page's right
    // edge.
    let l = s.layout(&text, px);
    assert_eq!(l.ok, Rect::new(280, 389, BUTTON_W, BUTTON_H));
    assert_eq!(l.ok.right(), 361 - MARGIN);
    assert_eq!(l.cancel.width, 0);
    assert_eq!(l.apply.width, 0);
}

/// A placed sheet reports its rects and holds its controls at the
/// origin; fitting and placing again move nothing.
#[test]
fn a_placed_sheet_moves_with_its_origin() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["General", "Advanced"], (338, 376));
    let before = s.layout(&text, px);
    s.place((3, 21));
    s.fit_tabs(&text, px);
    s.place((3, 21));
    assert_eq!(s.fit_tabs(&text, px), 0);
    let l = s.layout(&text, px);
    assert_eq!(l.strip, (9, 29));
    assert_eq!(l.page, Rect::new(9, 49, 326, 312));
    assert_eq!(l.ok, Rect::new(98, 367, BUTTON_W, BUTTON_H));
    assert_eq!(l.tabs[0].x, before.tabs[0].x + 3);
    assert_eq!(s.open().rect(OK), Some(l.ok));
    assert_eq!(s.open().rect("check"), Some(Rect::new(23, 71, 13, 13)));
    assert!(matches!(
        s.hit(24, 72, &text, px),
        Some(Hit::Control(form::Hit { item: 0, .. }))
    ));
}

/// A page set anew lands where the pages sit, under the sheet's
/// buttons, and keeps Apply as it stood.
#[test]
fn a_page_set_anew_joins_the_sheet() {
    let mut s = sheet(&["General", "Advanced"], (338, 376));
    s.place((3, 21));
    s.set_apply_enabled(true);
    let page = Form::new(vec![Item::new(
        "new",
        Rect::new(20, 50, 13, 13),
        Control::Check {
            label: "New".into(),
            checked: false,
            enabled: true,
            mixed: None,
        },
    )]);
    s.set_page(1, page);
    let p = &s.pages[1];
    assert_eq!(p.rect("new"), Some(Rect::new(23, 71, 13, 13)));
    assert_eq!(p.rect(OK), s.pages[0].rect(OK));
    assert!(matches!(
        p.item(APPLY).unwrap().control,
        Control::Button { enabled: true, .. }
    ));
}

/// OK and Cancel alone take the row's two right slots.
#[test]
fn ok_cancel_fills_the_right_slots() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let pages = vec![Form::new(vec![]), Form::new(vec![])];
    let s = PropSheet::with_buttons(&["General", "Web"], pages, (371, 424), Buttons::OkCancel);
    let l = s.layout(&text, px);
    assert_eq!(l.ok, Rect::new(209, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.cancel, Rect::new(290, 394, BUTTON_W, BUTTON_H));
    assert_eq!(l.apply.width, 0);
    for page in &s.pages {
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.item(OK).unwrap().rect, l.ok);
        assert_eq!(page.item(CANCEL).unwrap().rect, l.cancel);
        assert_eq!(page.focus(), page.index_of(OK));
    }
}

#[test]
#[should_panic(expected = "one page per tab")]
fn a_tab_without_a_page_is_refused() {
    PropSheet::new(&["General", "Sharing"], vec![Form::new(vec![])], (361, 424));
}

#[test]
#[should_panic(expected = "one page per tab")]
fn a_close_only_tab_without_a_page_is_refused() {
    close_only(&["General", "Support"], vec![Form::new(vec![])], (361, 419));
}

#[test]
fn ctrl_tab_cycles_the_pages() {
    let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
    assert_eq!(s.tabs.open, 0);
    assert_eq!(
        s.key(form::Key::Tab { back: false }, Some(false)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 1);
    assert!(
        matches!(&s.open().item("check").unwrap().control,
            Control::Check { label, .. } if label == "Sounds"),
        "the open page followed the tab"
    );
    assert_eq!(
        s.key(form::Key::Tab { back: true }, Some(true)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 0);
    assert_eq!(
        s.key(form::Key::Tab { back: true }, Some(true)),
        Event::Redraw
    );
    assert_eq!(s.tabs.open, 2, "wraps");
    // Without the Ctrl flag Tab walks the page's controls instead.
    let before = s.tabs.open;
    assert_eq!(s.key(form::Key::Tab { back: false }, None), Event::Redraw);
    assert_eq!(s.tabs.open, before);
}

#[test]
fn the_sheets_buttons_become_sheet_events() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    s.set_apply_enabled(true);
    let l = s.layout(&text, px);
    // Enter with the page's own control focused is the default button.
    assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
    // Press and release Apply.
    let hit = s.hit(l.apply.x + 5, l.apply.y + 5, &text, px);
    assert_eq!(s.click(hit), Event::Redraw, "the button goes down");
    assert_eq!(s.release(), Event::Apply);
    // A control of the page's own stays the page's.
    let check = s.open().index_of("check").unwrap();
    let hit = s.hit(
        s.open().items[check].rect.x + 3,
        s.open().items[check].rect.y + 3,
        &text,
        px,
    );
    assert!(matches!(hit, Some(Hit::Control(_))));
    assert_eq!(s.click(hit), Event::Redraw, "the box goes down");
    assert_eq!(s.release(), Event::Page(0, form::Event::Changed("check")));
}

#[test]
fn a_tab_hit_opens_its_page() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = sheet(&["Volume", "Sounds"], (361, 292));
    let l = s.layout(&text, px);
    let hit = s.hit(l.tabs[1].x + 4, l.tabs[1].y + 10, &text, px);
    assert_eq!(hit, Some(Hit::Tab(1)));
    assert_eq!(s.click(hit), Event::Redraw);
    assert_eq!(s.tabs.open, 1);
    assert!(!s.tabs.focused, "a closed tab opens its page");
    assert_eq!(s.open().focus(), Some(0), "on its first control");
    assert_eq!(s.click(hit), Event::Redraw, "the open tab takes the focus");
    assert!(s.tabs.focused);
    assert_eq!(s.open().focus(), None);
    assert_eq!(s.click(hit), Event::None, "already focused");
    // A control pressed on the page takes the focus back.
    let check = s.hit(25, 55, &text, px);
    s.click(check);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), Some(0));
}

/// Around the strip and back: Shift+Tab off the page's first control
/// reaches the strip, Right and Left turn the tabs with the focus kept
/// there, Up and Down do nothing, Ctrl+Tab turns them too, and Tab
/// goes into the open page.
#[test]
fn the_strip_is_a_tab_stop_its_arrows_turn_the_tabs() {
    let mut s = sheet(&["Volume", "Sounds", "Audio"], (361, 292));
    let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
    assert_eq!(s.open().focus(), Some(0));
    assert_eq!(tab(&mut s, true), Event::Redraw);
    assert!(s.tabs.focused);
    assert_eq!(s.open().focus(), None);
    assert_eq!(s.key(form::Key::Left, None), Event::None, "first tab");
    // The arrow as a host folds it for a field: an editing key.
    let right = form::Key::Edit(EditKey::Right {
        select: false,
        word: false,
    });
    assert_eq!(s.key(right, None), Event::Redraw);
    assert_eq!((s.tabs.open, s.tabs.focused), (1, true));
    assert_eq!(s.open().focus(), None, "the new page holds no focus");
    assert_eq!(s.key(form::Key::Down, None), Event::None);
    assert_eq!(s.key(form::Key::Up, None), Event::None);
    assert_eq!(s.tabs.open, 1);
    assert_eq!(
        s.key(form::Key::Tab { back: false }, Some(false)),
        Event::Redraw
    );
    assert_eq!((s.tabs.open, s.tabs.focused), (2, true));
    assert_eq!(s.key(form::Key::Right, None), Event::None, "last tab");
    // Enter on the strip is the default button, Escape Cancel.
    assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
    // Shift+Tab leaves for the page's last tab stop: Cancel, the
    // greyed Apply skipped.
    assert_eq!(tab(&mut s, true), Event::Redraw);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), s.open().index_of(CANCEL));
    tab(&mut s, false);
    tab(&mut s, false);
    assert_eq!(s.open().focus(), Some(0), "Tab from the strip");
    // Ctrl+Tab from the page lands on the new page's first control,
    // whatever that page last had focused.
    s.pages[0].focus_id(OK);
    s.key(form::Key::Tab { back: false }, Some(false));
    assert_eq!((s.tabs.open, s.tabs.focused), (0, false));
    assert_eq!(s.open().focus(), Some(0));
}

// A folder Properties sheet at a 361x424 client and the pixel each
// part of it lands on.
/// 361x424 client (367x448 outer less the 3 px frame and 21 px caption).
const FOLDER_SIZE: (i32, i32) = (361, 424);
/// The open tab's white top ring.
const FOLDER_STRIP_Y: i32 = 8;
/// Right of the last tab: the page's white top ring, and its
/// shadow/dk_shadow bottom.
const FOLDER_PAGE_TOP: i32 = 28;
const FOLDER_PAGE_BOTTOM: i32 = 387;
/// The page's white left ring and its shadow/dk_shadow right.
const FOLDER_PAGE_LEFT: i32 = 6;
const FOLDER_PAGE_RIGHT: i32 = 354;
/// OK's black default ring, then Cancel's and Apply's white ones.
const FOLDER_OK_X: i32 = 118;
const FOLDER_CANCEL_X: i32 = 199;
const FOLDER_APPLY_X: i32 = 280;
/// The button rings' top row; they run 23 rows.
const FOLDER_BUTTON_Y: i32 = 394;

/// Two of the compositor's sheets, in client coordinates: Recycle Bin
/// Properties (344x400 outer) and a 404x448 outer sheet.
const BIN: (i32, i32) = (338, 376);
const BIN_PAGE: Rect = Rect::new(6, 28, 326, 312);
const BIN_BUTTONS: (i32, i32, i32, i32) = (95, 176, 257, 346);
const DISPLAY: (i32, i32) = (398, 424);
const DISPLAY_PAGE: Rect = Rect::new(6, 28, 386, 360);
const DISPLAY_BUTTONS: (i32, i32, i32, i32) = (155, 236, 317, 394);

/// A folder-Properties-shaped page: a label, a check and a button of
/// the page's own, so the Tab order has somewhere to start.
fn folder_page(id: &'static str) -> Form {
    Form::new(vec![
        Item::new(
            "label",
            Rect::new(20, 50, 100, 13),
            Control::Label {
                text: "Type:".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
        Item::new(
            id,
            Rect::new(20, 70, 13, 13),
            Control::Check {
                label: "&Read-only".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "adv",
            Rect::new(250, 70, BUTTON_W, BUTTON_H),
            Control::Button {
                label: "Ad&vanced...".into(),
                enabled: true,
                default: false,
            },
        ),
    ])
}

fn folder_sheet() -> PropSheet {
    PropSheet::new(
        &["General", "Sharing", "Customize"],
        vec![
            folder_page("ro"),
            folder_page("share"),
            folder_page("custom"),
        ],
        FOLDER_SIZE,
    )
}

#[test]
fn folder_properties_geometry() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let l = folder_sheet().layout(&text, px);
    assert_eq!(l.tabs[0].y, FOLDER_STRIP_Y);
    assert_eq!(
        l.tabs[0].x, FOLDER_PAGE_LEFT,
        "the open tab starts at the margin"
    );
    assert_eq!((l.page.x, l.page.y), (FOLDER_PAGE_LEFT, FOLDER_PAGE_TOP));
    assert_eq!(l.page.right() - 1, FOLDER_PAGE_RIGHT);
    assert_eq!(l.page.bottom() - 1, FOLDER_PAGE_BOTTOM);
    for r in [l.ok, l.cancel, l.apply] {
        assert_eq!(
            (r.width, r.height, r.y),
            (BUTTON_W, BUTTON_H, FOLDER_BUTTON_Y)
        );
    }
    assert_eq!(
        (l.ok.x, l.cancel.x, l.apply.x),
        (FOLDER_OK_X, FOLDER_CANCEL_X, FOLDER_APPLY_X)
    );
    assert_eq!(l.apply.right(), FOLDER_SIZE.0 - MARGIN);
    assert_eq!(l.cancel.x - l.ok.right(), BUTTON_PITCH - BUTTON_W);
}

#[test]
fn the_same_rule_gives_the_compositors_sheets() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    for (size, want_page, want_buttons) in [
        (BIN, BIN_PAGE, BIN_BUTTONS),
        (DISPLAY, DISPLAY_PAGE, DISPLAY_BUTTONS),
    ] {
        let mut s = folder_sheet();
        s.size = size;
        let l = s.layout(&text, px);
        assert_eq!(l.page, want_page, "page of {size:?}");
        let (ok, cancel, apply, y) = want_buttons;
        assert_eq!(
            (l.ok.x, l.cancel.x, l.apply.x, l.ok.y),
            (ok, cancel, apply, y),
            "buttons of {size:?}"
        );
        assert_eq!(l.strip, STRIP);
    }
}

#[test]
fn tab_runs_off_the_page_onto_the_buttons_and_back() {
    let mut s = folder_sheet();
    let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
    let focus_id = |s: &PropSheet| s.open().focus().map(|i| s.open().items[i].id);
    // "adv" is the page's own last tab stop; Tab off it lands on OK.
    s.open_mut().focus_id("adv");
    assert_eq!(tab(&mut s, false), Event::Redraw);
    assert_eq!(focus_id(&s), Some(OK));
    assert_eq!(s.key(form::Key::Enter, None), Event::Ok);
    assert_eq!(s.key(form::Key::Space, None), Event::Ok);
    // Shift+Tab off OK goes back to the page's last control.
    assert_eq!(tab(&mut s, true), Event::Redraw);
    assert_eq!(focus_id(&s), Some("adv"));
    // Forward: OK, Cancel, then the tab strip and round to the page's
    // first control: a greyed Apply is not a tab stop.
    tab(&mut s, false);
    tab(&mut s, false);
    assert_eq!(focus_id(&s), Some(CANCEL));
    assert_eq!(s.key(form::Key::Enter, None), Event::Cancel);
    tab(&mut s, false);
    assert!(s.tabs.focused, "skipped the greyed Apply onto the strip");
    assert_eq!(focus_id(&s), None, "the page holds no focus meanwhile");
    tab(&mut s, false);
    assert!(!s.tabs.focused);
    assert_eq!(focus_id(&s), Some("ro"));
    // Live, Apply joins the order after Cancel.
    s.set_apply_enabled(true);
    s.open_mut().focus_id("adv");
    tab(&mut s, false);
    tab(&mut s, false);
    tab(&mut s, false);
    assert_eq!(focus_id(&s), Some(APPLY));
    assert_eq!(s.key(form::Key::Enter, None), Event::Apply);
}

#[test]
fn arrows_on_the_sheet_buttons_move_between_them() {
    let mut s = folder_sheet();
    let focus_id = |s: &PropSheet| s.open().focus().map(|i| s.open().items[i].id);
    s.open_mut().focus_id(OK);
    assert_eq!(s.key(form::Key::Right, None), Event::Redraw);
    assert_eq!(focus_id(&s), Some(CANCEL), "Right moves from OK to Cancel");
    assert_eq!(s.key(form::Key::Right, None), Event::None);
    assert_eq!(focus_id(&s), Some(CANCEL), "a greyed Apply is passed over");
    assert_eq!(s.key(form::Key::Enter, None), Event::Cancel);
    assert_eq!(s.key(form::Key::Left, None), Event::Redraw);
    assert_eq!(focus_id(&s), Some(OK), "Left moves back to OK");
    assert!(!s.tabs.focused, "the strip keeps out of it");
}

#[test]
fn alt_a_presses_apply_only_when_it_is_live() {
    let mut s = folder_sheet();
    // Greyed, nothing answers the access key the button still draws.
    assert_eq!(s.key(form::Key::Char('a'), None), Event::None);
    s.set_apply_enabled(true);
    assert_eq!(s.key(form::Key::Char('A'), None), Event::Apply);
    assert_eq!(s.key(form::Key::Char('a'), None), Event::Apply);
    // A letter the page wants still goes to the page: "&Read-only".
    assert_eq!(
        s.key(form::Key::Char('r'), None),
        Event::Page(0, form::Event::Changed("ro"))
    );
    assert_eq!(s.key(form::Key::Char('z'), None), Event::None);
}

#[test]
fn draws_the_folder_sheets_frames_and_buttons() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let cs = &theme.colors;
    let px = theme.fonts.size_px;
    let s = folder_sheet();
    let mut c = Canvas::new(FOLDER_SIZE.0 as u32, FOLDER_SIZE.1 as u32);
    c.fill_rect(c.bounds(), cs.button_face);
    draw(&mut c, &text, &classic_paint(cs, 1), &s, false, px);
    // The page's rings.
    assert_eq!(
        c.get(FOLDER_PAGE_LEFT, FOLDER_PAGE_TOP + 40),
        cs.button_hilight.argb()
    );
    assert_eq!(
        c.get(FOLDER_PAGE_RIGHT, FOLDER_PAGE_TOP + 40),
        cs.button_dk_shadow.argb()
    );
    assert_eq!(
        c.get(FOLDER_PAGE_LEFT + 40, FOLDER_PAGE_BOTTOM),
        cs.button_dk_shadow.argb()
    );
    // The open tab's left ring, and the page's top ring surviving to
    // the right of the last tab.
    assert_eq!(
        c.get(FOLDER_PAGE_LEFT, FOLDER_STRIP_Y + 12),
        cs.button_hilight.argb()
    );
    assert_eq!(
        c.get(FOLDER_PAGE_RIGHT - 20, FOLDER_PAGE_TOP),
        cs.button_hilight.argb()
    );
    // OK wears the black default ring; Cancel and the greyed Apply
    // are plain raised buttons.
    assert_eq!(c.get(FOLDER_OK_X, FOLDER_BUTTON_Y), cs.button_text.argb());
    assert_eq!(
        c.get(FOLDER_CANCEL_X, FOLDER_BUTTON_Y),
        cs.button_hilight.argb()
    );
    assert_eq!(
        c.get(FOLDER_APPLY_X, FOLDER_BUTTON_Y),
        cs.button_hilight.argb()
    );
    assert_eq!(
        c.get(FOLDER_APPLY_X + BUTTON_W - 1, FOLDER_BUTTON_Y),
        cs.button_dk_shadow.argb()
    );
}

/// Writes target/propsheet-folder.png.
#[test]
#[ignore]
fn render_propsheet_folder() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = folder_sheet();
    s.set_apply_enabled(true);
    let (w, h) = s.size;
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &s,
        true,
        theme.fonts.size_px,
    );
    crate::raster::write_preview(&c, "propsheet-folder.png");
}

#[test]
#[ignore]
fn render_propsheet() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let s = preview_sheet();
    let (w, h) = s.size;
    let mut c = Canvas::new(w as u32, h as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    draw(
        &mut c,
        &text,
        &classic_paint(&theme.colors, 1),
        &s,
        true,
        theme.fonts.size_px,
    );
    crate::raster::write_preview(&c, "propsheet.png");
}

/// A Volume page like the audio sheet's, at its 361x292 client.
fn preview_sheet() -> PropSheet {
    let page = Form::new(vec![
        Item::new(
            "device_group",
            Rect::new(14, 40, 321, 60),
            Control::Group("Device volume".into()),
        ),
        Item::new(
            "volume",
            Rect::new(28, 62, 180, crate::trackbar::HEIGHT),
            Control::TrackBar(crate::trackbar::TrackBar::new(10, 6)),
        ),
        Item::new(
            "mute",
            Rect::new(224, 66, 13, 13),
            Control::Check {
                label: "&Mute".into(),
                checked: false,
                enabled: true,
                mixed: None,
            },
        ),
        Item::new(
            "hint",
            Rect::new(14, 112, 321, 14),
            Control::Label {
                text: "Move the slider to set the speaker volume.".into(),
                enabled: true,
                align: Align::Left,
            },
        ),
    ]);
    let blank = Form::new(vec![]);
    PropSheet::new(
        &["Volume", "Sounds", "Audio"],
        vec![page, blank.clone(), blank],
        (361, 292),
    )
}

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

/// A sheet drawn under `theme`, which tells its pages whether the look
/// lights parts under the pointer.
fn drawn(s: &PropSheet, theme: &Theme) -> Canvas {
    let text = TextRenderer::load(&theme.fonts);
    let mut c = Canvas::new(s.size.0 as u32, s.size.1 as u32);
    c.fill_rect(c.bounds(), theme.colors.button_face);
    draw(&mut c, &text, &theme.paint(), s, false, theme.fonts.size_px);
    c
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

/// At 150% and 200% a tabbed sheet grows from its origin: the client, the page and the
/// buttons grow with the text, the tabs are as tall as the grown row with the open one
/// running into the page, and a click on a grown button lands on it. 100% is the layout as
/// built.
#[test]
fn a_grown_sheet_lays_its_page_tabs_and_buttons_out_with_the_text() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    for px in [11, 17, 22] {
        let u = TextUnits::new(px, 11);
        let mut s = sheet(&["General", "Sharing"], (398, 424));
        s.place((3, 21));
        let p = Paint {
            text: u,
            ..theme.paint()
        };
        s.drawn_with(&p);
        assert_eq!(s.client(), (u.len(398), u.len(424)));
        let l = s.layout(&text, px);
        let open = l.tabs[0];
        assert_eq!(open.height, u.len(tabstrip::TAB_H) + 2, "{px}");
        assert_eq!(
            open.bottom(),
            l.page.y,
            "{px}: the open tab runs into the page"
        );
        assert_eq!(l.page.x, 3 + u.len(MARGIN));
        assert_eq!(l.page.right(), 3 + u.len(398 - MARGIN));
        assert_eq!(l.apply.right(), 3 + u.len(398 - MARGIN));
        assert_eq!(l.apply.bottom(), 21 + u.len(424 - BUTTON_BOTTOM));
        let apply = s.open().rect(APPLY).unwrap();
        assert_eq!(
            apply, l.apply,
            "{px}: the page's button is where the layout says"
        );
        let mut c = Canvas::new(3 + u.len(398) as u32, 21 + u.len(424) as u32);
        draw(&mut c, &text, &p, &s, false, px);
        let ok = l.ok;
        let hit = s.hit(ok.x + ok.width / 2, ok.y + ok.height / 2, &text, px);
        assert!(matches!(hit, Some(Hit::Control(_))), "{px}");
        assert_eq!(s.click(hit), Event::Redraw);
        assert_eq!(s.release(), Event::Ok, "{px}");
    }
}

/// A list sheet at 150%: the list's rows grow with the text and its frame from the origin.
#[test]
fn a_grown_list_sheet_grows_its_rows() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let u = TextUnits::new(17, 11);
    let s = list_sheet();
    s.drawn_with(&Paint {
        text: u,
        ..theme.paint()
    });
    let l = s.layout(&text, 17);
    let list = l.list.unwrap();
    assert_eq!(list.row_h, u.len(17));
    assert_eq!(list.rows[1].y - list.rows[0].y, u.len(17));
    assert_eq!(list.frame.x, u.len(MARGIN));
    assert_eq!(l.page.x, u.len(pagelist::ADVANCE + MARGIN));
}

/// A 398x424 sheet shown 300 tall at 200%: the buttons keep to the client's bottom, the page
/// ends above them with a scrollbar, a control below the page's end neither draws nor takes
/// the pointer there, and the wheel or Tab onto it scrolls it into view, no further.
#[test]
fn a_short_client_keeps_its_buttons_and_scrolls_the_page() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let page = Form::new(vec![
        Item::check("top", 20, 50, "Top"),
        Item::check("low", 20, 350, "Low"),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(s.fit((398, 300)));
    assert!(!s.fit((398, 300)), "idempotent");
    assert_eq!(
        s.size,
        (398, 424),
        "the pages keep the size they were built for"
    );
    assert_eq!(s.client(), (u.len(398), u.len(300)));
    let l = s.layout(&text, px);
    assert_eq!(l.ok.bottom(), u.len(300 - BUTTON_BOTTOM));
    assert_eq!(l.page.bottom(), u.len(300 - PAGE_BOTTOM));
    let bar = l.bar.expect("a page that shows in part has a scrollbar");
    assert_eq!(bar.right(), l.page.right() - PAGE_EDGE);
    let low = s.open().rect("low").unwrap();
    assert!(low.y > l.page.bottom(), "Low starts below the page's end");
    assert!(
        s.hit(low.x + 4, low.y + low.height / 2, &text, px)
            .is_none(),
        "Low takes no pointer over the buttons' row"
    );
    let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
    draw(&mut c, &text, &p, &s, false, px);

    assert_eq!(s.wheel(None, 100), Event::Redraw);
    assert_eq!(
        s.page_offset(),
        (0, -101),
        "scrolled until Low's bottom meets the page's"
    );
    let low = s.open().rect("low").unwrap();
    assert!(
        low.y >= l.page.y && low.bottom() <= l.page.bottom(),
        "{low:?} shows"
    );
    let hit = s.hit(low.x + 4, low.y + low.height / 2, &text, px);
    assert!(
        matches!(hit, Some(Hit::Control(_))),
        "Low takes the pointer"
    );
    assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

    assert_eq!(s.wheel(None, -100), Event::Redraw);
    s.open_mut().focus_id("top");
    s.key(form::Key::Tab { back: false }, None);
    assert_eq!(s.open().focus().map(|i| s.open().items[i].id), Some("low"));
    let low = s.open().rect("low").unwrap();
    assert!(
        low.bottom() <= l.page.bottom(),
        "Tab onto Low brings it into view"
    );

    assert!(s.fit((398, 424)));
    assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
    assert_eq!(s.layout(&text, px).bar, None);
}

/// A note whose lines rewrap onto one more line than its rect holds pushes the controls under
/// it down a line, and the page scrolls through that line though the sheet is whole.
#[test]
fn a_rewrapped_note_pushes_the_page_down_and_scrolls_it() {
    let text = TextRenderer::load(&FontConfig::default());
    let theme = Theme::default();
    let (px, p) = (theme.fonts.size_px, theme.paint());
    // The second line is the wider, so in any face it no longer fits once the first line's
    // last word wraps onto it.
    let lines = [
        "Sunset and sunrise are worked out",
        "for the time zone's city, not this machine's.",
    ];
    assert!(text.measure(lines[1], false, px) > text.measure(lines[0], false, px));
    let w = text.measure(lines[0], false, px) - 20;
    let page = Form::new(vec![
        Item::lines("note", Rect::new(20, 340, w, 2 * LINE_H), &lines),
        Item::check("low", 20, 373, "Low"),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(!s.fit((398, 424)), "the sheet is whole");
    let mut c = Canvas::new(398, 424);
    draw(&mut c, &text, &p, &s, false, px);
    let note = s.open().rect("note").unwrap();
    assert_eq!(note.height, 3 * LINE_H, "the note takes its third line");
    assert_eq!(s.open().rect("low").unwrap().y, 373 + LINE_H);
    assert_eq!(s.open().rect(OK).unwrap(), s.layout(&text, px).ok);
    assert!(s.layout(&text, px).bar.is_some(), "the page scrolls");
    assert_eq!(s.page_span().1 - s.page_span().0, LINE_H);
    assert_eq!(s.wheel(None, 100), Event::Redraw);
    assert_eq!(s.page_offset().1, -LINE_H, "through the pushed line");

    let roomy = Form::new(vec![
        Item::lines("note", Rect::new(20, 340, 360, 2 * LINE_H), &lines),
        Item::check("low", 20, 373, "Low"),
    ]);
    s.set_page(0, roomy);
    draw(&mut c, &text, &p, &s, false, px);
    s.fit((398, 424));
    assert_eq!(s.page_offset().1, 0, "nothing left to scroll");
    assert_eq!(
        s.open().rect("low").unwrap().y,
        373,
        "a note that fits moves nothing"
    );
    assert_eq!(s.layout(&text, px).bar, None);
}

/// A press on the page's scrollbar arrow scrolls a line; its thumb drags the page.
#[test]
fn the_page_scrollbar_scrolls_the_page() {
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = tall_sheet();
    s.drawn_with(&Theme::default().paint());
    s.fit((398, 300));
    let bar = s.layout(&text, px).bar.unwrap();
    let down = s.hit(bar.x + 2, bar.bottom() - 3, &text, px);
    assert_eq!(down, Some(Hit::PageBar(BarHit::Forward)));
    assert_eq!(s.click(down), Event::Redraw);
    assert_eq!(s.page_offset().1, -LINE_H);
    s.release();
    let parts = s.bar_parts(bar);
    let grab = s.hit(bar.x + 2, parts.thumb.unwrap().y + 2, &text, px);
    assert!(
        matches!(grab, Some(Hit::PageBar(BarHit::Thumb(_)))),
        "{grab:?}"
    );
    s.click(grab);
    assert_eq!(s.motion(bar.x + 2, bar.bottom(), None), Event::Redraw);
    assert_eq!(s.page_offset().1, -124, "dragged to the end");
    s.release();
}

/// The page's arrow held repeats on the host's ticks like the kit's other scrollbars.
#[test]
fn a_held_page_arrow_repeats_on_the_hosts_ticks() {
    use crate::controls::scrollbar::{REPEAT_AFTER, REPEAT_EVERY};
    let text = TextRenderer::load(&FontConfig::default());
    let px = Theme::default().fonts.size_px;
    let mut s = tall_sheet();
    s.drawn_with(&Theme::default().paint());
    s.fit((398, 300));
    let bar = s.layout(&text, px).bar.unwrap();
    let (x, y) = (bar.x + 2, bar.bottom() - 3);
    let down = s.hit(x, y, &text, px);
    s.motion(x, y, down);
    assert!(!s.ticking());
    s.click(down);
    assert!(s.ticking());
    assert_eq!(s.page_offset().1, -LINE_H);
    let t0 = std::time::Instant::now();
    assert_eq!(s.tick(t0), Event::None, "the pause starts");
    assert_eq!(s.tick(t0 + REPEAT_AFTER), Event::Redraw);
    assert_eq!(s.page_offset().1, -2 * LINE_H);
    assert_eq!(s.tick(t0 + REPEAT_AFTER + REPEAT_EVERY), Event::Redraw);
    assert_eq!(s.page_offset().1, -3 * LINE_H);
    s.motion(x - 40, y, None);
    assert_eq!(
        s.tick(t0 + REPEAT_AFTER + 2 * REPEAT_EVERY),
        Event::None,
        "off the arrow"
    );
    s.release();
    assert!(!s.ticking());
}

/// A 398x424 sheet shown 300 wide at 200%: the buttons keep to the client's bottom-right
/// corner, the page ends at its right edge with a bar along its bottom, and the bar's arrow,
/// or Tab onto a control past that edge, scrolls the page sideways.
#[test]
fn a_narrow_client_keeps_its_buttons_and_scrolls_the_page_sideways() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let page = Form::new(vec![
        Item::check("left", 20, 50, "Left"),
        Item::button("right", Rect::new(300, 50, 75, 23), "Right", true, false),
    ]);
    let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
    s.drawn_with(&p);
    assert!(s.fit((300, 424)));
    assert!(!s.fit((300, 424)), "idempotent");
    assert_eq!(
        s.size,
        (398, 424),
        "the pages keep the size they were built for"
    );
    assert_eq!(s.client(), (u.len(300), u.len(424)));
    let l = s.layout(&text, px);
    assert_eq!(l.apply.right(), u.len(300 - MARGIN));
    assert_eq!(l.page.right(), u.len(300 - MARGIN));
    assert_eq!(l.bar, None, "the page shows whole down");
    let hbar = l.hbar.expect("a page cut narrow has a sideways bar");
    assert_eq!(hbar.bottom(), l.page.bottom() - PAGE_EDGE);
    assert_eq!(hbar.right(), l.page.right() - PAGE_EDGE);
    let mut c = Canvas::new(u.len(300) as u32, u.len(424) as u32);
    draw(&mut c, &text, &p, &s, false, px);
    let right = s.open().rect("right").unwrap();
    assert!(
        right.right() > hbar.right(),
        "Right reaches past the page's edge"
    );

    let arrow = s.hit(hbar.right() - 3, hbar.y + 2, &text, px);
    assert_eq!(arrow, Some(Hit::PageHBar(BarHit::Forward)));
    assert_eq!(s.click(arrow), Event::Redraw);
    s.release();
    assert_eq!(s.page_offset(), (-LINE_H, 0));

    s.open_mut().focus_id("left");
    s.key(form::Key::Tab { back: false }, None);
    assert_eq!(
        s.open().focus().map(|i| s.open().items[i].id),
        Some("right")
    );
    let right = s.open().rect("right").unwrap();
    assert!(
        right.right() <= hbar.right(),
        "Tab onto Right brings it into view: {right:?}"
    );
    assert_eq!(s.open().rect(OK).unwrap(), l.ok, "the buttons stay put");

    assert!(s.fit((398, 424)));
    assert_eq!(s.page_offset(), (0, 0), "a whole sheet does not scroll");
    assert_eq!(s.layout(&text, px).hbar, None);
}

/// A page cut short at 200% scrolls under a bar down its right edge. A control flush with
/// that edge would lie under the bar, so the page scrolls sideways by the bar's width as
/// well; one that keeps the page's margin leaves the page whole across.
#[test]
fn the_page_scrollbar_covers_no_control() {
    let theme = Theme::default().with_form_text(200);
    let (u, px) = (theme.text, theme.fonts.size_px);
    let text = TextRenderer::load(&FontConfig::default());
    let p = theme.paint();
    let edge = 398 - MARGIN - PAGE_EDGE;
    let at = |right: i32| {
        let page = Form::new(vec![
            Item::check("top", 20, 50, "Top"),
            Item::check("low", 20, 373, "Low"),
            Item::button(
                "flush",
                Rect::new(right - 75, 100, 75, 23),
                "Flush",
                true,
                false,
            ),
        ]);
        let mut s = PropSheet::new(&["General"], vec![page], (398, 424));
        s.drawn_with(&p);
        s.fit((398, 300));
        s
    };
    let mut s = at(edge);
    let l = s.layout(&text, px);
    let bar = l.bar.expect("a page cut short has a scrollbar");
    let hbar = l
        .hbar
        .expect("a control under the bar brings in the sideways bar");
    assert_eq!(hbar.right(), bar.x, "the bars meet at the corner");
    assert_eq!(bar.bottom(), hbar.y);
    assert_eq!(s.hrange(), u.design(s.open().controls().scroll));
    assert_eq!(
        s.wheel(Some(Hit::PageHBar(BarHit::Track)), 100),
        Event::Redraw
    );
    let flush = s.open().rect("flush").unwrap();
    assert!(
        flush.right() <= bar.x,
        "{flush:?} clears the bar at {bar:?}"
    );
    let mut c = Canvas::new(u.len(398) as u32, u.len(300) as u32);
    draw(&mut c, &text, &p, &s, false, px);

    let s = at(edge - 12);
    assert!(s.layout(&text, px).bar.is_some());
    assert_eq!(s.layout(&text, px).hbar, None, "the margin holds the bar");

    let mut s = at(edge + 30);
    s.fit((398, 424));
    let l = s.layout(&text, px);
    assert_eq!(
        (l.bar, l.hbar),
        (None, None),
        "a whole sheet does not scroll to a rect built past its page"
    );
}

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
    if crate::text::test_tahoma() {
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
}

/// At 200% the field doubles and the list starts the doubled gap below it.
#[test]
fn the_search_field_grows_with_the_text() {
    let theme = Theme::default();
    let text = TextRenderer::load(&FontConfig::default());
    let u = TextUnits::new(22, 11);
    let s = list_sheet();
    s.drawn_with(&Paint {
        text: u,
        ..theme.paint()
    });
    let l = s.layout(&text, 22);
    assert_eq!(l.search, Some(Rect::new(12, 16, 280, 40)));
    assert_eq!(l.list.unwrap().frame.y, 64);
}

#[test]
fn a_press_in_the_search_field_takes_the_keyboard_and_edits_report_the_text() {
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = list_sheet();
    let hit = s.hit(10, 15, &text, 11);
    assert_eq!(hit, Some(Hit::Search(0)));
    assert_eq!(s.click(hit), Event::Redraw);
    assert!(s.search_focused && !s.tabs.focused);
    assert_eq!(s.open().focus(), None);
    let edit = |s: &mut PropSheet, k| s.key(form::Key::Edit(k), None);
    assert_eq!(edit(&mut s, EditKey::Insert("mo".into())), Event::Search);
    assert_eq!(s.search.text(), "mo");
    assert_eq!(
        edit(&mut s, EditKey::Backspace { word: false }),
        Event::Search
    );
    assert_eq!(s.search.text(), "m");
    assert_eq!(
        edit(
            &mut s,
            EditKey::Left {
                select: false,
                word: false
            }
        ),
        Event::Redraw,
        "no change, no search"
    );
    edit(&mut s, EditKey::SelectAll);
    assert_eq!(s.paste("power\nignored".into()), Event::Search);
    assert_eq!(s.search.text(), "power");
    assert!(matches!(s.focused_edit(), Some(form::FocusedEdit::Line(e)) if e.text == "power"));
    assert_eq!(s.wheel(Some(Hit::Search(0)), 3), Event::None);
    s.click(s.hit(170, 55, &text, 11));
    assert!(!s.search_focused, "a page control takes the keyboard back");
    s.click(s.hit(10, 15, &text, 11));
    s.focus_control("check");
    assert!(!s.search_focused);
}

/// Tab runs field, list, the page's controls, its buttons, and back to the field.
#[test]
fn tab_runs_from_the_field_through_the_list_and_page_back_to_the_field() {
    let mut s = list_sheet();
    let tab = |s: &mut PropSheet, back| s.key(form::Key::Tab { back }, None);
    s.click(Some(Hit::Search(0)));
    tab(&mut s, false);
    assert!(s.tabs.focused && !s.search_focused);
    tab(&mut s, false);
    assert_eq!(s.open().focus(), Some(0));
    tab(&mut s, false); // OK
    tab(&mut s, false); // Cancel (Apply is greyed)
    tab(&mut s, false);
    assert!(s.search_focused, "off the last button onto the field");
    tab(&mut s, true);
    let f = s.open().focus().unwrap();
    assert_eq!(
        s.open().items[f].id,
        CANCEL,
        "Shift+Tab off the field to the page's last stop"
    );
    s.click(Some(Hit::Search(0)));
    tab(&mut s, false);
    tab(&mut s, true);
    assert!(s.search_focused, "Shift+Tab off the list to the field");
    tab(&mut s, false);
    tab(&mut s, false);
    tab(&mut s, true);
    assert!(
        s.tabs.focused,
        "Shift+Tab off the page's first control to the list"
    );
}

#[test]
fn a_reader_hears_the_search_field_before_the_list() {
    use crate::access::{ROOT, Tree, accesskit::Role};
    let text = TextRenderer::load(&FontConfig::default());
    let mut s = list_sheet();
    s.click(Some(Hit::Search(0)));
    let mut tree = Tree::window("T", s.size);
    let (_, focus) = s.access(&mut tree, ROOT, &text, 11);
    let field = tree.find(Role::TextInput, "Search").expect("the field");
    assert_eq!(focus, Some(field));
    let kids = tree.get(ROOT).unwrap().children().to_vec();
    let list = tree
        .nodes()
        .find(|(_, n)| n.role() == Role::List)
        .unwrap()
        .0;
    assert!(kids.iter().position(|&k| k == field) < kids.iter().position(|&k| k == list));
}

/// FNV-1a over the pixels, for pins.
fn hash(c: &Canvas) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..c.height as i32 {
        for x in 0..c.width as i32 {
            for b in c.get(x, y).to_le_bytes() {
                h = (h ^ b as u64).wrapping_mul(0x0100_0000_01b3);
            }
        }
    }
    h
}

/// Rest: the empty field showing its cue, the page focused. Typed: "mo" in the focused field,
/// caret shown. The field's pixels are `searchfield::draw`'s at the layout's rect.
#[test]
fn the_search_field_draws_in_every_look() {
    use crate::controls::searchfield;
    use crate::interface::Interface;
    use crate::textedit::{Caret, EditKey};
    let mut hashes = Vec::new();
    for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for sc in [1, 2] {
            for typed in [false, true] {
                let theme = crate::interfaces::theme_for(i).scaled(sc);
                let text = TextRenderer::load(&theme.fonts);
                let (p, px) = (theme.paint(), theme.fonts.size_px);
                let mut s = link_sheet();
                grey(&mut s, 1);
                if typed {
                    s.click(Some(Hit::Search(0)));
                    s.key(form::Key::Edit(EditKey::Insert("mo".into())), None);
                    s.open_mut().caret = true;
                }
                let (w, h) = ((s.size.0 * sc) as u32, (s.size.1 * sc) as u32);
                let mut c = Canvas::new(w, h);
                c.fill_rect(c.bounds(), theme.colors.button_face);
                draw(&mut c, &text, &p, &s, false, px);
                let l = s.layout(&text, px);
                let field = l.search.expect("a list sheet's field");
                let mut want = Canvas::new(w, h);
                want.fill_rect(want.bounds(), theme.colors.button_face);
                let caret = Caret::new(s.search_focused, s.open().caret);
                searchfield::draw(
                    &mut want,
                    text.dialog(s.font),
                    &p,
                    &s.search,
                    field,
                    s.search_focused,
                    caret,
                    px,
                );
                let f = field.scaled(sc);
                for y in f.y..f.bottom() {
                    for x in f.x..f.right() {
                        assert_eq!(
                            c.get(x, y),
                            want.get(x, y),
                            "{i:?} x{sc} typed {typed}: the field at ({x}, {y})"
                        );
                    }
                }
                let list = l.list.unwrap().frame.scaled(sc);
                for y in f.bottom()..list.y {
                    for x in f.x..f.right() {
                        assert_eq!(
                            c.get(x, y),
                            theme.colors.button_face.argb(),
                            "{i:?} x{sc}: the gap at ({x}, {y})"
                        );
                    }
                }
                hashes.push(hash(&c));
            }
        }
    }
    if crate::text::test_tahoma() {
        assert_eq!(
            hashes,
            [
                13_734_105_841_231_725_477,
                8_540_630_767_303_374_645,
                13_749_963_363_385_055_413,
                13_938_254_622_102_580_021,
                18_344_086_528_093_623_235,
                1_027_420_804_912_379_122,
                16_027_834_627_997_073_328,
                4_276_650_307_115_636_736,
                10_754_388_277_530_216_744,
                3_888_777_649_561_378_003,
                1_263_881_998_547_729_795,
                9_342_056_638_551_148_414
            ],
            "Classic, Blue, Glass; each at 1x then 2x; each at rest then typed"
        );
    }
}

#[test]
fn ctrl_f_is_the_find_key() {
    assert!(find_key(0x66, true, false, false));
    assert!(find_key(0x46, true, false, false), "with Caps Lock");
    assert!(!find_key(0x66, false, false, false));
    assert!(!find_key(0x66, true, true, false));
    assert!(!find_key(0x66, true, false, true));
}

#[test]
fn focus_search_selects_the_query() {
    let mut s = list_sheet();
    s.search.key(EditKey::Insert("abc".into()));
    assert_eq!(s.focus_search(), Event::Redraw);
    assert!(s.search_focused && s.search.edit.all_selected());
    assert_eq!(s.open().focus(), None);
    assert_eq!(sheet(&["General"], (398, 424)).focus_search(), Event::None);
}

#[test]
fn typing_on_the_list_goes_to_the_field() {
    let mut s = list_sheet();
    s.search.key(EditKey::Insert("ab".into()));
    s.click(Some(Hit::Tab(1)));
    assert_eq!(
        s.key(form::Key::Edit(EditKey::Insert("m".into())), None),
        Event::Search
    );
    assert_eq!(s.search.text(), "abm");
    assert!(s.search_focused && !s.tabs.focused);
    s.click(Some(Hit::Tab(1)));
    assert_eq!(
        s.key(form::Key::Edit(EditKey::Backspace { word: false }), None),
        Event::Search
    );
    assert_eq!(s.search.text(), "ab");
    s.click(Some(Hit::Tab(1)));
    s.key(form::Key::Edit(EditKey::Insert(" ".into())), None);
    assert!(
        s.tabs.focused && !s.search_focused,
        "Space stays on the list"
    );
}

#[test]
fn typing_in_a_page_field_stays_in_the_field() {
    let pages = vec![
        Form::new(vec![Item::edit(
            "name",
            Rect::new(20, 50, 100, 20),
            "",
            true,
        )]),
        check_page("Effects"),
        check_page("Mouse"),
    ];
    let mut s = PropSheet::with_list(LIST_GROUPS, pages, (398, 424));
    s.focus_control("name");
    assert_eq!(
        s.key(form::Key::Edit(EditKey::Insert("x".into())), None),
        Event::Page(0, form::Event::Changed("name"))
    );
    assert_eq!(s.search.text(), "");
}

#[test]
fn shift_tab_from_the_field_with_no_page_stop_keeps_the_field() {
    let mut s = list_sheet();
    for id in ["check", OK, CANCEL, APPLY] {
        s.open_mut().set_enabled(id, false);
    }
    s.focus_search();
    s.key(form::Key::Tab { back: true }, None);
    assert!(s.search_focused);
    assert!(!s.tabs.focused);
    assert_eq!(s.open().focus(), None);
}

#[test]
fn ctrl_home_and_end_move_the_field_caret() {
    let mut s = list_sheet();
    s.focus_search();
    s.key(form::Key::Edit(EditKey::Insert("night".into())), None);
    let edge = |end, select| form::Key::DocEdge { end, select };
    assert_eq!(s.key(edge(false, false), None), Event::Redraw);
    assert_eq!(s.search.edit.caret(), 0);
    assert_eq!(s.key(edge(true, true), None), Event::Redraw);
    assert_eq!(s.search.edit.selection(), 0..5);
    assert!(s.search_focused);
}

#[test]
fn ctrl_f_closes_a_dropped_combo_list() {
    let pages = vec![
        Form::new(vec![Item::combo("pick", 20, 50, 100, &["One", "Two"], 4)]),
        check_page("Effects"),
        check_page("Mouse"),
    ];
    let mut s = PropSheet::with_list(LIST_GROUPS, pages, (398, 424));
    s.focus_control("pick");
    s.key(form::Key::Drop, None);
    assert!(s.open().popup_open());
    assert_eq!(s.focus_search(), Event::Redraw);
    assert!(!s.open().popup_open());
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);
}

#[test]
fn escape_clears_the_query_then_cancels() {
    let mut s = list_sheet();
    s.focus_search();
    s.key(form::Key::Edit(EditKey::Insert("mo".into())), None);
    assert_eq!(s.key(form::Key::Escape, None), Event::Search);
    assert_eq!(s.search.text(), "");
    assert!(s.search_focused);
    assert_eq!(
        s.key(form::Key::Escape, None),
        Event::Cancel,
        "an empty field passes it on"
    );
    s.search.key(EditKey::Insert("mo".into()));
    s.click(Some(Hit::Tab(0)));
    assert_eq!(
        s.key(form::Key::Escape, None),
        Event::Search,
        "from the list too"
    );
    assert_eq!(s.search.text(), "");
    assert!(s.tabs.focused);
}

#[test]
fn escape_on_a_page_control_clears_the_query_before_it_cancels() {
    let mut first = check_page("Night Light");
    first
        .items
        .push(Item::combo("combo", 20, 80, 100, &["A", "B"], 4));
    let pages = vec![first, check_page("Effects"), check_page("Mouse")];
    let mut s = PropSheet::with_list(LIST_GROUPS, pages, (398, 424));
    let focused = |s: &PropSheet| s.open().focus().map(|f| s.open().items[f].id);
    s.search.key(EditKey::Insert("mo".into()));
    s.focus_control("check");
    assert_eq!(s.key(form::Key::Escape, None), Event::Search);
    assert_eq!(s.search.text(), "");
    assert_eq!(focused(&s), Some("check"), "the control keeps the keyboard");
    assert_eq!(s.key(form::Key::Escape, None), Event::Cancel);

    s.search.key(EditKey::Insert("mo".into()));
    s.focus_control("combo");
    if let Control::Combo(c) = &mut s.open_mut().item_mut("combo").unwrap().control {
        c.toggle();
    }
    assert!(s.open().popup_open());
    assert_ne!(s.key(form::Key::Escape, None), Event::Search);
    assert!(!s.open().popup_open(), "an open list closes first");
    assert_eq!(s.search.text(), "mo");
}

fn filtered_link_sheet(pages: [bool; 3], links: [bool; 2]) -> PropSheet {
    let mut s = link_sheet();
    s.list_mut().unwrap().set_filter(Some(&pagelist::Filter {
        pages: pages.to_vec(),
        links: links.to_vec(),
    }));
    s
}

#[test]
fn ctrl_tab_turns_only_to_the_pages_the_filter_shows() {
    let ctrl_tab = |s: &mut PropSheet| s.key(form::Key::Tab { back: false }, Some(false));
    let mut s = filtered_link_sheet([true, false, true], [true; 2]);
    assert_eq!(ctrl_tab(&mut s), Event::Redraw);
    assert_eq!(s.tabs.open, 2);
    ctrl_tab(&mut s);
    assert_eq!(s.tabs.open, 0, "wraps past the hidden page");
    let mut s = filtered_link_sheet([true, false, false], [true; 2]);
    assert_eq!(ctrl_tab(&mut s), Event::None, "no other page shows");
}

#[test]
fn focusing_a_missing_or_greyed_control_leaves_the_keyboard_where_it_is() {
    let mut s = list_sheet();
    s.focus_list();
    s.pages[0].set_enabled("check", false);
    assert!(!s.focus_control("check"));
    assert!(!s.focus_control("missing"));
    assert_eq!((s.tabs.focused, s.open().focus()), (true, None));
    s.pages[0].set_enabled("check", true);
    assert!(s.focus_control("check"));
    assert_eq!((s.tabs.focused, s.open().focus()), (false, Some(0)));
}

#[test]
fn a_page_the_host_opens_starts_scrolled_home_and_the_old_page_lets_go() {
    let theme = Theme::default().with_form_text(200);
    let page = || {
        Form::new(vec![
            Item::check("top", 20, 50, "Top"),
            Item::check("low", 20, 350, "Low"),
        ])
    };
    let mut s = PropSheet::new(&["One", "Two"], vec![page(), page()], (398, 424));
    s.drawn_with(&theme.paint());
    s.fit((398, 300));
    assert_eq!(s.wheel(None, 100), Event::Redraw);
    let text = TextRenderer::load(&FontConfig::default());
    let low = s.open().rect("low").unwrap();
    let hit = s.hit(
        low.x + 4,
        low.y + low.height / 2,
        &text,
        theme.fonts.size_px,
    );
    s.click(hit);
    assert!(s.open().holding());
    s.set_open(1);
    assert_eq!(s.page_offset(), (0, 0));
    assert!(!s.pages[0].holding());
    s.wheel(None, 100);
    s.set_open(1);
    assert_ne!(s.page_offset(), (0, 0), "the open page keeps its place");
}

#[test]
fn a_host_opening_a_page_or_taking_the_keyboard_lets_go_of_the_link_cursor() {
    let mut s = link_sheet();
    s.click(Some(Hit::Link(0)));
    s.set_open(2);
    assert_eq!((s.tabs.open, s.tabs.focused, cursor(&s)), (2, true, None));
    s.click(Some(Hit::Link(0)));
    s.blur();
    assert_eq!(
        (
            s.tabs.focused,
            s.search_focused,
            cursor(&s),
            s.open().focus()
        ),
        (false, false, None, None)
    );
    s.focus_list();
    assert_eq!((s.tabs.open, s.tabs.focused), (2, true));

    let mut s = filtered_link_sheet([false, true, false], [false; 2]);
    s.blur();
    s.focus_list();
    assert_eq!(
        (s.tabs.open, s.tabs.focused),
        (1, true),
        "the list lands on the first shown row"
    );
}

#[test]
fn enter_in_the_field_opens_the_first_row_the_filter_shows() {
    let filtered = |pages: [bool; 3], links: [bool; 2]| {
        let mut s = filtered_link_sheet(pages, links);
        s.focus_search();
        s.key(form::Key::Edit(EditKey::Insert("x".into())), None);
        s
    };
    let mut s = filtered([false, false, true], [true, false]);
    assert_eq!(
        s.key(form::Key::Enter, None),
        Event::Link(0),
        "Bluetooth's row comes first"
    );
    assert!(s.search_focused);
    let mut s = filtered([false, false, true], [false, false]);
    assert_eq!(s.key(form::Key::Enter, None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, s.search_focused),
        (2, true, false)
    );
    let mut s = filtered([false; 3], [false; 2]);
    assert_eq!(
        s.key(form::Key::Enter, None),
        Event::None,
        "nothing matches"
    );
    let mut s = link_sheet();
    s.focus_search();
    assert_eq!(s.key(form::Key::Enter, None), Event::None, "an empty field");
}

#[test]
fn down_in_the_field_steps_onto_the_first_shown_row() {
    let mut s = filtered_link_sheet([false, true, false], [false, false]);
    s.focus_search();
    assert_eq!(s.key(form::Key::Down, None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, s.search_focused),
        (1, true, false)
    );
    let mut s = filtered_link_sheet([false; 3], [false; 2]);
    s.focus_search();
    assert_eq!(s.key(form::Key::Down, None), Event::None, "nothing shows");
    assert!(s.search_focused);
}

/// With the open page's row hidden, the list taking the keyboard would show no row selected.
#[test]
fn the_list_takes_the_keyboard_on_the_first_shown_row_when_the_open_page_hides() {
    let tab = form::Key::Tab { back: false };
    let mut s = filtered_link_sheet([false, false, true], [false, false]);
    s.focus_search();
    assert_eq!(s.key(tab.clone(), None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, s.search_focused, cursor(&s)),
        (2, true, false, None)
    );

    let mut s = filtered_link_sheet([false, false, true], [true, false]);
    s.focus_search();
    assert_eq!(s.key(tab.clone(), None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, cursor(&s)),
        (0, true, Some(0)),
        "Bluetooth's link"
    );
    let mut s = filtered_link_sheet([false, false, true], [true, false]);
    s.focus_search();
    assert_eq!(s.key(form::Key::Down, None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, cursor(&s)),
        (0, true, Some(0)),
        "Down too"
    );

    let mut s = filtered_link_sheet([false, false, true], [false, false]);
    s.focus_control("check");
    assert_eq!(s.key(form::Key::Tab { back: true }, None), Event::Redraw);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, cursor(&s)),
        (2, true, None),
        "Shift+Tab off the page"
    );

    let mut s = filtered_link_sheet([true, false, true], [true, false]);
    s.focus_search();
    s.key(tab, None);
    assert_eq!(
        (s.tabs.open, s.tabs.focused, cursor(&s)),
        (0, true, None),
        "a shown open page keeps its row"
    );
}
