use std::{sync::Arc, time::Duration};

use smithay::{
    backend::{
        drm::{
            DrmEventMetadata, DrmEventTime,
            compositor::{CursorMove, FrameFlags, PrimaryPlaneElement},
        },
        renderer::{gles::GlesRenderer, utils::DamageSnapshot},
        session::Session,
    },
    output::Output,
    reexports::wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
    reexports::{
        calloop::timer::{TimeoutAction, Timer},
        drm::control::crtc,
    },
    utils::Monotonic,
    wayland::presentation::Refresh,
};

use crate::{
    compositor::State,
    framepace::Done,
    glass::{Hold, Under},
    reblur::Reblur,
    render::{OutputElement, Parts},
};

use super::gamma::{Gamma, flush_gamma};
use super::links::retrain_link;
use super::outputs::power_off;
use super::schedule::queue_render;
use super::steps::{on_quiet, watch};
use super::{
    Flip, InFlight, Out, Surface, Work, commit, crtc_of, lock, schedule, schedule_step, surface_mut,
};

fn flip_time(
    state: &State,
    meta: Option<&DrmEventMetadata>,
) -> (Duration, u64, wp_presentation_feedback::Kind) {
    use wp_presentation_feedback::Kind;
    let seq = meta.map_or(0, |m| u64::from(m.sequence));
    match meta.map(|m| m.time) {
        Some(DrmEventTime::Monotonic(t)) if !t.is_zero() => {
            (t, seq, Kind::Vsync | Kind::HwClock | Kind::HwCompletion)
        }
        _ => (state.clock.now().into(), seq, Kind::Vsync),
    }
}

pub(super) fn vblank(state: &mut State, crtc: crtc::Handle, meta: Option<&DrmEventMetadata>) {
    let flip = flip_time(state, meta);
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    if let Some(flip) = s.track.vblank(flip) {
        flipped(state, crtc, flip);
    }
}

/// The frame queued on `crtc` reached the screen at `flip`.
pub(super) fn flipped(state: &mut State, crtc: crtc::Handle, (time, seq, kind): Flip) {
    let counting = state.config.debug.overlay;
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    let submitted = lock(&s.compositor).frame_submitted();
    if let Err(e) = submitted {
        log::warn!("{}: frame submit: {e}", s.output.name());
    }
    if let Some(mut feedback) = s.presenting.take() {
        let refresh = s.output.current_mode().map_or(60_000, |m| m.refresh.max(1));
        let refresh = Duration::from_micros(1_000_000_000 / refresh as u64);
        feedback.presented::<_, Monotonic>(time, Refresh::fixed(refresh), seq, kind);
    }
    s.last_flip = Some(std::time::Instant::now());
    if counting {
        s.stats.flipped(time, s.needs_redraw);
    }
    let (output, redraw) = (s.output.clone(), s.needs_redraw);
    retrain_link(state, crtc);
    flush_gamma(state, crtc);
    crate::power::presented(state, &output);
    crate::sessionlock::presented(state, &output);
    crate::sessionfade::presented(state, &output);
    if redraw {
        render(state, crtc);
    } else {
        state.glass.output_stopped(&output);
    }
    watch(state);
}

