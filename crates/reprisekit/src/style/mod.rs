use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

pub mod frame;
pub(crate) mod startbloom;
pub mod taskbar;
pub(crate) mod trayarrow;

mod looks;
mod part;
mod spot;
mod text;

pub use looks::{
    BalloonLook, CaptionTabLook, DropList, DropShadow, FlyoutLook, GlassSpec, GroupLabel, MenuLook,
    NotifyInks, NotifyLook, PageInks, PanelSpec, SQUIRCLE_ROUND, SelectionBox, SheetShape,
    SheetSpec, TrackMarks, Well,
};
pub use part::{CHECK_STATES, Check, Layout, Part, PartImage, State};
pub use spot::{
    GlowingPair, SpotFit, SpotFrames, SpotLayer, SpotLight, TaskGlow, TaskSpot, glow_chroma,
};
pub use text::{CaptionTitle, GlowMask, Shadow, SoftShadow, TextSpec, TitleGlow};

use crate::metrics::ControlMetrics;

use crate::raster::{Canvas, Insets, Rect, nine_source};
use crate::taskbar::TaskbarShape;
use crate::theme::{Color, ColorScheme, Metrics};

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
    /// The halo under labels a client draws over its frame's glass ([`Part::ClientGlass`]).
    pub label_glow: Option<TitleGlow>,
    pub task_glow: Option<TaskGlow>,
    pub(crate) task_progress: Option<crate::taskbar::ProgressPainter>,
    task_spot: Option<Spot>,
    pub task_nudge: bool,
    pub greyed_field_frame: bool,
    pub combo_edit_border: bool,
    parts: HashMap<Part, Art>,
    pub controls: ControlMetrics,
    text_colors: HashMap<(Part, State), Ink>,
    pub group_label: GroupLabel,
    pub smooth_progress: bool,
    pub track_marks: TrackMarks,
    /// The help frames hold the ? box and the close box joined, the ? box at the left as wide as
    /// the frame lays it out.
    pub joined_help: bool,
    pub flat_menus: bool,
    pub buttons_over_caption: bool,
    /// Whether the caption title is centred across the frame, moved left
    /// as far as it must to end before the first caption button but never
    /// left of [`CaptionTitle::text_x`]. Unset, it starts at `text_x`.
    pub centred_title: bool,
    pub page: Option<PageInks>,
    pub drop_list: Option<DropList>,
    pub balloon: Option<BalloonLook>,
    pub notify: Option<NotifyLook>,
    pub menu: Option<MenuLook>,
    /// The Start panel's Shut down options menu, when it differs from [`Style::menu`].
    pub options_menu: Option<MenuLook>,
    pub selection_box: Option<SelectionBox>,
    pub flyout: Option<FlyoutLook>,
    pub caption_tabs: Option<CaptionTabLook>,
    pub(crate) tray_arrow: Option<Arc<trayarrow::TrayArrowArt>>,
    pub(crate) start_bloom: Option<Arc<startbloom::StartBloom>>,
    pub(crate) clock_face: Option<ClockPainter>,
    pub(crate) copy_flight: Option<crate::copyart::FlightPainter>,
    /// The copy dialog drawn as a page with a throughput graph instead of the flight.
    pub copy_graph: Option<crate::copyart::GraphLook>,
    pub(crate) client_glass_edges: Option<ClientGlassEdges>,
    pub chrome_colors: Option<ColorScheme>,
    rescale: Option<Rescale>,
    rescaled: Rescaled,
    button_side: Option<ButtonSide>,
    sided: Rescaled,
}

/// Art [`Style::warm`] found unpainted at one scale; [`Warm::paint`] paints it, on any thread.
pub struct Warm {
    style: Arc<Style>,
    scale: i32,
    parts: Vec<Part>,
    spot: bool,
    start_icon: Option<Arc<Canvas>>,
    /// The orb's blooms round `start_icon`, by state and strength.
    blooms: Vec<(State, u8)>,
    /// Caption button sides to build ([`Style::at_button_side`]).
    sides: Vec<i32>,
}

