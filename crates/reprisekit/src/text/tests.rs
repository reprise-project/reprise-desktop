use super::fonts::{needs_scan, test_fonts_dir};
use super::*;
use std::path::PathBuf;

#[test]
fn split_mnemonic_marks_one_char_and_unescapes_ampersand() {
    assert_eq!(split_mnemonic("&Programs"), ("Programs".into(), Some(0)));
    assert_eq!(split_mnemonic("Sear&ch"), ("Search".into(), Some(4)));
    assert_eq!(
        split_mnemonic("Files && Folders"),
        ("Files & Folders".into(), None)
    );
    assert_eq!(split_mnemonic("&A && &B"), ("A & B".into(), Some(0)));
    assert_eq!(split_mnemonic("Plain"), ("Plain".into(), None));
    assert_eq!(split_mnemonic("Trailing&"), ("Trailing".into(), None));
    assert_eq!(access_key("&Always use"), Some('a'));
    assert_eq!(access_key("Sear&ch"), Some('c'));
    assert_eq!(access_key("Plain"), None);
}

#[test]
fn an_escaped_name_reads_back_whole_with_no_access_key() {
    for name in ["R&D notes.txt", "&", "a&&b", "Plain", ""] {
        let label = escape_mnemonic(name);
        assert_eq!(split_mnemonic(&label), (name.to_string(), None), "{name}");
        assert_eq!(access_key(&label), None, "{name}");
    }
}

#[test]
fn ellipsize_keeps_fitting_text_and_trims_the_rest() {
    let r = TextRenderer::load(&FontConfig::default());
    let full = r.measure("Printers", false, 11);
    assert_eq!(r.ellipsize("Printers", full, false, 11), "Printers");
    let short = r.ellipsize("Printers", full - 20, false, 11);
    assert!(short.ends_with("..."), "{short}");
    assert!(short.len() < "Printers".len());
    assert!(r.measure(&short, false, 11) <= full - 20);
    for width in 0..r.measure("Recycle Bin", false, 11) {
        let short = r.ellipsize("Recycle Bin", width, false, 11);
        assert!(
            !short.ends_with(" ..."),
            "no space before the dots: {short}"
        );
    }
    assert_eq!(r.ellipsize("abc", 0, false, 11), "...");
}
use crate::raster::{Canvas, Rect};
use crate::theme::{Color, FontConfig};

fn renderer() -> TextRenderer {
    TextRenderer::load(&FontConfig::default())
}

#[test]
fn line_metrics_are_memoised_per_size() {
    let r = renderer();
    let rect = Rect::new(0, 0, 100, 20);
    let a = r.baseline(rect, false, 11);
    let b = r.baseline(rect, true, 22);
    assert_eq!(r.metrics.borrow().len(), 2);
    assert_eq!(r.baseline(rect, false, 11), a);
    assert_eq!(r.baseline(rect, true, 22), b);
    assert_eq!(r.metrics.borrow().len(), 2);
}

#[test]
fn without_a_second_dialog_font_every_dialog_takes_the_ui_font() {
    let with = |spec: Option<&str>| {
        TextRenderer::load(&FontConfig {
            shell_dlg: spec.map(String::from),
            ..FontConfig::default()
        })
    };
    for spec in [None, Some("No Such Font Family"), Some("/no/such/dlg.ttf")] {
        let r = with(spec);
        assert!(!r.has_shell_dlg(), "{spec:?}");
        assert!(std::ptr::eq(r.dialog(DialogFont::ShellDlg), &r), "{spec:?}");
    }
}

#[test]
fn the_second_dialog_font_loads_by_family_or_file_on_the_ui_baseline() {
    if !test_tahoma() {
        return;
    }
    let db = system_fonts(&[]);
    let Some((family, path)) = db.faces().find_map(|f| {
        let family = &f.families.first()?.0;
        match &f.source {
            fontdb::Source::File(p) if family != "Tahoma" && f.index == 0 => {
                Some((family.clone(), p.to_string_lossy().into_owned()))
            }
            _ => None,
        }
    }) else {
        return;
    };
    for spec in [family.clone(), path] {
        let r = TextRenderer::load(&FontConfig {
            shell_dlg: Some(spec.clone()),
            ..FontConfig::default()
        });
        assert!(r.has_shell_dlg(), "{spec}");
        assert!(r.has_face("Tahoma"), "the UI font stays: {spec}");
        let second = r.dialog(DialogFont::ShellDlg);
        assert!(second.has_face(&family), "{spec}");
        assert!(std::ptr::eq(r.dialog(DialogFont::Ui), &r));
        for h in 11..24 {
            let rect = Rect::new(0, 0, 50, h);
            for bold in [false, true] {
                assert_eq!(
                    second.baseline(rect, bold, 11),
                    r.baseline(rect, bold, 11),
                    "{spec} in a {h} px rect"
                );
            }
        }
    }
}

