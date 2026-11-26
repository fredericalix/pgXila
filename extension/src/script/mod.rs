//! Scripts: Lua 5.4, inside the extension.
//!
//! A script is the source of one object of a screen, a button or the screen
//! itself, with one function per event. Each event runs in a new Lua state
//! that can compute and reach the database through SQL, and nothing else. It
//! runs with the rights of the current role, inside a subtransaction, and is
//! stopped when it runs for too long or holds too much memory: a script can
//! never block a backend.
//!
//! What a script sees is written in Lua, in `prelude.lua`, on top of the few
//! functions defined here.

mod sql;

#[cfg(any(test, feature = "pg_test"))]
mod checks;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use mlua::chunk::ChunkMode;
use mlua::serde::ser::Options as SerializeOptions;
use mlua::{Function, HookTriggers, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value, VmState};
use serde_json::{json, Value as Json};

/// The most Lua instructions one event may run.
const INSTRUCTIONS: i64 = 50_000_000;
/// The longest one event may last, with the time its statements take.
const SECONDS: u64 = 5;
/// The most memory one event may hold.
const MEMORY: usize = 16 * 1024 * 1024;
/// How often a running script looks at its limits and at pending interrupts,
/// in instructions.
const TICK: u32 = 1000;
/// The most lines `print` keeps for the test report.
const PRINTED: usize = 200;

const PRELUDE: &str = include_str!("prelude.lua");

const TOO_LONG: &str = "The script ran for too long and was stopped.";
const TOO_BIG: &str = "The script used too much memory and was stopped.";
const NOT_HERE: &str =
    "The database and the screen can only be used inside a function such as on_click.";

/// Why a script was stopped for good, in a way it cannot catch.
enum Stop {
    TooLong,
    /// It asked the user a question; nothing it did is kept until the answer.
    Asked {
        text: String,
        label: String,
    },
}

/// What the Lua side and the extension share during one event.
struct Host {
    /// False while the script is only being read: its top level may define
    /// things, not act.
    running: Cell<bool>,
    stop: RefCell<Option<Stop>>,
    printed: RefCell<Vec<String>>,
    saved: Cell<u64>,
    deleted: Cell<u64>,
    rows: Cell<u64>,
    left: Cell<i64>,
    deadline: Instant,
}

impl Host {
    fn new() -> Rc<Self> {
        Rc::new(Host {
            running: Cell::new(false),
            stop: RefCell::new(None),
            printed: RefCell::new(Vec::new()),
            saved: Cell::new(0),
            deleted: Cell::new(0),
            rows: Cell::new(0),
            left: Cell::new(INSTRUCTIONS),
            deadline: Instant::now() + Duration::from_secs(SECONDS),
        })
    }

    fn stopped(&self) -> bool {
        self.stop.borrow().is_some()
    }

    /// Stops the script when its time is spent.
    fn watch_the_clock(&self) -> mlua::Result<()> {
        if self.left.get() < 0 || Instant::now() > self.deadline {
            self.stop.borrow_mut().get_or_insert(Stop::TooLong);
        }
        if self.stopped() {
            return Err(mlua::Error::runtime("stopped"));
        }
        Ok(())
    }
}

/// A script that failed: where, and why.
struct Problem {
    line: Option<u32>,
    message: String,
}

impl Problem {
    fn new(message: impl Into<String>) -> Self {
        Problem { line: None, message: message.into() }
    }

    fn json(&self) -> Json {
        json!({ "line": self.line, "message": self.message })
    }
}

impl From<mlua::Error> for Problem {
    fn from(error: mlua::Error) -> Self {
        let text = match &error {
            mlua::Error::MemoryError(_) => return Problem::new(TOO_BIG),
            mlua::Error::CallbackError { cause, .. } => return Problem::from((**cause).clone()),
            mlua::Error::SyntaxError { message, .. } | mlua::Error::RuntimeError(message) => {
                message.clone()
            }
            other => other.to_string(),
        };
        let text = text.split("\nstack traceback:").next().unwrap_or_default().trim();
        if text.contains("not enough memory") {
            return Problem::new(TOO_BIG);
        }
        // Lua says where: "script:4: attempt to …".
        if let Some((line, message)) = text.strip_prefix("script:").and_then(|rest| {
            let (line, message) = rest.split_once(": ")?;
            Some((line.parse().ok()?, message))
        }) {
            return Problem { line: Some(line), message: message.to_owned() };
        }
        Problem::new(text)
    }
}

