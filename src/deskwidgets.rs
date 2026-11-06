//! The desktop widgets on the scene: a tree each in `layers.background`, on its output's icon
//! grid, placed and kept in `store`, redrawn by one timer when what they show changes.

mod store;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reprisekit::access::Tree;
use reprisekit::access::accesskit::Role;
use reprisekit::controls::calendar::Calendar;
use reprisekit::controls::deskwidget::{
    self, Clock, Content, CpuTimes, Face, Kind, Meter, MeterStyle, Spot,
};
use reprisekit::look::panel;
use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::utils::Rectangle;

use crate::compositor::{SceneOwner, State};
use crate::desktop;
use crate::desktopview::{DesktopCommand, entry};
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::output::upload_scaled;
use crate::raster::Rect;
use crate::startmenu::MenuItem;
use crate::theme::Theme;

pub use store::Store;

/// `State::widgets`.
#[derive(Default)]
pub struct DeskWidgets {
    store: Store,
    shown: Vec<Shown>,
    /// The meter's samples, kept while it is hidden or removed.
    meter: Meter,
    cpu_before: Option<(Instant, CpuTimes)>,
    timer: Option<RegistrationToken>,
    /// The config theme the widgets were drawn from, and whether glass lay under them; a change
    /// redraws them all.
    drawn_theme: Option<(Theme, bool)>,
}

struct Shown {
    kind: Kind,
    /// In `layers.background`, tagged `SceneOwner::DeskWidget`: the glass tree, then the buffer.
    tree: TreeId,
    glass_tree: TreeId,
    glass: Backdrop,
    buffer: BufferId,
    output: Output,
    spot: Spot,
    /// The panel in layout coordinates.
    panel: Rect,
    /// What the buffer shows, at which scale.
    drawn: Option<(Content, i32)>,
}

/// A widget held by the left button: its grab offset from the panel's corner, and whether the
/// pointer has gone `desktop::DRAG_PX` yet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub kind: Kind,
    pub grab: (f64, f64),
    pub from: (f64, f64),
    pub moved: bool,
}

/// Samples `/proc` at most this often, however often the timer runs.
const SAMPLE: Duration = Duration::from_millis(900);

/// The timer's wait while every widget's desktop is covered: how soon one shows its news again.
const COVERED_POLL: Duration = Duration::from_secs(1);

fn since_epoch() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn local_now() -> libc::tm {
    reprisefs::locale::localtime(since_epoch().as_secs() as i64)
}

/// The clock's content now: the analog face's hands, or the digital face's time and long date in
/// the LC_TIME locale.
fn clock_now(store: &Store) -> Clock {
    let tm = local_now();
    let field = |v: i32| v.max(0) as u32;
    let time = (field(tm.tm_hour), field(tm.tm_min), field(tm.tm_sec));
    let (text, date) = match store.face {
        Face::Analog => (String::new(), String::new()),
        Face::Digital => {
            let fmt = match (reprisefs::locale::is_12h(), store.seconds) {
                (true, true) => c"%l:%M:%S %p",
                (true, false) => c"%l:%M %p",
                (false, true) => c"%H:%M:%S",
                (false, false) => c"%H:%M",
            };
            let text = reprisefs::locale::strftime(&tm, fmt).trim().to_string();
            (text, crate::taskbar::date_text())
        }
    };
    Clock::new(store.face, store.seconds, time, &text, &date)
}

fn calendar_now() -> Calendar {
    let tm = local_now();
    let (y, m, d) = (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32);
    let mut cal = Calendar::new(y, m, None, reprisefs::locale::first_weekday());
    cal.today = Some((y, m, d));
    cal
}

