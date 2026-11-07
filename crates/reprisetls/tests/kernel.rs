//! Kernel TLS between a loopback rustls client and a server that moves its keys into the kernel.

mod fixture;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::fd::AsFd;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use repriselogin::handover::{DirectionKeys, TlsState};
use reprisetls::cork::Cork;
use reprisetls::ktls::{self, KtlsStream, REKEY_MARGIN, TrafficSecrets};
use rustls::crypto::aws_lc_rs::cipher_suite;
use rustls::{ClientConnection, ProtocolVersion, StreamOwned, SupportedCipherSuite};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::timeout;
use zeroize::Zeroizing;

const SUITES: [SupportedCipherSuite; 6] = [
    cipher_suite::TLS13_AES_128_GCM_SHA256,
    cipher_suite::TLS13_AES_256_GCM_SHA384,
    cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
    cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
];

const TLS13: [SupportedCipherSuite; 3] = [SUITES[0], SUITES[1], SUITES[2]];
const TLS12: [SupportedCipherSuite; 3] = [SUITES[3], SUITES[4], SUITES[5]];

const ECHOED: usize = 4 << 20;
/// Per suite, so a fault fails the test instead of hanging it.
const LIMIT: Duration = Duration::from_secs(10);

/// A blocking rustls client offering only `suite`, past its handshake with `addr`. It returns
/// once the server's keys are in the kernel, so the server reads nothing past the handshake.
fn client(
    addr: SocketAddr,
    suite: SupportedCipherSuite,
    installed: mpsc::Receiver<()>,
) -> StreamOwned<ClientConnection, TcpStream> {
    let conn = ClientConnection::new(fixture::client_config(suite), "reprise".try_into().unwrap())
        .unwrap();
    let sock = TcpStream::connect(addr).unwrap();
    sock.set_read_timeout(Some(LIMIT)).unwrap();
    sock.set_write_timeout(Some(LIMIT)).unwrap();
    let mut stream = StreamOwned::new(conn, sock);
    while stream.conn.is_handshaking() {
        stream.conn.complete_io(&mut stream.sock).unwrap();
    }
    installed.recv_timeout(LIMIT).unwrap();
    stream
}

/// Accepts one connection, runs the handshake through `Cork`, hands its keys to the kernel and
/// tells the client through `installed`. It also gives the state as installed.
async fn accept(listener: &TcpListener, installed: mpsc::Sender<()>) -> (KtlsStream, TlsState) {
    accept_with(listener, installed, |_| {}).await
}

/// As `accept`, with `adjust` applied to the state before it is installed.
async fn accept_with(
    listener: &TcpListener,
    installed: mpsc::Sender<()>,
    adjust: impl FnOnce(&mut TlsState),
) -> (KtlsStream, TlsState) {
    let (tcp, _) = listener.accept().await.unwrap();
    let (config, secrets) = fixture::logged_server_config();
    let tls = tokio_rustls::TlsAcceptor::from(config)
        .accept(Cork::new(tcp))
        .await
        .unwrap();
    let (cork, conn) = tls.into_inner();
    let traffic = secrets
        .get("CLIENT_TRAFFIC_SECRET_0")
        .zip(secrets.get("SERVER_TRAFFIC_SECRET_0"))
        .map(|(client, server)| TrafficSecrets { client, server });
    let mut tls = ktls::hand_over(&cork, conn, traffic).unwrap();
    adjust(&mut tls);
    let tcp = cork.into_inner();
    ktls::install(tcp.as_fd(), &tls).unwrap();
    installed.send(()).unwrap();
    (KtlsStream::new(tcp, tls.clone()), tls)
}

/// The keys the client receives under now, from its extracted secrets.
fn client_rx(stream: StreamOwned<ClientConnection, TcpStream>) -> DirectionKeys {
    #[allow(deprecated)]
    let (seq, secrets) = stream.conn.dangerous_extract_secrets().unwrap().rx;
    ktls::keys_from_rustls(ProtocolVersion::TLSv1_3, seq, secrets, None)
        .unwrap()
        .1
}

