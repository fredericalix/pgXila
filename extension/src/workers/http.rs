//! The one HTTP request the agent worker makes: a conversation sent to an
//! endpoint that speaks the OpenAI chat API.
//!
//! This code runs on a thread of the worker and touches nothing of
//! PostgreSQL. It never logs, and what it gives back never holds the token.

use std::time::Duration;

/// How long the model may take to answer.
const PATIENCE: Duration = Duration::from_secs(120);
/// The most an answer may weigh.
const MOST_BYTES: u64 = 8 * 1024 * 1024;

/// What came back: the HTTP status and the body, or a negative status and a
/// word that says what failed (`timeout`, `token`, `address`, `tls`,
/// `unreachable`).
pub struct Outcome {
    pub status: i32,
    pub body: String,
}

fn failure(word: &str) -> Outcome {
    Outcome { status: -1, body: word.to_owned() }
}

/// True for an address the worker may call: `https://` anywhere, and plain
/// `http://` for this machine only. The same rule guards the settings table;
/// it is checked again here, where the token is about to leave.
pub fn allowed(url: &str) -> bool {
    if let Some(rest) = url.strip_prefix("https://") {
        let authority = rest.split('/').next().unwrap_or("");
        return !authority.is_empty() && !authority.contains('@');
    }
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(end) = authority.find(']') {
        // An IPv6 address, in brackets, then maybe a port.
        let after = &authority[end + 1..];
        if !(after.is_empty() || is_port(after)) {
            return false;
        }
        &authority[..=end]
    } else {
        match authority.split_once(':') {
            Some((host, port)) if is_port(&format!(":{port}")) => host,
            Some(_) => return false,
            None => authority,
        }
    };
    matches!(host, "localhost" | "127.0.0.1" | "[::1]" | "host.docker.internal")
}

fn is_port(text: &str) -> bool {
    text.strip_prefix(':')
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Sends `body` to `url`. The token is read from the environment variable
/// named by `token_variable`; an empty name means the endpoint needs none.
pub fn post(url: &str, token_variable: &str, body: &str) -> Outcome {
    if !allowed(url) {
        return failure("address");
    }
    let token = if token_variable.is_empty() {
        None
    } else {
        match std::env::var(token_variable) {
            Ok(value) if !value.trim().is_empty() => Some(value.trim().to_owned()),
            _ => return failure("token"),
        }
    };

    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(PATIENCE))
        .http_status_as_error(false)
        // The token is for this address and for no other.
        .max_redirects(0)
        .https_only(url.starts_with("https://"))
        .user_agent(concat!("pgxila/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();

    let mut request = agent.post(url).header("Content-Type", "application/json");
    if let Some(token) = &token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    match request.send(body) {
        Ok(mut response) => {
            let status = i32::from(response.status().as_u16());
            match response.body_mut().with_config().limit(MOST_BYTES).read_to_string() {
                // A zero byte cannot be stored as text.
                Ok(text) => Outcome { status, body: text.replace('\0', "") },
                Err(ureq::Error::Timeout(_)) => failure("timeout"),
                Err(_) => Outcome { status, body: String::new() },
            }
        }
        // The error of the library may name the address: only its kind is kept.
        Err(ureq::Error::Timeout(_)) => failure("timeout"),
        Err(ureq::Error::Tls(_) | ureq::Error::Rustls(_)) => failure("tls"),
        Err(_) => failure("unreachable"),
    }
}

#[cfg(test)]
mod tests {
    use super::allowed;

    #[test]
    fn only_https_leaves_the_machine() {
        for url in [
            "https://openrouter.ai/api/v1/chat/completions",
            "https://example.org:8443/v1/chat/completions",
            "http://localhost:1234/v1/chat/completions",
            "http://127.0.0.1/v1/chat/completions",
            "http://[::1]:1234/chat/completions",
            "http://host.docker.internal:1234/v1/chat/completions",
        ] {
            assert!(allowed(url), "{url}");
        }
        for url in [
            "http://openrouter.ai/api/v1/chat/completions",
            "http://192.168.1.10:1234/v1/chat/completions",
            "http://localhost.example.org/v1",
            "http://localhost@example.org/v1",
            "http://localhost:80@example.org/v1",
            "https://user@example.org/v1",
            "https:///v1",
            "ftp://localhost/v1",
            "localhost:1234/v1",
            "",
        ] {
            assert!(!allowed(url), "{url}");
        }
    }
}
