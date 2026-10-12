//! The message box, sized to its content, laid out in client
//! coordinates for 100% text: text measured at a grown size is reserved at
//! the 100% width that grows to fit it.
//!
//! One control serves both the one-button information box and the
//! three-button "save the changes?" prompt: the button list is a `Vec`.

use crate::classic::widgets::{BUTTON_GAP, BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{Form, Item};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;

/// The first button's id. A one-button box is an OK box.
pub const OK: &str = "btn0";

/// Ids are fixed strings, so they come from a fixed table.
const BUTTON_IDS: [&str; 3] = ["btn0", "btn1", "btn2"];

/// The id of button `i`. Panics past the third: a box has at most three
/// (Yes / No / Cancel).
pub fn button_id(i: usize) -> &'static str {
    BUTTON_IDS
        .get(i)
        .unwrap_or_else(|| panic!("a box has at most three buttons, not {}", i + 1))
}

// Client coordinates.

/// Right margin after the text, and the left one without an icon.
const MARGIN_X: i32 = 8;
const ICON: i32 = 32;
/// The icon's 32x32 bounding box.
const ICON_XY: (i32, i32) = (11, 15);
/// From the icon's right edge to the text's ink.
const ICON_GAP: i32 = 19;
/// The text row's top: its ink centres on the icon's own centre (y 31),
/// and a `px+4` row centred there starts at 24.
const TEXT_Y: i32 = 24;
/// The OK button's top.
const BUTTON_Y: i32 = 64;
/// The client height of a one-line box.
const CLIENT_H: i32 = 98;
const MIN_W: i32 = 162;

/// What the box shows.
pub struct Fields {
    /// May embed `\n`; each line is its own `LINE_H` row.
    pub text: String,
    /// 32x32, drawn at the left. `None` closes the column.
    pub icon: Option<Canvas>,
    /// Left to right.
    pub buttons: Vec<String>,
    /// Index into `buttons` of the default button (the one Enter presses
    /// and the ring marks): the first for most boxes, No for the file
    /// dialog's "already exists" prompt. The host maps `Event::Default` to it.
    pub default: usize,
    /// A reserved text height, for a box that keeps more room for its text
    /// than the text itself needs: Confirm Folder Delete holds a 51 px text
    /// box however short the folder's name is (the compositor's
    /// `src/dialog.rs::confirm_delete` passes the same 51). `None` sizes
    /// the box to its own lines.
    pub min_text_h: Option<i32>,
    /// The narrowest client width as drawn, for a caption title wider than the text
    /// and buttons ([`crate::look::caption_fit`]); 0 sizes the box to them.
    pub min_w: i32,
}

/// The height the text block takes: one row a line, never less than
/// `min_text_h`.
fn block_h(f: &Fields) -> i32 {
    let n = f.text.split('\n').count() as i32;
    (n * LINE_H).max(f.min_text_h.unwrap_or(0))
}

/// How much taller than a one-line box this text makes it.
fn extra_h(f: &Fields) -> i32 {
    (block_h(f) - LINE_H).max(0)
}

fn text_x(f: &Fields) -> i32 {
    if f.icon.is_some() {
        ICON_XY.0 + ICON + ICON_GAP
    } else {
        MARGIN_X
    }
}

/// The width of `n` buttons in a row, `BUTTON_GAP` apart. `client` and
/// `form` both need this to agree, or the box's computed size and its
/// drawn layout drift apart.
fn button_row_w(n: i32) -> i32 {
    n * BUTTON_W + (n - 1).max(0) * BUTTON_GAP
}

