//! Region select for a screenshot or a screen recording: the screen dims, a drag draws the
//! rectangle to keep (undimmed, with a 1 px white line just outside it) and a tip beside the
//! pointer reads its size, "W x H". A click that never moves `DRAG_PX` picks the point instead (the
//! host takes the window under it). A tip at the top of the pointer's screen names the mode, which
//! V switches.

use crate::access::{self, Tree, accesskit};
use crate::controls::tooltip::{draw_tip, tip_size};
use crate::raster::{Canvas, Rect};
use crate::text::TextRenderer;
use crate::theme::Theme;
use accesskit::{NodeId, Role};

/// How far the pointer moves (either axis, logical px) before a press becomes a drag.
pub const DRAG_PX: i32 = 4;

/// The dim over everything outside the selection: black at 40 %, premultiplied ARGB.
pub const DIM: u32 = 0x6600_0000;
pub const BORDER: u32 = 0xffff_ffff;

/// The readout's top-left sits this far right of and below the pointer, or as far left of and above
/// it where it would leave the screen.
pub const READOUT_OFFSET: i32 = 16;

/// The mode tip's top sits this far below the top of the pointer's screen.
pub const MODE_TOP: i32 = 24;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegionSelect {
    anchor: Option<(i32, i32)>,
    pointer: (i32, i32),
    dragging: bool,
    /// The pick starts a recording rather than taking a screenshot.
    record: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    None,
    Redraw,
    Cancel,
    Region(Rect),
    Point(i32, i32),
}

impl RegionSelect {
    pub fn new(pointer: (i32, i32), record: bool) -> RegionSelect {
        RegionSelect {
            pointer,
            record,
            ..RegionSelect::default()
        }
    }

    pub fn records(&self) -> bool {
        self.record
    }

    /// V: switch between a screenshot and a recording.
    pub fn toggle_record(&mut self) -> Event {
        self.record = !self.record;
        Event::Redraw
    }

    pub fn press(&mut self, p: (i32, i32)) -> Event {
        self.anchor = Some(p);
        self.pointer = p;
        self.dragging = false;
        Event::Redraw
    }

    pub fn motion(&mut self, p: (i32, i32)) -> Event {
        self.pointer = p;
        if let Some((ax, ay)) = self.anchor
            && ((p.0 - ax).abs() >= DRAG_PX || (p.1 - ay).abs() >= DRAG_PX)
        {
            self.dragging = true;
        }
        Event::Redraw
    }

    /// The button came up: the rectangle, the clicked point, or Cancel for a drag that ended where
    /// it began.
    pub fn release(&mut self, p: (i32, i32)) -> Event {
        self.motion(p);
        let Some(anchor) = self.anchor else {
            return Event::None;
        };
        if !self.dragging {
            return Event::Point(anchor.0, anchor.1);
        }
        match self.selection() {
            Some(r) if !r.is_empty() => Event::Region(r),
            _ => Event::Cancel,
        }
    }

    pub fn escape(&mut self) -> Event {
        Event::Cancel
    }

    /// The rectangle between the anchor and the pointer while dragging; the pointer's corner is
    /// exclusive, so a drag from (100, 100) to (300, 200) keeps 200 x 100.
    pub fn selection(&self) -> Option<Rect> {
        let (ax, ay) = self.anchor.filter(|_| self.dragging)?;
        let (px, py) = self.pointer;
        Some(Rect::new(
            ax.min(px),
            ay.min(py),
            (px - ax).abs(),
            (py - ay).abs(),
        ))
    }

    pub fn pointer(&self) -> (i32, i32) {
        self.pointer
    }
}

/// Where everything goes, in the coordinates of `area`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    /// Filled with `DIM`: `area` less the selection.
    pub dim: Vec<Rect>,
    /// Filled with `BORDER`, over the dim.
    pub border: Vec<Rect>,
    /// The size tip, while dragging.
    pub readout: Option<Rect>,
    /// The mode tip, top centre of the pointer's screen.
    pub mode: Rect,
}

