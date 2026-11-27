use super::*;
use crate::controls::fingerprints::tests::looks;
use crate::controls::form::Key;
use crate::interfaces;
use crate::theme::Theme;

fn hash(theme: &Theme, f: &Form, s: i32) -> u64 {
    let theme = theme.scaled(s);
    let text = TextRenderer::load(&theme.fonts);
    let p = theme.paint();
    let (w, h) = CLIENT;
    let mut c = Canvas::new((w * s) as u32, (h * s) as u32);
    c.fill_rect(c.bounds(), p.cs.button_face);
    draw(&mut c, &text, &p, f, theme.fonts.size_px);
    crate::pins::fnv(&c)
}

fn landscape_letter() -> PageSetup {
    PageSetup {
        landscape: true,
        margins: [1905, 1905, 2540, 2540],
        scaling: Scaling::Fit,
        ..PageSetup::on(&PAPERS[LETTER])
    }
}

fn with_headers() -> PageSetup {
    PageSetup {
        header: "&f".into(),
        footer: "Page &p".into(),
        ..PageSetup::default()
    }
}

#[test]
fn the_dialog_fits_its_text_under_every_interface() {
    let over = interfaces::form_overflows(|_, _, _| {
        vec![
            ("A4".into(), form(&PageSetup::default(), Kind::Picture)),
            ("Letter".into(), form(&landscape_letter(), Kind::Picture)),
            ("Headers".into(), form(&with_headers(), Kind::Text)),
        ]
    });
    assert!(over.is_empty(), "{}", over.join("\n"));
}

#[test]
fn the_dialog_pins_its_pixels_portrait_and_landscape_in_every_look() {
    crate::pins::fonts();
    let mut got = Vec::new();
    for (look, base) in looks() {
        for (state, s, kind) in [
            ("A4 portrait", PageSetup::default(), Kind::Picture),
            ("Letter landscape fitted", landscape_letter(), Kind::Picture),
            ("A4 with header and footer", with_headers(), Kind::Text),
        ] {
            for scale in [1, 2] {
                got.push((
                    format!("{look} {state} {scale}"),
                    hash(&base, &form(&s, kind), scale),
                ));
            }
        }
    }
    crate::pins::check("controls/pagesetup", &got);
}

#[test]
fn the_preview_lays_the_page_on_its_side_for_landscape() {
    let theme = Theme::default();
    let p = theme.paint();
    let r = Rect::new(0, 0, 120, 120);
    let ink = |s: &PageSetup| {
        let mut c = Canvas::new(120, 120);
        draw_preview(&mut c, &p, s, r);
        let frame = p.cs.window_frame.argb();
        let at: Vec<(i32, i32)> = (0..120)
            .flat_map(|y| (0..120).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == frame)
            .collect();
        let w = at.iter().map(|a| a.0).max().unwrap() - at.iter().map(|a| a.0).min().unwrap();
        let h = at.iter().map(|a| a.1).max().unwrap() - at.iter().map(|a| a.1).min().unwrap();
        (w, h)
    };
    let tall = ink(&PageSetup {
        margins: [0; 4],
        ..PageSetup::default()
    });
    let wide = ink(&PageSetup {
        margins: [0; 4],
        landscape: true,
        ..PageSetup::default()
    });
    assert!(tall.0 < tall.1, "portrait is taller than wide: {tall:?}");
    assert!(wide.0 > wide.1, "landscape is wider than tall: {wide:?}");
}

#[test]
fn ok_reads_the_paper_orientation_and_margins_back() {
    let mut f = form(&PageSetup::default(), Kind::Picture);
    f.set_combo(PAPER, LETTER);
    f.set_check(PORTRAIT, false);
    f.set_check(LANDSCAPE, true);
    f.focus_id(LEFT);
    f.select_edit_text(LEFT);
    for k in "19,05".chars() {
        f.key(Key::Char(k));
    }
    let s = read(&f).unwrap();
    assert_eq!(s.paper, "na_letter_8.5x11in");
    assert!(s.landscape);
    assert_eq!(s.margins, [1905, 2000, 2500, 2500]);
    assert_eq!(s.size(), (27940, 21590));
    assert_eq!(s.body(), Rect::new(1905, 2500, 27940 - 3905, 21590 - 5000));
}

