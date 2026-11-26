//! A list shown as a conversation: bubbles, written from the bottom up.
//!
//! What the person said is on the right, what the agent answered on the left
//! next to its face, and what the agent did in between, in small print. A
//! change the agent proposes is a dark box with what it does, the calls it
//! runs and, while it waits, the buttons that apply it or refuse it. The rows
//! come from the server newest first; see [`xila_core::talk`].

use xila_core::api::Widget;
use xila_core::session::GridState;
use xila_core::talk::{self, Offers, Speaker, State};

use crate::chrome::{draw_button, ButtonLook, BUTTON_HEIGHT};
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
/// Height of the band at the top of a proposal, which says what became of it.
const HEAD: i32 = 13;
/// Width of a button of a proposal, and the room between two of them.
const BUTTON: (i32, i32) = (54, 5);

/// What a button of a proposal does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Apply,
    Refuse,
    Show,
}

/// How a line of a block is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Plain,
    /// What a proposal does.
    Title,
    /// What there is to know about a proposal.
    Detail,
    /// The calls a proposal runs.
    Code,
}

/// One thing said, cut into the lines it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub speaker: Speaker,
    /// The key of the row: the name of a proposal for the server.
    pub key: String,
    pub lines: Vec<(Ink, String)>,
    /// Width of what is drawn, in pixels.
    pub width: i32,
    pub height: i32,
    /// The buttons of a proposal that waits, each with its label and its
    /// place from the top left corner of the box.
    pub buttons: Vec<(Button, String, Rect)>,
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
pub fn blocks(
    fonts: &Fonts,
    definition: &Widget,
    state: &GridState,
    offers: &Offers,
    width: i32,
) -> Vec<Block> {
    let inner = width - 2 * MARGIN - BAR;
    let regular = |text: &str| fonts.regular.width(text).max(0) as usize;
    let bold = |text: &str| fonts.bold.width(text).max(0) as usize;
    talk::lines(definition, &state.page)
        .into_iter()
        .map(|line| {
            if let Speaker::Proposal(became) = line.speaker {
                // A box as wide as the conversation allows, beside the face.
                let box_width = inner - FACE;
                let room = (box_width - 2 * PAD.0).max(20) as usize;
                let mut said = line.text.split('\n');
                let mut lines: Vec<(Ink, String)> =
                    talk::wrap(said.next().unwrap_or_default(), room, bold)
                        .into_iter()
                        .map(|text| (Ink::Title, text))
                        .collect();
                let mut buttons = Vec::new();
                // Once applied or refused, a proposal is only its title.
                if became == State::Pending {
                    for detail in said {
                        let cut = talk::wrap(detail, room, regular);
                        lines.extend(cut.into_iter().map(|text| (Ink::Detail, text)));
                    }
                    for call in line.code.split('\n').filter(|call| !call.is_empty()) {
                        let cut = talk::wrap(call, room, regular);
                        lines.extend(cut.into_iter().map(|text| (Ink::Code, text)));
                    }
                    let y = HEAD + PAD.1 + lines.len() as i32 * LINE + 4;
                    let offered = [
                        (Button::Apply, &offers.apply),
                        (Button::Refuse, &offers.refuse),
                        (Button::Show, &offers.show),
                    ];
                    for (button, offer) in offered {
                        let Some(offer) = offer else { continue };
                        let x = PAD.0 + buttons.len() as i32 * (BUTTON.0 + BUTTON.1);
                        let area = Rect::new(x, y, BUTTON.0, BUTTON_HEIGHT);
                        buttons.push((button, offer.label.clone(), area));
                    }
                }
                let text_height = lines.len() as i32 * LINE;
                let buttons_height = if buttons.is_empty() { 0 } else { BUTTON_HEIGHT + 5 };
                return Block {
                    speaker: line.speaker,
                    key: line.key.to_owned(),
                    width: box_width,
                    height: HEAD + PAD.1 + text_height + buttons_height + PAD.1 + 1,
                    lines,
                    buttons,
                };
            }

            let bubble = matches!(line.speaker, Speaker::User | Speaker::Agent);
            // A bubble leaves the other side of the area free: one sees at a
            // glance who said what.
            let room = if bubble { inner * 4 / 5 - FACE - 2 * PAD.0 } else { inner - FACE };
            let lines = talk::wrap(line.text, room.max(20) as usize, regular);
            let widest = lines.iter().map(|text| fonts.regular.width(text)).max().unwrap_or(0);
            let text_height = lines.len() as i32 * LINE;
            Block {
                speaker: line.speaker,
                key: line.key.to_owned(),
                width: widest,
                height: if bubble { text_height + 2 * PAD.1 + 1 } else { text_height },
                lines: lines.into_iter().map(|text| (Ink::Plain, text)).collect(),
                buttons: Vec::new(),
            }
        })
        .collect()
}

