//! Favorites' two boxes. Add Favorite is a [`NameBox`] titled [`ADD_TITLE`]. Organize Favorites
//! lists the favorites with Delete, Move Up and Move Down beside the list, and under it the
//! picked one's name in an edit with Rename, its address, and a greyed note when Explorer cannot
//! open it. The host keeps the rows and hands them back through [`set_rows`] whenever the
//! bookmarks change.

use crate::classic::widgets::{BUTTON_H, BUTTON_W, LINE_H};
use crate::controls::form::{Control, Form, Item};
use crate::controls::listbox::ListBox;
use crate::raster::Rect;

pub use crate::controls::namebox::NameBox;

pub const ADD_TITLE: &str = "Add Favorite";
pub const ORGANIZE_TITLE: &str = "Organize Favorites";

pub const LIST: &str = "favorites";
pub const NAME: &str = "name";
pub const ADDRESS: &str = "address";
pub const NOTE: &str = "note";
pub const RENAME: &str = "rename";
pub const DELETE: &str = "delete";
pub const UP: &str = "up";
pub const DOWN: &str = "down";
pub const CLOSE: &str = "close";

pub const CLIENT: (i32, i32) = (360, 290);

const MARGIN: i32 = 11;
const LIST_Y: i32 = 30;
const LIST_H: i32 = 150;
const BUTTON_X: i32 = CLIENT.0 - MARGIN - BUTTON_W;
const LIST_W: i32 = BUTTON_X - 8 - MARGIN;
const BUTTON_STEP: i32 = BUTTON_H + 4;
const FIELD_X: i32 = 67;
const NAME_Y: i32 = 190;
const EDIT_H: i32 = 20;
const ADDRESS_Y: i32 = 218;
const NOTE_Y: i32 = 236;

/// Whether a favorite opens, and why not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Opens,
    /// A folder that is not there, an unplugged drive's.
    Missing,
    /// An address of a kind Explorer has no way into.
    Unsupported,
}

impl State {
    fn note(self) -> &'static str {
        match self {
            State::Opens => "",
            State::Missing => "This folder is not there. Its drive may be unplugged.",
            State::Unsupported => "Explorer cannot open this kind of address.",
        }
    }
}

/// One favorite in the list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// The folder or URI, as the Address line shows it.
    pub address: String,
    pub state: State,
}

/// Organize Favorites over `rows`, `selected` picked.
pub fn organize(rows: &[Row], selected: Option<usize>) -> Form {
    let button = |id, i: i32, text| {
        let r = Rect::new(BUTTON_X, LIST_Y + i * BUTTON_STEP, BUTTON_W, BUTTON_H);
        Item::button(id, r, text, true, false)
    };
    let items = vec![
        Item::label_at(
            "intro",
            MARGIN,
            11,
            CLIENT.0 - 2 * MARGIN,
            "&Favorites, shared with GTK programs' file dialogs:",
        ),
        Item::new(
            LIST,
            Rect::new(MARGIN, LIST_Y, LIST_W, LIST_H),
            Control::ListBox {
                list: ListBox::new(Vec::new(), None),
                icons: Vec::new(),
            },
        ),
        button(DELETE, 0, "&Delete"),
        button(UP, 1, "Move &Up"),
        button(DOWN, 2, "Move Do&wn"),
        Item::label_at("name_l", MARGIN, NAME_Y + 4, FIELD_X - MARGIN - 4, "&Name:"),
        Item::edit(
            NAME,
            Rect::new(FIELD_X, NAME_Y, MARGIN + LIST_W - FIELD_X, EDIT_H),
            "",
            false,
        ),
        Item::button(
            RENAME,
            Rect::new(BUTTON_X, NAME_Y - 1, BUTTON_W, BUTTON_H),
            "Re&name",
            false,
            false,
        ),
        Item::label_at(
            "address_l",
            MARGIN,
            ADDRESS_Y,
            FIELD_X - MARGIN - 4,
            "Address:",
        ),
        Item::label_at(ADDRESS, FIELD_X, ADDRESS_Y, CLIENT.0 - MARGIN - FIELD_X, ""),
        Item::new(
            NOTE,
            Rect::new(FIELD_X, NOTE_Y, CLIENT.0 - MARGIN - FIELD_X, LINE_H),
            Control::GreyedLines(Vec::new()),
        ),
        Item::button(
            CLOSE,
            Rect::new(BUTTON_X, CLIENT.1 - MARGIN - BUTTON_H, BUTTON_W, BUTTON_H),
            "Close",
            true,
            true,
        ),
    ];
    let mut form = Form::new(items);
    set_rows(&mut form, rows, selected);
    form.focus_id(LIST);
    form
}

