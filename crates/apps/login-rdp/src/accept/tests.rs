use super::*;
use crate::link;
use ironrdp_connector::credssp::CredsspSequence;
use ironrdp_connector::sspi::credssp::write_ts_credentials;
use ironrdp_connector::sspi::credssp::{
    ClientMode, ClientState, CredSspClient, CredSspMode, EarlyUserAuthResult, TsRequest,
};
use ironrdp_connector::sspi::generator::GeneratorState;
use ironrdp_connector::sspi::ntlm::NtlmConfig;
use ironrdp_connector::sspi::{
    AuthIdentity, BufferType, ClientRequestFlags, CredentialUse, CredentialsBuffers,
    DataRepresentation, EncryptionFlags, NegotiateConfig, Ntlm, SecurityBuffer, SecurityBufferRef,
    SecurityStatus, Sspi as _, SspiImpl as _,
};
use ironrdp_connector::{ClientConnector, Config, ConnectorErrorKind, Credentials, custom_err};
use ironrdp_pdu::PduHint;
use ironrdp_pdu::gcc;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{
    AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, ExtendedClientInfo,
    ExtendedClientOptionalInfo, PasswordCookie,
};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use repriselogin::account::nt_hash;
use repriselogin::fdpass::FdFrameReader;
use repriselogin::frame;
use repriselogin::throttle::{FREE_FAILURES, PAUSE, PAUSE_AFTER};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::os::fd::AsFd;
use tokio::io::{AsyncReadExt, Interest};
use tokio::net::{TcpListener, UnixListener, UnixStream};
use tokio::sync::{mpsc, oneshot};
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

const HOST: &str = "reprise-box.lan";

pub(crate) fn ctx() -> Ctx {
    let dir = tempfile::tempdir().unwrap();
    Ctx {
        identity: reprisetls::cert::load_or_create(dir.path()).unwrap(),
        nla_users: Rc::default(),
        link: RefCell::new(None),
        host: HOST.to_owned(),
        nla_throttle: RefCell::default(),
    }
}

/// A context whose only Remote Desktop user is `anna`, with the password `anna-rd-pass`.
fn ctx_with_anna() -> Ctx {
    let ctx = ctx();
    ctx.nla_users.borrow_mut().push(NlaUser {
        user: "anna".into(),
        nt_hash: nt_hash("anna-rd-pass"),
    });
    ctx
}

/// The next request the front sent over `daemon`.
pub(crate) async fn request(daemon: &UnixStream, reader: &mut FdFrameReader) -> FrontRequest {
    loop {
        if let Some((body, _)) = reader.next().unwrap() {
            return frame::decode(&body).unwrap();
        }
        daemon.readable().await.unwrap();
        match daemon.try_io(Interest::READABLE, || reader.read_from(daemon.as_fd())) {
            Ok(n) => assert_ne!(n, 0, "the front keeps the link"),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => panic!("{e}"),
        }
    }
}

/// reprise-login's end of a link the front holds in its `Ctx`.
struct Daemon {
    _dir: tempfile::TempDir,
    sock: UnixStream,
    reader: FdFrameReader,
}

impl Daemon {
    async fn link(ctx: &Ctx) -> Daemon {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (sender, _events) = link::connect_to(&path).await.unwrap();
        *ctx.link.borrow_mut() = Some(sender);
        let (sock, _) = listener.accept().await.unwrap();
        let mut daemon = Daemon {
            _dir: dir,
            sock,
            reader: FdFrameReader::new(),
        };
        assert_eq!(daemon.request().await, FrontRequest::Hello);
        daemon
    }

    async fn request(&mut self) -> FrontRequest {
        request(&self.sock, &mut self.reader).await
    }

    /// Every request until the front drops `ctx`'s link.
    async fn rest(mut self, ctx: Ctx) -> Vec<FrontRequest> {
        drop(ctx);
        let mut all = Vec::new();
        loop {
            if let Some((body, _)) = self.reader.next().unwrap() {
                all.push(frame::decode(&body).unwrap());
                continue;
            }
            self.sock.readable().await.unwrap();
            let read = (self.sock).try_io(Interest::READABLE, || {
                self.reader.read_from(self.sock.as_fd())
            });
            match read {
                Ok(0) => return all,
                Ok(_) => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
}

#[derive(Debug)]
pub(crate) struct AnyCertificate;

impl ServerCertVerifier for AnyCertificate {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, tokio_rustls::rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, tokio_rustls::rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        aws_lc_rs::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn client_tls(key_log: Option<Arc<dyn KeyLog>>) -> TlsConnector {
    let mut config = ClientConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCertificate))
        .with_no_client_auth();
    if let Some(key_log) = key_log {
        config.key_log = key_log;
    }
    TlsConnector::from(Arc::new(config))
}

pub(crate) fn client_config(user: &str, password: &str) -> Config {
    Config {
        desktop_size: ironrdp_connector::DesktopSize {
            width: 1280,
            height: 800,
        },
        monitor_layout: None,
        desktop_scale_factor: 0,
        desktop_physical_size: None,
        desktop_orientation: None,
        device_scale_factor: None,
        monitor_extended_layout: None,
        relative_mouse_input: false,
        enable_tls: true,
        enable_credssp: false,
        enable_standard_rdp_security: false,
        credentials: Credentials::UsernamePassword {
            username: user.into(),
            password: password.into(),
        },
        domain: None,
        client_build: 0,
        client_name: "test".into(),
        keyboard_type: gcc::KeyboardType::IBM_ENHANCED,
        keyboard_subtype: 0,
        keyboard_functional_keys_count: 12,
        keyboard_layout: 0,
        connection_type: gcc::ConnectionType::Lan,
        ime_file_name: String::new(),
        bitmap: None,
        dig_product_id: String::new(),
        client_dir: String::new(),
        alternate_shell: String::new(),
        work_dir: String::new(),
        remote_application_mode: false,
        rail_support_level: RailSupportLevel::SUPPORTED,
        platform: MajorPlatformType::UNIX,
        hardware_id: None,
        request_data: None,
        autologon: false,
        enable_audio_playback: false,
        enable_audio_capture: false,
        performance_flags: Default::default(),
        license_cache: None,
        timezone_info: Default::default(),
        compression_type: None,
        enable_server_pointer: false,
        pointer_software_rendering: false,
        multitransport_flags: None,
        support_dyn_vc_gfx_protocol: false,
        rdp_version: gcc::RdpVersion::V5_PLUS,
    }
}

/// A client that logs on as `user` over TLS and then waits for the licence the front never sends.
pub(crate) async fn client(addr: SocketAddr, user: String, password: String) {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let local = tcp.local_addr().unwrap();
    let mut framed = TokioFramed::new(tcp);
    let mut connector = ClientConnector::new(client_config(&user, &password), local);
    let Ok(upgrade) = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await else {
        return;
    };
    let tcp = framed.into_inner_no_leftover();
    let name = ServerName::try_from("localhost").unwrap();
    let Ok(tls) = client_tls(None).connect(name, tcp).await else {
        return;
    };
    let _ = ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
    let mut framed = TokioFramed::new(tls);
    let mut buf = WriteBuf::new();
    while ironrdp_tokio::single_sequence_step(&mut framed, &mut connector, &mut buf)
        .await
        .is_ok()
    {}
}

async fn log_on(user: &str, password: &str) -> Result<Pending, Refused> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::spawn(client(addr, user.into(), password.into()));
    let (tcp, _) = listener.accept().await.unwrap();
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let pending = to_client_info(tcp, 7, &ctx(), &mut heard).await;
    client.abort();
    pending
}

pub(crate) fn client_info(pdu: &[u8]) -> ClientInfoPdu {
    let request = ironrdp_core::decode::<X224<SendDataRequest<'_>>>(pdu)
        .unwrap()
        .0;
    ironrdp_core::decode(request.user_data.as_ref()).unwrap()
}

#[tokio::test]
async fn a_tls_client_is_read_to_client_info_and_its_password_blanked() {
    let pending = log_on("anna", "secret").await.expect("the client logs on");
    assert_eq!(pending.conn, 7);
    assert_eq!(pending.user, "anna");
    assert_eq!(*pending.password, "secret");
    assert_eq!(pending.security, SecurityProtocol::SSL);
    assert!(!pending.nla);
    assert!(pending.leftover.is_empty());
    assert!(!format!("{pending:?}").contains("secret"));

    let Some(Step::Pdu { input, .. }) = pending.steps.last() else {
        panic!("the last step is a PDU: {:?}", pending.steps);
    };
    let info = client_info(input);
    assert_eq!(info.client_info.credentials.username, "anna");
    assert_eq!(info.client_info.credentials.password, "");

    match &pending.steps[..3] {
        [
            Step::Pdu {
                input: request,
                output: none,
            },
            Step::Pdu {
                input: empty,
                output: confirm,
            },
            Step::SecurityUpgradeDone,
        ] => {
            assert!(
                !request.is_empty() && none.is_empty(),
                "the request in, nothing out"
            );
            assert!(
                empty.is_empty() && !confirm.is_empty(),
                "nothing in, the confirm out"
            );
        }
        steps => panic!("X.224 request, confirm, then the upgrade: {steps:?}"),
    }
    assert!(
        pending.tls.get_ref().0.at_boundary(),
        "nothing of a TLS record is left half read"
    );
}

#[tokio::test(start_paused = true)]
async fn a_connection_silent_past_the_handshake_deadline_is_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let silent = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (tcp, _) = listener.accept().await.unwrap();
    let start = tokio::time::Instant::now();
    let (_tell, mut heard) = mpsc::unbounded_channel();
    assert_eq!(
        to_client_info(tcp, 1, &ctx(), &mut heard).await.err(),
        Some(Refused::Deadline)
    );
    let waited = start.elapsed();
    assert!(
        waited >= Duration::from_secs(60) && waited < Duration::from_secs(61),
        "closed at 60 s, after {waited:?}"
    );
    drop(silent);
}

