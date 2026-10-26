use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use reprisekit::{anim::HoverFade, icontheme::IconCache, style::TaskGlow};
use reprisescene::{BufferId, NodeId, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::{
    barplace::BarPlacement,
    compositor::{SceneOwner, State},
    glass::Backdrop,
    quicklaunch::QuickItem,
    raster::{Canvas, Rect},
    taskbar::{
        Bands, PagerButton, Place, TaskKey, TaskbarCounts, TaskbarHit, TaskbarLayout, TaskbarShape,
        TaskbarState,
    },
    taskslide,
    theme::{Color, Metrics},
    window::WindowId,
};

mod access;
mod draw;
mod pointer;

const DRAG_PX: f64 = 4.0;

const SPOT_TICK: Duration = Duration::from_millis(16);

pub struct BarShared {
    pub locked: bool,
    pub bands: Bands,
    pub quick_shown: bool,
    quick_moved: bool,
    quick_ghost: Option<BufferId>,
    pub(crate) outline: Option<(TreeId, Rect)>,
    pub(crate) side_width_drag: Option<i32>,
    pub slide_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    spot_timer: Option<smithay::reexports::calloop::RegistrationToken>,
    pub(crate) reveal_tick: Option<smithay::reexports::calloop::RegistrationToken>,
    pub(crate) reveal_idle: Vec<(TreeId, smithay::reexports::calloop::RegistrationToken)>,
    pub(crate) tray_pointer: Option<TreeId>,
    pub keys: Option<crate::barkeys::BarKeys>,
}

impl BarShared {
    pub fn from_persisted(p: &crate::state::Persisted) -> BarShared {
        BarShared {
            locked: p.taskbar_locked.unwrap_or(true),
            bands: p.bands,
            quick_shown: p.quicklaunch_shown.unwrap_or(true),
            quick_moved: false,
            quick_ghost: None,
            outline: None,
            side_width_drag: None,
            slide_timer: None,
            spot_timer: None,
            reveal_tick: None,
            reveal_idle: Vec::new(),
            tray_pointer: None,
            keys: None,
        }
    }
}

pub struct TaskWindows {
    windows: Vec<WindowId>,
    apps: Vec<String>,
    places: Vec<Place>,
    dialogs: Vec<TaskDialog>,
    pager: Vec<PagerButton>,
}

struct TaskDialog {
    id: u64,
    place: Place,
    title: String,
    active: bool,
    icon: Option<String>,
    progress: Option<reprisekit::taskbar::TaskProgress>,
}

impl TaskWindows {
    pub fn gather(state: &State) -> TaskWindows {
        let windows: Vec<WindowId> = state.task_windows();
        let apps = windows
            .iter()
            .filter_map(|&w| state.win(w))
            .map(|w| w.app_id())
            .collect();
        let places = windows
            .iter()
            .filter_map(|&w| state.win(w))
            .map(|w| Place {
                anchor: w.anchor,
                seq: w.seq,
            })
            .collect();
        let top = state.active_dialog_id();
        let dialogs = state
            .dialogs
            .views
            .iter()
            .filter(|v| v.dialog.on_taskbar())
            .map(|v| TaskDialog {
                id: v.id,
                place: Place {
                    anchor: v.anchor,
                    seq: v.seq,
                },
                title: v.dialog.title.clone(),
                active: top == Some(v.id),
                icon: v.dialog.task_icon().map(String::from),
                progress: v.dialog.task_progress(),
            })
            .collect();
        TaskWindows {
            windows,
            apps,
            places,
            dialogs,
            pager: state.pager_buttons(),
        }
    }
}

struct StartMix {
    dark: (Canvas, Option<Canvas>),
    lit: (Canvas, Option<Canvas>),
    held: bool,
    scale: i32,
    changed: Rect,
}

/// A bar picture and what it was drawn from.
pub(crate) struct DrawnBar {
    pub(crate) state: TaskbarState,
    pub(crate) layout: TaskbarLayout,
    pub(crate) scale: i32,
    pub(crate) canvas: Canvas,
}

pub struct Bar {
    pub output: Output,
    pub tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    /// `place_bar` cuts its source box by this: a Start fade frame keeps its
    /// cached pictures' scale, and a slide frame may precede the redraw.
    buffer_scale: i32,
    start_above: BufferId,
    orb_tree: TreeId,
    pub(crate) orb_up: Option<bool>,
    pub layout: TaskbarLayout,
    pub buttons: Vec<Vec<WindowId>>,
    pub pins: Vec<Option<usize>>,
    pub runs: Vec<Option<String>>,
    pub dialogs: Vec<Option<u64>>,
    pub start_pressed: bool,
    quick_hot: Option<usize>,
    pub quick_pressed: Option<usize>,
    pager_hot: Option<usize>,
    start_hot: bool,
    pub start_glow: HoverFade,
    start_mix: Option<StartMix>,
    /// The last full bar picture, so a change to Start alone redraws only Start's part.
    pub(crate) drawn: Option<DrawnBar>,
    /// By key, so a button closing under the pointer lights no neighbour that
    /// slides into its place.
    task_hot: Option<TaskKey<WindowId>>,
    task_held: Option<usize>,
    /// Logical px from the button's left edge.
    task_hot_x: Option<i32>,
    task_hot_x_drawn: Option<i32>,
    show_desktop_hot: bool,
    /// The » cell under the pointer: the task row's (More) or Quick Launch's (QuickMore).
    more_hot: Option<TaskbarHit>,
    pub(crate) chevron_hot: bool,
    pub chevron_held: bool,
    pub task_slides: Vec<(taskslide::Key, reprisekit::anim::Slide)>,
    pub task_float: Option<(taskslide::Key, reprisekit::anim::Slide)>,
    pub task_float_drawn: Option<i32>,
    quick_drop: Option<usize>,
    pub tray_open: bool,
    pub tray_reveal: reprisekit::anim::Slide,
    pub tray_reveal_drawn: i32,
    #[cfg(test)]
    pub draws: usize,
    #[cfg(test)]
    pub start_only_draws: usize,
}

fn empty_layout() -> TaskbarLayout {
    TaskbarLayout::compute(
        &Metrics::default(),
        &TaskbarShape::CLASSIC,
        1,
        true,
        TaskbarCounts::default(),
        0,
    )
}

fn stands_over_menu(l: &TaskbarLayout, held: bool, covered: bool) -> bool {
    !covered && l.start_orb(held) != l.start_above
}

fn to_next_minute() -> Duration {
    let tm = reprisefs::locale::localtime(crate::taskbar::clock_now());
    Duration::from_millis(((60 - tm.tm_sec.clamp(0, 60)) * 1000 + 100) as u64)
}

pub fn load_quicklaunch(config: &crate::config::Config) -> Vec<QuickItem> {
    let Some(dir) = reprisefs::quicklaunch::dir() else {
        return Vec::new();
    };
    let seeded = crate::quicklaunch::seed(&dir, &config.quicklaunch);
    let stored = reprisefs::quicklaunch::read_order(&dir);
    let order = seeded
        .or(stored.clone())
        .unwrap_or_else(|| crate::state::load().quicklaunch);
    let mut items = crate::quicklaunch::scan(&dir, &config.terminal);
    reprisefs::quicklaunch::apply_order(&mut items, &order, |q| &q.file);
    if stored.is_none()
        && let Err(e) = reprisefs::quicklaunch::write_order(&dir, &order)
    {
        log::warn!("quicklaunch: could not write the order: {e}");
    }
    items
}

pub(crate) fn app_icon(
    icons: &mut IconCache,
    index: &[reprisefs::apps::AppEntry],
    app_id: &str,
    px: i32,
) -> Option<Arc<Canvas>> {
    app_icon_waiting(icons, index, app_id, px).0
}

/// [`app_icon`], with the icon file it waits on when that file is not decoded at `px` yet.
pub(crate) fn app_icon_waiting(
    icons: &mut IconCache,
    index: &[reprisefs::apps::AppEntry],
    app_id: &str,
    px: i32,
) -> (Option<Arc<Canvas>>, Option<String>) {
    let Some(name) = app_icon_name(icons, index, app_id, px) else {
        return (None, None);
    };
    match icons.get_unread(name, px) {
        Some(icon) => (icon, None),
        None => (
            icons.get_shared(crate::icon::STAND_IN, px),
            Some(name.to_string()),
        ),
    }
}

/// The theme icon name or icon file [`app_icon`] draws for `app_id`.
pub(crate) fn app_icon_name<'a>(
    icons: &mut IconCache,
    index: &'a [reprisefs::apps::AppEntry],
    app_id: &'a str,
    px: i32,
) -> Option<&'a str> {
    if app_id.is_empty() {
        return None;
    }
    // The app id is a client-chosen string used as a theme icon name; one
    // with a path separator would be taken for a file path, so it only
    // ever goes through the `.desktop` match below.
    if !app_id.contains('/') && icons.get_shared(app_id, px).is_some() {
        return Some(app_id);
    }
    reprisefs::apps::icon_for_app_id(index, app_id)
}

