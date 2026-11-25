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
extension_sql_file!("../sql/author.sql", name = "author", requires = ["api"]);
extension_sql_file!("../sql/actions.sql", name = "actions", requires = ["author"]);
extension_sql_file!("../sql/system_stacks.sql", name = "system_stacks", requires = ["actions"]);
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

    // ----------------------------------------------------------------------
    // Author mode
    // ----------------------------------------------------------------------

    /// Creates the people of the author tests: ada and bob are authors, carol
    /// may edit what ada owns, dan is a plain user.
    fn create_people() {
        Spi::run(
            "CREATE ROLE ada LOGIN IN ROLE xila_author;
             CREATE ROLE bob LOGIN IN ROLE xila_author;
             CREATE ROLE carol LOGIN IN ROLE xila_author, ada;
             CREATE ROLE dan LOGIN IN ROLE xila_user;",
        )
        .unwrap();
    }

    /// Continues the test as another role, as if it had logged in.
    fn become_role(role: &str) {
        Spi::run(&format!("SET LOCAL SESSION AUTHORIZATION {role}")).unwrap();
    }

    fn become_admin() {
        Spi::run("RESET SESSION AUTHORIZATION").unwrap();
    }

    /// As ada: a stack of books with a title and a year, and two cards.
    fn create_books_as_ada() {
        create_people();
        become_role("ada");
        Spi::run(
            r#"SELECT xila.create_stack('Books');
               SELECT xila.add_field('books', 'Title');
               SELECT xila.add_field('books', 'Year', 'number');
               SELECT xila.save_card('books', NULL, '{"title": "Candide", "year": "1759"}');
               SELECT xila.save_card('books', NULL, '{"title": "Germinal", "year": "1885"}');"#,
        )
        .unwrap();
    }

    /// The table of the library stack, owned by a role, without any stack.
    fn create_library_table_for(owner: &str) {
        Spi::run(&format!(
            "CREATE SCHEMA library AUTHORIZATION {owner};
             CREATE TABLE library.books (
                 id     integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                 title  text NOT NULL,
                 author text,
                 year   integer,
                 bought date
             );
             ALTER TABLE library.books OWNER TO {owner};
             INSERT INTO library.books (title, author, year) VALUES
                 ('Les Misérables', 'Hugo', 1862), ('Candide', 'Voltaire', 1759);"
        ))
        .unwrap();
    }

    /// The id of the first recorded change of a stack that matches a condition.
    ///
    /// A test runs in a single transaction, which the history of a stack
    /// shows as a single line; the tests therefore name changes one by one.
    /// The integration tests check the lines, with one transaction per call.
    fn first_change(stack: &str, condition: &str) -> i64 {
        number(&format!(
            "SELECT min(h.id) FROM xila.history h JOIN xila.stack s ON s.id = h.stack_id
             WHERE s.name = '{stack}' AND {condition}"
        ))
    }

    fn last_change() -> i64 {
        number("SELECT max(id) FROM xila.history")
    }

    /// The keys of a screen, as "F6=New".
    fn keys_of(screen: &serde_json::Value) -> Vec<String> {
        screen["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|k| format!("{}={}", k["key"].as_str().unwrap(), k["label"].as_str().unwrap()))
            .collect()
    }

    /// The names of the fields of a stack, in their order.
    fn fields_of(stack: &str) -> serde_json::Value {
        json(&format!(
            "SELECT coalesce(jsonb_agg(name ORDER BY ord), '[]') FROM xila.field
             WHERE stack = '{stack}'"
        ))
    }

    #[pg_test]
    fn an_author_builds_a_stack_without_sql() {
        create_books_as_ada();

        // The stack keeps its cards in a table of its own, which ada owns.
        assert_eq!(
            text(
                "SELECT tableowner::text FROM pg_tables
                 WHERE schemaname = 'books' AND tablename = 'cards'"
            )
            .as_deref(),
            Some("ada")
        );
        assert_eq!(
            text("SELECT owner::text FROM xila.stack WHERE name = 'books'").as_deref(),
            Some("ada")
        );

        let screen = json("SELECT xila.open_stack('books')");
        assert_eq!(screen["stack"]["may_edit"], true);
        let fields: Vec<_> = screen["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["kind"] == "field")
            .map(|w| (w["name"].as_str().unwrap(), w["type"].as_str().unwrap()))
            .collect();
        assert_eq!(fields, [("title", "text"), ("year", "number")]);

        // The first field became the title of the cards.
        let card = json("SELECT xila.card('books')");
        assert_eq!(card["count"], 2);
        assert_eq!(card["texts"]["heading"], "Candide");

        // A new stack can be browsed, saved, and its cards deleted.
        assert_eq!(
            keys_of(&screen),
            [
                "F3=Quit",
                "F4=Delete",
                "F6=New",
                "F7=Prev.",
                "F8=Next",
                "F10=History",
                "F11=Author",
                "F12=Back",
                "Enter=Save"
            ]
        );
    }

    #[pg_test]
    fn the_name_of_a_stack_is_made_from_its_title() {
        create_people();
        become_role("ada");
        assert_eq!(
            text("SELECT xila.create_stack('Mes recettes préférées')").as_deref(),
            Some("mes_recettes_preferees")
        );
        let (state, message) = error_of("SELECT xila.create_stack('Mes recettes préférées')");
        assert_eq!(state, "XL003");
        assert_eq!(message, "A stack named “mes_recettes_preferees” already exists.");
        let (state, _) = error_of("SELECT xila.create_stack('Tables', 'pg_tables')");
        assert_eq!(state, "XL003");
        let (state, message) = error_of("SELECT xila.create_stack('   ')");
        assert_eq!((state.as_str(), message.as_str()), ("XL003", "A stack needs a title."));
    }

    #[pg_test]
    fn only_authors_create_stacks() {
        create_people();
        become_role("dan");
        let (state, message) = error_of("SELECT xila.create_stack('Notes')");
        assert_eq!(
            (state.as_str(), message.as_str()),
            ("XL004", "Only authors can create stacks.")
        );
        // The function that creates the schema cannot be used on its own.
        let (state, _) = error_of_statement("PERFORM xila.make_schema('notes')");
        assert_eq!(state, "42501");
    }

    #[pg_test]
    fn a_stack_can_only_be_changed_by_its_owner() {
        create_books_as_ada();

        become_role("bob");
        let (state, message) = error_of("SELECT xila.add_field('books', 'Author')");
        assert_eq!(state, "XL004");
        assert_eq!(message, "The stack “Books” belongs to ada: you cannot change it.");
        let (state, _) = error_of(r#"SELECT xila.set_stack('books', '{"title": "Mine"}')"#);
        assert_eq!(state, "XL004");
        let (state, _) = error_of("SELECT xila.delete_stack('books')");
        assert_eq!(state, "XL004");
        let (state, _) = error_of("SELECT xila.rewind('books', 'stack', 1)");
        assert_eq!(state, "XL004");

        // Writing the rows in SQL does not get around it: the tables refuse.
        Spi::run("UPDATE xila.stack SET title = 'Mine' WHERE name = 'books'").unwrap();
        Spi::run("DELETE FROM xila.widget WHERE name = 'title'").unwrap();
        let (state, _) = error_of_statement(
            "INSERT INTO xila.widget (background_id, name, kind)
             VALUES (xila.background_id('books'), 'spy', 'text')",
        );
        assert_eq!(state, "42501");
        let (state, _) = error_of_statement(
            "INSERT INTO xila.stack (name, title, owner) VALUES ('fake', 'Fake', 'ada')",
        );
        assert_eq!(state, "42501");

        // Nor may an author touch a system stack.
        Spi::run("UPDATE xila.stack SET title = 'Hacked' WHERE name = 'desktop'").unwrap();
        Spi::run("DELETE FROM xila.action WHERE name = 'quit'").unwrap();
        assert_eq!(json("SELECT xila.open_stack('desktop')")["stack"]["may_edit"], false);

        become_admin();
        assert_eq!(
            text("SELECT title FROM xila.stack WHERE name = 'books'").as_deref(),
            Some("Books")
        );
        assert_eq!(
            text("SELECT title FROM xila.stack WHERE name = 'desktop'").as_deref(),
            Some("Main menu")
        );
        assert_eq!(fields_of("books"), serde_json::json!(["title", "year"]));
        assert_eq!(number("SELECT count(*) FROM xila.action WHERE name = 'quit'"), 1);

        // A member of the owner role may edit: that is how editing is shared.
        become_role("carol");
        assert_eq!(text("SELECT xila.add_field('books', 'Author')").as_deref(), Some("author"));
    }

    #[pg_test]
    fn a_field_is_renamed_moved_and_made_required() {
        create_books_as_ada();
        Spi::run(
            r#"SELECT xila.set_field('books', 'year', '{"label": "Published", "name": "published"}');
               SELECT xila.move_field('books', 'published', -1);"#,
        )
        .unwrap();
        let fields = json(
            "SELECT jsonb_agg(jsonb_build_array(name, label, type) ORDER BY ord)
             FROM xila.field WHERE stack = 'books'",
        );
        assert_eq!(
            fields,
            serde_json::json!([["published", "Published", "number"], ["title", "Title", "text"]])
        );
        assert_eq!(json("SELECT xila.card('books')")["values"]["published"], "1759");

        // Required is refused while a card has no value, and says how many.
        Spi::run(r#"SELECT xila.save_card('books', NULL, '{"title": "Nana"}')"#).unwrap();
        let (state, message) =
            error_of(r#"SELECT xila.set_field('books', 'published', '{"required": true}')"#);
        assert_eq!(state, "XL003");
        assert_eq!(
            message,
            "The field “published” cannot be required yet: 1 card has no value for it."
        );
        Spi::run(r#"SELECT xila.set_field('books', 'title', '{"required": true}')"#).unwrap();
        let (state, message) =
            error_of(r#"SELECT xila.save_card('books', NULL, '{"published": "1900"}')"#);
        assert_eq!(
            (state.as_str(), message.as_str()),
            ("XL003", "The field “title” is required.")
        );

        // A renamed field is renamed in the title of the cards too.
        Spi::run(r#"SELECT xila.set_field('books', 'title', '{"name": "work"}')"#).unwrap();
        assert_eq!(
            text("SELECT card_title FROM xila.stack_info WHERE name = 'books'").as_deref(),
            Some("{work}")
        );
    }

    #[pg_test]
    fn a_removed_field_comes_back_with_its_values() {
        create_books_as_ada();
        Spi::run("SELECT xila.remove_field('books', 'year')").unwrap();
        assert_eq!(
            number(
                "SELECT count(*) FROM information_schema.columns
                 WHERE table_schema = 'books' AND column_name = 'year'"
            ),
            0
        );
        assert_eq!(json("SELECT xila.card('books')")["values"].get("year"), None);

        let removal = first_change(
            "books",
            "h.object_kind = 'widget' AND h.op = 'delete' AND h.before->>'name' = 'year'",
        );
        let before_rewind = last_change();
        let undone = json(&format!("SELECT xila.rewind('books', 'screen', {removal})"));
        assert_eq!(undone["undone"], 2, "the widget and the column");

        let years = json("SELECT jsonb_agg(year ORDER BY title) FROM books.cards");
        assert_eq!(years, serde_json::json!([1759, 1885]));
        assert_eq!(fields_of("books"), serde_json::json!(["title", "year"]));

        // The rewind is itself in history, and can be rewound.
        assert_eq!(
            text("SELECT xila.describe_change(object_kind, op, before, after)
                  FROM xila.history ORDER BY id DESC LIMIT 1")
            .as_deref(),
            Some("Screen rewound (2 changes undone)")
        );
        Spi::run(&format!("SELECT xila.rewind('books', 'screen', {})", before_rewind + 1)).unwrap();
        assert_eq!(fields_of("books"), serde_json::json!(["title"]));
        assert_eq!(
            number(
                "SELECT count(*) FROM information_schema.columns
                 WHERE table_schema = 'books' AND column_name = 'year'"
            ),
            0
        );
    }

    #[pg_test]
    fn a_card_is_rewound_alone() {
        create_books_as_ada();
        let key = json("SELECT xila.card('books')")["key"].as_str().unwrap().to_owned();
        Spi::run(&format!(
            r#"SELECT xila.save_card('books', '{key}', '{{"year": "1760"}}');
               SELECT xila.save_card('books', '{key}', '{{"title": "Candide ou l''Optimisme"}}');
               UPDATE books.cards SET year = 1 WHERE title = 'Germinal';"#
        ))
        .unwrap();

        let first_change = number(&format!(
            "SELECT min(id) FROM xila.card_history_line
             WHERE stack = 'books' AND card = '{key}' AND summary LIKE '%changed: year'"
        ));
        let undone =
            json(&format!("SELECT xila.rewind('books', 'card', {first_change}, '{key}')"));
        assert_eq!(undone["undone"], 2);
        let card = json(&format!("SELECT xila.card('books', '{key}', 'at')"));
        assert_eq!(card["values"]["title"], "Candide");
        assert_eq!(card["values"]["year"], "1759");
        // The other card was left alone.
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Germinal'"), 1);
    }

    #[pg_test]
    fn a_deleted_card_comes_back() {
        create_books_as_ada();
        let key = json("SELECT xila.card('books')")["key"].as_str().unwrap().to_owned();

        // Deleting asks first, then shows the next card.
        let asked = json(&format!(r#"SELECT xila.act('books', 'delete', '{{"card": "{key}"}}')"#));
        assert_eq!(
            asked,
            serde_json::json!([{ "do": "confirm", "text": "Delete this card?", "label": "Delete" }])
        );
        assert_eq!(number("SELECT count(*) FROM books.cards"), 2);
        let effects = json(&format!(
            r#"SELECT xila.act('books', 'delete', '{{"card": "{key}", "confirmed": true}}')"#
        ));
        assert_eq!(effects[0]["do"], "show");
        assert_ne!(effects[0]["card"], key.as_str());
        assert_eq!(effects[1]["text"], "Card deleted.");
        assert_eq!(number("SELECT count(*) FROM books.cards"), 1);

        let line = text(
            "SELECT id::text || ' ' || summary FROM xila.card_history_line
             WHERE stack = 'books' ORDER BY id DESC LIMIT 1",
        )
        .unwrap();
        let (id, summary) = line.split_once(' ').unwrap();
        assert_eq!(summary, "Card “Candide” deleted");
        Spi::run(&format!("SELECT xila.rewind('books', 'card', {id}, '{key}')")).unwrap();
        assert_eq!(
            json(&format!("SELECT xila.card('books', '{key}', 'at')"))["values"]["title"],
            "Candide"
        );
    }

    #[pg_test]
    fn the_screen_and_the_whole_stack_are_rewound_separately() {
        create_books_as_ada();
        // After this point: a field is added, a card is changed.
        Spi::run("SELECT xila.add_field('books', 'Author')").unwrap();
        Spi::run("UPDATE books.cards SET author = 'Voltaire', year = 1760 WHERE title = 'Candide'")
            .unwrap();
        let since = first_change("books", "h.object_kind = 'column' AND h.object_key = 'author'");

        // The screen alone: the field goes, the year typed since stays.
        Spi::run(&format!("SELECT xila.rewind('books', 'screen', {since})")).unwrap();
        assert_eq!(fields_of("books"), serde_json::json!(["title", "year"]));
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1760);

        // The whole stack: the cards go back too.
        Spi::run(&format!("SELECT xila.rewind('books', 'stack', {since})")).unwrap();
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1759);
        assert_eq!(fields_of("books"), serde_json::json!(["title", "year"]));
    }

    #[pg_test]
    fn a_rewind_is_all_or_nothing() {
        create_books_as_ada();
        let since = first_change("books", "h.object_kind = 'column' AND h.object_key = 'year'");
        // Someone alters the table by hand: history does not know.
        Spi::run("ALTER TABLE books.cards DROP COLUMN year").unwrap();
        Spi::run(r#"SELECT xila.set_stack('books', '{"title": "Novels"}')"#).unwrap();

        let (state, message) = error_of(&format!("SELECT xila.rewind('books', 'stack', {since})"));
        assert_eq!(state, "XL003");
        assert!(message.starts_with("The rewind stopped at the change of "), "{message}");
        assert!(message.ends_with("Nothing was changed."), "{message}");
        assert_eq!(
            text("SELECT title FROM xila.stack WHERE name = 'books'").as_deref(),
            Some("Novels")
        );
    }

    #[pg_test]
    fn a_deleted_stack_keeps_its_cards_and_can_be_restored() {
        create_books_as_ada();
        let screen_before = json("SELECT xila.open_stack('books')");
        Spi::run("SELECT xila.delete_stack('books')").unwrap();

        assert_eq!(number("SELECT count(*) FROM xila.stack WHERE name = 'books'"), 0);
        assert_eq!(number("SELECT count(*) FROM books.cards"), 2, "the cards are kept");
        assert_eq!(
            number(
                "SELECT count(*) FROM pg_trigger
                 WHERE tgrelid = 'books.cards'::regclass AND tgname = 'xila_card'"
            ),
            0,
            "the table is no longer a stack"
        );

        // Only its former owner sees it among the deleted stacks.
        become_role("bob");
        assert_eq!(number("SELECT count(*) FROM xila.deleted_stack"), 0);
        become_role("ada");
        let deleted = json("SELECT to_jsonb(d) FROM xila.deleted_stack d");
        assert_eq!(deleted["name"], "books");
        assert_eq!(deleted["title"], "Books");

        Spi::run(&format!("SELECT xila.rewind('books', 'stack', {})", deleted["id"])).unwrap();
        assert_eq!(json("SELECT xila.open_stack('books')"), screen_before);
        assert_eq!(json("SELECT xila.card('books')")["count"], 2);
        assert_eq!(number("SELECT count(*) FROM xila.deleted_stack"), 0);
        // Its cards are recorded again.
        Spi::run("UPDATE books.cards SET year = 2 WHERE title = 'Candide'").unwrap();
        assert_eq!(
            number(
                "SELECT count(*) FROM xila.card_history_line
                 WHERE stack = 'books' AND summary = 'Card “Candide” changed: year'"
            ),
            1
        );
    }

    #[pg_test]
    fn pgxila_never_alters_a_table_it_did_not_create() {
        create_people();
        create_library_table_for("ada");
        become_role("ada");
        assert_eq!(
            json("SELECT coalesce(jsonb_agg(name), '[]') FROM xila.own_table"),
            serde_json::json!(["library.books"])
        );
        assert_eq!(
            text("SELECT xila.create_stack('Library', NULL, 'library.books')").as_deref(),
            Some("library")
        );
        // One field per column, the key left out, and a title for the cards.
        assert_eq!(fields_of("library"), serde_json::json!(["title", "author", "year", "bought"]));
        assert_eq!(json("SELECT xila.card('library', NULL, 'last')")["texts"]["heading"], "Candide");
        assert_eq!(
            text("SELECT managed::text FROM xila.stack_info WHERE name = 'library'").as_deref(),
            Some("false")
        );
        assert_eq!(number("SELECT count(*) FROM xila.own_table"), 0);

        // Taking a field off the screen leaves the column where it is...
        Spi::run("SELECT xila.remove_field('library', 'bought')").unwrap();
        assert_eq!(
            number(
                "SELECT count(*) FROM information_schema.columns
                 WHERE table_schema = 'library' AND column_name = 'bought'"
            ),
            1
        );
        // ...and it can be shown again.
        assert_eq!(
            text("SELECT xila.add_field('library', 'Bought on', 'date', 'bought')").as_deref(),
            Some("bought")
        );
        // A field that would need a new column is refused, and so is a rename.
        let (state, message) = error_of("SELECT xila.add_field('library', 'Publisher')");
        assert_eq!(state, "XL003");
        assert!(
            message.starts_with("The table of this stack has no column “publisher”."),
            "{message}"
        );
        let (state, _) =
            error_of(r#"SELECT xila.set_field('library', 'year', '{"name": "published"}')"#);
        assert_eq!(state, "XL003");
        let (state, _) =
            error_of(r#"SELECT xila.set_field('library', 'year', '{"required": true}')"#);
        assert_eq!(state, "XL003");

        // A table that belongs to someone else cannot become a stack.
        Spi::run("SELECT xila.delete_stack('library'); GRANT USAGE ON SCHEMA library TO bob")
            .unwrap();
        become_role("bob");
        let (state, _) = error_of("SELECT xila.create_stack('Mine', NULL, 'library.books')");
        assert_eq!(state, "XL004");
    }

    #[pg_test]
    fn history_is_read_by_those_who_may_read_the_cards() {
        create_books_as_ada();
        Spi::run("GRANT USAGE ON SCHEMA books TO dan").unwrap();

        // Without access to the table: nothing of the stack.
        become_role("dan");
        assert_eq!(number("SELECT count(*) FROM xila.history_line WHERE stack = 'books'"), 0);

        // With access to the whole table: its changes, cards included.
        become_role("ada");
        Spi::run("GRANT SELECT, UPDATE ON books.cards TO dan").unwrap();
        become_role("dan");
        assert_eq!(number("SELECT count(*) FROM xila.card_history_line WHERE stack = 'books'"), 2);
        assert_eq!(number("SELECT count(*) FROM xila.history_line WHERE stack = 'books'"), 1);
        // ...and may rewind a card, but not the stack.
        let key = json("SELECT xila.card('books')")["key"].as_str().unwrap().to_owned();
        Spi::run(&format!(r#"SELECT xila.save_card('books', '{key}', '{{"year": "3"}}')"#))
            .unwrap();
        let change = number("SELECT max(id) FROM xila.card_history_line WHERE stack = 'books'");
        Spi::run(&format!("SELECT xila.rewind('books', 'card', {change}, '{key}')")).unwrap();
        assert_eq!(json("SELECT xila.card('books')")["values"]["year"], "1759");
        let (state, _) = error_of(&format!("SELECT xila.rewind('books', 'stack', {change})"));
        assert_eq!(state, "XL004");

        // When the table hides rows, history hides the cards from all but
        // those who may edit the stack.
        become_role("ada");
        Spi::run(
            "ALTER TABLE books.cards ENABLE ROW LEVEL SECURITY;
             CREATE POLICY few ON books.cards FOR SELECT TO dan USING (title = 'Candide');",
        )
        .unwrap();
        assert!(number("SELECT count(*) FROM xila.card_history_line WHERE stack = 'books'") > 0);
        become_role("dan");
        assert_eq!(number("SELECT count(*) FROM xila.card_history_line WHERE stack = 'books'"), 0);
        assert!(number("SELECT count(*) FROM xila.history_line WHERE stack = 'books'") > 0);

        // Nobody writes history by hand.
        let (state, _) = error_of_statement(
            "INSERT INTO xila.history (stack_id, object_kind, object_key, op)
             SELECT id, 'card', 'x', 'delete' FROM xila.stack WHERE name = 'books'",
        );
        assert_eq!(state, "42501");
    }

    #[pg_test]
    fn the_author_screens_are_stacks() {
        create_people();
        become_role("ada");

        // The list of stacks offers authors what it hides from the others.
        assert_eq!(
            keys_of(&json("SELECT xila.open_stack('stacks')")),
            ["F3=Quit", "F6=New stack", "F10=Deleted", "F11=Author", "F12=Back"]
        );
        become_role("dan");
        assert_eq!(keys_of(&json("SELECT xila.open_stack('stacks')")), ["F3=Quit", "F12=Back"]);
        let (state, _) = error_of("SELECT xila.act('stacks', 'new_stack')");
        assert_eq!(state, "XL004");

        // New stack: an empty form, then what was typed creates the stack.
        become_role("ada");
        assert_eq!(
            json("SELECT xila.act('stacks', 'new_stack')"),
            serde_json::json!([{ "do": "show", "stack": "new_stack" }, { "do": "new" }])
        );
        let form = json("SELECT xila.open_stack('new_stack')");
        let widgets = form["widgets"].as_array().unwrap();
        let source = widgets.iter().find(|w| w["name"] == "source").unwrap();
        assert_eq!(
            source["props"]["choices"],
            serde_json::json!([{ "value": "", "label": "An empty stack" }])
        );
        let effects = json(
            r#"SELECT xila.act('new_stack', 'create', '{"card": null, "dirty": {"title": "Books"}}')"#,
        );
        assert_eq!(effects[0], serde_json::json!({ "do": "back" }));
        assert_eq!(
            effects[1],
            serde_json::json!({ "do": "show", "stack": "author", "card": "books" })
        );

        // Author mode shows the stack as a card, and its fields as a grid.
        let card = json("SELECT xila.card('author', 'books', 'at')");
        assert_eq!(card["values"]["title"], "Books");
        assert_eq!(card["texts"]["heading"], "Books");
        assert_eq!(
            json(r#"SELECT xila.act('author', 'new_field', '{"card": "books"}')"#),
            serde_json::json!([
                { "do": "show", "stack": "field" },
                { "do": "new", "values": { "stack": "books", "type": "text" } }
            ])
        );
        for (label, kind) in [("Title", "text"), ("Read", "boolean"), ("Year", "number")] {
            let effects = json(&format!(
                r#"SELECT xila.act('field', 'save', '{{"card": null,
                       "dirty": {{"stack": "books", "label": "{label}", "type": "{kind}"}}}}')"#
            ));
            assert_eq!(effects[0], serde_json::json!({ "do": "back" }));
            assert_eq!(effects[1]["text"], format!("Field “{label}” added."));
        }
        let rows = json("SELECT xila.grid_rows('author', 'fields', 0, 100, 'books')");
        assert_eq!(rows["total"], 3);
        let cells: Vec<_> = rows["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| {
                format!(
                    "{} {}",
                    r["cells"]["name"].as_str().unwrap(),
                    r["cells"]["type_label"].as_str().unwrap()
                )
            })
            .collect();
        assert_eq!(cells, ["title Text", "read Yes or no", "year Number"]);
        // Without a card, a grid tied to the card has no rows.
        assert_eq!(json("SELECT xila.grid_rows('author', 'fields')")["total"], 0);

        // Moving a field keeps it selected.
        let year = rows["rows"][2]["key"].as_str().unwrap().to_owned();
        let effects = json(&format!(
            r#"SELECT xila.act('author', 'up', '{{"card": "books", "row": "{year}"}}')"#
        ));
        assert_eq!(
            effects,
            serde_json::json!([{ "do": "reload" }, { "do": "select", "row": year }])
        );
        assert_eq!(fields_of("books"), serde_json::json!(["title", "year", "read"]));

        // Editing a field: its card is the field; the type cannot change.
        let field = json(&format!("SELECT xila.card('field', '{year}', 'at')"));
        assert_eq!(field["values"]["label"], "Year");
        assert_eq!(field["values"]["required"], "false");
        let (state, _) = error_of(&format!(
            r#"SELECT xila.act('field', 'save', '{{"card": "{year}", "dirty": {{"type": "date"}}}}')"#
        ));
        assert_eq!(state, "XL003");
        Spi::run(&format!(
            r#"SELECT xila.act('field', 'save',
                   '{{"card": "{year}", "dirty": {{"label": "Published"}}}}')"#
        ))
        .unwrap();

        // Removing it asks first.
        Spi::run(r#"SELECT xila.save_card('books', NULL, '{"title": "Candide", "year": "1759"}')"#)
            .unwrap();
        let asked = json(&format!(r#"SELECT xila.act('field', 'remove', '{{"card": "{year}"}}')"#));
        assert_eq!(asked[0]["do"], "confirm");
        assert_eq!(
            asked[0]["text"],
            "Remove the field “Published”? 1 card has a value for it. History can bring it back."
        );
        let effects = json(&format!(
            r#"SELECT xila.act('field', 'remove', '{{"card": "{year}", "confirmed": true}}')"#
        ));
        assert_eq!(effects[1]["text"], "Field “Published” removed.");
        // A field that is gone is an error, not another field.
        let (state, message) = error_of(&format!("SELECT xila.card('field', '{year}', 'at')"));
        assert_eq!((state.as_str(), message.as_str()), ("XL002", "This no longer exists."));

        // History, and rewinding from it.
        assert_eq!(
            json(r#"SELECT xila.act('author', 'history', '{"card": "books"}')"#),
            serde_json::json!([{ "do": "show", "stack": "history", "card": "books" }])
        );
        // (This test is one transaction, hence one line: see first_change.)
        let lines = json("SELECT xila.grid_rows('history', 'changes', 0, 100, 'books')");
        assert_eq!(lines["total"], 1);
        assert_eq!(lines["rows"][0]["cells"]["summary"], "Stack created");
        assert_eq!(lines["rows"][0]["cells"]["actor"], "ada");
        let line = first_change(
            "books",
            "h.object_kind = 'widget' AND h.op = 'delete' AND h.before->>'name' = 'year'",
        );
        let asked = json(&format!(
            r#"SELECT xila.act('history', 'rewind_screen', '{{"card": "books", "row": "{line}"}}')"#
        ));
        assert_eq!(asked[0]["label"], "Rewind");
        let effects = json(&format!(
            r#"SELECT xila.act('history', 'rewind_screen',
                   '{{"card": "books", "row": "{line}", "confirmed": true}}')"#
        ));
        assert_eq!(effects[0], serde_json::json!({ "do": "back" }));
        assert_eq!(effects[1]["text"], "2 changes undone.");
        assert_eq!(number("SELECT year::bigint FROM books.cards"), 1759);

        // Deleting the stack, and bringing it back from the deleted stacks.
        let asked = json(r#"SELECT xila.act('author', 'delete_stack', '{"card": "books"}')"#);
        assert_eq!(asked[0]["do"], "confirm");
        Spi::run(
            r#"SELECT xila.act('author', 'delete_stack', '{"card": "books", "confirmed": true}')"#,
        )
        .unwrap();
        let deleted = json("SELECT xila.grid_rows('deleted_stacks', 'list')");
        assert_eq!(deleted["rows"][0]["cells"]["title"], "Books");
        let row = deleted["rows"][0]["key"].as_str().unwrap();
        let effects = json(&format!(
            r#"SELECT xila.act('deleted_stacks', 'restore',
                   '{{"row": "{row}", "confirmed": true}}')"#
        ));
        assert_eq!(effects[1]["text"], "Stack “Books” restored.");
        assert_eq!(json("SELECT xila.card('books')")["count"], 1);
    }

    #[pg_test]
    fn author_mode_is_offered_on_the_stacks_one_may_change() {
        create_books_as_ada();
        assert_eq!(
            json("SELECT xila.act('books', 'author')"),
            serde_json::json!([{ "do": "show", "stack": "author", "card": "books" }])
        );
        assert_eq!(
            json(r#"SELECT xila.act('stacks', 'author', '{"row": "books"}')"#),
            serde_json::json!([{ "do": "show", "stack": "author", "card": "books" }])
        );
        assert_eq!(
            json(r#"SELECT xila.act('author', 'browse', '{"card": "books"}')"#),
            serde_json::json!([{ "do": "show", "stack": "books" }])
        );

        Spi::run(
            "GRANT USAGE ON SCHEMA books TO xila_user;
             GRANT SELECT ON books.cards TO xila_user;",
        )
        .unwrap();
        become_role("bob");
        let screen = json("SELECT xila.open_stack('books')");
        assert_eq!(screen["stack"]["may_edit"], false);
        assert!(
            keys_of(&screen).iter().all(|key| !key.starts_with("F11")),
            "author mode is not offered on someone else's stack"
        );
        let (state, _) = error_of("SELECT xila.act('books', 'author')");
        assert_eq!(state, "XL004");
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
