//! One connection from the TCP accept to its Client Info PDU: X.224, TLS over a `Cork`, MCS and
//! the channel joins, each acceptor step recorded for the server that takes over.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use ironrdp_acceptor::{
    Acceptor, ConnectionSetupHandler, ConnectorError, ConnectorErrorExt as _, ConnectorResult,
    CredentialOrigin, CredsspAuthIdentity, DesktopSize, ReceivedCredentials,
};
use ironrdp_connector::sspi::{NetworkRequest, Username};
use ironrdp_connector::{ConnectorErrorKind, Sequence, ServerName};
use ironrdp_core::WriteBuf;
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::x224::X224;
use ironrdp_tokio::bytes::{Bytes, BytesMut};
use ironrdp_tokio::{FramedWrite, LocalTokioFramed, NetworkClient, TokioFramed};
use repriselogin::account::{nt_hash, same_hash};
use repriselogin::front::{FrontEvent, FrontRequest, NlaUser, Secret};
use repriselogin::handover::Step;
use repriselogin::throttle::Attempt;
use reprisetls::cert::Identity;
use reprisetls::cork::Cork;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::time::Instant;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::KeyLog;
use tokio_rustls::server::TlsStream;
use zeroize::{Zeroize, Zeroizing};

use crate::limits::{HANDSHAKE, MAX_PREAUTH_BYTES, MAX_RECORDING, MAX_USER, NlaThrottle, Refusal};
use crate::link::LinkSender;
use crate::negotiate;

const DESKTOP: DesktopSize = DesktopSize {
    width: 1024,
    height: 768,
};
/// For the access-denied PDU and close_notify to leave.
const FAREWELL: Duration = Duration::from_secs(2);
const CLIENT_TRAFFIC: &str = "CLIENT_TRAFFIC_SECRET_0";
const SERVER_TRAFFIC: &str = "SERVER_TRAFFIC_SECRET_0";
const NLA_OR_RDP: &str = "the client requires NLA or RDP security; choose TLS security in the client, or set a Remote Desktop password in Settings";
/// sspi's NTLM takes a password in this form as its NT hash.
const NT_HASH_PREFIX: &str = "$NTLM$:";

pub type ClientTls = TlsStream<Cork<Limited<TcpStream>>>;

/// What the front shares between its connections.
pub struct Ctx {
    pub identity: Identity,
    /// The users with a Remote Desktop password, as reprise-login last sent them.
    pub nla_users: Rc<RefCell<Vec<NlaUser>>>,
    /// `None` while reprise-login is out of reach.
    pub link: RefCell<Option<LinkSender>>,
    /// The machine's host name. The unit's ProtectHostname= hides later changes, so a new name
    /// applies when the front restarts.
    pub host: String,
    pub nla_throttle: RefCell<NlaThrottle>,
}

/// A connection that sent its credentials, waiting for reprise-login's verdict.
pub struct Pending {
    pub conn: u64,
    pub peer: SocketAddr,
    pub security: SecurityProtocol,
    /// The Client Info PDU, last, has its password blanked.
    pub steps: Vec<Step>,
    pub user: String,
    /// Empty under NLA, whose `Authenticate` went out after CredSSP.
    pub password: Zeroizing<String>,
    /// Logged on with Network Level Authentication.
    pub nla: bool,
    pub tls: ClientTls,
    /// Plaintext read past the Client Info PDU.
    pub leftover: BytesMut,
    pub acceptor: Acceptor,
    pub key_log: Arc<ConnKeyLog>,
}

