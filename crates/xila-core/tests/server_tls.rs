//! Integration tests of the transport security of a pgXila server.
//!
//! They need two running servers and are ignored by default. Run them with
//! `scripts/test-integration.sh`, which starts the servers, creates a
//! throwaway role and sets the `PGXILA_TEST_*` variables.
//!
//! - The main server uses the configuration of the image: `pg_hba.conf` only
//!   has `hostssl` lines.
//! - The "permissive" server is the same image with a `pg_hba.conf` that also
//!   accepts unencrypted connections. It shows that the extension refuses them
//!   on its own, whatever the configuration says.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use rustls::pki_types::pem::PemObject;
use rustls::pki_types::CertificateDer;
use tokio_postgres::error::SqlState;
use tokio_postgres::NoTls;
use tokio_postgres_rustls::MakeRustlsConnect;
use xila_core::connection::{connect, postgres_config, tls_config, ConnectError, ConnectParams};

const IGNORE_REASON: &str = "needs running pgXila servers: use scripts/test-integration.sh";

fn var(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set ({IGNORE_REASON})"))
}

fn params() -> ConnectParams {
    ConnectParams {
        host: var("PGXILA_TEST_HOST"),
        port: var("PGXILA_TEST_PORT").parse().expect("PGXILA_TEST_PORT"),
        database: var("PGXILA_TEST_DATABASE"),
        user: var("PGXILA_TEST_USER"),
        password: var("PGXILA_TEST_PASSWORD"),
        ca_file: Some(PathBuf::from(var("PGXILA_TEST_CA"))),
    }
}

fn permissive_params() -> ConnectParams {
    ConnectParams {
        port: var("PGXILA_TEST_PERMISSIVE_PORT")
            .parse()
            .expect("PGXILA_TEST_PERMISSIVE_PORT"),
        ca_file: Some(PathBuf::from(var("PGXILA_TEST_PERMISSIVE_CA"))),
        ..params()
    }
}

/// The same settings as a pgXila client, minus encryption.
fn plaintext_config(params: &ConnectParams) -> tokio_postgres::Config {
    let mut config = postgres_config(params);
    config
        .ssl_mode(tokio_postgres::config::SslMode::Disable)
        .channel_binding(tokio_postgres::config::ChannelBinding::Disable);
    config
}

/// Asserts that the server itself answered with a refusal. A server that is
/// simply not running would also make the connection fail, and must not make
/// these tests pass.
fn assert_refused_by_server(error: &tokio_postgres::Error) -> &tokio_postgres::error::DbError {
    let refusal = error
        .as_db_error()
        .unwrap_or_else(|| panic!("expected a refusal sent by the server, got: {error:?}"));
    assert_eq!(
        refusal.code(),
        &SqlState::INVALID_AUTHORIZATION_SPECIFICATION,
        "{refusal:?}"
    );
    refusal
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn a_client_connects_with_tls_1_3_and_scram() {
    let (client, connection) = connect(&params()).await.expect("connection");
    tokio::spawn(connection);

    let row = client
        .query_one(
            "SELECT ssl, version FROM pg_stat_ssl WHERE pid = pg_backend_pid()",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>("ssl"));
    assert_eq!(row.get::<_, &str>("version"), "TLSv1.3");
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn an_unencrypted_connection_is_refused() {
    let error = plaintext_config(&params())
        .connect(NoTls)
        .await
        .err()
        .expect("must be refused");
    assert_refused_by_server(&error);
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn the_extension_refuses_unencrypted_connections_even_if_pg_hba_allows_them() {
    let params = permissive_params();

    // The permissive server does accept this role over TLS...
    let (client, connection) = connect(&params).await.expect("TLS connection");
    tokio::spawn(connection);
    client.query_one("SELECT 1", &[]).await.unwrap();

    // ...and its pg_hba.conf lets the same role authenticate without TLS, but
    // the connection guard of the extension turns it away.
    let error = plaintext_config(&params)
        .connect(NoTls)
        .await
        .err()
        .expect("must be refused");
    let refusal = assert_refused_by_server(&error);
    assert_eq!(refusal.message(), "pgXila refuses unencrypted connections");
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn a_tls_1_2_connection_is_refused() {
    let params = params();
    let mut roots = rustls::RootCertStore::empty();
    for certificate in CertificateDer::pem_file_iter(params.ca_file.as_ref().unwrap()).unwrap() {
        roots.add(certificate.unwrap()).unwrap();
    }
    let tls_1_2_only = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();

    let error = postgres_config(&params)
        .connect(MakeRustlsConnect::new(tls_1_2_only))
        .await
        .err()
        .expect("TLS 1.2 must be refused");
    // The handshake fails: there is no session, so no server error message.
    assert!(error.as_db_error().is_none(), "{error:?}");
    let text = format!("{error:?}");
    assert!(
        text.contains("ProtocolVersion"),
        "expected a protocol version alert, got: {text}"
    );
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn the_client_refuses_a_certificate_from_an_unknown_authority() {
    // Trust the operating system's authorities instead of the server's own:
    // the development CA is not one of them.
    let params = ConnectParams {
        ca_file: None,
        ..params()
    };
    match connect(&params).await {
        Err(ConnectError::Postgres(error)) => {
            let text = format!("{error:?}");
            assert!(
                text.contains("UnknownIssuer"),
                "expected an unknown issuer, got: {text}"
            );
        }
        Err(other) => panic!("unexpected error: {other:?}"),
        Ok(_) => panic!("the server certificate must not be trusted"),
    }
}

#[tokio::test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
async fn the_client_refuses_a_certificate_for_another_host_name() {
    // Reach the same server, but under a name its certificate does not cover.
    let params = ConnectParams {
        host: "not-the-server.example".into(),
        ..params()
    };
    let address: IpAddr = "127.0.0.1".parse().unwrap();
    let mut config = postgres_config(&params);
    config.hostaddr(address);

    let tls = MakeRustlsConnect::new(tls_config(params.ca_file.as_deref()).unwrap());
    let error = config
        .connect(tls)
        .await
        .err()
        .expect("the host name must be verified");
    let text = format!("{error:?}");
    assert!(
        text.contains("NotValidForName"),
        "expected a host name mismatch, got: {text}"
    );
}