pub fn selected(form: &Form) -> Option<usize> {
    match form.item(LIST).map(|i| &i.control) {
        Some(Control::ListBox { list, .. }) => list.selected,
        _ => None,
    }
}

/// Lists `rows`, `selected` picked (clamped to the last row), and shows that row as
/// [`show_selected`] does.
pub fn set_rows(form: &mut Form, rows: &[Row], selected: Option<usize>) {
    if let Some(Control::ListBox { list, .. }) = form.item_mut(LIST).map(|i| &mut i.control) {
        list.items = rows.iter().map(|r| r.label.clone()).collect();
        list.greyed = rows.iter().map(|r| r.state != State::Opens).collect();
        list.selected = selected.map(|i| i.min(rows.len().saturating_sub(1)));
        if rows.is_empty() {
            list.selected = None;
        }
        list.top = list.top.min(rows.len().saturating_sub(1));
    }
    show_selected(form, rows);
}

/// The picked row's name, address and note, and the buttons live for it: Move Up off the first
/// row, Move Down off the last, all of them off with nothing picked.
pub fn show_selected(form: &mut Form, rows: &[Row]) {
    let at = selected(form);
    let row = at.and_then(|i| rows.get(i));
    form.show_edit_text(NAME, row.map_or("", |r| r.label.as_str()));
    form.select_edit_text(NAME);
    let address = row.map_or(String::new(), |r| crate::text::escape_mnemonic(&r.address));
    form.set_text(ADDRESS, &address);
    form.set_text(NOTE, row.map_or("", |r| r.state.note()));
    form.set_enabled(NAME, row.is_some());
    form.set_enabled("name_l", row.is_some());
    form.set_enabled(DELETE, row.is_some());
    form.set_enabled(UP, at.is_some_and(|i| i > 0));
    form.set_enabled(DOWN, at.is_some_and(|i| i + 1 < rows.len()));
    follow_name(form, rows);
}

/// Rename is live while the name typed is not blank and not the picked row's.
pub fn follow_name(form: &mut Form, rows: &[Row]) {
    let typed = form.edit_text(NAME).trim().to_string();
    let row = selected(form).and_then(|i| rows.get(i));
    let live = row.is_some_and(|r| !typed.is_empty() && typed != r.label);
    form.set_enabled(RENAME, live);
}

