//! The hand-over of a logged-on connection: its TLS keys go into the kernel and its socket to
//! reprise-login, which passes it to the user's reprise-rdp.

use std::fmt;
use std::io;
use std::os::fd::AsFd;

use repriselogin::front::FrontRequest;
use repriselogin::handover::HandOver;
use reprisetls::ktls::{self, TrafficSecrets};
use tokio::net::TcpStream;

use crate::accept::Pending;
use crate::link::LinkSender;

#[derive(Debug)]
pub enum HandOffError {
    /// Bytes of plaintext were read past the Client Info PDU.
    Leftover(usize),
    /// rustls still holds TLS records to send.
    Unsent,
    /// `ktls::hand_over` refused the connection.
    Tls(io::Error),
}

impl fmt::Display for HandOffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HandOffError::Leftover(n) => write!(f, "{n} bytes were read past the logon info"),
            HandOffError::Unsent => f.write_str("TLS records are still to be sent"),
            HandOffError::Tls(e) => e.fmt(f),
        }
    }
}

/// Takes the socket out of `pending`, with the record its server replays and the TLS keys to
/// install on it.
pub fn prepare(pending: Pending) -> Result<(TcpStream, HandOver), HandOffError> {
    let Pending {
        conn,
        peer,
        security,
        steps,
        user,
        tls,
        leftover,
        key_log,
        ..
    } = pending;
    if !leftover.is_empty() {
        return Err(HandOffError::Leftover(leftover.len()));
    }
    let (cork, server) = tls.into_inner();
    if server.wants_write() {
        return Err(HandOffError::Unsent);
    }
    let secrets = match (key_log.client_secret(), key_log.server_secret()) {
        (Some(client), Some(server)) => Some(TrafficSecrets { client, server }),
        _ => None,
    };
    let tls = ktls::hand_over(&cork, server, secrets).map_err(HandOffError::Tls)?;
    let handover = HandOver {
        conn,
        user,
        peer,
        security: security.bits(),
        steps,
        tls,
    };
    Ok((cork.into_inner().into_inner(), handover))
}

