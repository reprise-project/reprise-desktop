//! Kernel TLS: installing the traffic keys.

mod stream;

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use repriselogin::handover::{Cipher, DirectionKeys, TlsState, TlsVersion};
use rustls::{ConnectionCommon, ConnectionTrafficSecrets, ProtocolVersion};
use zeroize::{Zeroize, Zeroizing};

pub use stream::KtlsStream;

/// Moves `tls` into the kernel for the connected TCP socket `fd`; from then on plain reads and
/// writes on it carry TLS records.
pub fn install(fd: BorrowedFd<'_>, tls: &TlsState) -> io::Result<()> {
    let tx = CryptoInfo::new(Direction::Tx, tls.version, tls.cipher, &tls.tx)?;
    let rx = CryptoInfo::new(Direction::Rx, tls.version, tls.cipher, &tls.rx)?;
    // SAFETY: `fd` is a live socket and the option value is the 3 bytes of "tls".
    let r = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_TCP,
            libc::TCP_ULP,
            c"tls".as_ptr().cast(),
            3,
        )
    };
    if r < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ENOENT) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "kernel TLS needs the kernel's tls module, which isn't loaded",
            ));
        }
        return Err(e);
    }
    tx.set(fd, libc::TLS_TX)?;
    rx.set(fd, libc::TLS_RX)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Direction {
    Tx,
    Rx,
}

/// One direction's `setsockopt(SOL_TLS)` value; its key, iv and salt are zeroized on drop.
enum CryptoInfo {
    Aes128Gcm(libc::tls12_crypto_info_aes_gcm_128),
    Aes256Gcm(libc::tls12_crypto_info_aes_gcm_256),
    Chacha20Poly1305(libc::tls12_crypto_info_chacha20_poly1305),
}

impl CryptoInfo {
    fn new(
        direction: Direction,
        version: TlsVersion,
        cipher: Cipher,
        keys: &DirectionKeys,
    ) -> io::Result<CryptoInfo> {
        let iv: &[u8; 12] = keys.iv[..]
            .try_into()
            .map_err(|_| invalid("an iv not 12 bytes"))?;
        let key = keys.key.as_slice();
        if key.len() != key_len(cipher) {
            return Err(invalid("a key of the wrong length for its cipher"));
        }
        let info = |cipher_type| libc::tls_crypto_info {
            version: match version {
                TlsVersion::Tls12 => libc::TLS_1_2_VERSION,
                TlsVersion::Tls13 => libc::TLS_1_3_VERSION,
            },
            cipher_type,
        };
        let rec_seq = keys.seq.to_be_bytes();
        let salt = iv[..4].try_into().unwrap();
        let mut explicit: [u8; 8] = iv[4..].try_into().unwrap();
        // TLS 1.2 GCM sends count the explicit nonce up from `iv`, and receives read it off the
        // wire; TLS 1.3 and ChaCha20 XOR the sequence into the iv, as rustls does.
        if direction == Direction::Tx
            && version == TlsVersion::Tls12
            && cipher != Cipher::Chacha20Poly1305
        {
            explicit =
                tls12_tx_explicit_nonce(u64::from_be_bytes(explicit), keys.seq)?.to_be_bytes();
        }
        Ok(match cipher {
            Cipher::Aes128Gcm => CryptoInfo::Aes128Gcm(libc::tls12_crypto_info_aes_gcm_128 {
                info: info(libc::TLS_CIPHER_AES_GCM_128),
                iv: explicit,
                key: key.try_into().unwrap(),
                salt,
                rec_seq,
            }),
            Cipher::Aes256Gcm => CryptoInfo::Aes256Gcm(libc::tls12_crypto_info_aes_gcm_256 {
                info: info(libc::TLS_CIPHER_AES_GCM_256),
                iv: explicit,
                key: key.try_into().unwrap(),
                salt,
                rec_seq,
            }),
            Cipher::Chacha20Poly1305 => {
                CryptoInfo::Chacha20Poly1305(libc::tls12_crypto_info_chacha20_poly1305 {
                    info: info(libc::TLS_CIPHER_CHACHA20_POLY1305),
                    iv: *iv,
                    key: key.try_into().unwrap(),
                    salt: [],
                    rec_seq,
                })
            }
        })
    }

