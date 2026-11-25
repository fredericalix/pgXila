//! Background workers of pgXila.
//!
//! Long or blocking work never runs inside a SQL backend. It belongs to a
//! background worker, a separate process started and supervised by PostgreSQL.
//!
//! There is no worker yet. Two are planned, each as its own submodule that
//! registers itself from [`register`]:
//!
//! - `agent`: talks to the language model (phase 3);
//! - `link`: talks to the other pgXila instances (phase 4).
//!
//! Workers are independent processes: adding one does not touch the others.

/// Registers every background worker. Called from `_PG_init` while the
/// library is preloaded, the only moment PostgreSQL accepts static workers.
pub fn register() {}
