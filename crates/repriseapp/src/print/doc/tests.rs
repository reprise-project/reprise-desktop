use super::*;
use reprisekit::controls::pagesetup::{MM, PAPERS, Scaling};
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
        "notes.txt",
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
        "notes.txt",
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
fn a_header_or_footer_prints_the_strip_between_the_side_margins_top_to_bottom() {
    let s = PageSetup {
        footer: "Page &p".into(),
        ..a4(25 * MM)
    };
    assert_eq!(
        frame_px(&s),
        Frame {
            width: 1890,
            height: 3508,
            top: 295,
            bottom: 3508 - 295 - 2917,
        },
        "297 mm tall, the text's 247 mm 25 mm down"
    );
    let laid = Laid::new(
        Doc::Text {
            font: mono(12),
            lines: vec!["x".into()],
        },
        s,
        "notes.txt",
    );
    let mut pages = Vec::new();
    laid.render(Range::All, |p| pages.push(p));
    let p = &pages[0];
    assert_eq!((p.image.width, p.image.height), (1890, 3508));
    let [x, y, _, h] = p.at;
    assert!((x - 70.866).abs() < 0.01 && y == 0.0 && (h - 841.89).abs() < 0.01);
}

#[test]
fn a_large_picture_is_drawn_at_300_dpi_of_its_place_and_a_small_one_at_its_own_size() {
    let s = a4(25 * MM);
    let drawn = |size| fitted(size, picture_place(&s, size, 0, 96));
    assert_eq!(drawn((8000, 2000)), (1890, 472));
    assert_eq!(drawn((40, 10)), (40, 10));
    let laid = Laid::new(
        Doc::Picture {
            picture: Arc::new(Canvas::new(4000, 1000)),
            turns: 1,
            dpi: 96,
        },
        s,
        "notes.txt",
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

#[test]
fn laying_out_a_large_picture_shrinks_it_and_lets_the_full_one_go() {
    let full = Arc::new(Canvas::new(8000, 2000));
    let laid = Laid::new(
        Doc::Picture {
            picture: full.clone(),
            turns: 0,
            dpi: 96,
        },
        a4(25 * MM),
        "notes.txt",
    );
    assert_eq!(
        Arc::strong_count(&full),
        1,
        "the laid page holds no reference"
    );
    let Doc::Picture { picture, .. } = &laid.doc else {
        panic!("a picture");
    };
    assert_eq!((picture.width, picture.height), (1890, 472));
    let small = Arc::new(Canvas::new(40, 10));
    let laid = Laid::new(
        Doc::Picture {
            picture: small.clone(),
            turns: 1,
            dpi: 96,
        },
        a4(25 * MM),
        "notes.txt",
    );
    let Doc::Picture { picture, .. } = &laid.doc else {
        panic!("a picture");
    };
    assert!(
        Arc::ptr_eq(picture, &small),
        "a small picture is kept as it is"
    );
}

#[test]
fn an_icon_prints_at_its_own_size_unless_page_setup_fits_it() {
    let icon = Arc::new(Canvas::new(48, 48));
    let at = |scaling| {
        let s = PageSetup {
            scaling,
            ..a4(20 * MM)
        };
        let laid = Laid::new(
            Doc::Picture {
                picture: icon.clone(),
                turns: 0,
                dpi: 96,
            },
            s,
            "icon.png",
        );
        let mut got = Vec::new();
        laid.render(Range::All, |p| got.push(p));
        (got[0].at[2], (got[0].image.width, got[0].image.height))
    };
    let (w, px) = at(Scaling::Actual);
    assert!((w - points(1270)).abs() < 0.01, "12.7 mm wide, not {w} pt");
    assert_eq!(px, (48, 48), "drawn from its own pixels");
    let (w, _) = at(Scaling::Fit);
    assert!(
        (w - points(170 * MM)).abs() < 0.01,
        "fitted between 20 mm margins"
    );
}

#[test]
fn print_preview_draws_the_pages_print_sends() {
    let lines: Vec<String> = (0..150).map(|i| format!("line {i} of the notes")).collect();
    let s = PageSetup {
        header: "&f".into(),
        footer: "Page &p".into(),
        ..a4(20 * MM)
    };
    let doc = || Doc::Text {
        font: mono(12),
        lines: lines.clone(),
    };
    let laid = Laid::new(doc(), s.clone(), "notes.txt");
    assert!(laid.pages() >= 3, "{} pages", laid.pages());
    let again = Laid::new(doc(), s, "notes.txt");
    assert_eq!(
        again.rows, laid.rows,
        "the same document cuts into the same pages"
    );
    let mut printed = Vec::new();
    laid.render(Range::All, |p| printed.push((p.image, p.at)));
    let mut previewed = Vec::new();
    for i in 0..laid.pages() {
        laid.draw(Range::Pages(i + 1, i + 1), |n, c, at| {
            assert_eq!(n, i, "page {i} is the page asked for");
            let p = page(laid.setup(), c, at);
            previewed.push((p.image, p.at));
        });
    }
    assert_eq!(previewed.len(), printed.len());
    assert!(
        previewed == printed,
        "every page drawn alike and in the same place"
    );
}
