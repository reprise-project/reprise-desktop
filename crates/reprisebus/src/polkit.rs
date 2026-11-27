//! reprise-polkit's record of dismissed password prompts, on the session bus under one [`name`]
//! per logind session. A service behind polkit answers the same error for a dismissed prompt and
//! a refused password; a program asks here which one it was, and [`withdraw`]s the prompts it
//! stopped waiting for.
//! [`allowed_without_prompt`] and [`allowed_without_password`] ask polkit itself whether an action
//! needs a password, [`may_ask`] whether a password would allow it.

use std::collections::HashMap;
use std::time::Duration;

use zbus::Variant;

pub const PATH: &str = "/reprise/polkit/Dismissals";
pub const INTERFACE: &str = "reprise.polkit.Dismissals";

/// The session bus name of the reprise-polkit serving logind session `session`. None for an id
/// with a byte other than a letter or digit, which logind never gives.
pub fn name(session: &str) -> Option<String> {
    let id = !session.is_empty() && session.bytes().all(|b| b.is_ascii_alphanumeric());
    id.then(|| format!("reprise.polkit.session{session}"))
}

/// The logind session polkitd judges a process by, and whose agent shows its prompts: the session
/// its `cgroup` listing is in, else its user's display session from logind's user `record`.
pub fn judged_session<'a>(cgroup: &'a str, record: &'a str) -> Option<&'a str> {
    crate::logind::session_of_cgroup(cgroup).or_else(|| crate::logind::display_of_user(record))
}

/// [`judged_session`] of this process.
pub fn own_judged_session() -> Option<String> {
    let (cgroup, record) = (
        crate::logind::own_cgroup(),
        crate::logind::own_user_record(),
    );
    judged_session(&cgroup, &record).map(String::from)
}

/// [`name`] of the agent polkitd sends the prompts of the process with these files to.
fn agent_name(cgroup: &str, record: &str) -> Option<String> {
    judged_session(cgroup, record).and_then(name)
}

/// Calls `method`, which takes nothing, on the reprise-polkit that shows this process's prompts.
/// Blocks for at most [`crate::METHOD_TIMEOUT`].
fn ask_agent(method: &str) -> Result<zbus::Message, String> {
    let (cgroup, record) = (
        crate::logind::own_cgroup(),
        crate::logind::own_user_record(),
    );
    let agent = agent_name(&cgroup, &record)
        .ok_or("no logind session whose reprise-polkit shows this process's prompts")?;
    crate::session()
        .and_then(|conn| {
            crate::wait(conn.call_method(Some(agent.as_str()), PATH, Some(INTERFACE), method, &()))
        })
        .map_err(|e| format!("{agent}: {e}"))
}

/// Whether the last password prompt polkit opened for this process was
/// dismissed, forgetting it once asked. False when reprise-polkit does not
/// answer. Blocks for at most [`crate::METHOD_TIMEOUT`].
pub fn dismissed() -> bool {
    let answer = ask_agent("Take")
        .and_then(|reply| (reply.body().deserialize::<bool>()).map_err(|e| e.to_string()));
    answer.unwrap_or_else(|e| {
        log::warn!("no word whether the prompt was dismissed: {e}");
        false
    })
}

/// Withdraw the password prompts reprise-polkit shows for this process: the service that asked
/// polkit gets no password, so one typed later changes nothing. Blocks for at most
/// [`crate::METHOD_TIMEOUT`].
pub fn withdraw() {
    withdraw_with("Withdraw");
}

/// [`withdraw`] as this process's session ends: a prompt a service is about to open for it within
/// the next seconds is withdrawn too.
pub fn withdraw_ending() {
    withdraw_with("WithdrawEnding");
}

fn withdraw_with(method: &str) {
    let answer = ask_agent(method)
        .and_then(|reply| (reply.body().deserialize::<u32>()).map_err(|e| e.to_string()));
    match answer {
        Ok(0) => {}
        Ok(n) => log::info!("withdrew {n} password prompt(s)"),
        Err(e) => log::warn!("no password prompt withdrawn: {e}"),
    }
}

/// How long a call that may open a password prompt waits, the change it then makes included:
/// time to read the prompt, find the password and type it. Past it the prompt is withdrawn.
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

