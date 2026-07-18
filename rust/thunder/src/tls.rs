//! Optional TLS transport (SPEC-008 CAN-020, SRV-040, FR-29).
//!
//! TLS is an **additive, off-by-default** capability: the plaintext path is
//! untouched and carries no rustls dependency unless the crate is built with
//! `--features tls`. There is **no STARTTLS** — TLS is decided at connect time,
//! before any Thunder frame is exchanged, so the wire codec never sees the
//! difference between a plaintext and an encrypted byte stream.
//!
//! The config types ([`ServerTls`], [`ClientTls`]) are plain data and always
//! compile, so an application can carry TLS settings regardless of the feature;
//! only the rustls acceptor/connector builders below are feature-gated. A
//! deployment that sets TLS config without the `tls` feature is refused at
//! connect time with a clear error rather than silently running plaintext.

use std::path::PathBuf;

/// Server-side TLS material (SRV-040). Presence of this on the listener config
/// turns TLS on for that deployment; absence keeps it plaintext.
#[derive(Clone, Debug)]
pub struct ServerTls {
    /// PEM certificate chain path.
    pub cert_path: PathBuf,
    /// PEM private key path (PKCS#8 / RSA / SEC1).
    pub key_path: PathBuf,
}

/// Client-side TLS material (FR-29). Presence of this on the client config
/// makes the client dial TLS; absence keeps it plaintext.
#[derive(Clone, Debug, Default)]
pub struct ClientTls {
    /// Name to verify the server certificate against (SNI). When `None`, the
    /// endpoint host is used.
    pub server_name: Option<String>,
    /// A PEM file of trusted root(s) to pin. When `None`, the platform's
    /// native root store is used.
    pub ca_path: Option<PathBuf>,
}

#[cfg(feature = "tls")]
mod imp {
    use std::fs::File;
    use std::io::BufReader;
    use std::sync::Arc;

    use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
    use tokio_rustls::rustls::{self, ClientConfig, RootCertStore, ServerConfig};
    use tokio_rustls::{TlsAcceptor, TlsConnector};

    use super::{ClientTls, ServerTls};

    fn provider() -> Arc<rustls::crypto::CryptoProvider> {
        Arc::new(rustls::crypto::ring::default_provider())
    }

    fn load_certs(path: &std::path::Path) -> Result<Vec<CertificateDer<'static>>, String> {
        let mut reader = BufReader::new(
            File::open(path).map_err(|e| format!("open cert {}: {e}", path.display()))?,
        );
        rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read certs {}: {e}", path.display()))
    }

    fn load_key(path: &std::path::Path) -> Result<PrivateKeyDer<'static>, String> {
        let mut reader = BufReader::new(
            File::open(path).map_err(|e| format!("open key {}: {e}", path.display()))?,
        );
        rustls_pemfile::private_key(&mut reader)
            .map_err(|e| format!("read key {}: {e}", path.display()))?
            .ok_or_else(|| format!("no private key in {}", path.display()))
    }

    /// Build the server's `TlsAcceptor` from its cert/key (SRV-040). No client
    /// auth (mTLS is a later, additive capability).
    pub fn build_acceptor(cfg: &ServerTls) -> Result<TlsAcceptor, String> {
        let certs = load_certs(&cfg.cert_path)?;
        let key = load_key(&cfg.key_path)?;
        let config = ServerConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("rustls protocol versions: {e}"))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| format!("server cert/key: {e}"))?;
        Ok(TlsAcceptor::from(Arc::new(config)))
    }

    /// Build the client's `TlsConnector` (FR-29): pin the configured CA, or fall
    /// back to the platform's native root store.
    pub fn build_connector(cfg: &ClientTls) -> Result<TlsConnector, String> {
        let mut roots = RootCertStore::empty();
        match &cfg.ca_path {
            Some(path) => {
                for cert in load_certs(path)? {
                    roots
                        .add(cert)
                        .map_err(|e| format!("add CA from {}: {e}", path.display()))?;
                }
            }
            None => {
                let loaded = rustls_native_certs::load_native_certs();
                if roots.is_empty() && loaded.certs.is_empty() {
                    return Err(format!(
                        "no native root certificates available ({} load error(s))",
                        loaded.errors.len()
                    ));
                }
                for cert in loaded.certs {
                    let _ = roots.add(cert);
                }
            }
        }
        let config = ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("rustls protocol versions: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        Ok(TlsConnector::from(Arc::new(config)))
    }

    /// The SNI / verification name: the configured `server_name`, else `host`.
    pub fn server_name(cfg: &ClientTls, host: &str) -> Result<ServerName<'static>, String> {
        let name = cfg.server_name.clone().unwrap_or_else(|| host.to_owned());
        ServerName::try_from(name).map_err(|e| format!("invalid TLS server name: {e}"))
    }
}

#[cfg(feature = "tls")]
pub use imp::{build_acceptor, build_connector, server_name};
