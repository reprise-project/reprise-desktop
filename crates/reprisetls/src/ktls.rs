//! Kernel TLS: installing the traffic keys.

mod stream;

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use aws_lc_rs::{constant_time, hkdf};
use repriselogin::handover::{Cipher, DirectionKeys, TlsState, TlsVersion};
use rustls::{
    ConnectionCommon, ConnectionTrafficSecrets, ProtocolVersion, ServerConnection,
    SupportedCipherSuite,
};
use zeroize::{Zeroize, Zeroizing};

use crate::cork::Cork;

pub use stream::KtlsStream;

/// Moves `tls` into the kernel for the connected TCP socket `fd`; from then on plain reads and
/// writes on it carry TLS records.
pub fn install(fd: BorrowedFd<'_>, tls: &TlsState) -> io::Result<()> {
    checked_secret(tls.cipher, &tls.tx)?;
    checked_secret(tls.cipher, &tls.rx)?;
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
pub(crate) enum Direction {
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

/// Reads `name` into `value`, which the kernel fills whole, its key included.
fn getsockopt<T>(fd: BorrowedFd<'_>, name: libc::c_int, value: &mut T) -> io::Result<()> {
    let mut len = size_of::<T>() as libc::socklen_t;
    // SAFETY: `value` points at a live, writable `T` of the length passed.
    let r = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_TLS,
            name,
            (value as *mut T).cast(),
            &mut len,
        )
    };
    if r < 0 {
        Err(io::Error::last_os_error())
    } else if len as usize != size_of::<T>() {
        Err(invalid("a crypto info of the wrong length"))
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
fn check_drained<D>(conn: &mut ConnectionCommon<D>) -> io::Result<()> {
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
#[doc(hidden)]
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

/// The TLS 1.3 application traffic secrets, as rustls's `KeyLog` gives them under
/// `CLIENT_TRAFFIC_SECRET_0` and `SERVER_TRAFFIC_SECRET_0`.
pub struct TrafficSecrets {
    pub client: Zeroizing<Vec<u8>>,
    pub server: Zeroizing<Vec<u8>>,
}

/// Takes a server's keys out of rustls once nothing can be lost on the way into the kernel. TLS
/// 1.3 also needs `secrets`, the ones the keys came from, which key updates derive from.
pub fn hand_over<S>(
    cork: &Cork<S>,
    mut conn: ServerConnection,
    secrets: Option<TrafficSecrets>,
) -> io::Result<TlsState> {
    let refused = |why: &str| io::Error::other(format!("TLS hand-over refused: {why}"));
    if !cork.at_boundary() {
        return Err(refused("a partial record was read"));
    }
    check_drained(&mut conn)?;
    let (version, protocol, record_limit) = match conn.negotiated_cipher_suite() {
        Some(SupportedCipherSuite::Tls13(suite)) => (
            TlsVersion::Tls13,
            ProtocolVersion::TLSv1_3,
            suite.common.confidentiality_limit,
        ),
        Some(SupportedCipherSuite::Tls12(suite)) => (
            TlsVersion::Tls12,
            ProtocolVersion::TLSv1_2,
            suite.common.confidentiality_limit,
        ),
        None => return Err(refused("the handshake is not done")),
    };
    let (tx_secret, rx_secret) = match (version, secrets) {
        (TlsVersion::Tls13, Some(s)) => (Some(s.server), Some(s.client)),
        (TlsVersion::Tls13, None) => return Err(refused("TLS 1.3 without its traffic secrets")),
        (TlsVersion::Tls12, _) => (None, None),
    };
    #[allow(deprecated)]
    let extracted = conn
        .dangerous_extract_secrets()
        .map_err(|e| refused(&format!("rustls gave no secrets: {e}")))?;
    let (cipher, tx) = keys_from_rustls(protocol, extracted.tx.0, extracted.tx.1, tx_secret)?;
    let (_, rx) = keys_from_rustls(protocol, extracted.rx.0, extracted.rx.1, rx_secret)?;
    // A key update before the hand-over leaves the keys a generation past the logged secrets;
    // updating from those would send under the current key again from sequence 0.
    if !(keys_match_secret(cipher, &tx) & keys_match_secret(cipher, &rx)) {
        return Err(refused(
            "the traffic secrets don't match the connection's keys",
        ));
    }
    Ok(TlsState {
        version,
        cipher,
        tx,
        rx,
        record_limit,
    })
}

/// Whether `keys` are the key and iv of their TLS 1.3 traffic secret, compared in constant time.
/// Keys without a secret (TLS 1.2) match; a secret of the wrong length does not.
fn keys_match_secret(cipher: Cipher, keys: &DirectionKeys) -> bool {
    let secret = match checked_secret(cipher, keys) {
        Ok(Some(secret)) => secret,
        Ok(None) => return true,
        Err(_) => return false,
    };
    let (key, iv) = traffic_keys(cipher, secret);
    let key_matches = constant_time::verify_slices_are_equal(&key, &keys.key).is_ok();
    let iv_matches = constant_time::verify_slices_are_equal(&iv, &keys.iv).is_ok();
    key_matches & iv_matches
}

/// How many records a sender keeps in hand below the record limit.
pub const REKEY_MARGIN: u64 = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordAction {
    Continue,
    KeyUpdate,
    Close,
}

/// What a sender whose next record is `seq` does under a limit of `limit` records per key.
pub fn record_action(version: TlsVersion, seq: u64, limit: u64) -> RecordAction {
    if seq < limit.saturating_sub(REKEY_MARGIN) {
        RecordAction::Continue
    } else if version == TlsVersion::Tls13 {
        RecordAction::KeyUpdate
    } else {
        RecordAction::Close
    }
}

/// The traffic secret after a key update (RFC 8446 7.2).
pub fn next_traffic_secret(cipher: Cipher, secret: &[u8]) -> Zeroizing<Vec<u8>> {
    expand_label(hash(cipher), secret, b"traffic upd", secret_len(cipher))
}

/// The record key and iv of a TLS 1.3 traffic secret (RFC 8446 7.3).
pub fn traffic_keys(cipher: Cipher, secret: &[u8]) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
    let hash = hash(cipher);
    (
        expand_label(hash, secret, b"key", key_len(cipher)),
        expand_label(hash, secret, b"iv", 12),
    )
}

/// `keys`' TLS 1.3 traffic secret, refused unless it is its suite's hash long.
fn checked_secret(cipher: Cipher, keys: &DirectionKeys) -> io::Result<Option<&[u8]>> {
    match keys.secret.as_deref() {
        Some(secret) if secret.len() != secret_len(cipher) => Err(invalid(
            "a traffic secret of the wrong length for its cipher",
        )),
        secret => Ok(secret.map(Vec::as_slice)),
    }
}

fn secret_len(cipher: Cipher) -> usize {
    hash(cipher)
        .hmac_algorithm()
        .digest_algorithm()
        .output_len()
}

/// The hash of the one TLS 1.3 suite using `cipher`.
fn hash(cipher: Cipher) -> hkdf::Algorithm {
    match cipher {
        Cipher::Aes256Gcm => hkdf::HKDF_SHA384,
        Cipher::Aes128Gcm | Cipher::Chacha20Poly1305 => hkdf::HKDF_SHA256,
    }
}

struct OutputLen(usize);

impl hkdf::KeyType for OutputLen {
    fn len(&self) -> usize {
        self.0
    }
}

/// HKDF-Expand-Label with an empty context (RFC 8446 7.1).
fn expand_label(
    hash: hkdf::Algorithm,
    secret: &[u8],
    label: &[u8],
    len: usize,
) -> Zeroizing<Vec<u8>> {
    let mut info = Vec::with_capacity(16);
    info.extend_from_slice(&(len as u16).to_be_bytes());
    info.push((6 + label.len()) as u8);
    info.extend_from_slice(b"tls13 ");
    info.extend_from_slice(label);
    info.push(0);
    let mut out = Zeroizing::new(vec![0; len]);
    hkdf::Prk::new_less_safe(hash, secret)
        .expand(&[&info], OutputLen(len))
        .and_then(|okm| okm.fill(&mut out))
        .expect("a traffic secret expands to a key, iv or secret");
    out
}

/// Whether a TLS 1.3 handshake record is a KeyUpdate asking for one back; any other
/// handshake message is `InvalidData`.
pub(crate) fn key_update(version: TlsVersion, record: &[u8]) -> io::Result<bool> {
    match (version, record) {
        (TlsVersion::Tls13, [24, 0, 0, 1, requested @ (0 | 1)]) => Ok(*requested == 1),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a TLS handshake message other than a key update",
        )),
    }
}

