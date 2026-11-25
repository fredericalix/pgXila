//! Shared core of the pgXila clients.
//!
//! Everything the text client and the graphical client have in common lives
//! here: the connection to the server, the API, the state of a session and
//! the default placement of widgets. Only drawing and input differ between
//! the clients.

pub mod api;
pub mod connection;
pub mod server;
pub mod session;
pub mod text_layout;