/// The rows of a widget's menu: its options, then Remove.
pub fn widget_menu(kind: Kind, store: &Store) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    use DesktopCommand as Dc;
    let radio = |label: &str, on: bool| entry(label).radio(true).checked(on);
    let mut rows: Vec<(MenuItem, Option<Dc>)> = match kind {
        Kind::Clock => vec![
            (
                radio("&Analog", store.face == Face::Analog),
                Some(Dc::ClockFace(Face::Analog)),
            ),
            (
                radio("&Digital", store.face == Face::Digital),
                Some(Dc::ClockFace(Face::Digital)),
            ),
            (MenuItem::Separator, None),
            (
                entry("Show &Seconds").checked(store.seconds),
                Some(Dc::ClockSeconds),
            ),
        ],
        Kind::Meter => vec![
            (
                radio("&Bars", store.meter == MeterStyle::Bars),
                Some(Dc::MeterLook(MeterStyle::Bars)),
            ),
            (
                radio("&Graph", store.meter == MeterStyle::Graph),
                Some(Dc::MeterLook(MeterStyle::Graph)),
            ),
        ],
        Kind::Calendar => Vec::new(),
    };
    if !rows.is_empty() {
        rows.push((MenuItem::Separator, None));
    }
    rows.push((entry("&Remove"), Some(Dc::RemoveWidget(kind))));
    rows.into_iter().unzip()
}

/// The desktop menu's Widgets submenu: a check box row per widget.
pub fn widgets_menu(on: &[Kind]) -> (Vec<MenuItem>, Vec<Option<DesktopCommand>>) {
    Kind::ALL
        .into_iter()
        .map(|k| {
            let row = entry(k.label()).checked(on.contains(&k));
            (row, Some(DesktopCommand::ToggleWidget(k)))
        })
        .unzip()
}

/// The cells widgets standing at `spots` cover.
pub fn covered(spots: impl IntoIterator<Item = Spot>) -> HashSet<(i32, i32)> {
    spots.into_iter().flat_map(Spot::cells).collect()
}

/// A widget to stand on one output: its panel's size and its cell saved there, in 100% cells.
#[derive(Clone, Copy, Debug)]
pub struct Placing {
    pub kind: Kind,
    pub size: (i32, i32),
    pub saved: Option<(i32, i32)>,
}

/// Lays out `kinds` on one output's grid in order: each at its saved cell or the nearest spot off
/// earlier ones; a new one nearest the top-right, clear of `icons`. Returns each spot, and if new.
pub fn arrange(
    grid: desktop::Cell,
    area: Rect,
    kinds: &[Placing],
    icons: &HashSet<(i32, i32)>,
) -> Vec<(Spot, bool)> {
    let dims = deskwidget::dims(grid, area);
    let none = HashSet::new();
    let mut placed: Vec<Spot> = Vec::new();
    let mut out = Vec::new();
    for &Placing { size, saved, .. } in kinds {
        let span = deskwidget::span(grid, size);
        let (want, avoid) = match saved {
            Some(s) => (desktop::from_saved(grid, s), &none),
            None => ((dims.0 - span.0, 0), icons),
        };
        let cell = deskwidget::place(dims, span, want, &placed, avoid);
        let spot = Spot { cell, span };
        placed.push(spot);
        out.push((spot, saved.is_none()));
    }
    out
}

impl State {
    /// Read the store; the widgets come with the first layout.
    pub fn setup_widgets(&mut self) {
        self.widgets.store = Store::load();
        self.widgets.meter = Meter::new(self.widgets.store.meter);
    }

    /// The monitor ids of the outputs on the scene, by output.
    fn monitor_ids(&self) -> Vec<(Output, String)> {
        let outputs = self.scene.outputs();
        let refs: Vec<&Output> = outputs.iter().collect();
        let ids = crate::outputlayouts::monitor_ids(&refs);
        outputs.iter().cloned().zip(ids).collect()
    }

    fn widget_content(&self, kind: Kind) -> Content {
        match kind {
            Kind::Clock => Content::Clock(clock_now(&self.widgets.store)),
            Kind::Calendar => Content::Calendar(calendar_now()),
            Kind::Meter => {
                let mut m = self.widgets.meter.clone();
                m.style = self.widgets.store.meter;
                Content::Meter(m)
            }
        }
    }

    /// Whether the desktop under `output` is hidden: locked, the screen saver running, or a
    /// fullscreen window over it.
    fn desktop_covered(&self, output: &Output) -> bool {
        let b = self.layout_box(Some(output));
        self.locked_on(output) || crate::saverview::running(self) || self.fullscreen_on(b)
    }

