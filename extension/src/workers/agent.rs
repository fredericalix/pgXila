//! The agent worker: carries the conversations to the language model.
//!
//! Everything the agent decides is decided in SQL (`extension/sql/agent.sql`):
//! what to send, what an answer means, which tool to run and for whom. This
//! worker only carries. It asks the database which conversations wait, hands
//! each request to a thread that makes the HTTP call, and gives the answer
//! back to the database.
//!
//! The call is made on a thread so that the worker itself never blocks: it
//! keeps answering PostgreSQL, and stops at once when asked to, whatever the
//! model is doing. The threads touch nothing of PostgreSQL and receive no
//! signal.
//!
//! The worker connects as the role `xila_agent`, which cannot log in and may
//! only read what was opened to it.

use std::ffi::CStr;
use std::panic::AssertUnwindSafe;
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use pgrx::bgworkers::{
    BackgroundWorker, BackgroundWorkerBuilder, DynamicBackgroundWorker,
    DynamicBackgroundWorkerLoadError, SignalWakeFlags,
};
use pgrx::pg_sys::panic::CaughtError;
use pgrx::prelude::*;
use pgrx::{JsonB, PgXactCallbackEvent};

use super::{database, http};

/// The role the worker connects as.
pub const ROLE: &CStr = c"xila_agent";
/// The name PostgreSQL knows the worker by.
const TYPE: &CStr = c"pgxila agent";

/// How many conversations are answered at the same time.
const MOST_CALLS: usize = 4;
/// How long the worker sleeps when nobody wakes it. Asking wakes it at once:
/// this is only a safety net.
const IDLE: Duration = Duration::from_secs(5);
/// How often it looks whether a call has come back.
const BUSY: Duration = Duration::from_millis(50);
/// The most time a transaction of the worker may take: a tool that reads a
/// huge table must not keep every conversation waiting.
const MOST_MS: i32 = 15_000;
/// So that only one agent worker works in a database.
const LOCK: i64 = 0x7067_5869_6c61; // "pgXila"

/// Starts the agent worker. Called by the launcher.
pub fn start() -> Result<DynamicBackgroundWorker, DynamicBackgroundWorkerLoadError> {
    let name = TYPE.to_str().expect("an ASCII name");
    BackgroundWorkerBuilder::new(name)
        .set_type(name)
        .set_library("pgxila")
        .set_function("pgxila_agent_main")
        .enable_spi_access()
        // The launcher starts it again, after a pause.
        .set_restart_time(None)
        // SAFETY: read in the process that registers the worker.
        .set_notify_pid(unsafe { pg_sys::MyProcPid })
        .load_dynamic()
}

/// Wakes the agent worker once the current transaction is committed: before
/// that, the worker would not see what the transaction wrote.
pub fn wake_at_commit() {
    pgrx::register_xact_callback(PgXactCallbackEvent::Commit, wake);
}

/// Sets the latch of the agent worker, if it runs. Called after a commit, so
/// it must never raise an error: it only reads the process table.
fn wake() {
    // SAFETY: the array of processes lives in shared memory for as long as
    // the server runs; a slot is read without a lock, which at worst wakes a
    // process for nothing. Setting a latch is safe from any process.
    unsafe {
        let all = pg_sys::ProcGlobal;
        if all.is_null() {
            return;
        }
        for index in 0..(*all).allProcCount as usize {
            let process = (*all).allProcs.add(index);
            let pid = (*process).pid;
            if pid == 0 || pid == pg_sys::MyProcPid {
                continue;
            }
            let kind = pg_sys::GetBackgroundWorkerTypeByPid(pid);
            if !kind.is_null() && CStr::from_ptr(kind) == TYPE {
                pg_sys::SetLatch(&raw mut (*process).procLatch);
            }
        }
    }
}

/// A call to the model that has not come back yet.
struct Call {
    conversation: String,
    /// The line of the conversation the request was made at.
    upto: String,
    started: Instant,
    answer: Receiver<http::Outcome>,
}

