//! Ending the session: Log Off, Turn Off and Restart close programs first so nothing unsaved is lost without a word.
//! `begin` asks every window to close, then the session ends once no window or inhibitor holds it; after `PROMPT_AFTER`
//! the session-end prompt lists what is left. A critical battery kills the remainder after `PROMPT_AFTER` without a prompt.
//! Turn Off and Restart ask logind first, and a refusal before or after the programs close shows a message box.
//! Stand By and Hibernate close nothing; a refusal shows the same message box.

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
use crate::window::WindowId;

const PROMPT_AFTER: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(500);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest a session end waits for the remote desktop server to tell its clients.
const TELL_CLIENTS: Duration = Duration::from_secs(1);

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
    holders: Option<Vec<Holder>>,
    stop: Option<async_channel::Sender<()>>,
    /// The poll moves the portal's monitors to query-end ([`asks_portal`]).
    asked_portal: bool,
    timer: Option<RegistrationToken>,
    reports: Option<RegistrationToken>,
    mode: Mode,
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
    check(state, goal, cmd, false, origin);
}

pub fn begin_critical(state: &mut State, cmd: String) {
    check(state, Goal::TurnOff, Some(cmd), true, Origin::Console);
}

/// Start the end once logind says the action is allowed, so a refusal
/// closes nothing. A custom command has only its exit status to go by.
fn check(state: &mut State, goal: Goal, cmd: Option<String>, critical: bool, origin: Origin) {
    let verb = cmd.as_deref().and_then(logind_verb);
    let Some(verb) = verb.filter(|_| state.ending.is_none()) else {
        start(state, goal, cmd, critical, origin);
        return;
    };
    state.spawn_job(move || {
        let refused = zbus::block_on(can(verb));
        Box::new(move |state: &mut State| match refused {
            None => start(state, goal, cmd, critical, origin),
            Some(why) => refuse(state, Act::of(goal), &why),
        })
    });
}

