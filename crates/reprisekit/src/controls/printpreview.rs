//! Print Preview: the pages a program prints, one at a time on a grey ground, the whole page or
//! at its printed size, under a row of buttons. The host draws each page with the renderer its
//! print path uses, off its UI thread, and hands it over ([`Preview::show`]) at the size
//! [`Preview::wanted`] asks for.

use crate::access::{self, Tree, accesskit};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{self, Form, Item};
use crate::controls::imageview::{self, ImageView, Source, Zoom as ViewZoom};
use crate::controls::pagesetup;
use crate::look::Paint;
use crate::metrics::ControlMetrics;
use crate::raster::{Canvas, Rect};
use crate::text::{DialogFont, TextRenderer};
use crate::textedit::keysym;
use crate::theme::{Color, TextUnits};
use accesskit::{NodeId, Role};

pub const PRINT: &str = "print";
pub const NEXT: &str = "next";
pub const PREVIOUS: &str = "previous";
pub const ZOOM_IN: &str = "zoomin";
pub const ZOOM_OUT: &str = "zoomout";
pub const CLOSE: &str = "close";
const PAGE: &str = "page";
const BUTTONS: [&str; 6] = [PRINT, NEXT, PREVIOUS, ZOOM_IN, ZOOM_OUT, CLOSE];

/// The window's size when it opens, logical px at 100% text.
pub const CLIENT: (i32, i32) = (620, 500);
const PAD: i32 = 6;
const GAP: i32 = 4;
/// The button row's height at 100% text.
const BAR_H: i32 = BUTTON_H + 2 * PAD;
const PAGE_W: i32 = 120;
/// Room around the whole page, logical px.
const INSET: i32 = 10;
/// How far the sheet's shadow falls right and down, logical px.
const SHADOW: i32 = 3;
/// A page's dots per inch at 100% zoom and scale 1: a logical px is 1/96 inch.
const SCREEN_DPI: i32 = 96;
const SHEET: Color = Color::rgb(255, 255, 255);

/// How large the page shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Zoom {
    /// The whole page in the window.
    Page,
    /// At its printed size.
    Actual,
}

/// What a key or button asks of the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The preview changed; redraw it and ask for [`Preview::wanted`].
    Redraw,
    Print,
    Close,
    None,
}

pub struct Preview {
    /// The page shown, from 0.
    pub page: u32,
    /// The document's pages; 0 while the host lays it out.
    pub pages: u32,
    pub zoom: Zoom,
    /// The page as it lies, hundredths of a millimetre.
    pub paper: (i32, i32),
    pub form: Form,
    view: ImageView,
    /// The drawing of a page the host handed over: which page, at which size.
    sheet: Option<(u32, Canvas)>,
    /// The last page and size [`Preview::wanted`] asked for.
    asked: Option<(u32, (u32, u32))>,
}

impl Preview {
    /// A preview of pages `paper` (hundredths of a millimetre) as they lie, waiting for the
    /// host to lay the document out.
    pub fn new(paper: (i32, i32)) -> Preview {
        let mut pv = Preview {
            page: 0,
            pages: 0,
            zoom: Zoom::Page,
            paper,
            form: toolbar(),
            view: ImageView::new((0, 0)),
            sheet: None,
            asked: None,
        };
        pv.view.zoom = ViewZoom::Scale(1.0);
        pv.sync();
        pv
    }

    /// The document is laid out in `pages` pages.
    pub fn laid(&mut self, pages: u32) {
        self.pages = pages.max(1);
        self.page = self.page.min(self.pages - 1);
        self.sync();
    }

    /// Show `canvas`, the drawing of page `page` the host made for [`Preview::wanted`]; a
    /// drawing of a page no longer shown is dropped.
    pub fn show(&mut self, page: u32, canvas: Canvas) {
        if page != self.page {
            return;
        }
        self.view.size = (canvas.width, canvas.height);
        self.sheet = Some((page, canvas));
    }