/// Runs `work` on a thread that receives no signal: the signal handlers of
/// PostgreSQL must only ever run on the main thread.
fn spawn_deaf(work: impl FnOnce() + Send + 'static) {
    // SAFETY: plain libc calls on local signal sets. The mask is set before
    // the thread exists, so the thread never has a moment with signals open.
    unsafe {
        let mut all: libc::sigset_t = std::mem::zeroed();
        let mut before: libc::sigset_t = std::mem::zeroed();
        libc::sigfillset(&raw mut all);
        libc::pthread_sigmask(libc::SIG_BLOCK, &raw const all, &raw mut before);
        std::thread::spawn(work);
        libc::pthread_sigmask(libc::SIG_SETMASK, &raw const before, std::ptr::null_mut());
    }
}

/// Runs `work` in a transaction of its own. A PostgreSQL error rolls it back
/// and comes out as its SQLSTATE; the worker goes on.
fn in_transaction<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    // SAFETY: the sequence of a background worker around a unit of work:
    // start a transaction, give it a snapshot, commit; on error, abort.
    unsafe {
        pg_sys::SetCurrentStatementStartTimestamp();
        pg_sys::StartTransactionCommand();
        pg_sys::PushActiveSnapshot(pg_sys::GetTransactionSnapshot());
        pg_sys::enable_timeout_after(pg_sys::TimeoutId::STATEMENT_TIMEOUT, MOST_MS);

        PgTryBuilder::new(AssertUnwindSafe(|| {
            let value = work();
            pg_sys::disable_timeout(pg_sys::TimeoutId::STATEMENT_TIMEOUT, false);
            pg_sys::PopActiveSnapshot();
            pg_sys::CommitTransactionCommand();
            Ok(value)
        }))
        .catch_others(|cause: CaughtError| {
            pg_sys::disable_timeout(pg_sys::TimeoutId::STATEMENT_TIMEOUT, false);
            // Only the code: the message of an error may quote what a card says.
            let code = match &cause {
                CaughtError::PostgresError(report)
                | CaughtError::ErrorReport(report)
                | CaughtError::RustPanic { ereport: report, .. } => {
                    format!("{:?}", report.sql_error_code())
                }
            };
            pg_sys::AbortCurrentTransaction();
            pg_sys::QueryCancelPending = 0;
            Err(code)
        })
        .execute()
    }
}

/// True when the extension is created in this database.
fn installed() -> bool {
    in_transaction(|| {
        Spi::get_one::<bool>("SELECT to_regprocedure('xila.agent_work()') IS NOT NULL")
            .ok()
            .flatten()
            .unwrap_or(false)
    })
    .unwrap_or(false)
}

/// The conversations that wait, each with its latest line.
fn waiting() -> Vec<(String, String)> {
    in_transaction(|| {
        Spi::connect(|client| {
            client
                .select(
                    "SELECT conversation::text, upto::text FROM xila.agent_work()",
                    None,
                    &[],
                )
                .map(|rows| {
                    rows.filter_map(|row| {
                        Some((row.get::<String>(1).ok()??, row.get::<String>(2).ok()??))
                    })
                    .collect()
                })
                .unwrap_or_default()
        })
    })
    .unwrap_or_default()
}

/// What to send for a conversation: the address, the name of the variable of
/// the token, the line it is asked at, and the body. `None` when there is
/// nothing to send; the conversation then says why.
fn request(conversation: &str) -> Option<(String, String, String, String)> {
    let asked = in_transaction(|| {
        Spi::get_one_with_args::<JsonB>(
            "SELECT xila.agent_request($1::uuid)",
            &[conversation.into()],
        )
        .ok()
        .flatten()
    });
    match asked {
        Ok(Some(JsonB(asked))) => Some((
            asked["url"].as_str()?.to_owned(),
            asked["token_variable"].as_str().unwrap_or_default().to_owned(),
            asked["upto"].as_str()?.to_owned(),
            asked["body"].to_string(),
        )),
        Ok(None) => None,
        Err(code) => {
            give_up(conversation, &code);
            None
        }
    }
}

/// Gives the answer of the model to the database, which writes it and runs
/// the tools it asks for.
fn answer(call: &Call, outcome: &http::Outcome) {
    let took = i32::try_from(call.started.elapsed().as_millis()).unwrap_or(i32::MAX);
    let stored = in_transaction(|| {
        Spi::run_with_args(
            "SELECT xila.agent_answer($1::uuid, $2::uuid, $3, $4, $5)",
            &[
                call.conversation.as_str().into(),
                call.upto.as_str().into(),
                outcome.status.into(),
                outcome.body.as_str().into(),
                took.into(),
            ],
        )
        .is_ok()
    });
    if let Err(code) = stored {
        give_up(&call.conversation, &code);
    }
}

