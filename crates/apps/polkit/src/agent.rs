//! The agent's side of polkit: registration for this logind session and
//! `AuthenticationAgent` at `/reprise/polkit/Agent`. `BeginAuthentication` stays
//! pending until the dialog is done; zbus runs each call on its own task so
//! `CancelAuthentication` gets through. Dismissed prompts are kept for
//! [`reprisebus::polkit::dismissed`] on the session bus.

use reprisebus::mutex::lock;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zbus::message::Header;
use zbus::zvariant::OwnedValue;

pub const PATH: &str = "/reprise/polkit/Agent";
const BUS_NAME: &str = reprisebus::polkit::NAME;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    /// Cancel or Esc in the prompt.
    Cancelled,
    /// polkitd's `CancelAuthentication`: the service stopped waiting, so
    /// nobody dismissed anything.
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

fn users_of(ids: &[(String, HashMap<String, OwnedValue>)]) -> Vec<String> {
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

struct Agent {
    poster: repriseapp::Poster<Msg>,
    dismissed: Dismissals,
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
        identities: Vec<(String, HashMap<String, OwnedValue>)>,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> Result<(), Error> {
        check_caller(conn, &hdr).await?;
        // NSS can block on the network (LDAP, SSSD); off the executor, so
        // the connection keeps serving CancelAuthentication meanwhile.
        let (users, me) = blocking::unblock(move || (users_of(&identities), current_user())).await;
        log::info!("authentication for {action_id} as one of {users:?}");
        if users.is_empty() {
            return Err(Error::Failed("no unix-user identity to offer".into()));
        }
        let selected = preselect(&users, me.as_deref());
        let (done, wait) = async_channel::bounded(1);
        self.poster.post(Msg::Begin(Request {
            cookie: cookie.to_string(),
            message: message.to_string(),
            users,
            selected,
            done,
        }));
        match wait.recv().await {
            Ok(Outcome::Success) => Ok(()),
            Ok(Outcome::Cancelled) => {
                if let Some(pid) = details
                    .get("polkit.subject-pid")
                    .and_then(|p| p.parse().ok())
                {
                    lock(&self.dismissed).record(pid, Instant::now());
                }
                Err(Error::Cancelled("cancelled by the user".into()))
            }
            Ok(Outcome::Withdrawn) => Err(Error::Cancelled("withdrawn by polkitd".into())),
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

/// Answers [`reprisebus::polkit::dismissed`] for the calling process.
struct DismissalsIface(Dismissals);

#[zbus::interface(name = "reprise.polkit.Dismissals")]
impl DismissalsIface {
    async fn take(
        &self,
        #[zbus(connection)] conn: &zbus::Connection,
        #[zbus(header)] hdr: Header<'_>,
    ) -> zbus::fdo::Result<bool> {
        let sender = hdr
            .sender()
            .ok_or_else(|| zbus::fdo::Error::Failed("no sender".into()))?;
        let pid = zbus::fdo::DBusProxy::new(conn)
            .await?
            .get_connection_unix_process_id(sender.clone().into())
            .await?;
        Ok(lock(&self.0).take(pid, Instant::now()))
    }
}

/// The logind session to register for: `auto` resolves to the caller's session,
/// or the user's display session outside one.
async fn session_id(conn: &zbus::Connection) -> zbus::Result<String> {
    zbus::Proxy::new(
        conn,
        "org.freedesktop.login1",
        "/org/freedesktop/login1/session/auto",
        "org.freedesktop.login1.Session",
    )
    .await?
    .get_property::<String>("Id")
    .await
}

fn subject(session: &str) -> zbus_polkit::policykit1::Subject {
    let mut details = HashMap::new();
    details.insert(
        "session-id".to_string(),
        OwnedValue::from(zbus::zvariant::Str::from(session)),
    );
    zbus_polkit::policykit1::Subject {
        subject_kind: "unix-session".to_string(),
        subject_details: details,
    }
}

async fn single_instance(dismissed: Dismissals) {
    let Ok(conn) = zbus::Connection::session().await else {
        return;
    };
    // Served before the name is taken, so no call to the name finds it
    // missing; a second instance exits before anyone calls it.
    let iface = DismissalsIface(dismissed);
    if let Err(e) = conn
        .object_server()
        .at(reprisebus::polkit::PATH, iface)
        .await
    {
        log::warn!("cannot serve {}: {e}", reprisebus::polkit::INTERFACE);
    }
    let owned = conn
        .request_name_with_flags(BUS_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await;
    if !matches!(owned, Ok(zbus::fdo::RequestNameReply::PrimaryOwner)) {
        log::info!("another reprise-polkit owns {BUS_NAME}; exiting");
        std::process::exit(0);
    }
    let _ = SESSION_BUS.set(conn);
}

static SESSION_BUS: std::sync::OnceLock<zbus::Connection> = std::sync::OnceLock::new();

async fn run(poster: repriseapp::Poster<Msg>, ready: Sender<bool>) {
    let dismissed = Dismissals::default();
    single_instance(dismissed.clone()).await;
    let conn = match zbus::Connection::system().await {
        Ok(c) => c,
        Err(e) => {
            log::info!("no system bus ({e}); no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    let session = match session_id(&conn).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("cannot find the logind session ({e}); no polkit agent");
            let _ = ready.send(false);
            return;
        }
    };
    let agent = Agent { poster, dismissed };
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
    let subject = subject(&session);
    let locale = std::env::var("LANG").unwrap_or_else(|_| "C".into());
    if let Err(e) = authority
        .register_authentication_agent(&subject, &locale, PATH)
        .await
    {
        log::info!("polkitd refused the agent ({e}); no polkit agent");
        let _ = ready.send(false);
        return;
    }
    log::info!("registered as the polkit agent for session {session}");
    let _ = ready.send(true);
    std::future::pending::<()>().await;
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
            m.insert("uid".to_string(), OwnedValue::from(n));
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

    #[test]
    fn root_has_a_name() {
        assert_eq!(user_name(0).as_deref(), Some("root"));
    }

    #[test]
    fn a_buffer_too_small_for_the_entry_grows_until_it_fits() {
        assert_eq!(passwd_name(0, 1).as_deref(), Some("root"));
    }
}