    /// The page and its size in device px the host should draw next, once each time they
    /// change: the page shown, as large as the zoom makes it in `lay`.
    pub fn wanted(&mut self, lay: &Layout) -> Option<(u32, (u32, u32))> {
        if self.pages == 0 {
            return None;
        }
        let want = (self.page, lay.sheet);
        let have = self.sheet.as_ref().map(|(p, c)| (*p, (c.width, c.height)));
        if have == Some(want) || self.asked == Some(want) {
            return None;
        }
        self.asked = Some(want);
        Some(want)
    }

    /// Go `by` pages on, within the document.
    pub fn go(&mut self, by: i32) -> bool {
        let to = (self.page as i64 + by as i64).clamp(0, self.pages.max(1) as i64 - 1) as u32;
        if to == self.page {
            return false;
        }
        self.page = to;
        self.view.left = 0;
        self.view.top = 0;
        self.sync();
        true
    }

    pub fn zoom_to(&mut self, zoom: Zoom) -> bool {
        if zoom == self.zoom {
            return false;
        }
        self.zoom = zoom;
        self.view.left = 0;
        self.view.top = 0;
        self.sync();
        true
    }

    /// What the button `id` does.
    pub fn pressed(&mut self, id: &str) -> Action {
        let changed = match id {
            PRINT if self.pages > 0 => return Action::Print,
            CLOSE => return Action::Close,
            NEXT => self.go(1),
            PREVIOUS => self.go(-1),
            ZOOM_IN => self.zoom_to(Zoom::Actual),
            ZOOM_OUT => self.zoom_to(Zoom::Page),
            _ => false,
        };
        redraw(changed)
    }

    /// The preview's own keys, before the buttons get them: Page Down and Page Up turn the
    /// page, Escape closes, and the arrows, Home and End pan a page larger than the window.
    pub fn key(&mut self, sym: u32, lay: &Layout) -> Option<Action> {
        let pan = |pv: &mut Preview, dx: i32, dy: i32| redraw(pv.view.pan(dx, dy, &lay.view));
        let line = imageview::LINE;
        let page_h = lay.view.device_view().height;
        Some(match sym {
            keysym::PAGE_DOWN | keysym::KP_PAGE_DOWN => redraw(self.go(1)),
            keysym::PAGE_UP | keysym::KP_PAGE_UP => redraw(self.go(-1)),
            keysym::ESCAPE => Action::Close,
            keysym::DOWN | keysym::KP_DOWN => pan(self, 0, line),
            keysym::UP | keysym::KP_UP => pan(self, 0, -line),
            keysym::RIGHT | keysym::KP_RIGHT => pan(self, line, 0),
            keysym::LEFT | keysym::KP_LEFT => pan(self, -line, 0),
            keysym::HOME | keysym::KP_HOME => pan(self, 0, -lay.view.content.1),
            keysym::END | keysym::KP_END => pan(self, 0, lay.view.content.1.max(page_h)),
            _ => return None,
        })
    }

    /// A press in the page's box: on a scrollbar, or a grab of the page to drag it.
    pub fn press(&mut self, x: i32, y: i32, lay: &Layout) -> bool {
        self.view.press(x, y, &lay.view)
    }

    pub fn motion(&mut self, x: i32, y: i32, lay: &Layout) -> bool {
        self.view.motion(x, y, &lay.view)
    }

    /// A scrollbar's thumb or the page is being dragged.
    pub fn dragging(&self) -> bool {
        self.view.dragging()
    }

    pub fn release(&mut self) -> bool {
        self.view.release()
    }

    pub fn leave(&mut self) -> bool {
        self.view.leave()
    }

    /// The wheel pans `rows` rows down, or across with `across`.
    pub fn wheel(&mut self, rows: i32, across: bool, lay: &Layout) -> bool {
        let d = rows * imageview::LINE;
        match across {
            true => self.view.pan(d, 0, &lay.view),
            false => self.view.pan(0, d, &lay.view),
        }
    }

    /// What the page label says.
    pub fn label(&self) -> String {
        match self.pages {
            0 => String::new(),
            n => tr!("printpreview-page", page = self.page + 1, pages = n),
        }
    }

