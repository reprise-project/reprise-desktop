//! The folder window's task pane: collapsible sections, each a title band over link rows or plain text lines. The look paints the pieces through [`crate::look::widgets::taskpane`].

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::scrollbar_parts;
use crate::controls::metrics::{ControlMetrics, TaskMetrics};
use crate::look::Paint;
use crate::look::widgets::taskpane as look;
use crate::look::widgets::{Interaction, draw_scrollbar};
use crate::raster::{Canvas, Rect};
use crate::text::{Align, TextRenderer, wrap, wrap_bold};
use accesskit::{NodeId, Role};

pub const ROW_H: i32 = 20;
/// Background between one section's bottom border and the next header.
pub const GAP: i32 = 15;

const GROUP_GAP: i32 = 5;

/// A Details line's pitch: `lh` for text, a gap for an empty line.
fn line_h(line: &str, lh: i32) -> i32 {
    if line.is_empty() { GROUP_GAP } else { lh }
}
/// A row is its content plus this much room below it; the icon and first line both start at the row's top.
const ROW_PAD: i32 = 4;
const ICON: i32 = 16;
const ICON_GAP: i32 = 6;
/// Where the header's title, a `Lines` line and a link's icon all start, from the section's left border.
const TEXT_INSET: i32 = 13;
const BODY_TOP_PAD: i32 = 10;
const LINKS_BOTTOM_PAD: i32 = 6;
const LINES_BOTTOM_PAD: i32 = 10;
const TITLE_RIGHT: i32 = 18;
const SCROLL_STEP: i32 = ROW_H;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Row {
    pub icon: Option<Canvas>,
    pub text: String,
    pub enabled: bool,
}