#[test]
fn a_bitmap_font_file_loads_as_the_second_dialog_font() {
    let path = "/usr/share/wine/fonts/sserife.fon";
    if !std::path::Path::new(path).exists() {
        return;
    }
    let r = TextRenderer::load(&FontConfig {
        shell_dlg: Some(path.into()),
        ..FontConfig::default()
    });
    let second = r.dialog(DialogFont::ShellDlg);
    assert!(r.has_shell_dlg());
    assert_eq!(['E', 'N'].map(|c| second.advance(c, false, 11)), [7, 8]);
}

#[test]
fn a_bitmap_font_doubles_its_strike_at_scale_2_and_else_takes_the_ui_font() {
    let path = "/usr/share/wine/fonts/sserife.fon";
    if !std::path::Path::new(path).exists() {
        return;
    }
    let r = TextRenderer::load(&FontConfig {
        shell_dlg: Some(path.into()),
        ..FontConfig::default()
    });
    let second = r.dialog(DialogFont::ShellDlg);
    let text = "Reprise Settings";
    let rect = Rect::new(0, 0, 120, 20);
    let draw = |font: &TextRenderer, size, scale| {
        let mut c = Canvas::with_scale(120, 20, scale);
        let ink = Color::rgb(255, 255, 255);
        font.draw(&mut c, rect, text, false, ink, Align::Left, size);
        c
    };
    let (one, two) = (draw(second, 11, 1), draw(second, 11, 2));
    let width = second.measure(text, false, 11);
    let right = (0..120)
        .filter(|&x| (0..20).any(|y| one.get(x, y) != 0))
        .max()
        .expect("the strike inks at scale 1")
        + 1;
    assert!(
        (width - 2..=width).contains(&right),
        "ink ends at {right}, measure gives {width}"
    );
    for y in 0..40 {
        for x in 0..240 {
            assert_eq!(
                two.pixels[y * 240 + x],
                one.pixels[y / 2 * 120 + x / 2],
                "device pixel ({x}, {y}) at scale 2"
            );
        }
    }
    assert_eq!(second.measure(text, false, 12), r.measure(text, false, 12));
    for scale in [1, 2] {
        let ui = draw(&r, 12, scale);
        assert!(ui.pixels.iter().any(|&p| p != 0), "UI ink at scale {scale}");
        assert_eq!(draw(second, 12, scale).pixels, ui.pixels, "scale {scale}");
    }
}