/// A Quick Launch item's or pin's icon at `px`, its file never read on the loop.
pub(crate) fn quick_icon(icons: &mut IconCache, q: &QuickItem, px: i32) -> Option<Arc<Canvas>> {
    (q.entry.icon.as_deref()).and_then(|n| crate::icon::unread(icons, n, px))
}

const GLOW_KEEP: usize = 256;

type GlowKey = (usize, u32, u32, u64);

type GlowKept = (HashMap<GlowKey, (Option<Color>, u64)>, u64);

pub(crate) fn glow_colour(glow: TaskGlow, icon: &Canvas) -> Option<Color> {
    thread_local! {
        static KEPT: RefCell<GlowKept> = RefCell::new((HashMap::new(), 0));
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    icon.pixels.hash(&mut h);
    let key = (glow.colour as usize, icon.width, icon.height, h.finish());
    KEPT.with_borrow_mut(|(kept, tick)| {
        *tick += 1;
        if let Some((c, used)) = kept.get_mut(&key) {
            *used = *tick;
            return *c;
        }
        if kept.len() >= GLOW_KEEP {
            let oldest = kept
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| *k);
            if let Some(k) = oldest {
                kept.remove(&k);
            }
        }
        let c = (glow.colour)(icon);
        kept.insert(key, (c, *tick));
        c
    })
}