/// Ends a question the worker could not deal with, so that it is not tried
/// again and again.
fn give_up(conversation: &str, code: &str) {
    let detail = serde_json::json!({ "code": code }).to_string();
    let said = in_transaction(|| {
        Spi::run_with_args(
            "SELECT xila.agent_fail($1::uuid, \
             'The agent ran into a problem and stopped. Ask again.', $2::jsonb)",
            &[conversation.into(), detail.as_str().into()],
        )
        .is_ok()
    });
    if said.is_err() {
        log!("pgxila agent: a conversation could not be ended ({code})");
    }
}

#[pg_guard]
#[no_mangle]
pub extern "C-unwind" fn pgxila_agent_main(_arg: pg_sys::Datum) {
    BackgroundWorker::attach_signal_handlers(SignalWakeFlags::SIGHUP | SignalWakeFlags::SIGTERM);
    let name = database();
    // SAFETY: called once, at the start of a background worker. The role
    // cannot log in: a worker may connect as it all the same.
    unsafe {
        pg_sys::BackgroundWorkerInitializeConnection(
            name.as_ptr(),
            ROLE.as_ptr(),
            pg_sys::BGWORKER_BYPASS_ROLELOGINCHECK,
        );
    }

    // One agent worker per database, whatever started it.
    let alone = in_transaction(|| {
        Spi::get_one_with_args::<bool>("SELECT pg_try_advisory_lock($1)", &[LOCK.into()])
            .ok()
            .flatten()
            .unwrap_or(false)
    })
    .unwrap_or(false);
    if !alone {
        return;
    }

    let mut calls: Vec<Call> = Vec::new();
    let mut ready = false;
    let mut looked = Instant::now() - IDLE;
    loop {
        // SAFETY: the latch of this process, waited on from its main thread.
        let woken = unsafe {
            let pause = if calls.is_empty() { IDLE } else { BUSY };
            let events = pg_sys::WaitLatch(
                pg_sys::MyLatch,
                (pg_sys::WL_LATCH_SET | pg_sys::WL_TIMEOUT | pg_sys::WL_POSTMASTER_DEATH) as i32,
                pause.as_millis() as _,
                pg_sys::PG_WAIT_EXTENSION,
            );
            pg_sys::ResetLatch(pg_sys::MyLatch);
            if events & pg_sys::WL_POSTMASTER_DEATH as i32 != 0 {
                break;
            }
            events & pg_sys::WL_LATCH_SET as i32 != 0
        };
        if BackgroundWorker::sigterm_received() {
            break;
        }

        // The calls that came back.
        let mut back = Vec::new();
        calls.retain(|call| match call.answer.try_recv() {
            Ok(outcome) => {
                back.push((call.conversation.clone(), call.upto.clone(), call.started, outcome));
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => {
                let lost = http::Outcome { status: -1, body: "unreachable".to_owned() };
                back.push((call.conversation.clone(), call.upto.clone(), call.started, lost));
                false
            }
        });
        let came_back = !back.is_empty();
        for (conversation, upto, started, outcome) in back {
            let (_, nobody) = channel();
            answer(&Call { conversation, upto, started, answer: nobody }, &outcome);
        }

        // Look for work when woken, when an answer may have left a
        // conversation waiting for the next call, and now and then.
        if !(woken || came_back || looked.elapsed() >= IDLE) {
            continue;
        }
        looked = Instant::now();
        if !ready {
            ready = installed();
            if !ready {
                continue;
            }
        }
        for (conversation, upto) in waiting() {
            if calls.len() >= MOST_CALLS {
                break;
            }
            // Already asked at this line: the call is on its way.
            if calls.iter().any(|call| call.conversation == conversation && call.upto == upto) {
                continue;
            }
            let Some((url, token_variable, upto, body)) = request(&conversation) else {
                continue;
            };
            let (send, answer) = channel();
            spawn_deaf(move || {
                let _ = send.send(http::post(&url, &token_variable, &body));
            });
            calls.push(Call { conversation, upto, started: Instant::now(), answer });
        }
    }
}
