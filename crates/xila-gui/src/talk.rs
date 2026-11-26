//! A list shown as a conversation: bubbles, written from the bottom up.
//!
//! What the person said is on the right, what the agent answered on the left
//! next to its face, and what the agent did in between, in small print. The
//! rows come from the server newest first; see [`xila_core::talk`].

use xila_core::api::Widget;
use xila_core::session::GridState;
use xila_core::talk::{self, Speaker};

use crate::font::Fonts;
use crate::frame::{Frame, Rect, Sprite};
use crate::palette::slot;

/// Height of a line of text.
const LINE: i32 = 11;
/// Room between the edge of the area and what is written in it.
const MARGIN: i32 = 7;
/// Room between the frame of a bubble and its text.
const PAD: (i32, i32) = (6, 3);
/// Room between two blocks.
const GAP: i32 = 5;
/// Room kept on the left of what the agent says, for its face.
const FACE: i32 = 17;
/// Width of the bar that says where one is in a long conversation.
const BAR: i32 = 3;

/// One thing said, cut into the lines it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub speaker: Speaker,
    pub lines: Vec<String>,
    /// Width of the longest line, in pixels.
    pub width: i32,
    pub height: i32,
}

/// The face of the agent.
fn face() -> Sprite {
    Sprite::from_art(
        &[
            ".....##.....",
            "......#.....",
            ".##########.",
            "#vvvvvvvvvv#",
            "#v##vvvv##v#",
            "#v##vvvv##v#",
            "#vvvvvvvvvv#",
            "#vv#vvvv#vv#",
            "#vvv####vvv#",
            "#vvvvvvvvvv#",
            ".##########.",
        ],
        &[('#', slot::INK), ('v', slot::LAVENDER)],
    )
}

/// The blocks of a conversation, oldest first, wrapped for an area of a
/// given width.
pub fn blocks(fonts: &Fonts, definition: &Widget, state: &GridState, width: i32) -> Vec<Block> {
    let inner = width - 2 * MARGIN - BAR;
    talk::lines(definition, &state.page)
        .into_iter()
        .map(|line| {
            let bubble = matches!(line.speaker, Speaker::User | Speaker::Agent);
            // A bubble leaves the other side of the area free: one sees at a
            // glance who said what.
            let room = if bubble { inner * 4 / 5 - FACE - 2 * PAD.0 } else { inner - FACE };
            let lines = talk::wrap(line.text, room.max(20) as usize, |text| {
                fonts.regular.width(text).max(0) as usize
            });
            let widest = lines.iter().map(|text| fonts.regular.width(text)).max().unwrap_or(0);
            let text_height = lines.len() as i32 * LINE;
            Block {
                speaker: line.speaker,
                width: widest,
                height: if bubble { text_height + 2 * PAD.1 + 1 } else { text_height },
                lines,
            }
        })
        .collect()
}

/// Height of the blocks, with the room between them.
fn height_of(blocks: &[Block]) -> i32 {
    blocks.iter().map(|block| block.height + GAP).sum::<i32>()
        - if blocks.is_empty() { 0 } else { GAP }
}

/// How far back one can go in a conversation, in pixels: what does not fit
/// in the area.
pub fn overflow(blocks: &[Block], area: Rect) -> i32 {
    (height_of(blocks) - (area.h - 2 * MARGIN)).max(0)
}

/// How far a page goes.
pub fn page(area: Rect) -> i32 {
    (area.h - 2 * MARGIN - 2 * LINE).max(LINE)
}

/// Draws a conversation in its area, `back` pixels before its end.
pub fn draw(
    frame: &mut Frame,
    fonts: &Fonts,
    area: Rect,
    definition: &Widget,
    state: &GridState,
    back: i32,
) {
    frame.fill(area, slot::CREAM);
    frame.outline(area, slot::OCHRE);
    let blocks = blocks(fonts, definition, state, area.w);
    if blocks.is_empty() {
        let hint = "Ask the agent something.";
        let x = area.x + (area.w - fonts.regular.width(hint)) / 2;
        fonts.regular.draw(frame, x, area.y + area.h / 2, hint, slot::GREY);
        return;
    }

    let total = height_of(&blocks);
    let most = overflow(&blocks, area);
    let back = back.clamp(0, most);
    let previous = frame.clip_to(area.inset(1));
    let left = area.x + MARGIN;
    let right = area.right() - MARGIN - BAR;
    // The last block sits on the bottom of the area, like a chat.
    let mut y = area.bottom() - MARGIN - total + back;
    for block in &blocks {
        let bottom = y + block.height;
        if bottom >= area.y && y <= area.bottom() {
            draw_block(frame, fonts, block, left, right, y);
        }
        y = bottom + GAP;
    }
    frame.set_clip(previous);

    // Where one is in a conversation that does not fit.
    if most > 0 {
        let track = Rect::new(area.right() - BAR - 2, area.y + 2, BAR, area.h - 4);
        frame.fill(track, slot::SAND);
        let thumb_h = (track.h * (area.h - 2 * MARGIN) / total).max(8);
        let travel = track.h - thumb_h;
        let thumb_y = track.y + travel - travel * back / most;
        frame.fill(Rect::new(track.x, thumb_y, BAR, thumb_h), slot::OCHRE);
    }
}