pub(super) fn render(state: &mut State, crtc: crtc::Handle) {
    state.settle_moves();
    let Some(output) = due(state, crtc) else {
        return;
    };
    if held_back(state, crtc) {
        return;
    }
    let started = std::time::Instant::now();
    let mut asked_at = None;
    if let Some(s) = surface_mut(state, crtc) {
        s.needs_redraw = false;
        asked_at = s.asked_at.take();
        if std::mem::take(&mut s.pointer_only)
            && let Some(drawn) = pointer_frame(state, crtc, &output, started)
        {
            // A queued cursor frame finishes on its `Done` (`committed`).
            if !drawn {
                finish(state, crtc, &output, false, true, None, started);
            }
            watch(state);
            return;
        }
    }
    state.window_anims_frame(&output);
    state.switch_anim_frame(&output);
    state.overview_fade_frame();
    state.flip_frame(&output);
    state.water_frame(&output);
    crate::autohideview::frame(state, &output);
    crate::snapbarview::frame(state, &output);
    let border = crate::captureborder::elements(state, &output);
    let (fade, fade_next) = crate::sessionfade::element(state, &output);
    let pointer = state.pointer_location();
    let focus = state.magnifier_focus(&output);
    let State {
        backend,
        scene,
        cursor,
        saver,
        cursors,
        config,
        glass,
        water,
        interaction,
        magnifier,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
    let Some(b) = backend.as_mut() else {
        return;
    };
    let Some(s) = b.surfaces.get_mut(&crtc) else {
        return;
    };
    s.pointer_shown =
        reprisescene::output_rect(&output).is_some_and(|g| g.to_f64().contains(pointer));
    s.scene_seen = scene.changes();
    let now = std::time::Instant::now();
    let at = asked_at.unwrap_or(now);
    let cap = crate::reblur::cap(
        config.glass.blur_rate,
        crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh)),
    );
    let catch_up = std::mem::take(&mut s.catch_up);
    let reblur = crate::reblur::when(
        !std::mem::take(&mut s.scene_changed) && !catch_up,
        s.last_blur.map(|t| at.saturating_duration_since(t)),
        cap,
    );
    let hold = match reblur {
        Reblur::Now => {
            // A catch-up keeps the cap's phase: the program's next commit blurs on time.
            if !catch_up {
                s.last_blur = Some(at);
            }
            None
        }
        Reblur::Wait(wait) => Some(wait.saturating_sub(now - at)),
    };
    let stepped = water.step(&mut b.renderer, &output);
    let glass_hold = glass_hold(hold, std::mem::take(&mut s.backdrop_changed), stepped.moved);
    glass.begin(
        &output,
        config.glass,
        crate::glass::quality(interaction, &config.glass),
        glass_hold,
    );
    let clear = crate::render::clear_color(&config.theme.colors);
    let filter = crate::magnifier::filter_matrix(&config.accessibility);
    let wanted = crate::glass::wanted(config);
    glass.update_screen_backdrop(&mut b.renderer, scene, &output, clear, wanted);
    let mut build = |renderer: &mut GlesRenderer| {
        crate::magnifier::output_elements(
            magnifier,
            renderer,
            Parts {
                scene,
                cursor,
                cursors,
                pointer,
            },
            &output,
            clear,
            filter,
            focus,
        )
    };
    let mut elements = build(&mut b.renderer);
    let scale = smithay::utils::Scale::from(output.current_scale().fractional_scale());
    if glass.settle(&output, &elements, scale) {
        drop(elements);
        glass.update_backdrop(&mut b.renderer, scene, &output, clear, wanted);
        elements = build(&mut b.renderer);
    }
    if !border.is_empty() {
        use smithay::backend::renderer::element::Element;
        let below_pointer = elements
            .iter()
            .position(|e| e.kind() != smithay::backend::renderer::element::Kind::Cursor)
            .unwrap_or(elements.len());
        elements.splice(
            below_pointer..below_pointer,
            border.into_iter().map(OutputElement::Border),
        );
    }
    if let Some(fade) = fade {
        elements.insert(0, OutputElement::Border(fade));
    }
    b.shm_textures.update(&mut b.renderer);
    let mut empty = false;
    let shared = Arc::clone(&s.compositor);
    let mut compositor = lock(&shared);
    let rendered = compositor.render_frame(&mut b.renderer, &elements, clear, FrameFlags::DEFAULT);
    let modeset = compositor.surface().commit_pending();
    let mut feedback = None;
    let drawn = match rendered {
        Ok(r) => {
            if let PrimaryPlaneElement::Swapchain(e) = &r.primary_element {
                crate::gpufence::issued(&b.renderer, &e.sync);
            }
            let pass = scene.surface_pass(&output);
            scene.update_surfaces(&pass, &r.states);
            if r.is_empty {
                empty = true;
                false
            } else {
                if config.debug.overlay
                    && let PrimaryPlaneElement::Swapchain(e) = &r.primary_element
                {
                    count_damage(s, &e.damage, &output);
                }
                feedback = Some(scene.take_presentation_feedback(&pass, &r.states));
                true
            }
        }
        Err(e) => {
            log::warn!("{}: render: {e:?}", output.name());
            false
        }
    };
    drop(compositor);
    drop(elements);
    let held = hold.filter(|_| glass.any_stale());
    if held.is_some() {
        s.held += 1;
    }
    let owed = cap.filter(|_| hold.is_some() && held.is_none() && glass.owes(&output));
    let water_moves = water.next_step(&output).is_some();
    let held = crate::reblur::catch_up(held, water_moves);
    let owed = crate::reblur::catch_up(owed, water_moves);
    let flight = InFlight {
        started,
        held,
        feedback,
        pointer: false,
    };
    let flipping = drawn && send_flip(s, modeset, flight);
    if !flipping {
        finish(state, crtc, &output, false, empty, held, started);
    }
    if let Some(wait) = owed {
        state.arm_catch_up(&output, wait);
    }
    if state.magnifier.panning(&output)
        || stepped.fell
        || fade_next == crate::sessionfade::Next::Frame
    {
        schedule(state, &output);
    }
    if let Some(at) = state.water.next_step(&output) {
        schedule_step(state, &output, at);
    }
    let asked = (state.backend.as_ref())
        .and_then(|b| b.surfaces.get(&crtc))
        .is_none_or(|s| s.needs_redraw);
    if !flipping && !asked {
        state.glass.output_stopped(&output);
    }
    watch(state);
}

