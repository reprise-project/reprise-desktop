//! The main window: menu bar, tool box, the picture in its scrolled view, the colour box and the
//! status bar; where each sits, how they are drawn, and the access tree.

use super::*;
use crate::art;
use crate::doc::{FillStyle, TOOLS};
use crate::shapes::{Brush, Tip};
use reprisekit::access::ROOT;
use reprisekit::classic::widgets::{BarHit, ScrollParts, bar_hit};
use reprisekit::controls::colordlg;
use reprisekit::controls::menubar;
use reprisekit::controls::palette::{self, Palette};
use reprisekit::controls::scrollbar;
use reprisekit::controls::statusbar::{self, Panel, StatusBar};
use reprisekit::controls::toolbox::{self, Choice, ToolBox};
use reprisekit::look::widgets::{
    draw_hscrollbar, draw_marquee, draw_scroll_corner, draw_scrollbar,
};
use reprisekit::theme::Color;
use std::cell::RefCell;

pub const INITIAL_SIZE: (i32, i32) = (640, 480);
pub const MIN_SIZE: (i32, i32) = (240, 320);

/// The gap between the view's corner and the picture's.
pub const MARGIN: i32 = 4;

pub const ZOOMS: [i32; 6] = [1, 2, 3, 4, 6, 8];

/// The zooms the magnifier offers.
pub const MAGNIFY: [i32; 5] = [1, 2, 4, 6, 8];

/// Grid lines between pixels from this zoom up.
pub const GRID_FROM: i32 = 4;

const GRID_INK: u32 = 0xFF80_8080;

pub const WIDTHS: [i32; 5] = [1, 2, 3, 4, 5];
pub const ERASERS: [i32; 4] = [4, 6, 8, 10];
pub const SPRAYS: [i32; 3] = [4, 8, 12];
pub const TIPS: [(Tip, i32); 12] = [
    (Tip::Round, 7),
    (Tip::Round, 4),
    (Tip::Round, 1),
    (Tip::Square, 8),
    (Tip::Square, 5),
    (Tip::Square, 2),
    (Tip::Slash, 8),
    (Tip::Slash, 5),
    (Tip::Slash, 2),
    (Tip::Backslash, 8),
    (Tip::Backslash, 5),
    (Tip::Backslash, 2),
];
pub const FILLS: [FillStyle; 3] = [FillStyle::Outline, FillStyle::Both, FillStyle::Fill];

/// Where everything in the main window sits, logical px.
#[derive(Clone, Copy, Debug)]
pub struct Lay {
    pub tools: Rect,
    /// The part of `area` the picture shows in.
    pub view: Rect,
    pub vbar: Option<Rect>,
    pub hbar: Option<Rect>,
    pub colors: Rect,
    pub status: Option<Rect>,
    /// The picture's top-left corner on the window.
    pub origin: (i32, i32),
}

/// The tool and option glyphs drawn at one scale, kept until the scale changes.
#[derive(Default)]
pub struct Glyphs(RefCell<Option<(i32, Vec<Canvas>)>>);

impl Glyphs {
    fn tools(&self, s: i32) -> Vec<Canvas> {
        let mut slot = self.0.borrow_mut();
        if slot.as_ref().is_none_or(|(at, _)| *at != s) {
            *slot = Some((s, TOOLS.iter().map(|&t| art::tool_glyph(t, s)).collect()));
        }
        slot.as_ref().map(|(_, g)| g.clone()).unwrap_or_default()
    }
}

pub fn tool_name(t: Tool) -> String {
    match t {
        Tool::FreeSelect => tr!("tool-free-select"),
        Tool::Select => tr!("tool-select"),
        Tool::Eraser => tr!("tool-eraser"),
        Tool::Fill => tr!("tool-fill"),
        Tool::Picker => tr!("tool-picker"),
        Tool::Magnifier => tr!("tool-magnifier"),
        Tool::Pencil => tr!("tool-pencil"),
        Tool::Brush => tr!("tool-brush"),
        Tool::Airbrush => tr!("tool-airbrush"),
        Tool::Text => tr!("tool-text"),
        Tool::Line => tr!("tool-line"),
        Tool::Curve => tr!("tool-curve"),
        Tool::Rect => tr!("tool-rect"),
        Tool::Polygon => tr!("tool-polygon"),
        Tool::Ellipse => tr!("tool-ellipse"),
        Tool::RoundRect => tr!("tool-round-rect"),
    }
}

