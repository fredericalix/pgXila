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
        become_admin, become_role, create_books_as_ada, error_of, error_of_statement, json,
        number, text, widget_of,
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
        let error = error_of(&format!(
            "xila.tool_{name}('{as_role}', {})",
            quoted(&arguments.to_string())
        ));
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
        let asked = Spi::get_one::<pgrx::JsonB>(&format!(
            "SELECT xila.agent_request('{conversation}')"
        ))
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
        assert_eq!(text("SELECT agent::text FROM xila.stack_info WHERE name = 'books'").unwrap(), "false");

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
        assert_eq!((code.as_str(), message.as_str()), ("XL002", "The stack “books” does not exist."));
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
        assert_eq!(tool("find_cards", "ada", json!({"stack": "books", "text": "%"}))["total"], json!(0));

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

        let (code, message) = tool_error("read_card", "ada", json!({"stack": "books", "position": 9}));
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
        let (code, _) = error_of(r#"xila.act('agent', 'settings', '{}')"#);
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
        let shown = json(r#"SELECT xila.act('agent', 'settings', '{}')"#);
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
            json!("List the stacks, Find cards, Read a card")
        );
        assert_eq!(on_screen["values"]["today"], json!("0 actions, 0 refused"));

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
            text(&format!("SELECT title FROM xila.conversation WHERE id = '{conversation}'")).unwrap(),
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
        let stopped = json(&format!(
            r#"SELECT xila.act('agent', 'stop', '{{"card": "{conversation}"}}')"#
        ));
        assert_eq!(stopped[1]["text"], json!("Question given up."), "{stopped}");
        assert_eq!(state_of(&conversation), "idle");
        answer(&conversation, &before, 200, &says("Two."));
        assert_eq!(lines(&conversation).as_array().unwrap().len(), 2);

        // F9 again, from the desktop: the same conversation, about nothing.
        let again = json(r#"SELECT xila.act('desktop', 'agent', '{}')"#);
        assert_eq!(again[0]["card"], json!(conversation));
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["about"],
            json!("")
        );

        // F6 starts another one, and the list shows both.
        let fresh = json(&format!(
            r#"SELECT xila.act('agent', 'new', '{{"card": "{conversation}"}}')"#
        ));
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
        assert_eq!(tools, ["list_stacks", "find_cards", "read_card"]);
        assert_eq!(body["tools"][1]["function"]["parameters"]["required"], json!(["stack"]));

        // The model asks for a tool: the worker runs it, for Ada, and the
        // conversation waits for the next call.
        answer(&conversation, &first, 200, &calls("find_cards", json!({"stack": "books", "text": "candide"})));
        assert_eq!(state_of(&conversation), "waiting");
        assert_eq!(
            lines(&conversation),
            json!([["user", "Do I have Candide?"],
                   ["tool", "Looked for “candide” in Books: 1 card."],
                   ["wait", "The agent is working…"]])
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
            json!([["user", "Do I have Candide?"],
                   ["tool", "Looked for “candide” in Books: 1 card."],
                   ["agent", "Yes: Candide, from 1759."]])
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
            json!([["ada", "model", "Model: 1.1 s, 330 tokens"],
                   ["ada", "tool", "Looked for “candide” in Books: 1 card."],
                   ["ada", "model", "Model: 1.1 s, 354 tokens"]])
        );
        assert_eq!(number("SELECT count(*) FROM xila.agent_log l WHERE l.detail::text ILIKE '%1759%'"), 0);
        assert_eq!(
            json(&format!("SELECT xila.card('agent', '{conversation}', 'at')"))["values"]["today"],
            json!("1 action, 0 refused")
        );
        assert_eq!(
            json("SELECT xila.grid_rows('agent_journal', 'list')")["rows"][0]["cells"]["summary"],
            json!("Model: 1.1 s, 354 tokens")
        );

        // The screen lists the lines newest first, for the client to write
        // them from the bottom up.
        let talk = json(&format!("SELECT xila.grid_rows('agent', 'talk', 0, 100, '{conversation}')"));
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
            json!("0 actions, 1 refused")
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
            text("SELECT detail->>'said' FROM xila.agent_log WHERE detail->>'status' = '401'").unwrap(),
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
        let checked = json(
            r"SELECT xila.check_script(E'keys = { F9 = ''Mine'' }\nfunction on_key() end')",
        );
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
            .map(|key| format!("{}={}", key["key"].as_str().unwrap(), key["label"].as_str().unwrap()))
            .collect();
        assert_eq!(
            keys,
            ["F3=Quit", "F5=Stop", "F6=New", "F7=Journal", "F9=Agent", "F10=Conversations",
             "F12=Back", "Enter=Send"]
        );
        // The settings are offered to administrators only.
        become_role("eve");
        let keys = json("SELECT xila.open_stack('agent')")["keys"].clone();
        assert!(
            keys.as_array().unwrap().contains(&json!({"key": "F4", "label": "Settings", "action": "settings"})),
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
}
