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

// The system schema is plain SQL: tables, triggers and the client API are
// queries, and SQL says them best. Rust is kept for what SQL cannot do.
extension_sql_file!("../sql/schema.sql", name = "schema", bootstrap);
extension_sql_file!("../sql/api.sql", name = "api", requires = [xila::version]);
extension_sql_file!("../sql/system_stacks.sql", name = "system_stacks", requires = ["api"]);
extension_sql_file!("../sql/triggers.sql", name = "triggers", requires = ["system_stacks"]);

#[pg_schema]
mod xila {
    use pgrx::prelude::*;

    /// Version of the pgXila extension.
    #[pg_extern(immutable, parallel_safe)]
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

#[cfg(any(test, feature = "pg_test"))]
#[pg_schema]
mod tests {
    use crate::tls_guard::{check, Refusal, Transport};
    use pgrx::prelude::*;

    /// Runs a query that returns a single JSON value.
    fn json(query: &str) -> serde_json::Value {
        Spi::get_one::<pgrx::JsonB>(query).unwrap().expect("a JSON value").0
    }

    /// Runs a query that returns a single text value.
    fn text(query: &str) -> Option<String> {
        Spi::get_one::<String>(query).unwrap()
    }

    /// Runs a query that returns a single number.
    fn number(query: &str) -> i64 {
        Spi::get_one::<i64>(query).unwrap().expect("a number")
    }

    /// The SQLSTATE and message of the error a query raises.
    fn error_of(query: &str) -> (String, String) {
        error_of_statement(&format!("PERFORM ({query})"))
    }

    /// The SQLSTATE and message of the error a PL/pgSQL statement raises.
    fn error_of_statement(statement: &str) -> (String, String) {
        let probe = format!(
            "DO $probe$ BEGIN {statement}; RAISE EXCEPTION 'no error' USING ERRCODE = 'P0001';
             EXCEPTION WHEN OTHERS THEN
                 PERFORM set_config('test.sqlstate', SQLSTATE, true);
                 PERFORM set_config('test.message', SQLERRM, true);
             END $probe$"
        );
        Spi::run(&probe).unwrap();
        (
            text("SELECT current_setting('test.sqlstate')").unwrap(),
            text("SELECT current_setting('test.message')").unwrap(),
        )
    }

    /// A small stack with three cards, built the way a user would build one:
    /// a table, then rows in the xila tables.
    fn create_library_stack() {
        Spi::run(
            r#"
            CREATE SCHEMA library;
            CREATE TABLE library.books (
                id     integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                title  text NOT NULL,
                author text,
                year   integer,
                bought date
            );
            INSERT INTO library.books (title, author, year) VALUES
                ('Les Misérables', 'Hugo', 1862),
                ('Candide', 'Voltaire', 1759),
                ('Germinal', 'Zola', 1885);

            WITH s AS (
                INSERT INTO xila.stack (name, title) VALUES ('library', 'Library')
                RETURNING id),
            b AS (
                INSERT INTO xila.background
                    (stack_id, source_schema, source_table, key_column, order_columns)
                SELECT id, 'library', 'books', 'id', '{title}' FROM s RETURNING id),
            a AS (
                INSERT INTO xila.action (background_id, name, label, handler)
                SELECT b.id, v.name, v.label, v.name
                FROM b, (VALUES ('next_card', 'Next'), ('previous_card', 'Prev.'),
                                ('save_card', 'Save'), ('new_card', 'New')) v(name, label)
                RETURNING id, background_id, name),
            k AS (
                INSERT INTO xila.key_binding (background_id, key, action_id)
                SELECT background_id, 'F8', id FROM a WHERE name = 'next_card')
            INSERT INTO xila.widget (background_id, name, kind, ord, label, bound_column, props)
            SELECT b.id, v.name, v.kind, v.ord, v.label, v.col, v.props::jsonb
            FROM b, (VALUES
                ('heading', 'text', 10, NULL, NULL, '{"template": "{title} ({position}/{count})"}'),
                ('title', 'field', 20, 'Title', 'title', '{}'),
                ('year', 'field', 30, 'Year', 'year', '{}'),
                ('bought', 'field', 40, 'Bought on', 'bought', '{}')
            ) v(name, kind, ord, label, col, props);
            "#,
        )
        .unwrap();
    }

