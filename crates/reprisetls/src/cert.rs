//! The server's self-signed TLS certificate, kept as `cert.pem` and `key.pem` in its directory.

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::rustls::crypto::aws_lc_rs;
use tokio_rustls::rustls::pki_types::pem::{self, PemObject};
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio_rustls::rustls::{KeyLog, ServerConfig};

const CERT: &str = "cert.pem";
const KEY: &str = "key.pem";

pub struct Identity {
    pub acceptor: TlsAcceptor,
    /// SHA-256 of the certificate's DER, as uppercase hex pairs joined by `:`.
    pub fingerprint: String,
    config: Arc<ServerConfig>,
    public_key: Vec<u8>,
}

impl Identity {
    /// A copy of the TLS configuration for one connection, with secret extraction on when
    /// `extract` and the connection's secrets going to `key_log`.
    pub fn server_config(
        &self,
        extract: bool,
        key_log: Option<Arc<dyn KeyLog>>,
    ) -> Arc<ServerConfig> {
        let mut config = (*self.config).clone();
        config.enable_secret_extraction = extract;
        if let Some(key_log) = key_log {
            config.key_log = key_log;
        }
        Arc::new(config)
    }

    /// The subjectPublicKey bits of the certificate's SubjectPublicKeyInfo, which CredSSP binds to.
    pub fn public_key(&self) -> Vec<u8> {
        self.public_key.clone()
    }
}

/// The certificate in `dir`, or a new one when it is missing or invalid; any other error (a
/// read that may pass next time) is returned, so clients never see the fingerprint change for it.
pub fn load_or_create(dir: &Path) -> io::Result<Identity> {
    if let Err(e) = owner_only(dir) {
        log::warn!("{}: {e}", dir.display());
    }
    match load(dir) {
        Ok(identity) => Ok(identity),
        Err(e) if e.kind() == io::ErrorKind::NotFound => regenerate(dir),
        Err(e) if e.kind() == io::ErrorKind::InvalidData => {
            log::warn!("{}: {e}; making a new certificate", dir.display());
            regenerate(dir)
        }
        Err(e) => Err(e),
    }
}

/// Replace the certificate in `dir` with a new one.
pub fn regenerate(dir: &Path) -> io::Result<Identity> {
    let key = rcgen::KeyPair::generate().map_err(io::Error::other)?;
    let cert = rcgen::CertificateParams::new(vec![hostname()])
        .and_then(|p| p.self_signed(&key))
        .map_err(io::Error::other)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    owner_only(dir)?;
    replace(&dir.join(KEY), key.serialize_pem().as_bytes(), 0o600)?;
    replace(&dir.join(CERT), cert.pem().as_bytes(), 0o600)?;
    identity(
        cert.der().clone(),
        PrivateKeyDer::try_from(key.serialize_der()).map_err(io::Error::other)?,
    )
}

/// `dir` 0700 and the files in it 0600, whatever made them; nothing to do when it is missing.
fn owner_only(dir: &Path) -> io::Result<()> {
    chmod(dir, 0o700)?;
    chmod(&dir.join(KEY), 0o600)?;
    chmod(&dir.join(CERT), 0o600)
}

fn chmod(path: &Path, mode: u32) -> io::Result<()> {
    match fs::set_permissions(path, fs::Permissions::from_mode(mode)) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        set => set,
    }
}

fn load(dir: &Path) -> io::Result<Identity> {
    let cert = CertificateDer::from_pem_file(dir.join(CERT)).map_err(pem_error)?;
    let key = PrivateKeyDer::from_pem_file(dir.join(KEY)).map_err(pem_error)?;
    identity(cert, key)
}

fn pem_error(e: pem::Error) -> io::Error {
    match e {
        pem::Error::Io(e) => e,
        e => io::Error::new(io::ErrorKind::InvalidData, e),
    }
}

fn identity(cert: CertificateDer<'static>, key: PrivateKeyDer<'static>) -> io::Result<Identity> {
    let fingerprint = fingerprint(&cert);
    let public_key = rcgen::KeyPair::try_from(&key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?
        .public_key_raw()
        .to_vec();
    let config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(io::Error::other)?
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let config = Arc::new(config);
    Ok(Identity {
        acceptor: TlsAcceptor::from(config.clone()),
        fingerprint,
        config,
        public_key,
    })
}

fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Replaces `path` with `contents` through a temporary file of `mode`, whatever the umask, so it
/// never holds half of them.
pub fn replace(path: &Path, contents: &[u8], mode: u32) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    match fs::remove_file(&tmp) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)?;
    f.set_permissions(fs::Permissions::from_mode(mode))?;
    f.write_all(contents)?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

