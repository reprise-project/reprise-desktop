use super::*;
use crate::controls::form::{Control, Form};
use crate::interface::Interface;
use crate::interfaces;
use crate::raster::Canvas;
use crate::theme::{FontConfig, Theme};

fn sample() -> CharMap {
    let chars = ('\0'..='\u{7f}')
        .chain('\u{a0}'..='\u{17f}')
        .chain(['€', '₹', '→', '←', '★', '\u{e000}', '\u{378}']);
    CharMap::new(chars)
}

#[test]
fn the_characters_group_by_block_in_code_point_order() {
    let map = sample();
    let names: Vec<&str> = map.blocks.iter().map(|b| b.name).collect();
    assert_eq!(
        names,
        [
            "Basic Latin",
            "Latin-1 Supplement",
            "Latin Extended-A",
            "Currency Symbols",
            "Arrows",
            "Miscellaneous Symbols",
            "Private Use Area"
        ]
    );
    assert!(!map.chars.contains(&'\n'), "controls are left out");
    assert!(
        !map.chars.contains(&'\u{378}'),
        "unassigned code points are left out"
    );
    assert_eq!(map.shown.len(), map.chars.len(), "no block shows them all");
}

#[test]
fn a_block_shows_only_its_characters() {
    let mut map = sample();
    let arrows = map.blocks.iter().position(|b| b.name == "Arrows");
    map.set_block(arrows);
    assert_eq!(map.shown, ['←', '→']);
    assert_eq!(map.picked(), Some('←'), "the first is picked");
    map.set_block(None);
    assert_eq!(
        map.picked(),
        Some('←'),
        "the pick stays when it is still shown"
    );
}

#[test]
fn search_finds_names_codes_and_the_character_itself() {
    let mut map = sample();
    map.set_query("arrow");
    assert_eq!(map.shown, ['←', '→']);
    map.set_query("left arrow");
    assert_eq!(map.shown, ['←'], "every word must be in the name");
    map.set_query("U+20AC");
    assert_eq!(map.shown, ['€']);
    map.set_query("20b9");
    assert_eq!(map.shown, ['₹']);
    map.set_query("é");
    assert_eq!(
        map.shown,
        ['É', 'é'],
        "one character finds itself in either case"
    );
    map.set_query("rupee");
    assert_eq!(map.shown, ['₹']);
    map.set_query("nothing like this");
    assert_eq!((map.shown.len(), map.picked()), (0, None));
    let arrows = map.blocks.iter().position(|b| b.name == "Arrows");
    map.set_block(arrows);
    map.set_query("star");
    assert!(map.shown.is_empty(), "search stays inside the block");
}

#[test]
fn names_and_codes() {
    assert_eq!(name('a').as_deref(), Some("LATIN SMALL LETTER A"));
    assert_eq!(name('€').as_deref(), Some("EURO SIGN"));
    assert_eq!(code('é'), "U+00E9");
    assert_eq!(code('😀'), "U+1F600");
}

#[test]
fn arrows_move_the_pick_and_scroll_it_into_view() {
    let mut map = sample();
    assert_eq!(map.selected, Some(0));
    assert!(!map.step(Step::Left, 5), "stops at the start");
    assert!(map.step(Step::Right, 5));
    assert!(map.step(Step::Down, 5));
    assert_eq!(map.selected, Some(1 + COLS));
    map.step(Step::Up, 5);
    assert_eq!(map.selected, Some(1));
    assert!(!map.step(Step::Up, 5), "the top row stays");
    map.step(Step::PageDown, 5);
    assert_eq!(map.selected, Some(1 + 5 * COLS));
    assert_eq!(map.top, 1, "the pick's row came into view");
    map.step(Step::End, 5);
    assert_eq!(map.selected, Some(map.shown.len() - 1));
    assert_eq!(map.top, map.rows() - 5);
    map.step(Step::Home, 5);
    assert_eq!((map.selected, map.top), (Some(0), 0));
}

#[test]
fn a_new_font_keeps_the_block_and_pick_it_has() {
    let mut map = sample();
    let arrows = map.blocks.iter().position(|b| b.name == "Arrows");
    map.set_block(arrows);
    map.pick_char('→');
    map.set_chars(['a', '←', '→', '↑']);
    assert_eq!(map.block, Some(1));
    assert_eq!(map.picked(), Some('→'));
    map.set_chars(['a', 'b']);
    assert_eq!(
        (map.block, map.picked()),
        (None, Some('a')),
        "a block it lacks shows all"
    );
}