/// The overlay over `area` (every output), with the readout kept inside `screen` (the pointer's
/// output).
pub fn layout(
    m: &RegionSelect,
    text: &TextRenderer,
    theme: &Theme,
    area: Rect,
    screen: Rect,
) -> Layout {
    let mode = place_mode(tip_size(text, theme, mode_text(m)), screen);
    let Some(sel) = m.selection().map(|s| s.intersect(&area)) else {
        return Layout {
            dim: vec![area],
            border: Vec::new(),
            readout: None,
            mode,
        };
    };
    let dim = [
        Rect::new(area.x, area.y, area.width, sel.y - area.y),
        Rect::new(
            area.x,
            sel.bottom(),
            area.width,
            area.bottom() - sel.bottom(),
        ),
        Rect::new(area.x, sel.y, sel.x - area.x, sel.height),
        Rect::new(sel.right(), sel.y, area.right() - sel.right(), sel.height),
    ];
    let border = [
        Rect::new(sel.x - 1, sel.y - 1, sel.width + 2, 1),
        Rect::new(sel.x - 1, sel.bottom(), sel.width + 2, 1),
        Rect::new(sel.x - 1, sel.y, 1, sel.height),
        Rect::new(sel.right(), sel.y, 1, sel.height),
    ];
    let keep = |rs: [Rect; 4]| -> Vec<Rect> {
        rs.iter()
            .map(|r| r.intersect(&area))
            .filter(|r| !r.is_empty())
            .collect()
    };
    let (w, h) = tip_size(text, theme, &label(sel));
    Layout {
        dim: keep(dim),
        border: keep(border),
        readout: Some(place_readout(m.pointer, (w, h), screen)),
        mode,
    }
}

/// What the mode tip says: what a pick does now, and that V switches.
pub fn mode_text(m: &RegionSelect) -> &'static str {
    match m.record {
        false => "Screenshot: drag an area or click a window. Press V to record instead.",
        true => "Record: drag an area or click a window. Press V for a screenshot instead.",
    }
}

/// `MODE_TOP` below the top of `screen`, centred, kept inside it.
fn place_mode((w, h): (i32, i32), screen: Rect) -> Rect {
    let x = screen.x + (screen.width - w) / 2;
    Rect::new(x.max(screen.x), screen.y + MODE_TOP, w, h)
}

fn label(sel: Rect) -> String {
    format!("{} x {}", sel.width, sel.height)
}

/// `READOUT_OFFSET` right of and below `p`, flipped to the other side on an axis where it would run
/// past `screen`, then kept inside it.
fn place_readout(p: (i32, i32), (w, h): (i32, i32), screen: Rect) -> Rect {
    let along = |p: i32, len: i32, lo: i32, hi: i32| {
        let after = p + READOUT_OFFSET;
        let at = if after + len > hi {
            p - READOUT_OFFSET - len
        } else {
            after
        };
        at.min(hi - len).max(lo)
    };
    Rect::new(
        along(p.0, w, screen.x, screen.right()),
        along(p.1, h, screen.y, screen.bottom()),
        w,
        h,
    )
}

/// The size tip, while dragging.
pub fn draw_readout(theme: &Theme, text: &TextRenderer, m: &RegionSelect) -> Option<Canvas> {
    m.selection().map(|sel| draw_tip(theme, text, &label(sel)))
}

pub fn draw_mode(theme: &Theme, text: &TextRenderer, m: &RegionSelect) -> Canvas {
    draw_tip(theme, text, mode_text(m))
}

/// The mode tip and, while dragging, the size tip, as labels under `parent`.
pub fn access(m: &RegionSelect, lay: &Layout, tree: &mut Tree, parent: NodeId) {
    tree.push(
        parent,
        "mode",
        access::titled(Role::Label, lay.mode, mode_text(m)),
    );
    if let (Some(r), Some(sel)) = (lay.readout, m.selection()) {
        tree.push(parent, "size", access::titled(Role::Label, r, &label(sel)));
    }
}