#[test]
fn a_stub_second_font_inks_its_pens() {
    let ui = renderer();
    let rect = Rect::new(0, 0, 60, 13);
    let Some(baseline) = ui.baseline(rect, false, 11) else {
        return;
    };
    let r = ui.with_shell_dlg(TextRenderer::stub(11, |c| if c == 'E' { 7 } else { 6 }));
    let second = r.dialog(DialogFont::ShellDlg);
    assert_eq!(second.measure("En d", false, 11), 25);
    let mut c = Canvas::new(60, 13);
    let ink = Color::rgb(255, 255, 255);
    second.draw(&mut c, rect, "En d", false, ink, Align::Left, 11);
    let dots: Vec<(i32, i32)> = (0..60)
        .flat_map(|x| (0..13).map(move |y| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != 0)
        .collect();
    let row = baseline - 1;
    assert_eq!(dots, vec![(0, row), (7, row), (19, row)]);
}

#[test]
fn only_the_second_dialog_font_leads() {
    let ui = renderer();
    assert_eq!(ui.lead(), 0);
    assert_eq!(ui.dialog(DialogFont::ShellDlg).lead(), 0);
    let r = ui.with_shell_dlg(TextRenderer::stub(11, |_| 6));
    assert_eq!(r.lead(), 0);
    assert_eq!(r.dialog(DialogFont::ShellDlg).lead(), 1);
}

#[test]
fn measures_wider_for_longer_text() {
    let r = renderer();
    assert!(r.measure("Start", true, 11) > r.measure("S", true, 11));
    assert_eq!(r.measure("", true, 11), 0);
    assert!(r.measure("Start", false, 22) > r.measure("Start", false, 11));
}

#[test]
fn has_face_reports_loaded_family() {
    let r = renderer();
    assert!(r.has_face("Tahoma") || r.has_face("DejaVu Sans"));
    assert!(!r.has_face("No Such Font Family"));
}

#[test]
fn draws_pixels_inside_rect_only() {
    let r = renderer();
    let mut c = Canvas::new(60, 20);
    let rect = Rect::new(2, 2, 40, 16);
    r.draw(
        &mut c,
        rect,
        "Start",
        true,
        Color::rgb(255, 255, 255),
        Align::Left,
        11,
    );
    let inside = (0..60)
        .flat_map(|x| (0..20).map(move |y| (x, y)))
        .filter(|&(x, y)| c.get(x, y) != 0)
        .count();
    assert!(inside > 10, "expected glyph pixels");
    for x in 0..60 {
        for y in 0..20 {
            if c.get(x, y) != 0 {
                assert!(rect.contains(x, y), "pixel outside clip at {x},{y}");
            }
        }
    }
}

#[test]
fn mono_mode_yields_solid_pixels() {
    let r = TextRenderer::load(&FontConfig {
        antialias: false,
        ..FontConfig::default()
    });
    let mut c = Canvas::new(60, 20);
    r.draw(
        &mut c,
        Rect::new(0, 0, 60, 20),
        "Wg",
        true,
        Color::rgb(255, 255, 255),
        Align::Left,
        11,
    );
    for p in &c.pixels {
        assert!(
            *p == 0 || *p == 0xFFFFFFFF,
            "unexpected blended pixel {p:#x}"
        );
    }
    // `blit_mask` thresholds when antialias is off, so the coverage FreeType returned must
    // itself be bilevel.
    let cache = r.cache.borrow();
    assert!(!cache.is_empty(), "expected rasterized glyphs");
    for g in cache.values() {
        assert!(
            g.bitmap.iter().all(|&b| b == 0 || b == 255),
            "mono coverage has a blended byte"
        );
    }
}

#[test]
fn reprise_test_fonts_supplies_the_ui_tahoma() {
    if test_fonts_dir().is_some() {
        assert!(test_tahoma(), "REPRISE_TEST_FONTS holds no Tahoma");
    }
}

#[test]
fn grayscale_mode_blends() {
    // Wine's Tahoma has bilevel bitmap strikes from 8 to 16 px but none at 14.
    let r = TextRenderer::load(&FontConfig {
        antialias: true,
        ..FontConfig::default()
    });
    let mut c = Canvas::new(60, 20);
    c.fill_rect(Rect::new(0, 0, 60, 20), Color::rgb(0, 0, 0));
    r.draw(
        &mut c,
        Rect::new(0, 0, 60, 20),
        "Wg",
        false,
        Color::rgb(255, 255, 255),
        Align::Left,
        14,
    );
    let partial = c.pixels.iter().any(|&p| p != 0xFF000000 && p != 0xFFFFFFFF);
    assert!(partial, "expected at least one antialiased pixel");
    assert!(
        r.cache
            .borrow()
            .values()
            .any(|g| g.bitmap.iter().any(|&b| b > 0 && b < 255)),
        "grayscale coverage is bilevel"
    );
}

#[test]
fn italic_is_the_families_slanted_face_when_installed() {
    let cfg = FontConfig {
        family: vec!["DejaVu Sans".into()],
        ..FontConfig::default()
    };
    let t = TextRenderer::load(&cfg);
    if !t.has_face("DejaVu Sans") {
        crate::pins::skip!("DejaVu Sans is not installed");
    }
    let slanted = t.italic();
    assert!(slanted.is_italic(), "DejaVu Sans Oblique");
    assert!(std::ptr::eq(slanted.italic(), slanted));
    let none = TextRenderer::fontless();
    assert!(std::ptr::eq(none.italic(), &none), "no config to load");
}

/// Tahoma has no italic face: its upright faces lean right, a stem's top ink right of
/// its foot.
#[test]
fn italic_shears_a_family_without_a_slanted_face() {
    if !test_tahoma() {
        return;
    }
    let t = TextRenderer::load(&FontConfig::default());
    let slanted = t.italic();
    assert!(!std::ptr::eq(slanted, &t));
    assert!(!slanted.is_italic(), "no slanted face");
    let stem = |face: &TextRenderer| {
        let mut c = Canvas::new(20, 20);
        face.draw(
            &mut c,
            Rect::new(4, 2, 16, 16),
            "l",
            false,
            Color::rgb(0, 0, 0),
            Align::Left,
            11,
        );
        let ink: Vec<(i32, i32)> = (0..20)
            .flat_map(|y| (0..20).map(move |x| (x, y)))
            .filter(|&(x, y)| c.get(x, y) >> 24 != 0)
            .collect();
        let top = ink.iter().map(|p| p.1).min().unwrap();
        let foot = ink.iter().map(|p| p.1).max().unwrap();
        let left_at = |y: i32| ink.iter().filter(|p| p.1 == y).map(|p| p.0).min().unwrap();
        left_at(top) - left_at(foot)
    };
    assert_eq!(stem(&t), 0, "upright");
    assert!(stem(slanted) > 0, "leans right");
}

#[test]
fn an_italic_config_loads_an_italic_face_when_one_exists() {
    let db = system_fonts(&[]);
    let mut slanted: Vec<&str> = db
        .faces()
        .filter(|f| f.style == fontdb::Style::Italic)
        .filter_map(|f| f.families.first().map(|(name, _)| name.as_str()))
        .collect();
    slanted.sort_unstable();
    let Some(&family) = slanted.first() else {
        return;
    };
    let cfg = FontConfig {
        family: vec![family.into()],
        italic: true,
        ..FontConfig::default()
    };
    let t = TextRenderer::load(&cfg);
    assert!(t.has_face(family), "{family} loads");
    assert!(t.is_italic(), "{family} loads its italic face");
}

#[test]
fn center_alignment_moves_text_right() {
    let r = renderer();
    let mut left = Canvas::new(100, 20);
    let mut center = Canvas::new(100, 20);
    r.draw(
        &mut left,
        Rect::new(0, 0, 100, 20),
        "ab",
        false,
        Color::rgb(255, 255, 255),
        Align::Left,
        11,
    );
    r.draw(
        &mut center,
        Rect::new(0, 0, 100, 20),
        "ab",
        false,
        Color::rgb(255, 255, 255),
        Align::Center,
        11,
    );
    let first = |c: &Canvas| {
        (0..100)
            .find(|&x| (0..20).any(|y| c.get(x, y) != 0))
            .unwrap()
    };
    assert!(first(&center) > first(&left) + 20);
}

#[test]
fn glyph_cache_is_reused() {
    let r = renderer();
    assert_eq!(r.cache.borrow().len(), 0);
    r.measure("aa", false, 11);
    assert_eq!(r.cache.borrow().len(), 1);
    r.measure("ab", true, 11);
    assert_eq!(r.cache.borrow().len(), 3);
    r.measure("a", false, 12);
    assert_eq!(r.cache.borrow().len(), 4);
}

#[test]
fn tahoma_widths_match_hdmx() {
    let r = TextRenderer::load(&FontConfig {
        antialias: false,
        ..FontConfig::default()
    });
    if !test_tahoma() {
        return;
    }
    assert_eq!(r.measure("Start", false, 11), 24);
    assert_eq!(r.measure("Start", true, 11), 29);
}

fn missing_chars(r: &TextRenderer, n: usize) -> Vec<char> {
    let face = r.face(false).expect("a face");
    let v: Vec<char> = (0x10_0000u32..0x10_FFFD)
        .filter_map(char::from_u32)
        .filter(|&ch| face.get_char_index(ch as usize).is_none())
        .filter(|&ch| {
            r.fallback_face(ch, false, false).is_none()
                && r.fallback_face(ch, true, false).is_none()
        })
        .take(n)
        .collect();
    assert_eq!(v.len(), n, "the faces cover the private use plane");
    v
}

#[test]
fn a_char_the_ui_face_lacks_draws_from_a_fallback_face() {
    let r = TextRenderer::load(&FontConfig {
        antialias: true,
        ..FontConfig::default()
    });
    let face = r.face(false).expect("a face");
    if face.get_char_index('中' as usize).is_some()
        || crate::fontfallback::fontconfig_face_for('中', false, false).is_none()
    {
        return;
    }
    let draw = |text: &str| {
        let mut c = Canvas::new(40, 20);
        let white = Color::rgb(255, 255, 255);
        r.draw(
            &mut c,
            Rect::new(0, 0, 40, 20),
            text,
            false,
            white,
            Align::Left,
            11,
        );
        c.pixels
    };
    let han = draw("中");
    assert!(han.iter().any(|&p| p != 0), "the glyph inks");
    let graded = han.iter().any(|&p| p != 0 && (p >> 16) & 0xFF != 0xFF);
    assert!(graded, "a fallback glyph is smoothed");
    assert!(r.advance('中', false, 11) >= 8, "a full-width advance");
    let notdef = r.advance(missing_chars(&r, 1)[0], false, 11);
    assert_ne!(han, draw("\u{10FFFD}"), "not the notdef box ({notdef} px)");
}

#[test]
fn mono_text_draws_a_fallback_char_from_a_bitmap_face_at_scale_1_only() {
    let r = TextRenderer::load(&FontConfig {
        antialias: false,
        ..FontConfig::default()
    });
    let face = r.face(false).expect("a face");
    if face.get_char_index('中' as usize).is_some()
        || crate::fontfallback::fontconfig_face_for('中', false, false).is_none()
    {
        return;
    }
    let bitmap = r.fallback_face('中', false, true).is_some_and(|f| {
        let strikes = fixed_sizes(&f)
            .iter()
            .map(|s| ((s.y_ppem + 32) >> 6) as u32);
        fallback_strike(strikes, 11).is_some()
    });
    let draw = |scale: i32| {
        let mut c = Canvas::with_scale(40, 20, scale);
        let white = Color::rgb(255, 255, 255);
        r.draw(
            &mut c,
            Rect::new(0, 0, 40, 20),
            "中",
            false,
            white,
            Align::Left,
            11,
        );
        c
    };
    let is_mono = |c: &Canvas| c.pixels.iter().all(|&p| p == 0 || p == 0xFFFF_FFFF);
    let (one, two) = (draw(1), draw(2));
    assert!(one.pixels.iter().any(|&p| p != 0), "the glyph inks");
    assert_eq!(
        is_mono(&one),
        bitmap,
        "mono from a bitmap face, else smoothed"
    );
    assert!(
        two.pixels.iter().any(|&p| p != 0),
        "the glyph inks at scale 2"
    );
    assert!(!is_mono(&two), "smoothed outline at scale 2");
}

/// A BDF face with one strike at `px` whose only glyph, 中, is a solid `px` x `px` block.
fn block_strike_face(px: u32) -> (PathBuf, Face) {
    let row = format!("{:06X}", ((1u32 << px) - 1) << (24 - px));
    let rows = vec![row; px as usize].join("\n");
    let bdf = format!(
        "STARTFONT 2.1\nFONT -reprise-block-medium-r-normal--{px}-{d}-75-75-c-{d}-iso10646-1\n\
         SIZE {px} 75 75\nFONTBOUNDINGBOX {px} {px} 0 -2\nSTARTPROPERTIES 5\nPIXEL_SIZE {px}\n\
         FONT_ASCENT {a}\nFONT_DESCENT 2\nCHARSET_REGISTRY \"ISO10646\"\nCHARSET_ENCODING \"1\"\n\
         ENDPROPERTIES\nCHARS 1\nSTARTCHAR U+4E2D\nENCODING 20013\nSWIDTH 1000 0\n\
         DWIDTH {px} 0\nBBX {px} {px} 0 -2\nBITMAP\n{rows}\nENDCHAR\nENDFONT\n",
        d = px * 10,
        a = px - 2,
    );
    let path = std::env::temp_dir().join(format!("reprise-block-{px}-{}.bdf", std::process::id()));
    std::fs::write(&path, bdf).expect("write the BDF");
    let face = Library::init()
        .expect("FreeType")
        .new_face(&path, 0)
        .expect("FreeType opens the BDF");
    (path, face)
}

#[test]
fn a_bitmap_strike_at_the_device_size_draws_only_for_an_output_at_scale_1() {
    let r = TextRenderer::load(&FontConfig {
        antialias: false,
        ..FontConfig::default()
    });
    if r.face(false)
        .is_none_or(|f| f.get_char_index('中' as usize).is_some())
    {
        return;
    }
    let (path, face) = block_strike_face(22);
    let at_2 = r.for_output(2);
    for t in [&r, at_2] {
        t.fallback_mono.borrow_mut().pick(
            '中',
            false,
            |f: &Face, c| f.get_char_index(c as usize).is_some(),
            |_, _| Some((path.clone(), 0)),
            |_| Some(face.clone()),
        );
    }
    let draw = |t: &TextRenderer| {
        let mut c = Canvas::new(40, 30);
        let white = Color::rgb(255, 255, 255);
        t.draw(
            &mut c,
            Rect::new(0, 0, 40, 30),
            "中",
            false,
            white,
            Align::Left,
            22,
        );
        c
    };
    let lit = |c: &Canvas| c.pixels.iter().filter(|&&p| p == 0xFFFF_FFFF).count();
    let (one, two) = (draw(&r), draw(at_2));
    std::fs::remove_file(&path).ok();
    assert_eq!(
        lit(&one),
        22 * 22,
        "the 22 px strike's block at output scale 1"
    );
    assert!(
        two.pixels.iter().any(|&p| p != 0),
        "the glyph inks at output scale 2"
    );
    assert_ne!(lit(&two), 22 * 22, "no strike block at output scale 2");
    if crate::fontfallback::fontconfig_face_for('中', false, false).is_some() {
        assert!(
            two.pixels.iter().any(|&p| p != 0 && p != 0xFFFF_FFFF),
            "smoothed outline at output scale 2"
        );
    }
    assert!(std::ptr::eq(r.for_output(1), &r), "itself at scale 1");
}

#[test]
fn missing_chars_share_one_notdef_entry() {
    let r = renderer();
    let missing: String = missing_chars(&r, 200).into_iter().collect();
    let first = missing.chars().next().unwrap();
    let notdef = r.advance(first, false, 11);
    assert_eq!(r.measure(&missing, false, 11), notdef * 200);
    assert_eq!(r.cache.borrow().len(), 1, "one notdef entry for all 200");
    r.measure(&missing, true, 11);
    assert_eq!(r.cache.borrow().len(), 2, "one notdef entry per weight");
}

#[test]
fn a_title_of_private_use_or_unassigned_chars_asks_fontconfig_a_handful_of_times() {
    let r = renderer();
    let asked = || crate::fontfallback::FONTCONFIG_ASKS.get();
    let text = |from: u32| -> String { (from..).filter_map(char::from_u32).take(300).collect() };
    let before = asked();
    r.measure(&text(0x10_0000), false, 11);
    assert!(
        asked() - before <= 12,
        "at most two asks per block, three blocks, two fallback caches; asked {}",
        asked() - before
    );
    let before = asked();
    r.measure(&text(0x4_0000), false, 11);
    assert_eq!(asked(), before, "unassigned chars ask nothing");
}

#[test]
fn glyph_cache_never_exceeds_its_cap() {
    let r = renderer();
    let face = r.face(false).expect("a face");
    let present: Vec<char> = (0x20u32..0x3_0000)
        .filter_map(char::from_u32)
        .filter(|&ch| face.get_char_index(ch as usize).is_some())
        .take(600)
        .collect();
    let mut peak = 0;
    'fill: for size in 8..40 {
        for bold in [false, true] {
            for &ch in &present {
                r.advance(ch, bold, size);
                let len = r.cache.borrow().len();
                assert!(len <= GLYPH_CAP, "cache holds {len}, cap {GLYPH_CAP}");
                peak = peak.max(len);
                if peak == GLYPH_CAP && len < 10 {
                    break 'fill;
                }
            }
        }
    }
    assert_eq!(peak, GLYPH_CAP, "the fill reaches the cap");
    assert!(
        r.cache.borrow().len() < 10,
        "the cache was cleared at the cap"
    );
}

