use super::{button, field_texts, moves};
use crate::classic::widgets::BUTTON_H;
use crate::controls::form::{Control, Form, Item};
use crate::raster::Rect;

pub const SUMMARY_IDS: [&str; 6] = [
    "sum_title",
    "sum_subject",
    "sum_author",
    "sum_category",
    "sum_keywords",
    "sum_comments",
];
pub const SUMMARY_ADVANCED: &str = "sum_advanced";
const SUMMARY_LABELS: [&str; 6] = [
    "&Title:",
    "&Subject:",
    "A&uthor:",
    "&Category:",
    "&Keywords:",
    "Co&mments:",
];
const SUM_LABEL_X: i32 = 22;
const SUM_FIELD_X: i32 = 81;
const SUM_FIELD_W: i32 = 263;
const SUM_LABEL_DY: i32 = 3;
const SUM_ROWS: [i32; 5] = [41, 72, 103, 152, 183];
const SUM_FIELD_H: i32 = 21;
pub(super) const SUM_COMMENTS: Rect = Rect::new(81, 214, 263, 80);
const SUM_SEP: Rect = Rect::new(22, 137, 318, 2);
const SUM_ADVANCED: Rect = Rect::new(244, 305, 99, BUTTON_H);

pub fn summary_page(values: Option<&[String; 6]>) -> Form {
    let live = values.is_some();
    let value = |k: usize| values.map_or("", |v| v[k].as_str());
    let mut items = Vec::new();
    for (k, &y) in SUM_ROWS.iter().enumerate() {
        items.push(Item::label_at(
            ["sum_l0", "sum_l1", "sum_l2", "sum_l3", "sum_l4"][k],
            SUM_LABEL_X,
            y + SUM_LABEL_DY,
            SUM_FIELD_X - SUM_LABEL_X,
            SUMMARY_LABELS[k],
        ));
        items.push(Item::edit(
            SUMMARY_IDS[k],
            Rect::new(SUM_FIELD_X, y, SUM_FIELD_W, SUM_FIELD_H),
            value(k),
            live,
        ));
        if k == 2 {
            items.push(Item::new("sum_sep", SUM_SEP, Control::Separator));
        }
    }
    items.push(Item::label_at(
        "sum_l5",
        SUM_LABEL_X,
        SUM_COMMENTS.y + SUM_LABEL_DY,
        SUM_FIELD_X - SUM_LABEL_X,
        SUMMARY_LABELS[5],
    ));
    items.push(Item::memo(SUMMARY_IDS[5], SUM_COMMENTS, value(5), live));
    items.push(button(
        SUMMARY_ADVANCED,
        SUM_ADVANCED,
        "Ad&vanced >>",
        false,
    ));
    Form::new(items)
}

pub fn summary_values(page: &Form) -> [String; 6] {
    field_texts(page, SUMMARY_IDS)
}

pub fn summary_moves(page: &Form, opened: &[String; 6]) -> Vec<(usize, String)> {
    moves(summary_values(page), opened)
}

pub fn summary_of(pages: &[Form]) -> Option<&Form> {
    pages.iter().find(|p| p.item(SUMMARY_IDS[0]).is_some())
}

pub fn no_summary_page() -> Form {
    Form::new(vec![
        Item::lines_at(
            "ns_text",
            22,
            44,
            310,
            &[
                "There are no summary properties in common among the selected",
                "sources.",
            ],
        ),
        button(
            "ns_simple",
            Rect::new(244, 410, 99, BUTTON_H),
            "<< &Simple",
            false,
        ),
    ])
}
