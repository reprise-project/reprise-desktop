//! A file's Checksums page: SHA-256 and MD5 with Copy, Calculate with its progress, and a field
//! a pasted hash is compared from.

use super::{button, sep};
use crate::classic::widgets::BUTTON_H;
use crate::controls::form::{Form, Item};
use crate::raster::Rect;

pub const SUMS_CALCULATE: &str = "sums_calc";
pub const SUMS_COPY_SHA256: &str = "sums_copy_sha";
pub const SUMS_COPY_MD5: &str = "sums_copy_md5";
pub const SUMS_COMPARE: &str = "sums_compare";
const SUMS_SHA256: &str = "sums_sha";
const SUMS_MD5: &str = "sums_md5";
const SUMS_STATUS: &str = "sums_status";
const SUMS_VERDICT: &str = "sums_verdict";

const X: i32 = 22;
const W: i32 = 317;
const BUTTON_X: i32 = 264;
const BUTTON_W: i32 = 75;
const SHA_Y: i32 = 44;
const MD5_Y: i32 = 100;
const VALUE_DY: i32 = 18;
const BUTTON_DY: i32 = -5;
const STATUS_Y: i32 = 151;
const SEP_Y: i32 = 181;
const COMPARE_Y: i32 = 196;
const FIELD_DY: i32 = 17;
const FIELD_H: i32 = 21;
const VERDICT_DY: i32 = 29;

/// What the page shows of the sums.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SumsState {
    Idle,
    /// Percent read.
    Working(u8),
    Done {
        sha256: String,
        md5: String,
    },
    Failed(String),
}

/// The page with nothing calculated.
pub fn checksums_page() -> Form {
    Form::new(vec![
        Item::label_at("sums_l0", X, SHA_Y, BUTTON_X - X, "SHA-256:"),
        Item::lines_at(SUMS_SHA256, X, SHA_Y + VALUE_DY, W, &["", ""]),
        copy(SUMS_COPY_SHA256, SHA_Y, "&Copy"),
        Item::label_at("sums_l1", X, MD5_Y, BUTTON_X - X, "MD5:"),
        Item::lines_at(SUMS_MD5, X, MD5_Y + VALUE_DY, W, &[""]),
        copy(SUMS_COPY_MD5, MD5_Y, "Co&py"),
        Item::label_at(SUMS_STATUS, X, STATUS_Y, BUTTON_X - X - 8, ""),
        button(
            SUMS_CALCULATE,
            Rect::new(BUTTON_X, STATUS_Y + BUTTON_DY, BUTTON_W, BUTTON_H),
            "C&alculate",
            true,
        ),
        sep("sums_sep", SEP_Y, X + W),
        Item::label_at("sums_l2", X, COMPARE_Y, W, "Co&mpare with:"),
        Item::edit(
            SUMS_COMPARE,
            Rect::new(X, COMPARE_Y + FIELD_DY, W, FIELD_H),
            "",
            true,
        ),
        Item::label_at(SUMS_VERDICT, X, COMPARE_Y + FIELD_DY + VERDICT_DY, W, ""),
    ])
}

fn copy(id: &'static str, y: i32, label: &str) -> Item {
    button(
        id,
        Rect::new(BUTTON_X, y + BUTTON_DY, BUTTON_W, BUTTON_H),
        label,
        false,
    )
}

/// Put `state` on `page`. A SHA-256 goes on two lines of 32 digits, which the page's width fits.
pub fn show_sums(page: &mut Form, state: &SumsState) {
    let (sha, md5) = match state {
        SumsState::Done { sha256, md5 } => {
            let cut = sha256.len().min(32);
            (
                vec![sha256[..cut].to_string(), sha256[cut..].to_string()],
                md5.clone(),
            )
        }
        _ => (vec![String::new(), String::new()], String::new()),
    };
    page.set_lines(SUMS_SHA256, &sha);
    page.set_text(SUMS_MD5, &md5);
    let done = matches!(state, SumsState::Done { .. });
    page.set_enabled(SUMS_COPY_SHA256, done);
    page.set_enabled(SUMS_COPY_MD5, done);
    let calculate = matches!(state, SumsState::Idle | SumsState::Failed(_));
    if !calculate && page.focus() == page.index_of(SUMS_CALCULATE) {
        page.focus_id(SUMS_COMPARE);
    }
    page.set_enabled(SUMS_CALCULATE, calculate);
    let status = match state {
        SumsState::Working(p) => format!("Calculating... {p}%"),
        SumsState::Failed(why) => why.clone(),
        SumsState::Idle | SumsState::Done { .. } => String::new(),
    };
    page.set_text(SUMS_STATUS, &status);
}

/// What the page says of the hash in its Compare field.
pub fn show_verdict(page: &mut Form, text: &str) {
    page.set_text(SUMS_VERDICT, text);
}

/// The text in the Compare field.
pub fn compare_text(page: &Form) -> &str {
    page.edit_text(SUMS_COMPARE)
}

pub fn checksums_of(pages: &mut [Form]) -> Option<&mut Form> {
    pages.iter_mut().find(|p| p.item(SUMS_CALCULATE).is_some())
}
