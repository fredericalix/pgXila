//! pgXila graphical client: the screens of a pgXila server in pixel art.
//!
//! The client holds no business logic. It draws what the server describes on
//! a screen of 480 by 300 pixels, one byte per pixel, with its own fonts and
//! the colours of `xila.palette`, and sends back what the user does. The
//! window enlarges that screen by a whole factor.
//!
//! Everything but the window itself lives in this library, so that the
//! screens can be drawn and driven without a display.

pub mod app;
pub mod chrome;
pub mod design;
pub mod draw;
pub mod font;
pub mod frame;
pub mod layout;
pub mod login;
pub mod palette;
pub mod present;
pub mod screen;
pub mod script;

#[cfg(test)]
mod testing;
