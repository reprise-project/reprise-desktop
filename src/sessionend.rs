//! Ending the session: Log Off, Turn Off and Restart close programs first so nothing unsaved is lost without a word.
//! `begin` asks every window to close, then the session ends once no window, inhibitor or recording holds it; after `PROMPT_AFTER`
//! the session-end prompt lists what is left. A program holding off log off through the Inhibit portal isn't asked until it lets go:
//! closing its window could stop the work it holds for, as closing a copy's progress dialog cancels the copy. A critical battery kills the remainder after `PROMPT_AFTER` without a prompt.
//! Turn Off and Restart ask logind first (a Restart Into for its target too), and a refusal before or after the programs close shows a message box;
//! one that needs a password runs before the fade, so polkit's prompt shows over the desktop.
//! Stand By and Hibernate close nothing; a refusal shows the same message box.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reprisekit::controls::endsession::{self, Choice, Goal, Row};
use smithay::reexports::calloop::{
    RegistrationToken, channel,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::fsjobs::OnLoop;
use crate::lockview::Origin;
use crate::restartinto::BootTarget;
use crate::window::WindowId;

const PROMPT_AFTER: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(500);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest a session end waits for the remote desktop server to tell its clients.
const TELL_CLIENTS: Duration = Duration::from_secs(1);
/// The longest a session end waits for screenshot files under way to be written.
const SAVE_SHOTS: Duration = Duration::from_secs(15);

const PORTAL: &str = "org.freedesktop.impl.portal.desktop.reprise";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SESSION_END: &str = "org.reprise.SessionEnd";
const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Holder {
    pub who: String,
    pub why: String,
    pub shutdown_only: bool,
    /// The holder's process, when the portal could tell.
    pub pid: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub pid: Option<i32>,
    pub app_id: String,
    pub name: String,
}

pub struct Ending {
    pub goal: Goal,
    cmd: Option<String>,
    /// Where a Restart lands, set with logind just before the reboot.
    boot: Option<BootTarget>,
    holders: Option<Vec<Holder>>,
    stop: Option<async_channel::Sender<()>>,
    /// The poll moves the portal's monitors to query-end ([`asks_portal`]).
    asked_portal: bool,
    timer: Option<RegistrationToken>,
    reports: Option<RegistrationToken>,
    mode: Mode,
    password: Password,
    /// The windows open when the end began that are not asked to close yet ([`ask_windows`]).
    unasked: Vec<WindowId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Waiting,
    Prompted,
    Critical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Wait,
    Prompt,
    Force,
}

impl Mode {
    fn new(critical: bool) -> Mode {
        if critical {
            Mode::Critical
        } else {
            Mode::Waiting
        }
    }

    fn waited(self) -> (Mode, Step) {
        match self {
            Mode::Critical => (Mode::Critical, Step::Force),
            Mode::Waiting | Mode::Prompted => (Mode::Prompted, Step::Prompt),
        }
    }

    /// Another end was asked for while this one is under way. A critical
    /// one takes over, forcing at once if the wait is already over; any
    /// other shows the prompt, unless a critical end is under way, which
    /// it leaves alone.
    fn again(self, critical: bool) -> (Mode, Step) {
        match (self, critical) {
            (Mode::Critical, _) => (Mode::Critical, Step::Wait),
            (Mode::Waiting, true) => (Mode::Critical, Step::Wait),
            (Mode::Prompted, true) => (Mode::Critical, Step::Force),
            (_, false) => (Mode::Prompted, Step::Prompt),
        }
    }
}

pub fn begin(state: &mut State, goal: Goal, cmd: Option<String>, origin: Origin) {
    check(state, goal, cmd, None, false, origin);
}

/// Restart with `cmd` into `boot`.
pub fn begin_into(state: &mut State, cmd: String, boot: BootTarget) {
    check(
        state,
        Goal::Restart,
        Some(cmd),
        Some(boot),
        false,
        Origin::Desktop,
    );
}

pub fn begin_critical(state: &mut State, cmd: String) {
    check(state, Goal::TurnOff, Some(cmd), None, true, Origin::Console);
}

/// Start the end once logind says the action and its restart target are allowed, so a refusal
/// closes nothing. A custom command has only its exit status to go by.
fn check(
    state: &mut State,
    goal: Goal,
    cmd: Option<String>,
    boot: Option<BootTarget>,
    critical: bool,
    origin: Origin,
) {
    if critical || goal == Goal::LogOff {
        if state.powering.give_up_all() {
            log::info!("session end: {goal:?} gives up the power action under way");
        }
    } else if state.powering.busy() {
        log::info!("session end: {goal:?} while a power action is under way; ignored");
        return;
    }
    let verb = cmd.as_deref().and_then(logind_verb);
    if state.ending.is_some() || (verb.is_none() && boot.is_none()) {
        let password = Password::No.with(boot.as_ref());
        start(state, goal, cmd, boot, critical, origin, password);
        return;
    }
    state.spawn_job(move || {
        let answer = zbus::block_on(allowed(verb, boot));
        Box::new(move |state: &mut State| match answer {
            Ok((password, boot)) => start(state, goal, cmd, boot, critical, origin, password),
            Err(why) => refuse(state, Act::of(goal), &why),
        })
    });
}

/// logind's answer for `verb` and `boot`, with `boot`'s password as logind answers now.
async fn allowed(
    verb: Option<Verb>,
    boot: Option<BootTarget>,
) -> Result<(Password, Option<BootTarget>), Refused> {
    let password = match verb {
        Some(verb) => can(verb).await?,
        None => Password::No,
    };
    let boot = match boot {
        Some(b) => match connect(true).await {
            Some(conn) => Some(target_now(&conn, b).await?),
            None => Some(b),
        },
        None => None,
    };
    Ok((password.with(boot.as_ref()), boot))
}

/// `boot` as logind on `conn` offers it now: refused when logind no longer sets a target of its
/// kind or no longer lists its entry. A call that fails lets it through; setting it reports.
async fn target_now(conn: &zbus::Connection, mut boot: BootTarget) -> Result<BootTarget, Refused> {
    let proxy = match crate::restartinto::manager(conn).await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("session end: restart target: {e}");
            return Ok(boot);
        }
    };
    let method = match boot.entry {
        Some(_) => "CanRebootToBootLoaderEntry",
        None => "CanRebootToFirmwareSetup",
    };
    match proxy.call::<_, _, String>(method, &()).await {
        Ok(answer) => {
            log::info!("session end: {method} says {answer:?}");
            boot.password = can_answer(&answer)? == Password::Yes;
        }
        Err(e) => {
            log::warn!("session end: {method}: {e}");
            return Ok(boot);
        }
    }
    if let Some(id) = &boot.entry {
        match proxy.get_property::<Vec<String>>("BootLoaderEntries").await {
            Ok(ids) if !ids.contains(id) => {
                return Err(Refused::Failed(format!(
                    "\"{}\" is no longer a boot entry.",
                    boot.title
                )));
            }
            Ok(_) => {}
            Err(e) => log::warn!("session end: BootLoaderEntries: {e}"),
        }
    }
    Ok(boot)
}

fn start(
    state: &mut State,
    goal: Goal,
    cmd: Option<String>,
    boot: Option<BootTarget>,
    critical: bool,
    origin: Origin,
    password: Password,
) {
    if let Some(e) = state.ending.as_mut() {
        let (mode, step) = e.mode.again(critical);
        if e.mode != Mode::Critical {
            log::info!("session end: now {goal:?}");
            e.goal = goal;
            e.cmd = cmd;
            e.boot = boot;
            e.password = password;
        }
        e.mode = mode;
        run_step(state, step);
        return;
    }
    log::info!(
        "session end: {goal:?}{}",
        if critical { " (critical battery)" } else { "" }
    );
    state.session.kept = false;
    crate::session::end(state);
    state.session.kept = true;
    if state.locked_for(origin) {
        log::info!("session end: locked, nobody to ask");
        go_on(state, goal, cmd, boot, false, password);
        return;
    }
    let (tx, rx) = channel::channel::<Vec<Holder>>();
    let reports = state
        .handle
        .insert_source(rx, |event, _, state| {
            if let channel::Event::Msg(h) = event
                && let Some(e) = state.ending.as_mut()
            {
                e.holders = Some(h);
                ask_windows(state);
            }
        })
        .map_err(|e| log::warn!("session end: no report channel: {}", e.error))
        .ok();
    let ask_portal = asks_portal(&state.env, goal, crate::spawn::another_session_runs_reprise);
    let stop = spawn_poll(tx, ask_portal);
    let timer = state
        .handle
        .insert_source(Timer::from_duration(PROMPT_AFTER), |_, _, state| {
            if let Some(e) = state.ending.as_mut() {
                e.timer = None;
                let (mode, step) = e.mode.waited();
                e.mode = mode;
                run_step(state, step);
            }
            TimeoutAction::Drop
        })
        .map_err(|e| log::warn!("session end: no timer: {}", e.error))
        .ok();
    // The compositor's own dialogs (Run, a hide question) go first.
    // The windows are asked once the end is under way, so a program
    // already known to hang gets no End Program of its own.
    state.close_all_dialogs();
    let unasked: Vec<WindowId> = state
        .windows
        .iter()
        .filter(|w| w.mapped && !w.override_redirect)
        .map(|w| w.id)
        .collect();
    state.ending = Some(Ending {
        goal,
        cmd,
        boot,
        // Without the thread nothing is known to hold the session.
        holders: stop.is_none().then(Vec::new),
        stop,
        asked_portal: ask_portal,
        timer,
        reports,
        mode: Mode::new(critical),
        password,
        unasked,
    });
    ask_windows(state);
}

/// Ask the windows still unasked to close once the holders are known, all but those of a process
/// holding off log off through the portal, which wait until it lets go, and modal dialogs over
/// their program's windows ([`modal_over_own`]).
fn ask_windows(state: &mut State) {
    let Some(e) = state.ending.as_mut() else {
        return;
    };
    let Some(holders) = e.holders.as_ref() else {
        return;
    };
    let held: Vec<i32> = (holders.iter())
        .filter(|h| !h.shutdown_only)
        .filter_map(|h| h.pid)
        .collect();
    let unasked = std::mem::take(&mut e.unasked);
    let (wait, ask): (Vec<WindowId>, Vec<WindowId>) = (unasked.into_iter())
        .partition(|&w| state.client_pid(w).is_some_and(|pid| held.contains(&pid)));
    if let Some(e) = state.ending.as_mut() {
        e.unasked = wait;
    }
    for w in ask {
        if !modal_over_own(state, w) {
            state.close(w);
        }
    }
}

/// Whether `w` is a modal dialog over another window of its program. It is not asked to close:
/// a program takes that as its answer (Cancel), and the window under it, asked instead, drops the
/// request while the dialog is up, so the program stays listed with its question on screen.
fn modal_over_own(state: &State, w: WindowId) -> bool {
    let pid = state.client_pid(w);
    state.win(w).is_some_and(|x| x.is_modal())
        && (state.owner(w)).is_some_and(|o| pid.is_none() || state.client_pid(o) == pid)
}

pub fn tick(state: &mut State) {
    let Some(e) = state.ending.as_ref() else {
        return;
    };
    let programs = programs(state);
    let holders = e.holders.as_ref().map(|h| holding(h, e.goal));
    let busy = crate::screenrecord::finishing(state);
    if programs.is_empty() && !busy && holders.as_ref().is_some_and(Vec::is_empty) {
        log::info!("session end: every program closed");
        finish(state, false);
        return;
    }
    if e.mode == Mode::Prompted {
        sync_prompt(state);
    }
}