fn start(state: &mut State, goal: Goal, cmd: Option<String>, critical: bool, origin: Origin) {
    if let Some(e) = state.ending.as_mut() {
        let (mode, step) = e.mode.again(critical);
        if e.mode != Mode::Critical {
            log::info!("session end: now {goal:?}");
            e.goal = goal;
            e.cmd = cmd;
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
        go_on(state, goal, cmd, false);
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
    state.ending = Some(Ending {
        goal,
        cmd,
        // Without the thread nothing is known to hold the session.
        holders: stop.is_none().then(Vec::new),
        stop,
        asked_portal: ask_portal,
        timer,
        reports,
        mode: Mode::new(critical),
    });
    // The compositor's own dialogs (Run, a Properties sheet) go first.
    // The windows are asked once the end is under way, so a program
    // already known to hang gets no End Program of its own.
    state.close_all_dialogs();
    let open: Vec<WindowId> = state
        .windows
        .iter()
        .filter(|w| w.mapped && !w.override_redirect)
        .map(|w| w.id)
        .collect();
    for w in open {
        state.close(w);
    }
}

pub fn tick(state: &mut State) {
    let Some(e) = state.ending.as_ref() else {
        return;
    };
    let programs = programs(state);
    let holders = e.holders.as_ref().map(|h| holding(h, e.goal));
    if programs.is_empty() && holders.as_ref().is_some_and(Vec::is_empty) {
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
        end_process(state, pid);
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
    go_on(state, e.goal, e.cmd, override_inhibitors);
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

/// Fade the screen out, then end: stop the compositor for Log Off, else run `cmd`. Remote clients
/// hear the session logged off; the end waits for the server's answer, [`TELL_CLIENTS`] at most.
fn go_on(state: &mut State, goal: Goal, cmd: Option<String>, override_inhibitors: bool) {
    crate::crash::ending();
    let powers_off = goal != Goal::LogOff;
    let end: OnLoop = Box::new(move |state| act(state, goal, cmd, override_inhibitors));
    // A deferring session's Turn Off also ends the other session's clients.
    let server_ours = state.standby.rdp_on() || (powers_off && state.env.defers_user_units);
    if !crate::console::remote_side(state) && !server_ours {
        crate::sessionfade::out(state, end);
        return;
    }
    let wait = Arc::new(Mutex::new(Wait {
        faded: false,
        told: false,
        end: Some(end),
    }));
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
    crate::sessionfade::out(
        state,
        Box::new(move |state| arrive(&wait, state, |w| w.faded = true)),
    );
}

/// A session end waiting for its fade and for the remote clients to hear of it.
struct Wait {
    faded: bool,
    told: bool,
    end: Option<OnLoop>,
}

/// Mark what arrived, then end once both have.
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

/// End the session now, whatever is open (a SIGTERM, a GPU gone): fade out, then stop the loop.
pub fn quit(state: &mut State) {
    crate::crash::ending();
    crate::sessionfade::out(state, Box::new(|state| state.signal.stop()));
}

fn act(state: &mut State, goal: Goal, cmd: Option<String>, override_inhibitors: bool) {
    let Some(cmd) = cmd else {
        state.signal.stop();
        return;
    };
    log::info!("session end: {goal:?} runs {cmd:?}");
    attempt(state, Act::of(goal), cmd, override_inhibitors);
}

/// Put the computer to sleep with a `[power]` standby or hibernate `cmd`, closing nothing.
pub fn sleep(state: &mut State, act: Act, cmd: String) {
    if cmd.trim().is_empty() {
        return;
    }
    log::info!("power: {act:?} runs {cmd:?}");
    attempt(state, act, cmd, false);
}

/// Run `cmd` off the loop, on logind when it names a logind action; a refusal shows a message box.
fn attempt(state: &mut State, act: Act, cmd: String, override_inhibitors: bool) {
    let env = state.env.clone();
    let interactive = prompts(state);
    state.spawn_job(move || {
        let failed = match logind_verb(&cmd) {
            Some(verb) => zbus::block_on(call(verb, override_inhibitors, interactive)),
            None => run(&cmd, &env),
        };
        Box::new(move |state: &mut State| {
            if let Some(why) = failed {
                refuse(state, act, &why);
            }
        })
    });
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

/// Why the computer stayed up.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Refused {
    NotAllowed,
    Unsupported,
    Held(Vec<Holder>),
    Failed(String),
}

fn refuse(state: &mut State, act: Act, why: &Refused) {
    log::warn!("session end: {act:?} refused: {why:?}");
    if matches!(act, Act::TurnOff | Act::Restart) {
        crate::crash::not_ending();
    }
    crate::sessionfade::restore(state);
    // Anyone at the computer reads the lock: it names no program and quotes no command's error.
    let unnamed = match why {
        Refused::Held(_) => Refused::Held(Vec::new()),
        Refused::Failed(_) => Refused::Failed(LOCK_FAILED.into()),
        why => why.clone(),
    };
    crate::lockview::refused(state, refusal_text(act, &unnamed));
    state.open_dialog(crate::dialog::Dialog::error(
        act.title(),
        refusal_text(act, why),
    ));
}

/// What the lock shows for a failed command or call; the dialog under it keeps the error.
const LOCK_FAILED: &str = "An error stopped it.";

fn refusal_text(act: Act, why: &Refused) -> String {
    let reason = match why {
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
    };
    format!("Reprise could not {}.\n\n{reason}", act.on("the"))
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

/// logind's `Can*` answer as a refusal. `challenge` goes ahead, as polkit
/// asks for a password; so does `inhibited`, whose holders the prompt lists.
fn refused_by(answer: &str) -> Option<Refused> {
    match answer {
        "yes" | "challenge" | "inhibited" => None,
        "na" => Some(Refused::Unsupported),
        _ => Some(Refused::NotAllowed),
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
async fn can(verb: Verb) -> Option<Refused> {
    let conn = connect(true).await?;
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
            Some(with_holders(&conn, verb, refused_by(&answer)?).await)
        }
        Err(e) => {
            log::warn!("session end: {}: {e}", verb.can());
            None
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

async fn call(verb: Verb, override_inhibitors: bool, interactive: bool) -> Option<Refused> {
    // No method timeout: a password prompt stays up as long as polkit lets it.
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return Some(Refused::Failed(e.to_string())),
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
    Some(with_holders(&conn, verb, failure(&e, dismissed)?).await)
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

/// Run a custom `[power]` command and wait for it.
fn run(cmd: &str, env: &crate::spawn::SessionEnv) -> Option<Refused> {
    let Some(mut c) = crate::spawn::command(cmd, env, None) else {
        return Some(Refused::NotAllowed);
    };
    crate::spawn::detach(&mut c);
    c.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    match c.output() {
        Ok(o) if o.status.success() => None,
        Ok(o) => Some(Refused::Failed(command_failure(cmd, &o.status, &o.stderr))),
        Err(e) => Some(Refused::Failed(format!("\"{cmd}\" did not start: {e}"))),
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

/// SIGKILL `pid`, as End Program's End Now does: never the compositor or
/// the Xwayland server.
fn end_process(state: &State, pid: i32) {
    if !state.may_end(pid) {
        return;
    }
    log::info!("session end: killing pid {pid}");
    // SAFETY: plain syscall on a pid, no pointers.
    if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
        log::warn!(
            "session end: kill {pid}: {}",
            std::io::Error::last_os_error()
        );
    }
}

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
        let window = programs
            .iter()
            .position(|p| !h.shutdown_only && !h.who.is_empty() && p.app_id == h.who);
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

fn prompt_index(state: &State) -> Option<usize> {
    state.dialogs.views.iter().position(|v| {
        matches!(&v.dialog.style, crate::dialog::Style::Form(f)
            if f.kind == crate::dialog::Kind::EndSession)
    })
}

fn sync_prompt(state: &mut State) {
    let Some(e) = state.ending.as_ref().filter(|e| e.mode == Mode::Prompted) else {
        return;
    };
    let goal = e.goal;
    let holders = holding(e.holders.as_deref().unwrap_or(&[]), goal);
    let programs = programs(state);
    let apps = &state.app_index;
    let name_of = |app_id: &str| {
        if app_id.is_empty() {
            return "A program".to_string();
        }
        reprisefs::apps::entry_for_app_id(apps, app_id, |_| true)
            .map(|e| e.name.clone())
            .unwrap_or_else(|| app_id.to_string())
    };
    let mut fields = endsession::Fields {
        goal,
        rows: rows(&programs, &holders, name_of),
        icon: None,
    };
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

/// The prompt's picture ([`crate::icon::unread`]).
fn warning_icon(state: &mut State) -> Option<crate::raster::Canvas> {
    crate::icon::unread(&mut state.icons, "dialog-warning", 32).map(Arc::unwrap_or_clone)
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
        .and_then(|m| m.body().deserialize::<Vec<(String, String)>>());
    match reply {
        Ok(v) => v
            .into_iter()
            .map(|(who, why)| Holder {
                who,
                why,
                shutdown_only: false,
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
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cold_warning_icon_lands_on_the_open_prompt() {
        use reprisekit::controls::form::Control;
        let root = crate::icon::svg_theme(&["dialog-warning"], &[]);
        let mut h = crate::icon::two_outputs("classic", &root);
        h.state.ending = Some(Ending {
            goal: Goal::LogOff,
            cmd: None,
            // Unknown holders keep the end from finishing while the loop turns.
            holders: None,
            stop: None,
            asked_portal: false,
            timer: None,
            reports: None,
            mode: Mode::Prompted,
        });
        sync_prompt(&mut h.state);
        let picture = |h: &crate::harness::Harness| {
            let i = prompt_index(&h.state).expect("the prompt");
            let page = h.state.dialogs.views[i].dialog.form()?.open_page()?;
            match &page.item("icon")?.control {
                Control::Picture { canvas, .. } => canvas.clone(),
                _ => None,
            }
        };
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
        go_on(&mut h.state, Goal::LogOff, None, false);
        assert_eq!(
            h.state.ended_clients,
            [crate::console::ClientsEnd::LoggedOff]
        );

        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        go_on(&mut h.state, Goal::LogOff, None, false);
        assert!(h.state.ended_clients.is_empty(), "no client to tell");
        assert!(h.state.session_fade.fading_out(), "the fade runs");
    }

    #[test]
    fn a_log_off_with_the_remote_desktop_server_running_tells_a_view_only_client() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::standby::rdp_changed(&mut h.state, true);
        assert!(!h.state.remote_controlled());
        go_on(&mut h.state, Goal::LogOff, None, false);
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
        go_on(&mut h.state, Goal::LogOff, None, false);
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
            holders: None,
            stop: None,
            asked_portal,
            timer: None,
            reports: None,
            mode: Mode::Waiting,
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
        go_on(&mut h.state, Goal::TurnOff, Some("true".into()), false);
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
    fn a_refused_turn_off_lets_a_later_panic_restart_the_session() {
        let mut h = crate::harness::Harness::new();
        h.output("TEST-1", 1024, 768);
        crate::crash::ending();
        refuse(&mut h.state, Act::TurnOff, &Refused::NotAllowed);
        assert!(!crate::crash::is_ending());
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
        assert_eq!(refused_by("yes"), None);
        assert_eq!(refused_by("challenge"), None);
        assert_eq!(refused_by("no"), Some(Refused::NotAllowed));
        assert_eq!(refused_by("na"), Some(Refused::Unsupported));
        assert_eq!(refused_by("inhibited"), None);
        assert_eq!(refused_by("maybe"), Some(Refused::NotAllowed));
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