    /// Bring the widgets in line with `desktop_widgets`, the outputs and the store, and redraw what
    /// changed. `relayout_desktop` runs it first, so the icons see where the widgets stand.
    pub fn lay_out_widgets(&mut self) {
        let wanted = self.config.desktop_widgets.clone();
        let gone: Vec<usize> = (self.widgets.shown.iter().enumerate())
            .filter(|(_, s)| !wanted.contains(&s.kind))
            .map(|(i, _)| i)
            .collect();
        for i in gone.into_iter().rev() {
            let s = self.widgets.shown.remove(i);
            self.drop_widget(s);
        }
        let ids = self.monitor_ids();
        let Some(primary) = self.first_output() else {
            return;
        };
        let icon_cells = self.icon_cells();
        let grid = self.desktop_grid();
        let theme = self.config.theme.clone();
        let cm = theme.controls();
        let mut saved_any = false;
        for (out, id) in &ids {
            let area = self.usable_area(Some(out));
            if area.width <= 0 {
                continue;
            }
            let home = |k: Kind| {
                let id_of = |o: &Output| ids.iter().find(|(x, _)| x == o).map(|p| &p.1);
                match self.widgets.store.homes.get(&k) {
                    Some(h) if ids.iter().any(|(_, i)| i == h) => h == id,
                    _ => id_of(&primary) == Some(id),
                }
            };
            let here: Vec<Placing> = (wanted.iter().copied())
                .filter(|&k| home(k))
                .map(|kind| Placing {
                    kind,
                    size: deskwidget::size(&self.widget_content(kind), &cm),
                    saved: (self.widgets.store.cells.get(&kind)).and_then(|c| c.get(id).copied()),
                })
                .collect();
            let none = HashSet::new();
            let icons = if *out == primary { &icon_cells } else { &none };
            for (p, (spot, new)) in here.iter().zip(arrange(grid, area, &here, icons)) {
                if new && !crate::virtualout::is_remote(out) {
                    let cells = self.widgets.store.cells.entry(p.kind).or_default();
                    cells.insert(id.clone(), desktop::to_saved(grid, spot.cell));
                    saved_any = true;
                }
                let panel = spot.panel(grid, area, p.size);
                self.stand_widget(p.kind, out, spot, panel);
            }
        }
        if saved_any {
            self.widgets.store.save();
        }
        let look = (theme, crate::glass::wanted(&self.config));
        if self.widgets.drawn_theme.as_ref() != Some(&look) {
            self.widgets.drawn_theme = Some(look);
            for i in 0..self.widgets.shown.len() {
                self.widgets.shown[i].drawn = None;
                let p = self.widgets.shown[i].panel;
                self.place_widget_glass(i, (p.width, p.height));
            }
        }
        self.refresh_widgets();
        self.arm_widget_timer();
    }

    /// Put `kind`'s tree at `panel` on `out`, making it if it is not standing yet.
    fn stand_widget(&mut self, kind: Kind, out: &Output, spot: Spot, panel: Rect) {
        let at = self.widgets.shown.iter().position(|s| s.kind == kind);
        let i = match at {
            Some(i) => i,
            None => {
                let scene = &mut self.scene;
                let tree = scene.tree_create(self.layers.background);
                scene.set_data(tree, Some(SceneOwner::DeskWidget(kind)));
                let glass_tree = scene.tree_create(tree);
                let buffer = scene.buffer_create(tree);
                self.widgets.shown.push(Shown {
                    kind,
                    tree,
                    glass_tree,
                    glass: Backdrop::new(&self.glass),
                    buffer,
                    output: out.clone(),
                    spot,
                    panel,
                    drawn: None,
                });
                crate::access::touch(self);
                self.widgets.shown.len() - 1
            }
        };
        let s = &mut self.widgets.shown[i];
        let moved = s.panel != panel || s.output != *out;
        if s.output != *out {
            s.drawn = None;
        }
        (s.output, s.spot, s.panel) = (out.clone(), spot, panel);
        if moved || at.is_none() {
            let (tree, size) = (s.tree, (panel.width, panel.height));
            let was = self.scene.outputs_of(tree);
            self.scene.set_position(tree, panel.x, panel.y);
            self.place_widget_glass(i, size);
            self.redraw_nodes(was, &[tree.into()]);
        }
    }

