//! Lock mode: while the remote desktop server's sessions hold lock-screen objects, the desktop
//! works on Remote outputs made for the client and the computer's own outputs keep the lock.

use smithay::output::Output;
use smithay::reexports::wayland_server::{Resource, backend::ObjectId};
use smithay::utils::{Logical, Point, Transform};

use reprisekit::powerpanel::Leave;

use crate::compositor::State;
use crate::fsjobs::OnLoop;
use crate::inputproto::InputSource;
use crate::inputproto::proto::reprise_lock_screen_v1::{Reason, RepriseLockScreenV1};
use crate::output::quarter_scale;
use crate::raster::Rect;
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
    layout: Vec<RemoteMonitor>,
    /// Remote outputs this layout removes, at their last boxes, for `homes::refit_remote`.
    pub(crate) gone: Vec<(Output, Rect)>,
    pub(crate) keys: keys::Keys,
    /// The computer's own pointer in lock mode (`pointer`).
    pub(crate) pointer: Point<f64, Logical>,
    /// Windows lock mode brought back, waiting for an output (`homes::land_held`).
    pub(crate) held: Vec<homes::Held>,
    /// The client screen, by layout index, each window lock mode brought back was on.
    pub(crate) left: Vec<(crate::window::WindowId, usize)>,
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

/// Why an object may not begin: `[power] locker` is set, or a live locker client holds the lock.
pub fn refusal(state: &mut State) -> Option<Reason> {
    crate::sessionlock::check_alive(state);
    if !state.config.power.locker.trim().is_empty() {
        Some(Reason::Unavailable)
    } else if crate::sessionlock::holds_input(state) {
        Some(Reason::Replaced)
    } else {
        None
    }
}

/// `object`'s first layout. The first object enters lock mode with it; a later one applies it
/// as [`relayout`] does, or hears the screens there are when it is the same.
pub fn begin(state: &mut State, object: ObjectId, layout: Vec<RemoteMonitor>) {
    let first = state.console.objects.is_empty();
    state.console.objects.push(object.clone());
    if !first {
        if state.console.layout != layout {
            return relayout(state, layout);
        }
        send_screens(state, &[object]);
        return;
    }
    log::info!("lock mode: begins with {} remote screens", layout.len());
    state.console.owner = Owner::Remote;
    apply(state, layout);
    crate::lockview::lock(state);
    keys::begin(state);
    pointer::begin(state);
    crate::idle::lock_mode_began(state);
    send_screens(state, &state.console.objects);
}

