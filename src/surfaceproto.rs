//! Protocols that only shape how a surface is drawn: viewporter,
//! fractional-scale, single-pixel-buffer, content-type, alpha-modifier,
//! ext-background-effect (`crate::clientglass`), linux-drm-syncobj, fifo and
//! commit-timing. Smithay keeps their state on the surface; Reprise adds the
//! globals, a first preferred scale and the commit blockers.
//!
//! A surface's primary output passes its fifo barriers and commit timers (`frame_done`,
//! `before_frame`), asked at commit for the frame a timer shows in (`ask_frame`); a surface no
//! output shows has them passed at the refresh rate (`tick`).
//!
//! A commit with a dmabuf waits for the buffer to be ready before it
//! applies: on its explicit-sync acquire point when it set one, else on
//! the dmabuf's implicit fence. The blocker follows Smithay's
//! `anvil/src/shell/mod.rs`.

use std::{collections::HashMap, time::Duration};

use smithay::{
    backend::drm::DrmDeviceFd,
    output::Output,
    reexports::{
        calloop::{
            Interest, RegistrationToken,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{
            Client, DisplayHandle, Resource, Weak, backend::ClientId,
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{Monotonic, Time},
    wayland::{
        alpha_modifier::AlphaModifierState,
        background_effect::BackgroundEffectState,
        commit_timing::{
            CommitTimerBarrierStateUserData, CommitTimerStateUserData, CommitTimingManagerState,
        },
        compositor::{
            Barrier, BufferAssignment, CompositorHandler, SurfaceAttributes, add_blocker,
            add_pre_commit_hook, get_parent, with_states,
        },
        content_type::ContentTypeState,
        dmabuf::get_dmabuf,
        drm_syncobj::{
            DrmSyncobjCachedState, DrmSyncobjHandler, DrmSyncobjState, supports_syncobj_eventfd,
        },
        fifo::{FifoBarrierCachedState, FifoCachedState, FifoManagerState},
        fractional_scale::{
            FractionalScaleHandler, FractionalScaleManagerState, with_fractional_scale,
        },
        single_pixel_buffer::SinglePixelBufferState,
        viewporter::ViewporterState,
    },
};

use crate::compositor::State;

pub struct SurfaceProtos {
    #[allow(dead_code)]
    viewporter: ViewporterState,
    #[allow(dead_code)]
    fractional_scale: FractionalScaleManagerState,
    #[allow(dead_code)]
    single_pixel_buffer: SinglePixelBufferState,
    #[allow(dead_code)]
    content_type: ContentTypeState,
    #[allow(dead_code)]
    alpha_modifier: AlphaModifierState,
    #[allow(dead_code)]
    background_effect: BackgroundEffectState,
    syncobj: Option<DrmSyncobjState>,
    #[allow(dead_code)]
    fifo: FifoManagerState,
    #[allow(dead_code)]
    commit_timing: CommitTimingManagerState,
    /// Surfaces with a commit waiting on a fifo barrier or a commit timer.
    waiting: Vec<Waiting>,
    /// The refresh-rate timer running while `waiting` holds a surface (`tick`).
    tick: Option<RegistrationToken>,
}

impl SurfaceProtos {
    pub fn new(dh: &DisplayHandle) -> SurfaceProtos {
        SurfaceProtos {
            viewporter: ViewporterState::new::<State>(dh),
            fractional_scale: FractionalScaleManagerState::new::<State>(dh),
            single_pixel_buffer: SinglePixelBufferState::new::<State>(dh),
            content_type: ContentTypeState::new::<State>(dh),
            alpha_modifier: AlphaModifierState::new::<State>(dh),
            background_effect: BackgroundEffectState::new::<State>(dh),
            syncobj: None,
            fifo: FifoManagerState::new::<State>(dh),
            commit_timing: CommitTimingManagerState::new::<State>(dh),
            waiting: Vec::new(),
            tick: None,
        }
    }

    /// Offer linux-drm-syncobj when `device` can wait on a timeline point
    /// through an eventfd; otherwise clients keep implicit sync.
    pub fn init_syncobj(&mut self, dh: &DisplayHandle, device: DrmDeviceFd) {
        if self.syncobj.is_some() {
            return;
        }
        if supports_syncobj_eventfd(&device) {
            self.syncobj = Some(DrmSyncobjState::new::<State>(dh, device));
            log::info!("explicit sync: linux-drm-syncobj offered");
        } else {
            log::info!("explicit sync: the GPU has no syncobj eventfd; not offered");
        }
    }
}

impl DrmSyncobjHandler for State {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.surface_protos.syncobj.as_mut()
    }
}

pub fn on_new_surface(surface: &WlSurface) {
    add_pre_commit_hook::<State, _>(surface, |state, _dh, surface| {
        let (barrier, timed) = commit_waits(surface);
        if barrier.is_some() || timed.is_some() {
            wait(state, surface, barrier);
        }
        if let Some(at) = timed
            && let Some(output) = state.scene.surface_primary_output(&root(surface))
        {
            ask_frame(state, &output, at);
        }
        let mut acquire = None;
        let dmabuf = with_states(surface, |data| {
            acquire.clone_from(
                &data
                    .cached_state
                    .get::<DrmSyncobjCachedState>()
                    .pending()
                    .acquire_point,
            );
            data.cached_state
                .get::<SurfaceAttributes>()
                .pending()
                .buffer
                .as_ref()
                .and_then(|a| match a {
                    BufferAssignment::NewBuffer(b) => get_dmabuf(b).cloned().ok(),
                    _ => None,
                })
        });
        let Some(dmabuf) = dmabuf else {
            return;
        };
        let Some(client) = surface.client() else {
            return;
        };
        if let Some(point) = acquire
            && let Ok((blocker, source)) = point.generate_blocker()
        {
            let client = client.clone();
            let inserted = state.handle.insert_source(source, move |_, _, state| {
                let dh = state.dh.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
                return;
            }
        }
        if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
            let inserted = state.handle.insert_source(source, move |_, _, state| {
                let dh = state.dh.clone();
                state
                    .client_compositor_state(&client)
                    .blocker_cleared(state, &dh);
                Ok(())
            });
            if inserted.is_ok() {
                add_blocker(surface, blocker);
            }
        }
    });
}

/// Preferred scale: the output under the pointer until the surface shows
/// somewhere, then its primary output's.
impl FractionalScaleHandler for State {
    fn new_fractional_scale(&mut self, surface: WlSurface) {
        let pointer = self.pointer_location();
        let outputs = self.scene.outputs();
        let output = outputs
            .iter()
            .find(|o| reprisescene::output_rect(o).is_some_and(|r| r.to_f64().contains(pointer)))
            .or(outputs.first());
        let Some(scale) = output.map(|o| o.current_scale().fractional_scale()) else {
            return;
        };
        with_states(&surface, |data| {
            with_fractional_scale(data, |fs| fs.set_preferred_scale(scale));
        });
    }
}

/// A surface's commits waiting on fifo barriers, kept until the barriers pass, or until its
/// commit timers have.
struct Waiting {
    surface: Weak<WlSurface>,
    barriers: Vec<Barrier>,
}

/// What the commit about to apply waits on: the fifo barrier, and its commit timer's time
/// (monotonic). Reads the pending barrier before Smithay's fifo hook, added later, takes it.
fn commit_waits(surface: &WlSurface) -> (Option<Barrier>, Option<Duration>) {
    with_states(surface, |data| {
        let cache = &data.cached_state;
        let waits =
            cache.has::<FifoCachedState>() && cache.get::<FifoCachedState>().pending().wait_barrier;
        let barrier = waits
            .then(|| {
                (cache.get::<FifoBarrierCachedState>().pending().barrier)
                    .clone()
                    .filter(|b| !b.is_signaled())
            })
            .flatten();
        let timed = (data.data_map.get::<CommitTimerStateUserData>())
            .and_then(|t| t.borrow().timestamp)
            .map(|t| Time::<Monotonic>::from(t).into());
        (barrier, timed)
    })
}

fn wait(state: &mut State, surface: &WlSurface, barrier: Option<Barrier>) {
    let protos = &mut state.surface_protos;
    match protos.waiting.iter_mut().find(|w| w.surface == *surface) {
        Some(w) => w.barriers.extend(barrier),
        None => protos.waiting.push(Waiting {
            surface: surface.downgrade(),
            barriers: barrier.into_iter().collect(),
        }),
    }
    if protos.tick.is_some() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(refresh(state)), |_, _, state| {
            if tick(state) {
                TimeoutAction::ToDuration(refresh(state))
            } else {
                state.surface_protos.tick = None;
                TimeoutAction::Drop
            }
        });
    match token {
        Ok(t) => state.surface_protos.tick = Some(t),
        Err(e) => log::warn!("fifo timer: {}", e.error),
    }
}

/// Ask `output` for the frame that shows at `at` (monotonic). The refresh-rate tick runs out of
/// phase with the output's flips: a frame it asked for could start just before the refresh it
/// meant to follow and show the commit a refresh late.
fn ask_frame(state: &mut State, output: &Output, at: Duration) {
    let at = std::time::Instant::now() + at.saturating_sub(state.now());
    crate::backend::schedule_shows_at(state, output, at);
}

/// The fastest output's refresh interval; 60 Hz with no output.
fn refresh(state: &State) -> Duration {
    let fastest = (state.scene.outputs().iter())
        .filter_map(|o| o.current_mode())
        .map(|m| m.refresh)
        .max();
    crate::framepace::interval(fastest.unwrap_or(0))
}

/// The time `surface`'s earliest commit timer waits for (monotonic).
fn next_timer(surface: &WlSurface) -> Option<Duration> {
    with_states(surface, |data| {
        (data.data_map.get::<CommitTimerBarrierStateUserData>())
            .and_then(|t| t.lock().ok()?.next_deadline())
            .map(|t| Time::<Monotonic>::from(t).into())
    })
}

fn root(surface: &WlSurface) -> WlSurface {
    let mut root = surface.clone();
    while let Some(parent) = get_parent(&root) {
        root = parent;
    }
    root
}

/// What one release frees on a surface: its current fifo barrier, and its commit timers due by
/// a time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Release {
    fifo: bool,
    timers_by: Option<Duration>,
}

