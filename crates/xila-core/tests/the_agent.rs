//! Integration tests of the agent against a real pgXila server: the worker,
//! the tools and the conversation, with a fake model in the place of the
//! real one.
//!
//! They run against the throwaway "scratch" server and are ignored by
//! default. Run them with `scripts/test-integration.sh`, which tells the
//! worker of that server where the fake model listens.
//!
//! This file is named so that its tests run after the others of this crate:
//! one of them builds a stack and deletes it, which the tests of author mode
//! would find in the list of deleted stacks.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::json;
use xila_core::connection::ConnectParams;
use xila_core::server::Server;
use xila_core::session::Session;
use xila_core::talk::{self, Speaker};

#[path = "support/fake_model.rs"]
mod fake_model;
use fake_model::FakeModel;

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

/// The lines of the conversation on screen, oldest first.
fn said(session: &Session<Server>) -> Vec<(Speaker, String)> {
    let view = session.view();
    let widget = view.screen.widgets.iter().find(|w| w.name == "talk").expect("the conversation");
    talk::lines(widget, &view.grids["talk"].page)
        .into_iter()
        .map(|line| (line.speaker, line.text.to_owned()))
        .collect()
}

/// Lets the worker work until the agent has stopped working, as the client
/// does: it only reads the screen again when the server says something
/// changed.
fn wait_for_the_answer(session: &mut Session<Server>) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        session.poll();
        let lines = said(session);
        if lines.last().is_some_and(|(speaker, _)| *speaker != Speaker::Wait)
            && lines.iter().any(|(speaker, _)| matches!(speaker, Speaker::Agent | Speaker::Error))
            && !lines.iter().any(|(speaker, _)| *speaker == Speaker::Wait)
        {
            return;
        }
        assert!(Instant::now() < deadline, "the agent did not answer: {lines:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Opens the agent from the address book, on a new conversation.
fn with_the_agent(session: &mut Session<Server>) {
    session.run("open_stacks");
    session.activate("list");
    assert_eq!(session.view().stack(), "addressbook");
    assert!(session.press("F9"), "F9 calls the agent from any screen");
    assert_eq!(session.view().stack(), "agent");
    assert!(session.press("F6"));
    assert_eq!(said(session), []);
}

fn ask(session: &mut Session<Server>, question: &str) {
    session.edit("ask", question.to_owned());
    assert!(session.press("Enter"));
    assert_eq!(session.message(), None);
    // The question is on screen at once; the answer is the worker's business.
    assert_eq!(session.view().value("ask"), "");
}

#[test]
#[ignore = "needs the scratch pgXila server and the fake model"]
fn the_agent_reads_the_stacks_to_answer() {
    let model = FakeModel::start();
    model.will_call("find_cards", json!({"stack": "addressbook", "text": "london"}));
    model.will_say("Two people live in London: Ada Lovelace and Charles Babbage.");

    let mut session = session_as("PGXILA_TEST_USER");
    with_the_agent(&mut session);
    assert_eq!(session.view().text("about"), "Looking at Address book");
    assert_eq!(session.view().text("model"), "fake");
    assert_eq!(session.view().text("host"), "host.docker.internal");

    ask(&mut session, "Who lives in London?");
    wait_for_the_answer(&mut session);
    assert_eq!(
        said(&session),
        [
            (Speaker::User, "Who lives in London?".to_owned()),
            (Speaker::Tool, "Looked for “london” in Address book: 2 cards.".to_owned()),
            (
                Speaker::Agent,
                "Two people live in London: Ada Lovelace and Charles Babbage.".to_owned()
            ),
        ]
    );
    // What the agent did today is counted on its screen.
    let today = session.view().text("today").to_owned();
    assert!(today.ends_with(", 0 refused") && !today.starts_with("0 "), "{today}");

    // What the worker sent: the token of the server, never in the database;
    // the question; the tools; then what the tool found.
    let requests = model.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    for request in &requests {
        assert_eq!(request.path, "/v1/chat/completions");
        assert_eq!(request.authorization.as_deref(), Some("Bearer test-token-for-the-fake-model"));
        assert_eq!(request.body["model"], "fake");
    }
    let first = &requests[0].body;
    assert_eq!(first["messages"][0]["role"], "system");
    let briefing = first["messages"][0]["content"].as_str().unwrap();
    assert!(
        briefing.contains("looking at the stack “Address book” (name: addressbook)"),
        "{briefing}"
    );
    assert_eq!(first["messages"][1], json!({"role": "user", "content": "Who lives in London?"}));
    let tools: Vec<&str> = first["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(tools, ["list_stacks", "find_cards", "read_card"]);

    let second = &requests[1].body["messages"];
    assert_eq!(second[2]["tool_calls"][0]["function"]["name"], "find_cards");
    assert_eq!(second[3]["role"], "tool");
    let found: serde_json::Value =
        serde_json::from_str(second[3]["content"].as_str().unwrap()).unwrap();
    assert_eq!(found["total"], 2);
    let names: Vec<&str> = found["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|card| card["values"]["last_name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Babbage", "Lovelace"]);

    // The journal says what was done; the list of conversations has this one.
    assert!(session.press("F10"));
    assert_eq!(session.view().stack(), "conversations");
    let first_row = &session.view().grids["list"].page.rows[0];
    assert_eq!(first_row.cells["title"].as_deref(), Some("Who lives in London?"));
    assert!(session.press("F7"));
    assert_eq!(session.view().stack(), "agent_journal");
    let journal: Vec<String> = session.view().grids["list"]
        .page
        .rows
        .iter()
        .take(3)
        .map(|row| row.cells["summary"].clone().unwrap_or_default())
        .collect();
    assert!(journal[0].starts_with("Model: "), "{journal:?}");
    assert_eq!(journal[1], "Looked for “london” in Address book: 2 cards.");
    assert!(journal[2].starts_with("Model: "), "{journal:?}");
    // The settings are not for everyone.
    assert!(session.press("F12"));
    assert!(!session.press("F4"));
}

#[test]
#[ignore = "needs the scratch pgXila server and the fake model"]
fn a_model_that_fails_is_said_in_the_conversation() {
    let model = FakeModel::start();
    model.will_answer(401, json!({"error": {"message": "No auth credentials found"}}));

    let mut session = session_as("PGXILA_TEST_USER");
    with_the_agent(&mut session);
    ask(&mut session, "Hello?");
    wait_for_the_answer(&mut session);
    let lines = said(&session);
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert_eq!(lines[1].0, Speaker::Error);
    assert!(lines[1].1.starts_with("The model refused the token (HTTP 401)."), "{lines:?}");

    // The conversation goes on: the next question is answered.
    model.will_say("Hello.");
    ask(&mut session, "Hello again?");
    wait_for_the_answer(&mut session);
    assert_eq!(said(&session).last().unwrap(), &(Speaker::Agent, "Hello.".to_owned()));
    // What went wrong was for the person, not for the model.
    let sent = model.requests().last().unwrap().body["messages"].to_string();
    assert!(!sent.contains("refused the token"), "{sent}");
}

#[test]
#[ignore = "needs the scratch pgXila server and the fake model"]
fn the_agent_does_not_read_what_is_closed_to_it_nor_what_the_person_cannot_open() {
    let model = FakeModel::start();
    let mut author = session_as("PGXILA_TEST_AUTHOR");
    let mut user = session_as("PGXILA_TEST_USER");

    // The author builds a stack: the user cannot open it.
    author.run("open_stacks");
    assert!(author.press("F6"));
    author.edit("title", "Agent secrets".into());
    assert!(author.press("Enter"));
    assert_eq!(author.message(), Some("Stack created. F6 adds a field."));

    model.will_call("list_stacks", json!({}));
    model.will_call("find_cards", json!({"stack": "agent_secrets"}));
    model.will_say("I do not see such a stack.");
    with_the_agent(&mut user);
    ask(&mut user, "What is in Agent secrets?");
    wait_for_the_answer(&mut user);
    let requests = model.requests();
    let listed = requests[1].body["messages"][3]["content"].as_str().unwrap().to_owned();
    assert!(listed.contains("addressbook"), "{listed}");
    assert!(!listed.contains("agent_secrets"), "{listed}");
    let refused = requests[2].body["messages"][5]["content"].as_str().unwrap().to_owned();
    assert!(refused.contains("The stack “agent_secrets” does not exist."), "{refused}");

    // For its author it is there; once closed to the agent, only its name is.
    let mut author_talk = session_as("PGXILA_TEST_AUTHOR");
    model.will_call("list_stacks", json!({}));
    model.will_say("You have two stacks.");
    with_the_agent(&mut author_talk);
    ask(&mut author_talk, "Which stacks do I have?");
    wait_for_the_answer(&mut author_talk);
    let listed = model.requests().last().unwrap().body["messages"][3]["content"].to_string();
    assert!(listed.contains("agent_secrets") && !listed.contains("closed"), "{listed}");

    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let as_author = |statements: &'static str| {
        runtime.block_on(async {
            let (client, connection) =
                xila_core::connection::connect(&params("PGXILA_TEST_AUTHOR")).await.unwrap();
            tokio::spawn(connection);
            client.batch_execute(statements).await.unwrap();
        });
    };
    as_author(r#"SELECT xila.set_stack('agent_secrets', '{"agent": false}')"#);
    model.will_call("list_stacks", json!({}));
    model.will_call("find_cards", json!({"stack": "agent_secrets"}));
    model.will_say("That stack is closed to me.");
    ask(&mut author_talk, "And now?");
    wait_for_the_answer(&mut author_talk);
    let requests = model.requests();
    let count = requests.len();
    let listed = requests[count - 2].body["messages"].to_string();
    assert!(listed.contains(r#"\"closed\": true"#), "{listed}");
    let refused = requests[count - 1].body["messages"].to_string();
    assert!(refused.contains("is closed to the agent"), "{refused}");
    assert_eq!(author_talk.view().text("today"), "2 actions, 1 refused");

    as_author(
        "DELETE FROM xila.stack WHERE name = 'agent_secrets'; DROP SCHEMA agent_secrets CASCADE",
    );
}
