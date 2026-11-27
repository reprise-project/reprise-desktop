use super::*;
use crate::controls::fingerprints::tests::{hash_form, looks};
use crate::controls::form::Key;
use crate::interfaces;

fn dests() -> Vec<Dest> {
    vec![
        Dest {
            name: "Laser".into(),
            info: "Office laser".into(),
            location: "Second floor".into(),
            state: DestState::Ready,
        },
        Dest {
            name: "PDF".into(),
            info: "Virtual PDF Printer".into(),
            location: String::new(),
            state: DestState::Stopped("Out of paper".into()),
        },
    ]
}

fn states() -> Vec<(&'static str, Form)> {
    let d = dests();
    vec![
        ("two printers", form(Ok(&d), 1, 12)),
        ("one page", form(Ok(&d), 0, 1)),
        ("not running", form(Err(NoDest::NotRunning), 0, 3)),
        ("looking", form(Err(NoDest::Looking), 0, 1)),
    ]
}

fn label(f: &Form, id: &str) -> String {
    match f.item(id).map(|i| &i.control) {
        Some(Control::Label { text, .. }) => text.clone(),
        _ => String::new(),
    }
}

#[test]
fn the_dialog_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        states()
            .into_iter()
            .map(|(n, f)| (n.to_string(), f))
            .collect()
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
fn the_dialog_pins_its_pixels_in_every_state_and_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (state, f) in states() {
            for s in [1, 2] {
                got.push((
                    format!("{look} {state} {s}"),
                    hash_form(&base, &f, CLIENT, s),
                ));
            }
        }
    }
    crate::pins::check("controls/printdlg", &got);
}

#[test]
fn the_rows_under_the_name_follow_the_printer_picked() {
    let d = dests();
    let mut f = form(Ok(&d), 0, 3);
    assert_eq!(label(&f, STATUS), "Ready");
    assert_eq!(label(&f, WHERE), "Second floor");
    f.set_combo(PRINTER, 1);
    follow(&mut f, &d);
    assert_eq!(label(&f, STATUS), "Paused: Out of paper");
    assert_eq!(label(&f, COMMENT), "Virtual PDF Printer");
}

#[test]
fn print_reads_all_pages_and_the_copies() {
    let d = dests();
    let mut f = form(Ok(&d), 1, 5);
    f.focus_id(COPIES);
    f.key(Key::Up);
    f.key(Key::Up);
    assert_eq!(
        read(&f, 5),
        Ok(Choice {
            dest: 1,
            copies: 3,
            range: Range::All
        })
    );
    assert_eq!(Range::All.of(5), 0..5);
}

#[test]
fn typing_a_page_picks_pages_and_a_range_past_the_end_is_refused() {
    let d = dests();
    let mut f = form(Ok(&d), 0, 5);
    f.focus_id(FROM);
    f.select_edit_text(FROM);
    f.key(Key::Char('2'));
    changed(&mut f, &d, FROM);
    assert_eq!(f.checked(PAGES), Some(true));
    assert_eq!(f.checked(ALL), Some(false));
    assert_eq!(read(&f, 5).map(|c| c.range), Ok(Range::Pages(2, 5)));
    assert_eq!(Range::Pages(2, 5).of(5), 1..5);
    f.focus_id(TO);
    f.select_edit_text(TO);
    f.key(Key::Char('9'));
    assert_eq!(read(&f, 5), Err(Refusal::BadRange { pages: 5 }));
}

#[test]
fn with_no_printer_print_is_greyed_and_the_box_says_why() {
    let f = form(Err(NoDest::NotRunning), 0, 3);
    assert!(matches!(
        f.item(PRINT).map(|i| &i.control),
        Some(Control::Button { enabled: false, .. })
    ));
    assert!(matches!(
        f.item(PRINTER).map(|i| &i.control),
        Some(Control::Combo(c)) if !c.enabled && c.items.len() == 1
    ));
}

#[test]
fn a_one_page_document_greys_pages_and_its_boxes() {
    let d = dests();
    let f = form(Ok(&d), 0, 1);
    for id in [FROM, TO] {
        assert!(
            matches!(
                f.item(id).map(|i| &i.control),
                Some(Control::Edit { enabled: false, .. })
            ),
            "{id} is greyed"
        );
    }
    assert!(matches!(
        f.item(PAGES).map(|i| &i.control),
        Some(Control::Radio { enabled: false, .. })
    ));
}

#[test]
fn more_pages_than_one_job_prints_are_refused_and_a_range_of_them_goes() {
    let d = dests();
    let many = MAX_PAGES + 20;
    let mut f = form(Ok(&d), 0, many);
    assert_eq!(read(&f, many), Err(Refusal::TooMany));
    f.focus_id(FROM);
    f.select_edit_text(FROM);
    for k in "21".chars() {
        f.key(Key::Char(k));
    }
    changed(&mut f, &d, FROM);
    assert_eq!(
        read(&f, many).map(|c| c.range),
        Ok(Range::Pages(21, many)),
        "exactly MAX_PAGES pages"
    );
    assert_eq!(
        Range::Pages(0, 3).of(5),
        0..3,
        "a page 0 reads as the first"
    );
}