/// The keys the server sends under after one key update.
fn updated_tx(tls: &TlsState) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
    let next = ktls::next_traffic_secret(tls.cipher, tls.tx.secret.as_ref().unwrap());
    ktls::traffic_keys(tls.cipher, &next)
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
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
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
        timeout(LIMIT, async {
            let (mut stream, _) = accept(&listener, installed).await;
            let mut buf = vec![0; 1 << 16];
            let mut echoed = 0;
            while echoed < ECHOED {
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0, "{:?} ended after {echoed} bytes", suite.suite());
                stream.write_all(&buf[..n]).await.unwrap();
                echoed += n;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} echoes within {LIMIT:?}", suite.suite()));
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn a_client_close_notify_ends_the_stream_cleanly() {
    for suite in SUITES {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            stream.conn.send_close_notify();
            stream.flush().unwrap();
            let _ = stream.sock.read(&mut [0; 1]);
        });
        timeout(LIMIT, async {
            let (mut stream, _) = accept(&listener, installed).await;
            let mut buf = [0; 64];
            assert_eq!(
                stream.read(&mut buf).await.unwrap(),
                0,
                "{:?}",
                suite.suite()
            );
            assert_eq!(stream.read(&mut buf).await.unwrap(), 0, "EOF stays");
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} reads EOF within {LIMIT:?}", suite.suite()));
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn shutdown_sends_close_notify_after_the_data() {
    for suite in SUITES {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            let mut got = Vec::new();
            stream.read_to_end(&mut got).unwrap();
            assert_eq!(
                Sha256::digest(&got),
                Sha256::digest(seeded(ECHOED)),
                "{:?} reads every byte, then a clean EOF",
                suite.suite()
            );
        });
        timeout(LIMIT, async {
            let (mut stream, _) = accept(&listener, installed).await;
            stream.write_all(&seeded(ECHOED)).await.unwrap();
            stream.shutdown().await.unwrap();
            assert!(
                stream.write(b"late").await.is_err(),
                "nothing is sent after close_notify"
            );
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} shuts down within {LIMIT:?}", suite.suite()));
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn a_client_key_update_rekeys_the_receiver() {
    const SENT: usize = 1 << 20;
    for suite in TLS13 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            stream.conn.refresh_traffic_keys().unwrap();
            let sent = seeded(SENT);
            let mut back = vec![0; SENT];
            for (out, back) in sent.chunks(1 << 16).zip(back.chunks_mut(1 << 16)) {
                stream.write_all(out).unwrap();
                stream.read_exact(back).unwrap();
            }
            assert_eq!(
                Sha256::digest(&back),
                Sha256::digest(&sent),
                "{:?} echoes the bytes sent after a key update",
                suite.suite()
            );
        });
        timeout(LIMIT, async {
            let (mut stream, _) = accept(&listener, installed).await;
            let mut buf = vec![0; 1 << 16];
            let mut echoed = 0;
            while echoed < SENT {
                let n = stream.read(&mut buf).await.unwrap();
                assert!(n > 0, "{:?} ended after {echoed} bytes", suite.suite());
                stream.write_all(&buf[..n]).await.unwrap();
                echoed += n;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} echoes within {LIMIT:?}", suite.suite()));
        client.join().unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn a_requested_update_is_answered() {
    for suite in TLS13 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            stream.conn.refresh_traffic_keys().unwrap();
            stream.write_all(b"ping").unwrap();
            stream.flush().unwrap();
            let mut got = [0; 4];
            stream.read_exact(&mut got).unwrap();
            assert_eq!(&got, b"pong", "{:?}", suite.suite());
            client_rx(stream)
        });
        let tls = timeout(LIMIT, async {
            let (mut stream, tls) = accept(&listener, installed).await;
            // Less room than the 5-byte key update record.
            let mut got = [0; 4];
            stream.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"ping", "{:?}", suite.suite());
            stream.write_all(b"pong").await.unwrap();
            tls
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} answers within {LIMIT:?}", suite.suite()));
        let rx = client.join().unwrap();
        let (key, iv) = updated_tx(&tls);
        assert!(
            rx.seq == 1 && rx.key == key && rx.iv == iv,
            "{:?}: the client reads the answer under the server's next keys",
            suite.suite()
        );
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn the_sender_rekeys_before_the_limit() {
    const WRITES: usize = 2048;
    for suite in TLS13 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            let mut got = vec![0; WRITES];
            stream.read_exact(&mut got).unwrap();
            assert_eq!(
                got,
                seeded(WRITES),
                "{:?} receives every byte",
                suite.suite()
            );
            client_rx(stream)
        });
        let tls = timeout(LIMIT, async {
            let (mut stream, tls) = accept_with(&listener, installed, |tls| {
                tls.record_limit = tls.tx.seq + REKEY_MARGIN + 10;
            })
            .await;
            for byte in seeded(WRITES) {
                stream.write_all(&[byte]).await.unwrap();
            }
            tls
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} writes within {LIMIT:?}", suite.suite()));
        assert_eq!(tls.tx.seq, tls.record_limit - REKEY_MARGIN - 10);
        let rx = client.join().unwrap();
        let (key, iv) = updated_tx(&tls);
        assert!(
            rx.seq == 1024 && rx.key == key && rx.iv == iv,
            "{:?}: one key update after the first 1024 writes, got seq {}",
            suite.suite(),
            rx.seq
        );
    }
}

#[tokio::test]
#[ignore = "needs the kernel tls module; run on a VM"]
async fn a_tls12_sender_closes_before_the_limit() {
    for suite in TLS12 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (installed, ready) = mpsc::channel();
        let client = thread::spawn(move || {
            let mut stream = client(addr, suite, ready);
            let mut got = Vec::new();
            stream.read_to_end(&mut got).unwrap();
            assert_eq!(
                got,
                seeded(1024),
                "{:?} reads the first 1024 writes, then a clean EOF",
                suite.suite()
            );
        });
        timeout(LIMIT, async {
            let (mut stream, _) = accept_with(&listener, installed, |tls| {
                tls.record_limit = tls.tx.seq + REKEY_MARGIN + 10;
            })
            .await;
            let bytes = seeded(1025);
            for &byte in &bytes[..1024] {
                stream.write_all(&[byte]).await.unwrap();
            }
            let err = stream.write_all(&bytes[1024..]).await.unwrap_err();
            assert_eq!(
                err.kind(),
                std::io::ErrorKind::ConnectionReset,
                "{:?}",
                suite.suite()
            );
        })
        .await
        .unwrap_or_else(|_| panic!("{:?} closes within {LIMIT:?}", suite.suite()));
        client.join().unwrap();
    }
}
