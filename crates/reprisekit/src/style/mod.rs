use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub mod frame;
pub(crate) mod startbloom;
pub mod taskbar;
pub(crate) mod trayarrow;

use crate::metrics::ControlMetrics;

use crate::raster::{Canvas, Fill, Insets, Rect, nine_source};
use crate::taskbar::TaskbarShape;
use crate::text::TextRenderer;
use crate::theme::{Color, ColorScheme, FontConfig, Metrics};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Caption,
    MaxCaption,
    FrameLeft,
    FrameRight,
    FrameBottom,
    /// A window drawn as a bare sheet of the frame's glass over its whole
    /// rect (Normal focused, Inactive otherwise), while a peek hides it. A
    /// look without it keeps the frame.
    WindowSheet,
    MinButton,
    MaxButton,
    RestoreButton,
    CloseButton,
    /// A state one button takes has one whole-strip frame per button, in
    /// button order.
    CaptionGroup,
    MaxCaptionGroup,
    Taskbar,
    StartButton,
    StartAbove,
    TaskButton,
    TaskStack,
    LabelRow,
    SlotEdge,
    SlotLight,
    SlotBand,
    PagerTile,
    TaskLabel,
    /// The column pairs at a labelled button's two ends, one frame each.
    TaskLabelEdge,
    TrayArea,
    PushButton,
    CheckBox,
    Radio,
    GroupBox,
    ScrollArrowUp,
    ScrollArrowDown,
    ScrollArrowLeft,
    ScrollArrowRight,
    ScrollThumbV,
    ScrollThumbH,
    ScrollGripV,
    ScrollGripH,
    ScrollTrackV,
    ScrollTrackH,
    EditBorder,
    ListBorder,
    WellFrame,
    ComboDrop,
    ComboList,
    TabItem,
    TabPane,
    ProgressBar,
    ProgressChunk,
    TrackbarTrack,
    TrackbarThumb,
    TrackbarTrackV,
    TrackbarThumbV,
    SpinUp,
    SpinDown,
    HelpButton,
    SizeBox,
    CalendarBand,
    /// A month calendar's frame, drawn round the whole box over its fill.
    CalendarFrame,
    ShowDesktop,
    PreviewFrame,
    PreviewThumb,
    PreviewClose,
    SwitcherFrame,
    SwitcherIconFrame,
    SwitcherSelect,
    SwitcherHot,
    SwitcherShadow,
    SnapFrame,
    ToolbarButton,
    ToolbarDrop,
    ToolbarSeparator,
    Rebar,
    RebarBand,
    RebarGripper,
    HeaderItem,
    HeaderSortArrow,
    HeaderFiller,
    StatusBar,
    StatusPane,
    Tooltip,
    BalloonClose,
    TaskPane,
    TaskGroupHeader,
    TaskGroupBody,
    TaskCollapse,
    TaskExpand,
    /// [`Part::TaskCollapse`] without the band behind it, for a band grown with the text.
    TaskCollapseMark,
    /// [`Part::TaskExpand`] without the band behind it, for a band grown with the text.
    TaskExpandMark,
    ExplorerBarHeader,
    ExplorerBarClose,
    PlacesBar,
    PlacesTile,
    StartPanel,
    StartPane,
    StartJumpPane,
    StartHotLeft,
    StartNewPin,
    StartHotAll,
    StartHotSplit,
    StartHotJump,
    StartLitResult,
    StartHotRight,
    StartRuleLeft,
    StartRuleRight,
    StartJumpRule,
    StartSearchRule,
    StartPictureFrame,
    StartShutdown,
    StartShutdownArrow,
    StartShutdownLight,
    StartShutdownArrowLight,
    StartSearchField,
    StartMagnifier,
    StartClear,
    StartTriangleRight,
    StartTriangleLeft,
    StartRowArrow,
    MenuItem,
    MenuCheck,
    MenuCheckMark,
    MenuRadioMark,
    /// A window's menu bar strip, its full [`BAR_H`] rows.
    ///
    /// [`BAR_H`]: crate::controls::menubar::BAR_H
    MenuBar,
    MenuBarItem,
    FlyoutShadow,
    CalendarToday,
    CalendarPrev,
    CalendarNext,
    ListItem,
    DesktopItem,
    TreeExpander,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum State {
    Normal,
    Hot,
    Pressed,
    Disabled,
    Inactive,
    InactiveHot,
    Checked,
    CheckedHot,
    Default,
    Focused,
    BarHot,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Check {
    Unchecked,
    Checked,
    Mixed,
}

/// The states within each [`Check`]'s run of a check atlas, in atlas
/// order.
pub const CHECK_STATES: [State; 4] = [State::Normal, State::Hot, State::Pressed, State::Disabled];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Vertical,
    Horizontal,
}

/// One part's art: an atlas of equal frames, one per state in `states`
/// order, drawn as a nine-grid. A check box's or radio button's frames
/// also carry the [`Check`] each shows.
#[derive(Clone, Debug)]
pub struct PartImage {
    pub atlas: Canvas,
    pub states: Vec<State>,
    pub checks: Vec<Check>,
    pub layout: Layout,
    pub sizing: Insets,
    pub sizing_at_height: Option<(i32, Insets)>,
    /// Two frame rows (design px) that a target taller than the frame
    /// repeats, each alone, in place of [`PartImage::sizing`]'s one band.
    pub two_bands: Option<(i32, i32)>,
    pub content: Insets,
    pub fill: Fill,
    pub border_only: bool,
    pub true_size: bool,
    pub clip_ends: bool,
    pub split_ends: bool,
    pub outset: Insets,
    /// Atlas pixels per design pixel; every other field counts design pixels.
    pub scale: i32,
}

impl PartImage {
    pub fn new(atlas: Canvas, states: &[State], layout: Layout) -> PartImage {
        let along = match layout {
            Layout::Vertical => atlas.height,
            Layout::Horizontal => atlas.width,
        };
        debug_assert!(
            states.is_empty() || (along as usize).is_multiple_of(states.len()),
            "atlas size {along} along {layout:?} does not divide evenly by {} states",
            states.len()
        );
        PartImage {
            atlas,
            states: states.to_vec(),
            checks: Vec::new(),
            layout,
            sizing: Insets::ZERO,
            sizing_at_height: None,
            two_bands: None,
            content: Insets::ZERO,
            fill: Fill::Stretch,
            border_only: false,
            true_size: false,
            clip_ends: false,
            split_ends: false,
            outset: Insets::ZERO,
            scale: 1,
        }
    }

    /// One run of [`CHECK_STATES`] per entry of `checks`, `checks` major.
    pub fn new_check(atlas: Canvas, checks: &[Check], layout: Layout) -> PartImage {
        let states: Vec<State> = checks.iter().flat_map(|_| CHECK_STATES).collect();
        let checks: Vec<Check> = checks
            .iter()
            .flat_map(|&c| [c; CHECK_STATES.len()])
            .collect();
        debug_assert_eq!(checks.len(), states.len(), "one check per frame");
        PartImage {
            checks,
            ..PartImage::new(atlas, &states, layout)
        }
    }