#[tokio::test]
async fn more_than_max_preauth_bytes_is_refused() {
    let mut exact = Limited::new(&[7u8; MAX_PREAUTH_BYTES][..]);
    let mut all = Vec::new();
    exact
        .read_to_end(&mut all)
        .await
        .expect("the limit itself is read");
    assert_eq!(all.len(), MAX_PREAUTH_BYTES);

    let over = vec![7u8; MAX_PREAUTH_BYTES + 1];
    let mut over = Limited::new(&over[..]);
    let e = over.read_to_end(&mut Vec::new()).await.unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidData);
    assert!(over_limit(&e));
    assert_eq!(read_error(e), Refused::TooLong);
}

#[tokio::test]
async fn an_empty_password_is_refused_with_a_hint() {
    let refused = log_on("anna", "").await.unwrap_err();
    assert_eq!(refused, Refused::NoPassword);
    assert!(
        refused
            .to_string()
            .contains("save the user name and password in the client")
    );
}

#[tokio::test]
async fn an_oversized_user_name_is_refused() {
    let user = "a".repeat(MAX_USER + 1);
    assert_eq!(
        log_on(&user, "secret").await.unwrap_err(),
        Refused::UserTooLong
    );
}

#[tokio::test]
async fn a_user_name_empty_without_its_domain_is_refused() {
    for user in ["@host", "DOM\\"] {
        assert_eq!(
            log_on(user, "secret").await.unwrap_err(),
            Refused::NoUser,
            "{user}"
        );
    }
}

#[tokio::test]
async fn a_client_offering_no_protocol_in_common_is_told_and_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let request = ironrdp_core::encode_vec(&X224(ironrdp_pdu::nego::ConnectionRequest {
        nego_data: None,
        flags: ironrdp_pdu::nego::RequestFlags::empty(),
        protocol: SecurityProtocol::HYBRID,
        correlation_info: None,
    }))
    .unwrap();
    client.write_all(&request).await.unwrap();
    let (tcp, _) = listener.accept().await.unwrap();
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let refused = to_client_info(tcp, 1, &ctx(), &mut heard)
        .await
        .unwrap_err();
    assert_eq!(refused, Refused::Negotiation(NLA_OR_RDP.into()));
    let mut answer = Vec::new();
    client.read_to_end(&mut answer).await.unwrap();
    let answer = ironrdp_core::decode::<X224<ironrdp_pdu::nego::ConnectionConfirm>>(&answer)
        .unwrap()
        .0;
    assert_eq!(
        answer,
        ironrdp_pdu::nego::ConnectionConfirm::Failure {
            code: ironrdp_pdu::nego::FailureCode::SSL_REQUIRED_BY_SERVER
        }
    );
}

#[test]
fn a_recording_over_12_kib_is_refused() {
    let pdu = |n: usize| Step::Pdu {
        input: Zeroizing::new(vec![0; n]),
        output: Zeroizing::new(Vec::new()),
    };
    assert_eq!(
        check_recording(&[
            pdu(MAX_RECORDING / 2),
            Step::SecurityUpgradeDone,
            pdu(MAX_RECORDING / 2)
        ]),
        Ok(())
    );
    assert_eq!(
        check_recording(&[pdu(MAX_RECORDING), pdu(1)]),
        Err(Refused::TooLong)
    );
}

