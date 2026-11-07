//! Kernel TLS between a loopback rustls client and a server that moves its keys into the kernel.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::fd::AsFd;
use std::sync::Arc;
use std::thread;

use repriselogin::handover::{TlsState, TlsVersion};
use reprisetls::cork::Cork;
use reprisetls::ktls::{self, KtlsStream};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs::{self as provider, cipher_suite};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, ProtocolVersion, ServerConfig,
    SignatureScheme, StreamOwned, SupportedCipherSuite,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const SUITES: [SupportedCipherSuite; 6] = [
    cipher_suite::TLS13_AES_128_GCM_SHA256,
    cipher_suite::TLS13_AES_256_GCM_SHA384,
    cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
];

const ECHOED: usize = 4 << 20;

#[derive(Debug)]
struct AnyCert;

impl ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        provider::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn server_config() -> Arc<ServerConfig> {
    let key = rcgen::KeyPair::generate().unwrap();
    let cert = rcgen::CertificateParams::new(vec!["reprise".into()])
        .unwrap()
        .self_signed(&key)
        .unwrap();
    let mut config = ServerConfig::builder_with_provider(Arc::new(provider::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivateKeyDer::try_from(key.serialize_der()).unwrap(),
        )
        .unwrap();
    config.enable_secret_extraction = true;
    Arc::new(config)
}

/// A blocking rustls client offering only `suite`, past its handshake with `addr`.
fn client(
    addr: SocketAddr,
    suite: SupportedCipherSuite,
) -> StreamOwned<ClientConnection, TcpStream> {
    let mut provider = provider::default_provider();
    provider.cipher_suites = vec![suite];
    let config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[suite.version()])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
    let conn = ClientConnection::new(Arc::new(config), "reprise".try_into().unwrap()).unwrap();
    let mut stream = StreamOwned::new(conn, TcpStream::connect(addr).unwrap());
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).unwrap();
    }
    stream
}

/// Accepts one connection, runs the handshake through `Cork` and moves its keys into the kernel.
async fn accept(listener: &TcpListener) -> KtlsStream {
    let (tcp, _) = listener.accept().await.unwrap();
    let tls = tokio_rustls::TlsAcceptor::from(server_config())
        .accept(Cork::new(tcp))
        .await
        .unwrap();
    let (cork, conn) = tls.into_inner();
    assert!(
        cork.at_boundary(),
        "the handshake ends at a record boundary"
    );
    assert!(!conn.wants_write(), "the handshake is flushed");
    let version = conn.protocol_version().unwrap();
    #[allow(deprecated)]
    let secrets = conn.dangerous_extract_secrets().unwrap();
    let (cipher, tx) = ktls::keys_from_rustls(version, secrets.tx.0, secrets.tx.1, None).unwrap();
    let (_, rx) = ktls::keys_from_rustls(version, secrets.rx.0, secrets.rx.1, None).unwrap();
    let tls = TlsState {
        version: match version {
            ProtocolVersion::TLSv1_2 => TlsVersion::Tls12,
            _ => TlsVersion::Tls13,
        },
        cipher,
        tx,
        rx,
        record_limit: 1 << 24,
    };
    let tcp = cork.into_inner();
    ktls::install(tcp.as_fd(), &tls).unwrap();
    KtlsStream::new(tcp, tls)
}

fn seeded(len: usize) -> Vec<u8> {
    let mut x = 0x9e37_79b9_7f4a_7c15_u64;
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn echo_4_mib_through_ktls_for_every_suite() {
    for suite in SUITES {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite);
            let sent = seeded(ECHOED);
            let mut back = vec![0; ECHOED];
            for (out, back) in sent.chunks(1 << 16).zip(back.chunks_mut(1 << 16)) {
                stream.write_all(out).unwrap();
                stream.read_exact(back).unwrap();
            }
            assert_eq!(
                Sha256::digest(&back),
                Sha256::digest(&sent),
                "{:?} echoes the bytes sent",
                suite.suite()
            );
        });
        let mut stream = accept(&listener).await;
        let mut buf = vec![0; 1 << 16];
        let mut echoed = 0;
        while echoed < ECHOED {
            let n = stream.read(&mut buf).await.unwrap();
            assert!(n > 0, "{:?} ended after {echoed} bytes", suite.suite());
            stream.write_all(&buf[..n]).await.unwrap();
            echoed += n;
        }
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn a_client_close_notify_ends_the_stream_cleanly() {
    for suite in SUITES {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite);
            stream.conn.send_close_notify();
            stream.flush().unwrap();
            let _ = stream.sock.read(&mut [0; 1]);
        });
        let mut stream = accept(&listener).await;
        let mut buf = [0; 64];
        assert_eq!(
            stream.read(&mut buf).await.unwrap(),
            0,
            "{:?}",
            suite.suite()
        );
        assert_eq!(stream.read(&mut buf).await.unwrap(), 0, "EOF stays");
        drop(stream);
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn shutdown_sends_close_notify() {
    for suite in SUITES {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite);
            let read = stream.read(&mut [0; 64]);
            assert_eq!(read.unwrap(), 0, "{:?} reads a clean EOF", suite.suite());
        });
        let mut stream = accept(&listener).await;
        stream.shutdown().await.unwrap();
        client.join().unwrap();
    }
}
