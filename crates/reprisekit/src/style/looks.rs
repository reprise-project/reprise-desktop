use crate::raster::{Canvas, Insets, Rect};
use crate::theme::Color;

use super::State;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlassSpec {
    /// Pixels outside the frame per row, outer row first; all four corners
    /// mirror it.
    pub corner: &'static [i32],
    pub panel: PanelSpec,
    /// How the glass lifts the backdrop under a focused and an unfocused frame's art.
    pub lift: [crate::backdrop::Lift; 2],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PanelSpec {
    pub shadow: Insets,
    pub corner: &'static [i32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GroupLabel {
    pub nudge: (i32, i32),
    pub before: i32,
    pub after: i32,
    pub rows: i32,
}

impl GroupLabel {
    pub const LINE: GroupLabel = GroupLabel {
        nudge: (0, 0),
        before: 3,
        after: 1,
        rows: 1,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrackMarks {
    pub drop: i32,
    pub tick_y: i32,
    pub tick_len: i32,
    pub end_len: i32,
    pub end_len_v: i32,
    pub sparse: bool,
    pub ink: Color,
}

impl TrackMarks {
    pub const LOW: TrackMarks = TrackMarks {
        drop: 1,
        tick_y: 23,
        tick_len: 3,
        end_len: 3,
        end_len_v: 4,
        sparse: false,
        ink: Color::rgb(0x80, 0x80, 0x80),
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DropList {
    pub top: Color,
    pub sides: Color,
    pub bottom: Color,
    pub top_corners: Color,
    pub bottom_corners: Color,
    pub shadow: Option<&'static DropShadow>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DropShadow {
    pub opacity: f64,
    /// Gaussian standard deviation, design px.
    pub blur: f64,
    pub spread: f64,
    pub inset: f64,
}

// The fields are fixed design constants, never NaN.
impl Eq for DropShadow {}

impl DropShadow {
    pub const N: usize = 5;

    pub fn alpha(&self, w: f64, h: f64, x: f64, y: f64) -> f64 {
        let n = Self::N as f64;
        if (x < w && y < h) || x < 0.0 || y < 0.0 || x >= w + n || y >= h + n {
            return 0.0;
        }
        let ramp = |t: f64| 0.5 * (1.0 + erf(t / (self.blur * std::f64::consts::SQRT_2)));
        self.opacity
            * ramp(x - self.inset)
            * ramp(w + self.spread - x)
            * ramp(y - self.inset)
            * ramp(h + self.spread - y)
    }
}

/// Abramowitz and Stegun 7.1.26, within 1.5e-7.
fn erf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let y = 1.0 - poly * (-x * x).exp();
    y.copysign(x)
}

/// How a look draws its popup menus when it gives them an icon gutter.
/// Sizes are logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuLook {
    pub frame: Color,
    /// The two rows inside the frame; None leaves them face.
    pub ring: Option<Color>,
    /// None: the scheme's menu colour.
    pub face: Option<Color>,
    pub row: i32,
    pub separator: i32,
    pub gutter: i32,
    pub gutter_fill: Color,
    pub divider: (Color, Color),
    pub rule: (Color, Color),
    pub text_x: i32,
    pub icon_x: i32,
    pub right_pad: i32,
    pub shortcut_end: i32,
    pub arrow_inset: i32,
    pub arrow: Color,
    pub greyed: Color,
    pub check_at: (i32, i32),
    pub radio_at: (i32, i32),
    /// None: the menu casts no shadow.
    pub shadow: Option<&'static DropShadow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionBox {
    pub border: Color,
    pub fill: Color,
    pub alpha: u8,
}

impl SelectionBox {
    pub fn rects(&self, r: Rect, t: i32) -> Vec<(Rect, Color, u8)> {
        let t = t.max(1);
        let (top, left) = (t.min(r.height), t.min(r.width));
        let bottom = (r.height - t).max(top);
        let right = (r.width - t).max(left);
        let line = |x, y, w, h| (Rect::new(r.x + x, r.y + y, w, h), self.border, 255);
        [
            (
                Rect::new(r.x + left, r.y + top, right - left, bottom - top),
                self.fill,
                self.alpha,
            ),
            line(0, 0, r.width, top),
            line(0, bottom, r.width, r.height - bottom),
            line(0, top, left, bottom - top),
            line(right, top, r.width - right, bottom - top),
        ]
        .into_iter()
        .filter(|(q, _, _)| !q.is_empty())
        .collect()
    }
}

/// Colours are (colour, alpha), an alpha of 0 laying nothing; sizes are
/// logical px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaptionTabLook {
    pub radius: i32,
    pub selected: (Color, u8),
    pub outline: (Color, u8),
    pub hot: (Color, u8),
    pub rest: (Color, u8),
    pub ghost: (Color, u8),
    pub close: i32,
    pub close_hot: (Color, u8),
    pub close_held: (Color, u8),
    pub other_ink: (Color, Color),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BalloonLook {
    pub frame: Color,
    pub fill: (Color, Color),
    pub corner: &'static [i32],
    pub tail: (i32, i32, i32),
    pub title: Color,
    pub title_px: u32,
    pub title_y: i32,
    pub text: Color,
    pub body_y: i32,
    pub line_h: i32,
    pub bottom: i32,
    pub icon_at: (i32, i32),
    pub text_x: i32,
    pub right: i32,
    pub close: (i32, i32, i32, i32),
    pub cross_at: (i32, i32),
    pub cross: &'static [u16],
    pub cross_ink: (Color, Color),
    pub shadow: Option<&'static DropShadow>,
}

/// A notification sheet's outline. Every `match` on it is exhaustive, so a
/// shape comes with its art.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetShape {
    Lozenge,
    Halo,
    Droplet,
}

/// A squircle well's corner radius as a share of its side; a picture in the slot rounds the same.
pub const SQUIRCLE_ROUND: f64 = 0.3;

/// The glass well under a notification's icon or badge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Well {
    Squircle,
    Round,
    Badge,
}

/// What a notification sheet is painted for, in design px.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SheetSpec {
    pub shape: SheetShape,
    pub size: (i32, i32),
    pub tint: Option<Color>,
    /// The pointer is over the sheet: its rim lifts.
    pub hot_close: bool,
    /// An icon well standing out of the sheet, from the sheet's top-left.
    pub breakout: Option<Rect>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotifyInks {
    pub title: Color,
    pub text: Color,
    pub app: Color,
}

/// A look's notification sheets and the parts on them. Every painter takes
/// design px and returns a canvas of device px at `d` per design px.
#[derive(Clone, Copy, Debug)]
pub struct NotifyLook {
    pub inks: NotifyInks,
    pub dark: bool,
    pub corner: fn(SheetShape) -> &'static [i32],
    /// The shadow's room around the sheet and its breakout.
    pub margin: fn(SheetShape) -> Insets,
    /// `size` (and `breakout`) grown by `margin`.
    pub sheet: fn(&NotifyLook, &SheetSpec, d: i32) -> Canvas,
    /// A round drop of the sheet's glass `size` across, centred in its canvas.
    pub bead: fn(&NotifyLook, size: i32, d: i32) -> Canvas,
    /// `size` square grown evenly on every side for its shadow: it centres on
    /// the slot.
    pub well: fn(&NotifyLook, Well, size: i32, d: i32) -> Canvas,
    pub pill: fn(&NotifyLook, (i32, i32), State, d: i32) -> Canvas,
    /// The round close button, its glyph drawn.
    pub close: fn(&NotifyLook, State, d: i32) -> Canvas,
    /// The "+N more" chip's background; the control draws its text.
    pub chip: fn(&NotifyLook, (i32, i32), State, d: i32) -> Canvas,
    /// The colour an icon reads as, if it has one.
    pub colour: fn(&Canvas) -> Option<Color>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlyoutLook {
    pub shadow: Insets,
    pub opaque_under: Color,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PageInks {
    pub heading: Color,
    pub link: Color,
    pub link_hot: Color,
    pub rule: Color,
    pub detail: Color,
    pub list_frame: Color,
    pub band_rule: [Color; 5],
    pub band: Color,
}

impl PageInks {
    pub const LIGHT: PageInks = PageInks {
        heading: Color::rgb(0x00, 0x33, 0x99),
        link: Color::rgb(0x00, 0x66, 0xCC),
        link_hot: Color::rgb(0x33, 0x99, 0xFF),
        rule: Color::rgb(0xCC, 0xCC, 0xCC),
        detail: Color::rgb(0x6D, 0x6D, 0x6D),
        list_frame: Color::rgb(0x00, 0x00, 0x00),
        band_rule: [
            Color::rgb(0xCC, 0xD9, 0xEA),
            Color::rgb(0xD9, 0xE3, 0xF0),
            Color::rgb(0xE8, 0xEE, 0xF7),
            Color::rgb(0xED, 0xF2, 0xF9),
            Color::rgb(0xF0, 0xF4, 0xFA),
        ],
        band: Color::rgb(0xF1, 0xF5, 0xFB),
    };

    pub fn map(self, f: impl Fn(Color) -> Color) -> PageInks {
        PageInks {
            heading: f(self.heading),
            link: f(self.link),
            link_hot: f(self.link_hot),
            rule: f(self.rule),
            detail: f(self.detail),
            list_frame: f(self.list_frame),
            band_rule: self.band_rule.map(&f),
            band: f(self.band),
        }
    }
}