fn client_info_pdu(password: &str, cookie: Option<Vec<u8>>) -> Vec<u8> {
    let info = ClientInfoPdu {
        security_header: BasicSecurityHeader {
            flags: BasicSecurityHeaderFlags::INFO_PKT,
        },
        client_info: ClientInfo {
            credentials: ironrdp_pdu::rdp::client_info::Credentials {
                username: "anna".into(),
                password: password.into(),
                domain: Some("HOME".into()),
            },
            password_cookie: cookie.map(PasswordCookie),
            code_page: 1252,
            flags: ClientInfoFlags::MOUSE | ClientInfoFlags::UNICODE | ClientInfoFlags::AUTOLOGON,
            compression_type: CompressionType::K8,
            alternate_shell: "shell".into(),
            work_dir: "/home/anna".into(),
            extra_info: ExtendedClientInfo {
                address_family: AddressFamily::INET,
                address: "192.0.2.9".into(),
                dir: "C:\\client".into(),
                optional_data: ExtendedClientOptionalInfo::default(),
            },
        },
    };
    let request = SendDataRequest {
        initiator_id: 1007,
        channel_id: 1003,
        user_data: ironrdp_core::encode_vec(&info).unwrap().into(),
    };
    ironrdp_core::encode_vec(&X224(request)).unwrap()
}

#[test]
fn blank_password_keeps_every_other_field() {
    let pdu = client_info_pdu("secret", None);
    let blanked = blank_password(&pdu).unwrap();
    let before = client_info(&pdu);
    let after = client_info(&blanked);
    assert_eq!(before.client_info.credentials.password, "secret");
    assert_eq!(after.client_info.credentials.password, "");
    let mut expected = before.clone();
    expected.client_info.credentials.password.clear();
    assert_eq!(after, expected, "the domain, flags and user survive");
    let request = |pdu: &[u8]| {
        let r = ironrdp_core::decode::<X224<SendDataRequest<'_>>>(pdu)
            .unwrap()
            .0;
        (r.initiator_id, r.channel_id)
    };
    assert_eq!(request(&blanked), request(&pdu));
    assert!(!blanked.windows(6).any(|w| w == b"secret"));
}

#[derive(Debug, Default)]
struct AllLabels(Mutex<HashMap<String, Vec<u8>>>);

impl KeyLog for AllLabels {
    fn log(&self, label: &str, _: &[u8], secret: &[u8]) {
        self.0.lock().unwrap().insert(label.into(), secret.to_vec());
    }
}

#[tokio::test]
async fn the_key_log_keeps_only_the_two_traffic_secrets() {
    let ctx = ctx();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(AllLabels::default());
    let client_log: Arc<dyn KeyLog> = seen.clone();
    let client = tokio::spawn(async move {
        let tcp = TcpStream::connect(addr).await.unwrap();
        let name = ServerName::try_from("localhost").unwrap();
        let mut tls = client_tls(Some(client_log))
            .connect(name, tcp)
            .await
            .unwrap();
        tls.write_all(b"x").await.unwrap();
        tls
    });
    let (tcp, _) = listener.accept().await.unwrap();
    let key_log = Arc::new(ConnKeyLog::default());
    let config = ctx.identity.server_config(true, Some(key_log.clone()));
    let mut tls = TlsAcceptor::from(config)
        .accept(Cork::new(tcp))
        .await
        .unwrap();
    tls.read_exact(&mut [0u8; 1]).await.unwrap();
    let _client = client.await.unwrap();

    let seen = seen.0.lock().unwrap();
    assert!(
        seen.len() > 2,
        "the client logs more labels: {:?}",
        seen.keys()
    );
    assert_eq!(key_log.client_secret().as_deref(), seen.get(CLIENT_TRAFFIC));
    assert_eq!(key_log.server_secret().as_deref(), seen.get(SERVER_TRAFFIC));
    assert!(key_log.client_secret().is_some() && key_log.server_secret().is_some());
    for label in seen.keys() {
        assert_eq!(
            key_log.will_log(label),
            label == CLIENT_TRAFFIC || label == SERVER_TRAFFIC,
            "{label}"
        );
    }
    let before = (key_log.client_secret(), key_log.server_secret());
    key_log.log("CLIENT_HANDSHAKE_TRAFFIC_SECRET", &[], &[1; 48]);
    key_log.log("EXPORTER_SECRET", &[], &[2; 48]);
    assert_eq!((key_log.client_secret(), key_log.server_secret()), before);
    assert_eq!(format!("{key_log:?}"), "ConnKeyLog(<redacted>)");
}

/// xorshift64*, so a failing buffer can be found again from its seed.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[test]
fn garbage_never_panics() {
    let valid = [
        client_info_pdu("secret", None),
        client_info_pdu("", Some(vec![1, 2, 3])),
        ironrdp_core::encode_vec(&X224(ironrdp_pdu::nego::ConnectionRequest {
            nego_data: Some(ironrdp_pdu::nego::NegoRequestData::cookie("anna".into())),
            flags: ironrdp_pdu::nego::RequestFlags::empty(),
            protocol: SecurityProtocol::SSL | SecurityProtocol::HYBRID,
            correlation_info: None,
        }))
        .unwrap(),
    ];
    let mut rng = Seeded(0x5EED_F00D);
    for _ in 0..10_000 {
        let len = (rng.next() % 601) as usize;
        let mut buf: Vec<u8> = match rng.next() % 3 {
            // A valid PDU with some bytes changed, cut or padded to `len`.
            0 => {
                let mut b = valid[(rng.next() % valid.len() as u64) as usize].clone();
                b.resize(len, 0);
                b
            }
            // A TPKT header that matches the buffer, so parsing goes past it.
            1 => {
                let mut b: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
                if len >= 4 {
                    b[..4].copy_from_slice(&[3, 0, (len >> 8) as u8, len as u8]);
                }
                b
            }
            _ => (0..len).map(|_| rng.next() as u8).collect(),
        };
        for _ in 0..(rng.next() % 4) {
            if !buf.is_empty() {
                let at = (rng.next() % buf.len() as u64) as usize;
                buf[at] = rng.next() as u8;
            }
        }
        let _ = negotiate::peek_request(&buf);
        let _ = blank_password(&buf);
    }
}

/// How a test client runs CredSSP: the connector's own bare NTLM, or NTLM in SPNEGO as
/// `user` in `domain`, which most RDP clients send.
enum Credssp {
    Connector,
    Spnego {
        user: &'static str,
        domain: Option<&'static str>,
    },
    /// SPNEGO as the logon's user, delegating empty credentials.
    NoPassword,
    /// Bare NTLM as `ntlm`'s user and password, delegating `user` and `password` instead.
    Delegating {
        ntlm: (&'static str, &'static str),
        user: &'static str,
        password: &'static str,
    },
    /// Quits after TLS, or once NTLM's NEGOTIATE has its CHALLENGE, with no AUTHENTICATE sent.
    Quits {
        after_negotiate: bool,
    },
    /// Pauses tokio's clock after TLS and sends nothing more, as a client held at a
    /// certificate prompt.
    Stalls,
    /// Bare NTLM as anna with `password`, delegating the same, held at NTLM's CHALLENGE.
    Held {
        password: &'static str,
        held: Held,
    },
}

/// The client's side of a [`Hold`].
struct Held {
    at_challenge: oneshot::Sender<()>,
    go: oneshot::Receiver<()>,
}

