//! Where the Start panel's parts go in each mode, and which spot a point falls on. Panel coordinates: the glass's top-left is (0, 0); the user picture stands above it, so the canvas starts above 0.
//! The panel stands 411x476 at 100% text and grows with the text size ([`StartPanel::text`]).
//! With both pinned and frequent programs the rule between them adds 4 px and everything from All Programs down moves with it; program rows that do not fit above the All Programs rule are not shown.
//! On a screen too short for the panel ([`StartPanel::room`]) program rows come off the list, 38 px each, and everything from All Programs down moves up with them; the places close up to fit above Shut down.
//! On a screen too narrow for it the text size steps down ([`StartPanel::fit`]) and the jump pane narrows; at 100% the right column narrows by up to 40 px.

use super::{Mode, Spot, StartPanel};
use crate::raster::Rect;

pub const W: i32 = 411;
pub const H: i32 = 476;
pub const JUMP_W: i32 = 491;
pub const OVERHANG: i32 = 26;
const PICTURE_FRAME: Rect = Rect::new(304, -OVERHANG, 64, 62);

// The white left pane: wider in Search mode and while a jump list shows. Its last 45 rows are the band the field stands on.
const PANE: Rect = Rect::new(9, 9, 250, 458);
const SEARCH_PANE_W: i32 = 393;
const JUMP_PANE_W: i32 = 473;

const ROW_X: i32 = 11;
const ROW_W: i32 = 246;
const PROGRAM_TOP: i32 = 10;
const PROGRAM_H: i32 = 38;
const PIN_RULE_H: i32 = 4;
const PIN_RULE_DOWN: i32 = 2;
const LEFT_RULE_X: i32 = 20;
const LEFT_RULE_W: i32 = 232;
const RULE_H: i32 = 1;

// From the bottom up: the rule over All Programs, the All Programs row, the search field and its clear button, Shut down and its arrow.
const ALL_RULE_Y: i32 = 391;
const ALL_PROGRAMS: Rect = Rect::new(ROW_X, 395, ROW_W, 23);
const SEARCH: Rect = Rect::new(21, 434, super::search::FIELD.0, super::search::FIELD.1);
const CLEAR: Rect = Rect::new(220, 435, 21, 20);
const SHUTDOWN: Rect = Rect::new(267, 434, 72, 22);
const SHUTDOWN_ARROW: Rect = Rect::new(339, 434, 22, 22);

const TREE_TOP: i32 = 10;
pub const TREE_ROW_H: i32 = 19;
pub const TREE_INDENT: i32 = 19;
const TREE_BAR_X: i32 = 242;
const TREE_BAR_W: i32 = 16;

// Search results: group headers, then rows, down to the bottom of the All Programs row.
const RESULTS_TOP: i32 = 26;
const HEADER_X: i32 = 17;
const HEADER_W: i32 = 375;
pub const HEADER_H: i32 = 21;
const RESULT_X: i32 = 25;
const RESULT_W: i32 = 371;
pub const RESULT_H: i32 = 21;
pub const GROUP_GAP: i32 = 11;

const JUMP_PANE: Rect = Rect::new(259, 9, 223, 413);
const JUMP_X: i32 = 274;
const JUMP_ROW_W: i32 = 204;
const JUMP_HEADER_Y: i32 = 12;
const JUMP_ROW_H: i32 = 22;

const PLACE_X: i32 = 266;
const PLACE_W: i32 = 138;
const PLACE_TOP: i32 = 44;
const PLACE_H: i32 = 33;
const PLACE_PITCH: i32 = 35;
const PLACE_PITCH_LEAST: i32 = 22;
const PLACE_RULE_H: i32 = 3;
const PLACE_RULE_X: i32 = 274;
const PLACE_RULE_W: i32 = 124;
const PLACE_RULE_LINE: i32 = 2;

/// The most the right column narrows at 100% text: the panel's right edge stays 10 px past the Shut down arrow.
const NARROW_MOST: i32 = 40;

/// Where the parts go, in the layout's pixels (`scale` device pixels per
/// logical one). `spots` lists every spot shown, the clear button before
/// the field it sits in; `headers` are the result group headers (the
/// group's index) in Search mode, the jump list's header (0) in Jump mode.
/// `pane` is the white left pane, `jump_pane` the jump list's light one.
/// `rules` are the left pane's 1 px rules and the places' 2 px ones.
/// `picture` is the user picture's frame, in the modes that show it.
/// `tree_bar` is the tree's scroll bar when its rows overflow.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub panel: Rect,
    pub canvas: Rect,
    pub pane: Rect,
    pub jump_pane: Option<Rect>,
    pub spots: Vec<(Spot, Rect)>,
    pub rules: Vec<Rect>,
    pub headers: Vec<(usize, Rect)>,
    pub picture: Option<Rect>,
    pub tree_bar: Option<Rect>,
    pub scale: i32,
}

