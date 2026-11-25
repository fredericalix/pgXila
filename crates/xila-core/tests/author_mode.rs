//! Integration tests of author mode against a real pgXila server: an author
//! builds a stack from the client, without a line of SQL.
//!
//! They run against the throwaway "scratch" server and are ignored by default.
//! Run them with `scripts/test-integration.sh`.
//!
//! Here every action is a transaction of its own, as in real use: this is
//! where the lines of the history of a stack are checked.

use std::path::PathBuf;

use xila_core::connection::ConnectParams;
use xila_core::server::Server;
use xila_core::session::Session;

const STACK: &str = "test_books";

fn var(name: &str) -> String {
    std::env::var(name)
        .unwrap_or_else(|_| panic!("{name} is not set: use scripts/test-integration.sh"))
}

fn params(user_variable: &str) -> ConnectParams {
    ConnectParams {
        host: var("PGXILA_TEST_HOST"),
        port: var("PGXILA_TEST_SCRATCH_PORT").parse().expect("PGXILA_TEST_SCRATCH_PORT"),
        database: var("PGXILA_TEST_DATABASE"),
        user: var(user_variable),
        password: var("PGXILA_TEST_PASSWORD"),
        ca_file: Some(PathBuf::from(var("PGXILA_TEST_SCRATCH_CA"))),
    }
}

fn session_as(user_variable: &str) -> Session<Server> {
    let server = Server::connect(&params(user_variable)).expect("connection to the scratch server");
    Session::start(server).expect("session")
}

/// Runs SQL statements as the author, outside the pgXila API.
fn execute_as_author(statements: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let (client, connection) =
            xila_core::connection::connect(&params("PGXILA_TEST_AUTHOR")).await.unwrap();
        tokio::spawn(connection);
        client.batch_execute(statements).await.unwrap();
    });
}

/// Removes what a previous run may have left behind.
fn forget_the_test_stack() {
    execute_as_author(&format!(
        "DELETE FROM xila.stack WHERE name = '{STACK}'; DROP SCHEMA IF EXISTS {STACK} CASCADE"
    ));
}

/// One column of the rows of a grid of the current screen.
fn column(session: &Session<Server>, grid: &str, name: &str) -> Vec<String> {
    session.view().grids[grid]
        .page
        .rows
        .iter()
        .map(|row| row.cells.get(name).cloned().flatten().unwrap_or_default())
        .collect()
}

fn keys(session: &Session<Server>) -> Vec<String> {
    session.view().screen.keys.iter().map(|k| format!("{}={}", k.key, k.label)).collect()
}

/// Selects the row of a grid whose column has a given value.
fn select(session: &mut Session<Server>, grid: &str, name: &str, value: &str) {
    let row = column(session, grid, name)
        .iter()
        .position(|cell| cell == value)
        .unwrap_or_else(|| panic!("no row with {name} = {value}"));
    session.select(grid, row);
}

