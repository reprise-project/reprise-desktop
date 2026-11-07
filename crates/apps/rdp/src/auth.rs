//! Logins: the session user's password, checked by the compositor's PAM conversation helper, a
//! per-address back-off after repeated failures, and a pause of every login after many; or a
//! connection reprise-login logged in.

use crate::intake::Admission;
use ironrdp_server::{
    CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
};
use repriselogin::throttle::{self, Attempt, Pause};
use std::io::{self, BufRead, BufReader, Write};
use std::net::{IpAddr, SocketAddr};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::Instant;
use zeroize::Zeroizing;

/// The `reprise` beside this server (`/usr/local/bin` or `/usr/bin`), whose
/// `--rdp-password-conversation USER` reads the password from stdin and answers in lines.
pub fn helper() -> PathBuf {
    helper_beside(&std::env::current_exe().unwrap_or_default())
}

fn helper_beside(server: &Path) -> PathBuf {
    if server.parent() == Some(Path::new("/usr/local/bin")) {
        PathBuf::from("/usr/local/bin/reprise")
    } else {
        PathBuf::from("/usr/bin/reprise")
    }
}

/// The helper's environment: what PAM modules and their messages read. Everything else is dropped.
const KEPT_ENV: [&str; 5] = ["PATH", "LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES"];

/// The time a connection has to log in. A password check in flight when it runs out is finished
/// first.
pub const HANDSHAKE: Duration = Duration::from_secs(10);

/// How long the helper has to give its verdict before it is killed and the check fails.
const HELPER_LIMIT: Duration = Duration::from_secs(30);

/// [`throttle::Throttle`] on tokio's clock, so tests with paused time move it.
#[derive(Default)]
pub struct Throttle(throttle::Throttle);

impl Throttle {
    pub fn delay(&self, peer: IpAddr) -> Duration {
        self.0.delay(peer, now())
    }

    pub fn failed(&mut self, peer: IpAddr) -> Attempt {
        self.0.failed(peer, now())
    }

    pub fn succeeded(&mut self, peer: IpAddr, attempt: Attempt) {
        self.0.succeeded(peer, attempt, now())
    }

    /// Whether every login is paused. Logs the start of each pause once.
    pub fn paused(&mut self) -> bool {
        match self.0.paused(now()) {
            Pause::Off => false,
            Pause::Started => {
                log::warn!(
                    "Remote Desktop logins are paused for 10 minutes after repeated failures"
                );
                true
            }
            Pause::Running => true,
        }
    }

    #[cfg(test)]
    pub fn failures(&self, peer: IpAddr) -> u32 {
        self.0.failures(peer)
    }
}

fn now() -> std::time::Instant {
    Instant::now().into_std()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accepted,
    Rejected,
    /// PAM refused the login and said why, e.g. for an expired account.
    Refused(String),
    /// PAM asked for more than the password, which a remote login cannot answer.
    Prompted,
}

/// The verdict in the helper's output: `= 0` accepts unless a `?` or `??` prompt came first;
/// `E` lines before another code are PAM's reason.
pub fn helper_verdict(lines: &str) -> Verdict {
    let mut verdict = Verdict::Rejected;
    let mut notices = Vec::new();
    for line in lines.lines() {
        if line.starts_with('?') {
            return Verdict::Prompted;
        }
        if let Some(notice) = line.strip_prefix("E ") {
            notices.push(notice);
        }
        if let Some(rc) = line.strip_prefix("= ") {
            verdict = match rc {
                "0" => Verdict::Accepted,
                _ if notices.is_empty() => Verdict::Rejected,
                _ => Verdict::Refused(notices.join(" ")),
            };
        }
    }
    verdict
}

/// One connection's login, followed while it runs.
pub trait Login: CredentialValidator + Send + Sync + 'static {
    /// Resolves once this connection has logged in.
    fn logged_in(&self) -> impl Future<Output = ()>;
    /// Whether this connection has logged in.
    fn accepted(&self) -> bool;
    /// Whether this connection has logged in, once no check is in flight.
    fn settled(&self) -> impl Future<Output = bool>;
}