    /// Grey the buttons that would do nothing, moving the focus off one greyed, and say which
    /// page shows.
    fn sync(&mut self) {
        let laid = self.pages > 0;
        let on = [
            (PRINT, laid),
            (NEXT, laid && self.page + 1 < self.pages),
            (PREVIOUS, laid && self.page > 0),
            (ZOOM_IN, laid && self.zoom == Zoom::Page),
            (ZOOM_OUT, laid && self.zoom == Zoom::Actual),
        ];
        for (id, enabled) in on {
            self.form.set_enabled(id, enabled);
        }
        let focused = self.form.focus().map(|i| self.form.items[i].id);
        let greyed = |id: &str| on.iter().any(|&(o, e)| o == id && !e);
        if let Some(id) = focused.filter(|id| greyed(id)) {
            let to = match id {
                NEXT => PREVIOUS,
                PREVIOUS => NEXT,
                ZOOM_IN => ZOOM_OUT,
                ZOOM_OUT => ZOOM_IN,
                _ => CLOSE,
            };
            self.form.focus_id(if greyed(to) { CLOSE } else { to });
        }
        let label = self.label();
        self.form.set_text(PAGE, &label);
    }
}

fn redraw(changed: bool) -> Action {
    match changed {
        true => Action::Redraw,
        false => Action::None,
    }
}

/// Print..., Next Page, Prev Page, Zoom In, Zoom Out and Close, then the page n of m.
fn toolbar() -> Form {
    let labels = [
        tr!("printpreview-print"),
        tr!("printpreview-next"),
        tr!("printpreview-previous"),
        tr!("printpreview-zoom-in"),
        tr!("printpreview-zoom-out"),
        tr!("printpreview-close"),
    ];
    let mut items: Vec<Item> = BUTTONS
        .iter()
        .zip(&labels)
        .enumerate()
        .map(|(i, (id, label))| {
            let x = PAD + i as i32 * (BUTTON_W + GAP);
            Item::button(
                id,
                Rect::new(x, PAD, BUTTON_W, BUTTON_H),
                label,
                true,
                false,
            )
        })
        .collect();
    let x = PAD + BUTTONS.len() as i32 * (BUTTON_W + GAP) + 2 * GAP;
    items.push(Item::label_at(PAGE, x, PAD + 5, PAGE_W, ""));
    let mut f = Form::new(items)
        .set_in(DialogFont::ShellDlg)
        .labels_at_ink();
    f.focus_id(CLOSE);
    f
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The button row, logical px.
    pub bar: Rect,
    /// The page's box under it, the page shown in it.
    pub view: imageview::Layout,
    /// The page's size at the zoom, device px: what [`Preview::wanted`] asks the host for.
    pub sheet: (u32, u32),
}

/// The part of the window under logical `(x, y)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Buttons,
    /// The page's box and its scrollbars.
    Page,
}

pub fn hit(lay: &Layout, x: i32, y: i32) -> Part {
    match lay.bar.contains(x, y) {
        true => Part::Buttons,
        false => Part::Page,
    }
}

/// Lay `pv` out in logical `r` at `s` device px per logical px, its buttons grown by `units`.
pub fn layout(pv: &Preview, r: Rect, s: i32, units: TextUnits, cm: &ControlMetrics) -> Layout {
    let s = s.max(1);
    let bar_h = units.len(BAR_H).min(r.height);
    let bar = Rect::new(r.x, r.y, r.width, bar_h);
    let room = Rect::new(r.x, r.y + bar_h, r.width, r.height - bar_h);
    Layout {
        bar,
        view: imageview::layout(&pv.view, room, s, cm),
        sheet: sheet_size(pv.paper, pv.zoom, room, s),
    }
}

/// The page `paper` (hundredths of a millimetre) in device px at `zoom` in the box `room`:
/// whole, as large as fits inside its inset and shadow, or at 96 dpi a logical px.
pub fn sheet_size(paper: (i32, i32), zoom: Zoom, room: Rect, s: i32) -> (u32, u32) {
    let (pw, ph) = (paper.0.max(1) as i64, paper.1.max(1) as i64);
    let (w, h) = match zoom {
        Zoom::Actual => {
            let dpi = SCREEN_DPI * s;
            (
                pagesetup::px(paper.0, dpi) as i64,
                pagesetup::px(paper.1, dpi) as i64,
            )
        }
        Zoom::Page => {
            let rw = ((room.width - 2 * INSET - SHADOW) * s) as i64;
            let rh = ((room.height - 2 * INSET - SHADOW) * s) as i64;
            match rw * ph <= rh * pw {
                true => (rw, rw * ph / pw),
                false => (rh * pw / ph, rh),
            }
        }
    };
    (w.max(1) as u32, h.max(1) as u32)
}

