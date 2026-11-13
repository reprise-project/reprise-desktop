//! The clock popup's event list: the listed day's heading over [`AGENDA_ROWS`] rows of time and
//! summary, the last counting the rest when they overflow. The host reads and words the events.

use super::{MONTHS, PopupHit, weekday};
use crate::access::{self, Tree, accesskit};
use crate::look::Paint;
use crate::look::widgets::item::{self, Mark};
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer};
use accesskit::{NodeId, Role};

pub const AGENDA_ROWS: usize = 4;
const HEADING_H: i32 = 16;
const ROW_H: i32 = 16;
/// Text inside a row, from either end.
const INSET: i32 = 4;
/// Between the time column and the summary.
const GAP: i32 = 8;

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayEvent {
    pub all_day: bool,
    /// Its start in seconds, ordering the timed events.
    pub start: i64,
    /// The time column as the host words it: "All day", "9:30", "Until 7:00".
    pub when: String,
    pub summary: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agenda {
    /// The day listed: year, month, day.
    pub day: (i32, u32, u32),
    /// None until read.
    pub events: Option<Vec<DayEvent>>,
    /// A click on a row opens a calendar program; without one the rows neither light nor click.
    pub opens: bool,
}

/// What a row of the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row<'a> {
    Event(&'a DayEvent),
    /// How many events the rows above leave out.
    More(usize),
    /// The day has none.
    Empty,
}

impl Agenda {
    pub fn new(day: (i32, u32, u32), opens: bool) -> Agenda {
        Agenda {
            day,
            events: None,
            opens,
        }
    }

    /// Lists `events`: all-day ones first, then by start, then by summary.
    pub fn set_events(&mut self, mut events: Vec<DayEvent>) {
        events.sort_by(|a, b| {
            (!a.all_day, a.start, &a.summary).cmp(&(!b.all_day, b.start, &b.summary))
        });
        self.events = Some(events);
    }

    /// The rows shown: none until read, "No events" for an empty day, else the events, the last
    /// row counting the rest when they outnumber the rows.
    pub fn rows(&self) -> Vec<Row<'_>> {
        let Some(events) = &self.events else {
            return Vec::new();
        };
        if events.is_empty() {
            return vec![Row::Empty];
        }
        if events.len() <= AGENDA_ROWS {
            return events.iter().map(Row::Event).collect();
        }
        let shown = AGENDA_ROWS - 1;
        let mut rows: Vec<Row<'_>> = events[..shown].iter().map(Row::Event).collect();
        rows.push(Row::More(events.len() - shown));
        rows
    }

    /// "Today", else the day's weekday and date, with its year when that is not this year.
    pub fn heading(&self, today: Option<(i32, u32, u32)>) -> String {
        if Some(self.day) == today {
            return "Today".into();
        }
        let (y, m, d) = self.day;
        let name = format!(
            "{}, {} {d}",
            WEEKDAYS[weekday(y, m, d) as usize],
            MONTHS[(m.clamp(1, 12) - 1) as usize]
        );
        match today {
            Some((year, ..)) if year != y => format!("{name}, {y}"),
            _ => name,
        }
    }
}