/// A later layout of an object holding lock mode. Every object relays the one server's layout,
/// so whichever sends a change first applies it and the others' copies are no-ops.
pub fn relayout(state: &mut State, layout: Vec<RemoteMonitor>) {
    if state.console.layout == layout {
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

/// End lock mode for every object with `ended(reason)`, every sender and keys object cut off
/// first; the lock stays.
pub fn stop(state: &mut State, reason: Reason) {
    if state.console.objects.is_empty() {
        return;
    }
    state.eis_revoke_senders();
    state.inputproto.keys.revoke_all();
    for id in std::mem::take(&mut state.console.objects) {
        if let Ok(r) = RepriseLockScreenV1::from_id(&state.dh, id) {
            crate::inputproto::send_ended(&r, reason);
        }
    }
    leave(state);
}

/// The console's lock unlocked in lock mode: every session loses its input, objects left or not,
/// and lock mode ends with `ended(console_unlocked)`; the caller hands the keyboard back.
pub fn console_unlocked(state: &mut State) {
    log::info!("lock mode: the console unlocked");
    if state.console.objects.is_empty() {
        state.eis_revoke_senders();
        state.inputproto.keys.revoke_all();
    }
    stop(state, Reason::ConsoleUnlocked);
    if remote_side(state) {
        leave(state);
    }
}

fn leave(state: &mut State) {
    log::info!("lock mode: ends");
    // Before the seal, so the window holding the remote's keys hears them released.
    keys::end(state);
    crate::lockview::drop_remote(state);
    // What the remote side opened, dragged or focused would stay up under the cover, or carry
    // over to the console after an unlock.
    crate::lockview::seal(state, |_| {});
    crate::idle::lock_mode_ends(state);
    on_end(state);
    apply(state, Vec::new());
    pointer::end(state);
}

/// Lock mode ends; the Remote outputs go after this.
fn on_end(state: &mut State) {
    homes::bring_back(state);
    state.console.owner = Owner::Console;
}

pub fn remote_side(state: &State) -> bool {
    state.console.owner == Owner::Remote
}

/// How the remote desktop server lets every client go: `Disconnect` leaves the session running,
/// `LoggedOff` tells them the session logged off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClientsEnd {
    Disconnect,
    LoggedOff,
}

/// Whether Disconnect takes Switch User's and Stand By's places: on the remote side, or while a
/// remote client drives a session that is off the seat.
pub fn disconnect_offered(state: &State) -> bool {
    remote_side(state) || (state.lock.seat_away && state.remote_controlled())
}

/// What the Log Off panel and the Shut down options offer besides Log Off.
pub fn leave_offered(state: &State) -> Leave {
    if disconnect_offered(state) {
        Leave::Disconnect
    } else {
        Leave::SwitchUser {
            live: crate::loginlink::desk_offers(state),
        }
    }
}

/// Lock the console, then ask the server to end every client; the session keeps running.
pub fn disconnect(state: &mut State) {
    log::info!("remote desktop: disconnect");
    crate::lockview::lock(state);
    end_clients(state, ClientsEnd::Disconnect);
}

pub(crate) fn end_clients(state: &mut State, why: ClientsEnd) {
    end_clients_then(state, why, Box::new(|_| {}));
}

/// [`end_clients`], running `then` on the loop once the server answered or the call failed.
pub(crate) fn end_clients_then(state: &mut State, why: ClientsEnd, then: OnLoop) {
    #[cfg(test)]
    state.ended_clients.push(why);
    state.spawn_job(move || {
        let told = match why {
            ClientsEnd::Disconnect => reprisebus::remote::disconnect_all(),
            ClientsEnd::LoggedOff => reprisebus::remote::session_ending(),
        };
        Box::new(move |state: &mut State| {
            if let Err(e) = told {
                log::info!("remote desktop: ending the clients ({why:?}): {e}");
            }
            then(state);
        })
    });
}

/// Whether the input being handled is the computer's own and lock mode keeps it for the console:
/// its keys go to `keys`, its pointer to `pointer`, and nothing of it reaches the seat.
pub fn physical(state: &State) -> bool {
    remote_side(state) && state.inputproto.source == InputSource::Physical
}

/// Whether `output` is the desktop's: any outside lock mode, a Remote output in it.
pub fn on_desk(state: &State, output: &Output) -> bool {
    !remote_side(state) || side_of(output) == Side::Remote
}

pub fn side_of(output: &Output) -> Side {
    if crate::virtualout::is_remote(output) {
        Side::Remote
    } else {
        Side::Console
    }
}

/// Make the Remote outputs `layout`'s, in order: resize those there are, make or remove the
/// rest and lay out, then re-decide the standby, whose windows that first layout carried.
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
    if remote_side(state) {
        state.console.gone = (gone.iter())
            .map(|o| (o.clone(), state.layout_box(Some(o))))
            .collect();
    }
    for o in gone {
        crate::virtualout::remove(state, o);
        state.forget_output(o);
    }
    state.update_layout();
    let standby = crate::standby::remotes_changed(state);
    if standby != crate::standby::Change::Keep {
        state.update_layout();
    }
    if !gone.is_empty() || standby == crate::standby::Change::Remove {
        crate::notifyview::output_removed(state);
    }
    state.console.layout = layout;
    state.eis_readvertise();
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

pub mod homes;
pub mod keys;
pub mod pointer;

#[cfg(test)]
mod inputtests;
#[cfg(test)]
mod locktests;
#[cfg(test)]
pub(crate) mod tests;
#[cfg(test)]
mod unlocktests;
