//! The standby screen: a virtual output holding the desktop while Remote Desktop is on and no
//! monitor is in the layout (docs/features.md, Remote Desktop).

use futures_util::{Stream, StreamExt};
use smithay::{output::Output, reexports::calloop::channel, utils::Transform};

use crate::{
    compositor::State,
    config::OutputConfig,
    virtualout::{Kind, Spec},
};

#[derive(Default)]
pub struct Standby {
    rdp_on: bool,
    /// The last monitor to leave the layout, as it ran.
    last: Option<Spec>,
    output: Option<Output>,
}

/// Whether the standby exists: Remote Desktop is on and no monitor is in the layout.
pub fn wanted(rdp_on: bool, monitors: usize) -> bool {
    rdp_on && monitors == 0
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Add,
    Remove,
    Keep,
}

pub fn change(exists: bool, wanted: bool) -> Change {
    match (exists, wanted) {
        (false, true) => Change::Add,
        (true, false) => Change::Remove,
        _ => Change::Keep,
    }
}

const FALLBACK: Spec = Spec {
    size: (1920, 1080),
    refresh: 60_000,
    scale: 1.0,
    transform: Transform::Normal,
    position: (0, 0),
};

/// How the standby runs: as the `last` monitor did, else as `[[outputs]]` sets the primary (or
/// the first entry), else [`FALLBACK`].
pub fn spec(last: Option<Spec>, outputs: &[OutputConfig]) -> Spec {
    if let Some(s) = last {
        return s;
    }
    let Some(c) = outputs.iter().find(|o| o.primary).or(outputs.first()) else {
        return FALLBACK;
    };
    Spec {
        size: c.mode.map_or(FALLBACK.size, |m| (m.width, m.height)),
        refresh: (c.mode.and_then(|m| m.refresh_mhz)).unwrap_or(FALLBACK.refresh),
        scale: f64::from(repriseconf::config::quarter_scale(c.scale)),
        transform: crate::output::transform(c.transform),
        position: c.position.unwrap_or(FALLBACK.position),
    }
}

/// What `output` runs, as a spec for the standby.
fn spec_of(output: &Output) -> Option<Spec> {
    let mode = output.current_mode()?;
    let at = output.current_location();
    Some(Spec {
        size: (mode.size.w, mode.size.h),
        refresh: mode.refresh,
        scale: output.current_scale().fractional_scale(),
        transform: output.current_transform(),
        position: (at.x, at.y),
    })
}

/// [`wanted`] for the layout now; never while remote screens hold the desktop.
fn wanted_now(state: &State) -> bool {
    wanted(state.standby.rdp_on, monitors(state))
        && crate::virtualout::remote_outputs(state).is_empty()
}

fn monitors(state: &State) -> usize {
    (state.scene.outputs().iter())
        .filter(|o| !crate::backend::is_virtual(o))
        .count()
}

fn add(state: &mut State) -> Output {
    let spec = spec(state.standby.last, &state.config.outputs);
    let name = crate::virtualout::next_name(state, Kind::Standby);
    log::info!("standby: Remote Desktop is on and no monitor is in the layout; {name} stands in");
    let output = crate::virtualout::create(state, &name, spec, Kind::Standby);
    state.standby.output = Some(output.clone());
    output
}

/// Remote Desktop came on or went off: the standby exists while it is on, no monitor is in the
/// layout and no Remote output holds the desktop.
pub fn rdp_changed(state: &mut State, on: bool) {
    if state.standby.rdp_on == on {
        return;
    }
    log::info!("standby: Remote Desktop {}", if on { "on" } else { "off" });
    state.standby.rdp_on = on;
    let exists = state.standby.output.is_some();
    match change(exists, wanted_now(state)) {
        Change::Add => {
            let output = add(state);
            state.output_added(&output);
        }
        Change::Remove => {
            if let Some(output) = state.standby.output.take() {
                crate::virtualout::remove(state, &output);
                state.output_removed(&output);
            }
        }
        Change::Keep => {}
    }
}

/// Remote outputs came or went: the standby goes while they hold the desktop and comes back when
/// the last goes with Remote Desktop on and no monitor. The caller lays out after either.
pub fn remotes_changed(state: &mut State) -> Change {
    let exists = state.standby.output.is_some();
    let change = change(exists, wanted_now(state));
    match change {
        Change::Add => {
            add(state);
        }
        Change::Remove => {
            if let Some(output) = state.standby.output.take() {
                crate::virtualout::remove(state, &output);
                state.forget_output(&output);
            }
        }
        Change::Keep => {}
    }
    change
}

/// `output` came into the layout: a monitor takes over the standby's box, keeping its windows in
/// place, and the standby it replaced is returned for the caller's one layout pass.
pub fn monitor_added(state: &mut State, output: &Output) -> Option<Output> {
    if crate::backend::is_virtual(output) {
        return None;
    }
    let standby = state.standby.output.take()?;
    log::info!(
        "standby: {} takes over from {}",
        output.name(),
        standby.name()
    );
    crate::output::inherit_box(output, &standby);
    crate::virtualout::remove(state, &standby);
    state.forget_output(&standby);
    Some(standby)
}

/// `output` left: if it was the last monitor, Remote Desktop is on and no Remote output holds the
/// desktop, the standby takes its mode, scale and place, the windows staying; the caller lays out.
pub fn monitor_removed(state: &mut State, output: &Output) {
    if crate::backend::is_virtual(output) {
        return;
    }
    if let Some(s) = spec_of(output) {
        state.standby.last = Some(s);
    }
    let exists = state.standby.output.is_some();
    if change(exists, wanted_now(state)) == Change::Add {
        let standby = add(state);
        crate::output::inherit_box(&standby, output);
    }
}

/// Follow whether Remote Desktop is on: the server owns its name on the session bus from its
/// start until it exits.
pub fn setup(state: &mut State) {
    let (tx, rx) = channel::channel::<bool>();
    if let Err(e) = state.handle.insert_source(rx, |event, _, state| {
        if let channel::Event::Msg(on) = event {
            rdp_changed(state, on);
        }
    }) {
        log::warn!("standby: no report channel ({})", e.error);
        return;
    }
    let spawned = std::thread::Builder::new()
        .name("standby-dbus".into())
        .spawn(move || zbus::block_on(follow(tx)));
    if let Err(e) = spawned {
        log::warn!("standby: no bus thread ({e}); no standby screen for Remote Desktop");
    }
}

async fn follow(tx: channel::Sender<bool>) {
    let name = reprisebus::remote::NAME;
    let conn = match zbus::Connection::session().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("standby: no session bus ({e}); no standby screen for Remote Desktop");
            return;
        }
    };
    let changes = reprisebus::signals(&conn, name, Vec::new()).await;
    let conn = &conn;
    let owned = move || async move { reprisebus::name_owner(conn, name).await.is_some() };
    if report_owner(changes, owned, |on| tx.send(on).is_ok()).await {
        log::warn!(
            "standby: the session bus stopped reporting {name}'s owner; Remote Desktop counts as off"
        );
    }
}

/// Report whether the name is `owned`, at the start and after each of `changes`, until `report`
/// fails. When `changes` ends, an on is followed by a last off, and the result is true.
async fn report_owner<F: Future<Output = bool>>(
    mut changes: impl Stream + Unpin,
    mut owned: impl FnMut() -> F,
    mut report: impl FnMut(bool) -> bool,
) -> bool {
    let on = loop {
        let on = owned().await;
        if !report(on) {
            return false;
        }
        if changes.next().await.is_none() {
            break on;
        }
    };
    if on {
        report(false);
    }
    true
}

#[cfg(test)]
mod tests;