/// An NLA logon from `from`: `user` goes in the Client Info PDU, and in CredSSP unless
/// `credssp` names another.
struct NlaLogon {
    from: IpAddr,
    protocol: SecurityProtocol,
    user: &'static str,
    domain: Option<&'static str>,
    password: &'static str,
    credssp: Credssp,
}

#[derive(Debug)]
struct TsRequestHint;

impl PduHint for TsRequestHint {
    fn find_size(&self, bytes: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        match TsRequest::read_length(bytes) {
            Ok(length) => Ok(Some((true, length))),
            Err(_) => Ok(None),
        }
    }
}

#[derive(Debug)]
struct EarlyUserAuthResultHint;

impl PduHint for EarlyUserAuthResultHint {
    fn find_size(&self, _: &[u8]) -> ironrdp_core::DecodeResult<Option<(bool, usize)>> {
        Ok(Some((true, 4)))
    }
}

/// Runs CredSSP with NTLM in SPNEGO, then reads HYBRID_EX's Early User Authorization Result.
async fn spnego<S>(
    framed: &mut TokioFramed<S>,
    username: Username,
    password: &str,
    public_key: Vec<u8>,
    hybrid_ex: bool,
    mode: CredSspMode,
) -> ConnectorResult<()>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let identity = AuthIdentity {
        username,
        password: password.to_owned().into(),
    };
    let mut client = CredSspClient::new(
        public_key,
        identity.into(),
        mode,
        ClientMode::Negotiate(NegotiateConfig {
            protocol_config: Box::new(NtlmConfig::default()),
            package_list: Some("!kerberos,!pku2u".to_owned()),
            client_computer_name: "client".to_owned(),
        }),
        format!("TERMSRV/{HOST}"),
    )
    .unwrap();
    let mut request = TsRequest::default();
    loop {
        let state = match client.process(request).start() {
            GeneratorState::Completed(state) => state,
            GeneratorState::Suspended(_) => panic!("NTLM makes no network requests"),
        };
        let state = state.map_err(|e| custom_err!("SPNEGO", e))?;
        let (reply, last) = match state {
            ClientState::ReplyNeeded(reply) => (reply, false),
            ClientState::FinalMessage(reply) => (reply, true),
        };
        let mut out = Vec::new();
        reply.encode_ts_request(&mut out).unwrap();
        framed.write_all(&out).await.unwrap();
        if last {
            break;
        }
        let pdu =
            (framed.read_by_hint(&TsRequestHint).await).map_err(|e| custom_err!("read", e))?;
        request = TsRequest::from_buffer(&pdu).map_err(|e| custom_err!("TsRequest", e))?;
        if request.error_code.is_some() {
            return Err(ConnectorError::general("the server failed CredSSP"));
        }
    }
    if hybrid_ex {
        let pdu = (framed.read_by_hint(&EarlyUserAuthResultHint).await)
            .map_err(|e| custom_err!("read", e))?;
        let result = EarlyUserAuthResult::from_buffer(&pdu[..]).unwrap();
        if matches!(result, EarlyUserAuthResult::AccessDenied) {
            return Err(ConnectorError::new(
                "CredSSP",
                ConnectorErrorKind::AccessDenied,
            ));
        }
    }
    Ok(())
}

/// Runs CredSSP with bare NTLM as `user`, then delegates `delegated` in its TSCredentials.
/// CredSSP version 4 echoes the public key, which needs no hash.
async fn delegate_other<S>(
    framed: &mut TokioFramed<S>,
    user: &str,
    password: &str,
    delegated: AuthIdentity,
    public_key: &[u8],
    mut held: Option<Held>,
) -> ConnectorResult<()>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let identity = AuthIdentity {
        username: Username::new(user, None).unwrap(),
        password: password.to_owned().into(),
    };
    let mut ntlm = Ntlm::new();
    let mut handle = (ntlm.acquire_credentials_handle())
        .with_credential_use(CredentialUse::Outbound)
        .with_auth_data(&identity)
        .execute(&mut ntlm)
        .unwrap()
        .credentials_handle;
    let mut input = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let mut out = TsRequest {
        version: 4,
        ..TsRequest::default()
    };
    loop {
        let mut output = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
        let target = format!("TERMSRV/{HOST}");
        let mut builder = (ntlm.initialize_security_context())
            .with_credentials_handle(&mut handle)
            .with_context_requirements(
                ClientRequestFlags::MUTUAL_AUTH
                    | ClientRequestFlags::USE_SESSION_KEY
                    | ClientRequestFlags::INTEGRITY
                    | ClientRequestFlags::CONFIDENTIALITY,
            )
            .with_target_data_representation(DataRepresentation::Native)
            .with_target_name(&target)
            .with_input(&mut input)
            .with_output(&mut output);
        let status = (ntlm.initialize_security_context_impl(&mut builder))
            .and_then(|mut g| g.resolve_to_result())
            .map_err(|e| custom_err!("NTLM", e))?
            .status;
        out.nego_tokens = Some(output.remove(0).buffer);
        if status == SecurityStatus::Ok {
            break;
        }
        let reply = ts_request(framed, &out).await?;
        if let Some(held) = held.take() {
            let _ = held.at_challenge.send(());
            let _ = held.go.await;
        }
        input[0].buffer = reply.nego_tokens.unwrap_or_default();
    }
    out.pub_key_auth = Some(seal(&mut ntlm, public_key));
    ts_request(framed, &out).await?;
    let delegated = CredentialsBuffers::AuthIdentity(delegated.into());
    let credentials = write_ts_credentials(&delegated, CredSspMode::WithCredentials).unwrap();
    let last = TsRequest {
        version: 4,
        auth_info: Some(seal(&mut ntlm, &credentials)),
        ..TsRequest::default()
    };
    let mut bytes = Vec::new();
    last.encode_ts_request(&mut bytes).unwrap();
    framed.write_all(&bytes).await.unwrap();
    Ok(())
}

/// Sends `request` and reads the server's answer.
async fn ts_request<S>(
    framed: &mut TokioFramed<S>,
    request: &TsRequest,
) -> ConnectorResult<TsRequest>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let mut out = Vec::new();
    request.encode_ts_request(&mut out).unwrap();
    framed.write_all(&out).await.unwrap();
    let pdu = (framed.read_by_hint(&TsRequestHint).await).map_err(|e| custom_err!("read", e))?;
    let reply = TsRequest::from_buffer(&pdu).map_err(|e| custom_err!("TsRequest", e))?;
    match reply.error_code {
        Some(_) => Err(ConnectorError::general("the server failed CredSSP")),
        None => Ok(reply),
    }
}