/// JSON null becomes nil, as a script expects.
fn plain() -> SerializeOptions {
    SerializeOptions::new().serialize_none_to_null(false).serialize_unit_to_null(false)
}

/// A Lua state with the script read into it, and what the prelude gives to
/// drive it.
struct Machine {
    lua: Lua,
    parts: Table,
}

impl Machine {
    fn new(host: &Rc<Host>, stack: &str, context: &Json, source: &str) -> Result<Self, Problem> {
        // By default mlua lets `pcall` catch a Rust panic, and a PostgreSQL
        // error travels as one: a script could then swallow a cancelled
        // query. This option makes `pcall` let them through.
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH | StdLib::UTF8,
            LuaOptions::new().catch_rust_panics(false),
        )?;
        lua.set_memory_limit(MEMORY)?;

        let ticking = Rc::clone(host);
        lua.set_global_hook(HookTriggers::new().every_nth_instruction(TICK), move |_, _| {
            pgrx::check_for_interrupts();
            ticking.left.set(ticking.left.get() - i64::from(TICK));
            ticking.watch_the_clock()?;
            Ok(VmState::Continue)
        })?;

        let parts: Table =
            lua.load(PRELUDE).set_name("=pgxila").call(host_table(&lua, host, stack, context)?)?;
        // Only source text is read: compiled Lua could do anything.
        lua.load(source)
            .set_name("=script")
            .set_mode(ChunkMode::Text)
            .set_environment(parts.get::<Table>("env")?)
            .exec()?;
        Ok(Machine { lua, parts })
    }

    /// Calls a function of the prelude that answers with a value, or with
    /// nothing and what went wrong.
    fn call(&self, name: &str, args: impl mlua::IntoLuaMulti) -> Result<Json, Problem> {
        let (answer, problem): (Value, Option<String>) =
            self.parts.get::<Function>(name)?.call(args)?;
        match problem {
            Some(message) => Err(Problem::new(message)),
            None => Ok(self.lua.from_value(answer)?),
        }
    }
}

/// What the prelude is given to build on.
fn host_table(lua: &Lua, host: &Rc<Host>, stack: &str, context: &Json) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("null", lua.null())?;
    table.set("array", lua.array_metatable())?;
    table.set("stack", stack)?;
    table.set("context", lua.to_value_with(context, plain())?)?;
    table.set("confirmed", context["confirmed"] == json!(true))?;

    let shared = Rc::clone(host);
    table.set(
        "query",
        lua.create_function(move |lua, (statement, values): (String, Table)| {
            if !shared.running.get() {
                return Ok((Value::Nil, Some(NOT_HERE.to_owned())));
            }
            let answer = sql::query(&statement, &arguments(&values)?);
            shared.watch_the_clock()?;
            Ok(match answer {
                Ok(rows) => (lua.to_value_with(&rows, plain())?, None),
                Err(message) => (Value::Nil, Some(message)),
            })
        })?,
    )?;

    let shared = Rc::clone(host);
    table.set(
        "execute",
        lua.create_function(move |_, (statement, values): (String, Table)| {
            if !shared.running.get() {
                return Ok((None, Some(NOT_HERE.to_owned())));
            }
            let answer = sql::execute(&statement, &arguments(&values)?);
            shared.watch_the_clock()?;
            Ok(match answer {
                Ok(count) => {
                    shared.rows.set(shared.rows.get() + count);
                    (Some(count), None)
                }
                Err(message) => (None, Some(message)),
            })
        })?,
    )?;

    table.set(
        "json",
        lua.create_function(|lua, value: Value| {
            let value: Json = lua.from_value(value)?;
            Ok(value.to_string())
        })?,
    )?;

    let shared = Rc::clone(host);
    table.set(
        "print",
        lua.create_function(move |_, line: String| {
            let mut printed = shared.printed.borrow_mut();
            if printed.len() < PRINTED {
                printed.push(line);
            }
            Ok(())
        })?,
    )?;

    let shared = Rc::clone(host);
    table.set(
        "ask",
        lua.create_function(move |_, (text, label): (String, String)| {
            if !shared.running.get() {
                return Err(mlua::Error::runtime(NOT_HERE));
            }
            shared.stop.borrow_mut().get_or_insert(Stop::Asked { text, label });
            Ok(())
        })?,
    )?;

    let shared = Rc::clone(host);
    table.set("stopped", lua.create_function(move |_, ()| Ok(shared.stopped()))?)?;

    let shared = Rc::clone(host);
    table.set(
        "count",
        lua.create_function(move |_, what: String| {
            let counter = if what == "saved" { &shared.saved } else { &shared.deleted };
            counter.set(counter.get() + 1);
            Ok(())
        })?,
    )?;
    Ok(table)
}

