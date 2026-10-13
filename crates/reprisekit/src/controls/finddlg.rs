//! The Find and Replace dialogs, laid out in client coordinates. Replace's
//! label column is wider, so each dialog has its own constants.

use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::raster::Rect;
use crate::text::DialogFont;

pub const QUERY: &str = "query";
pub const WITH: &str = "with";
pub const MATCH_CASE: &str = "matchcase";
pub const UP: &str = "up";
pub const DOWN: &str = "down";
pub const FIND_NEXT: &str = "findnext";
pub const REPLACE: &str = "replace";
pub const REPLACE_ALL: &str = "replaceall";
pub const CANCEL: &str = "cancel";

pub const FIND_CLIENT: (i32, i32) = (354, 102);
pub const REPLACE_CLIENT: (i32, i32) = (345, 154);

const LABEL_X: i32 = 7;
const ROW_Y: [i32; 2] = [12, 40];
const EDIT_H: i32 = 20;
const CHECK: i32 = 13;

const FIND_EDIT_X: i32 = 71;
const FIND_EDIT_W: i32 = 192;
const FIND_BTN_X: i32 = 273;
const FIND_BTN_Y: [i32; 2] = [9, 38];
const FIND_CHECK_XY: (i32, i32) = (6, 72);
const GROUP: Rect = Rect::new(161, 49, 102, 40);
const RADIO_XY: [(i32, i32); 2] = [(168, 66), (208, 66)];

const REPLACE_EDIT_X: i32 = 81;
const REPLACE_EDIT_W: i32 = 171;
const REPLACE_BTN_X: i32 = 261;
const REPLACE_BTN_Y: [i32; 4] = [8, 35, 63, 90];
const REPLACE_CHECK_XY: (i32, i32) = (8, 105);

pub struct FindFields {
    pub query: String,
    pub match_case: bool,
    pub down: bool,
}

pub struct ReplaceFields {
    pub query: String,
    pub with: String,
    pub match_case: bool,
}

fn button(id: &'static str, x: i32, y: i32, text: &str, live: bool, default: bool) -> Item {
    Item::button(id, Rect::new(x, y, BUTTON_W, BUTTON_H), text, live, default)
}

fn match_case_row(xy: (i32, i32), checked: bool) -> Item {
    Item::new(
        MATCH_CASE,
        Rect::new(xy.0, xy.1, CHECK, CHECK),
        Control::Check {
            label: "Match &case".into(),
            checked,
            enabled: true,
            mixed: None,
        },
    )
}

pub fn find_form(f: &FindFields) -> Form {
    let live = !f.query.is_empty();
    Form::new(vec![
        Item::label(
            "querylabel",
            Rect::new(LABEL_X, ROW_Y[0], FIND_EDIT_X - LABEL_X, EDIT_H),
            "Fi&nd what:",
        ),
        Item::edit(
            QUERY,
            Rect::new(FIND_EDIT_X, ROW_Y[0], FIND_EDIT_W, EDIT_H),
            &f.query,
            true,
        ),
        button(
            FIND_NEXT,
            FIND_BTN_X,
            FIND_BTN_Y[0],
            "&Find Next",
            live,
            true,
        ),
        button(CANCEL, FIND_BTN_X, FIND_BTN_Y[1], "Cancel", true, false),
        match_case_row(FIND_CHECK_XY, f.match_case),
        Item::group("dirgroup", GROUP, "Direction"),
        Item::radio(UP, RADIO_XY[0].0, RADIO_XY[0].1, "&Up", "dir").checked(!f.down),
        Item::radio(DOWN, RADIO_XY[1].0, RADIO_XY[1].1, "&Down", "dir").checked(f.down),
    ])
    .set_in(DialogFont::ShellDlg)
    .labels_at_ink()
}