    pub fn check_frame(&self, check: Check, state: State) -> Rect {
        if self.checks.is_empty() {
            return self.frame(state);
        }
        let at = |c: Check, s: State| {
            self.checks
                .iter()
                .zip(&self.states)
                .position(|(&k, &t)| (k, t) == (c, s))
        };
        let i = at(check, state)
            .or_else(|| at(check, State::Normal))
            .or_else(|| at(Check::Unchecked, State::Normal))
            .unwrap_or(0) as i32;
        let n = self.states.len().max(1) as i32;
        let (w, h) = (self.atlas.width as i32, self.atlas.height as i32);
        match self.layout {
            Layout::Vertical => Rect::new(0, i * (h / n), w, h / n),
            Layout::Horizontal => Rect::new(i * (w / n), 0, w / n, h),
        }
    }

    pub fn sizing(mut self, m: Insets) -> Self {
        self.sizing = m;
        self
    }

    pub fn sizing_at_height(mut self, h: i32, m: Insets) -> Self {
        self.sizing_at_height = Some((h, m));
        self
    }

    pub fn two_bands(mut self, a: i32, b: i32) -> Self {
        debug_assert!(a < b, "the first band ({a}) sits above the second ({b})");
        self.two_bands = Some((a, b));
        self
    }

    pub fn content(mut self, m: Insets) -> Self {
        self.content = m;
        self
    }

    pub fn fill(mut self, f: Fill) -> Self {
        self.fill = f;
        self
    }

    pub fn clip_ends(mut self) -> Self {
        self.clip_ends = true;
        self
    }

    pub fn split_ends(mut self) -> Self {
        self.split_ends = true;
        self
    }

    pub fn outset(mut self, m: Insets) -> Self {
        self.outset = m;
        self
    }

    pub fn at_scale(mut self, n: i32) -> Self {
        self.scale = n.max(1);
        self
    }

    fn per_atlas(&self, s: i32) -> i32 {
        debug_assert!(
            s % self.scale == 0,
            "an atlas painted at scale {} drawn at scale {s}",
            self.scale
        );
        (s / self.scale).max(1)
    }

    pub(crate) fn blit(&self, c: &mut Canvas, frame: Rect, r: Rect, s: i32) {
        let k = c.scale();
        if self.scale > 1 && k > 1 {
            return c.in_device_px(|c| self.blit(c, frame, r.scaled(k), s.max(1) * k));
        }
        let s = scale_for(c, s);
        let per = self.per_atlas(s);
        for (from, to, m) in self.nines(frame, r, s).into_iter().flatten() {
            c.blit_nine(&self.atlas, from, to, m, self.fill, self.border_only, per);
        }
    }

    pub fn frame(&self, state: State) -> Rect {
        self.frame_nth(state, 0)
    }

    pub fn frame_nth(&self, state: State, n: usize) -> Rect {
        let mut own = self
            .states
            .iter()
            .enumerate()
            .filter(|&(_, &s)| s == state)
            .map(|(i, _)| i);
        let first = own.next();
        let i = if n == 0 {
            first
        } else {
            own.nth(n - 1).or(first)
        };
        let i = i.unwrap_or(0) as i32;
        let n = self.states.len().max(1) as i32;
        let (w, h) = (self.atlas.width as i32, self.atlas.height as i32);
        match self.layout {
            Layout::Vertical => Rect::new(0, i * (h / n), w, h / n),
            Layout::Horizontal => Rect::new(i * (w / n), 0, w / n, h),
        }
    }

    fn target_at(&self, r: Rect, s: i32) -> Rect {
        if !self.true_size {
            let m = self.outset;
            return Rect::new(
                r.x - s * m.left,
                r.y - s * m.top,
                r.width + s * (m.left + m.right),
                r.height + s * (m.top + m.bottom),
            );
        }
        let f = self.frame(State::Normal);
        let (w, h) = (f.width / self.scale, f.height / self.scale);
        Rect::new(
            r.x + s * ((r.width / s - w) / 2),
            r.y + s * ((r.height / s - h) / 2),
            w * s,
            h * s,
        )
    }

    fn nines(&self, from: Rect, r: Rect, s: i32) -> [Option<(Rect, Rect, Insets)>; 2] {
        let to = self.target_at(r, s);
        let m = match self.sizing_at_height {
            Some((h, m)) if to.height == h * s => m,
            _ => self.sizing,
        };
        let from = from.scaled(self.per_atlas(s));
        match self.two_bands {
            Some(bands) if to.height > from.height => {
                Self::banded(from, to, m.scaled(s), bands, s).map(Some)
            }
            _ => self.pieces(from, to, m.scaled(s), s),
        }
    }

    /// The two nine-grids of a [`PartImage::two_bands`] part drawn from
    /// `from` into the taller `to` (both as [`PartImage::nines`] has them).
    fn banded(
        from: Rect,
        to: Rect,
        m: Insets,
        (a, b): (i32, i32),
        s: i32,
    ) -> [(Rect, Rect, Insets); 2] {
        let cut = s * ((a + b + 1) / 2);
        let up = s * (((to.height - from.height) / s + 1) / 2);
        let (a, b) = (a * s, b * s);
        [
            (
                Rect::new(from.x, from.y, from.width, cut),
                Rect::new(to.x, to.y, to.width, cut + up),
                Insets::new(m.left, a, m.right, cut - a - s),
            ),
            (
                Rect::new(from.x, from.y + cut, from.width, from.height - cut),
                Rect::new(to.x, to.y + cut + up, to.width, to.height - cut - up),
                Insets::new(m.left, b - cut, m.right, from.height - b - s),
            ),
        ]
    }

