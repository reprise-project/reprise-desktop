use super::*;

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
    crate::pins::fonts();
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
