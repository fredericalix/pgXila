//! In-database tests of scripts.

use pgrx::prelude::*;

#[pg_schema]
mod tests {
    use pgrx::prelude::*;
    use serde_json::{json, Value as Json};

    use crate::tests::{
        become_admin, become_role, create_books_as_ada, error_of, json, number, text, widget_of,
    };

    /// Text given to SQL as it is, whatever quotes it holds.
    fn quoted(text: &str) -> String {
        format!("$lua${text}$lua$")
    }

    /// Runs one event of a script on the books of Ada, and gives the answer
    /// of the engine.
    fn play(source: &str, event: Json, context: Json) -> Json {
        json(&format!(
            "SELECT xila.run_script('books', {}, '{event}', {})",
            quoted(source),
            quoted(&context.to_string())
        ))
    }

    /// The effects of a click on a button that has this script.
    fn click(source: &str, context: Json) -> Json {
        let played = play(source, json!({"on": "click"}), context);
        assert_eq!(played.get("error"), None, "{played}");
        played["effects"].clone()
    }

    /// What goes wrong when a button that has this script is clicked.
    fn failure(source: &str) -> Json {
        play(source, json!({"on": "click"}), json!({}))["error"].clone()
    }

    /// Adds a button to the books and gives it a script.
    fn button_with(source: &str) -> (String, Json) {
        let name = text("SELECT xila.add_widget('books', 'button', '{\"label\": \"Stamp\"}')")
            .expect("the name of the button");
        let saved = json(&format!("SELECT xila.set_script('books', '{name}', {})", quoted(source)));
        (name, saved)
    }

    /// The key of a book.
    fn book(title: &str) -> String {
        text(&format!("SELECT id::text FROM books.cards WHERE title = '{title}'"))
            .expect("a book of that title")
    }

    fn books() -> i64 {
        number("SELECT count(*) FROM books.cards")
    }

    const NEW_BOOK: &str = r#"
keys = { F6 = "New book" }

function on_click()
  local new = stack.new_card()
  new.title = "Untitled"
  new.year = 2000
  go_to(new)
  focus("title")
end

function on_key(key)
  if key == "F6" then on_click() end
end
"#;

    #[pg_test]
    fn a_script_is_checked_without_being_run() {
        assert_eq!(
            json(&format!("SELECT xila.check_script({})", quoted(NEW_BOOK))),
            json!({"ok": true, "events": ["click", "key"], "keys": {"F6": "New book"}})
        );
        assert_eq!(
            json("SELECT xila.check_script('-- nothing yet')"),
            json!({"ok": true, "events": [], "keys": {}})
        );

        let wrong = json("SELECT xila.check_script(E'function on_click()\n  local = 3\nend')");
        assert_eq!(wrong["ok"], json!(false));
        assert_eq!(wrong["line"], json!(2));

        // The top level of a script defines things; it does not act.
        let acting =
            json("SELECT xila.check_script(E'\n\nquery(\"SELECT 1\")\nfunction on_click() end')");
        assert_eq!(acting["ok"], json!(false), "{acting}");
        assert_eq!(acting["line"], json!(3));
        assert!(acting["message"].as_str().unwrap().contains("only be used inside a function"));

        // Compiled Lua is never loaded.
        let compiled = json(r"SELECT xila.check_script(E'\x1bLua')");
        assert_eq!(compiled["ok"], json!(false), "{compiled}");
    }

    #[pg_test]
    fn a_script_says_which_keys_it_wants() {
        for (source, problem) in [
            ("keys = { F3 = 'Leave' }\nfunction on_key() end", "F3, F11 and F12 cannot be taken"),
            ("keys = { F12 = 'Stay' }\nfunction on_key() end", "F3, F11 and F12 cannot be taken"),
            ("keys = { F6 = 'New' }", "there is no function on_key"),
            ("keys = { F25 = 'New' }\nfunction on_key() end", "is not a key"),
            ("keys = { F6 = '' }\nfunction on_key() end", "needs a label"),
            ("keys = 'F6'\nfunction on_key() end", "“keys” is a table"),
        ] {
            let checked = json(&format!("SELECT xila.check_script({})", quoted(source)));
            assert_eq!(checked["ok"], json!(false), "{source}");
            let message = checked["message"].as_str().unwrap();
            assert!(message.contains(problem), "{source}: {message}");
        }
    }