fn run_step(state: &mut State, step: Step) {
    match step {
        Step::Wait => {}
        Step::Prompt => sync_prompt(state),
        Step::Force => {
            log::info!("session end: critical battery, ending what is left");
            end_now(state, true);
        }
    }
}

/// `override_inhibitors`: End Now, after the prompt showed the inhibitors,
/// or a critical battery, which takes the power anyway.
fn end_now(state: &mut State, override_inhibitors: bool) {
    let pids: Vec<i32> = programs(state).iter().filter_map(|p| p.pid).collect();
    for pid in pids {
        log::info!("session end: ending pid {pid}");
        state.end_process(pid);
    }
    finish(state, override_inhibitors);
}

pub fn answer(state: &mut State, choice: Choice) {
    if let Some(i) = prompt_index(state) {
        state.close_dialog_at(i);
    }
    match choice {
        Choice::EndNow => {
            log::info!("session end: End Now");
            end_now(state, true);
        }
        Choice::Cancel => cancel(state),
    }
}

fn cancel(state: &mut State) {
    let Some(mut e) = state.ending.take() else {
        return;
    };
    log::info!("session end: cancelled");
    if let Some(stop) = e.stop.take() {
        let _ = stop.try_send(());
    }
    drop_sources(state, &mut e);
    state.session.kept = false;
}

fn finish(state: &mut State, override_inhibitors: bool) {
    let Some(mut e) = state.ending.take() else {
        return;
    };
    drop_sources(state, &mut e);
    if let Some(i) = prompt_index(state) {
        state.close_dialog_at(i);
    }
    if resets_portal(&e, &state.env, crate::spawn::another_session_runs_reprise)
        && let Some(stop) = e.stop.take()
    {
        let _ = stop.try_send(());
    }
    go_on(
        state,
        e.goal,
        e.cmd,
        e.boot,
        override_inhibitors,
        e.password,
    );
}

/// Whether a committed end moves the portal's monitors back to running: a command (Turn Off,
/// Restart) leaves the compositor running, and another session may have started since the ask.
fn resets_portal(
    e: &Ending,
    env: &crate::spawn::SessionEnv,
    another_runs: impl FnOnce() -> bool,
) -> bool {
    e.cmd.is_some() || (e.asked_portal && !asks_portal(env, e.goal, another_runs))
}

/// Wait for screenshot files under way, [`SAVE_SHOTS`] at most, fade the screen out, then end: stop
/// the compositor for Log Off, else run `cmd`. A `cmd` that asks for a password runs before the
/// fade, its prompt over the desktop, and the session winds down once the action is under way.
fn go_on(
    state: &mut State,
    goal: Goal,
    cmd: Option<String>,
    boot: Option<BootTarget>,
    override_inhibitors: bool,
    password: Password,
) {
    crate::crash::ending();
    match cmd {
        Some(cmd) if password.asked(override_inhibitors) && prompts(state) => {
            log::info!("session end: {goal:?} runs {cmd:?} before the fade, for its password");
            crate::screenshot::when_saved(
                state,
                SAVE_SHOTS,
                Box::new(move |state| {
                    let act = Act::of(goal);
                    attempt(state, act, cmd, boot, override_inhibitors, Some(goal));
                }),
            );
        }
        cmd => wind_down(
            state,
            goal,
            Box::new(move |state| act(state, goal, cmd, boot, override_inhibitors)),
        ),
    }
}

/// Fade the screen out, then run `end`. Remote clients hear the session logged off; `end` waits
/// for the server's answer, [`TELL_CLIENTS`] at most.
fn wind_down(state: &mut State, goal: Goal, end: OnLoop) {
    let powers_off = goal != Goal::LogOff;
    // A deferring session's Turn Off also ends the other session's clients.
    let server_ours = state.standby.rdp_on() || (powers_off && state.env.defers_user_units);
    let tells = crate::console::remote_side(state) || server_ours;
    let wait = fade_out(state, !tells, SAVE_SHOTS, end);
    if !tells {
        return;
    }
    let told = Arc::clone(&wait);
    let why = match powers_off {
        true => crate::console::ClientsEnd::PoweredOff,
        false => crate::console::ClientsEnd::LoggedOff,
    };
    crate::console::end_clients_then(
        state,
        why,
        Box::new(move |state| arrive(&told, state, |w| w.told = true)),
    );
    let late = Arc::clone(&wait);
    let armed =
        (state.handle).insert_source(Timer::from_duration(TELL_CLIENTS), move |_, _, state| {
            arrive(&late, state, |w| w.told = true);
            TimeoutAction::Drop
        });
    if let Err(e) = armed {
        log::warn!("session end: no timer for the clients: {}", e.error);
        arrive(&wait, state, |w| w.told = true);
    }
}

/// Wait for screenshot files under way (`saves` at most) with the desktop up, then fade out; `end`
/// runs once the screen is black and the clients are `told`.
fn fade_out(state: &mut State, told: bool, saves: Duration, end: OnLoop) -> Arc<Mutex<Wait>> {
    let wait = Arc::new(Mutex::new(Wait {
        faded: false,
        told,
        end: Some(end),
    }));
    let faded = Arc::clone(&wait);
    crate::screenshot::when_saved(
        state,
        saves,
        Box::new(move |state| {
            crate::sessionfade::out(
                state,
                Box::new(move |state| arrive(&faded, state, |w| w.faded = true)),
            );
        }),
    );
    wait
}

/// A session end waiting for its fade and for the remote clients to hear of it.
struct Wait {
    faded: bool,
    told: bool,
    end: Option<OnLoop>,
}

/// Mark what arrived, then end once everything has.
fn arrive(wait: &Mutex<Wait>, state: &mut State, mark: impl FnOnce(&mut Wait)) {
    let end = {
        let mut w = wait.lock().unwrap_or_else(|e| e.into_inner());
        mark(&mut w);
        if w.faded && w.told {
            w.end.take()
        } else {
            None
        }
    };
    if let Some(end) = end {
        end(state);
    }
}

/// End the session now, whatever is open (a SIGTERM, a GPU gone): once screenshot files under way
/// are written, fade out, then stop the loop.
pub fn quit(state: &mut State) {
    crate::crash::ending();
    fade_out(
        state,
        true,
        SAVE_SHOTS,
        Box::new(|state| state.signal.stop()),
    );
}

fn act(
    state: &mut State,
    goal: Goal,
    cmd: Option<String>,
    boot: Option<BootTarget>,
    override_inhibitors: bool,
) {
    let Some(cmd) = cmd else {
        let stale = std::mem::take(&mut state.powering.stale);
        let spawned = !stale.is_empty()
            && state.spawn_job(move || {
                clear_stale(&stale);
                Box::new(|state: &mut State| state.signal.stop())
            });
        if !spawned {
            state.signal.stop();
        }
        return;
    };
    log::info!("session end: {goal:?} runs {cmd:?}");
    attempt(state, Act::of(goal), cmd, boot, override_inhibitors, None);
}

/// Put the computer to sleep with a `[power]` standby or hibernate `cmd`, closing nothing.
pub fn sleep(state: &mut State, act: Act, cmd: String) {
    if cmd.trim().is_empty() {
        return;
    }
    if state.powering.busy() {
        log::info!("power: {act:?} while a power action is under way; ignored");
        return;
    }
    log::info!("power: {act:?} runs {cmd:?}");
    attempt(state, act, cmd, None, false, None);
}

/// The power actions run off the loop ([`attempt`]), one at a time. Each is numbered; one given up
/// on (it ran past [`ANSWER_WITHIN`], or a newer attempt, a critical battery or Log Off took over)
/// answers nothing.
#[derive(Default)]
pub struct Powering {
    /// The newest number handed out; a thread reads it to learn its attempt was given up on.
    newest: Arc<AtomicU64>,
    /// The attempt waiting for its answer and the restart target it sets.
    under_way: Option<(u64, Option<BootTarget>)>,
    /// Custom Stand By and Hibernate commands still running past [`ANSWER_WITHIN`] ([`late`]).
    lingering: Vec<u64>,
    /// Restart targets that may still be set, each with the attempt that set it, cleared before
    /// the next power action.
    stale: Vec<(u64, BootTarget)>,
}

impl Powering {
    pub fn busy(&self) -> bool {
        self.under_way.is_some()
    }

    /// Number a new attempt, giving up on the one under way.
    fn begin(&mut self, boot: Option<BootTarget>) -> u64 {
        self.give_up();
        let id = self.newest.fetch_add(1, Ordering::SeqCst) + 1;
        self.under_way = Some((id, boot));
        id
    }

    /// The restart target attempt `id` set when it may still answer, which it no longer may.
    fn answer(&mut self, id: u64) -> Option<Option<BootTarget>> {
        if let Some(i) = self.lingering.iter().position(|&n| n == id) {
            self.lingering.remove(i);
            return Some(None);
        }
        if !self.under_way.as_ref().is_some_and(|(n, _)| *n == id) {
            return None;
        }
        self.under_way.take().map(|(_, boot)| boot)
    }

    /// Give up on the attempt under way, if any.
    fn give_up(&mut self) -> bool {
        let Some((id, boot)) = self.under_way.take() else {
            return false;
        };
        self.newest.fetch_add(1, Ordering::SeqCst);
        self.stale.extend(boot.map(|b| (id, b)));
        true
    }

    /// [`Powering::give_up`], and on the lingering commands too.
    fn give_up_all(&mut self) -> bool {
        let lingered = !self.lingering.is_empty();
        self.lingering.clear();
        self.give_up() || lingered
    }

    /// Attempt `id`, under way, no longer holds the next one back but still answers.
    fn linger(&mut self, id: u64) {
        if self.under_way.as_ref().is_some_and(|(n, _)| *n == id) {
            self.under_way = None;
            self.lingering.push(id);
        }
    }

    /// Whether attempt `id` is still the newest, as its thread sees it.
    fn still(newest: &AtomicU64, id: u64) -> bool {
        newest.load(Ordering::SeqCst) == id
    }
}

/// How long a power action may take to answer before Reprise gives up on it ([`late`]): logind
/// stops waiting for a password after 25 s a call, and a Restart Into makes up to three.
const ANSWER_WITHIN: Duration = Duration::from_secs(120);

/// Run `cmd` off the loop, on logind when it names a logind action, into `boot` when set, then
/// [`answered`]. Restart targets left by attempts given up on are cleared first.
fn attempt(
    state: &mut State,
    act: Act,
    cmd: String,
    boot: Option<BootTarget>,
    override_inhibitors: bool,
    wind_down_for: Option<Goal>,
) {
    let env = state.env.clone();
    let interactive = prompts(state);
    let stale = std::mem::take(&mut state.powering.stale);
    let id = state.powering.begin(boot.clone());
    let newest = Arc::clone(&state.powering.newest);
    let lingers = matches!(act, Act::StandBy | Act::Hibernate) && logind_verb(&cmd).is_none();
    let deadline =
        (state.handle).insert_source(Timer::from_duration(ANSWER_WITHIN), move |_, _, state| {
            late(state, id, act, lingers);
            TimeoutAction::Drop
        });
    if let Err(e) = deadline {
        log::warn!("session end: no deadline for {act:?}: {}", e.error);
    }
    let spawned = state.spawn_job(move || {
        let failed = unpanicked(|| {
            clear_stale(&stale);
            let go = || match logind_verb(&cmd) {
                Some(verb) => zbus::block_on(call(verb, override_inhibitors, interactive)),
                None => run(&cmd, &env).map(Failed::Refused),
            };
            match boot {
                None => go(),
                Some(boot) => match zbus::block_on(zbus::Connection::system()) {
                    Ok(conn) => {
                        let still = || Powering::still(&newest, id);
                        restart_into(&conn, &boot, interactive, still, go)
                    }
                    Err(e) => Some(Failed::Refused(Refused::Failed(e.to_string()))),
                },
            }
        });
        Box::new(move |state: &mut State| answered(state, id, act, failed, wind_down_for))
    });
    if !spawned {
        state.powering.answer(id);
    }
}