    #[pg_test]
    fn version_is_the_crate_version() {
        assert_eq!(text("SELECT xila.version()").as_deref(), Some(env!("CARGO_PKG_VERSION")));
    }

    #[pg_test]
    fn the_system_palette_has_32_colours() {
        assert_eq!(number("SELECT count(*) FROM xila.palette()"), 32);
        assert_eq!(
            text("SELECT name || ' ' || rgb FROM xila.palette() WHERE slot = 24").as_deref(),
            Some("orange #e8703a")
        );
    }

    #[pg_test]
    fn hello_names_the_home_stack() {
        let hello = json("SELECT xila.hello()");
        assert_eq!(hello["api"], 1);
        assert_eq!(hello["home"], "desktop");
    }

    #[pg_test]
    fn the_desktop_is_a_stack_that_opens_the_stack_list() {
        let desktop = json("SELECT xila.open_stack('desktop')");
        assert_eq!(desktop["stack"]["kind"], "system");
        assert_eq!(desktop["source"]["has_cards"], false);
        let widgets = desktop["widgets"].as_array().unwrap();
        let button = widgets.iter().find(|w| w["name"] == "stacks").unwrap();
        assert_eq!(button["kind"], "button");
        assert_eq!(button["action"], "open_stacks");
        assert_eq!(button["layout"]["graphic"]["style"]["variant"], "primary");
        // The welcome texts belong to the window, not to the text menu.
        let banner = widgets.iter().find(|w| w["name"] == "banner").unwrap();
        assert_eq!(banner["layout"]["text"]["style"]["hidden"], true);

        let effects = json("SELECT xila.act('desktop', 'open_stacks')");
        assert_eq!(effects, serde_json::json!([{ "do": "show", "stack": "stacks" }]));
    }

    #[pg_test]
    fn a_static_screen_can_say_where_and_who_we_are() {
        let card = json("SELECT xila.card('desktop')");
        assert_eq!(card["texts"]["banner"], "pgXila");
        assert_eq!(card["texts"]["instance"], json("SELECT xila.hello()")["instance"]);
        assert_eq!(
            card["texts"]["user"].as_str().map(str::to_owned),
            text("SELECT session_user::text")
        );
    }

    #[pg_test]
    fn an_image_is_read_by_its_identifier() {
        Spi::run(
            r"INSERT INTO xila.image (id, width, height, pixels)
              VALUES ('00000000-0000-7000-8000-000000000001', 3, 2, '\x000102030405')",
        )
        .unwrap();
        let image = json(
            "SELECT to_jsonb(i) FROM (
                 SELECT width, height, encode(pixels, 'hex') AS pixels
                 FROM xila.image('00000000-0000-7000-8000-000000000001')) i",
        );
        assert_eq!(image, serde_json::json!({ "width": 3, "height": 2, "pixels": "000102030405" }));
        assert_eq!(
            number("SELECT count(*) FROM xila.image('00000000-0000-7000-8000-000000000002')"),
            0
        );
        // The size of an image and its pixels must agree.
        let (code, _) = error_of_statement(
            r"INSERT INTO xila.image (width, height, pixels) VALUES (3, 2, '\x0001')",
        );
        assert_eq!(code, "23514");
    }

    #[pg_test]
    fn global_keys_are_part_of_every_screen() {
        let keys = json("SELECT xila.open_stack('desktop')->'keys'");
        assert_eq!(
            keys,
            serde_json::json!([
                { "key": "F3", "label": "Quit", "action": "quit" },
                { "key": "F12", "label": "Back", "action": "back" }
            ])
        );
        assert_eq!(
            json("SELECT xila.act('desktop', 'quit')"),
            serde_json::json!([{ "do": "quit" }])
        );
    }

    #[pg_test]
    fn an_unknown_stack_is_a_user_error() {
        let (state, message) = error_of("SELECT xila.open_stack('nowhere')");
        assert_eq!(state, "XL002");
        assert_eq!(message, "The stack “nowhere” does not exist.");
    }