fn draw_block(frame: &mut Frame, fonts: &Fonts, block: &Block, left: i32, right: i32, y: i32) {
    match block.speaker {
        Speaker::User | Speaker::Agent => {
            let mine = block.speaker == Speaker::User;
            let w = block.width + 2 * PAD.0;
            let x = if mine { right - w } else { left + FACE };
            let bubble = Rect::new(x, y, w, block.height);
            // A thin shadow lifts the bubble off the paper.
            frame.fill(bubble.moved(1, 1), slot::OCHRE);
            frame.fill(bubble, if mine { slot::ICE } else { slot::WHITE });
            frame.outline(bubble, slot::INK);
            if !mine {
                frame.sprite(&face(), left, y);
            }
            for (index, line) in block.lines.iter().enumerate() {
                let baseline = y + PAD.1 + 9 + index as i32 * LINE;
                fonts.regular.draw(frame, x + PAD.0, baseline, line, slot::INK);
            }
        }
        Speaker::Tool | Speaker::Error | Speaker::Wait => {
            let (mark, ink) = match block.speaker {
                Speaker::Tool => (slot::LAVENDER, slot::GREY),
                Speaker::Error => (slot::RED, slot::RED),
                _ => (slot::AMBER, slot::GREY),
            };
            let mark_area = Rect::new(left + FACE - 9, y + 3, 5, 5);
            frame.fill(mark_area, mark);
            frame.outline(mark_area, slot::INK);
            for (index, line) in block.lines.iter().enumerate() {
                let baseline = y + 9 + index as i32 * LINE;
                fonts.regular.draw(frame, left + FACE, baseline, line, ink);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xila_core::api::GridPage;

    fn widget() -> Widget {
        serde_json::from_value(serde_json::json!({
            "name": "talk", "kind": "grid", "ord": 1,
            "props": {"view": "talk", "who": "kind",
                      "columns": [{"column": "kind"}, {"column": "text"}]}
        }))
        .unwrap()
    }

    fn state(rows: &[(&str, &str)]) -> GridState {
        let rows: Vec<serde_json::Value> = rows
            .iter()
            .rev()
            .enumerate()
            .map(|(index, (kind, text))| {
                serde_json::json!({"key": index.to_string(), "cells": {"kind": kind, "text": text}})
            })
            .collect();
        let page: GridPage =
            serde_json::from_value(serde_json::json!({"total": rows.len(), "rows": rows})).unwrap();
        GridState { page, selected: 0 }
    }

    #[test]
    fn a_long_answer_is_wrapped_in_its_bubble() {
        let fonts = Fonts::builtin();
        let long = "Two people live in London: Charles Babbage, of Difference Works, and Ada \
                    Lovelace, of Analytical Engines.";
        let state =
            state(&[("user", "Who lives in London?"), ("tool", "Looked."), ("agent", long)]);
        let blocks = blocks(&fonts, &widget(), &state, 300);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].speaker, Speaker::User);
        assert_eq!(blocks[0].lines, ["Who lives in London?"]);
        assert!(blocks[2].lines.len() > 1, "{:?}", blocks[2].lines);
        // No line is wider than the room a bubble has.
        assert!(blocks[2].width <= 300 * 4 / 5, "{}", blocks[2].width);
        assert_eq!(blocks[2].lines.join(" "), long);
        // A bubble is taller than its text; a line of what the agent did is not.
        assert_eq!(blocks[0].height, LINE + 2 * PAD.1 + 1);
        assert_eq!(blocks[1].height, LINE);
    }

    #[test]
    fn a_conversation_that_fits_cannot_be_scrolled() {
        let fonts = Fonts::builtin();
        let area = Rect::new(0, 0, 300, 190);
        let short = state(&[("user", "Hello"), ("agent", "Hello.")]);
        assert_eq!(overflow(&blocks(&fonts, &widget(), &short, area.w), area), 0);

        let turns: Vec<(&str, &str)> =
            (0..20).flat_map(|_| [("user", "Again?"), ("agent", "Again.")]).collect();
        let long = state(&turns);
        assert!(overflow(&blocks(&fonts, &widget(), &long, area.w), area) > 0);
    }

    #[test]
    fn the_end_of_a_conversation_is_at_the_bottom() {
        let fonts = Fonts::builtin();
        let area = Rect::new(10, 20, 300, 190);
        let state = state(&[("user", "Hello"), ("agent", "Hello.")]);
        let mut frame = Frame::new();
        draw(&mut frame, &fonts, area, &widget(), &state, 0);

        // The bubble of the agent ends just above the bottom margin, on the
        // left; the one of the person is above it, on the right.
        let row = area.bottom() - MARGIN - 1;
        assert_eq!(frame.pixel(area.x + MARGIN + FACE, row), Some(slot::INK));
        assert_eq!(frame.pixel(area.right() - MARGIN - BAR - 1, row), Some(slot::CREAM));
        let above = row - (LINE + 2 * PAD.1 + 1) - GAP;
        assert_eq!(frame.pixel(area.right() - MARGIN - BAR - 1, above), Some(slot::INK));
        // The top of the area is empty.
        assert_eq!(frame.pixel(area.x + 40, area.y + 10), Some(slot::CREAM));
    }
}