/// A `tick`'s choice for a surface: one no output shows passes its barrier and the timers due by
/// `now`; a shown one is left to its output.
fn on_tick(shown: bool, now: Duration) -> Release {
    if shown {
        return Release::default();
    }
    Release {
        fifo: true,
        timers_by: Some(now),
    }
}

/// Release `pick`'s choice on each waiting surface, given its root's primary output, then let
/// the commits that waited apply.
fn release(state: &mut State, mut pick: impl FnMut(&WlSurface, Option<Output>) -> Release) {
    let surfaces: Vec<WlSurface> = (state.surface_protos.waiting.iter())
        .filter_map(|w| w.surface.upgrade().ok())
        .collect();
    #[allow(clippy::mutable_key_type)]
    let mut clients: HashMap<ClientId, Client> = HashMap::new();
    for surface in surfaces {
        let primary = state.scene.surface_primary_output(&root(&surface));
        let r = pick(&surface, primary);
        let freed = with_states(&surface, |data| {
            let mut freed = false;
            if r.fifo
                && data.cached_state.has::<FifoBarrierCachedState>()
                && let Some(b) = (data.cached_state.get::<FifoBarrierCachedState>().current())
                    .barrier
                    .take()
            {
                b.signal();
                freed = true;
            }
            if let Some(by) = r.timers_by
                && let Some(t) = data.data_map.get::<CommitTimerBarrierStateUserData>()
                && let Ok(mut t) = t.lock()
            {
                freed |= t.signal_until(Time::<Monotonic>::from(by));
            }
            freed
        });
        if freed && let Some(c) = surface.client() {
            clients.insert(c.id(), c);
        }
    }
    let dh = state.dh.clone();
    for client in clients.into_values() {
        state
            .client_compositor_state(&client)
            .blocker_cleared(state, &dh);
    }
    state.surface_protos.waiting.retain_mut(|w| {
        let Ok(s) = w.surface.upgrade() else {
            return false;
        };
        w.barriers.retain(|b| !b.is_signaled());
        !w.barriers.is_empty() || next_timer(&s).is_some()
    });
}