impl fmt::Debug for Pending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pending")
            .field("conn", &self.conn)
            .field("peer", &self.peer)
            .field("security", &self.security)
            .field("steps", &self.steps)
            .field("user", &self.user)
            .field("password", &format_args!("<redacted>"))
            .field("nla", &self.nla)
            .field("leftover", &self.leftover.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refused {
    Deadline,
    TooLong,
    Negotiation(String),
    Tls(String),
    NoPassword,
    NoUser,
    UserTooLong,
    Io(io::ErrorKind),
    Nla(String),
    NoDelegation,
    /// The Client Info PDU names another user than CredSSP authenticated.
    UserChanged,
    /// reprise-login refused the delegated password before HYBRID_EX's result.
    Denied {
        backoff_s: u32,
    },
    /// The delegated user and password are not a listed user and that user's Remote Desktop
    /// password.
    DelegationMismatch,
    /// reprise-login did not verify the delegated password within `HANDSHAKE`.
    NoVerdict,
    /// Refused at CredSSP's start or at the client's NTLM AUTHENTICATE after failed passwords.
    Throttled(Refusal),
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refused::Deadline => write!(f, "no logon info within {} s", HANDSHAKE.as_secs()),
            Refused::TooLong => f.write_str("the client sent too much before logging on"),
            Refused::Negotiation(why) => f.write_str(why),
            Refused::Tls(e) => write!(f, "TLS handshake failed: {e}"),
            Refused::NoPassword => f.write_str(
                "no password in the client's logon info; save the user name and password in the client",
            ),
            Refused::NoUser => f.write_str("no user name in the client's logon info"),
            Refused::UserTooLong => write!(f, "a user name over {MAX_USER} bytes"),
            Refused::Io(kind) => write!(f, "connection error: {kind}"),
            Refused::Nla(e) => write!(f, "Network Level Authentication failed: {e}"),
            Refused::NoDelegation => f.write_str(
                "the client sent no password (Restricted Admin or Remote Credential Guard), which Reprise cannot use",
            ),
            Refused::UserChanged => {
                f.write_str("the logon info names another user than the one authenticated")
            }
            Refused::Denied { .. } => f.write_str("reprise-login refused the logon"),
            Refused::DelegationMismatch => f.write_str(
                "the client delegated another user or password than its Remote Desktop password",
            ),
            Refused::NoVerdict => write!(
                f,
                "reprise-login did not verify the password within {} s",
                HANDSHAKE.as_secs()
            ),
            Refused::Throttled(refusal) => refusal.fmt(f),
        }
    }
}

/// Keeps a connection's TLS 1.3 traffic secrets, which a key update derives the next keys from.
#[derive(Default)]
pub struct ConnKeyLog {
    client: Mutex<Option<Zeroizing<Vec<u8>>>>,
    server: Mutex<Option<Zeroizing<Vec<u8>>>>,
}

impl ConnKeyLog {
    pub fn client_secret(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.client
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn server_secret(&self) -> Option<Zeroizing<Vec<u8>>> {
        self.server
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn slot(&self, label: &str) -> Option<&Mutex<Option<Zeroizing<Vec<u8>>>>> {
        match label {
            CLIENT_TRAFFIC => Some(&self.client),
            SERVER_TRAFFIC => Some(&self.server),
            _ => None,
        }
    }
}

impl fmt::Debug for ConnKeyLog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConnKeyLog(<redacted>)")
    }
}

impl KeyLog for ConnKeyLog {
    fn log(&self, label: &str, _client_random: &[u8], secret: &[u8]) {
        if let Some(slot) = self.slot(label) {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(Zeroizing::new(secret.to_vec()));
        }
    }

    fn will_log(&self, label: &str) -> bool {
        self.slot(label).is_some()
    }
}

/// A reader that fails with `InvalidData` once `MAX_PREAUTH_BYTES` have been read.
pub struct Limited<S> {
    inner: S,
    left: usize,
}

#[derive(Debug)]
struct OverLimit;

impl fmt::Display for OverLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "over {MAX_PREAUTH_BYTES} bytes before logging on")
    }
}

impl std::error::Error for OverLimit {}

fn over_limit(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<OverLimit>())
}

