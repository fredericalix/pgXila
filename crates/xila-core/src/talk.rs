//! A list shown as a conversation.
//!
//! A grid whose properties say `"view": "talk"` is not a table of rows: each
//! row is a line that someone said. The server gives the rows newest first;
//! the clients write them from the bottom up, wrapped to the width they
//! have. What the two clients share is here: whose line a row is, and how a
//! text is cut into lines.

use crate::api::{GridPage, Widget};

/// Whose line it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    /// The person at the screen.
    User,
    /// The agent, in words.
    Agent,
    /// Something the agent did.
    Tool,
    /// Something that went wrong.
    Error,
    /// The agent is working.
    Wait,
}

/// One line of a conversation, as said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line<'a> {
    pub speaker: Speaker,
    pub text: &'a str,
}

/// The lines of a conversation, oldest first.
pub fn lines<'a>(widget: &Widget, page: &'a GridPage) -> Vec<Line<'a>> {
    let who = widget.talk().unwrap_or("who");
    // What was said is in the first column that does not say who said it.
    let said = widget
        .props
        .get("columns")
        .and_then(|columns| columns.as_array())
        .into_iter()
        .flatten()
        .filter_map(|column| column.get("column")?.as_str())
        .find(|name| *name != who)
        .unwrap_or("text");
    let cell = |row: &'a crate::api::GridRow, name: &str| {
        row.cells.get(name).and_then(|value| value.as_deref()).unwrap_or("")
    };
    page.rows
        .iter()
        .rev()
        .map(|row| Line {
            speaker: match cell(row, who) {
                "user" => Speaker::User,
                "tool" => Speaker::Tool,
                "error" => Speaker::Error,
                "wait" => Speaker::Wait,
                _ => Speaker::Agent,
            },
            text: cell(row, said),
        })
        .collect()
}

/// Cuts a text into lines no wider than `limit`, at spaces where it can.
/// `width_of` measures a piece of text: in characters for the terminal, in
/// pixels for the window. A line break in the text is kept; a word wider
/// than a line is cut.
pub fn wrap(text: &str, limit: usize, width_of: impl Fn(&str) -> usize) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line = String::new();
        for word in paragraph.split(' ').filter(|word| !word.is_empty()) {
            let joined = if line.is_empty() { word.to_owned() } else { format!("{line} {word}") };
            if width_of(&joined) <= limit {
                line = joined;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // A word that does not fit on a line of its own is cut.
            let mut rest = word;
            while width_of(rest) > limit {
                let mut end = 0;
                for (index, c) in rest.char_indices() {
                    let next = index + c.len_utf8();
                    if width_of(&rest[..next]) > limit && end > 0 {
                        break;
                    }
                    end = next;
                }
                lines.push(rest[..end].to_owned());
                rest = &rest[end..];
            }
            line = rest.to_owned();
        }
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chars(text: &str) -> usize {
        text.chars().count()
    }

    #[test]
    fn a_text_is_cut_at_spaces() {
        assert_eq!(wrap("one two three four", 9, chars), ["one two", "three", "four"]);
        assert_eq!(wrap("short", 9, chars), ["short"]);
        assert_eq!(wrap("", 9, chars), [""]);
    }

    #[test]
    fn line_breaks_are_kept_and_long_words_are_cut() {
        assert_eq!(wrap("first\n\nsecond line", 20, chars), ["first", "", "second line"]);
        assert_eq!(wrap("abcdefghij kl", 4, chars), ["abcd", "efgh", "ij", "kl"]);
        assert_eq!(wrap("é".repeat(5).as_str(), 2, chars), ["éé", "éé", "é"]);
    }

    #[test]
    fn the_rows_of_a_conversation_are_read_oldest_first() {
        let widget: Widget = serde_json::from_value(serde_json::json!({
            "name": "talk", "kind": "grid", "ord": 1,
            "props": {"view": "talk", "who": "kind",
                      "columns": [{"column": "kind"}, {"column": "text"}]}
        }))
        .unwrap();
        let page: GridPage = serde_json::from_value(serde_json::json!({
            "total": 3,
            "rows": [
                {"key": "3", "cells": {"kind": "wait", "text": "Working…"}},
                {"key": "2", "cells": {"kind": "tool", "text": "Read a card."}},
                {"key": "1", "cells": {"kind": "user", "text": "Hello"}}
            ]
        }))
        .unwrap();
        assert_eq!(
            lines(&widget, &page),
            [
                Line { speaker: Speaker::User, text: "Hello" },
                Line { speaker: Speaker::Tool, text: "Read a card." },
                Line { speaker: Speaker::Wait, text: "Working…" },
            ]
        );
    }
}
