//! In-database tests of the agent: its rights, its tools, and what the
//! worker asks the database to do. The worker itself and the model are not
//! here: a pgrx test is one transaction in one backend. The integration
//! tests run them, against a fake model.

use pgrx::prelude::*;

#[pg_schema]
mod tests {
    use pgrx::prelude::*;
    use serde_json::{json, Value as Json};

    use crate::tests::{
        become_admin, become_role, create_books_as_ada, error_of, error_of_statement, json, number,
        text, widget_of,
    };

    /// Text given to SQL as it is, whatever quotes it holds.
    fn quoted(text: &str) -> String {
        format!("$said${text}$said$")
    }

    /// Ada's books, an administrator (eve), and a model set up by her.
    fn set_up() {
        create_books_as_ada();
        become_admin();
        Spi::run(
            r#"CREATE ROLE eve LOGIN IN ROLE xila_admin;
               SET LOCAL SESSION AUTHORIZATION eve;
               SELECT xila.set_agent('{"address": "https://models.example/v1",
                                       "model": "test-model",
                                       "token_variable": "TEST_TOKEN"}');"#,
        )
        .unwrap();
        become_role("ada");
    }

    /// Runs a tool as the worker, for someone.
    fn tool(name: &str, as_role: &str, arguments: Json) -> Json {
        become_role("xila_agent");
        let result = json(&format!(
            "SELECT xila.tool_{name}('{as_role}', {})",
            quoted(&arguments.to_string())
        ));
        become_role("ada");
        result
    }

    /// What a tool refuses with, as the worker, for someone.
    fn tool_error(name: &str, as_role: &str, arguments: Json) -> (String, String) {
        become_role("xila_agent");
        let error =
            error_of(&format!("xila.tool_{name}('{as_role}', {})", quoted(&arguments.to_string())));
        become_role("ada");
        error
    }

