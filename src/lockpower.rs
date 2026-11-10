//! The lock's power menu: Stand By, plus Restart and Turn Off while the session is the
//! computer's only one and no one logs in by themselves after a restart.

use std::path::Path;

use reprisekit::controls::endsession::Goal;
use reprisekit::logon;

use crate::compositor::State;
use crate::console::Side;
use crate::lockview::{self, Origin};
use crate::menuview::MenuKind;
use crate::startmenu::MenuItem;

/// A row of the lock's power menu. Each runs its `[power]` command through `sessionend`, so
/// logind's block inhibitors hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockPower {
    StandBy,
    Restart,
    TurnOff,
}

/// Only the console's lock offers power, and not in lock mode nor while a remote viewer drives
/// the seat: whoever is at the computer reaches it, nobody from afar.
pub fn offered(state: &State, side: Side) -> bool {
    side == Side::Console && !crate::console::remote_side(state) && !state.remote_controlled()
}

/// Open the menu over the screen once `Helpers::alone` has answered off the loop.
pub fn open(state: &mut State, side: Side) {
    if !offered(state, side) || menu_open(state) {
        return;
    }
    let alone = state.lock.helpers.alone;
    state.spawn_job(move || {
        let alone = alone();
        Box::new(move |state: &mut State| show(state, alone))
    });
}

fn show(state: &mut State, alone: bool) {
    if !offered(state, Side::Console) || !lockview::screen_shows(state, Side::Console) {
        return;
    }
    if menu_open(state) {
        return;
    }
    let rows = rows(!state.config.power.standby.trim().is_empty(), alone);
    if rows.is_empty() {
        return lockview::refused(state, logon::text::unlock_to_power());
    }
    let items = (rows.iter()).map(|r| MenuItem::item(&label(*r))).collect();
    let host = lockview::host(Side::Console);
    crate::logonview::open_power_menu(state, host, items, MenuKind::LockPower(rows));
}

/// Stand By when `[power] standby` is set; Restart and Turn Off only when `alone`.
fn rows(standby: bool, alone: bool) -> Vec<LockPower> {
    let ends = [LockPower::Restart, LockPower::TurnOff];
    (standby.then_some(LockPower::StandBy).into_iter())
        .chain(ends.into_iter().filter(|_| alone))
        .collect()
}

fn label(p: LockPower) -> String {
    match p {
        LockPower::StandBy => logon::text::stand_by(),
        LockPower::Restart => logon::text::restart(),
        LockPower::TurnOff => logon::text::turn_off_row(),
    }
}

/// A picked row; Restart and Turn Off ask `Helpers::alone` again first.
pub fn pick(state: &mut State, p: LockPower) {
    if !offered(state, Side::Console) {
        return;
    }
    if p == LockPower::StandBy {
        let cmd = state.config.power.standby.clone();
        return crate::sessionend::sleep(state, crate::sessionend::Act::StandBy, cmd);
    }
    let alone = state.lock.helpers.alone;
    state.spawn_job(move || {
        let alone = alone();
        Box::new(move |state: &mut State| end(state, p, alone))
    });
}

fn end(state: &mut State, p: LockPower, alone: bool) {
    if !offered(state, Side::Console) || !state.console_locked() {
        return;
    }
    if !alone {
        return lockview::refused(state, logon::text::unlock_to_power());
    }
    let cfg = &state.config.power;
    let (goal, cmd) = match p {
        LockPower::Restart => (Goal::Restart, cfg.reboot.clone()),
        _ => (Goal::TurnOff, cfg.poweroff.clone()),
    };
    crate::sessionend::begin(state, goal, Some(cmd), Origin::Console);
}

/// The lock's power menu is open; no other menu opens while locked.
pub fn menu_open(state: &State) -> bool {
    (state.menus.iter()).any(|m| matches!(m.kind, MenuKind::LockPower(_)))
}

pub fn close(state: &mut State) {
    if menu_open(state) {
        state.close_menu();
    }
}

/// Whether logind lists no session but this one that holds someone's work, and login.toml logs
/// no one in by themselves. Blocks on the system bus; any doubt reads as not alone.
pub fn alone() -> bool {
    if autologin(Path::new(repriselogin::machine::MACHINE_CONFIG)) {
        return false;
    }
    zbus::block_on(only_session()).unwrap_or_else(|e| {
        log::warn!("lock: cannot list logind's sessions ({e}); no Restart or Turn Off");
        false
    })
}

/// login.toml at `path` names an autologin user, or can't be read.
fn autologin(path: &Path) -> bool {
    match std::fs::read_to_string(path) {
        Ok(text) => (repriselogin::machine::MachineConfig::parse(&text).0)
            .login
            .autologin_user
            .is_some(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            log::warn!("lock: {}: {e}", path.display());
            true
        }
    }
}

async fn only_session() -> zbus::Result<bool> {
    use crate::power::{LOGIN1_MANAGER, LOGIN1_NAME, LOGIN1_PATH};
    let conn = zbus::Connection::system().await?;
    let login1 = zbus::Proxy::new(&conn, LOGIN1_NAME, LOGIN1_PATH, LOGIN1_MANAGER).await?;
    let own = crate::power::session_path(&login1).await?;
    let reply = login1.call_method("ListSessions", &()).await?;
    type Listed = (String, u32, String, String, zbus::OwnedObjectPath);
    for (id, _, user, _, path) in reply.body().deserialize::<Vec<Listed>>()? {
        if path == own {
            continue;
        }
        let class: String = (crate::power::session_proxy(&conn, path).await?)
            .get_property("Class")
            .await?;
        if holds_work(&class) {
            log::info!("lock: session {id} of {user} ({class}) keeps Restart and Turn Off off");
            return Ok(false);
        }
    }
    Ok(true)
}

/// logind's user and background classes, their early and light kinds included; greeters, lock
/// screens and service managers hold no one's work.
fn holds_work(class: &str) -> bool {
    class.starts_with("user") || class.starts_with("background")
}

#[cfg(test)]
mod tests;
