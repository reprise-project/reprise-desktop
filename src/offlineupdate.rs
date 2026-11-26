//! Installing PackageKit's prepared offline update at the next start. Whether one is prepared is
//! read off the loop when a Turn off computer panel or the Glass Start panel opens, at most once
//! per [`FRESH`]; the panel shows its Install updates check box from the last answer, and a box a
//! later answer brings shows unticked. The Glass Shut down options list their two install rows
//! from the last answer.
//!
//! The update is triggered on the power attempt's own thread ([`around`]), just before logind or
//! the `[power]` command turns the computer off or restarts it, so a session end that never gets
//! there leaves nothing set. PackageKit restarts into the desktop once the update is in, whether
//! it was a Turn Off or a Restart. A trigger made there is taken back when the action fails, and
//! when the attempt is given up on while its action runs, by the next attempt or the session's
//! end ([`Made`]), unless the computer is going down by then; a trigger another program made is
//! taken back only when the user unticks a box that showed.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use reprisebus::offline::{After, Status};
use reprisekit::powerpanel::Updates;

use crate::compositor::State;

/// How long an answer stands before a panel opening reads PackageKit again, which starts
/// packagekitd when it is not running.
const FRESH: Duration = Duration::from_secs(10);

/// A bus call's answer: the error is what to tell the user, None for a dismissed password prompt.
pub type Answer = Result<(), Option<String>>;

/// What Reprise asks PackageKit, and logind whether the computer is going down. `interactive`
/// lets polkit ask for a password.
pub trait Bus: Send + Sync {
    /// None when PackageKit does not answer.
    fn status(&self) -> Option<Status>;
    fn trigger(&self, after: After, interactive: bool) -> Answer;
    fn cancel(&self, interactive: bool) -> Answer;
    /// Whether the computer is going down ([`crate::power::shutting_down`]).
    fn shutting_down(&self) -> bool;
}

#[cfg(not(test))]
struct PackageKit;

#[cfg(not(test))]
impl Bus for PackageKit {
    fn status(&self) -> Option<Status> {
        reprisebus::offline::read()
    }
    fn trigger(&self, after: After, interactive: bool) -> Answer {
        reprisebus::offline::trigger_now(after, interactive)
    }
    fn cancel(&self, interactive: bool) -> Answer {
        reprisebus::offline::cancel_now(interactive)
    }
    fn shutting_down(&self) -> bool {
        crate::power::shutting_down()
    }
}

/// Tests see no PackageKit unless they put a fake in.
#[cfg(test)]
struct Absent;

#[cfg(test)]
impl Bus for Absent {
    fn status(&self) -> Option<Status> {
        None
    }
    fn trigger(&self, _: After, _: bool) -> Answer {
        Err(Some("no PackageKit".into()))
    }
    fn cancel(&self, _: bool) -> Answer {
        Err(Some("no PackageKit".into()))
    }
    fn shutting_down(&self) -> bool {
        false
    }
}

pub struct Offline {
    /// PackageKit's last answer.
    pub known: Status,
    /// When it came.
    read: Option<Instant>,
    reading: bool,
    pub(crate) bus: Arc<dyn Bus>,
    pub(crate) made: Made,
}

impl Default for Offline {
    fn default() -> Offline {
        #[cfg(not(test))]
        let bus: Arc<dyn Bus> = Arc::new(PackageKit);
        #[cfg(test)]
        let bus: Arc<dyn Bus> = Arc::new(Absent);
        Offline::on(bus)
    }
}

impl Offline {
    pub fn on(bus: Arc<dyn Bus>) -> Offline {
        Offline {
            known: Status::default(),
            read: None,
            reading: false,
            bus,
            made: Made::default(),
        }
    }
}