/// Page `drawn`, which goes at `at` (hundredths of a millimetre) on a page `paper` as it lies,
/// drawn onto white paper `size` device px: the drawing a host hands [`Preview::show`].
pub fn sheet(drawn: &Canvas, at: Rect, paper: (i32, i32), size: (u32, u32)) -> Canvas {
    let mut c = Canvas::new(size.0.max(1), size.1.max(1));
    c.fill_rect(c.bounds(), SHEET);
    let along = |v: i32, of: i32, len: u32| (v as i64 * len as i64 / of.max(1) as i64) as i32;
    let x0 = along(at.x, paper.0, size.0);
    let y0 = along(at.y, paper.1, size.1);
    let x1 = along(at.right(), paper.0, size.0).max(x0 + 1);
    let y1 = along(at.bottom(), paper.1, size.1).max(y0 + 1);
    let (dw, dh) = ((x1 - x0) as u32, (y1 - y0) as u32);
    let sw = drawn.width as usize;
    let scaled = Canvas::resample_box_from(drawn.width, drawn.height, dw, dh, |x, y| {
        drawn.pixels[y as usize * sw + x as usize]
    });
    c.blit_canvas(x0, y0, &scaled);
    c
}

/// The buttons on the face, then the page's box on grey with the page and its shadow, or a
/// note while the page is drawn.
pub fn draw(c: &mut Canvas, text: &TextRenderer, p: &Paint, pv: &Preview, lay: &Layout, px: u32) {
    c.fill_rect(lay.bar, p.cs.button_face);
    form::draw(c, text, p, &pv.form, true, px);
    let ground = p.cs.button_shadow;
    let Some((_, sheet)) = pv.sheet.as_ref().filter(|(p, _)| *p == pv.page) else {
        let note = match pv.pages {
            0 => tr!("printpreview-laying-out"),
            _ => tr!("printpreview-drawing"),
        };
        let r = Rect::new(
            lay.bar.x,
            lay.bar.bottom(),
            lay.bar.width,
            lay.view.view.height,
        );
        imageview::draw_message(c, text, r, &note, (ground, p.cs.button_hilight), px);
        return;
    };
    let src = Source {
        picture: sheet,
        rendered: Some(sheet),
    };
    imageview::draw(c, p, &pv.view, src, &lay.view, ground);
    let (ox, oy) = pv.view.origin(&lay.view);
    let s = lay.view.s;
    let (w, h) = (sheet.width as i32, sheet.height as i32);
    let frame = p.cs.window_frame;
    let shadow = p.cs.button_dk_shadow;
    let clip = lay.view.device_view();
    c.in_device_px(|c| {
        c.with_clip(clip, |c| {
            c.fill_rect(
                Rect::new(ox + w + s, oy + SHADOW * s, SHADOW * s, h + s),
                shadow,
            );
            c.fill_rect(
                Rect::new(ox + SHADOW * s, oy + h + s, w + s, SHADOW * s),
                shadow,
            );
            for edge in [
                Rect::new(ox - s, oy - s, w + 2 * s, s),
                Rect::new(ox - s, oy + h, w + 2 * s, s),
                Rect::new(ox - s, oy, s, h),
                Rect::new(ox + w, oy, s, h),
            ] {
                c.fill_rect(edge, frame);
            }
        })
    });
}

/// The buttons, the page label and the page as an image named by the label.
pub fn access(
    pv: &Preview,
    lay: &Layout,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
) -> Option<NodeId> {
    let focus = pv.form.access(tree, parent, text, px);
    let name = match pv.pages {
        0 => tr!("printpreview-laying-out"),
        _ => pv.label(),
    };
    tree.push(
        parent,
        "sheet",
        access::titled(Role::Image, lay.view.view, &name),
    );
    focus
}

#[cfg(test)]
mod tests;
