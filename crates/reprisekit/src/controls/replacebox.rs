//! The replace prompt: a copy or a move is about to land on a name the
//! folder already holds. Confirm File Replace shows the file that is
//! there and the one coming in, each with its icon, size and date;
//! Confirm Folder Replace says the folders merge. A batch of several
//! items answers Yes, Yes to All, No or Cancel; a single item Yes or No.
//!
//! The model is [`Fields`]; [`form`] lays it out over [`client`]'s size
//! as a [`Form`], which draws ([`draw`]), hit-tests (`Form::hit`) and adds
//! its screen reader nodes (`Form::access`) like any other form.
//! [`choice`] reads a form event as the button it stands for.

use crate::classic::widgets::{BUTTON_H, LINE_H};
use crate::controls::form::{self, Event, Form, Item};
use crate::look::Paint;
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::TextUnits;

pub const YES: &str = "yes";
pub const YES_ALL: &str = "yes_all";
pub const NO: &str = "no";
pub const CANCEL: &str = "cancel";

/// One side of Confirm File Replace: the file already there or the one
/// coming in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Side {
    /// "512 bytes", "1.23 KB".
    pub size: String,
    /// "modified: ...".
    pub modified: String,
    /// The file's 32 px icon; `None` leaves the slot empty.
    pub icon: Option<Canvas>,
}

/// What the prompt asks about.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject {
    /// A file onto a file.
    File {
        existing: Box<Side>,
        incoming: Box<Side>,
    },
    /// A folder onto a folder: the two merge.
    Folder,
}

/// The prompt's model.
#[derive(Clone, Debug, PartialEq)]
pub struct Fields {
    /// The name both share.
    pub name: String,
    /// For a file inside a folder merge, the folder holding it, from the merged folder down
    /// (`Reports/2024`); `None` for an item of the put itself.
    pub folder: Option<String>,
    pub subject: Subject,
    /// The 32 px picture at the top left; `None` leaves the slot empty.
    pub icon: Option<Canvas>,
    /// The batch holds more than this one item, so Yes to All and Cancel
    /// join Yes and No.
    pub many: bool,
}

impl Fields {
    /// The caption the host puts on the frame.
    pub fn title(&self) -> &'static str {
        match self.subject {
            Subject::File { .. } => "Confirm File Replace",
            Subject::Folder => "Confirm Folder Replace",
        }
    }
}

/// A button on the prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Yes,
    YesToAll,
    No,
    Cancel,
}

/// The button a form event stands for: Enter presses Yes, Escape (and
/// the frame's close box, which the host sends as `Event::Cancel`) is
/// Cancel. Anything else is no answer yet.
pub fn choice(ev: &Event) -> Option<Choice> {
    match ev {
        Event::Default | Event::Pressed(YES) => Some(Choice::Yes),
        Event::Pressed(YES_ALL) => Some(Choice::YesToAll),
        Event::Pressed(NO) => Some(Choice::No),
        Event::Cancel | Event::Pressed(CANCEL) => Some(Choice::Cancel),
        _ => None,
    }
}

// Client coordinates.

const ICON: i32 = 32;
/// The picture's top-left.
const ICON_XY: (i32, i32) = (14, 14);
const TEXT_X: i32 = 62;
/// The first line's row.
const TEXT_Y: i32 = 16;
/// The margin right of the widest line, and under the buttons.
const MARGIN: i32 = 11;
/// The buttons: 80 px wide and 7 apart, ending `button_right` from the
/// right edge.
const BUTTON_W: i32 = 80;
const BUTTON_GAP: i32 = 7;

/// Confirm File Replace's rows: the question, the file already there,
/// "with this one?", the file coming in, the buttons.
const FILE_QUESTION_Y: i32 = 57;
const FILE_EXISTING_Y: i32 = 80;
const FILE_WITH_Y: i32 = 122;
const FILE_INCOMING_Y: i32 = 145;
const FILE_BUTTON_Y: i32 = 195;
/// A file's icon, and its size and date lines right of it, the size
/// 3 px below the icon's top.
const SIDE_ICON_X: i32 = 77;
const SIDE_TEXT_X: i32 = 119;
const SIDE_TEXT_DY: i32 = 3;
/// The narrowest Confirm File Replace, and its buttons' right margin.
const FILE_MIN_W: i32 = 420;
const FILE_BUTTON_RIGHT: i32 = 11;