#[test]
fn ok_reads_the_header_and_footer_back_and_only_their_boxes_set_them() {
    let mut f = form(&with_headers(), Kind::Text);
    f.focus_id(FOOTER);
    f.select_edit_text(FOOTER);
    for k in "&p of &d".chars() {
        f.key(Key::Char(k));
    }
    let s = read(&f).unwrap();
    assert_eq!((s.header.as_str(), s.footer.as_str()), ("&f", "&p of &d"));
    let s = read(&form(&with_headers(), Kind::Picture)).unwrap();
    assert_eq!(
        (s.header.as_str(), s.footer.as_str()),
        ("", ""),
        "a dialog without the boxes prints neither"
    );
}

#[test]
fn ok_refuses_a_margin_that_is_no_number_or_leaves_no_room() {
    let mut f = form(&PageSetup::default(), Kind::Picture);
    f.focus_id(TOP);
    f.select_edit_text(TOP);
    f.key(Key::Char('x'));
    assert_eq!(read(&f), Err(Refusal::Number(TOP)));
    f.select_edit_text(TOP);
    for k in "260".chars() {
        f.key(Key::Char(k));
    }
    assert_eq!(
        read(&f),
        Err(Refusal::NoRoom(TOP)),
        "297 mm less 260 and 25 is under 25; Top is the larger"
    );
}

#[test]
fn margins_read_in_millimetres_to_a_hundredth() {
    assert_eq!(parse_mm("20"), Some(2000));
    assert_eq!(parse_mm(" 19.05 "), Some(1905));
    assert_eq!(parse_mm("0,5"), Some(50));
    assert_eq!(parse_mm(".5"), Some(50));
    for bad in ["", ".", "-1", "1.234", "1e2", "ten", "1000.01", "21474836"] {
        assert_eq!(parse_mm(bad), None, "{bad:?}");
    }
    assert_eq!(mm_text(1905), "19.05");
    assert_eq!(mm_text(1950), "19.5");
    assert_eq!(mm_text(2000), "20");
    assert_eq!(mm_text(-50), "-0.5");
}

#[test]
fn letter_is_the_paper_of_the_americas_and_a4_of_the_rest() {
    assert_eq!(paper_for_locale("en_US.UTF-8").media, "na_letter_8.5x11in");
    assert_eq!(paper_for_locale("fr_CA").media, "na_letter_8.5x11in");
    assert_eq!(
        paper_for_locale("de_DE.UTF-8@euro").media,
        "iso_a4_210x297mm"
    );
    assert_eq!(paper_for_locale("C").media, "iso_a4_210x297mm");
}

#[test]
fn hundredths_of_a_millimetre_land_on_device_pixels_and_points() {
    assert_eq!(px(2540, 300), 300);
    assert_eq!(px(21000, 300), 2480);
    assert!((points(2540) - 72.0).abs() < 1e-9);
}

#[test]
fn margins_out_of_range_never_fit_and_never_overflow() {
    let wide = PageSetup {
        margins: [21_474_836, 21_474_836, 0, 0],
        ..PageSetup::default()
    };
    assert!(!wide.fits(), "a margin past a metre");
    assert_eq!(
        wide.body().width,
        0,
        "the body saturates instead of wrapping"
    );
    let negative = PageSetup {
        margins: [-1, 0, 0, 0],
        ..PageSetup::default()
    };
    assert!(!negative.fits());
}

#[test]
fn a_picture_reads_its_scaling_back_and_a_text_keeps_actual_size() {
    let mut f = form(&PageSetup::default(), Kind::Picture);
    assert_eq!(f.checked(ACTUAL), Some(true), "actual size unless picked");
    f.set_check(ACTUAL, false);
    f.set_check(FIT, true);
    assert_eq!(read(&f).unwrap().scaling, Scaling::Fit);
    let fitted = PageSetup {
        scaling: Scaling::Fit,
        ..PageSetup::default()
    };
    assert_eq!(
        form(&fitted, Kind::Picture).checked(FIT),
        Some(true),
        "the dialog opens on the scaling set"
    );
    assert_eq!(
        read(&form(&fitted, Kind::Text)).unwrap().scaling,
        Scaling::Actual
    );
}