/// Adds a field through the dialog, the way an author does.
fn add_field(session: &mut Session<Server>, label: &str, kind: &str) {
    session.press("F6");
    assert_eq!(session.view().stack(), "field", "{:?}", session.message());
    assert!(session.view().is_new);
    assert_eq!(session.view().value("type"), "text", "a new field starts as a text");

    session.edit("label", label.into());
    for _ in 0..4 {
        if session.view().value("type") == kind {
            break;
        }
        assert!(session.cycle("type", true));
    }
    assert_eq!(session.view().value("type"), kind);
    session.press("Enter");

    assert_eq!(session.view().stack(), "author", "{:?}", session.message());
    assert_eq!(session.message(), Some(format!("Field “{label}” added.").as_str()));
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn an_author_builds_a_stack_from_the_client() {
    forget_the_test_stack();
    let mut session = session_as("PGXILA_TEST_AUTHOR");
    assert!(session.hello().author);

    // From the list of stacks: a new stack, named after its title.
    session.activate("stacks");
    assert_eq!(
        keys(&session),
        ["F3=Quit", "F6=New stack", "F10=Deleted", "F11=Author", "F12=Back"]
    );
    session.press("F6");
    assert_eq!(session.view().stack(), "new_stack");
    session.edit("title", "Test books".into());
    session.press("Enter");

    assert_eq!(session.view().stack(), "author", "{:?}", session.message());
    assert_eq!(session.view().card.key.as_deref(), Some(STACK));
    assert_eq!(session.message(), Some("Stack created. F6 adds a field."));
    assert_eq!(session.view().card.texts["heading"], "Test books");
    assert!(column(&session, "fields", "name").is_empty());

    // Three fields, of three types.
    add_field(&mut session, "Title", "text");
    add_field(&mut session, "Year", "number");
    add_field(&mut session, "Read", "boolean");
    assert_eq!(column(&session, "fields", "name"), ["title", "year", "read"]);
    assert_eq!(column(&session, "fields", "type_label"), ["Text", "Number", "Yes or no"]);
    assert_eq!(session.view().value("card_title"), "{title}", "the first field titles the cards");

    // Moving a field up keeps it selected.
    select(&mut session, "fields", "name", "read");
    session.press("F7");
    assert_eq!(column(&session, "fields", "name"), ["title", "read", "year"]);
    assert_eq!(session.view().grids["fields"].selected, 1);

    // The stack is ready to use: two cards.
    session.press("F11");
    assert_eq!(session.view().stack(), STACK);
    assert_eq!(
        keys(&session),
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
    for (title, year) in [("Dune", "1965"), ("Emma", "1815")] {
        session.press("F6");
        session.edit("title", title.into());
        session.edit("year", year.into());
        assert!(session.cycle("read", true), "a boolean field is a choice");
        session.press("Enter");
        assert_eq!(session.message(), Some("Card saved."));
        assert_eq!(session.view().card.texts["heading"], title);
        assert_eq!(session.view().value("read"), "true");
    }
    assert_eq!(session.view().card.count, 2);

    // A card deleted by mistake: it asks first, and history brings it back.
    session.press("F4");
    assert_eq!(session.question().map(|q| q.text.as_str()), Some("Delete this card?"));
    session.answer(true);
    assert_eq!(session.message(), Some("Card deleted."));
    assert_eq!(session.view().card.count, 1);

    session.press("F10");
    assert_eq!(session.view().stack(), "card_history");
    assert_eq!(
        column(&session, "changes", "summary"),
        ["Card “Emma” deleted", "Card “Emma” added", "Card “Dune” added"]
    );
    assert!(column(&session, "changes", "at")[0].starts_with("20"), "a date and a time");
    session.activate("changes");
    assert!(session.question().is_some());
    session.answer(true);
    assert_eq!(session.view().stack(), STACK, "{:?}", session.message());
    assert_eq!(session.message(), Some("1 change undone."));
    assert_eq!(session.view().card.count, 2);

    // Back in author mode: the history of the stack, one line per gesture.
    session.press("F11");
    assert_eq!(session.view().stack(), "author");
    session.press("F10");
    assert_eq!(session.view().stack(), "history");
    let lines = column(&session, "changes", "summary");
    assert_eq!(lines[0], "Card rewound (1 change undone)");
    assert_eq!(lines[1], "Card “Emma” deleted");
    assert_eq!(lines[2], "Card “Emma” added");
    assert_eq!(lines[3], "Card “Dune” added");
    assert!(lines[4].ends_with("moved"), "{}", lines[4]);
    assert_eq!(
        lines[5..],
        ["Field “read” added", "Field “year” added", "Field “title” added", "Stack created"]
    );
    assert!(column(&session, "changes", "actor")
        .iter()
        .all(|who| *who == var("PGXILA_TEST_AUTHOR")));

    // Rewinding the screen to before the second field: the fields go, the
    // cards stay, with the values they still have a field for.
    select(&mut session, "changes", "summary", "Field “year” added");
    session.press("F6");
    let question = session.question().expect("rewinding asks first").text.clone();
    assert!(question.starts_with("Undo 6 changes to the screen of “Test books”?"), "{question}");
    session.answer(true);
    assert_eq!(session.view().stack(), "author", "{:?}", session.message());
    assert_eq!(session.message(), Some("6 changes undone."));
    assert_eq!(column(&session, "fields", "name"), ["title"]);

    session.press("F11");
    assert_eq!(session.view().card.count, 2);
    assert!(!session.view().card.values.contains_key("year"));
    assert_eq!(session.view().value("title"), "Dune");

    // That rewind can itself be undone: the years come back.
    session.press("F11");
    session.press("F10");
    assert_eq!(column(&session, "changes", "summary")[0], "Screen rewound (6 changes undone)");
    session.select("changes", 0);
    session.press("F6");
    session.answer(true);
    assert_eq!(column(&session, "fields", "name"), ["title", "read", "year"]);
    session.press("F11");
    assert_eq!(session.view().value("year"), "1965");

    // Deleting the stack leads back to the list; the stack can be restored.
    session.press("F11");
    session.activate("delete_stack");
    assert!(session.question().is_some());
    session.answer(true);
    assert_eq!(session.view().stack(), "stacks", "{:?}", session.message());
    assert!(!column(&session, "list", "title").contains(&"Test books".to_string()));

    session.press("F10");
    assert_eq!(session.view().stack(), "deleted_stacks");
    assert_eq!(column(&session, "list", "title"), ["Test books"]);
    session.activate("list");
    session.answer(true);
    assert_eq!(session.message(), Some("Stack “Test books” restored."));
    assert!(column(&session, "list", "title").is_empty());
    session.press("F12");
    assert!(column(&session, "list", "title").contains(&"Test books".to_string()));

    forget_the_test_stack();
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn author_mode_is_kept_from_those_who_may_not_use_it() {
    forget_the_test_stack();
    execute_as_author(&format!(
        "SELECT xila.create_stack('Test books');
         SELECT xila.add_field('{STACK}', 'Title');
         GRANT USAGE ON SCHEMA {STACK} TO xila_user;
         GRANT SELECT, INSERT, UPDATE, DELETE ON {STACK}.cards TO xila_user;"
    ));

    // A plain user sees the stack and may fill it in, but not change it.
    let mut session = session_as("PGXILA_TEST_USER");
    assert!(!session.hello().author);
    session.activate("stacks");
    assert_eq!(keys(&session), ["F3=Quit", "F12=Back"]);
    select(&mut session, "list", "title", "Test books");
    session.activate("list");
    assert_eq!(session.view().stack(), STACK);
    assert!(!session.view().screen.stack.may_edit);
    assert!(!keys(&session).iter().any(|key| key.starts_with("F11")));

    session.run("author");
    assert_eq!(session.view().stack(), STACK);
    let refusal = session.message().expect("a refusal").to_owned();
    assert!(refusal.starts_with("The stack “Test books” belongs to "), "{refusal}");

    // Writing a card is theirs to do, and so is its history.
    session.press("F6");
    session.edit("title", "Persuasion".into());
    session.press("Enter");
    assert_eq!(session.message(), Some("Card saved."));
    session.press("F10");
    assert_eq!(column(&session, "changes", "summary"), ["Card “Persuasion” added"]);

    forget_the_test_stack();
}