/// `call`, or a timeout error once `limit` passes: zbus applies a connection's method timeout to
/// plain calls only, not to `call_with_flags`.
pub async fn within<T>(
    limit: Duration,
    call: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    let late = async {
        async_io::Timer::after(limit).await;
        let e = std::io::Error::from(std::io::ErrorKind::TimedOut);
        Err(zbus::Error::InputOutput(std::sync::Arc::new(e)))
    };
    futures_lite::future::or(call, late).await
}

/// [`within`] for a call that lets polkit ask for a password when `interactive`. A call that
/// gives up on its prompt ([`gave_up`]) or is dropped meanwhile leaves the prompt open; since
/// [`withdraw`] takes every prompt of this process, the last such call to end withdraws them.
pub async fn asking<T>(
    limit: Duration,
    interactive: bool,
    call: impl Future<Output = zbus::Result<T>>,
) -> zbus::Result<T> {
    asking_with(limit, interactive, call, &IN_FLIGHT, withdraw).await
}

/// A `NoReply` this long after the call is the service giving up on its prompt (systemd's
/// services wait 25 s); one sooner is the service quitting under the call.
const SERVICE_GAVE_UP: Duration = Duration::from_secs(20);

/// Whether `e`, from [`asking`], is its call giving up on the prompt: [`within`]'s timeout, or
/// the service's late `NoReply`. The change may have been made as the call gave up.
pub fn gave_up(e: &zbus::Error) -> bool {
    matches!(e, zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut)
}

/// What a program tells the user after a call that may open a password prompt. The texts are
/// the program's: [`told_text!`](crate::told_text) turns one into reprisekit's words for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Told {
    /// The call gave up on its prompt ([`gave_up`]).
    GaveUp,
    /// [`Refusal::TimedOut`].
    TimedOut,
    /// [`Refusal::Denied`].
    NotAllowed,
    /// The service's own message.
    Text(String),
}

/// A [`Told`] as the user's words, from reprisekit's `polkit-gave-up`, `polkit-timed-out` and
/// `polkit-not-allowed`. The caller links reprisekit.
#[macro_export]
macro_rules! told_text {
    ($told:expr) => {
        match $told {
            $crate::polkit::Told::GaveUp => ::reprisekit::kit_tr!("polkit-gave-up"),
            $crate::polkit::Told::TimedOut => ::reprisekit::kit_tr!("polkit-timed-out"),
            $crate::polkit::Told::NotAllowed => ::reprisekit::kit_tr!("polkit-not-allowed"),
            $crate::polkit::Told::Text(text) => text,
        }
    };
}

fn timeout() -> zbus::Error {
    let e = std::io::Error::from(std::io::ErrorKind::TimedOut);
    zbus::Error::InputOutput(std::sync::Arc::new(e))
}

/// `e` from a call that took `after`, with a late `NoReply` read as a timeout.
fn classified(e: zbus::Error, after: Duration) -> zbus::Error {
    let no_reply =
        crate::error::name(&e).is_some_and(|n| n == "org.freedesktop.DBus.Error.NoReply");
    match no_reply && after >= SERVICE_GAVE_UP {
        true => timeout(),
        false => e,
    }
}

async fn asking_with<T, W: FnOnce()>(
    limit: Duration,
    interactive: bool,
    call: impl Future<Output = zbus::Result<T>>,
    flight: &InFlight,
    withdraw: W,
) -> zbus::Result<T> {
    let start = std::time::Instant::now();
    let ending = interactive.then(|| Ending::new(flight, withdraw));
    let answer = within(limit, call)
        .await
        .map_err(|e| classified(e, start.elapsed()));
    if let Some(ending) = ending {
        ending.end(matches!(&answer, Err(e) if gave_up(e)));
    }
    answer
}

/// The calls that may prompt under way in this process, and whether one that ended left its
/// prompt open.
struct InFlight(std::sync::Mutex<(usize, bool)>);

static IN_FLIGHT: InFlight = InFlight(std::sync::Mutex::new((0, false)));

impl InFlight {
    fn begin(&self) {
        crate::mutex::lock(&self.0).0 += 1;
    }

    /// One call ended, leaving its prompt `open`; whether to withdraw the prompts now.
    fn end(&self, open: bool) -> bool {
        let mut f = crate::mutex::lock(&self.0);
        f.0 = f.0.saturating_sub(1);
        f.1 |= open;
        let now = f.0 == 0 && f.1;
        if now {
            f.1 = false;
        }
        now
    }
}

/// A call under way in [`InFlight`]; dropped before [`Ending::end`], it left its prompt open.
struct Ending<'a, W: FnOnce()> {
    flight: &'a InFlight,
    withdraw: Option<W>,
}

