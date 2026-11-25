//! pgXila text client: the screens of a pgXila server in a terminal, in the
//! manner of an AS/400 display.
//!
//! The client holds no business logic. It draws what the server describes on
//! an 80 by 24 character grid and sends back what the user does.

pub mod input;
pub mod login;
pub mod screen;
pub mod theme;
