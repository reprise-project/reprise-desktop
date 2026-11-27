//! The agent's side of polkit: registration for its own logind session and
//! `AuthenticationAgent` at `/reprise/polkit/Agent`. `BeginAuthentication` stays
//! pending until the dialog is done; zbus runs each call on its own task so
//! `CancelAuthentication` gets through. Dismissed prompts are kept for
//! [`reprisebus::polkit::dismissed`] on the session bus, where a program also
//! withdraws its own open prompts ([`reprisebus::polkit::withdraw`]).

use reprisebus::mutex::lock;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::OwnedVariant;
use zbus::message::Header;
use zbus_polkit::policykit1::Subject;

pub const PATH: &str = "/reprise/polkit/Agent";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Cancel or Esc in the prompt.
    Cancelled,
    /// polkitd's `CancelAuthentication` (the service stopped waiting), or
    /// the program the prompt is for withdrew it: nobody dismissed anything.
    Withdrawn,
    Failed,
}

pub struct Request {
    pub cookie: String,
    pub message: String,
    pub users: Vec<String>,
    pub selected: usize,
    pub done: async_channel::Sender<Outcome>,
}

pub enum Msg {
    Begin(Request),
    Cancel(String),
    /// polkitd's name changed owner: the cookies the old polkitd issued are dead.
    CancelAll,
}

pub fn user_name(uid: u32) -> Option<String> {
    passwd_name(uid, 1024)
}

/// `user_name` with a first buffer of `len` bytes, doubled on ERANGE up to 64 KiB.
fn passwd_name(uid: u32, len: usize) -> Option<String> {
    let mut buf = vec![0u8; len.max(1)];
    loop {
        // SAFETY: `libc::passwd` is a plain C struct; all-zeroes is valid.
        let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        // SAFETY: getpwuid_r writes only through the three pointers, each
        // to a live local; `buf` outlives the `pw_name` read below.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                &mut pwd,
                buf.as_mut_ptr() as *mut libc::c_char,
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 64 * 1024 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
            return None;
        }
        // SAFETY: success, so `pw_name` is NUL-terminated inside `buf`.
        return Some(
            unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }
                .to_string_lossy()
                .into_owned(),
        );
    }
}

fn current_user() -> Option<String> {
    // SAFETY: getuid has no preconditions and cannot fail.
    user_name(unsafe { libc::getuid() })
}

/// The row the combo starts on: `me` when offered, else root, else the first.
/// polkitd's first row can be another admin whose account PAM locks after
/// a few wrong passwords.
fn preselect(users: &[String], me: Option<&str>) -> usize {
    me.and_then(|me| users.iter().position(|u| u == me))
        .or_else(|| users.iter().position(|u| u == "root"))
        .unwrap_or(0)
}

fn users_of(ids: &[(String, HashMap<String, OwnedVariant>)]) -> Vec<String> {
    ids.iter()
        .filter(|(kind, _)| kind == "unix-user")
        .filter_map(|(_, d)| d.get("uid"))
        .filter_map(|v| u32::try_from(v).ok())
        .filter_map(user_name)
        .collect()
}

const POLKITD: &str = "org.freedesktop.PolicyKit1";

#[derive(zbus::DBusError, Debug)]
#[zbus(prefix = "org.freedesktop")]
enum Error {
    #[zbus(error)]
    ZBus(zbus::Error),
    #[zbus(name = "PolicyKit1.Error.Cancelled")]
    Cancelled(String),
    #[zbus(name = "PolicyKit1.Error.Failed")]
    Failed(String),
    #[zbus(name = "DBus.Error.AccessDenied")]
    AccessDenied(String),
}

async fn check_caller(conn: &zbus::Connection, hdr: &Header<'_>) -> Result<(), Error> {
    let owner = reprisebus::name_owner(conn, POLKITD).await;
    let sender = hdr.sender().map(|s| s.as_str());
    if reprisebus::sent_by(sender, owner.as_deref()) {
        return Ok(());
    }
    log::warn!("refusing an agent call from {sender:?}, which is not {POLKITD}");
    Err(Error::AccessDenied(format!(
        "only {POLKITD} may call the agent"
    )))
}

/// How long a dismissal waits for its program to ask about it.
const DISMISSAL_TTL: Duration = Duration::from_secs(30);