/// The keys after a key update, from `keys`' traffic secret, starting at sequence 0.
pub(crate) fn next_keys(cipher: Cipher, keys: &DirectionKeys) -> io::Result<DirectionKeys> {
    let secret = checked_secret(cipher, keys)?
        .ok_or_else(|| invalid("a key update without the traffic secret"))?;
    let next = next_traffic_secret(cipher, secret);
    let (key, iv) = traffic_keys(cipher, &next);
    Ok(DirectionKeys {
        seq: 0,
        key,
        iv,
        secret: Some(next),
    })
}

/// Installs `keys` as `direction`'s keys on a socket already in kernel TLS.
pub(crate) fn set_keys(
    fd: BorrowedFd<'_>,
    direction: Direction,
    version: TlsVersion,
    cipher: Cipher,
    keys: &DirectionKeys,
) -> io::Result<()> {
    let name = match direction {
        Direction::Tx => libc::TLS_TX,
        Direction::Rx => libc::TLS_RX,
    };
    CryptoInfo::new(direction, version, cipher, keys)?.set(fd, name)
}

/// The sequence number of the next record the kernel sends on `fd`.
pub(crate) fn sent_seq(fd: BorrowedFd<'_>, cipher: Cipher) -> io::Result<u64> {
    // SAFETY: these are plain C structs of integers and byte arrays, valid all zero.
    let mut info = unsafe {
        match cipher {
            Cipher::Aes128Gcm => CryptoInfo::Aes128Gcm(std::mem::zeroed()),
            Cipher::Aes256Gcm => CryptoInfo::Aes256Gcm(std::mem::zeroed()),
            Cipher::Chacha20Poly1305 => CryptoInfo::Chacha20Poly1305(std::mem::zeroed()),
        }
    };
    let rec_seq = match &mut info {
        CryptoInfo::Aes128Gcm(c) => getsockopt(fd, libc::TLS_TX, c).map(|()| c.rec_seq),
        CryptoInfo::Aes256Gcm(c) => getsockopt(fd, libc::TLS_TX, c).map(|()| c.rec_seq),
        CryptoInfo::Chacha20Poly1305(c) => getsockopt(fd, libc::TLS_TX, c).map(|()| c.rec_seq),
    }?;
    Ok(u64::from_be_bytes(rec_seq))
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

    use std::sync::Arc;

    use super::*;
    use rustls::crypto::aws_lc_rs::cipher_suite;
    use rustls::crypto::cipher::AeadKey;
    use rustls::server::UnbufferedServerConnection;
    use rustls::unbuffered::ConnectionState;
    use rustls::{ClientConnection, ServerConfig};
    use tokio::io::AsyncReadExt;

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
        connected_to(suite, fixture::logged_server_config().0)
    }

    fn connected_to(
        suite: SupportedCipherSuite,
        config: Arc<ServerConfig>,
    ) -> (ClientConnection, ServerConnection) {
        let mut client =
            ClientConnection::new(fixture::client_config(suite), "reprise".try_into().unwrap())
                .unwrap();
        let mut server = ServerConnection::new(config).unwrap();
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

    /// An unbuffered server past its handshake with a client offering only `suite`, and the
    /// server's key log.
    fn unbuffered(
        suite: SupportedCipherSuite,
    ) -> (UnbufferedServerConnection, Arc<fixture::Secrets>) {
        let (config, secrets) = fixture::logged_server_config();
        let mut client =
            ClientConnection::new(fixture::client_config(suite), "reprise".try_into().unwrap())
                .unwrap();
        let mut server = UnbufferedServerConnection::new(config).unwrap();
        let mut incoming = Vec::new();
        for _ in 0..10 {
            while client.wants_write() {
                client.write_tls(&mut incoming).unwrap();
            }
            let mut outgoing = Vec::new();
            loop {
                let status = server.process_tls_records(&mut incoming);
                let discard = status.discard;
                let more = match status.state.unwrap() {
                    ConnectionState::EncodeTlsData(mut data) => {
                        let mut buf = vec![0; 1 << 16];
                        let n = data.encode(&mut buf).unwrap();
                        outgoing.extend_from_slice(&buf[..n]);
                        true
                    }
                    ConnectionState::TransmitTlsData(data) => {
                        data.done();
                        true
                    }
                    ConnectionState::BlockedHandshake | ConnectionState::WriteTraffic(_) => false,
                    _ => panic!("the handshake reaches an unexpected state"),
                };
                incoming.drain(..discard);
                if !more {
                    break;
                }
            }
            let mut rd = &outgoing[..];
            while !rd.is_empty() {
                client.read_tls(&mut rd).unwrap();
                client.process_new_packets().unwrap();
            }
            if !server.is_handshaking() && !client.is_handshaking() && !client.wants_write() {
                return (server, secrets);
            }
        }
        panic!("the handshake ends within 10 rounds");
    }

    fn traffic(secrets: &fixture::Secrets) -> Option<TrafficSecrets> {
        Some(TrafficSecrets {
            client: secrets.get("CLIENT_TRAFFIC_SECRET_0")?,
            server: secrets.get("SERVER_TRAFFIC_SECRET_0")?,
        })
    }

    #[test]
    fn the_next_traffic_secret_matches_rustls() {
        let suites = [
            (cipher_suite::TLS13_AES_128_GCM_SHA256, Cipher::Aes128Gcm),
            (cipher_suite::TLS13_AES_256_GCM_SHA384, Cipher::Aes256Gcm),
            (
                cipher_suite::TLS13_CHACHA20_POLY1305_SHA256,
                Cipher::Chacha20Poly1305,
            ),
        ];
        for (suite, cipher) in suites {
            let name = suite.suite();
            let (server, secrets) = unbuffered(suite);
            let TrafficSecrets {
                client: mut rx,
                server: mut tx,
            } = traffic(&secrets).unwrap();
            let (_, mut kernel) = server.dangerous_into_kernel_connection().unwrap();
            for generation in 1..=2 {
                tx = next_traffic_secret(cipher, &tx);
                rx = next_traffic_secret(cipher, &rx);
                let updates = [
                    (&tx, kernel.update_tx_secret().unwrap(), "send"),
                    (&rx, kernel.update_rx_secret().unwrap(), "receive"),
                ];
                for (secret, (seq, theirs), side) in updates {
                    let (got, theirs) =
                        keys_from_rustls(ProtocolVersion::TLSv1_3, seq, theirs, None).unwrap();
                    let (key, iv) = traffic_keys(cipher, secret);
                    assert_eq!((got, seq), (cipher, 0), "{name:?}");
                    assert!(
                        key == theirs.key && iv == theirs.iv,
                        "{name:?} {side} generation {generation}: the keys equal rustls's"
                    );
                }
            }
        }
    }

    #[test]
    fn record_action_rekeys_tls13_and_closes_tls12_before_the_limit() {
        const L: u64 = 1 << 24;
        let cases = [
            (
                TlsVersion::Tls13,
                L - REKEY_MARGIN - 1,
                L,
                RecordAction::Continue,
            ),
            (
                TlsVersion::Tls13,
                L - REKEY_MARGIN,
                L,
                RecordAction::KeyUpdate,
            ),
            (TlsVersion::Tls12, L - REKEY_MARGIN, L, RecordAction::Close),
            (TlsVersion::Tls13, 1 << 40, u64::MAX, RecordAction::Continue),
        ];
        for (version, seq, limit, action) in cases {
            assert_eq!(
                record_action(version, seq, limit),
                action,
                "{version:?} at {seq} of {limit}"
            );
        }
    }

    #[test]
    fn only_a_whole_tls13_key_update_is_taken() {
        assert!(!key_update(TlsVersion::Tls13, &[24, 0, 0, 1, 0]).unwrap());
        assert!(key_update(TlsVersion::Tls13, &[24, 0, 0, 1, 1]).unwrap());
        let refused: [(TlsVersion, &[u8]); 5] = [
            (TlsVersion::Tls13, &[24, 0, 0, 1, 2]),
            (TlsVersion::Tls13, &[24, 0, 0, 1, 0, 24, 0, 0, 1, 0]),
            (TlsVersion::Tls13, &[4, 0, 0, 1, 0]),
            (TlsVersion::Tls12, &[24, 0, 0, 1, 0]),
            (TlsVersion::Tls12, &[0, 0, 0, 0]),
        ];
        for (version, record) in refused {
            let err = key_update(version, record).unwrap_err();
            assert_eq!(
                err.kind(),
                io::ErrorKind::InvalidData,
                "{version:?} {record:?}"
            );
        }
    }

    #[test]
    fn a_traffic_secret_not_its_hash_long_is_refused_at_hand_over_install_and_key_update() {
        let cases = [
            (Cipher::Aes128Gcm, 65),
            (Cipher::Aes128Gcm, 48),
            (Cipher::Aes256Gcm, 32),
            (Cipher::Chacha20Poly1305, 0),
        ];
        for (cipher, len) in cases {
            let keys = DirectionKeys {
                seq: 0,
                key: Zeroizing::new(vec![1; key_len(cipher)]),
                iv: Zeroizing::new(vec![2; 12]),
                secret: Some(Zeroizing::new(vec![3; len])),
            };
            assert!(!keys_match_secret(cipher, &keys), "{cipher:?} {len}");
            let err = next_keys(cipher, &keys).unwrap_err();
            assert!(
                err.to_string()
                    .contains("traffic secret of the wrong length"),
                "{cipher:?} {len}: {err}"
            );
            let tls = TlsState {
                version: TlsVersion::Tls13,
                cipher,
                tx: keys.clone(),
                rx: keys,
                record_limit: 1 << 24,
            };
            let (socket, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
            let err = install(std::os::fd::AsFd::as_fd(&socket), &tls).unwrap_err();
            assert!(
                err.to_string()
                    .contains("traffic secret of the wrong length"),
                "{cipher:?} {len}: {err}"
            );
        }
    }

    #[test]
    fn hand_over_refuses_traffic_secrets_behind_the_connection_keys() {
        let suite = cipher_suite::TLS13_AES_128_GCM_SHA256;
        let boundary = Cork::new(tokio::io::empty());
        let refused = |result: io::Result<TlsState>, case: &str| {
            let err = result.unwrap_err().to_string();
            assert!(err.contains("don't match"), "{case}: {err}");
        };

        let (config, secrets) = fixture::logged_server_config();
        let (mut client, mut server) = connected_to(suite, config);
        client.refresh_traffic_keys().unwrap();
        send(&mut client, &mut server);
        refused(
            hand_over(&boundary, server, traffic(&secrets)),
            "the client updated its keys and asked for the server's",
        );

        let (config, secrets) = fixture::logged_server_config();
        let (mut client, mut server) = connected_to(suite, config);
        server.refresh_traffic_keys().unwrap();
        send(&mut server, &mut client);
        refused(
            hand_over(&boundary, server, traffic(&secrets)),
            "the server updated its send keys",
        );

        let (config, secrets) = fixture::logged_server_config();
        let (_, server) = connected_to(suite, config);
        let TrafficSecrets { client, server: s } = traffic(&secrets).unwrap();
        let swapped = TrafficSecrets {
            client: s,
            server: client,
        };
        refused(
            hand_over(&boundary, server, Some(swapped)),
            "swapped secrets",
        );

        let (config, secrets) = fixture::logged_server_config();
        let (_, server) = connected_to(suite, config);
        assert!(
            hand_over(&boundary, server, traffic(&secrets)).is_ok(),
            "a connection that never updated its keys hands over"
        );
    }

    #[tokio::test]
    async fn hand_over_refuses_a_cut_record_held_data_and_no_extraction() {
        let suite = cipher_suite::TLS13_AES_128_GCM_SHA256;
        let boundary = Cork::new(tokio::io::empty());
        let mut cut = Cork::new(&[23u8, 3, 3][..]);
        assert_eq!(
            cut.read(&mut [0; 8]).await.unwrap(),
            3,
            "a header cut short"
        );
        let refusal = |result: io::Result<TlsState>| result.unwrap_err().to_string();

        let (config, secrets) = fixture::logged_server_config();
        let (_, server) = connected_to(suite, config);
        let err = refusal(hand_over(&cut, server, traffic(&secrets)));
        assert!(err.contains("partial record"), "a cut record: {err}");

        let (config, secrets) = fixture::logged_server_config();
        let (_, mut server) = connected_to(suite, config);
        server.writer().write_all(b"late").unwrap();
        let err = refusal(hand_over(&boundary, server, traffic(&secrets)));
        assert!(err.contains("rustls holds"), "held data: {err}");

        let (config, secrets) = fixture::logged_server_config();
        let mut config = (*config).clone();
        config.enable_secret_extraction = false;
        let (_, server) = connected_to(suite, Arc::new(config));
        let err = refusal(hand_over(&boundary, server, traffic(&secrets)));
        assert!(err.contains("no secrets"), "no extraction: {err}");

        let (_, server) = connected(suite);
        let err = refusal(hand_over(&boundary, server, None));
        assert!(
            err.contains("traffic secrets"),
            "TLS 1.3 without secrets: {err}"
        );

        let (config, secrets) = fixture::logged_server_config();
        let (_, server) = connected_to(suite, config);
        let tls = hand_over(&boundary, server, traffic(&secrets)).unwrap();
        let TrafficSecrets { client, server } = traffic(&secrets).unwrap();
        assert_eq!(
            (tls.version, tls.cipher, tls.record_limit),
            (TlsVersion::Tls13, Cipher::Aes128Gcm, 1 << 24)
        );
        assert!(
            tls.tx.secret == Some(server) && tls.rx.secret == Some(client),
            "a server sends under the server secret"
        );

        let (_, server) = connected(cipher_suite::TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256);
        let tls = hand_over(&boundary, server, None).unwrap();
        assert_eq!(
            (tls.version, tls.record_limit, tls.tx.secret),
            (TlsVersion::Tls12, u64::MAX, None)
        );
    }
}