impl Layout {
    pub fn scaled(&self, s: i32) -> Layout {
        let s = s.max(1);
        Layout {
            panel: self.panel.scaled(s),
            canvas: self.canvas.scaled(s),
            pane: self.pane.scaled(s),
            jump_pane: self.jump_pane.map(|r| r.scaled(s)),
            spots: self.spots.iter().map(|&(k, r)| (k, r.scaled(s))).collect(),
            rules: self.rules.iter().map(|r| r.scaled(s)).collect(),
            headers: self
                .headers
                .iter()
                .map(|&(g, r)| (g, r.scaled(s)))
                .collect(),
            picture: self.picture.map(|r| r.scaled(s)),
            tree_bar: self.tree_bar.map(|r| r.scaled(s)),
            scale: self.scale * s,
        }
    }

    pub fn rect(&self, s: Spot) -> Option<Rect> {
        self.spots.iter().find(|(k, _)| *k == s).map(|&(_, r)| r)
    }
}

pub fn hit(l: &Layout, x: i32, y: i32) -> Option<Spot> {
    l.spots
        .iter()
        .find(|(_, r)| r.contains(x, y))
        .map(|&(s, _)| s)
}

/// How far the parts from All Programs down sit below their place in a
/// 476 tall panel: the pinned rows' rule, when there is one, less the program
/// rows taken off to fit [`StartPanel::room`].
fn grow(p: &StartPanel) -> i32 {
    pin_rule(p) - PROGRAM_H * rows_cut(p)
}

fn pin_rule(p: &StartPanel) -> i32 {
    if p.pinned.is_empty() || p.frequent.is_empty() {
        0
    } else {
        PIN_RULE_H
    }
}

/// How many program rows come off the bottom of the list so the panel stands in
/// [`StartPanel::room`] at its text size; one row always stays.
fn rows_cut(p: &StartPanel) -> i32 {
    let Some((_, room)) = p.room else {
        return 0;
    };
    let tall = H + pin_rule(p);
    let most = (ALL_RULE_Y - PROGRAM_TOP) / PROGRAM_H - 1;
    (0..most)
        .find(|n| p.text.len(tall - n * PROGRAM_H) <= room)
        .unwrap_or(most)
}

/// The places' pitch: 35, closer on a panel too short to stack them all above Shut down, but
/// never under 22; places that still do not fit are not shown.
fn place_pitch(p: &StartPanel) -> i32 {
    let n = p.places.len() as i32;
    let rules = p.places.iter().skip(1).filter(|pl| pl.rule_before).count() as i32;
    let gap = PLACE_PITCH - PLACE_H;
    let room = SHUTDOWN.y + grow(p) - PLACE_TOP - rules * PLACE_RULE_H + gap;
    (room / n.max(1)).clamp(PLACE_PITCH_LEAST, PLACE_PITCH)
}

/// How far a closed-up place's text and arrow sit above their place in a 33 px row.
pub fn place_lift(p: &StartPanel) -> i32 {
    (PLACE_PITCH - place_pitch(p)) / 2
}

/// How much narrower the right column is so the panel stands in [`StartPanel::room`]'s width once
/// the text is down to 100%; never more than [`NARROW_MOST`].
pub fn narrow(p: &StartPanel) -> i32 {
    let Some((room, _)) = p.room else {
        return 0;
    };
    (W - p.text.within(room)).clamp(0, NARROW_MOST)
}

/// How much narrower the jump pane is so the Jump panel stands in [`StartPanel::room`]'s width;
/// the panel never gets narrower than the Home one.
fn jump_squeeze(p: &StartPanel) -> i32 {
    let Some((room, _)) = p.room else {
        return 0;
    };
    (JUMP_W - p.text.within(room)).clamp(0, JUMP_W - W + narrow(p))
}

fn down(r: Rect, by: i32) -> Rect {
    Rect::new(r.x, r.y + by, r.width, r.height)
}

