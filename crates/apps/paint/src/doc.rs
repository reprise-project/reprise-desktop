//! The picture and what each tool does to it under the pointer, in picture pixels. A stroke,
//! shape or selection move is one undo step, from [`Doc::begin`] to [`Doc::commit`].

use crate::bitmap::Bitmap;
use crate::history::History;
use crate::shapes::{self, BoxShape, Brush, Pts, Spray, Tip};
use reprisekit::raster::Rect;

pub type P = (i32, i32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    FreeSelect,
    Select,
    Eraser,
    Fill,
    Picker,
    Magnifier,
    Pencil,
    Brush,
    Airbrush,
    Text,
    Line,
    Curve,
    Rect,
    Polygon,
    Ellipse,
    RoundRect,
}

/// The tool box's order, two to a row.
pub const TOOLS: [Tool; 16] = [
    Tool::FreeSelect,
    Tool::Select,
    Tool::Eraser,
    Tool::Fill,
    Tool::Picker,
    Tool::Magnifier,
    Tool::Pencil,
    Tool::Brush,
    Tool::Airbrush,
    Tool::Text,
    Tool::Line,
    Tool::Curve,
    Tool::Rect,
    Tool::Polygon,
    Tool::Ellipse,
    Tool::RoundRect,
];

impl Tool {
    pub fn selects(self) -> bool {
        matches!(self, Tool::Select | Tool::FreeSelect)
    }
}

/// Which colour a press draws with: the left button the primary, the right the secondary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Primary,
    Secondary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FillStyle {
    Outline,
    Both,
    Fill,
}

/// The tool options the options well sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// Line, curve and shape outline width.
    pub width: i32,
    pub brush: Brush,
    pub eraser: i32,
    pub spray: i32,
    pub fill: FillStyle,
    /// A selection or text box covers what is under it in the secondary colour too.
    pub opaque: bool,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            width: 1,
            brush: Brush {
                tip: Tip::Round,
                size: 4,
            },
            eraser: 8,
            spray: 8,
            fill: FillStyle::Outline,
            opaque: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
}

/// A selection: `rect` where it sits now; `mask`, for a free-form one, which of its pixels are
/// in; `float`, once moved, the pixels it carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub rect: Rect,
    pub mask: Option<Vec<bool>>,
    pub float: Option<Bitmap>,
}

impl Selection {
    fn holds(&self, at: P) -> bool {
        if !self.rect.contains(at.0, at.1) {
            return false;
        }
        self.mask.as_ref().is_none_or(|m| {
            m[((at.1 - self.rect.y) * self.rect.width + at.0 - self.rect.x) as usize]
        })
    }

    fn masked(&self, i: usize) -> bool {
        self.mask.as_ref().is_some_and(|m| !m[i])
    }
}