/// The processes whose prompt was dismissed, by the pid polkitd names as the
/// subject, each answered once through [`reprisebus::polkit::dismissed`].
#[derive(Default)]
struct Dismissed(Vec<(u32, Instant)>);

impl Dismissed {
    fn record(&mut self, pid: u32, now: Instant) {
        self.forget_old(now);
        self.0.push((pid, now));
    }

    fn take(&mut self, pid: u32, now: Instant) -> bool {
        self.forget_old(now);
        let before = self.0.len();
        self.0.retain(|(p, _)| *p != pid);
        self.0.len() != before
    }

    fn forget_old(&mut self, now: Instant) {
        self.0
            .retain(|(_, at)| now.saturating_duration_since(*at) < DISMISSAL_TTL);
    }
}

type Dismissals = Arc<Mutex<Dismissed>>;

/// How long a withdrawal as its session ends also withdraws a request polkitd sends for the same
/// pid: one already on its way from the service when the program withdrew.
const WITHDRAWN_FOR: Duration = Duration::from_secs(3);

/// The requests under way, each cookie with the pid polkitd names as its subject, and the pids
/// that withdrew their prompts, with when.
#[derive(Default)]
struct Requests {
    open: Vec<(String, u32)>,
    withdrawn: Vec<(u32, Instant)>,
}

#[derive(Clone, Default)]
struct Open(Arc<Mutex<Requests>>);

impl Open {
    /// Registers request `cookie` for `pid` from its start, before its prompt opens; None when
    /// `pid` withdrew its prompts within [`WITHDRAWN_FOR`] of `now`.
    fn add(&self, cookie: &str, pid: u32, now: Instant) -> Option<Entry> {
        let mut r = lock(&self.0);
        r.withdrawn
            .retain(|(_, at)| now.saturating_duration_since(*at) < WITHDRAWN_FOR);
        if r.withdrawn.iter().any(|(p, _)| *p == pid) {
            return None;
        }
        r.open.push((cookie.to_string(), pid));
        Some(Entry {
            open: self.clone(),
            cookie: cookie.to_string(),
        })
    }

    /// Runs `post`, which opens the prompt, unless request `cookie` was withdrawn meanwhile.
    /// Under the lock, so a withdrawal's cancel never overtakes the prompt it cancels.
    fn post_unless_withdrawn(&self, cookie: &str, post: impl FnOnce()) -> bool {
        let r = lock(&self.0);
        let open = r.open.iter().any(|(c, _)| c == cookie);
        if open {
            post();
        }
        open
    }

    /// Withdraws `pid`'s requests, running `cancel` on each open one's cookie, and when `marked`
    /// at that instant also those that arrive within [`WITHDRAWN_FOR`]. The number withdrawn.
    fn withdraw(&self, pid: u32, marked: Option<Instant>, cancel: impl Fn(String)) -> usize {
        let mut r = lock(&self.0);
        if let Some(now) = marked {
            r.withdrawn.push((pid, now));
        }
        let mut n = 0;
        r.open.retain(|(c, p)| {
            let mine = *p == pid;
            if mine {
                cancel(c.clone());
                n += 1;
            }
            !mine
        });
        n
    }
}

/// A request's place in [`Open`], left when the request ends.
struct Entry {
    open: Open,
    cookie: String,
}

impl Drop for Entry {
    fn drop(&mut self) {
        lock(&self.open.0).open.retain(|(c, _)| *c != self.cookie);
    }
}