/// Read the update's state off the loop unless an answer is under [`FRESH`] old or on its way,
/// then show or hide the check box of any Turn off computer panel open.
pub fn refresh(state: &mut State) {
    let o = &mut state.offline;
    if o.reading || o.read.is_some_and(|t| t.elapsed() < FRESH) {
        return;
    }
    o.reading = true;
    let bus = Arc::clone(&o.bus);
    let spawned = state.spawn_job(move || {
        let status = bus.status();
        Box::new(move |state: &mut State| known(state, status))
    });
    if !spawned {
        state.offline.reading = false;
    }
}

fn known(state: &mut State, status: Option<Status>) {
    let status = status.unwrap_or_default();
    if status != state.offline.known {
        log::info!("offline update: {status:?}");
    }
    let o = &mut state.offline;
    (o.known, o.read, o.reading) = (status, Some(Instant::now()), false);
    for i in 0..state.dialogs.views.len() {
        if state.dialogs.views[i]
            .dialog
            .offer_updates(status.prepared, true)
        {
            state.refit_dialog(i);
            state.redraw_dialog(i);
        }
    }
}

/// The power attempt whose own trigger stands while its action runs. Locked across each Trigger
/// and across each Cancel of an attempt's own trigger, so taking back a trigger an attempt left
/// never meets one being made; Skip cancels another program's trigger unlocked.
#[derive(Clone, Default)]
pub struct Made(Arc<Mutex<Option<u64>>>);

impl Made {
    fn lock(&self) -> MutexGuard<'_, Option<u64>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Take back, blocking, a trigger an attempt other than `but` made and left; false when it
    /// stays.
    pub fn take_back(&self, bus: &dyn Bus, but: Option<u64>) -> bool {
        let down = going_down(self, bus, |id| Some(id) != but);
        left_behind(&mut self.lock(), bus, but, down)
    }

    /// Whether attempt `id`'s trigger stands, leaving it to stay as another program's would: nothing
    /// takes it back after this. False while a Trigger or Cancel holds the lock.
    pub fn hand_over(&self, id: u64) -> bool {
        (self.0.try_lock()).is_ok_and(|mut made| made.take_if(|n| *n == id).is_some())
    }

    #[cfg(test)]
    pub(crate) fn by(&self) -> Option<u64> {
        *self.lock()
    }

    #[cfg(test)]
    pub(crate) fn set(&self, by: Option<u64>) {
        *self.lock() = by;
    }
}

/// Whether the computer is going down while `made` holds a trigger `of` picks, read off the lock:
/// a shutdown's SIGTERM, or its kill of a `[power]` command, must not take back its update.
fn going_down(made: &Made, bus: &dyn Bus, of: impl Fn(u64) -> bool) -> bool {
    let held = made.lock().is_some_and(of);
    held && bus.shutting_down()
}

fn stays_down(id: u64) -> bool {
    log::info!("offline update: the computer is going down; attempt {id}'s trigger stays");
    false
}

/// [`Made::take_back`] with its lock held, `down` read before.
fn left_behind(made: &mut Option<u64>, bus: &dyn Bus, but: Option<u64>, down: bool) -> bool {
    let Some(id) = made.take_if(|id| Some(*id) != but) else {
        return true;
    };
    if down {
        return stays_down(id);
    }
    match bus.cancel(false) {
        Ok(()) => {
            log::info!("offline update: took back the trigger attempt {id} left");
            true
        }
        Err(why) => {
            log::warn!("offline update: the trigger attempt {id} left stays ({why:?})");
            false
        }
    }
}

/// Take back, blocking, a trigger a power attempt given up on left, as the session ends. One that
/// stays is only logged; the next Turn off computer panel offers the update.
pub fn take_back_left(state: &State) {
    state.offline.made.take_back(&*state.offline.bus, None);
}

/// Why [`around`] did not get to the power action, or what its failure left.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Held<F> {
    /// A password prompt was dismissed, or the attempt was given up on.
    Dismissed,
    /// The update could not be set to install; what PackageKit said.
    NotSet(String),
    /// An update another program set to install could not be taken back.
    StillSet,
    /// The power action failed so, and the trigger made for it could not be taken back.
    Stays(F),
}