#[test]
fn a_cleared_cache_draws_and_measures_the_same() {
    let r = renderer();
    let odd = missing_chars(&r, 1)[0];
    let text = format!("Start Wg{odd}é");
    let snapshot = |r: &TextRenderer| {
        let mut c = Canvas::new(120, 20);
        r.draw(
            &mut c,
            Rect::new(0, 0, 120, 20),
            &text,
            true,
            Color::rgb(255, 255, 255),
            Align::Left,
            11,
        );
        let advances: Vec<i32> = text.chars().map(|ch| r.advance(ch, true, 11)).collect();
        (c.pixels, advances)
    };
    let before = snapshot(&r);
    let mut size = 12;
    let mut ch = 0x21u32;
    while r.cache.borrow().len() > 1 {
        if let Some(c) = char::from_u32(ch) {
            r.advance(c, false, size);
        }
        ch += 1;
        if ch == 0x2000 {
            ch = 0x21;
            size += 1;
        }
    }
    assert_eq!(snapshot(&r), before);
}

#[test]
fn font_database_is_rescanned_only_for_a_missing_preferred_family() {
    let db = system_fonts(&[]);
    let present = db
        .faces()
        .find_map(|f| f.families.first())
        .map(|(name, _)| name.clone())
        .expect("a system font");
    let missing = "No Such Font Family".to_string();
    assert!(!needs_scan(&db, &[]));
    assert!(!needs_scan(&db, std::slice::from_ref(&present)));
    assert!(!needs_scan(&db, &[present.clone(), missing.clone()]));
    assert!(needs_scan(&db, &[missing, present]));
}

