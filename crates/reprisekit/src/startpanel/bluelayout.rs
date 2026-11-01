//! Where the Blue Start menu's parts go at 100% text, the band's top-left at (0, 0); the columns are
//! as tall as the taller one needs. [`super::layout::layout`] grows it with the text size.

use super::layout::Layout;
use super::{Spot, StartPanel};
use crate::raster::Rect;

pub const W: i32 = 380;
/// The band over the columns, its accent line included.
pub const BAND_H: i32 = 64;
pub const FOOT_H: i32 = 40;
/// The frame left and right of the columns.
pub const EDGE: i32 = 2;
const LEFT_W: i32 = 188;
/// The frequent programs shown under the pinned ones.
pub const FREQUENT: usize = 6;

const PICTURE_FRAME: Rect = Rect::new(6, 5, 52, 52);

const PAD: i32 = 6;
const ROW_X: i32 = EDGE + 2;
const ROW_W: i32 = LEFT_W - 4;
pub const PROGRAM_H: i32 = 40;
/// A rule's room in either column, its line in the middle.
const RULE_ROOM: i32 = 9;
const RULE_H: i32 = 1;
const ALL_H: i32 = 26;
const LEFT_RULE_X: i32 = 12;
const RULE_W: i32 = 168;

const PLACE_X: i32 = EDGE + LEFT_W + 2;
const PLACE_W: i32 = W - EDGE - 2 - PLACE_X;
pub const PLACE_H: i32 = 30;
const RIGHT_RULE_X: i32 = PLACE_X + 8;

const BUTTON_Y: i32 = 7;
const BUTTON_H: i32 = 26;
/// A bottom button's label pen from its left edge; the label's right pad to its right edge.
pub const BUTTON_PEN: i32 = 31;
const BUTTON_PAD: i32 = 6;
const BUTTON_GAP: i32 = 4;
const BUTTON_RIGHT: i32 = 8;

fn shown_frequent(p: &StartPanel) -> usize {
    p.frequent.len().min(FREQUENT)
}

fn pin_rule(p: &StartPanel, rows: usize) -> bool {
    !p.pinned.is_empty() && rows > p.pinned.len()
}

fn left_height(p: &StartPanel, rows: usize) -> i32 {
    let rule = if pin_rule(p, rows) { RULE_ROOM } else { 0 };
    PAD + rows as i32 * PROGRAM_H + rule + RULE_ROOM + ALL_H + PAD
}

fn right_height(p: &StartPanel) -> i32 {
    let rules = p.places.iter().skip(1).filter(|pl| pl.rule_before).count() as i32;
    PAD + p.places.len() as i32 * PLACE_H + rules * RULE_ROOM + PAD
}

/// The program rows shown: every pinned row and [`FREQUENT`] frequent ones at most, less the rows
/// that come off the bottom so the panel stands in [`StartPanel::room`] at its text size.
pub fn programs_shown(p: &StartPanel) -> usize {
    let all = p.pinned.len() + shown_frequent(p);
    let Some((_, room)) = p.room else {
        return all;
    };
    let tall = |rows: usize| BAND_H + left_height(p, rows).max(right_height(p)) + FOOT_H;
    (0..=all)
        .rev()
        .find(|&rows| p.text.len(tall(rows)) <= room)
        .unwrap_or(0)
}

fn body_height(p: &StartPanel) -> i32 {
    left_height(p, programs_shown(p)).max(right_height(p))
}

