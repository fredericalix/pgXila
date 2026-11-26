//! How a script reaches the database.
//!
//! A script hands over one statement and its values. The statement is checked
//! to be one that reads or writes rows, then prepared with parameters whose
//! types PostgreSQL works out itself, as it does for a client: `WHERE id = $1`
//! needs no cast. It runs in a subtransaction, so that its failure is a
//! message the script may catch, as an exception block of PL/pgSQL does.

use std::ffi::{c_int, c_long, c_void, CStr, CString};
use std::panic::AssertUnwindSafe;
use std::ptr;

use pgrx::pg_sys::panic::CaughtError;
use pgrx::prelude::*;
use pgrx::{FromDatum, JsonB, PgList};
use serde_json::Value as Json;

/// The most rows a query gives to a script.
pub const MOST_ROWS: usize = 1000;

/// A value handed to a statement, in its text form; `None` is NULL.
pub type Arg = Option<String>;

/// What a script may ask of a statement.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Read,
    Write,
}

/// Runs a `SELECT` and gives its rows, each a JSON object by column name.
pub fn query(sql: &str, args: &[Arg]) -> Result<Vec<Json>, String> {
    statement(sql, args, Kind::Read).map(|done| done.rows)
}

/// Runs an `INSERT`, `UPDATE`, `DELETE` or `MERGE` and gives the number of
/// rows it touched.
pub fn execute(sql: &str, args: &[Arg]) -> Result<u64, String> {
    statement(sql, args, Kind::Write).map(|done| done.count)
}

struct Done {
    rows: Vec<Json>,
    count: u64,
}