/// Height of the blocks, with the room between them.
fn height_of(blocks: &[Block]) -> i32 {
    let gaps = (blocks.len() as i32 - 1).max(0) * GAP;
    blocks.iter().map(|block| block.height).sum::<i32>() + gaps
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

/// Where the blocks are on the screen: the top of each, the last one sitting
/// on the bottom of the area, like a chat, `back` pixels before its end.
fn tops(blocks: &[Block], area: Rect, back: i32) -> Vec<i32> {
    let back = back.clamp(0, overflow(blocks, area));
    let mut y = area.bottom() - MARGIN - height_of(blocks) + back;
    blocks
        .iter()
        .map(|block| {
            let top = y;
            y += block.height + GAP;
            top
        })
        .collect()
}

/// The left edge of the box of a proposal.
fn box_left(area: Rect) -> i32 {
    area.x + MARGIN + FACE
}

/// The button of a proposal under a point of the screen, with the key of
/// that proposal.
pub fn button_at(
    blocks: &[Block],
    area: Rect,
    back: i32,
    x: i32,
    y: i32,
) -> Option<(String, Button)> {
    if !area.inset(1).contains(x, y) {
        return None;
    }
    let left = box_left(area);
    blocks.iter().zip(tops(blocks, area, back)).find_map(|(block, top)| {
        block
            .buttons
            .iter()
            .find(|(_, _, place)| place.moved(left, top).contains(x, y))
            .map(|(button, _, _)| (block.key.clone(), *button))
    })
}

/// Draws a conversation in its area, `back` pixels before its end. `pressed`
/// is the button the pointer is held on.
pub fn draw(
    frame: &mut Frame,
    fonts: &Fonts,
    area: Rect,
    blocks: &[Block],
    back: i32,
    pressed: Option<&(String, Button)>,
) {
    frame.fill(area, slot::CREAM);
    frame.outline(area, slot::OCHRE);
    if blocks.is_empty() {
        let hint = "Ask the agent something.";
        let x = area.x + (area.w - fonts.regular.width(hint)) / 2;
        fonts.regular.draw(frame, x, area.y + area.h / 2, hint, slot::GREY);
        return;
    }

    let most = overflow(blocks, area);
    let back = back.clamp(0, most);
    let previous = frame.clip_to(area.inset(1));
    let left = area.x + MARGIN;
    let right = area.right() - MARGIN - BAR;
    for (block, top) in blocks.iter().zip(tops(blocks, area, back)) {
        if top + block.height >= area.y && top <= area.bottom() {
            draw_block(frame, fonts, block, left, right, top, pressed);
        }
    }
    frame.set_clip(previous);

    // Where one is in a conversation that does not fit.
    if most > 0 {
        let track = Rect::new(area.right() - BAR - 2, area.y + 2, BAR, area.h - 4);
        frame.fill(track, slot::SAND);
        let thumb_h = (track.h * (area.h - 2 * MARGIN) / height_of(blocks)).max(8);
        let travel = track.h - thumb_h;
        let thumb_y = track.y + travel - travel * back / most;
        frame.fill(Rect::new(track.x, thumb_y, BAR, thumb_h), slot::OCHRE);
    }
}

fn draw_block(
    frame: &mut Frame,
    fonts: &Fonts,
    block: &Block,
    left: i32,
    right: i32,
    y: i32,
    pressed: Option<&(String, Button)>,
) {
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
            for (index, (_, line)) in block.lines.iter().enumerate() {
                let baseline = y + PAD.1 + 9 + index as i32 * LINE;
                fonts.regular.draw(frame, x + PAD.0, baseline, line, slot::INK);
            }
        }
        Speaker::Proposal(became) => {
            let x = left + FACE;
            let area = Rect::new(x, y, block.width, block.height);
            frame.fill(area.moved(2, 2), slot::OCHRE);
            frame.fill(area, slot::NIGHT);
            frame.outline(area, slot::INK);
            // The band at the top says what became of it.
            frame.fill(Rect::new(x + 1, y + 1, block.width - 2, HEAD - 1), slot::PLUM);
            let (word, ink) = match became {
                State::Pending => ("Proposed", slot::LAVENDER),
                State::Applied => ("Applied", slot::GREEN),
                State::Refused => ("Refused", slot::SILVER),
            };
            fonts.bold.draw(frame, x + PAD.0, y + 10, word, ink);
            for (index, (kind, line)) in block.lines.iter().enumerate() {
                let baseline = y + HEAD + PAD.1 + 9 + index as i32 * LINE;
                match kind {
                    Ink::Title if became == State::Pending => {
                        fonts.bold.draw(frame, x + PAD.0, baseline, line, slot::WHITE);
                    }
                    // What is settled steps back.
                    Ink::Title => {
                        fonts.regular.draw(frame, x + PAD.0, baseline, line, slot::PEARL);
                    }
                    Ink::Code => {
                        fonts.regular.draw(frame, x + PAD.0, baseline, line, slot::YELLOW);
                    }
                    Ink::Detail | Ink::Plain => {
                        fonts.regular.draw(frame, x + PAD.0, baseline, line, slot::ICE);
                    }
                }
            }
            for (button, label, place) in &block.buttons {
                let look = ButtonLook {
                    primary: *button == Button::Apply,
                    pressed: pressed.is_some_and(|(key, held)| key == &block.key && held == button),
                    ..ButtonLook::default()
                };
                draw_button(frame, fonts, place.moved(x, y), label, look);
            }
        }
        Speaker::Tool | Speaker::Note | Speaker::Error | Speaker::Wait => {
            let (mark, ink) = match block.speaker {
                Speaker::Tool => (slot::LAVENDER, slot::GREY),
                Speaker::Note => (slot::GREEN, slot::GREY),
                Speaker::Error => (slot::RED, slot::RED),
                _ => (slot::AMBER, slot::GREY),
            };
            let mark_area = Rect::new(left + FACE - 9, y + 3, 5, 5);
            frame.fill(mark_area, mark);
            frame.outline(mark_area, slot::INK);
            for (index, (_, line)) in block.lines.iter().enumerate() {
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
    use xila_core::talk::Offer;

    fn widget() -> Widget {
        serde_json::from_value(serde_json::json!({
            "name": "talk", "kind": "grid", "ord": 1,
            "props": {"view": "talk", "who": "kind", "state": "state", "code": "code",
                      "columns": [{"column": "kind"}, {"column": "text"},
                                  {"column": "state"}, {"column": "code"}]}
        }))
        .unwrap()
    }

    fn state(rows: &[(&str, &str)]) -> GridState {
        let rows: Vec<serde_json::Value> = rows
            .iter()
            .rev()
            .enumerate()
            .map(|(index, (kind, text))| {
                let (kind, became) = kind.split_once(':').unwrap_or((kind, ""));
                serde_json::json!({"key": index.to_string(), "cells": {
                    "kind": kind, "text": text, "state": became,
                    "code": if kind == "proposal" { "xila.add_field('books', 'Bought', 'date')" } else { "" }}})
            })
            .collect();
        let page: GridPage =
            serde_json::from_value(serde_json::json!({"total": rows.len(), "rows": rows})).unwrap();
        GridState { page, selected: 0 }
    }

    fn offers() -> Offers {
        let offer = |action: &str, label: &str| {
            Some(Offer { action: action.into(), label: label.into(), key: None })
        };
        Offers {
            apply: offer("apply", "Apply"),
            refuse: offer("refuse", "Refuse"),
            show: offer("view", "View"),
        }
    }

    fn said(rows: &[(&str, &str)], width: i32) -> Vec<Block> {
        blocks(&Fonts::builtin(), &widget(), &state(rows), &offers(), width)
    }

    #[test]
    fn a_long_answer_is_wrapped_in_its_bubble() {
        let long = "Two people live in London: Charles Babbage, of Difference Works, and Ada \
                    Lovelace, of Analytical Engines.";
        let blocks =
            said(&[("user", "Who lives in London?"), ("tool", "Looked."), ("agent", long)], 300);

        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].speaker, Speaker::User);
        assert_eq!(blocks[0].lines, [(Ink::Plain, "Who lives in London?".to_owned())]);
        assert!(blocks[2].lines.len() > 1, "{:?}", blocks[2].lines);
        // No line is wider than the room a bubble has.
        assert!(blocks[2].width <= 300 * 4 / 5, "{}", blocks[2].width);
        let text: Vec<&str> = blocks[2].lines.iter().map(|(_, line)| line.as_str()).collect();
        assert_eq!(text.join(" "), long);
        // A bubble is taller than its text; a line of what the agent did is not.
        assert_eq!(blocks[0].height, LINE + 2 * PAD.1 + 1);
        assert_eq!(blocks[1].height, LINE);
    }

    #[test]
    fn a_conversation_that_fits_cannot_be_scrolled() {
        let area = Rect::new(0, 0, 300, 190);
        let short = said(&[("user", "Hello"), ("agent", "Hello.")], area.w);
        assert_eq!(overflow(&short, area), 0);

        let turns: Vec<(&str, &str)> =
            (0..20).flat_map(|_| [("user", "Again?"), ("agent", "Again.")]).collect();
        assert!(overflow(&said(&turns, area.w), area) > 0);
    }

    #[test]
    fn the_end_of_a_conversation_is_at_the_bottom() {
        let fonts = Fonts::builtin();
        let area = Rect::new(10, 20, 300, 190);
        let blocks = said(&[("user", "Hello"), ("agent", "Hello.")], area.w);
        let mut frame = Frame::new();
        draw(&mut frame, &fonts, area, &blocks, 0, None);

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

    #[test]
    fn a_proposal_that_waits_has_its_buttons_and_a_settled_one_has_none() {
        let fonts = Fonts::builtin();
        let area = Rect::new(10, 20, 300, 190);
        let text = "Add the field “Bought” to Books\nA field of type date.";
        let waiting = said(&[("user", "Do it."), ("proposal:pending", text)], area.w);
        let proposal = &waiting[1];
        assert_eq!(proposal.speaker, Speaker::Proposal(State::Pending));
        let inks: Vec<Ink> = proposal.lines.iter().map(|(ink, _)| *ink).collect();
        assert_eq!(inks, [Ink::Title, Ink::Detail, Ink::Code]);
        let offered: Vec<Button> = proposal.buttons.iter().map(|(button, _, _)| *button).collect();
        assert_eq!(offered, [Button::Apply, Button::Refuse, Button::Show]);

        // A click on a button names the proposal and what to do with it.
        let mut frame = Frame::new();
        draw(&mut frame, &fonts, area, &waiting, 0, None);
        let top = area.bottom() - MARGIN - proposal.height;
        let (_, _, place) = &proposal.buttons[1];
        let on_refuse = place.moved(box_left(area), top);
        assert_eq!(
            button_at(&waiting, area, 0, on_refuse.x + 3, on_refuse.y + 3),
            Some(("0".to_owned(), Button::Refuse))
        );
        assert_eq!(button_at(&waiting, area, 0, area.x + 3, area.y + 3), None);
        // The box is dark, and its main button is the yellow one.
        assert_eq!(frame.pixel(box_left(area) + 3, top + HEAD + 2), Some(slot::NIGHT));
        let (_, _, apply) = &proposal.buttons[0];
        let on_apply = apply.moved(box_left(area), top);
        assert_eq!(frame.pixel(on_apply.x + 4, on_apply.y + 4), Some(slot::YELLOW));

        // Applied: only what it did is left, and nothing to click.
        let settled = said(&[("user", "Do it."), ("proposal:applied", text)], area.w);
        assert_eq!(settled[1].lines.len(), 1);
        assert!(settled[1].buttons.is_empty());
        assert!(settled[1].height < proposal.height);
    }
}