fn program_rows(p: &StartPanel) -> (Vec<Rect>, Option<Rect>) {
    let g = grow(p);
    let bottom = ALL_RULE_Y + g;
    let mut y = PROGRAM_TOP;
    let mut rows = Vec::new();
    let mut rule = None;
    for i in 0..p.pinned.len() + p.frequent.len() {
        if i == p.pinned.len() && g > 0 {
            rule = Some(Rect::new(
                LEFT_RULE_X,
                y + PIN_RULE_DOWN,
                LEFT_RULE_W,
                RULE_H,
            ));
            y += PIN_RULE_H;
        }
        if y + PROGRAM_H > bottom {
            break;
        }
        rows.push(Rect::new(ROW_X, y, ROW_W, PROGRAM_H));
        y += PROGRAM_H;
    }
    (rows, rule)
}

pub fn programs_shown(p: &StartPanel) -> usize {
    program_rows(p).0.len()
}

pub fn tree_capacity(p: &StartPanel) -> usize {
    ((ALL_RULE_Y + grow(p) - TREE_TOP) / TREE_ROW_H).max(0) as usize
}

fn jump_pane(p: &StartPanel) -> Rect {
    Rect::new(
        JUMP_PANE.x,
        JUMP_PANE.y,
        JUMP_PANE.width - jump_squeeze(p),
        JUMP_PANE.height + grow(p),
    )
}

pub fn jumps_shown(p: &StartPanel) -> usize {
    let room = jump_pane(p).bottom() - (JUMP_HEADER_Y + JUMP_ROW_H);
    p.jump.len().min((room / JUMP_ROW_H).max(0) as usize)
}

/// Where the parts go at the panel's text size: the 100% layout with every rect grown
/// ([`crate::theme::TextUnits::rect`]). Rules keep their thickness, the tree's bar its width at the rows' right,
/// and the user picture's frame its size, centred where the grown frame would be.
pub fn layout(p: &StartPanel) -> Layout {
    let l = layout_100(p);
    let u = p.text;
    if u.is_one() {
        return l;
    }
    let kept = |r: Rect| {
        let mid = u.len(r.x + r.width / 2);
        Rect::new(mid - r.width / 2, r.y, r.width, r.height)
    };
    let tree_bar = l.tree_bar.map(|r| {
        let g = u.rect(r);
        Rect::new(g.right() - r.width, g.y, r.width, g.height)
    });
    let spots = l
        .spots
        .iter()
        .map(|&(k, r)| {
            let g = match k {
                Spot::Picture => kept(r),
                _ => u.rect(r),
            };
            match (k, tree_bar) {
                (Spot::Tree(_), Some(bar)) => (k, Rect::new(g.x, g.y, bar.x - g.x, g.height)),
                _ => (k, g),
            }
        })
        .collect();
    let rules = l
        .rules
        .iter()
        .map(|&r| {
            let g = u.rect(r);
            Rect::new(g.x, u.len(r.y), g.width, r.height)
        })
        .collect();
    let panel = u.rect(l.panel);
    let picture = l.picture.map(kept);
    Layout {
        panel,
        canvas: picture.map_or(panel, |r| panel.union(&r)),
        pane: u.rect(l.pane),
        jump_pane: l.jump_pane.map(|r| u.rect(r)),
        spots,
        rules,
        headers: l.headers.iter().map(|&(g, r)| (g, u.rect(r))).collect(),
        picture,
        tree_bar,
        scale: l.scale,
    }
}