/// A frame asked for by pointer motion alone (`Ask::Pointer`): when the last
/// frame put the pointer on its own plane, move the plane and queue that.
/// Some(whether a frame was queued); None when the frame must be drawn normally.
pub(super) fn pointer_frame(
    state: &mut State,
    crtc: crtc::Handle,
    output: &Output,
    started: std::time::Instant,
) -> Option<bool> {
    let pointer = state.pointer_location();
    let State {
        backend,
        scene,
        cursor,
        saver,
        cursors,
        magnifier,
        ..
    } = state;
    let cursor = crate::saverview::cursor(saver, cursor);
    let b = backend.as_mut()?;
    let s = b.surfaces.get_mut(&crtc)?;
    if magnifier.zooms(output) || s.callback_timer.is_some() || s.scene_seen != scene.changes() {
        return None;
    }
    let geo = reprisescene::output_rect(output)?;
    if !geo.to_f64().contains(pointer) {
        return (!s.pointer_shown).then_some(false);
    }
    let parts = Parts {
        scene,
        cursor,
        cursors,
        pointer,
    };
    let elements =
        crate::render::pointer_elements(&mut b.renderer, parts, output, pointer - geo.loc.to_f64());
    let [element] = elements.as_slice() else {
        return None;
    };
    let mut compositor = lock(&s.compositor);
    let moved = compositor.reposition_cursor(element);
    let modeset = compositor.surface().commit_pending();
    drop(compositor);
    match moved {
        CursorMove::Unavailable => None,
        CursorMove::Unchanged => Some(false),
        CursorMove::Prepared => {
            let flight = InFlight {
                started,
                held: None,
                feedback: None,
                pointer: true,
            };
            Some(send_flip(s, modeset, flight))
        }
    }
}

/// Hand the frame `compositor` has ready to the output's commit thread; false when the thread
/// is gone. The frame finishes on its `Done` (`committed`).
fn send_flip(s: &mut Surface, modeset: bool, flight: InFlight) -> bool {
    if !send_job(s, commit::Kind::Flip { modeset }, Work::None) {
        return false;
    }
    s.flight = Some(flight);
    true
}

/// Hand `kind` to the output's commit thread; false when the thread is gone.
pub(super) fn send_job(s: &mut Surface, kind: commit::Kind, work: Work) -> bool {
    s.track.sent(kind, std::time::Instant::now());
    if !s.thread.send(commit::Job::Run(kind, work)) {
        log::error!("{}: the commit thread is gone", s.output.name());
        s.track.unsent();
        return false;
    }
    true
}