impl State {
    /// The windows with a task button: mapped, on the current desktop or
    /// on all of them, not owned by another, and not a message box or help
    /// dialog unless minimized.
    pub fn task_windows(&self) -> Vec<WindowId> {
        self.windows
            .iter()
            .filter(|w| {
                w.mapped
                    && !w.is_owned()
                    && (w.minimized || !w.skips_taskbar())
                    && self.on_current_desktop(w.id)
            })
            .map(|w| w.id)
            .collect()
    }

    pub fn sync_bars(&mut self) {
        let outputs = self.scene.outputs().to_vec();
        let (kept, gone): (Vec<Bar>, Vec<Bar>) = std::mem::take(&mut self.bars)
            .into_iter()
            .partition(|b| outputs.contains(&b.output));
        self.bars = kept;
        // A gone bar's Show desktop peek has nothing to leave onto.
        if gone.iter().any(|b| b.show_desktop_hot) {
            crate::peek::leave(self);
        }
        for bar in gone {
            self.scene.destroy(bar.tree);
            self.scene.destroy(bar.orb_tree);
        }
        for out in outputs {
            if !self.bars.iter().any(|b| b.output == out) {
                let bar = self.new_bar(out);
                self.bars.push(bar);
            }
        }
        if !self.bars.is_empty() {
            self.arm_clock();
        }
        self.redraw_taskbars();
    }

