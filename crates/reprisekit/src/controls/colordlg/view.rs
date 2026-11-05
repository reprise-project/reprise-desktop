//! The Color dialog's window: the wells, Define Custom Colors >>, OK and Cancel down the left;
//! once widened, the field, the luminosity bar, the preview, the spinners and Add to Custom
//! Colors to the right. The wells, field, bar and preview draw over the form's blank items.

use super::{BASIC_ROWS, COLS, CUSTOM_ROWS, ColorDlg, Grid, HLS_MAX, Outcome, Well, rgb_at};
use crate::access::accesskit::{NodeId, Role};
use crate::access::{self, Tree};
use crate::classic::widgets::{BUTTON_H, BUTTON_W};
use crate::controls::form::{self, Control, Form, Item};
use crate::controls::updown::{self, UpDown};
use crate::look::Paint;
use crate::look::widgets::{draw_field_frame, draw_focus_ring};
use crate::raster::{Canvas, Rect};
use crate::shape;
use crate::text::TextRenderer;
use crate::theme::Color;

pub const BASIC_LABEL: &str = "basiclabel";
pub const BASIC_WELLS: &str = "basic";
pub const CUSTOM_LABEL: &str = "customlabel";
pub const CUSTOM_WELLS: &str = "custom";
pub const DEFINE: &str = "define";
pub const OK: &str = "ok";
pub const CANCEL: &str = "cancel";
pub const FIELD: &str = "field";
pub const LUM_BAR: &str = "lumbar";
pub const PREVIEW: &str = "preview";
pub const HUE: &str = "hue";
pub const SAT: &str = "sat";
pub const LUM: &str = "lum";
pub const RED: &str = "red";
pub const GREEN: &str = "green";
pub const BLUE: &str = "blue";
pub const ADD: &str = "add";

/// The client before and after Define Custom Colors >>.
pub const CLIENT_NARROW: (i32, i32) = (224, 296);
pub const CLIENT_WIDE: (i32, i32) = (492, 296);

const M: i32 = 8;
const WELLS_W: i32 = 208;
const CELL_H: i32 = 22;
const BASIC_Y: i32 = 24;
const CUSTOM_LABEL_Y: i32 = 164;
const CUSTOM_Y: i32 = 180;
const DEFINE_Y: i32 = 232;
const BUTTONS_Y: i32 = 265;
const RIGHT_X: i32 = 232;
const FIELD_W: i32 = 216;
const FIELD_H: i32 = 184;
/// The luminosity item: the bar, a 2 px gap, then the arrow's column.
const LUM_X: i32 = RIGHT_X + FIELD_W + 8;
const BAR_W: i32 = 12;
const ARROW_GAP: i32 = 2;
const ARROW: (i32, i32) = (7, 11);
const PREVIEW_Y: i32 = 198;
const PREVIEW_SIZE: (i32, i32) = (64, 40);
const SPIN_PITCH: i32 = 22;
const SPIN_COLS: [(i32, i32, i32); 2] = [(304, 30, 336), (392, 40, 434)];
/// A well's inset from its cell, and the crosshair's gap from its point and arm length.
const WELL_INSET: i32 = 3;
const CROSS_GAP: i32 = 3;
const CROSS_ARM: i32 = 5;
/// The crosshair is black in every look: it sits on the field's colours, not the window's.
const CROSS_INK: Color = Color::rgb(0, 0, 0);

pub fn client(dlg: &ColorDlg) -> (i32, i32) {
    if dlg.expanded() {
        CLIENT_WIDE
    } else {
        CLIENT_NARROW
    }
}

fn blank(id: &'static str, r: Rect) -> Item {
    Item::new(id, r, Control::Blank)
}

fn spin(id: &'static str, x: i32, y: i32, value: i32, max: i32) -> Item {
    Item::new(
        id,
        Rect::new(x, y, updown::WIDTH, updown::HEIGHT),
        Control::UpDown(UpDown::new(value, 0, max)),
    )
}