/// The client size this box needs at 100%, for text set at `px` that grows by `u`: wide enough
/// for its widest line, its button row and `min_w`, and tall enough for every line of text (one
/// line keeps `CLIENT_H`; each embedded `\n` adds one `LINE_H`, and a `min_text_h` box keeps its
/// reserved rows whatever the text is). The window is `u.size` of it.
pub fn client(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> (i32, i32) {
    let lines: Vec<&str> = f.text.split('\n').collect();
    let widest = lines
        .iter()
        .map(|l| u.design(text.measure(l, false, px)))
        .max()
        .unwrap_or(0);
    let msg = text_x(f) + widest + MARGIN_X;
    let n = f.buttons.len() as i32;
    let row = MARGIN_X * 2 + button_row_w(n);
    (
        msg.max(row).max(MIN_W).max(u.design(f.min_w)),
        CLIENT_H + extra_h(f),
    )
}

/// Build the box over the size `client` reports. Panics on a fourth
/// button, through [`button_id`].
pub fn form(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> Form {
    let (w, _) = client(f, text, px, u);
    let tx = text_x(f);
    let lines: Vec<&str> = f.text.split('\n').collect();
    let extra = extra_h(f);
    let mut items = Vec::new();
    if let Some(icon) = &f.icon {
        items.push(Item::picture(
            "icon",
            Rect::new(ICON_XY.0, ICON_XY.1, ICON, ICON),
            Some(icon.clone()),
        ));
    }
    // A reserved-height box top-aligns its rows, so it draws
    // through `Lines` like a multi-line message rather than through the
    // vertically centred single `Label` row.
    items.push(if lines.len() > 1 || f.min_text_h.is_some() {
        Item::lines(
            "text",
            Rect::new(tx, TEXT_Y, w - tx - MARGIN_X, block_h(f)),
            &lines,
        )
    } else {
        Item::label(
            "text",
            Rect::new(tx, TEXT_Y, w - tx - MARGIN_X, u.base_px(px) as i32 + 4),
            &f.text,
        )
    });
    // The row is centred as a block, so one button sits in the middle and
    // three sit together. A taller message pushes the row down by the
    // same lines it grew by.
    let button_y = BUTTON_Y + extra;
    let n = f.buttons.len() as i32;
    let row_w = button_row_w(n);
    let mut x = (w - row_w) / 2;
    for (i, label) in f.buttons.iter().enumerate() {
        items.push(Item::button(
            button_id(i),
            Rect::new(x, button_y, BUTTON_W, BUTTON_H),
            label,
            true,
            i == f.default,
        ));
        x += BUTTON_W + BUTTON_GAP;
    }
    // The default button has the focus, so Space presses the same button
    // Enter does.
    let mut form = Form::new(items);
    form.focus_id(button_id(f.default));
    form
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::form::Control;
    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::{FontConfig, Theme};

    fn rend() -> (TextRenderer, u32) {
        let text = TextRenderer::load(&FontConfig::default());
        let px = Theme::default().fonts.size_px;
        (text, px)
    }
    fn notfound() -> Fields {
        Fields {
            text: "Cannot find \"zzzzq\"".into(),
            icon: Some(Canvas::new(32, 32)),
            buttons: vec!["OK".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
        }
    }

    /// Confirm Folder Delete keeps four rows of text whatever the
    /// question is: the box grows to the reserved height rather than to
    /// its lines, and the buttons move down with it.
    #[test]
    fn a_reserved_text_height_grows_the_box_past_its_lines() {
        const FOLDER_TEXT_H: i32 = 51;
        let (text, px) = rend();
        let mut tall = notfound();
        tall.min_text_h = Some(FOLDER_TEXT_H);
        let plain_h = client(&notfound(), &text, px, TextUnits::ONE).1;
        assert_eq!(
            client(&tall, &text, px, TextUnits::ONE).1,
            plain_h + FOLDER_TEXT_H - LINE_H
        );
        let f = form(&tall, &text, px, TextUnits::ONE);
        let row = f.item("text").expect("the text row exists");
        assert_eq!(row.rect.height, FOLDER_TEXT_H);
        assert!(
            matches!(row.control, Control::Lines(_)),
            "a reserved height top-aligns its rows"
        );
        assert_eq!(
            f.item(OK).expect("the OK button exists").rect.y,
            BUTTON_Y + FOLDER_TEXT_H - LINE_H
        );
    }

    #[test]
    fn the_box_lays_out_on_its_grid() {
        if !crate::text::test_tahoma() {
            return;
        }
        const EXPECTED_ICON: (i32, i32) = (11, 15);
        const EXPECTED_OK: (i32, i32, i32, i32) = (44, 64, 75, 23);
        let (text, px) = rend();
        let f = form(&notfound(), &text, px, TextUnits::ONE);
        let icon = f.item("icon").expect("the icon slot is filled").rect;
        assert_eq!((icon.x, icon.y), EXPECTED_ICON);
        assert_eq!((icon.width, icon.height), (32, 32));
        let ok = f.item(OK).expect("the OK button exists").rect;
        assert_eq!((ok.x, ok.y, ok.width, ok.height), EXPECTED_OK);
        assert_eq!(ok.width, crate::classic::widgets::BUTTON_W);
    }

    /// The box sizes to its text rather than to a fixed width, which is
    /// what lets one control serve "Cannot find" and the longer
    /// "save the changes?" prompt.
    #[test]
    fn the_box_grows_with_its_text() {
        let (text, px) = rend();
        let narrow = client(&notfound(), &text, px, TextUnits::ONE);
        let mut wordy = notfound();
        wordy.text = "Cannot find \"a considerably longer search string than that\"".into();
        let wide = client(&wordy, &text, px, TextUnits::ONE);
        assert!(wide.0 > narrow.0, "a longer message makes a wider box");
        assert_eq!(wide.1, narrow.1, "one line of text keeps the same height");
    }

    /// The single OK button is the default, and it is centred.
    #[test]
    fn one_button_is_centred_and_default() {
        let (text, px) = rend();
        let f = form(&notfound(), &text, px, TextUnits::ONE);
        let (w, _) = client(&notfound(), &text, px, TextUnits::ONE);
        let ok = f.item(OK).expect("the OK button exists").rect;
        let slack = (w - ok.width) / 2;
        assert!((ok.x - slack).abs() <= 1, "the lone button is centred");
        assert_eq!(f.default_button().map(|i| f.items[i].id), Some(OK));
    }

    /// Three buttons is the save prompt's shape. They share the row, in
    /// order, BUTTON_GAP apart, and the first is the default.
    #[test]
    fn three_buttons_share_the_row_in_order() {
        let (text, px) = rend();
        let prompt = Fields {
            text: "The text in the Untitled file has changed.".into(),
            icon: Some(Canvas::new(32, 32)),
            buttons: vec!["Yes".into(), "No".into(), "Cancel".into()],
            default: 0,
            min_text_h: None,
            min_w: 0,
        };
        let f = form(&prompt, &text, px, TextUnits::ONE);
        let r = |i: usize| f.item(button_id(i)).expect("the button exists").rect;
        assert_eq!(r(0).y, r(1).y, "the buttons share a row");
        assert_eq!(r(1).y, r(2).y);
        assert_eq!(r(1).x - r(0).right(), crate::classic::widgets::BUTTON_GAP);
        assert_eq!(r(2).x - r(1).right(), crate::classic::widgets::BUTTON_GAP);
        assert_eq!(
            f.default_button().map(|i| f.items[i].id),
            Some(button_id(0))
        );
    }

    /// The default button has the focus, so Space and Enter press the
    /// same button even when the default is not the first.
    #[test]
    fn space_and_enter_press_the_default_button() {
        use crate::controls::form::{Event, Key};
        let (text, px) = rend();
        let replace = Fields {
            text: "a.txt already exists.".into(),
            icon: None,
            buttons: vec!["Yes".into(), "No".into()],
            default: 1,
            min_text_h: None,
            min_w: 0,
        };
        let mut f = form(&replace, &text, px, TextUnits::ONE);
        assert_eq!(f.focus().map(|i| f.items[i].id), Some(button_id(1)));
        assert_eq!(f.key(Key::Space), Event::Pressed(button_id(1)));
        assert_eq!(f.key(Key::Enter), Event::Pressed(button_id(1)));
    }

    /// The save-changes prompt embeds a second line with `\n`; the box
    /// grows one `LINE_H` taller to fit it, draws it as two rows through
    /// `Control::Lines` rather than one squashed `Control::Label` row,
    /// and the button row moves down to clear it.
    #[test]
    fn a_second_line_makes_the_box_taller_and_pushes_the_buttons_down() {
        // Hardcoded rather than derived from BUTTON_Y/LINE_H, so a change to
        // either constant fails this test instead of sliding through it.
        const SECOND_LINE_DELTA: i32 = 13; // one extra text row
        const SECOND_LINE_BUTTON_Y: i32 = 77; // BUTTON_Y (64) + one row (13)
        let (text, px) = rend();
        let one = notfound();
        let one_h = client(&one, &text, px, TextUnits::ONE).1;
        let mut two = notfound();
        two.text =
            "The text in the Untitled file has changed.\nDo you want to save the changes?".into();
        let (_, two_h) = client(&two, &text, px, TextUnits::ONE);
        assert_eq!(
            two_h,
            one_h + SECOND_LINE_DELTA,
            "one extra line, one extra row of height"
        );
        let f = form(&two, &text, px, TextUnits::ONE);
        match &f.item("text").expect("the text row exists").control {
            Control::Lines(lines) => assert_eq!(
                lines,
                &vec![
                    "The text in the Untitled file has changed.".to_string(),
                    "Do you want to save the changes?".to_string(),
                ]
            ),
            other => panic!("expected Control::Lines, got {other:?}"),
        }
        let ok = f.item(OK).expect("the OK button exists").rect;
        assert_eq!(ok.y, SECOND_LINE_BUTTON_Y);
    }

    #[test]
    #[should_panic(expected = "at most three buttons")]
    fn form_refuses_a_fourth_button() {
        let (text, px) = rend();
        let mut four = notfound();
        four.buttons = vec!["A".into(), "B".into(), "C".into(), "D".into()];
        form(&four, &text, px, TextUnits::ONE);
    }

    #[test]
    #[should_panic(expected = "at most three buttons, not 4")]
    fn button_id_refuses_a_fourth_slot() {
        assert_eq!(button_id(2), "btn2");
        button_id(3);
    }

    /// No icon leaves the slot empty and pulls the text to the left
    /// margin, so a box without one is not a box with a hole in it.
    #[test]
    fn an_absent_icon_closes_its_column() {
        let (text, px) = rend();
        let with = form(&notfound(), &text, px, TextUnits::ONE);
        let mut bare = notfound();
        bare.icon = None;
        let without = form(&bare, &text, px, TextUnits::ONE);
        assert!(with.item("icon").is_some());
        assert!(without.item("icon").is_none());
        let tx = |f: &Form| f.item("text").expect("the text row exists").rect.x;
        assert!(
            tx(&without) < tx(&with),
            "the text moves left without an icon"
        );
    }

    /// At a grown text size the box is laid out at 100% with its text reserved at the 100%
    /// width that grows to fit it: grown, every line fits its row and the buttons stay inside.
    #[test]
    fn a_grown_box_fits_its_text_and_buttons() {
        let text = TextRenderer::load(&FontConfig::default());
        for px in [17, 22] {
            let u = TextUnits::new(px, 11);
            let f = notfound();
            let (w, h) = u.size(client(&f, &text, px, u));
            let form = form(&f, &text, px, u);
            let lines = form.item("text").unwrap().rect;
            let measured = text.measure(&f.text, false, px);
            assert!(
                u.rect(lines).width >= measured,
                "{px}: the line fits its row"
            );
            let ok = u.rect(form.item(OK).unwrap().rect);
            assert!(
                ok.right() <= w && ok.bottom() <= h,
                "{px}: OK inside {w}x{h}"
            );
            assert_eq!(ok.height, u.len(64 + BUTTON_H) - u.len(64));
        }
    }
}
