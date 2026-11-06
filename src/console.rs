//! Lock mode: while the remote desktop server's sessions hold lock-screen objects, the desktop
//! works on Remote outputs made for the client and the computer's own outputs keep the lock.

use smithay::output::Output;
use smithay::reexports::wayland_server::{Resource, backend::ObjectId};
use smithay::utils::Transform;

use crate::compositor::State;
use crate::inputproto::proto::reprise_lock_screen_v1::{Reason, RepriseLockScreenV1};
use crate::output::quarter_scale;
use crate::virtualout::{Kind, Spec};

/// mHz; a Remote output's frames follow its captures, not this.
const REFRESH: i32 = 60_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Owner {
    #[default]
    Console,
    Remote,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Console,
    Remote,
}

/// One monitor of the remote client, in its physical pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoteMonitor {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    /// Percent.
    pub scale: u32,
}

#[derive(Default)]
pub struct Console {
    pub owner: Owner,
    objects: Vec<ObjectId>,
    /// The object whose layouts apply, kept after it goes until `objects` empties.
    leader: Option<ObjectId>,
    layout: Vec<RemoteMonitor>,
}

/// The most monitors a layout may have.
const MAX_MONITORS: usize = 16;

/// The pixels a monitor may have on a side (MS-RDPEDISP).
const SIDES: std::ops::RangeInclusive<i32> = 200..=8192;

/// The farthest a monitor may start from the origin on either axis. With [`SIDES`] it keeps
/// every sum of positions and lengths far inside i32.
const MAX_COORD: i32 = 8192 * 16;

/// `set_layout`'s array: x, y, width, height (i32) and scale percent (u32) per monitor, in native
/// byte order. None for no monitor, too many, a partial entry, or a side or start out of bounds.
pub fn parse_layout(bytes: &[u8]) -> Option<Vec<RemoteMonitor>> {
    let (entries, rest) = bytes.as_chunks::<20>();
    if !rest.is_empty() || !(1..=MAX_MONITORS).contains(&entries.len()) {
        return None;
    }
    let word = |e: &[u8; 20], i: usize| [e[i * 4], e[i * 4 + 1], e[i * 4 + 2], e[i * 4 + 3]];
    let monitors: Vec<RemoteMonitor> = (entries.iter())
        .map(|e| RemoteMonitor {
            x: i32::from_ne_bytes(word(e, 0)),
            y: i32::from_ne_bytes(word(e, 1)),
            width: i32::from_ne_bytes(word(e, 2)),
            height: i32::from_ne_bytes(word(e, 3)),
            scale: u32::from_ne_bytes(word(e, 4)),
        })
        .collect();
    let fits = |m: &RemoteMonitor| {
        SIDES.contains(&m.width)
            && SIDES.contains(&m.height)
            && (-MAX_COORD..=MAX_COORD).contains(&m.x)
            && (-MAX_COORD..=MAX_COORD).contains(&m.y)
    };
    monitors.iter().all(fits).then_some(monitors)
}

/// Whether `object` has begun and still holds lock mode.
pub fn holds(state: &State, object: &ObjectId) -> bool {
    state.console.objects.contains(object)
}

/// Why an object may not begin: `[power] locker` is set, or a locker client holds the lock.
pub fn refusal(state: &State) -> Option<Reason> {
    if !state.config.power.locker.trim().is_empty() {
        Some(Reason::Unavailable)
    } else if crate::sessionlock::holds_input(state) {
        Some(Reason::Replaced)
    } else {
        None
    }
}

/// `object`'s first layout. The first object enters lock mode with it and leads it; a later one
/// hears the screens there are.
pub fn begin(state: &mut State, object: ObjectId, layout: Vec<RemoteMonitor>) {
    let first = state.console.objects.is_empty();
    state.console.objects.push(object.clone());
    if !first {
        send_screens(state, &[object]);
        return;
    }
    state.console.leader = Some(object);
    log::info!("lock mode: begins with {} remote screens", layout.len());
    apply(state, layout);
    crate::lockview::lock(state);
    on_begin(state);
    send_screens(state, &state.console.objects);
}

/// A later layout of `object`; only the leader's applies.
pub fn relayout(state: &mut State, object: ObjectId, layout: Vec<RemoteMonitor>) {
    if state.console.leader.as_ref() != Some(&object) || state.console.layout == layout {
        return;
    }
    log::info!("lock mode: {} remote screens", layout.len());
    apply(state, layout);
    send_screens(state, &state.console.objects);
}

/// `object` went. Lock mode ends with the last one; the lock stays.
pub fn end(state: &mut State, object: ObjectId) {
    let objects = &mut state.console.objects;
    let Some(i) = objects.iter().position(|o| *o == object) else {
        return;
    };
    objects.remove(i);
    if objects.is_empty() {
        leave(state);
    }
}