/// `work`'s answer, or a refusal when it panics, so the attempt still answers.
fn unpanicked(work: impl FnOnce() -> Option<Failed>) -> Option<Failed> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| {
        log::error!("session end: a power action panicked");
        Some(Failed::Refused(Refused::Internal))
    })
}

/// Attempt `id` ran past [`ANSWER_WITHIN`]: unless it answered, give up on it and refuse. One that
/// `lingers`, a custom Stand By or Hibernate command, may wait on purpose (a locker, the resume):
/// it only stops holding the next action back, and a failure still shows.
fn late(state: &mut State, id: u64, act: Act, lingers: bool) {
    if !state
        .powering
        .under_way
        .as_ref()
        .is_some_and(|(n, _)| *n == id)
    {
        return;
    }
    if lingers {
        log::info!("power: {act:?} still runs after {ANSWER_WITHIN:?}");
        state.powering.linger(id);
        return;
    }
    state.powering.give_up();
    refuse(state, act, &Refused::Late);
}

/// Clear restart targets attempts given up on may have set, asking for no password; one that
/// fails is logged and the power action goes ahead.
fn clear_stale(stale: &[(u64, BootTarget)]) {
    if stale.is_empty() {
        return;
    }
    if let Some(conn) = zbus::block_on(connect(true)) {
        zbus::block_on(clear(&conn, stale));
    }
}

async fn clear(conn: &zbus::Connection, stale: &[(u64, BootTarget)]) {
    for (_, b) in stale {
        match crate::restartinto::aim(conn, b, false, false).await {
            Ok(()) => log::info!("session end: cleared the restart target {:?}", b.entry),
            Err(e) => log::warn!(
                "session end: clearing the restart target {:?}: {e}",
                b.entry
            ),
        }
    }
}

/// Set `boot` with logind on `conn`, then `reboot` while the attempt is `still` the newest; a
/// reboot that does not happen clears it, so a later restart lands where it always does. The clear
/// may prompt as the set did; one that fails is named in the refusal.
fn restart_into(
    conn: &zbus::Connection,
    boot: &BootTarget,
    interactive: bool,
    still: impl FnOnce() -> bool,
    reboot: impl FnOnce() -> Option<Failed>,
) -> Option<Failed> {
    if let Err(e) = zbus::block_on(crate::restartinto::aim(conn, boot, true, interactive)) {
        log::warn!("session end: restart into {:?}: {e}", boot.entry);
        let dismissed = || interactive && reprisebus::polkit::dismissed();
        return Some(failure(&e, dismissed).map_or(Failed::Dismissed, Failed::Refused));
    }
    let failed = match still() {
        true => reboot()?,
        false => {
            log::info!("session end: restart into {:?} given up on", boot.entry);
            Failed::Dismissed
        }
    };
    match zbus::block_on(crate::restartinto::aim(conn, boot, false, interactive)) {
        Ok(()) => Some(failed),
        Err(e) => {
            log::warn!("session end: clearing the restart target: {e}");
            let why = match failed {
                Failed::Refused(why) => Some(Box::new(why)),
                Failed::Dismissed => None,
            };
            Some(Failed::Refused(Refused::Aimed(why, boot.title.clone())))
        }
    }
}

/// A refusal shows a message box; a dismissed password prompt brings the session back without
/// one. An action under way winds the session down for `wind_down_for`. An attempt given up on
/// answers nothing. A restart target a panic or a failed clear left set is stale.
fn answered(
    state: &mut State,
    id: u64,
    act: Act,
    failed: Option<Failed>,
    wind_down_for: Option<Goal>,
) {
    let Some(boot) = state.powering.answer(id) else {
        log::info!("session end: {act:?} given up on answered {failed:?}");
        return;
    };
    if let Some(Failed::Refused(Refused::Internal | Refused::Aimed(..))) = &failed {
        state.powering.stale.extend(boot.map(|b| (id, b)));
    }
    match failed {
        None => {
            if let Some(goal) = wind_down_for {
                wind_down(state, goal, Box::new(|_| {}));
            }
        }
        Some(Failed::Dismissed) => {
            log::info!("session end: {act:?} password prompt dismissed");
            resume(state, act);
        }
        Some(Failed::Refused(why)) => refuse(state, act, &why),
    }
}

/// Whether polkit may ask for a password: not while the session is locked, where its prompt
/// would map under the lock and the screen would stay faded out waiting for it.
fn prompts(state: &State) -> bool {
    !state.console_locked()
}

/// What a refusal's message box names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    TurnOff,
    Restart,
    StandBy,
    Hibernate,
}

impl Act {
    fn of(goal: Goal) -> Act {
        match goal {
            Goal::Restart => Act::Restart,
            Goal::TurnOff | Goal::LogOff => Act::TurnOff,
        }
    }

    fn title(self) -> &'static str {
        match self {
            Act::TurnOff => "Turn Off Computer",
            Act::Restart => "Restart Computer",
            Act::StandBy => "Stand By",
            Act::Hibernate => "Hibernate",
        }
    }

    /// The action done to `the` computer ("this", "the").
    fn on(self, the: &str) -> String {
        match self {
            Act::TurnOff => format!("turn off {the} computer"),
            Act::Restart => format!("restart {the} computer"),
            Act::StandBy => format!("put {the} computer on stand by"),
            Act::Hibernate => format!("hibernate {the} computer"),
        }
    }
}

/// How a power action did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Failed {
    Dismissed,
    Refused(Refused),
}

/// Why the computer stayed up.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Refused {
    NotAllowed,
    Unsupported,
    Held(Vec<Holder>),
    Failed(String),
    /// The reboot did not happen (`why`, None for a dismissed password prompt) and the restart
    /// target set for it could not be cleared: the next restart goes to the target titled here.
    Aimed(Option<Box<Refused>>, String),
    /// No answer came within [`ANSWER_WITHIN`].
    Late,
    /// Reprise itself failed: the attempt panicked, or its thread did not start.
    Internal,
}

/// The session goes on after a power action that did not happen.
fn resume(state: &mut State, act: Act) {
    if matches!(act, Act::TurnOff | Act::Restart) {
        crate::crash::not_ending();
        state.session.kept = false;
    }
    crate::sessionfade::restore(state);
}

fn refuse(state: &mut State, act: Act, why: &Refused) {
    log::warn!("session end: {act:?} refused: {why:?}");
    match why {
        // The action may still go through, and the session stays saved as it ended.
        Refused::Late => crate::sessionfade::restore(state),
        _ => resume(state, act),
    }
    crate::lockview::refused(state, refusal_text(act, &unnamed(why)));
    state.open_dialog(crate::dialog::Dialog::error(
        act.title(),
        refusal_text(act, why),
    ));
}

/// `why` for the lock, which anyone at the computer reads: it names no program and quotes no
/// command's error.
fn unnamed(why: &Refused) -> Refused {
    match why {
        Refused::Held(_) => Refused::Held(Vec::new()),
        Refused::Failed(_) => Refused::Failed(LOCK_FAILED.into()),
        Refused::Aimed(why, title) => {
            Refused::Aimed(why.as_deref().map(|w| Box::new(unnamed(w))), title.clone())
        }
        why => why.clone(),
    }
}

/// What the lock shows for a failed command or call; the dialog under it keeps the error.
const LOCK_FAILED: &str = "An error stopped it.";

fn refusal_text(act: Act, why: &Refused) -> String {
    format!(
        "Reprise could not {}.\n\n{}",
        act.on("the"),
        reason(act, why)
    )
}

fn reason(act: Act, why: &Refused) -> String {
    match why {
        Refused::NotAllowed => format!("You are not allowed to {}.", act.on("this")),
        Refused::Unsupported => "This computer does not support it.".to_string(),
        Refused::Held(holders) if holders.is_empty() => "A program is preventing it.".to_string(),
        Refused::Held(holders) => {
            let mut text = "These programs are preventing it:".to_string();
            for h in holders {
                text.push('\n');
                text.push_str(&h.who);
                if !h.why.is_empty() {
                    text.push_str(": ");
                    text.push_str(&h.why);
                }
            }
            text
        }
        Refused::Failed(m) => m.clone(),
        Refused::Late => format!(
            "It had not happened after {} minutes.",
            ANSWER_WITHIN.as_secs() / 60
        ),
        Refused::Internal => "An error in Reprise stopped it.".to_string(),
        Refused::Aimed(why, title) => {
            let kept = format!("The next restart still starts {title}.");
            match why {
                Some(why) => format!("{}\n\n{kept}", reason(act, why)),
                None => kept,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    PowerOff,
    Reboot,
    Suspend,
    Hibernate,
}

impl Verb {
    fn can(self) -> &'static str {
        match self {
            Verb::PowerOff => "CanPowerOff",
            Verb::Reboot => "CanReboot",
            Verb::Suspend => "CanSuspend",
            Verb::Hibernate => "CanHibernate",
        }
    }

    fn method(self) -> &'static str {
        match self {
            Verb::PowerOff => "PowerOff",
            Verb::Reboot => "Reboot",
            Verb::Suspend => "Suspend",
            Verb::Hibernate => "Hibernate",
        }
    }

    fn method_with_flags(self) -> &'static str {
        match self {
            Verb::PowerOff => "PowerOffWithFlags",
            Verb::Reboot => "RebootWithFlags",
            Verb::Suspend => "SuspendWithFlags",
            Verb::Hibernate => "HibernateWithFlags",
        }
    }

    /// The inhibitor `what` that blocks this action.
    fn inhibited_by(self) -> &'static str {
        match self {
            Verb::PowerOff | Verb::Reboot => "shutdown",
            Verb::Suspend | Verb::Hibernate => "sleep",
        }
    }
}

/// The logind action `cmd` is when it says exactly `systemctl` or `loginctl`
/// with `poweroff`, `reboot`, `suspend` or `hibernate`; logind is then called directly, so polkit
/// sees the compositor and a dismissed prompt can be told apart.
fn logind_verb(cmd: &str) -> Option<Verb> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let [program, verb] = words[..] else {
        return None;
    };
    let name = program.rsplit('/').next()?;
    if name != "systemctl" && name != "loginctl" {
        return None;
    }
    match verb {
        "poweroff" => Some(Verb::PowerOff),
        "reboot" => Some(Verb::Reboot),
        "suspend" => Some(Verb::Suspend),
        "hibernate" => Some(Verb::Hibernate),
        _ => None,
    }
}

/// Whether polkit asks for a password before a logind action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Password {
    No,
    Yes,
    /// Only to ignore the inhibitors, as End Now does.
    OverInhibitors,
}

impl Password {
    /// A target polkit guards asks for the password up front, as the action would.
    fn with(self, boot: Option<&BootTarget>) -> Password {
        match boot {
            Some(b) if b.password => Password::Yes,
            _ => self,
        }
    }

    fn asked(self, override_inhibitors: bool) -> bool {
        match self {
            Password::No => false,
            Password::Yes => true,
            Password::OverInhibitors => override_inhibitors,
        }
    }
}

/// logind's `Can*` answer: a refusal, else whether a password is asked. `inhibited` goes ahead,
/// its holders listed by the prompt.
fn can_answer(answer: &str) -> Result<Password, Refused> {
    match answer {
        "yes" => Ok(Password::No),
        "challenge" => Ok(Password::Yes),
        "inhibited" => Ok(Password::OverInhibitors),
        "na" => Err(Refused::Unsupported),
        _ => Err(Refused::NotAllowed),
    }
}