    #[pg_test]
    fn a_screen_describes_its_widgets_and_field_types() {
        create_library_stack();
        let screen = json("SELECT xila.open_stack('library')");
        assert_eq!(screen["stack"]["title"], "Library");
        assert_eq!(screen["source"], serde_json::json!({ "has_cards": true, "count": 3 }));

        let widgets = screen["widgets"].as_array().unwrap();
        let names: Vec<_> = widgets.iter().map(|w| w["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["heading", "title", "year", "bought"]);
        assert_eq!(widgets[1]["type"], "text");
        assert_eq!(widgets[2]["type"], "number");
        assert_eq!(widgets[3]["type"], "date");
        assert_eq!(widgets[1]["layout"], serde_json::json!({}));
    }

    #[pg_test]
    fn cards_are_browsed_in_the_order_of_the_stack() {
        create_library_stack();
        let first = json("SELECT xila.card('library')");
        assert_eq!(first["values"]["title"], "Candide");
        assert_eq!(first["position"], 1);
        assert_eq!(first["count"], 3);
        assert_eq!(first["texts"]["heading"], "Candide (1/3)");
        // Values travel as text.
        assert_eq!(first["values"]["year"], "1759");

        let key = first["key"].as_str().unwrap();
        let next = json(&format!("SELECT xila.card('library', '{key}', 'next')"));
        assert_eq!(next["values"]["title"], "Germinal");

        let last = json("SELECT xila.card('library', NULL, 'last')");
        assert_eq!(last["values"]["title"], "Les Misérables");
        let key = last["key"].as_str().unwrap();
        let after_last = json(&format!("SELECT xila.card('library', '{key}', 'next')"));
        assert_eq!(after_last["key"], last["key"], "next stops at the last card");
    }

    #[pg_test]
    fn an_empty_stack_has_no_card() {
        create_library_stack();
        Spi::run("DELETE FROM library.books").unwrap();
        let card = json("SELECT xila.card('library')");
        assert_eq!(card["key"], serde_json::Value::Null);
        assert_eq!(card["count"], 0);
    }

    #[pg_test]
    fn saving_a_card_writes_the_changed_columns() {
        create_library_stack();
        let key = json("SELECT xila.card('library')")["key"].as_str().unwrap().to_owned();
        let saved = json(&format!(
            r#"SELECT xila.save_card('library', '{key}', '{{"year": "1760", "bought": "2026-11-25"}}')"#
        ));
        assert_eq!(saved["values"]["year"], "1760");
        assert_eq!(saved["values"]["bought"], "2026-11-25");
        assert_eq!(saved["values"]["title"], "Candide");
        assert_eq!(number("SELECT year::bigint FROM library.books WHERE title = 'Candide'"), 1760);
    }

    #[pg_test]
    fn an_empty_input_clears_a_value_that_is_not_text() {
        create_library_stack();
        let key = json("SELECT xila.card('library')")["key"].as_str().unwrap().to_owned();
        let saved =
            json(&format!(r#"SELECT xila.save_card('library', '{key}', '{{"year": ""}}')"#));
        assert_eq!(saved["values"]["year"], serde_json::Value::Null);
    }

    #[pg_test]
    fn a_rejected_value_names_the_field() {
        create_library_stack();
        let key = json("SELECT xila.card('library')")["key"].as_str().unwrap().to_owned();

        let (state, message) = error_of(&format!(
            r#"SELECT xila.save_card('library', '{key}', '{{"year": "soon"}}')"#
        ));
        assert_eq!(state, "XL003");
        assert_eq!(message, "The value “soon” is not valid for the field “year”.");

        let (state, message) =
            error_of(&format!(r#"SELECT xila.save_card('library', '{key}', '{{"title": null}}')"#));
        assert_eq!(state, "XL003");
        assert_eq!(message, "The field “title” is required.");
    }

    #[pg_test]
    fn a_new_card_is_created_when_there_is_no_key() {
        create_library_stack();
        let created = json(
            r#"SELECT xila.save_card('library', NULL, '{"title": "Bel-Ami", "year": "1885"}')"#,
        );
        assert_eq!(created["values"]["title"], "Bel-Ami");
        assert_eq!(created["count"], 4);
        assert_eq!(created["position"], 1);
    }

    #[pg_test]
    fn moving_to_another_card_saves_the_current_one() {
        create_library_stack();
        let key = json("SELECT xila.card('library')")["key"].as_str().unwrap().to_owned();
        let effects = json(&format!(
            r#"SELECT xila.act('library', 'next_card',
                   '{{"card": "{key}", "dirty": {{"author": "Arouet"}}}}')"#
        ));
        assert_eq!(effects[0]["do"], "show");
        assert_eq!(effects[0]["stack"], "library");
        assert_ne!(effects[0]["card"], key.as_str());
        assert_eq!(
            text("SELECT author FROM library.books WHERE title = 'Candide'").as_deref(),
            Some("Arouet")
        );
    }

    #[pg_test]
    fn every_change_is_recorded_in_history() {
        create_library_stack();
        let key = json("SELECT xila.card('library')")["key"].as_str().unwrap().to_owned();
        Spi::run(&format!(r#"SELECT xila.save_card('library', '{key}', '{{"year": "1760"}}')"#))
            .unwrap();

        let change = json(&format!(
            "SELECT jsonb_build_object('op', op, 'before', before->'year', 'after', after->'year',
                                       'stack', (SELECT name FROM xila.stack WHERE id = stack_id))
             FROM xila.history
             WHERE object_kind = 'card' AND object_key = '{key}' ORDER BY id DESC LIMIT 1"
        ));
        assert_eq!(
            change,
            serde_json::json!({ "op": "update", "before": 1759, "after": 1760, "stack": "library" })
        );

        // Building the stack was recorded too: its widgets, for instance.
        assert_eq!(
            number(
                "SELECT count(*) FROM xila.history h JOIN xila.stack s ON s.id = h.stack_id
                 WHERE s.name = 'library' AND h.object_kind = 'widget' AND h.op = 'insert'"
            ),
            4
        );
    }

    #[pg_test]
    fn history_starts_empty_in_a_new_database() {
        assert_eq!(number("SELECT count(*) FROM xila.history"), 0);
    }

    #[pg_test]
    fn a_restore_does_not_record_what_it_reloads() {
        create_library_stack();
        let before = number("SELECT count(*) FROM xila.history");

        // pg_dump output runs with an empty search_path, as a superuser.
        Spi::run(
            "SELECT pg_catalog.set_config('search_path', '', true);
             UPDATE library.books SET year = 1 WHERE title = 'Candide';
             UPDATE xila.stack SET title = 'Restored' WHERE name = 'library';",
        )
        .unwrap();
        assert_eq!(number("SELECT count(*) FROM xila.history"), before);

        Spi::run(
            "SET LOCAL search_path = public;
             UPDATE library.books SET year = 2 WHERE title = 'Candide';",
        )
        .unwrap();
        assert_eq!(number("SELECT count(*) FROM xila.history"), before + 1);
    }

    #[pg_test]
    fn a_source_table_needs_a_single_column_primary_key() {
        Spi::run(
            "CREATE TABLE public.pairs (a integer, b integer, PRIMARY KEY (a, b));
             INSERT INTO xila.stack (name, title) VALUES ('pairs', 'Paires');",
        )
        .unwrap();
        let (state, message) = error_of_statement(
            "INSERT INTO xila.background (stack_id, source_schema, source_table, key_column)
             SELECT id, 'public', 'pairs', 'a' FROM xila.stack WHERE name = 'pairs'",
        );
        assert!(message.contains("primary key"), "{message}");
        assert_eq!(state, "XL003");
    }

    #[pg_test]
    fn the_stack_list_is_a_grid_of_the_visible_stacks() {
        create_library_stack();
        let rows = json("SELECT xila.grid_rows('stacks', 'list')");
        assert_eq!(rows["total"], 1);
        assert_eq!(rows["rows"][0]["key"], "library");
        assert_eq!(
            rows["rows"][0]["cells"],
            serde_json::json!({ "title": "Library", "cards": "3" })
        );

        let effects = json(r#"SELECT xila.act('stacks', 'open', '{"row": "library"}')"#);
        assert_eq!(effects, serde_json::json!([{ "do": "show", "stack": "library" }]));
    }

    #[pg_test]
    fn guard_accepts_the_local_socket() {
        assert_eq!(check(Transport::UnixSocket), Ok(()));
    }

    #[pg_test]
    fn guard_accepts_tls_1_3() {
        assert_eq!(check(Transport::Tcp { tls_version: Some("TLSv1.3") }), Ok(()));
    }

    #[pg_test]
    fn guard_refuses_plain_tcp() {
        assert_eq!(check(Transport::Tcp { tls_version: None }), Err(Refusal::NotEncrypted));
    }

    #[pg_test]
    fn guard_refuses_older_tls() {
        for version in ["TLSv1.2", "TLSv1.1", "TLSv1", "SSLv3", ""] {
            assert_eq!(
                check(Transport::Tcp { tls_version: Some(version) }),
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