/// Sends bare NTLM's NEGOTIATE and reads the server's CHALLENGE.
async fn negotiate_only<S>(framed: &mut TokioFramed<S>) -> ConnectorResult<()>
where
    S: AsyncRead + AsyncWrite + Send + Sync + Unpin,
{
    let identity = AuthIdentity {
        username: Username::new("anna", None).unwrap(),
        password: "anna-rd-pass".to_owned().into(),
    };
    let mut ntlm = Ntlm::new();
    let mut handle = (ntlm.acquire_credentials_handle())
        .with_credential_use(CredentialUse::Outbound)
        .with_auth_data(&identity)
        .execute(&mut ntlm)
        .unwrap()
        .credentials_handle;
    let mut input = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let mut output = vec![SecurityBuffer::new(Vec::new(), BufferType::Token)];
    let target = format!("TERMSRV/{HOST}");
    let mut builder = (ntlm.initialize_security_context())
        .with_credentials_handle(&mut handle)
        .with_context_requirements(ClientRequestFlags::MUTUAL_AUTH)
        .with_target_data_representation(DataRepresentation::Native)
        .with_target_name(&target)
        .with_input(&mut input)
        .with_output(&mut output);
    (ntlm.initialize_security_context_impl(&mut builder))
        .and_then(|mut g| g.resolve_to_result())
        .map_err(|e| custom_err!("NTLM", e))?;
    let negotiate = TsRequest {
        version: 4,
        nego_tokens: Some(output.remove(0).buffer),
        ..TsRequest::default()
    };
    let challenge = ts_request(framed, &negotiate).await?;
    assert!(
        challenge.nego_tokens.is_some(),
        "the server sends its CHALLENGE"
    );
    Ok(())
}

/// `data` sealed for the server, its signature first, as CredSSP sends it.
fn seal(ntlm: &mut Ntlm, data: &[u8]) -> Vec<u8> {
    let mut token = [0u8; 1024];
    let mut data = data.to_vec();
    let mut buffers = [
        SecurityBufferRef::token_buf(&mut token),
        SecurityBufferRef::data_buf(&mut data),
    ];
    ntlm.encrypt_message(EncryptionFlags::empty(), &mut buffers)
        .unwrap();
    let mut sealed = buffers[0].data().to_vec();
    sealed.extend_from_slice(buffers[1].data());
    sealed
}

/// Logs on over NLA and fails with why the client stopped; past Client Info it waits for the
/// licence the front never sends.
async fn nla_client(
    addr: SocketAddr,
    logon: NlaLogon,
    public_key: Vec<u8>,
) -> ConnectorResult<Infallible> {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::new(logon.from, 0)).unwrap();
    let tcp = socket.connect(addr).await.unwrap();
    let local = tcp.local_addr().unwrap();
    let mut framed = TokioFramed::new(tcp);
    let mut config = client_config(logon.user, logon.password);
    config.enable_tls = false;
    config.enable_credssp = true;
    config.domain = logon.domain.map(str::to_owned);
    let mut connector = ClientConnector::new(config, local);
    let mut buf = WriteBuf::new();
    connector.initiate_with_security_protocol(logon.protocol, &mut buf)?;
    framed.write_all(buf.filled()).await.unwrap();
    let upgrade = ironrdp_tokio::connect_begin(&mut framed, &mut connector).await?;
    let tcp = framed.into_inner_no_leftover();
    let name = tokio_rustls::rustls::pki_types::ServerName::try_from("localhost").unwrap();
    let tls = (client_tls(None).connect(name, tcp).await).map_err(|e| custom_err!("TLS", e))?;
    let _ = ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
    let mut framed = TokioFramed::new(tls);
    match logon.credssp {
        Credssp::Connector => {
            let (sequence, request) = CredsspSequence::init_for_connector(
                &connector,
                ironrdp_connector::ServerName::new(HOST),
                public_key,
                None,
            )?;
            let sequence = ironrdp_tokio::perform_credssp_with_context(
                &mut framed,
                &mut NoNetwork,
                &mut buf,
                sequence,
                request,
            )
            .await?;
            connector.complete_credssp(sequence)?;
        }
        Credssp::Spnego { user, domain } => {
            let username = Username::new(user, domain).unwrap();
            let hybrid_ex = logon.protocol == SecurityProtocol::HYBRID_EX;
            let mode = CredSspMode::WithCredentials;
            spnego(
                &mut framed,
                username,
                logon.password,
                public_key,
                hybrid_ex,
                mode,
            )
            .await?;
            connector.mark_credssp_as_done();
        }
        Credssp::NoPassword => {
            let username = Username::new(logon.user, None).unwrap();
            let hybrid_ex = logon.protocol == SecurityProtocol::HYBRID_EX;
            let mode = CredSspMode::CredentialLess;
            spnego(
                &mut framed,
                username,
                logon.password,
                public_key,
                hybrid_ex,
                mode,
            )
            .await?;
            connector.mark_credssp_as_done();
        }
        Credssp::Delegating {
            ntlm: (ntlm_user, ntlm_password),
            user,
            password,
        } => {
            let delegated = AuthIdentity {
                username: Username::new(user, None).unwrap(),
                password: password.to_owned().into(),
            };
            delegate_other(
                &mut framed,
                ntlm_user,
                ntlm_password,
                delegated,
                &public_key,
                None,
            )
            .await?;
            connector.mark_credssp_as_done();
        }
        Credssp::Held { password, held } => {
            let delegated = AuthIdentity {
                username: Username::new("anna", None).unwrap(),
                password: password.to_owned().into(),
            };
            delegate_other(
                &mut framed,
                "anna",
                password,
                delegated,
                &public_key,
                Some(held),
            )
            .await?;
            connector.mark_credssp_as_done();
        }
        Credssp::Quits { after_negotiate } => {
            if after_negotiate {
                negotiate_only(&mut framed).await?;
            }
            return Err(ConnectorError::general("the client quits"));
        }
        Credssp::Stalls => {
            tokio::time::pause();
            std::future::pending::<()>().await;
        }
    }
    loop {
        ironrdp_tokio::single_sequence_step(&mut framed, &mut connector, &mut buf).await?;
    }
}

/// Runs `logon` against the front until it returns, with `heard` as the daemon's events.
async fn nla_log_on(
    ctx: &Ctx,
    logon: NlaLogon,
    heard: &mut UnboundedReceiver<FrontEvent>,
) -> (
    Result<Pending, Refused>,
    tokio::task::JoinHandle<ConnectorResult<Infallible>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let client = tokio::spawn(nla_client(addr, logon, ctx.identity.public_key()));
    let (tcp, _) = listener.accept().await.unwrap();
    let pending = to_client_info(tcp, 7, ctx, heard).await;
    (pending, client)
}

fn hybrid(user: &'static str, password: &'static str) -> NlaLogon {
    NlaLogon {
        from: Ipv4Addr::LOCALHOST.into(),
        protocol: SecurityProtocol::HYBRID,
        user,
        domain: None,
        password,
        credssp: Credssp::Connector,
    }
}