/// Installs `handover`'s keys on `tcp` and sends both to reprise-login. On failure reprise-login
/// hears the connection is gone. `tcp` is closed here either way.
pub fn send(link: &LinkSender, tcp: TcpStream, handover: HandOver) -> io::Result<()> {
    let conn = handover.conn;
    let sent = ktls::install(tcp.as_fd(), &handover.tls).and_then(|()| {
        let ready = FrontRequest::Ready {
            conn,
            handover: Box::new(handover),
        };
        link.send(&ready, Some(tcp.as_fd()))
    });
    if sent.is_err() {
        let _ = link.send(&FrontRequest::Gone { conn }, None);
    }
    sent
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accept::tests::{AnyCertificate, client_config, client_info, ctx, request};
    use crate::{accept, link};
    use ironrdp_connector::{ClientConnector, ClientConnectorState, Sequence};
    use ironrdp_core::WriteBuf;
    use ironrdp_pdu::nego::SecurityProtocol;
    use ironrdp_tokio::{FramedWrite, TokioFramed};
    use repriselogin::fdpass::FdFrameReader;
    use repriselogin::handover::{Cipher, DirectionKeys, Step, TlsState, TlsVersion};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, UnixListener};
    use tokio::sync::mpsc;
    use tokio_rustls::TlsConnector;
    use tokio_rustls::rustls::crypto::{CryptoProvider, aws_lc_rs};
    use tokio_rustls::rustls::pki_types::ServerName;
    use tokio_rustls::rustls::{
        ClientConfig, SupportedCipherSuite, SupportedProtocolVersion, version,
    };
    use zeroize::Zeroizing;

    /// A client offering `versions`, and only `suite` when given.
    fn connector(
        versions: &[&'static SupportedProtocolVersion],
        suite: Option<SupportedCipherSuite>,
    ) -> TlsConnector {
        let mut provider = aws_lc_rs::default_provider();
        if let Some(suite) = suite {
            provider.cipher_suites = vec![suite];
        }
        let provider: Arc<CryptoProvider> = Arc::new(provider);
        let config = ClientConfig::builder_with_provider(provider)
            .with_protocol_versions(versions)
            .unwrap()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCertificate))
            .with_no_client_auth();
        TlsConnector::from(Arc::new(config))
    }

    /// Logs on as anna over `tls` and sends `extra` in the Client Info PDU's record, then waits.
    async fn client(addr: SocketAddr, tls: TlsConnector, extra: &'static [u8]) {
        let tcp = TcpStream::connect(addr).await.unwrap();
        let local = tcp.local_addr().unwrap();
        let mut framed = TokioFramed::new(tcp);
        let mut connector = ClientConnector::new(client_config("anna", "secret"), local);
        let upgrade = (ironrdp_tokio::connect_begin(&mut framed, &mut connector).await).unwrap();
        let tcp = framed.into_inner_no_leftover();
        let name = ServerName::try_from("localhost").unwrap();
        let tls = tls.connect(name, tcp).await.unwrap();
        ironrdp_tokio::mark_as_upgraded(upgrade, &mut connector);
        let mut framed = TokioFramed::new(tls);
        let mut buf = WriteBuf::new();
        while !matches!(
            connector.state,
            ClientConnectorState::SecureSettingsExchange { .. }
        ) {
            ironrdp_tokio::single_sequence_step(&mut framed, &mut connector, &mut buf)
                .await
                .unwrap();
        }
        buf.clear();
        connector.step_no_input(&mut buf).unwrap();
        let mut record = buf.filled().to_vec();
        record.extend_from_slice(extra);
        framed.write_all(&record).await.unwrap();
        std::future::pending::<()>().await;
    }

    /// The suite the front negotiated with a client over `tls`, and `prepare`'s result.
    async fn prepared(
        tls: TlsConnector,
        extra: &'static [u8],
    ) -> (
        SupportedCipherSuite,
        Result<(TcpStream, HandOver), HandOffError>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = tokio::spawn(client(listener.local_addr().unwrap(), tls, extra));
        let (tcp, _) = listener.accept().await.unwrap();
        let (_tell, mut heard) = mpsc::unbounded_channel();
        let pending = accept::to_client_info(tcp, 7, &ctx(), &mut heard)
            .await
            .expect("the client reaches Client Info");
        let suite = pending.tls.get_ref().1.negotiated_cipher_suite().unwrap();
        let prepared = prepare(pending);
        client.abort();
        (suite, prepared)
    }

    fn confidentiality_limit(suite: SupportedCipherSuite) -> u64 {
        match suite {
            SupportedCipherSuite::Tls13(s) => s.common.confidentiality_limit,
            SupportedCipherSuite::Tls12(s) => s.common.confidentiality_limit,
        }
    }

    #[tokio::test]
    async fn the_hand_over_record_carries_the_suite_the_secrets_and_the_steps() {
        let suites = [
            (
                aws_lc_rs::cipher_suite::TLS13_AES_128_GCM_SHA256,
                Cipher::Aes128Gcm,
                32,
            ),
            (
                aws_lc_rs::cipher_suite::TLS13_AES_256_GCM_SHA384,
                Cipher::Aes256Gcm,
                48,
            ),
            (
                aws_lc_rs::cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
                Cipher::Chacha20Poly1305,
                32,
            ),
        ];
        for (offered, cipher, secret_len) in suites {
            let tls = connector(&[&version::TLS13], Some(offered));
            let (suite, prepared) = prepared(tls, b"").await;
            assert_eq!(suite.suite(), offered.suite());
            let (_tcp, handover) = prepared.expect("the connection is handed over");
            assert_eq!(handover.conn, 7);
            assert_eq!(handover.user, "anna");
            assert_eq!(handover.security, SecurityProtocol::SSL.bits());
            let tls = &handover.tls;
            assert_eq!(tls.version, TlsVersion::Tls13);
            assert_eq!(tls.cipher, cipher, "{offered:?}");
            for keys in [&tls.tx, &tls.rx] {
                let secret = keys
                    .secret
                    .as_ref()
                    .expect("TLS 1.3 carries its traffic secret");
                assert_eq!(secret.len(), secret_len, "{offered:?}");
            }
            assert_ne!(
                tls.tx.secret, tls.rx.secret,
                "each direction has its own secret"
            );
            assert_eq!(tls.record_limit, confidentiality_limit(suite));
            let Some(Step::Pdu { input, .. }) = handover.steps.last() else {
                panic!("the last step is the Client Info PDU: {:?}", handover.steps);
            };
            let info = client_info(input);
            assert_eq!(info.client_info.credentials.username, "anna");
            assert_eq!(info.client_info.credentials.password, "");
        }
    }

    #[tokio::test]
    async fn a_tls12_record_carries_no_traffic_secret() {
        let (suite, prepared) = prepared(connector(&[&version::TLS12], None), b"").await;
        let (_tcp, handover) = prepared.expect("the connection is handed over");
        assert_eq!(handover.tls.version, TlsVersion::Tls12);
        assert_eq!(handover.tls.tx.secret, None);
        assert_eq!(handover.tls.rx.secret, None);
        assert_eq!(handover.tls.record_limit, confidentiality_limit(suite));
    }

    #[tokio::test]
    async fn a_connection_with_plaintext_left_unread_is_not_handed_over() {
        let tls = connector(&[&version::TLS13], None);
        let (_, prepared) = prepared(tls, b"0123456789").await;
        assert!(
            matches!(prepared, Err(HandOffError::Leftover(10))),
            "{:?}",
            prepared.map(|(_, h)| h)
        );
    }

    #[test]
    fn a_full_recording_with_its_tls_state_fits_one_ready_frame() {
        let worst = |n: usize| Zeroizing::new(vec![0xFF; n]);
        let steps = vec![
            Step::Pdu {
                input: worst(crate::limits::MAX_RECORDING / 2),
                output: worst(crate::limits::MAX_RECORDING / 2),
            },
            Step::SecurityUpgradeDone,
            Step::CredsspDone {
                user: "u".repeat(crate::limits::MAX_USER),
                domain: Some("d".repeat(255)),
            },
        ];
        assert_eq!(accept::check_recording(&steps), Ok(()));
        let keys = || DirectionKeys {
            seq: u64::MAX,
            key: worst(32),
            iv: worst(12),
            secret: Some(worst(48)),
        };
        let handover = HandOver {
            conn: u64::MAX,
            user: "u".repeat(crate::limits::MAX_USER),
            peer: "[ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff]:65535"
                .parse()
                .unwrap(),
            security: u32::MAX,
            steps,
            tls: TlsState {
                version: TlsVersion::Tls13,
                cipher: Cipher::Chacha20Poly1305,
                tx: keys(),
                rx: keys(),
                record_limit: u64::MAX,
            },
        };
        let ready = FrontRequest::Ready {
            conn: u64::MAX,
            handover: Box::new(handover),
        };
        assert!(repriselogin::frame::encode(&ready).is_ok());
    }

    #[tokio::test]
    async fn a_hand_over_that_cannot_be_installed_is_gone_and_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("front.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let (sender, _events) = link::connect_to(&path).await.unwrap();
        let (daemon, _) = listener.accept().await.unwrap();
        let mut reader = FdFrameReader::new();
        assert_eq!(request(&daemon, &mut reader).await, FrontRequest::Hello);

        let rdp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut client = TcpStream::connect(rdp.local_addr().unwrap()).await.unwrap();
        let (tcp, peer) = rdp.accept().await.unwrap();
        // A key of the wrong length is refused before the socket is touched.
        let keys = DirectionKeys {
            seq: 0,
            key: Zeroizing::new(vec![1; 5]),
            iv: Zeroizing::new(vec![2; 12]),
            secret: None,
        };
        let handover = HandOver {
            conn: 4,
            user: "anna".into(),
            peer,
            security: SecurityProtocol::SSL.bits(),
            steps: Vec::new(),
            tls: TlsState {
                version: TlsVersion::Tls12,
                cipher: Cipher::Aes128Gcm,
                tx: keys.clone(),
                rx: keys,
                record_limit: 1 << 24,
            },
        };
        let e = send(&sender, tcp, handover).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{e}");
        assert_eq!(
            request(&daemon, &mut reader).await,
            FrontRequest::Gone { conn: 4 }
        );
        let mut byte = [0; 1];
        assert_eq!(
            client.read(&mut byte).await.unwrap(),
            0,
            "the client is closed"
        );
    }
}