pub fn replace_form(f: &ReplaceFields) -> Form {
    let live = !f.query.is_empty();
    Form::new(vec![
        Item::label(
            "querylabel",
            Rect::new(LABEL_X, ROW_Y[0], REPLACE_EDIT_X - LABEL_X, EDIT_H),
            "Fi&nd what:",
        ),
        Item::edit(
            QUERY,
            Rect::new(REPLACE_EDIT_X, ROW_Y[0], REPLACE_EDIT_W, EDIT_H),
            &f.query,
            true,
        ),
        Item::label(
            "withlabel",
            Rect::new(LABEL_X, ROW_Y[1], REPLACE_EDIT_X - LABEL_X, EDIT_H),
            "Re&place with:",
        ),
        Item::edit(
            WITH,
            Rect::new(REPLACE_EDIT_X, ROW_Y[1], REPLACE_EDIT_W, EDIT_H),
            &f.with,
            true,
        ),
        button(
            FIND_NEXT,
            REPLACE_BTN_X,
            REPLACE_BTN_Y[0],
            "&Find Next",
            live,
            true,
        ),
        button(
            REPLACE,
            REPLACE_BTN_X,
            REPLACE_BTN_Y[1],
            "&Replace",
            live,
            false,
        ),
        button(
            REPLACE_ALL,
            REPLACE_BTN_X,
            REPLACE_BTN_Y[2],
            "Replace &All",
            live,
            false,
        ),
        button(
            CANCEL,
            REPLACE_BTN_X,
            REPLACE_BTN_Y[3],
            "Cancel",
            true,
            false,
        ),
        match_case_row(REPLACE_CHECK_XY, f.match_case),
    ])
    .set_in(DialogFont::ShellDlg)
    .labels_at_ink()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_and_replace_fit_their_text_under_every_interface() {
        let over = crate::interfaces::form_overflows(|_, _, _| {
            let find = find_form(&FindFields {
                query: "hello".into(),
                match_case: true,
                down: true,
            });
            let replace = replace_form(&ReplaceFields {
                query: "hello".into(),
                with: "world".into(),
                match_case: false,
            });
            vec![("Find".into(), find), ("Replace".into(), replace)]
        });
        assert!(over.is_empty(), "{}", over.join("\n"));
    }

    fn find_sample() -> FindFields {
        FindFields {
            query: "hello".into(),
            match_case: false,
            down: true,
        }
    }
    fn replace_sample() -> ReplaceFields {
        ReplaceFields {
            query: "hello".into(),
            with: "world".into(),
            match_case: false,
        }
    }

    #[test]
    fn find_lays_out_on_its_grid() {
        const EXPECTED_QUERY: (i32, i32, i32, i32) = (71, 12, 192, 20);
        const EXPECTED_FIND_NEXT: (i32, i32) = (273, 9);
        const EXPECTED_CANCEL: (i32, i32) = (273, 38);
        let f = find_form(&find_sample());
        let r = |id: &str| f.item(id).expect("the item exists").rect;
        let q = r(QUERY);
        assert_eq!((q.x, q.y, q.width, q.height), EXPECTED_QUERY);
        assert_eq!((r(FIND_NEXT).x, r(FIND_NEXT).y), EXPECTED_FIND_NEXT);
        assert_eq!((r(CANCEL).x, r(CANCEL).y), EXPECTED_CANCEL);
        assert_eq!(r(FIND_NEXT).width, crate::classic::widgets::BUTTON_W);
        assert_eq!(r(FIND_NEXT).height, crate::classic::widgets::BUTTON_H);
    }

    fn drawn(f: &Form, size: (i32, i32)) -> (crate::raster::Canvas, crate::theme::Theme) {
        use crate::theme::{FontConfig, Theme};
        let theme = Theme::default();
        let text = crate::text::TextRenderer::load(&FontConfig::default());
        let mut c = crate::raster::Canvas::new(size.0 as u32, size.1 as u32);
        c.fill_rect(c.bounds(), theme.colors.button_face);
        crate::controls::form::draw(&mut c, &text, &theme.paint(), f, false, 11);
        (c, theme)
    }

    #[test]
    fn find_draws_its_rows_from_the_client_top() {
        const EXPECTED_CLIENT: (i32, i32) = (354, 102);
        assert_eq!(FIND_CLIENT, EXPECTED_CLIENT);
        let (c, theme) = drawn(&find_form(&find_sample()), FIND_CLIENT);
        let cs = &theme.colors;
        let (face, shadow, light) = (
            cs.button_face.argb(),
            cs.button_shadow.argb(),
            cs.button_hilight.argb(),
        );
        assert_eq!(c.get(150, 11), face, "the row over the field is face");
        assert_eq!(
            c.get(150, 12),
            shadow,
            "the field's border starts on row 12"
        );
        assert_eq!(c.get(150, 31), light, "the field's last row is 31");
        assert_eq!(c.get(150, 32), face);
        assert_eq!(c.get(300, 8), face, "the row over Find Next is face");
        assert_ne!(c.get(300, 9), face, "Find Next's ring starts on row 9");
        assert_eq!(c.get(10, 71), face, "the row over the check box is face");
        assert_eq!(c.get(10, 72), shadow, "the check box starts on row 72");
        assert_eq!(c.get(200, 87), shadow, "the group's etched bottom");
        assert_eq!(c.get(200, 88), light);
        assert_eq!(c.get(200, 89), face);
        assert_eq!(c.get(261, 60), shadow, "the group's etched right edge");
        assert_eq!(c.get(262, 60), light);
        assert_eq!(c.get(263, 60), face, "the group ends at column 263");
    }

    #[test]
    fn replace_draws_its_rows_from_the_client_top() {
        const EXPECTED_CLIENT: (i32, i32) = (345, 154);
        assert_eq!(REPLACE_CLIENT, EXPECTED_CLIENT);
        let (c, theme) = drawn(&replace_form(&replace_sample()), REPLACE_CLIENT);
        let cs = &theme.colors;
        let (face, shadow) = (cs.button_face.argb(), cs.button_shadow.argb());
        for top in [12, 40] {
            assert_eq!(c.get(150, top - 1), face, "the row over the field at {top}");
            assert_eq!(
                c.get(150, top),
                shadow,
                "a field's border starts on row {top}"
            );
        }
        for top in [8, 35, 63, 90] {
            assert_eq!(
                c.get(290, top - 1),
                face,
                "the row over the button at {top}"
            );
            assert_ne!(c.get(290, top), face, "a button's ring starts on row {top}");
        }
        assert_eq!(c.get(12, 104), face, "the row over the check box is face");
        assert_eq!(c.get(12, 105), shadow, "the check box starts on row 105");
    }

    #[test]
    fn only_find_has_a_direction_group() {
        let find = find_form(&find_sample());
        assert!(find.item(UP).is_some(), "Find has an Up radio");
        assert!(find.item(DOWN).is_some(), "Find has a Down radio");
        let replace = replace_form(&replace_sample());
        assert!(replace.item(UP).is_none(), "Replace has no Direction group");
        assert!(replace.item(DOWN).is_none());
        assert!(replace.item(WITH).is_some(), "Replace has a second field");
        assert!(find.item(WITH).is_none(), "Find has only one field");
    }

    #[test]
    fn replace_stacks_four_buttons() {
        const EXPECTED_BUTTON_X: i32 = 261;
        const EXPECTED_BUTTON_Y: [i32; 4] = [8, 35, 63, 90];
        let f = replace_form(&replace_sample());
        let r = |id: &str| f.item(id).expect("the item exists").rect;
        for (id, want) in [FIND_NEXT, REPLACE, REPLACE_ALL, CANCEL]
            .iter()
            .zip(EXPECTED_BUTTON_Y)
        {
            assert_eq!(r(id).x, EXPECTED_BUTTON_X, "{id} is in the button column");
            assert_eq!(r(id).y, want, "{id} is on row {want}");
        }
    }

    #[test]
    fn an_empty_query_greys_the_action_buttons() {
        let enabled = |f: &Form, id: &str| match &f.item(id).expect("the item exists").control {
            Control::Button { enabled, .. } => *enabled,
            other => panic!("{id} is {other:?}, not a button"),
        };
        let mut empty = find_sample();
        empty.query = String::new();
        let f = find_form(&empty);
        assert!(!enabled(&f, FIND_NEXT), "Find Next is greyed with no query");
        assert!(enabled(&f, CANCEL), "Cancel is always live");
        let g = find_form(&find_sample());
        assert!(enabled(&g, FIND_NEXT), "a query enables Find Next");

        let mut bare = replace_sample();
        bare.query = String::new();
        let h = replace_form(&bare);
        for id in [FIND_NEXT, REPLACE, REPLACE_ALL] {
            assert!(!enabled(&h, id), "{id} is greyed with no query");
        }
    }

    #[test]
    fn find_next_is_the_default_button() {
        let default = |f: &Form| f.default_button().map(|i| f.items[i].id);
        assert_eq!(default(&find_form(&find_sample())), Some(FIND_NEXT));
        assert_eq!(default(&replace_form(&replace_sample())), Some(FIND_NEXT));
    }

    #[test]
    fn the_fields_carry_their_values() {
        let value = |f: &Form, id: &str| match &f.item(id).expect("the item exists").control {
            Control::Edit { edit, .. } => edit.text.clone(),
            other => panic!("{id} is {other:?}, not an edit"),
        };
        let f = replace_form(&replace_sample());
        assert_eq!(value(&f, QUERY), "hello");
        assert_eq!(value(&f, WITH), "world");
    }
}