#[test]
fn span_is_the_prefix_width_and_the_char_advance() {
    let r = renderer();
    let text = "Séarch";
    for (i, ch) in text.chars().enumerate() {
        let prefix: String = text.chars().take(i).collect();
        assert_eq!(
            r.span(text, i, true, 11),
            (r.measure(&prefix, true, 11), r.advance(ch, true, 11))
        );
    }
    assert_eq!(r.span(text, 6, false, 11), (r.measure(text, false, 11), 0));
    assert_eq!(
        r.span_and_measure(text, 2, true, 11),
        (
            r.measure("Sé", true, 11),
            r.advance('a', true, 11),
            r.measure(text, true, 11)
        )
    );
}

#[test]
fn wrap_chars_fills_lines_to_the_column_count() {
    assert_eq!(
        wrap_chars("aaa bbb cc dddd", 7),
        vec!["aaa bbb", "cc dddd"],
        "a line takes a word that ends exactly on the column"
    );
    assert_eq!(wrap_chars("one\ntwo three", 20), vec!["one", "two three"]);
    assert_eq!(
        wrap_chars("toolongword x", 4),
        vec!["tool", "ongw", "ord", "x"]
    );
    assert!(wrap_chars("  ", 10).is_empty(), "blank text has no lines");
}

#[test]
fn wrap_chars_breaks_a_long_word_after_its_last_separator_that_fits() {
    assert_eq!(
        wrap_chars("in /srv/projects/Downloads/archive", 18),
        vec!["in", "/srv/projects/", "Downloads/archive"]
    );
    assert_eq!(
        wrap_chars("a_very-long.name", 7),
        vec!["a_very-", "long.", "name"]
    );
    assert_eq!(wrap_chars("/abcdef", 3), vec!["/ab", "cde", "f"]);
    assert_eq!(
        wrap_chars("ab", 0),
        vec!["a", "b"],
        "a line holds one char at least"
    );
}