/// `output`'s frame callbacks went out: the surfaces it is primary for pass their fifo barrier.
pub fn frame_done(state: &mut State, output: &Output) {
    if state.surface_protos.waiting.is_empty() {
        return;
    }
    release(state, |_, primary| Release {
        fifo: primary.as_ref() == Some(output),
        timers_by: None,
    });
}

/// `output` is about to draw a frame that shows at `shows_at` (monotonic): the surfaces it is
/// primary for apply the commits timed by then.
pub fn before_frame(state: &mut State, output: &Output, shows_at: Duration) {
    if state.surface_protos.waiting.is_empty() {
        return;
    }
    release(state, |_, primary| Release {
        fifo: false,
        timers_by: (primary.as_ref() == Some(output)).then_some(shows_at),
    });
    // A commit timed later asked at its own commit, but only the earliest ask stands.
    let next = (state.surface_protos.waiting.iter())
        .filter_map(|w| w.surface.upgrade().ok())
        .filter(|s| state.scene.surface_primary_output(&root(s)).as_ref() == Some(output))
        .filter_map(|s| next_timer(&s))
        .min();
    if let Some(at) = next {
        ask_frame(state, output, at);
    }
}

/// The refresh-rate timer's turn: whether it runs again.
fn tick(state: &mut State) -> bool {
    let now = state.now();
    release(state, |_, primary| on_tick(primary.is_some(), now));
    !state.surface_protos.waiting.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::{Harness, TestClient};
    use smithay::output::Mode;
    use wayland_client::{Proxy, protocol::wl_surface::WlSurface as ClientSurface};
    use wayland_protocols::wp::{
        commit_timing::v1::client::{
            wp_commit_timer_v1::WpCommitTimerV1,
            wp_commit_timing_manager_v1::WpCommitTimingManagerV1,
        },
        fifo::v1::client::{wp_fifo_manager_v1::WpFifoManagerV1, wp_fifo_v1::WpFifoV1},
    };

    const HZ60: Duration = Duration::from_nanos(16_666_666);

    /// A harness with one output refreshing once a second, so the tick timer never fires on
    /// its own during a test; the tests run `tick` themselves.
    fn harness() -> (Harness, Output) {
        let mut h = Harness::new();
        let output = h.output("A", 800, 600);
        let mode = Mode {
            size: (800, 600).into(),
            refresh: 1_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        (h, output)
    }

    fn server_side(h: &Harness, c: &TestClient, surface: &ClientSurface) -> WlSurface {
        c.server
            .object_from_protocol_id(&h.state.dh, surface.id().protocol_id())
            .expect("the surface on the compositor's side")
    }

    fn buffer_scale(surface: &WlSurface) -> i32 {
        with_states(surface, |d| {
            d.cached_state
                .get::<SurfaceAttributes>()
                .current()
                .buffer_scale
        })
    }

    /// Set a barrier, then commit buffer scale 2 waiting on it.
    fn commit_behind_barrier(h: &mut Harness, c: &mut TestClient, surface: &ClientSurface) {
        let manager: WpFifoManagerV1 = c.bind("wp_fifo_manager_v1", 1);
        let fifo: WpFifoV1 = manager.get_fifo(surface, &c.qh(), ());
        fifo.set_barrier();
        surface.commit();
        h.roundtrip(c);
        fifo.wait_barrier();
        fifo.set_barrier();
        surface.set_buffer_scale(2);
        surface.commit();
        h.roundtrip(c);
    }

    /// Commit buffer scale 2 timed to show at `at` on the monotonic clock.
    fn commit_timed(h: &mut Harness, c: &mut TestClient, surface: &ClientSurface, at: Duration) {
        let manager: WpCommitTimingManagerV1 = c.bind("wp_commit_timing_manager_v1", 1);
        let timer: WpCommitTimerV1 = manager.get_timer(surface, &c.qh(), ());
        let secs = at.as_secs();
        timer.set_timestamp((secs >> 32) as u32, secs as u32, at.subsec_nanos());
        surface.set_buffer_scale(2);
        surface.commit();
        h.roundtrip(c);
    }

    #[test]
    fn a_hidden_surface_passes_its_fifo_barrier_on_the_tick() {
        let (mut h, _) = harness();
        let mut c = h.connect();
        let surface = h.bare_client_surface(&mut c);
        let server = server_side(&h, &c, &surface);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert_eq!(buffer_scale(&server), 1, "the commit waits on the barrier");
        assert!(!tick(&mut h.state), "nothing is left to wait on");
        assert_eq!(buffer_scale(&server), 2, "the tick passes the barrier");
    }

    #[test]
    fn a_shown_surface_passes_its_fifo_barrier_only_on_its_outputs_frame() {
        let (mut h, output) = harness();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "fifo", 200, 150);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        assert_eq!(
            h.state.scene.surface_primary_output(&server).as_ref(),
            Some(&output)
        );
        commit_behind_barrier(&mut h, &mut c, &surface);
        tick(&mut h.state);
        assert_eq!(
            buffer_scale(&server),
            1,
            "the tick leaves a shown surface to its output"
        );
        frame_done(&mut h.state, &output);
        assert_eq!(
            buffer_scale(&server),
            2,
            "the output's frame passes the barrier"
        );
    }

    #[test]
    fn a_commit_that_only_waits_passes_on_the_frame_after_the_one_before_it() {
        let (mut h, output) = harness();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "fifo", 200, 150);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        let manager: WpFifoManagerV1 = c.bind("wp_fifo_manager_v1", 1);
        let fifo: WpFifoV1 = manager.get_fifo(&surface, &c.qh(), ());
        fifo.set_barrier();
        surface.commit();
        fifo.set_barrier();
        fifo.wait_barrier();
        surface.set_buffer_scale(2);
        surface.commit();
        fifo.wait_barrier();
        surface.set_buffer_scale(3);
        surface.commit();
        h.roundtrip(&mut c);
        assert_eq!(buffer_scale(&server), 1, "both commits wait");
        frame_done(&mut h.state, &output);
        assert_eq!(buffer_scale(&server), 2, "the first frame passes one");
        frame_done(&mut h.state, &output);
        assert_eq!(buffer_scale(&server), 3, "the next frame passes the other");
    }

    #[test]
    fn a_hidden_timed_commit_applies_on_the_first_tick_past_its_time() {
        let (mut h, _) = harness();
        let mut c = h.connect();
        let surface = h.bare_client_surface(&mut c);
        let server = server_side(&h, &c, &surface);
        let at = h.state.now() + Duration::from_millis(30);
        commit_timed(&mut h, &mut c, &surface, at);
        tick(&mut h.state);
        assert_eq!(buffer_scale(&server), 1, "too early");
        std::thread::sleep(Duration::from_millis(40));
        tick(&mut h.state);
        assert_eq!(buffer_scale(&server), 2, "its time passed");
    }

    #[test]
    fn a_shown_timed_commit_applies_before_the_frame_that_shows_at_its_time() {
        let (mut h, output) = harness();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "timed", 200, 150);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        let at = h.state.now() + Duration::from_secs(60);
        commit_timed(&mut h, &mut c, &surface, at);
        before_frame(&mut h.state, &output, at - HZ60);
        assert_eq!(buffer_scale(&server), 1, "the frame shows before its time");
        before_frame(&mut h.state, &output, at);
        assert_eq!(buffer_scale(&server), 2, "the frame shows at its time");
    }

    #[test]
    fn a_tick_frees_a_hidden_surface_and_leaves_a_shown_one_to_its_output() {
        let now = Duration::from_secs(100);
        let all = Release {
            fifo: true,
            timers_by: Some(now),
        };
        assert_eq!(on_tick(false, now), all);
        assert_eq!(on_tick(true, now), Release::default());
    }
}