/// Confirm Folder Replace: its paragraph starts after one blank row and
/// wraps to a 330 px column; the buttons sit 23 px under it.
const FOLDER_PARA_Y: i32 = TEXT_Y + 2 * LINE_H;
const FOLDER_WRAP_W: i32 = 330;
const FOLDER_BUTTON_GAP: i32 = 23;
/// The narrowest Confirm Folder Replace, and its buttons' right margin:
/// the row starts at the text column.
const FOLDER_MIN_W: i32 = 413;
const FOLDER_BUTTON_RIGHT: i32 = FOLDER_MIN_W - TEXT_X - (4 * BUTTON_W + 3 * BUTTON_GAP);

/// The widest the first line runs: the prompt is at most 553 px wide.
const FIRST_MAX_W: i32 = 480;

const MERGE_TEXT: &str = "If files in the existing folder have the same names as files in \
                          the folder you are moving or copying, they will be replaced. Do \
                          you still want to move or copy the folder?";

fn buttons(f: &Fields) -> Vec<(&'static str, &'static str)> {
    if f.many {
        vec![
            (YES, "&Yes"),
            (YES_ALL, "Yes to &All"),
            (NO, "&No"),
            (CANCEL, "Cancel"),
        ]
    } else {
        vec![(YES, "&Yes"), (NO, "&No")]
    }
}

/// The lines of text with their rows, each file block's row, and the
/// buttons' row.
struct Plan {
    lines: Vec<(String, i32)>,
    /// Each file block's top, existing then incoming.
    sides: Vec<(i32, Side)>,
    button_y: i32,
    min_w: i32,
    button_right: i32,
}

