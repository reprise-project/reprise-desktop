//! The Rename dialog for several items: find and replace or a numbered base name, the
//! extensions kept or not, and a preview of every old and new name. The host plans the names
//! from [`fields`] and hands the rows back through [`set_preview`].

use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{Control, Form, Item};
use crate::controls::listbox::ListBox;
use crate::raster::Rect;

pub const REPLACE: &str = "replace";
pub const FIND: &str = "find";
pub const WITH: &str = "with";
pub const IGNORE_CASE: &str = "ignorecase";
pub const NUMBER: &str = "number";
pub const BASE: &str = "base";
pub const KEEP_EXT: &str = "keepext";
pub const PREVIEW: &str = "preview";
pub const STATUS: &str = "status";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";

pub const CLIENT: (i32, i32) = (360, 354);

const MARGIN: i32 = 11;
const INDENT: i32 = 29;
const FIELD_X: i32 = 108;
const FIELD_W: i32 = CLIENT.0 - MARGIN - FIELD_X;
const EDIT_H: i32 = 20;
const LIST_Y: i32 = 192;
const LIST_H: i32 = 108;
const STATUS_Y: i32 = 304;
const BUTTONS_Y: i32 = 320;
const BUTTON_GAP: i32 = 6;

/// What the dialog's controls hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fields {
    pub number: bool,
    pub find: String,
    pub with: String,
    pub ignore_case: bool,
    pub base: String,
    pub keep_ext: bool,
}

/// One preview line; `problem` says why the new name cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub old: String,
    pub new: String,
    pub problem: Option<String>,
}

fn field(label_id: &'static str, label: &str, id: &'static str, y: i32, text: &str) -> [Item; 2] {
    [
        Item::label_at(label_id, INDENT, y + 3, FIELD_X - INDENT - 4, label),
        Item::edit(id, Rect::new(FIELD_X, y, FIELD_W, EDIT_H), text, true),
    ]
}

/// The dialog as `f` sets it, its preview empty and OK greyed until [`set_preview`].
pub fn form(f: &Fields) -> Form {
    let right = CLIENT.0 - MARGIN;
    let mut items =
        vec![Item::radio(REPLACE, MARGIN, 11, "&Replace text", "scheme").checked(!f.number)];
    items.extend(field("findlabel", "Fi&nd:", FIND, 29, &f.find));
    items.extend(field("withlabel", "Replace &with:", WITH, 54, &f.with));
    items.push(Item::check(IGNORE_CASE, FIELD_X, 80, "&Ignore case").checked(f.ignore_case));
    items.push(Item::radio(NUMBER, MARGIN, 104, "N&umber the names", "scheme").checked(f.number));
    items.extend(field("baselabel", "&Base name:", BASE, 122, &f.base));
    items.push(Item::check(KEEP_EXT, MARGIN, 152, "Keep &extensions").checked(f.keep_ext));
    items.push(Item::label_at(
        "previewlabel",
        MARGIN,
        176,
        200,
        "&Preview:",
    ));
    items.push(Item::new(
        PREVIEW,
        Rect::new(MARGIN, LIST_Y, right - MARGIN, LIST_H),
        Control::ListBox {
            list: ListBox::new(Vec::new(), None),
            icons: Vec::new(),
        },
    ));
    items.push(Item::label_at(STATUS, MARGIN, STATUS_Y, right - MARGIN, ""));
    let cancel_x = right - BUTTON_W;
    let ok_x = cancel_x - BUTTON_GAP - BUTTON_W;
    items.push(Item::button(
        OK,
        Rect::new(ok_x, BUTTONS_Y, BUTTON_W, BUTTON_H),
        "OK",
        false,
        true,
    ));
    items.push(Item::button(
        CANCEL,
        Rect::new(cancel_x, BUTTONS_Y, BUTTON_W, BUTTON_H),
        "Cancel",
        true,
        false,
    ));
    let mut form = Form::new(items);
    follow(&mut form);
    form
}

pub fn fields(form: &Form) -> Fields {
    Fields {
        number: form.checked(NUMBER) == Some(true),
        find: form.edit_text(FIND).to_string(),
        with: form.edit_text(WITH).to_string(),
        ignore_case: form.checked(IGNORE_CASE) == Some(true),
        base: form.edit_text(BASE).to_string(),
        keep_ext: form.checked(KEEP_EXT) == Some(true),
    }
}

/// Greys the fields of the scheme not picked.
pub fn follow(form: &mut Form) {
    let number = form.checked(NUMBER) == Some(true);
    for id in ["findlabel", FIND, "withlabel", WITH, IGNORE_CASE] {
        form.set_enabled(id, !number);
    }
    for id in ["baselabel", BASE] {
        form.set_enabled(id, number);
    }
}