fn tool_hint(t: Tool) -> String {
    match t {
        Tool::FreeSelect => tr!("hint-free-select"),
        Tool::Select => tr!("hint-select"),
        Tool::Eraser => tr!("hint-eraser"),
        Tool::Fill => tr!("hint-fill"),
        Tool::Picker => tr!("hint-picker"),
        Tool::Magnifier => tr!("hint-magnifier"),
        Tool::Pencil => tr!("hint-pencil"),
        Tool::Brush => tr!("hint-brush"),
        Tool::Airbrush => tr!("hint-airbrush"),
        Tool::Text => tr!("hint-text"),
        Tool::Line => tr!("hint-line"),
        Tool::Curve => tr!("hint-curve"),
        Tool::Rect => tr!("hint-rect"),
        Tool::Polygon => tr!("hint-polygon"),
        Tool::Ellipse => tr!("hint-ellipse"),
        Tool::RoundRect => tr!("hint-round-rect"),
    }
}

/// The options well's rows for `tool`: what the window draws, and which one is picked.
pub enum Options {
    None,
    Opaque,
    Eraser,
    Zoom,
    Tips,
    Spray,
    Widths,
    Fills,
}

pub fn options_of(tool: Tool) -> Options {
    match tool {
        Tool::Select | Tool::FreeSelect | Tool::Text => Options::Opaque,
        Tool::Eraser => Options::Eraser,
        Tool::Magnifier => Options::Zoom,
        Tool::Brush => Options::Tips,
        Tool::Airbrush => Options::Spray,
        Tool::Line | Tool::Curve => Options::Widths,
        Tool::Rect | Tool::Polygon | Tool::Ellipse | Tool::RoundRect => Options::Fills,
        Tool::Fill | Tool::Picker | Tool::Pencil => Options::None,
    }
}

impl Paint {
    pub(crate) fn bar_layout(&self, ctx: &Ctx) -> menubar::Layout {
        self.bar
            .layout(
                &ctx.text,
                ctx.theme.fonts.size_px,
                Rect::new(0, 0, ctx.size.0, ctx.size.1),
                &ctx.theme.controls(),
            )
            .reaching(ctx.menu_within())
    }

    pub(crate) fn lay(&self, ctx: &Ctx) -> Lay {
        let cm = ctx.theme.controls();
        let (w, h) = ctx.size;
        let top = menubar::bar_h(&cm);
        let status_h = if self.status_bar {
            statusbar::height(&cm)
        } else {
            0
        };
        let colors = Rect::new(0, h - status_h - palette::HEIGHT, w, palette::HEIGHT);
        let tools = Rect::new(0, top, toolbox::WIDTH, colors.y - top);
        let area = Rect::new(tools.right(), top, w - tools.right(), colors.y - top);
        let (pw, ph) = self.extent();
        let bar = cm.scroll;
        let mut vbar = ph > area.height;
        let hbar = pw > area.width - if vbar { bar } else { 0 };
        vbar |= hbar && ph > area.height - bar;
        let view = Rect::new(
            area.x,
            area.y,
            area.width - if vbar { bar } else { 0 },
            area.height - if hbar { bar } else { 0 },
        );
        let left = scrollbar::held(self.hscroll.top, view.width, pw);
        let up = scrollbar::held(self.vscroll.top, view.height, ph);
        Lay {
            tools,
            view,
            vbar: vbar.then(|| Rect::new(view.right(), view.y, bar, view.height)),
            hbar: hbar.then(|| Rect::new(view.x, view.bottom(), view.width, bar)),
            colors,
            status: (status_h > 0).then(|| Rect::new(0, h - status_h, w, status_h)),
            origin: (view.x + MARGIN - left, view.y + MARGIN - up),
        }
    }

    /// The scrolled extent: the zoomed picture with its margin on each side.
    pub(crate) fn extent(&self) -> (i32, i32) {
        (
            self.doc.img.w * self.zoom + 2 * MARGIN,
            self.doc.img.h * self.zoom + 2 * MARGIN,
        )
    }

    /// The picture pixel under window point `(x, y)`, off the picture too.
    pub(crate) fn to_picture(&self, l: &Lay, x: i32, y: i32) -> (i32, i32) {
        (
            (x - l.origin.0).div_euclid(self.zoom),
            (y - l.origin.1).div_euclid(self.zoom),
        )
    }