    /// Sets it as `name` (`TLS_TX` or `TLS_RX`) on a socket already under the `tls` ULP.
    fn set(&self, fd: BorrowedFd<'_>, name: libc::c_int) -> io::Result<()> {
        match self {
            CryptoInfo::Aes128Gcm(c) => setsockopt(fd, name, c),
            CryptoInfo::Aes256Gcm(c) => setsockopt(fd, name, c),
            CryptoInfo::Chacha20Poly1305(c) => setsockopt(fd, name, c),
        }
    }
}

impl Drop for CryptoInfo {
    fn drop(&mut self) {
        match self {
            CryptoInfo::Aes128Gcm(c) => {
                c.key.zeroize();
                c.iv.zeroize();
                c.salt.zeroize();
            }
            CryptoInfo::Aes256Gcm(c) => {
                c.key.zeroize();
                c.iv.zeroize();
                c.salt.zeroize();
            }
            CryptoInfo::Chacha20Poly1305(c) => {
                c.key.zeroize();
                c.iv.zeroize();
            }
        }
    }
}

/// Where the kernel's TLS 1.2 AES-GCM explicit nonce starts; it adds one per record sent.
fn tls12_tx_explicit_nonce(explicit: u64, seq: u64) -> io::Result<u64> {
    if seq >= 1 << 63 {
        return Err(invalid("a TLS 1.2 send sequence of 2^63 or more"));
    }
    // rustls sent `explicit ^ k` for each k < seq, all inside explicit's aligned block of
    // `seq.next_power_of_two()`; starting past that block reuses none of them.
    let low = seq.max(1).next_power_of_two() - 1;
    Ok((explicit | low).wrapping_add(1))
}

