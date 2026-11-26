//! The launcher: the worker that starts the others.
//!
//! When the server starts, the database of pgXila may not exist yet: the
//! extension is preloaded before anyone could create it. A worker that needs
//! that database cannot be started blindly. The launcher connects to no
//! database, looks at the shared catalogs until the database and the role of
//! the agent are there, and only then starts the workers. It starts again,
//! after a pause, a worker that died.

use std::ffi::{c_char, CStr};
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

/// True when a shared catalog has a row with this name. The catalog is read
/// from end to end, without its indexes: a process that has no database
/// cannot use them, which is also why the autovacuum launcher reads
/// `pg_database` this way.
///
/// # Safety
/// Must be called inside a transaction. `name_of` must give the name column
/// of a row of that catalog.
unsafe fn has_row_named(
    catalog: pg_sys::Oid,
    name: &CStr,
    name_of: unsafe fn(*const u8) -> *const c_char,
) -> bool {
    let relation = pg_sys::table_open(catalog, pg_sys::AccessShareLock as i32);
    let scan = pg_sys::table_beginscan_catalog(relation, 0, std::ptr::null_mut());
    let mut found = false;
    loop {
        let tuple = pg_sys::heap_getnext(scan, pg_sys::ScanDirection::ForwardScanDirection);
        if tuple.is_null() {
            break;
        }
        let header = (*tuple).t_data;
        let row = header.cast::<u8>().add(usize::from((*header).t_hoff));
        if CStr::from_ptr(name_of(row)) == name {
            found = true;
            break;
        }
    }
    pg_sys::heap_endscan(scan);
    pg_sys::table_close(relation, pg_sys::AccessShareLock as i32);
    found
}

unsafe fn database_name(row: *const u8) -> *const c_char {
    (*row.cast::<pg_sys::FormData_pg_database>()).datname.data.as_ptr()
}

unsafe fn role_name(row: *const u8) -> *const c_char {
    (*row.cast::<pg_sys::FormData_pg_authid>()).rolname.data.as_ptr()
}

/// True when the database of pgXila and the role of the agent exist.
fn ready() -> bool {
    let name = database();
    BackgroundWorker::transaction(move || {
        // SAFETY: inside a transaction, on two shared catalogs.
        unsafe {
            has_row_named(pg_sys::DatabaseRelationId, &name, database_name)
                && has_row_named(pg_sys::AuthIdRelationId, agent::ROLE, role_name)
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