#[tokio::test]
async fn nla_with_the_remote_desktop_password_reaches_client_info_as_that_user() {
    let ctx = ctx_with_anna();
    let mut daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = hybrid("anna", "anna-rd-pass");
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    let pending = pending.expect("the client logs on");
    assert!(pending.nla);
    assert_eq!(pending.user, "anna");
    assert_eq!(pending.security, SecurityProtocol::HYBRID);
    assert!(
        pending.password.is_empty(),
        "the password went to reprise-login"
    );
    match daemon.request().await {
        FrontRequest::Authenticate {
            conn: 7,
            user,
            secret,
            ..
        } => {
            assert_eq!(user, "anna");
            assert_eq!(
                secret,
                Secret::Delegated(Zeroizing::new("anna-rd-pass".into()))
            );
        }
        other => panic!("an Authenticate for conn 7: {other:?}"),
    }
    let markers: Vec<&Step> = (pending.steps.iter())
        .filter(|s| !matches!(s, Step::Pdu { .. }))
        .collect();
    assert_eq!(
        markers,
        [
            &Step::SecurityUpgradeDone,
            &Step::CredsspDone {
                user: "anna".into(),
                domain: None
            }
        ]
    );
    let Some(Step::Pdu { input, .. }) = pending.steps.last() else {
        panic!("the last step is a PDU: {:?}", pending.steps);
    };
    let info = client_info(input);
    assert_eq!(info.client_info.credentials.username, "anna");
    assert_eq!(info.client_info.credentials.password, "");
    assert!(!input.windows(12).any(|w| w == b"anna-rd-pass"));
    drop(pending);
    assert_eq!(daemon.rest(ctx).await, [], "no Gone for a logon handed on");
}

#[tokio::test]
async fn nla_with_a_wrong_password_never_reaches_the_daemon() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let (pending, client) = nla_log_on(&ctx, hybrid("anna", "wrong"), &mut heard).await;
    assert!(
        matches!(pending, Err(Refused::Nla(_))),
        "CredSSP fails: {pending:?}"
    );
    assert!(client.await.unwrap().is_err());
    assert_eq!(daemon.rest(ctx).await, [], "no Authenticate");
}

#[tokio::test]
async fn a_hybrid_ex_client_hears_access_denied_when_the_daemon_denies() {
    let ctx = ctx_with_anna();
    let mut daemon = Daemon::link(&ctx).await;
    let (tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        protocol: SecurityProtocol::HYBRID_EX,
        ..hybrid("anna", "anna-rd-pass")
    };
    let answer = async {
        let authenticate = daemon.request().await;
        assert!(
            matches!(authenticate, FrontRequest::Authenticate { conn: 7, .. }),
            "{authenticate:?}"
        );
        tell.send(FrontEvent::Denied {
            conn: 7,
            backoff_s: 4,
        })
        .unwrap();
    };
    let ((pending, client), ()) = tokio::join!(nla_log_on(&ctx, logon, &mut heard), answer);
    assert_eq!(pending.unwrap_err(), Refused::Denied { backoff_s: 4 });
    let e = client.await.unwrap().unwrap_err();
    assert!(
        matches!(e.kind(), ConnectorErrorKind::AccessDenied),
        "the early user authorization result denies access: {e}"
    );
    assert_eq!(daemon.rest(ctx).await, [], "no Gone after a Denied");
}

#[tokio::test]
async fn a_hybrid_ex_client_goes_on_once_the_daemon_verifies() {
    let ctx = ctx_with_anna();
    let mut daemon = Daemon::link(&ctx).await;
    let (tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        protocol: SecurityProtocol::HYBRID_EX,
        ..hybrid("anna", "anna-rd-pass")
    };
    let answer = async {
        assert!(matches!(
            daemon.request().await,
            FrontRequest::Authenticate { conn: 7, .. }
        ));
        tell.send(FrontEvent::Verified { conn: 7 }).unwrap();
    };
    let ((pending, client), ()) = tokio::join!(nla_log_on(&ctx, logon, &mut heard), answer);
    client.abort();
    let pending = pending.expect("the client reaches Client Info");
    assert_eq!(pending.security, SecurityProtocol::HYBRID_EX);
    assert!(pending.nla);
}

#[tokio::test]
async fn a_client_info_user_other_than_the_credssp_user_is_refused() {
    let ctx = ctx_with_anna();
    let mut daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Spnego {
            user: "anna",
            domain: None,
        },
        ..hybrid("bob", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.unwrap_err(), Refused::UserChanged);
    assert!(matches!(
        daemon.request().await,
        FrontRequest::Authenticate { conn: 7, .. }
    ));
    assert_eq!(
        daemon.rest(ctx).await,
        [FrontRequest::Gone { conn: 7 }],
        "reprise-login hears the logon is gone"
    );
}

/// `ctx_with_anna`, with `bert` listed under `bert-rd-pass`.
fn ctx_with_anna_and_bert() -> Ctx {
    let ctx = ctx_with_anna();
    ctx.nla_users.borrow_mut().push(NlaUser {
        user: "bert".into(),
        nt_hash: nt_hash("bert-rd-pass"),
    });
    ctx
}

#[tokio::test]
async fn a_client_authenticated_as_anna_that_delegates_bert_with_a_wrong_password_is_refused() {
    let ctx = ctx_with_anna_and_bert();
    let daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Delegating {
            ntlm: ("anna", "anna-rd-pass"),
            user: "bert",
            password: "bert-pass",
        },
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.unwrap_err(), Refused::DelegationMismatch);
    assert_eq!(daemon.rest(ctx).await, [], "no Authenticate");
}

#[tokio::test]
async fn a_client_authenticated_as_anna_that_delegates_bert_with_his_password_logs_on_as_bert() {
    let ctx = ctx_with_anna_and_bert();
    let mut daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Delegating {
            ntlm: ("anna", "anna-rd-pass"),
            user: "bert",
            password: "bert-rd-pass",
        },
        ..hybrid("bert", "bert-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.expect("the client logs on").user, "bert");
    match daemon.request().await {
        FrontRequest::Authenticate { user, secret, .. } => {
            assert_eq!(user, "bert");
            assert_eq!(
                secret,
                Secret::Delegated(Zeroizing::new("bert-rd-pass".into()))
            );
        }
        other => panic!("an Authenticate for bert: {other:?}"),
    }
}

#[tokio::test]
async fn a_client_that_delegates_another_password_than_it_authenticated_with_is_refused() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Delegating {
            ntlm: ("anna", "anna-rd-pass"),
            user: "anna",
            password: "other-pass",
        },
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.unwrap_err(), Refused::DelegationMismatch);
    assert_eq!(daemon.rest(ctx).await, [], "no Authenticate");
}

#[tokio::test]
async fn a_hand_built_client_that_delegates_its_own_credentials_logs_on() {
    let ctx = ctx_with_anna();
    let mut daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Delegating {
            ntlm: ("anna", "anna-rd-pass"),
            user: "anna",
            password: "anna-rd-pass",
        },
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.expect("the client logs on").user, "anna");
    assert!(matches!(
        daemon.request().await,
        FrontRequest::Authenticate { conn: 7, .. }
    ));
}