impl Warm {
    pub fn paint(&self) {
        for &part in &self.parts {
            self.style.part(part);
        }
        if self.spot {
            self.style.task_spot();
        }
        let icon = self.start_icon.as_deref();
        for &(state, strength) in &self.blooms {
            taskbar::orb_bloom(&self.style, self.scale, icon, state, strength);
        }
        for &side in &self.sides {
            self.style.at_button_side(side);
        }
    }
}

#[derive(Default)]
struct Rescaled(Mutex<Vec<(i32, Arc<Style>)>>);

impl Clone for Rescaled {
    fn clone(&self) -> Self {
        Rescaled::default()
    }
}

type Rescale = Arc<dyn Fn(i32) -> Style + Send + Sync>;

/// Paints a look's caption and ? buttons `side` design px square at the style's scale.
type ButtonSide = Arc<dyn Fn(i32) -> Vec<(Part, PartImage)> + Send + Sync>;

/// Draws a look's client glass edges ([`crate::glass::frame::client_glass_edges`]): canvas,
/// band, client height, state, scale.
pub(crate) type ClientGlassEdges = Arc<dyn Fn(&mut Canvas, Rect, i32, State, i32) + Send + Sync>;

/// Paints the clock popup's analog clock at `time` (hour 0..23, minute, second), `side` design px
/// square, at scale `d`.
pub(crate) type ClockPainter = fn(time: (u32, u32, u32), side: i32, d: i32) -> Canvas;

/// A part's art, painted the first time it is asked for; clones share it.
type Art = Arc<LazyLock<PartImage, Box<dyn FnOnce() -> PartImage + Send>>>;
type Ink = Arc<LazyLock<Color, Box<dyn FnOnce() -> Color + Send>>>;