/// A refusal names the shutdown inhibitors, when there are any.
async fn with_holders(conn: &zbus::Connection, verb: Verb, why: Refused) -> Refused {
    if !matches!(why, Refused::NotAllowed | Refused::Held(_)) {
        return why;
    }
    let holders = logind_holders(conn, verb.inhibited_by()).await;
    if holders.is_empty() {
        why
    } else {
        Refused::Held(holders)
    }
}

/// Unreachable logind lets the end go ahead: the action itself reports.
async fn can(verb: Verb) -> Result<Password, Refused> {
    let Some(conn) = connect(true).await else {
        return Ok(Password::No);
    };
    let reply = conn
        .call_method(
            Some(LOGIND),
            LOGIND_PATH,
            Some(LOGIND_MANAGER),
            verb.can(),
            &(),
        )
        .await
        .and_then(|m| m.body().deserialize::<String>());
    match reply {
        Ok(answer) => {
            log::info!("session end: {} says {answer:?}", verb.can());
            match can_answer(&answer) {
                Ok(password) => Ok(password),
                Err(why) => Err(with_holders(&conn, verb, why).await),
            }
        }
        Err(e) => {
            log::warn!("session end: {}: {e}", verb.can());
            Ok(Password::No)
        }
    }
}

/// `SD_LOGIND_SKIP_INHIBITORS` from org.freedesktop.login1(5): bypasses
/// block inhibitors, subject to polkit's `*-ignore-inhibit`.
const SKIP_INHIBITORS: u64 = 1 << 4;

fn logind_flags(override_inhibitors: bool) -> u64 {
    if override_inhibitors {
        SKIP_INHIBITORS
    } else {
        0
    }
}

/// A logind without `*WithFlags` (before 248) or without the skip flag
/// (before 257); the plain call lets polkit decide on the inhibitors there.
fn too_old_for_flags(e: &zbus::Error) -> bool {
    matches!(
        reprisebus::error::name(e).as_deref(),
        Some("org.freedesktop.DBus.Error.UnknownMethod" | "org.freedesktop.DBus.Error.InvalidArgs")
    )
}

async fn call(verb: Verb, override_inhibitors: bool, interactive: bool) -> Option<Failed> {
    // No method timeout: a password prompt stays up as long as polkit lets it.
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return Some(Failed::Refused(Refused::Failed(e.to_string()))),
    };
    let reply = async {
        let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(&conn)
            .destination(LOGIND)
            .path(LOGIND_PATH)
            .interface(LOGIND_MANAGER)
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let auth = match interactive {
            true => zbus::proxy::MethodFlags::ALLOW_INTERACTIVE_AUTH,
            false => zbus::proxy::MethodFlags::empty(),
        };
        let flags = logind_flags(override_inhibitors);
        match proxy
            .call_with_flags::<_, _, ()>(verb.method_with_flags(), auth, &(flags,))
            .await
        {
            Err(e) if too_old_for_flags(&e) => {
                log::info!("session end: {}: {e}", verb.method_with_flags());
                proxy
                    .call_with_flags::<_, _, ()>(verb.method(), auth, &(interactive,))
                    .await
            }
            r => r,
        }
    }
    .await;
    let e = reply.err()?;
    let dismissed = || interactive && reprisebus::polkit::dismissed();
    let Some(why) = failure(&e, dismissed) else {
        return Some(Failed::Dismissed);
    };
    Some(Failed::Refused(with_holders(&conn, verb, why).await))
}

/// A failed logind call as a refusal; a dismissed password prompt is none.
fn failure(e: &zbus::Error, dismissed: impl FnOnce() -> bool) -> Option<Refused> {
    let refused = |name: &str| {
        name == "org.freedesktop.DBus.Error.AccessDenied"
            || name == "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired"
    };
    match reprisebus::polkit::refusal(e, refused, dismissed) {
        Some(reprisebus::polkit::Refusal::Dismissed) => None,
        Some(reprisebus::polkit::Refusal::Denied) => Some(Refused::NotAllowed),
        Some(reprisebus::polkit::Refusal::TimedOut) => {
            Some(Refused::Failed(reprisebus::polkit::TIMED_OUT.to_string()))
        }
        None => Some(match reprisebus::error::name(e).as_deref() {
            Some("org.freedesktop.login1.BlockedByInhibitorLock") => Refused::Held(Vec::new()),
            Some("org.freedesktop.DBus.Error.Timeout") => {
                Refused::Failed(reprisebus::polkit::TIMED_OUT.to_string())
            }
            _ => Refused::Failed(
                reprisebus::error::message(e).map_or_else(|| e.to_string(), String::from),
            ),
        }),
    }
}

/// Run a custom `[power]` command and wait for it. Only the command is waited for: a child it
/// leaves running (a locker started with `-f`) may hold its stderr open for as long as it runs.
fn run(cmd: &str, env: &crate::spawn::SessionEnv) -> Option<Refused> {
    let Some(mut c) = crate::spawn::command(cmd, env, None) else {
        return Some(Refused::NotAllowed);
    };
    crate::spawn::detach(&mut c);
    c.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    let mut child = match c.spawn() {
        Ok(child) => child,
        Err(e) => return Some(Refused::Failed(format!("\"{cmd}\" did not start: {e}"))),
    };
    let said = Arc::new(Mutex::new(Vec::new()));
    let (closed, on_close) = std::sync::mpsc::channel();
    if let Some(stderr) = child.stderr.take() {
        let said = Arc::clone(&said);
        std::thread::spawn(move || {
            read_tail(stderr, &said);
            let _ = closed.send(());
        });
    }
    let status = child.wait();
    let _ = on_close.recv_timeout(STDERR_AFTER_EXIT);
    let said = said.lock().unwrap_or_else(|e| e.into_inner()).clone();
    match status {
        Ok(s) if s.success() => None,
        Ok(s) => Some(Refused::Failed(command_failure(cmd, &s, &said))),
        Err(e) => Some(Refused::Failed(format!("\"{cmd}\" did not start: {e}"))),
    }
}

/// How long a command's last stderr lines may trail its exit.
const STDERR_AFTER_EXIT: Duration = Duration::from_millis(200);
/// The most of a command's stderr kept, its end.
const STDERR_KEPT: usize = 8192;

/// Read `from` until it closes, keeping its last [`STDERR_KEPT`] bytes in `into`.
fn read_tail(mut from: impl std::io::Read, into: &Mutex<Vec<u8>>) {
    let mut buf = [0; 4096];
    loop {
        let n = match from.read(&mut buf) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return,
        };
        let mut kept = into.lock().unwrap_or_else(|e| e.into_inner());
        kept.extend_from_slice(&buf[..n]);
        let over = kept.len().saturating_sub(STDERR_KEPT);
        kept.drain(..over);
    }
}

/// The last line `cmd` wrote to stderr, else its exit status.
fn command_failure(cmd: &str, status: &std::process::ExitStatus, stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    match text.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        Some(line) => line.to_string(),
        None => format!("\"{cmd}\" failed ({status})."),
    }
}

fn drop_sources(state: &mut State, e: &mut Ending) {
    for t in [e.timer.take(), e.reports.take()].into_iter().flatten() {
        state.handle.remove(t);
    }
}

/// The programs whose windows are open.
fn programs(state: &State) -> Vec<Program> {
    let open: Vec<Shown<WindowId>> = state
        .windows
        .iter()
        .filter(|w| w.mapped && !w.override_redirect)
        .map(|w| Shown {
            id: w.id,
            pid: state.client_pid(w.id),
            app_id: w.app(),
            title: w.title(),
            owner: state.parent_window(w.id),
        })
        .collect();
    group(open)
}