impl<S> Limited<S> {
    pub fn new(inner: S) -> Limited<S> {
        Limited {
            inner,
            left: MAX_PREAUTH_BYTES,
        }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Limited<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if this.left == 0 {
            let mut probe = [0u8; 1];
            let mut probe = ReadBuf::new(&mut probe);
            ready!(Pin::new(&mut this.inner).poll_read(cx, &mut probe))?;
            return Poll::Ready(match probe.filled().len() {
                0 => Ok(()),
                _ => Err(io::Error::new(io::ErrorKind::InvalidData, OverLimit)),
            });
        }
        let n = this.left.min(buf.remaining());
        let mut part = ReadBuf::new(buf.initialize_unfilled_to(n));
        ready!(Pin::new(&mut this.inner).poll_read(cx, &mut part))?;
        let got = part.filled().len();
        this.left -= got;
        buf.advance(got);
        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Limited<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Runs `tcp` to its Client Info PDU within `HANDSHAKE`. Under NLA it sends `Authenticate`
/// after CredSSP; `heard` carries reprise-login's events for `conn`.
pub async fn to_client_info(
    tcp: TcpStream,
    conn: u64,
    accepted: Instant,
    ctx: &Ctx,
    heard: &mut UnboundedReceiver<FrontEvent>,
) -> Result<Pending, Refused> {
    let peer = tcp.peer_addr().map_err(|e| Refused::Io(e.kind()))?;
    let attempt = Cell::default();
    let link = Link {
        ctx,
        conn,
        peer,
        accepted,
        heard,
        asked: false,
        attempt: &attempt,
    };
    tokio::time::timeout(HANDSHAKE, accept(tcp, link))
        .await
        .unwrap_or(Err(Refused::Deadline))
}

/// One connection's side of the link. Dropped after its `Authenticate` with no verdict yet,
/// it tells reprise-login the connection is gone.
struct Link<'a> {
    ctx: &'a Ctx,
    conn: u64,
    peer: SocketAddr,
    accepted: Instant,
    heard: &'a mut UnboundedReceiver<FrontEvent>,
    asked: bool,
    /// The password check counted as failed until the delegated password is listed.
    attempt: &'a Cell<Option<Attempt>>,
}

impl Drop for Link<'_> {
    fn drop(&mut self) {
        // A failed check may have started a pause, which is logged once.
        if self.attempt.take().is_some()
            && let Ok(mut throttle) = self.ctx.nla_throttle.try_borrow_mut()
        {
            throttle.paused();
        }
        if self.asked
            && let Ok(link) = self.ctx.link.try_borrow()
            && let Some(link) = &*link
        {
            let _ = link.send(&FrontRequest::Gone { conn: self.conn }, None);
        }
    }
}