    /// Picture rect `r` on the window.
    pub(crate) fn on_window(&self, l: &Lay, r: Rect) -> Rect {
        let z = self.zoom;
        Rect::new(
            l.origin.0 + r.x * z,
            l.origin.1 + r.y * z,
            r.width * z,
            r.height * z,
        )
    }

    pub(crate) fn hbar_parts(&self, bar: Rect, l: &Lay, ctx: &Ctx) -> ScrollParts {
        let cm = ctx.theme.controls();
        self.hscroll
            .parts(bar, l.view.width, self.extent().0, cm.thumb_min)
    }

    pub(crate) fn vbar_parts(&self, bar: Rect, l: &Lay, ctx: &Ctx) -> ScrollParts {
        let cm = ctx.theme.controls();
        self.vscroll
            .parts(bar, l.view.height, self.extent().1, cm.thumb_min)
    }

    /// The scrollbar part under `(x, y)`; `true` for the horizontal bar.
    pub(crate) fn bar_at(&self, l: &Lay, x: i32, y: i32, ctx: &Ctx) -> Option<(bool, BarHit)> {
        if let Some(b) = l.hbar.filter(|b| b.contains(x, y)) {
            return Some((true, bar_hit(&self.hbar_parts(b, l, ctx), x, y, true)));
        }
        if let Some(b) = l.vbar.filter(|b| b.contains(x, y)) {
            return Some((false, bar_hit(&self.vbar_parts(b, l, ctx), x, y, false)));
        }
        None
    }

    /// Holds both scroll offsets to their range after a zoom, resize or picture change.
    pub(crate) fn hold_scroll(&mut self, ctx: &Ctx) {
        let l = self.lay(ctx);
        let (pw, ph) = self.extent();
        self.hscroll.top = scrollbar::held(self.hscroll.top, l.view.width, pw);
        self.vscroll.top = scrollbar::held(self.vscroll.top, l.view.height, ph);
    }

    pub(crate) fn toolbox(&self, s: i32, ctx: &Ctx) -> ToolBox {
        let tools = self.glyphs.tools(s);
        let tools = TOOLS
            .iter()
            .zip(tools)
            .map(|(&t, g)| toolbox::Tool {
                name: tool_name(t),
                glyph: Some(g),
            })
            .collect();
        let (options, option, option_cols) = self.choices(s, ctx);
        ToolBox {
            tools,
            current: TOOLS.iter().position(|&t| t == self.tool).unwrap_or(0),
            hot: self.tool_hot,
            pressed: self.tool_pressed,
            options,
            option_cols,
            option,
        }
    }

    /// The options well's choices for the current tool, the picked one, and its columns.
    fn choices(&self, s: i32, ctx: &Ctx) -> (Vec<Choice>, Option<usize>, usize) {
        let o = &self.doc.opts;
        let ch = |name: String, glyph: Canvas| Choice {
            name,
            glyph: Some(glyph),
        };
        match options_of(self.tool) {
            Options::None => (Vec::new(), None, 1),
            Options::Opaque => (
                vec![
                    ch(tr!("option-opaque"), art::opaque_glyph(true, s)),
                    ch(tr!("option-transparent"), art::opaque_glyph(false, s)),
                ],
                Some(if o.opaque { 0 } else { 1 }),
                1,
            ),
            Options::Eraser => (
                ERASERS
                    .iter()
                    .map(|&n| ch(tr!("option-eraser", n = n), art::eraser_glyph(n, s)))
                    .collect(),
                ERASERS.iter().position(|&n| n == o.eraser),
                1,
            ),
            Options::Zoom => (
                MAGNIFY
                    .iter()
                    .map(|&n| {
                        let name = tr!("option-zoom", n = n);
                        let g = zoom_glyph(&name, s, ctx);
                        ch(name, g)
                    })
                    .collect(),
                MAGNIFY.iter().position(|&n| n == self.magnify),
                1,
            ),
            Options::Tips => (
                TIPS.iter()
                    .map(|&(tip, n)| {
                        let name = match tip {
                            Tip::Round => tr!("option-tip-round", n = n),
                            Tip::Square => tr!("option-tip-square", n = n),
                            Tip::Slash => tr!("option-tip-slash", n = n),
                            Tip::Backslash => tr!("option-tip-backslash", n = n),
                        };
                        ch(name, art::tip_glyph(tip, n, s))
                    })
                    .collect(),
                TIPS.iter()
                    .position(|&(tip, size)| Brush { tip, size } == o.brush),
                3,
            ),
            Options::Spray => (
                SPRAYS
                    .iter()
                    .map(|&n| ch(tr!("option-spray", n = n), art::spray_glyph(n, s)))
                    .collect(),
                SPRAYS.iter().position(|&n| n == o.spray),
                1,
            ),
            Options::Widths => (
                WIDTHS
                    .iter()
                    .map(|&n| ch(tr!("option-width", n = n), art::width_glyph(n, s)))
                    .collect(),
                WIDTHS.iter().position(|&n| n == o.width),
                1,
            ),
            Options::Fills => (
                FILLS
                    .iter()
                    .map(|&f| {
                        let name = match f {
                            FillStyle::Outline => tr!("option-outline"),
                            FillStyle::Both => tr!("option-both"),
                            FillStyle::Fill => tr!("option-fill"),
                        };
                        ch(name, art::fill_glyph(f, s))
                    })
                    .collect(),
                FILLS.iter().position(|&f| f == o.fill),
                1,
            ),
        }
    }