impl<'a, W: FnOnce()> Ending<'a, W> {
    fn new(flight: &'a InFlight, withdraw: W) -> Self {
        flight.begin();
        Ending {
            flight,
            withdraw: Some(withdraw),
        }
    }

    fn end(mut self, open: bool) {
        self.finish(open);
    }

    fn finish(&mut self, open: bool) {
        if let Some(withdraw) = self.withdraw.take()
            && self.flight.end(open)
        {
            withdraw();
        }
    }
}

impl<W: FnOnce()> Drop for Ending<'_, W> {
    fn drop(&mut self) {
        self.finish(true);
    }
}

/// How a service behind polkit said no.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The password prompt was dismissed: nothing to tell.
    Dismissed,
    /// The password was refused, or polkit allows no one.
    Denied,
    /// The service stopped waiting for the password prompt and answered
    /// `NoReply` (systemd's services wait 25 s), or the caller's own method
    /// timeout ran out first.
    TimedOut,
}

/// The refusal `e` is, when its error name passes `refused` or is `NoReply`, or
/// it is the connection's method timeout; None for any other error. `dismissed`, normally [`dismissed`], is asked only for a
/// refusal, since asking forgets the dismissal.
pub fn refusal(
    e: &zbus::Error,
    refused: impl FnOnce(&str) -> bool,
    dismissed: impl FnOnce() -> bool,
) -> Option<Refusal> {
    if crate::error::timed_out(e) {
        return Some(Refusal::TimedOut);
    }
    let name = crate::error::name(e)?;
    refused(&name).then(|| {
        if dismissed() {
            Refusal::Dismissed
        } else {
            Refusal::Denied
        }
    })
}

/// Whether polkit lets the owner of `conn` do `action` with no password prompt. A service that
/// asks polkit with interaction allowed whatever the call's flags (fwupd does) prompts otherwise.
/// False when polkit does not answer.
pub async fn allowed_without_prompt(conn: &zbus::Connection, action: &str) -> bool {
    check(conn, action).await.is_some_and(|c| c.authorized)
}

/// Whether polkit lets the owner of `conn` do `action`, at once or after a password prompt. True
/// when polkit does not answer, so the service itself says no.
pub async fn may_ask(conn: &zbus::Connection, action: &str) -> bool {
    check(conn, action)
        .await
        .is_none_or(|c| c.authorized || c.challenge)
}

/// What polkit says to an action, asked without interaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    /// After a password prompt.
    Prompts,
    Refused,
}

/// polkit's [`Verdict`] on `action` for the owner of `conn`; Allowed when polkit does not answer,
/// so the service itself says no. A password kept from an earlier call (`auth_admin_keep`)
/// Prompts, as it may lapse before the action runs.
pub async fn verdict(conn: &zbus::Connection, action: &str) -> Verdict {
    judged(check(conn, action).await.as_ref())
}

fn judged(checked: Option<&Checked>) -> Verdict {
    match checked {
        None => Verdict::Allowed,
        Some(c) if lasting(c.authorized, &c.details) => Verdict::Allowed,
        Some(c) if c.authorized || c.challenge => Verdict::Prompts,
        Some(_) => Verdict::Refused,
    }
}

/// [`allowed_without_prompt`] for good: a password typed for an earlier call, which polkit keeps
/// a few minutes (`auth_admin_keep`), does not count.
pub async fn allowed_without_password(conn: &zbus::Connection, action: &str) -> bool {
    check(conn, action)
        .await
        .is_some_and(|c| lasting(c.authorized, &c.details))
}

fn lasting(authorized: bool, details: &HashMap<String, String>) -> bool {
    authorized && !details.contains_key("polkit.temporary_authorization_id")
}

/// polkit's answer to `CheckAuthorization`.
struct Checked {
    authorized: bool,
    /// A password prompt would authorize it.
    challenge: bool,
    details: HashMap<String, String>,
}