/// Run `power`, attempt `id`'s logind call or `[power]` command, on its thread with the update as
/// `updates` asks, first taking back a trigger an earlier attempt left. Install triggers it unless
/// the attempt is no longer `still` the newest, nothing is prepared any more, or it is set to
/// install already, and takes back a trigger it made when the attempt was given up on meanwhile
/// or `power` fails, unless the computer is going down. Skip takes back a trigger another program
/// made first.
pub fn around<F>(
    bus: &dyn Bus,
    made: &Made,
    id: u64,
    updates: Updates,
    interactive: bool,
    still: &dyn Fn() -> bool,
    power: impl FnOnce() -> Option<F>,
) -> Result<Option<F>, Held<F>> {
    match updates {
        Updates::Leave => {
            let left = !made.take_back(bus, Some(id));
            stays_if(left, power())
        }
        Updates::Skip => {
            made.take_back(bus, Some(id));
            // Cancel succeeds with nothing triggered, so a status that did not come cancels too.
            if bus.status().is_none_or(|s| s.triggered) {
                bus.cancel(interactive).map_err(|why| match why {
                    None => Held::Dismissed,
                    Some(_) => Held::StillSet,
                })?;
                log::info!("offline update: took back a trigger made elsewhere");
            }
            Ok(power())
        }
        Updates::Install => install(bus, made, id, interactive, still, power),
    }
}

fn install<F>(
    bus: &dyn Bus,
    made: &Made,
    id: u64,
    interactive: bool,
    still: &dyn Fn() -> bool,
    power: impl FnOnce() -> Option<F>,
) -> Result<Option<F>, Held<F>> {
    let down = going_down(made, bus, |n| n != id);
    let mut held = made.lock();
    let left = !left_behind(&mut held, bus, Some(id), down);
    if !still() {
        return Err(Held::Dismissed);
    }
    let status = bus.status();
    match status {
        Some(s) if !s.prepared => {
            log::info!("offline update: nothing is prepared any more; the action goes ahead");
            drop(held);
            return Ok(power());
        }
        Some(s) if s.triggered => {
            log::info!("offline update: already set to install");
            drop(held);
            return stays_if(left, power());
        }
        _ => {}
    }
    bus.trigger(After::Reboot, interactive)
        .map_err(|why| why.map_or(Held::Dismissed, Held::NotSet))?;
    log::info!("offline update: triggered");
    // Over a status that did not come, the trigger may be another program's.
    if status.is_none() {
        drop(held);
        return Ok(power());
    }
    *held = Some(id);
    drop(held);
    if !still() {
        take_own(made, bus, id);
        return Err(Held::Dismissed);
    }
    match power() {
        None => {
            made.lock().take_if(|n| *n == id);
            Ok(None)
        }
        Some(failed) if take_own(made, bus, id) => Ok(Some(failed)),
        Some(failed) => Err(Held::Stays(failed)),
    }
}

/// `failed`, as a failure that leaves the update set to install when a trigger an earlier attempt
/// `left` stays.
fn stays_if<F>(left: bool, failed: Option<F>) -> Result<Option<F>, Held<F>> {
    match failed {
        Some(failed) if left => Err(Held::Stays(failed)),
        failed => Ok(failed),
    }
}

/// Take back attempt `id`'s trigger unless something already did; false when it stays.
fn take_own(made: &Made, bus: &dyn Bus, id: u64) -> bool {
    let down = going_down(made, bus, |n| n == id);
    let mut held = made.lock();
    if held.take_if(|n| *n == id).is_none() {
        return true;
    }
    if down {
        return stays_down(id);
    }
    match bus.cancel(false) {
        Ok(()) => {
            log::info!("offline update: took the trigger back");
            true
        }
        Err(why) => {
            log::warn!("offline update: the trigger stays ({why:?})");
            false
        }
    }
}

#[cfg(test)]
mod tests;