#[test]
fn wrap_breaks_a_word_wider_than_the_line_at_the_widest_prefix() {
    let r = renderer();
    let word = "ProjectArchiveWithoutAnySpacesInItsName";
    let max_w = r.measure(word, false, 11) / 3;
    let lines = wrap(&r, word, max_w, 11);
    assert!(lines.len() >= 3, "{lines:?}");
    assert_eq!(lines.concat(), word);
    for pair in lines.windows(2) {
        assert!(
            r.measure(&pair[0], false, 11) <= max_w,
            "{pair:?} fits {max_w}"
        );
        let more = format!("{}{}", pair[0], pair[1].chars().next().unwrap());
        assert!(
            r.measure(&more, false, 11) > max_w,
            "{more:?} is over {max_w}"
        );
    }
}

#[test]
fn wrap_keeps_words_that_fit_whole() {
    let r = renderer();
    let body = "Original folder /srv/projects/Documents";
    let max_w = r.measure("/srv/projects/Documents", false, 11);
    assert_eq!(
        wrap(&r, body, max_w, 11),
        vec!["Original folder", "/srv/projects/Documents"]
    );
}

#[test]
fn wrap_breaks_on_width_and_newlines() {
    let r = renderer();
    assert_eq!(
        wrap(&r, "one two\nthree", 10_000, 11),
        vec!["one two", "three"]
    );
    assert!(wrap(&r, "aaaa bbbb cccc dddd eeee ffff", 40, 11).len() > 1);
}

