use super::*;
use ironrdp_connector::{ClientConnector, Config, Credentials};
use ironrdp_pdu::gcc;
use ironrdp_pdu::rdp::capability_sets::{MajorPlatformType, RailSupportLevel};
use ironrdp_pdu::rdp::client_info::{
    AddressFamily, ClientInfo, ClientInfoFlags, CompressionType, ExtendedClientInfo,
    ExtendedClientOptionalInfo, PasswordCookie,
};
use ironrdp_pdu::rdp::headers::{BasicSecurityHeader, BasicSecurityHeaderFlags};
use std::collections::HashMap;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::client::danger::{
    HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier,
};
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};

pub(crate) fn ctx() -> Ctx {
    let dir = tempfile::tempdir().unwrap();
    Ctx {
        identity: reprisetls::cert::load_or_create(dir.path()).unwrap(),
        nla_users: Rc::default(),
        link: RefCell::new(None),
    }
}

#[derive(Debug)]
struct AnyCertificate;

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

fn client_config(user: &str, password: &str) -> Config {
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
    let pending = to_client_info(tcp, 7, &ctx()).await;
    client.abort();
    pending
}

fn client_info(pdu: &[u8]) -> ClientInfoPdu {
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
    assert_eq!(
        to_client_info(tcp, 1, &ctx()).await.err(),
        Some(Refused::Deadline)
    );
    let waited = start.elapsed();
    assert!(
        waited >= HANDSHAKE && waited < HANDSHAKE + Duration::from_secs(1),
        "closed at 10 s, after {waited:?}"
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
    let refused = to_client_info(tcp, 1, &ctx()).await.unwrap_err();
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