/// A window as [`group`] sees it.
struct Shown<I> {
    id: I,
    pid: Option<i32>,
    app_id: String,
    title: String,
    owner: Option<I>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Key<I> {
    Pid(i32),
    Window(I),
}

/// [`programs`] over the open windows: windows of one pid are one program,
/// named by its first unowned window's title, else its app id. A window with
/// no pid (an X11 client without `_NET_WM_PID`) joins its owner's program, or
/// is a program of its own.
fn group<I: Copy + Eq>(windows: Vec<Shown<I>>) -> Vec<Program> {
    let key = |w: &Shown<I>| {
        let mut at = w;
        for _ in 0..windows.len() {
            if let Some(pid) = at.pid {
                return Key::Pid(pid);
            }
            match at.owner.and_then(|o| windows.iter().find(|x| x.id == o)) {
                Some(owner) => at = owner,
                None => break,
            }
        }
        Key::Window(at.id)
    };
    let mut out: Vec<(Key<I>, Program, bool)> = Vec::new();
    for w in &windows {
        let k = key(w);
        let owned = w.owner.is_some();
        let name = if w.title.is_empty() {
            w.app_id.clone()
        } else {
            w.title.clone()
        };
        match out.iter_mut().find(|(at, _, _)| *at == k) {
            // An owned window names the program only until an unowned
            // one comes.
            Some((_, p, named_by_owned)) => {
                if *named_by_owned && !owned {
                    p.name = name;
                    *named_by_owned = false;
                }
            }
            None => out.push((
                k,
                Program {
                    pid: w.pid,
                    app_id: w.app_id.clone(),
                    name,
                },
                owned,
            )),
        }
    }
    out.into_iter().map(|(_, p, _)| p).collect()
}

fn holding(holders: &[Holder], goal: Goal) -> Vec<Holder> {
    holders
        .iter()
        .filter(|h| !h.shutdown_only || goal != Goal::LogOff)
        .cloned()
        .collect()
}

fn rows(programs: &[Program], holders: &[Holder], name_of: impl Fn(&str) -> String) -> Vec<Row> {
    let mut rows: Vec<Row> = programs
        .iter()
        .map(|p| Row {
            name: p.name.clone(),
            reason: String::new(),
        })
        .collect();
    for h in holders {
        let ours = |p: &Program| {
            (h.pid.is_some() && p.pid == h.pid) || (!h.who.is_empty() && p.app_id == h.who)
        };
        let window = (!h.shutdown_only)
            .then(|| programs.iter().position(ours))
            .flatten();
        match window {
            Some(i) if rows[i].reason.is_empty() || rows[i].reason == h.why => {
                rows[i].reason = h.why.clone();
            }
            _ => {
                let row = Row {
                    name: if h.shutdown_only {
                        h.who.clone()
                    } else {
                        name_of(&h.who)
                    },
                    reason: h.why.clone(),
                };
                // A program may hold the session twice for one reason.
                if !rows.contains(&row) {
                    rows.push(row);
                }
            }
        }
    }
    rows
}

fn is_prompt(d: &crate::dialog::Dialog) -> bool {
    matches!(&d.style, crate::dialog::Style::Form(f) if f.kind == crate::dialog::Kind::EndSession)
}

fn prompt_index(state: &State) -> Option<usize> {
    (state.dialogs.views.iter()).position(|v| is_prompt(&v.dialog))
}

/// Whether the session end waits on its prompt.
pub(crate) fn prompting(state: &State) -> bool {
    (state.ending.as_ref()).is_some_and(|e| e.mode == Mode::Prompted)
}

fn sync_prompt(state: &mut State) {
    let Some(e) = state.ending.as_ref().filter(|e| e.mode == Mode::Prompted) else {
        return;
    };
    // A prompt waiting under a Welcome panel takes its rows once it opens.
    if state.dialogs.held.iter().any(is_prompt) {
        return;
    }
    // No box opens without an output, so the prompt is built once one arrives.
    if state.scene.outputs().is_empty() {
        return;
    }
    let goal = e.goal;
    let holders = holding(e.holders.as_deref().unwrap_or(&[]), goal);
    let programs = programs(state);
    let apps = &state.app_index;
    let name_of = |app_id: &str| {
        if app_id.is_empty() {
            return "A program".to_string();
        }
        reprisefs::apps::AppId::new(app_id)
            .and_then(|a| reprisefs::apps::entry_for_app_id(apps, &a, |_| true))
            .map(|e| e.name.clone())
            .unwrap_or_else(|| app_id.to_string())
    };
    let mut fields = endsession::Fields {
        goal,
        rows: rows(&programs, &holders, name_of),
        icon: None,
    };
    if crate::screenrecord::finishing(state) {
        fields.rows.push(Row {
            name: "Screen recording".into(),
            reason: "Finishing the screen recording".into(),
        });
    }
    match prompt_index(state) {
        Some(i) if state.dialogs.views[i].dialog.title == fields.title() => {
            let changed = state.dialogs.views[i]
                .dialog
                .end_session_form()
                .is_some_and(|f| endsession::set_rows(f, &fields.rows));
            if changed {
                state.redraw_dialog(i);
            }
        }
        found => {
            if let Some(i) = found {
                state.close_dialog_at(i);
            }
            log::info!("session end: prompting for {} rows", fields.rows.len());
            let waits = state.icons.waits();
            fields.icon = warning_icon(state);
            let waited = state.icons.waits() != waits;
            let t = &state.config.theme;
            let d =
                crate::dialog::Dialog::end_session(&fields, &state.text, t.fonts.size_px, t.text);
            state.open_dialog(d);
            if waited {
                state.wait_for_icons(crate::icon::IconWaiter::SessionEnd);
            }
        }
    }
}

/// The prompt's picture ([`crate::icon::unread_first`]): the warning sign, else the question mark.
fn warning_icon(state: &mut State) -> Option<crate::raster::Canvas> {
    let names = ["dialog-warning", "dialog-question"];
    crate::icon::unread_first(&mut state.icons, names, 32).map(Arc::unwrap_or_clone)
}

/// Icons landed while the prompt showed a stand-in for its picture: it looks it up again.
pub(crate) fn icons_landed(state: &mut State) {
    let Some(i) = prompt_index(state) else {
        return;
    };
    let waits = state.icons.waits();
    let icon = warning_icon(state);
    state.note_icon_waits(waits, crate::icon::IconWaiter::SessionEnd);
    let form = state.dialogs.views[i].dialog.form_mut();
    if let (Some(f), Some(icon)) = (form, icon)
        && f.set_icon(icon)
    {
        state.redraw_dialog(i);
    }
}

/// Whether a session end asks the portal, moving its monitors to query-end: a Log Off does only
/// with no other Reprise session of the user beside it, which shares the portal.
fn asks_portal(
    env: &crate::spawn::SessionEnv,
    goal: Goal,
    another_runs: impl FnOnce() -> bool,
) -> bool {
    goal != Goal::LogOff || (!env.defers_user_units && !another_runs())
}

/// Polls the inhibitors, and the portal's with `ask_portal`.
fn spawn_poll(
    tx: channel::Sender<Vec<Holder>>,
    ask_portal: bool,
) -> Option<async_channel::Sender<()>> {
    let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
    let thread = std::thread::Builder::new()
        .name("session-end".into())
        .spawn(move || zbus::block_on(poll(tx, stop_rx, ask_portal)));
    match thread {
        Ok(_) => Some(stop_tx),
        Err(e) => {
            log::warn!("session end: no holder thread: {e}");
            None
        }
    }
}

async fn connect(system: bool) -> Option<zbus::Connection> {
    let b = if system {
        zbus::connection::Builder::system()
    } else {
        zbus::connection::Builder::session()
    };
    b.method_timeout(CALL_TIMEOUT)
        .build()
        .await
        .map_err(|e| log::warn!("session end: bus: {e}"))
        .ok()
}

async fn poll(
    tx: channel::Sender<Vec<Holder>>,
    stop: async_channel::Receiver<()>,
    ask_portal: bool,
) {
    let session = if ask_portal {
        connect(false).await
    } else {
        None
    };
    let system = connect(true).await;
    loop {
        let mut held = match &session {
            Some(c) => portal_holders(c, true).await,
            None => Vec::new(),
        };
        if let Some(c) = &system {
            held.extend(logind_holders(c, Verb::PowerOff.inhibited_by()).await);
        }
        if tx.send(held).is_err() {
            return;
        }
        let stopped = futures_lite::future::or(async { Some(stop.recv().await.is_ok()) }, async {
            async_io::Timer::after(POLL).await;
            None
        })
        .await;
        match stopped {
            None => {}
            // Cancelled: the portal's monitors go back to running.
            Some(true) => {
                if let Some(c) = &session {
                    portal_holders(c, false).await;
                }
                return;
            }
            Some(false) => return,
        }
    }
}

async fn portal_holders(conn: &zbus::Connection, ending: bool) -> Vec<Holder> {
    let has_owner = zbus::fdo::DBusProxy::new(conn)
        .await
        .ok()
        .map(|p| async move {
            let name = zbus::names::BusName::try_from(PORTAL).ok()?;
            p.name_has_owner(name).await.ok()
        });
    let running = match has_owner {
        Some(f) => f.await.unwrap_or(false),
        None => false,
    };
    if !running {
        return Vec::new();
    }
    let reply = conn
        .call_method(
            Some(PORTAL),
            PORTAL_PATH,
            Some(SESSION_END),
            "Query",
            &(ending,),
        )
        .await
        .and_then(|m| m.body().deserialize::<Vec<(String, String, u32)>>());
    match reply {
        Ok(v) => v
            .into_iter()
            .map(|(who, why, pid)| Holder {
                who,
                why,
                shutdown_only: false,
                pid: i32::try_from(pid).ok().filter(|&p| p > 0),
            })
            .collect(),
        Err(e) => {
            log::warn!("session end: portal: {e}");
            Vec::new()
        }
    }
}

async fn logind_holders(conn: &zbus::Connection, blocked: &str) -> Vec<Holder> {
    type Inhibitor = (String, String, String, String, u32, u32);
    let reply = conn
        .call_method(
            Some(LOGIND),
            LOGIND_PATH,
            Some(LOGIND_MANAGER),
            "ListInhibitors",
            &(),
        )
        .await
        .and_then(|m| m.body().deserialize::<Vec<Inhibitor>>());
    match reply {
        Ok(v) => blocks(
            v.into_iter()
                .map(|(what, who, why, mode, _, _)| (what, who, why, mode)),
            blocked,
        ),
        Err(e) => {
            log::warn!("session end: logind: {e}");
            Vec::new()
        }
    }
}

/// The block inhibitors in `list` (what, who, why, mode) that hold `blocked`.
fn blocks(
    list: impl Iterator<Item = (String, String, String, String)>,
    blocked: &str,
) -> Vec<Holder> {
    list.filter(|(what, _, _, mode)| mode == "block" && what.split(':').any(|w| w == blocked))
        .map(|(_, who, why, _)| Holder {
            who,
            why,
            shutdown_only: true,
            pid: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A harness with icons from `root` and the session-end prompt open.
    fn prompted(root: &std::path::Path) -> crate::harness::Harness {
        let mut h = crate::icon::two_outputs("classic", root);
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            // Unknown holders keep the end from finishing while the loop turns.
            holders: None,
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: Vec::new(),
        });
        sync_prompt(&mut h.state);
        h
    }

    /// The open prompt's picture.
    fn picture(h: &crate::harness::Harness) -> Option<crate::raster::Canvas> {
        use reprisekit::controls::form::Control;
        let i = prompt_index(&h.state).expect("the prompt");
        let page = h.state.dialogs.views[i].dialog.form()?.open_page();
        match &page.item("icon")?.control {
            Control::Picture { canvas, .. } => canvas.clone(),
            _ => None,
        }
    }

    #[test]
    fn a_restart_into_sets_its_target_first_and_clears_it_when_the_reboot_fails() {
        let Some(f) = crate::restartinto::tests::logind("yes", "yes", &[]) else {
            return;
        };
        let target = BootTarget {
            entry: Some("b.conf".into()),
            title: "B".into(),
            password: false,
        };
        let mut rebooted = false;
        let failed = restart_into(
            &f.client,
            &target,
            false,
            || true,
            || {
                rebooted = true;
                Some(Failed::Refused(Refused::Unsupported))
            },
        );
        assert!(rebooted);
        assert_eq!(failed, Some(Failed::Refused(Refused::Unsupported)));
        assert_eq!(*f.set.lock().unwrap(), ["entry \"b.conf\"", "entry \"\""]);
        f.set.lock().unwrap().clear();
        assert_eq!(
            restart_into(&f.client, &target, false, || true, || None),
            None
        );
        assert_eq!(
            *f.set.lock().unwrap(),
            ["entry \"b.conf\""],
            "a reboot under way keeps it"
        );
    }

    #[test]
    fn a_failed_reboot_clears_its_target_with_the_password_prompt_the_set_had() {
        let Some(f) = crate::restartinto::tests::logind_asking_to_clear() else {
            return;
        };
        let target = BootTarget {
            entry: Some("b.conf".into()),
            title: "B".into(),
            password: false,
        };
        let unsupported = || Some(Failed::Refused(Refused::Unsupported));
        assert_eq!(
            restart_into(&f.client, &target, true, || true, unsupported),
            Some(Failed::Refused(Refused::Unsupported))
        );
        assert_eq!(*f.set.lock().unwrap(), ["entry \"b.conf\"", "entry \"\""]);
        let kept = restart_into(&f.client, &target, false, || true, unsupported);
        let Some(Failed::Refused(why)) = kept else {
            panic!("a refusal, not {kept:?}");
        };
        assert_eq!(
            refusal_text(Act::Restart, &why),
            "Reprise could not restart the computer.\n\nThis computer does not support it.\n\n\
             The next restart still starts B."
        );
        let dismissed = restart_into(
            &f.client,
            &target,
            false,
            || true,
            || Some(Failed::Dismissed),
        );
        assert_eq!(
            dismissed,
            Some(Failed::Refused(Refused::Aimed(None, "B".into()))),
            "a dismissed prompt still says the target stays"
        );
    }

    fn target_b() -> BootTarget {
        BootTarget {
            entry: Some("b.conf".into()),
            title: "B".into(),
            password: false,
        }
    }

    fn dialogs(h: &crate::harness::Harness) -> usize {
        h.state.dialogs.views.len()
    }

    #[test]
    fn no_other_power_action_starts_while_one_is_under_way() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let id = h.state.powering.begin(None);
        begin_into(&mut h.state, "true".into(), target_b());
        begin(
            &mut h.state,
            Goal::Restart,
            Some("true".into()),
            Origin::Desktop,
        );
        assert!(h.state.ending.is_none());
        sleep(&mut h.state, Act::StandBy, "true".into());
        assert!(h.state.powering.busy(), "Stand By did not run and answer");
        answered(
            &mut h.state,
            id,
            Act::Restart,
            Some(Failed::Dismissed),
            None,
        );
        assert!(!h.state.powering.busy());
        begin(
            &mut h.state,
            Goal::Restart,
            Some("true".into()),
            Origin::Desktop,
        );
        assert!(h.state.ending.is_some(), "a Restart once it answered");
    }