#[tokio::test]
async fn a_client_that_delegates_no_password_is_refused() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::NoPassword,
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert_eq!(pending.unwrap_err(), Refused::NoDelegation);
    assert_eq!(daemon.rest(ctx).await, [], "no Authenticate");
}

#[tokio::test(start_paused = true)]
async fn a_verdict_not_heard_within_verdict_wait_of_the_authenticate_is_refused() {
    let ctx = ctx();
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let start = Instant::now();
    let mut link = Link {
        ctx: &ctx,
        conn: 7,
        peer: "127.0.0.1:1".parse().unwrap(),
        heard: &mut heard,
        asked: true,
        attempt: &Cell::default(),
    };
    let waited = tokio::time::timeout(HANDSHAKE * 2, link.verified()).await;
    assert_eq!(waited, Ok(Err(Refused::NoVerdict)));
    assert_eq!(start.elapsed(), VERDICT_WAIT);
}

#[tokio::test]
async fn spnego_clients_log_on_under_each_name_form_of_the_user() {
    let forms = [
        (None, "anna"),
        (Some("REPRISE-BOX"), "anna"),
        (Some("reprise-box"), "anna"),
        (Some("."), "anna"),
        (None, "anna@reprise-box.lan"),
    ];
    for (domain, user) in forms {
        let ctx = ctx_with_anna();
        let _daemon = Daemon::link(&ctx).await;
        let (_tell, mut heard) = mpsc::unbounded_channel();
        let logon = NlaLogon {
            domain,
            credssp: Credssp::Spnego { user, domain },
            ..hybrid("anna", "anna-rd-pass")
        };
        let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
        client.abort();
        let pending = pending.unwrap_or_else(|e| panic!("{domain:?} {user}: {e}"));
        assert_eq!(pending.user, "anna", "{domain:?} {user}");
    }
}

#[tokio::test]
async fn a_spnego_client_with_another_domain_is_refused() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        credssp: Credssp::Spnego {
            user: "anna",
            domain: Some("ELSEWHERE"),
        },
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
    client.abort();
    assert!(matches!(pending, Err(Refused::Nla(_))), "{pending:?}");
    assert_eq!(daemon.rest(ctx).await, []);
}

#[test]
fn the_credssp_list_holds_each_user_under_four_names_with_its_nt_hash() {
    let users = [NlaUser {
        user: "anna".into(),
        nt_hash: nt_hash("anna-rd-pass"),
    }];
    let list = credssp_users(&users, HOST);
    let names: Vec<String> = list.iter().map(|i| format!("{:?}", i.username)).collect();
    assert_eq!(list.len(), 4, "{names:?}");
    let want = [
        Username::new("anna", None).unwrap(),
        Username::new_down_level_logon_name("anna", ".").unwrap(),
        Username::new_down_level_logon_name("anna", "REPRISE-BOX").unwrap(),
        Username::new_upn("anna", HOST).unwrap(),
    ];
    for (got, want) in list.iter().zip(&want) {
        assert_eq!(&got.username, want);
        assert_eq!(
            got.password.as_ref().as_str(),
            format!("{NT_HASH_PREFIX}{}", *nt_hash("anna-rd-pass"))
        );
    }
}

/// Moves tokio's clock on by `by` at once; handshakes run on the real clock.
async fn skip(by: Duration) {
    tokio::time::pause();
    tokio::time::advance(by).await;
    tokio::time::resume();
}

/// A wrong password, or a right one delegating another, from `from`.
fn failing_logon(from: IpAddr, i: u32) -> NlaLogon {
    let logon = NlaLogon {
        from,
        ..hybrid("anna", "wrong")
    };
    match i % 2 {
        0 => logon,
        _ => NlaLogon {
            credssp: Credssp::Delegating {
                ntlm: ("anna", "anna-rd-pass"),
                user: "anna",
                password: "other-pass",
            },
            ..logon
        },
    }
}

/// Fails `n` NLA logons from `from` at the front.
async fn fail_logons(ctx: &Ctx, from: IpAddr, n: u32) {
    let (_tell, mut heard) = mpsc::unbounded_channel();
    for i in 0..n {
        let (pending, client) = nla_log_on(ctx, failing_logon(from, i), &mut heard).await;
        client.abort();
        assert!(
            matches!(pending, Err(Refused::Nla(_) | Refused::DelegationMismatch)),
            "{pending:?}"
        );
    }
}

/// Logs on as anna with her Remote Desktop password from `from`.
async fn right_logon(ctx: &Ctx, from: IpAddr) -> Result<Pending, Refused> {
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let logon = NlaLogon {
        from,
        ..hybrid("anna", "anna-rd-pass")
    };
    let (pending, client) = nla_log_on(ctx, logon, &mut heard).await;
    client.abort();
    pending
}

#[tokio::test]
async fn failed_nla_logons_back_the_address_off_even_from_the_right_password() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    let here: IpAddr = Ipv4Addr::LOCALHOST.into();
    fail_logons(&ctx, here, FREE_FAILURES + 1).await;
    let Err(Refused::Throttled(Refusal::BackedOff(left))) = right_logon(&ctx, here).await else {
        panic!("the right password is refused inside the back-off");
    };
    assert!(left <= Duration::from_secs(2), "{left:?}");
    skip(left / 2).await;
    let refused = right_logon(&ctx, here).await.err();
    assert!(
        matches!(refused, Some(Refused::Throttled(Refusal::BackedOff(_)))),
        "still refused halfway through the back-off: {refused:?}"
    );
    right_logon(&ctx, Ipv4Addr::new(127, 0, 0, 2).into())
        .await
        .expect("another address logs on");
    let requests = daemon.rest(ctx).await;
    assert_eq!(requests.len(), 1, "one Authenticate: {requests:?}");
}

#[tokio::test]
async fn failed_nla_logons_from_many_addresses_pause_every_nla_logon() {
    let ctx = ctx_with_anna();
    let _daemon = Daemon::link(&ctx).await;
    for i in 1..=PAUSE_AFTER {
        fail_logons(&ctx, Ipv4Addr::new(127, 0, 0, i as u8).into(), 1).await;
    }
    let fresh: IpAddr = Ipv4Addr::new(127, 0, 0, 200).into();
    assert_eq!(
        right_logon(&ctx, fresh).await.err(),
        Some(Refused::Throttled(Refusal::Paused))
    );
    skip(PAUSE).await;
    right_logon(&ctx, fresh)
        .await
        .expect("the pause ends after 10 minutes");
}