#[test]
fn wrap_fills_each_line_to_the_measured_width() {
    let r = renderer();
    let body = "The quick brown fox jumps over the lazy dog and keeps running past the fence";
    for bold in [false, true] {
        for max_w in [60, 90, 150] {
            let lines = wrap_bold(&r, body, max_w, 11, bold);
            assert_eq!(lines.join(" "), body);
            for pair in lines.windows(2) {
                assert!(
                    r.measure(&pair[0], bold, 11) <= max_w,
                    "{pair:?} fits {max_w}"
                );
                let next = pair[1].split(' ').next().unwrap();
                let joined = format!("{} {next}", pair[0]);
                assert!(
                    r.measure(&joined, bold, 11) > max_w,
                    "{joined:?} is over {max_w}"
                );
            }
        }
    }
}

fn ink_box(c: &Canvas) -> (i32, i32, i32, i32) {
    let w = c.device_size().0 as i32;
    let mut b = (i32::MAX, i32::MAX, 0, 0);
    for (i, &p) in c.pixels.iter().enumerate() {
        if p != 0 {
            let (x, y) = (i as i32 % w, i as i32 / w);
            b = (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1));
        }
    }
    b
}

#[test]
fn text_on_a_scale_2_canvas_is_rasterized_at_twice_the_size_on_the_same_layout() {
    let r = renderer();
    let draw = |s: i32| {
        let mut c = Canvas::with_scale(60, 20, s);
        let rect = Rect::new(4, 0, 56, 20);
        r.draw(
            &mut c,
            rect,
            "HHH",
            false,
            Color::rgb(0, 0, 0),
            Align::Left,
            11,
        );
        ink_box(&c)
    };
    let (a, b) = (draw(1), draw(2));
    assert!(a.2 > a.0, "expected ink at scale 1");
    let near = |got: i32, want: i32| (got - want).abs() <= 2;
    assert!(near(b.0, 2 * a.0), "left edge {b:?} is twice {a:?}");
    assert!(
        near(b.1, 2 * a.1) && near(b.3, 2 * a.3),
        "rows {b:?} are twice {a:?}"
    );
    assert!(near(b.2, 2 * a.2), "right edge {b:?} is twice {a:?}");
}