    pub(crate) fn palette(&self) -> Palette {
        Palette {
            swatches: self
                .wells
                .iter()
                .zip(art::PALETTE)
                .map(|(&c, (first, name))| {
                    let name = match c == first {
                        true => name.to_string(),
                        false => colordlg::hex(c),
                    };
                    (c, name)
                })
                .collect(),
            rows: 2,
            primary: art::color_of(self.doc.primary),
            secondary: art::color_of(self.doc.secondary),
        }
    }

    pub(crate) fn status(&self, l: &Lay, ctx: &Ctx) -> Option<(StatusBar, Rect)> {
        let r = l.status?;
        let pos = self
            .hover
            .map(|(x, y)| tr!("status-position", x = x, y = y))
            .unwrap_or_default();
        let size = self
            .doc
            .sel
            .as_ref()
            .map(|s| s.rect)
            .or(self.doc.dragged_box())
            .or(self.doc.text_box)
            .map(|r| tr!("status-size", w = r.width, h = r.height))
            .unwrap_or_default();
        let panel = |text: String, width: i32| Panel {
            text,
            width,
            pad: statusbar::TEXT_PAD,
            icon: None,
        };
        let bar = StatusBar {
            panels: vec![
                panel(tool_hint(self.tool), 0),
                panel(pos, 96),
                panel(size, 96),
            ],
            grip: !ctx.maximized,
        };
        Some((bar, r))
    }

    pub(crate) fn draw_main(&self, c: &mut Canvas, ctx: &Ctx) {
        let cs = &ctx.theme.colors;
        let l = self.lay(ctx);
        let paint = Look {
            s: 1,
            ..ctx.theme.paint()
        };
        c.fill_rect(c.bounds(), cs.button_face);
        c.fill_rect(l.view, cs.button_shadow);
        c.with_clip(l.view, |c| self.draw_picture(c, &l, ctx));
        let s = c.scale();
        let dev = Look {
            s,
            ..ctx.theme.paint()
        };
        let tb = self.toolbox(s, ctx);
        let tlay = toolbox::layout(&tb, l.tools);
        c.in_device_px(|d| toolbox::draw(d, &dev, &tb, &tlay));
        let pal = self.palette();
        let play = palette::layout(&pal, l.colors);
        c.in_device_px(|d| palette::draw(d, &dev, &pal, &play));
        let (pw, ph) = self.extent();
        if let Some(b) = l.vbar {
            let (top, vis) = (self.vscroll.top as usize, l.view.height as usize);
            draw_scrollbar(c, &paint, b, top, vis, ph as usize, self.vscroll.lit());
        }
        if let Some(b) = l.hbar {
            let (left, vis) = (self.hscroll.top as usize, l.view.width as usize);
            draw_hscrollbar(c, &paint, b, left, vis, pw as usize, self.hscroll.lit());
        }
        if let (Some(v), Some(h)) = (l.vbar, l.hbar) {
            draw_scroll_corner(c, &paint, Rect::new(v.x, h.y, v.width, h.height), false);
        }
        if let Some((bar, r)) = self.status(&l, ctx) {
            statusbar::draw(c, &ctx.text, &paint, &bar, r, ctx.theme.fonts.size_px);
        }
        let lay = self.bar_layout(ctx);
        self.bar.draw(c, &ctx.text, &ctx.theme, &lay);
    }

