//! The debug overlay (`[debug] overlay`): one `reprisekit::controls::
//! statspanel` in the top-right corner of each output's usable area,
//! refreshed twice a second from the counters the backend keeps per
//! output ([`FrameStats`]) and a count of clients, windows and surfaces.
//! Its trees in `layers.overlay` are tagged `SceneOwner::Debug`; the
//! buffer lets the pointer through. A panel is redrawn only when its text
//! changes, and the renders a redraw causes come off its renders per
//! second, so an idle screen reads 0.
//!
//! Captures leave the panels out unless `[debug] overlay_in_captures`
//! ([`State::without_debug_panels`]).

use std::time::{Duration, Instant};

use reprisekit::controls::statspanel::{self, Row, StatsPanel};
use reprisekit::look::panel;
use reprisescene::{BufferId, Input, TreeId};
use smithay::output::Output;
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::compositor::{SceneOwner, State};
use crate::glass::{Backdrop, Pane, PaneShape};
use crate::output::upload_scaled;
use crate::raster::Rect;

const PERIOD: Duration = Duration::from_millis(500);
const GAP: i32 = 8;

/// One output's frame counters since the panel last read them: sums and
/// maxima, so counting allocates nothing. The backend counts only while
/// the overlay is on.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameStats {
    pub renders: u32,
    pub cpu_total: Duration,
    pub cpu_max: Duration,
    /// Flip intervals counted: flips that followed another back to back.
    pub flips: u32,
    pub flip_total: Duration,
    pub flip_max: Duration,
    /// Frames whose damage is known (drawn through the swapchain).
    pub damaged: u32,
    pub rects: u32,
    /// Damaged physical pixels, overlaps counted twice.
    pub area: u64,
    /// Commits the output's thread finished; handoff is main to thread and back, less the ioctl.
    pub commits: u32,
    pub handoff_total: Duration,
    pub handoff_max: Duration,
    pub ioctl_total: Duration,
    pub ioctl_max: Duration,
    last_flip: Option<Duration>,
    /// A frame was waiting at the last flip: the next flip follows it
    /// back to back, so the time between them is a frame time, not idle.
    chained: bool,
}

impl FrameStats {
    /// A frame went to the screen after `cpu` of work.
    pub fn rendered(&mut self, cpu: Duration) {
        self.renders += 1;
        self.cpu_total += cpu;
        self.cpu_max = self.cpu_max.max(cpu);
    }

    /// A drawn frame repainted `rects` rectangles of `area` pixels.
    pub fn damage(&mut self, rects: u32, area: u64) {
        self.damaged += 1;
        self.rects += rects;
        self.area += area;
    }

    /// A commit took `ioctl` in the kernel and `handoff` getting there and back.
    pub fn committed(&mut self, handoff: Duration, ioctl: Duration) {
        self.commits += 1;
        self.handoff_total += handoff;
        self.handoff_max = self.handoff_max.max(handoff);
        self.ioctl_total += ioctl;
        self.ioctl_max = self.ioctl_max.max(ioctl);
    }

    /// A page flip at `at` (monotonic); `waiting` says a frame is already
    /// due after it.
    pub fn flipped(&mut self, at: Duration, waiting: bool) {
        if let Some(last) = self.last_flip.filter(|_| self.chained) {
            let d = at.saturating_sub(last);
            self.flips += 1;
            self.flip_total += d;
            self.flip_max = self.flip_max.max(d);
        }
        self.last_flip = Some(at);
        self.chained = waiting;
    }

    /// The counts so far, starting the next period from zero.
    pub fn take(&mut self) -> FrameStats {
        let (last_flip, chained) = (self.last_flip, self.chained);
        std::mem::replace(
            self,
            FrameStats {
                last_flip,
                chained,
                ..FrameStats::default()
            },
        )
    }
}

/// What the whole compositor counts, the same on every panel.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub clients: usize,
    pub windows: usize,
    pub surfaces: usize,
    /// Resident memory in bytes; None when /proc does not say.
    pub rss: Option<u64>,
}