pub fn typed_name(form: &Form) -> &str {
    form.edit_text(NAME).trim()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interface::Interface;
    use crate::interfaces;
    use crate::raster::Canvas;
    use crate::text::TextRenderer;
    use crate::theme::Theme;

    fn rows() -> Vec<Row> {
        vec![
            Row {
                label: "Projects".into(),
                address: "/home/u/Projects".into(),
                state: State::Opens,
            },
            Row {
                label: "Server".into(),
                address: "sftp://h/srv/share".into(),
                state: State::Opens,
            },
            Row {
                label: "Recent".into(),
                address: "recent:///".into(),
                state: State::Unsupported,
            },
        ]
    }

    fn enabled(f: &Form, id: &str) -> bool {
        match &f.item(id).expect("the item").control {
            Control::Button { enabled, .. }
            | Control::Edit { enabled, .. }
            | Control::Label { enabled, .. } => *enabled,
            other => panic!("{id} is {other:?}"),
        }
    }

    fn label(f: &Form, id: &str) -> String {
        match &f.item(id).expect("the item").control {
            Control::Label { text, .. } => text.clone(),
            Control::GreyedLines(l) => l.join("\n"),
            other => panic!("{id} is {other:?}"),
        }
    }

    #[test]
    fn the_picked_row_fills_the_fields_and_gates_the_moves() {
        let rows = rows();
        let f = organize(&rows, Some(0));
        assert_eq!(f.edit_text(NAME), "Projects");
        assert_eq!(label(&f, ADDRESS), "/home/u/Projects");
        assert_eq!(label(&f, NOTE), "");
        assert!(!enabled(&f, UP) && enabled(&f, DOWN) && enabled(&f, DELETE));
        assert!(!enabled(&f, RENAME), "the name is unchanged");
        let f = organize(&rows, Some(2));
        assert!(enabled(&f, UP) && !enabled(&f, DOWN));
        assert_eq!(
            label(&f, NOTE),
            "Explorer cannot open this kind of address."
        );
        let Control::ListBox { list, .. } = &f.item(LIST).unwrap().control else {
            unreachable!()
        };
        assert_eq!(list.greyed, [false, false, true]);
    }

    #[test]
    fn an_ampersand_in_the_address_draws_as_itself() {
        let rows = [Row {
            label: "Tom".into(),
            address: "/home/u/Tom & Jerry".into(),
            state: State::Opens,
        }];
        let f = organize(&rows, Some(0));
        assert_eq!(
            crate::text::split_mnemonic(&label(&f, ADDRESS)),
            ("/home/u/Tom & Jerry".to_string(), None)
        );
    }

    #[test]
    fn nothing_picked_greys_every_action() {
        let f = organize(&rows(), None);
        for id in [DELETE, UP, DOWN, RENAME, NAME] {
            assert!(!enabled(&f, id), "{id}");
        }
        assert!(enabled(&f, CLOSE));
        let f = organize(&[], Some(3));
        assert_eq!(selected(&f), None, "an empty list picks nothing");
    }

    #[test]
    fn rename_lights_for_a_new_name_only() {
        let rows = rows();
        let mut f = organize(&rows, Some(1));
        if let Some(Control::Edit { edit, .. }) = f.item_mut(NAME).map(|i| &mut i.control) {
            edit.set_selected("  Build box ");
        }
        follow_name(&mut f, &rows);
        assert!(enabled(&f, RENAME));
        assert_eq!(typed_name(&f), "Build box");
        if let Some(Control::Edit { edit, .. }) = f.item_mut(NAME).map(|i| &mut i.control) {
            edit.set_selected("   ");
        }
        follow_name(&mut f, &rows);
        assert!(!enabled(&f, RENAME));
    }

    #[test]
    fn new_rows_keep_the_pick_in_range() {
        let rows = rows();
        let mut f = organize(&rows, Some(2));
        set_rows(&mut f, &rows[..2], Some(2));
        assert_eq!(selected(&f), Some(1));
        assert_eq!(f.edit_text(NAME), "Server");
    }

    #[test]
    fn the_boxes_fit_their_text_under_every_interface() {
        let over = interfaces::form_overflows(|_, _, _| {
            vec![
                (ORGANIZE_TITLE.into(), organize(&rows(), Some(2))),
                (ADD_TITLE.into(), NameBox::new("Projects").form),
            ]
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

    fn drawn(theme: &Theme, f: &Form, client: (i32, i32), s: i32) -> Canvas {
        let theme = theme.scaled(s);
        let text = TextRenderer::load(&theme.fonts);
        let p = theme.paint();
        let mut c = Canvas::new((client.0 * s) as u32, (client.1 * s) as u32);
        c.fill_rect(c.bounds(), p.cs.button_face);
        crate::controls::form::draw(&mut c, &text, &p, f, true, theme.fonts.size_px);
        c
    }

    #[test]
    fn favorites_boxes_pin_at_scale_1_and_2() {
        crate::pins::fonts();
        let organize_unsupported = organize(&rows(), Some(2));
        let organize_first = organize(&rows(), Some(0));
        let add = NameBox::new("Projects").form;
        let states = [
            (
                "organize, an address Explorer cannot open",
                &organize_unsupported,
                CLIENT,
            ),
            ("organize, the first row", &organize_first, CLIENT),
            ("add", &add, crate::controls::namebox::CLIENT),
        ];
        let mut got = Vec::new();
        for (name, theme) in looks() {
            for (state, f, client) in states {
                for s in [1, 2] {
                    let c = drawn(&theme, f, client, s);
                    got.push((format!("{name} {state} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("favorites", &got);
    }
}
