//! The launcher: the worker that starts the others.
//!
//! When the server starts, the database of pgXila may not exist yet: the
//! extension is preloaded before anyone could create it. A worker that needs
//! that database cannot be started blindly. The launcher connects to no
//! database, looks at the shared catalogs until the database and the role of
//! the agent are there, and only then starts the workers. It starts again,
//! after a pause, a worker that died.

use std::time::{Duration, Instant};

use pgrx::bgworkers::{
    BackgroundWorker, BackgroundWorkerBuilder, BackgroundWorkerStatus, BgWorkerStartTime,
    DynamicBackgroundWorker, SignalWakeFlags,
};
use pgrx::prelude::*;

use super::{agent, database};

/// How long a worker that died is left alone before it is started again.
const PAUSE: Duration = Duration::from_secs(5);

pub fn register() {
    BackgroundWorkerBuilder::new("pgxila launcher")
        .set_type("pgxila launcher")
        .set_library("pgxila")
        .set_function("pgxila_launcher_main")
        .enable_spi_access()
        .set_start_time(BgWorkerStartTime::RecoveryFinished)
        .set_restart_time(Some(PAUSE))
        .load();
}

/// True when the database of pgXila and the role of the agent exist.
fn ready() -> bool {
    let name = database();
    BackgroundWorker::transaction(move || {
        // SAFETY: catalog lookups, inside a transaction, that report a
        // missing object as an invalid identifier.
        unsafe {
            pg_sys::get_database_oid(name.as_ptr(), true) != pg_sys::InvalidOid
                && pg_sys::get_role_oid(agent::ROLE.as_ptr(), true) != pg_sys::InvalidOid
        }
    })
}

#[pg_guard]
#[no_mangle]
pub extern "C-unwind" fn pgxila_launcher_main(_arg: pg_sys::Datum) {
    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGHUP | SignalWakeFlags::SIGTERM);
    // No database: the shared catalogs are all the launcher reads.
    BackgroundWorker::connect_worker_to_spi(None, None);

    let mut started: Option<(DynamicBackgroundWorker, Instant)> = None;
    // The latch is set when a worker starts or stops; otherwise the launcher
    // looks again every few seconds.
    while BackgroundWorker::wait_latch(Some(Duration::from_secs(2))) {
        if let Some((worker, since)) = &started {
            match worker.pid() {
                Ok(_) | Err(BackgroundWorkerStatus::NotYetStarted) => continue,
                Err(_) if since.elapsed() < PAUSE => continue,
                Err(_) => started = None,
            }
        }
        if ready() {
            match agent::start() {
                Ok(worker) => started = Some((worker, Instant::now())),
                Err(_) => {
                    log!("pgxila: the agent worker could not be started (no free worker slot)");
                }
            }
        }
    }
}