    fn pieces(&self, from: Rect, to: Rect, m: Insets, s: i32) -> [Option<(Rect, Rect, Insets)>; 2] {
        if !(self.clip_ends || self.split_ends) || to.width >= m.left + m.right {
            return [Some((from, to, m)), None];
        }
        let rv = if self.split_ends {
            s * (to.width / s / 2)
        } else {
            m.right.min(to.width)
        };
        let lv = to.width - rv;
        let rows = Insets::new(0, m.top, 0, m.bottom);
        [
            Some((
                Rect::new(from.x, from.y, lv, from.height),
                Rect::new(to.x, to.y, lv, to.height),
                rows,
            )),
            Some((
                Rect::new(from.right() - rv, from.y, rv, from.height),
                Rect::new(to.right() - rv, to.y, rv, to.height),
                rows,
            )),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Color,
    pub dx: i32,
    pub dy: i32,
    pub soft: Option<SoftShadow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftShadow {
    pub size: i32,
    /// Scales the coverage summed under each pixel (numerator, denominator),
    /// capped at full.
    pub gain: (u32, u32),
}

impl Shadow {
    pub fn alpha(&self, coverage: &[u8], w: usize, h: usize, d: i32) -> Vec<u8> {
        let (ox, oy) = (self.dx * d, self.dy * d);
        let (n, gain) = match self.soft {
            Some(soft) => (soft.size * d, soft.gain),
            None => (1, (1, 1)),
        };
        let mut sums = vec![0u64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0u64;
            for x in 0..w {
                row += u64::from(coverage[y * w + x]);
                sums[(y + 1) * (w + 1) + x + 1] = sums[y * (w + 1) + x + 1] + row;
            }
        }
        let area = |x0: i32, y0: i32, x1: i32, y1: i32| -> u64 {
            let cx = |x: i32| x.clamp(0, w as i32) as usize;
            let cy = |y: i32| y.clamp(0, h as i32) as usize;
            let (x0, x1, y0, y1) = (cx(x0), cx(x1), cy(y0), cy(y1));
            if x0 >= x1 || y0 >= y1 {
                return 0;
            }
            let at = |x: usize, y: usize| sums[y * (w + 1) + x];
            at(x1, y1) + at(x0, y0) - at(x0, y1) - at(x1, y0)
        };
        // A pixel p lands its block on [p + o - n / 2, p + o - n / 2 + n).
        let lo = |x: i32, o: i32| x - o + n / 2 - n + 1;
        let (num, den) = (u64::from(gain.0), u64::from(gain.1) * (n * n) as u64);
        let mut out = vec![0u8; w * h];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let (x0, y0) = (lo(x, ox), lo(y, oy));
                let sum = area(x0, y0, x0 + n, y0 + n);
                out[y as usize * w + x as usize] = (sum * num / den).min(255) as u8;
            }
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextSpec {
    pub family: Vec<String>,
    pub size_px: u32,
    pub family_sizes: Vec<(String, u32)>,
    pub bold: bool,
    pub italic: bool,
    pub antialias: bool,
    pub full_hinting: bool,
    pub color: Color,
    pub inactive: Color,
    pub shadow: Option<Shadow>,
    /// Exponent on each pixel's coverage (0 to 1) before inking; under 1 sets
    /// the text heavier.
    pub ink_gamma: f32,
}

impl TextSpec {
    pub fn font(&self) -> FontConfig {
        FontConfig {
            family: self.family.clone(),
            size_px: self.size_px,
            antialias: self.antialias,
            italic: self.italic,
            full_hinting: self.full_hinting,
            shell_dlg: None,
        }
    }

    /// The size to draw this spec at in `text`, a renderer loaded from
    /// [`TextSpec::font`]: the `family_sizes` entry for the face it
    /// loaded, else `size_px`.
    pub fn size_in(&self, text: &TextRenderer) -> u32 {
        self.family_sizes
            .iter()
            .find(|(family, _)| text.has_face(family))
            .map_or(self.size_px, |&(_, px)| px)
    }

    /// This spec at `pct` percent of its size, `family_sizes` too
    /// ([`crate::theme::text_px`]).
    pub fn with_text_size(&self, pct: u32) -> TextSpec {
        let px = |n| crate::theme::text_px(n, pct);
        TextSpec {
            size_px: px(self.size_px),
            family_sizes: self
                .family_sizes
                .iter()
                .map(|(f, n)| (f.clone(), px(*n)))
                .collect(),
            ..self.clone()
        }
    }

    pub fn weigh(&self, coverage: &mut [u8]) {
        if self.ink_gamma == 1.0 {
            return;
        }
        let table: Vec<u8> = (0..=255u8)
            .map(|c| (255.0 * (f32::from(c) / 255.0).powf(self.ink_gamma)).round() as u8)
            .collect();
        for c in coverage {
            *c = table[*c as usize];
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CaptionTitle {
    pub icon: [(i32, i32); 2],
    pub text_x: i32,
    pub baseline: Option<[i32; 2]>,
    pub glow: Option<TitleGlow>,
}

impl CaptionTitle {
    pub const CENTRED: CaptionTitle = CaptionTitle {
        icon: [(2, 4), (2, 4)],
        text_x: 22,
        baseline: None,
        glow: None,
    };
}

#[derive(Clone, Copy, Debug)]
pub struct TitleGlow {
    /// Device pixels, given device pixels per design pixel.
    pub reach: fn(i32) -> i32,
    pub color: Color,
    /// At scale 1: rows below the band's top it starts, rows above the client's
    /// top it ends; restored then maximized.
    pub rows: [(i32, i32); 2],
    pub alpha: fn(&GlowMask) -> Arc<[u8]>,
}

pub struct GlowMask<'a> {
    pub coverage: &'a [u8],
    pub width: usize,
    pub height: usize,
    pub rows: (usize, usize),
    pub active: bool,
    pub scale: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct TaskGlow {
    pub colour: fn(&Canvas) -> Option<Color>,
    pub label_edges: Option<GlowingPair>,
}

pub type GlowingPair = fn(&PartImage, State, usize, Color) -> PartImage;

#[derive(Clone, Debug)]
pub struct TaskSpot {
    pub fit: SpotFit,
    pub button: SpotFrames,
    pub labelled: Option<SpotFrames>,
    pub edges: Option<SpotFrames>,
}

#[derive(Clone, Debug)]
pub struct SpotFrames {
    pub base: PartImage,
    /// Laid out like `base`; each pixel's gain in 32nds in its low byte.
    pub gain: PartImage,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotFit {
    pub dx: f32,
    pub halo: SpotLayer,
    pub core: SpotLayer,
}

impl SpotFit {
    pub fn in_colour(&self, glow: Option<Color>) -> SpotLight {
        SpotLight {
            layers: [self.halo, self.core].map(|l| (l, l.colour(glow))),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    layers: [(SpotLayer, [f32; 3]); 2],
}

impl SpotLight {
    pub fn light(&self, under: u32, g: f32, x: f32, y: f32, w: f32, h: f32) -> u32 {
        if g == 0.0 {
            return under;
        }
        let (mut keep, mut add) = (1.0f32, [0.0f32; 3]);
        for (layer, colour) in &self.layers {
            let a = 1.0 - (1.0 - layer.alpha(x, y, w, h)).powf(g);
            keep *= 1.0 - a;
            for (v, c) in add.iter_mut().zip(colour) {
                *v = *v * (1.0 - a) + a * c;
            }
        }
        let ch = |sh: u32, add: f32| {
            let v = ((under >> sh) & 0xFF) as f32 * keep + add;
            (v.round().clamp(0.0, 255.0) as u32) << sh
        };
        ch(24, 255.0 * (1.0 - keep)) | ch(16, add[0]) | ch(8, add[1]) | ch(0, add[2])
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotLayer {
    pub peak: f32,
    pub rx: f32,
    pub ry: f32,
    pub power: f32,
    pub foot: f32,
    pub white: [u8; 3],
    pub tint: f32,
}

impl SpotLayer {
    pub fn alpha(&self, x: f32, y: f32, w: f32, h: f32) -> f32 {
        let d = (x / (self.rx * w)).powi(2) + ((y - self.foot * h) / (self.ry * h)).powi(2);
        (self.peak / (1.0 + d.powf(self.power))).min(1.0)
    }

    pub fn colour(&self, glow: Option<Color>) -> [f32; 3] {
        let white = self.white.map(f32::from);
        match glow.map(glow_chroma).filter(|c| *c != [0.0; 3]) {
            Some(c) => [0, 1, 2].map(|i| (white[i] + self.tint * c[i]).clamp(0.0, 255.0)),
            None => [(0.299 * white[0] + 0.587 * white[1] + 0.114 * white[2]).round(); 3],
        }
    }
}

pub fn glow_chroma(c: Color) -> [f32; 3] {
    let v = [c.r, c.g, c.b].map(f32::from);
    let hi = v[0].max(v[1]).max(v[2]);
    let lo = v[0].min(v[1]).min(v[2]);
    if hi == lo {
        return [0.0; 3];
    }
    let full = v.map(|x| (x - lo) * 255.0 / (hi - lo));
    let mean = full.iter().sum::<f32>() / 3.0;
    full.map(|x| x - mean)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlassSpec {
    /// Pixels outside the frame per row, outer row first; all four corners
    /// mirror it.
    pub corner: &'static [i32],
    pub panel: PanelSpec,
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
    pub ring: Color,
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

#[derive(Clone)]
pub struct Style {
    pub name: &'static str,
    pub colors: ColorScheme,
    pub metrics: Metrics,
    pub caption_text: TextSpec,
    pub start_text: TextSpec,
    pub bar_text: Color,
    pub taskbar: TaskbarShape,
    pub glass: Option<GlassSpec>,
    pub caption_title: CaptionTitle,
    pub task_glow: Option<TaskGlow>,
    pub task_spot: Option<TaskSpot>,
    pub task_nudge: bool,
    pub greyed_field_frame: bool,
    pub default_needs_active: bool,
    pub combo_edit_border: bool,
    parts: HashMap<Part, Art>,
    pub controls: ControlMetrics,
    text_colors: HashMap<(Part, State), Color>,
    pub group_label: GroupLabel,
    pub smooth_progress: bool,
    pub track_marks: TrackMarks,
    /// The ? box's width at the help frames' left, its divider included.
    pub joined_help: Option<i32>,
    pub flat_menus: bool,
    pub buttons_over_caption: bool,
    /// Whether the caption title is centred across the frame, moved left
    /// as far as it must to end before the first caption button but never
    /// left of [`CaptionTitle::text_x`]. Unset, it starts at `text_x`.
    pub centred_title: bool,
    pub page: Option<PageInks>,
    pub drop_list: Option<DropList>,
    pub balloon: Option<BalloonLook>,
    pub menu: Option<MenuLook>,
    pub selection_box: Option<SelectionBox>,
    pub flyout: Option<FlyoutLook>,
    pub caption_tabs: Option<CaptionTabLook>,
    pub(crate) tray_arrow: Option<Arc<trayarrow::TrayArrowArt>>,
    pub(crate) start_bloom: Option<Arc<startbloom::StartBloom>>,
    pub chrome_colors: Option<ColorScheme>,
    rescale: Option<Rescale>,
    rescaled: Rescaled,
}

#[derive(Default)]
struct Rescaled(Mutex<Vec<(i32, Arc<Style>)>>);

impl Clone for Rescaled {
    fn clone(&self) -> Self {
        Rescaled::default()
    }
}

type Rescale = Arc<dyn Fn(i32) -> Style + Send + Sync>;

/// A part's art, painted the first time it is asked for; clones share it.
type Art = Arc<LazyLock<PartImage, Box<dyn FnOnce() -> PartImage + Send>>>;

impl Style {
    pub fn new(
        name: &'static str,
        colors: ColorScheme,
        metrics: Metrics,
        caption_text: TextSpec,
        start_text: TextSpec,
        bar_text: Color,
        taskbar: TaskbarShape,
    ) -> Style {
        Style {
            name,
            colors,
            metrics,
            caption_text,
            start_text,
            bar_text,
            taskbar,
            glass: None,
            caption_title: CaptionTitle::CENTRED,
            task_glow: None,
            task_spot: None,
            task_nudge: true,
            greyed_field_frame: false,
            default_needs_active: false,
            combo_edit_border: false,
            parts: HashMap::new(),
            controls: ControlMetrics::CLASSIC,
            text_colors: HashMap::new(),
            group_label: GroupLabel::LINE,
            smooth_progress: false,
            track_marks: TrackMarks::LOW,
            joined_help: None,
            flat_menus: false,
            buttons_over_caption: false,
            centred_title: false,
            page: None,
            drop_list: None,
            menu: None,
            selection_box: None,
            caption_tabs: None,
            tray_arrow: None,
            start_bloom: None,
            chrome_colors: None,
            balloon: None,
            flyout: None,
            rescale: None,
            rescaled: Rescaled::default(),
        }
    }

    pub fn with_rescale(mut self, build: impl Fn(i32) -> Style + Send + Sync + 'static) -> Style {
        self.rescale = Some(Arc::new(build));
        self
    }

    pub fn at_scale(self: &Arc<Style>, n: i32) -> Arc<Style> {
        let Some(build) = self.rescale.as_ref().filter(|_| n > 1) else {
            return self.clone();
        };
        let mut kept = self.rescaled.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, st)) = kept.iter().find(|(k, _)| *k == n) {
            return st.clone();
        }
        let st = Arc::new(build(n));
        kept.push((n, st.clone()));
        st
    }

    pub fn with_text_size(self: &Arc<Style>, pct: u32) -> Arc<Style> {
        if pct == 100 {
            return self.clone();
        }
        let mut st = Style::clone(self);
        st.caption_text = st.caption_text.with_text_size(pct);
        st.rescale = self.rescale.clone().map(|build| -> Rescale {
            Arc::new(move |n| {
                let mut st = build(n);
                st.caption_text = st.caption_text.with_text_size(pct);
                st
            })
        });
        Arc::new(st)
    }

    pub fn with_glass(mut self, g: GlassSpec) -> Style {
        self.glass = Some(g);
        self
    }

    pub fn with_caption_title(mut self, t: CaptionTitle) -> Style {
        self.caption_title = t;
        self
    }

    pub fn with_task_glow(mut self, g: Option<TaskGlow>) -> Style {
        self.task_glow = g;
        self
    }

    pub fn with_task_spot(mut self, t: Option<TaskSpot>) -> Style {
        self.task_spot = t;
        self
    }

    pub fn without_task_nudge(mut self) -> Style {
        self.task_nudge = false;
        self
    }

    pub fn with_greyed_field_frame(mut self) -> Style {
        self.greyed_field_frame = true;
        self
    }

    pub fn with_combo_edit_border(mut self) -> Style {
        self.combo_edit_border = true;
        self
    }

    pub fn with_smooth_progress(mut self) -> Style {
        self.smooth_progress = true;
        self
    }

    pub fn with_track_marks(mut self, m: TrackMarks) -> Style {
        self.track_marks = m;
        self
    }

    pub fn with_page(mut self, inks: PageInks) -> Style {
        self.page = Some(inks);
        self
    }

    pub fn with_drop_list(mut self, d: DropList) -> Style {
        self.drop_list = Some(d);
        self
    }

    pub fn with_menu(mut self, m: MenuLook) -> Style {
        self.menu = Some(m);
        self
    }

    pub fn with_caption_tabs(mut self, t: CaptionTabLook) -> Style {
        self.caption_tabs = Some(t);
        self
    }

    pub(crate) fn with_tray_arrow(mut self, art: trayarrow::TrayArrowArt) -> Style {
        self.tray_arrow = Some(Arc::new(art));
        self
    }

    pub(crate) fn with_start_bloom(mut self, paint: startbloom::BloomPainter) -> Style {
        self.start_bloom = Some(Arc::new(startbloom::StartBloom::new(paint)));
        self
    }

    pub fn with_selection_box(mut self, b: SelectionBox) -> Style {
        self.selection_box = Some(b);
        self
    }

    pub fn with_balloon(mut self, b: BalloonLook) -> Style {
        self.balloon = Some(b);
        self
    }

    pub fn with_flyout(mut self, f: FlyoutLook) -> Style {
        self.flyout = Some(f);
        self
    }

    pub fn with_joined_help(mut self, cell: i32) -> Style {
        self.joined_help = Some(cell);
        self
    }

    pub fn with_chrome_colors(mut self, cs: ColorScheme) -> Style {
        self.chrome_colors = Some(cs);
        self
    }

    pub fn with_flat_menus(mut self) -> Style {
        self.flat_menus = true;
        self
    }

    pub fn with_default_needing_active(mut self) -> Style {
        self.default_needs_active = true;
        self
    }

    pub fn with_part(self, part: Part, img: PartImage) -> Style {
        self.with_painted(part, move || img)
    }

    /// `part` drawn by `paint`, which runs the first time the part is asked for:
    /// a program draws only a few of the parts its style holds.
    pub fn with_painted(
        mut self,
        part: Part,
        paint: impl FnOnce() -> PartImage + Send + 'static,
    ) -> Style {
        self.parts
            .insert(part, Arc::new(LazyLock::new(Box::new(paint))));
        self
    }

    pub fn without_part(mut self, part: Part) -> Style {
        self.parts.remove(&part);
        self
    }

    pub fn part(&self, part: Part) -> Option<&PartImage> {
        self.parts.get(&part).map(|art| LazyLock::force(art))
    }

    /// Every part this style draws, in no particular order.
    pub fn part_keys(&self) -> impl Iterator<Item = Part> + '_ {
        self.parts.keys().copied()
    }

    /// A drawer painting several parts asks for all of them, so a style short
    /// of one falls back whole.
    pub fn has_parts(&self, parts: &[Part]) -> bool {
        parts.iter().all(|p| self.parts.contains_key(p))
    }

    pub fn with_controls(mut self, cm: ControlMetrics) -> Style {
        self.controls = cm;
        self
    }

    pub fn with_group_label(mut self, g: GroupLabel) -> Style {
        self.group_label = g;
        self
    }

    pub fn with_text_color(mut self, part: Part, state: State, c: Color) -> Style {
        self.text_colors.insert((part, state), c);
        self
    }

    pub fn text_color(&self, part: Part, state: State) -> Option<Color> {
        self.text_colors.get(&(part, state)).copied()
    }

    pub fn draw_check(
        &self,
        c: &mut Canvas,
        r: Rect,
        part: Part,
        check: Check,
        state: State,
        s: i32,
    ) {
        let Some(p) = self.part(part) else { return };
        p.blit(c, p.check_frame(check, state), r, s);
    }

    /// `s` is device pixels per design pixel in `r`: a program on a scaled
    /// canvas passes 1, the compositor on a scale-1 canvas the output's scale.
    pub fn draw(&self, c: &mut Canvas, r: Rect, part: Part, state: State, s: i32) {
        self.draw_nth(c, r, part, state, 0, s);
    }

    pub fn draw_nth(&self, c: &mut Canvas, r: Rect, part: Part, state: State, n: usize, s: i32) {
        let Some(p) = self.part(part) else { return };
        p.blit(c, p.frame_nth(state, n), r, s);
    }

    pub fn draw_mixed(
        &self,
        c: &mut Canvas,
        r: Rect,
        part: Part,
        rest: (State, usize),
        lit: &[(State, usize, f32)],
        s: i32,
    ) {
        let on: Vec<(State, usize, f32)> = lit.iter().copied().filter(|l| l.2 > 0.0).collect();
        match on[..] {
            [] => return self.draw_nth(c, r, part, rest.0, rest.1, s),
            [(st, n, l)] if l >= 1.0 => return self.draw_nth(c, r, part, st, n, s),
            _ => {}
        }
        let local = Rect::new(0, 0, r.width, r.height);
        let under = c.copy_rect(r);
        let frame = |st: State, n: usize| {
            let mut f = under.clone();
            self.draw_nth(&mut f, local, part, st, n, s);
            f
        };
        let mut mixed = frame(rest.0, rest.1);
        let lits: Vec<(Canvas, f32)> = on.iter().map(|&(st, n, l)| (frame(st, n), l)).collect();
        let lits: Vec<(&Canvas, f32)> = lits.iter().map(|(f, l)| (f, *l)).collect();
        mixed.mix_all(&lits, local);
        c.clear_rect(r);
        c.blit_over(r.x, r.y, &mixed);
    }

    pub fn draw_page(&self, c: &mut Canvas, page: Rect, s: i32) -> bool {
        if self.part(Part::TabPane).is_none() {
            return false;
        }
        self.draw(c, page, Part::TabPane, State::Normal, s);
        true
    }

    pub fn content_rect(&self, part: Part, r: Rect, s: i32) -> Rect {
        let m = self
            .part(part)
            .map_or(Insets::ZERO, |p| p.content)
            .scaled(s);
        Rect::new(
            r.x + m.left,
            r.y + m.top,
            (r.width - m.left - m.right).max(0),
            (r.height - m.top - m.bottom).max(0),
        )
    }

    pub fn opaque_at(&self, part: Part, state: State, r: Rect, s: i32, x: i32, y: i32) -> bool {
        let Some(p) = self.part(part) else {
            return false;
        };
        let s = s.max(1);
        let k = p.per_atlas(s);
        p.nines(p.frame(state), r, s)
            .into_iter()
            .flatten()
            .find_map(|(from, to, m)| nine_source(from, to, m, p.fill, p.border_only, k, x, y))
            .is_some_and(|(sx, sy)| p.atlas.get(sx, sy) >> 24 != 0)
    }
}

/// `s` for a draw on `c`, at least 1. A scaled canvas already multiplies
/// every rect, so a draw on one at `s > 1` would come out `s` times too big.
fn scale_for(c: &Canvas, s: i32) -> i32 {
    debug_assert!(
        c.scale() == 1 || s <= 1,
        "a style draw at scale {s} on a canvas at scale {}",
        c.scale()
    );
    s.max(1)
}

impl std::fmt::Debug for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Style")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::FontConfig;

    fn spec() -> TextSpec {
        TextSpec {
            family: FontConfig::default().family,
            size_px: 11,
            family_sizes: Vec::new(),
            bold: false,
            italic: false,
            antialias: false,
            full_hinting: false,
            color: Color::rgb(255, 255, 255),
            inactive: Color::rgb(200, 200, 200),
            shadow: None,
            ink_gamma: 1.0,
        }
    }

    fn soft_shadow() -> Shadow {
        Shadow {
            color: Color::rgb(0, 0, 0),
            dx: 1,
            dy: 1,
            soft: Some(SoftShadow {
                size: 4,
                gain: (3, 2),
            }),
        }
    }

    fn block(alpha: &[u8], w: usize, value: u8) -> (usize, usize, usize, usize) {
        let lit: Vec<(usize, usize)> = (0..alpha.len())
            .filter(|&i| alpha[i] != 0)
            .map(|i| (i % w, i / w))
            .collect();
        assert!(
            lit.iter().all(|&(x, y)| alpha[y * w + x] == value),
            "every lit pixel is {value}"
        );
        let xs = lit.iter().map(|p| p.0);
        let ys = lit.iter().map(|p| p.1);
        let (x0, x1) = (xs.clone().min().unwrap(), xs.max().unwrap());
        let (y0, y1) = (ys.clone().min().unwrap(), ys.max().unwrap());
        assert_eq!(lit.len(), (x1 - x0 + 1) * (y1 - y0 + 1), "a full block");
        (x0, x1, y0, y1)
    }

    #[test]
    fn a_soft_shadow_spreads_an_inked_pixel_from_1_px_up_left_to_2_down_right() {
        let mut mask = vec![0u8; 12 * 12];
        mask[4 * 12 + 4] = 255;
        let alpha = soft_shadow().alpha(&mask, 12, 12, 1);
        // 255 over 16 pixels at 3/2: 23 each.
        assert_eq!(block(&alpha, 12, 23), (3, 6, 3, 6));
    }

    #[test]
    fn a_soft_shadow_caps_at_full_under_solid_ink() {
        let mask = vec![255u8; 8 * 8];
        let alpha = soft_shadow().alpha(&mask, 8, 8, 1);
        assert_eq!(alpha[4 * 8 + 4], 255, "3/2 of full coverage stays full");
    }

    #[test]
    fn a_soft_shadow_at_2_device_pixels_a_pixel_is_twice_as_far_and_wide() {
        let mut mask = vec![0u8; 20 * 20];
        mask[8 * 20 + 8] = 255;
        let alpha = soft_shadow().alpha(&mask, 20, 20, 2);
        // An 8x8 block from 2 px up and left: 255 * 3 / (2 * 64) = 5.
        assert_eq!(block(&alpha, 20, 5), (6, 13, 6, 13));
    }

    #[test]
    fn a_hard_shadow_is_the_coverage_moved_by_its_offset() {
        let mut mask = vec![0u8; 6 * 6];
        mask[2 * 6 + 1] = 200;
        let sh = Shadow {
            soft: None,
            ..soft_shadow()
        };
        let alpha = sh.alpha(&mask, 6, 6, 1);
        assert_eq!(block(&alpha, 6, 200), (2, 2, 3, 3));
        let alpha = sh.alpha(&mask, 6, 6, 2);
        assert_eq!(block(&alpha, 6, 200), (3, 3, 4, 4), "moved 2 px at d 2");
    }

    #[test]
    fn an_ink_gamma_of_one_half_takes_the_square_root_of_the_coverage() {
        let mut c = [0, 64, 128, 255];
        TextSpec {
            ink_gamma: 0.5,
            ..spec()
        }
        .weigh(&mut c);
        assert_eq!(c, [0, 128, 181, 255]);
        let mut c = [0, 64, 128, 255];
        spec().weigh(&mut c);
        assert_eq!(c, [0, 64, 128, 255], "a gamma of 1 inks as rasterized");
    }

    fn style() -> Style {
        let mut atlas = Canvas::new(3, 4);
        atlas.fill_rect(Rect::new(0, 0, 3, 2), Color::rgb(255, 0, 0));
        atlas.fill_rect(Rect::new(0, 2, 3, 2), Color::rgb(0, 255, 0));
        atlas.clear_rect(Rect::new(0, 0, 1, 1));
        atlas.clear_rect(Rect::new(0, 2, 1, 1));
        Style::new(
            "Test",
            ColorScheme::default(),
            Metrics::default(),
            spec(),
            spec(),
            Color::rgb(0, 0, 0),
            crate::taskbar::TaskbarShape::CLASSIC,
        )
        .with_part(
            Part::Caption,
            PartImage::new(atlas, &[State::Normal, State::Hot], Layout::Vertical)
                .sizing(Insets::new(1, 1, 1, 0))
                .content(Insets::new(2, 0, 2, 0)),
        )
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "does not divide evenly")]
    fn a_mismatched_frame_count_panics_in_debug() {
        let atlas = Canvas::new(3, 5);
        PartImage::new(atlas, &[State::Normal, State::Hot], Layout::Vertical);
    }

    #[test]
    fn a_state_picks_its_frame_and_a_missing_one_falls_back_to_normal() {
        let s = style();
        let p = s.part(Part::Caption).unwrap();
        assert_eq!(p.frame(State::Normal), Rect::new(0, 0, 3, 2));
        assert_eq!(p.frame(State::Hot), Rect::new(0, 2, 3, 2));
        assert_eq!(p.frame(State::Pressed), Rect::new(0, 0, 3, 2));
    }

    #[test]
    fn a_painted_part_paints_once_on_first_ask_and_clones_share_it() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static PAINTS: AtomicUsize = AtomicUsize::new(0);
        let s = style().with_painted(Part::Taskbar, || {
            PAINTS.fetch_add(1, Ordering::Relaxed);
            PartImage::new(Canvas::new(2, 2), &[State::Normal], Layout::Vertical)
        });
        let copy = s.clone();
        assert!(s.has_parts(&[Part::Taskbar]));
        assert_eq!(PAINTS.load(Ordering::Relaxed), 0);
        assert_eq!(s.part(Part::Taskbar).unwrap().atlas.width, 2);
        assert_eq!(copy.part(Part::Taskbar).unwrap().atlas.width, 2);
        assert_eq!(PAINTS.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_repeated_state_keeps_one_frame_per_button() {
        let atlas = Canvas::new(2, 8);
        let p = PartImage::new(
            atlas,
            &[State::Normal, State::Hot, State::Hot, State::Hot],
            Layout::Vertical,
        );
        assert_eq!(p.frame_nth(State::Hot, 0), Rect::new(0, 2, 2, 2), "first");
        assert_eq!(p.frame_nth(State::Hot, 2), Rect::new(0, 6, 2, 2), "third");
        assert_eq!(
            p.frame_nth(State::Hot, 3),
            Rect::new(0, 2, 2, 2),
            "past the last: the state's first"
        );
        assert_eq!(
            p.frame_nth(State::Normal, 1),
            Rect::new(0, 0, 2, 2),
            "a state listed once"
        );
        assert_eq!(
            p.frame_nth(State::Pressed, 1),
            Rect::new(0, 0, 2, 2),
            "a missing state: the atlas's first"
        );
    }

    #[test]
    fn draw_nine_grids_the_frame_into_the_rect() {
        let s = style();
        let mut c = Canvas::new(10, 3);
        s.draw(&mut c, Rect::new(0, 0, 10, 3), Part::Caption, State::Hot, 1);
        assert_eq!(c.get(0, 0), 0);
        assert_eq!(c.get(5, 1), 0xFF00_FF00);
        s.draw(
            &mut c,
            Rect::new(0, 0, 10, 3),
            Part::Taskbar,
            State::Normal,
            1,
        );
        assert_eq!(c.get(0, 0), 0);
    }

    fn check_part(checks: &[Check]) -> PartImage {
        let n = checks.len() * CHECK_STATES.len();
        let mut atlas = Canvas::new(2, 2 * n as u32);
        for i in 0..n {
            atlas.fill_rect(Rect::new(0, 2 * i as i32, 2, 2), Color::rgb(i as u8, 0, 0));
        }
        PartImage::new_check(atlas, checks, Layout::Vertical)
    }

    fn nth(i: i32) -> Rect {
        Rect::new(0, 2 * i, 2, 2)
    }

    #[test]
    fn check_frames_run_check_major_then_normal_hot_pressed_disabled() {
        let p = check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]);
        let mut i = 0;
        for check in [Check::Unchecked, Check::Checked, Check::Mixed] {
            for state in [State::Normal, State::Hot, State::Pressed, State::Disabled] {
                assert_eq!(
                    p.check_frame(check, state),
                    nth(i),
                    "{check:?} {state:?} is frame {i} of 12"
                );
                i += 1;
            }
        }
    }

    #[test]
    fn a_missing_check_state_falls_back_to_normal_of_the_same_check() {
        let p = check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]);
        assert_eq!(
            p.check_frame(Check::Mixed, State::Inactive),
            nth(8),
            "Mixed Inactive falls back to Mixed Normal, frame 8"
        );
    }

    #[test]
    fn a_radio_has_eight_frames_and_mixed_falls_back_to_unchecked_normal() {
        let p = check_part(&[Check::Unchecked, Check::Checked]);
        assert_eq!(
            p.check_frame(Check::Checked, State::Disabled),
            nth(7),
            "Checked Disabled is the last of 8 frames"
        );
        assert_eq!(
            p.check_frame(Check::Mixed, State::Hot),
            nth(0),
            "a radio's Mixed falls back to Unchecked Normal, frame 0"
        );
    }

    #[test]
    fn draw_check_paints_the_frame_for_the_check_and_state() {
        let s = style().with_part(
            Part::CheckBox,
            check_part(&[Check::Unchecked, Check::Checked, Check::Mixed]),
        );
        let mut c = Canvas::new(4, 4);
        s.draw_check(
            &mut c,
            Rect::new(0, 0, 4, 4),
            Part::CheckBox,
            Check::Checked,
            State::Hot,
            1,
        );
        assert_eq!(
            c.get(1, 1),
            Color::rgb(5, 0, 0).argb(),
            "Checked Hot paints frame 5"
        );
    }

    #[test]
    fn content_rect_insets_by_the_content_margins() {
        assert_eq!(
            style().content_rect(Part::Caption, Rect::new(0, 0, 10, 3), 1),
            Rect::new(2, 0, 6, 3)
        );
    }

    #[test]
    fn clipped_ends_keep_their_pixels_in_a_narrow_rect() {
        let mut atlas = Canvas::new(5, 1);
        for x in 0..5 {
            atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
        }
        let s = style().with_part(
            Part::Taskbar,
            PartImage::new(atlas, &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 0, 2, 0))
                .clip_ends(),
        );
        let row = |w: i32| {
            let mut c = Canvas::new(w as u32, 1);
            s.draw(
                &mut c,
                Rect::new(0, 0, w, 1),
                Part::Taskbar,
                State::Normal,
                1,
            );
            (0..w)
                .map(|x| (c.get(x, 0) >> 16) & 0xFF)
                .collect::<Vec<u32>>()
        };
        assert_eq!(row(6), vec![10, 20, 30, 30, 40, 50], "wide: a nine-grid");
        assert_eq!(row(3), vec![10, 40, 50], "the left end, then the right");
        assert_eq!(row(1), vec![50], "only the right end's last column");
        assert!(s.opaque_at(Part::Taskbar, State::Normal, Rect::new(0, 0, 3, 1), 1, 1, 0));
    }

    #[test]
    fn split_ends_share_a_narrow_rect() {
        let mut atlas = Canvas::new(5, 1);
        for x in 0..5 {
            atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
        }
        let s = style().with_part(
            Part::Taskbar,
            PartImage::new(atlas, &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 0, 2, 0))
                .split_ends(),
        );
        let row = |w: i32| {
            let mut c = Canvas::new(w as u32, 1);
            s.draw(
                &mut c,
                Rect::new(0, 0, w, 1),
                Part::Taskbar,
                State::Normal,
                1,
            );
            (0..w)
                .map(|x| (c.get(x, 0) >> 16) & 0xFF)
                .collect::<Vec<u32>>()
        };
        assert_eq!(row(5), vec![10, 20, 30, 40, 50], "wide: a nine-grid");
        assert_eq!(
            row(3),
            vec![10, 20, 50],
            "the left end's first 2, the right's last 1"
        );
        assert_eq!(row(2), vec![10, 50], "one column of each");
    }

    #[test]
    fn opaque_at_follows_the_atlas_alpha() {
        let s = style();
        let r = Rect::new(10, 10, 10, 3);
        assert!(!s.opaque_at(Part::Caption, State::Normal, r, 1, 10, 10));
        assert!(s.opaque_at(Part::Caption, State::Normal, r, 1, 11, 10));
        assert!(!s.opaque_at(Part::Caption, State::Normal, r, 1, 30, 10));
    }

    fn ramp_atlas(frames: i32) -> Canvas {
        let mut a = Canvas::new(6, 5 * frames as u32);
        for y in 0..5 * frames {
            for x in 0..6 {
                a.fill_rect(
                    Rect::new(x, y, 1, 1),
                    Color::rgb(40 * x as u8 + 1, 12 * y as u8 + 1, 7),
                );
            }
        }
        for f in 0..frames {
            a.clear_rect(Rect::new(0, 5 * f, 1, 1));
            a.clear_rect(Rect::new(5, 5 * f + 4, 1, 1));
        }
        a
    }

    #[test]
    fn a_part_painted_at_2_lands_pixel_for_pixel_on_a_scale_2_canvas() {
        let mut atlas = Canvas::new(8, 6);
        for (i, px) in atlas.pixels.iter_mut().enumerate() {
            *px = 0xFF00_0000 | (i as u32 * 0x0005_0309);
        }
        atlas.clear_rect(Rect::new(0, 0, 1, 1));
        let s = with_taskbar(
            PartImage::new(atlas, &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(1, 1, 1, 1))
                .at_scale(2),
        );
        let r = Rect::new(1, 1, 7, 5);
        for clip in [None, Some(Rect::new(2, 2, 4, 3))] {
            let mut program = Canvas::with_scale(10, 8, 2);
            let mut compositor = Canvas::new(20, 16);
            let dev = r.scaled(2);
            match clip {
                Some(cl) => compositor.with_clip(cl.scaled(2), |c| {
                    s.draw(c, dev, Part::Taskbar, State::Normal, 2)
                }),
                None => s.draw(&mut compositor, dev, Part::Taskbar, State::Normal, 2),
            }
            match clip {
                Some(cl) => {
                    program.with_clip(cl, |c| s.draw(c, r, Part::Taskbar, State::Normal, 1))
                }
                None => s.draw(&mut program, r, Part::Taskbar, State::Normal, 1),
            }
            assert!(
                compositor.pixels.iter().any(|&p| p != 0),
                "{clip:?}: the draw paints something"
            );
            assert!(
                program.pixels == compositor.pixels,
                "{clip:?}: the program's device pixels are the compositor's"
            );
            assert_eq!(program.scale(), 2, "the canvas keeps its scale");
        }
    }

    fn with_taskbar(img: PartImage) -> Style {
        style().with_part(Part::Taskbar, img)
    }

    fn assert_doubles(what: &str, w: u32, h: u32, draw: impl Fn(&mut Canvas, i32)) {
        let mut one = Canvas::new(w, h);
        draw(&mut one, 1);
        let mut two = Canvas::new(2 * w, 2 * h);
        draw(&mut two, 2);
        assert!(
            (0..h as i32).any(|y| (0..w as i32).any(|x| one.get(x, y) != 0)),
            "{what}: the s = 1 draw paints something"
        );
        assert!(
            crate::look::styled::fixture::doubles_at_scale_1(&one, &two),
            "{what}: the s = 2 draw is the s = 1 draw with every pixel a 2x2 block"
        );
    }

    #[test]
    fn a_stretched_grid_at_scale_2_doubles_the_scale_1_grid() {
        let s = style();
        let r = Rect::new(1, 1, 11, 3);
        assert_doubles("Stretch caption", 14, 5, |c, k| {
            s.draw(c, r.scaled(k), Part::Caption, State::Hot, k)
        });
        let s = with_taskbar(
            PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 1, 1, 2)),
        );
        for r in [Rect::new(1, 2, 13, 9), Rect::new(0, 0, 4, 3)] {
            assert_doubles("Stretch ramp", 15, 12, |c, k| {
                s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
            });
        }
    }

    #[test]
    fn a_tiled_and_a_centred_grid_at_scale_2_double_the_scale_1_grid() {
        for fill in [Fill::Tile, Fill::StretchCentred] {
            let s = with_taskbar(
                PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
                    .sizing(Insets::new(2, 1, 1, 2))
                    .fill(fill),
            );
            let r = Rect::new(1, 2, 13, 9);
            assert_doubles(&format!("{fill:?}"), 15, 12, |c, k| {
                s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
            });
        }
    }

    #[test]
    fn a_border_only_grid_at_scale_2_doubles_the_scale_1_grid() {
        let mut img = PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
            .sizing(Insets::new(2, 1, 1, 2));
        img.border_only = true;
        let s = with_taskbar(img);
        let r = Rect::new(1, 2, 13, 9);
        assert_doubles("border only", 15, 12, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }

    #[test]
    fn an_outset_at_scale_2_reaches_twice_as_far() {
        let s = with_taskbar(
            PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 1, 1, 2))
                .outset(Insets::new(1, 2, 0, 1)),
        );
        let r = Rect::new(2, 3, 10, 6);
        assert_doubles("outset", 15, 12, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }

    #[test]
    fn a_true_size_frame_at_scale_2_is_twice_its_size() {
        let mut img = PartImage::new(ramp_atlas(1), &[State::Normal], Layout::Vertical);
        img.true_size = true;
        let s = with_taskbar(img);
        let r = Rect::new(1, 1, 10, 9);
        assert_doubles("true size", 12, 11, |c, k| {
            s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
        });
    }

    #[test]
    fn a_check_at_scale_2_doubles_the_checked_hot_frame() {
        let atlas = {
            let mut a = Canvas::new(6, 5 * 12);
            for f in 0..12 {
                let one = ramp_atlas(1);
                for y in 0..5 {
                    for x in 0..6 {
                        a.put_raw(x, 5 * f + y, one.get(x, y) ^ (f as u32) << 8);
                    }
                }
            }
            a
        };
        let s = style().with_part(
            Part::CheckBox,
            PartImage::new_check(
                atlas,
                &[Check::Unchecked, Check::Checked, Check::Mixed],
                Layout::Vertical,
            )
            .sizing(Insets::new(2, 1, 1, 2)),
        );
        let r = Rect::new(1, 1, 9, 7);
        assert_doubles("Checked Hot", 11, 9, |c, k| {
            s.draw_check(
                c,
                r.scaled(k),
                Part::CheckBox,
                Check::Checked,
                State::Hot,
                k,
            )
        });
    }

    #[test]
    fn the_nth_frame_at_scale_2_doubles_the_scale_1_frame() {
        let s = with_taskbar(
            PartImage::new(
                ramp_atlas(3),
                &[State::Normal, State::Hot, State::Hot],
                Layout::Vertical,
            )
            .sizing(Insets::new(2, 1, 1, 2)),
        );
        let r = Rect::new(1, 2, 13, 9);
        assert_doubles("second Hot frame", 15, 12, |c, k| {
            s.draw_nth(c, r.scaled(k), Part::Taskbar, State::Hot, 1, k)
        });
    }

    #[test]
    fn clipped_ends_at_scale_2_double_the_scale_1_ends() {
        let mut atlas = Canvas::new(5, 1);
        for x in 0..5 {
            atlas.fill_rect(Rect::new(x, 0, 1, 1), Color::rgb(10 * (x as u8 + 1), 0, 0));
        }
        let s = with_taskbar(
            PartImage::new(atlas, &[State::Normal], Layout::Vertical)
                .sizing(Insets::new(2, 0, 2, 0))
                .clip_ends(),
        );
        for w in [7, 3, 1] {
            let r = Rect::new(0, 0, w, 1);
            assert_doubles(&format!("clipped ends {w} wide"), w as u32, 1, |c, k| {
                s.draw(c, r.scaled(k), Part::Taskbar, State::Normal, k)
            });
        }
    }

    #[test]
    fn content_rect_at_scale_2_insets_twice_the_margins() {
        assert_eq!(
            style().content_rect(Part::Caption, Rect::new(0, 0, 20, 6), 2),
            Rect::new(4, 0, 12, 6)
        );
    }

    #[test]
    fn opaque_at_scale_2_answers_for_the_scale_1_pixel_under_it() {
        let s = style();
        let r = Rect::new(1, 1, 11, 3);
        for y in 0..10 {
            for x in 0..26 {
                assert_eq!(
                    s.opaque_at(Part::Caption, State::Normal, r.scaled(2), 2, x, y),
                    s.opaque_at(Part::Caption, State::Normal, r, 1, x / 2, y / 2),
                    "({x}, {y}) at s = 2 is ({}, {}) at s = 1",
                    x / 2,
                    y / 2
                );
            }
        }
    }
}