    #[pg_test]
    fn a_script_sees_nothing_but_pgxila() {
        let source = r#"
function on_click()
  local found = {}
  for _, name in ipairs({"io", "os", "package", "require", "debug", "dofile", "loadfile",
                         "load", "collectgarbage", "getmetatable", "setmetatable", "rawset",
                         "_G", "host", "coroutine"}) do
    if _ENV[name] ~= nil then found[#found + 1] = name end
  end
  if string.dump ~= nil then found[#found + 1] = "string.dump" end
  message(table.concat(found, ","))
end"#;
        create_books_as_ada();
        assert_eq!(click(source, json!({})), json!([{"do": "message", "text": ""}]));
    }

    #[pg_test]
    fn a_button_runs_its_script() {
        create_books_as_ada();
        let (button, saved) = button_with(NEW_BOOK);
        assert_eq!(saved["ok"], json!(true), "{saved}");

        // The screen says so: the button names the action of all scripts, and
        // the key of the script takes the place of the one of the screen.
        let screen = json("SELECT xila.open_stack('books')");
        assert_eq!(widget_of(&screen, &button)["action"], json!("script"));
        let f6 = screen["keys"].as_array().unwrap().iter().find(|k| k["key"] == "F6").unwrap();
        assert_eq!(*f6, json!({"key": "F6", "label": "New book", "action": "script"}));
        assert_eq!(screen["events"], json!([]));

        let expected = json!([
            {"do": "new", "values": {"title": "Untitled", "year": "2000"}},
            {"do": "focus", "widget": "title"},
        ]);
        let clicked = json(&format!(
            r#"SELECT xila.act('books', 'script', '{{"event": {{"on": "click", "widget": "{button}"}}}}')"#
        ));
        assert_eq!(clicked, expected);
        let pressed =
            json(r#"SELECT xila.act('books', 'script', '{"event": {"on": "key", "key": "F6"}}')"#);
        assert_eq!(pressed, expected);

        // Nothing was saved: the new card is for the user to complete.
        assert_eq!(books(), 2);

        let (code, _) =
            error_of(r#"xila.act('books', 'script', '{"event": {"on": "key", "key": "F9"}}')"#);
        assert_eq!(code, "XL002");
    }

    #[pg_test]
    fn a_script_reads_and_writes_the_card_on_screen() {
        create_books_as_ada();
        let source = r#"
function on_click()
  card.year = card.year + 1
  message(card.title .. ", by " .. user .. ", in " .. stack.title)
end"#;
        // What the user typed and has not saved is what the script reads.
        let context = json!({"card": book("Candide"), "dirty": {"title": "Candide, or Optimism"}});
        assert_eq!(
            click(source, context),
            json!([
                {"do": "set", "values": {"year": "1760"}},
                {"do": "message", "text": "Candide, or Optimism, by ada, in Books"},
            ])
        );
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1759);

        // A value that is emptied is written as nothing.
        let emptied =
            click("function on_click() card.year = nil end", json!({"card": book("Candide")}));
        assert_eq!(emptied, json!([{"do": "set", "values": {"year": null}}]));

        // On a card that is not saved yet, there is what was typed.
        let typed = click(
            "function on_click() message(card.title .. ' ' .. tostring(card.year)) end",
            json!({"card": null, "dirty": {"title": "Zadig", "year": "1747"}}),
        );
        assert_eq!(typed, json!([{"do": "message", "text": "Zadig 1747"}]));
    }

    #[pg_test]
    fn a_script_finds_saves_and_deletes_cards() {
        create_books_as_ada();
        let source = r#"
function on_click()
  local first = stack.cards{year = 1759}[1]
  local sequel = stack.new_card()
  sequel.title = first.title .. " II"
  sequel.year = first.year + 1
  save(sequel)
  for _, found in ipairs(stack.cards{title = "Germinal"}) do delete(found) end
  card.title = "Candide, or Optimism"
  save(card)
  local titles = {}
  for i, each in ipairs(stack.cards()) do titles[i] = each.title end
  message(table.concat(titles, "; "))
end"#;
        let candide = book("Candide");
        assert_eq!(
            click(source, json!({"card": candide})),
            json!([
                {"do": "show", "stack": "books", "card": candide},
                {"do": "message", "text": "Candide, or Optimism; Candide II"},
            ])
        );
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide II'"), 1760);
        assert_eq!(books(), 2);
        // The changes are those of who clicked, and are in the history.
        assert_eq!(
            number(
                "SELECT count(*) FROM xila.history
                 WHERE object_kind = 'card' AND actor = 'ada' AND op = 'delete'"
            ),
            1
        );
    }

    #[pg_test]
    fn a_script_reaches_the_database_through_sql() {
        create_books_as_ada();
        // Parameters take the type the statement gives them: no cast is needed
        // for the uuid, the number or the missing value.
        let source = r#"
function on_click()
  local rows = query("SELECT title, year FROM books.cards WHERE id = $1 AND year > $2",
                     card.id, 1700)
  local changed = execute("UPDATE books.cards SET year = $1 WHERE id <> $2", nil, card.id)
  local left = query("SELECT count(*) AS n FROM books.cards WHERE year IS NULL")[1].n
  message(rows[1].title .. " " .. rows[1].year .. ", " .. changed .. " changed, " .. left
          .. " without a year, on " .. #today() .. " characters")
end"#;
        assert_eq!(
            click(source, json!({"card": book("Candide")})),
            json!([{"do": "message",
                    "text": "Candide 1759, 1 changed, 1 without a year, on 10 characters"}])
        );
    }

    #[pg_test]
    fn a_script_only_reads_and_writes_rows() {
        create_books_as_ada();
        let source = r#"
function on_click()
  local refused = {}
  for _, attempt in ipairs({
    {execute, "DROP TABLE books.cards"},
    {execute, "GRANT ALL ON books.cards TO PUBLIC"},
    {execute, "SET ROLE postgres"},
    {execute, "SELECT 1"},
    {query, "SELECT 1; SELECT 2"},
    {query, "SELECT 1 AS n INTO books.copy"},
    {query, "DELETE FROM books.cards RETURNING id"},
    {query, "SELECT $1 + 1 AS n"},
  }) do
    local ok, problem = pcall(attempt[1], attempt[2])
    refused[#refused + 1] = ok and "ran" or "refused"
  end
  message(table.concat(refused, " "))
end"#;
        assert_eq!(
            click(source, json!({})),
            json!([{"do": "message",
                    "text": "refused refused refused refused refused refused refused refused"}])
        );
        assert_eq!(books(), 2);
    }

    #[pg_test]
    fn a_script_that_fails_leaves_nothing_behind() {
        create_books_as_ada();
        let (button, _) = button_with(
            r#"
function on_click()
  local draft = stack.new_card()
  draft.title = "Draft"
  save(draft)
  error("no more paper")
end"#,
        );
        let (code, message) = error_of(&format!(
            r#"xila.act('books', 'script', '{{"event": {{"on": "click", "widget": "{button}"}}}}')"#
        ));
        assert_eq!(code, "XL005");
        assert_eq!(message, "The script of “Stamp” failed at line 6: no more paper");
        assert_eq!(books(), 2);

        // A mistake of the language is told the same way.
        let problem = failure("function on_click()\n  local n = nothing.at_all\nend");
        assert_eq!(problem["line"], json!(2), "{problem}");
        assert!(problem["message"].as_str().unwrap().contains("nil value"), "{problem}");
    }

    #[pg_test]
    fn a_script_catches_the_error_of_a_statement_and_goes_on() {
        create_books_as_ada();
        let source = r#"
function on_click()
  execute("INSERT INTO books.cards (title) VALUES ('Before')")
  local ok, problem = pcall(execute, "INSERT INTO books.cards (id) SELECT id FROM books.cards")
  execute("INSERT INTO books.cards (title) VALUES ('After')")
  message(tostring(ok) .. ": " .. problem)
end"#;
        let effects = click(source, json!({}));
        let said = effects[0]["text"].as_str().unwrap();
        assert!(said.starts_with("false: ") && said.contains("duplicate key"), "{said}");
        assert_eq!(books(), 4);
    }

    #[pg_test]
    fn a_script_cannot_run_forever() {
        create_books_as_ada();
        for source in [
            "function on_click() while true do end end",
            // Nor by catching its own limit.
            "function on_click() while true do pcall(function() while true do end end) end end",
        ] {
            let problem = failure(source);
            assert_eq!(problem["message"], json!("The script ran for too long and was stopped."));
        }
        // Reading a script is limited the same way.
        let checked = json("SELECT xila.check_script('while true do end')");
        assert_eq!(checked["message"], json!("The script ran for too long and was stopped."));
    }

    #[pg_test]
    fn a_script_cannot_hold_too_much_memory() {
        create_books_as_ada();
        let problem = failure(
            r#"
function on_click()
  local held = {}
  for i = 1, 1000 do held[i] = string.rep("x", 1000000) .. i end
end"#,
        );
        assert_eq!(problem["message"], json!("The script used too much memory and was stopped."));
    }

    #[pg_test]
    fn a_script_asks_before_it_acts() {
        create_books_as_ada();
        let source = r#"
function on_click()
  execute("INSERT INTO books.cards (title) VALUES ('Trace')")
  if confirm("Delete this book?", "Delete") then
    delete(card)
    message("Gone.")
  end
end"#;
        let candide = book("Candide");
        let germinal = book("Germinal");

        // The question first: nothing the script did before it is kept, and
        // the script cannot catch it.
        let asked = play(source, json!({"on": "click"}), json!({"card": candide}));
        assert_eq!(
            asked["effects"],
            json!([{"do": "confirm", "text": "Delete this book?", "label": "Delete"}])
        );
        assert_eq!(books(), 2);
        let caught = play(
            "function on_click() pcall(confirm, 'Sure?') message('went on') end",
            json!({"on": "click"}),
            json!({}),
        );
        assert_eq!(caught["effects"], json!([{"do": "confirm", "text": "Sure?", "label": "OK"}]));

        // Then, once accepted, the whole script.
        let done = click(source, json!({"card": candide, "confirmed": true}));
        assert_eq!(
            done,
            json!([
                {"do": "show", "stack": "books", "card": germinal},
                {"do": "message", "text": "Gone."},
            ])
        );
        assert_eq!(number("SELECT count(*) FROM books.cards WHERE title = 'Candide'"), 0);
        assert_eq!(books(), 2);
    }

    #[pg_test]
    fn a_script_runs_the_actions_of_the_screen() {
        create_books_as_ada();
        let candide = book("Candide");
        let germinal = book("Germinal");
        // Moving to another card saves what was typed, and what the script
        // wrote, as the key of the screen does.
        let source = r#"
function on_click()
  card.year = 1760
  go_to("next")
  message("Next.")
end"#;
        assert_eq!(
            click(source, json!({"card": candide, "dirty": {"title": "Candide!"}})),
            json!([
                {"do": "show", "stack": "books", "card": germinal},
                {"do": "message", "text": "Next."},
            ])
        );
        assert_eq!(
            text(&format!(
                "SELECT title || ' ' || year FROM books.cards WHERE id::text = '{candide}'"
            )),
            Some("Candide! 1760".to_owned())
        );

        // An action that asks is the question of the script.
        let asked = play(
            "function on_click() act('delete') end",
            json!({"on": "click"}),
            json!({"card": candide}),
        );
        assert_eq!(asked["effects"][0]["do"], json!("confirm"), "{asked}");
        assert_eq!(books(), 2);

        let problem = failure("function on_click() act('script') end");
        assert!(problem["message"].as_str().unwrap().contains("cannot run the action"));
    }

    #[pg_test]
    fn scripts_do_not_run_for_a_superuser() {
        create_books_as_ada();
        let (button, _) = button_with("function on_click() message('Hello') end");
        become_admin();
        let (code, message) = error_of(&format!(
            r#"xila.act('books', 'script', '{{"event": {{"on": "click", "widget": "{button}"}}}}')"#
        ));
        assert_eq!(code, "XL004");
        assert!(message.contains("Scripts do not run for a superuser"), "{message}");
    }

    #[pg_test]
    fn a_script_has_the_rights_of_who_triggers_it() {
        create_books_as_ada();
        let (button, _) = button_with(
            r#"
function on_click()
  local ok, problem = pcall(execute, "UPDATE books.cards SET year = 0")
  local saved, why = pcall(function() card.year = 1; save(card) end)
  message(tostring(ok) .. "/" .. tostring(saved) .. ": " .. problem .. " | " .. why)
end"#,
        );
        Spi::run("GRANT USAGE ON SCHEMA books TO dan; GRANT SELECT ON books.cards TO dan").unwrap();
        let candide = book("Candide");

        become_admin();
        become_role("dan");
        // Dan may read the script he is about to run, and not change it.
        assert_eq!(number("SELECT count(*) FROM xila.script"), 1);
        let (code, _) = error_of(&format!("xila.set_script('books', '{button}', '-- mine now')"));
        assert_eq!(code, "XL004");

        let effects = json(&format!(
            r#"SELECT xila.act('books', 'script',
                   '{{"card": "{candide}", "event": {{"on": "click", "widget": "{button}"}}}}')"#
        ));
        // What the script wrote on the card and could not save is still typed.
        assert_eq!(effects[0], json!({"do": "set", "values": {"year": "1"}}));
        let said = effects[1]["text"].as_str().unwrap();
        assert!(said.starts_with("false/false: "), "{said}");
        assert!(said.contains("permission denied"), "{said}");
        assert!(said.contains("You are not allowed to change this stack."), "{said}");
        become_admin();
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1759);
    }

    #[pg_test]
    fn each_event_has_its_place() {
        create_books_as_ada();
        // The screen has a script too: it is told when a card comes on screen.
        let saved = json(
            "SELECT xila.set_script('books', NULL,
                 E'function on_open()\n  message(\"Welcome to \" .. card.title)\nend')",
        );
        assert_eq!(saved["events"], json!(["open"]), "{saved}");
        assert_eq!(json("SELECT xila.open_stack('books')")["events"], json!(["open"]));
        let opened = json(&format!(
            r#"SELECT xila.act('books', 'script', '{{"card": "{}", "event": {{"on": "open"}}}}')"#,
            book("Candide")
        ));
        assert_eq!(opened, json!([{"do": "message", "text": "Welcome to Candide"}]));

        // A script in the wrong place is kept, and handles nothing.
        let misplaced = json("SELECT xila.set_script('books', NULL, 'function on_click() end')");
        assert_eq!(misplaced["ok"], json!(false));
        assert!(misplaced["message"]
            .as_str()
            .unwrap()
            .contains("belongs to the script of a button"));
        assert_eq!(json("SELECT xila.open_stack('books')")["events"], json!([]));
        assert_eq!(
            text("SELECT source FROM xila.script").as_deref(),
            Some("function on_click() end")
        );

        let (first, _) = button_with("keys = { F9 = 'Stamp' }\nfunction on_key() end");
        let (_, second) = button_with("keys = { F9 = 'Again' }\nfunction on_key() end");
        assert_eq!(second["ok"], json!(false));
        assert_eq!(second["message"], json!("F9 already belongs to the script of “Stamp”."));
        let (_, opening) = button_with("function on_open() end");
        assert!(opening["message"]
            .as_str()
            .unwrap()
            .contains("belongs to the script of the screen"));

        // Only a button, or the screen, has a script; and an empty one is no
        // script at all.
        let (code, _) = error_of("xila.set_script('books', 'title', '-- no')");
        assert_eq!(code, "XL003");
        let removed = json(&format!("SELECT xila.set_script('books', '{first}', E'  \n')"));
        assert_eq!(removed["removed"], json!(true));
        assert_eq!(number("SELECT count(*) FROM xila.script s WHERE s.keys ? 'F9'"), 0);
    }

    #[pg_test]
    fn a_script_with_an_error_is_kept_and_handles_nothing() {
        create_books_as_ada();
        let (button, saved) = button_with("function on_click()\n  message('unfinished'\nend");
        assert_eq!(saved["ok"], json!(false));
        assert_eq!(saved["line"], json!(3), "{saved}");
        assert_eq!(number("SELECT count(*) FROM xila.script WHERE events = '{}'"), 1);
        let screen = json("SELECT xila.open_stack('books')");
        assert_eq!(widget_of(&screen, &button).get("action"), None);
    }

    #[pg_test]
    fn a_script_is_part_of_the_history_of_the_screen() {
        create_books_as_ada();
        let (button, _) = button_with("function on_click() message('Hello') end");
        assert_eq!(
            text(
                "SELECT xila.describe_change(h.object_kind, h.op, h.before, h.after, 'Stamp')
                 FROM xila.history h WHERE h.object_kind = 'script'"
            )
            .as_deref(),
            Some("Script of “Stamp” written")
        );
        // Undoing the change takes the script away, and the button with it
        // goes back to doing nothing.
        Spi::run("SELECT xila.undo_change(h) FROM xila.history h WHERE h.object_kind = 'script'")
            .unwrap();
        assert_eq!(number("SELECT count(*) FROM xila.script"), 0);
        let screen = json("SELECT xila.open_stack('books')");
        assert_eq!(widget_of(&screen, &button).get("action"), None);
    }

    #[pg_test]
    fn a_test_shows_what_a_script_would_do_and_keeps_nothing() {
        create_books_as_ada();
        let source = r#"
function on_click()
  print("starting", 1)
  local draft = stack.new_card()
  draft.title = "Draft"
  save(draft)
  execute("UPDATE books.cards SET year = 1")
  if confirm("Keep going?", "Go") then
    card.title = "Changed"
    focus("title")
    message("Done.")
  end
end"#;
        let context = json!({
            "card": book("Candide"), "renderer": "graphic",
            "input": {"source": source, "event": {"on": "click"}},
        });
        let tested = json(&format!(
            "SELECT xila.act('books', 'test_script', {})",
            quoted(&context.to_string())
        ));
        assert_eq!(
            tested,
            json!([{
                "do": "tested", "ok": true, "line": null,
                "lines": [
                    "It asks: Keep going? (Go)",
                    "Once accepted:",
                    "print: starting\t1",
                    "In the database: 1 card saved, 3 rows written.",
                    "It writes title = Changed on the card.",
                    "It puts the cursor in “title”.",
                    "It says: Done.",
                    "This was a test: nothing was kept.",
                ],
            }])
        );
        assert_eq!(books(), 2);
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1759);

        let broken = json!({
            "input": {"source": "function on_click()\n  nothing()\nend", "event": {"on": "click"}},
        });
        let tested = json(&format!(
            "SELECT xila.act('books', 'test_script', {})",
            quoted(&broken.to_string())
        ));
        assert_eq!(tested[0]["ok"], json!(false));
        assert_eq!(tested[0]["line"], json!(2));
        assert!(tested[0]["lines"][0].as_str().unwrap().starts_with("Line 2: "), "{tested}");

        // A key event is tried with the first key the script asks for.
        let keyed = json!({
            "input": {
                "source": "keys = { F9 = 'Nine', F10 = 'Ten' }\nfunction on_key(key) message(key) end",
                "event": {"on": "key"},
            },
        });
        let tested = json(&format!(
            "SELECT xila.act('books', 'test_script', {})",
            quoted(&keyed.to_string())
        ));
        assert_eq!(tested[0]["lines"], json!(["It says: F9"]), "{tested}");
    }

    #[pg_test]
    fn the_script_editor_opens_saves_and_removes_a_script() {
        create_books_as_ada();
        let button =
            text("SELECT xila.add_widget('books', 'button', '{\"label\": \"Stamp\"}')").unwrap();
        let edit = |widget: Json| {
            json(&format!(
                "SELECT xila.act('books', 'edit_script', {})",
                quoted(&json!({"renderer": "graphic", "input": {"widget": widget}}).to_string())
            ))
        };
        let save = |widget: Json, source: &str| {
            json(&format!(
                "SELECT xila.act('books', 'save_script', {})",
                quoted(&json!({"input": {"widget": widget, "source": source}}).to_string())
            ))
        };

        assert_eq!(
            edit(json!(button)),
            json!([{"do": "script", "widget": button, "title": "Script: button “Stamp”",
                    "source": ""}])
        );
        assert_eq!(
            edit(Json::Null),
            json!([{"do": "script", "title": "Script: screen of “Books”", "source": ""}])
        );

        let source = "function on_click() message('Hello') end";
        assert_eq!(
            save(json!(button), source),
            json!([{"do": "reload"}, {"do": "message", "text": "Script saved."}])
        );
        assert_eq!(edit(json!(button))[0]["source"], json!(source));
        assert_eq!(
            save(json!(button), "function on_click(")[1]["text"]
                .as_str()
                .map(|said| said.starts_with("Script saved with an error at line 1: ")),
            Some(true)
        );
        assert_eq!(save(json!(button), "")[1]["text"], json!("Script removed."));
        assert_eq!(number("SELECT count(*) FROM xila.script"), 0);

        // A text client is told where scripts are written.
        let told = json("SELECT xila.act('books', 'edit_script', '{\"renderer\": \"text\"}')");
        assert_eq!(told[0]["do"], json!("message"));
    }

    #[pg_test]
    fn the_texts_and_lists_of_lua_work_as_in_lua() {
        create_books_as_ada();
        // These functions are not those of the library of Lua: they are
        // counted. They answer the same.
        let source = r#"
function on_click()
  local out = {}
  local function say(...) out[#out + 1] = table.concat({...}, ",") end
  say(("hello world"):find("o w"))
  say(("hello world"):find("(o)(%s)"))
  say(string.find("a.b", ".", 1, true))
  say(("key = value"):match("^(%w+)%s*=%s*(%w+)$"))
  say(("  padded  "):match("^%s*(.-)%s*$"))
  say(("2026-11-26"):match("()-"))
  local words = {}
  for word in ("one two  three"):gmatch("%a+") do words[#words + 1] = word end
  say(#words, words[3])
  local found = {}
  for name, value in ("a=1, b=2"):gmatch("(%w+)=(%w+)") do found[#found + 1] = name .. value end
  say(table.concat(found, "/"))
  say(("hello world"):gsub("o", "0"))
  say(("hello world"):gsub("(%w+)", "<%1>"))
  say(("hello"):gsub("", "-"))
  say(("abc"):gsub("%w", "%0%0", 2))
  say(("$name is $age"):gsub("%$(%w+)", {name = "Ada", age = 36}))
  say(("abc"):gsub("%w", function(c) if c ~= "b" then return c:upper() end end))
  say(("hello"):gsub("^h", "J"))
  say(("x"):rep(3, "-"), ("ab"):rep(2), #(""):rep(math.maxinteger))
  local list = {1, 2, 3}
  table.insert(list, 4)
  table.insert(list, 1, 0)
  say(table.concat(list, ""))
  say(table.remove(list), table.remove(list, 1), table.concat(list, ""))
  say(tostring(("abc"):find("z")), tostring(("abc"):match("z")))
  say(select(2, pcall(string.find, "a", "[a")))
  say(tostring(string.pack), tostring(table.move), tostring(string.dump))
  message(table.concat(out, " | "))
end"#;
        let expected = [
            "5,7",
            "5,6,o, ",
            "2,2",
            "key,value",
            "padded",
            "5",
            "3,three",
            "a1/b2",
            "hell0 w0rld,2",
            "<hello> <world>,2",
            "-h-e-l-l-o-,6",
            "aabbc,2",
            "Ada is 36,2",
            "AbC,3",
            "Jello,1",
            "x-x-x,abab,0",
            "01234",
            "4,0,123",
            "nil,nil",
            "malformed pattern (missing ']')",
            "nil,nil,nil",
        ];
        assert_eq!(
            click(source, json!({})),
            json!([{"do": "message", "text": expected.join(" | ")}])
        );
    }

    #[pg_test]
    fn a_script_cannot_wait_inside_the_library() {
        create_books_as_ada();
        // A pattern that tries every way to cut a long text is counted like
        // anything else a script does.
        let problem =
            failure(r#"function on_click() return (("a"):rep(5000)):find(".-.-.-.-.-b") end"#);
        assert_eq!(problem["message"], json!("The script ran for too long and was stopped."));
        let problem =
            failure(r#"function on_click() return (("a"):rep(5000)):gsub(".-.-.-.-.-b", "") end"#);
        assert_eq!(problem["message"], json!("The script ran for too long and was stopped."));

        // A list made to look immensely long: shifting its items ends, one
        // way or the other.
        let played = play(
            r#"
function on_click()
  local list = {}
  for i = 1, 9 do list[i] = true end
  local i = 16
  while i > 0 do list[i] = true i = i * 2 end
  table.insert(list, 1, 0)
  table.remove(list, 1)
  message("done")
end"#,
            json!({"on": "click"}),
            json!({}),
        );
        match played.get("error") {
            Some(problem) => assert_eq!(
                problem["message"],
                json!("The script ran for too long and was stopped.")
            ),
            None => assert_eq!(played["effects"], json!([{"do": "message", "text": "done"}])),
        }
    }

    #[pg_test]
    fn a_value_nested_too_deeply_is_refused() {
        create_books_as_ada();
        for source in [
            // A hundred thousand tables, one inside the other.
            r#"function on_click()
                 local deep = {}
                 for i = 1, 100000 do deep = {deep} end
                 query("SELECT $1::jsonb AS x", deep)
               end"#,
            // A table that holds itself.
            r#"function on_click()
                 local loop = {}
                 loop.again = loop
                 query("SELECT $1::jsonb AS x", loop)
               end"#,
        ] {
            let problem = failure(source);
            assert_eq!(
                problem["message"],
                json!("This value is nested too deeply, or holds too much, to be stored."),
                "{problem}"
            );
        }
        // What is nested within reason is JSON.
        let stored = click(
            r#"function on_click()
                 local rows = query("SELECT $1::jsonb -> 'a' -> 1 AS x", {a = {10, 20}})
                 message(rows[1].x)
               end"#,
            json!({}),
        );
        assert_eq!(stored, json!([{"do": "message", "text": "20"}]));
    }

    #[pg_test]
    fn a_script_cannot_run_another_script() {
        create_books_as_ada();
        let source = r#"
function on_click()
  local rows = query([[
    SELECT xila.run_script('books', 'function on_click() message("inner") end',
                           '{"on": "click"}', '{}') AS played]])
  message(rows[1].played.error.message)
end"#;
        let expected = json!([{"do": "message", "text": "A script cannot run another script."}]);
        assert_eq!(click(source, json!({})), expected);
        // Its turn is handed back when it ends, whether it ends well or not.
        assert_eq!(click(source, json!({})), expected);
        failure("function on_click() error('no') end");
        assert_eq!(click(source, json!({})), expected);
    }

    #[pg_test]
    fn a_query_cannot_bring_back_more_than_a_script_may_hold() {
        create_books_as_ada();
        let source = r#"
function on_click()
  local ok, problem = pcall(query, "SELECT repeat('x', 5000000) AS big")
  local rows = query("SELECT repeat('x', 1000) AS small")
  message(tostring(ok) .. ": " .. problem .. " " .. #rows[1].small)
end"#;
        assert_eq!(
            click(source, json!({})),
            json!([{"do": "message",
                    "text": "false: A query gives 4 MB at most. This one gives more. 1000"}])
        );
    }
}