/// Makes the login of each new connection.
pub trait Logins {
    type Check: Login;
    /// `peer`'s login as `admission` asks and the time it has to log in.
    fn check(&self, peer: SocketAddr, admission: Admission) -> (Arc<Self::Check>, Duration);
    /// How long connections from `peer` are still refused after failed logins.
    fn backoff(&self, peer: IpAddr) -> Duration;
}

/// The session user's password, with one [`Throttle`] for every connection.
pub struct PasswordLogins {
    user: String,
    helper: PathBuf,
    throttle: Arc<Mutex<Throttle>>,
}

impl PasswordLogins {
    pub fn new(user: String) -> PasswordLogins {
        PasswordLogins {
            user,
            helper: helper(),
            throttle: Arc::default(),
        }
    }

    #[cfg(test)]
    pub fn sharing(user: String, throttle: Arc<Mutex<Throttle>>) -> PasswordLogins {
        PasswordLogins {
            throttle,
            ..PasswordLogins::new(user)
        }
    }
}

impl Logins for PasswordLogins {
    type Check = PasswordCheck;

    fn check(&self, peer: SocketAddr, _: Admission) -> (Arc<PasswordCheck>, Duration) {
        let check =
            PasswordCheck::new(self.user.clone(), &self.helper, peer, self.throttle.clone());
        (Arc::new(check), HANDSHAKE)
    }

    fn backoff(&self, peer: IpAddr) -> Duration {
        self.throttle.lock().unwrap().delay(peer)
    }
}

/// One connection's login check: only the session user, with their password.
pub struct PasswordCheck {
    user: String,
    helper: PathBuf,
    peer: SocketAddr,
    throttle: Arc<Mutex<Throttle>>,
    login: watch::Sender<Stage>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Waiting,
    Checking,
    Accepted,
}

/// Marks a check in flight until dropped, including when the check is cancelled.
struct InFlight<'a>(&'a watch::Sender<Stage>);

impl<'a> InFlight<'a> {
    fn start(login: &'a watch::Sender<Stage>) -> Self {
        login.send_replace(Stage::Checking);
        InFlight(login)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.send_if_modified(|l| {
            let checking = *l == Stage::Checking;
            if checking {
                *l = Stage::Waiting;
            }
            checking
        });
    }
}

impl PasswordCheck {
    /// `helper` is [`helper()`] outside tests.
    pub fn new(
        user: String,
        helper: &Path,
        peer: SocketAddr,
        throttle: Arc<Mutex<Throttle>>,
    ) -> Self {
        PasswordCheck {
            user,
            helper: helper.to_path_buf(),
            peer,
            throttle,
            login: watch::Sender::new(Stage::Waiting),
        }
    }

    fn delay(&self) -> Duration {
        self.throttle.lock().unwrap().delay(self.peer.ip())
    }

    async fn check(&self, credentials: &Credentials) -> io::Result<bool> {
        let peer = self.peer;
        // The typed name stays out of the log: it may be a password typed into the name field.
        if repriselogin::account_name(&credentials.username) != self.user {
            log::warn!(
                "{peer}: refused a login as another user; only {} can log in",
                self.user
            );
            return Ok(false);
        }
        if credentials.password.contains('\n') {
            log::warn!("{peer}: refused a password with a line break");
            return Ok(false);
        }
        let mut line = Zeroizing::new(Vec::with_capacity(credentials.password.len() + 1));
        line.extend_from_slice(credentials.password.as_bytes());
        line.push(b'\n');
        let helper = self.helper.clone();
        let user = self.user.clone();
        let attempt = self.throttle.lock().unwrap().failed(peer.ip());
        let verdict =
            tokio::task::spawn_blocking(move || converse(&helper, &user, &line, HELPER_LIMIT))
                .await
                .map_err(io::Error::other)??;
        match verdict {
            Verdict::Accepted => {
                self.throttle.lock().unwrap().succeeded(peer.ip(), attempt);
                return Ok(true);
            }
            Verdict::Rejected => log::warn!("{peer}: wrong password for {}", self.user),
            Verdict::Refused(why) => log::warn!("{peer}: refused {}: {why}", self.user),
            Verdict::Prompted => log::warn!(
                "{peer}: refused {}: PAM asks for more than the password",
                self.user
            ),
        }
        Ok(false)
    }
}

