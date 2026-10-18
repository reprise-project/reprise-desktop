//! Ending the session: Log Off, Turn Off and Restart close programs first so nothing unsaved is lost without a word.
//! `begin` asks every window to close, then the session ends once no window or inhibitor holds it; after `PROMPT_AFTER`
//! the session-end prompt lists what is left. A critical battery kills the remainder after `PROMPT_AFTER` without a prompt.
//! Turn Off and Restart ask logind first, and a refusal before or after the programs close shows a message box.

use std::time::Duration;

use reprisekit::controls::endsession::{self, Choice, Goal, Row};
use smithay::reexports::calloop::{
    RegistrationToken, channel,
    timer::{TimeoutAction, Timer},
};

use crate::compositor::State;
use crate::window::WindowId;

const PROMPT_AFTER: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(500);
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

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

pub fn begin(state: &mut State, goal: Goal, cmd: Option<String>) {
    check(state, goal, cmd, false);
}

pub fn begin_critical(state: &mut State, cmd: String) {
    check(state, Goal::TurnOff, Some(cmd), true);
}

/// Start the end once logind says the action is allowed, so a refusal
/// closes nothing. A custom command has only its exit status to go by.
fn check(state: &mut State, goal: Goal, cmd: Option<String>, critical: bool) {
    let verb = cmd.as_deref().and_then(logind_verb);
    let Some(verb) = verb.filter(|_| state.ending.is_none()) else {
        start(state, goal, cmd, critical);
        return;
    };
    state.spawn_job(move || {
        let refused = zbus::block_on(can(verb));
        Box::new(move |state: &mut State| match refused {
            None => start(state, goal, cmd, critical),
            Some(why) => refuse(state, goal, &why),
        })
    });
}

fn start(state: &mut State, goal: Goal, cmd: Option<String>, critical: bool) {
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
    if state.is_locked() {
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
    let stop = spawn_poll(tx);
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
    // A command (Turn Off, Restart) leaves the compositor running until
    // the system stops it, or for good if it fails: the portal's monitors
    // go back to running. Log Off just ends the thread with the loop.
    if e.cmd.is_some()
        && let Some(stop) = e.stop.take()
    {
        let _ = stop.try_send(());
    }
    go_on(state, e.goal, e.cmd, override_inhibitors);
}

/// Fade the screen out, then end: stop the compositor for Log Off, else run `cmd`.
fn go_on(state: &mut State, goal: Goal, cmd: Option<String>, override_inhibitors: bool) {
    crate::sessionfade::out(
        state,
        Box::new(move |state| act(state, goal, cmd, override_inhibitors)),
    );
}

fn act(state: &mut State, goal: Goal, cmd: Option<String>, override_inhibitors: bool) {
    let Some(cmd) = cmd else {
        state.signal.stop();
        return;
    };
    log::info!("session end: {goal:?} runs {cmd:?}");
    let env = state.env.clone();
    state.spawn_job(move || {
        let failed = match logind_verb(&cmd) {
            Some(verb) => zbus::block_on(call(verb, override_inhibitors)),
            None => run(&cmd, &env),
        };
        Box::new(move |state: &mut State| {
            if let Some(why) = failed {
                refuse(state, goal, &why);
            }
        })
    });
}

/// Why the computer stayed up.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Refused {
    NotAllowed,
    Unsupported,
    Held(Vec<Holder>),
    Failed(String),
}

fn refuse(state: &mut State, goal: Goal, why: &Refused) {
    log::warn!("session end: {goal:?} refused: {why:?}");
    crate::sessionfade::restore(state);
    let title = match goal {
        Goal::Restart => "Restart Computer",
        Goal::TurnOff | Goal::LogOff => "Turn Off Computer",
    };
    state.open_dialog(crate::dialog::Dialog::error(title, refusal_text(goal, why)));
}

fn refusal_text(goal: Goal, why: &Refused) -> String {
    let what = match goal {
        Goal::Restart => "restart",
        Goal::TurnOff | Goal::LogOff => "turn off",
    };
    let reason = match why {
        Refused::NotAllowed => format!("You are not allowed to {what} this computer."),
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
    format!("Reprise could not {what} the computer.\n\n{reason}")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    PowerOff,
    Reboot,
}

impl Verb {
    fn can(self) -> &'static str {
        match self {
            Verb::PowerOff => "CanPowerOff",
            Verb::Reboot => "CanReboot",
        }
    }

    fn method(self) -> &'static str {
        match self {
            Verb::PowerOff => "PowerOff",
            Verb::Reboot => "Reboot",
        }
    }

    fn method_with_flags(self) -> &'static str {
        match self {
            Verb::PowerOff => "PowerOffWithFlags",
            Verb::Reboot => "RebootWithFlags",
        }
    }
}