    /// The glass pane under widget `i`'s panel of `size` under a look with glass, else none.
    fn place_widget_glass(&mut self, i: usize, size: (i32, i32)) {
        let pane = match self.config.theme.glass() {
            Some(g) if crate::glass::wanted(&self.config) => Some(Pane {
                shapes: vec![PaneShape {
                    rect: Rect::new(0, 0, size.0, size.1),
                    corner: g.panel.corner,
                    square_bottom: false,
                    square_top: false,
                }],
                holes: Vec::new(),
            }),
            _ => None,
        };
        let s = &mut self.widgets.shown[i];
        s.glass
            .set(&mut self.scene, &self.glass, s.glass_tree, pane);
    }

    fn drop_widget(&mut self, mut s: Shown) {
        s.glass
            .set(&mut self.scene, &self.glass, s.glass_tree, None);
        self.destroy_redrawn(s.tree);
        crate::access::touch(self);
    }

    /// Redraw each standing widget whose content or scale changed since it was drawn, its
    /// desktop showing.
    fn refresh_widgets(&mut self) {
        for i in 0..self.widgets.shown.len() {
            let (kind, out) = {
                let s = &self.widgets.shown[i];
                (s.kind, s.output.clone())
            };
            if self.desktop_covered(&out) && self.widgets.shown[i].drawn.is_some() {
                continue;
            }
            let content = self.widget_content(kind);
            let scale = self.draw_scale_at(Some(&out)).max(1);
            if self.widgets.shown[i].drawn.as_ref() == Some(&(content.clone(), scale)) {
                continue;
            }
            let theme = self.config.theme.scaled(scale);
            let canvas = deskwidget::draw(&theme, self.text.for_output(theme.scale), &content);
            let m = panel::margin(&self.config.theme);
            let size = deskwidget::size(&content, &self.config.theme.controls());
            let s = &mut self.widgets.shown[i];
            let scene = &mut self.scene;
            scene.set_position(s.buffer, -m.left, -m.top);
            scene.set_input(
                s.buffer,
                Input::Rect(Rectangle::new((m.left, m.top).into(), size.into())),
            );
            upload_scaled(scene, s.buffer, &canvas, scale);
            s.drawn = Some((content, scale));
            let tree = s.tree;
            self.redraw_nodes(Vec::new(), &[tree.into()]);
            crate::access::frame_changed(self);
        }
    }