/// The lit task buttons' art, painted the first time it is asked for; clones share it.
type Spot = Arc<LazyLock<TaskSpot, Box<dyn FnOnce() -> TaskSpot + Send>>>;

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
            label_glow: None,
            task_glow: None,
            task_progress: None,
            task_spot: None,
            task_nudge: true,
            greyed_field_frame: false,
            combo_edit_border: false,
            parts: HashMap::new(),
            controls: ControlMetrics::CLASSIC,
            text_colors: HashMap::new(),
            group_label: GroupLabel::LINE,
            smooth_progress: false,
            track_marks: TrackMarks::LOW,
            joined_help: false,
            flat_menus: false,
            buttons_over_caption: false,
            centred_title: false,
            page: None,
            drop_list: None,
            menu: None,
            options_menu: None,
            selection_box: None,
            caption_tabs: None,
            tray_arrow: None,
            start_bloom: None,
            clock_face: None,
            copy_flight: None,
            copy_graph: None,
            client_glass_edges: None,
            chrome_colors: None,
            balloon: None,
            notify: None,
            flyout: None,
            rescale: None,
            rescaled: Rescaled::default(),
            button_side: None,
            sided: Rescaled::default(),
        }
    }

    pub fn with_rescale(mut self, build: impl Fn(i32) -> Style + Send + Sync + 'static) -> Style {
        self.rescale = Some(Arc::new(build));
        self
    }

    /// Caption buttons (and the ? box) that `paint` draws at any side, for a caption grown with the
    /// text size ([`Style::at_button_side`]).
    pub(crate) fn with_button_side(
        mut self,
        paint: impl Fn(i32) -> Vec<(Part, PartImage)> + Send + Sync + 'static,
    ) -> Style {
        self.button_side = Some(Arc::new(paint));
        self
    }

    /// This style with its caption buttons `side` design px square, built once per side; `None`
    /// when they already are or the style draws them at one size only.
    pub fn at_button_side(&self, side: i32) -> Option<Arc<Style>> {
        let paint = self.button_side.as_ref()?;
        let close = self.part(Part::CloseButton)?;
        if close.frame(State::Normal).height / close.scale == side {
            return None;
        }
        let mut kept = self.sided.0.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, st)) = kept.iter().find(|(k, _)| *k == side) {
            return Some(st.clone());
        }
        let st = Arc::new(
            paint(side)
                .into_iter()
                .fold(self.clone(), |st, (part, img)| {
                    st.with_painted(part, move || img)
                }),
        );
        kept.push((side, st.clone()));
        Some(st)
    }

    /// Whether [`Style::at_button_side`] would build `side` now: a side other than the metrics'
    /// own, not built yet. Reads no art, so the host may ask on its loop.
    fn builds_side(&self, side: i32) -> bool {
        self.button_side.is_some() && side != crate::frame::button_side(&self.metrics) && {
            let kept = self.sided.0.lock().unwrap_or_else(|e| e.into_inner());
            !kept.iter().any(|(k, _)| *k == side)
        }
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

    /// The art a first draw at scale `n` would paint: the parts, lit task buttons and orb blooms
    /// drawn at a scale this style keeps and not yet at `n`, the blooms round `start_icon` (the
    /// orb's logo at `n`, device px; None for the mark). None when there is nothing to paint.
    pub fn warm(self: &Arc<Style>, n: i32, start_icon: Option<Arc<Canvas>>) -> Option<Warm> {
        let at_n = self.at_scale(n);
        if Arc::ptr_eq(&at_n, self) {
            return None;
        }
        let kept: Vec<Arc<Style>> = {
            let kept = self.rescaled.0.lock().unwrap_or_else(|e| e.into_inner());
            kept.iter().map(|(_, st)| st.clone()).collect()
        };
        let drawn = |part: &Part| {
            let at = |st: &Style| {
                st.parts
                    .get(part)
                    .is_some_and(|a| LazyLock::get(a).is_some())
            };
            at(self) || kept.iter().any(|st| at(st))
        };
        let parts: Vec<Part> = (at_n.parts.iter())
            .filter(|(part, art)| LazyLock::get(art).is_none() && drawn(part))
            .map(|(part, _)| *part)
            .collect();
        let spot_drawn = |st: &Style| {
            st.task_spot
                .as_deref()
                .is_some_and(|s| LazyLock::get(s).is_some())
        };
        let spot = !spot_drawn(&at_n) && (spot_drawn(self) || kept.iter().any(|st| spot_drawn(st)));
        let mut blooms: Vec<(State, u8)> = Vec::new();
        for st in std::iter::once(self).chain(&kept) {
            for b in (st.start_bloom.as_ref()).map_or(Vec::new(), |b| b.states()) {
                if !blooms.contains(&b) {
                    blooms.push(b);
                }
            }
        }
        let icon = start_icon.as_deref();
        blooms.retain(|&(st, strength)| !taskbar::keeps_orb_bloom(&at_n, n, icon, st, strength));
        (!parts.is_empty() || spot || !blooms.is_empty()).then_some(Warm {
            style: at_n,
            scale: n,
            parts,
            spot,
            start_icon,
            blooms,
            sides: Vec::new(),
        })
    }

    /// The art of `parts` a first draw at scale `n` would paint: those this style draws at `n`
    /// and has not painted there yet. None when there is nothing to paint.
    pub fn warm_parts(self: &Arc<Style>, n: i32, parts: &[Part]) -> Option<Warm> {
        let at_n = self.at_scale(n);
        let parts: Vec<Part> = (parts.iter().copied())
            .filter(|part| (at_n.parts.get(part)).is_some_and(|a| LazyLock::get(a).is_none()))
            .collect();
        (!parts.is_empty()).then_some(Warm {
            style: at_n,
            scale: n,
            parts,
            spot: false,
            start_icon: None,
            blooms: Vec::new(),
            sides: Vec::new(),
        })
    }

    /// The caption buttons a first frame at scale `n` would build at `sides`
    /// ([`crate::frame::caption_button_sides`]): those this style grows and has not built there
    /// yet. None when there is nothing to build.
    pub fn warm_button_sides(self: &Arc<Style>, n: i32, sides: &[i32]) -> Option<Warm> {
        let at_n = self.at_scale(n);
        let sides: Vec<i32> = (sides.iter().copied())
            .filter(|&side| at_n.builds_side(side))
            .collect();
        (!sides.is_empty()).then_some(Warm {
            style: at_n,
            scale: n,
            parts: Vec::new(),
            spot: false,
            start_icon: None,
            blooms: Vec::new(),
            sides,
        })
    }

    /// Drops the styles [`Style::at_scale`] kept for the scales `keep` turns down.
    pub fn keep_scales(&self, keep: impl Fn(i32) -> bool) {
        let mut kept = self.rescaled.0.lock().unwrap_or_else(|e| e.into_inner());
        kept.retain(|(n, _)| keep(*n));
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

    pub fn with_label_glow(mut self, g: TitleGlow) -> Style {
        self.label_glow = Some(g);
        self
    }

    pub(crate) fn with_task_progress(mut self, paint: crate::taskbar::ProgressPainter) -> Style {
        self.task_progress = Some(paint);
        self
    }

    pub fn with_task_glow(mut self, g: Option<TaskGlow>) -> Style {
        self.task_glow = g;
        self
    }

    pub fn with_task_spot(mut self, t: Option<TaskSpot>) -> Style {
        self.task_spot = None;
        match t {
            Some(t) => self.with_painted_task_spot(move || t),
            None => self,
        }
    }

    /// The lit task buttons drawn by `paint`, which runs the first time they are asked for.
    pub fn with_painted_task_spot(
        mut self,
        paint: impl FnOnce() -> TaskSpot + Send + 'static,
    ) -> Style {
        self.task_spot = Some(Arc::new(LazyLock::new(Box::new(paint))));
        self
    }

    pub fn task_spot(&self) -> Option<&TaskSpot> {
        self.task_spot.as_deref().map(LazyLock::force)
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

    pub fn with_options_menu(mut self, m: MenuLook) -> Style {
        self.options_menu = Some(m);
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

    pub(crate) fn with_client_glass_edges(mut self, paint: ClientGlassEdges) -> Style {
        self.client_glass_edges = Some(paint);
        self
    }

    pub(crate) fn with_clock_face(mut self, paint: ClockPainter) -> Style {
        self.clock_face = Some(paint);
        self
    }

    pub(crate) fn with_copy_flight(mut self, paint: crate::copyart::FlightPainter) -> Style {
        self.copy_flight = Some(paint);
        self
    }

    pub(crate) fn with_copy_graph(mut self, look: crate::copyart::GraphLook) -> Style {
        self.copy_graph = Some(look);
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

    pub fn with_notify(mut self, n: NotifyLook) -> Style {
        self.notify = Some(n);
        self
    }

    pub fn with_flyout(mut self, f: FlyoutLook) -> Style {
        self.flyout = Some(f);
        self
    }

    pub fn with_joined_help(mut self) -> Style {
        self.joined_help = true;
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

    /// `parts` drawn together by `paint`, which runs the first time any of them is
    /// asked for and must yield each of them.
    pub fn with_painted_group(
        self,
        parts: &[Part],
        paint: impl FnOnce() -> Vec<(Part, PartImage)> + Send + 'static,
    ) -> Style {
        let group = Arc::new(LazyLock::new(move || {
            Mutex::new(paint().into_iter().collect::<HashMap<_, _>>())
        }));
        parts.iter().fold(self, |st, &part| {
            let group = group.clone();
            st.with_painted(part, move || {
                let mut painted = group.lock().unwrap_or_else(|e| e.into_inner());
                painted
                    .remove(&part)
                    .unwrap_or_else(|| panic!("the group's painter yields no {part:?}"))
            })
        })
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

    pub fn with_text_color(self, part: Part, state: State, c: Color) -> Style {
        self.with_picked_text_color(part, state, move || c)
    }

    /// The text colour for `part` in `state`, picked by `pick` the first time it is asked for.
    pub fn with_picked_text_color(
        mut self,
        part: Part,
        state: State,
        pick: impl FnOnce() -> Color + Send + 'static,
    ) -> Style {
        self.text_colors
            .insert((part, state), Arc::new(LazyLock::new(Box::new(pick))));
        self
    }

    pub fn text_color(&self, part: Part, state: State) -> Option<Color> {
        self.text_colors
            .get(&(part, state))
            .map(|c| *LazyLock::force(c))
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

impl std::fmt::Debug for Style {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Style")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