/// Lists `rows` as `old -> new`, each problem dimmed after its row; the line under the list
/// counts the problems. OK is live only with `ok`.
pub fn set_preview(form: &mut Form, rows: &[Row], ok: bool) {
    if let Some(Control::ListBox { list, .. }) = form.item_mut(PREVIEW).map(|i| &mut i.control) {
        list.items = rows
            .iter()
            .map(|r| format!("{}  \u{2192}  {}", r.old, r.new))
            .collect();
        list.detail = rows
            .iter()
            .map(|r| r.problem.clone().unwrap_or_default())
            .collect();
        list.greyed = rows.iter().map(|r| r.problem.is_some()).collect();
        list.selected = rows.iter().position(|r| r.problem.is_some());
        list.top = list.top.min(rows.len().saturating_sub(1));
    }
    let bad = rows.iter().filter(|r| r.problem.is_some()).count();
    let status = match bad {
        0 => String::new(),
        1 => "1 new name cannot be used.".to_string(),
        n => format!("{n} new names cannot be used."),
    };
    form.set_text(STATUS, &status);
    form.set_enabled(OK, ok);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::Theme;

    fn sample() -> Fields {
        Fields {
            number: false,
            find: "IMG_".into(),
            with: "Holiday ".into(),
            ignore_case: true,
            base: "Photo".into(),
            keep_ext: true,
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            Row {
                old: "IMG_001.jpg".into(),
                new: "Holiday 001.jpg".into(),
                problem: None,
            },
            Row {
                old: "img_002.jpg".into(),
                new: "Holiday 002.jpg".into(),
                problem: Some("in use".into()),
            },
        ]
    }

    #[test]
    fn the_form_reads_back_what_it_was_built_from() {
        let f = form(&sample());
        assert_eq!(fields(&f), sample());
        let numbered = Fields {
            number: true,
            ..sample()
        };
        assert_eq!(fields(&form(&numbered)), numbered);
    }

    #[test]
    fn the_scheme_not_picked_is_greyed() {
        let enabled = |f: &Form, id: &str| match &f.item(id).expect("the item exists").control {
            Control::Edit { enabled, .. }
            | Control::Check { enabled, .. }
            | Control::Label { enabled, .. } => *enabled,
            other => panic!("{id} is {other:?}"),
        };
        let mut f = form(&sample());
        assert!(enabled(&f, FIND) && enabled(&f, WITH) && enabled(&f, IGNORE_CASE));
        assert!(!enabled(&f, BASE) && !enabled(&f, "baselabel"));
        f.set_check(REPLACE, false);
        f.set_check(NUMBER, true);
        follow(&mut f);
        assert!(!enabled(&f, FIND) && !enabled(&f, "withlabel") && !enabled(&f, IGNORE_CASE));
        assert!(enabled(&f, BASE));
        assert!(enabled(&f, KEEP_EXT), "both schemes keep extensions");
    }

    #[test]
    fn the_preview_lists_rows_flags_problems_and_gates_ok() {
        let mut f = form(&sample());
        let ok = |f: &Form| match &f.item(OK).expect("OK").control {
            Control::Button { enabled, .. } => *enabled,
            _ => unreachable!(),
        };
        assert!(!ok(&f), "OK waits for a preview");
        set_preview(&mut f, &rows(), false);
        let Control::ListBox { list, .. } = &f.item(PREVIEW).expect("the list").control else {
            unreachable!()
        };
        assert_eq!(list.items[0], "IMG_001.jpg  \u{2192}  Holiday 001.jpg");
        assert_eq!(list.detail, ["", "in use"]);
        assert_eq!(list.greyed, [false, true]);
        assert_eq!(list.selected, Some(1), "the first problem is picked out");
        assert!(!ok(&f));
        let Control::Label { text, .. } = &f.item(STATUS).expect("status").control else {
            unreachable!()
        };
        assert_eq!(text, "1 new name cannot be used.");
        set_preview(&mut f, &rows()[..1], true);
        assert!(ok(&f));
    }

    #[test]
    fn the_dialog_fits_its_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, _, _| {
            let mut f = form(&sample());
            set_preview(&mut f, &rows(), false);
            vec![("Rename".into(), f)]
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

    fn looks() -> Vec<(String, Theme)> {
        let mut out = vec![("Classic".to_string(), Theme::default())];
        out.push((
            "Blue".to_string(),
            interfaces::blue_theme(crate::blue::Variant::ALL[0]),
        ));
        for mode in [crate::glass::Mode::Light, crate::glass::Mode::Dark] {
            let g = crate::glass::Settings {
                mode,
                ..Default::default()
            };
            let t = interfaces::theme_with(Interface::Glass, Default::default(), &g);
            out.push((format!("Glass {mode:?}"), t));
        }
        out
    }

    fn drawn(theme: &Theme, f: &Form, s: i32) -> Canvas {
        let theme = theme.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut c = Canvas::new((CLIENT.0 * s) as u32, (CLIENT.1 * s) as u32);
        c.fill_rect(c.bounds(), p.cs.button_face);
        crate::controls::form::draw(&mut c, &text, &p, f, true, theme.fonts.size_px);
        c
    }

    #[test]
    fn rename_dialog_pins_at_scale_1_and_2() {
        crate::pins::fonts();
        let mut replace = form(&sample());
        set_preview(&mut replace, &rows(), false);
        let mut number = form(&Fields {
            number: true,
            ..sample()
        });
        let ok = vec![Row {
            problem: None,
            ..rows()[1].clone()
        }];
        set_preview(&mut number, &ok, true);
        let mut got = Vec::new();
        for (name, theme) in looks() {
            for (state, f) in [("replace with a clash", &replace), ("number", &number)] {
                for s in [1, 2] {
                    got.push((format!("{name} {state} {s}"), fnv(&drawn(&theme, f, s))));
                }
            }
        }
        crate::pins::check("batch_rename", &got);
    }
}
