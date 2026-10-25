use crate::raster::{Canvas, Fill, Insets, Rect};

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
    /// The frame's glass where it runs down under a restored window's top
    /// ([`crate::frame::FrameGeometry::glass_top`]), Normal focused, Inactive
    /// otherwise. Its outset covers the client's edges beside and above it.
    ClientGlass,
    /// [`Part::ClientGlass`] for a maximized window: the frame's plain glass.
    MaxClientGlass,
    MinButton,
    MaxButton,
    RestoreButton,
    CloseButton,
    /// A state one button takes has one whole-strip frame per button, in
    /// button order.
    CaptionGroup,
    MaxCaptionGroup,
    /// A windowed dialog's minimize and close boxes joined, framed as [`Part::CaptionGroup`]'s
    /// first and last buttons.
    MinCloseGroup,
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
    /// A document tab over the frame's glass: Normal, Hot and Checked (open).
    GlassTab,
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
    /// A toolbar separator over the frame's glass, the same in every mode.
    GlassSeparator,
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
    /// A task button whose window wants attention, in a lit phase.
    Attention,
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

    pub(super) fn per_atlas(&self, s: i32) -> i32 {
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

    pub(super) fn nines(&self, from: Rect, r: Rect, s: i32) -> [Option<(Rect, Rect, Insets)>; 2] {
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