/// The logind action `cmd` is when it says exactly `systemctl` or `loginctl`
/// with `poweroff` or `reboot`; logind is then called directly, so polkit
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
async fn with_holders(conn: &zbus::Connection, why: Refused) -> Refused {
    if !matches!(why, Refused::NotAllowed | Refused::Held(_)) {
        return why;
    }
    let holders = logind_holders(conn).await;
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
            Some(with_holders(&conn, refused_by(&answer)?).await)
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
        error_name(e),
        Some("org.freedesktop.DBus.Error.UnknownMethod" | "org.freedesktop.DBus.Error.InvalidArgs")
    )
}

async fn call(verb: Verb, override_inhibitors: bool) -> Option<Refused> {
    // No method timeout: a password prompt stays up as long as polkit lets it.
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => return Some(Refused::Failed(e.to_string())),
    };
    let reply = async {
        let proxy: zbus::Proxy<'_> = zbus::proxy::Builder::new(&conn)
            .destination(LOGIND)?
            .path(LOGIND_PATH)?
            .interface(LOGIND_MANAGER)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let interactive = zbus::proxy::MethodFlags::AllowInteractiveAuth.into();
        let flags = logind_flags(override_inhibitors);
        match proxy
            .call_with_flags::<_, _, ()>(verb.method_with_flags(), interactive, &(flags,))
            .await
        {
            Err(e) if too_old_for_flags(&e) => {
                log::info!("session end: {}: {e}", verb.method_with_flags());
                proxy
                    .call_with_flags::<_, _, ()>(verb.method(), interactive, &(true,))
                    .await
            }
            r => r,
        }
    }
    .await;
    let e = reply.err()?;
    Some(with_holders(&conn, failure(&e, reprisebus::polkit::dismissed)?).await)
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
        None if error_name(e) == Some("org.freedesktop.login1.BlockedByInhibitorLock") => {
            Some(Refused::Held(Vec::new()))
        }
        None => Some(Refused::Failed(match e {
            _ if error_name(e) == Some("org.freedesktop.DBus.Error.Timeout") => {
                reprisebus::polkit::TIMED_OUT.to_string()
            }
            zbus::Error::MethodError(_, Some(m), _) => m.clone(),
            e => e.to_string(),
        })),
    }
}

fn error_name(e: &zbus::Error) -> Option<&str> {
    match e {
        zbus::Error::MethodError(name, _, _) => Some(name.as_str()),
        _ => None,
    }
}

/// Run a custom `[power]` command and wait for it.
fn run(cmd: &str, env: &crate::spawn::SessionEnv) -> Option<Refused> {
    let mut c = crate::spawn::command(cmd, env, None);
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
    let open: Vec<(Option<i32>, String, String, bool)> = state
        .windows
        .iter()
        .filter(|w| w.mapped && !w.override_redirect)
        .map(|w| {
            (
                state.client_pid(w.id),
                w.app(),
                w.title(),
                state.parent_window(w.id).is_some(),
            )
        })
        .collect();
    group(open)
}

/// [`programs`] over (pid, app id, title, owned) per window: windows of
/// one pid are one program, named by its first unowned window's title,
/// else its app id. A window with no pid is a program of its own.
fn group(windows: Vec<(Option<i32>, String, String, bool)>) -> Vec<Program> {
    let mut out: Vec<(Program, bool)> = Vec::new();
    for (pid, app_id, title, owned) in windows {
        let name = if title.is_empty() {
            app_id.clone()
        } else {
            title
        };
        match out.iter_mut().find(|(p, _)| pid.is_some() && p.pid == pid) {
            // An owned window names the program only until an unowned
            // one comes.
            Some((p, named_by_owned)) => {
                if *named_by_owned && !owned {
                    p.name = name;
                    *named_by_owned = false;
                }
            }
            None => out.push((Program { pid, app_id, name }, owned)),
        }
    }
    out.into_iter().map(|(p, _)| p).collect()
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
            fields.icon = state.icons.get("dialog-warning", 32);
            let t = &state.config.theme;
            let d =
                crate::dialog::Dialog::end_session(&fields, &state.text, t.fonts.size_px, t.text);
            state.open_dialog(d);
        }
    }
}

fn spawn_poll(tx: channel::Sender<Vec<Holder>>) -> Option<async_channel::Sender<()>> {
    let (stop_tx, stop_rx) = async_channel::bounded::<()>(1);
    let thread = std::thread::Builder::new()
        .name("session-end".into())
        .spawn(move || zbus::block_on(poll(tx, stop_rx)));
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
    match b.map(|b| b.method_timeout(CALL_TIMEOUT)) {
        Ok(b) => b
            .build()
            .await
            .map_err(|e| log::warn!("session end: bus: {e}"))
            .ok(),
        Err(e) => {
            log::warn!("session end: bus: {e}");
            None
        }
    }
}