    fn new_bar(&mut self, output: Output) -> Bar {
        let scene = &mut self.scene;
        let tree = scene.tree_create(self.layers.taskbar);
        scene.set_data(tree, Some(SceneOwner::Taskbar(tree)));
        crate::glass::leave_out(scene, tree);
        // Before the buffer, so it lies beneath it.
        let glass_tree = scene.tree_create(tree);
        let buffer = scene.buffer_create(tree);
        let start_above = scene.buffer_create(tree);
        scene.set_enabled(start_above, false);
        let orb_tree = scene.tree_create(self.layers.start_orb);
        scene.set_data(orb_tree, Some(SceneOwner::Taskbar(tree)));
        crate::glass::leave_out(scene, orb_tree);
        scene.set_enabled(orb_tree, false);
        Bar {
            output,
            tree,
            glass_tree,
            glass: Backdrop::new(&self.glass),
            buffer,
            buffer_scale: 1,
            start_above,
            orb_tree,
            orb_up: None,
            layout: empty_layout(),
            buttons: Vec::new(),
            pins: Vec::new(),
            runs: Vec::new(),
            dialogs: Vec::new(),
            start_pressed: false,
            quick_hot: None,
            quick_pressed: None,
            pager_hot: None,
            start_hot: false,
            start_glow: HoverFade::default(),
            start_mix: None,
            drawn: None,
            task_hot: None,
            task_held: None,
            task_hot_x: None,
            task_hot_x_drawn: None,
            show_desktop_hot: false,
            more_hot: None,
            chevron_hot: false,
            chevron_held: false,
            task_slides: Vec::new(),
            task_float: None,
            task_float_drawn: None,
            quick_drop: None,
            tray_open: false,
            tray_reveal: reprisekit::anim::Slide::at(0),
            tray_reveal_drawn: 0,
            #[cfg(test)]
            draws: 0,
            #[cfg(test)]
            start_only_draws: 0,
        }
    }

    /// The outputs bar `tree` draws on: its own and any its trees reach.
    fn bar_outputs(&self, tree: TreeId) -> Vec<Output> {
        let Some(b) = self.bar(tree) else {
            return Vec::new();
        };
        let mut out = self.outputs_of_nodes(&[b.tree.into(), b.orb_tree.into()]);
        if !out.contains(&b.output) {
            out.push(b.output.clone());
        }
        out
    }

    pub fn bar(&self, tree: TreeId) -> Option<&Bar> {
        self.bars.iter().find(|b| b.tree == tree)
    }

    pub fn bar_mut(&mut self, tree: TreeId) -> Option<&mut Bar> {
        self.bars.iter_mut().find(|b| b.tree == tree)
    }

    /// Run `f` on bar `tree` taken out of `bars`, so it may use the rest
    /// of the state freely; `f` must not look bars up.
    pub(crate) fn with_bar<R>(
        &mut self,
        tree: TreeId,
        f: impl FnOnce(&mut State, &mut Bar) -> R,
    ) -> Option<R> {
        let i = self.bars.iter().position(|b| b.tree == tree)?;
        let mut bar = self.bars.remove(i);
        let r = f(self, &mut bar);
        self.bars.insert(i, bar);
        Some(r)
    }

    fn arm_clock(&mut self) {
        if self.clock_timer.is_some() {
            return;
        }
        let token =
            self.handle
                .insert_source(Timer::from_duration(to_next_minute()), |_, _, state| {
                    reprisefs::locale::refresh_zone();
                    state.redraw_taskbars();
                    TimeoutAction::ToDuration(to_next_minute())
                });
        match token {
            Ok(t) => self.clock_timer = Some(t),
            Err(e) => log::warn!("clock timer: {}", e.error),
        }
    }

    pub fn redraw_taskbars(&mut self) {
        if self.bars.is_empty() {
            return;
        }
        let tasks = TaskWindows::gather(self);
        let was: Vec<Vec<Output>> = self.bars.iter().map(|b| self.bar_outputs(b.tree)).collect();
        let mut bars = std::mem::take(&mut self.bars);
        let mut asks: Vec<(Vec<Output>, [NodeId; 2])> = Vec::new();
        for (bar, was) in bars.iter_mut().zip(was) {
            if self.redraw_taskbar_with(bar, &tasks) {
                asks.push((was, [bar.tree.into(), bar.orb_tree.into()]));
            }
        }
        self.bars = bars;
        crate::trayflyout::bar_redrawn(self, None);
        for (was, nodes) in asks {
            self.redraw_nodes(was, &nodes);
        }
        crate::access::touch(self);
        crate::autohideview::poke(self);
    }