impl Row {
    pub fn new(text: impl Into<String>, icon: Option<Canvas>) -> Self {
        Self {
            icon,
            text: text.into(),
            enabled: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Links(Vec<Row>),
    Lines(Vec<(String, bool)>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub title: String,
    pub open: bool,
    pub body: Body,
}

impl Section {
    pub fn links(title: impl Into<String>, rows: Vec<Row>) -> Self {
        Self {
            title: title.into(),
            open: true,
            body: Body::Links(rows),
        }
    }

    pub fn lines(title: impl Into<String>, lines: Vec<(String, bool)>) -> Self {
        Self {
            title: title.into(),
            open: true,
            body: Body::Lines(lines),
        }
    }

    /// What a section holds: links, or the Details pane's plain lines (bold when flagged); an empty line is the 5 px gap between groups.
    pub fn hidden(&self) -> bool {
        matches!(&self.body, Body::Links(rows) if rows.is_empty())
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct TaskPane {
    pub sections: Vec<Section>,
    /// A links section with no links is hidden: no header or body, but the gap under it stays.
    pub top: i32,
    pub hot: Option<(usize, usize)>,
    pub hot_header: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Layout {
    pub frame: Rect,
    pub back: Rect,
    /// What the look's background covers: the frame after the metrics' `lead` columns and above its `foot` rows.
    pub text_dx: i32,
    /// A line of text's pitch ([`TaskMetrics::line`]).
    pub line_h: i32,
    pub headers: Vec<Rect>,
    pub rows: Vec<Vec<Rect>>,
    pub link_lines: Vec<Vec<Vec<String>>>,
    pub lines: Vec<Vec<(String, bool)>>,
    pub bodies: Vec<Rect>,
    pub content_h: i32,
    pub scrollbar: Option<Rect>,
    pub thumb_min: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Header(usize),
    Row(usize, usize),
    ScrollUp,
    ScrollDown,
    Scrollbar,
    Nothing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Toggle(usize),
    Row(usize, usize),
}

const TITLE_DY: i32 = 2;

/// Where a section's text stops: [`TEXT_INSET`] in from its right border, matching the inset titles, icons and lines start at.
fn text_right(sect_x: i32, sect_w: i32) -> i32 {
    sect_x + sect_w - 1 - TEXT_INSET
}

/// A `Lines` body's text box inside a section `sect_w` wide: the symmetric inset [`text_right`] gives a link, with no icon to step past.
fn lines_width(sect_w: i32) -> i32 {
    (sect_w - 1 - 2 * TEXT_INSET).max(1)
}

/// A `Lines` body's text wrapped to `w` px, each output line keeping its bold flag; an empty line stays one empty line.
fn lines_of(text: &TextRenderer, lines: &[(String, bool)], w: i32, px: u32) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for (line, bold) in lines {
        if line.is_empty() {
            out.push((String::new(), *bold));
            continue;
        }
        for part in wrap_bold(text, line, w.max(1), px, *bold) {
            out.push((part, *bold));
        }
    }
    out
}

/// Every section's header, body and rows inside `r`, offset by `tp.top`. When the sections need more room than `r` has, a scrollbar takes the pane's right edge and the layout is redone in what is left.
pub fn layout(tp: &TaskPane, text: &TextRenderer, px: u32, r: Rect, cm: &ControlMetrics) -> Layout {
    let mut lay = lay_in(tp, text, px, r, None, &cm.task);
    if lay.content_h > r.height {
        let sb = Rect::new(r.right() - cm.scroll, r.y, cm.scroll, r.height);
        lay = lay_in(tp, text, px, r, Some(sb), &cm.task);
    }
    lay.thumb_min = cm.thumb_min;
    lay
}

fn lay_in(
    tp: &TaskPane,
    text: &TextRenderer,
    px: u32,
    r: Rect,
    sb: Option<Rect>,
    tm: &TaskMetrics,
) -> Layout {
    let pane_w = r.width - sb.map_or(0, |s| s.width);
    let sect_w = (pane_w - tm.section_inset).max(1);
    let sect_x = r.x + tm.section_x;
    let text_x = sect_x + TEXT_INSET + ICON + ICON_GAP + tm.text_dx;
    let text_w = (text_right(sect_x, sect_w) - text_x).max(1);
    let lead = tm.lead.min(r.width);
    let foot = tm.foot.min(r.height);
    let mut lay = Layout {
        frame: r,
        back: Rect::new(r.x + lead, r.y, r.width - lead, r.height - foot),
        text_dx: tm.text_dx,
        line_h: tm.line,
        scrollbar: sb,
        ..Default::default()
    };
    let mut y = r.y + tm.top - tp.top;
    for s in &tp.sections {
        if s.hidden() {
            lay.headers.push(Rect::new(sect_x, y, sect_w, 0));
            lay.bodies.push(Rect::new(sect_x, y, sect_w, 0));
            lay.rows.push(Vec::new());
            lay.link_lines.push(Vec::new());
            lay.lines.push(Vec::new());
            y += GAP;
            continue;
        }
        lay.headers.push(Rect::new(sect_x, y, sect_w, tm.header));
        y += tm.header;
        if !s.open {
            lay.bodies.push(Rect::new(sect_x, y, sect_w, 0));
            lay.rows.push(Vec::new());
            lay.link_lines.push(Vec::new());
            lay.lines.push(Vec::new());
            y += GAP;
            continue;
        }
        let mut rows = Vec::new();
        let mut row_lines = Vec::new();
        let mut body_lines = Vec::new();
        let body_h = match &s.body {
            Body::Links(links) => {
                let mut ry = y + BODY_TOP_PAD;
                for row in links {
                    let wrapped = wrap(text, &row.text, text_w, px);
                    let h = (wrapped.len().max(1) as i32 * tm.line).max(ICON) + ROW_PAD;
                    rows.push(Rect::new(sect_x + TEXT_INSET, ry, sect_w - TEXT_INSET, h));
                    row_lines.push(wrapped);
                    ry += h;
                }
                ry - y + LINKS_BOTTOM_PAD + 1
            }
            Body::Lines(lines) => {
                body_lines = lines_of(text, lines, lines_width(sect_w), px);
                let text_h: i32 = body_lines.iter().map(|(l, _)| line_h(l, tm.line)).sum();
                BODY_TOP_PAD + text_h + LINES_BOTTOM_PAD + 1
            }
        };
        lay.bodies.push(Rect::new(sect_x, y, sect_w, body_h));
        lay.rows.push(rows);
        lay.link_lines.push(row_lines);
        lay.lines.push(body_lines);
        y += body_h + GAP;
    }
    lay.content_h = y + tp.top - r.y;
    lay
}

pub fn hit(tp: &TaskPane, lay: &Layout, x: i32, y: i32) -> Hit {
    if let Some(sb) = lay.scrollbar
        && sb.contains(x, y)
    {
        let view = lay.frame.height.max(1) as usize;
        let parts = scrollbar_parts(
            sb,
            1,
            tp.top.max(0) as usize,
            view,
            lay.content_h.max(0) as usize,
            false,
            lay.thumb_min,
        );
        return if parts.back.contains(x, y) {
            Hit::ScrollUp
        } else if parts.forward.contains(x, y) {
            Hit::ScrollDown
        } else {
            Hit::Scrollbar
        };
    }
    for (i, h) in lay.headers.iter().enumerate() {
        if h.contains(x, y) {
            return Hit::Header(i);
        }
    }
    for (i, rows) in lay.rows.iter().enumerate() {
        if !tp.sections.get(i).is_some_and(|s| s.open) {
            continue;
        }
        for (j, row) in rows.iter().enumerate() {
            if row.contains(x, y) {
                return Hit::Row(i, j);
            }
        }
    }
    Hit::Nothing
}

impl TaskPane {
    /// Act on a click: a header collapses or expands its section, an enabled link reports itself, an arrow scrolls. The host re-lays out and clamps `top` against `content_h`; only the top end is clamped here.
    pub fn click(&mut self, hit: Hit) -> Event {
        match hit {
            Hit::Header(i) => {
                if let Some(s) = self.sections.get_mut(i) {
                    s.open = !s.open;
                    return Event::Toggle(i);
                }
                Event::None
            }
            Hit::Row(i, j) => match &self.sections.get(i).map(|s| &s.body) {
                Some(Body::Links(rows)) if rows.get(j).is_some_and(|r| r.enabled) => {
                    Event::Row(i, j)
                }
                _ => Event::None,
            },
            Hit::ScrollUp => {
                self.top = (self.top - SCROLL_STEP).max(0);
                Event::Redraw
            }
            Hit::ScrollDown => {
                self.top += SCROLL_STEP;
                Event::Redraw
            }
            Hit::Scrollbar | Hit::Nothing => Event::None,
        }
    }
}

/// The pane: background, each section's title band and body, links and lines, and the scrollbar when it overflows.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, tp: &TaskPane, lay: &Layout, px: u32) {
    let Paint { cs, s, .. } = *p;
    c.fill_rect(lay.frame.scaled(s), cs.window);
    look::draw_back(c, p, lay.back.scaled(s));
    for (i, sect) in tp.sections.iter().enumerate() {
        let (Some(&head), Some(&body)) = (lay.headers.get(i), lay.bodies.get(i)) else {
            continue;
        };
        if sect.hidden() {
            continue;
        }
        let hr = head.scaled(s);
        let title = Rect::new(
            hr.x + TEXT_INSET * s,
            hr.y + TITLE_DY * s,
            (head.width - TEXT_INSET - TITLE_RIGHT).max(0) * s,
            hr.height,
        );
        let ptr = if tp.hot_header == Some(i) {
            Interaction::Hot
        } else {
            Interaction::Idle
        };
        look::draw_header(c, text, p, hr, title, &sect.title, sect.open, ptr, px);
        if !sect.open || body.height == 0 {
            continue;
        }
        look::draw_body(c, p, body.scaled(s));
        match &sect.body {
            Body::Links(links) => {
                for (j, row) in links.iter().enumerate() {
                    let (Some(&rr), Some(wrapped)) = (
                        lay.rows.get(i).and_then(|r| r.get(j)),
                        lay.link_lines.get(i).and_then(|l| l.get(j)),
                    ) else {
                        continue;
                    };
                    if let Some(icon) = &row.icon {
                        c.blit_over(rr.x * s, rr.y * s, icon);
                    }
                    let tx = rr.x + ICON + ICON_GAP + lay.text_dx;
                    let tw = (text_right(body.x, body.width) - tx).max(1);
                    let hot = row.enabled && tp.hot == Some((i, j));
                    let ink = look::link_ink(p, row.enabled, hot);
                    let mut ly = rr.y;
                    for line in wrapped {
                        let lr = Rect::new(tx * s, ly * s, tw * s, lay.line_h * s);
                        text.draw(c, lr, line, false, ink, Align::Left, px);
                        if hot {
                            let w = text.measure(line, false, px);
                            c.fill_rect(Rect::new(lr.x, lr.bottom() - s, w, s), ink);
                        }
                        ly += lay.line_h;
                    }
                }
            }
            Body::Lines(_) => {
                let tx = body.x + TEXT_INSET + lay.text_dx;
                let tw = (text_right(body.x, body.width) - tx).max(1);
                let mut ly = body.y + BODY_TOP_PAD;
                for (line, bold) in lay.lines.get(i).into_iter().flatten() {
                    let lr = Rect::new(tx * s, ly * s, tw * s, lay.line_h * s);
                    text.draw(c, lr, line, *bold, cs.window_text, Align::Left, px);
                    ly += line_h(line, lay.line_h);
                }
            }
        }
    }
    if let Some(sb) = lay.scrollbar {
        let view = lay.frame.height.max(1) as usize;
        draw_scrollbar(
            c,
            p,
            sb.scaled(s),
            tp.top.max(0) as usize,
            view,
            lay.content_h.max(0) as usize,
            None,
        );
    }
}

/// The strip `gap` between the pane and the host's view, in pane coordinates: the window colour, with the look's pane background carried through over the pane's rows so a gradient runs on unbroken.
pub fn draw_gap(c: &mut Canvas, p: &Paint, lay: &Layout, gap: Rect) {
    let s = p.s;
    c.fill_rect(gap.scaled(s), p.cs.window);
    let back = Rect::new(gap.x, lay.back.y, gap.width, lay.back.height);
    look::draw_back(c, p, back.intersect(&gap).scaled(s));
}

/// The task pane's `access` part: each section a group named by its title, holding its links or lines while open.
pub fn access(tp: &TaskPane, lay: &Layout, tree: &mut Tree, parent: NodeId) -> NodeId {
    let pane = tree.push(parent, "tasks", access::node(Role::Pane, lay.frame));
    for (k, s) in tp.sections.iter().enumerate() {
        if s.hidden() {
            continue;
        }
        let head = lay.headers.get(k).copied().unwrap_or_default();
        let body = lay.bodies.get(k).copied().unwrap_or_default();
        let mut g = access::named(Role::Group, head.union(&body), &s.title);
        g.set_expanded(s.open);
        let g = tree.push(pane, k, g);
        if !s.open {
            continue;
        }
        match &s.body {
            Body::Links(rows) => {
                for (i, row) in rows.iter().enumerate() {
                    let r = lay
                        .rows
                        .get(k)
                        .and_then(|rs| rs.get(i))
                        .copied()
                        .unwrap_or_default();
                    let mut n = access::named(Role::Link, r, &row.text);
                    access::focusable(&mut n, row.enabled);
                    tree.push(g, i, n);
                }
            }
            Body::Lines(_) => {
                let lines = lay.lines.get(k).map(Vec::as_slice).unwrap_or_default();
                let text: Vec<&str> = lines.iter().map(|(l, _)| l.as_str()).collect();
                tree.push(g, "lines", access::label(body, &text.join(" "), true));
            }
        }
    }
    pane
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::classic_paint;
    use crate::theme::{Color, FontConfig, Theme};

    const CHEVRON_RIGHT: i32 = 18;
    const CHEVRON_Y_OPEN: i32 = 7;
    const CHEVRON_Y_SHUT: i32 = 8;

    const PANE: Rect = Rect::new(0, 82, 200, 500);
    const HEADER0_Y: i32 = 95;
    const BODY0_Y: i32 = 118;
    const BODY0_BORDER: i32 = 204;
    const HEADER1_Y: i32 = 220;
    const ROW_TOPS: [i32; 3] = [128, 148, 178];
    const SECTION_L: i32 = 12;
    const SECTION_R: i32 = 197;
    const ICON_X: i32 = 25;
    const LINK_INK_X: i32 = 48;
    const DETAILS_H: i32 = 47;

    fn icon(colour: Color) -> Canvas {
        let mut i = Canvas::new(16, 16);
        i.fill_rect(i.bounds(), colour);
        i
    }

    fn pane() -> TaskPane {
        TaskPane {
            sections: vec![
                Section::links(
                    "File and Folder Tasks",
                    vec![
                        Row::new("Make a new folder", Some(icon(Color::rgb(255, 255, 153)))),
                        Row::new(
                            "Publish this folder to the Web",
                            Some(icon(Color::rgb(0, 128, 0))),
                        ),
                        Row::new("Share this folder", Some(icon(Color::rgb(255, 204, 0)))),
                    ],
                ),
                Section::links(
                    "Other Places",
                    vec![
                        Row::new("Desktop", Some(icon(Color::rgb(0, 96, 192)))),
                        Row::new("Public", Some(icon(Color::rgb(255, 255, 153)))),
                        Row::new("My Computer", Some(icon(Color::rgb(128, 160, 192)))),
                        Row::new("My Network Places", Some(icon(Color::rgb(0, 128, 128)))),
                    ],
                ),
                Section::lines(
                    "Details",
                    vec![
                        ("My Documents".into(), true),
                        ("System Folder".into(), false),
                    ],
                ),
            ],
            ..Default::default()
        }
    }

    fn renderer() -> (Theme, TextRenderer) {
        (Theme::default(), TextRenderer::load(&FontConfig::default()))
    }

    #[test]
    fn sections_stack_with_their_gaps() {
        let (theme, text) = renderer();
        let tp = pane();
        let lay = layout(
            &tp,
            &text,
            theme.fonts.size_px,
            PANE,
            &ControlMetrics::CLASSIC,
        );
        assert_eq!(lay.headers[0], Rect::new(SECTION_L, HEADER0_Y, 186, 23));
        assert_eq!(lay.headers[0].right() - 1, SECTION_R);
        assert_eq!(lay.headers[0].y, PANE.y + TaskMetrics::CLASSIC.top);
        assert_eq!(lay.bodies[0].y, BODY0_Y);
        assert_eq!(lay.bodies[0].bottom() - 1, BODY0_BORDER);
        assert_eq!(lay.headers[1].y, HEADER1_Y);
        assert_eq!(lay.bodies[2].height, DETAILS_H);
        assert!(lay.scrollbar.is_none());
    }

    /// At 150% and 200% a title band grows with the text. Classic's chevrons and Blue's fold
    /// button move down half the growth, staying centred; Blue's band keeps its top rows.
    #[test]
    fn a_grown_title_band_keeps_its_chevrons_and_button_centred() {
        let (_, text) = renderer();
        for (pct, px) in [(150, 17), (200, 22)] {
            let u = crate::theme::TextUnits::new(px, 11);
            let theme = Theme::default();
            let cm = ControlMetrics::CLASSIC.grown(u);
            let lay = layout(&pane(), &text, px, PANE, &cm);
            let band = u.len(TaskMetrics::CLASSIC.header);
            assert_eq!(lay.headers[0].height, band, "{pct}%");
            assert_eq!(lay.bodies[0].y, HEADER0_Y + band, "{pct}%");
            let mut c = Canvas::new(200, 600);
            draw(
                &mut c,
                &text,
                &classic_paint(&theme.colors, 1),
                &pane(),
                &lay,
                px,
            );
            let h = lay.headers[0];
            let ink = theme.colors.button_text.argb();
            let x = h.right() - CHEVRON_RIGHT + 3;
            let rows: Vec<i32> = (h.y..h.bottom()).filter(|&y| c.get(x, y) == ink).collect();
            let top = h.y + CHEVRON_Y_OPEN + (band - TaskMetrics::CLASSIC.header) / 2;
            assert_eq!(rows.first(), Some(&top), "{pct}%: the open chevron's tip");

            let blue = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
            let native = blue.look.controls().task.header;
            let grown = blue.look.controls().grown(u);
            let band = grown.task.header;
            assert_eq!(band, u.len(native), "{pct}%");
            let r = Rect::new(0, 0, 186, native);
            let mut one = Canvas::new(186, native as u32);
            draw_band(&mut one, &text, &blue, r, px);
            let mut big = Canvas::new(186, band as u32);
            draw_band(&mut big, &text, &blue, Rect::new(0, 0, 186, band), px);
            let mark = crate::blue::Variant::Blue
                .scheme()
                .task_collapse
                .normal
                .mark
                .unwrap()
                .argb();
            let marks = |c: &Canvas, h: i32| -> Vec<(i32, i32)> {
                (0..h)
                    .flat_map(|y| (186 - 24..186).map(move |x| (x, y)))
                    .filter(|&(x, y)| c.get(x, y) == mark)
                    .collect()
            };
            let down = (band - native) / 2;
            let moved: Vec<(i32, i32)> = marks(&one, native)
                .into_iter()
                .map(|(x, y)| (x, y + down))
                .collect();
            assert!(!moved.is_empty(), "{pct}%: the chevrons show");
            assert_eq!(
                marks(&big, band),
                moved,
                "{pct}%: the chevrons move down half the growth"
            );
            for y in 0..native - 2 {
                assert_eq!(big.get(100, y), one.get(100, y), "{pct}%: the band at {y}");
            }
            let fill = big.get(100, native - 2);
            assert_eq!(
                big.get(100, band - 2),
                fill,
                "{pct}%: the stretched fill row"
            );
            assert_eq!(
                big.get(100, band - 1),
                one.get(100, native - 1),
                "{pct}%: the last row"
            );
        }
    }

    fn draw_band(c: &mut Canvas, text: &TextRenderer, theme: &Theme, r: Rect, px: u32) {
        let title = Rect::new(r.x + 13, r.y + 2, 0, r.height);
        look::draw_header(
            c,
            text,
            &theme.paint(),
            r,
            title,
            "",
            true,
            Interaction::Idle,
            px,
        );
    }

    #[test]
    fn blues_background_stops_a_row_short_of_the_bottom() {
        let (_, text) = renderer();
        let theme = crate::interfaces::blue_theme(crate::blue::Variant::Blue);
        let px = theme.fonts.size_px;
        let r = Rect::new(0, 86, 200, 457);
        let lay = layout(&pane(), &text, px, r, &theme.look.controls());
        assert_eq!(lay.back, Rect::new(1, 86, 199, 456));
        let mut c = Canvas::new(200, 543);
        draw(&mut c, &text, &theme.paint(), &pane(), &lay, px);
        let window = theme.colors.window.argb();
        assert_eq!(c.get(100, 542), window, "the last row");
        assert_ne!(c.get(100, 541), window, "the background above it");
    }

    #[test]
    fn a_long_details_line_wraps_and_grows_its_body() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (theme, text) = renderer();
        let px = theme.fonts.size_px;
        let mut tp = pane();
        tp.sections[2] = Section::lines(
            "Details",
            vec![
                ("My Documents".into(), true),
                ("System Folder".into(), false),
                (String::new(), false),
                (
                    "Date Modified: Yesterday, September 13, 2026, 8:25 PM".into(),
                    false,
                ),
            ],
        );
        let lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        let sect_w = lay.bodies[2].width;
        let wrapped = &lay.lines[2];
        assert_eq!(wrapped.len(), 5);
        assert_eq!(wrapped[0], ("My Documents".to_string(), true));
        assert_eq!(wrapped[2], (String::new(), false), "the gap stays one line");
        assert!(wrapped[3].0.starts_with("Date Modified: "));
        assert!(!wrapped[4].0.is_empty());
        // A bold line wraps against the bold width: laid out regular it would fit on one line and then overrun the box when drawn.
        let long = "a very long file name that must wrap in the pane.txt";
        let bold = lines_of(&text, &[(long.into(), true)], lines_width(sect_w), px);
        assert!(bold.len() > 1, "{bold:?}");
        for (line, _) in &bold {
            assert!(
                text.measure(line, true, px) <= lines_width(sect_w),
                "{line:?} overruns the pane"
            );
        }
        assert_eq!(
            bold.iter()
                .map(|(l, _)| l.as_str())
                .collect::<Vec<_>>()
                .join(" "),
            long,
            "no word is lost"
        );
        assert_eq!(
            lay.bodies[2].height,
            BODY_TOP_PAD + 4 * TaskMetrics::CLASSIC.line + GROUP_GAP + LINES_BOTTOM_PAD + 1
        );
    }

    #[test]
    fn a_wrapped_link_row_is_thirty_tall_and_pushes_the_next_one_down() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (theme, text) = renderer();
        let tp = pane();
        let lay = layout(
            &tp,
            &text,
            theme.fonts.size_px,
            PANE,
            &ControlMetrics::CLASSIC,
        );
        let rows = &lay.rows[0];
        assert_eq!(rows.iter().map(|r| r.y).collect::<Vec<_>>(), ROW_TOPS);
        assert_eq!(rows[0].height, ROW_H);
        assert_eq!(rows[1].height, 30);
        assert_eq!(rows[2].height, ROW_H);
        assert_eq!(rows[0].x, ICON_X);
        let two = &lay.rows[1];
        assert_eq!(two.len(), 4);
        assert!(two.windows(2).all(|w| w[1].y - w[0].y == ROW_H));
        let text_w =
            text_right(lay.bodies[0].x, lay.bodies[0].width) - (rows[1].x + ICON + ICON_GAP);
        assert_eq!(text_w, 137);
        assert_eq!(
            lay.link_lines[0][1],
            vec!["Publish this folder to the", "Web"]
        );
    }

    #[test]
    fn collapsing_a_section_drops_its_body_and_lifts_the_rest() {
        let (theme, text) = renderer();
        let mut tp = pane();
        let px = theme.fonts.size_px;
        let open = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        tp.sections[0].open = false;
        let shut = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        assert_eq!(shut.bodies[0].height, 0);
        assert!(shut.rows[0].is_empty());
        assert_eq!(shut.headers[0], open.headers[0]);
        assert_eq!(shut.headers[1].y, open.headers[0].bottom() + GAP);
        assert_eq!(shut.content_h, open.content_h - open.bodies[0].height);
    }

    #[test]
    fn an_empty_details_line_is_a_five_pixel_gap() {
        if !crate::text::test_tahoma() {
            return;
        }
        let (theme, text) = renderer();
        let px = theme.fonts.size_px;
        let mut tp = pane();
        let gap = (String::new(), false);
        tp.sections[2] = Section::lines(
            "Details",
            vec![
                gap.clone(),
                ("New Rich Text Document".into(), true),
                ("Rich Text Document".into(), false),
                gap.clone(),
                (
                    "Date Modified: Monday, September 14, 2026, 3:43 PM".into(),
                    false,
                ),
                gap,
                ("Size: 4.00 KB".into(), false),
            ],
        );
        let lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        assert_eq!(lay.lines[2].len(), 8, "the date wraps once");
        assert_eq!(lay.bodies[2].height, 101);
        if !crate::text::test_tahoma() {
            return;
        }
        let cs = &theme.colors;
        let mut c = Canvas::new(200, 700);
        draw(&mut c, &text, &classic_paint(cs, 1), &tp, &lay, px);
        let body = lay.bodies[2];
        let white = cs.window.argb();
        let ink = |y: i32| (body.x + 2..body.right() - 2).any(|x| c.get(x, y) != white);
        let size_top = body.y + 77;
        assert!(ink(size_top + 10), "the size's baseline row");
        assert!(!ink(size_top + 11), "nothing hangs below it");
        assert!(!ink(size_top - 1), "the gap above it is white");
    }

    #[test]
    fn a_links_section_without_links_leaves_only_its_gap() {
        let (theme, text) = renderer();
        let px = theme.fonts.size_px;
        let mut tp = pane();
        tp.sections[0] = Section::links("File and Folder Tasks", Vec::new());
        let lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        assert_eq!(lay.headers[1].y, PANE.y + 28);
        assert_eq!(lay.headers[0].height, 0);
        assert_eq!(lay.bodies[0].height, 0);
        assert_eq!(hit(&tp, &lay, 100, HEADER0_Y + 5), Hit::Nothing);

        let cs = &theme.colors;
        let mut c = Canvas::new(200, 600);
        draw(&mut c, &text, &classic_paint(cs, 1), &tp, &lay, px);
        let white = cs.window.argb();
        for y in PANE.y..PANE.y + 28 {
            assert_eq!(c.get(SECTION_L, y), white, "white at y {y}");
        }
        assert_eq!(c.get(SECTION_L, PANE.y + 28), cs.button_face.argb());

        let mut tree = Tree::window("Recycle Bin", (200, 600));
        access(&tp, &lay, &mut tree, crate::access::ROOT);
        let groups: Vec<&str> = tree
            .nodes()
            .filter(|(_, n)| n.role() == Role::Group)
            .filter_map(|(_, n)| n.label())
            .collect();
        assert_eq!(groups, ["Other Places", "Details"]);
    }

    #[test]
    fn clicks_toggle_headers_and_report_enabled_links() {
        let (theme, text) = renderer();
        let mut tp = pane();
        let px = theme.fonts.size_px;
        let lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        let h = hit(&tp, &lay, 100, HEADER0_Y + 5);
        assert_eq!(h, Hit::Header(0));
        assert_eq!(tp.click(h), Event::Toggle(0));
        assert!(!tp.sections[0].open);
        assert_eq!(tp.click(h), Event::Toggle(0));
        assert!(tp.sections[0].open);
        let h = hit(&tp, &lay, LINK_INK_X, ROW_TOPS[1] + 4);
        assert_eq!(h, Hit::Row(0, 1));
        assert_eq!(tp.click(h), Event::Row(0, 1));
        if let Body::Links(rows) = &mut tp.sections[0].body {
            rows[1].enabled = false;
        }
        assert_eq!(tp.click(h), Event::None);
        assert_eq!(hit(&tp, &lay, 100, BODY0_BORDER + 6), Hit::Nothing);
        assert_eq!(hit(&tp, &lay, 100, PANE.y + 2), Hit::Nothing);
    }

    #[test]
    fn an_overflowing_pane_takes_a_scrollbar_and_narrows_its_sections() {
        let (theme, text) = renderer();
        let tp = pane();
        let px = theme.fonts.size_px;
        let short = Rect::new(0, 82, 200, 200);
        let lay = layout(&tp, &text, px, short, &ControlMetrics::CLASSIC);
        let sb = lay.scrollbar.expect("the sections do not fit");
        assert_eq!(
            sb,
            Rect::new(
                short.right() - ControlMetrics::CLASSIC.scroll,
                short.y,
                ControlMetrics::CLASSIC.scroll,
                200
            )
        );
        assert_eq!(lay.headers[0].width, 186 - ControlMetrics::CLASSIC.scroll);
        assert!(lay.content_h > short.height);
        assert_eq!(hit(&tp, &lay, sb.x + 2, sb.y + 2), Hit::ScrollUp);
        assert_eq!(hit(&tp, &lay, sb.x + 2, sb.bottom() - 2), Hit::ScrollDown);
        assert_eq!(hit(&tp, &lay, sb.x + 2, sb.y + 60), Hit::Scrollbar);
        let mut tp2 = tp.clone();
        assert_eq!(tp2.click(Hit::ScrollDown), Event::Redraw);
        assert_eq!(tp2.top, ROW_H);
        assert_eq!(tp2.click(Hit::ScrollUp), Event::Redraw);
        assert_eq!(tp2.top, 0);
        assert_eq!(tp2.click(Hit::ScrollUp), Event::Redraw);
        assert_eq!(tp2.top, 0, "never above the first section");
        tp2.top = 40;
        let scrolled = layout(&tp2, &text, px, short, &ControlMetrics::CLASSIC);
        assert_eq!(scrolled.headers[0].y, lay.headers[0].y - 40);
        assert_eq!(scrolled.content_h, lay.content_h);
    }

    #[test]
    fn draw_paints_borders_chevrons_and_link_ink() {
        let (theme, text) = renderer();
        let mut tp = pane();
        let cs = &theme.colors;
        let px = theme.fonts.size_px;
        let lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(200, 582);
        draw(&mut c, &text, &classic_paint(cs, 1), &tp, &lay, px);
        let face = cs.button_face.argb();
        let white = cs.window.argb();
        let ink = cs.button_text.argb();
        assert_eq!(c.get(5, 200), white);
        assert_eq!(c.get(100, HEADER0_Y), face);
        assert_eq!(c.get(100, HEADER0_Y + 22), face);
        assert_eq!(c.get(SECTION_L, 120), face);
        assert_eq!(c.get(SECTION_L + 1, 120), white);
        assert_eq!(c.get(SECTION_R, 120), face);
        assert_eq!(c.get(100, BODY0_BORDER), face);
        assert_eq!(c.get(100, BODY0_BORDER + 1), white);
        let cx = lay.headers[0].right() - CHEVRON_RIGHT;
        let cy = HEADER0_Y + CHEVRON_Y_OPEN;
        assert_eq!(c.get(cx + 3, cy), ink);
        assert_eq!(c.get(cx, cy + 3), ink);
        assert_eq!(c.get(cx + 6, cy + 3), ink, "the arrow's right foot");
        assert_eq!(c.get(cx + 3, cy + 4), ink, "the second arrow");
        assert_eq!(c.get(cx + 3, cy - 1), face);
        assert!(
            (HEADER0_Y..HEADER0_Y + 23).any(|y| c.get(SECTION_L + TEXT_INSET, y) == ink
                || c.get(SECTION_L + TEXT_INSET + 1, y) == ink)
        );
        assert_eq!(c.get(ICON_X, ROW_TOPS[0]), Color::rgb(255, 255, 153).argb());
        assert!(
            (ROW_TOPS[0]..ROW_TOPS[0] + 16).any(|y| c.get(LINK_INK_X, y) == cs.window_text.argb())
        );
        let under = lay.rows[0][0].y + lay.line_h - 1;
        assert_eq!(c.get(LINK_INK_X, under), white);
        tp.hot = Some((0, 0));
        draw(&mut c, &text, &classic_paint(cs, 1), &tp, &lay, px);
        assert_eq!(c.get(LINK_INK_X, under), cs.window_text.argb());
        tp.hot = None;
        tp.sections[0].open = false;
        let shut = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(200, 582);
        draw(&mut c, &text, &classic_paint(cs, 1), &tp, &shut, px);
        let cy = HEADER0_Y + CHEVRON_Y_SHUT;
        assert_eq!(c.get(cx + 3, cy + 3), ink, "the apex is at the bottom now");
        assert_eq!(c.get(cx, cy), ink);
        assert_eq!(c.get(100, BODY0_Y), white, "no body under it");
    }

    #[test]
    fn a_layout_missing_a_sections_rows_draws_its_header_without_links() {
        let (theme, text) = renderer();
        let tp = pane();
        let px = theme.fonts.size_px;
        let mut lay = layout(&tp, &text, px, PANE, &ControlMetrics::CLASSIC);
        let desktop = lay.rows[1][0];
        lay.rows.truncate(1);
        let mut c = Canvas::new(200, 582);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &tp,
            &lay,
            px,
        );
        assert_eq!(c.get(100, HEADER1_Y), theme.colors.button_face.argb());
        assert_eq!(c.get(desktop.x, desktop.y), theme.colors.window.argb());
    }

    fn blue_pane(
        v: crate::blue::Variant,
        lit: Option<usize>,
        hot: Option<(usize, usize)>,
        details_open: bool,
    ) -> Canvas {
        let theme = crate::interfaces::blue_theme(v);
        let text = TextRenderer::load(&theme.fonts);
        let px = theme.fonts.size_px;
        let links = |rows: &[&str]| {
            rows.iter()
                .map(|r| Row::new(*r, Some(Canvas::new(16, 16))))
                .collect()
        };
        let mut details_section = Section::lines(
            "Details",
            vec![("p3".into(), true), ("File Folder".into(), false)],
        );
        details_section.open = details_open;
        let tp = TaskPane {
            sections: vec![
                Section::links(
                    "File and Folder Tasks",
                    links(&[
                        "Make a new folder",
                        "Publish this folder to the Web",
                        "Share this folder",
                    ]),
                ),
                Section::links(
                    "Other Places",
                    links(&[
                        "Local Disk (C:)",
                        "My Documents",
                        "Shared Documents",
                        "My Computer",
                        "My Network Places",
                    ]),
                ),
                details_section,
            ],
            hot,
            hot_header: lit,
            ..Default::default()
        };
        let r = Rect::new(0, 0, 200, 456);
        let lay = layout(&tp, &text, px, r, &theme.look.controls());
        let mut c = Canvas::new(210, 456);
        let p = theme.paint();
        draw(&mut c, &text, &p, &tp, &lay, px);
        draw_gap(&mut c, &p, &lay, Rect::new(200, 0, 10, 456));
        c
    }

    #[test]
    #[ignore]
    fn render_taskpane_blue() {
        for v in crate::blue::Variant::ALL {
            crate::raster::write_preview(
                &blue_pane(v, None, None, false),
                &format!("taskpane-blue-{}-preview.png", v.as_str()),
            );
            crate::raster::write_preview(
                &blue_pane(v, Some(0), Some((0, 0)), false),
                &format!("taskpane-blue-{}-lit-preview.png", v.as_str()),
            );
            crate::raster::write_preview(
                &blue_pane(v, None, None, true),
                &format!("taskpane-blue-{}-details-preview.png", v.as_str()),
            );
        }
    }

    #[test]
    #[ignore]
    fn render_taskpane() {
        let (theme, text) = renderer();
        let mut tp = pane();
        tp.sections.push(Section::links(
            "Collapsed Tasks",
            vec![Row::new("Nothing to see", None)],
        ));
        tp.sections[3].open = false;
        let r = Rect::new(0, 0, 200, 420);
        let lay = layout(&tp, &text, theme.fonts.size_px, r, &ControlMetrics::CLASSIC);
        let mut c = Canvas::new(200, 420);
        draw(
            &mut c,
            &text,
            &classic_paint(&theme.colors, 1),
            &tp,
            &lay,
            theme.fonts.size_px,
        );
        crate::raster::write_preview(&c, "taskpane-preview.png");
    }
}