/// A period's rows: `stats` over `secs`, less the `own` renders the
/// panel's last redraw caused, on an output of `pixels` physical pixels.
/// A period whose renders were all the panel's own shows no frame
/// figures, so redrawing the panel does not keep changing it.
pub fn rows(stats: &FrameStats, secs: f64, own: u32, pixels: u64, totals: &Totals) -> Vec<Row> {
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    let avg_max = |n: u32, total: Duration, max: Duration| {
        format!("{:.1} ms avg, {:.1} max", ms(total) / f64::from(n), ms(max))
    };
    let renders = stats.renders.saturating_sub(own);
    let none = || "-".to_string();
    let frames = renders > 0;
    let cpu = if frames {
        avg_max(stats.renders, stats.cpu_total, stats.cpu_max)
    } else {
        none()
    };
    let flips = if frames && stats.flips > 0 {
        avg_max(stats.flips, stats.flip_total, stats.flip_max)
    } else {
        none()
    };
    let damage = if frames && stats.damaged > 0 {
        let n = f64::from(stats.damaged);
        let share = stats.area as f64 / n / pixels.max(1) as f64 * 100.0;
        format!(
            "{:.1} rects, {:.1}% area",
            f64::from(stats.rects) / n,
            share.min(100.0)
        )
    } else {
        none()
    };
    let commits = if frames && stats.commits > 0 {
        let n = f64::from(stats.commits);
        format!(
            "{:.2} + {:.2} ms avg, {:.2} + {:.2} max",
            ms(stats.handoff_total) / n,
            ms(stats.ioctl_total) / n,
            ms(stats.handoff_max),
            ms(stats.ioctl_max)
        )
    } else {
        none()
    };
    let rss = totals
        .rss
        .map_or_else(none, |b| format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)));
    vec![
        Row::new(
            "Renders/s",
            format!("{:.0}", f64::from(renders) / secs.max(1e-3)),
        ),
        Row::new("CPU/render", cpu),
        Row::new("Flip interval", flips),
        Row::new("Handoff + commit", commits),
        Row::new("Damage", damage),
        Row::new("Clients", totals.clients.to_string()),
        Row::new("Windows", totals.windows.to_string()),
        Row::new("Surfaces", totals.surfaces.to_string()),
        Row::new("Memory", rss),
    ]
}

fn title(output: &Output) -> String {
    match output.current_mode() {
        Some(m) => format!(
            "{} ({}x{} at {:.0} Hz)",
            output.name(),
            m.size.w,
            m.size.h,
            f64::from(m.refresh) / 1000.0
        ),
        None => output.name(),
    }
}

/// The resident set size from /proc/self/statm, in bytes.
fn rss() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    // SAFETY: sysconf only reads a constant.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    Some(pages * u64::try_from(page).ok()?)
}

/// Where a panel `w` wide goes in `usable`: `GAP` in from its top-right
/// corner, its left edge kept inside.
fn panel_origin(usable: Rect, w: i32) -> (i32, i32) {
    let x = usable.right() - w - GAP;
    (x.max(usable.x), usable.y + GAP)
}

/// The panels while the overlay is on, and the timer that refreshes them.
#[derive(Default)]
pub struct DebugView {
    timer: Option<RegistrationToken>,
    panels: Vec<Panel>,
    /// When the counters were last read.
    read: Option<Instant>,
}

struct Panel {
    output: Output,
    tree: TreeId,
    /// Holds the glass, under `buffer`.
    glass_tree: TreeId,
    backdrop: Backdrop,
    buffer: BufferId,
    shown: StatsPanel,
    /// Redraws since the counters were last read: the renders they cost.
    own: u32,
}

/// Start or stop the overlay after a config change, and rebuild the
/// panels for the theme and scale in force. Runs at startup and on every
/// reload.
pub fn config_changed(state: &mut State) {
    clear(state);
    if !state.config.debug.overlay {
        if let Some(t) = state.debug.timer.take() {
            state.handle.remove(t);
        }
        state.debug.read = None;
        return;
    }
    if state.debug.timer.is_none() {
        let token = state
            .handle
            .insert_source(Timer::from_duration(PERIOD), |_, _, state| {
                tick(state);
                TimeoutAction::ToDuration(PERIOD)
            });
        match token {
            Ok(t) => state.debug.timer = Some(t),
            Err(e) => log::warn!("debug overlay: no timer: {}", e.error),
        }
    }
    tick(state);
}