    pub fn redraw_taskbar(&mut self, tree: TreeId) {
        let was = self.bar_outputs(tree);
        let tasks = TaskWindows::gather(self);
        self.with_bar(tree, |s, bar| {
            s.redraw_taskbar_with(bar, &tasks);
        });
        let output = self.bar(tree).map(|b| b.output.clone());
        crate::trayflyout::bar_redrawn(self, output.as_ref());
        let nodes: Vec<NodeId> = self
            .bar(tree)
            .map(|b| vec![b.tree.into(), b.orb_tree.into()])
            .unwrap_or_default();
        self.redraw_nodes(was, &nodes);
        crate::access::touch(self);
        crate::autohideview::poke(self);
    }

    pub fn bar_placement(&self, output: &Output) -> BarPlacement {
        let hidden = if self.bar_hide.headed_out(output) {
            1.0
        } else {
            0.0
        };
        self.bar_at(output, hidden)
    }

    pub fn bar_drawn(&self, output: &Output) -> BarPlacement {
        self.bar_at(output, self.bar_hide.drawn(output))
    }

    fn bar_at(&self, output: &Output, hidden: f32) -> BarPlacement {
        let b = self.layout_box(Some(output));
        let (theme, tb) = (&self.config.theme, &self.config.taskbar);
        let t = crate::barplace::thickness(
            tb.position,
            &theme.metrics,
            &theme.taskbar_shape(),
            tb.side_width,
            b,
        );
        // A width drag lays the bars out at the dragged width; windows keep the
        // configured one until the release.
        let hide = tb.auto_hide.then_some(hidden);
        match self
            .bar_shared
            .side_width_drag
            .filter(|_| tb.position.is_side())
        {
            Some(w) => {
                let live = crate::barplace::thickness(
                    tb.position,
                    &theme.metrics,
                    &theme.taskbar_shape(),
                    w,
                    b,
                );
                crate::barplace::place_resizing(b, tb.position, t, live, hide)
            }
            None => crate::barplace::place_hiding(b, tb.position, t, hide),
        }
    }

    pub(crate) fn bar_local(&self, bar: &Bar, x: f64, y: f64) -> (i32, i32) {
        let s = self.bar_drawn(&bar.output).shown;
        (x as i32 - s.x, y as i32 - s.y)
    }

    pub(crate) fn bar_along(&self, bar: &Bar, x: f64, y: f64) -> i32 {
        let (lx, ly) = self.bar_local(bar, x, y);
        bar.layout.along(lx, ly)
    }

    pub fn bar_hit(&self, bar: &Bar, x: f64, y: f64) -> TaskbarHit {
        if self.bar_hide.drawn(&bar.output) > 0.0 {
            return TaskbarHit::Nothing;
        }
        let s = self.bar_drawn(&bar.output).shown;
        bar.layout.hit(x as i32 - s.x, y as i32 - s.y)
    }

    pub(crate) fn under_fullscreen(&self, bar: &Bar) -> bool {
        let Some(w) = self.bind_window().and_then(|w| self.win(w)) else {
            return false;
        };
        if !w.fullscreen {
            return false;
        }
        let r = self.bar_placement(&bar.output).shown;
        !w.frame_rect().intersect(&r).is_empty()
    }
}

/// The icon theme's start-here at the orb's size at draw scale `scale`, when `start_icon` asks for
/// it; None for the Reprise mark.
pub fn start_icon(
    config: &crate::config::Config,
    icons: &mut IconCache,
    scale: i32,
) -> Option<Arc<Canvas>> {
    let px = config.theme.taskbar_shape().start_icon * scale;
    config
        .start_icon
        .then(|| icons.get_shared("start-here", px))
        .flatten()
}

#[cfg(test)]
mod tests;