/// End lock mode for every object with `ended(reason)`; the lock stays.
pub fn stop(state: &mut State, reason: Reason) {
    if state.console.objects.is_empty() {
        return;
    }
    for id in std::mem::take(&mut state.console.objects) {
        if let Ok(r) = RepriseLockScreenV1::from_id(&state.dh, id) {
            crate::inputproto::send_ended(&r, reason);
        }
    }
    leave(state);
}

fn leave(state: &mut State) {
    log::info!("lock mode: ends");
    state.console.leader = None;
    on_end(state);
    apply(state, Vec::new());
}

/// The Remote outputs are laid out and the console is locked.
fn on_begin(state: &mut State) {
    state.console.owner = Owner::Remote;
}

/// Lock mode ends; the Remote outputs go after this.
fn on_end(state: &mut State) {
    state.console.owner = Owner::Console;
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the lock, input and window moves follow the side")
)]
pub fn remote_side(state: &State) -> bool {
    state.console.owner == Owner::Remote
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the lock, input and window moves follow the side")
)]
pub fn side_of(output: &Output) -> Side {
    if crate::virtualout::is_remote(output) {
        Side::Remote
    } else {
        Side::Console
    }
}

/// Make the Remote outputs `layout`'s, in order: resize those there are, make or remove the
/// rest, re-decide the standby, then lay out once.
fn apply(state: &mut State, layout: Vec<RemoteMonitor>) {
    let specs = specs(&layout);
    let remotes = crate::virtualout::remote_outputs(state);
    for (o, s) in remotes.iter().zip(&specs) {
        crate::virtualout::resize(state, o, s.size, s.scale);
        crate::output::set_placement(o, Some(s.position.into()));
    }
    for s in specs.iter().skip(remotes.len()) {
        let name = crate::virtualout::next_name(state, Kind::Remote);
        crate::virtualout::create(state, &name, *s, Kind::Remote);
    }
    let gone = remotes.get(specs.len()..).unwrap_or_default();
    for o in gone {
        crate::virtualout::remove(state, o);
        state.forget_output(o);
    }
    let standby_gone = crate::standby::remotes_changed(state);
    state.update_layout();
    if !gone.is_empty() || standby_gone {
        crate::notifyview::output_removed(state);
    }
    state.console.layout = layout;
}

fn send_screens(state: &State, objects: &[ObjectId]) {
    let remotes = crate::virtualout::remote_outputs(state);
    for id in objects {
        let Ok(r) = RepriseLockScreenV1::from_id(&state.dh, id.clone()) else {
            continue;
        };
        for o in &remotes {
            r.screen(o.name());
        }
        r.done();
    }
}

/// Each monitor as a Remote output: its physical size at its scale snapped to a quarter, placed
/// in layout units.
fn specs(layout: &[RemoteMonitor]) -> Vec<Spec> {
    let scales: Vec<f64> = (layout.iter())
        .map(|m| f64::from(quarter_scale(m.scale as f32 / 100.0)))
        .collect();
    let logical = |len: i32, i: usize| (f64::from(len) / scales[i]).round() as i32;
    let widths: Vec<i32> = (layout.iter().enumerate())
        .map(|(i, m)| logical(m.width, i))
        .collect();
    let heights: Vec<i32> = (layout.iter().enumerate())
        .map(|(i, m)| logical(m.height, i))
        .collect();
    let xs = axis(layout, |m| (m.x, m.width, m.y, m.height), &widths, &scales);
    let ys = axis(layout, |m| (m.y, m.height, m.x, m.width), &heights, &scales);
    (layout.iter().enumerate())
        .map(|(i, m)| Spec {
            size: (m.width, m.height),
            refresh: REFRESH,
            scale: scales[i],
            transform: Transform::Normal,
            position: (xs[i], ys[i]),
        })
        .collect()
}

/// One axis of [`specs`]; `along` gives a start and length on this axis, then across, in pixels.
/// A monitor that meets another edge to edge still meets it; any other sits at start / scale.
fn axis(
    layout: &[RemoteMonitor],
    along: impl Fn(&RemoteMonitor) -> (i32, i32, i32, i32),
    logical: &[i32],
    scales: &[f64],
) -> Vec<i32> {
    let mut order: Vec<usize> = (0..layout.len()).collect();
    order.sort_by_key(|&i| along(&layout[i]).0);
    let mut at: Vec<Option<i32>> = vec![None; layout.len()];
    for &i in &order {
        let (start, _, across, across_len) = wide(along(&layout[i]));
        let meets = (0..layout.len()).find_map(|j| {
            let placed = at[j]?;
            let (s, len, a, a_len) = wide(along(&layout[j]));
            (s + len == start && a < across + across_len && across < a + a_len)
                .then_some(placed.saturating_add(logical[j]))
        });
        at[i] = Some(meets.unwrap_or_else(|| (start as f64 / scales[i]).round() as i32));
    }
    at.into_iter().map(|a| a.unwrap_or(0)).collect()
}

fn wide((a, b, c, d): (i32, i32, i32, i32)) -> (i64, i64, i64, i64) {
    (a.into(), b.into(), c.into(), d.into())
}

#[cfg(test)]
mod tests;
