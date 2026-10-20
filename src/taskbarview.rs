use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::Arc,
    time::{Duration, Instant},
};

use reprisekit::taskbar::{BarSpan, ClockText};
use reprisekit::{anim::HoverFade, icontheme::IconCache, style::TaskGlow};
use reprisescene::{BufferId, Input, TreeId};
use smithay::{
    output::Output,
    reexports::calloop::timer::{TimeoutAction, Timer},
};

use crate::{
    barplace::BarPlacement,
    compositor::{SceneOwner, State},
    glass::{self, Backdrop, Pane, PaneShape},
    input::{BTN_LEFT, BTN_MIDDLE, BTN_RIGHT, Hit, Interaction},
    menuview::MenuRoot,
    output::upload_scaled,
    quicklaunch::QuickItem,
    raster::{Canvas, Rect},
    taskbar::{
        Bands, PagerButton, Place, PopupCell, TaskEntry, TaskKey, TaskbarCounts, TaskbarHit,
        TaskbarLayout, TaskbarShape, TaskbarState,
    },
    taskslide,
    theme::{Color, Metrics},
    window::WindowId,
};

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
    // SAFETY: `time` accepts a null out-pointer.
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    // SAFETY: `libc::tm` is a plain C struct; all-zeroes is valid.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are to live locals.
    unsafe { libc::localtime_r(&now, &mut tm) };
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
    if app_id.is_empty() {
        return (None, None);
    }
    // The app id is a client-chosen string used as a theme icon name; one
    // with a path separator would be taken for a file path, so it only
    // ever goes through the `.desktop` match below.
    if !app_id.contains('/')
        && let Some(c) = icons.get_shared(app_id, px)
    {
        return (Some(c), None);
    }
    let Some(name) = reprisefs::apps::icon_for_app_id(index, app_id) else {
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
        let mut bars = std::mem::take(&mut self.bars);
        for bar in &mut bars {
            self.redraw_taskbar_with(bar, &tasks);
        }
        self.bars = bars;
        crate::trayflyout::bar_redrawn(self, None);
        self.redraw_all();
        crate::access::touch(self);
        crate::autohideview::poke(self);
    }

    pub fn redraw_taskbar(&mut self, tree: TreeId) {
        let tasks = TaskWindows::gather(self);
        self.with_bar(tree, |s, bar| s.redraw_taskbar_with(bar, &tasks));
        let output = self.bar(tree).map(|b| b.output.clone());
        crate::trayflyout::bar_redrawn(self, output.as_ref());
        self.redraw_all();
        crate::access::touch(self);
        crate::autohideview::poke(self);
    }

    pub fn task_button_name(&self, bar: &Bar, i: usize) -> Option<(String, bool)> {
        if let Some(id) = bar.dialogs.get(i).copied().flatten() {
            let v = self.dialogs.views.iter().find(|v| v.id == id)?;
            return Some((v.dialog.title.clone(), self.active_dialog_id() == Some(id)));
        }
        let live = |w: &crate::window::Window| self.focused == Some(w.id) && !w.minimized;
        let wins: Vec<&crate::window::Window> = (bar.buttons.get(i)?.iter())
            .filter_map(|&w| self.win(w))
            .collect();
        match wins.as_slice() {
            [] => {
                let k = bar.pins.get(i).copied().flatten()?;
                self.quicklaunch
                    .get(k)
                    .map(|q| (q.entry.label.clone(), false))
            }
            [w] => Some((w.title(), live(w))),
            many => {
                let name = crate::taskbar::group_name(&many[0].app_id());
                let active = many.iter().any(|w| live(w));
                Some((format!("{name} ({})", many.len()), active))
            }
        }
    }

    fn tray_cell_name(&self, bar: TreeId, i: usize) -> String {
        let revealed = self.bar(bar).map_or(0, |b| b.layout.tray_hidden);
        match crate::tray::cell_at(self, bar, i) {
            crate::tray::Cell::Hidden(_) => (self.tray.as_ref())
                .and_then(|t| t.item_at(revealed, 0, i))
                .map(|it| {
                    if it.tooltip.is_empty() {
                        it.title.clone()
                    } else {
                        it.tooltip.clone()
                    }
                })
                .unwrap_or_default(),
            crate::tray::Cell::Item(k) => (self.tray.as_ref())
                .and_then(|t| t.items.get(k))
                .map(|it| {
                    if it.tooltip.is_empty() {
                        it.title.clone()
                    } else {
                        it.tooltip.clone()
                    }
                })
                .unwrap_or_default(),
            crate::tray::Cell::Builtin(b) => (crate::tray::builtin::shown(self).get(b))
                .and_then(|&b| crate::tray::builtin::label(self, b))
                .unwrap_or_default(),
        }
    }

    fn more_open(&self, tree: TreeId) -> bool {
        self.menu_root() == Some(MenuRoot::TaskOverflow(tree))
    }

    fn quick_more_open(&self, tree: TreeId) -> bool {
        self.menu_root() == Some(MenuRoot::QuickOverflow(tree))
    }

    fn chevron_open(&self, bar: &Bar) -> bool {
        bar.tray_open || crate::trayflyout::open_on(self).as_ref() == Some(&bar.output)
    }

    pub fn taskbar_access(&self, bar: &Bar) -> reprisekit::access::Tree {
        let l = &bar.layout;
        let names = crate::taskbar::TaskbarNames {
            start_open: bar.start_pressed,
            quick: (0..l.quick.len())
                .map(|i| (self.quicklaunch.get(i)).map_or(String::new(), |q| q.entry.label.clone()))
                .collect(),
            desktop: self.desktops.current,
            pager_attention: (self.pager_buttons().iter()).map(|p| p.attention).collect(),
            tasks: (0..l.tasks.len())
                .map(|i| self.task_button_name(bar, i).unwrap_or_default())
                .collect(),
            attention: (0..l.tasks.len())
                .map(|i| {
                    (bar.buttons.get(i).into_iter().flatten())
                        .any(|&w| self.win(w).is_some_and(|w| w.attention))
                })
                .collect(),
            focus: self.bar_focus(bar),
            tray: (0..l.tray.len())
                .map(|i| self.tray_cell_name(bar.tree, i))
                .collect(),
            hidden_open: self.chevron_open(bar),
            more_open: self.more_open(bar.tree),
            quick_more_open: self.quick_more_open(bar.tree),
            clock: crate::taskbar::clock_text(),
            date: crate::taskbar::date_text(),
        };
        let rect = self.bar_placement(&bar.output).shown;
        let mut tree =
            crate::access::root(reprisekit::access::accesskit::Role::Window, "Taskbar", rect);
        crate::taskbar::access(l, &names, &mut tree, reprisekit::access::ROOT);
        tree
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

    /// The pointer's x inside task button `i` of `bar`, logical px from the
    /// button's left edge, as the last draw laid the row out; None if `i`
    /// is stale.
    fn task_pointer_x(&self, bar: &Bar, i: usize) -> Option<i32> {
        let r = bar.layout.tasks.get(i)?;
        let s = self.bar_drawn(&bar.output).shown;
        Some(self.pointer_location().x as i32 - s.x - r.x)
    }

    fn spot_moved(&mut self, tree: TreeId) {
        let spot = (self.config.theme.look.style()).is_some_and(|s| s.task_spot().is_some());
        let Some(bar) = self.bar(tree).filter(|_| spot) else {
            return;
        };
        let armed = self.bar_shared.spot_timer.is_some();
        if crate::taskbar::spot_due(bar.task_hot_x_drawn, bar.task_hot_x, armed)
            != crate::taskbar::SpotStep::Arm
        {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(SPOT_TICK), |_, _, state| {
                state.spot_tick();
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.bar_shared.spot_timer = Some(t),
            Err(e) => log::warn!("spot timer: {}", e.error),
        }
    }

    fn spot_tick(&mut self) {
        self.bar_shared.spot_timer = None;
        let due: Vec<TreeId> = (self.bars.iter())
            .filter(|b| {
                crate::taskbar::spot_due(b.task_hot_x_drawn, b.task_hot_x, false)
                    == crate::taskbar::SpotStep::Arm
            })
            .map(|b| b.tree)
            .collect();
        for t in due {
            self.redraw_taskbar(t);
        }
    }

    fn held_task(&self, bar: &Bar) -> Option<usize> {
        let Interaction::TaskPress {
            bar: tree,
            key,
            dragging: false,
            ..
        } = self.interaction
        else {
            return None;
        };
        if tree != bar.tree {
            return None;
        }
        let i = crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key)?;
        let p = self.pointer_location();
        (self.bar_hit(bar, p.x, p.y) == TaskbarHit::Task(i)).then_some(i)
    }

    /// An output out of the layout has an empty box: its bar hides and forgets
    /// its buttons, so neither a stale bar nor a closed window can be hit.
    fn redraw_taskbar_with(&mut self, bar: &mut Bar, tasks: &TaskWindows) {
        #[cfg(test)]
        {
            bar.draws += 1;
        }
        let b = self.layout_box(Some(&bar.output));
        if b.width <= 0 {
            self.scene.set_enabled(bar.tree, false);
            self.scene.set_enabled(bar.orb_tree, false);
            bar.glass
                .set(&mut self.scene, &self.glass, bar.glass_tree, None);
            bar.buttons.clear();
            bar.pins.clear();
            bar.runs.clear();
            bar.dialogs.clear();
            bar.start_mix = None;
            bar.drawn = None;
            bar.orb_up = None;
            bar.layout = empty_layout();
            return;
        }
        let scale = self.draw_scale_at(Some(&bar.output));
        let p = self.bar_placement(&bar.output);
        let chevron = crate::tray::chevron_shown(self);
        let now = Instant::now();
        let reveal = bar.tray_reveal.x(now).max(0);
        let hidden = crate::trayreveal::hidden(self);
        let revealed = crate::trayreveal::cells(
            p.edge.is_side(),
            &self.config.theme.taskbar_shape(),
            hidden,
            reveal,
        );
        let mut tray_icons =
            crate::tray::bar_icons(self, reprisekit::icon::ICON_SIZE * scale, revealed);
        let base = &self.config.theme;
        let m = &base.metrics;
        let (locked, bands) = (self.bar_shared.locked, self.bar_shared.bands);
        let show_desktop = self.config.taskbar.show_desktop_button;
        let both = crate::taskbar::bar_clock_text(true);
        let (time, date) = both.split_once('\n').unwrap_or((both.as_str(), ""));
        let measure = |s: &str| self.text.measure(s, false, base.fonts.size_px);
        let clock_text = ClockText {
            time: measure(time),
            date: measure(date),
        };
        let time = time.to_string();
        let theme = if scale == 1 {
            Cow::Borrowed(base)
        } else {
            Cow::Owned(base.scaled(scale))
        };
        let px = theme.icon_size();
        let icons = &mut self.icons;
        let pinned = self.config.taskbar.pinned;
        // No band with Toolbars > Quick Launch unchecked, nor while
        // `pinned` puts the Quick Launch items on the task row, which
        // shows them whatever the band would.
        let quick_icons: Vec<Option<Arc<Canvas>>> = if pinned || !self.bar_shared.quick_shown {
            Vec::new()
        } else {
            self.quicklaunch
                .iter()
                .map(|q| quick_icon(icons, q, px))
                .collect()
        };
        let TaskWindows {
            windows,
            apps,
            places,
            dialogs,
            pager,
        } = tasks;
        let (pin_count, pin_of, pin_icons): (usize, Vec<Option<usize>>, Vec<Option<String>>) =
            if pinned {
                let pins: &[QuickItem] = &self.quicklaunch;
                let index = &self.app_index;
                let pin_of = apps
                    .iter()
                    .map(|a| {
                        let id = reprisefs::apps::entry_for_app_id(index, a, |_| true);
                        crate::taskbar::pin_for(pins, a, id.map(|e| e.id.as_str()))
                    })
                    .collect();
                let icons = pins.iter().map(|q| q.entry.icon.clone()).collect();
                (pins.len(), pin_of, icons)
            } else {
                (0, Vec::new(), Vec::new())
            };
        let shape = base.taskbar_shape();
        let span = BarSpan {
            edge: p.edge,
            length: if p.edge.is_side() { b.height } else { b.width },
            thickness: p.thickness,
        };
        let mut layout = TaskbarLayout::compute_on(
            m,
            &shape,
            span,
            locked,
            TaskbarCounts {
                quick: quick_icons.len(),
                pager: pager.len(),
                tasks: apps.len() + pin_count + dialogs.len(),
                tray: tray_icons.len() - revealed,
                show_desktop,
                chevron,
                hidden,
                reveal,
            },
            clock_text,
            bands,
        );
        crate::tray::keep_run(&mut tray_icons, revealed, layout.tray_hidden);
        let clock = if layout.clock_lines.is_some() {
            both
        } else {
            time
        };
        let mode = self.config.taskbar.buttons;
        let dialog_tasks: Vec<crate::taskbar::DialogTask> = dialogs
            .iter()
            .map(|d| crate::taskbar::DialogTask {
                id: d.id,
                place: d.place,
            })
            .collect();
        let slots = crate::taskbar::lay_slots(
            &mut layout,
            m,
            &shape,
            apps,
            places,
            &pin_of,
            pin_count,
            &dialog_tasks,
            mode,
        );
        let task_px = shape.task_icon * scale;
        let task_glow = base.look.style().and_then(|s| s.task_glow);
        let glow_of = |icon: &Option<Arc<Canvas>>| {
            task_glow
                .zip(icon.as_deref())
                .and_then(|(g, ic)| glow_colour(g, ic))
        };
        let index = &self.app_index;
        let focused = self.focused;
        let wins = &self.windows;
        let win = |i: usize| wins.iter().find(|w| w.id == windows[i]);
        let live = |i: &usize| win(*i).is_some_and(|w| focused == Some(w.id) && !w.minimized);
        let asks = |i: &usize| win(*i).is_some_and(|w| w.attention);
        let entries: Vec<TaskEntry> = slots
            .iter()
            .map(|slot| {
                let group = &slot.windows;
                if let Some(d) = slot
                    .dialog
                    .and_then(|id| dialogs.iter().find(|d| d.id == id))
                {
                    let icon = (d.icon.as_deref()).and_then(|n| icons.get_shared(n, task_px));
                    return TaskEntry {
                        glow: glow_of(&icon),
                        icon,
                        ..crate::taskbar::dialog_entry(&d.title, d.active)
                    };
                }
                if group.is_empty() {
                    let name = slot.pin.and_then(|k| pin_icons.get(k).cloned().flatten());
                    return TaskEntry {
                        title: String::new(),
                        active: false,
                        attention: false,
                        icon: name.and_then(|n| crate::icon::unread(icons, &n, task_px)),
                        glow: None,
                    };
                }
                let icon = match win(group[0]) {
                    Some(w) => crate::icon::window_icon(icons, index, w, task_px),
                    None => app_icon(icons, index, &apps[group[0]], task_px),
                };
                let title = if group.len() == 1 {
                    win(group[0]).map(|w| w.title()).unwrap_or_default()
                } else {
                    let name = crate::taskbar::group_name(&apps[group[0]]);
                    format!("{name} ({})", group.len())
                };
                TaskEntry {
                    title,
                    active: group.iter().any(live),
                    attention: group.iter().any(asks),
                    glow: glow_of(&icon),
                    icon,
                }
            })
            .collect();
        bar.buttons = slots
            .iter()
            .map(|s| s.windows.iter().map(|&i| windows[i]).collect())
            .collect();
        bar.pins = slots.iter().map(|s| s.pin).collect();
        bar.runs = if mode == crate::config::TaskbarButtons::Group {
            slots
                .iter()
                .map(|s| match (s.pin, s.windows.first()) {
                    (None, Some(&w)) => apps.get(w).filter(|a| !a.is_empty()).cloned(),
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        bar.dialogs = slots.iter().map(|s| s.dialog).collect();
        bar.layout = layout;
        let start_icon = start_icon(&self.config, icons, scale);
        let task_hot = bar
            .task_hot
            .and_then(|key| crate::taskbar::find_task(&bar.buttons, &bar.pins, &bar.dialogs, &key));
        bar.task_held = self.held_task(bar);
        // An idle pin's slot light has no spot, so moves over it redraw nothing.
        let spot = bar.task_held.or(task_hot).is_some_and(|i| {
            (base.look.style()).is_some_and(|s| {
                reprisekit::style::taskbar::task_draws_spot(s, bar.layout.pinned_idle(i))
            })
        });
        bar.task_hot_x_drawn = bar.task_hot_x.filter(|_| spot);
        let (task_shift, task_float) = taskslide::draw_state(bar, Instant::now());
        let pid = |w: WindowId| self.client_pid(w).and_then(|p| u32::try_from(p).ok());
        let any_shown = self.launcher_entries.any_shown();
        let owners: HashSet<u32> = match any_shown {
            true => self.windows.iter().filter_map(|w| pid(w.id)).collect(),
            false => HashSet::new(),
        };
        let task_progress = (bar.buttons.iter().zip(&bar.dialogs))
            .map(|(wins, dialog)| {
                if let Some(id) = dialog {
                    return (dialogs.iter().find(|d| d.id == *id)).and_then(|d| d.progress);
                }
                if !any_shown {
                    return None;
                }
                wins.iter().find_map(|id| {
                    let w = self.windows.iter().find(|w| w.id == *id)?;
                    let entries = &self.launcher_entries;
                    entries.progress(pid(w.id), &w.app_id(), &self.app_index, |p| {
                        owners.contains(&p)
                    })
                })
            })
            .collect();
        let state = TaskbarState {
            start_pressed: bar.start_pressed,
            start_focus: bar.start_pressed && self.menu_keys,
            focus: self.bar_focus(bar),
            start_icon,
            orb_glow: self.config.glass.orb_glow,
            quick_icons,
            quick_hot: bar.quick_hot,
            quick_pressed: bar.quick_pressed,
            quick_drop: bar.quick_drop,
            quick_more: PopupCell {
                hot: bar.more_hot == Some(TaskbarHit::QuickMore),
                open: self.quick_more_open(bar.tree),
                held: false,
            },
            pager: pager.clone(),
            pager_hot: bar.pager_hot,
            pager_mark: self.config.pager_mark,
            start_hot: bar.start_glow.lit(),
            task_hot,
            task_pressed: bar.task_held,
            task_hot_x: bar.task_hot_x_drawn,
            task_shift,
            task_float,
            task_progress,
            show_desktop_hot: bar.show_desktop_hot,
            more: PopupCell {
                hot: bar.more_hot == Some(TaskbarHit::More),
                open: self.more_open(bar.tree),
                held: false,
            },
            chevron: PopupCell {
                hot: bar.chevron_hot,
                open: self.chevron_open(bar),
                held: bar.chevron_held,
            },
            tasks: entries,
            tray_icons,
            clock,
        };
        let layout = bar.layout.scaled(scale);
        let (text, start_text) = (
            self.text.for_output(scale),
            self.start_text.for_output(scale),
        );
        // The orb's own buffer: its rows past the bar, and held the whole
        // orb, which then stands over the menu unless a fullscreen window
        // covers the bar.
        let draw_orb = |state: &TaskbarState| {
            layout.start_orb(state.start_pressed).map(|r| {
                let mut c = Canvas::new(r.width as u32, r.height as u32);
                reprisekit::look::draw_start_above(&mut c, &theme, &layout, state);
                c
            })
        };
        let draw_bar = |state: &TaskbarState| {
            let mut canvas = Canvas::new(layout.width.max(1) as u32, layout.height as u32);
            reprisekit::look::draw_taskbar(&mut canvas, &theme, text, start_text, &layout, state);
            canvas
        };
        let draw = |state: &TaskbarState| (draw_bar(state), draw_orb(state));
        let held = state.start_pressed;
        let fading = !bar.start_glow.settled(Instant::now()) && base.look.style().is_some();
        if fading {
            let mut state = state;
            state.start_hot = false;
            let dark = draw(&state);
            state.start_hot = true;
            let lit = draw(&state);
            let changed = dark.0.differing_rect(&lit.0);
            bar.start_mix = Some(StartMix {
                dark,
                lit,
                held,
                scale,
                changed,
            });
            bar.drawn = None;
            self.redraw_start(bar);
        } else {
            bar.start_mix = None;
            let canvas = match bar.drawn.take() {
                Some(mut d)
                    if d.scale == scale && d.layout == layout && d.state.same_but_start(&state) =>
                {
                    #[cfg(test)]
                    {
                        bar.start_only_draws += 1;
                    }
                    reprisekit::look::redraw_taskbar_start(
                        &mut d.canvas,
                        &theme,
                        text,
                        start_text,
                        &layout,
                        &state,
                    );
                    d.canvas
                }
                _ => draw_bar(&state),
            };
            let orb = draw_orb(&state);
            bar.orb_up = self.upload_bar(bar, &canvas, orb, held, scale);
            bar.drawn = Some(DrawnBar {
                state,
                layout,
                scale,
                canvas,
            });
        }
        crate::trayreveal::follow_layout(self, bar, now);
        bar.tray_reveal_drawn = bar.tray_reveal.x(now);
        self.scene.set_enabled(bar.tree, true);
        self.place_bar(bar);
    }

    pub(crate) fn place_bar(&mut self, bar: &mut Bar) {
        let p = self.bar_drawn(&bar.output);
        let rows = crate::barplace::on_output(&p, self.layout_box(Some(&bar.output)));
        let orb = crate::barplace::orb_opacity(self.bar_hide.drawn(&bar.output));
        let scale = bar.buffer_scale;
        let wanted = glass::wanted(&self.config);
        let scene = &mut self.scene;
        scene.set_position(bar.tree, p.shown.x, p.shown.y);
        scene.set_position(bar.orb_tree, p.shown.x, p.shown.y);
        scene.set_position(bar.buffer, rows.x, rows.y);
        let whole = rows == Rect::new(0, 0, bar.layout.width, bar.layout.height);
        let source = (!whole).then(|| {
            smithay::utils::Rectangle::new(
                (f64::from(rows.x * scale), f64::from(rows.y * scale)).into(),
                (
                    f64::from(rows.width * scale),
                    f64::from(rows.height * scale),
                )
                    .into(),
            )
        });
        scene.buffer_set_source_box(bar.buffer, source);
        scene.buffer_set_dest_size(bar.buffer, Some((rows.width, rows.height).into()));
        scene.set_enabled(bar.buffer, !rows.is_empty());
        let up = bar.orb_up.filter(|_| orb.is_some());
        scene.set_enabled(bar.start_above, up.is_some());
        scene.set_opacity(bar.start_above, orb.unwrap_or(0.0));
        scene.set_enabled(bar.orb_tree, up == Some(true));
        let pane = (wanted && !rows.is_empty()).then(|| Pane {
            shapes: vec![PaneShape {
                rect: rows,
                corner: &[],
                square_bottom: false,
                square_top: false,
            }],
            holes: Vec::new(),
        });
        bar.glass
            .set(&mut self.scene, &self.glass, bar.glass_tree, pane);
        crate::backend::schedule(self, &bar.output);
    }

    fn redraw_start(&mut self, bar: &mut Bar) {
        let Some(m) = &bar.start_mix else { return };
        let now = Instant::now();
        let l = bar.start_glow.level(now);
        let mut canvas = m.dark.0.clone();
        canvas.mix(&m.lit.0, m.changed, l);
        let orb = m.dark.1.as_ref().zip(m.lit.1.as_ref()).map(|(d, h)| {
            let mut c = d.clone();
            c.mix(h, c.bounds(), l);
            c
        });
        let (held, scale) = (m.held, m.scale);
        bar.orb_up = self.upload_bar(bar, &canvas, orb, held, scale);
        self.place_bar(bar);
        if bar.start_glow.settled(now) {
            bar.start_mix = None;
        }
    }

    pub fn start_fade_tick(&mut self, tree: TreeId) -> bool {
        self.with_bar(tree, |s, bar| {
            s.redraw_start(bar);
            bar.start_mix.is_some()
        })
        .unwrap_or(false)
    }

    fn upload_bar(
        &mut self,
        bar: &mut Bar,
        canvas: &Canvas,
        orb: Option<Canvas>,
        held: bool,
        scale: i32,
    ) -> Option<bool> {
        let covered = self.under_fullscreen(bar);
        let scene = &mut self.scene;
        upload_scaled(scene, bar.buffer, canvas, scale);
        bar.buffer_scale = scale;
        let l = &bar.layout;
        let over_menu = stands_over_menu(l, held, covered);
        let parent = if over_menu { bar.orb_tree } else { bar.tree };
        if scene.parent(bar.start_above) != Some(parent) {
            scene.reparent(bar.start_above, parent);
        }
        let pictured = l.start_orb(held).is_some() && orb.is_some();
        match (l.start_orb(held), orb) {
            (Some(r), Some(c)) => {
                upload_scaled(scene, bar.start_above, &c, scale);
                scene.set_position(bar.start_above, r.x, r.y);
                scene.set_enabled(bar.start_above, true);
                // Only Start takes the pointer, so a click beside the disc or on its
                // glow reaches what lies beneath.
                let layout = l.clone();
                scene.set_input(
                    bar.start_above,
                    Input::Test(Box::new(move |p| {
                        let (x, y) = (p.x.floor() as i32 + r.x, p.y.floor() as i32 + r.y);
                        layout.hit(x, y) == TaskbarHit::Start
                    })),
                );
            }
            _ => scene.set_enabled(bar.start_above, false),
        }
        scene.set_enabled(bar.orb_tree, over_menu);
        pictured.then_some(over_menu)
    }

    pub fn taskbar_hover(&mut self, tree: Option<TreeId>, hit: TaskbarHit) {
        let area = tree.filter(|&t| {
            matches!(hit, TaskbarHit::Tray(_) | TaskbarHit::Chevron)
                || self.bar(t).is_some_and(|b| {
                    let p = self.pointer_location();
                    let (x, y) = self.bar_local(b, p.x, p.y);
                    b.layout.notification.contains(x, y)
                })
        });
        crate::trayreveal::hover(self, area);
        let none = (false, None, None, None, false, None, false);
        let (fade_in, fade_out) = self.config.effects.hover_ms();
        let now = Instant::now();
        let hot_x = match (hit, tree.and_then(|t| self.bar(t))) {
            (TaskbarHit::Task(i), Some(bar)) => self.task_pointer_x(bar, i),
            _ => None,
        };
        let mut changed = Vec::new();
        let mut moved = Vec::new();
        let (mut strip_was, mut strip_now) = (false, false);
        for bar in &mut self.bars {
            bar.task_hot_x = if Some(bar.tree) == tree { hot_x } else { None };
            let want = if Some(bar.tree) != tree {
                none
            } else {
                match hit {
                    TaskbarHit::Start => (true, None, None, None, false, None, false),
                    TaskbarHit::Quick(i) => (false, Some(i), None, None, false, None, false),
                    TaskbarHit::Pager(i) => (false, None, Some(i), None, false, None, false),
                    TaskbarHit::Task(i) => {
                        let key =
                            crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i);
                        (false, None, None, key, false, None, false)
                    }
                    TaskbarHit::ShowDesktop => (false, None, None, None, true, None, false),
                    TaskbarHit::More | TaskbarHit::QuickMore => {
                        (false, None, None, None, false, Some(hit), false)
                    }
                    TaskbarHit::Chevron => (false, None, None, None, false, None, true),
                    _ => none,
                }
            };
            let was = (
                bar.start_hot,
                bar.quick_hot,
                bar.pager_hot,
                bar.task_hot,
                bar.show_desktop_hot,
                bar.more_hot,
                bar.chevron_hot,
            );
            strip_was |= was.4;
            strip_now |= want.4;
            if was != want {
                (
                    bar.start_hot,
                    bar.quick_hot,
                    bar.pager_hot,
                    bar.task_hot,
                    bar.show_desktop_hot,
                    bar.more_hot,
                    bar.chevron_hot,
                ) = want;
                bar.start_glow.set(want.0, now, fade_in, fade_out);
                changed.push(bar.tree);
            } else {
                moved.push(bar.tree);
            }
        }
        for t in changed {
            self.redraw_taskbar(t);
            self.watch_bar(t);
        }
        for t in moved {
            self.spot_moved(t);
        }
        if strip_was != strip_now {
            crate::peek::show_desktop_hover(self, strip_now);
        }
    }

    /// Hover the bars afresh where the pointer rests: their hot cells are
    /// the last layout's, and a bar moved or relaid under a still pointer
    /// would keep one lit until the next motion.
    pub(crate) fn rehover_bars(&mut self) {
        if self.pointer_free() {
            let p = self.pointer_location();
            self.repoint(p, smithay::backend::input::InputTime::now());
        } else {
            self.taskbar_hover(None, TaskbarHit::Nothing);
        }
    }

    pub fn taskbar_press(
        &mut self,
        tree: TreeId,
        hit: TaskbarHit,
        button: u32,
        at: (i32, i32),
        shift: bool,
        closed: Option<MenuRoot>,
    ) {
        let pinned = self.config.taskbar.pinned;
        match hit {
            TaskbarHit::Task(i) => {
                if let Some(id) = self.task_button_dialog(tree, i) {
                    if button == BTN_LEFT {
                        self.task_dialog_click(id);
                    }
                    return;
                }
                if pinned && button == BTN_MIDDLE {
                    self.task_button_launch(tree, i);
                    return;
                }
                if button == BTN_RIGHT {
                    self.task_button_menu(tree, i, at, shift, closed);
                    return;
                }
                if button != BTN_LEFT {
                    return;
                }
                match crate::taskbar::press_timing(pinned) {
                    crate::taskbar::PressTiming::OnRelease => {
                        let p = self.pointer_location();
                        let held = self.bar(tree).and_then(|b| {
                            let key = crate::taskbar::task_key(&b.buttons, &b.pins, &b.dialogs, i)?;
                            let slot = b.layout.tasks.get(i).map_or(0, |r| b.layout.to_along(*r).x);
                            let grab = self.bar_along(b, f64::from(at.0), f64::from(at.1)) - slot;
                            Some((key, grab, b.layout.along_f(p.x, p.y)))
                        });
                        if let Some((key, grab, along0)) = held {
                            self.interaction = Interaction::TaskPress {
                                bar: tree,
                                key,
                                along0,
                                grab,
                                dragging: false,
                            };
                            self.redraw_taskbar(tree);
                        }
                    }
                    crate::taskbar::PressTiming::OnPress => {
                        let list_was_open = closed == Some(MenuRoot::WindowList(i));
                        self.task_button_click(tree, i, list_was_open);
                    }
                }
            }
            TaskbarHit::Start if button == BTN_LEFT => self.toggle_start(tree),
            TaskbarHit::Quick(index) if button == BTN_LEFT => {
                let icon_px = self.config.theme.icon_size();
                let grab = self.bar(tree).map_or((0, 0), |b| {
                    let Some(r) = b.layout.quick.get(index) else {
                        return (0, 0);
                    };
                    let (ix, iy) = crate::taskbar::quick_icon_origin(*r, icon_px, 1);
                    let (lx, ly) = self.bar_local(b, f64::from(at.0), f64::from(at.1));
                    (lx - ix, ly - iy)
                });
                let p = self.pointer_location();
                self.interaction = Interaction::QuickPress {
                    bar: tree,
                    index,
                    p0: (p.x, p.y),
                    grab,
                    dragging: false,
                };
                if let Some(b) = self.bar_mut(tree) {
                    b.quick_pressed = Some(index);
                }
                self.redraw_taskbar(tree);
            }
            TaskbarHit::Quick(index) if button == BTN_RIGHT => self.open_quick_menu(index, at),
            TaskbarHit::Nothing | TaskbarHit::Resize | TaskbarHit::Clock | TaskbarHit::Pager(_)
                if button == BTN_RIGHT =>
            {
                self.open_taskbar_menu(at)
            }
            TaskbarHit::ShowDesktop if button == BTN_LEFT => self.toggle_show_desktop(),
            TaskbarHit::More if button == BTN_LEFT => {
                if closed != Some(MenuRoot::TaskOverflow(tree)) {
                    self.open_task_overflow(tree);
                }
            }
            TaskbarHit::QuickMore if button == BTN_LEFT => {
                if closed != Some(MenuRoot::QuickOverflow(tree)) {
                    self.open_quick_overflow(tree);
                }
            }
            TaskbarHit::Pager(i) if button == BTN_LEFT => self.switch_desktop(i),
            TaskbarHit::Grip(i) if button == BTN_LEFT => {
                let Some(b) = self.bar(tree) else { return };
                let Some(g) = b.layout.grips.get(i) else {
                    return;
                };
                let dx =
                    self.bar_along(b, f64::from(at.0), f64::from(at.1)) - b.layout.to_along(*g).x;
                let band = b.layout.band_at_grip(i);
                let edge = b.layout.edge;
                self.interaction = Interaction::Band {
                    bar: tree,
                    band,
                    dx,
                };
                self.set_cursor(crate::taskbar::grip_cursor(edge));
            }
            TaskbarHit::Nothing if button == BTN_LEFT && !self.bar_shared.locked => {
                self.begin_bar_move(tree)
            }
            TaskbarHit::Resize if button == BTN_LEFT => self.begin_bar_width(tree),
            _ => {}
        }
    }

    fn task_button_dialog(&self, tree: TreeId, i: usize) -> Option<u64> {
        self.bar(tree)?.dialogs.get(i).copied().flatten()
    }

    fn task_button_menu(
        &mut self,
        tree: TreeId,
        i: usize,
        at: (i32, i32),
        shift: bool,
        closed: Option<MenuRoot>,
    ) {
        if self.config.taskbar.pinned && !shift {
            if closed != Some(MenuRoot::JumpList(i)) {
                self.open_jump_list(tree, i, at);
            }
            return;
        }
        let Some(wins) = self.bar(tree).and_then(|b| b.buttons.get(i).cloned()) else {
            return;
        };
        match wins.as_slice() {
            [] => {}
            [w] => {
                if closed != Some(MenuRoot::Window(*w)) {
                    self.open_window_menu(*w, at, true);
                }
            }
            _ => {
                if closed != Some(MenuRoot::WindowList(i)) {
                    self.open_window_list(tree, i);
                }
            }
        }
    }

    pub fn taskbar_press_moved(&mut self) {
        let p = self.pointer_location();
        match self.interaction {
            Interaction::Band {
                bar: tree,
                band,
                dx,
            } => {
                let Some(b) = self.bar(tree) else { return };
                let at = self.bar_along(b, p.x, p.y);
                let next = b.layout.drag_band(self.bar_shared.bands, band, at - dx);
                if next != self.bar_shared.bands {
                    self.bar_shared.bands = next;
                    self.redraw_taskbars();
                }
            }
            Interaction::TaskPress {
                bar: tree,
                key,
                along0,
                grab,
                dragging,
            } => {
                if !dragging && let Some(b) = self.bar(tree) {
                    let held = self.held_task(b);
                    let x = held.and_then(|i| self.task_pointer_x(b, i));
                    let changed = held != b.task_held;
                    if let Some(b) = self.bar_mut(tree) {
                        b.task_hot_x = x;
                    }
                    if changed {
                        self.redraw_taskbar(tree);
                    } else {
                        self.spot_moved(tree);
                    }
                }
                let Some((button, along, at)) = self.bar(tree).and_then(|b| {
                    let i = crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key)?;
                    Some((i, b.layout.along_f(p.x, p.y), self.bar_along(b, p.x, p.y)))
                }) else {
                    return;
                };
                let moved = (along - along0).abs() >= DRAG_PX;
                if !crate::taskbar::press_drags(dragging, moved) {
                    return;
                }
                let button = self.row_drag(tree, button, at);
                let key = self
                    .bar(tree)
                    .and_then(|b| crate::taskbar::task_key(&b.buttons, &b.pins, &b.dialogs, button))
                    .unwrap_or(key);
                self.interaction = Interaction::TaskPress {
                    bar: tree,
                    key,
                    along0,
                    grab,
                    dragging: true,
                };
                self.slide_follow(tree, key, grab);
                if !dragging {
                    if let Some(b) = self.bar_mut(tree) {
                        b.task_hot_x = None;
                    }
                    self.redraw_taskbar(tree);
                }
            }
            Interaction::ArrowPress { bar } => crate::trayreveal::press_moved(self, bar),
            Interaction::QuickPress {
                bar: tree,
                index,
                p0,
                grab,
                dragging,
            } => {
                let Some(b) = self.bar(tree) else { return };
                let moved = b.layout.quick_drag(p.x - p0.0, p.y - p0.1);
                if !dragging && (self.bar_shared.locked || moved < DRAG_PX) {
                    let want =
                        (self.bar_hit(b, p.x, p.y) == TaskbarHit::Quick(index)).then_some(index);
                    if b.quick_pressed != want {
                        if let Some(b) = self.bar_mut(tree) {
                            b.quick_pressed = want;
                        }
                        self.redraw_taskbar(tree);
                    }
                    return;
                }
                let (lx, ly) = self.bar_local(b, p.x, p.y);
                let slot = b.layout.drop_slot(lx, ly);
                let output = b.output.clone();
                self.interaction = Interaction::QuickPress {
                    bar: tree,
                    index,
                    p0,
                    grab,
                    dragging: true,
                };
                if !dragging {
                    self.quick_ghost_start(&output, index);
                }
                if let Some(ghost) = self.bar_shared.quick_ghost {
                    self.scene
                        .set_position(ghost, p.x as i32 - grab.0, p.y as i32 - grab.1);
                    self.redraw_all();
                }
                let Some(b) = self.bar_mut(tree) else { return };
                if b.quick_pressed.is_some() || b.quick_drop != Some(slot) || b.quick_hot.is_some()
                {
                    b.quick_pressed = None;
                    b.quick_hot = None;
                    b.quick_drop = Some(slot);
                    self.redraw_taskbar(tree);
                }
            }
            _ => {}
        }
    }

    pub fn end_taskbar_press(&mut self) {
        match self.interaction {
            Interaction::Band { .. } => {
                self.interaction = Interaction::Idle;
                self.save_taskbar_state();
            }
            Interaction::TaskPress {
                bar: tree,
                dragging: true,
                ..
            } => {
                self.interaction = Interaction::Idle;
                self.slide_release(tree);
                self.quick_store_order();
            }
            Interaction::TaskPress { bar: tree, .. } => {
                let held = self.bar(tree).and_then(|b| self.held_task(b));
                self.interaction = Interaction::Idle;
                self.redraw_taskbar(tree);
                if let Some(i) = held {
                    self.task_button_click(tree, i, false);
                }
            }
            Interaction::QuickPress {
                bar: tree,
                index,
                dragging,
                ..
            } => {
                self.interaction = Interaction::Idle;
                self.quick_ghost_end();
                let p = self.pointer_location();
                let on = self
                    .bar(tree)
                    .is_some_and(|b| self.bar_hit(b, p.x, p.y) == TaskbarHit::Quick(index));
                let slot = self.bar_mut(tree).and_then(|b| {
                    b.quick_pressed = None;
                    b.quick_drop.take()
                });
                self.redraw_taskbar(tree);
                if dragging {
                    if let Some(slot) = slot {
                        self.quick_move(index, slot);
                    }
                } else if on {
                    self.run_quick(index);
                }
            }
            Interaction::ArrowPress { bar } => {
                self.interaction = Interaction::Idle;
                crate::trayreveal::release(self, bar);
            }
            _ => {}
        }
    }

    pub fn cancel_taskbar_press(&mut self, was: Interaction) {
        match was {
            Interaction::TaskPress {
                bar: tree,
                dragging: true,
                ..
            } => {
                self.slide_release(tree);
                self.quick_store_order();
            }
            Interaction::TaskPress { bar: tree, .. } => self.redraw_taskbar(tree),
            Interaction::QuickPress { bar: tree, .. } => {
                self.quick_ghost_end();
                if let Some(b) = self.bar_mut(tree) {
                    b.quick_pressed = None;
                    b.quick_drop = None;
                }
                self.redraw_taskbar(tree);
            }
            Interaction::ArrowPress { bar } => crate::trayreveal::set_held(self, bar, false),
            Interaction::Band { .. } => self.save_taskbar_state(),
            _ => {}
        }
    }

    pub fn save_taskbar_state(&self) {
        let s = &self.bar_shared;
        crate::state::save_taskbar(s.locked, s.bands, s.quick_shown);
    }

    pub fn toggle_taskbar_lock(&mut self) {
        self.bar_shared.locked = !self.bar_shared.locked;
        self.save_taskbar_state();
        self.redraw_taskbars();
    }

    pub fn toggle_quick_shown(&mut self) {
        self.bar_shared.quick_shown = !self.bar_shared.quick_shown;
        self.save_taskbar_state();
        self.redraw_taskbars();
    }

    fn row_drag(&mut self, tree: TreeId, i: usize, at: i32) -> usize {
        let Some(bar) = self.bar(tree) else { return i };
        let Some(key) = crate::taskbar::task_key(&bar.buttons, &bar.pins, &bar.dialogs, i) else {
            return i;
        };
        let slot = crate::taskbar::row_drop_slot(
            &bar.layout.along_rects(&bar.layout.tasks),
            &bar.pins,
            &bar.runs,
            i,
            at,
        );
        let Some((order, anchors)) = crate::taskbar::reorder_row(&bar.pins, &bar.runs, i, slot)
        else {
            return i;
        };
        let before = taskslide::row(bar);
        // The buttons are from the last draw; a window or dialog may have closed.
        let moved: Vec<(Vec<WindowId>, Option<u64>, usize)> = anchors
            .iter()
            .map(|(u, anchor)| {
                let wins = u
                    .clone()
                    .filter_map(|b| bar.buttons.get(b))
                    .flatten()
                    .copied()
                    .collect();
                (wins, bar.dialogs.get(u.start).copied().flatten(), *anchor)
            })
            .collect();
        if order.iter().enumerate().any(|(n, &k)| n != k) {
            let mut old: Vec<Option<QuickItem>> = std::mem::take(&mut self.quicklaunch)
                .into_iter()
                .map(Some)
                .collect();
            self.quicklaunch = order.iter().filter_map(|&k| old[k].take()).collect();
            self.bar_shared.quick_moved = true;
            self.pins_reordered(&order);
        }
        let live = self.task_windows();
        for (wins, dialog, anchor) in moved {
            let seq = crate::taskbar::next_seq();
            // Each window its own seq, in row order, so a run keeps its order.
            for (n, w) in wins.into_iter().filter(|w| live.contains(w)).enumerate() {
                let seq = if n == 0 {
                    seq
                } else {
                    crate::taskbar::next_seq()
                };
                if let Some(win) = self.win_mut(w) {
                    win.anchor = anchor;
                    win.seq = seq;
                }
            }
            if let Some(v) =
                dialog.and_then(|id| self.dialogs.views.iter_mut().find(|v| v.id == id))
            {
                v.anchor = anchor;
                v.seq = seq;
            }
        }
        self.redraw_taskbars();
        // A pin's index is its place in the old order; name it by the new one.
        let Some(key) = crate::taskbar::pin_renamed(key, &order) else {
            return i;
        };
        self.slide_row_moved(tree, &before, &order, key);
        self.bar(tree)
            .and_then(|b| crate::taskbar::find_task(&b.buttons, &b.pins, &b.dialogs, &key))
            .unwrap_or(i)
    }

    fn pins_reordered(&mut self, order: &[usize]) {
        for b in &mut self.bars {
            crate::taskbar::pins_renamed(
                &mut b.task_hot,
                &mut b.task_float,
                &mut b.task_slides,
                order,
            );
        }
        if let Interaction::TaskPress { key, .. } = &mut self.interaction
            && let Some(k) = crate::taskbar::pin_renamed(*key, order)
        {
            *key = k;
        }
    }

    /// The callers redraw the bars afterwards.
    pub fn end_task_press_for(&mut self, w: WindowId) {
        let Interaction::TaskPress {
            key: TaskKey::Window(x),
            dragging,
            ..
        } = self.interaction
        else {
            return;
        };
        if x != w {
            return;
        }
        self.interaction = Interaction::Idle;
        if dragging {
            for b in &mut self.bars {
                taskslide::forget(b, TaskKey::Window(w));
            }
            self.quick_store_order();
        }
    }

    fn quick_move(&mut self, from: usize, slot: usize) {
        if crate::taskbar::move_button(&mut self.quicklaunch, from, slot) {
            self.bar_shared.quick_moved = true;
            self.quick_store_order();
            self.redraw_taskbars();
        }
    }

    /// The folder changed under us (`quickwatch`): take the buttons and
    /// the order file as they are now (`quicklaunch::reread`), keeping a
    /// move of our own not yet written.
    pub fn quick_reread(&mut self, dir: &std::path::Path) {
        let items = crate::quicklaunch::reread(
            dir,
            &self.config.terminal,
            &self.quicklaunch,
            self.bar_shared.quick_moved,
        );
        if items == self.quicklaunch {
            return;
        }
        // The held index may name another button now: let go, dropping nothing.
        if matches!(self.interaction, Interaction::QuickPress { .. }) {
            self.cancel_interaction();
        }
        self.quicklaunch = items;
        self.redraw_taskbars();
    }

    /// Write the Quick Launch order to the folder's order file after a
    /// move of ours, on the state writer's thread since it fsyncs; the
    /// order file is otherwise left to whoever wrote it last.
    pub fn quick_store_order(&mut self) {
        if !std::mem::take(&mut self.bar_shared.quick_moved) {
            return;
        }
        let Some(dir) = reprisefs::quicklaunch::dir() else {
            return;
        };
        let order: Vec<String> = self.quicklaunch.iter().map(|q| q.file.clone()).collect();
        crate::state::run_in_order(move || {
            if let Err(e) = reprisefs::quicklaunch::write_order(&dir, &order) {
                log::warn!("quicklaunch: could not write the order: {e}");
            }
        });
    }

    pub fn quick_order_changed(&mut self) {
        self.bar_shared.quick_moved = true;
        self.quick_store_order();
    }

    fn quick_ghost_start(&mut self, output: &Output, index: usize) {
        self.quick_ghost_end();
        let scale = self.draw_scale_at(Some(output)).max(1);
        let theme = self.config.theme.scaled(scale);
        let icon_px = theme.icon_size();
        let Some(item) = self.quicklaunch.get(index) else {
            return;
        };
        let mut canvas = quick_icon(&mut self.icons, item, icon_px).map_or_else(
            || reprisekit::icon::chrome_fallback(&theme, icon_px),
            Arc::unwrap_or_clone,
        );
        canvas.fade(1, 2);
        let buffer = self.scene.buffer_create(self.layers.overlay);
        self.scene.set_input(buffer, Input::Passthrough);
        upload_scaled(&mut self.scene, buffer, &canvas, scale);
        self.bar_shared.quick_ghost = Some(buffer);
    }

    fn quick_ghost_end(&mut self) {
        if let Some(b) = self.bar_shared.quick_ghost.take() {
            self.scene.destroy(b);
            self.redraw_all();
        }
    }

    pub fn task_button_click(&mut self, tree: TreeId, i: usize, list_was_open: bool) {
        if let Some(id) = self.task_button_dialog(tree, i) {
            self.task_dialog_click(id);
            return;
        }
        let Some(wins) = self.bar(tree).and_then(|b| b.buttons.get(i).cloned()) else {
            return;
        };
        match wins.as_slice() {
            [] => self.task_button_launch(tree, i),
            [w] => {
                let w = *w;
                let Some(win) = self.win(w) else { return };
                if win.minimized {
                    self.set_minimized(w, false);
                } else if self.focused == Some(w) {
                    self.set_minimized(w, true);
                } else {
                    self.focus_window(w);
                }
            }
            _ if !self.config.taskbar.pinned => {
                if !list_was_open {
                    self.open_window_list(tree, i);
                }
            }
            wins => {
                let live = self.task_windows();
                let wins: Vec<WindowId> =
                    wins.iter().copied().filter(|w| live.contains(w)).collect();
                if let Some(w) =
                    crate::taskbar::group_click_pick(&wins, &self.focus_order, self.focused)
                {
                    self.bring_up(w);
                }
            }
        }
    }

    pub(crate) fn bring_up(&mut self, w: WindowId) {
        if self.win(w).is_some_and(|x| x.minimized) {
            self.set_minimized(w, false);
        } else {
            self.focus_window(w);
        }
    }

    fn task_button_launch(&mut self, tree: TreeId, i: usize) {
        if let Some(k) = self
            .bar(tree)
            .and_then(|b| b.pins.get(i).copied().flatten())
        {
            self.run_quick(k);
        }
    }

    pub fn run_quick(&mut self, k: usize) {
        let Some(action) = self.quicklaunch.get(k).map(|q| q.entry.action.clone()) else {
            return;
        };
        self.run_action(&action, 0);
    }

    pub fn forget_task_window(&mut self, w: WindowId) {
        for bar in &mut self.bars {
            if bar.task_hot == Some(TaskKey::Window(w)) {
                bar.task_hot = None;
            }
            taskslide::forget(bar, TaskKey::Window(w));
        }
    }

    pub fn taskbar_scene_hit(&self, tree: TreeId, x: f64, y: f64) -> Hit {
        let hit = self
            .bar(tree)
            .map_or(TaskbarHit::Nothing, |b| self.bar_hit(b, x, y));
        Hit::Taskbar { bar: tree, hit }
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
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn a_full_glow_cache_drops_the_least_recently_used_icon() {
        static WORKED_OUT: AtomicUsize = AtomicUsize::new(0);
        fn colour(_: &Canvas) -> Option<Color> {
            WORKED_OUT.fetch_add(1, Ordering::Relaxed);
            None
        }
        let glow = TaskGlow {
            colour,
            label_edges: None,
        };
        let icon = |i: usize| {
            let mut c = Canvas::new(1, 1);
            c.pixels[0] = i as u32;
            c
        };
        let ask = |i: usize| {
            let before = WORKED_OUT.load(Ordering::Relaxed);
            glow_colour(glow, &icon(i));
            WORKED_OUT.load(Ordering::Relaxed) > before
        };
        for i in 0..GLOW_KEEP {
            assert!(ask(i), "icon {i} is new");
        }
        assert!(!ask(0), "the first icon is kept");
        assert!(ask(GLOW_KEEP), "one past the cap");
        assert!(!ask(0), "the first icon, asked for since, is still kept");
        assert!(ask(1), "the second, the least recently used, went");
    }

    #[test]
    fn a_top_bars_held_orb_stands_over_the_menu_below_the_bar() {
        use reprisekit::taskbar::Edge;
        let style = reprisekit::glass::small_bar_style(&Default::default());
        let theme = reprisekit::interfaces::theme_for(reprisekit::interface::Interface::Glass);
        let m = Metrics {
            taskbar_height: reprisekit::glass::SMALL_TASKBAR_HEIGHT,
            ..theme.metrics
        };
        let l = TaskbarLayout::compute(&m, &style.taskbar, 800, true, TaskbarCounts::default(), 40)
            .on_edge(Edge::Top);
        assert_eq!(
            l.start_orb(false),
            Some(Rect::new(0, 30, 54, 13)),
            "at rest the orb's buffer is the 13 rows below the bar"
        );
        assert!(
            !stands_over_menu(&l, false, false),
            "at rest it stays with the bar"
        );
        assert!(
            stands_over_menu(&l, true, false),
            "held it stands over the menu"
        );
        assert!(
            !stands_over_menu(&l, true, true),
            "under a fullscreen window it stays with the bar"
        );
    }

    #[test]
    fn a_bar_moved_under_a_still_pointer_lights_only_the_cell_under_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.pointer_to(4.0, 764.0);
        assert!(h.state.bars[0].start_hot, "Start under the pointer");
        let mut top = h.state.config.clone();
        top.taskbar.position = reprisekit::taskbar::Edge::Top;
        crate::configwatch::reload(&mut h.state, top);
        h.dispatch();
        assert!(
            !h.state.bars[0].start_hot,
            "Start moved out from under the pointer"
        );
        h.pointer_to(4.0, 4.0);
        let mut bottom = h.state.config.clone();
        bottom.taskbar.position = reprisekit::taskbar::Edge::Bottom;
        crate::configwatch::reload(&mut h.state, bottom);
        h.dispatch();
        assert!(!h.state.bars[0].start_hot, "Start moved away again");
        let mut top = h.state.config.clone();
        top.taskbar.position = reprisekit::taskbar::Edge::Top;
        crate::configwatch::reload(&mut h.state, top);
        h.dispatch();
        assert!(
            h.state.bars[0].start_hot,
            "Start moved in under the still pointer"
        );
    }
}