    /// Read `/proc` for the meter when it stands on a showing desktop and the last sample is
    /// [`SAMPLE`] old.
    fn sample_meter(&mut self) {
        let showing = (self.widgets.shown.iter())
            .find(|s| s.kind == Kind::Meter)
            .is_some_and(|s| !self.desktop_covered(&s.output));
        if !showing {
            return;
        }
        let now = Instant::now();
        let w = &mut self.widgets;
        if w.cpu_before.is_some_and(|(at, _)| now - at < SAMPLE) {
            return;
        }
        let times = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|t| CpuTimes::parse(&t));
        let memory = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|t| deskwidget::memory_share(&t));
        // A first reading, or one after the desktop was covered a while, has nothing recent to
        // measure against.
        let cpu = match (w.cpu_before, times) {
            (Some((at, before)), Some(after)) if now - at < 3 * SAMPLE => {
                deskwidget::cpu_share(before, after)
            }
            _ => None,
        };
        w.cpu_before = times.map(|t| (now, t));
        w.meter.push(cpu, memory);
    }

    fn arm_widget_timer(&mut self) {
        if let Some(t) = self.widgets.timer.take() {
            self.handle.remove(t);
        }
        if self.widgets.shown.is_empty() {
            return;
        }
        let showing = (self.widgets.shown.iter()).any(|s| !self.desktop_covered(&s.output));
        let each_second = self.widgets.shown.iter().any(|s| match s.kind {
            Kind::Meter => true,
            Kind::Clock => self.widgets.store.seconds,
            Kind::Calendar => false,
        });
        let wait = if showing {
            deskwidget::next_tick(each_second, since_epoch())
        } else {
            COVERED_POLL
        };
        let token = self
            .handle
            .insert_source(Timer::from_duration(wait), |_, _, state| {
                state.widgets.timer = None;
                state.sample_meter();
                state.refresh_widgets();
                state.arm_widget_timer();
                TimeoutAction::Drop
            });
        match token {
            Ok(t) => self.widgets.timer = Some(t),
            Err(e) => log::warn!("desktop widgets: no timer: {}", e.error),
        }
    }

    /// The cells the widgets on the icons' output cover, which the icons flow round.
    pub(crate) fn icon_blocked(&self) -> HashSet<(i32, i32)> {
        let primary = self.first_output();
        covered(
            (self.widgets.shown.iter())
                .filter(|s| primary.as_ref() == Some(&s.output))
                .map(|s| s.spot),
        )
    }

    /// The cells the icons stand in, from their last layout.
    fn icon_cells(&self) -> HashSet<(i32, i32)> {
        let (grid, area) = (self.desktop_grid(), self.desktop_area());
        (self.desktop.iter())
            .flat_map(|v| &v.cells)
            .map(|r| desktop::cell_at(grid, area, r.x, r.y))
            .collect()
    }

    /// Show or take away `kind`, saving `desktop_widgets`. One added from the desktop menu stands
    /// on the output under the menu.
    pub fn toggle_widget(&mut self, kind: Kind) {
        let list = &mut self.config.desktop_widgets;
        if let Some(i) = list.iter().position(|&k| k == kind) {
            list.remove(i);
        } else {
            list.push(kind);
            let under = self.desktop_menu_at.and_then(|(x, y)| {
                let out = self.output_at(f64::from(x), f64::from(y))?;
                let ids = self.monitor_ids();
                ids.into_iter().find(|(o, _)| *o == out).map(|(_, id)| id)
            });
            if let Some(id) = under {
                self.widgets.store.homes.insert(kind, id);
                self.widgets.store.save();
            }
        }
        let names: toml_edit::Array = (self.config.desktop_widgets.iter())
            .map(|k| k.name())
            .collect();
        crate::configwrite::save_value(
            &self.config_path,
            repriseconf::key("", "desktop_widgets"),
            toml_edit::Value::Array(names),
        );
        self.relayout_desktop();
    }

    /// Change the clock's face or seconds, or the meter's style, from a widget's menu.
    pub fn widget_option(&mut self, cmd: &DesktopCommand) {
        let store = &mut self.widgets.store;
        match *cmd {
            DesktopCommand::ClockFace(f) => store.face = f,
            DesktopCommand::ClockSeconds => store.seconds = !store.seconds,
            DesktopCommand::MeterLook(m) => store.meter = m,
            _ => return,
        }
        store.save();
        self.relayout_desktop();
    }

    pub fn open_widget_menu(&mut self, kind: Kind, at: (i32, i32)) {
        self.close_menu();
        let (items, commands) = widget_menu(kind, &self.widgets.store);
        self.desktop_menu_at = None;
        let kind = crate::menuview::MenuKind::Desktop {
            commands,
            target: None,
        };
        self.push_context_menu(at, items, kind, crate::menuview::MenuFrom::Elsewhere);
    }

    /// The panel of `kind` in layout coordinates, while it stands.
    pub fn widget_panel(&self, kind: Kind) -> Option<Rect> {
        (self.widgets.shown.iter())
            .find(|s| s.kind == kind)
            .map(|s| s.panel)
    }

    /// A left press on `kind` at `pos` holds it for a drag.
    pub fn widget_press(&mut self, kind: Kind, pos: (f64, f64)) {
        let Some(p) = self.widget_panel(kind) else {
            return;
        };
        self.interaction = crate::input::Interaction::WidgetDrag(Held {
            kind,
            grab: (pos.0 - f64::from(p.x), pos.1 - f64::from(p.y)),
            from: pos,
            moved: false,
        });
    }

    /// The held widget follows the pointer, once it has gone `desktop::DRAG_PX`.
    pub fn widget_drag(&mut self, pos: (f64, f64)) {
        let crate::input::Interaction::WidgetDrag(held) = &mut self.interaction else {
            return;
        };
        let far = (pos.0 - held.from.0).abs().max((pos.1 - held.from.1).abs());
        held.moved |= far >= f64::from(desktop::DRAG_PX);
        if !held.moved {
            return;
        }
        let held = *held;
        let Some(s) = self.widgets.shown.iter().find(|s| s.kind == held.kind) else {
            return;
        };
        let tree = s.tree;
        let was = self.scene.outputs_of(tree);
        let (x, y) = (pos.0 - held.grab.0, pos.1 - held.grab.1);
        self.scene
            .set_position(tree, x.round() as i32, y.round() as i32);
        self.redraw_nodes(was, &[tree.into()]);
    }

    /// Land the held widget on the grid cell nearest its corner on the output under the pointer, off
    /// the other widgets there; the icons make way. A press that never moved changes nothing.
    pub fn widget_drop(&mut self, held: Held, pos: (f64, f64)) {
        if !held.moved {
            return;
        }
        let out = self.output_at(pos.0, pos.1).or_else(|| {
            self.widgets
                .shown
                .iter()
                .find(|s| s.kind == held.kind)
                .map(|s| s.output.clone())
        });
        let (Some(out), Some(s)) = (out, self.widgets.shown.iter().find(|s| s.kind == held.kind))
        else {
            return;
        };
        let size = (s.panel.width, s.panel.height);
        let span = s.spot.span;
        let grid = self.desktop_grid();
        let area = self.usable_area(Some(&out));
        let others: Vec<Spot> = (self.widgets.shown.iter())
            .filter(|o| o.kind != held.kind && o.output == out)
            .map(|o| o.spot)
            .collect();
        let corner = drop_corner(grid, span, size, (pos.0 - held.grab.0, pos.1 - held.grab.1));
        let want = (
            (corner.0 - area.x).div_euclid(grid.w),
            (corner.1 - area.y).div_euclid(grid.h),
        );
        let cell = deskwidget::place(
            deskwidget::dims(grid, area),
            span,
            want,
            &others,
            &HashSet::new(),
        );
        let Some(id) = self
            .monitor_ids()
            .into_iter()
            .find(|(o, _)| *o == out)
            .map(|p| p.1)
        else {
            return;
        };
        let store = &mut self.widgets.store;
        store.homes.insert(held.kind, id.clone());
        store
            .cells
            .entry(held.kind)
            .or_default()
            .insert(id, desktop::to_saved(grid, cell));
        store.save();
        // Back where it stood, so `stand_widget` sees the move and redraws both places.
        if let Some(s) = self.widgets.shown.iter_mut().find(|s| s.kind == held.kind) {
            s.panel = Rect::default();
        }
        self.relayout_desktop();
    }

    /// A drag that lost the pointer puts the widget back.
    pub fn widget_drag_cancel(&mut self, held: Held) {
        if let Some(s) = self.widgets.shown.iter_mut().find(|s| s.kind == held.kind) {
            s.panel = Rect::default();
        }
        self.relayout_desktop();
    }

    pub fn widgets_shown(&self) -> bool {
        !self.widgets.shown.is_empty()
    }

    /// The widgets as a reader sees them, while any stands.
    pub fn widgets_access(&self) -> Option<Tree> {
        if self.widgets.shown.is_empty() {
            return None;
        }
        let mut tree = crate::access::root(Role::Pane, "Desktop widgets", Rect::new(0, 0, 1, 1));
        let cm = self.config.theme.controls();
        let text = self.text.for_output(1);
        let px = self.config.theme.fonts.size_px;
        for s in &self.widgets.shown {
            if let Some((content, _)) = &s.drawn {
                deskwidget::access(
                    content,
                    s.panel,
                    &cm,
                    text,
                    px,
                    &mut tree,
                    reprisekit::access::ROOT,
                    s.kind,
                );
            }
        }
        Some(tree)
    }
}

/// The top-left corner, plus half a cell, of the cells a widget covering `span` takes with its
/// panel of `size` (centred in them) dragged to `panel_at`: its cell is the one holding this point.
pub fn drop_corner(
    grid: desktop::Cell,
    span: (i32, i32),
    size: (i32, i32),
    panel_at: (f64, f64),
) -> (i32, i32) {
    let inset = (
        (span.0 * grid.w - size.0) / 2,
        (span.1 * grid.h - size.1) / 2,
    );
    (
        panel_at.0.round() as i32 - inset.0 + grid.w / 2,
        panel_at.1.round() as i32 - inset.1 + grid.h / 2,
    )
}