async fn accept(tcp: TcpStream, mut link: Link<'_>) -> Result<Pending, Refused> {
    let (ctx, conn, peer) = (link.ctx, link.conn, link.peer);
    let mut framed = TokioFramed::new(Limited::new(tcp));
    let request = framed
        .read_by_hint(&ironrdp_pdu::X224_HINT)
        .await
        .map_err(read_error)?;
    let (offered, cookie) = negotiate::peek_request(&request)
        .ok_or_else(|| Refused::Negotiation("not an X.224 Connection Request".into()))?;
    let names: Vec<String> = (ctx.nla_users.borrow().iter())
        .map(|u| u.user.clone())
        .collect();
    let chosen = negotiate::choose_protocol(offered, cookie.as_deref(), &names);
    // With no protocol to agree on, an SSL acceptor still answers RDP_NEG_FAILURE.
    let mut acceptor = Acceptor::new(
        chosen.unwrap_or(SecurityProtocol::SSL),
        DESKTOP,
        Vec::new(),
        None,
    );
    let mut steps = Vec::new();
    step(&mut framed, &mut acceptor, &mut steps, Some(request)).await?;
    let Some(security) = chosen else {
        let _ = step(&mut framed, &mut acceptor, &mut steps, None).await;
        return Err(Refused::Negotiation(NLA_OR_RDP.into()));
    };
    while acceptor.reached_security_upgrade().is_none() {
        step(&mut framed, &mut acceptor, &mut steps, None).await?;
    }
    let (limited, early) = framed.into_inner();
    if !early.is_empty() {
        return Err(Refused::Negotiation("data before the TLS handshake".into()));
    }

    let key_log = Arc::new(ConnKeyLog::default());
    let config = (ctx.identity).server_config(true, Some(key_log.clone()));
    let tls = TlsAcceptor::from(config)
        .accept(Cork::new(limited))
        .await
        .map_err(|e| match over_limit(&e) {
            true => Refused::TooLong,
            false => Refused::Tls(e.to_string()),
        })?;
    acceptor.mark_security_upgrade_as_done();
    steps.push(Step::SecurityUpgradeDone);

    let mut framed = TokioFramed::new(tls);
    let nla_user = match acceptor.should_perform_credssp() {
        true => Some(credssp(&mut framed, &mut acceptor, &mut steps, &mut link).await?),
        false => None,
    };
    while acceptor.client_info_flags().is_none() {
        step(&mut framed, &mut acceptor, &mut steps, None).await?;
    }
    let nla = nla_user.is_some();
    let (user, password) = match nla_user {
        Some(user) => {
            let info = recorded_client_info(&steps)?;
            if negotiate::strip_domain(&info.client_info.credentials.username) != user {
                return Err(Refused::UserChanged);
            }
            (user, Zeroizing::new(String::new()))
        }
        None => {
            let credentials = match acceptor.received_credentials() {
                Some(c) if c.origin == CredentialOrigin::ClientInfo => &c.credentials,
                _ => return Err(Refused::NoPassword),
            };
            let user = checked_user(&credentials.username)?;
            if credentials.password.is_empty() {
                return Err(Refused::NoPassword);
            }
            (user, Zeroizing::new(credentials.password.clone()))
        }
    };
    if let Some(Step::Pdu { input, .. }) = steps.last_mut() {
        let blanked = blank_password(input).map_err(|e| Refused::Negotiation(e.to_string()))?;
        *input = Zeroizing::new(blanked);
    }
    check_recording(&steps)?;
    let (tls, leftover) = framed.into_inner();
    link.asked = false;
    Ok(Pending {
        conn,
        peer,
        security,
        steps,
        user,
        password,
        nla,
        tls,
        leftover,
        acceptor,
        key_log,
    })
}

/// A user name with its domain stripped, refused when empty or too long.
fn checked_user(name: &str) -> Result<String, Refused> {
    let user = negotiate::strip_domain(name);
    if user.is_empty() {
        return Err(Refused::NoUser);
    }
    if user.len() > MAX_USER {
        return Err(Refused::UserTooLong);
    }
    Ok(user.to_owned())
}

/// The Client Info PDU, the last step recorded.
fn recorded_client_info(steps: &[Step]) -> Result<ClientInfoPdu, Refused> {
    let Some(Step::Pdu { input, .. }) = steps.last() else {
        return Err(Refused::Negotiation("no Client Info PDU".into()));
    };
    let (_, info) = read_client_info(input).map_err(|e| Refused::Negotiation(e.to_string()))?;
    Ok(info)
}