fn plan(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> Plan {
    let what = match f.subject {
        Subject::File { .. } => "file",
        Subject::Folder => "folder",
    };
    let mut lines = vec![(first_line(f, what, text, px, u), TEXT_Y)];
    match &f.subject {
        Subject::File { existing, incoming } => {
            lines.push((
                "Would you like to replace the existing file".into(),
                FILE_QUESTION_Y,
            ));
            lines.push(("with this one?".into(), FILE_WITH_Y));
            Plan {
                lines,
                sides: vec![
                    (FILE_EXISTING_Y, Side::clone(existing)),
                    (FILE_INCOMING_Y, Side::clone(incoming)),
                ],
                button_y: FILE_BUTTON_Y,
                min_w: FILE_MIN_W,
                button_right: FILE_BUTTON_RIGHT,
            }
        }
        Subject::Folder => {
            let mut y = FOLDER_PARA_Y;
            for l in crate::text::wrap(text, MERGE_TEXT, u.len(FOLDER_WRAP_W), px) {
                lines.push((l, y));
                y += LINE_H;
            }
            Plan {
                lines,
                sides: Vec::new(),
                button_y: y + FOLDER_BUTTON_GAP,
                min_w: FOLDER_MIN_W,
                button_right: FOLDER_BUTTON_RIGHT,
            }
        }
    }
}

/// The first line on one row: a long name widens the prompt up to `FIRST_MAX_W`, then the folder
/// loses its start at a slash (".../2024/Q3") and, past that, the name its end.
fn first_line(f: &Fields, what: &str, text: &TextRenderer, px: u32, u: TextUnits) -> String {
    let line = |dir: Option<&str>, name: &str| match dir {
        None => format!("This folder already contains a {what} named '{name}'."),
        Some(dir) => format!("The folder '{dir}' already contains a {what} named '{name}'."),
    };
    let max = u.len(FIRST_MAX_W);
    let full = line(f.folder.as_deref(), &f.name);
    if text.measure(&full, false, px) <= max {
        return full;
    }
    let dots = "...";
    if let Some(dir) = &f.folder {
        let mut room = max - text.measure(&line(Some(dots), &f.name), false, px);
        let mut cut = dir.len();
        for (i, ch) in dir.char_indices().rev() {
            room -= text.advance(ch, false, px);
            if room < 0 {
                break;
            }
            cut = i;
        }
        if cut < dir.len() {
            cut += dir[cut..].find('/').unwrap_or(0);
            return line(Some(&format!("{dots}{}", &dir[cut..])), &f.name);
        }
    }
    let dir = f.folder.as_ref().map(|_| dots);
    let room = max - text.measure(&line(dir, ""), false, px);
    line(dir, &text.ellipsize(&f.name, room, false, px))
}

fn button_row_w(n: usize) -> i32 {
    n as i32 * BUTTON_W + (n as i32 - 1).max(0) * BUTTON_GAP
}

/// The client size the prompt needs at 100%, for text set at `px` that grows by `u`: its own
/// width, or wider for a long name, and tall enough for its rows and buttons.
pub fn client(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> (i32, i32) {
    let p = plan(f, text, px, u);
    let lines = p
        .lines
        .iter()
        .map(|(l, _)| TEXT_X + u.design(text.measure(l, false, px)));
    let facts = p.sides.iter().flat_map(|(_, s)| {
        [&s.size, &s.modified].map(|t| SIDE_TEXT_X + u.design(text.measure(t, false, px)))
    });
    let widest = lines.chain(facts).max().unwrap_or(0);
    let w = (widest + MARGIN)
        .max(TEXT_X + button_row_w(buttons(f).len()) + p.button_right)
        .max(p.min_w);
    (w, p.button_y + BUTTON_H + MARGIN)
}

/// Lay the prompt out over [`client`]'s size. Yes is the default button.
pub fn form(f: &Fields, text: &TextRenderer, px: u32, u: TextUnits) -> Form {
    const LINE_IDS: [&str; 8] = [
        "line0", "line1", "line2", "line3", "line4", "line5", "line6", "line7",
    ];
    const SIDE_IDS: [(&str, &str, &str); 2] = [
        ("existing_icon", "existing_size", "existing_modified"),
        ("incoming_icon", "incoming_size", "incoming_modified"),
    ];
    let (w, _) = client(f, text, px, u);
    let p = plan(f, text, px, u);
    let mut items = Vec::new();
    if let Some(icon) = &f.icon {
        items.push(Item::picture(
            "icon",
            Rect::new(ICON_XY.0, ICON_XY.1, ICON, ICON),
            Some(icon.clone()),
        ));
    }
    for ((line, y), id) in p.lines.iter().zip(LINE_IDS) {
        let line = crate::text::escape_mnemonic(line);
        items.push(Item::label_at(id, TEXT_X, *y, w - TEXT_X - MARGIN, &line));
    }
    for ((y, side), (icon_id, size_id, date_id)) in p.sides.iter().zip(SIDE_IDS) {
        if let Some(icon) = &side.icon {
            items.push(Item::picture(
                icon_id,
                Rect::new(SIDE_ICON_X, *y, ICON, ICON),
                Some(icon.clone()),
            ));
        }
        let (x, ty, tw) = (SIDE_TEXT_X, y + SIDE_TEXT_DY, w - SIDE_TEXT_X - MARGIN);
        items.push(Item::label_at(size_id, x, ty, tw, &side.size));
        items.push(Item::label_at(date_id, x, ty + LINE_H, tw, &side.modified));
    }
    let row = buttons(f);
    let mut x = w - p.button_right - button_row_w(row.len());
    for (i, (id, label)) in row.into_iter().enumerate() {
        items.push(Item::button(
            id,
            Rect::new(x, p.button_y, BUTTON_W, BUTTON_H),
            label,
            true,
            i == 0,
        ));
        x += BUTTON_W + BUTTON_GAP;
    }
    Form::new(items)
}

/// Paint the prompt's `form` on `c`, whose top-left is the client's.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, form: &Form, px: u32) {
    c.fill_rect(c.bounds(), p.cs.button_face);
    form::draw(c, text, p, form, true, px);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{FontConfig, Theme};

    fn rend() -> (TextRenderer, u32) {
        let text = TextRenderer::load(&FontConfig::default());
        (text, Theme::default().fonts.size_px)
    }

    fn side(size: &str) -> Side {
        Side {
            size: size.into(),
            modified: "modified: Monday, September 21, 2026, 4:05:12 PM".into(),
            icon: Some(Canvas::new(32, 32)),
        }
    }

    fn file() -> Fields {
        Fields {
            name: "notes.txt".into(),
            folder: None,
            subject: Subject::File {
                existing: Box::new(side("512 bytes")),
                incoming: Box::new(side("1.23 KB")),
            },
            icon: Some(Canvas::new(32, 32)),
            many: false,
        }
    }

    fn folder() -> Fields {
        Fields {
            name: "Reports".into(),
            folder: None,
            subject: Subject::Folder,
            icon: Some(Canvas::new(32, 32)),
            many: true,
        }
    }

    fn button_ids(f: &Form) -> Vec<&'static str> {
        f.items
            .iter()
            .filter(|i| matches!(i.control, form::Control::Button { .. }))
            .map(|i| i.id)
            .collect()
    }

    /// One item gets Yes and No; a batch adds Yes to All and Cancel.
    /// Enter is Yes and Escape is Cancel either way.
    #[test]
    fn the_buttons_follow_the_batch_and_map_to_answers() {
        let (text, px) = rend();
        let one = form(&file(), &text, px, TextUnits::ONE);
        assert_eq!(button_ids(&one), [YES, NO]);
        let many = form(&folder(), &text, px, TextUnits::ONE);
        assert_eq!(button_ids(&many), [YES, YES_ALL, NO, CANCEL]);
        assert_eq!(many.default_button().map(|i| many.items[i].id), Some(YES));
        assert_eq!(choice(&Event::Default), Some(Choice::Yes));
        assert_eq!(choice(&Event::Pressed(YES_ALL)), Some(Choice::YesToAll));
        assert_eq!(choice(&Event::Pressed(NO)), Some(Choice::No));
        assert_eq!(choice(&Event::Cancel), Some(Choice::Cancel));
        assert_eq!(choice(&Event::Redraw), None);
    }

    /// Confirm File Replace is 420x229: the question, the file already
    /// there, "with this one?" and the file coming in, then Yes and No
    /// 80 px wide and 7 apart, 11 px in from the right and the bottom.
    #[test]
    fn the_file_prompt_lays_out_on_its_grid() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (text, px) = rend();
        assert_eq!(client(&file(), &text, px, TextUnits::ONE), (420, 229));
        let f = form(&file(), &text, px, TextUnits::ONE);
        let r = |id: &str| {
            let r = f.item(id).expect("the item exists").rect;
            (r.x, r.y)
        };
        assert_eq!(r("icon"), (14, 14));
        assert_eq!(r("line0"), (62, 16));
        assert_eq!(r("line1"), (62, 57));
        assert_eq!(r("existing_icon"), (77, 80));
        assert_eq!(r("existing_size"), (119, 83));
        assert_eq!(r("existing_modified"), (119, 96));
        assert_eq!(r("line2"), (62, 122));
        assert_eq!(r("incoming_icon"), (77, 145));
        assert_eq!(r("incoming_size"), (119, 148));
        assert_eq!(r(YES), (242, 195));
        assert_eq!(r(NO), (329, 195));
        assert_eq!(f.item(NO).unwrap().rect.width, 80);
    }

    /// Confirm Folder Replace is 413x138: its paragraph starts two rows
    /// under the first line and wraps to three lines, and the four
    /// buttons start at the text column 23 px under it.
    #[test]
    fn the_folder_prompt_wraps_its_paragraph_over_the_buttons() {
        let (text, px) = rend();
        let fields = folder();
        let f = form(&fields, &text, px, TextUnits::ONE);
        assert!(f.item("existing_size").is_none());
        let rows: Vec<i32> = f
            .items
            .iter()
            .filter(|i| i.id.starts_with("line"))
            .map(|i| i.rect.y)
            .collect();
        assert_eq!(rows[..2], [16, 42]);
        assert_eq!(rows.len(), 4, "the paragraph wraps to three lines");
        let yes = f.item(YES).unwrap().rect;
        assert_eq!(yes.y, rows[rows.len() - 1] + LINE_H + 23);
        assert_eq!(yes.x, 62);
        assert_eq!(client(&fields, &text, px, TextUnits::ONE), (413, 138));
        assert_eq!(fields.title(), "Confirm Folder Replace");
        assert_eq!(file().title(), "Confirm File Replace");
    }

    /// A file inside a folder merge names the folder holding it under the merged one.
    #[test]
    fn a_file_inside_a_merge_names_its_folder() {
        let (text, px) = rend();
        let mut inside = file();
        inside.folder = Some("Reports/2024".into());
        let f = form(&inside, &text, px, TextUnits::ONE);
        let line0 = |f: &Form| match &f.item("line0").expect("the first line").control {
            form::Control::Label { text, .. } => text.clone(),
            other => panic!("a label, not {other:?}"),
        };
        assert_eq!(
            line0(&f),
            "The folder 'Reports/2024' already contains a file named 'notes.txt'."
        );
        assert_eq!(
            line0(&form(&file(), &text, px, TextUnits::ONE)),
            "This folder already contains a file named 'notes.txt'."
        );
    }

    #[test]
    fn an_ampersand_in_the_name_or_folder_shows_and_is_no_access_key() {
        let (text, px) = rend();
        let mut amp = file();
        amp.name = "R&D notes.txt".into();
        amp.folder = Some("Q&A".into());
        let f = form(&amp, &text, px, TextUnits::ONE);
        let form::Control::Label { text: label, .. } = &f.item("line0").unwrap().control else {
            panic!("the first line is a label");
        };
        assert_eq!(
            crate::text::split_mnemonic(label),
            (
                "The folder 'Q&A' already contains a file named 'R&D notes.txt'.".to_string(),
                None
            )
        );
    }

    /// A long name widens the prompt, and the buttons stay at its right.
    #[test]
    fn a_long_name_widens_the_prompt() {
        let (text, px) = rend();
        let mut long = file();
        long.name = "a considerably longer file name than the prompt starts with.txt".into();
        let (w, _) = client(&long, &text, px, TextUnits::ONE);
        assert!(w > 420);
        let f = form(&long, &text, px, TextUnits::ONE);
        assert_eq!(f.item(NO).unwrap().rect.right(), w - 11);
    }

    fn line0(f: &Form) -> String {
        match &f.item("line0").expect("the first line").control {
            form::Control::Label { text, .. } => text.clone(),
            other => panic!("a label, not {other:?}"),
        }
    }

    /// A deep folder stops widening the prompt at 553 px: the folder loses its start, so the
    /// folders nearest the file and the file's name stay.
    #[test]
    fn a_deep_folder_is_elided_from_its_start() {
        let (text, px) = rend();
        for (px, u) in [
            (px, TextUnits::ONE),
            (px * 3 / 2, TextUnits::new(px * 3 / 2, px)),
        ] {
            let mut deep = file();
            deep.folder = Some(format!("{}/2024", ["Reports"; 80].join("/")));
            let (w, _) = client(&deep, &text, px, u);
            assert!(w <= 553, "{w} px wide");
            let line = line0(&form(&deep, &text, px, u));
            assert!(line.starts_with("The folder '.../Reports/"), "{line}");
            assert!(
                line.ends_with("/2024' already contains a file named 'notes.txt'."),
                "{line}"
            );
        }
    }

    /// A name too long for the widest prompt keeps its start and ends in "...".
    #[test]
    fn a_name_past_the_widest_prompt_is_cut_at_its_end() {
        let (text, px) = rend();
        let mut long = file();
        long.name = "a".repeat(400);
        long.folder = Some("Reports".into());
        let (w, _) = client(&long, &text, px, TextUnits::ONE);
        assert!(w <= 553, "{w} px wide");
        let line = line0(&form(&long, &text, px, TextUnits::ONE));
        assert!(
            line.starts_with("The folder '...' already contains a file named 'aaa"),
            "{line}"
        );
        assert!(line.ends_with("a...'."), "{line}");
    }

    /// The pixels: the default ring around Yes, the plain bevel of No,
    /// the size line's ink beside the incoming file, and the empty margin
    /// under the buttons.
    #[test]
    fn the_prompt_draws_its_buttons_and_facts() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (text, px) = rend();
        let theme = Theme::default();
        let cs = &theme.colors;
        let fields = file();
        let (w, h) = client(&fields, &text, px, TextUnits::ONE);
        let f = form(&fields, &text, px, TextUnits::ONE);
        let mut c = Canvas::new(w as u32, h as u32);
        draw(&mut c, &text, &theme.paint(), &f, px);
        assert_eq!(
            c.get(242, 200),
            cs.button_text.argb(),
            "Yes wears the black default ring at x 242"
        );
        assert_eq!(
            c.get(329, 200),
            cs.button_hilight.argb(),
            "No's plain raised edge at x 329"
        );
        let ink = (119..160)
            .flat_map(|x| (148..148 + LINE_H).map(move |y| (x, y)))
            .filter(|&(x, y)| c.get(x, y) == cs.button_text.argb())
            .count();
        assert!(ink > 10, "the size line is drawn at (119, 148): {ink}");
        assert!(
            (0..w).all(|x| c.get(x, h - 1) == cs.button_face.argb()),
            "the bottom row is empty"
        );
    }

    /// Writes target/replacebox-file-preview.png and
    /// target/replacebox-folder-preview.png for a visual check.
    #[test]
    #[ignore]
    fn render_replacebox() {
        let (text, px) = rend();
        let theme = Theme {
            access_keys: true,
            ..Theme::default()
        };
        for (fields, name) in [
            (file(), "replacebox-file-preview.png"),
            (folder(), "replacebox-folder-preview.png"),
        ] {
            let (w, h) = client(&fields, &text, px, TextUnits::ONE);
            let mut c = Canvas::new(w as u32, h as u32);
            draw(
                &mut c,
                &text,
                &theme.paint(),
                &form(&fields, &text, px, TextUnits::ONE),
                px,
            );
            crate::raster::write_preview(&c, name);
        }
    }
}
