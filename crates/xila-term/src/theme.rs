//! Colours of the text client.
//!
//! The text screen draws with the system palette, which lives in the
//! database: which slot is used for what is decided here, the colours
//! themselves are not.

use ratatui::style::{Color, Modifier, Style};
use xila_core::api::PaletteColour;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub background: Color,
    pub text: Color,
    pub label: Color,
    pub value: Color,
    pub title: Color,
    pub key: Color,
    pub message: Color,
    pub selection: Color,
}

impl Theme {
    /// The colours used before a connection exists: those of the terminal.
    pub fn terminal() -> Self {
        Theme {
            background: Color::Reset,
            text: Color::Green,
            label: Color::Cyan,
            value: Color::White,
            title: Color::White,
            key: Color::Yellow,
            message: Color::Yellow,
            selection: Color::Blue,
        }
    }

    /// The colours of a session, taken from the system palette.
    pub fn from_palette(palette: &[PaletteColour]) -> Self {
        let fallback = Theme::terminal();
        let slot = |wanted: u8, otherwise: Color| {
            palette
                .iter()
                .find(|colour| colour.slot == wanted)
                .and_then(PaletteColour::components)
                .map_or(otherwise, |(r, g, b)| Color::Rgb(r, g, b))
        };
        Theme {
            background: slot(0, fallback.background),
            text: slot(18, fallback.text),
            label: slot(21, fallback.label),
            value: slot(7, fallback.value),
            title: slot(7, fallback.title),
            key: slot(22, fallback.key),
            message: slot(24, fallback.message),
            selection: slot(2, fallback.selection),
        }
    }

    pub fn base(&self) -> Style {
        Style::new().fg(self.text).bg(self.background)
    }

    pub fn of(&self, colour: Color) -> Style {
        Style::new().fg(colour).bg(self.background)
    }

    /// An input field: underlined, and highlighted when it has the focus.
    pub fn field(&self, focused: bool) -> Style {
        let style = Style::new().fg(self.value).add_modifier(Modifier::UNDERLINED);
        if focused {
            style.bg(self.selection)
        } else {
            style.bg(self.background)
        }
    }

    pub fn selected(&self) -> Style {
        Style::new().fg(self.value).bg(self.selection)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_come_from_the_palette_slots() {
        let palette = vec![
            PaletteColour { slot: 0, name: "black".into(), rgb: "#14121f".into() },
            PaletteColour { slot: 22, name: "jaune".into(), rgb: "#f7d046".into() },
        ];
        let theme = Theme::from_palette(&palette);
        assert_eq!(theme.background, Color::Rgb(0x14, 0x12, 0x1f));
        assert_eq!(theme.key, Color::Rgb(0xf7, 0xd0, 0x46));
        // A slot the palette does not have falls back to a terminal colour.
        assert_eq!(theme.text, Color::Green);
    }
}
