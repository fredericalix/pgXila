//! A connection to a pgXila server, as blocking calls.
//!
//! User interfaces run a plain loop: read a key, call the server, draw. This
//! type hides the asynchronous driver behind such calls. A background thread
//! keeps the connection alive between calls and collects the notifications
//! the server pushes.

use std::sync::mpsc;

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use tokio_postgres::types::{Json, ToSql};
use tokio_postgres::AsyncMessage;

use crate::api::{
    Api, ApiError, Card, Context, Effect, GridPage, Hello, Image, Move, Notification,
    PaletteColour, Screen,
};
use crate::connection::{connect, ConnectError, ConnectParams};

pub struct Server {
    runtime: tokio::runtime::Runtime,
    client: tokio_postgres::Client,
    notifications: mpsc::Receiver<Notification>,
}

impl Server {
    /// Connects, over TLS 1.3 and nothing else, and starts listening to the
    /// notification channels.
    pub fn connect(params: &ConnectParams) -> Result<Self, ConnectError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(ConnectError::Runtime)?;

        let (client, mut connection) = runtime.block_on(connect(params))?;

        let (sender, notifications) = mpsc::channel();
        runtime.spawn(async move {
            let mut messages =
                futures_util::stream::poll_fn(move |context| connection.poll_message(context));
            while let Some(Ok(message)) = messages.next().await {
                if let AsyncMessage::Notification(raw) = message {
                    if let Some(notification) = Notification::parse(raw.channel(), raw.payload()) {
                        if sender.send(notification).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        runtime.block_on(client.batch_execute("LISTEN xila_card; LISTEN xila_screen"))?;
        Ok(Server { runtime, client, notifications })
    }

    /// Runs a query that returns one JSON value and decodes it.
    fn call<T: DeserializeOwned>(
        &self,
        query: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<T, ApiError> {
        let row = self.runtime.block_on(self.client.query_one(query, params)).map_err(api_error)?;
        let value: serde_json::Value = row.try_get(0).map_err(api_error)?;
        serde_json::from_value(value)
            .map_err(|error| ApiError::Other(format!("unexpected answer from the server: {error}")))
    }
}

/// Errors of SQLSTATE class XL are written for the user; the rest is not.
fn api_error(error: tokio_postgres::Error) -> ApiError {
    match error.as_db_error() {
        Some(db) if db.code().code().starts_with("XL") => {
            ApiError::User { code: db.code().code().to_owned(), message: db.message().to_owned() }
        }
        Some(db) => ApiError::Other(db.message().to_owned()),
        None => ApiError::Other(error.to_string()),
    }
}

impl Api for Server {
    fn hello(&mut self) -> Result<Hello, ApiError> {
        self.call("SELECT xila.hello()", &[])
    }

    fn palette(&mut self) -> Result<Vec<PaletteColour>, ApiError> {
        let rows = self
            .runtime
            .block_on(self.client.query("SELECT slot, name, rgb FROM xila.palette()", &[]))
            .map_err(api_error)?;
        Ok(rows
            .iter()
            .map(|row| PaletteColour {
                slot: row.get::<_, i16>("slot") as u8,
                name: row.get("name"),
                rgb: row.get("rgb"),
            })
            .collect())
    }

    fn image(&mut self, id: &str) -> Result<Option<Image>, ApiError> {
        let row =
            self.runtime
                .block_on(self.client.query_opt(
                    "SELECT width, height, pixels FROM xila.image($1::text::uuid)",
                    &[&id],
                ))
                .map_err(api_error)?;
        Ok(row.map(|row| Image {
            width: row.get::<_, i16>("width") as u16,
            height: row.get::<_, i16>("height") as u16,
            pixels: row.get("pixels"),
        }))
    }

    fn open_stack(&mut self, stack: &str) -> Result<Screen, ApiError> {
        self.call("SELECT xila.open_stack($1)", &[&stack])
    }

    fn card(&mut self, stack: &str, key: Option<&str>, to: Move) -> Result<Card, ApiError> {
        self.call("SELECT xila.card($1, $2, $3)", &[&stack, &key, &to.as_str()])
    }

    fn grid_rows(&mut self, stack: &str, widget: &str) -> Result<GridPage, ApiError> {
        self.call("SELECT xila.grid_rows($1, $2)", &[&stack, &widget])
    }

    fn act(
        &mut self,
        stack: &str,
        action: &str,
        context: &Context,
    ) -> Result<Vec<Effect>, ApiError> {
        self.call("SELECT xila.act($1, $2, $3)", &[&stack, &action, &Json(context)])
    }

    fn notifications(&mut self) -> Vec<Notification> {
        self.notifications.try_iter().collect()
    }
}