/// A disabled output's thread dropped its compositor and ended.
pub(super) fn retired(state: &mut State, crtc: crtc::Handle) {
    let Some(b) = state.backend.as_mut() else {
        return;
    };
    if let Some(h) = b.retiring.remove(&crtc) {
        let _ = h.join();
        log::debug!("{crtc:?}: its commit thread dropped the compositor and ended");
    }
    on_quiet(state);
    render_waiting(state);
}

/// The commit thread finished `done` on `crtc`.
pub(super) fn committed(state: &mut State, crtc: crtc::Handle, done: commit::Done<Out>) {
    let overlay = state.config.debug.overlay;
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    let waited = s.track.waited(done.ended);
    if s.track.stalled()
        && let Some(w) = waited
    {
        let kind = done.kind;
        log::warn!(
            "{}: {kind:?} returned after {:.1} s",
            s.output.name(),
            w.as_secs_f64()
        );
    }
    if let commit::Phase::Busy { since, .. } = s.track.phase() {
        let t = commit::Times {
            to_thread: done.started.saturating_duration_since(since),
            ioctl: done.ended.saturating_duration_since(done.started),
            to_main: done.ended.elapsed(),
        };
        if overlay {
            s.stats.committed(t.to_thread + t.to_main, t.ioctl);
        }
        if log::log_enabled!(log::Level::Debug) {
            s.commit_times.push(t);
        }
    }
    let ok = done.result.is_ok();
    let name = s.output.name();
    match (&done.result, done.kind) {
        (Err(e), commit::Kind::Flip { modeset: true }) => log::warn!("{name}: modeset: {e}"),
        (Err(e), commit::Kind::Flip { .. }) => log::warn!("{name}: queue frame: {e}"),
        (Err(e), commit::Kind::Clear) => log::warn!("{name}: power off: {e}"),
        (Err(e), commit::Kind::Gamma) => log::warn!("{name}: set gamma: {e}"),
        (Err(e), commit::Kind::Retrain) => log::warn!("{name}: link retrain: {e}"),
        (Err(e), commit::Kind::Reset) => log::warn!("{name}: device reset: {e}"),
        (Ok(_), commit::Kind::Retrain) => log::info!("{name}: link retrained"),
        (Ok(out), commit::Kind::Gamma) => {
            log::debug!("{name}: gamma set");
            if let Some((lut, blob)) = out.gamma.clone() {
                s.hw.lut = Some(lut);
                if let Some(Gamma::Lut { blob: b, .. }) = s.hw.gamma.as_mut() {
                    *b = blob;
                }
            }
        }
        (Ok(_), _) => {}
    }
    let gamma_failed = !ok && done.kind == commit::Kind::Gamma;
    let after = s.track.done(ok, std::time::Instant::now());
    if let Some(wait) = after.backoff {
        log::warn!(
            "{name}: the next modeset waits {} s (failed or slow)",
            wait.as_secs()
        );
    }
    let flipped_frame = matches!(done.kind, commit::Kind::Flip { .. });
    let mut flight = s.flight.take().filter(|_| flipped_frame);
    let feedback = flight.as_mut().and_then(|f| f.feedback.take());
    if ok && flipped_frame {
        s.lit_at = Some(done.started);
        s.hw.unlit = false;
        s.presenting = feedback;
    } else if let Some(mut f) = feedback {
        f.discarded();
    }
    let output = s.output.clone();
    if after.off {
        power_off(state, crtc);
    }
    if let Some(f) = flight {
        if ok && let Some(s) = surface_mut(state, crtc) {
            count_frame(s, overlay, f.started, f.pointer);
        }
        finish(state, crtc, &output, ok, false, f.held, f.started);
    }
    if gamma_failed {
        crate::nightlight::gamma_failed(state, &output);
    }
    match after.vblank {
        Some(flip) => flipped(state, crtc, flip),
        None => {
            flush_gamma(state, crtc);
            retrain_link(state, crtc);
            queue_render(state, crtc);
        }
    }
    on_quiet(state);
    if done.kind.blocking() || waited.is_some_and(|w| w > commit::STALL) {
        render_waiting(state);
    }
    watch(state);
}

