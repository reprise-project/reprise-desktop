use super::*;
use crate::controls::form::{Control, Key};
use crate::interface::Interface;
use crate::raster::Canvas;
use crate::text::TextRenderer;
use crate::theme::Theme;

/// Focuses `id` on the open page and presses `key` there.
fn press(s: &mut FolderSheet, id: &'static str, key: Key) -> Outcome {
    s.sheet.open_mut().focus_id(id);
    let ev = s.sheet.key(key, None);
    s.event(ev)
}

fn apply_on(s: &FolderSheet) -> bool {
    match s.sheet.open().item("apply").map(|i| &i.control) {
        Some(Control::Button { enabled, .. }) => *enabled,
        _ => panic!("Apply is on the page"),
    }
}

#[test]
fn every_page_fits_its_text_in_every_look_and_size() {
    let over = crate::interfaces::form_overflows(|_, _, _| {
        let s = FolderSheet::new(Settings::default());
        vec![
            ("general".to_string(), s.page_general()),
            ("view".to_string(), s.page_view()),
            ("types".to_string(), s.page_types()),
        ]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
fn the_boxes_read_into_the_settings_and_apply_greys_once_saved() {
    let mut s = FolderSheet::new(Settings::default());
    assert!(!apply_on(&s));
    press(&mut s, "single", Key::Space);
    press(&mut s, "own", Key::Space);
    assert!(s.pending.single_click && s.pending.new_window);
    assert!(apply_on(&s));
    s.sheet.open_tab(1);
    press(&mut s, "show", Key::Space);
    press(&mut s, "extensions", Key::Space);
    press(&mut s, "thumbs", Key::Space);
    press(&mut s, "tips", Key::Space);
    let want = Settings {
        new_window: true,
        single_click: true,
        show_hidden: true,
        hide_extensions: false,
        tips: false,
        cache_thumbnails: false,
    };
    assert_eq!(s.pending, want);
    assert_eq!(
        s.event(propsheet::Event::Apply),
        Outcome::Save {
            settings: want,
            close: false
        }
    );
    assert!(!apply_on(&s));
    assert_eq!(s.sheet.tabs.open, 1, "Apply keeps the page");
    assert_eq!(s.sheet.open().checked("show"), Some(true));
}

#[test]
fn restore_defaults_puts_back_only_its_own_pages_boxes() {
    let all = Settings {
        new_window: true,
        single_click: true,
        show_hidden: true,
        hide_extensions: false,
        tips: false,
        cache_thumbnails: false,
    };
    let mut s = FolderSheet::new(all);
    assert_eq!(press(&mut s, DEFAULTS, Key::Space), Outcome::Redraw);
    assert_eq!(
        s.pending,
        Settings {
            new_window: false,
            single_click: false,
            ..all
        }
    );
    assert_eq!(s.sheet.open().checked("double"), Some(true));
    assert!(apply_on(&s));
    let mut s = FolderSheet::new(all);
    s.sheet.open_tab(1);
    press(&mut s, DEFAULTS, Key::Space);
    assert_eq!(
        s.pending,
        Settings {
            new_window: true,
            single_click: true,
            ..Settings::default()
        }
    );
    assert_eq!(s.sheet.open().checked("hide"), Some(true));
}

#[test]
fn file_types_button_asks_for_default_programs() {
    let mut s = FolderSheet::new(Settings::default());
    s.sheet.open_tab(2);
    assert_eq!(
        press(&mut s, PROGRAMS, Key::Space),
        Outcome::DefaultPrograms
    );
    assert_eq!(s.event(propsheet::Event::Cancel), Outcome::Close);
}

#[test]
fn a_save_elsewhere_shows_unless_the_boxes_were_changed_here() {
    let mut s = FolderSheet::new(Settings::default());
    let hidden = Settings {
        show_hidden: true,
        ..Settings::default()
    };
    s.saved_elsewhere(hidden);
    assert_eq!(s.pending, hidden);
    assert!(!apply_on(&s));
    press(&mut s, "single", Key::Space);
    s.saved_elsewhere(Settings::default());
    assert!(
        s.pending.single_click && s.pending.show_hidden,
        "the boxes stay"
    );
    assert!(apply_on(&s));
}

/// Each tab as it opens with the defaults, and General and View with every box flipped, in each
/// look at scale 1 and 2.
#[test]
fn the_sheet_draws_its_pins() {
    let flipped = Settings {
        new_window: true,
        single_click: true,
        show_hidden: true,
        hide_extensions: false,
        tips: false,
        cache_thumbnails: false,
    };
    let states = [
        ("general", Settings::default(), 0),
        ("view", Settings::default(), 1),
        ("types", Settings::default(), 2),
        ("general_flipped", flipped, 0),
        ("view_flipped", flipped, 1),
    ];
    let mut got = Vec::new();
    for look in [Interface::Classic, Interface::Blue, Interface::Glass] {
        for (name, settings, tab) in states {
            for scale in [1, 2] {
                let theme = Theme {
                    access_keys: true,
                    ..crate::interfaces::theme_for(look).scaled(scale)
                };
                let text = TextRenderer::load(&theme.fonts);
                let p = theme.paint();
                let mut s = FolderSheet::new(settings);
                s.sheet.open_tab(tab);
                let (w, h) = s.sheet.size;
                let mut c = Canvas::new((w * scale) as u32, (h * scale) as u32);
                c.fill_rect(c.bounds(), p.cs.button_face);
                propsheet::draw(&mut c, &text, &p, &s.sheet, true, theme.fonts.size_px);
                got.push((format!("{look:?} {name} {scale}"), crate::pins::fnv(&c)));
            }
        }
    }
    crate::pins::check("folderopts", &got);
}