/// Runs CredSSP against the Remote Desktop passwords and sends reprise-login the delegated
/// password; HYBRID_EX's result waits for reprise-login to verify it. Returns the user.
async fn credssp(
    framed: &mut TokioFramed<ClientTls>,
    acceptor: &mut Acceptor,
    steps: &mut Vec<Step>,
    link: &mut Link<'_>,
) -> Result<String, Refused> {
    let ctx = link.ctx;
    (ctx.nla_throttle.borrow_mut().check(link.peer.ip())).map_err(Refused::Throttled)?;
    acceptor.set_credssp_users(credssp_users(&ctx.nla_users.borrow(), &ctx.host));
    let public_key = ctx.identity.public_key();
    let (tls, leftover) = framed.get_inner_mut();
    let throttled = Cell::new(None);
    let proofs = Proofs {
        tls,
        tail: Vec::new(),
        seen: false,
        throttle: &ctx.nla_throttle,
        ip: link.peer.ip(),
        attempt: link.attempt,
        throttled: &throttled,
    };
    let mut client = LocalTokioFramed::new_with_leftover(proofs, std::mem::take(leftover));
    let mut verify = Verify {
        link: &mut *link,
        user: None,
        refused: None,
    };
    let done = ironrdp_acceptor::accept_credssp_with(
        &mut client,
        acceptor,
        &mut NoNetwork,
        ServerName::new(ctx.host.as_str()),
        public_key,
        None,
        &mut verify,
    )
    .await;
    *leftover = client.into_inner().1;
    if let Some(refusal) = throttled.take() {
        return Err(Refused::Throttled(refusal));
    }
    let (user, refused) = (verify.user, verify.refused);
    if let Some(refused) = refused {
        return Err(refused);
    }
    done.map_err(|e| Refused::Nla(e.to_string()))?;
    let user = match user {
        Some(user) => user,
        None => link.delegate(acceptor.received_credentials())?,
    };
    let Some(ReceivedCredentials { credentials, .. }) = acceptor.received_credentials() else {
        return Err(Refused::NoDelegation);
    };
    steps.push(Step::CredsspDone {
        user: credentials.username.clone(),
        domain: credentials.domain.clone(),
    });
    Ok(user)
}

impl Link<'_> {
    /// Sends the password CredSSP delegated to reprise-login. sspi does not say which user NTLM
    /// authenticated, so the delegated pair must be a listed user and its listed password.
    fn delegate(&mut self, received: Option<&ReceivedCredentials>) -> Result<String, Refused> {
        let credentials = match received {
            Some(c) if c.origin == CredentialOrigin::CredSspDelegated => &c.credentials,
            _ => return Err(Refused::NoDelegation),
        };
        if credentials.password.is_empty() {
            return Err(Refused::NoDelegation);
        }
        let user = checked_user(&credentials.username)?;
        let hash = nt_hash(&credentials.password);
        let listed = (self.ctx.nla_users.borrow().iter())
            .any(|u| u.user == user && same_hash(&u.nt_hash, &hash));
        if !listed {
            return Err(Refused::DelegationMismatch);
        }
        if let Some(attempt) = self.attempt.take() {
            (self.ctx.nla_throttle.borrow_mut()).succeeded(self.peer.ip(), attempt);
        }
        let secret = Secret::Delegated(Zeroizing::new(credentials.password.clone()));
        authenticate(
            self.ctx,
            self.conn,
            self.peer,
            self.accepted,
            user.clone(),
            secret,
        )
        .map_err(|e| Refused::Io(e.kind()))?;
        self.asked = true;
        Ok(user)
    }

    /// Waits for `Verified`, or `Denied`, until `HANDSHAKE` after the accept.
    async fn verified(&mut self) -> Result<(), Refused> {
        let deadline = self.accepted + HANDSHAKE;
        loop {
            let event = tokio::time::timeout_at(deadline, self.heard.recv()).await;
            match event.map_err(|_| Refused::NoVerdict)? {
                Some(FrontEvent::Verified { .. }) => return Ok(()),
                Some(FrontEvent::Denied { backoff_s, .. }) => {
                    self.asked = false;
                    return Err(Refused::Denied { backoff_s });
                }
                Some(_) => {}
                None => return Err(Refused::Io(io::ErrorKind::NotConnected)),
            }
        }
    }
}

/// NTLM's signature and AUTHENTICATE's message type, which sspi reads before the proof.
const AUTHENTICATE: &[u8] = b"NTLMSSP\0\x03\0\0\0";

/// The TLS stream while CredSSP runs. A failed password counts from the client's NTLM
/// AUTHENTICATE, the first message with a password proof; a client that stops before counts none.
struct Proofs<'a> {
    tls: &'a mut ClientTls,
    /// The end of the bytes read so far, for a header split across reads.
    tail: Vec<u8>,
    seen: bool,
    throttle: &'a RefCell<NlaThrottle>,
    ip: IpAddr,
    attempt: &'a Cell<Option<Attempt>>,
    /// Why the AUTHENTICATE was refused unread, its address having backed off or the pause
    /// started since CredSSP's start.
    throttled: &'a Cell<Option<Refusal>>,
}