/// Render every output that asked for a frame while something held them all.
pub(super) fn render_waiting(state: &mut State) {
    let waiting: Vec<crtc::Handle> = state
        .backend
        .iter()
        .flat_map(|b| &b.surfaces)
        .filter(|(_, s)| s.needs_redraw)
        .map(|(c, _)| *c)
        .collect();
    for c in waiting {
        queue_render(state, c);
    }
}

/// What a frame `hold`ing for a wait keeps of the last blur, its asks having changed what lies
/// under glass at `asked`: water that `stepped` this frame changed it anywhere, whatever asked.
pub(super) fn glass_hold(hold: Option<Duration>, asked: Under, stepped: bool) -> Hold {
    match hold {
        None => Hold::No,
        Some(_) if stepped => Hold::Keep(Under::Anywhere),
        Some(_) => Hold::Keep(asked),
    }
}

/// After a frame on `crtc` that was `drawn` or `empty`, with its glass
/// `held` for a wait: pace the next render, send the frame callbacks and
/// fill the captures that waited for it.
fn finish(
    state: &mut State,
    crtc: crtc::Handle,
    output: &Output,
    drawn: bool,
    empty: bool,
    held: Option<Duration>,
    started: std::time::Instant,
) {
    if let Some(s) = surface_mut(state, crtc) {
        let interval = crate::framepace::interval(output.current_mode().map_or(0, |m| m.refresh));
        s.pace.not_before = crate::framepace::next_render(drawn, s.needs_redraw, started, interval);
    }
    if let Some(wait) = held {
        state.arm_reblur(output, wait);
    }
    crate::sessionlock::rendered(state, output, drawn, empty);
    crate::sessionfade::rendered(state, output, empty);
    crate::power::rendered(state, output, drawn, empty);
    callbacks_after_a_frame(state, crtc, drawn);
    if drawn {
        crate::capture::output_drawn(state, output);
    }
}

fn held_back(state: &mut State, crtc: crtc::Handle) -> bool {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return false;
    };
    let retry = s
        .track
        .retry_at()
        .filter(|_| lock(&s.compositor).surface().commit_pending());
    let not_before = s.pace.not_before.max(retry);
    let Some(wait) = crate::framepace::render_wait(not_before, std::time::Instant::now()) else {
        s.pace.not_before = None;
        return false;
    };
    if s.render_timer.is_some() {
        return true;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            if let Some(s) = surface_mut(state, crtc) {
                s.render_timer = None;
            }
            render(state, crtc);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => {
            s.render_timer = Some(t);
            true
        }
        Err(e) => {
            log::warn!("render timer: {}", e.error);
            false
        }
    }
}

pub(super) fn due(state: &State, crtc: crtc::Handle) -> Option<Output> {
    let b = state.backend.as_ref()?;
    let s = b.surfaces.get(&crtc)?;
    (!b.dead
        && s.track.idle()
        && b.quiet()
        && !b.back.pending()
        && !s.hw.off
        && b.session.is_active())
    .then(|| s.output.clone())
}

fn count_frame(s: &mut Surface, overlay: bool, started: std::time::Instant, cursor_only: bool) {
    if overlay {
        s.stats.rendered(started.elapsed());
    }
    if !log::log_enabled!(log::Level::Debug) {
        return;
    }
    let now = std::time::Instant::now();
    let from = *s.counted_from.get_or_insert(now);
    s.frames += 1;
    s.cursor_frames += u32::from(cursor_only);
    if s.frames == 120 {
        let secs = now.duration_since(from).as_secs_f64().max(1e-3);
        let t = commit::tail(&s.commit_times);
        log::debug!(
            "{}: 120 frames in {secs:.2} s ({:.1}/s), {} held the glass's blur, {} cursor-only; commit p99 {} us to the thread, {} us in the kernel, {} us back",
            s.output.name(),
            119.0 / secs,
            s.held,
            s.cursor_frames,
            t.to_thread.as_micros(),
            t.ioctl.as_micros(),
            t.to_main.as_micros()
        );
        s.commit_times.clear();
        s.frames = 0;
        s.held = 0;
        s.cursor_frames = 0;
        s.counted_from = None;
    }
}

