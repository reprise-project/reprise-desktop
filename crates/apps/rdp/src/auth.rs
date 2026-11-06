//! Logins: the session user's password, checked by the compositor's PAM conversation helper, a
//! per-address wait after repeated failures, and a pause of every login after many.

use ironrdp_server::{
    CredentialDecision, CredentialValidationError, CredentialValidator, Credentials,
};
use std::collections::{HashMap, VecDeque};
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

/// The time a connection has to log in, on top of any wait its address is serving. A password
/// check in flight when it runs out is finished first.
pub const HANDSHAKE: Duration = Duration::from_secs(10);

/// How long the helper has to give its verdict before it is killed and the check fails.
const HELPER_LIMIT: Duration = Duration::from_secs(30);

const FREE_FAILURES: u32 = 3;
const MAX_WAIT: Duration = Duration::from_secs(60);
/// How long after its last failure an address is forgotten.
const FORGET: Duration = Duration::from_secs(1200);
/// `PAUSE_AFTER` failures from any addresses within `PAUSE_WINDOW` pause every login for `PAUSE`.
const PAUSE_AFTER: usize = 10;
const PAUSE_WINDOW: Duration = Duration::from_secs(600);
const PAUSE: Duration = Duration::from_secs(600);

/// Failed logins: from the fourth an address waits, doubling up to `MAX_WAIT`, until a success
/// or `FORGET` without a failure.
/// A login counts as failed from the start of its check, so a check cut short still counts.
#[derive(Default)]
pub struct Throttle {
    peers: HashMap<IpAddr, Failures>,
    /// Failures from every address, oldest first, dropped once older than `PAUSE_WINDOW + PAUSE`.
    recent: VecDeque<Instant>,
    /// The end of the pause last logged.
    announced: Option<Instant>,
}

/// One counted login, handed back to [`Throttle::succeeded`].
pub struct Attempt(Instant);

struct Failures {
    count: u32,
    until: Instant,
}

impl Throttle {
    /// How long `peer` still has to wait before its next login is checked.
    pub fn delay(&self, peer: IpAddr) -> Duration {
        self.peers.get(&peer).map_or(Duration::ZERO, |f| {
            f.until.saturating_duration_since(Instant::now())
        })
    }

    pub fn failed(&mut self, peer: IpAddr) -> Attempt {
        let now = Instant::now();
        self.peers
            .retain(|_, f| now.duration_since(f.until - wait_after(f.count)) < FORGET);
        let f = self.peers.entry(peer).or_insert(Failures {
            count: 0,
            until: now,
        });
        f.count += 1;
        f.until = now + wait_after(f.count);
        while self
            .recent
            .front()
            .is_some_and(|&t| now.duration_since(t) >= PAUSE_WINDOW + PAUSE)
        {
            self.recent.pop_front();
        }
        self.recent.push_back(now);
        Attempt(now)
    }

    /// Takes back `attempt`'s failure and forgets `peer`'s; the failures of other logins stay.
    pub fn succeeded(&mut self, peer: IpAddr, attempt: Attempt) {
        self.peers.remove(&peer);
        if let Some(i) = self.recent.iter().rposition(|&t| t == attempt.0) {
            self.recent.remove(i);
        }
    }

    /// Whether every login is paused. Logs the start of each pause once.
    pub fn paused(&mut self) -> bool {
        let Some(until) = self.pause_end() else {
            return false;
        };
        if Instant::now() >= until {
            return false;
        }
        if self.announced != Some(until) {
            self.announced = Some(until);
            log::warn!("Remote Desktop logins are paused for 10 minutes after repeated failures");
        }
        true
    }

    /// The end of the pause started by the latest failures. No login is checked during a pause,
    /// so only the latest `PAUSE_AFTER` can have started the one still running.
    fn pause_end(&self) -> Option<Instant> {
        let n = self.recent.len();
        let first = *self.recent.get(n.checked_sub(PAUSE_AFTER)?)?;
        let last = *self.recent.back()?;
        (last.duration_since(first) < PAUSE_WINDOW).then_some(last + PAUSE)
    }