impl AsyncRead for Proofs<'_> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let start = buf.filled().len();
        ready!(Pin::new(&mut *this.tls).poll_read(cx, buf))?;
        if !this.seen && authenticate_in(&mut this.tail, &buf.filled()[start..]) {
            this.seen = true;
            // Checked and counted in one borrow, so no other connection's proof slips between.
            let mut throttle = this.throttle.borrow_mut();
            if let Err(refusal) = throttle.check(this.ip) {
                buf.set_filled(start);
                this.throttled.set(Some(refusal));
                return Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    refusal.to_string(),
                )));
            }
            this.attempt.set(Some(throttle.failed(this.ip)));
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Proofs<'_> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut *self.get_mut().tls).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().tls).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut *self.get_mut().tls).poll_shutdown(cx)
    }
}

/// Whether `read`, after the `tail` of the reads before it, holds an NTLM AUTHENTICATE header.
fn authenticate_in(tail: &mut Vec<u8>, read: &[u8]) -> bool {
    tail.extend_from_slice(read);
    let found = tail.windows(AUTHENTICATE.len()).any(|w| w == AUTHENTICATE);
    tail.drain(..tail.len().saturating_sub(AUTHENTICATE.len() - 1));
    found
}

/// Called before HYBRID_EX's result, which is success only once reprise-login verifies the
/// delegated password.
struct Verify<'a, 'b> {
    link: &'a mut Link<'b>,
    user: Option<String>,
    refused: Option<Refused>,
}

#[async_trait::async_trait(?Send)]
impl ConnectionSetupHandler for Verify<'_, '_> {
    async fn handle_credentials(
        &mut self,
        credentials: Option<ReceivedCredentials>,
    ) -> ConnectorResult<()> {
        let verified = match self.link.delegate(credentials.as_ref()) {
            Ok(user) => {
                self.user = Some(user);
                self.link.verified().await
            }
            Err(refused) => Err(refused),
        };
        verified.map_err(|refused| {
            self.refused = Some(refused);
            ConnectorError::new("reprise-login", ConnectorErrorKind::AccessDenied)
        })
    }
}

/// NTLM makes no network requests.
struct NoNetwork;

impl NetworkClient for NoNetwork {
    async fn send(&mut self, _: &NetworkRequest) -> ConnectorResult<Vec<u8>> {
        Err(ConnectorError::general("NTLM needs no network"))
    }
}

/// Each user under the four name forms an SPNEGO client may send, which sspi matches in
/// full: `anna`, `.\anna`, `HOST\anna` and `anna@host`. The password is the NT hash.
fn credssp_users(users: &[NlaUser], host: &str) -> Vec<CredsspAuthIdentity> {
    let domain = host.split('.').next().unwrap_or(host).to_uppercase();
    let mut list = Vec::with_capacity(users.len() * 4);
    for u in users {
        let password = Zeroizing::new(format!("{NT_HASH_PREFIX}{}", *u.nt_hash));
        let names = [
            Username::new(&u.user, None),
            Username::new_down_level_logon_name(&u.user, "."),
            Username::new_down_level_logon_name(&u.user, &domain),
            Username::new_upn(&u.user, host),
        ];
        for username in names.into_iter().filter_map(Result::ok) {
            list.push(CredsspAuthIdentity {
                username,
                password: (*password).clone().into(),
            });
        }
    }
    list
}

/// The recording travels to the session in one frame, so a large one is refused here.
fn check_recording(steps: &[Step]) -> Result<(), Refused> {
    let bytes: usize = (steps.iter())
        .map(|s| match s {
            Step::Pdu { input, output } => input.len() + output.len(),
            _ => 0,
        })
        .sum();
    match bytes > MAX_RECORDING {
        true => Err(Refused::TooLong),
        false => Ok(()),
    }
}

