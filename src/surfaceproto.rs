//! Protocols that only shape how a surface is drawn: viewporter,
//! fractional-scale, single-pixel-buffer, content-type, alpha-modifier,
//! ext-background-effect (`crate::clientglass`), linux-drm-syncobj, fifo and
//! commit-timing. Smithay keeps their state on the surface; Reprise adds the
//! globals, a first preferred scale and the commit blockers.
//!
//! A surface's primary output passes its fifo barriers and commit timers (`frame_done`,
//! `before_frame`), asked at commit for the frame a timer shows in (`ask_frame`); a surface no
//! output shows (hidden, or covered by opaque windows) has them passed at the refresh rate, or
//! at `[windows] hidden_frame_rate` when that is slower, without asking a frame (`tick`).
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
        rustix::time::Timespec,
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
            Timestamp,
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
    /// The timer running while `waiting` holds a surface (`tick`, every `tick_interval`).
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
            .and_then(deadline);
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
    if protos.tick.is_none() {
        start_tick(state);
    }
}

fn start_tick(state: &mut State) {
    let token =
        state
            .handle
            .insert_source(Timer::from_duration(tick_interval(state)), |_, _, state| {
                if tick(state) {
                    TimeoutAction::ToDuration(tick_interval(state))
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

/// `[windows] hidden_frame_rate` changed: a running tick restarts at the new interval.
pub fn hidden_rate_changed(state: &mut State) {
    if let Some(token) = state.surface_protos.tick.take() {
        state.handle.remove(token);
        start_tick(state);
    }
}

/// Ask `output` for the frame that shows at `at` (monotonic). The tick runs out of
/// phase with the output's flips: a frame it asked for could start just before the refresh it
/// meant to follow and show the commit a refresh late.
fn ask_frame(state: &mut State, output: &Output, at: Duration) {
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

/// How often `tick` runs: the refresh interval, or `[windows] hidden_frame_rate`'s when longer.
fn tick_interval(state: &State) -> Duration {
    capped(refresh(state), state.config.hidden_frame_rate)
}

/// `refresh`, no shorter than a frame at `hz` frames a second; 0 is no cap.
fn capped(refresh: Duration, hz: u32) -> Duration {
    match hz {
        0 => refresh,
        hz => refresh.max(Duration::from_secs(1) / hz),
    }
}

/// The time `surface`'s earliest commit timer waits for (monotonic).
fn next_timer(surface: &WlSurface) -> Option<Duration> {
    with_states(surface, |data| {
        (data.data_map.get::<CommitTimerBarrierStateUserData>())
            .and_then(|t| t.lock().ok()?.next_deadline())
            .and_then(deadline)
    })
}

/// A commit timer's time on the monotonic clock; `None` for a timespec out of range (negative,
/// or 1e9 ns or more), which the client sent as it liked.
fn deadline(t: Timestamp) -> Option<Duration> {
    let tp = Timespec::from(Time::<Monotonic>::from(t));
    let secs = u64::try_from(tp.tv_sec).ok()?;
    let nanos = u32::try_from(tp.tv_nsec)
        .ok()
        .filter(|n| *n < 1_000_000_000)?;
    Some(Duration::new(secs, nanos))
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

/// The tick timer's turn: whether it runs again.
fn tick(state: &mut State) -> bool {
    let now = state.now();
    release(state, |_, primary| on_tick(primary.is_some(), now));
    !state.surface_protos.waiting.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::harness::{Harness, Presented, TestClient, Toplevel};
    use smithay::output::Mode;
    use wayland_client::{Proxy, protocol::wl_surface::WlSurface as ClientSurface};
    use wayland_protocols::wp::{
        commit_timing::v1::client::{
            wp_commit_timer_v1::{self, WpCommitTimerV1},
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
        let secs = at.as_secs();
        commit_timed_raw(
            h,
            c,
            surface,
            ((secs >> 32) as u32, secs as u32, at.subsec_nanos()),
        );
        h.roundtrip(c);
    }

    /// Commit buffer scale 2 under a commit timer set to `(tv_sec_hi, tv_sec_lo, tv_nsec)`, and
    /// let the compositor read it: the interface and code of the protocol error the client got.
    fn commit_timed_raw(
        h: &mut Harness,
        c: &mut TestClient,
        surface: &ClientSurface,
        (hi, lo, nsec): (u32, u32, u32),
    ) -> Option<(String, u32)> {
        let manager: WpCommitTimingManagerV1 = c.bind("wp_commit_timing_manager_v1", 1);
        let timer: WpCommitTimerV1 = manager.get_timer(surface, &c.qh(), ());
        timer.set_timestamp(hi, lo, nsec);
        surface.set_buffer_scale(2);
        surface.commit();
        c.queue.flush().expect("client flush");
        h.dispatch();
        if let Some(guard) = c.queue.prepare_read() {
            let _ = guard.read();
        }
        let _ = c.queue.dispatch_pending(&mut c.side);
        let error = wayland_client::Connection::from_backend(timer.backend().upgrade()?)
            .protocol_error()?;
        Some((error.object_interface, error.code))
    }

    #[test]
    fn a_timestamp_out_of_range_leaves_the_compositor_serving_frames() {
        let invalid = Some((
            WpCommitTimerV1::interface().name.to_string(),
            wp_commit_timer_v1::Error::InvalidTimestamp as u32,
        ));
        let timestamps = [
            ((u32::MAX, u32::MAX, u32::MAX), invalid.clone()),
            ((0x8000_0000, 0, 0), invalid),
            ((0x7fff_ffff, u32::MAX, 999_999_999), None),
        ];
        for (raw, error) in timestamps {
            let (mut h, output) = harness();
            let mut c = h.connect();
            let t = h.map_toplevel(&mut c, "timed", 200, 150);
            let surface = c.side.shells[t.shell].surface.clone();
            assert_eq!(
                commit_timed_raw(&mut h, &mut c, &surface, raw),
                error,
                "the protocol error for {raw:x?}"
            );
            let now = h.state.now();
            before_frame(&mut h.state, &output, now);
            tick(&mut h.state);

            let mut other = h.connect();
            let o = h.map_toplevel(&mut other, "other", 200, 150);
            let os = other.side.shells[o.shell].surface.clone();
            let done = other.frame(&os);
            os.commit();
            h.roundtrip(&mut other);
            crate::backend::send_frame_callbacks(&mut h.state, &output);
            h.roundtrip(&mut other);
            assert!(
                done.load(std::sync::atomic::Ordering::Relaxed),
                "another client's frame callback is done after {raw:x?}"
            );
        }
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
    fn an_alpha_only_fade_that_uncovers_a_waiting_fifo_surface_asks_its_output() {
        use wayland_protocols::wp::alpha_modifier::v1::client::wp_alpha_modifier_v1::WpAlphaModifierV1;
        let (mut h, output) = harness();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "fifo", 200, 150);
        let f = h.map_toplevel(&mut c, "player", 300, 200);
        h.state.set_fullscreen(f.id, true);
        h.roundtrip(&mut c);
        h.draw_opaque(&mut c, f, 0xff00_0000);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        assert_eq!(
            h.state.scene.surface_primary_output(&server),
            None,
            "covered"
        );
        let am: WpAlphaModifierV1 = c.bind("wp_alpha_modifier_v1", 1);
        let fs = c.side.shells[f.shell].surface.clone();
        let ams = am.get_surface(&fs, &c.qh(), ());
        h.roundtrip(&mut c);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert_eq!(buffer_scale(&server), 1, "waits on the barrier");
        crate::backend::take_asks();
        ams.set_multiplier(u32::MAX / 2);
        fs.commit();
        h.roundtrip(&mut c);
        assert_eq!(
            h.state.scene.surface_primary_output(&server).as_ref(),
            Some(&output),
            "shown through the faded window"
        );
        tick(&mut h.state);
        let asked = crate::backend::take_asks();
        assert_eq!(buffer_scale(&server), 1, "the tick leaves it to its output");
        assert!(
            !asked.is_empty(),
            "the output is asked for the frame that passes the barrier: {asked:?}"
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

    /// The standby, an uncaptured virtual screen ticking once a second, with a drawn window on it
    /// and its clock run until idle; the window's surface on both sides.
    fn on_standby() -> (Harness, Output, TestClient, Toplevel, WlSurface) {
        let mut h = Harness::new();
        crate::standby::rdp_changed(&mut h.state, true);
        h.dispatch();
        let output = h.state.scene.outputs()[0].clone();
        assert!(crate::backend::is_virtual(&output));
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "timed", 200, 150);
        h.draw_self(&mut c, t, 0xff00_8000);
        h.run_for(1100);
        assert_eq!(crate::virtualout::armed(&h.state, &output), None, "idle");
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        assert_eq!(
            h.state.scene.surface_primary_output(&server).as_ref(),
            Some(&output)
        );
        (h, output, c, t, server)
    }

    fn apart(a: std::time::Instant, b: std::time::Instant) -> Duration {
        a.max(b) - a.min(b)
    }

    #[test]
    fn a_timed_commit_on_a_virtual_screen_arms_its_tick_for_its_time_and_asks_none_before() {
        let (mut h, output, mut c, t, server) = on_standby();
        let surface = c.side.shells[t.shell].surface.clone();
        let lead = Duration::from_secs(60);
        let expect = std::time::Instant::now() + lead;
        let at = h.state.now() + lead;
        commit_timed(&mut h, &mut c, &surface, at);
        let (armed, wants) =
            crate::virtualout::armed(&h.state, &output).expect("the clock is armed");
        assert_eq!(wants, None, "nothing asks a tick before the commit's time");
        assert!(
            apart(armed, expect) < Duration::from_millis(5),
            "the tick is armed 60 s ahead, {:?} off",
            apart(armed, expect)
        );
        assert_eq!(buffer_scale(&server), 1, "the commit waits");
    }

    #[test]
    fn later_timed_commits_on_a_virtual_screen_keep_the_earliest_tick_and_ask_no_frame() {
        let (mut h, output, mut c, t, server) = on_standby();
        let surface = c.side.shells[t.shell].surface.clone();
        let manager: WpCommitTimingManagerV1 = c.bind("wp_commit_timing_manager_v1", 1);
        let timer: WpCommitTimerV1 = manager.get_timer(&surface, &c.qh(), ());
        let expect = std::time::Instant::now() + Duration::from_secs(60);
        let now = h.state.now();
        for (lead, scale) in [(60, 2), (120, 3)] {
            let at = now + Duration::from_secs(lead);
            let secs = at.as_secs();
            timer.set_timestamp((secs >> 32) as u32, secs as u32, at.subsec_nanos());
            surface.set_buffer_scale(scale);
            surface.commit();
            h.roundtrip(&mut c);
        }
        for _ in 0..3 {
            let now = h.state.now();
            before_frame(&mut h.state, &output, now);
        }
        let (armed, wants) =
            crate::virtualout::armed(&h.state, &output).expect("the clock is armed");
        assert_eq!(wants, None, "the asks again add no frame");
        assert!(
            apart(armed, expect) < Duration::from_millis(5),
            "the tick stays armed for the first commit, {:?} off",
            apart(armed, expect)
        );
        assert_eq!(buffer_scale(&server), 1, "both commits wait");
    }

    /// The standby's next tick, as its clock arms it for an ask now.
    fn next_tick(h: &Harness, output: &Output) -> std::time::Instant {
        let last = crate::virtualout::last_tick(&h.state, output);
        crate::virtualout::next_tick(last, Duration::from_secs(1), std::time::Instant::now())
    }

    #[test]
    fn an_untimed_or_overdue_commit_on_a_virtual_screen_asks_its_next_tick() {
        let (mut h, output, mut c, t, server) = on_standby();
        let surface = c.side.shells[t.shell].surface.clone();
        let now = h.state.now();
        commit_timed(&mut h, &mut c, &surface, now - Duration::from_millis(1));
        let (armed, wants) =
            crate::virtualout::armed(&h.state, &output).expect("the clock is armed");
        let next = next_tick(&h, &output);
        assert_eq!(wants, None);
        assert!(
            apart(armed, next) < Duration::from_millis(5),
            "an overdue commit ticks at the next tick, {:?} off",
            apart(armed, next)
        );
        crate::virtualout::tick_now(&mut h.state, &output);
        assert_eq!(buffer_scale(&server), 2, "the next tick applies it");

        let (mut h, output, mut c, t, _) = on_standby();
        h.draw_self(&mut c, t, 0xff00_0080);
        let (armed, wants) =
            crate::virtualout::armed(&h.state, &output).expect("the clock is armed");
        let next = next_tick(&h, &output);
        assert_eq!(wants, Some(crate::virtualout::Wants::Frame));
        assert!(
            apart(armed, next) < Duration::from_millis(5),
            "an untimed commit ticks at the next tick, {:?} off",
            apart(armed, next)
        );
    }

    #[test]
    fn a_virtual_screens_tick_before_a_timed_commits_time_leaves_it_waiting() {
        let (mut h, output, mut c, t, server) = on_standby();
        let surface = c.side.shells[t.shell].surface.clone();
        let at = h.state.now() + Duration::from_millis(200);
        commit_timed(&mut h, &mut c, &surface, at);
        crate::virtualout::tick_now(&mut h.state, &output);
        assert_eq!(buffer_scale(&server), 1, "too early");
        std::thread::sleep(Duration::from_millis(250));
        crate::virtualout::tick_now(&mut h.state, &output);
        assert_eq!(
            buffer_scale(&server),
            2,
            "the tick past its time applies it"
        );
    }

    /// Draw `t` in `argb` under a commit timer already gone by, so the standby's next tick
    /// applies it.
    fn draw_overdue(h: &mut Harness, c: &mut TestClient, t: Toplevel, argb: u32) {
        let surface = c.side.shells[t.shell].surface.clone();
        let manager: WpCommitTimingManagerV1 = c.bind("wp_commit_timing_manager_v1", 1);
        let timer: WpCommitTimerV1 = manager.get_timer(&surface, &c.qh(), ());
        let at = h.state.now() - Duration::from_millis(1);
        let secs = at.as_secs();
        timer.set_timestamp((secs >> 32) as u32, secs as u32, at.subsec_nanos());
        h.draw_self(c, t, argb);
    }

    fn commit_asked(output: &Output) -> bool {
        (crate::backend::take_asks().iter())
            .any(|(o, a)| *o == output.name() && matches!(a, crate::backend::Ask::Commit(_)))
    }

    /// Ticks the standby's clock for a frame now and waits for it, so its next tick is a whole
    /// second away.
    fn fresh_tick(h: &mut Harness, output: &Output) -> std::time::Instant {
        let last = crate::virtualout::last_tick(&h.state, output).expect("the standby ticked");
        crate::virtualout::ask(&mut h.state, output, crate::backend::Ask::Scene);
        tick_after(h, output, last)
    }

    /// Runs the loop until the standby's clock ticks after `last`; that tick's time.
    fn tick_after(
        h: &mut Harness,
        output: &Output,
        last: std::time::Instant,
    ) -> std::time::Instant {
        let o = output.clone();
        h.until("the standby's tick", move |s| {
            crate::virtualout::last_tick(s, &o) != Some(last)
        });
        crate::virtualout::last_tick(&h.state, output).expect("the standby ticked")
    }

    /// What the standby's armed clock ticks for besides a timed commit, `None` when unarmed.
    fn armed_for(h: &Harness, output: &Output) -> Option<Option<crate::virtualout::Wants>> {
        crate::virtualout::armed(&h.state, output).map(|(_, wants)| wants)
    }

    #[test]
    fn a_timed_commit_a_virtual_tick_applies_takes_one_frame_and_leaves_its_clock_unarmed() {
        let (mut h, output, mut c, t, _) = on_standby();
        let surface = c.side.shells[t.shell].surface.clone();
        let shown = c.presentation_feedback(&surface);
        let last = fresh_tick(&mut h, &output);
        draw_overdue(&mut h, &mut c, t, 0xff00_0080);
        let ticked = tick_after(&mut h, &output, last);
        assert_eq!(
            armed_for(&h, &output),
            None,
            "no clock stays armed after the frame that showed the commit"
        );
        h.run_for(1100);
        assert_eq!(
            crate::virtualout::last_tick(&h.state, &output),
            Some(ticked),
            "no tick after it"
        );
        h.roundtrip(&mut c);
        assert!(
            matches!(*shown.lock().unwrap(), Some(Presented::Shown { .. })),
            "the tick's frame presented the commit"
        );
    }

    #[test]
    fn an_animation_stepped_in_a_virtual_tick_that_applies_a_timed_commit_gets_its_next_tick() {
        let (mut h, output, mut c, t, _) = on_standby();
        let last = fresh_tick(&mut h, &output);
        h.state.config.accessibility.visual_bell = crate::config::VisualBell::Screen;
        h.state.flash_bell(None, std::time::Instant::now());
        draw_overdue(&mut h, &mut c, t, 0xff00_0080);
        crate::backend::take_asks();
        tick_after(&mut h, &output, last);
        assert!(commit_asked(&output), "the tick applies the commit");
        assert_eq!(
            armed_for(&h, &output),
            Some(Some(crate::virtualout::Wants::Frame)),
            "the flash's next step"
        );
    }

    #[test]
    fn a_commit_after_a_virtual_tick_that_applied_a_timed_one_gets_the_next_tick() {
        let (mut h, output, mut c, t, _) = on_standby();
        let last = fresh_tick(&mut h, &output);
        draw_overdue(&mut h, &mut c, t, 0xff00_0080);
        tick_after(&mut h, &output, last);
        assert_eq!(armed_for(&h, &output), None);
        h.draw_self(&mut c, t, 0xff80_0000);
        assert_eq!(
            armed_for(&h, &output),
            Some(Some(crate::virtualout::Wants::Frame)),
            "the new commit's frame"
        );
    }

    #[test]
    fn a_virtual_screens_tick_for_another_ask_arms_its_clock_again_for_a_timed_commit() {
        let asks: [fn(&mut State, &Output); 2] = [
            |state, _| crate::virtualout::answer_frames(state),
            |state, output| crate::virtualout::ask(state, output, crate::backend::Ask::Scene),
        ];
        for ask in asks {
            let (mut h, output, mut c, t, server) = on_standby();
            let surface = c.side.shells[t.shell].surface.clone();
            let expect = std::time::Instant::now() + Duration::from_secs(60);
            let at = h.state.now() + Duration::from_secs(60);
            commit_timed(&mut h, &mut c, &surface, at);
            ask(&mut h.state, &output);
            let (armed, _) =
                crate::virtualout::armed(&h.state, &output).expect("the clock is armed");
            assert!(armed < expect, "the other ask ticks first");
            h.run_for(1100);
            let (armed, wants) = crate::virtualout::armed(&h.state, &output)
                .expect("the clock is armed again for the commit");
            assert_eq!(wants, None);
            assert!(
                apart(armed, expect) < Duration::from_millis(5),
                "the tick is armed 60 s ahead, {:?} off",
                apart(armed, expect)
            );
            assert_eq!(buffer_scale(&server), 1, "the commit waits");
        }
    }

    /// As a FIFO swapchain presents: a commit timed for `at`, waiting on the last barrier and
    /// setting the next, with buffer scale `scale`.
    fn present(
        h: &mut Harness,
        c: &mut TestClient,
        surface: &ClientSurface,
        (fifo, timer): (&WpFifoV1, &WpCommitTimerV1),
        at: Duration,
        scale: i32,
    ) {
        let secs = at.as_secs();
        timer.set_timestamp((secs >> 32) as u32, secs as u32, at.subsec_nanos());
        fifo.wait_barrier();
        fifo.set_barrier();
        surface.set_buffer_scale(scale);
        surface.commit();
        h.roundtrip(c);
    }

    #[test]
    fn a_covered_surface_asks_no_frame_and_passes_its_barrier_and_timer_on_the_tick() {
        use crate::backend::{Ask, take_asks};
        let (mut h, output) = harness();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "fifo", 200, 150);
        let cover = h.map_toplevel(&mut c, "cover", 300, 200);
        h.state.set_fullscreen(cover.id, true);
        h.roundtrip(&mut c);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        let fifos: WpFifoManagerV1 = c.bind("wp_fifo_manager_v1", 1);
        let fifo: WpFifoV1 = fifos.get_fifo(&surface, &c.qh(), ());
        let timers: WpCommitTimingManagerV1 = c.bind("wp_commit_timing_manager_v1", 1);
        let timer: WpCommitTimerV1 = timers.get_timer(&surface, &c.qh(), ());
        fifo.set_barrier();
        surface.commit();
        h.roundtrip(&mut c);

        take_asks();
        let now = h.state.now();
        present(&mut h, &mut c, &surface, (&fifo, &timer), now, 2);
        assert!(
            take_asks().contains(&(output.name(), Ask::Timed)),
            "under a transparent cover the timed commit asks its frame"
        );
        tick(&mut h.state);
        assert_eq!(buffer_scale(&server), 1, "the tick leaves it to its output");
        frame_done(&mut h.state, &output);
        let now = h.state.now();
        before_frame(&mut h.state, &output, now);
        assert_eq!(buffer_scale(&server), 2, "the output's frame passes it");

        h.draw_opaque(&mut c, cover, 0xff00_0000);
        assert_eq!(h.state.scene.surface_primary_output(&server), None);
        take_asks();
        let now = h.state.now();
        present(&mut h, &mut c, &surface, (&fifo, &timer), now, 3);
        assert_eq!(buffer_scale(&server), 2, "the covered commit waits");
        tick(&mut h.state);
        assert_eq!(
            buffer_scale(&server),
            3,
            "the tick passes barrier and timer"
        );
        assert_eq!(take_asks(), vec![], "neither commit nor apply asks a frame");
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

    #[test]
    fn the_tick_runs_at_the_refresh_rate_or_the_hidden_frame_rate_when_that_is_slower() {
        assert_eq!(capped(HZ60, 0), HZ60, "0 keeps the refresh rate");
        assert_eq!(capped(HZ60, 30), Duration::from_nanos(33_333_333));
        assert_eq!(capped(HZ60, 120), HZ60, "a cap above the refresh rate");
    }

    /// A harness whose one output refreshes 100 times a second, with `[windows]
    /// hidden_frame_rate = rate`.
    fn at_100hz(rate: u32) -> Harness {
        let config = Config::parse(&format!("[windows]\nhidden_frame_rate = {rate}\n"));
        let mut h = Harness::with_config(config.expect("the config"));
        let output = h.output("A", 800, 600);
        let mode = Mode {
            size: (800, 600).into(),
            refresh: 100_000,
        };
        output.change_current_state(Some(mode), None, None, None);
        h
    }

    /// Turns of the loop until `server` shows buffer scale 2, for at most `limit`.
    fn applies_within(h: &mut Harness, server: &WlSurface, limit: Duration) -> bool {
        let end = std::time::Instant::now() + limit;
        while buffer_scale(server) != 2 && std::time::Instant::now() < end {
            (h.event_loop)
                .dispatch(Some(Duration::from_millis(2)), &mut h.state)
                .expect("dispatch");
            h.state.turn_end();
        }
        buffer_scale(server) == 2
    }

    #[test]
    fn a_hidden_fifo_surface_passes_its_barrier_at_the_hidden_frame_rate() {
        let mut h = at_100hz(0);
        assert_eq!(tick_interval(&h.state), Duration::from_millis(10));
        let mut c = h.connect();
        let surface = h.bare_client_surface(&mut c);
        let server = server_side(&h, &c, &surface);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert!(
            applies_within(&mut h, &server, Duration::from_millis(300)),
            "the 10 ms tick passes the barrier"
        );

        let mut h = at_100hz(4);
        assert_eq!(tick_interval(&h.state), Duration::from_millis(250));
        let mut c = h.connect();
        let surface = h.bare_client_surface(&mut c);
        let server = server_side(&h, &c, &surface);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert!(
            !applies_within(&mut h, &server, Duration::from_millis(100)),
            "no tick before 250 ms"
        );
        assert!(
            applies_within(&mut h, &server, Duration::from_secs(2)),
            "the 250 ms tick passes the barrier"
        );
    }

    #[test]
    fn a_shown_fifo_surface_passes_its_barrier_on_its_outputs_frame_under_a_hidden_frame_rate() {
        let mut h = at_100hz(1);
        let output = h.state.scene.outputs()[0].clone();
        let mut c = h.connect();
        let t = h.map_toplevel(&mut c, "fifo", 200, 150);
        let surface = c.side.shells[t.shell].surface.clone();
        let server = server_side(&h, &c, &surface);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert_eq!(buffer_scale(&server), 1, "the commit waits on the barrier");
        frame_done(&mut h.state, &output);
        assert_eq!(buffer_scale(&server), 2, "the output's frame passes it");
    }

    #[test]
    fn a_reload_restarts_the_tick_at_the_new_hidden_frame_rate() {
        let mut h = at_100hz(1);
        let mut c = h.connect();
        let surface = h.bare_client_surface(&mut c);
        let server = server_side(&h, &c, &surface);
        commit_behind_barrier(&mut h, &mut c, &surface);
        assert_eq!(tick_interval(&h.state), Duration::from_secs(1));
        crate::configwatch::reload(&mut h.state, Config::default());
        assert_eq!(tick_interval(&h.state), Duration::from_millis(10));
        assert!(
            applies_within(&mut h, &server, Duration::from_millis(500)),
            "the restarted tick passes the barrier well before the old 1 s one"
        );
        let capped = Config::parse("[windows]\nhidden_frame_rate = 30\n").unwrap();
        crate::configwatch::reload(&mut h.state, capped);
        assert_eq!(tick_interval(&h.state), Duration::from_nanos(33_333_333));
    }
}
