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
        ["F3=Quit", "F6=New stack", "F9=Agent", "F10=Deleted", "F11=Author", "F12=Back"]
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
            "F9=Agent",
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
    assert_eq!(keys(&session), ["F3=Quit", "F9=Agent", "F12=Back"]);
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

/// Opens the test stack from the list of stacks.
fn open_the_test_stack(session: &mut Session<Server>) {
    session.activate("stacks");
    select(session, "list", "title", "Test books");
    session.activate("list");
    assert_eq!(session.view().stack(), STACK, "{:?}", session.message());
}

/// The rectangle the server holds for a widget in the window, if any.
fn place_of(session: &Session<Server>, widget: &str) -> Option<(i32, i32)> {
    let widget = session.view().widgets().find(|w| w.name == widget)?;
    widget.layout.get("graphic").filter(|place| place.w > 0).map(|place| (place.x, place.y))
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn undo_goes_back_one_change_at_a_time() {
    forget_the_test_stack();
    execute_as_author("SELECT xila.create_stack('Test books')");
    execute_as_author(&format!("SELECT xila.add_field('{STACK}', 'Title')"));
    execute_as_author(&format!("SELECT xila.add_field('{STACK}', 'Year', 'number')"));
    let mut session = session_as("PGXILA_TEST_AUTHOR");
    open_the_test_stack(&mut session);

    // Three changes, as the visual editor makes them: each is an action, and
    // so a transaction and a line of history.
    session.run_with(
        "place_widgets",
        serde_json::json!({ "placements": {
            "title": { "x": 100, "y": 44, "w": 200, "h": 14 },
            "year": { "x": 100, "y": 64, "w": 60, "h": 14 }
        } }),
    );
    assert_eq!(place_of(&session, "year"), Some((100, 64)), "{:?}", session.message());
    session.run_with(
        "add_widget",
        serde_json::json!({ "kind": "text", "props": {
            "placement": { "x": 20, "y": 90, "w": 90, "h": 14 }
        } }),
    );
    assert_eq!(session.view().focus.as_deref(), Some("text1"), "the new widget is pointed at");
    session.run_with(
        "set_widget",
        serde_json::json!({ "widget": "text1", "changes": { "text": "Hello" } }),
    );
    let template = |session: &Session<Server>| {
        session.view().widgets().find(|w| w.name == "text1").map(|w| w.props["template"].clone())
    };
    assert_eq!(template(&session), Some("Hello".into()));

    // Undo walks back through them, newest first.
    session.run("undo");
    assert_eq!(session.message(), Some("Undone: Text of “text1” changed."));
    assert_eq!(template(&session), Some("Text".into()));
    session.run("undo");
    assert_eq!(session.message(), Some("Undone: “text1” added to the screen."));
    assert_eq!(template(&session), None);
    session.run("undo");
    assert_eq!(session.message(), Some("Undone: Layout changed."));
    assert_eq!(place_of(&session, "year"), None);

    // A new change starts over from the most recent line.
    session.run_with(
        "place_widgets",
        serde_json::json!({ "placements": { "title": { "x": 120, "y": 44, "w": 200, "h": 14 } } }),
    );
    assert_eq!(place_of(&session, "title"), Some((120, 44)));
    session.run("undo");
    assert_eq!(session.message(), Some("Undone: “title” moved."));
    assert_eq!(place_of(&session, "title"), None);

    // Every undo is in the history of the stack, where it can be rewound.
    session.run("stack_history");
    assert_eq!(session.view().stack(), "history");
    let lines = column(&session, "changes", "summary");
    assert_eq!(lines[0], "Screen rewound (1 change undone)");
    assert_eq!(lines[1], "“title” moved");
    assert!(lines.contains(&"Layout changed".to_string()));
    session.press("F12");

    // The change of someone else is not undone with a key.
    let mut editor = session_as("PGXILA_TEST_EDITOR");
    open_the_test_stack(&mut editor);
    assert!(editor.view().screen.stack.may_edit, "a member of the owner role may edit");
    editor.run_with(
        "place_widgets",
        serde_json::json!({ "placements": { "year": { "x": 8, "y": 120, "w": 60, "h": 14 } } }),
    );
    assert_eq!(editor.message(), None);
    session.run("undo");
    assert_eq!(
        session.message(),
        Some(
            format!(
                "The last change was made by {}: use the history of the stack to rewind it.",
                var("PGXILA_TEST_EDITOR")
            )
            .as_str()
        )
    );
    // Their own, they may undo.
    editor.run("undo");
    assert_eq!(editor.message(), Some("Undone: “year” moved."));

    forget_the_test_stack();
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn pictures_are_added_and_never_overwritten() {
    forget_the_test_stack();
    execute_as_author(&format!(
        "SELECT xila.create_stack('Test books');
         SELECT xila.add_field('{STACK}', 'Title');
         SELECT xila.add_field('{STACK}', 'Cover', 'picture');
         SELECT xila.save_card('{STACK}', NULL, '{{\"title\": \"Dune\"}}');"
    ));
    let mut session = session_as("PGXILA_TEST_AUTHOR");
    session.be_graphic();
    open_the_test_stack(&mut session);
    let drawing = |pixels: &str| {
        serde_json::json!({
            "target": "card", "widget": "cover", "width": 2, "height": 2, "pixels": pixels
        })
    };

    // The picture of the card: nothing yet, a frame of the size of a portrait.
    session.run_with("draw", serde_json::json!({ "target": "card", "widget": "cover" }));
    let open = session.view().drawing.clone().expect("the drawing editor is asked for");
    assert_eq!((open.image, open.width, open.height), (None, 68, 80));
    session.stop_drawing();

    // A first drawing, then another: the card points to a new image each time.
    assert!(session.run_with("save_drawing", drawing("0018ff07")), "{:?}", session.message());
    assert_eq!(session.message(), Some("Picture saved."));
    let first = session.view().value("cover").to_owned();
    assert_eq!(session.image(&first).map(|image| image.pixels.clone()), Some(vec![0, 24, 255, 7]));
    assert!(session.run_with("save_drawing", drawing("01010101")));
    let second = session.view().value("cover").to_owned();
    assert_ne!(second, first);

    // Drawing it again starts from the image the card has now.
    session.run_with("draw", serde_json::json!({ "target": "card", "widget": "cover" }));
    let open = session.view().drawing.clone().unwrap();
    assert_eq!((open.image.as_deref(), open.width, open.height), (Some(second.as_str()), 2, 2));
    session.stop_drawing();

    // The history of the card goes back to the first picture, which is
    // still there: an image is never changed.
    session.press("F10");
    assert_eq!(column(&session, "changes", "summary")[0], "Card “Dune” changed: cover");
    session.activate("changes");
    session.answer(true);
    assert_eq!(session.view().stack(), STACK, "{:?}", session.message());
    assert_eq!(session.view().value("cover"), first);

    // A colour of the stack, for the pictures of its cards.
    assert!(session.run_with("set_colour", serde_json::json!({ "rgb": "#AB12CD" })));
    let colours = &session.view().screen.colours;
    assert_eq!((colours[0].slot, colours[0].rgb.as_str()), (32, "#ab12cd"));
    assert!(session.run_with("save_drawing", drawing("20202020")));

    // The icon of the stack only has the colours every stack has.
    let icon = |pixels: &str| serde_json::json!({ "target": "icon", "width": 2, "height": 2, "pixels": pixels });
    assert!(!session.run_with("save_drawing", icon("20202020")));
    assert_eq!(session.message(), Some("An icon is drawn with the 32 system colours only."));
    assert!(session.run_with("save_drawing", icon("18181818")));

    // The list of stacks gives the icon of each stack, and the client reads it.
    session.press("F12");
    assert_eq!(session.view().stack(), "stacks");
    let row = column(&session, "list", "title").iter().position(|t| t == "Test books").unwrap();
    let image = column(&session, "list", "icon")[row].clone();
    assert_eq!(session.image(&image).map(|image| image.pixels.clone()), Some(vec![24; 4]));

    forget_the_test_stack();
}

#[test]
#[ignore = "needs running pgXila servers: use scripts/test-integration.sh"]
fn scripts_run_for_whoever_uses_the_stack() {
    forget_the_test_stack();
    execute_as_author(&format!(
        "SELECT xila.create_stack('Test books');
         SELECT xila.add_field('{STACK}', 'Title');
         SELECT xila.add_field('{STACK}', 'Year', 'number');
         SELECT xila.save_card('{STACK}', NULL, '{{\"title\": \"Emma\", \"year\": \"1815\"}}');
         SELECT xila.save_card('{STACK}', NULL, '{{\"title\": \"Dune\", \"year\": \"1965\"}}');
         GRANT USAGE ON SCHEMA {STACK} TO xila_user;
         GRANT SELECT, INSERT, UPDATE, DELETE ON {STACK}.cards TO xila_user;"
    ));

    // The author adds a button and writes its script, as the editors of the
    // window do: with actions.
    let mut author = session_as("PGXILA_TEST_AUTHOR");
    open_the_test_stack(&mut author);
    author.run_with("add_widget", serde_json::json!({ "kind": "button", "props": {} }));
    assert_eq!(author.view().focus.as_deref(), Some("button1"), "{:?}", author.message());
    author.run_with(
        "set_widget",
        serde_json::json!({ "widget": "button1", "changes": { "label": "Sequel" } }),
    );
    let source = r#"keys = { F2 = "Sequel" }

function on_click()
  local sequel = stack.new_card()
  sequel.title = card.title .. " II"
  sequel.year = card.year + 1
  save(sequel)
  go_to(sequel)
  message("Written by " .. user .. ".")
end

function on_key(key)
  on_click()
end
"#;
    assert!(author
        .run_with("save_script", serde_json::json!({ "widget": "button1", "source": source })));
    assert_eq!(author.message(), Some("Script saved."));
    // The screen says so at once: the button runs its script, and so does F2.
    let button = |session: &Session<Server>| {
        session.view().widgets().find(|w| w.name == "button1").and_then(|w| w.action.clone())
    };
    assert_eq!(button(&author).as_deref(), Some("script"));
    assert!(keys(&author).contains(&"F2=Sequel".to_owned()), "{:?}", keys(&author));

    // A test keeps nothing, and tells what the script would do.
    author.be_graphic();
    author.run_with("edit_script", serde_json::json!({ "widget": "button1" }));
    let scripting = author.view().scripting.clone().expect("the script editor is open");
    assert_eq!(scripting.title, "Script: button “Sequel”");
    assert_eq!(scripting.source, source);
    author.run_with(
        "test_script",
        serde_json::json!({ "widget": "button1", "source": source, "event": { "on": "click" } }),
    );
    let report = author.view().scripting.as_ref().unwrap().report.clone().expect("a report");
    assert!(report.ok, "{report:?}");
    assert!(report.lines.contains(&"In the database: 1 card saved.".to_owned()), "{report:?}");
    assert!(report.lines.contains(&"This was a test: nothing was kept.".to_owned()));
    author.stop_scripting();
    assert_eq!(author.view().card.count, 2, "the test wrote nothing");

    // A plain user presses the button: the script acts with their rights, on
    // the card they have on screen, and its changes are theirs.
    let mut user = session_as("PGXILA_TEST_USER");
    open_the_test_stack(&mut user);
    assert_eq!(user.view().value("title"), "Emma");
    user.activate("button1");
    assert_eq!(user.message(), Some(format!("Written by {}.", user.hello().role).as_str()));
    assert_eq!(user.view().value("title"), "Emma II");
    assert_eq!(user.view().value("year"), "1816");
    assert_eq!(user.view().card.count, 3);
    // The key does the same, from the card it is pressed on.
    assert!(user.press("F2"));
    assert_eq!(user.view().value("title"), "Emma II II");
    user.press("F10");
    let changes = column(&user, "changes", "summary");
    assert_eq!(changes[..2], ["Card “Emma II II” added", "Card “Emma II” added"]);
    user.press("F12");

    // A user may not change the script, and may read it.
    assert!(!user.run_with("save_script", serde_json::json!({ "widget": "button1", "source": "" })));
    let refusal = user.message().expect("a refusal").to_owned();
    assert!(refusal.starts_with("The stack “Test books” belongs to "), "{refusal}");

    // A script that fails says where, and leaves nothing behind.
    let broken = "function on_click()\n  save(stack.new_card())\n  error(\"no more paper\")\nend";
    assert!(author
        .run_with("save_script", serde_json::json!({ "widget": "button1", "source": broken })));
    user.activate("button1");
    assert_eq!(user.message(), Some("The script of “Sequel” failed at line 3: no more paper"));
    assert_eq!(user.view().card.count, 4);

    // The script of the screen greets each card that comes on screen.
    let greeting = "function on_open()\n  message(\"This is \" .. card.title .. \".\")\nend";
    assert!(author.run_with("save_script", serde_json::json!({ "source": greeting })));
    let mut user = session_as("PGXILA_TEST_USER");
    open_the_test_stack(&mut user);
    assert_eq!(user.message(), Some("This is Emma."));
    user.press("F8");
    assert_eq!(user.message(), Some("This is Dune."));

    // The history of the stack has a line for each script, and undo takes the
    // last one back.
    author.run("author_form");
    assert_eq!(author.view().stack(), "author", "{:?}", author.message());
    author.press("F10");
    let lines = column(&author, "changes", "summary");
    let scripts: Vec<&str> =
        lines.iter().map(String::as_str).filter(|line| line.starts_with("Script")).collect();
    assert_eq!(
        scripts,
        [
            "Script of the screen written",
            "Script of “button1” changed",
            "Script of “button1” written"
        ]
    );

    forget_the_test_stack();
}