fn count_damage(
    s: &mut Surface,
    damage: &DamageSnapshot<i32, smithay::utils::Buffer>,
    output: &Output,
) {
    let commit = damage.current_commit();
    if s.damage_seen.replace(commit) == Some(commit) {
        s.stats.damage(0, 0);
        return;
    }
    match damage.raw().next() {
        Some(rects) => {
            let (mut n, mut area) = (0, 0u64);
            for r in rects {
                n += 1;
                area += (r.size.w.max(0) as u64) * (r.size.h.max(0) as u64);
            }
            s.stats.damage(n, area);
        }
        None => {
            let full = output
                .current_mode()
                .map_or(0, |m| (m.size.w.max(0) as u64) * (m.size.h.max(0) as u64));
            s.stats.damage(1, full);
        }
    }
}

pub fn take_stats(state: &mut State, output: &Output) -> Option<crate::debugview::FrameStats> {
    let crtc = crtc_of(output)?;
    let s = surface_mut(state, crtc)?;
    Some(s.stats.take())
}

pub(super) fn callbacks_after_a_frame(state: &mut State, crtc: crtc::Handle, shown: bool) {
    let Some(s) = state
        .backend
        .as_mut()
        .and_then(|b| b.surfaces.get_mut(&crtc))
    else {
        return;
    };
    let interval = crate::framepace::interval(s.output.current_mode().map_or(0, |m| m.refresh));
    let wait = match s.pace.done(shown, std::time::Instant::now(), interval) {
        Done::Now if shown => {
            if let Some(t) = s.callback_timer.take() {
                state.handle.remove(t);
            }
            callbacks_done(state, crtc);
            return;
        }
        Done::Now => Duration::ZERO,
        Done::After(wait) => wait,
    };
    if s.callback_timer.is_some() {
        return;
    }
    let token = state
        .handle
        .insert_source(Timer::from_duration(wait), move |_, _, state| {
            let Some(b) = state.backend.as_mut() else {
                return TimeoutAction::Drop;
            };
            let active = b.session.is_active();
            let held = b.held(crtc, std::time::Instant::now());
            let Some(s) = b.surfaces.get_mut(&crtc) else {
                return TimeoutAction::Drop;
            };
            s.callback_timer = None;
            if waits_for_frame(s.needs_redraw || s.idle_scheduled, s.hw.off, held, active) {
                return TimeoutAction::Drop;
            }
            callbacks_done(state, crtc);
            TimeoutAction::Drop
        });
    match token {
        Ok(t) => s.callback_timer = Some(t),
        Err(e) => log::warn!("frame callback timer: {}", e.error),
    }
}

/// A commit since asked for a frame: its callback waits for that frame, or the client would
/// draw again before the screen showed its last. An output off, held or away from its session
/// shows no frame, so its callbacks go out on the timer.
pub(super) fn waits_for_frame(asked: bool, off: bool, held: bool, active: bool) -> bool {
    asked && !off && !held && active
}

pub fn answer_frames(state: &mut State) {
    let Some(b) = state.backend.as_ref() else {
        return;
    };
    let idle: Vec<crtc::Handle> = b
        .surfaces
        .iter()
        .filter(|(_, s)| !s.needs_redraw && !s.idle_scheduled && !s.hw.off)
        .map(|(c, _)| *c)
        .collect();
    for crtc in idle {
        callbacks_after_a_frame(state, crtc, false);
    }
}

fn callbacks_done(state: &mut State, crtc: crtc::Handle) {
    let Some(s) = surface_mut(state, crtc) else {
        return;
    };
    s.pace.last_done = Some(std::time::Instant::now());
    let output = s.output.clone();
    send_frame_callbacks(state, &output);
}

fn send_frame_callbacks(state: &mut State, output: &Output) {
    let now = state.now();
    let pass = state.scene.surface_pass(output);
    (state.scene).send_frames(&pass, now, Some(Duration::from_secs(1)));
    if let smithay::input::pointer::CursorImageStatus::Surface(s) = &state.cursor {
        smithay::desktop::utils::send_frames_surface_tree(
            s,
            output,
            now,
            Some(Duration::ZERO),
            |_, _| Some(output.clone()),
        );
    }
}