/// The kernel's host name, or `localhost` when it has none.
pub fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|h| h.trim().to_string())
        .ok()
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "localhost".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loose_modes_and_a_stale_temporary_key_are_made_owner_only() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("rdp");
        let first = load_or_create(&dir).unwrap();
        let loosen = |path: &Path, mode| {
            fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap()
        };
        loosen(&dir, 0o755);
        loosen(&dir.join(KEY), 0o644);
        loosen(&dir.join(CERT), 0o644);
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let loaded = load_or_create(&dir).unwrap();
        assert_eq!(loaded.fingerprint, first.fingerprint);
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&dir.join(KEY)), 0o600);
        assert_eq!(mode(&dir.join(CERT)), 0o600);

        let stale = dir.join("key.pem.tmp");
        fs::write(&stale, "old").unwrap();
        loosen(&stale, 0o644);
        regenerate(&dir).unwrap();
        assert!(!stale.exists(), "the temporary key was renamed into place");
        assert_eq!(mode(&dir.join(KEY)), 0o600);
    }

    #[test]
    fn the_certificate_is_created_once_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().join("rdp");
        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first.fingerprint, second.fingerprint);
        let mode = |name| fs::metadata(dir.join(name)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(KEY), 0o600);
        assert_eq!(mode(CERT), 0o600);
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }

    #[test]
    fn regenerate_changes_the_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        let second = regenerate(dir.path()).unwrap();
        assert_ne!(first.fingerprint, second.fingerprint);
        assert_eq!(
            load_or_create(dir.path()).unwrap().fingerprint,
            second.fingerprint
        );
    }

    #[test]
    fn the_fingerprint_is_uppercase_hex_pairs() {
        let f = load_or_create(tempfile::tempdir().unwrap().path())
            .unwrap()
            .fingerprint;
        assert_eq!(f.len(), 32 * 3 - 1);
        assert!(f.split(':').all(|p| {
            p.len() == 2
                && p.chars()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        }));
    }

    #[test]
    fn the_public_key_is_the_certificates_subject_key() {
        let dir = tempfile::tempdir().unwrap();
        let created = load_or_create(dir.path()).unwrap().public_key();
        assert_eq!(created.len(), 65, "an uncompressed P-256 point");
        assert_eq!(created[0], 4);
        let cert = CertificateDer::from_pem_file(dir.path().join(CERT)).unwrap();
        let mut bit_string = vec![0x03, created.len() as u8 + 1, 0];
        bit_string.extend_from_slice(&created);
        assert!(
            cert.windows(bit_string.len()).any(|w| w == bit_string),
            "the certificate's subjectPublicKey is the key"
        );
        assert_eq!(load_or_create(dir.path()).unwrap().public_key(), created);
    }

    #[derive(Debug)]
    struct NoLog;

    impl KeyLog for NoLog {
        fn log(&self, _: &str, _: &[u8], _: &[u8]) {}
    }

    #[test]
    fn a_connection_config_carries_extraction_and_its_key_log() {
        let identity = load_or_create(tempfile::tempdir().unwrap().path()).unwrap();
        let key_log: Arc<dyn KeyLog> = Arc::new(NoLog);
        let config = identity.server_config(true, Some(key_log.clone()));
        assert!(config.enable_secret_extraction);
        assert!(std::ptr::addr_eq(
            Arc::as_ptr(&config.key_log),
            Arc::as_ptr(&key_log)
        ));
        let plain = identity.server_config(false, None);
        assert!(!plain.enable_secret_extraction);
        assert!(!std::ptr::addr_eq(
            Arc::as_ptr(&plain.key_log),
            Arc::as_ptr(&key_log)
        ));
    }

    #[test]
    fn an_invalid_key_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let first = load_or_create(dir.path()).unwrap();
        fs::write(dir.path().join(KEY), "not a key").unwrap();
        assert_ne!(
            load_or_create(dir.path()).unwrap().fingerprint,
            first.fingerprint
        );
    }

    #[test]
    fn a_read_error_is_returned_and_keeps_the_key() {
        let dir = tempfile::tempdir().unwrap();
        load_or_create(dir.path()).unwrap();
        let key = fs::read(dir.path().join(KEY)).unwrap();
        fs::remove_file(dir.path().join(CERT)).unwrap();
        fs::create_dir(dir.path().join(CERT)).unwrap();
        let e = load_or_create(dir.path())
            .err()
            .expect("a certificate that cannot be read is an error");
        assert_eq!(e.kind(), io::ErrorKind::IsADirectory);
        assert_eq!(
            fs::read(dir.path().join(KEY)).unwrap(),
            key,
            "the key is kept"
        );
    }
}