fn clear(state: &mut State) {
    let panels = std::mem::take(&mut state.debug.panels);
    if panels.is_empty() {
        return;
    }
    for p in panels {
        remove(state, p);
    }
    state.redraw_all();
}

fn remove(state: &mut State, mut p: Panel) {
    p.backdrop
        .set(&mut state.scene, &state.glass, p.glass_tree, None);
    state.scene.destroy(p.tree);
}

/// Read the counters, then show each output's panel where its text
/// changed: one per output, gone with its output.
fn tick(state: &mut State) {
    let now = Instant::now();
    // The first read after the overlay came on drops what an earlier
    // spell left in the counters.
    let last = state.debug.read.replace(now);
    let secs = last.map_or(PERIOD.as_secs_f64(), |t| (now - t).as_secs_f64());
    let totals = totals(state);
    let outputs = state.scene.outputs().to_vec();
    let (mut panels, gone): (Vec<Panel>, Vec<Panel>) = std::mem::take(&mut state.debug.panels)
        .into_iter()
        .partition(|p| outputs.contains(&p.output));
    for p in gone {
        remove(state, p);
    }
    for o in &outputs {
        let stats = crate::backend::take_stats(state, o)
            .filter(|_| last.is_some())
            .unwrap_or_default();
        let pixels = o
            .current_mode()
            .map_or(1, |m| (m.size.w.max(0) as u64) * (m.size.h.max(0) as u64));
        let at = panels.iter().position(|p| &p.output == o);
        let own = at.map_or(0, |i| std::mem::take(&mut panels[i].own));
        let want = StatsPanel {
            title: title(o),
            rows: rows(&stats, secs, own, pixels, &totals),
        };
        let usable = state.usable_area(Some(o));
        let origin = panel_origin(usable, statspanel::W);
        match at {
            Some(i) => {
                let p = &mut panels[i];
                if state.scene.position(p.tree) != Some(origin.into()) {
                    state.scene.set_position(p.tree, origin.0, origin.1);
                }
                if p.shown != want {
                    draw(state, p, want);
                }
            }
            None => panels.push(build(state, o, origin, want)),
        }
    }
    state.debug.panels = panels;
}

fn build(state: &mut State, output: &Output, origin: (i32, i32), want: StatsPanel) -> Panel {
    let scene = &mut state.scene;
    let tree = scene.tree_create(state.layers.overlay);
    scene.set_data(tree, Some(SceneOwner::Debug));
    crate::glass::leave_out(scene, tree);
    scene.set_position(tree, origin.0, origin.1);
    let glass_tree = scene.tree_create(tree);
    let buffer = scene.buffer_create(tree);
    scene.set_input(buffer, Input::Passthrough);
    let mut p = Panel {
        output: output.clone(),
        tree,
        glass_tree,
        backdrop: Backdrop::new(&state.glass),
        buffer,
        shown: StatsPanel::default(),
        own: 0,
    };
    draw(state, &mut p, want);
    p
}

fn draw(state: &mut State, p: &mut Panel, want: StatsPanel) {
    let scale = state.draw_scale_at(Some(&p.output)).max(1);
    let theme = state.config.theme.scaled(scale);
    let canvas = statspanel::draw(&theme, state.text.for_output(theme.scale), &want);
    let m = panel::margin(&state.config.theme);
    let h = statspanel::height(want.rows.len());
    let pane = match state.config.theme.glass() {
        Some(g) if crate::glass::wanted(&state.config) => Some(Pane {
            shapes: vec![PaneShape {
                rect: Rect::new(0, 0, statspanel::W, h),
                corner: g.panel.corner,
                square_bottom: false,
                square_top: false,
            }],
            holes: Vec::new(),
        }),
        _ => None,
    };
    p.backdrop
        .set(&mut state.scene, &state.glass, p.glass_tree, pane);
    state.scene.set_position(p.buffer, -m.left, -m.top);
    upload_scaled(&mut state.scene, p.buffer, &canvas, scale);
    p.shown = want;
    p.own += 1;
    crate::backend::schedule(state, &p.output);
}

