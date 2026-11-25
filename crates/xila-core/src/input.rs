//! Keys, and the editing of one line of text.
//!
//! Each client translates the events of its own library into [`Key`]; what a
//! key means is then the same in the terminal and in the window.

/// The keys the clients understand, independent of any input library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    F(u8),
    Enter,
    Tab,
    BackTab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    Char(char),
    /// Leave at once: Ctrl-C in a terminal.
    Interrupt,
}

/// Applies an editing key to a line of text. `cursor` counts characters.
/// Returns true when the key was an editing key.
pub fn edit_line(text: &mut String, cursor: &mut usize, key: Key, max_length: usize) -> bool {
    let length = text.chars().count();
    *cursor = (*cursor).min(length);
    let byte_at = |text: &str, position: usize| {
        text.char_indices().nth(position).map_or(text.len(), |(index, _)| index)
    };
    match key {
        Key::Char(c) => {
            if length < max_length {
                text.insert(byte_at(text, *cursor), c);
                *cursor += 1;
            }
        }
        Key::Backspace => {
            if *cursor > 0 {
                *cursor -= 1;
                text.remove(byte_at(text, *cursor));
            }
        }
        Key::Delete => {
            if *cursor < length {
                text.remove(byte_at(text, *cursor));
            }
        }
        Key::Left => *cursor = cursor.saturating_sub(1),
        Key::Right => *cursor = (*cursor + 1).min(length),
        Key::Home => *cursor = 0,
        Key::End => *cursor = length,
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(start: &str, cursor: usize, keys: &[Key]) -> (String, usize) {
        let mut text = start.to_owned();
        let mut cursor = cursor;
        for key in keys {
            edit_line(&mut text, &mut cursor, *key, 20);
        }
        (text, cursor)
    }

    #[test]
    fn characters_are_inserted_at_the_cursor() {
        assert_eq!(typed("Pais", 2, &[Key::Char('r')]), ("Paris".into(), 3));
    }

    #[test]
    fn accented_characters_count_as_one() {
        assert_eq!(
            typed("Socit", 4, &[Key::Char('é'), Key::End, Key::Char('é')]),
            ("Société".into(), 7)
        );
        assert_eq!(typed("Société", 7, &[Key::Backspace]), ("Sociét".into(), 6));
        assert_eq!(typed("Société", 4, &[Key::Delete]), ("Socité".into(), 4));
    }

    #[test]
    fn the_cursor_stays_inside_the_text() {
        assert_eq!(typed("ab", 0, &[Key::Left, Key::Backspace]), ("ab".into(), 0));
        assert_eq!(typed("ab", 2, &[Key::Right, Key::Delete]), ("ab".into(), 2));
        assert_eq!(typed("ab", 9, &[Key::Home, Key::Char('x')]), ("xab".into(), 1));
    }

    #[test]
    fn typing_stops_at_the_maximum_length() {
        let mut text = "abc".to_owned();
        let mut cursor = 3;
        edit_line(&mut text, &mut cursor, Key::Char('d'), 3);
        assert_eq!(text, "abc");
    }

    #[test]
    fn other_keys_are_not_editing_keys() {
        let mut text = String::new();
        let mut cursor = 0;
        assert!(!edit_line(&mut text, &mut cursor, Key::Enter, 10));
        assert!(!edit_line(&mut text, &mut cursor, Key::F(3), 10));
    }
}
