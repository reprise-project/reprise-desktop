use super::*;

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
