//! One connection from the TCP accept to its Client Info PDU: X.224, TLS over a `Cork`, MCS and
//! the channel joins, each acceptor step recorded for the server that takes over.

use std::cell::RefCell;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::Duration;

use ironrdp_acceptor::{Acceptor, CredentialOrigin, DesktopSize};
use ironrdp_connector::Sequence;
use ironrdp_core::WriteBuf;
use ironrdp_pdu::mcs::SendDataRequest;
use ironrdp_pdu::nego::SecurityProtocol;
use ironrdp_pdu::rdp::ClientInfoPdu;
use ironrdp_pdu::x224::X224;
use ironrdp_tokio::bytes::{Bytes, BytesMut};
use ironrdp_tokio::{FramedWrite, TokioFramed};
use repriselogin::front::{FrontEvent, NlaUser};
use repriselogin::handover::Step;
use reprisetls::cert::Identity;
use reprisetls::cork::Cork;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::KeyLog;
use tokio_rustls::server::TlsStream;
use zeroize::{Zeroize, Zeroizing};

use crate::limits::{HANDSHAKE, MAX_PREAUTH_BYTES, MAX_RECORDING, MAX_USER};
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

pub type ClientTls = TlsStream<Cork<Limited<TcpStream>>>;

/// What the front shares between its connections.
pub struct Ctx {
    pub identity: Identity,
    /// The users with a Remote Desktop password, as reprise-login last sent them.
    pub nla_users: Rc<RefCell<Vec<NlaUser>>>,
    /// `None` while reprise-login is out of reach.
    pub link: RefCell<Option<LinkSender>>,
}

/// A connection that sent its credentials, waiting for reprise-login's verdict.
pub struct Pending {
    pub conn: u64,
    pub peer: SocketAddr,
    pub security: SecurityProtocol,
    /// The Client Info PDU, last, has its password blanked.
    pub steps: Vec<Step>,
    pub user: String,
    pub password: Zeroizing<String>,
    pub tls: ClientTls,
    /// Plaintext read past the Client Info PDU.
    pub leftover: BytesMut,
    pub acceptor: Acceptor,
    #[expect(dead_code, reason = "the hand-over reads the traffic secrets")]
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
        }
    }
}

/// Keeps a connection's TLS 1.3 traffic secrets, which a key update derives the next keys from.
#[derive(Default)]
pub struct ConnKeyLog {
    client: Mutex<Option<Zeroizing<Vec<u8>>>>,
    server: Mutex<Option<Zeroizing<Vec<u8>>>>,
}

#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the hand-over reads the traffic secrets")
)]
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

    #[expect(dead_code, reason = "the hand-over takes the socket out")]
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

/// Runs `tcp` to its Client Info PDU within `HANDSHAKE`.
pub async fn to_client_info(tcp: TcpStream, conn: u64, ctx: &Ctx) -> Result<Pending, Refused> {
    let peer = tcp.peer_addr().map_err(|e| Refused::Io(e.kind()))?;
    tokio::time::timeout(HANDSHAKE, accept(tcp, peer, conn, ctx))
        .await
        .unwrap_or(Err(Refused::Deadline))
}

async fn accept(
    tcp: TcpStream,
    peer: SocketAddr,
    conn: u64,
    ctx: &Ctx,
) -> Result<Pending, Refused> {
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
    if acceptor.should_perform_credssp() {
        return Err(Refused::Negotiation(
            "Network Level Authentication is not available".into(),
        ));
    }

    let mut framed = TokioFramed::new(tls);
    while acceptor.client_info_flags().is_none() {
        step(&mut framed, &mut acceptor, &mut steps, None).await?;
    }
    let credentials = match acceptor.received_credentials() {
        Some(c) if c.origin == CredentialOrigin::ClientInfo => &c.credentials,
        _ => return Err(Refused::NoPassword),
    };
    let user = negotiate::strip_domain(&credentials.username).to_owned();
    if user.is_empty() {
        return Err(Refused::NoUser);
    }
    if user.len() > MAX_USER {
        return Err(Refused::UserTooLong);
    }
    if credentials.password.is_empty() {
        return Err(Refused::NoPassword);
    }
    let password = Zeroizing::new(credentials.password.clone());
    if let Some(Step::Pdu { input, .. }) = steps.last_mut() {
        let blanked = blank_password(input).map_err(|e| Refused::Negotiation(e.to_string()))?;
        *input = Zeroizing::new(blanked);
    }
    check_recording(&steps)?;
    let (tls, leftover) = framed.into_inner();
    Ok(Pending {
        conn,
        peer,
        security,
        steps,
        user,
        password,
        tls,
        leftover,
        acceptor,
        key_log,
    })
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

/// The Client Info PDU `pdu` (X.224 and MCS Send Data Request) with its password emptied.
pub fn blank_password(pdu: &[u8]) -> io::Result<Vec<u8>> {
    let invalid = |e: &dyn fmt::Display| io::Error::new(io::ErrorKind::InvalidData, e.to_string());
    let request = ironrdp_core::decode::<X224<SendDataRequest<'_>>>(pdu)
        .map_err(|e| invalid(&e))?
        .0;
    let mut info: ClientInfoPdu =
        ironrdp_core::decode(request.user_data.as_ref()).map_err(|e| invalid(&e))?;
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

/// The connection a verdict is for.
pub fn verdict_conn(event: &FrontEvent) -> Option<u64> {
    match event {
        FrontEvent::Accepted { conn } | FrontEvent::Denied { conn, .. } => Some(*conn),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests;
