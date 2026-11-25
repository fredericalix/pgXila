//! pgXila: a fantasy OS that lives inside PostgreSQL.
//!
//! This extension is the system layer. It must be loaded through
//! `shared_preload_libraries`, because two of its duties can only be set up
//! when the server starts:
//!
//! - the connection guard, which refuses unencrypted client connections;
//! - the background workers.

use pgrx::prelude::*;

mod tls_guard;
mod workers;

::pgrx::pg_module_magic!(name, version);

#[allow(non_snake_case)]
#[pg_guard]
pub extern "C-unwind" fn _PG_init() {
    // SAFETY: this global is set by the postmaster and only read here.
    let preloading = unsafe { pg_sys::process_shared_preload_libraries_in_progress };

    if !preloading {
        // The pgrx test framework loads the library on demand, in a server
        // that it talks to without TLS. Test builds therefore run without the
        // guard and without workers. Shipped builds have no such escape hatch.
        if cfg!(feature = "pg_test") {
            return;
        }
        ereport!(
            ERROR,
            PgSqlErrorCode::ERRCODE_OBJECT_NOT_IN_PREREQUISITE_STATE,
            "pgxila must be loaded through shared_preload_libraries",
            "Add pgxila to shared_preload_libraries in postgresql.conf and restart the server."
        );
    }

    tls_guard::install();
    workers::register();
}

#[pg_schema]
mod xila {
    use pgrx::prelude::*;

    /// Version of the pgXila extension.
    #[pg_extern(immutable, parallel_safe)]
    fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use crate::tls_guard::{check, Refusal, Transport};
    use pgrx::prelude::*;

    #[pg_test]
    fn version_is_the_crate_version() {
        let version = Spi::get_one::<String>("SELECT xila.version()").unwrap();
        assert_eq!(version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[pg_test]
    fn guard_accepts_the_local_socket() {
        assert_eq!(check(Transport::UnixSocket), Ok(()));
    }

    #[pg_test]
    fn guard_accepts_tls_1_3() {
        assert_eq!(
            check(Transport::Tcp {
                tls_version: Some("TLSv1.3")
            }),
            Ok(())
        );
    }

    #[pg_test]
    fn guard_refuses_plain_tcp() {
        assert_eq!(
            check(Transport::Tcp { tls_version: None }),
            Err(Refusal::NotEncrypted)
        );
    }

    #[pg_test]
    fn guard_refuses_older_tls() {
        for version in ["TLSv1.2", "TLSv1.1", "TLSv1", "SSLv3", ""] {
            assert_eq!(
                check(Transport::Tcp {
                    tls_version: Some(version)
                }),
                Err(Refusal::ProtocolTooOld),
                "{version} must be refused"
            );
        }
    }
}

/// This module is required by `cargo pgrx test` invocations.
/// It must be visible at the root of the extension crate.
#[cfg(test)]
pub mod pg_test {
    pub fn setup(_options: Vec<&str>) {}

    #[must_use]
    pub fn postgresql_conf_options() -> Vec<&'static str> {
        vec![]
    }
}