async fn poll(tx: channel::Sender<Vec<Holder>>, stop: async_channel::Receiver<()>) {
    let session = connect(false).await;
    let system = connect(true).await;
    loop {
        let mut held = match &session {
            Some(c) => portal_holders(c, true).await,
            None => Vec::new(),
        };
        if let Some(c) = &system {
            held.extend(logind_holders(c).await);
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

async fn logind_holders(conn: &zbus::Connection) -> Vec<Holder> {
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
        Ok(v) => shutdown_blocks(
            v.into_iter()
                .map(|(what, who, why, mode, _, _)| (what, who, why, mode)),
        ),
        Err(e) => {
            log::warn!("session end: logind: {e}");
            Vec::new()
        }
    }
}

fn shutdown_blocks(list: impl Iterator<Item = (String, String, String, String)>) -> Vec<Holder> {
    list.filter(|(what, _, _, mode)| mode == "block" && what.split(':').any(|w| w == "shutdown"))
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

    fn win(
        pid: Option<i32>,
        app: &str,
        title: &str,
        owned: bool,
    ) -> (Option<i32>, String, String, bool) {
        (pid, app.into(), title.into(), owned)
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
            win(Some(10), "reprise-notepad", "Notepad", true),
            win(Some(10), "reprise-notepad", "Untitled - Notepad", false),
            win(Some(11), "foot", "", false),
            win(None, "x", "No pid", false),
            win(None, "x", "No pid either", false),
            win(Some(10), "reprise-notepad", "notes.txt - Notepad", false),
        ]);
        let names: Vec<&str> = got.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(
            names,
            ["Untitled - Notepad", "foot", "No pid", "No pid either"]
        );
        assert_eq!(got[0].pid, Some(10));
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
    fn only_shutdown_blocks_are_listed() {
        let s = |a: &str, b: &str, c: &str, d: &str| (a.into(), b.into(), c.into(), d.into());
        let got = shutdown_blocks(
            vec![
                s("shutdown:sleep", "packagekit", "Installing", "block"),
                s("shutdown", "NetworkManager", "Saving", "delay"),
                s("sleep", "portal", "Playing", "block"),
                s("handle-power-key:handle-lid-switch", "reprise", "", "block"),
            ]
            .into_iter(),
        );
        assert_eq!(got, [holder("packagekit", "Installing", true)]);
    }

    #[test]
    fn only_a_bare_systemctl_or_loginctl_command_is_called_on_logind() {
        assert_eq!(logind_verb("systemctl poweroff"), Some(Verb::PowerOff));
        assert_eq!(logind_verb(" loginctl  reboot "), Some(Verb::Reboot));
        assert_eq!(logind_verb("/usr/bin/systemctl reboot"), Some(Verb::Reboot));
        assert_eq!(logind_verb("systemctl poweroff -i"), None);
        assert_eq!(logind_verb("sudo systemctl poweroff"), None);
        assert_eq!(logind_verb("systemctl suspend"), None);
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

    fn method_error(name: &'static str, text: &str) -> zbus::Error {
        let msg = zbus::Message::method_call("/", "M")
            .and_then(|b| b.build(&()))
            .expect("a method call");
        zbus::Error::MethodError(
            zbus::names::OwnedErrorName::try_from(name).expect("an error name"),
            Some(text.into()),
            msg,
        )
    }

    #[test]
    fn a_dismissed_prompt_is_no_refusal_and_other_errors_keep_their_text() {
        let denied = method_error(
            "org.freedesktop.DBus.Error.AccessDenied",
            "Permission denied",
        );
        assert_eq!(failure(&denied, || true), None);
        assert_eq!(failure(&denied, || false), Some(Refused::NotAllowed));
        let other = method_error("org.freedesktop.login1.OperationInProgress", "Busy");
        let got = failure(&other, || panic!("only a refusal asks about the prompt"));
        assert_eq!(got, Some(Refused::Failed("Busy".into())));
        let late = method_error(
            "org.freedesktop.DBus.Error.NoReply",
            "Method call timed out",
        );
        assert_eq!(
            failure(&late, || false),
            Some(Refused::Failed(reprisebus::polkit::TIMED_OUT.into()))
        );
        let blocked = method_error(
            "org.freedesktop.login1.BlockedByInhibitorLock",
            "Operation denied due to active block inhibitor",
        );
        assert_eq!(failure(&blocked, || false), Some(Refused::Held(Vec::new())));
    }

    #[test]
    fn an_override_skips_the_inhibitors_and_an_old_logind_falls_back() {
        assert_eq!(logind_flags(true), 0x10);
        assert_eq!(logind_flags(false), 0);
        let old = method_error("org.freedesktop.DBus.Error.UnknownMethod", "Unknown method");
        assert!(too_old_for_flags(&old));
        let blocked = method_error("org.freedesktop.login1.BlockedByInhibitorLock", "Blocked");
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
            refusal_text(Goal::Restart, &Refused::NotAllowed),
            "Reprise could not restart the computer.\n\nYou are not allowed to restart this computer."
        );
        let held = Refused::Held(vec![
            holder("packagekit", "Installing updates", true),
            holder("backup", "", true),
        ]);
        assert_eq!(
            refusal_text(Goal::TurnOff, &held),
            "Reprise could not turn off the computer.\n\n\
             These programs are preventing it:\npackagekit: Installing updates\nbackup"
        );
        assert!(
            refusal_text(Goal::TurnOff, &Refused::Held(Vec::new()))
                .ends_with("\n\nA program is preventing it.")
        );
    }
}