fn setsockopt<T>(fd: BorrowedFd<'_>, name: libc::c_int, value: &T) -> io::Result<()> {
    // SAFETY: `value` points at a live `T` of the length passed.
    let r = unsafe {
        libc::setsockopt(
            fd.as_raw_fd(),
            libc::SOL_TLS,
            name,
            (value as *const T).cast(),
            size_of::<T>() as libc::socklen_t,
        )
    };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn key_len(cipher: Cipher) -> usize {
    match cipher {
        Cipher::Aes128Gcm => 16,
        Cipher::Aes256Gcm | Cipher::Chacha20Poly1305 => 32,
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("kernel TLS: {what}"))
}

/// Refuses a hand-over while rustls holds what extracting its secrets would drop: plaintext or a
/// close_notify it read past the handshake, or records it has yet to send.
pub fn check_drained<D>(conn: &mut ConnectionCommon<D>) -> io::Result<()> {
    let state = conn.process_new_packets().map_err(io::Error::other)?;
    let held = if state.plaintext_bytes_to_read() > 0 {
        "data read past the handshake"
    } else if state.peer_has_closed() {
        "a close_notify read past the handshake"
    } else if state.tls_bytes_to_write() > 0 {
        "records not yet sent"
    } else {
        return Ok(());
    };
    Err(io::Error::other(format!(
        "TLS hand-over refused: rustls holds {held}"
    )))
}

/// One direction's keys as rustls extracted them, with the TLS 1.3 `traffic_secret` a key update
/// derives the next keys from.
pub fn keys_from_rustls(
    version: ProtocolVersion,
    seq: u64,
    secrets: ConnectionTrafficSecrets,
    traffic_secret: Option<Zeroizing<Vec<u8>>>,
) -> io::Result<(Cipher, DirectionKeys)> {
    if !matches!(version, ProtocolVersion::TLSv1_2 | ProtocolVersion::TLSv1_3) {
        return Err(invalid(&format!("{version:?}")));
    }
    let (cipher, key, iv) = match &secrets {
        ConnectionTrafficSecrets::Aes128Gcm { key, iv } => (Cipher::Aes128Gcm, key, iv),
        ConnectionTrafficSecrets::Aes256Gcm { key, iv } => (Cipher::Aes256Gcm, key, iv),
        ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv } => {
            (Cipher::Chacha20Poly1305, key, iv)
        }
        _ => return Err(invalid("a cipher the kernel does not take")),
    };
    let key = key.as_ref();
    if key.len() != key_len(cipher) {
        return Err(invalid("a key of the wrong length for its cipher"));
    }
    Ok((
        cipher,
        DirectionKeys {
            seq,
            key: Zeroizing::new(key.to_vec()),
            iv: Zeroizing::new(iv.as_ref().to_vec()),
            secret: traffic_secret,
        },
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Alert {
    CloseNotify,
    Other { level: u8, description: u8 },
}

/// The body of an alert record: a level and a description, close_notify being description 0.
pub(crate) fn alert(bytes: &[u8]) -> Alert {
    match *bytes {
        [_, 0] => Alert::CloseNotify,
        [level, description] => Alert::Other { level, description },
        _ => Alert::Other {
            level: bytes.first().copied().unwrap_or(0),
            description: bytes.get(1).copied().unwrap_or(0),
        },
    }
}

#[cfg(test)]
#[path = "../tests/fixture/mod.rs"]
mod fixture;

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::io::Write;

    use super::*;
    use rustls::crypto::aws_lc_rs::cipher_suite;
    use rustls::crypto::cipher::AeadKey;
    use rustls::{ClientConnection, ServerConnection, SupportedCipherSuite};

    /// Moves every record `from` has to send into `to` and processes them.
    fn send<A, B>(from: &mut ConnectionCommon<A>, to: &mut ConnectionCommon<B>) {
        let mut wire = Vec::new();
        while from.wants_write() {
            from.write_tls(&mut wire).unwrap();
        }
        let mut rd = &wire[..];
        while !rd.is_empty() {
            to.read_tls(&mut rd).unwrap();
            to.process_new_packets().unwrap();
        }
    }

    /// A client offering only `suite` and a server, in memory and past their handshake.
    fn connected(suite: SupportedCipherSuite) -> (ClientConnection, ServerConnection) {
        let mut client =
            ClientConnection::new(fixture::client_config(suite), "reprise".try_into().unwrap())
                .unwrap();
        let mut server = ServerConnection::new(fixture::server_config()).unwrap();
        while client.is_handshaking() || server.is_handshaking() || server.wants_write() {
            send(&mut client, &mut server);
            send(&mut server, &mut client);
        }
        (client, server)
    }

    #[test]
    fn an_alert_close_notify_is_eof_and_others_are_errors() {
        assert_eq!(alert(&[1, 0]), Alert::CloseNotify);
        assert_eq!(
            alert(&[2, 40]),
            Alert::Other {
                level: 2,
                description: 40
            }
        );
    }

    #[test]
    fn keys_from_rustls_maps_every_suite() {
        let suites = [
            (
                cipher_suite::TLS13_AES_128_GCM_SHA256,
                Cipher::Aes128Gcm,
                16,
            ),
            (
                cipher_suite::TLS13_AES_256_GCM_SHA384,
                Cipher::Aes256Gcm,
                32,
            ),
            (
                cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
                Cipher::Chacha20Poly1305,
                32,
            ),
            (
                cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
                Cipher::Aes128Gcm,
                16,
            ),
            (
                cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,
                Cipher::Aes256Gcm,
                32,
            ),
            (
                cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256,
                Cipher::Chacha20Poly1305,
                32,
            ),
        ];
        for (suite, cipher, key_len) in suites {
            let name = suite.suite();
            let (_, server) = connected(suite);
            let version = server.protocol_version().unwrap();
            let secrets = server.dangerous_extract_secrets().unwrap();
            let (seq, tx) = secrets.tx;
            let secret = (version == ProtocolVersion::TLSv1_3).then(|| Zeroizing::new(vec![7; 48]));
            let (got, keys) = keys_from_rustls(version, seq, tx, secret.clone()).unwrap();
            assert_eq!(got, cipher, "{name:?}");
            assert_eq!(keys.seq, seq, "{name:?}");
            assert_eq!(keys.key.len(), key_len, "{name:?}");
            assert_eq!(keys.iv.len(), 12, "{name:?}");
            assert_eq!(keys.secret, secret, "{name:?}");
        }
        let long = ConnectionTrafficSecrets::Aes128Gcm {
            key: AeadKey::from([0; 32]),
            iv: rustls::crypto::cipher::Iv::new([0; 12]),
        };
        assert!(
            keys_from_rustls(ProtocolVersion::TLSv1_3, 0, long, None).is_err(),
            "a 32-byte AES-128 key is refused"
        );
    }

    #[test]
    fn the_tls12_send_nonce_starts_past_every_nonce_rustls_sent() {
        let explicits = [
            0x0123_4567_89ab_cd00,
            0x0123_4567_89ab_cd01,
            0x0123_4567_89ab_cdff,
            u64::MAX - 3,
            u64::MAX,
        ];
        for explicit in explicits {
            for seq in [0, 1, 2, 3, 5, 1000, (1 << 20) + 7] {
                let start = tls12_tx_explicit_nonce(explicit, seq).unwrap();
                let kernel: HashSet<u64> = (0..=10_000).map(|j| start.wrapping_add(j)).collect();
                for k in 0..seq {
                    assert!(
                        !kernel.contains(&(explicit ^ k)),
                        "explicit {explicit:#x}, seq {seq}: rustls's nonce for record {k} is not sent again"
                    );
                }
            }
        }
        assert!(
            tls12_tx_explicit_nonce(0, 1 << 63).is_err(),
            "a sequence of 2^63 is refused"
        );
    }

    #[test]
    fn a_tls12_gcm_send_key_starts_the_kernel_past_rustls_nonces() {
        let (_, mut server) = connected(cipher_suite::TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256);
        for _ in 0..3 {
            server.writer().write_all(b"data").unwrap();
            let mut wire = Vec::new();
            while server.wants_write() {
                server.write_tls(&mut wire).unwrap();
            }
        }
        let secrets = server.dangerous_extract_secrets().unwrap();
        let ((seq, tx), (rx_seq, rx)) = (secrets.tx, secrets.rx);
        assert_eq!(seq, 4, "Finished and three data records were sent");
        let (cipher, tx) = keys_from_rustls(ProtocolVersion::TLSv1_2, seq, tx, None).unwrap();
        let (_, rx) = keys_from_rustls(ProtocolVersion::TLSv1_2, rx_seq, rx, None).unwrap();
        let explicit = |keys: &DirectionKeys| u64::from_be_bytes(keys.iv[4..].try_into().unwrap());
        let start = tls12_tx_explicit_nonce(explicit(&tx), seq).unwrap();
        assert_ne!(start, explicit(&tx));
        let CryptoInfo::Aes128Gcm(info) =
            &CryptoInfo::new(Direction::Tx, TlsVersion::Tls12, cipher, &tx).unwrap()
        else {
            panic!("an AES-128-GCM send key");
        };
        assert_eq!(
            info.iv,
            start.to_be_bytes(),
            "the kernel starts past rustls's nonces"
        );
        assert_eq!(info.salt, tx.iv[..4], "the salt is the iv's first 4 bytes");
        assert_eq!(info.rec_seq, seq.to_be_bytes());
        let CryptoInfo::Aes128Gcm(info) =
            &CryptoInfo::new(Direction::Rx, TlsVersion::Tls12, cipher, &rx).unwrap()
        else {
            panic!("an AES-128-GCM receive key");
        };
        assert_eq!(
            info.iv,
            rx.iv[4..],
            "a receive key keeps rustls's explicit iv"
        );
    }

    #[test]
    fn a_hand_over_is_refused_while_rustls_holds_unread_data() {
        let suite = cipher_suite::TLS13_AES_128_GCM_SHA256;
        let (_, mut server) = connected(suite);
        assert!(
            check_drained(&mut server).is_ok(),
            "a drained connection hands over"
        );

        let (mut client, mut server) = connected(suite);
        client.writer().write_all(b"early").unwrap();
        let mut wire = Vec::new();
        while client.wants_write() {
            client.write_tls(&mut wire).unwrap();
        }
        server.read_tls(&mut &wire[..]).unwrap();
        assert!(
            check_drained(&mut server).is_err(),
            "an unprocessed data record"
        );

        let (mut client, mut server) = connected(suite);
        client.writer().write_all(b"early").unwrap();
        send(&mut client, &mut server);
        assert!(check_drained(&mut server).is_err(), "unread plaintext");

        let (mut client, mut server) = connected(suite);
        client.send_close_notify();
        send(&mut client, &mut server);
        assert!(
            check_drained(&mut server).is_err(),
            "a received close_notify"
        );

        let (_, mut server) = connected(suite);
        server.writer().write_all(b"late").unwrap();
        assert!(check_drained(&mut server).is_err(), "a record not yet sent");
    }
}