/// polkit's `CheckAuthorization` for the owner of `conn`, without interaction. None when polkit
/// does not answer.
async fn check(conn: &zbus::Connection, action: &str) -> Option<Checked> {
    let name = conn.unique_name()?;
    let subject = (
        "system-bus-name",
        HashMap::from([("name", Variant::from(name.as_str()))]),
    );
    let details: HashMap<&str, &str> = HashMap::new();
    let reply = conn
        .call_method(
            Some("org.freedesktop.PolicyKit1"),
            "/org/freedesktop/PolicyKit1/Authority",
            Some("org.freedesktop.PolicyKit1.Authority"),
            "CheckAuthorization",
            &(subject, action, details, 0u32, ""),
        )
        .await
        .and_then(|m| {
            m.body()
                .deserialize::<(bool, bool, HashMap<String, String>)>()
        });
    match reply {
        Ok((authorized, challenge, details)) => Some(Checked {
            authorized,
            challenge,
            details,
        }),
        Err(e) => {
            log::info!("polkit does not say whether {action} needs a password: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(name: &str) -> zbus::Error {
        crate::error::reply(name, "Not authorized")
    }

    #[test]
    fn a_call_past_its_limit_fails_as_timed_out() {
        let call = futures_lite::future::pending::<zbus::Result<()>>();
        let e = zbus::block_on(within(Duration::from_millis(20), call)).unwrap_err();
        assert!(crate::error::timed_out(&e), "{e}");
        assert_eq!(
            zbus::block_on(within(Duration::from_secs(60), async { Ok(7) })).unwrap(),
            7
        );
    }

    fn flight() -> InFlight {
        InFlight(std::sync::Mutex::new((0, false)))
    }

    #[test]
    fn only_an_interactive_call_that_gave_up_withdraws_its_prompt() {
        let limit = Duration::from_millis(20);
        let withdrawn = |interactive: bool, call: zbus::Result<()>| {
            let n = std::cell::Cell::new(0);
            let call = async {
                match call {
                    Ok(()) => futures_lite::future::pending().await,
                    Err(e) => Err(e),
                }
            };
            let f = flight();
            let withdraw = || n.set(n.get() + 1);
            let _: zbus::Result<()> =
                zbus::block_on(asking_with(limit, interactive, call, &f, withdraw));
            n.get()
        };
        assert_eq!(withdrawn(true, Ok(())), 1, "past its limit");
        assert_eq!(withdrawn(false, Ok(())), 0, "no prompt to withdraw");
        let no_reply = error("org.freedesktop.DBus.Error.NoReply");
        assert_eq!(
            withdrawn(true, Err(no_reply)),
            0,
            "the service quit at once"
        );
        let denied = error("org.freedesktop.DBus.Error.AccessDenied");
        assert_eq!(withdrawn(true, Err(denied)), 0, "the prompt was answered");
        let f = flight();
        let answered = zbus::block_on(asking_with(limit, true, async { Ok(7) }, &f, || panic!()));
        assert_eq!(answered.unwrap(), 7);
    }

    #[test]
    fn a_late_no_reply_is_the_service_giving_up_and_an_early_one_its_failure() {
        let no_reply = || error("org.freedesktop.DBus.Error.NoReply");
        assert!(gave_up(&classified(no_reply(), Duration::from_secs(25))));
        let early = classified(no_reply(), Duration::from_secs(1));
        assert!(!gave_up(&early));
        assert!(
            crate::error::name(&early).is_some_and(|n| n == "org.freedesktop.DBus.Error.NoReply")
        );
        let denied = classified(
            error("org.freedesktop.DBus.Error.AccessDenied"),
            Duration::from_secs(60),
        );
        assert!(!gave_up(&denied));
    }

    #[test]
    fn the_last_call_out_withdraws_the_prompts_an_earlier_one_left_open() {
        let f = flight();
        f.begin();
        f.begin();
        assert!(!f.end(true), "another call's prompt may be up");
        assert!(f.end(false), "the last call out sweeps the open prompt");
        f.begin();
        assert!(!f.end(false), "nothing left open");
        f.begin();
        assert!(f.end(true), "alone, it withdraws its own");
    }

    #[test]
    fn a_call_dropped_while_its_prompt_is_up_withdraws_it() {
        let (f, n) = (flight(), std::cell::Cell::new(0));
        let pending = futures_lite::future::pending::<zbus::Result<()>>();
        let mut call = Box::pin(asking_with(PROMPT_TIMEOUT, true, pending, &f, || {
            n.set(n.get() + 1)
        }));
        assert!(zbus::block_on(futures_lite::future::poll_once(&mut call)).is_none());
        assert_eq!(n.get(), 0);
        drop(call);
        assert_eq!(n.get(), 1);
    }

    #[test]
    fn a_kept_password_is_no_lasting_permission() {
        let none = HashMap::new();
        assert!(lasting(true, &none));
        assert!(!lasting(false, &none));
        let kept = HashMap::from([(
            "polkit.temporary_authorization_id".to_string(),
            "tmpauthz7".to_string(),
        )]);
        assert!(!lasting(true, &kept));
    }

    #[test]
    fn a_kept_password_prompts_in_a_verdict() {
        let checked = |authorized, challenge, kept: bool| Checked {
            authorized,
            challenge,
            details: (kept.then(|| ("polkit.temporary_authorization_id".into(), "t".into())))
                .into_iter()
                .collect(),
        };
        let judge = |c: Checked| judged(Some(&c));
        assert_eq!(judged(None), Verdict::Allowed, "the service says no itself");
        assert_eq!(judge(checked(true, false, false)), Verdict::Allowed);
        assert_eq!(judge(checked(true, false, true)), Verdict::Prompts);
        assert_eq!(judge(checked(false, true, false)), Verdict::Prompts);
        assert_eq!(judge(checked(false, false, false)), Verdict::Refused);
    }

    #[test]
    fn each_logind_session_has_its_own_agent_name() {
        assert_eq!(name("3").as_deref(), Some("reprise.polkit.session3"));
        assert_eq!(name("c2").as_deref(), Some("reprise.polkit.sessionc2"));
        for bad in ["", "3.1", "3-1", "3 1", "*"] {
            assert_eq!(name(bad), None, "{bad:?} is no logind session id");
        }
    }

    #[test]
    fn a_program_asks_the_agent_of_the_session_polkit_judges_it_by() {
        let record = "NAME=ann\nDISPLAY=3\nSESSIONS=7 3\n";
        let in_session = "0::/user.slice/user-1000.slice/session-7.scope\n";
        assert_eq!(
            agent_name(in_session, record).as_deref(),
            Some("reprise.polkit.session7"),
            "a program in its own session asks that session's agent"
        );
        let app_scope =
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-x-1.scope\n";
        assert_eq!(
            agent_name(app_scope, record).as_deref(),
            Some("reprise.polkit.session3"),
            "an app scope is judged by the display session"
        );
        assert_eq!(agent_name(app_scope, "NAME=ann\nSESSIONS=7\n"), None);
    }

    #[test]
    fn only_a_refusal_asks_whether_the_prompt_was_dismissed() {
        let refused = |n: &str| n.ends_with("PermissionDenied");
        let denied = error("org.example.PermissionDenied");
        assert_eq!(refusal(&denied, refused, || true), Some(Refusal::Dismissed));
        assert_eq!(refusal(&denied, refused, || false), Some(Refusal::Denied));
        let other = refusal(&error("org.example.Failed"), refused, || {
            panic!("asking forgets a dismissal another error has no claim to")
        });
        assert_eq!(other, None);
        let local = refusal(
            &zbus::Error::InvalidReply,
            |_| true,
            || panic!("only a method error is a refusal"),
        );
        assert_eq!(local, None);
    }

    #[test]
    fn no_reply_is_a_timed_out_prompt_whatever_the_service_refuses_with() {
        let timed_out = refusal(
            &error("org.freedesktop.DBus.Error.NoReply"),
            |n| n.ends_with("PermissionDenied"),
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
    }

    #[test]
    fn the_callers_own_method_timeout_is_a_timed_out_prompt() {
        let io = zbus::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        let timed_out = refusal(
            &io,
            |_| true,
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
        let reset = zbus::Error::from(std::io::Error::from(std::io::ErrorKind::ConnectionReset));
        assert_eq!(refusal(&reset, |_| true, || false), None);
    }

    #[test]
    fn a_reply_named_in_the_dbus_namespace_is_a_refusal_or_a_timeout_too() {
        let reply = |name| crate::error::reply(name, "Permission denied");
        let denied = reply("org.freedesktop.DBus.Error.AccessDenied");
        let refused = |n: &str| n == "org.freedesktop.DBus.Error.AccessDenied";
        assert_eq!(refusal(&denied, refused, || true), Some(Refusal::Dismissed));
        assert_eq!(refusal(&denied, refused, || false), Some(Refusal::Denied));
        let no_reply = reply("org.freedesktop.DBus.Error.NoReply");
        let timed_out = refusal(
            &no_reply,
            |_| false,
            || panic!("asking forgets a dismissal a timeout has no claim to"),
        );
        assert_eq!(timed_out, Some(Refusal::TimedOut));
    }
}