/// What a press, move or release asks of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    Redraw,
    Picked(Ink, u32),
    Magnify(P),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Drag {
    Stroke { last: P, ink: Ink },
    Spray { at: P, ink: Ink },
    Shape { from: P, ink: Ink },
    Marquee { from: P, to: P },
    Lasso { pts: Pts },
    Move { grab: P },
    TextBox { from: P, to: P },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Curve {
    a: P,
    b: P,
    c1: Option<P>,
    ink: Ink,
}

#[derive(Debug)]
pub struct Doc {
    pub img: Bitmap,
    pub hist: History,
    pub primary: u32,
    pub secondary: u32,
    pub opts: Options,
    pub sel: Option<Selection>,
    /// The text box being typed into, in picture coordinates.
    pub text_box: Option<Rect>,
    /// Some edit was made since the picture was opened or saved.
    pub dirty: bool,
    base: Option<Bitmap>,
    drag: Option<Drag>,
    poly: Vec<P>,
    poly_ink: Ink,
    curve: Option<Curve>,
    spray: Spray,
}

/// Pixels this close to the polygon's first corner close it.
const CLOSE_PX: i32 = 3;

impl Doc {
    pub fn new(img: Bitmap) -> Doc {
        Doc {
            img,
            hist: History::default(),
            primary: crate::bitmap::BLACK,
            secondary: crate::bitmap::WHITE,
            opts: Options::default(),
            sel: None,
            text_box: None,
            dirty: false,
            base: None,
            drag: None,
            poly: Vec::new(),
            poly_ink: Ink::Primary,
            curve: None,
            spray: Spray(0x2545_F491),
        }
    }

    fn color(&self, ink: Ink) -> u32 {
        match ink {
            Ink::Primary => self.primary,
            Ink::Secondary => self.secondary,
        }
    }

    fn other(&self, ink: Ink) -> u32 {
        match ink {
            Ink::Primary => self.secondary,
            Ink::Secondary => self.primary,
        }
    }

    /// Keeps the picture as it is now for the step that starts.
    pub fn begin(&mut self) {
        if self.base.is_none() {
            self.base = Some(self.img.clone());
        }
    }

    /// Ends the running step, recording it when it changed anything.
    pub fn commit(&mut self) {
        if let Some(b) = self.base.take()
            && self.hist.record(&b, &self.img)
        {
            self.dirty = true;
        }
    }

    fn restore(&mut self) {
        if let Some(b) = &self.base {
            self.img.clone_from(b);
        }
    }

    /// The whole picture replaced by `img` as one step, the selection dropped first.
    pub fn replace(&mut self, img: Bitmap) {
        self.finish();
        self.begin();
        self.img = img;
        self.commit();
    }

    /// A pointer press at `at` with `tool`; `double` when it pairs with the press before it.
    pub fn press(&mut self, tool: Tool, at: P, ink: Ink, m: Mods, double: bool) -> Effect {
        if !tool.selects() {
            self.drop_selection();
        }
        if tool != Tool::Polygon && !self.poly.is_empty() {
            self.close_polygon();
        }
        if tool != Tool::Curve && self.curve.is_some() {
            self.curve = None;
            self.commit();
        }
        match tool {
            Tool::Select | Tool::FreeSelect => {
                if self.sel.as_ref().is_some_and(|s| s.holds(at)) {
                    self.lift(m.ctrl);
                    let r = self.sel.as_ref().map_or(Rect::default(), |s| s.rect);
                    self.drag = Some(Drag::Move {
                        grab: (at.0 - r.x, at.1 - r.y),
                    });
                } else {
                    self.drop_selection();
                    self.drag = Some(if tool == Tool::Select {
                        Drag::Marquee { from: at, to: at }
                    } else {
                        Drag::Lasso { pts: vec![at] }
                    });
                }
                Effect::Redraw
            }
            Tool::Text => {
                self.drag = Some(Drag::TextBox { from: at, to: at });
                Effect::Redraw
            }
            Tool::Picker => match self.img.get(at.0, at.1) {
                Some(c) => Effect::Picked(ink, c),
                None => Effect::None,
            },
            Tool::Magnifier => Effect::Magnify(at),
            Tool::Fill => {
                self.begin();
                let c = self.color(ink);
                self.img.flood(at.0, at.1, c);
                self.commit();
                Effect::Redraw
            }
            Tool::Pencil | Tool::Brush | Tool::Eraser => {
                self.begin();
                self.stroke(tool, &[at], ink);
                self.drag = Some(Drag::Stroke { last: at, ink });
                Effect::Redraw
            }
            Tool::Airbrush => {
                self.begin();
                self.drag = Some(Drag::Spray { at, ink });
                self.spray_once();
                Effect::Redraw
            }
            Tool::Line | Tool::Rect | Tool::Ellipse | Tool::RoundRect => {
                self.begin();
                self.drag = Some(Drag::Shape { from: at, ink });
                self.shape(tool, at, at, ink, m);
                Effect::Redraw
            }
            Tool::Curve => {
                match self.curve {
                    None => {
                        self.begin();
                        self.drag = Some(Drag::Shape { from: at, ink });
                        self.shape(tool, at, at, ink, m);
                    }
                    Some(c) => {
                        self.drag = Some(Drag::Shape {
                            from: at,
                            ink: c.ink,
                        });
                        self.bend(at);
                    }
                }
                Effect::Redraw
            }
            Tool::Polygon => {
                if self.poly.is_empty() {
                    self.begin();
                    self.poly = vec![at, at];
                    self.poly_ink = ink;
                } else {
                    let first = self.poly[0];
                    let near =
                        (at.0 - first.0).abs() <= CLOSE_PX && (at.1 - first.1).abs() <= CLOSE_PX;
                    if double || near {
                        self.close_polygon();
                        return Effect::Redraw;
                    }
                    self.poly.push(at);
                }
                self.drag = Some(Drag::Shape {
                    from: at,
                    ink: self.poly_ink,
                });
                self.draw_polyline();
                Effect::Redraw
            }
        }
    }

    pub fn motion(&mut self, tool: Tool, at: P, m: Mods) -> Effect {
        let Some(drag) = self.drag.clone() else {
            return Effect::None;
        };
        match drag {
            Drag::Stroke { last, ink } => {
                let path = shapes::line(last, at);
                self.stroke(tool, &path, ink);
                self.drag = Some(Drag::Stroke { last: at, ink });
            }
            Drag::Spray { ink, .. } => {
                self.drag = Some(Drag::Spray { at, ink });
                self.spray_once();
            }
            Drag::Shape { from, ink } => match tool {
                Tool::Curve if self.curve.is_some() => self.bend(at),
                Tool::Polygon => {
                    let to = if m.shift && self.poly.len() >= 2 {
                        shapes::snapped(self.poly[self.poly.len() - 2], at)
                    } else {
                        at
                    };
                    if let Some(last) = self.poly.last_mut() {
                        *last = to;
                    }
                    self.draw_polyline();
                }
                _ => self.shape(tool, from, at, ink, m),
            },
            Drag::Marquee { from, .. } => self.drag = Some(Drag::Marquee { from, to: at }),
            Drag::TextBox { from, .. } => self.drag = Some(Drag::TextBox { from, to: at }),
            Drag::Lasso { mut pts } => {
                if pts.last() != Some(&at) {
                    pts.push(at);
                }
                self.drag = Some(Drag::Lasso { pts });
            }
            Drag::Move { grab } => {
                if let Some(s) = &mut self.sel {
                    s.rect.x = at.0 - grab.0;
                    s.rect.y = at.1 - grab.1;
                }
            }
        }
        Effect::Redraw
    }

    pub fn release(&mut self, tool: Tool, at: P, m: Mods) -> Effect {
        let fx = self.motion(tool, at, m);
        let Some(drag) = self.drag.take() else {
            return fx;
        };
        match drag {
            Drag::Stroke { .. } | Drag::Spray { .. } => self.commit(),
            Drag::Shape { from, ink } => match tool {
                Tool::Curve => match &mut self.curve {
                    None => {
                        self.curve = Some(Curve {
                            a: from,
                            b: at,
                            c1: None,
                            ink,
                        })
                    }
                    Some(c) if c.c1.is_none() => c.c1 = Some(at),
                    Some(_) => {
                        self.curve = None;
                        self.commit();
                    }
                },
                Tool::Polygon => {}
                _ => self.commit(),
            },
            Drag::Marquee { from, to } => {
                let r = shapes::span(from, to).intersect(&self.img.bounds());
                if from != to && !r.is_empty() {
                    self.sel = Some(Selection {
                        rect: r,
                        mask: None,
                        float: None,
                    });
                }
            }
            Drag::Lasso { pts } => self.lasso(&pts),
            Drag::TextBox { from, to } => {
                let r = shapes::span(from, to).intersect(&self.img.bounds());
                if !r.is_empty() {
                    self.text_box = Some(r);
                }
            }
            Drag::Move { .. } => {}
        }
        Effect::Redraw
    }

    /// A step of the airbrush held still.
    pub fn tick(&mut self) -> Effect {
        if matches!(self.drag, Some(Drag::Spray { .. })) {
            self.spray_once();
            return Effect::Redraw;
        }
        Effect::None
    }

    pub fn spraying(&self) -> bool {
        matches!(self.drag, Some(Drag::Spray { .. }))
    }

    /// The marquee or text box being dragged out, for the window to outline.
    pub fn dragged_box(&self) -> Option<Rect> {
        match &self.drag {
            Some(Drag::Marquee { from, to } | Drag::TextBox { from, to }) => {
                Some(shapes::span(*from, *to))
            }
            _ => None,
        }
    }

    /// The free-form outline being drawn.
    pub fn lasso_path(&self) -> Option<&[P]> {
        match &self.drag {
            Some(Drag::Lasso { pts }) => Some(pts),
            _ => None,
        }
    }

    /// A curve or polygon is waiting for more points.
    pub fn pending(&self) -> bool {
        self.curve.is_some() || !self.poly.is_empty()
    }

    /// Ends whatever is in progress: closes a polygon, keeps a curve as drawn, drops the
    /// selection.
    pub fn finish(&mut self) {
        if !self.poly.is_empty() {
            self.close_polygon();
        }
        if self.curve.take().is_some() {
            self.commit();
        }
        self.drag = None;
        self.drop_selection();
    }

    /// Escape: a shape or curve in progress is taken back, a selection is dropped where it is.
    pub fn cancel(&mut self) {
        if self.drag.is_some() || self.pending() {
            self.drag = None;
            self.poly.clear();
            self.curve = None;
            if self.sel.as_ref().is_none_or(|s| s.float.is_none()) {
                self.restore();
                self.base = None;
            }
        }
        self.drop_selection();
        self.text_box = None;
    }

    fn stroke(&mut self, tool: Tool, path: &[P], ink: Ink) {
        match tool {
            Tool::Pencil => {
                let c = self.color(ink);
                self.img.plot(path, c);
            }
            Tool::Brush => {
                let c = self.color(ink);
                self.img.plot(&shapes::stamp(path, self.opts.brush), c);
            }
            _ => {
                let pts = shapes::stamp(path, Brush::square(self.opts.eraser));
                let (from, to) = (self.primary, self.secondary);
                for (x, y) in pts {
                    match ink {
                        Ink::Primary => self.img.set(x, y, to),
                        Ink::Secondary if self.img.get(x, y) == Some(from) => {
                            self.img.set(x, y, to)
                        }
                        Ink::Secondary => {}
                    }
                }
            }
        }
    }

    fn spray_once(&mut self) {
        if let Some(Drag::Spray { at, ink }) = self.drag {
            let r = self.opts.spray;
            let pts = self.spray.burst(at, r, (r as usize * 3 / 2).max(4));
            let c = self.color(ink);
            self.img.plot(&pts, c);
        }
    }

    fn shape(&mut self, tool: Tool, from: P, to: P, ink: Ink, m: Mods) {
        self.restore();
        let (line, fill) = (self.color(ink), self.other(ink));
        let w = self.opts.width;
        let to = match tool {
            Tool::Line | Tool::Curve if m.shift => shapes::snapped(from, to),
            Tool::Line | Tool::Curve => to,
            _ if m.shift => shapes::squared(from, to),
            _ => to,
        };
        let kind = match tool {
            Tool::Ellipse => BoxShape::Ellipse,
            Tool::RoundRect => BoxShape::RoundRect,
            Tool::Rect => BoxShape::Rect,
            _ => {
                self.img.plot(
                    &shapes::stamp(&shapes::line(from, to), Brush::square(w)),
                    line,
                );
                return;
            }
        };
        let r = shapes::span(from, to);
        match self.opts.fill {
            FillStyle::Outline => self.img.plot(&shapes::outline(kind, r, w), line),
            FillStyle::Both => {
                self.img.plot(&shapes::interior(kind, r, w), fill);
                self.img.plot(&shapes::outline(kind, r, w), line);
            }
            FillStyle::Fill => self.img.plot(&shapes::interior(kind, r, 0), line),
        }
    }

    fn bend(&mut self, at: P) {
        let Some(c) = self.curve else {
            return;
        };
        self.restore();
        let (c1, c2) = match c.c1 {
            None => (at, at),
            Some(c1) => (c1, at),
        };
        let pts = shapes::stamp(
            &shapes::curve(c.a, c1, c2, c.b),
            Brush::square(self.opts.width),
        );
        let ink = self.color(c.ink);
        self.img.plot(&pts, ink);
    }

    fn draw_polyline(&mut self) {
        self.restore();
        let pts = shapes::stamp(
            &shapes::polyline(&self.poly),
            Brush::square(self.opts.width),
        );
        let c = self.color(self.poly_ink);
        self.img.plot(&pts, c);
    }

    fn close_polygon(&mut self) {
        let mut corners = std::mem::take(&mut self.poly);
        corners.dedup();
        self.restore();
        let (line, fill) = (self.color(self.poly_ink), self.other(self.poly_ink));
        let w = self.opts.width;
        let inner = shapes::polygon_fill(&corners);
        let ring = shapes::polygon_outline(&corners, w);
        match self.opts.fill {
            FillStyle::Outline => self.img.plot(&ring, line),
            FillStyle::Both => {
                self.img.plot(&inner, fill);
                self.img.plot(&ring, line);
            }
            FillStyle::Fill => {
                self.img.plot(&inner, line);
            }
        }
        self.drag = None;
        self.commit();
    }

    fn lasso(&mut self, pts: &[P]) {
        let inside = shapes::polygon_fill(pts);
        let b = self.img.bounds();
        let inside: Pts = inside
            .into_iter()
            .filter(|p| b.contains(p.0, p.1))
            .collect();
        let Some(x0) = inside.iter().map(|p| p.0).min() else {
            return;
        };
        let x1 = inside.iter().map(|p| p.0).max().unwrap_or(x0);
        let y0 = inside.iter().map(|p| p.1).min().unwrap_or(0);
        let y1 = inside.iter().map(|p| p.1).max().unwrap_or(y0);
        let r = Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        let mut mask = vec![false; (r.width * r.height) as usize];
        for (x, y) in inside {
            mask[((y - r.y) * r.width + x - r.x) as usize] = true;
        }
        self.sel = Some(Selection {
            rect: r,
            mask: Some(mask),
            float: None,
        });
    }

    /// Picks the selection's pixels up to move them; `keep` leaves a copy where they were (a
    /// Ctrl drag), else the place they leave takes the secondary colour.
    fn lift(&mut self, keep: bool) {
        let secondary = self.secondary;
        let opaque = self.opts.opaque;
        let Some(s) = self.sel.as_mut() else {
            return;
        };
        if let Some(f) = &s.float {
            if keep {
                stamp_float(&mut self.img, s, f, secondary, opaque);
            }
            return;
        }
        if self.base.is_none() {
            self.base = Some(self.img.clone());
        }
        let float = self.img.copy(s.rect, secondary);
        if !keep {
            for y in 0..s.rect.height {
                for x in 0..s.rect.width {
                    if !s.masked((y * s.rect.width + x) as usize) {
                        self.img.set(s.rect.x + x, s.rect.y + y, secondary);
                    }
                }
            }
        }
        s.float = Some(float);
    }

    /// Lays a moved selection down where it is and ends its step.
    pub fn drop_selection(&mut self) {
        if let Some(s) = self.sel.take()
            && let Some(f) = &s.float
        {
            stamp_float(&mut self.img, &s, f, self.secondary, self.opts.opaque);
            self.commit();
        }
    }

    /// The selection's pixels; masked-out ones are the secondary colour.
    pub fn selected(&self) -> Option<Bitmap> {
        let s = self.sel.as_ref()?;
        let mut out = match &s.float {
            Some(f) => f.clone(),
            None => self.img.copy(s.rect, self.secondary),
        };
        for (i, p) in out.px.iter_mut().enumerate() {
            if s.masked(i) {
                *p = self.secondary;
            }
        }
        Some(out)
    }

    /// Clears the selection to the secondary colour, as one step.
    pub fn delete_selection(&mut self) {
        let Some(s) = self.sel.take() else {
            return;
        };
        self.begin();
        if s.float.is_none() {
            for y in 0..s.rect.height {
                for x in 0..s.rect.width {
                    if !s.masked((y * s.rect.width + x) as usize) {
                        self.img.set(s.rect.x + x, s.rect.y + y, self.secondary);
                    }
                }
            }
        }
        self.commit();
    }

    /// `pic` laid on the picture as a selection at `at`, ready to move; the picture grows to
    /// hold it.
    pub fn paste(&mut self, pic: Bitmap, at: P) {
        self.finish();
        self.begin();
        let (w, h) = (self.img.w.max(at.0 + pic.w), self.img.h.max(at.1 + pic.h));
        if (w, h) != (self.img.w, self.img.h) {
            self.img = self.img.with_size(w, h, self.secondary);
        }
        self.sel = Some(Selection {
            rect: Rect::new(at.0, at.1, pic.w, pic.h),
            mask: None,
            float: Some(pic),
        });
    }

    pub fn select_all(&mut self) {
        self.finish();
        self.sel = Some(Selection {
            rect: self.img.bounds(),
            mask: None,
            float: None,
        });
    }

    /// The picture cut down to the selection, as one step.
    pub fn crop(&mut self) {
        let Some(pic) = self.selected() else {
            return;
        };
        self.replace(pic);
    }

    /// Takes back the last step; a step still in progress (a pasted or moved selection, an open
    /// curve or polygon) is taken back instead.
    pub fn undo(&mut self) -> bool {
        if self.base.is_some() {
            self.drag = None;
            self.poly.clear();
            self.curve = None;
            self.sel = None;
            self.restore();
            self.base = None;
            return true;
        }
        self.finish();
        let done = self.hist.undo(&mut self.img);
        self.dirty |= done;
        done
    }

    pub fn redo(&mut self) -> bool {
        self.finish();
        let done = self.hist.redo(&mut self.img);
        self.dirty |= done;
        done
    }

    /// The picture as shown over `r`: with a moved selection laid on it.
    pub fn composite(&self, r: Rect) -> Bitmap {
        let mut out = self.img.copy(r, 0xFF80_8080);
        if let Some(s) = &self.sel
            && let Some(f) = &s.float
        {
            let moved = Selection {
                rect: Rect::new(s.rect.x - r.x, s.rect.y - r.y, s.rect.width, s.rect.height),
                mask: s.mask.clone(),
                float: None,
            };
            stamp_float(&mut out, &moved, f, self.secondary, self.opts.opaque);
        }
        out
    }

    /// Lays text coverage `cover` (one byte per pixel of `r`) on the picture in the primary colour,
    /// over the secondary colour when opaque, as one step.
    pub fn ink_text(&mut self, r: Rect, cover: &[u8]) {
        self.begin();
        if self.opts.opaque {
            self.img.fill_rect(r, self.secondary);
        }
        blend_cover(&mut self.img, r, cover, self.primary);
        self.commit();
        self.text_box = None;
    }
}

/// `cover` (one byte per pixel of `r`) mixed onto `img` in `ink`.
pub fn blend_cover(img: &mut Bitmap, r: Rect, cover: &[u8], ink: u32) {
    for y in 0..r.height {
        for x in 0..r.width {
            let a = cover[(y * r.width + x) as usize] as u32;
            if a == 0 {
                continue;
            }
            let Some(d) = img.get(r.x + x, r.y + y) else {
                continue;
            };
            let ch = |k: u32| (((ink >> k) & 0xFF) * a + ((d >> k) & 0xFF) * (255 - a) + 127) / 255;
            img.set(
                r.x + x,
                r.y + y,
                0xFF00_0000 | ch(16) << 16 | ch(8) << 8 | ch(0),
            );
        }
    }
}

/// Lays `f` at `s.rect`; a transparent one leaves out its secondary-coloured pixels.
fn stamp_float(img: &mut Bitmap, s: &Selection, f: &Bitmap, secondary: u32, opaque: bool) {
    img.draw(s.rect.x, s.rect.y, f, |i, c| {
        s.masked(i) || (!opaque && c == secondary)
    });
}

#[cfg(test)]
mod tests;