#[test]
fn coverage_unpacks_every_bitmap_depth() {
    use freetype::ffi::{
        FT_Bitmap, FT_PIXEL_MODE_GRAY, FT_PIXEL_MODE_GRAY2, FT_PIXEL_MODE_GRAY4, FT_PIXEL_MODE_MONO,
    };
    let expand = |mode, pitch: i32, mut buf: Vec<u8>| {
        let raw = FT_Bitmap {
            rows: 2,
            width: 5,
            pitch,
            buffer: buf.as_mut_ptr(),
            num_grays: 0,
            pixel_mode: mode as _,
            palette_mode: 0,
            palette: std::ptr::null_mut(),
        };
        // SAFETY: `raw` and `buf` outlive the bitmap, whose buffer holds `pitch * rows` bytes.
        coverage(&unsafe { freetype::Bitmap::from_raw(&raw) })
    };
    let want = vec![255, 0, 255, 0, 255, 0, 0, 0, 0, 255];
    assert_eq!(expand(FT_PIXEL_MODE_MONO, 1, vec![0xA8, 0x08]), want);
    assert_eq!(
        expand(FT_PIXEL_MODE_GRAY2, 2, vec![0xCC, 0xC0, 0x00, 0x40]),
        [255, 0, 255, 0, 255, 0, 0, 0, 0, 85]
    );
    assert_eq!(
        expand(
            FT_PIXEL_MODE_GRAY4,
            3,
            vec![0xF0, 0xF0, 0xF0, 0x00, 0x00, 0x80]
        ),
        [255, 0, 255, 0, 255, 0, 0, 0, 0, 136]
    );
    assert_eq!(expand(FT_PIXEL_MODE_GRAY, 5, want.clone()), want);
}