#[tokio::test]
async fn the_right_nla_password_clears_the_address() {
    let ctx = ctx_with_anna();
    let _daemon = Daemon::link(&ctx).await;
    let here: IpAddr = Ipv4Addr::LOCALHOST.into();
    fail_logons(&ctx, here, FREE_FAILURES + 1).await;
    let Err(Refused::Throttled(Refusal::BackedOff(left))) = right_logon(&ctx, here).await else {
        panic!("the right password is refused inside the back-off");
    };
    skip(left).await;
    right_logon(&ctx, here)
        .await
        .expect("logs on once the back-off ends");
    fail_logons(&ctx, here, FREE_FAILURES).await;
    right_logon(&ctx, here)
        .await
        .expect("the free failures start again after a logon");
}

#[tokio::test]
async fn clients_that_quit_before_the_ntlm_authenticate_count_no_failed_password() {
    let ctx = ctx_with_anna();
    let _daemon = Daemon::link(&ctx).await;
    let here: IpAddr = Ipv4Addr::LOCALHOST.into();
    let (_tell, mut heard) = mpsc::unbounded_channel();
    for i in 0..2 * (FREE_FAILURES + 1) {
        let logon = NlaLogon {
            credssp: Credssp::Quits {
                after_negotiate: i % 2 == 1,
            },
            ..hybrid("anna", "anna-rd-pass")
        };
        let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
        client.abort();
        assert!(matches!(pending, Err(Refused::Nla(_))), "{pending:?}");
    }
    right_logon(&ctx, here)
        .await
        .expect("the address is not backed off");
}

#[tokio::test]
async fn clients_stalled_after_tls_past_the_deadline_count_no_failed_password() {
    let ctx = ctx_with_anna();
    let _daemon = Daemon::link(&ctx).await;
    let (_tell, mut heard) = mpsc::unbounded_channel();
    for i in 1..=PAUSE_AFTER {
        let logon = NlaLogon {
            from: Ipv4Addr::new(127, 0, 0, i as u8).into(),
            credssp: Credssp::Stalls,
            ..hybrid("anna", "anna-rd-pass")
        };
        let (pending, client) = nla_log_on(&ctx, logon, &mut heard).await;
        tokio::time::resume();
        client.abort();
        assert_eq!(pending.err(), Some(Refused::Deadline));
    }
    right_logon(&ctx, Ipv4Addr::new(127, 0, 0, 200).into())
        .await
        .expect("NLA logons are not paused");
}

#[test]
fn an_ntlm_authenticate_header_split_across_reads_is_seen() {
    let mut tail = Vec::new();
    assert!(!authenticate_in(
        &mut tail,
        b"0\x82NTLMSSP\0\x01\0\0\0..NTLMS"
    ));
    assert!(authenticate_in(&mut tail, b"SP\0\x03\0\0\0.."));
    assert!(!authenticate_in(&mut tail, b"NTLMSSP\0\x02\0\0\0"));
}

/// A logon held at NTLM's CHALLENGE until `go`; `done` fires once the front returns.
struct Hold {
    at_challenge: oneshot::Receiver<()>,
    go: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}

/// An NLA logon as anna with `password` from `from`, held at NTLM's CHALLENGE.
fn held_logon(from: IpAddr, password: &'static str) -> ((NlaLogon, oneshot::Sender<()>), Hold) {
    let (at_challenge, reached) = oneshot::channel();
    let (go, wait) = oneshot::channel();
    let (done, front_done) = oneshot::channel();
    let held = Held {
        at_challenge,
        go: wait,
    };
    let logon = NlaLogon {
        from,
        credssp: Credssp::Held { password, held },
        ..hybrid("anna", password)
    };
    let hold = Hold {
        at_challenge: reached,
        go,
        done: front_done,
    };
    ((logon, done), hold)
}

/// Runs a held logon's front side, then fires its `done`.
async fn held_front(
    ctx: &Ctx,
    (logon, done): (NlaLogon, oneshot::Sender<()>),
) -> Result<Pending, Refused> {
    let (_tell, mut heard) = mpsc::unbounded_channel();
    let (pending, client) = nla_log_on(ctx, logon, &mut heard).await;
    client.abort();
    let _ = done.send(());
    pending
}

/// Waits until every logon holds at CHALLENGE, then lets each send its AUTHENTICATE in turn,
/// the next once the front is done with the one before.
async fn in_turn<const N: usize>(holds: [Hold; N]) {
    let mut waiting = Vec::new();
    for hold in holds {
        let _ = hold.at_challenge.await;
        waiting.push((hold.go, hold.done));
    }
    for (go, done) in waiting {
        let _ = go.send(());
        let _ = done.await;
    }
}

#[tokio::test]
async fn a_connection_held_at_challenge_while_its_address_backs_off_is_refused_at_its_authenticate()
{
    let ctx = ctx_with_anna();
    let _daemon = Daemon::link(&ctx).await;
    let here: IpAddr = Ipv4Addr::LOCALHOST.into();
    fail_logons(&ctx, here, FREE_FAILURES).await;
    let (first, first_hold) = held_logon(here, "wrong");
    let (second, second_hold) = held_logon(here, "wrong");
    let (first, second, ()) = tokio::join!(
        held_front(&ctx, first),
        held_front(&ctx, second),
        in_turn([first_hold, second_hold]),
    );
    assert!(matches!(first, Err(Refused::Nla(_))), "{first:?}");
    assert!(
        matches!(second, Err(Refused::Throttled(Refusal::BackedOff(_)))),
        "{second:?}"
    );
    let Err(Refused::Throttled(Refusal::BackedOff(left))) = right_logon(&ctx, here).await else {
        panic!("the address backs off after its first held logon");
    };
    assert!(
        left <= Duration::from_secs(2),
        "one failure past the free ones, not two: {left:?}"
    );
}

#[tokio::test]
async fn connections_held_at_challenge_when_the_pause_starts_are_refused_at_their_authenticate() {
    let ctx = ctx_with_anna();
    let daemon = Daemon::link(&ctx).await;
    for i in 1..PAUSE_AFTER {
        fail_logons(&ctx, Ipv4Addr::new(127, 0, 0, i as u8).into(), 1).await;
    }
    let (last, last_hold) = held_logon(Ipv4Addr::new(127, 0, 0, 100).into(), "wrong");
    let (right, right_hold) = held_logon(Ipv4Addr::new(127, 0, 0, 101).into(), "anna-rd-pass");
    let (wrong, wrong_hold) = held_logon(Ipv4Addr::new(127, 0, 0, 102).into(), "wrong");
    let (last, right, wrong, ()) = tokio::join!(
        held_front(&ctx, last),
        held_front(&ctx, right),
        held_front(&ctx, wrong),
        in_turn([last_hold, right_hold, wrong_hold]),
    );
    assert!(matches!(last, Err(Refused::Nla(_))), "{last:?}");
    assert_eq!(right.err(), Some(Refused::Throttled(Refusal::Paused)));
    assert_eq!(wrong.err(), Some(Refused::Throttled(Refusal::Paused)));
    assert_eq!(
        daemon.rest(ctx).await,
        [],
        "the daemon hears no Authenticate"
    );
}
