//! Painting Droplet opening layers on a worker thread ahead of use.

use reprisekit::controls::notifysheet::{
    SheetIcons, SheetOpening, WarmJob, WarmLayers, build_warm,
};
use reprisekit::style::SheetShape;
use reprisekit::taskbar::Edge;
use smithay::reexports::calloop::channel;
use std::time::Instant;

use crate::compositor::State;
use crate::text::TextRenderer;

use super::layout::{Drawn, droplet_ends};
use super::paint::{relative, sheet_icons};
use super::{Balloon, NotifyView, droplets};

/// Once the loop is idle, sends each Droplet at rest to have its layers for its next move warmed
/// off the loop's thread: under the lit rim while collapsed, the plain one while open.
pub(super) fn warm_openings(state: &mut State) {
    if !droplets(state) || state.notify.warm_queued {
        return;
    }
    state.notify.warm_queued = true;
    state.handle.insert_idle(|state| {
        state.notify.warm_queued = false;
        let now = Instant::now();
        let Some(edge) = state.notify.edge else {
            return;
        };
        let shape = state.config.glass.notification.shape();
        for i in 0..state.notify.stack.len() {
            let mut opening = std::mem::take(&mut state.notify.stack[i].opening);
            warm_one(state, i, &mut opening, edge, shape, now);
            state.notify.stack[i].opening = opening;
        }
    });
}

fn warm_one(
    state: &mut State,
    i: usize,
    opening: &mut SheetOpening,
    edge: Edge,
    shape: SheetShape,
    now: Instant,
) {
    let b = &state.notify.stack[i];
    // A reply field's caret blinks, which would rebuild the layers on every blink.
    if !b.motion.settled(now) || b.reply.is_some() || !matches!(b.drawn, Some(Drawn::Sheet(_))) {
        return;
    }
    let theme = state.config.theme.scaled(b.scale);
    let when = relative(b.shown_at.elapsed());
    let Some([shut, open]) = droplet_ends(
        &theme,
        state.text.for_output(theme.scale),
        &b.item,
        false,
        edge,
        shape,
        &when,
    ) else {
        return;
    };
    let lit = b.open_drawn < 1.0;
    let (app, picture) = sheet_icons(&mut state.icons, b, &open);
    let icons = SheetIcons {
        app: app.as_ref(),
        picture: picture.as_ref(),
    };
    let tint = b.tint.flatten();
    let Some(job) = opening.warm_job(&b.glass, &theme, &shut, &open, icons, tint, lit) else {
        return;
    };
    let Some(tx) = warmer(state) else {
        return;
    };
    let b = &mut state.notify.stack[i];
    b.warm_gen += 1;
    if tx.send((b.serial, b.warm_gen, job)).is_err() {
        opening.forget_job();
        lose_warmer(&mut state.notify);
    }
}

/// The painter thread ended: the jobs it held are asked again of the next one.
fn lose_warmer(v: &mut NotifyView) {
    v.warmer = None;
    for b in &mut v.stack {
        b.opening.forget_job();
    }
}

/// The sender to the thread that paints opening layers, started on first use; its results land
/// through [`land_warm`].
fn warmer(state: &mut State) -> Option<std::sync::mpsc::Sender<(u64, u64, WarmJob)>> {
    if let Some(tx) = &state.notify.warmer {
        return Some(tx.clone());
    }
    if state.notify.warm_spawns >= WARM_SPAWNS {
        return None;
    }
    if let Some(gone) = state.notify.warm_source.take() {
        state.handle.remove(gone);
    }
    let (done, landed) = channel::channel::<(u64, u64, Option<WarmLayers>)>();
    let inserted = state
        .handle
        .insert_source(landed, |event, _, state| match event {
            channel::Event::Msg((serial, generation, built)) => {
                land_warm(&mut state.notify.stack, serial, generation, built);
            }
            channel::Event::Closed => lose_warmer(&mut state.notify),
        });
    match inserted {
        Ok(token) => state.notify.warm_source = Some(token),
        Err(e) => {
            log::warn!("notify: no warm channel: {}", e.error);
            return None;
        }
    }
    let (tx, jobs) = std::sync::mpsc::channel::<(u64, u64, WarmJob)>();
    let spawned = std::thread::Builder::new()
        .name("notify-warm".into())
        .spawn(move || {
            paint_warm_jobs(jobs, build_warm, |serial, generation, built| {
                done.send((serial, generation, built)).is_ok()
            });
        });
    if let Err(e) = spawned {
        log::warn!("notify: no warm thread: {e}");
        return None;
    }
    state.notify.warm_spawns += 1;
    if state.notify.warm_spawns == WARM_SPAWNS {
        log::warn!("notify: the warm thread was started {WARM_SPAWNS} times; no more after this");
    }
    state.notify.warmer = Some(tx.clone());
    Some(tx)
}

const WARM_SPAWNS: u32 = 3;

/// The painter thread's loop: paints each job with `build` and hands `done` its layers, `None`
/// for a job that panicked, until `done` returns false.
pub(super) fn paint_warm_jobs(
    jobs: std::sync::mpsc::Receiver<(u64, u64, WarmJob)>,
    build: impl Fn(WarmJob, &TextRenderer) -> WarmLayers,
    done: impl Fn(u64, u64, Option<WarmLayers>) -> bool,
) {
    let mut text: Option<(reprisekit::theme::FontConfig, TextRenderer)> = None;
    for (serial, generation, job) in jobs {
        let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if text.as_ref().is_none_or(|(fonts, _)| fonts != job.fonts()) {
                text = Some((job.fonts().clone(), TextRenderer::load(job.fonts())));
            }
            let Some((_, text)) = &text else {
                unreachable!("loaded above");
            };
            build(job, text)
        }));
        if built.is_err() {
            log::error!("notify: painting balloon {serial}'s opening panicked; it paints in frame");
            text = None;
        }
        if !done(serial, generation, built.ok()) {
            return;
        }
    }
}

/// Gives balloon `serial` the layers its warm job `generation` painted, unless it left or sent a
/// later job since; true when taken. `None`, a job that panicked, is not sent again.
pub(super) fn land_warm(
    stack: &mut [Balloon],
    serial: u64,
    generation: u64,
    built: Option<WarmLayers>,
) -> bool {
    match stack.iter_mut().find(|b| b.serial == serial) {
        Some(b) if b.warm_gen == generation => match built {
            Some(built) => {
                b.opening.install(&mut b.glass, built);
                true
            }
            None => {
                b.opening.job_failed();
                false
            }
        },
        _ => false,
    }
}
