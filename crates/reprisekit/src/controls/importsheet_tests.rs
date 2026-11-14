use super::*;
use crate::controls::form::{Control, Key};
use crate::interface::Interface;
use crate::interfaces;
use crate::theme::FontConfig;

fn line(what: &str, now: &str, new: &str) -> Line {
    Line {
        what: what.into(),
        now: now.into(),
        new: new.into(),
        greyed: false,
    }
}

fn greyed(what: &str, why: &str, found: &str) -> Line {
    Line {
        greyed: true,
        ..line(what, why, found)
    }
}

fn gnome() -> Source {
    Source {
        name: "GNOME".into(),
        lines: vec![
            line("Wallpaper", "None", "/home/sam/Pictures/fjord.jpg"),
            line("Dark style", "Off", "On"),
            line("Keyboard layouts", "us", "us, de (nodeadkeys)"),
            line("Text size", "100%", "125%"),
            greyed("Clock", "follows the formats locale", "24-hour"),
        ],
    }
}

fn plasma() -> Source {
    Source {
        name: "Plasma".into(),
        lines: vec![line("Tap to click", "On", "Off")],
    }
}

fn env() -> (TextRenderer, Theme) {
    (TextRenderer::load(&FontConfig::default()), Theme::default())
}

fn button(s: &Sheet, id: &str) -> Option<(bool, bool)> {
    match s.form.item(id).map(|i| &i.control) {
        Some(Control::Button {
            enabled, default, ..
        }) => Some((*enabled, *default)),
        _ => None,
    }
}

fn lines(s: &Sheet, id: &str) -> String {
    match s.form.item(id).map(|i| &i.control) {
        Some(Control::Lines(l)) => l.join(" "),
        other => panic!("{id} is {other:?}"),
    }
}

fn key(s: &mut Sheet, k: Key, text: &TextRenderer, theme: &Theme) -> Option<Outcome> {
    s.list_key(&k, text, theme)
}

#[test]
fn every_change_starts_checked_and_greyed_rows_never_count() {
    let (text, theme) = env();
    let s = Sheet::new(vec![gnome()], &text, &theme);
    assert_eq!(s.checked(), vec![0, 1, 2, 3]);
    assert_eq!(s.list.checks, Some(vec![true, true, true, true, false]));
    let groups: Vec<_> = s.list.groups.iter().map(|g| g.first).collect();
    assert_eq!(groups, vec![0, 4], "changes, then what has no setting");
    assert_eq!(button(&s, IMPORT), Some((true, true)));
}

#[test]
fn space_flips_a_change_but_not_a_greyed_row() {
    let (text, theme) = env();
    let mut s = Sheet::new(vec![gnome()], &text, &theme);
    key(&mut s, Key::Space, &text, &theme);
    assert_eq!(s.checked(), vec![1, 2, 3]);
    key(&mut s, Key::End, &text, &theme);
    assert_eq!(s.list.selection.focus, Some(4));
    assert_eq!(key(&mut s, Key::Space, &text, &theme), Some(Outcome::None));
    assert!(!s.list.checks.as_ref().unwrap()[4]);
    assert!(lines(&s, ABOUT).contains("24-hour (follows the formats locale)"));
}

#[test]
fn with_nothing_checked_import_greys_and_enter_cancels() {
    let (text, theme) = env();
    let mut s = Sheet::new(vec![plasma()], &text, &theme);
    key(&mut s, Key::Space, &text, &theme);
    assert!(s.checked().is_empty());
    assert_eq!(button(&s, IMPORT), Some((false, false)));
    assert_eq!(button(&s, CANCEL), Some((true, true)));
    assert_eq!(s.event(form::Event::Default, &text, &theme), Outcome::Close);
    key(&mut s, Key::Space, &text, &theme);
    assert_eq!(
        s.event(form::Event::Default, &text, &theme),
        Outcome::Import
    );
}

#[test]
fn the_source_box_swaps_the_rows() {
    let (text, theme) = env();
    let mut s = Sheet::new(vec![gnome(), plasma()], &text, &theme);
    assert!(lines(&s, HEAD).contains("GNOME"));
    s.form.set_combo(SOURCE, 1);
    s.event(form::Event::Changed(SOURCE), &text, &theme);
    assert_eq!(s.source, 1);
    assert_eq!(s.list.rows, plasma().lines);
    assert!(lines(&s, HEAD).contains("Plasma"));
    let one = Sheet::new(vec![plasma()], &text, &theme);
    assert!(one.form.item(SOURCE).is_none(), "one source needs no box");
}

