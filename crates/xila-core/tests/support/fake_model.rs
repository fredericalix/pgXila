//! A fake language model for the integration tests: a small HTTP server that
//! speaks just enough of the OpenAI chat API.
//!
//! The agent worker of the scratch server is set up to call it. A test says
//! in advance what the model will answer, and reads afterwards what the
//! worker sent. No test ever calls a real model.

// Each test file that includes this one uses a part of it.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

/// What the worker sent to the model.
#[derive(Debug, Clone)]
pub struct Request {
    pub path: String,
    pub authorization: Option<String>,
    pub body: Value,
}

type Answers = Arc<Mutex<VecDeque<(u16, String)>>>;

pub struct FakeModel {
    answers: Answers,
    requests: Arc<Mutex<Vec<Request>>>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl FakeModel {
    /// Starts listening where the scratch server was told the model is.
    pub fn start() -> FakeModel {
        let port = std::env::var("PGXILA_TEST_MODEL_PORT")
            .expect("PGXILA_TEST_MODEL_PORT is not set: use scripts/test-integration.sh");
        let bind = std::env::var("PGXILA_TEST_MODEL_BIND").unwrap_or_else(|_| "127.0.0.1".into());
        let listener =
            TcpListener::bind(format!("{bind}:{port}")).expect("the port of the fake model");
        listener.set_nonblocking(true).unwrap();

        let answers: Answers = Arc::default();
        let requests: Arc<Mutex<Vec<Request>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::spawn({
            let (answers, requests, stop) = (answers.clone(), requests.clone(), stop.clone());
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => serve(stream, &answers, &requests),
                        Err(_) => std::thread::sleep(Duration::from_millis(10)),
                    }
                }
            }
        });
        FakeModel { answers, requests, stop, thread: Some(thread) }
    }

    /// The next answer: words.
    pub fn will_say(&self, words: &str) {
        self.will_answer(
            200,
            json!({
                "model": "fake",
                "choices": [{"finish_reason": "stop",
                             "message": {"role": "assistant", "content": words}}],
                "usage": {"prompt_tokens": 100, "completion_tokens": 10}
            }),
        );
    }

    /// The next answer: a call to a tool.
    pub fn will_call(&self, tool: &str, arguments: Value) {
        self.will_answer(200, json!({
            "model": "fake",
            "choices": [{"finish_reason": "tool_calls", "message": {
                "role": "assistant", "content": null,
                "tool_calls": [{"id": "call_1", "type": "function",
                                "function": {"name": tool, "arguments": arguments.to_string()}}]}}],
            "usage": {"prompt_tokens": 100, "completion_tokens": 10}
        }));
    }

    /// The next answer: an HTTP status and a body.
    pub fn will_answer(&self, status: u16, body: Value) {
        self.answers.lock().unwrap().push_back((status, body.to_string()));
    }

    /// What the worker sent so far.
    pub fn requests(&self) -> Vec<Request> {
        self.requests.lock().unwrap().clone()
    }
}

impl Drop for FakeModel {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(stream: TcpStream, answers: &Answers, requests: &Mutex<Vec<Request>>) {
    stream.set_nonblocking(false).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut reader = BufReader::new(stream);

    let mut first = String::new();
    if reader.read_line(&mut first).is_err() {
        return;
    }
    let path = first.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let mut length = 0;
    let mut authorization = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            match name.to_ascii_lowercase().as_str() {
                "content-length" => length = value.trim().parse().unwrap_or(0),
                "authorization" => authorization = Some(value.trim().to_owned()),
                _ => {}
            }
        }
    }
    let mut body = vec![0; length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let body = serde_json::from_slice(&body).unwrap_or(Value::Null);
    requests.lock().unwrap().push(Request { path, authorization, body });

    let (status, answer) = answers.lock().unwrap().pop_front().unwrap_or_else(|| {
        (500, json!({"error": {"message": "the fake model has nothing to say"}}).to_string())
    });
    let mut stream = reader.into_inner();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fake\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n{answer}",
        answer.len()
    );
}