/// The window for `dlg`, narrow or widened as it is; the focus on OK.
pub fn form(dlg: &ColorDlg) -> Form {
    let wide = dlg.expanded();
    let mut items = vec![
        Item::label_at(BASIC_LABEL, M, M, WELLS_W, &tr!("colordlg-basic")),
        blank(
            BASIC_WELLS,
            Rect::new(M, BASIC_Y, WELLS_W, CELL_H * BASIC_ROWS as i32),
        ),
        Item::label_at(
            CUSTOM_LABEL,
            M,
            CUSTOM_LABEL_Y,
            WELLS_W,
            &tr!("colordlg-custom"),
        ),
        blank(
            CUSTOM_WELLS,
            Rect::new(M, CUSTOM_Y, WELLS_W, CELL_H * CUSTOM_ROWS as i32),
        ),
        Item::button(
            DEFINE,
            Rect::new(M, DEFINE_Y, WELLS_W, BUTTON_H),
            &tr!("colordlg-define"),
            !wide,
            false,
        ),
        Item::button(
            OK,
            Rect::new(M, BUTTONS_Y, BUTTON_W, BUTTON_H),
            &tr!("ok"),
            true,
            true,
        ),
        Item::button(
            CANCEL,
            Rect::new(M + BUTTON_W + 6, BUTTONS_Y, BUTTON_W, BUTTON_H),
            &tr!("cancel"),
            true,
            false,
        ),
    ];
    if wide {
        let hsl = dlg.hsl();
        let c = dlg.color();
        items.extend([
            blank(FIELD, Rect::new(RIGHT_X, M, FIELD_W, FIELD_H)),
            blank(
                LUM_BAR,
                Rect::new(LUM_X, M, BAR_W + ARROW_GAP + ARROW.0 + 1, FIELD_H),
            ),
            blank(
                PREVIEW,
                Rect::new(RIGHT_X, PREVIEW_Y, PREVIEW_SIZE.0, PREVIEW_SIZE.1),
            ),
            Item::label_at(
                "previewlabel",
                RIGHT_X,
                PREVIEW_Y + PREVIEW_SIZE.1 + 4,
                PREVIEW_SIZE.0 + 8,
                &tr!("colordlg-preview"),
            ),
        ]);
        let rows = [
            [
                (HUE, "huelabel", tr!("colordlg-hue"), hsl.h, HLS_MAX - 1),
                (RED, "redlabel", tr!("colordlg-red"), i32::from(c.r), 255),
            ],
            [
                (SAT, "satlabel", tr!("colordlg-sat"), hsl.s, HLS_MAX),
                (
                    GREEN,
                    "greenlabel",
                    tr!("colordlg-green"),
                    i32::from(c.g),
                    255,
                ),
            ],
            [
                (LUM, "lumlabel", tr!("colordlg-lum"), hsl.l, HLS_MAX),
                (BLUE, "bluelabel", tr!("colordlg-blue"), i32::from(c.b), 255),
            ],
        ];
        // Hue, Sat, Lum come before Red, Green, Blue in the tab order.
        for col in 0..2 {
            let (lx, lw, sx) = SPIN_COLS[col];
            for (row, cells) in rows.iter().enumerate() {
                let (id, label_id, label, value, max) = &cells[col];
                let y = PREVIEW_Y + row as i32 * SPIN_PITCH;
                items.push(Item::label_at(label_id, lx, y + 3, lw, label));
                items.push(spin(id, sx, y, *value, *max));
            }
        }
        let right = SPIN_COLS[1].2 + updown::WIDTH;
        items.push(Item::button(
            ADD,
            Rect::new(RIGHT_X, BUTTONS_Y, right - RIGHT_X, BUTTON_H),
            &tr!("colordlg-add"),
            true,
            false,
        ));
    }
    let mut f = Form::new(items);
    f.focus_id(OK);
    if dlg.grid().is_some() {
        f.set_focus(None);
    }
    f
}

/// Brings the spinners to `dlg`'s colour, all but `skip`, the one being typed in.
pub fn follow(f: &mut Form, dlg: &ColorDlg, skip: Option<&str>) {
    let (hsl, c) = (dlg.hsl(), dlg.color());
    let values = [
        (HUE, hsl.h),
        (SAT, hsl.s),
        (LUM, hsl.l),
        (RED, i32::from(c.r)),
        (GREEN, i32::from(c.g)),
        (BLUE, i32::from(c.b)),
    ];
    for (id, v) in values {
        if Some(id) == skip {
            continue;
        }
        if let Some(Control::UpDown(u)) = f.item_mut(id).map(|i| &mut i.control) {
            u.set(v);
        }
    }
}

pub fn spin_value(f: &Form, id: &str) -> Option<i32> {
    match &f.item(id)?.control {
        Control::UpDown(u) => Some(u.value()),
        _ => None,
    }
}

/// The access key label `id` underlines, lower case.
pub(super) fn access_char(f: &Form, id: &str) -> Option<char> {
    let Control::Label { text, .. } = &f.item(id)?.control else {
        return None;
    };
    crate::text::access_key(text).map(|c| c.to_ascii_lowercase())
}

