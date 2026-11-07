//! Kernel TLS: installing the traffic keys, key updates and the record limit.

mod stream;

use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};

use repriselogin::handover::{Cipher, DirectionKeys, TlsState, TlsVersion};
use rustls::{ConnectionTrafficSecrets, ProtocolVersion};
use zeroize::{Zeroize, Zeroizing};

pub use stream::KtlsStream;

/// Moves `tls` into the kernel for the connected TCP socket `fd`; from then on plain reads and
/// writes on it carry TLS records.
pub fn install(fd: BorrowedFd<'_>, tls: &TlsState) -> io::Result<()> {
    let version = match tls.version {
        TlsVersion::Tls12 => libc::TLS_1_2_VERSION,
        TlsVersion::Tls13 => libc::TLS_1_3_VERSION,
    };
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
    set_keys(fd, libc::TLS_TX, version, tls.cipher, &tls.tx)?;
    set_keys(fd, libc::TLS_RX, version, tls.cipher, &tls.rx)
}

/// Sets one direction (`TLS_TX` or `TLS_RX`) of a socket already under the `tls` ULP.
fn set_keys(
    fd: BorrowedFd<'_>,
    direction: libc::c_int,
    version: u16,
    cipher: Cipher,
    keys: &DirectionKeys,
) -> io::Result<()> {
    let iv: &[u8; 12] = keys.iv[..]
        .try_into()
        .map_err(|_| invalid("an iv not 12 bytes"))?;
    let key = keys.key.as_slice();
    if key.len() != key_len(cipher) {
        return Err(invalid("a key of the wrong length for its cipher"));
    }
    let info = |cipher_type| libc::tls_crypto_info {
        version,
        cipher_type,
    };
    let rec_seq = keys.seq.to_be_bytes();
    match cipher {
        Cipher::Aes128Gcm => {
            let mut c = libc::tls12_crypto_info_aes_gcm_128 {
                info: info(libc::TLS_CIPHER_AES_GCM_128),
                iv: iv[4..].try_into().unwrap(),
                key: key.try_into().unwrap(),
                salt: iv[..4].try_into().unwrap(),
                rec_seq,
            };
            let set = setsockopt(fd, direction, &c);
            c.key.zeroize();
            c.iv.zeroize();
            c.salt.zeroize();
            set
        }
        Cipher::Aes256Gcm => {
            let mut c = libc::tls12_crypto_info_aes_gcm_256 {
                info: info(libc::TLS_CIPHER_AES_GCM_256),
                iv: iv[4..].try_into().unwrap(),
                key: key.try_into().unwrap(),
                salt: iv[..4].try_into().unwrap(),
                rec_seq,
            };
            let set = setsockopt(fd, direction, &c);
            c.key.zeroize();
            c.iv.zeroize();
            c.salt.zeroize();
            set
        }
        Cipher::Chacha20Poly1305 => {
            let mut c = libc::tls12_crypto_info_chacha20_poly1305 {
                info: info(libc::TLS_CIPHER_CHACHA20_POLY1305),
                iv: *iv,
                key: key.try_into().unwrap(),
                salt: [],
                rec_seq,
            };
            let set = setsockopt(fd, direction, &c);
            c.key.zeroize();
            c.iv.zeroize();
            set
        }
    }
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
    // An `AeadKey` built from an array is 32 bytes long; AES-128 takes the first 16.
    let key = key
        .as_ref()
        .get(..key_len(cipher))
        .ok_or_else(|| invalid("a short key"))?;
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
mod tests {
    use super::*;
    use rustls::crypto::cipher::{AeadKey, Iv};

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
        type Variant = fn(AeadKey, Iv) -> ConnectionTrafficSecrets;
        let suites: [(Variant, Cipher, usize); 3] = [
            (
                |key, iv| ConnectionTrafficSecrets::Aes128Gcm { key, iv },
                Cipher::Aes128Gcm,
                16,
            ),
            (
                |key, iv| ConnectionTrafficSecrets::Aes256Gcm { key, iv },
                Cipher::Aes256Gcm,
                32,
            ),
            (
                |key, iv| ConnectionTrafficSecrets::Chacha20Poly1305 { key, iv },
                Cipher::Chacha20Poly1305,
                32,
            ),
        ];
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let iv: [u8; 12] = std::array::from_fn(|i| 100 + i as u8);
        for version in [ProtocolVersion::TLSv1_2, ProtocolVersion::TLSv1_3] {
            for (variant, cipher, key_len) in suites {
                let secret =
                    (version == ProtocolVersion::TLSv1_3).then(|| Zeroizing::new(vec![7; 48]));
                let (got, keys) = keys_from_rustls(
                    version,
                    9,
                    variant(AeadKey::from(key), Iv::new(iv)),
                    secret.clone(),
                )
                .unwrap();
                assert_eq!(got, cipher, "{version:?}");
                assert_eq!(keys.seq, 9);
                assert_eq!(*keys.key, key[..key_len], "{version:?} {cipher:?}");
                assert_eq!(*keys.iv, iv, "{version:?} {cipher:?}");
                assert_eq!(keys.secret, secret);
            }
        }
    }
}
