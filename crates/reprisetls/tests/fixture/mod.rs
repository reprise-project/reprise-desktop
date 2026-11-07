//! A self-signed server with secret extraction and a key log, and a client that offers one suite,
//! accepts any certificate and can extract its secrets.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::aws_lc_rs as provider;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    ClientConfig, DigitallySignedStruct, KeyLog, ServerConfig, SignatureScheme,
    SupportedCipherSuite,
};
use zeroize::Zeroizing;

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

/// Every secret one connection's key log gives, by label.
#[derive(Debug, Default)]
pub struct Secrets(Mutex<Vec<(String, Vec<u8>)>>);

impl KeyLog for Secrets {
    fn log(&self, label: &str, _: &[u8], secret: &[u8]) {
        self.0.lock().unwrap().push((label.into(), secret.to_vec()));
    }

    fn will_log(&self, _: &str) -> bool {
        true
    }
}

impl Secrets {
    pub fn get(&self, label: &str) -> Option<Zeroizing<Vec<u8>>> {
        let logged = self.0.lock().unwrap();
        let (_, secret) = logged.iter().find(|(l, _)| l == label)?;
        Some(Zeroizing::new(secret.clone()))
    }
}

/// A server config for one connection, with the key log that connection writes to.
pub fn logged_server_config() -> (Arc<ServerConfig>, Arc<Secrets>) {
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
    let secrets = Arc::new(Secrets::default());
    config.key_log = secrets.clone();
    (Arc::new(config), secrets)
}

pub fn client_config(suite: SupportedCipherSuite) -> Arc<ClientConfig> {
    let mut provider = provider::default_provider();
    provider.cipher_suites = vec![suite];
    let mut config = ClientConfig::builder_with_provider(Arc::new(provider))
        .with_protocol_versions(&[suite.version()])
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AnyCert))
        .with_no_client_auth();
    config.enable_secret_extraction = true;
    Arc::new(config)
}