    /// The picture over `r` as shown: a moved selection laid on it, the text box's text and
    /// caret.
    pub(crate) fn shown(&self, r: Rect, caret_on: bool) -> Bitmap {
        let mut out = self.doc.composite(r);
        if let Some(b) = self.doc.text_box {
            let b = self.text.fit(b);
            let moved = Rect::new(b.x - r.x, b.y - r.y, b.width, b.height);
            if self.doc.opts.opaque {
                out.fill_rect(moved, self.doc.secondary);
            }
            let cover = self.text.cover(b.width, b.height);
            doc::blend_cover(&mut out, moved, &cover, self.doc.primary);
            if caret_on {
                let k = self.text.caret();
                let k = Rect::new(moved.x + k.x.min(b.width - 1), moved.y + k.y, 1, k.height);
                out.fill_rect(k.intersect(&moved), self.doc.primary);
            }
        }
        out
    }

    fn draw_picture(&self, c: &mut Canvas, l: &Lay, ctx: &Ctx) {
        let z = self.zoom;
        let pic = self.on_window(l, self.doc.img.bounds());
        let vis = pic.intersect(&l.view);
        if vis.is_empty() {
            return;
        }
        let (x0, y0) = self.to_picture(l, vis.x, vis.y);
        let (x1, y1) = self.to_picture(l, vis.right() - 1, vis.bottom() - 1);
        let r = Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1);
        let shown = self.shown(r, ctx.caret_on);
        let s = c.scale();
        let grid = self.grid && z >= GRID_FROM;
        c.in_device_px(|d| {
            let dz = z * s;
            let (ox, oy) = (l.origin.0 * s, l.origin.1 * s);
            let dv = vis.scaled(s);
            let dw = d.width as i32;
            for dy in dv.y..dv.bottom() {
                let iy = (dy - oy).div_euclid(dz) - r.y;
                let on_row = grid && (dy - oy).rem_euclid(dz) == 0;
                for dx in dv.x..dv.right() {
                    let ix = (dx - ox).div_euclid(dz) - r.x;
                    let px = if on_row || (grid && (dx - ox).rem_euclid(dz) == 0) {
                        GRID_INK
                    } else {
                        shown.px[(iy * shown.w + ix) as usize]
                    };
                    if let Some(p) = d.pixels.get_mut((dy * dw + dx) as usize) {
                        *p = px;
                    }
                }
            }
        });
        let ink = Color::rgb(0, 0, 0);
        let paint = Look {
            s,
            ..ctx.theme.paint()
        };
        let mut rings: Vec<Rect> = Vec::new();
        rings.extend(self.doc.sel.as_ref().map(|s| s.rect));
        rings.extend(self.doc.dragged_box());
        rings.extend(self.doc.text_box.map(|b| self.text.fit(b)));
        for r in rings {
            let ring = self.on_window(l, r).inset(-1);
            c.in_device_px(|d| draw_marquee(d, &paint, ring.scaled(s), ink));
        }
        if let Some(path) = self.doc.lasso_path() {
            for &(x, y) in &crate::shapes::polyline(path) {
                c.fill_rect(self.on_window(l, Rect::new(x, y, 1, 1)), ink);
            }
        }
    }

    pub(crate) fn access_main(&self, tree: &mut Tree, ctx: &Ctx) {
        let l = self.lay(ctx);
        let menus = self.bar.access(&self.bar_layout(ctx), tree, ROOT);
        let tb = self.toolbox(1, ctx);
        let tools = toolbox::access(&tb, &toolbox::layout(&tb, l.tools), tree, ROOT);
        let pal = self.palette();
        palette::access(&pal, &palette::layout(&pal, l.colors), tree, ROOT);
        if let Some((bar, r)) = self.status(&l, ctx) {
            statusbar::access(&bar, r, ctx.theme.text, tree, ROOT);
        }
        tree.set_focus(menus.unwrap_or(tools));
    }
}

/// A zoom option's label as a mask in a 32x12 cell.
fn zoom_glyph(label: &str, s: i32, ctx: &Ctx) -> Canvas {
    let mut c = Canvas::with_scale(32, 12, s);
    let px = ctx.theme.fonts.size_px;
    let text = ctx.text.for_output(s);
    c.in_device_px(|d| {
        let r = Rect::new(0, 0, 32 * s, 12 * s);
        text.draw(
            d,
            r,
            label,
            false,
            Color::rgb(255, 255, 255),
            reprisekit::text::Align::Center,
            px * s as u32,
        );
    });
    c
}