fn totals(state: &State) -> Totals {
    use smithay::reexports::wayland_server::backend::ClientId;
    let handle = state.dh.backend_handle();
    let mut clients: Vec<ClientId> = Vec::new();
    handle.with_all_clients(|c| clients.push(c));
    let mut surfaces = 0;
    for c in &clients {
        let _ = handle.with_all_objects_for(c.clone(), |o| {
            if o.interface().name == "wl_surface" {
                surfaces += 1;
            }
        });
    }
    Totals {
        clients: clients.len(),
        windows: state.windows.iter().filter(|w| w.mapped).count(),
        surfaces,
        rss: rss(),
    }
}

impl State {
    /// Run `f`, a capture of the screen, with the debug panels hidden
    /// unless `shown`. Screenshots and screencasts pass `[debug]
    /// overlay_in_captures`; stills the desktop shows itself pass false.
    pub fn without_debug_panels<T>(&mut self, shown: bool, f: impl FnOnce(&mut State) -> T) -> T {
        let trees: Vec<TreeId> = self.debug.panels.iter().map(|p| p.tree).collect();
        if shown || trees.is_empty() {
            return f(self);
        }
        for &t in &trees {
            self.scene.set_enabled(t, false);
        }
        let out = f(self);
        for t in trees {
            self.scene.set_enabled(t, true);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn only_back_to_back_flips_count_as_frame_times() {
        let mut s = FrameStats::default();
        s.flipped(ms(1000), true);
        s.flipped(ms(1016), true);
        s.flipped(ms(1050), false);
        // Idle after a flip with nothing waiting: the gap is not a frame.
        s.flipped(ms(4000), true);
        s.flipped(ms(4017), false);
        let t = s.take();
        assert_eq!(t.flips, 3);
        assert_eq!(t.flip_total, ms(16 + 34 + 17));
        assert_eq!(t.flip_max, ms(34));
        // The next period starts from zero but remembers the last flip.
        s.flipped(ms(4033), false);
        assert_eq!(s.take().flips, 0, "4017 had nothing waiting");
    }

    #[test]
    fn the_rows_report_rates_averages_and_maxima() {
        let mut s = FrameStats::default();
        for cpu in [1, 3] {
            s.rendered(ms(cpu));
            s.damage(2, 1000);
        }
        s.committed(Duration::from_micros(50), ms(1));
        s.committed(Duration::from_micros(150), ms(3));
        s.flipped(ms(0), true);
        s.flipped(ms(16), true);
        s.flipped(ms(36), false);
        let totals = Totals {
            clients: 3,
            windows: 2,
            surfaces: 9,
            rss: Some(80 * 1024 * 1024),
        };
        let r = rows(&s, 0.5, 0, 10_000, &totals);
        let v: Vec<(&str, &str)> = r
            .iter()
            .map(|r| (r.label.as_str(), r.value.as_str()))
            .collect();
        assert_eq!(
            v,
            [
                ("Renders/s", "4"),
                ("CPU/render", "2.0 ms avg, 3.0 max"),
                ("Flip interval", "18.0 ms avg, 20.0 max"),
                ("Handoff + commit", "0.10 + 2.00 ms avg, 0.15 + 3.00 max"),
                ("Damage", "2.0 rects, 10.0% area"),
                ("Clients", "3"),
                ("Windows", "2"),
                ("Surfaces", "9"),
                ("Memory", "80.0 MB"),
            ]
        );
    }

    #[test]
    fn the_panels_own_redraws_leave_an_idle_screen_at_zero() {
        let mut s = FrameStats::default();
        s.rendered(ms(2));
        s.damage(1, 500);
        let r = rows(&s, 0.5, 1, 10_000, &Totals::default());
        assert_eq!(r[0].value, "0");
        for row in &r[1..5] {
            assert_eq!(row.value, "-", "{} shows nothing", row.label);
        }
        assert_eq!(r[8].value, "-", "no /proc, no memory figure");
    }

    #[test]
    fn the_panel_sits_in_the_usable_areas_top_right_corner() {
        assert_eq!(
            panel_origin(Rect::new(0, 30, 1920, 1050), 236),
            (1920 - 236 - GAP, 30 + GAP)
        );
        assert_eq!(panel_origin(Rect::new(100, 0, 200, 600), 236), (100, GAP));
    }
}
