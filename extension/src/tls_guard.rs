//! Connection guard: every client connection must be encrypted with TLS 1.3.
//!
//! `pg_hba.conf` already says so in the pgXila server image, but that file can
//! be edited. The guard enforces the rule from inside the server, whatever the
//! configuration says, and it has no setting to turn it off.
//!
//! The only connections accepted without TLS are those that come through the
//! Unix socket, which never leaves the machine. They are used to administer the
//! server from inside its container.
//!
//! The guard runs in PostgreSQL's client authentication hook, which is called
//! once the authentication exchange is over and before the session starts. A
//! refused client therefore never gets a session, and never sends a query.

use pgrx::prelude::*;
use std::ffi::{c_char, c_int, CStr};

/// The only TLS protocol version pgXila accepts.
const REQUIRED_TLS_VERSION: &str = "TLSv1.3";

/// How a client reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport<'a> {
    /// A local connection through the Unix socket.
    UnixSocket,
    /// A network connection, with the TLS protocol version if it is encrypted.
    Tcp { tls_version: Option<&'a str> },
}

/// Why a connection is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NotEncrypted,
    ProtocolTooOld,
}

/// Decides whether a connection may proceed.
pub fn check(transport: Transport) -> Result<(), Refusal> {
    match transport {
        Transport::UnixSocket => Ok(()),
        Transport::Tcp { tls_version: None } => Err(Refusal::NotEncrypted),
        Transport::Tcp {
            tls_version: Some(REQUIRED_TLS_VERSION),
        } => Ok(()),
        Transport::Tcp {
            tls_version: Some(_),
        } => Err(Refusal::ProtocolTooOld),
    }
}

/// PostgreSQL's client authentication hook: called with the connection and
/// the result of the authentication exchange.
type AuthenticationHook = Option<unsafe extern "C-unwind" fn(*mut pg_sys::Port, c_int)>;

// Implemented in tls_guard.c, against the PostgreSQL headers.
extern "C" {
    fn pgxila_port_is_unix_socket(port: *const pg_sys::Port) -> bool;
    fn pgxila_port_tls_version(port: *mut pg_sys::Port) -> *const c_char;
    fn pgxila_swap_client_authentication_hook(hook: AuthenticationHook) -> AuthenticationHook;
}

/// The hook that was installed before ours, if any. Written once, while the
/// postmaster loads the library, before any other process or thread exists.
static mut PREVIOUS_HOOK: AuthenticationHook = None;

/// Installs the guard. Must be called from `_PG_init` during preloading.
pub fn install() {
    // SAFETY: `_PG_init` runs single-threaded in the postmaster. Backends
    // inherit both hooks when they are forked and only read them.
    unsafe {
        PREVIOUS_HOOK = pgxila_swap_client_authentication_hook(Some(guard_hook));
    }
}

#[pg_guard]
unsafe extern "C-unwind" fn guard_hook(port: *mut pg_sys::Port, status: c_int) {
    // SAFETY: PostgreSQL passes a valid Port for the connecting client, and
    // the version string it returns lives as long as the connection.
    let transport = if pgxila_port_is_unix_socket(port) {
        Transport::UnixSocket
    } else {
        let version = pgxila_port_tls_version(port);
        let tls_version = if version.is_null() {
            None
        } else {
            Some(CStr::from_ptr(version).to_str().unwrap_or(""))
        };
        Transport::Tcp { tls_version }
    };

    match check(transport) {
        Ok(()) => {}
        Err(Refusal::NotEncrypted) => {
            ereport!(
                FATAL,
                PgSqlErrorCode::ERRCODE_INVALID_AUTHORIZATION_SPECIFICATION,
                "pgXila refuses unencrypted connections",
                "Connect with TLS 1.3 and verify the server certificate (sslmode=verify-full)."
            );
        }
        Err(Refusal::ProtocolTooOld) => {
            ereport!(
                FATAL,
                PgSqlErrorCode::ERRCODE_INVALID_AUTHORIZATION_SPECIFICATION,
                "pgXila only accepts TLS 1.3 connections",
                "This connection was encrypted with an older protocol version."
            );
        }
    }

    // SAFETY: see `PREVIOUS_HOOK`; the value is copied, never borrowed.
    if let Some(previous) = PREVIOUS_HOOK {
        previous(port, status);
    }
}