fn read_error(e: io::Error) -> Refused {
    match over_limit(&e) {
        true => Refused::TooLong,
        false => Refused::Io(e.kind()),
    }
}

/// One acceptor step on `input`, or on the PDU its hint asks for; a send-only step records
/// empty input. Whatever the acceptor wrote goes out even when the step failed.
async fn step<S>(
    framed: &mut TokioFramed<S>,
    acceptor: &mut Acceptor,
    steps: &mut Vec<Step>,
    input: Option<Bytes>,
) -> Result<(), Refused>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let input = match (input, acceptor.next_pdu_hint()) {
        (Some(input), _) => input,
        (None, Some(hint)) => framed.read_by_hint(hint).await.map_err(read_error)?,
        (None, None) => Bytes::new(),
    };
    let mut output = WriteBuf::new();
    let stepped = acceptor.step(&input, None, &mut output);
    if !output.filled().is_empty() {
        framed
            .write_all(output.filled())
            .await
            .map_err(|e| Refused::Io(e.kind()))?;
    }
    stepped.map_err(|e| Refused::Negotiation(e.to_string()))?;
    steps.push(Step::Pdu {
        input: Zeroizing::new(input.to_vec()),
        output: Zeroizing::new(output.filled().to_vec()),
    });
    Ok(())
}

fn invalid(e: &dyn fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

/// The Client Info PDU `pdu` (X.224 and MCS Send Data Request), with its Send Data Request.
fn read_client_info(pdu: &[u8]) -> io::Result<(SendDataRequest<'_>, ClientInfoPdu)> {
    let request = ironrdp_core::decode::<X224<SendDataRequest<'_>>>(pdu)
        .map_err(|e| invalid(&e))?
        .0;
    let info = ironrdp_core::decode(request.user_data.as_ref()).map_err(|e| invalid(&e))?;
    Ok((request, info))
}

/// The Client Info PDU `pdu` with its password emptied.
pub fn blank_password(pdu: &[u8]) -> io::Result<Vec<u8>> {
    let (request, mut info) = read_client_info(pdu)?;
    info.client_info.credentials.password.zeroize();
    if let Some(mut cookie) = info.client_info.password_cookie.take() {
        cookie.0.zeroize();
    }
    let user_data = ironrdp_core::encode_vec(&info).map_err(|e| invalid(&e))?;
    let blanked = SendDataRequest {
        initiator_id: request.initiator_id,
        channel_id: request.channel_id,
        user_data: user_data.into(),
    };
    ironrdp_core::encode_vec(&X224(blanked)).map_err(|e| invalid(&e))
}

/// Tells the client its logon was refused, then closes the connection.
pub async fn deny(mut pending: Pending) {
    let mut out = WriteBuf::new();
    let farewell = async {
        if pending.acceptor.encode_access_denied(&mut out).is_ok() && !out.filled().is_empty() {
            pending.tls.write_all(out.filled()).await?;
        }
        pending.tls.shutdown().await
    };
    let _ = tokio::time::timeout(FAREWELL, farewell).await;
}

/// Asks reprise-login to log `user` on with `secret`.
pub fn authenticate(
    ctx: &Ctx,
    conn: u64,
    peer: SocketAddr,
    accepted: Instant,
    user: String,
    secret: Secret,
) -> io::Result<()> {
    let request = FrontRequest::Authenticate {
        conn,
        peer,
        user,
        secret,
        elapsed_ms: u32::try_from(accepted.elapsed().as_millis()).unwrap_or(u32::MAX),
    };
    match &*ctx.link.borrow() {
        Some(link) => link.send(&request, None),
        None => Err(io::ErrorKind::NotConnected.into()),
    }
}

/// The connection an event is for.
pub fn event_conn(event: &FrontEvent) -> Option<u64> {
    match event {
        FrontEvent::Verified { conn }
        | FrontEvent::Accepted { conn }
        | FrontEvent::Denied { conn, .. } => Some(*conn),
        FrontEvent::Passwords { .. } => None,
    }
}

#[cfg(test)]
pub(crate) mod tests;