struct Agent {
    poster: repriseapp::Poster<Msg>,
    dismissed: Dismissals,
    open: Open,
    session: String,
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl Agent {
    #[expect(clippy::too_many_arguments)]
    async fn begin_authentication(
        &self,
        action_id: &str,
        message: &str,
        _icon_name: &str,
        details: HashMap<&str, &str>,
        cookie: &str,
        identities: Vec<(String, HashMap<String, OwnedVariant>)>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        let pid: Option<u32> = details
            .get("polkit.subject-pid")
            .and_then(|p| p.parse().ok());
        let entry = match pid.map(|pid| self.open.add(cookie, pid, Instant::now())) {
            Some(None) => {
                log::info!("{action_id}: its program withdrew its prompts");
                // A retry from a session end cancelled meanwhile then comes back without a box.
                if let Some(pid) = pid {
                    lock(&self.dismissed).record(pid, Instant::now());
                }
                return Err(Error::Cancelled("withdrawn".into()));
            }
            entry => entry.flatten(),
        };
        // polkitd names the subject by pid alone, so there is no start time to pin it with.
        let cgroup =
            pid.and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok());
        if let Some(other) = (cgroup.as_deref()).and_then(|c| foreign_session(c, &self.session)) {
            log::warn!(
                "refusing {action_id}: its program belongs to session {other}, not session {}",
                self.session
            );
            return Err(Error::Failed(format!(
                "the program belongs to session {other}, not to this screen's session"
            )));
        }
        // NSS can block on the network (LDAP, SSSD); off the executor, so
        // the connection keeps serving CancelAuthentication meanwhile.
        let (users, me) = blocking::unblock(move || (users_of(&identities), current_user())).await;
        log::info!("authentication for {action_id} as one of {users:?}");
        if users.is_empty() {
            return Err(Error::Failed("no unix-user identity to offer".into()));
        }
        let selected = preselect(&users, me.as_deref());
        let (done, wait) = async_channel::bounded(1);
        let request = Request {
            cookie: cookie.to_string(),
            message: message.to_string(),
            users,
            selected,
            done,
        };
        let begin = || self.poster.post(Msg::Begin(request));
        let posted = match entry {
            Some(_) => self.open.post_unless_withdrawn(cookie, begin),
            None => {
                begin();
                true
            }
        };
        if !posted {
            return Err(Error::Cancelled("withdrawn".into()));
        }
        let outcome = wait.recv().await;
        drop(entry);
        match outcome {
            Ok(Outcome::Success) => Ok(()),
            Ok(Outcome::Cancelled) => {
                if let Some(pid) = pid {
                    lock(&self.dismissed).record(pid, Instant::now());
                }
                Err(Error::Cancelled("cancelled by the user".into()))
            }
            Ok(Outcome::Withdrawn) => Err(Error::Cancelled("withdrawn".into())),
            Ok(Outcome::Failed) | Err(_) => Err(Error::Failed("authentication failed".into())),
        }
    }

    async fn cancel_authentication(
        &self,
        cookie: &str,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        self.poster.post(Msg::Cancel(cookie.to_string()));
        Ok(())
    }
}

/// Answers [`reprisebus::polkit::dismissed`] and [`reprisebus::polkit::withdraw`] for the calling
/// process.
struct DismissalsIface {
    dismissed: Dismissals,
    open: Open,
    poster: repriseapp::Poster<Msg>,
}

async fn caller_pid(conn: &zbus::Connection, hdr: &Header<'_>) -> zbus::fdo::Result<u32> {
    let sender = hdr
        .sender()
        .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?;
    zbus::fdo::DBusProxy::new(conn)
        .await?
        .get_connection_unix_process_id(sender.clone().into())
        .await
}