impl Login for PasswordCheck {
    async fn logged_in(&self) {
        let _ = (self.login.subscribe())
            .wait_for(|l| *l == Stage::Accepted)
            .await;
    }

    fn accepted(&self) -> bool {
        *self.login.borrow() == Stage::Accepted
    }

    async fn settled(&self) -> bool {
        let mut login = self.login.subscribe();
        login
            .wait_for(|l| *l != Stage::Checking)
            .await
            .is_ok_and(|l| *l == Stage::Accepted)
    }
}

/// Run the helper with `line` (the password and a newline) on its stdin. A further prompt gets
/// EOF, which fails the conversation, and a helper still running after `limit` is killed.
fn converse(helper: &Path, user: &str, line: &[u8], limit: Duration) -> io::Result<Verdict> {
    let mut child = Command::new(helper)
        .arg("--rdp-password-conversation")
        .arg(user)
        .env_clear()
        .envs(
            KEPT_ENV
                .iter()
                .filter_map(|k| Some((k, std::env::var_os(k)?))),
        )
        // Otherwise the helper moves its stdout, which carries the verdict, onto a journal stream.
        .env("REPRISE_LOG_STDERR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Its own group, so the watchdog also kills a PAM module's child that holds stdout open.
        .process_group(0)
        .spawn()?;
    let mut stdin = child.stdin.take();
    let stdout = child.stdout.take().ok_or(io::ErrorKind::BrokenPipe)?;
    // Only this thread reaps the child, after the watchdog ends, so a kill never hits a reused pid.
    let child = Arc::new(Mutex::new(child));
    let (answered, answer) = mpsc::channel::<()>();
    let watchdog = std::thread::spawn({
        let child = child.clone();
        move || {
            let late = answer.recv_timeout(limit) == Err(mpsc::RecvTimeoutError::Timeout);
            if late {
                let pid = child.lock().unwrap().id() as libc::pid_t;
                // SAFETY: killpg takes no pointers; the group's leader is unreaped until we return.
                unsafe { libc::killpg(pid, libc::SIGKILL) };
            }
            late
        }
    });
    if let Some(s) = stdin.as_mut() {
        s.write_all(line)?;
    }
    let mut out = String::new();
    let mut ended = false;
    for l in BufReader::new(stdout).lines() {
        let l = l?;
        if l.starts_with('?') {
            stdin = None;
        }
        ended |= l.starts_with("= ");
        out.push_str(&l);
        out.push('\n');
    }
    drop(stdin);
    drop(answered);
    let late = watchdog.join().unwrap_or(false);
    let status = child.lock().unwrap().wait()?;
    if late {
        return Err(io::Error::other(format!(
            "{} gave no verdict within {limit:?}",
            helper.display()
        )));
    }
    if !ended {
        return Err(io::Error::other(format!(
            "{} gave no verdict ({status})",
            helper.display()
        )));
    }
    Ok(helper_verdict(&out))
}

#[async_trait::async_trait]
impl CredentialValidator for PasswordCheck {
    async fn validate(
        &self,
        credentials: &Credentials,
    ) -> Result<CredentialDecision, CredentialValidationError> {
        let _in_flight = InFlight::start(&self.login);
        // Another connection from this address may have failed since this one was let in.
        let backoff = self.delay();
        if !backoff.is_zero() {
            log::info!(
                "{}: refused; {} s of back-off left",
                self.peer,
                backoff.as_secs_f32().ceil()
            );
            return Ok(CredentialDecision::Reject);
        }
        if self.throttle.lock().unwrap().paused() {
            log::info!("{}: refused; logins are paused", self.peer);
            return Ok(CredentialDecision::Reject);
        }
        let ok = self.check(credentials).await.map_err(|e| {
            log::error!("{}: cannot check the password: {e}", self.peer);
            CredentialValidationError::new(e)
        })?;
        if ok {
            self.login.send_replace(Stage::Accepted);
            log::info!("{}: logged in as {}", self.peer, self.user);
            return Ok(CredentialDecision::Accept);
        }
        // Logs a pause this failure starts.
        self.throttle.lock().unwrap().paused();
        Ok(CredentialDecision::Reject)
    }
}

/// Connections reprise-login handed over, for the session user only.
pub struct HandedOverLogins {
    user: String,
}

impl HandedOverLogins {
    pub fn new(user: String) -> HandedOverLogins {
        HandedOverLogins { user }
    }
}

impl Logins for HandedOverLogins {
    type Check = HandedOverCheck;

    fn check(&self, peer: SocketAddr, admission: Admission) -> (Arc<HandedOverCheck>, Duration) {
        let admitted = match admission {
            Admission::HandedOver { user } if user == self.user => true,
            Admission::HandedOver { user } => {
                log::warn!(
                    "{peer}: refused a hand-over for {user:?}; only {} can log in",
                    self.user
                );
                false
            }
            Admission::Password => {
                log::warn!("{peer}: refused; this server takes only hand-overs");
                false
            }
        };
        let check = HandedOverCheck {
            user: self.user.clone(),
            admitted,
            logged_in: watch::Sender::new(false),
        };
        (Arc::new(check), HANDSHAKE)
    }

    fn backoff(&self, _: IpAddr) -> Duration {
        Duration::ZERO
    }
}

/// A hand-over's login, accepted once the replayed exchange reaches its credential check. That
/// check sees the session user (bare, `DOMAIN\user` or `user@host`) with an empty password, since
/// the password never reaches this server.
pub struct HandedOverCheck {
    user: String,
    /// The hand-over named the session user.
    admitted: bool,
    logged_in: watch::Sender<bool>,
}

impl Login for HandedOverCheck {
    async fn logged_in(&self) {
        let _ = self.logged_in.subscribe().wait_for(|l| *l).await;
    }

    fn accepted(&self) -> bool {
        *self.logged_in.borrow()
    }

    async fn settled(&self) -> bool {
        self.accepted()
    }
}

#[async_trait::async_trait]
impl CredentialValidator for HandedOverCheck {
    async fn validate(
        &self,
        credentials: &Credentials,
    ) -> Result<CredentialDecision, CredentialValidationError> {
        let ok = self.admitted
            && repriselogin::account_name(&credentials.username) == self.user
            && credentials.password.is_empty();
        if !ok {
            return Ok(CredentialDecision::Reject);
        }
        self.logged_in.send_replace(true);
        Ok(CredentialDecision::Accept)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use repriselogin::throttle::{FREE_FAILURES, PAUSE, PAUSE_AFTER};

    #[test]
    fn the_helper_is_the_reprise_beside_the_server() {
        let helper = |server: &str| helper_beside(Path::new(server));
        assert_eq!(
            helper("/usr/local/bin/reprise-rdp"),
            Path::new("/usr/local/bin/reprise")
        );
        for server in [
            "/usr/bin/reprise-rdp",
            "/home/x/target/debug/reprise-rdp",
            "",
        ] {
            assert_eq!(helper(server), Path::new("/usr/bin/reprise"), "{server:?}");
        }
    }
    use std::net::Ipv4Addr;
    use std::os::unix::fs::PermissionsExt;

    const A: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));

    /// Serialises writing a helper script with spawning one: a fork while another script is still
    /// open for writing makes its exec fail with ETXTBSY.
    pub(crate) static SCRIPTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn the_helper_lines_give_the_verdict() {
        assert_eq!(helper_verdict("= 0\n"), Verdict::Accepted);
        assert_eq!(helper_verdict("= 7\n"), Verdict::Rejected);
        assert_eq!(
            helper_verdict("E Your account has expired.\nE Ask the admin.\n= 7\n"),
            Verdict::Refused("Your account has expired. Ask the admin.".into())
        );
        assert_eq!(helper_verdict("? Verification code:\n"), Verdict::Prompted);
        assert_eq!(
            helper_verdict("? Verification code:\n= 0\n"),
            Verdict::Prompted
        );
        assert_eq!(helper_verdict(""), Verdict::Rejected);
    }

    /// An executable `helper` in `dir` running `body` under sh. Hold [`SCRIPTS`] from writing it
    /// until it has run.
    pub(crate) fn script(dir: &Path, body: &str) -> PathBuf {
        let path = dir.join("helper");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    /// A helper that records its arguments and two variables in `args` and accepts the password
    /// `secret`.
    fn helper(dir: &Path) -> PathBuf {
        script(
            dir,
            &format!(
                "echo \"$@ $REPRISE_LOG_STDERR ${{HOME:-no-home}}\" > '{}'\nread -r pw\n[ \"$pw\" = secret ] && echo '= 0' || echo '= 7'\n",
                dir.join("args").display()
            ),
        )
    }

    /// A helper that creates `started` in `dir`, then rejects the password after 300 ms.
    pub(crate) fn slow_helper(dir: &Path) -> PathBuf {
        script(
            dir,
            &format!(
                "read -r pw\ntouch '{}'\nsleep 0.3\necho '= 7'\n",
                dir.join("started").display()
            ),
        )
    }

    /// Resolves once `slow_helper` in `dir` has started.
    pub(crate) async fn started(dir: &Path) {
        while !dir.join("started").exists() {
            tokio::task::yield_now().await;
        }
    }

    pub(crate) fn credentials(username: &str, password: &str) -> Credentials {
        Credentials {
            username: username.into(),
            password: password.into(),
            domain: None,
        }
    }

    async fn validate(dir: &Path, username: &str, password: &str) -> CredentialDecision {
        validate_from(dir, &Arc::default(), username, password).await
    }

    async fn validate_from(
        dir: &Path,
        throttle: &Arc<Mutex<Throttle>>,
        username: &str,
        password: &str,
    ) -> CredentialDecision {
        let _scripts = SCRIPTS.lock().await;
        let check = PasswordCheck::new(
            "alice".into(),
            &helper(dir),
            SocketAddr::new(A, 50000),
            throttle.clone(),
        );
        let decision = check
            .validate(&credentials(username, password))
            .await
            .unwrap();
        assert_eq!(check.accepted(), decision == CredentialDecision::Accept);
        decision
    }

    #[tokio::test]
    async fn another_user_is_rejected_without_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate(dir.path(), "root", "secret").await,
            CredentialDecision::Reject
        );
        assert!(!dir.path().join("args").exists(), "the helper must not run");
    }

    #[tokio::test]
    async fn a_refused_user_name_stays_out_of_the_log() {
        crate::listen::tests::capture_log();
        let dir = tempfile::tempdir().unwrap();
        let typed = "hunter2-typed-as-a-name";
        assert_eq!(
            validate(dir.path(), typed, "secret").await,
            CredentialDecision::Reject
        );
        assert!(crate::listen::tests::logged(
            "192.0.2.1:50000: refused a login as another user; only alice can log in"
        ));
        assert!(!crate::listen::tests::logged_containing(typed));
    }

    #[tokio::test]
    async fn the_user_name_is_case_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate(dir.path(), "Alice", "secret").await,
            CredentialDecision::Reject
        );
        assert!(!dir.path().join("args").exists(), "the helper must not run");
    }

    #[tokio::test]
    async fn a_domain_prefix_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate(dir.path(), "WORK\\alice", "secret").await,
            CredentialDecision::Accept
        );
        let args = std::fs::read_to_string(dir.path().join("args")).unwrap();
        assert_eq!(args, "--rdp-password-conversation alice 1 no-home\n");
    }

    #[tokio::test]
    async fn a_helper_without_a_verdict_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = SCRIPTS.lock().await;
        let path = dir.path().join("helper");
        std::fs::write(&path, "#!/bin/sh\nexit 2\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        let check = PasswordCheck::new(
            "alice".into(),
            &path,
            SocketAddr::new(A, 50000),
            throttle.clone(),
        );
        assert!(
            check
                .validate(&credentials("alice", "secret"))
                .await
                .is_err()
        );
        assert_eq!(
            throttle.lock().unwrap().failures(A),
            1,
            "counted like a wrong password"
        );
    }

    #[test]
    fn a_helper_that_hangs_is_killed_and_fails_the_check() {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = SCRIPTS.blocking_lock();
        let path = script(dir.path(), "read -r pw\nexec sleep 60\n");
        let started = std::time::Instant::now();
        let e = converse(&path, "alice", b"secret\n", Duration::from_millis(200)).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "killed, not waited for"
        );
        assert!(
            e.to_string().ends_with("gave no verdict within 200ms"),
            "{e}"
        );
    }

    #[test]
    fn a_hung_helper_s_child_holding_stdout_is_killed_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = SCRIPTS.blocking_lock();
        let path = script(dir.path(), "read -r pw\nsleep 60 &\nexec sleep 60\n");
        let started = std::time::Instant::now();
        let e = converse(&path, "alice", b"secret\n", Duration::from_millis(200)).unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the background sleep is killed too"
        );
        assert!(
            e.to_string().ends_with("gave no verdict within 200ms"),
            "{e}"
        );
    }

    #[tokio::test]
    async fn an_account_pam_refuses_logs_pams_reason() {
        crate::listen::tests::capture_log();
        let dir = tempfile::tempdir().unwrap();
        let _scripts = SCRIPTS.lock().await;
        let path = script(
            dir.path(),
            "read -r pw\necho 'E Your account has expired.'\necho '= 7'\n",
        );
        let check = PasswordCheck::new(
            "alice".into(),
            &path,
            SocketAddr::new(A, 50000),
            Arc::default(),
        );
        let decision = check
            .validate(&credentials("alice", "secret"))
            .await
            .unwrap();
        assert_eq!(decision, CredentialDecision::Reject);
        assert!(crate::listen::tests::logged(
            "192.0.2.1:50000: refused alice: Your account has expired."
        ));
    }

    #[tokio::test]
    async fn a_wrong_password_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            validate(dir.path(), "alice", "guess").await,
            CredentialDecision::Reject
        );
    }

    #[tokio::test]
    async fn a_check_cut_short_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let _scripts = SCRIPTS.lock().await;
        let throttle = Arc::<Mutex<Throttle>>::default();
        let check = PasswordCheck::new(
            "alice".into(),
            &slow_helper(dir.path()),
            SocketAddr::new(A, 50000),
            throttle.clone(),
        );
        let guess = credentials("alice", "guess");
        tokio::select! {
            _ = check.validate(&guess) => panic!("the helper takes 300 ms"),
            () = started(dir.path()) => {}
        }
        assert_eq!(throttle.lock().unwrap().failures(A), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn an_address_in_back_off_is_rejected_at_once_without_the_helper() {
        let dir = tempfile::tempdir().unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        for _ in 0..FREE_FAILURES + 1 {
            throttle.lock().unwrap().failed(A);
        }
        let started = Instant::now();
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "secret").await,
            CredentialDecision::Reject
        );
        assert_eq!(started.elapsed(), Duration::ZERO, "no wait is served");
        assert!(!dir.path().join("args").exists(), "the helper must not run");
        assert_eq!(
            throttle.lock().unwrap().failures(A),
            4,
            "no failure is added"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_failure_that_starts_a_back_off_is_rejected_at_once() {
        let dir = tempfile::tempdir().unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        for _ in 0..FREE_FAILURES {
            throttle.lock().unwrap().failed(A);
        }
        let started = Instant::now();
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "guess").await,
            CredentialDecision::Reject
        );
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "the back-off is not waited out"
        );
        assert_eq!(throttle.lock().unwrap().delay(A), Duration::from_secs(2));
    }

    /// `PAUSE_AFTER` failures, each from its own address.
    fn spread_failures(throttle: &Arc<Mutex<Throttle>>, n: usize) {
        let mut t = throttle.lock().unwrap();
        for i in 0..n {
            t.failed(IpAddr::V4(Ipv4Addr::new(198, 51, 100, i as u8)));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn the_eleventh_login_in_10_minutes_is_refused_without_the_helper() {
        crate::listen::tests::capture_log();
        let dir = tempfile::tempdir().unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        spread_failures(&throttle, PAUSE_AFTER);
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "secret").await,
            CredentialDecision::Reject
        );
        assert!(!dir.path().join("args").exists(), "the helper must not run");
        assert!(crate::listen::tests::logged(
            "Remote Desktop logins are paused for 10 minutes after repeated failures"
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn logins_work_again_10_minutes_after_the_pause_starts() {
        let dir = tempfile::tempdir().unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        spread_failures(&throttle, PAUSE_AFTER);
        tokio::time::advance(PAUSE - Duration::from_secs(1)).await;
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "secret").await,
            CredentialDecision::Reject
        );
        tokio::time::advance(Duration::from_secs(1)).await;
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "secret").await,
            CredentialDecision::Accept
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_success_keeps_the_failures_of_other_logins() {
        let dir = tempfile::tempdir().unwrap();
        let throttle = Arc::<Mutex<Throttle>>::default();
        spread_failures(&throttle, PAUSE_AFTER - 1);
        assert_eq!(
            validate_from(dir.path(), &throttle, "alice", "secret").await,
            CredentialDecision::Accept
        );
        assert!(
            !throttle.lock().unwrap().paused(),
            "a success is no failure"
        );
        throttle
            .lock()
            .unwrap()
            .failed(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1)));
        assert!(throttle.lock().unwrap().paused());
    }
    const B: SocketAddr = SocketAddr::new(A, 50000);

    async fn decide(check: &HandedOverCheck, username: &str, password: &str) -> CredentialDecision {
        check
            .validate(&credentials(username, password))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn a_handed_over_login_is_accepted_at_its_replayed_check_for_the_session_user() {
        let logins = HandedOverLogins::new("anna".into());
        let admission = Admission::HandedOver {
            user: "anna".into(),
        };
        let (check, budget) = logins.check(B, admission);
        assert_eq!(budget, HANDSHAKE);
        assert_eq!(logins.backoff(A), Duration::ZERO);
        assert!(!check.accepted(), "not before the replay reaches its check");
        assert!(!check.settled().await);
        assert_eq!(decide(&check, "bert", "").await, CredentialDecision::Reject);
        assert!(!check.accepted(), "nor after a refused one");
        assert_eq!(decide(&check, "anna", "").await, CredentialDecision::Accept);
        tokio::time::timeout(Duration::ZERO, check.logged_in())
            .await
            .expect("logged in at the check");
        assert!(check.accepted());
        assert!(check.settled().await);
        assert_eq!(
            decide(&check, "DOM\\anna", "").await,
            CredentialDecision::Accept
        );
        assert_eq!(decide(&check, "bert", "").await, CredentialDecision::Reject);
        assert_eq!(
            decide(&check, "anna", "x").await,
            CredentialDecision::Reject
        );
    }

    #[tokio::test]
    async fn a_hand_over_takes_the_session_user_in_upn_form() {
        let logins = HandedOverLogins::new("anna".into());
        let admission = Admission::HandedOver {
            user: "anna".into(),
        };
        let (check, _) = logins.check(B, admission);
        for other in ["bert@host", "@host", "annabel@host"] {
            assert_eq!(
                decide(&check, other, "").await,
                CredentialDecision::Reject,
                "{other}"
            );
        }
        for upn in ["anna@host", "anna@"] {
            assert_eq!(
                decide(&check, upn, "").await,
                CredentialDecision::Accept,
                "{upn}: the front logs it on as anna"
            );
        }
    }

    #[tokio::test]
    async fn a_hand_over_for_another_user_or_a_password_login_is_refused() {
        let logins = HandedOverLogins::new("anna".into());
        let other = Admission::HandedOver {
            user: "bert".into(),
        };
        for admission in [other, Admission::Password] {
            let (check, _) = logins.check(B, admission.clone());
            assert!(!check.accepted(), "{admission:?}");
            assert!(!check.settled().await, "{admission:?}");
            assert!(
                tokio::time::timeout(Duration::from_millis(50), check.logged_in())
                    .await
                    .is_err(),
                "{admission:?} never logs in"
            );
            assert_eq!(decide(&check, "anna", "").await, CredentialDecision::Reject);
        }
    }
}
