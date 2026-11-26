//! A list shown as a conversation.
//!
//! A grid whose properties say `"view": "talk"` is not a table of rows: each
//! row is a line that someone said. The server gives the rows newest first;
//! the clients write them from the bottom up, wrapped to the width they
//! have. What the two clients share is here: whose line a row is, and how a
//! text is cut into lines.
//!
//! A row may also be a change the agent proposes. The person applies it or
//! refuses it with an action of the screen, which the grid names.

use crate::api::{GridPage, Screen, Widget};

/// Whose line it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    /// The person at the screen.
    User,
    /// The agent, in words.
    Agent,
    /// Something the agent did.
    Tool,
    /// Something that happened, said to everyone.
    Note,
    /// Something that went wrong.
    Error,
    /// The agent is working.
    Wait,
    /// A change the agent proposes.
    Proposal(State),
}

/// What became of a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Pending,
    Applied,
    Refused,
}

/// One line of a conversation, as said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line<'a> {
    pub speaker: Speaker,
    /// What is said. For a proposal: what it does on the first line, then
    /// what there is to know about it.
    pub text: &'a str,
    /// For a proposal: the calls that applying it runs.
    pub code: &'a str,
    /// The key of the row, which names a proposal to the server.
    pub key: &'a str,
}

/// The actions a conversation offers on a proposal, with the key of the
/// screen that runs each, if there is one.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Offers {
    pub apply: Option<Offer>,
    pub refuse: Option<Offer>,
    pub show: Option<Offer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    /// The action to run, with `{"proposal": key}` as what the tool says.
    pub action: String,
    pub label: String,
    pub key: Option<String>,
}

/// What a conversation offers to do with a proposal, as its grid names it.
pub fn offers(widget: &Widget, screen: &Screen) -> Offers {
    let offer = |property: &str| {
        let action = widget.props.get(property)?.as_str()?;
        let label = screen.actions.iter().find(|known| known.name == action)?.label.clone();
        let key = screen.keys.iter().find(|bound| bound.action == action).map(|k| k.key.clone());
        Some(Offer { action: action.to_owned(), label, key })
    };
    Offers { apply: offer("apply"), refuse: offer("refuse"), show: offer("show") }
}

/// The lines of a conversation, oldest first.
pub fn lines<'a>(widget: &Widget, page: &'a GridPage) -> Vec<Line<'a>> {
    let who = widget.talk().unwrap_or("who");
    // What was said is in the first column that does not say who said it.
    let said: &str = widget
        .props
        .get("columns")
        .and_then(|columns| columns.as_array())
        .into_iter()
        .flatten()
        .filter_map(|column| column.get("column")?.as_str())
        .find(|name| *name != who)
        .unwrap_or("text");
    // The column a property of the grid names, or else one of that name.
    fn named<'w>(widget: &'w Widget, property: &'static str) -> &'w str {
        widget.props.get(property).and_then(|name| name.as_str()).unwrap_or(property)
    }
    let (state, code) = (named(widget, "state"), named(widget, "code"));
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
                "note" => Speaker::Note,
                "error" => Speaker::Error,
                "wait" => Speaker::Wait,
                "proposal" => Speaker::Proposal(match cell(row, state) {
                    "applied" => State::Applied,
                    "refused" => State::Refused,
                    _ => State::Pending,
                }),
                _ => Speaker::Agent,
            },
            text: cell(row, said),
            code: cell(row, code),
            key: &row.key,
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
        let said: Vec<(Speaker, &str)> =
            lines(&widget, &page).iter().map(|line| (line.speaker, line.text)).collect();
        assert_eq!(
            said,
            [
                (Speaker::User, "Hello"),
                (Speaker::Tool, "Read a card."),
                (Speaker::Wait, "Working…"),
            ]
        );
    }

    #[test]
    fn a_row_may_be_a_proposal_with_its_state_and_what_the_screen_offers() {
        let screen: Screen = serde_json::from_value(serde_json::json!({
            "stack": { "name": "agent", "title": "Agent", "kind": "system" },
            "source": { "has_cards": true, "count": 1 },
            "widgets": [{
                "name": "talk", "kind": "grid", "ord": 1,
                "props": {"view": "talk", "who": "kind", "state": "state", "code": "code",
                          "apply": "apply", "refuse": "refuse", "show": "view",
                          "columns": [{"column": "kind"}, {"column": "text"},
                                      {"column": "state"}, {"column": "code"}]}
            }],
            "actions": [{"name": "apply", "label": "Apply"}, {"name": "refuse", "label": "Refuse"},
                        {"name": "view", "label": "View"}],
            "keys": [{"key": "F2", "label": "Apply", "action": "apply"},
                     {"key": "F4", "label": "Refuse", "action": "refuse"}]
        }))
        .unwrap();
        let widget = &screen.widgets[0];
        let page: GridPage = serde_json::from_value(serde_json::json!({
            "total": 2,
            "rows": [
                {"key": "n1", "cells": {"kind": "note", "text": "Applied: done.", "state": null}},
                {"key": "p1", "cells": {"kind": "proposal", "text": "Add a field\nA date.",
                                        "state": "applied", "code": "xila.add_field()"}}
            ]
        }))
        .unwrap();
        let said = lines(widget, &page);
        assert_eq!(said[0].speaker, Speaker::Proposal(State::Applied));
        assert_eq!((said[0].key, said[0].code), ("p1", "xila.add_field()"));
        assert_eq!(said[1].speaker, Speaker::Note);

        let offered = offers(widget, &screen);
        let apply = offered.apply.unwrap();
        assert_eq!((apply.action.as_str(), apply.key.as_deref()), ("apply", Some("F2")));
        // An action without a key is still offered: the window has a button.
        let show = offered.show.unwrap();
        assert_eq!((show.label.as_str(), show.key), ("View", None));
    }
}