pub(super) fn layout_100(p: &StartPanel) -> Layout {
    let body = body_height(p);
    let panel = Rect::new(0, 0, W, BAND_H + body + FOOT_H);
    let pane = Rect::new(EDGE, BAND_H, LEFT_W, body);
    let mut spots = Vec::new();
    let mut rules = Vec::new();

    let rows = programs_shown(p);
    let mut y = BAND_H + PAD;
    for i in 0..rows {
        if i == p.pinned.len() && pin_rule(p, rows) {
            rules.push(Rect::new(LEFT_RULE_X, y + RULE_ROOM / 2, RULE_W, RULE_H));
            y += RULE_ROOM;
        }
        spots.push((Spot::Program(i), Rect::new(ROW_X, y, ROW_W, PROGRAM_H)));
        y += PROGRAM_H;
    }
    let all = Rect::new(ROW_X, pane.bottom() - PAD - ALL_H, ROW_W, ALL_H);
    rules.push(Rect::new(
        LEFT_RULE_X,
        all.y - RULE_ROOM + RULE_ROOM / 2,
        RULE_W,
        RULE_H,
    ));
    spots.push((Spot::AllPrograms, all));

    let mut y = BAND_H + PAD;
    for (i, place) in p.places.iter().enumerate() {
        if place.rule_before && i > 0 {
            rules.push(Rect::new(RIGHT_RULE_X, y + RULE_ROOM / 2, RULE_W, RULE_H));
            y += RULE_ROOM;
        }
        spots.push((Spot::Place(i), Rect::new(PLACE_X, y, PLACE_W, PLACE_H)));
        y += PLACE_H;
    }

    let y = pane.bottom() + BUTTON_Y;
    let [log_off, turn_off] = p.foot_labels.map(|w| BUTTON_PEN + w + BUTTON_PAD);
    let turn_off = Rect::new(W - BUTTON_RIGHT - turn_off, y, turn_off, BUTTON_H);
    let log_off = Rect::new(turn_off.x - BUTTON_GAP - log_off, y, log_off, BUTTON_H);
    spots.push((Spot::LogOff, log_off));
    spots.push((Spot::Shutdown, turn_off));
    spots.push((Spot::Picture, PICTURE_FRAME));

    Layout {
        panel,
        canvas: panel,
        pane,
        jump_pane: None,
        spots,
        rules,
        headers: Vec::new(),
        picture: Some(PICTURE_FRAME),
        tree_bar: None,
        scale: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blue::startpanel::{TEXT_PX, foot_labels};
    use crate::startpanel::layout::layout;
    use crate::startpanel::search::Grow;
    use crate::startpanel::tests::{place, prow};
    use crate::startpanel::{PlaceRow, Shape};
    use crate::text::TextRenderer;
    use crate::theme::TextUnits;

    pub(crate) fn blue() -> StartPanel {
        let mut p = StartPanel::new();
        p.shape = Shape::Blue;
        p.pinned = vec![prow("Files", false), prow("Terminal", false)];
        p.frequent = (0..8).map(|k| prow(&format!("App {k}"), false)).collect();
        p.places = [
            ("Documents", false),
            ("Recent Items", false),
            ("Pictures", false),
            ("Music", false),
            ("Computer", false),
            ("Control Panel", true),
            ("Printers", false),
            ("Help and Support", true),
            ("Run...", false),
        ]
        .iter()
        .map(|&(l, rule)| PlaceRow {
            rule_before: rule,
            submenu: l == "Recent Items",
            ..place(l)
        })
        .collect();
        p
    }

    #[test]
    fn the_band_columns_and_bottom_band_stack_up() {
        let mut p = blue();
        p.foot_labels = [40, 100];
        let l = layout(&p);
        let body = PAD + 8 * PROGRAM_H + RULE_ROOM + RULE_ROOM + ALL_H + PAD;
        assert_eq!(body, 376);
        assert_eq!(l.panel, Rect::new(0, 0, 380, 64 + 376 + 40));
        assert_eq!(l.canvas, l.panel, "nothing stands outside the panel");
        assert_eq!(l.pane, Rect::new(2, 64, 188, 376));
        assert_eq!(l.rect(Spot::Program(0)), Some(Rect::new(4, 70, 184, 40)));
        assert_eq!(
            l.rect(Spot::Program(2)),
            Some(Rect::new(4, 159, 184, 40)),
            "the frequent rows start under the pinned ones' rule"
        );
        assert_eq!(l.rect(Spot::Program(8)), None, "six frequent rows at most");
        assert_eq!(l.rect(Spot::AllPrograms), Some(Rect::new(4, 408, 184, 26)));
        assert_eq!(l.rect(Spot::Place(0)), Some(Rect::new(192, 70, 184, 30)));
        assert_eq!(
            l.rect(Spot::Place(5)),
            Some(Rect::new(192, 229, 184, 30)),
            "a rule stands before Control Panel"
        );
        assert_eq!(l.rect(Spot::Shutdown), Some(Rect::new(235, 447, 137, 26)));
        assert_eq!(l.rect(Spot::LogOff), Some(Rect::new(154, 447, 77, 26)));
        assert_eq!(l.picture, Some(Rect::new(6, 5, 52, 52)));
        assert_eq!(l.rules.len(), 4, "pins, All Programs and two place rules");
    }

    #[test]
    fn the_bottom_buttons_hug_their_labels_from_the_right_margin() {
        let button = |advance: i32, text: TextUnits| {
            let mut p = blue();
            p.text = text;
            let stub = TextRenderer::stub(text.px(TEXT_PX), move |_| advance);
            p.foot_labels = foot_labels(&stub, Grow { s: 1, u: text });
            let l = layout(&p);
            (
                l.panel,
                l.rect(Spot::LogOff).unwrap(),
                l.rect(Spot::Shutdown).unwrap(),
            )
        };
        let (panel, log_off, turn_off) = button(6, TextUnits::ONE);
        assert_eq!(turn_off.right(), panel.right() - BUTTON_RIGHT);
        assert_eq!(turn_off.width, BUTTON_PEN + 17 * 6 + BUTTON_PAD);
        assert_eq!(log_off.right(), turn_off.x - BUTTON_GAP);
        assert_eq!(log_off.width, BUTTON_PEN + 7 * 6 + BUTTON_PAD);
        let (_, _, wider) = button(8, TextUnits::ONE);
        assert_eq!(
            wider.width - turn_off.width,
            17 * 2,
            "the width follows the label"
        );
        assert_eq!(wider.right(), panel.right() - BUTTON_RIGHT);

        let (panel, log_off, turn_off) = button(12, TextUnits::new(22, 11));
        assert_eq!(turn_off.right(), panel.right() - 2 * BUTTON_RIGHT);
        assert_eq!(turn_off.width, 2 * (BUTTON_PEN + 17 * 6 + BUTTON_PAD));
        assert_eq!(log_off.right(), turn_off.x - 2 * BUTTON_GAP);
    }

    #[test]
    fn without_pins_the_places_set_the_height() {
        let mut p = blue();
        p.pinned.clear();
        p.frequent.truncate(2);
        let l = layout(&p);
        assert_eq!(l.pane.height, right_height(&p));
        assert_eq!(
            l.rect(Spot::AllPrograms).map(|r| r.bottom()),
            Some(l.pane.bottom() - PAD),
            "All Programs stands at the bottom of the column"
        );
        assert_eq!(l.rules.len(), 3, "no rule under the pins");
    }

    #[test]
    fn a_short_screen_takes_program_rows_off() {
        let mut p = blue();
        p.room = Some((1920, 480 - PROGRAM_H));
        assert_eq!(programs_shown(&p), 7);
        assert_eq!(layout(&p).panel.height, 440);
        p.room = Some((1920, 100));
        assert_eq!(programs_shown(&p), 0, "every row can go");
    }

    #[test]
    fn larger_text_grows_the_panel_but_not_the_picture() {
        let mut p = blue();
        p.text = TextUnits::new(22, 11);
        let l = layout(&p);
        assert_eq!(l.panel.width, 2 * W);
        assert_eq!(
            l.picture.map(|r| (r.width, r.height)),
            Some((52, 52)),
            "the picture keeps its size"
        );
    }
}
