//! Background workers of pgXila.
//!
//! Long or blocking work never runs inside a SQL backend. It belongs to a
//! background worker, a separate process started and supervised by PostgreSQL.
//!
//! One worker is registered when the server starts: the [`launcher`]. It
//! starts the others once the database they work in exists, and starts them
//! again if they die. Each worker is a submodule with a `start` function for
//! the launcher to call:
//!
//! - [`agent`]: carries the conversations to the language model (phase 3);
//! - `link`: will talk to the other pgXila instances (phase 4).
//!
//! Workers are independent processes: adding one does not touch the others.

use std::ffi::CString;

use pgrx::{GucContext, GucFlags, GucRegistry, GucSetting};

pub mod agent;
#[cfg(any(test, feature = "pg_test"))]
mod checks;
mod http;
mod launcher;

/// The database the workers work in: the one where the extension is created.
static DATABASE: GucSetting<Option<CString>> =
    GucSetting::<Option<CString>>::new(Some(c"pgxila"));

/// The name of that database.
fn database() -> CString {
    DATABASE.get().unwrap_or_else(|| c"pgxila".to_owned())
}

/// Registers the launcher. Called from `_PG_init` while the library is
/// preloaded, the only moment PostgreSQL accepts static workers.
pub fn register() {
    GucRegistry::define_string_guc(
        c"pgxila.database",
        c"The database the pgXila workers work in.",
        c"The database in which the pgxila extension is created.",
        &DATABASE,
        GucContext::Postmaster,
        GucFlags::default(),
    );
    launcher::register();
}