#[test]
fn a_cell_hit_is_the_shown_character_under_it() {
    let map = sample();
    let cm = ControlMetrics::CLASSIC;
    let lay = layout(Rect::new(0, 0, 400, 200), &cm);
    let r = lay.cell_rect(&map, 3).expect("in view");
    assert_eq!(hit(&map, &lay, &cm, r.x + 2, r.y + 2), Some(Hit::Cell(3)));
    let r = lay.cell_rect(&map, COLS + 1).expect("in view");
    assert_eq!(
        hit(&map, &lay, &cm, r.x + 2, r.y + 2),
        Some(Hit::Cell(COLS + 1))
    );
    assert!(matches!(
        hit(&map, &lay, &cm, lay.bar.x + 2, lay.bar.y + 2),
        Some(Hit::Bar(_))
    ));
    assert!(
        lay.cell_rect(&map, lay.rows * COLS).is_none(),
        "below the view"
    );
}

#[test]
fn select_appends_to_the_characters_to_copy() {
    let map = sample();
    let mut f = form(&["Sans".into()], 0, &map);
    append(&mut f, 'é');
    append(&mut f, '€');
    assert_eq!(f.edit_text(TO_COPY), "é€");
}

#[test]
fn the_form_follows_the_blocks_and_the_pick() {
    let mut map = sample();
    let mut f = form(&["Sans".into(), "Serif".into()], 1, &map);
    let combo = |f: &Form, id| match &f.item(id).expect("combo").control {
        Control::Combo(c) => (c.items.len(), c.selected),
        _ => unreachable!(),
    };
    assert_eq!(combo(&f, FONT), (2, 1));
    assert_eq!(
        combo(&f, BLOCK),
        (map.blocks.len() + 1, 0),
        "All blocks first"
    );
    map.set_block(Some(3));
    map.pick_char('€');
    follow(&mut f, &map);
    assert_eq!(combo(&f, BLOCK).1, 4);
    assert_eq!(picked_block(&f), Some(3));
    let label = |f: &Form, id| match &f.item(id).expect("label").control {
        Control::Label { text, .. } => text.clone(),
        _ => unreachable!(),
    };
    assert_eq!(label(&f, NAME), "EURO SIGN");
    assert_eq!(label(&f, CODE), "U+20AC");
}

#[test]
fn the_window_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        vec![(
            "Character Map".into(),
            form(&["DejaVu Sans".into()], 0, &sample()),
        )]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

fn fnv(c: &Canvas) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for v in [c.width, c.height]
        .into_iter()
        .chain(c.pixels.iter().copied())
    {
        for b in v.to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

fn looks() -> Vec<(String, Theme)> {
    let mut out = vec![
        ("Classic".to_string(), Theme::default()),
        (
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ),
    ];
    for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
        let g = crate::glass::Settings {
            mode,
            ..Default::default()
        };
        let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
        out.push((format!("Glass {mode:?}"), t));
    }
    out
}

fn drawn(theme: &Theme, f: &Form, map: &CharMap, focused: bool, s: i32) -> Canvas {
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    draw_window(
        &mut c,
        &text,
        &text,
        &p,
        f,
        map,
        focused,
        theme.fonts.size_px,
    );
    c
}

/// Drawn in the UI font, which the pins' test fonts fix; the program draws the grid in the
/// font picked.
#[test]
fn character_map_pins_at_scale_1_and_2() {
    if !crate::text::test_tahoma() {
        return;
    }
    let fonts = vec![FontConfig::default().family[0].clone()];
    let mut all = sample();
    all.pick_char('A');
    let mut f_all = form(&fonts, 0, &all);
    append(&mut f_all, 'é');
    let mut search = sample();
    search.set_query("arrow");
    let f_search = form(&fonts, 0, &search);
    let mut got = Vec::new();
    for (name, theme) in looks() {
        for (state, f, map, focused) in [
            ("all, grid focused", &f_all, &all, true),
            ("searched", &f_search, &search, false),
        ] {
            for s in [1, 2] {
                got.push((
                    format!("{name} {state} {s}"),
                    fnv(&drawn(&theme, f, map, focused, s)),
                ));
            }
        }
    }
    crate::pins::check("charmap", &got);
}