    #[cfg(test)]
    pub fn failures(&self, peer: IpAddr) -> u32 {
        self.peers.get(&peer).map_or(0, |f| f.count)
    }
}

fn wait_after(failures: u32) -> Duration {
    match failures.checked_sub(FREE_FAILURES) {
        None | Some(0) => Duration::ZERO,
        Some(n) => Duration::from_secs(1u64 << n.min(6)).min(MAX_WAIT),
    }
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
    /// `peer`'s login and the time it has to log in.
    fn check(&self, peer: SocketAddr) -> (Arc<Self::Check>, Duration);
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
}

impl Logins for PasswordLogins {
    type Check = PasswordCheck;

    fn check(&self, peer: SocketAddr) -> (Arc<PasswordCheck>, Duration) {
        let check =
            PasswordCheck::new(self.user.clone(), &self.helper, peer, self.throttle.clone());
        let budget = HANDSHAKE + self.throttle.lock().unwrap().delay(peer.ip());
        (Arc::new(check), budget)
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
        let name = match credentials.username.rsplit_once('\\') {
            Some((_, name)) => name,
            None => &credentials.username,
        };
        if name != self.user {
            log::warn!(
                "{peer}: refused a login as {name:?}; only {} can log in",
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
        // A wait cut short by a dropped connection is served before the next check.
        tokio::time::sleep(self.delay()).await;
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
        tokio::time::sleep(self.delay()).await;
        Ok(CredentialDecision::Reject)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

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
    const B: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));

    /// Serialises writing a helper script with spawning one: a fork while another script is still
    /// open for writing makes its exec fail with ETXTBSY.
    pub(crate) static SCRIPTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test(start_paused = true)]
    async fn the_throttle_waits_from_the_fourth_failure() {
        let mut t = Throttle::default();
        let mut waits = Vec::new();
        for _ in 0..10 {
            t.failed(A);
            waits.push(t.delay(A).as_secs());
        }
        assert_eq!(waits, [0, 0, 0, 2, 4, 8, 16, 32, 60, 60]);
    }

    #[tokio::test(start_paused = true)]
    async fn success_resets_the_address() {
        let mut t = Throttle::default();
        for _ in 0..4 {
            t.failed(A);
        }
        let attempt = t.failed(A);
        t.succeeded(A, attempt);
        assert_eq!(t.delay(A), Duration::ZERO);
        for _ in 0..3 {
            t.failed(A);
        }
        assert_eq!(
            t.delay(A),
            Duration::ZERO,
            "the count starts again after a success"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn addresses_are_throttled_apart() {
        let mut t = Throttle::default();
        for _ in 0..5 {
            t.failed(A);
        }
        t.failed(B);
        assert_eq!(t.delay(A), Duration::from_secs(4));
        assert_eq!(t.delay(B), Duration::ZERO);
    }

    #[tokio::test(start_paused = true)]
    async fn an_address_is_forgotten_20_minutes_after_its_last_failure() {
        let mut t = Throttle::default();
        for _ in 0..5 {
            t.failed(A);
        }
        tokio::time::advance(FORGET - Duration::from_secs(1)).await;
        t.failed(B);
        assert_eq!(t.failures(A), 5, "a failure 1 s short of FORGET is kept");
        tokio::time::advance(Duration::from_secs(1)).await;
        t.failed(B);
        assert_eq!(t.failures(A), 0);
        assert_eq!(t.failures(B), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn the_wait_runs_down() {
        let mut t = Throttle::default();
        for _ in 0..4 {
            t.failed(A);
        }
        tokio::time::advance(Duration::from_millis(1500)).await;
        assert_eq!(t.delay(A), Duration::from_millis(500));
    }

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
}