fn statement(sql: &str, args: &[Arg], wanted: Kind) -> Result<Done, String> {
    let plain = sql.trim().trim_end_matches(';').trim_end();
    let source = c_string(plain)?;
    // A query is read through one JSON column, whatever its own columns are.
    let text = match wanted {
        Kind::Read => c_string(&format!(
            "SELECT to_jsonb(q) FROM (\n{plain}\n) AS q LIMIT {}",
            MOST_ROWS + 1
        ))?,
        Kind::Write => source.clone(),
    };
    let values = args
        .iter()
        .map(|arg| arg.as_deref().map(c_string).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    in_subtransaction(|| {
        // SAFETY: the strings outlive the calls, and SPI is connected for
        // the time the plan and its rows are used.
        let done = unsafe {
            if kind_of(&source) != Some(wanted) {
                Err(match wanted {
                    Kind::Read => "“query” takes one SELECT statement.".to_owned(),
                    Kind::Write => {
                        "“execute” takes one INSERT, UPDATE, DELETE or MERGE statement.".to_owned()
                    }
                })
            } else {
                Spi::connect_mut(|_| run(&text, &values, wanted))
            }
        };
        let keep = done.is_ok();
        (done, keep)
    })?
}

fn c_string(text: &str) -> Result<CString, String> {
    CString::new(text).map_err(|_| "A statement cannot hold a zero byte.".to_owned())
}

/// What kind of statement this is, or `None` when a script may not run it:
/// several statements, or anything that is not about rows.
unsafe fn kind_of(source: &CStr) -> Option<Kind> {
    let parsed = pg_sys::pg_parse_query(source.as_ptr());
    let statements = PgList::<pg_sys::RawStmt>::from_pg(parsed);
    if statements.len() != 1 {
        return None;
    }
    let node = (*statements.get_ptr(0)?).stmt;
    match (*node).type_ {
        pg_sys::NodeTag::T_SelectStmt => {
            // SELECT … INTO creates a table.
            let select = node.cast::<pg_sys::SelectStmt>();
            (*select).intoClause.is_null().then_some(Kind::Read)
        }
        pg_sys::NodeTag::T_InsertStmt
        | pg_sys::NodeTag::T_UpdateStmt
        | pg_sys::NodeTag::T_DeleteStmt
        | pg_sys::NodeTag::T_MergeStmt => Some(Kind::Write),
        _ => None,
    }
}

/// The types PostgreSQL gives to the parameters of a statement.
#[repr(C)]
struct Parameters {
    types: *mut pg_sys::Oid,
    count: c_int,
}

/// Lets the parser work out the type of each parameter from how it is used.
#[pg_guard]
unsafe extern "C-unwind" fn variable_parameters(
    pstate: *mut pg_sys::ParseState,
    parameters: *mut c_void,
) {
    let parameters = parameters.cast::<Parameters>();
    pg_sys::setup_parse_variable_parameters(
        pstate,
        &raw mut (*parameters).types,
        &raw mut (*parameters).count,
    );
}

unsafe fn run(text: &CStr, args: &[Option<CString>], wanted: Kind) -> Result<Done, String> {
    let mut parameters = Parameters { types: ptr::null_mut(), count: 0 };
    let plan = pg_sys::SPI_prepare_params(
        text.as_ptr(),
        Some(variable_parameters),
        (&raw mut parameters).cast(),
        0,
    );
    if plan.is_null() {
        return Err("The statement could not be prepared.".to_owned());
    }

    let expected = usize::try_from(parameters.count).unwrap_or(0);
    if args.len() != expected {
        return Err(format!(
            "The statement has {expected} parameter{}, and {} value{} given.",
            if expected == 1 { "" } else { "s" },
            args.len(),
            if args.len() == 1 { " was" } else { "s were" },
        ));
    }

    // The values go with the types the parser found for them.
    let list = pg_sys::makeParamList(parameters.count);
    for (index, arg) in args.iter().enumerate() {
        let kind = *parameters.types.add(index);
        if kind == pg_sys::InvalidOid || kind == pg_sys::UNKNOWNOID {
            let n = index + 1;
            return Err(format!(
                "The type of ${n} cannot be told from the statement. Say it, as in ${n}::text."
            ));
        }
        let (value, isnull) = match arg {
            None => (pg_sys::Datum::from(0usize), true),
            Some(text) => {
                let mut input = pg_sys::InvalidOid;
                let mut io_parameter = pg_sys::InvalidOid;
                pg_sys::getTypeInputInfo(kind, &raw mut input, &raw mut io_parameter);
                let value =
                    pg_sys::OidInputFunctionCall(input, text.as_ptr().cast_mut(), io_parameter, -1);
                (value, false)
            }
        };
        (*list).params.as_mut_ptr().add(index).write(pg_sys::ParamExternData {
            value,
            isnull,
            pflags: pg_sys::PARAM_FLAG_CONST as u16,
            ptype: kind,
        });
    }

    let limit = match wanted {
        Kind::Read => MOST_ROWS as c_long + 1,
        Kind::Write => 0,
    };
    let code = pg_sys::SPI_execute_plan_with_paramlist(plan, list, false, limit);
    if code < 0 {
        return Err(format!("The statement could not be run (SPI code {code})."));
    }

    let count = pg_sys::SPI_processed;
    let mut rows = Vec::new();
    if wanted == Kind::Read {
        let found = usize::try_from(count).unwrap_or(usize::MAX);
        if found > MOST_ROWS {
            return Err(format!("A query gives {MOST_ROWS} rows at most. This one has more."));
        }
        let table = pg_sys::SPI_tuptable;
        rows.reserve(found);
        for index in 0..found {
            let tuple = *(*table).vals.add(index);
            let mut is_null = false;
            let datum = pg_sys::SPI_getbinval(tuple, (*table).tupdesc, 1, &raw mut is_null);
            rows.push(JsonB::from_datum(datum, is_null).map_or(Json::Null, |row| row.0));
        }
    }
    Ok(Done { rows, count })
}

/// Runs `work` in a subtransaction, kept when `work` says so. A PostgreSQL
/// error rolls it back and comes out as its message. A cancelled query is
/// never turned into a message: it goes on as the error it is.
pub fn in_subtransaction<T>(work: impl FnOnce() -> (T, bool)) -> Result<T, String> {
    // SAFETY: this is the sequence PL/pgSQL and PL/Python use around a block
    // that may fail: remember the memory context and the resource owner, open
    // the subtransaction, and put both back once it is closed.
    unsafe {
        let context = pg_sys::CurrentMemoryContext;
        let owner = pg_sys::CurrentResourceOwner;
        pg_sys::BeginInternalSubTransaction(ptr::null());
        pg_sys::MemoryContextSwitchTo(context);

        let restore = move || {
            pg_sys::MemoryContextSwitchTo(context);
            pg_sys::CurrentResourceOwner = owner;
        };

        PgTryBuilder::new(AssertUnwindSafe(|| {
            let (value, keep) = work();
            if keep {
                pg_sys::ReleaseCurrentSubTransaction();
            } else {
                pg_sys::RollbackAndReleaseCurrentSubTransaction();
            }
            restore();
            Ok(value)
        }))
        .catch_others(move |cause: CaughtError| {
            pg_sys::MemoryContextSwitchTo(context);
            pg_sys::RollbackAndReleaseCurrentSubTransaction();
            restore();
            let (code, message) = match &cause {
                CaughtError::PostgresError(report)
                | CaughtError::ErrorReport(report)
                | CaughtError::RustPanic { ereport: report, .. } => {
                    (report.sql_error_code(), report.message().to_owned())
                }
            };
            if code == PgSqlErrorCode::ERRCODE_QUERY_CANCELED {
                cause.rethrow();
            }
            Err(message)
        })
        .execute()
    }
}