    #[test]
    fn log_off_gives_up_the_power_action_under_way() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let id = h.state.powering.begin(Some(target_b()));
        begin(&mut h.state, Goal::LogOff, None, Origin::Desktop);
        assert!(
            h.state
                .ending
                .as_ref()
                .is_some_and(|e| e.goal == Goal::LogOff)
        );
        assert!(!h.state.powering.busy());
        assert_eq!(h.state.powering.stale, [(id, target_b())]);
        let open = dialogs(&h);
        answered(
            &mut h.state,
            id,
            Act::StandBy,
            Some(Failed::Refused(Refused::Unsupported)),
            None,
        );
        assert_eq!(dialogs(&h), open, "the given-up Stand By refuses nothing");
        assert!(h.state.ending.is_some());
    }

    #[test]
    fn a_critical_battery_gives_up_the_power_action_under_way() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let id = h.state.powering.begin(Some(target_b()));
        begin_critical(&mut h.state, "true".into());
        assert!(
            h.state
                .ending
                .as_ref()
                .is_some_and(|e| e.mode == Mode::Critical)
        );
        assert_eq!(
            h.state.powering.stale,
            [(id, target_b())],
            "cleared before it powers off"
        );
        let open = dialogs(&h);
        answered(
            &mut h.state,
            id,
            Act::Restart,
            Some(Failed::Refused(Refused::Unsupported)),
            None,
        );
        assert_eq!(dialogs(&h), open);
        let critical = h.state.powering.begin(None);
        answered(&mut h.state, id, Act::Restart, None, None);
        assert!(
            h.state.powering.busy(),
            "the old answer leaves the critical attempt {critical}"
        );
    }

    #[test]
    fn a_power_action_with_no_answer_in_time_is_refused_and_given_up() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let id = h.state.powering.begin(Some(target_b()));
        let open = dialogs(&h);
        crate::crash::ending();
        h.state.session.kept = true;
        late(&mut h.state, id, Act::Restart, false);
        assert_eq!(dialogs(&h), open + 1);
        assert!(
            crate::crash::is_ending() && h.state.session.kept,
            "a restart that goes through late restores the saved session"
        );
        crate::crash::not_ending();
        assert!(!h.state.powering.busy());
        assert_eq!(h.state.powering.stale, [(id, target_b())]);
        answered(
            &mut h.state,
            id,
            Act::Restart,
            Some(Failed::Refused(Refused::Unsupported)),
            None,
        );
        assert_eq!(dialogs(&h), open + 1, "the late answer shows nothing");
        let next = h.state.powering.begin(None);
        late(&mut h.state, id, Act::Restart, false);
        assert!(
            h.state.powering.busy(),
            "the old deadline leaves attempt {next}"
        );
        assert_eq!(
            reason(Act::Restart, &Refused::Late),
            "It had not happened after 2 minutes."
        );
    }

    #[test]
    fn a_custom_sleep_command_past_the_deadline_holds_nothing_back_and_still_reports() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let id = h.state.powering.begin(None);
        let open = dialogs(&h);
        late(&mut h.state, id, Act::StandBy, true);
        assert_eq!(dialogs(&h), open, "no box at the deadline");
        assert!(!h.state.powering.busy());
        let next = h.state.powering.begin(None);
        let failed = Some(Failed::Refused(Refused::Failed("no locker".into())));
        answered(&mut h.state, id, Act::StandBy, failed.clone(), None);
        assert_eq!(dialogs(&h), open + 1, "a failure after it still shows");
        assert!(h.state.powering.busy(), "attempt {next} is under way");
        late(&mut h.state, next, Act::StandBy, true);
        begin(&mut h.state, Goal::LogOff, None, Origin::Desktop);
        let shown = dialogs(&h);
        answered(&mut h.state, next, Act::StandBy, failed, None);
        assert_eq!(dialogs(&h), shown, "Log Off gave it up");
    }

    #[test]
    fn a_panicking_power_action_still_answers() {
        assert_eq!(
            unpanicked(|| panic!("a bug")),
            Some(Failed::Refused(Refused::Internal))
        );
        assert_eq!(unpanicked(|| None), None);
    }

    #[test]
    fn a_new_attempt_gives_up_the_one_under_way() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let first = h.state.powering.begin(Some(target_b()));
        let second = h.state.powering.begin(None);
        assert_eq!(h.state.powering.stale, [(first, target_b())]);
        let open = dialogs(&h);
        answered(
            &mut h.state,
            first,
            Act::Restart,
            Some(Failed::Refused(Refused::Unsupported)),
            None,
        );
        assert_eq!(dialogs(&h), open, "the first answers nothing");
        assert!(h.state.powering.busy(), "attempt {second} is under way");
    }

    #[test]
    fn a_target_a_panic_or_a_failed_clear_left_set_is_cleared_before_the_next_action() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let refusals = [
            (Refused::Unsupported, false),
            (Refused::Internal, true),
            (Refused::Aimed(None, "B".into()), true),
        ];
        for (why, stale) in refusals {
            h.state.powering.stale.clear();
            let id = h.state.powering.begin(Some(target_b()));
            answered(
                &mut h.state,
                id,
                Act::Restart,
                Some(Failed::Refused(why.clone())),
                None,
            );
            let want: &[(u64, BootTarget)] = if stale { &[(id, target_b())] } else { &[] };
            assert_eq!(h.state.powering.stale, want, "after {why:?}");
        }
    }

    #[test]
    fn a_restart_into_given_up_on_before_its_reboot_clears_its_target() {
        let Some(f) = crate::restartinto::tests::logind("yes", "yes", &[]) else {
            return;
        };
        let failed = restart_into(
            &f.client,
            &target_b(),
            false,
            || false,
            || panic!("no reboot"),
        );
        assert_eq!(failed, Some(Failed::Dismissed));
        assert_eq!(*f.set.lock().unwrap(), ["entry \"b.conf\"", "entry \"\""]);
    }

    #[test]
    fn stale_restart_targets_clear_without_a_password_prompt() {
        let Some(f) = crate::restartinto::tests::logind("yes", "yes", &[]) else {
            return;
        };
        let firmware = BootTarget::firmware(false);
        zbus::block_on(clear(&f.client, &[(1, target_b()), (2, firmware.clone())]));
        assert_eq!(*f.set.lock().unwrap(), ["entry \"\"", "firmware false"]);
        let Some(f) = crate::restartinto::tests::logind_asking_to_clear() else {
            return;
        };
        zbus::block_on(clear(&f.client, &[(1, target_b()), (2, firmware)]));
        assert_eq!(
            *f.set.lock().unwrap(),
            ["firmware false"],
            "a refused clear goes on to the next"
        );
    }

    #[test]
    fn a_refused_restart_target_reboots_nothing() {
        let Some(f) = crate::restartinto::tests::logind("no", "no", &[]) else {
            return;
        };
        let target = BootTarget {
            entry: Some("b.conf".into()),
            title: "B".into(),
            password: false,
        };
        let failed = restart_into(&f.client, &target, false, || true, || panic!("no reboot"));
        assert_eq!(failed, Some(Failed::Refused(Refused::NotAllowed)));
        assert!(f.set.lock().unwrap().is_empty());
    }

    #[test]
    fn a_restart_target_is_asked_about_again_before_anything_closes() {
        let Some(f) = crate::restartinto::tests::logind("challenge", "na", &["a.conf"]) else {
            return;
        };
        let entry = |id: &str| BootTarget {
            entry: Some(id.into()),
            title: id.to_uppercase(),
            password: false,
        };
        let now = zbus::block_on(target_now(&f.client, entry("a.conf")));
        assert_eq!(
            now.map(|b| b.password),
            Ok(true),
            "the password logind asks for now"
        );
        assert_eq!(
            zbus::block_on(target_now(&f.client, entry("b.conf"))),
            Err(Refused::Failed(
                "\"B.CONF\" is no longer a boot entry.".into()
            ))
        );
        let firmware = BootTarget {
            entry: None,
            title: "Firmware Setup".into(),
            password: false,
        };
        assert_eq!(
            zbus::block_on(target_now(&f.client, firmware)),
            Err(Refused::Unsupported)
        );
        assert!(f.set.lock().unwrap().is_empty());
    }

    #[test]
    fn a_guarded_restart_target_asks_for_the_password_up_front() {
        let guarded = BootTarget {
            entry: None,
            title: "Firmware Setup".into(),
            password: true,
        };
        assert_eq!(Password::No.with(Some(&guarded)), Password::Yes);
        assert_eq!(
            Password::OverInhibitors.with(None),
            Password::OverInhibitors
        );
        let open = BootTarget {
            password: false,
            ..guarded
        };
        assert_eq!(Password::No.with(Some(&open)), Password::No);
    }

    #[test]
    fn a_cold_warning_icon_lands_on_the_open_prompt() {
        let root = crate::icon::svg_theme(&["dialog-warning"], &[]);
        let mut h = prompted(&root);
        let blank = picture(&h).expect("a stand-in in the picture's place");
        assert!(
            blank.pixels.iter().all(|&p| p == 0),
            "the stand-in is blank"
        );
        crate::icon::land(&mut h);
        assert!(
            picture(&h).is_some_and(|c| crate::icon::red(&c)),
            "the prompt took the icon"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_prompt_waits_once_under_a_welcome_panel_and_opens_after_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let leave = reprisekit::powerpanel::Leave::SwitchUser { live: false };
        h.state.open_dialog(crate::dialog::Dialog::log_off(leave));
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: None,
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: Vec::new(),
        });
        sync_prompt(&mut h.state);
        sync_prompt(&mut h.state);
        assert_eq!(prompt_index(&h.state), None, "not over the panel");
        assert!(h.state.dialog_is_modal());
        assert_eq!(h.state.dialogs.held.len(), 1, "held once");
        h.state.close_dialog_at(0);
        assert!(prompt_index(&h.state).is_some(), "up once the panel closes");
    }

    #[test]
    fn with_no_output_the_prompt_is_built_once_one_arrives() {
        use reprisekit::icontheme::{IconCache, IconTheme};
        let root = crate::icon::svg_theme(&["dialog-warning"], &[]);
        let mut h = crate::harness::Harness::new();
        crate::fsjobs::setup(&mut h.state);
        h.state.icons = IconCache::new(IconTheme::load("test", std::slice::from_ref(&root)));
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: None,
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: Vec::new(),
        });
        for _ in 0..3 {
            tick(&mut h.state);
        }
        assert!(
            !crate::icon::waits_on(&h, crate::icon::IconWaiter::SessionEnd),
            "no prompt is built while no output can show it"
        );
        h.output("TEST-1", 1024, 768);
        tick(&mut h.state);
        assert!(
            prompt_index(&h.state).is_some(),
            "up once an output arrives"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_close_box_on_the_prompt_cancels_the_session_end() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: None,
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: Vec::new(),
        });
        sync_prompt(&mut h.state);
        let i = prompt_index(&h.state).expect("the prompt");
        h.click_dialog_close(i);
        assert!(h.state.ending.is_none(), "the session end is cancelled");
        tick(&mut h.state);
        assert_eq!(prompt_index(&h.state), None, "and the prompt stays closed");
    }

    #[test]
    fn a_warning_icon_that_never_decodes_gives_way_to_the_question_mark() {
        let root = crate::icon::svg_theme(&["dialog-question"], &["dialog-warning"]);
        let mut h = prompted(&root);
        crate::icon::land(&mut h);
        assert!(
            picture(&h).is_some_and(|c| crate::icon::red(&c)),
            "the question mark shows once the warning sign fails"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn win(id: u32, pid: Option<i32>, app: &str, title: &str, owner: Option<u32>) -> Shown<u32> {
        Shown {
            id,
            pid,
            app_id: app.into(),
            title: title.into(),
            owner,
        }
    }

    fn holder(who: &str, why: &str, shutdown_only: bool) -> Holder {
        Holder {
            who: who.into(),
            why: why.into(),
            shutdown_only,
            pid: None,
        }
    }

    #[test]
    fn windows_group_into_programs_by_process() {
        let got = group(vec![
            win(1, Some(10), "reprise-notepad", "Notepad", Some(2)),
            win(2, Some(10), "reprise-notepad", "Untitled - Notepad", None),
            win(3, Some(11), "foot", "", None),
            win(4, None, "x", "No pid", None),
            win(5, None, "x", "No pid either", None),
            win(6, Some(10), "reprise-notepad", "notes.txt - Notepad", None),
        ]);
        let names: Vec<&str> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Untitled - Notepad", "foot", "No pid", "No pid either"]
        );
        assert_eq!(got[0].pid, Some(10));
    }

    #[test]
    fn a_window_without_a_pid_joins_its_owners_program() {
        let got = group(vec![
            win(1, None, "", "Discard the unsaved text?", Some(2)),
            win(2, None, "", "Unsaved note", None),
            win(3, None, "", "Options", Some(1)),
            win(4, None, "", "Unsaved note", None),
        ]);
        let names: Vec<&str> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Unsaved note", "Unsaved note"]);
        assert_eq!(got[0].pid, None);
    }

    #[test]
    fn holders_join_their_programs_rows() {
        let programs = [Program {
            pid: Some(4),
            app_id: "org.gnome.TextEditor".into(),
            name: "Untitled Document 1 - Text Editor".into(),
        }];
        let holders = [
            holder("org.gnome.TextEditor", "Unsaved documents", false),
            holder("org.gnome.TextEditor", "Unsaved documents", false),
            holder("", "Burning a disc", false),
            holder("", "Burning a disc", false),
            holder("packagekit", "Installing updates", true),
        ];
        let got = rows(&programs, &holders, |id| {
            if id.is_empty() {
                "A program".into()
            } else {
                id.into()
            }
        });
        let pairs: Vec<(&str, &str)> = got
            .iter()
            .map(|r| (r.name.as_str(), r.reason.as_str()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("Untitled Document 1 - Text Editor", "Unsaved documents"),
                ("A program", "Burning a disc"),
                ("packagekit", "Installing updates"),
            ]
        );
    }

    #[test]
    fn a_finishing_screen_recording_holds_the_end_and_is_listed() {
        use crate::screenrecord::{Recording, Target};
        let mut h = crate::harness::Harness::new();
        let output = h.output("TEST-1", 1024, 768);
        h.state.recording = Some(Recording::stopped(Target { output, area: None }));
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: Some(Vec::new()),
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: Vec::new(),
        });
        tick(&mut h.state);
        assert!(h.state.ending.is_some(), "held");
        let i = prompt_index(&h.state).expect("the prompt");
        let form = h.state.dialogs.views[i].dialog.end_session_form().unwrap();
        let row = Row {
            name: "Screen recording".into(),
            reason: "Finishing the screen recording".into(),
        };
        assert!(!endsession::set_rows(form, &[row]), "its one row");
        h.state.recording = None;
        tick(&mut h.state);
        assert!(h.state.ending.is_none(), "it ends once the file is closed");
    }

    #[test]
    fn a_window_whose_process_holds_off_log_off_is_asked_to_close_once_it_lets_go() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        h.map_titled(&mut c, "reprise.explorer", "Copying...", (300, 200));
        let w = h.state.windows[0].id;
        let copying = Holder {
            who: String::new(),
            why: "Copying files".into(),
            shutdown_only: false,
            pid: h.state.client_pid(w),
        };
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: Some(vec![copying]),
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
            password: Password::No,
            unasked: vec![w],
        });
        ask_windows(&mut h.state);
        tick(&mut h.state);
        h.roundtrip(&mut c);
        assert!(c.side.closes.is_empty(), "the copy's window stays open");
        let i = prompt_index(&h.state).expect("the prompt");
        let form = h.state.dialogs.views[i].dialog.end_session_form().unwrap();
        let row = Row {
            name: "Copying...".into(),
            reason: "Copying files".into(),
        };
        assert!(
            !endsession::set_rows(form, &[row]),
            "one row for the window and its reason"
        );
        h.state.ending.as_mut().unwrap().holders = Some(Vec::new());
        ask_windows(&mut h.state);
        ask_windows(&mut h.state);
        h.roundtrip(&mut c);
        assert_eq!(c.side.closes, [0], "asked once, after the copy let go");
    }

    #[test]
    fn a_modal_box_over_its_programs_window_is_not_asked_to_close() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        let mut c = h.connect();
        let notepad = h.map_toplevel(&mut c, "reprise.notepad", 400, 300);
        let save = h.map_modal(&mut c, "reprise.notepad", notepad, (300, 120));
        let find = h.map_dialog(&mut c, "reprise.notepad", Some(notepad), "", (200, 100));
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            boot: None,
            holders: Some(Vec::new()),
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Waiting,
            password: Password::No,
            unasked: vec![notepad.id, save.id, find.id],
        });
        ask_windows(&mut h.state);
        h.roundtrip(&mut c);
        assert_eq!(
            c.side.closes,
            [notepad.shell, find.shell],
            "the window under the box and its other window, not the box"
        );
    }

    #[test]
    fn a_log_off_ends_the_session_for_the_crash_hook_before_its_fade() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers.finger = |_| None;
        crate::lockview::lock(&mut h.state);
        assert!(!crate::crash::is_ending());
        begin(&mut h.state, Goal::LogOff, None, Origin::Desktop);
        assert!(h.state.session_fade.fading_out(), "the fade runs");
        assert!(
            crate::crash::is_ending(),
            "a panic in the fade ends the process"
        );
    }

    #[test]
    fn a_committed_log_off_with_a_client_tells_it_the_session_logged_off() {
        let (mut h, _portal, _lock) = crate::console::tests::in_lock_mode();
        go_on(&mut h.state, Goal::LogOff, None, None, false, Password::No);
        assert_eq!(
            h.state.ended_clients,
            [crate::console::ClientsEnd::LoggedOff]
        );

        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        go_on(&mut h.state, Goal::LogOff, None, None, false, Password::No);
        assert!(h.state.ended_clients.is_empty(), "no client to tell");
        assert!(h.state.session_fade.fading_out(), "the fade runs");
    }

    #[test]
    fn a_log_off_with_the_remote_desktop_server_running_tells_a_view_only_client() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::standby::rdp_changed(&mut h.state, true);
        assert!(!h.state.remote_controlled());
        go_on(&mut h.state, Goal::LogOff, None, None, false, Password::No);
        assert_eq!(
            h.state.ended_clients,
            [crate::console::ClientsEnd::LoggedOff]
        );
    }

    #[test]
    fn a_log_off_beside_another_reprise_session_leaves_the_remote_desktop_server_to_it() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.env.defers_user_units = true;
        crate::standby::rdp_changed(&mut h.state, true);
        assert!(
            !h.state.standby.rdp_on(),
            "the server is the other session's"
        );
        go_on(&mut h.state, Goal::LogOff, None, None, false, Password::No);
        assert!(h.state.ended_clients.is_empty(), "its clients stay");
    }

    #[test]
    fn a_log_off_beside_another_reprise_session_leaves_the_shared_portal_running() {
        let alone = crate::spawn::SessionEnv::default();
        let log_off = Goal::LogOff;
        assert!(
            asks_portal(&alone, log_off, || false),
            "the only session asks"
        );
        assert!(
            !asks_portal(&alone, log_off, || true),
            "a session another one started beside leaves it"
        );
        let deferring = crate::spawn::SessionEnv {
            defers_user_units: true,
            ..alone
        };
        assert!(
            !asks_portal(&deferring, log_off, || true),
            "a deferring one leaves it"
        );
        for goal in [Goal::TurnOff, Goal::Restart] {
            assert!(
                asks_portal(&deferring, goal, || true),
                "{goal:?} ends them all"
            );
        }
    }

    #[test]
    fn a_log_off_hands_the_portal_back_to_a_session_started_during_its_prompt() {
        let env = crate::spawn::SessionEnv::default();
        let ending = |goal, cmd: Option<&str>, asked_portal| Ending {
            goal,
            cmd: cmd.map(String::from),
            boot: None,
            holders: None,
            stop: None,
            asked_portal,
            timer: None,
            reports: None,
            mode: Mode::Waiting,
            password: Password::No,
            unasked: Vec::new(),
        };
        let asked = ending(Goal::LogOff, None, true);
        assert!(
            resets_portal(&asked, &env, || true),
            "another session runs by now"
        );
        assert!(
            !resets_portal(&asked, &env, || false),
            "the portal ends with it"
        );
        let unasked = ending(Goal::LogOff, None, false);
        assert!(!resets_portal(&unasked, &env, || true), "it never moved");
        let turn_off = ending(Goal::TurnOff, Some("true"), true);
        assert!(
            resets_portal(&turn_off, &env, || false),
            "a command keeps the compositor"
        );
    }

    #[test]
    fn a_turn_off_beside_another_reprise_session_tells_its_remote_clients() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.env.defers_user_units = true;
        crate::standby::rdp_changed(&mut h.state, true);
        go_on(
            &mut h.state,
            Goal::TurnOff,
            Some("true".into()),
            None,
            false,
            Password::No,
        );
        assert_eq!(
            h.state.ended_clients,
            [crate::console::ClientsEnd::PoweredOff]
        );
    }

    #[test]
    fn the_end_runs_once_after_both_the_fade_and_the_answer() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut h = crate::harness::Harness::new();
        let faded: fn(&mut Wait) = |w| w.faded = true;
        let told: fn(&mut Wait) = |w| w.told = true;
        for (first, second) in [(faded, told), (told, faded)] {
            let ends = Arc::new(AtomicUsize::new(0));
            let counted = Arc::clone(&ends);
            let wait = Mutex::new(Wait {
                faded: false,
                told: false,
                end: Some(Box::new(move |_| {
                    counted.fetch_add(1, Ordering::Relaxed);
                })),
            });
            arrive(&wait, &mut h.state, first);
            assert_eq!(ends.load(Ordering::Relaxed), 0, "one of the two arrived");
            arrive(&wait, &mut h.state, second);
            assert_eq!(ends.load(Ordering::Relaxed), 1, "both arrived");
            arrive(&wait, &mut h.state, told);
            assert_eq!(
                ends.load(Ordering::Relaxed),
                1,
                "the timer's late answer ends nothing"
            );
        }
    }

    #[test]
    fn the_end_saves_screenshots_with_the_desktop_up_and_fades_after() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.config.effects.session_fade = true;
        h.state.config.effects.session_fade_out_ms = 400;
        let saving = h.state.shots.saves.hold();
        let ends = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&ends);
        let end: OnLoop = Box::new(move |_| {
            counted.fetch_add(1, Ordering::Relaxed);
        });
        fade_out(&mut h.state, true, SAVE_SHOTS, end);
        h.run_for(300);
        assert!(
            !h.state.session_fade.fading_out(),
            "no fade while the file is under way"
        );
        let notice = h.state.notify.log.get(crate::notifyview::OWN_ID);
        assert_eq!(
            notice.map(|e| e.n.summary.as_str()),
            Some("Saving screenshots...")
        );
        drop(saving);
        h.run_for(200);
        assert!(h.state.session_fade.fading_out(), "the file landed");
        assert_eq!(ends.load(Ordering::Relaxed), 0, "the fade is under way");
        h.run_for(400);
        assert_eq!(ends.load(Ordering::Relaxed), 1, "faded out");
    }

    #[test]
    fn a_refused_turn_off_lets_a_later_panic_restart_the_session() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::crash::ending();
        h.state.session.kept = true;
        refuse(&mut h.state, Act::TurnOff, &Refused::NotAllowed);
        assert!(!crate::crash::is_ending());
        assert!(!h.state.session.kept, "the session saves resume");
    }

    fn locked() -> crate::harness::Harness {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.lock.helpers.finger = |_| None;
        crate::lockview::lock(&mut h.state);
        h
    }

    #[test]
    fn the_lock_names_no_program_holding_off_a_turn_off() {
        let mut h = locked();
        let held = Refused::Held(vec![holder("backup", "Copying the photos", true)]);
        refuse(&mut h.state, Act::TurnOff, &held);
        let l = h.state.lock.console.as_ref().unwrap();
        let shown = l.message().unwrap_or_default();
        assert!(shown.contains("A program is preventing it."), "{shown}");
        assert!(!shown.contains("backup"), "{shown}");
    }

    #[test]
    fn the_lock_quotes_no_failed_commands_error() {
        let mut h = locked();
        let failed = Refused::Failed("\"my-poweroff\" failed: /home/anna/.secret: denied".into());
        refuse(&mut h.state, Act::TurnOff, &failed);
        let l = h.state.lock.console.as_ref().unwrap();
        let shown = l.message().unwrap_or_default();
        assert!(shown.contains(LOCK_FAILED), "{shown}");
        assert!(!shown.contains("my-poweroff"), "{shown}");
    }

    #[test]
    fn a_locked_session_asks_logind_without_a_password_prompt() {
        let mut h = locked();
        assert!(!prompts(&h.state));
        crate::lockview::unlock(&mut h.state);
        assert!(prompts(&h.state));
    }

    #[test]
    fn shutdown_inhibitors_hold_only_a_shutdown() {
        let h = [holder("a", "", false), holder("b", "", true)];
        assert_eq!(holding(&h, Goal::LogOff).len(), 1);
        assert_eq!(holding(&h, Goal::TurnOff).len(), 2);
        assert_eq!(holding(&h, Goal::Restart).len(), 2);
    }

    #[test]
    fn a_critical_end_forces_without_the_prompt() {
        let m = Mode::new(true);
        assert_eq!(m.waited(), (Mode::Critical, Step::Force));
        assert_eq!(m.again(false), (Mode::Critical, Step::Wait));
        assert_eq!(m.again(true), (Mode::Critical, Step::Wait));
        assert_eq!(Mode::Waiting.again(true), (Mode::Critical, Step::Wait));
        assert_eq!(Mode::Prompted.again(true), (Mode::Critical, Step::Force));

        let m = Mode::new(false);
        assert_eq!(m.waited(), (Mode::Prompted, Step::Prompt));
        assert_eq!(m.again(false), (Mode::Prompted, Step::Prompt));
    }

    #[test]
    fn only_blocks_of_the_action_are_listed() {
        let s = |a: &str, b: &str, c: &str, d: &str| (a.into(), b.into(), c.into(), d.into());
        let list = || {
            vec![
                s("shutdown:sleep", "packagekit", "Installing", "block"),
                s("shutdown", "NetworkManager", "Saving", "delay"),
                s("sleep", "portal", "Playing", "block"),
                s("handle-power-key:handle-lid-switch", "reprise", "", "block"),
            ]
            .into_iter()
        };
        assert_eq!(
            blocks(list(), Verb::PowerOff.inhibited_by()),
            [holder("packagekit", "Installing", true)]
        );
        assert_eq!(
            blocks(list(), Verb::Suspend.inhibited_by()),
            [
                holder("packagekit", "Installing", true),
                holder("portal", "Playing", true)
            ]
        );
    }

    #[test]
    fn only_a_bare_systemctl_or_loginctl_command_is_called_on_logind() {
        assert_eq!(logind_verb("systemctl poweroff"), Some(Verb::PowerOff));
        assert_eq!(logind_verb(" loginctl  reboot "), Some(Verb::Reboot));
        assert_eq!(logind_verb("/usr/bin/systemctl reboot"), Some(Verb::Reboot));
        assert_eq!(logind_verb("systemctl poweroff -i"), None);
        assert_eq!(logind_verb("sudo systemctl poweroff"), None);
        assert_eq!(logind_verb("systemctl suspend"), Some(Verb::Suspend));
        assert_eq!(logind_verb("loginctl hibernate"), Some(Verb::Hibernate));
        assert_eq!(logind_verb("systemctl suspend-then-hibernate"), None);
        assert_eq!(logind_verb("my-poweroff"), None);
        assert_eq!(logind_verb(""), None);
    }

    #[test]
    fn only_no_and_na_keep_the_programs_open() {
        assert_eq!(can_answer("yes"), Ok(Password::No));
        assert_eq!(can_answer("challenge"), Ok(Password::Yes));
        assert_eq!(can_answer("no"), Err(Refused::NotAllowed));
        assert_eq!(can_answer("na"), Err(Refused::Unsupported));
        assert_eq!(can_answer("inhibited"), Ok(Password::OverInhibitors));
        assert_eq!(can_answer("maybe"), Err(Refused::NotAllowed));
    }

    #[test]
    fn only_end_now_asks_for_the_password_to_ignore_the_inhibitors() {
        assert!(Password::OverInhibitors.asked(true));
        assert!(!Password::OverInhibitors.asked(false));
        assert!(Password::Yes.asked(false));
        assert!(!Password::No.asked(true));
    }

    fn fading() -> crate::harness::Harness {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        h.state.config.effects.session_fade = true;
        h.state.config.effects.session_fade_out_ms = 400;
        h
    }

    #[test]
    fn a_turn_off_that_asks_for_a_password_fades_only_once_it_is_under_way() {
        let mut h = fading();
        let off = |h: &mut crate::harness::Harness, cmd: &str, password| {
            go_on(
                &mut h.state,
                Goal::TurnOff,
                Some(cmd.into()),
                None,
                false,
                password,
            );
        };
        off(&mut h, "false", Password::Yes);
        assert!(
            !h.state.session_fade.fading_out(),
            "no fade before the answer"
        );
        assert!(!crate::crash::is_ending(), "the session goes on");
        assert_eq!(h.state.dialogs.views.len(), 1, "the refusal's message box");
        off(&mut h, "true", Password::Yes);
        assert!(h.state.session_fade.fading_out(), "under way, it fades");

        let mut h = fading();
        off(&mut h, "false", Password::No);
        assert!(
            h.state.session_fade.fading_out(),
            "without a password the fade goes first"
        );
    }

    #[test]
    fn a_locked_turn_off_fades_before_it_asks_logind() {
        let mut h = locked();
        h.state.config.effects.session_fade = true;
        h.state.config.effects.session_fade_out_ms = 400;
        go_on(
            &mut h.state,
            Goal::TurnOff,
            Some("false".into()),
            None,
            false,
            Password::Yes,
        );
        assert!(h.state.session_fade.fading_out());
    }

    #[test]
    fn a_dismissed_password_prompt_brings_the_desktop_back_without_a_message() {
        let mut h = fading();
        crate::crash::ending();
        h.state.session.kept = true;
        crate::sessionfade::out(&mut h.state, Box::new(|_| {}));
        assert!(h.state.session_fade.fading_out());
        let id = h.state.powering.begin(None);
        answered(
            &mut h.state,
            id,
            Act::TurnOff,
            Some(Failed::Dismissed),
            None,
        );
        assert!(!h.state.session_fade.fading_out(), "the desktop shows");
        assert!(!crate::crash::is_ending(), "the session goes on");
        assert!(!h.state.session.kept, "the session saves resume");
        assert!(h.state.dialogs.views.is_empty(), "no message box");
    }

    #[test]
    fn a_dismissed_prompt_is_no_refusal_and_other_errors_keep_their_text() {
        let denied = reprisebus::error::reply(
            "org.freedesktop.DBus.Error.AccessDenied",
            "Permission denied",
        );
        assert_eq!(failure(&denied, || true), None);
        assert_eq!(failure(&denied, || false), Some(Refused::NotAllowed));
        let other = reprisebus::error::reply("org.freedesktop.login1.OperationInProgress", "Busy");
        let got = failure(&other, || panic!("only a refusal asks about the prompt"));
        assert_eq!(got, Some(Refused::Failed("Busy".into())));
        let late = reprisebus::error::reply(
            "org.freedesktop.DBus.Error.NoReply",
            "Method call timed out",
        );
        assert_eq!(
            failure(&late, || false),
            Some(Refused::Failed(reprisebus::polkit::TIMED_OUT.into()))
        );
        let timeout =
            reprisebus::error::reply("org.freedesktop.DBus.Error.Timeout", "Connection timed out");
        assert_eq!(
            failure(&timeout, || false),
            Some(Refused::Failed(reprisebus::polkit::TIMED_OUT.into()))
        );
        let blocked = reprisebus::error::reply(
            "org.freedesktop.login1.BlockedByInhibitorLock",
            "Operation denied due to active block inhibitor",
        );
        assert_eq!(failure(&blocked, || false), Some(Refused::Held(Vec::new())));
    }

    #[test]
    fn an_override_skips_the_inhibitors_and_an_old_logind_falls_back() {
        assert_eq!(logind_flags(true), 0x10);
        assert_eq!(logind_flags(false), 0);
        let old =
            reprisebus::error::reply("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method");
        assert!(too_old_for_flags(&old));
        let blocked =
            reprisebus::error::reply("org.freedesktop.login1.BlockedByInhibitorLock", "Blocked");
        assert!(!too_old_for_flags(&blocked));
    }

    #[test]
    fn a_failed_command_reports_its_last_error_line_else_its_status() {
        use std::os::unix::process::ExitStatusExt;
        let st = std::process::ExitStatus::from_raw(1 << 8);
        assert_eq!(
            command_failure(
                "off",
                &st,
                b"warning\nCall to PowerOff failed: Access denied\n\n"
            ),
            "Call to PowerOff failed: Access denied"
        );
        assert_eq!(
            command_failure("off", &st, b""),
            "\"off\" failed (exit status: 1)."
        );
    }

    #[test]
    fn a_command_is_done_when_it_exits_while_a_child_it_left_holds_stderr() {
        let env = crate::spawn::SessionEnv::default();
        let started = std::time::Instant::now();
        assert_eq!(run("sleep 20 & exit 0", &env), None);
        assert_eq!(
            run("sleep 20 & echo 'no suspend' >&2; exit 1", &env),
            Some(Refused::Failed("no suspend".into()))
        );
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "waited {:?} for the children",
            started.elapsed()
        );
    }

    #[test]
    fn the_message_names_the_action_and_why() {
        assert_eq!(
            refusal_text(Act::Restart, &Refused::NotAllowed),
            "Reprise could not restart the computer.\n\nYou are not allowed to restart this computer."
        );
        let held = Refused::Held(vec![
            holder("packagekit", "Installing updates", true),
            holder("backup", "", true),
        ]);
        assert_eq!(
            refusal_text(Act::TurnOff, &held),
            "Reprise could not turn off the computer.\n\n\
             These programs are preventing it:\npackagekit: Installing updates\nbackup"
        );
        assert!(
            refusal_text(Act::TurnOff, &Refused::Held(Vec::new()))
                .ends_with("\n\nA program is preventing it.")
        );
        assert_eq!(
            refusal_text(Act::StandBy, &Refused::NotAllowed),
            "Reprise could not put the computer on stand by.\n\n\
             You are not allowed to put this computer on stand by."
        );
        assert_eq!(
            refusal_text(
                Act::Hibernate,
                &Refused::Failed("Unit hibernate.target is masked.".into())
            ),
            "Reprise could not hibernate the computer.\n\nUnit hibernate.target is masked."
        );
    }
}
