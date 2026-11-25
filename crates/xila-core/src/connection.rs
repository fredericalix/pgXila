//! Connection to a pgXila server.
//!
//! A pgXila client is an ordinary PostgreSQL client, with one fixed policy:
//! the connection is always encrypted with TLS 1.3, the server certificate and
//! host name are always verified, and authentication is always SCRAM-SHA-256
//! bound to the TLS channel. None of this can be turned off: there is no
//! setting for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use tokio_postgres::config::{ChannelBinding, SslMode};
use tokio_postgres::Socket;
use tokio_postgres_rustls::MakeRustlsConnect;

/// Name under which the clients appear in `pg_stat_activity`.
const APPLICATION_NAME: &str = "pgxila";

/// What is needed to reach a pgXila server.
#[derive(Clone)]
pub struct ConnectParams {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    /// Certificate authority that signed the server certificate, as a PEM
    /// file. When set, it is the only authority trusted. When absent, the
    /// certificate authorities of the operating system are trusted.
    pub ca_file: Option<PathBuf>,
}

/// The driver side of a connection: a future that must be polled for the
/// [`tokio_postgres::Client`] to make progress.
pub type Connection = tokio_postgres::Connection<
    Socket,
    <MakeRustlsConnect as tokio_postgres::tls::MakeTlsConnect<Socket>>::Stream,
>;

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("cannot read the certificate authority file {path}: {source}")]
    CaFile {
        path: PathBuf,
        #[source]
        source: rustls::pki_types::pem::Error,
    },
    #[error("the certificate authority file {path} contains no certificate")]
    EmptyCaFile { path: PathBuf },
    #[error("no trusted certificate authority was found on this system")]
    NoSystemRoots,
    #[error("TLS configuration error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("cannot start the network runtime: {0}")]
    Runtime(#[source] std::io::Error),
    #[error(transparent)]
    Postgres(#[from] tokio_postgres::Error),
}

/// Builds the TLS configuration of a pgXila client: TLS 1.3 only, with
/// verification of the certificate chain and of the host name.
pub fn tls_config(ca_file: Option<&Path>) -> Result<ClientConfig, ConnectError> {
    let roots = match ca_file {
        Some(path) => roots_from_file(path)?,
        None => system_roots()?,
    };
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(config)
}

fn roots_from_file(path: &Path) -> Result<RootCertStore, ConnectError> {
    let mut roots = RootCertStore::empty();
    let certificates = CertificateDer::pem_file_iter(path)
        .map_err(|source| ConnectError::CaFile { path: path.to_owned(), source })?;
    for certificate in certificates {
        let certificate =
            certificate.map_err(|source| ConnectError::CaFile { path: path.to_owned(), source })?;
        roots.add(certificate)?;
    }
    if roots.is_empty() {
        return Err(ConnectError::EmptyCaFile { path: path.to_owned() });
    }
    Ok(roots)
}

fn system_roots() -> Result<RootCertStore, ConnectError> {
    let mut roots = RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if roots.is_empty() {
        return Err(ConnectError::NoSystemRoots);
    }
    Ok(roots)
}

/// The PostgreSQL connection settings of a pgXila client.
///
/// `SslMode::Require` makes the driver give up if the server does not speak
/// TLS; the rustls configuration then verifies the certificate and the host
/// name, which together amount to libpq's `sslmode=verify-full`.
/// `ChannelBinding::Require` only succeeds with SCRAM-SHA-256-PLUS, so a
/// server that asks for anything weaker than SCRAM is refused as well.
pub fn postgres_config(params: &ConnectParams) -> tokio_postgres::Config {
    let mut config = tokio_postgres::Config::new();
    config
        .host(&params.host)
        .port(params.port)
        .dbname(&params.database)
        .user(&params.user)
        .password(&params.password)
        .application_name(APPLICATION_NAME)
        .ssl_mode(SslMode::Require)
        .channel_binding(ChannelBinding::Require);
    config
}

/// Connects to a pgXila server.
///
/// Returns the client and the connection that drives it. The caller must
/// poll the connection, usually by spawning it on the runtime.
pub async fn connect(
    params: &ConnectParams,
) -> Result<(tokio_postgres::Client, Connection), ConnectError> {
    let tls = MakeRustlsConnect::new(tls_config(params.ca_file.as_deref())?);
    Ok(postgres_config(params).connect(tls).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> ConnectParams {
        ConnectParams {
            host: "db.example".into(),
            port: 5419,
            database: "pgxila".into(),
            user: "ada".into(),
            password: "secret".into(),
            ca_file: None,
        }
    }

    #[test]
    fn encryption_and_channel_binding_are_always_required() {
        let config = postgres_config(&params());
        assert_eq!(config.get_ssl_mode(), SslMode::Require);
        assert_eq!(config.get_channel_binding(), ChannelBinding::Require);
    }

    #[test]
    fn a_missing_ca_file_is_reported_with_its_path() {
        let error = tls_config(Some(Path::new("/nonexistent/ca.crt"))).unwrap_err();
        assert!(matches!(error, ConnectError::CaFile { .. }), "{error}");
        assert!(error.to_string().contains("/nonexistent/ca.crt"));
    }

    #[test]
    fn a_ca_file_without_certificate_is_refused() {
        let path = std::env::temp_dir().join(format!("xila-empty-ca-{}.pem", std::process::id()));
        std::fs::write(&path, "not a certificate\n").unwrap();
        let error = tls_config(Some(&path)).unwrap_err();
        std::fs::remove_file(&path).unwrap();
        assert!(matches!(error, ConnectError::EmptyCaFile { .. }), "{error}");
    }
}