pub fn draw(
    theme: &Theme,
    text: &TextRenderer,
    m: &RegionSelect,
    area: Rect,
    under: &Canvas,
) -> Canvas {
    let mut c = under.clone();
    let lay = layout(m, text, theme, area, area);
    let at = |r: Rect| Rect::new(r.x - area.x, r.y - area.y, r.width, r.height);
    for r in &lay.dim {
        dim_rect(&mut c, at(*r));
    }
    for r in &lay.border {
        let r = at(*r).intersect(&c.bounds());
        for y in r.y..r.bottom() {
            for x in r.x..r.right() {
                c.put_raw(x, y, BORDER);
            }
        }
    }
    if let (Some(r), Some(tip)) = (lay.readout, draw_readout(theme, text, m)) {
        let r = at(r);
        c.blit_over(r.x, r.y, &tip);
    }
    let r = at(lay.mode);
    c.blit_over(r.x, r.y, &draw_mode(theme, text, m));
    c
}

fn dim_rect(c: &mut Canvas, r: Rect) {
    let r = r.intersect(&c.bounds());
    let keep = 255 - (DIM >> 24);
    for y in r.y..r.bottom() {
        for x in r.x..r.right() {
            let p = c.get(x, y);
            let ch = |shift: u32| (((p >> shift) & 0xff) * keep / 255) << shift;
            c.put_raw(x, y, (DIM & 0xff00_0000) + ch(24) + ch(16) + ch(8) + ch(0));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    fn dragged(from: (i32, i32), to: (i32, i32)) -> RegionSelect {
        let mut m = RegionSelect::new(from, false);
        m.press(from);
        m.motion(to);
        m
    }

    #[test]
    fn a_drag_in_any_direction_keeps_the_same_rectangle() {
        let want = Rect::new(100, 100, 200, 100);
        for (from, to) in [
            ((100, 100), (300, 200)),
            ((300, 200), (100, 100)),
            ((300, 100), (100, 200)),
            ((100, 200), (300, 100)),
        ] {
            let mut m = dragged(from, to);
            assert_eq!(m.release(to), Event::Region(want), "{from:?} to {to:?}");
        }
    }

    #[test]
    fn a_press_that_stays_within_drag_px_is_a_click_on_its_point() {
        let mut m = RegionSelect::new((0, 0), false);
        m.press((50, 60));
        m.motion((53, 57));
        assert_eq!(m.selection(), None);
        assert_eq!(m.release((53, 57)), Event::Point(50, 60));
    }

    #[test]
    fn a_drag_that_comes_back_to_its_start_cancels() {
        let mut m = dragged((50, 50), (80, 80));
        assert_eq!(m.release((50, 50)), Event::Cancel);
    }

    #[test]
    fn escape_cancels_and_a_release_without_a_press_does_nothing() {
        let mut m = RegionSelect::new((0, 0), false);
        assert_eq!(m.release((10, 10)), Event::None);
        assert_eq!(m.escape(), Event::Cancel);
    }

    #[test]
    fn before_a_drag_the_whole_area_is_dimmed_and_nothing_else_is_shown() {
        let text = TextRenderer::load(&FontConfig::default());
        let area = Rect::new(0, 0, 800, 600);
        let lay = layout(
            &RegionSelect::new((5, 5), false),
            &text,
            &Theme::default(),
            area,
            area,
        );
        assert_eq!(lay.dim, vec![area]);
        assert!(lay.border.is_empty());
        assert_eq!(lay.readout, None);
    }

    #[test]
    fn the_dim_goes_around_the_selection_with_a_border_just_outside_it() {
        let text = TextRenderer::load(&FontConfig::default());
        let area = Rect::new(0, 0, 800, 600);
        let m = dragged((100, 100), (300, 200));
        let lay = layout(&m, &text, &Theme::default(), area, area);
        assert_eq!(
            lay.dim,
            vec![
                Rect::new(0, 0, 800, 100),
                Rect::new(0, 200, 800, 400),
                Rect::new(0, 100, 100, 100),
                Rect::new(300, 100, 500, 100),
            ]
        );
        assert_eq!(
            lay.border,
            vec![
                Rect::new(99, 99, 202, 1),
                Rect::new(99, 200, 202, 1),
                Rect::new(99, 100, 1, 100),
                Rect::new(300, 100, 1, 100),
            ]
        );
    }

    #[test]
    fn the_readout_sits_below_right_of_the_pointer_and_flips_at_the_screen_edge() {
        let screen = Rect::new(0, 0, 800, 600);
        assert_eq!(
            place_readout((300, 200), (60, 18), screen),
            Rect::new(316, 216, 60, 18)
        );
        assert_eq!(
            place_readout((790, 590), (60, 18), screen),
            Rect::new(714, 556, 60, 18)
        );
        assert_eq!(
            place_readout((20, 590), (60, 18), Rect::new(0, 0, 70, 600)),
            Rect::new(0, 556, 60, 18)
        );
    }

    #[test]
    fn draw_dims_outside_keeps_inside_and_rings_the_selection_in_white() {
        let text = TextRenderer::load(&FontConfig::default());
        let theme = Theme::default();
        let area = Rect::new(0, 0, 400, 300);
        let under = Canvas::from_pixels(400, 300, vec![0xff64_c8fa; 400 * 300]);
        let m = dragged((100, 100), (200, 150));
        let c = draw(&theme, &text, &m, area, &under);
        assert_eq!(c.get(10, 10), 0xff3c_7896, "dimmed outside");
        assert_eq!(c.get(150, 120), 0xff64_c8fa, "untouched inside");
        assert_eq!(c.get(150, 99), BORDER, "border above");
        assert_eq!(c.get(200, 120), BORDER, "border right");
        assert_eq!(c.get(99, 150), BORDER, "border bottom-left corner");
        let lay = layout(&m, &text, &theme, area, area);
        let tip = lay.readout.unwrap();
        assert_eq!((tip.x, tip.y), (216, 166));
        assert_eq!(c.get(tip.x, tip.y) & 0xff_ffff, 0);
    }

    #[test]
    fn v_switches_the_pick_between_a_screenshot_and_a_recording() {
        let mut m = RegionSelect::new((0, 0), false);
        assert!(!m.records());
        assert_eq!(m.toggle_record(), Event::Redraw);
        assert!(m.records());
        assert!(mode_text(&m).starts_with("Record:"));
        m.toggle_record();
        assert!(mode_text(&m).starts_with("Screenshot:"));
    }

    #[test]
    fn the_mode_tip_sits_top_centre_of_the_pointer_screen() {
        let screen = Rect::new(1920, 0, 1280, 1024);
        assert_eq!(
            place_mode((400, 20), screen),
            Rect::new(2360, MODE_TOP, 400, 20)
        );
        assert_eq!(
            place_mode((1400, 20), screen),
            Rect::new(1920, MODE_TOP, 1400, 20),
            "a tip wider than the screen starts at its left edge"
        );
    }

    #[test]
    fn a_reader_hears_the_mode_and_the_size() {
        let text = TextRenderer::load(&FontConfig::default());
        let area = Rect::new(0, 0, 800, 600);
        let m = dragged((100, 100), (300, 200));
        let lay = layout(&m, &text, &Theme::default(), area, area);
        let mut tree = Tree::window("Screenshot", (800, 600));
        access(&m, &lay, &mut tree, access::ROOT);
        assert!(tree.find(Role::Label, mode_text(&m)).is_some());
        assert!(tree.find(Role::Label, "200 x 100").is_some());
    }

    #[test]
    fn the_mode_tip_pins_its_pixels_in_both_modes_under_every_interface() {
        use crate::interface::Interface;
        crate::pins::fonts();
        let mut got = Vec::new();
        for i in [Interface::Classic, Interface::Blue, Interface::Glass] {
            let base = crate::interfaces::theme_for(i);
            let text = TextRenderer::load(&base.fonts);
            for record in [false, true] {
                for s in [1, 2] {
                    let m = RegionSelect::new((0, 0), record);
                    let c = draw_mode(&base.scaled(s), text.for_output(s), &m);
                    got.push((format!("{i:?} record={record} {s}"), fnv(&c)));
                }
            }
        }
        crate::pins::check("region_mode_tip", &got);
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
}