#[test]
fn an_open_source_box_takes_the_pointer_over_the_list() {
    let (text, theme) = env();
    let mut s = Sheet::new(vec![gnome(), plasma()], &text, &theme);
    let inside = (LIST.x + 40, LIST.y + 60);
    let press = listview::Input::Press(inside.0, inside.1);
    s.form.focus_id(SOURCE);
    s.form.key(Key::Drop);
    assert!(s.form.popup_open());
    assert_eq!(s.list_pointer(press, &text, &theme), None);
    s.form.key(Key::Escape);
    assert!(s.list_pointer(press, &text, &theme).is_some());
}

#[test]
fn done_says_what_changed_and_leaves_close() {
    let (text, theme) = env();
    let mut s = Sheet::new(vec![gnome(), plasma()], &text, &theme);
    s.done("Imported 4 GNOME settings.".into(), true, &text, &theme);
    assert_eq!(lines(&s, HEAD), "Imported 4 GNOME settings.");
    assert_eq!(button(&s, IMPORT), None);
    assert_eq!(button(&s, CLOSE), Some((true, true)));
    assert!(s.form.item(SOURCE).is_none());
    assert_eq!(
        key(&mut s, Key::Tab { back: false }, &text, &theme),
        Some(Outcome::Redraw)
    );
    assert_eq!(s.list.rows, gnome().lines[..4], "the rows written stay");
    assert_eq!(s.list.checks, None);
    let mut failed = Sheet::new(vec![gnome()], &text, &theme);
    failed.done(
        "Reprise couldn't import the settings.".into(),
        false,
        &text,
        &theme,
    );
    assert_eq!(
        failed.list.rows,
        gnome().lines,
        "a failed write keeps the list"
    );
    let none = Sheet::new(Vec::new(), &text, &theme);
    assert!(lines(&none, HEAD).contains("found no GNOME or Plasma settings"));
    assert_eq!(button(&none, CLOSE), Some((true, true)));
}

#[test]
fn a_greyed_row_reads_as_disabled() {
    let (text, theme) = env();
    let s = Sheet::new(vec![gnome()], &text, &theme);
    let mut tree = Tree::window("", CLIENT);
    s.access(&mut tree, &text, &theme);
    let options: Vec<_> = tree
        .nodes()
        .filter(|(_, n)| n.role() == crate::access::accesskit::Role::ListBoxOption)
        .map(|(_, n)| (n.label().unwrap_or_default().to_string(), n.is_disabled()))
        .collect();
    assert_eq!(options.len(), 5);
    assert_eq!(options[4], ("Clock".into(), true));
    assert!(!options[0].1);
}

#[test]
fn the_sheet_fits_its_text() {
    let over = interfaces::form_overflows(|_, forms, text| {
        let mut s = Sheet::new(vec![gnome(), plasma()], text, forms);
        let asking = s.form.clone();
        s.done("Imported 4 GNOME settings.".into(), true, text, forms);
        vec![("asking".into(), asking), ("done".into(), s.form)]
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

#[test]
fn the_sheet_pins_its_pixels_in_every_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, i) in [
        ("Classic", Interface::Classic),
        ("Blue", Interface::Blue),
        ("Glass", Interface::Glass),
    ] {
        for scale in [1, 2] {
            let theme = interfaces::theme_for(i).scaled(scale);
            let text = TextRenderer::load(&theme.fonts);
            let px = theme.fonts.size_px;
            let mut asking = Sheet::new(vec![gnome(), plasma()], &text, &theme);
            key(&mut asking, Key::Down, &text, &theme);
            key(&mut asking, Key::Space, &text, &theme);
            let mut done = Sheet::new(vec![plasma()], &text, &theme);
            done.done("Imported 1 Plasma setting.".into(), true, &text, &theme);
            let none = Sheet::new(Vec::new(), &text, &theme);
            for (name, s) in [("asking", asking), ("done", done), ("none", none)] {
                let (w, h) = Sheet::size(TextUnits::ONE);
                let mut c = Canvas::new((w * scale) as u32, (h * scale) as u32);
                draw(&mut c, &text, &theme.paint(), &s, px);
                got.push((format!("{look} {name} {scale}"), fnv(&c)));
            }
        }
    }
    crate::pins::check("importsheet", &got);
}