fn row_label(row: Row<'_>) -> String {
    match row {
        Row::Event(e) => format!("{} {}", e.when, e.summary),
        Row::More(1) => "1 more event".into(),
        Row::More(n) => format!("{n} more events"),
        Row::Empty => "No events".into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub heading: Rect,
    pub rows: [Rect; AGENDA_ROWS],
}

/// The list's height at `cm.text`.
pub(super) fn height(cm: &ControlMetrics) -> i32 {
    cm.text.len(HEADING_H) + AGENDA_ROWS as i32 * cm.text.len(ROW_H)
}

/// The list from `r`'s top-left, `r.width` wide.
pub(super) fn layout(r: Rect, cm: &ControlMetrics) -> Layout {
    let heading = Rect::new(r.x, r.y, r.width, cm.text.len(HEADING_H));
    let h = cm.text.len(ROW_H);
    let rows = std::array::from_fn(|i| Rect::new(r.x, heading.bottom() + i as i32 * h, r.width, h));
    Layout { heading, rows }
}

/// The row point (`x`, `y`) lands on, when a click there opens something.
pub(super) fn hit(a: &Agenda, lay: &Layout, x: i32, y: i32) -> Option<usize> {
    if !a.opens {
        return None;
    }
    let rows = a.rows();
    let i = lay.rows.iter().position(|r| r.contains(x, y))?;
    matches!(rows.get(i), Some(Row::Event(_) | Row::More(_))).then_some(i)
}

/// The heading in bold where `bold`, then the rows, row `hot` names lit as a hovered list item.
#[expect(clippy::too_many_arguments)]
pub(super) fn draw(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    a: &Agenda,
    today: Option<(i32, u32, u32)>,
    lay: &Layout,
    hot: PopupHit,
    bold: bool,
    px: u32,
) {
    let (cs, s) = (p.cs, p.s);
    let inset = lay.heading.scaled(s);
    let inset = Rect::new(
        inset.x + INSET * s,
        inset.y,
        inset.width - 2 * INSET * s,
        inset.height,
    );
    let heading = a.heading(today);
    text.draw(c, inset, &heading, bold, cs.button_text, Align::Left, px);
    let rows = a.rows();
    let when_w = (rows.iter())
        .filter_map(|r| match r {
            Row::Event(e) => Some(text.measure(&e.when, false, px)),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    for (i, row) in rows.iter().enumerate() {
        let r = lay.rows[i].scaled(s);
        let mut ink = cs.button_text;
        if a.opens && hot == PopupHit::Event(i) && !matches!(row, Row::Empty) {
            let mark = Mark {
                hot: true,
                ..Mark::default()
            };
            if !item::draw_item(c, p, r, mark) {
                c.fill_rect(r, cs.hilight);
                ink = cs.hilight_text;
            }
        }
        let inner = Rect::new(r.x + INSET * s, r.y, r.width - 2 * INSET * s, r.height);
        match row {
            Row::Event(e) => {
                let when = Rect::new(inner.x, inner.y, when_w, inner.height);
                text.draw(c, when, &e.when, false, ink, Align::Left, px);
                let x = when.right() + GAP * s;
                let summary = Rect::new(x, inner.y, (inner.right() - x).max(0), inner.height);
                let label = text.ellipsize(&e.summary, summary.width, false, px);
                text.draw(c, summary, &label, false, ink, Align::Left, px);
            }
            Row::More(_) => {
                text.draw(c, inner, &row_label(*row), false, ink, Align::Left, px);
            }
            Row::Empty => {
                text.draw(
                    c,
                    inner,
                    &row_label(*row),
                    false,
                    cs.gray_text,
                    Align::Left,
                    px,
                );
            }
        }
    }
}

/// The heading as a label and each row: a button a reader can click when a click opens it, else a
/// label.
pub(super) fn access(
    a: &Agenda,
    today: Option<(i32, u32, u32)>,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
) {
    let heading = access::named(Role::Label, lay.heading, &a.heading(today));
    tree.push(parent, "agenda", heading);
    for (i, row) in a.rows().into_iter().enumerate() {
        let opens = a.opens && !matches!(row, Row::Empty);
        let role = if opens { Role::Button } else { Role::Label };
        let mut n = access::named(role, lay.rows[i], &row_label(row));
        if opens {
            n.add_action(accesskit::Action::Click);
        }
        tree.push(parent, ("agenda", i), n);
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Calendar, draw_popup, popup_access, popup_hit, popup_layout, popup_size};
    use super::*;
    use crate::interface::Interface;
    use crate::theme::Theme;

    fn fnv(c: &Canvas) -> u64 {
        let words = [c.width, c.height]
            .into_iter()
            .chain(c.pixels.iter().copied());
        words
            .flat_map(u32::to_le_bytes)
            .fold(0xcbf2_9ce4_8422_2325, |h, b| {
                (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
            })
    }

    fn event(all_day: bool, start: i64, when: &str, summary: &str) -> DayEvent {
        DayEvent {
            all_day,
            start,
            when: when.into(),
            summary: summary.into(),
        }
    }

    /// November 13, 2026, today, with `n` events: an all-day one listed last, then timed ones.
    fn agenda(n: usize) -> Agenda {
        let mut a = Agenda::new((2026, 11, 13), true);
        let mut events: Vec<DayEvent> = (1..n)
            .map(|i| {
                event(
                    false,
                    3600 * i as i64,
                    &format!("{}:00", 8 + i),
                    &format!("Meeting {i}"),
                )
            })
            .collect();
        if n > 0 {
            events.push(event(true, 0, "All day", "Team offsite planning"));
        }
        a.set_events(events);
        a
    }

    fn popup(n: usize) -> Calendar {
        let mut cal = Calendar::new(2026, 11, None, 0);
        cal.today = Some((2026, 11, 13));
        cal.agenda = Some(agenda(n));
        cal
    }

    #[test]
    fn all_day_events_come_first_and_past_the_rows_the_last_counts_the_rest() {
        let a = agenda(3);
        let labels: Vec<String> = a.rows().into_iter().map(row_label).collect();
        assert_eq!(
            labels,
            [
                "All day Team offsite planning",
                "9:00 Meeting 1",
                "10:00 Meeting 2"
            ]
        );
        let a = agenda(7);
        let labels: Vec<String> = a.rows().into_iter().map(row_label).collect();
        assert_eq!(
            labels,
            [
                "All day Team offsite planning",
                "9:00 Meeting 1",
                "10:00 Meeting 2",
                "4 more events"
            ]
        );
        assert_eq!(agenda(5).rows()[3], Row::More(2));
        assert_eq!(agenda(4).rows().len(), 4, "four fit without a count");
        assert_eq!(agenda(0).rows(), [Row::Empty]);
        assert_eq!(
            Agenda::new((2026, 11, 13), true).rows(),
            [],
            "nothing until read"
        );
    }

    #[test]
    fn the_heading_names_today_or_the_date() {
        let mut a = Agenda::new((2026, 11, 13), true);
        let today = Some((2026, 11, 13));
        assert_eq!(a.heading(today), "Today");
        a.day = (2026, 11, 14);
        assert_eq!(a.heading(today), "Saturday, November 14");
        a.day = (2027, 1, 1);
        assert_eq!(a.heading(today), "Friday, January 1, 2027");
    }

    #[test]
    fn the_list_grows_the_popup_under_the_month_and_its_rows_click() {
        let cm = ControlMetrics::CLASSIC;
        assert_eq!(popup_size(&cm, true), (174, 162 + 4 + 16 + 4 * 16));
        let lay = popup_layout(&cm, true);
        let list = lay.agenda.unwrap();
        assert_eq!(list.heading, Rect::new(6, 160, 162, 16));
        assert_eq!(list.rows[3], Rect::new(6, 224, 162, 16));
        assert_eq!(list.rows[3].bottom() + 6, popup_size(&cm, true).1);
        let cal = popup(7);
        let mid = |r: Rect| (r.x + r.width / 2, r.y + r.height / 2);
        let (x, y) = mid(list.rows[3]);
        assert_eq!(
            popup_hit(&cm, &cal, x, y),
            PopupHit::Event(3),
            "the count opens too"
        );
        let (x, y) = mid(list.heading);
        assert_eq!(popup_hit(&cm, &cal, x, y), PopupHit::Panel);
        let cell = lay.grid.cell(cal.cell_of(20));
        assert_eq!(
            popup_hit(&cm, &cal, cell.x + 2, cell.y + 2),
            PopupHit::Day(20)
        );
        let empty = popup(0);
        let (x, y) = mid(list.rows[0]);
        assert_eq!(
            popup_hit(&cm, &empty, x, y),
            PopupHit::Panel,
            "No events is inert"
        );
        let mut nothing_opens = popup(3);
        nothing_opens.agenda.as_mut().unwrap().opens = false;
        assert_eq!(popup_hit(&cm, &nothing_opens, x, y), PopupHit::Panel);
        let bare = Calendar::new(2026, 11, None, 0);
        assert_eq!(
            popup_hit(&cm, &bare, cell.x + 2, cell.y + 2),
            PopupHit::Panel,
            "without a list the days stay read-only"
        );
        let glass = crate::interfaces::theme_for(Interface::Glass).controls();
        let list = popup_layout(&glass, true).agenda.unwrap();
        let time = popup_layout(&glass, false).time.unwrap();
        assert_eq!(
            (list.heading.y, list.heading.width),
            (time.bottom() + 4, 327 - 24)
        );
    }

    /// Each look's popup with an event list, its first row lit, at scale 1 and 2.
    #[test]
    fn the_list_pins_in_each_look_at_scale_1_and_2() {
        crate::pins::fonts();
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            for scale in [1, 2] {
                let theme = crate::interfaces::theme_for(i).scaled(scale);
                for (key, cal, hot) in [
                    ("five", popup(5), PopupHit::Event(0)),
                    ("empty", popup(0), PopupHit::Panel),
                ] {
                    let c = draw_popup(&theme, &text, &cal, hot);
                    got.push((format!("{i:?} {key} {scale}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("controls/calendar_agenda", &got);
    }

    #[test]
    fn a_lit_row_differs_and_the_picked_day_is_highlighted() {
        let theme = Theme::default();
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let cal = popup(3);
        let plain = draw_popup(&theme, &text, &cal, PopupHit::Panel);
        let lit = draw_popup(&theme, &text, &cal, PopupHit::Event(1));
        let row = popup_layout(&ControlMetrics::CLASSIC, true)
            .agenda
            .unwrap()
            .rows[1];
        let changed = plain.differing_rect(&lit);
        assert!(!changed.is_empty());
        assert_eq!(changed.intersect(&row), changed, "only row 1 lights");
        assert_eq!(lit.get(row.x + 1, row.y + 1), theme.colors.hilight.argb());
        let mut picked = cal.clone();
        picked.agenda.as_mut().unwrap().day = (2026, 11, 20);
        let other = draw_popup(&theme, &text, &picked, PopupHit::Panel);
        let lay = popup_layout(&ControlMetrics::CLASSIC, true);
        let cell = lay.grid.cell(cal.cell_of(20));
        let hilit = (cell.y..cell.bottom())
            .flat_map(|y| (cell.x..cell.right()).map(move |x| (x, y)))
            .filter(|&(x, y)| other.get(x, y) == theme.colors.hilight.argb())
            .count();
        assert!(hilit > 20, "the 20th is picked: {hilit} highlight pixels");
    }

    #[test]
    fn a_reader_hears_the_heading_and_clicks_the_rows() {
        let text = TextRenderer::load(&crate::theme::FontConfig::default());
        let cm = ControlMetrics::CLASSIC;
        let mut t = Tree::window("", popup_size(&cm, true));
        let mut cal = popup(2);
        cal.agenda.as_mut().unwrap().day = (2026, 11, 20);
        let grid = popup_access(&cal, &cm, &text, 11, &mut t, access::ROOT);
        let run = t.get(grid).unwrap().children()[0];
        assert_eq!(t.get(run).unwrap().value(), Some("November 20, 2026"));
        assert!(t.find(Role::Label, "Friday, November 20").is_some());
        let row = t.find(Role::Button, "9:00 Meeting 1").expect("a row");
        assert!(
            t.get(row)
                .unwrap()
                .supports_action(accesskit::Action::Click)
        );
    }
}