    /// As ada: calls the agent from her books and asks a question. Gives the
    /// conversation.
    fn ask(question: &str) -> String {
        become_role("ada");
        let shown = json(r#"SELECT xila.act('books', 'agent', '{}')"#);
        let conversation = shown[0]["card"].as_str().expect("a conversation").to_owned();
        let context = json!({"card": conversation, "dirty": {"ask": question}});
        Spi::run(&format!("SELECT xila.act('agent', 'send', {})", quoted(&context.to_string())))
            .unwrap();
        conversation
    }

    /// As the worker: what to send for a conversation.
    fn request(conversation: &str) -> Option<Json> {
        become_role("xila_agent");
        let asked =
            Spi::get_one::<pgrx::JsonB>(&format!("SELECT xila.agent_request('{conversation}')"))
                .unwrap()
                .map(|found| found.0);
        become_role("ada");
        asked
    }

    /// As the worker: the answer of the model to a request.
    fn answer(conversation: &str, asked: &Json, status: i32, body: &str) {
        become_role("xila_agent");
        Spi::run(&format!(
            "SELECT xila.agent_answer('{conversation}', '{}', {status}, {}, 1100)",
            asked["upto"].as_str().unwrap(),
            quoted(body)
        ))
        .unwrap();
        become_role("ada");
    }

    /// An answer of the model, in words.
    fn says(words: &str) -> String {
        json!({
            "model": "test-model",
            "choices": [{"finish_reason": "stop",
                         "message": {"role": "assistant", "content": words, "refusal": null}}],
            "usage": {"prompt_tokens": 300, "completion_tokens": 54}
        })
        .to_string()
    }

    /// An answer of the model that asks for a tool.
    fn calls(name: &str, arguments: Json) -> String {
        json!({
            "choices": [{"finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{"id": "call_1", "type": "function", "index": 0,
                                "function": {"name": name,
                                             "arguments": arguments.to_string()}}]}}],
            "usage": {"prompt_tokens": 310, "completion_tokens": 20}
        })
        .to_string()
    }

    /// The lines of a conversation as its owner reads them, oldest first.
    fn lines(conversation: &str) -> Json {
        json(&format!(
            "SELECT coalesce(jsonb_agg(jsonb_build_array(l.kind, l.text) ORDER BY l.at, l.id), '[]')
             FROM xila.agent_line l WHERE l.conversation = '{conversation}'"
        ))
    }

    fn state_of(conversation: &str) -> String {
        text(&format!("SELECT state FROM xila.conversation WHERE id = '{conversation}'")).unwrap()
    }

    fn agent_may_read_books() -> bool {
        Spi::get_one::<bool>("SELECT has_table_privilege('xila_agent', 'books.cards', 'SELECT')")
            .unwrap()
            .unwrap()
    }

    #[pg_test]
    fn a_stack_is_open_to_the_agent_until_someone_closes_it() {
        create_books_as_ada();
        assert!(agent_may_read_books());

        // Closing is a privilege taken away, not a flag.
        Spi::run(r#"SELECT xila.set_stack('books', '{"agent": false}')"#).unwrap();
        assert!(!agent_may_read_books());
        assert_eq!(
            text(
                "SELECT xila.describe_change(h.object_kind, h.op, h.before, h.after)
                 FROM xila.history h ORDER BY h.id DESC LIMIT 1"
            ),
            Some("Stack closed to the agent".to_owned())
        );
        assert_eq!(
            text("SELECT agent::text FROM xila.stack_info WHERE name = 'books'").unwrap(),
            "false"
        );

        // History brings it back like any other property.
        Spi::run("SELECT xila.rewind('books', 'screen', (SELECT max(id) FROM xila.history))")
            .unwrap();
        assert!(agent_may_read_books());

        // Someone who may not edit the stack cannot close it.
        become_role("bob");
        let (code, _) = error_of(r#"xila.set_stack('books', '{"agent": false}')"#);
        assert_eq!(code, "XL004");
        become_role("ada");

        // A stack that is deleted leaves a table that the agent no longer reads.
        Spi::run("SELECT xila.delete_stack('books')").unwrap();
        assert!(!agent_may_read_books());
    }

    #[pg_test]
    fn the_agent_reads_for_someone_and_never_more() {
        create_books_as_ada();

        let listed = tool("list_stacks", "ada", json!({}));
        assert_eq!(listed["stacks"][0]["name"], json!("books"), "{listed}");
        assert_eq!(listed["stacks"][0]["cards"], json!(2));
        assert_eq!(
            listed["stacks"][0]["fields"],
            json!([{"name": "title", "label": "Title", "type": "text"},
                   {"name": "year", "label": "Year", "type": "number"}])
        );
        assert_eq!(listed["summary"], json!("Listed the stacks: 1."));

        // Dan cannot open the books: for him they do not exist, whatever the
        // agent itself may read.
        assert_eq!(tool("list_stacks", "dan", json!({}))["stacks"], json!([]));
        let (code, message) = tool_error("find_cards", "dan", json!({"stack": "books"}));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("XL002", "The stack “books” does not exist.")
        );
        let (code, _) = tool_error("read_card", "dan", json!({"stack": "books", "position": 1}));
        assert_eq!(code, "XL002");

        // Once Dan may read them, the agent reads them for him too.
        Spi::run("GRANT USAGE ON SCHEMA books TO dan; GRANT SELECT ON books.cards TO dan").unwrap();
        assert_eq!(tool("find_cards", "dan", json!({"stack": "books"}))["total"], json!(2));

        // Closed: the agent knows the stack is there, and nothing else.
        Spi::run(r#"SELECT xila.set_stack('books', '{"agent": false}')"#).unwrap();
        assert_eq!(
            tool("list_stacks", "ada", json!({}))["stacks"],
            json!([{"name": "books", "title": "Books", "closed": true}])
        );
        let (code, message) = tool_error("find_cards", "ada", json!({"stack": "books"}));
        assert_eq!(code, "XL004");
        assert!(message.starts_with("The stack “Books” is closed to the agent."), "{message}");

        // The tools are for the worker: nobody else can run them for someone.
        let (code, _) = error_of(r#"xila.tool_list_stacks('ada', '{}')"#);
        assert_eq!(code, "42501");
        let (code, _) = error_of("xila.agent_work()");
        assert_eq!(code, "42501");
    }

    #[pg_test]
    fn the_agent_never_shows_a_row_that_the_table_hides() {
        create_books_as_ada();
        Spi::run(
            "ALTER TABLE books.cards ENABLE ROW LEVEL SECURITY;
             CREATE POLICY old ON books.cards FOR SELECT TO dan USING (year < 1800);
             CREATE POLICY agent ON books.cards FOR SELECT TO xila_agent USING (true);
             GRANT USAGE ON SCHEMA books TO dan; GRANT SELECT ON books.cards TO dan",
        )
        .unwrap();

        // Dan sees one book. The agent sees both, so it does not read for him.
        let (code, _) = tool_error("find_cards", "dan", json!({"stack": "books"}));
        assert_eq!(code, "XL004");
        assert_eq!(tool("list_stacks", "dan", json!({}))["stacks"][0]["closed"], json!(true));
        // Ada may edit the stack: nothing of it is hidden from her.
        assert_eq!(tool("find_cards", "ada", json!({"stack": "books"}))["total"], json!(2));
    }

    #[pg_test]
    fn the_tools_find_and_read_cards() {
        create_books_as_ada();
        Spi::run(
            r#"SELECT xila.add_field('books', 'Cover', 'picture');
               SELECT xila.save_card('books', NULL, '{"title": "Les Misérables", "year": "1862"}')"#,
        )
        .unwrap();

        let found = tool("find_cards", "ada", json!({"stack": "books", "text": "CANDIDE"}));
        assert_eq!(found["total"], json!(1), "{found}");
        assert_eq!(found["cards"][0]["title"], json!("Candide"));
        assert_eq!(found["cards"][0]["values"], json!({"title": "Candide", "year": "1759"}));
        assert_eq!(found["summary"], json!("Looked for “CANDIDE” in Books: 1 card."));

        // Every word must be there, in whatever field.
        assert_eq!(
            tool("find_cards", "ada", json!({"stack": "books", "text": "1862 mis"}))["total"],
            json!(1)
        );
        assert_eq!(
            tool("find_cards", "ada", json!({"stack": "books", "text": "1862 candide"}))["total"],
            json!(0)
        );
        // A text is looked for as it is written, not as a pattern.
        assert_eq!(
            tool("find_cards", "ada", json!({"stack": "books", "text": "%"}))["total"],
            json!(0)
        );

        // Without a text: the cards in the order of the stack, a few at a time.
        let page = tool("find_cards", "ada", json!({"stack": "books", "limit": 2, "offset": 1}));
        assert_eq!(page["total"], json!(3));
        assert_eq!(page["cards"].as_array().unwrap().len(), 2);
        assert_eq!(page["summary"], json!("Read cards 2 to 3 of Books (3)."));
        let many = tool("find_cards", "ada", json!({"stack": "books", "limit": 500}));
        assert_eq!(many["cards"].as_array().unwrap().len(), 3);

        // The title of a stack is accepted for its name.
        assert_eq!(tool("find_cards", "ada", json!({"stack": "Books"}))["total"], json!(3));

        let key = found["cards"][0]["key"].as_str().unwrap();
        let card = tool("read_card", "ada", json!({"stack": "books", "key": key}));
        assert_eq!(card["values"]["year"], json!("1759"));
        assert_eq!(card["summary"], json!("Read the card “Candide” of Books."));
        let second = tool("read_card", "ada", json!({"stack": "books", "position": 2}));
        assert_eq!(second["title"], json!("Germinal"));
        assert_eq!(second["count"], json!(3));

        let (code, message) =
            tool_error("read_card", "ada", json!({"stack": "books", "position": 9}));
        assert_eq!(code, "XL002");
        assert_eq!(message, "There is no such card in Books. The stack has 3 cards.");
        let (code, _) = tool_error("read_card", "ada", json!({"stack": "books"}));
        assert_eq!(code, "XL003");
        let (code, _) = tool_error("find_cards", "ada", json!({"stack": "books", "limit": "many"}));
        assert_eq!(code, "XL003");

        // A long text is cut, and a picture is only said to be there.
        Spi::run(&format!(
            "UPDATE books.cards SET title = repeat('a', 5000),
                    cover = xila.new_image(1, 1, '\\x00') WHERE id = '{key}'"
        ))
        .unwrap();
        let long = tool("read_card", "ada", json!({"stack": "books", "key": key}));
        assert_eq!(long["values"]["title"].as_str().unwrap().chars().count(), 2001);
        assert_eq!(long["values"]["cover"], json!("(picture)"));
    }

    #[pg_test]
    fn the_settings_are_for_administrators() {
        set_up();

        // An author is not an administrator.
        let (code, _) = error_of(r#"xila.set_agent('{"model": "mine"}')"#);
        assert_eq!(code, "XL004");
        let (code, _) = error_of("(SELECT address FROM xila.agent_setting)");
        assert_eq!(code, "42501");
        let (code, _) = error_of(r#"xila.act('conversations', 'settings', '{}')"#);
        assert_eq!(code, "XL004");
        // Everyone may know which model it is and where the questions go.
        assert_eq!(
            json("SELECT to_jsonb(i) FROM xila.agent_info i"),
            json!({"model": "test-model", "host": "models.example", "ready": true})
        );

        become_role("eve");
        for (changes, column) in [
            (r#"{"address": "http://models.example/v1"}"#, "address"),
            (r#"{"address": "http://192.168.1.20:1234/v1"}"#, "address"),
            (r#"{"address": "models.example"}"#, "address"),
            (r#"{"token_variable": "PGPASSWORD"}"#, "token_variable"),
            (r#"{"token_variable": "PG_COLOR"}"#, "token_variable"),
            (r#"{"token_variable": "POSTGRES_PASSWORD"}"#, "token_variable"),
            (r#"{"token_variable": "my token"}"#, "token_variable"),
            (r#"{"options": "[1]"}"#, "options"),
        ] {
            let (code, message) = error_of(&format!("xila.set_agent('{changes}')"));
            assert_eq!(code, "XL003", "{changes}: {message}");
            assert!(message.contains(column.split('_').next().unwrap()), "{changes}: {message}");
        }
        let (code, _) = error_of(r#"xila.set_agent('{"token": "secret"}')"#);
        assert_eq!(code, "XL003");

        // A variable of pgXila is not one of PostgreSQL.
        Spi::run(r#"SELECT xila.set_agent('{"token_variable": "PGXILA_AGENT_TOKEN"}')"#).unwrap();

        // A model on the machine itself may be called without TLS.
        Spi::run(
            r#"SELECT xila.set_agent('{"address": "http://localhost:1234/v1", "model": "local",
                                        "token_variable": "", "options": {"temperature": 0.2}}')"#,
        )
        .unwrap();
        assert_eq!(text("SELECT host FROM xila.agent_info").unwrap(), "localhost");
        // The table itself refuses what the function refuses.
        become_admin();
        let (code, _) = error_of_statement(
            "UPDATE xila.agent_setting SET address = 'http://models.example/v1'",
        );
        assert_eq!(code, "23514");

        // The dialog of the settings.
        become_role("eve");
        let shown = json(r#"SELECT xila.act('conversations', 'settings', '{}')"#);
        assert_eq!(shown, json!([{"do": "show", "stack": "agent_settings", "card": "agent"}]));
        assert_eq!(
            json("SELECT xila.card('agent_settings', 'agent', 'at')")["values"]["model"],
            json!("local")
        );
        let saved = json(
            r#"SELECT xila.act('agent_settings', 'save', '{"card": "agent", "dirty": {"model": "other"}}')"#,
        );
        assert_eq!(saved[1]["text"], json!("Agent settings saved."), "{saved}");
        assert_eq!(text("SELECT model FROM xila.agent_info").unwrap(), "other");
    }

    #[pg_test]
    fn asking_leaves_the_question_to_the_worker() {
        set_up();
        let card = text("SELECT id::text FROM books.cards ORDER BY title LIMIT 1").unwrap();

        // F9 on a card of the books: the conversation knows where from.
        let shown = json(&format!(r#"SELECT xila.act('books', 'agent', '{{"card": "{card}"}}')"#));
        assert_eq!(shown[0]["stack"], json!("agent"), "{shown}");
        let conversation = shown[0]["card"].as_str().unwrap().to_owned();
        let on_screen = json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"));
        assert_eq!(on_screen["values"]["title"], json!("New conversation"), "{on_screen}");
        assert_eq!(on_screen["values"]["about"], json!("Looking at Books"));
        assert_eq!(on_screen["values"]["model"], json!("test-model"));
        assert_eq!(on_screen["values"]["host"], json!("models.example"));
        assert_eq!(
            on_screen["values"]["tools"],
            json!(
                "List the stacks, Find cards, Read a card, Add a field, Change a field, \
                 Remove a field, Create a stack, Write a card, Delete a card"
            )
        );
        assert_eq!(on_screen["values"]["today"], json!("0 actions, 0 applied, 0 refused"));

        // Nothing to send.
        let (code, _) = error_of(&format!(
            r#"xila.act('agent', 'send', '{{"card": "{conversation}", "dirty": {{"ask": "  "}}}}')"#
        ));
        assert_eq!(code, "XL003");

        let asked = json(&format!(
            r#"SELECT xila.act('agent', 'send',
                   '{{"card": "{conversation}", "dirty": {{"ask": " Which books do I have? "}}}}')"#
        ));
        assert_eq!(asked, json!([{"do": "show", "stack": "agent", "card": conversation}]));
        assert_eq!(state_of(&conversation), "waiting");
        assert_eq!(
            lines(&conversation),
            json!([["user", "Which books do I have?"], ["wait", "The agent is working…"]])
        );
        assert_eq!(
            text(&format!("SELECT title FROM xila.conversation WHERE id = '{conversation}'"))
                .unwrap(),
            "Which books do I have?"
        );

        // One question at a time.
        let (code, message) = error_of(&format!(
            r#"xila.act('agent', 'send', '{{"card": "{conversation}", "dirty": {{"ask": "And?"}}}}')"#
        ));
        assert_eq!(code, "XL003");
        assert!(message.starts_with("The agent is still answering."), "{message}");

        // F5 gives the question up; what comes back for it later is dropped.
        let before = request(&conversation).expect("a request");
        let stopped =
            json(&format!(r#"SELECT xila.act('agent', 'stop', '{{"card": "{conversation}"}}')"#));
        assert_eq!(stopped[1]["text"], json!("Question given up."), "{stopped}");
        assert_eq!(state_of(&conversation), "idle");
        answer(&conversation, &before, 200, &says("Two."));
        assert_eq!(lines(&conversation).as_array().unwrap().len(), 2);

        // F9 from the list of conversations comes back to it as it was.
        let back = json(r#"SELECT xila.act('conversations', 'agent', '{}')"#);
        assert_eq!(back[0]["card"], json!(conversation));
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["about"],
            json!("Looking at Books")
        );

        // F9 again, from the desktop: the same conversation, about nothing.
        let again = json(r#"SELECT xila.act('desktop', 'agent', '{}')"#);
        assert_eq!(again[0]["card"], json!(conversation));
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["about"],
            json!("")
        );

        // F6 starts another one, and the list shows both.
        let fresh =
            json(&format!(r#"SELECT xila.act('agent', 'new', '{{"card": "{conversation}"}}')"#));
        assert_ne!(fresh[0]["card"], json!(conversation));
        assert_eq!(json(r#"SELECT xila.act('agent', 'new', '{}')"#)[0]["card"], fresh[0]["card"]);
        let listed = json("SELECT xila.grid_rows('conversations', 'list')");
        assert_eq!(listed["total"], json!(2), "{listed}");
        assert_eq!(listed["rows"][0]["cells"]["title"], json!("New conversation"));
        assert_eq!(listed["rows"][1]["cells"]["title"], json!("Which books do I have?"));
        let opened = json(&format!(
            r#"SELECT xila.act('conversations', 'open', '{{"row": "{conversation}"}}')"#
        ));
        assert_eq!(opened, json!([{"do": "show", "stack": "agent", "card": conversation}]));
    }

    #[pg_test]
    fn the_agent_cannot_be_asked_before_it_has_a_model() {
        create_books_as_ada();
        let shown = json(r#"SELECT xila.act('books', 'agent', '{}')"#);
        let conversation = shown[0]["card"].as_str().unwrap();
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["model"],
            json!("No model yet")
        );
        let (code, message) = error_of(&format!(
            r#"xila.act('agent', 'send', '{{"card": "{conversation}", "dirty": {{"ask": "Hello"}}}}')"#
        ));
        assert_eq!(code, "XL003");
        assert!(message.starts_with("The agent has no model yet."), "{message}");
    }

    #[pg_test]
    fn the_worker_sends_a_conversation_and_writes_the_answer() {
        set_up();
        let conversation = ask("Do I have Candide?");

        become_role("xila_agent");
        let work = json("SELECT jsonb_agg(w.conversation) FROM xila.agent_work() w");
        become_role("ada");
        assert_eq!(work, json!([conversation]));

        let first = request(&conversation).expect("a request");
        assert_eq!(first["url"], json!("https://models.example/v1/chat/completions"));
        assert_eq!(first["token_variable"], json!("TEST_TOKEN"));
        let body = &first["body"];
        assert_eq!(body["model"], json!("test-model"));
        assert_eq!(body["stream"], json!(false));
        assert_eq!(body["messages"].as_array().unwrap().len(), 2, "{body}");
        assert_eq!(body["messages"][0]["role"], json!("system"));
        let briefing = body["messages"][0]["content"].as_str().unwrap();
        assert!(briefing.contains("the person whose role is “ada”"), "{briefing}");
        assert!(briefing.contains("looking at the stack “Books” (name: books)"), "{briefing}");
        assert_eq!(body["messages"][1], json!({"role": "user", "content": "Do I have Candide?"}));
        let tools: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            tools,
            [
                "list_stacks",
                "find_cards",
                "read_card",
                "add_field",
                "change_field",
                "remove_field",
                "create_stack",
                "save_card",
                "delete_card"
            ]
        );
        assert_eq!(body["tools"][1]["function"]["parameters"]["required"], json!(["stack"]));

        // The model asks for a tool: the worker runs it, for Ada, and the
        // conversation waits for the next call.
        answer(
            &conversation,
            &first,
            200,
            &calls("find_cards", json!({"stack": "books", "text": "candide"})),
        );
        assert_eq!(state_of(&conversation), "waiting");
        assert_eq!(
            lines(&conversation),
            json!([
                ["user", "Do I have Candide?"],
                ["tool", "Looked for “candide” in Books: 1 card."],
                ["wait", "The agent is working…"]
            ])
        );

        // The next request carries what the model said and what the tool gave.
        let second = request(&conversation).expect("a second request");
        assert_ne!(second["upto"], first["upto"]);
        let messages = second["body"]["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 4, "{messages:?}");
        assert_eq!(messages[2]["role"], json!("assistant"));
        assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], json!("find_cards"));
        assert_eq!(messages[2].get("content"), None);
        assert_eq!(messages[3]["role"], json!("tool"));
        assert_eq!(messages[3]["tool_call_id"], json!("call_1"));
        let given: Json = serde_json::from_str(messages[3]["content"].as_str().unwrap()).unwrap();
        assert_eq!(given["cards"][0]["values"]["title"], json!("Candide"));

        // An answer to the first request, coming late, is for nobody.
        answer(&conversation, &first, 200, &says("Too late."));
        assert_eq!(state_of(&conversation), "waiting");

        answer(&conversation, &second, 200, &says("  Yes: Candide, from 1759.\n"));
        assert_eq!(state_of(&conversation), "idle");
        assert_eq!(
            lines(&conversation),
            json!([
                ["user", "Do I have Candide?"],
                ["tool", "Looked for “candide” in Books: 1 card."],
                ["agent", "Yes: Candide, from 1759."]
            ])
        );
        become_role("xila_agent");
        assert_eq!(number("SELECT count(*) FROM xila.agent_work()"), 0);
        become_role("ada");

        // The journal says what was done, for whom, and not what the cards say.
        let journal = json(
            "SELECT jsonb_agg(jsonb_build_array(l.role, l.event, l.summary) ORDER BY l.at, l.id)
             FROM xila.agent_log l",
        );
        assert_eq!(
            journal,
            json!([
                ["ada", "model", "Model: 1.1 s, 330 tokens"],
                ["ada", "tool", "Looked for “candide” in Books: 1 card."],
                ["ada", "model", "Model: 1.1 s, 354 tokens"]
            ])
        );
        assert_eq!(
            number("SELECT count(*) FROM xila.agent_log l WHERE l.detail::text ILIKE '%1759%'"),
            0
        );
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["today"],
            json!("1 action, 0 applied, 0 refused")
        );
        assert_eq!(
            json("SELECT xila.grid_rows('agent_journal', 'list')")["rows"][0]["cells"]["summary"],
            json!("Model: 1.1 s, 354 tokens")
        );

        // The screen lists the lines newest first, for the client to write
        // them from the bottom up.
        let talk =
            json(&format!("SELECT xila.grid_rows('agent', 'talk', 0, 100, '{conversation}')"));
        assert_eq!(talk["total"], json!(3), "{talk}");
        assert_eq!(talk["rows"][0]["cells"]["kind"], json!("agent"));
        assert_eq!(talk["rows"][2]["cells"]["text"], json!("Do I have Candide?"));
    }

    #[pg_test]
    fn what_a_tool_refuses_goes_back_to_the_model() {
        set_up();
        Spi::run(r#"SELECT xila.set_stack('books', '{"agent": false}')"#).unwrap();
        let conversation = ask("What is in my books?");
        let first = request(&conversation).unwrap();
        // The stack on screen is closed: the model is not told which card.
        let briefing = first["body"]["messages"][0]["content"].as_str().unwrap();
        assert!(briefing.contains("looking at the stack “Books”"), "{briefing}");
        assert!(!briefing.contains("at the card"), "{briefing}");

        answer(&conversation, &first, 200, &calls("find_cards", json!({"stack": "books"})));
        let second = request(&conversation).unwrap();
        let given = second["body"]["messages"][3]["content"].as_str().unwrap();
        assert!(given.contains("is closed to the agent"), "{given}");
        assert_eq!(
            text("SELECT event FROM xila.agent_log ORDER BY at DESC, id DESC LIMIT 1").unwrap(),
            "refused"
        );

        // A tool that does not exist, and arguments that are not arguments.
        answer(&conversation, &second, 200, &calls("drop_everything", json!({})));
        let third = request(&conversation).unwrap();
        let given = third["body"]["messages"][5]["content"].as_str().unwrap();
        assert!(given.contains("There is no tool “drop_everything”."), "{given}");
        let broken = calls("find_cards", json!({})).replace("{}", "not json");
        answer(&conversation, &third, 200, &broken);
        let fourth = request(&conversation).unwrap();
        let given = fourth["body"]["messages"][7]["content"].as_str().unwrap();
        // What went wrong inside is not for the model.
        assert_eq!(given, r#"{"error": "The tool “find_cards” failed."}"#);

        answer(&conversation, &fourth, 200, &says("Your books are closed to me."));
        assert_eq!(state_of(&conversation), "idle");
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["today"],
            json!("0 actions, 0 applied, 0 refused")
        );
    }

    #[pg_test]
    fn a_failure_of_the_model_is_said_in_the_conversation() {
        set_up();
        for (status, body, said) in [
            (-1, "timeout", "The model did not answer in time."),
            (-1, "token", "The token of the model is missing on the server: the variable TEST_TOKEN is not set."),
            (-1, "unreachable", "The model could not be reached."),
            (401, r#"{"error": {"message": "No auth credentials found"}}"#, "The model refused the token (HTTP 401)."),
            (429, "", "The model is busy, or out of credit (HTTP 429)."),
            (500, "<html>oops</html>", "The model answered with an error (HTTP 500)."),
            (200, "not json", "The model answered something that is not an answer."),
            (200, r#"{"choices": []}"#, "The model answered something that is not an answer."),
        ] {
            let conversation = ask("Hello?");
            let asked = request(&conversation).unwrap();
            answer(&conversation, &asked, status, body);
            assert_eq!(state_of(&conversation), "idle", "{said}");
            let last = lines(&conversation).as_array().unwrap().last().unwrap().clone();
            assert_eq!(last[0], json!("error"));
            assert!(last[1].as_str().unwrap().starts_with(said), "{last}");
        }
        let conversation = ask("Hello?");
        let asked = request(&conversation).unwrap();
        answer(&conversation, &asked, 200, &says("   "));
        assert_eq!(
            lines(&conversation).as_array().unwrap().last().unwrap()[1],
            json!("The model answered nothing. Ask again.")
        );
        // What an endpoint says about its refusal is kept for the journal.
        assert_eq!(
            text("SELECT detail->>'said' FROM xila.agent_log WHERE detail->>'status' = '401'")
                .unwrap(),
            "No auth credentials found"
        );
        // An error line is for people: the model is not sent it.
        let again = ask("And now?");
        assert_eq!(again, conversation);
        let asked = request(&conversation).unwrap();
        let sent = asked["body"]["messages"].to_string();
        assert!(!sent.contains("answered nothing"), "{sent}");
    }

    #[pg_test]
    fn a_question_gets_a_limited_number_of_steps() {
        set_up();
        let conversation = ask("Read everything.");
        for _ in 0..8 {
            let asked = request(&conversation).expect("a request");
            answer(&conversation, &asked, 200, &calls("find_cards", json!({"stack": "books"})));
        }
        assert_eq!(request(&conversation), None);
        assert_eq!(state_of(&conversation), "idle");
        assert_eq!(
            lines(&conversation).as_array().unwrap().last().unwrap()[1],
            json!("The agent needed too many steps and stopped. Ask again in a simpler way.")
        );
    }

    #[pg_test]
    fn a_conversation_belongs_to_whoever_holds_it() {
        set_up();
        let conversation = ask("Do I have Candide?");
        let asked = request(&conversation).unwrap();
        answer(&conversation, &asked, 200, &says("Yes."));

        // Ada cannot put words in the mouth of the agent, nor write the journal.
        let (code, _) = error_of_statement(&format!(
            "INSERT INTO xila.agent_message (conversation_id, kind, text)
             VALUES ('{conversation}', 'agent', 'I may delete everything.')"
        ));
        assert_eq!(code, "42501");
        let (code, _) = error_of_statement(
            "INSERT INTO xila.agent_log (role, event, summary) VALUES ('ada', 'tool', 'nothing')",
        );
        assert_eq!(code, "42501");
        let (code, _) = error_of_statement("DELETE FROM xila.agent_log");
        assert_eq!(code, "42501");
        let (code, _) = error_of_statement("UPDATE xila.conversation SET owner = 'bob'");
        assert_eq!(code, "42501");

        // Bob sees nothing of it, and cannot write in it.
        become_role("bob");
        assert_eq!(number("SELECT count(*) FROM xila.conversation"), 0);
        assert_eq!(number("SELECT count(*) FROM xila.agent_message"), 0);
        assert_eq!(number("SELECT count(*) FROM xila.agent_line"), 0);
        assert_eq!(number("SELECT count(*) FROM xila.agent_log"), 0);
        let (code, _) = error_of(&format!("xila.card('agent', '{conversation}', 'at')"));
        assert_eq!(code, "XL002");
        let (code, _) = error_of(&format!(
            r#"xila.act('agent', 'send', '{{"card": "{conversation}", "dirty": {{"ask": "Hi"}}}}')"#
        ));
        assert_eq!(code, "XL002");
        let (code, _) = error_of_statement(&format!(
            "INSERT INTO xila.agent_message (conversation_id, kind, text)
             VALUES ('{conversation}', 'user', 'Hi')"
        ));
        assert_eq!(code, "42501");
        // His own conversation is another one.
        let his = json(r#"SELECT xila.act('desktop', 'agent', '{}')"#);
        assert_ne!(his[0]["card"], json!(conversation));

        // An administrator reads the whole journal, not the conversations.
        become_role("eve");
        assert_eq!(number("SELECT count(*) FROM xila.agent_log"), 1);
        assert_eq!(number("SELECT count(*) FROM xila.agent_message"), 0);
    }

    #[pg_test]
    fn a_long_conversation_is_sent_from_its_latest_lines() {
        set_up();
        let conversation = ask("First question");
        let asked = request(&conversation).unwrap();
        answer(&conversation, &asked, 200, &says(&"old ".repeat(40_000)));
        ask("Second question");
        let asked = request(&conversation).unwrap();
        answer(&conversation, &asked, 200, &says(&"recent ".repeat(20_000)));
        ask("Third question");

        let asked = request(&conversation).unwrap();
        let messages = asked["body"]["messages"].as_array().unwrap();
        // The first exchange no longer fits; what is sent starts at a question.
        assert_eq!(messages.len(), 4, "{}", messages.len());
        assert_eq!(messages[1]["content"], json!("Second question"));
        assert_eq!(messages[3]["content"], json!("Third question"));
    }

    #[pg_test]
    fn f9_belongs_to_the_agent() {
        create_books_as_ada();
        let screen = json("SELECT xila.open_stack('books')");
        let keys = screen["keys"].as_array().unwrap();
        assert!(
            keys.contains(&json!({"key": "F9", "label": "Agent", "action": "agent"})),
            "{keys:?}"
        );

        // Neither a script nor a key of a stack can take it.
        let checked =
            json(r"SELECT xila.check_script(E'keys = { F9 = ''Mine'' }\nfunction on_key() end')");
        assert_eq!(checked["ok"], json!(false), "{checked}");
        let (code, _) = error_of_statement(
            "INSERT INTO xila.key_binding (background_id, key, action_id)
             VALUES (xila.background_id('books'), 'F9', xila.action_id('books', 'new'))",
        );
        assert_eq!(code, "42501");
    }

    #[pg_test]
    fn the_screens_of_the_agent_are_stacks() {
        set_up();
        // The desktop leads to the agent.
        let desktop = json("SELECT xila.open_stack('desktop')");
        assert_eq!(widget_of(&desktop, "agent")["action"], json!("agent"));

        let screen = json("SELECT xila.open_stack('agent')");
        let talk = widget_of(&screen, "talk");
        assert_eq!(talk["kind"], json!("grid"));
        assert_eq!(talk["props"]["view"], json!("talk"));
        assert_eq!(talk["props"]["who"], json!("kind"));
        assert_eq!(widget_of(&screen, "ask")["kind"], json!("field"));
        let keys: Vec<String> = screen["keys"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| {
                format!("{}={}", key["key"].as_str().unwrap(), key["label"].as_str().unwrap())
            })
            .collect();
        assert_eq!(
            keys,
            [
                "F2=Apply",
                "F3=Quit",
                "F4=Refuse",
                "F5=Stop",
                "F6=New",
                "F10=List",
                "F12=Back",
                "Enter=Send"
            ]
        );
        // The conversation has a place on the text screen, and one line there
        // for what the window shows on its side.
        assert_eq!(talk["layout"]["text"]["h"], json!(14));
        assert_eq!(widget_of(&screen, "model")["layout"]["text"]["style"]["hidden"], json!(true));
        assert_eq!(
            widget_of(&screen, "status")["layout"]["graphic"]["style"]["hidden"],
            json!(true)
        );

        // The journal and the settings are reached from the list of
        // conversations; the settings are offered to administrators only.
        let list = json("SELECT xila.open_stack('conversations')");
        assert!(!list["keys"].to_string().contains("Settings"), "{list}");
        assert_eq!(
            json(r#"SELECT xila.act('conversations', 'journal', '{}')"#),
            json!([{"do": "show", "stack": "agent_journal"}])
        );
        become_role("eve");
        let keys = json("SELECT xila.open_stack('conversations')")["keys"].clone();
        assert!(
            keys.as_array()
                .unwrap()
                .contains(&json!({"key": "F4", "label": "Settings", "action": "settings"})),
            "{keys}"
        );

        // The author dialog says whether the agent may read the cards.
        become_role("ada");
        let author = json("SELECT xila.open_stack('author')");
        assert_eq!(widget_of(&author, "agent")["type"], json!("boolean"));
        Spi::run(
            r#"SELECT xila.act('author', 'save', '{"card": "books", "dirty": {"agent": "false"}}')"#,
        )
        .unwrap();
        assert!(!agent_may_read_books());
    }

    /// Has the model ask for a tool that changes something, then say a word.
    /// Gives the conversation.
    fn propose(question: &str, tool: &str, arguments: Json) -> String {
        let conversation = ask(question);
        let asked = request(&conversation).expect("a request");
        answer(&conversation, &asked, 200, &calls(tool, arguments));
        let again = request(&conversation).expect("a second request");
        answer(&conversation, &again, 200, &says("I propose it."));
        assert_eq!(state_of(&conversation), "idle");
        conversation
    }

    /// The proposals of a conversation, the latest first.
    fn proposals(conversation: &str) -> Json {
        json(&format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY p.at DESC, p.id DESC), '[]')
             FROM xila.agent_proposal p WHERE p.conversation_id = '{conversation}'"
        ))
    }

    /// What the model was told the latest tool gave.
    fn told(conversation: &str) -> String {
        json(&format!(
            "SELECT m.payload->'result' FROM xila.agent_message m
             WHERE m.conversation_id = '{conversation}' AND m.kind = 'tool'
             ORDER BY m.at DESC, m.id DESC LIMIT 1"
        ))
        .to_string()
    }

    fn act(action: &str, context: Json) -> Json {
        json(&format!("SELECT xila.act('agent', '{action}', {})", quoted(&context.to_string())))
    }

    fn has_column(column: &str) -> bool {
        number(&format!(
            "SELECT count(*) FROM pg_attribute
             WHERE attrelid = 'books.cards'::regclass AND attname = '{column}' AND NOT attisdropped"
        )) == 1
    }

    #[pg_test]
    fn the_agent_proposes_and_the_person_applies() {
        set_up();
        let conversation = propose(
            "Add when I bought each book.",
            "add_field",
            json!({"stack": "books", "label": "Bought", "type": "date"}),
        );

        // Nothing changed: the change waits, described, in the conversation.
        assert!(!has_column("bought"));
        let waiting = proposals(&conversation);
        assert_eq!(waiting.as_array().unwrap().len(), 1, "{waiting}");
        assert_eq!(waiting[0]["state"], json!("pending"));
        assert_eq!(waiting[0]["title"], json!("Add the field “Bought” to Books"));
        let detail = "A field of type date, at the end of the cards. It adds the column bought \
                      to the table books.cards.";
        assert_eq!(waiting[0]["detail"], json!(detail));
        assert_eq!(waiting[0]["code"], json!("xila.add_field('books', 'Bought', 'date')"));
        let proposal = waiting[0]["id"].as_str().unwrap().to_owned();
        assert_eq!(
            lines(&conversation),
            json!([
                ["user", "Add when I bought each book."],
                ["proposal", format!("Add the field “Bought” to Books\n{detail}")],
                ["agent", "I propose it."]
            ])
        );
        // The model was told that it is only proposed.
        let given = told(&conversation);
        assert!(given.contains(r#""proposed":true"#), "{given}");
        assert!(given.contains("Nothing has changed."), "{given}");
        assert_eq!(
            text(
                "SELECT summary FROM xila.agent_log WHERE event = 'tool' ORDER BY at DESC LIMIT 1"
            )
            .unwrap(),
            "Proposed: add the field “Bought” to Books."
        );
        // The screen gives the proposal its state and the call it runs.
        let talk =
            json(&format!("SELECT xila.grid_rows('agent', 'talk', 0, 100, '{conversation}')"));
        let row = &talk["rows"][1];
        assert_eq!(row["key"], json!(proposal));
        assert_eq!(row["cells"]["state"], json!("pending"));
        assert_eq!(row["cells"]["code"], json!("xila.add_field('books', 'Bought', 'date')"));

        // The same thing asked again is not proposed twice.
        let again = ask("Did you do it?");
        let asked = request(&again).unwrap();
        let same = json!({"stack": "Books", "label": "Bought", "type": "date"});
        answer(&again, &asked, 200, &calls("add_field", same));
        assert_eq!(proposals(&conversation).as_array().unwrap().len(), 1);
        let asked = request(&again).unwrap();
        answer(&again, &asked, 200, &says("It waits for you."));

        // F2 applies the proposal that waits: Ada's session makes the change.
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[0], json!({"do": "show", "stack": "agent", "card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Field “Bought” added to Books."));
        assert!(has_column("bought"));
        assert_eq!(proposals(&conversation)[0]["state"], json!("applied"));
        assert_eq!(
            lines(&conversation).as_array().unwrap().last().unwrap(),
            &json!(["note", "Applied: Field “Bought” added to Books."])
        );
        // History knows the change came from the agent, and who applied it.
        assert_eq!(
            json(
                "SELECT jsonb_agg(DISTINCT jsonb_build_array(h.actor, h.origin)) FROM xila.history h
                 WHERE h.object_kind = 'column'"
            ),
            json!([["ada", "agent"], ["ada", "user"]])
        );
        assert_eq!(
            text("SELECT actor FROM xila.history_line WHERE stack = 'books'").unwrap(),
            "ada (agent)"
        );
        // What is done next is Ada's again.
        Spi::run("SELECT xila.add_field('books', 'Pages', 'number')").unwrap();
        assert_eq!(
            text("SELECT h.origin FROM xila.history h ORDER BY h.id DESC LIMIT 1").unwrap(),
            "user"
        );
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["today"],
            json!("2 actions, 1 applied, 0 refused")
        );

        // The model reads what became of its proposal with the next question.
        ask("Thanks.");
        let sent = request(&conversation).unwrap()["body"]["messages"].to_string();
        assert!(sent.contains("(Applied: Field “Bought” added to Books.)"), "{sent}");

        // Applied once: nothing is left to apply, and it cannot be applied again.
        let nothing = act("apply", json!({"card": conversation}));
        assert_eq!(nothing[0]["text"], json!("The agent has proposed nothing that waits."));
        let (code, message) = error_of(&format!("xila.apply_proposal('{proposal}')"));
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("XL003", "This proposal was already applied.")
        );

        // What the agent proposed is undone like anything else.
        Spi::run(
            "SELECT xila.rewind('books', 'screen',
                 (SELECT min(h.id) FROM xila.history h WHERE h.origin = 'agent'))",
        )
        .unwrap();
        assert!(!has_column("bought"));
    }

    #[pg_test]
    fn a_proposal_that_is_refused_changes_nothing() {
        set_up();
        let conversation =
            propose("Remove the year.", "remove_field", json!({"stack": "books", "field": "Year"}));
        let waiting = proposals(&conversation);
        assert_eq!(waiting[0]["title"], json!("Remove the field “Year” from Books"));
        assert_eq!(
            waiting[0]["detail"],
            json!(
                "2 cards have a value for it. History can bring the field back, with its values."
            )
        );
        let proposal = waiting[0]["id"].as_str().unwrap().to_owned();

        // The window names the proposal its button belongs to.
        let refused = act("refuse", json!({"card": conversation, "input": {"proposal": proposal}}));
        assert_eq!(refused[1]["text"], json!("Proposal refused."));
        assert!(has_column("year"));
        assert_eq!(proposals(&conversation)[0]["state"], json!("refused"));
        assert_eq!(
            lines(&conversation).as_array().unwrap().last().unwrap(),
            &json!(["note", "Refused: Remove the field “Year” from Books."])
        );
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["today"],
            json!("1 action, 0 applied, 1 refused")
        );
        let (code, _) = error_of(&format!("xila.apply_proposal('{proposal}')"));
        assert_eq!(code, "XL003");
        // The journal says what became of it.
        assert_eq!(
            json("SELECT xila.grid_rows('agent_journal', 'list')")["rows"][0]["cells"]["summary"],
            json!("Refused: Remove the field “Year” from Books")
        );

        // Applied, it removes the field, without another question.
        let conversation =
            propose("Remove it.", "remove_field", json!({"stack": "books", "field": "year"}));
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Field “Year” removed from Books."));
        assert!(!has_column("year"));
    }

    #[pg_test]
    fn the_agent_proposes_what_a_card_should_say() {
        set_up();
        let candide = text("SELECT id::text FROM books.cards WHERE title = 'Candide'").unwrap();
        let conversation = propose(
            "Candide is from 1760.",
            "save_card",
            json!({"stack": "books", "key": candide, "values": {"Year": 1760, "title": "Candide"}}),
        );
        let waiting = proposals(&conversation);
        // What the card already says is not part of the change.
        assert_eq!(waiting[0]["title"], json!("Change the card “Candide” of Books"));
        assert_eq!(waiting[0]["detail"], json!("Year: 1759 → 1760"));
        assert_eq!(
            waiting[0]["code"],
            json!(format!(r#"xila.save_card('books', '{candide}', '{{"year": "1760"}}')"#))
        );
        assert_eq!(waiting[0]["card"], json!(candide));

        // Someone changes the card in the meantime: applying overwrites nothing.
        Spi::run("UPDATE books.cards SET year = 1761 WHERE title = 'Candide'").unwrap();
        let (code, message) =
            error_of(&format!(r#"xila.act('agent', 'apply', '{{"card": "{conversation}"}}')"#));
        assert_eq!(code, "XL003");
        assert!(message.starts_with("The card changed since the agent proposed this"), "{message}");
        assert_eq!(proposals(&conversation)[0]["state"], json!("pending"));
        Spi::run("UPDATE books.cards SET year = 1759 WHERE title = 'Candide'").unwrap();

        // "View" goes to the card the proposal is about.
        let shown = act("view", json!({"card": conversation}));
        assert_eq!(shown, json!([{"do": "show", "stack": "books", "card": candide}]));

        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Card “Candide” of Books changed."));
        assert_eq!(number("SELECT year::bigint FROM books.cards WHERE title = 'Candide'"), 1760);
        assert_eq!(
            text("SELECT actor FROM xila.card_history_line ORDER BY id DESC LIMIT 1").unwrap(),
            "ada (agent)"
        );

        // A new card.
        let conversation = propose(
            "Add Les Misérables.",
            "save_card",
            json!({"stack": "books", "values": {"title": "Les Misérables", "year": "1862"}}),
        );
        assert_eq!(proposals(&conversation)[0]["title"], json!("Add a card to Books"));
        assert_eq!(
            proposals(&conversation)[0]["detail"],
            json!("Title: Les Misérables\nYear: 1862")
        );
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Card “Les Misérables” added to Books."));
        assert_eq!(number("SELECT count(*) FROM books.cards"), 3);
        // The proposal now leads to the card it made.
        let made = text("SELECT id::text FROM books.cards WHERE title = 'Les Misérables'").unwrap();
        assert_eq!(act("view", json!({"card": conversation}))[0]["card"], json!(made));

        // And a card deleted.
        let conversation =
            propose("Delete it.", "delete_card", json!({"stack": "books", "key": made}));
        assert_eq!(
            proposals(&conversation)[0]["title"],
            json!("Delete the card “Les Misérables” of Books")
        );
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Card “Les Misérables” deleted from Books."));
        assert_eq!(number("SELECT count(*) FROM books.cards"), 2);
    }

    #[pg_test]
    fn what_cannot_be_proposed_is_said_to_the_model() {
        set_up();
        let candide = text("SELECT id::text FROM books.cards WHERE title = 'Candide'").unwrap();
        let cases = [
            ("add_field", json!({"stack": "books", "label": "Year"}), "already has a field “Year”"),
            (
                "add_field",
                json!({"stack": "books", "label": "Cover", "type": "picture"}),
                "text, number, date or boolean",
            ),
            ("add_field", json!({"stack": "books"}), "A field needs a label."),
            (
                "change_field",
                json!({"stack": "books", "field": "pages", "label": "P"}),
                "has no field “pages”. Its fields are: title, year.",
            ),
            (
                "change_field",
                json!({"stack": "books", "field": "year", "label": "Year"}),
                "There is nothing to change",
            ),
            (
                "save_card",
                json!({"stack": "books", "key": candide, "values": {"year": "long ago"}}),
                "“long ago” cannot be the value of “Year”, which is a number field.",
            ),
            (
                "save_card",
                json!({"stack": "books", "key": candide, "values": {"year": 1759}}),
                "The card already says so",
            ),
            (
                "save_card",
                json!({"stack": "books", "key": "nothing", "values": {"year": 1}}),
                "There is no card with the key “nothing”",
            ),
            ("save_card", json!({"stack": "books", "values": {}}), "“values” holds what to write"),
            ("delete_card", json!({"stack": "books", "key": ""}), "There is no card with the key"),
            (
                "create_stack",
                json!({"title": "Books", "fields": [{"label": "Title"}]}),
                "The name “books” is already taken",
            ),
            ("create_stack", json!({"title": "Films", "fields": []}), "“fields” lists the fields"),
            (
                "create_stack",
                json!({"title": "Films", "fields": [{"label": "Title"}, {"label": "title"}]}),
                "Two fields cannot both be named “title”.",
            ),
        ];
        for (tool, arguments, said) in cases {
            let conversation = propose("Do it.", tool, arguments.clone());
            let given = told(&conversation);
            assert!(given.contains(said), "{tool} {arguments}: {given}");
            assert_eq!(proposals(&conversation), json!([]), "{tool} {arguments}");
        }

        // Required: a card that has no value stands in the way, and a new
        // card needs what its table requires.
        Spi::run(
            r#"SELECT xila.set_field('books', 'title', '{"required": true}');
               SELECT xila.save_card('books', NULL, '{"title": "Zadig"}')"#,
        )
        .unwrap();
        let wanted = json!({"stack": "books", "field": "year", "required": true});
        let conversation = propose("Do it.", "change_field", wanted);
        let given = told(&conversation);
        assert!(given.contains("1 card has no value for “Year”"), "{given}");
        let wanted = json!({"stack": "books", "values": {"year": 1900}});
        let conversation = propose("Do it.", "save_card", wanted);
        let given = told(&conversation);
        assert!(given.contains("A card of Books needs a value for: Title."), "{given}");
    }

    #[pg_test]
    fn the_agent_proposes_no_more_than_the_person_could_do() {
        set_up();
        // Dan reads the books and nothing else; Bob is an author who does
        // not own them.
        Spi::run(
            "GRANT USAGE ON SCHEMA books TO dan, bob; GRANT SELECT ON books.cards TO dan, bob",
        )
        .unwrap();
        let candide = text("SELECT id::text FROM books.cards WHERE title = 'Candide'").unwrap();
        let cases = [
            ("dan", "add_field", json!({"stack": "books", "label": "Pages"}), "XL004"),
            ("bob", "remove_field", json!({"stack": "books", "field": "year"}), "XL004"),
            (
                "dan",
                "save_card",
                json!({"stack": "books", "key": candide, "values": {"year": 1}}),
                "XL004",
            ),
            ("dan", "delete_card", json!({"stack": "books", "key": candide}), "XL004"),
            (
                "dan",
                "create_stack",
                json!({"title": "Films", "fields": [{"label": "Title"}]}),
                "XL004",
            ),
            // Someone who cannot open the stack: it does not exist.
            ("eve", "add_field", json!({"stack": "books", "label": "Pages"}), "XL002"),
        ];
        for (as_role, tool, arguments, code) in cases {
            become_role("xila_agent");
            let (found, message) = error_of(&format!(
                "xila.propose_{tool}('{as_role}', {})",
                quoted(&arguments.to_string())
            ));
            become_role("ada");
            assert_eq!(found, code, "{as_role} {tool}: {message}");
        }
        become_role("xila_agent");
        let (_, message) =
            error_of(r#"xila.propose_add_field('bob', '{"stack": "books", "label": "Pages"}')"#);
        assert_eq!(message, "The stack “Books” belongs to ada: bob cannot change its fields.");
        become_role("ada");

        // Nothing can be proposed on a stack that is closed to the agent.
        Spi::run(r#"SELECT xila.set_stack('books', '{"agent": false}')"#).unwrap();
        let conversation =
            propose("Do it.", "add_field", json!({"stack": "books", "label": "Pages"}));
        let given = told(&conversation);
        assert!(given.contains("is closed to the agent"), "{given}");
        Spi::run(r#"SELECT xila.set_stack('books', '{"agent": true}')"#).unwrap();

        // The functions that propose are for the worker, which still cannot
        // write a card or a definition itself.
        let (code, _) =
            error_of(r#"xila.propose_add_field('ada', '{"stack": "books", "label": "Pages"}')"#);
        assert_eq!(code, "42501");
        become_role("xila_agent");
        let (code, _) = error_of_statement("UPDATE books.cards SET year = 1");
        assert_eq!(code, "42501");
        let (code, _) = error_of(
            r#"xila.apply_add_field('{"stack": "books", "label": "Pages", "type": "text"}')"#,
        );
        assert_eq!(code, "XL004");
        let (code, _) = error_of_statement("UPDATE xila.agent_proposal SET state = 'applied'");
        assert_eq!(code, "42501");
        become_role("ada");

        // A proposal belongs to the conversation of its owner: nobody else
        // sees it or applies it, and its owner cannot rewrite it.
        let conversation =
            propose("Do it.", "add_field", json!({"stack": "books", "label": "Pages"}));
        let proposal = proposals(&conversation)[0]["id"].as_str().unwrap().to_owned();
        let (code, _) = error_of_statement("UPDATE xila.agent_proposal SET arguments = '{}'");
        assert_eq!(code, "42501");
        become_role("bob");
        assert_eq!(number("SELECT count(*) FROM xila.agent_proposal"), 0);
        let (code, _) = error_of(&format!("xila.apply_proposal('{proposal}')"));
        assert_eq!(code, "XL002");
        let (code, _) = error_of(&format!("xila.refuse_proposal('{proposal}')"));
        assert_eq!(code, "XL002");
        become_role("ada");
        assert!(!has_column("pages"));
    }

    #[pg_test]
    fn the_agent_proposes_a_stack_and_changes_to_its_fields() {
        set_up();
        let conversation = propose(
            "I want to keep my films.",
            "create_stack",
            json!({"title": "My films", "fields": [
                {"label": "Title"},
                {"label": "Seen on", "type": "date"},
                {"label": "Stars", "type": "number"}]}),
        );
        let waiting = proposals(&conversation);
        assert_eq!(waiting[0]["title"], json!("Create the stack “My films”"));
        assert_eq!(waiting[0]["detail"], json!("Title: text\nSeen on: date\nStars: number"));
        assert_eq!(
            waiting[0]["code"],
            json!(
                "xila.create_stack('My films')\n\
                 xila.add_field('my_films', 'Title', 'text')\n\
                 xila.add_field('my_films', 'Seen on', 'date')\n\
                 xila.add_field('my_films', 'Stars', 'number')"
            )
        );
        // There is nothing to look at before it is applied.
        assert_eq!(
            act("view", json!({"card": conversation}))[0]["text"],
            json!("There is nothing to look at yet.")
        );
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Stack “My films” created, with 3 fields."));
        assert_eq!(
            text("SELECT owner::text FROM xila.stack WHERE name = 'my_films'").unwrap(),
            "ada"
        );
        assert_eq!(
            json("SELECT jsonb_agg(f.name ORDER BY f.ord) FROM xila.field f WHERE f.stack = 'my_films'"),
            json!(["title", "seen_on", "stars"])
        );
        assert_eq!(
            act("view", json!({"card": conversation})),
            json!([{"do": "show", "stack": "my_films"}])
        );
        // The agent may read what it helped to make.
        assert_eq!(tool("find_cards", "ada", json!({"stack": "my_films"}))["total"], json!(0));

        // A field changes its label and its name, and becomes required.
        let conversation = propose(
            "Call it rating.",
            "change_field",
            json!({"stack": "my_films", "field": "Stars", "label": "Rating", "name": "rating",
                   "required": true}),
        );
        let waiting = proposals(&conversation);
        assert_eq!(waiting[0]["title"], json!("Change the field “Stars” of My films"));
        assert_eq!(
            waiting[0]["detail"],
            json!(
                "label: Stars → Rating\n\
                 name: stars → rating (the column is renamed)\n\
                 required: no → yes"
            )
        );
        let applied = act("apply", json!({"card": conversation}));
        assert_eq!(applied[1]["text"], json!("Applied: Field “Stars” of My films changed."));
        assert_eq!(
            text("SELECT label FROM xila.field f WHERE f.stack = 'my_films' AND f.name = 'rating'")
                .unwrap(),
            "Rating"
        );
    }

    #[pg_test]
    fn a_table_pgxila_did_not_create_is_never_altered_by_a_proposal() {
        set_up();
        Spi::run(
            "CREATE TABLE books.loans (id integer GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
                                       who text NOT NULL, back date, note text);
             SELECT xila.create_stack('Loans', NULL, 'books.loans'::regclass);
             SELECT xila.remove_field('loans', 'note')",
        )
        .unwrap();
        // Only a column that is there can become a field.
        let conversation =
            propose("Do it.", "add_field", json!({"stack": "loans", "label": "Phone"}));
        let given = told(&conversation);
        assert!(given.contains("only one of its columns can become a field"), "{given}");
        assert!(given.contains("Columns not shown yet: note."), "{given}");
        let wanted = json!({"stack": "loans", "label": "Note", "name": "note"});
        let conversation = propose("Do it.", "add_field", wanted);
        assert_eq!(
            proposals(&conversation)[0]["detail"],
            json!("It shows the column note of the table books.loans, which is not changed.")
        );
        act("apply", json!({"card": conversation}));
        assert_eq!(
            number("SELECT count(*) FROM xila.field WHERE stack = 'loans' AND name = 'note'"),
            1
        );

        // A label may change; a name and what is required may not.
        let wanted = json!({"stack": "loans", "field": "who", "name": "person"});
        let conversation = propose("Do it.", "change_field", wanted);
        let given = told(&conversation);
        assert!(given.contains("only the label of its fields can be changed"), "{given}");
        let wanted = json!({"stack": "loans", "field": "back"});
        let conversation = propose("Do it.", "remove_field", wanted);
        assert_eq!(
            proposals(&conversation)[0]["detail"],
            json!("It is only taken off the screen: the table is not changed.")
        );
    }
}