#[zbus::interface(name = "reprise.polkit.Dismissals")]
impl DismissalsIface {
    async fn take(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> zbus::fdo::Result<bool> {
        let pid = caller_pid(conn, &hdr).await?;
        Ok(lock(&self.dismissed).take(pid, Instant::now()))
    }

    /// Withdraws the caller's open prompts, the number withdrawn. A program withdraws only its
    /// own, so no other program's prompt goes.
    async fn withdraw(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> zbus::fdo::Result<u32> {
        self.withdraw_for(conn, &hdr, None).await
    }

    /// [`Self::withdraw`] as the caller's session ends: prompts on their way are withdrawn too
    /// ([`WITHDRAWN_FOR`]).
    async fn withdraw_ending(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> zbus::fdo::Result<u32> {
        self.withdraw_for(conn, &hdr, Some(Instant::now())).await
    }
}

impl DismissalsIface {
    async fn withdraw_for(
        &self,
        conn: &zbus::Connection,
        hdr: &Header<'_>,
        marked: Option<Instant>,
    ) -> zbus::fdo::Result<u32> {
        let pid = caller_pid(conn, hdr).await?;
        let cancel = |cookie| self.poster.post(Msg::Cancel(cookie));
        let n = self.open.withdraw(pid, marked, cancel);
        log::info!("pid {pid} withdraws {n} prompt(s)");
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }
}

/// The logind session an agent serves, and the bus name it takes for it.
#[derive(Debug, PartialEq, Eq)]
struct Served {
    session: String,
    name: String,
}

/// The agent's `XDG_SESSION_ID`, inherited from the compositor, when polkitd judges the agent by it
/// too (`judged`). A user unit is judged by the display session, whose name it must not take.
fn served_session(xdg: Option<&str>, judged: Option<&str>) -> Result<Served, String> {
    let own = xdg
        .filter(|s| !s.is_empty())
        .ok_or("no XDG_SESSION_ID: not started in a logind session")?;
    if judged != Some(own) {
        return Err(format!(
            "polkitd judges this process by session {}, not its own session {own}",
            judged.unwrap_or("none")
        ));
    }
    let name = reprisebus::polkit::name(own)
        .ok_or_else(|| format!("session {own:?} has no reprise-polkit name"))?;
    Ok(Served {
        session: own.to_owned(),
        name,
    })
}

/// The logind session other than `session` a prompt's subject belongs to, by its cgroup: its
/// `session-<id>.scope`, or the session its Reprise app scope was launched from.
fn foreign_session<'a>(cgroup: &'a str, session: &str) -> Option<&'a str> {
    reprisebus::logind::session_of_cgroup(cgroup)
        .or_else(|| reprisefs::scope::launch_session_of_cgroup(cgroup))
        .filter(|s| *s != session)
}

/// Takes the served session's agent name, exiting when another reprise-polkit holds it.
async fn single_instance(served: &Served, iface: DismissalsIface) {
    let name = &served.name;
    let Ok(conn) = zbus::Connection::session().await else {
        return;
    };
    // Served before the name is taken, so no call to the name finds it
    // missing; a second instance exits before anyone calls it.
    if let Err(e) = conn
        .object_server()
        .at(reprisebus::polkit::PATH, iface)
        .await
    {
        log::warn!("cannot serve {}: {e}", reprisebus::polkit::INTERFACE);
    }
    let owned = conn
        .request_name_with_flags(name.as_str(), zbus::fdo::RequestNameFlags::DO_NOT_QUEUE)
        .await;
    if !matches!(owned, Ok(zbus::fdo::RequestNameReply::PrimaryOwner)) {
        log::info!("another reprise-polkit owns {name}; exiting");
        std::process::exit(0);
    }
    let _ = SESSION_BUS.set(conn);
}

static SESSION_BUS: std::sync::OnceLock<zbus::Connection> = std::sync::OnceLock::new();

async fn run(poster: repriseapp::Poster<Msg>, ready: Sender<bool>) {
    let xdg = std::env::var("XDG_SESSION_ID").ok();
    let judged = reprisebus::polkit::own_judged_session();
    let served = match served_session(xdg.as_deref(), judged.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("{e}; no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    let (dismissed, open) = (Dismissals::default(), Open::default());
    let iface = DismissalsIface {
        dismissed: dismissed.clone(),
        open: open.clone(),
        poster: poster.clone(),
    };
    single_instance(&served, iface).await;
    let session = served.session;
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    let agent = Agent {
        poster: poster.clone(),
        dismissed,
        open,
        session: session.clone(),
    };
    if let Err(e) = conn.object_server().at(PATH, agent).await {
        log::warn!("cannot serve the agent: {e}");
        let _ = ready.send(false);
        return;
    }
    let authority = match zbus_polkit::policykit1::AuthorityProxy::new(&conn).await {
        Ok(a) => a,
        Err(e) => {
            log::info!("no polkitd ({e}); no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
    keep_registered(authority.inner(), &session, &locale, ready, &poster).await;
}

/// Registers [`PATH`] as the agent for `session` with whichever polkitd owns the authority.
async fn register(authority: &zbus::Proxy<'_>, session: &str, locale: &str) -> zbus::Result<()> {
    authority
        .call::<_, _, ()>(
            "RegisterAuthenticationAgent",
            &(Subject::new_for_unix_session(session), locale, PATH),
        )
        .await
}

/// Registers once, answering `ready`, then again for every new owner of the authority's name:
/// polkitd keeps agents in memory only, so a restarted one knows none. Each change of owner posts
/// [`Msg::CancelAll`]. Returns only on failure.
async fn keep_registered(
    authority: &zbus::Proxy<'_>,
    session: &str,
    locale: &str,
    ready: Sender<bool>,
    poster: &repriseapp::Poster<Msg>,
) {
    use futures_util::StreamExt;
    // Subscribed before the first registration, so a restart in between is not missed.
    let mut owners = match authority.receive_owner_changed().await {
        Ok(o) => o,
        Err(e) => {
            log::info!("cannot follow polkitd ({e}); no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    if let Err(e) = register(authority, session, locale).await {
        log::info!("polkitd refused the agent ({e}); no polkit agent");
        let _ = ready.send(false);
        return;
    }
    log::info!("registered as the polkit agent for session {session}");
    let _ = ready.send(true);
    while let Some(owner) = owners.next().await {
        poster.post(Msg::CancelAll);
        if owner.is_none() {
            continue;
        }
        match register(authority, session, locale).await {
            Ok(()) => log::info!("registered again with the restarted polkitd"),
            Err(e) => log::warn!("the restarted polkitd refused the agent: {e}"),
        }
    }
}

pub fn start(waker: repriseapp::Waker) -> Option<Receiver<Msg>> {
    let (poster, rx) = repriseapp::Poster::channel(waker);
    let (ready_tx, ready_rx) = channel();
    std::thread::Builder::new()
        .name("reprise-polkit-bus".into())
        .spawn(move || zbus::block_on(run(poster, ready_tx)))
        .ok()?;
    ready_rx.recv().ok().filter(|ok| *ok).map(|_| rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_become_user_names() {
        let uid = |n: u32| {
            let mut m = HashMap::new();
            m.insert("uid".to_string(), OwnedVariant::from(n));
            m
        };
        let ids = vec![
            ("unix-user".to_string(), uid(0)),
            ("unix-group".to_string(), uid(0)),
            ("unix-user".to_string(), uid(u32::MAX - 7)),
        ];
        assert_eq!(users_of(&ids), vec!["root".to_string()]);
    }

    #[test]
    fn the_session_user_is_preselected_over_an_earlier_identity() {
        let me = current_user().expect("the test user has a passwd entry");
        let users = vec!["alice".to_string(), me.clone()];
        assert_eq!(preselect(&users, Some(&me)), 1);
        let users = vec!["alice".to_string(), "root".to_string()];
        assert_eq!(preselect(&users, Some("bob")), 1, "root when bob is absent");
        let users = vec!["alice".to_string(), "carol".to_string()];
        assert_eq!(preselect(&users, Some("bob")), 0, "else the first row");
    }

    #[test]
    fn a_dismissal_is_answered_once_for_its_own_pid_within_the_ttl() {
        let t0 = Instant::now();
        let mut d = Dismissed::default();
        d.record(10, t0);
        d.record(20, t0);
        assert!(!d.take(30, t0), "no prompt of pid 30 was dismissed");
        assert!(d.take(10, t0));
        assert!(!d.take(10, t0), "the answer is given once");
        assert!(
            !d.take(20, t0 + DISMISSAL_TTL),
            "a dismissal nobody asked about lapses"
        );
    }

    #[test]
    fn a_request_withdrawn_before_its_prompt_opens_never_opens() {
        let open = Open::default();
        let t0 = Instant::now();
        let entry = open.add("c", 10, t0).expect("nothing withdrawn yet");
        let cancelled = std::cell::RefCell::new(Vec::new());
        let n = open.withdraw(10, None, |c| cancelled.borrow_mut().push(c));
        assert_eq!((n, cancelled.take()), (1, vec!["c".to_string()]));
        let mut opened = false;
        assert!(!open.post_unless_withdrawn("c", || opened = true));
        assert!(!opened, "the prompt does not open after its user lookup");
        drop(entry);
    }

    #[test]
    fn a_request_on_its_way_when_its_program_withdrew_is_withdrawn_too() {
        let open = Open::default();
        let t0 = Instant::now();
        assert_eq!(open.withdraw(10, None, |_| ()), 0, "nothing open yet");
        let retry = open
            .add("retry", 10, t0)
            .expect("a plain withdrawal marks nothing");
        drop(retry);
        assert_eq!(open.withdraw(10, Some(t0), |_| ()), 0, "the session ends");
        assert!(open.add("late", 10, t0 + Duration::from_secs(1)).is_none());
        assert!(open.add("other", 20, t0).is_some(), "another program's");
        let after = t0 + WITHDRAWN_FOR;
        let entry = open.add("next", 10, after).expect("a later request opens");
        assert!(open.post_unless_withdrawn("next", || ()));
        drop(entry);
        assert!(
            !open.post_unless_withdrawn("next", || ()),
            "ended requests leave"
        );
    }

    #[test]
    fn error_names_keep_their_namespaces() {
        use zbus::DBusError;
        assert_eq!(
            Error::Cancelled(String::new()).name().as_str(),
            "org.freedesktop.PolicyKit1.Error.Cancelled"
        );
        assert_eq!(
            Error::Failed(String::new()).name().as_str(),
            "org.freedesktop.PolicyKit1.Error.Failed"
        );
        assert_eq!(
            Error::AccessDenied(String::new()).name().as_str(),
            "org.freedesktop.DBus.Error.AccessDenied"
        );
    }

    const USER_RECORD: &str = "NAME=ann\nDISPLAY=3\nSESSIONS=7 3\n";

    fn judged(cgroup: &str) -> Option<&str> {
        reprisebus::polkit::judged_session(cgroup, USER_RECORD)
    }

    #[test]
    fn the_agent_registers_for_its_own_session() {
        let tty = judged("0::/user.slice/user-1000.slice/session-7.scope\n");
        let served = |session: &str| Served {
            session: session.into(),
            name: format!("reprise.polkit.session{session}"),
        };
        assert_eq!(served_session(Some("7"), tty), Ok(served("7")));
        let subject = Subject::new_for_unix_session("7");
        assert_eq!(subject.kind(), "unix-session");
        assert_eq!(subject.session_id(), Some("7"));
        let app_scope = judged(
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-p-1.scope\n",
        );
        assert_eq!(
            served_session(Some("3"), app_scope),
            Ok(served("3")),
            "the display session's agent runs in an app scope"
        );
    }

    #[test]
    fn an_agent_outside_its_session_serves_none() {
        let unit = judged("0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.service\n");
        assert!(served_session(None, unit).is_err(), "a user unit");
        assert!(
            served_session(Some("7"), unit).is_err(),
            "polkit judges a user unit by the display session"
        );
        let tty = judged("0::/user.slice/user-1000.slice/session-7.scope\n");
        assert!(served_session(Some("3"), tty).is_err());
        assert!(served_session(Some(""), tty).is_err());
        assert!(
            served_session(Some("3.1"), Some("3.1")).is_err(),
            "an id with no bus name"
        );
    }

    #[test]
    fn a_prompt_for_a_program_of_another_session_is_refused() {
        let app = |tag: &str| {
            format!(
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-reprise-foot-{tag}1ax5x0.scope\n"
            )
        };
        let session = |id: &str| format!("0::/user.slice/user-1000.slice/session-{id}.scope\n");
        assert_eq!(
            foreign_session(&app("s7x"), "3"),
            Some("7"),
            "launched from session 7 before session 3 became the display session"
        );
        assert_eq!(foreign_session(&session("7"), "3"), Some("7"));
        assert_eq!(foreign_session(&app("s3x"), "3"), None);
        assert_eq!(foreign_session(&session("3"), "3"), None);
        assert_eq!(
            foreign_session(&app(""), "3"),
            None,
            "an app scope with no session"
        );
        let unit = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/x.service\n";
        assert_eq!(foreign_session(unit, "3"), None, "a user service");
    }

    #[test]
    fn root_has_a_name() {
        assert_eq!(user_name(0).as_deref(), Some("root"));
    }

    #[test]
    fn a_buffer_too_small_for_the_entry_grows_until_it_fits() {
        assert_eq!(passwd_name(0, 1).as_deref(), Some("root"));
    }

    mod bus {
        use std::collections::HashMap;
        use std::sync::mpsc::{Receiver, Sender, channel};
        use std::time::Duration;

        use zbus::OwnedVariant;

        use super::super::{PATH, keep_registered};

        const NAME: &str = "org.freedesktop.PolicyKit1";

        use reprisebus::testbus::bus;

        use reprisebus::testbus::connect;

        /// A registration: the subject's kind and session id, the locale and the agent's path.
        type Registration = (String, String, String, String);

        struct Authority(Sender<Registration>);

        #[zbus::interface(name = "org.freedesktop.PolicyKit1.Authority")]
        impl Authority {
            fn register_authentication_agent(
                &self,
                subject: (String, HashMap<String, OwnedVariant>),
                locale: String,
                path: String,
            ) {
                let (kind, mut details) = subject;
                let id = details
                    .remove("session-id")
                    .and_then(|v| String::try_from(v).ok())
                    .unwrap_or_default();
                let _ = self.0.send((kind, id, locale, path));
            }
        }

        /// A polkitd that takes the authority's name on `address` and reports registrations.
        async fn polkitd(address: &str, seen: Sender<Registration>) -> zbus::Connection {
            let conn = connect(address).await;
            reprisebus::testbus::serve(
                &conn,
                "/org/freedesktop/PolicyKit1/Authority",
                Authority(seen),
            )
            .await;
            conn.request_name(NAME).await.unwrap();
            conn
        }

        fn next(seen: &Receiver<Registration>, what: &str) -> Registration {
            seen.recv_timeout(Duration::from_secs(5))
                .unwrap_or_else(|_| panic!("{what}"))
        }

        #[test]
        fn a_restarted_polkitd_cancels_every_request_and_gets_the_registration_again() {
            let Some(bus) = bus() else {
                return;
            };
            let (seen_tx, seen) = channel();
            let first = zbus::block_on(polkitd(&bus.1, seen_tx.clone()));
            let address = bus.1.clone();
            let (ready_tx, ready) = channel();
            let (poster, msgs) = repriseapp::Poster::channel(repriseapp::Waker::noop());
            std::thread::spawn(move || {
                zbus::block_on(async {
                    let conn = connect(&address).await;
                    let authority = zbus_polkit::policykit1::AuthorityProxy::new(&conn)
                        .await
                        .unwrap();
                    let (session, locale) = ("7", "de_DE.UTF-8");
                    keep_registered(authority.inner(), session, locale, ready_tx, &poster).await;
                })
            });
            assert_eq!(ready.recv_timeout(Duration::from_secs(5)), Ok(true));
            let expected = (
                "unix-session".to_string(),
                "7".to_string(),
                "de_DE.UTF-8".to_string(),
                PATH.to_string(),
            );
            assert_eq!(next(&seen, "registered at start"), expected);

            assert!(msgs.try_recv().is_err(), "nothing is cancelled at start");

            zbus::block_on(first.close()).unwrap();
            let left = msgs.recv_timeout(Duration::from_secs(5));
            assert!(
                matches!(left, Ok(super::super::Msg::CancelAll)),
                "the requests of the polkitd that left are cancelled"
            );
            let _second = zbus::block_on(polkitd(&bus.1, seen_tx));
            assert_eq!(next(&seen, "registered with the new owner"), expected);
            assert!(
                seen.recv_timeout(Duration::from_millis(200)).is_err(),
                "one registration per owner"
            );
        }

        #[test]
        fn a_program_withdraws_its_own_open_prompts_and_no_other_programs() {
            let Some(bus) = bus() else {
                return;
            };
            let (poster, msgs) = repriseapp::Poster::channel(repriseapp::Waker::noop());
            let open = super::super::Open::default();
            let me = std::process::id();
            let now = std::time::Instant::now();
            let _mine = open.add("mine", me, now);
            let _theirs = open.add("theirs", me + 1, now);
            let iface = super::super::DismissalsIface {
                dismissed: Default::default(),
                open: open.clone(),
                poster,
            };
            let withdrawn: u32 = zbus::block_on(async {
                let agent = connect(&bus.1).await;
                reprisebus::testbus::serve(&agent, reprisebus::polkit::PATH, iface).await;
                let program = connect(&bus.1).await;
                let reply = program
                    .call_method(
                        agent.unique_name().map(|n| n.as_str()),
                        reprisebus::polkit::PATH,
                        Some(reprisebus::polkit::INTERFACE),
                        "Withdraw",
                        &(),
                    )
                    .await
                    .unwrap();
                reply.body().deserialize().unwrap()
            });
            assert_eq!(withdrawn, 1);
            let cancelled: Vec<String> = msgs
                .try_iter()
                .filter_map(|m| match m {
                    super::super::Msg::Cancel(cookie) => Some(cookie),
                    _ => None,
                })
                .collect();
            assert_eq!(cancelled, ["mine"]);
            assert!(
                open.post_unless_withdrawn("theirs", || ()),
                "another program's prompt stays"
            );
        }
    }
}