fn layout_100(p: &StartPanel) -> Layout {
    let g = grow(p);
    let mode = p.mode();
    let squeeze = jump_squeeze(p);
    let narrow = narrow(p);
    let panel = Rect::new(
        0,
        0,
        if let Mode::Jump(_) = mode {
            JUMP_W - squeeze
        } else {
            W - narrow
        },
        H + g,
    );
    let pane_w = match mode {
        Mode::Search => SEARCH_PANE_W - narrow,
        Mode::Jump(_) => JUMP_PANE_W - squeeze,
        Mode::Home | Mode::Programs => PANE.width,
    };
    let pane = Rect::new(PANE.x, PANE.y, pane_w, PANE.height + g);
    let mut spots = Vec::new();
    let mut rules = Vec::new();
    let mut headers = Vec::new();
    let mut tree_bar = None;
    let all_rule = Rect::new(LEFT_RULE_X, ALL_RULE_Y + g, LEFT_RULE_W, RULE_H);

    match mode {
        Mode::Home | Mode::Jump(_) => {
            let (rows, rule) = program_rows(p);
            spots.extend(
                rows.into_iter()
                    .enumerate()
                    .map(|(i, r)| (Spot::Program(i), r)),
            );
            rules.extend(rule);
            rules.push(all_rule);
            spots.push((Spot::AllPrograms, down(ALL_PROGRAMS, g)));
        }
        Mode::Programs => {
            let rows = p.tree_rows();
            let cap = tree_capacity(p);
            let w = if rows.len() > cap {
                tree_bar = Some(Rect::new(
                    TREE_BAR_X,
                    TREE_TOP,
                    TREE_BAR_W,
                    cap as i32 * TREE_ROW_H,
                ));
                TREE_BAR_X - ROW_X
            } else {
                ROW_W
            };
            let shown = rows.iter().skip(p.tree_top).take(cap);
            spots.extend(shown.enumerate().map(|(k, &i)| {
                let y = TREE_TOP + k as i32 * TREE_ROW_H;
                (Spot::Tree(i), Rect::new(ROW_X, y, w, TREE_ROW_H))
            }));
            rules.push(all_rule);
            spots.push((Spot::AllPrograms, down(ALL_PROGRAMS, g)));
        }
        Mode::Search => {
            let bottom = ALL_PROGRAMS.bottom() + g;
            let mut y = RESULTS_TOP;
            'groups: for (gi, grp) in p.results.iter().enumerate() {
                if grp.rows.is_empty() {
                    continue;
                }
                if y + HEADER_H > bottom {
                    break;
                }
                headers.push((gi, Rect::new(HEADER_X, y, HEADER_W - narrow, HEADER_H)));
                y += HEADER_H;
                for ri in 0..grp.rows.len() {
                    if y + RESULT_H > bottom {
                        break 'groups;
                    }
                    let r = Rect::new(RESULT_X, y, RESULT_W - narrow, RESULT_H);
                    spots.push((Spot::Result(gi, ri), r));
                    y += RESULT_H;
                }
                y += GROUP_GAP;
            }
        }
    }

    if !p.query().is_empty() {
        spots.push((Spot::Clear, down(CLEAR, g)));
    }
    spots.push((Spot::Search, down(SEARCH, g)));

    let mut picture = None;
    let mut jump = None;
    match mode {
        Mode::Home | Mode::Programs => {
            let mut y = PLACE_TOP;
            let pitch = place_pitch(p);
            let h = pitch - (PLACE_PITCH - PLACE_H);
            for (i, place) in p.places.iter().enumerate() {
                let rule = place.rule_before && i > 0;
                let top = if rule { y + PLACE_RULE_H } else { y };
                if top + h > SHUTDOWN.y + g {
                    break;
                }
                if rule {
                    let w = PLACE_RULE_W - narrow;
                    rules.push(Rect::new(PLACE_RULE_X, y, w, PLACE_RULE_LINE));
                }
                spots.push((Spot::Place(i), Rect::new(PLACE_X, top, PLACE_W - narrow, h)));
                y = top + pitch;
            }
            let frame = Rect::new(
                PICTURE_FRAME.x - narrow / 2,
                PICTURE_FRAME.y,
                PICTURE_FRAME.width,
                PICTURE_FRAME.height,
            );
            picture = Some(frame);
            spots.push((Spot::Picture, frame));
        }
        Mode::Jump(_) => {
            let jp = jump_pane(p);
            jump = Some(jp);
            let row_w = JUMP_ROW_W - squeeze;
            headers.push((0, Rect::new(JUMP_X, JUMP_HEADER_Y, row_w, JUMP_ROW_H)));
            let top = JUMP_HEADER_Y + JUMP_ROW_H;
            spots.extend((0..jumps_shown(p)).map(|k| {
                let y = top + k as i32 * JUMP_ROW_H;
                (Spot::Jump(k), Rect::new(JUMP_X, y, row_w, JUMP_ROW_H))
            }));
        }
        Mode::Search => {}
    }
    spots.push((Spot::Shutdown, down(SHUTDOWN, g)));
    spots.push((Spot::ShutdownArrow, down(SHUTDOWN_ARROW, g)));

    let canvas = picture.map_or(panel, |r| panel.union(&r));
    Layout {
        panel,
        canvas,
        pane,
        jump_pane: jump,
        spots,
        rules,
        headers,
        picture,
        tree_bar,
        scale: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startpanel::tests::{place, prow, row};
    use crate::startpanel::{Key, ResultGroup, TreeRow};
    use crate::textedit::EditKey;
    use crate::theme::TextUnits;

    #[test]
    fn home_puts_the_pane_rows_and_buttons_where_the_design_says() {
        let mut p = StartPanel::new();
        p.frequent = (0..9).map(|k| prow(&format!("App {k}"), false)).collect();
        let l = layout(&p);
        assert_eq!(l.panel, Rect::new(0, 0, W, H));
        assert_eq!(l.pane, Rect::new(9, 9, 250, 458));
        assert_eq!(l.rect(Spot::Program(0)), Some(Rect::new(11, 10, 246, 38)));
        assert_eq!(l.rect(Spot::Program(1)), Some(Rect::new(11, 48, 246, 38)));
        assert_eq!(l.rect(Spot::AllPrograms), Some(Rect::new(11, 395, 246, 23)));
        assert_eq!(l.rect(Spot::Search), Some(Rect::new(21, 434, 222, 22)));
        assert_eq!(
            l.rect(Spot::Clear),
            None,
            "no clear button on an empty field"
        );
        assert_eq!(l.rect(Spot::Shutdown), Some(Rect::new(267, 434, 72, 22)));
        assert_eq!(
            l.rect(Spot::ShutdownArrow),
            Some(Rect::new(339, 434, 22, 22))
        );
        assert_eq!(
            l.rules,
            [Rect::new(20, 391, 232, 1)],
            "the rule over All Programs"
        );
        let pic = l.picture.expect("Home shows the picture");
        assert_eq!(pic, Rect::new(304, -OVERHANG, 64, 62));
        assert_eq!(
            l.canvas,
            Rect::new(0, -OVERHANG, W, H + OVERHANG),
            "the canvas is the panel and the picture above it: no shadow"
        );
    }

    #[test]
    fn ten_program_rows_fit_and_a_pinned_rule_grows_the_panel_by_four() {
        let mut p = StartPanel::new();
        p.frequent = (0..12).map(|k| prow(&format!("App {k}"), false)).collect();
        assert_eq!(programs_shown(&p), 10);
        assert_eq!(layout(&p).rect(Spot::Program(10)), None);
        p.pinned = vec![prow("Explorer", false)];
        let l = layout(&p);
        assert_eq!(l.panel.height, H + 4);
        assert_eq!(l.rect(Spot::Program(1)), Some(Rect::new(11, 52, 246, 38)));
        assert_eq!(l.rules[0], Rect::new(20, 50, 232, 1), "the pinned rule");
        assert_eq!(l.rect(Spot::Search), Some(Rect::new(21, 438, 222, 22)));
        assert_eq!(l.rect(Spot::AllPrograms), Some(Rect::new(11, 399, 246, 23)));
        assert_eq!(l.pane.bottom(), 471, "the pane grows with the panel");
        assert_eq!(programs_shown(&p), 10);
    }

    #[test]
    fn a_short_room_takes_program_rows_off_and_moves_the_bottom_up() {
        let mut p = StartPanel::new();
        p.frequent = (0..12).map(|k| prow(&format!("App {k}"), false)).collect();
        p.room = Some((1024, H - 1));
        let l = layout(&p);
        assert_eq!(l.panel.height, H - PROGRAM_H, "a whole row comes off");
        assert_eq!(programs_shown(&p), 9);
        assert_eq!(l.rect(Spot::AllPrograms), Some(Rect::new(11, 357, 246, 23)));
        assert_eq!(l.rect(Spot::Search), Some(Rect::new(21, 396, 222, 22)));
        assert_eq!(l.rect(Spot::Shutdown), Some(Rect::new(267, 396, 72, 22)));
        assert_eq!(l.pane.bottom(), 429, "the pane ends with the panel");
        assert_eq!(tree_capacity(&p), 18);

        p.text = TextUnits::new(22, 11);
        p.room = Some((1024, 700));
        let l = layout(&p);
        assert_eq!(
            (l.panel.height, programs_shown(&p)),
            (2 * (H - 4 * PROGRAM_H), 6),
            "at 200% four rows come off a 700 px room"
        );
        assert_eq!(
            l.picture.unwrap().y,
            -OVERHANG,
            "the picture keeps its overhang"
        );

        p.room = Some((1024, 10));
        assert_eq!(programs_shown(&p), 1, "one row always stays");
    }

    fn ten_places() -> Vec<crate::startpanel::PlaceRow> {
        let mut v: Vec<_> = (0..10).map(|k| place(&format!("Place {k}"))).collect();
        v[4].rule_before = true;
        v[7].rule_before = true;
        v
    }

    #[test]
    fn on_a_short_room_the_places_close_up_to_22_px_and_all_ten_show() {
        let mut p = StartPanel::new();
        p.frequent = (0..12).map(|k| prow(&format!("App {k}"), false)).collect();
        p.places = ten_places();
        assert_eq!(place_lift(&p), 0);
        assert_eq!(l_place(&p, 9), Some(Rect::new(266, 365, 138, 33)));

        p.room = Some((1024, 340));
        let l = layout(&p);
        let shutdown = l.rect(Spot::Shutdown).unwrap();
        assert_eq!(shutdown.y, 434 - 4 * PROGRAM_H);
        assert_eq!(place_pitch(&p), 23);
        assert_eq!(
            place_lift(&p),
            6,
            "text and arrow move up into the 21 px row"
        );
        assert_eq!(l.rect(Spot::Place(1)), Some(Rect::new(266, 67, 138, 21)));
        let last = l.rect(Spot::Place(9)).expect("all ten places show");
        assert!(last.bottom() <= shutdown.y, "{last:?} above {shutdown:?}");
        assert!(l.rules.contains(&Rect::new(274, 136, 124, 2)));

        p.room = Some((1024, 10));
        assert_eq!(place_pitch(&p), PLACE_PITCH_LEAST);
        let l = layout(&p);
        let shown = (0..10)
            .filter(|&i| l.rect(Spot::Place(i)).is_some())
            .count();
        assert!(shown < 10, "places that do not fit at 22 px are left out");
        assert!(
            l.rect(Spot::Place(shown - 1)).unwrap().bottom() <= l.rect(Spot::Shutdown).unwrap().y
        );
    }

    fn l_place(p: &StartPanel, i: usize) -> Option<Rect> {
        layout(p).rect(Spot::Place(i))
    }

    #[test]
    fn a_narrow_room_steps_the_text_size_down_and_narrows_the_jump_pane() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.places = ten_places();
        p.jump = vec![row("a.txt")];
        let two = TextUnits::new(22, 11);
        p.fit(two, (1920, 1080));
        assert_eq!(p.text, two, "room enough");
        assert_eq!(layout(&p).panel.width, 2 * W);

        p.fit(two, (800, 600));
        assert_eq!(p.text, TextUnits::new(21, 11));
        assert!(layout(&p).panel.width <= 800);
        p.set_mode(Mode::Jump(0));
        let l = layout(&p);
        assert!(l.panel.width <= 800, "{:?}", l.panel);
        assert_eq!(l.pane.right(), l.jump_pane.unwrap().right());
        assert_eq!(
            l.rect(Spot::Jump(0)).unwrap().right(),
            l.headers[0].1.right()
        );

        p.fit(TextUnits::ONE, (450, 600));
        let l = layout(&p);
        assert_eq!(l.panel.width, 450);
        assert_eq!(l.jump_pane, Some(Rect::new(259, 9, 223 - 41, 413)));
        assert_eq!(
            l.rect(Spot::Jump(0)),
            Some(Rect::new(274, 34, 204 - 41, 22))
        );

        p.fit(TextUnits::ONE, (300, 600));
        let jump_w = layout(&p).panel.width;
        p.set_mode(Mode::Home);
        assert_eq!(
            jump_w,
            layout(&p).panel.width,
            "never narrower than the Home panel"
        );
    }

    #[test]
    fn a_room_narrower_than_the_panel_at_100_percent_narrows_the_right_column() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", false)];
        p.places = ten_places();
        p.jump = vec![row("a.txt")];
        p.fit(TextUnits::new(22, 11), (400, 300));
        assert!(p.text.is_one(), "the text steps down first");
        let l = layout(&p);
        assert_eq!(l.panel.width, 400);
        let place = l.rect(Spot::Place(0)).unwrap();
        assert_eq!((place.x, place.width), (266, 127), "7 px short of the edge");
        let rule = l.rules.iter().find(|r| r.x == 274).unwrap();
        assert_eq!(rule.width, 113);
        assert_eq!(l.picture, Some(Rect::new(299, -OVERHANG, 64, 62)));
        assert_eq!(l.canvas.right(), 400, "the picture stays inside");
        assert_eq!(
            l.rect(Spot::ShutdownArrow).unwrap().right(),
            361,
            "Shut down stays put"
        );

        p.set_mode(Mode::Jump(0));
        let l = layout(&p);
        assert_eq!(l.panel.width, 400);
        assert_eq!(l.jump_pane.unwrap().right(), 391);

        p.set_mode(Mode::Home);
        p.key(Key::Char(EditKey::Insert("n".into())));
        p.results = vec![ResultGroup {
            title: "Programs".into(),
            rows: vec![row("Notepad")],
        }];
        let l = layout(&p);
        assert_eq!(p.mode(), Mode::Search);
        assert_eq!(l.panel.width, 400);
        assert_eq!(l.pane.right(), 391);
        assert_eq!(l.headers[0].1.right(), 381);
        assert_eq!(l.rect(Spot::Result(0, 0)).unwrap().right(), 385);

        p.fit(TextUnits::ONE, (300, 300));
        assert_eq!(
            layout(&p).panel.width,
            W - NARROW_MOST,
            "never past 40 px narrower"
        );
    }

    #[test]
    fn places_stack_at_a_35_px_pitch_with_3_px_for_a_rule() {
        let mut p = StartPanel::new();
        p.places = ["user", "Documents", "Pictures", "Music", "Computer"]
            .map(place)
            .to_vec();
        p.places[4].rule_before = true;
        let l = layout(&p);
        assert_eq!(l.rect(Spot::Place(0)), Some(Rect::new(266, 44, 138, 33)));
        assert_eq!(l.rect(Spot::Place(3)), Some(Rect::new(266, 149, 138, 33)));
        assert_eq!(l.rect(Spot::Place(4)), Some(Rect::new(266, 187, 138, 33)));
        assert!(l.rules.contains(&Rect::new(274, 184, 124, 2)));
    }

    #[test]
    fn search_mode_widens_the_pane_and_hides_the_places() {
        let mut p = StartPanel::new();
        p.places = vec![place("Documents")];
        p.key(Key::Char(EditKey::Insert("x".into())));
        p.set_results(vec![
            ResultGroup {
                title: "Programs (2)".into(),
                rows: vec![row("Xterm"), row("Xed")],
            },
            ResultGroup {
                title: "Files (1)".into(),
                rows: vec![row("x.txt")],
            },
        ]);
        let l = layout(&p);
        assert_eq!(l.pane.right(), 402);
        assert_eq!(l.rect(Spot::Place(0)), None);
        assert_eq!(l.picture, None);
        assert_eq!(l.rect(Spot::AllPrograms), None);
        assert_eq!(l.headers[0], (0, Rect::new(17, 26, 375, 21)));
        assert_eq!(l.rect(Spot::Result(0, 0)), Some(Rect::new(25, 47, 371, 21)));
        assert_eq!(l.headers[1], (1, Rect::new(17, 100, 375, 21)));
        assert_eq!(
            l.rect(Spot::Result(1, 0)),
            Some(Rect::new(25, 121, 371, 21))
        );
        let clear = l.rect(Spot::Clear).expect("a clear button once typed");
        assert_eq!(hit(&l, clear.x + 2, clear.y + 2), Some(Spot::Clear));
        assert_eq!(l.rect(Spot::Shutdown), Some(Rect::new(267, 434, 72, 22)));
    }

    #[test]
    fn two_full_groups_of_results_fit() {
        let mut p = StartPanel::new();
        p.key(Key::Char(EditKey::Insert("a".into())));
        let full = || ResultGroup {
            title: "G".into(),
            rows: (0..8).map(|k| row(&format!("r{k}"))).collect(),
        };
        p.set_results(vec![full(), full()]);
        let l = layout(&p);
        let last = l.rect(Spot::Result(1, 7)).expect("the last row shows");
        assert!(last.bottom() <= 418, "above the All Programs row's bottom");
    }

    #[test]
    fn jump_mode_widens_the_panel_and_lists_the_jump_rows_in_their_pane() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", true)];
        p.places = vec![place("Documents")];
        p.jump = vec![row("a.txt"), row("b.txt")];
        p.set_mode(Mode::Jump(0));
        let l = layout(&p);
        assert_eq!(l.panel.width, JUMP_W);
        assert_eq!(l.pane, Rect::new(9, 9, 473, 458));
        assert_eq!(l.jump_pane, Some(Rect::new(259, 9, 223, 413)));
        assert_eq!(l.headers, [(0, Rect::new(274, 12, 204, 22))]);
        assert_eq!(l.rect(Spot::Jump(1)), Some(Rect::new(274, 56, 204, 22)));
        assert_eq!(l.rect(Spot::Place(0)), None);
        assert_eq!(l.picture, None);
        assert!(l.rect(Spot::Program(0)).is_some(), "the programs stay");
        assert!(l.rect(Spot::Shutdown).is_some());
    }

    #[test]
    fn the_tree_shows_a_window_of_19_px_rows_with_a_bar_when_long() {
        let mut p = StartPanel::new();
        p.tree = (0..25)
            .map(|k| TreeRow::App {
                name: format!("App {k}"),
                icon: None,
                depth: 0,
            })
            .collect();
        p.set_mode(Mode::Programs);
        assert_eq!(tree_capacity(&p), 20);
        p.tree_top = 3;
        let l = layout(&p);
        assert_eq!(l.rect(Spot::Tree(3)), Some(Rect::new(11, 10, 231, 19)));
        assert_eq!(l.rect(Spot::Tree(22)), Some(Rect::new(11, 371, 231, 19)));
        assert_eq!(l.rect(Spot::Tree(2)), None);
        assert_eq!(l.rect(Spot::Tree(23)), None);
        assert_eq!(l.tree_bar, Some(Rect::new(242, 10, 16, 380)));
        assert!(l.rect(Spot::Program(0)).is_none());
        assert!(l.rect(Spot::AllPrograms).is_some(), "Back");
    }

    #[test]
    fn scaled_doubles_every_rect_and_hit_finds_spots() {
        let mut p = StartPanel::new();
        p.frequent = vec![prow("Notepad", false)];
        let l = layout(&p);
        let s = l.scaled(2);
        assert_eq!(s.scale, 2);
        assert_eq!(s.panel, l.panel.scaled(2));
        assert_eq!(s.canvas, l.canvas.scaled(2));
        assert_eq!(s.rect(Spot::Program(0)), Some(Rect::new(22, 20, 492, 76)));
        assert_eq!(hit(&s, 30, 30), Some(Spot::Program(0)));
        assert_eq!(hit(&l, 30, 30), Some(Spot::Program(0)));
        assert_eq!(hit(&l, 330, 0), Some(Spot::Picture));
        assert_eq!(hit(&l, 2, 2), None, "the glass edge");
    }

    /// At 200% every part's rect doubles and as many rows show; the rules keep their pixel, the
    /// picture's frame its 64x62 centred where the doubled frame would be.
    #[test]
    fn at_200_percent_the_panel_grows_and_its_rules_and_picture_keep_their_size() {
        let mut p = StartPanel::new();
        p.frequent = (0..12).map(|k| prow(&format!("App {k}"), false)).collect();
        p.text = TextUnits::new(22, 11);
        let l = layout(&p);
        assert_eq!(l.panel, Rect::new(0, 0, 2 * W, 2 * H));
        assert_eq!(l.pane, Rect::new(18, 18, 500, 916));
        assert_eq!(l.rect(Spot::Program(1)), Some(Rect::new(22, 96, 492, 76)));
        assert_eq!(l.rect(Spot::Search), Some(Rect::new(42, 868, 444, 44)));
        assert_eq!(l.rect(Spot::Shutdown), Some(Rect::new(534, 868, 144, 44)));
        assert_eq!(programs_shown(&p), 10);
        assert_eq!(l.rules, [Rect::new(40, 782, 464, 1)], "a pixel thick");
        let frame = Rect::new(640, -OVERHANG, 64, 62);
        assert_eq!(l.picture, Some(frame));
        assert_eq!(l.rect(Spot::Picture), Some(frame));
        assert_eq!(l.canvas, Rect::new(0, -OVERHANG, 2 * W, 2 * H + OVERHANG));
        assert_eq!(hit(&l, 30, 100), Some(Spot::Program(1)));
    }

    /// The tree's bar keeps its 16 px at the grown rows' right, and the rows reach it.
    #[test]
    fn at_200_percent_the_tree_bar_keeps_its_width() {
        let mut p = StartPanel::new();
        p.tree = (0..25)
            .map(|k| TreeRow::App {
                name: format!("App {k}"),
                icon: None,
                depth: 0,
            })
            .collect();
        p.set_mode(Mode::Programs);
        p.text = TextUnits::new(22, 11);
        assert_eq!(tree_capacity(&p), 20);
        let l = layout(&p);
        assert_eq!(l.tree_bar, Some(Rect::new(500, 20, 16, 760)));
        assert_eq!(l.rect(Spot::Tree(1)), Some(Rect::new(22, 58, 478, 38)));
    }
}