/// The values of a statement, as the prelude packs them: texts, with nil for
/// NULL, and their number in `n`.
fn arguments(values: &Table) -> mlua::Result<Vec<sql::Arg>> {
    let count: usize = values.get("n")?;
    (1..=count)
        .map(|index| {
            Ok(match values.get::<Value>(index)? {
                Value::Nil => None,
                Value::String(text) => Some(text.to_string_lossy()),
                Value::Integer(number) => Some(number.to_string()),
                Value::Number(number) => Some(number.to_string()),
                Value::Boolean(flag) => Some(flag.to_string()),
                other => {
                    return Err(mlua::Error::runtime(format!(
                        "a {} cannot be given to a statement",
                        other.type_name()
                    )))
                }
            })
        })
        .collect()
}

/// Reads a script without running any of its functions, and says what it
/// handles: `{"ok": true, "events": ["click"], "keys": {"F6": "New card"}}`,
/// or `{"ok": false, "line": 4, "message": "…"}`.
pub fn check(source: &str) -> Json {
    let host = Host::new();
    let described = Machine::new(&host, "", &json!({}), source)
        .and_then(|machine| machine.call("describe", ()));
    match described {
        Ok(found) => json!({ "ok": true, "events": found["events"], "keys": found["keys"] }),
        Err(_) if matches!(*host.stop.borrow(), Some(Stop::TooLong)) => {
            json!({ "ok": false, "line": null, "message": TOO_LONG })
        }
        Err(problem) => json!({ "ok": false, "line": problem.line, "message": problem.message }),
    }
}

/// Runs one event of a script: `{"on": "click"}`, `{"on": "key", "key":
/// "F6"}` or `{"on": "open"}`.
///
/// Answers with `{"effects": […], "printed": […], "wrote": {…}}`, or with
/// `{"error": {"line": 4, "message": "…"}, "printed": […]}`. What the script
/// wrote is kept only when it succeeded and `keep` is true. When it asked the
/// user a question that is not answered yet, nothing is kept and the only
/// effect is that question.
pub fn run(stack: &str, source: &str, event: &Json, context: &Json, keep: bool) -> Json {
    // The script of someone else would act with every right.
    // SAFETY: reads the role of the current backend.
    if unsafe { pgrx::pg_sys::superuser() } {
        return json!({ "error": {
            "line": null,
            "code": "XL004",
            "message": "Scripts do not run for a superuser. Use an ordinary role.",
        }});
    }
    let (function, argument) = match event["on"].as_str() {
        Some("click") => ("on_click", None),
        Some("key") => ("on_key", event["key"].as_str()),
        Some("open") => ("on_open", None),
        _ => return json!({ "error": Problem::new("This is not an event of a script.").json() }),
    };

    let host = Host::new();
    let played = sql::in_subtransaction(|| {
        let played = Machine::new(&host, stack, context, source).and_then(|machine| {
            host.running.set(true);
            machine.call("run", (function, argument))
        });
        let keep = keep && played.is_ok() && !host.stopped();
        (played, keep)
    });

    let printed = host.printed.borrow().clone();
    let stop = host.stop.borrow_mut().take();
    match (stop, played) {
        (Some(Stop::Asked { text, label }), _) => json!({
            "effects": [{ "do": "confirm", "text": text, "label": label }],
            "asked": true,
            "printed": printed,
        }),
        (Some(Stop::TooLong), _) => {
            json!({ "error": Problem::new(TOO_LONG).json(), "printed": printed })
        }
        (None, Ok(Ok(effects))) => json!({
            "effects": effects,
            "printed": printed,
            "wrote": {
                "saved": host.saved.get(),
                "deleted": host.deleted.get(),
                "rows": host.rows.get(),
            },
        }),
        (None, Ok(Err(problem))) => json!({ "error": problem.json(), "printed": printed }),
        (None, Err(message)) => {
            json!({ "error": Problem::new(message).json(), "printed": printed })
        }
    }
}