/// Where the parts drawn over the form land, from the rects it laid its blanks out at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Layout {
    pub basic: Rect,
    pub custom: Rect,
    /// The field's frame, and the hues and saturations inside it; `None` while narrow.
    pub field: Option<Rect>,
    pub bar: Option<Rect>,
    /// The arrow's column right of the bar, which takes presses too.
    pub arrow: Option<Rect>,
    pub preview: Option<Rect>,
}

pub fn layout(f: &Form) -> Layout {
    let grown = f.rect(LUM_BAR);
    let u = f.units();
    Layout {
        basic: f.rect(BASIC_WELLS).unwrap_or_default(),
        custom: f.rect(CUSTOM_WELLS).unwrap_or_default(),
        field: f.rect(FIELD),
        bar: grown.map(|r| Rect::new(r.x, r.y, u.len(BAR_W), r.height)),
        arrow: grown.map(|r| {
            let x = r.x + u.len(BAR_W);
            Rect::new(x, r.y, r.right() - x, r.height)
        }),
        preview: f.rect(PREVIEW),
    }
}

impl Layout {
    fn grid(&self, grid: Grid) -> Rect {
        match grid {
            Grid::Basic => self.basic,
            Grid::Custom => self.custom,
        }
    }

    /// Well `i`'s cell in `grid`.
    pub fn cell(&self, grid: Grid, i: usize) -> Rect {
        let r = self.grid(grid);
        let rows = grid.rows() as i32;
        let (col, row) = ((i % COLS) as i32, (i / COLS) as i32);
        let x0 = r.x + col * r.width / COLS as i32;
        let x1 = r.x + (col + 1) * r.width / COLS as i32;
        let y0 = r.y + row * r.height / rows;
        let y1 = r.y + (row + 1) * r.height / rows;
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn well(&self, grid: Grid, i: usize) -> Rect {
        self.cell(grid, i).inset(WELL_INSET)
    }

    /// The hues and saturations: the field inside its frame.
    fn inside(&self) -> Option<Rect> {
        self.field.map(|r| r.inset(2))
    }

    fn lum_inside(&self) -> Option<Rect> {
        self.bar.map(|r| r.inset(2))
    }

    /// The hue and saturation at (`x`, `y`), clamped into the field.
    pub fn hue_sat_at(&self, x: i32, y: i32) -> Option<(i32, i32)> {
        let r = self.inside()?;
        let h = ((2 * (x - r.x) + 1) * HLS_MAX / (2 * r.width.max(1))).clamp(0, HLS_MAX - 1);
        let s = HLS_MAX - ((y - r.y) * HLS_MAX / (r.height - 1).max(1)).clamp(0, HLS_MAX);
        Some((h, s))
    }

    pub fn lum_at(&self, y: i32) -> Option<i32> {
        let r = self.lum_inside()?;
        Some(HLS_MAX - ((y - r.y) * HLS_MAX / (r.height - 1).max(1)).clamp(0, HLS_MAX))
    }

    /// The point the crosshair marks for hue `h` and saturation `s`.
    pub fn cross_at(&self, h: i32, s: i32) -> Option<(i32, i32)> {
        let r = self.inside()?;
        Some((
            r.x + (h * r.width + r.width / 2) / HLS_MAX,
            r.y + (HLS_MAX - s) * (r.height - 1) / HLS_MAX,
        ))
    }

    /// The row the arrow points at for luminosity `l`.
    pub fn arrow_at(&self, l: i32) -> Option<i32> {
        let r = self.lum_inside()?;
        Some(r.y + (HLS_MAX - l) * (r.height - 1) / HLS_MAX)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Well(Well),
    Field { h: i32, s: i32 },
    Lum(i32),
}

/// What (`x`, `y`) presses: a well's cell, a point of the field, or a row of the bar or its arrow.
pub fn hit(lay: &Layout, x: i32, y: i32) -> Option<Hit> {
    for grid in [Grid::Basic, Grid::Custom] {
        if lay.grid(grid).contains(x, y) {
            let n = COLS * grid.rows();
            return (0..n)
                .find(|&i| lay.cell(grid, i).contains(x, y))
                .map(|i| Hit::Well(Well { grid, i }));
        }
    }
    if lay.inside().is_some_and(|r| r.contains(x, y)) {
        let (h, s) = lay.hue_sat_at(x, y)?;
        return Some(Hit::Field { h, s });
    }
    let on_lum = [lay.bar, lay.arrow]
        .into_iter()
        .flatten()
        .any(|r| r.contains(x, y));
    if on_lum {
        return lay.lum_at(y).map(Hit::Lum);
    }
    None
}

impl ColorDlg {
    /// A primary press at (`x`, `y`): `None` when it misses the wells, field and bar, for the form
    /// to take.
    pub fn press(&mut self, f: &mut Form, x: i32, y: i32) -> Option<Outcome> {
        if f.popup_open() {
            return None;
        }
        match hit(&layout(f), x, y)? {
            Hit::Well(w) => {
                self.pick(w);
                f.set_focus(None);
            }
            Hit::Field { h, s } => {
                self.press_field(h, s);
            }
            Hit::Lum(l) => {
                self.press_lum(l);
            }
        }
        follow(f, self, None);
        Some(Outcome::Redraw)
    }

    /// The pointer at (`x`, `y`): while a press on the field or bar drags, the colour follows it.
    pub fn motion(&mut self, f: &mut Form, x: i32, y: i32) -> Option<Outcome> {
        if !self.dragging() {
            return None;
        }
        let lay = layout(f);
        let (h, s) = lay.hue_sat_at(x, y)?;
        let l = lay.lum_at(y)?;
        if self.drag_to(h, s, l) {
            follow(f, self, None);
            return Some(Outcome::Redraw);
        }
        Some(Outcome::None)
    }
}

/// `c` as `#rrggbb`.
pub fn hex(c: Color) -> String {
    format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b)
}

/// The window: the form, then the wells, and once widened the field with its crosshair, the bar
/// with its arrow and the preview. Those are drawn on device pixels, at the canvas's scale times
/// `p.s`.
pub fn draw_window(
    c: &mut Canvas,
    text: &TextRenderer,
    p: &Paint,
    f: &Form,
    dlg: &ColorDlg,
    px: u32,
) {
    form::draw(c, text, p, f, true, px);
    let dev = Paint {
        s: p.s * c.scale(),
        ..*p
    };
    c.in_device_px(|d| draw_parts(d, &dev, &layout(f), dlg));
}

fn draw_parts(c: &mut Canvas, p: &Paint, lay: &Layout, dlg: &ColorDlg) {
    for grid in [Grid::Basic, Grid::Custom] {
        draw_wells(c, p, lay, dlg, grid);
    }
    let hsl = dlg.hsl();
    if let Some(r) = lay.inside() {
        frame(c, p, r.inset(-2));
        draw_field(c, p.s, r);
        if let Some(at) = lay.cross_at(hsl.h, hsl.s) {
            draw_cross(c, p.s, r, at);
        }
    }
    if let Some(r) = lay.lum_inside() {
        frame(c, p, r.inset(-2));
        draw_lum(c, p.s, r, hsl.h, hsl.s);
    }
    if let (Some(col), Some(y)) = (lay.arrow, lay.arrow_at(hsl.l)) {
        draw_arrow(c, p, col.x + ARROW_GAP, y);
    }
    if let Some(r) = lay.preview {
        let d = r.scaled(p.s);
        c.fill_rect(d, dlg.color());
        draw_field_frame(c, p, d, dlg.color());
    }
}

fn frame(c: &mut Canvas, p: &Paint, r: Rect) {
    let d = r.scaled(p.s);
    c.fill_rect(d, p.cs.window);
    draw_field_frame(c, p, d, p.cs.window);
}

fn draw_wells(c: &mut Canvas, p: &Paint, lay: &Layout, dlg: &ColorDlg, grid: Grid) {
    let s = p.s;
    let n = COLS * grid.rows();
    for i in 0..n {
        let color = dlg.well_color(Well { grid, i });
        let well = lay.well(grid, i).scaled(s);
        c.fill_rect(well, color);
        draw_field_frame(c, p, well, color);
        if dlg.picked() == Some(Well { grid, i }) {
            let ring = lay.well(grid, i).inset(-2).scaled(s);
            c.ring_t(ring, p.cs.button_text, p.cs.button_text, s);
        }
        if dlg.grid() == Some(grid) && dlg.cursor(grid) == i {
            draw_focus_ring(c, p, lay.cell(grid, i).scaled(s), p.cs.button_text);
        }
    }
}

/// Every hue across `r` (logical, drawn at `s`), full saturation at the top to grey at the
/// bottom, at the middle luminosity; each device pixel takes the colour at its middle.
fn draw_field(c: &mut Canvas, s: i32, r: Rect) {
    let d = r.scaled(s);
    let max = f64::from(HLS_MAX);
    let (w, h) = (f64::from(r.width), f64::from((r.height - 1).max(1)));
    for y in 0..d.height {
        let sat = max - shape::centre(y, s) * max / h;
        for x in 0..d.width {
            let hue = (shape::centre(x, s) + 0.5) * max / w;
            c.put(d.x + x, d.y + y, rgb_at(hue, sat, max / 2.0));
        }
    }
}

/// Luminosity from white at the top of `r` to black at the bottom, at hue `h` and saturation `s`.
fn draw_lum(c: &mut Canvas, scale: i32, r: Rect, h: i32, s: i32) {
    let d = r.scaled(scale);
    let max = f64::from(HLS_MAX);
    let span = f64::from((r.height - 1).max(1));
    for y in 0..d.height {
        let l = max - shape::centre(y, scale) * max / span;
        let color = rgb_at(f64::from(h), f64::from(s), l);
        c.fill_rect(Rect::new(d.x, d.y + y, d.width, 1), color);
    }
}

/// The crosshair round `at`: four arms 3 px thick, clear of the point, kept inside `field`, at `s`.
fn draw_cross(c: &mut Canvas, s: i32, field: Rect, (x, y): (i32, i32)) {
    let (g, a) = (CROSS_GAP, CROSS_ARM);
    let arms = [
        Rect::new(x - g - a, y - 1, a, 3),
        Rect::new(x + g + 1, y - 1, a, 3),
        Rect::new(x - 1, y - g - a, 3, a),
        Rect::new(x - 1, y + g + 1, 3, a),
    ];
    c.with_clip(field.scaled(s), |c| {
        for r in arms {
            c.fill_rect(r.scaled(s), CROSS_INK);
        }
    });
}

/// The arrow pointing left at row `y`, its tip at column `x`.
fn draw_arrow(c: &mut Canvas, p: &Paint, x: i32, y: i32) {
    let s = p.s;
    let (w, h) = ARROW;
    let k = f64::from(s);
    let mid = f64::from(h) / 2.0 * k;
    let pts = [
        (0.0, mid),
        (f64::from(w) * k, 0.0),
        (f64::from(w) * k, f64::from(h) * k),
    ];
    let white = Color::rgb(0xFF, 0xFF, 0xFF);
    let mask = shape::draw(w, h, s, |px, py| {
        let cover = shape::polygon(px, py, &pts);
        shape::premul(white, (cover.clamp(0.0, 1.0) * 255.0).round() as u8)
    });
    c.blit_alpha(x * s, (y - h / 2) * s, &mask, p.cs.button_text);
}

/// The window's access part: the form's controls, a group of buttons per grid, the field as a
/// colour well and the bar as a slider. Returns the node with the keyboard.
pub fn access_window(
    f: &Form,
    dlg: &ColorDlg,
    tree: &mut Tree,
    parent: NodeId,
    text: &TextRenderer,
    px: u32,
) -> Option<NodeId> {
    let mut focus = f.access(tree, parent, text, px);
    let lay = layout(f);
    for (grid, label, key) in [
        (Grid::Basic, BASIC_LABEL, "basicwells"),
        (Grid::Custom, CUSTOM_LABEL, "customwells"),
    ] {
        let name = match f.item(label).map(|i| &i.control) {
            Some(Control::Label { text, .. }) => text.clone(),
            _ => String::new(),
        };
        let g = tree.push(
            parent,
            key,
            access::named(Role::Group, lay.grid(grid), &name),
        );
        for i in 0..COLS * grid.rows() {
            let w = Well { grid, i };
            let mut n = access::button(lay.well(grid, i), &hex(dlg.well_color(w)), true, false);
            n.set_selected(dlg.picked() == Some(w));
            let id = tree.push(g, i, n);
            if dlg.grid() == Some(grid) && dlg.cursor(grid) == i {
                focus = Some(id);
            }
        }
    }
    if let Some(r) = lay.field {
        let mut n = access::named(Role::ColorWell, r, &tr!("colordlg-field"));
        n.set_value(hex(dlg.color()));
        tree.push(parent, "field", n);
    }
    if let Some(r) = lay.bar {
        let mut n = access::named(Role::Slider, r, &tr!("colordlg-lum-bar"));
        n.set_numeric_value(f64::from(dlg.hsl().l));
        n.set_min_numeric_value(0.0);
        n.set_max_numeric_value(f64::from(HLS_MAX));
        tree.push(parent, "lumbar", n);
    }
    focus
}
