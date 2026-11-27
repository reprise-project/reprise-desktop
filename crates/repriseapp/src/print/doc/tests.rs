use super::*;
use reprisekit::controls::pagesetup::{MM, PAPERS};
use std::cell::Cell;

thread_local! {
    /// Raw pages alive now, and the most alive at once.
    pub(super) static LIVE: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

fn a4(margin: i32) -> PageSetup {
    PageSetup {
        margins: [margin; 4],
        ..PageSetup::on(&PAPERS[4])
    }
}

fn mono(pt: u32) -> TextFont {
    TextFont {
        family: String::new(),
        bold: false,
        italic: false,
        pt,
    }
}

#[test]
fn a_long_text_is_drawn_one_raw_page_at_a_time() {
    let s = PageSetup {
        margins: [92 * MM, 93 * MM, 136 * MM, 136 * MM],
        ..PageSetup::on(&PAPERS[4])
    };
    assert!(s.fits(), "a 25 mm square body");
    let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
    let laid = Laid::new(
        Doc::Text {
            font: mono(10),
            lines,
        },
        s,
    );
    assert!(laid.pages() > 20, "{} pages", laid.pages());
    LIVE.with(|l| l.set((0, 0)));
    let mut w = pdf::Writer::new(laid.pages() as usize);
    let mut n = 0;
    laid.render(Range::All, |p| {
        n += 1;
        w.page(&p);
    });
    assert_eq!(n, laid.pages());
    assert_eq!(
        LIVE.with(Cell::get),
        (0, 1),
        "one raw page at most, none left"
    );
    assert!(w.finish().starts_with(b"%PDF-1.4"));
}

#[test]
fn a_text_wraps_between_the_margins_and_a_range_draws_only_its_pages() {
    let s = a4(25 * MM);
    assert_eq!(body_px(&s), (1890, 2917), "160 x 247 mm at 300 dpi");
    let per_page = (2917 / (12 * DPI / 72)) as usize;
    let lines = vec!["x".to_string(); per_page + 1];
    let laid = Laid::new(
        Doc::Text {
            font: mono(12),
            lines,
        },
        s,
    );
    assert_eq!(laid.pages(), 2);
    let mut pages = Vec::new();
    laid.render(Range::Pages(2, 2), |p| pages.push(p));
    assert_eq!(pages.len(), 1);
    let p = &pages[0];
    assert_eq!((p.image.width, p.image.height), (1890, 2917));
    assert!((p.size.0 - 595.28).abs() < 0.01 && (p.size.1 - 841.89).abs() < 0.01);
    assert!((p.at[0] - 70.866).abs() < 0.01, "the body starts 25 mm in");
}

#[test]
fn a_large_picture_is_drawn_at_300_dpi_of_its_place_and_a_small_one_at_its_own_size() {
    let s = a4(25 * MM);
    assert_eq!(picture_size(&s, (8000, 2000)), (1890, 472));
    assert_eq!(picture_size(&s, (40, 10)), (40, 10));
    let laid = Laid::new(
        Doc::Picture {
            picture: Arc::new(Canvas::new(4000, 1000)),
            turns: 1,
        },
        s,
    );
    let mut got = Vec::new();
    laid.render(Range::All, |p| got.push(p));
    let p = &got[0];
    assert_eq!(
        (p.image.width, p.image.height),
        (729, 2917),
        "turned on its side, fitted to the height"
    );
    let [x, _, w, h] = p.at;
    assert!((h - points(247 * MM)).abs() < 0.01);
    assert!(
        (x + w / 2.0 - points(21000) / 2.0).abs() < 0.1,
        "centred across"
    );
}
