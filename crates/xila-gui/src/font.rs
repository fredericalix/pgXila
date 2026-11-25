//! Bitmap fonts.
//!
//! The client draws its own text. Its fonts are BDF files compiled into the
//! program: Xila Sans, regular and bold, 9 pixels per em, with capitals 7
//! pixels tall. See `assets/fonts/README.md` for where they come from.

use std::collections::HashMap;

use crate::frame::Frame;

const REGULAR: &str = include_str!("../assets/fonts/xila-sans-9.bdf");
const BOLD: &str = include_str!("../assets/fonts/xila-sans-bold-9.bdf");

/// Height of a capital letter, in pixels.
pub const CAP_HEIGHT: i32 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Glyph {
    /// How far the pen moves after the glyph.
    advance: i32,
    /// Offset of the bitmap from the pen position, to the right.
    left: i32,
    /// Row of the top of the bitmap, counted upwards from the baseline.
    top: i32,
    width: i32,
    /// One entry per row, top first. Bit 31 is the leftmost pixel.
    rows: Vec<u32>,
}

#[derive(Debug, Clone)]
pub struct Font {
    glyphs: HashMap<char, Glyph>,
    /// Rows above the baseline, for the tallest accented capital.
    pub ascent: i32,
    /// Rows below the baseline, for the tail of a `g`.
    pub descent: i32,
}

impl Font {
    /// Reads a BDF font.
    pub fn parse(bdf: &str) -> Result<Font, String> {
        let mut font = Font { glyphs: HashMap::new(), ascent: 0, descent: 0 };
        let mut lines = bdf.lines();
        while let Some(line) = lines.next() {
            let mut words = line.split_ascii_whitespace();
            match words.next() {
                Some("FONT_ASCENT") => font.ascent = number(words.next(), line)?,
                Some("FONT_DESCENT") => font.descent = number(words.next(), line)?,
                Some("STARTCHAR") => {
                    if let Some((character, glyph)) = parse_glyph(&mut lines)? {
                        font.glyphs.insert(character, glyph);
                    }
                }
                _ => {}
            }
        }
        if font.glyphs.is_empty() {
            return Err("the font has no glyph".into());
        }
        if !font.glyphs.contains_key(&'?') {
            return Err("the font has no question mark, which stands for missing glyphs".into());
        }
        Ok(font)
    }

    fn glyph(&self, character: char) -> &Glyph {
        self.glyphs.get(&character).unwrap_or_else(|| &self.glyphs[&'?'])
    }

    /// Width of a text, in pixels.
    pub fn width(&self, text: &str) -> i32 {
        text.chars().map(|c| self.glyph(c).advance).sum()
    }

    /// How many characters of a text fit in a width.
    pub fn fitting(&self, text: &str, width: i32) -> usize {
        let mut used = 0;
        for (count, character) in text.chars().enumerate() {
            used += self.glyph(character).advance;
            if used > width {
                return count;
            }
        }
        text.chars().count()
    }

    /// Draws a text. `baseline` is the first row below the letters without a
    /// tail. Returns where the pen ends.
    pub fn draw(&self, frame: &mut Frame, x: i32, baseline: i32, text: &str, slot: u8) -> i32 {
        self.draw_scaled(frame, x, baseline, text, slot, 1)
    }

    /// Draws a text with each pixel enlarged `scale` times.
    pub fn draw_scaled(
        &self,
        frame: &mut Frame,
        x: i32,
        baseline: i32,
        text: &str,
        slot: u8,
        scale: i32,
    ) -> i32 {
        let mut pen = x;
        for character in text.chars() {
            let glyph = self.glyph(character);
            for (row, bits) in glyph.rows.iter().enumerate() {
                let y = baseline - (glyph.top - row as i32) * scale;
                for column in 0..glyph.width {
                    if bits & (0x8000_0000 >> column) != 0 {
                        let px = pen + (glyph.left + column) * scale;
                        if scale == 1 {
                            frame.set(px, y, slot);
                        } else {
                            frame.fill(crate::frame::Rect::new(px, y, scale, scale), slot);
                        }
                    }
                }
            }
            pen += glyph.advance * scale;
        }
        pen
    }
}

fn number(word: Option<&str>, line: &str) -> Result<i32, String> {
    word.and_then(|w| w.parse().ok()).ok_or_else(|| format!("unreadable font line: {line}"))
}

/// Reads one glyph, from the line after STARTCHAR to ENDCHAR. Glyphs without
/// a Unicode character are skipped.
fn parse_glyph<'a>(
    lines: &mut impl Iterator<Item = &'a str>,
) -> Result<Option<(char, Glyph)>, String> {
    let mut character = None;
    let mut glyph = Glyph { advance: 0, left: 0, top: 0, width: 0, rows: Vec::new() };
    let mut height = 0;
    let mut in_bitmap = false;
    for line in lines {
        let mut words = line.split_ascii_whitespace();
        let Some(first) = words.next() else { continue };
        match first {
            "ENDCHAR" => {
                if glyph.rows.len() != height as usize {
                    return Err(format!(
                        "a glyph has {} rows instead of {height}",
                        glyph.rows.len()
                    ));
                }
                return Ok(character.map(|c| (c, glyph)));
            }
            "ENCODING" => {
                character =
                    u32::try_from(number(words.next(), line)?).ok().and_then(char::from_u32);
            }
            "DWIDTH" => glyph.advance = number(words.next(), line)?,
            "BBX" => {
                glyph.width = number(words.next(), line)?;
                height = number(words.next(), line)?;
                glyph.left = number(words.next(), line)?;
                let bottom = number(words.next(), line)?;
                glyph.top = bottom + height;
                if !(0..=32).contains(&glyph.width) || height < 0 {
                    return Err(format!("a glyph is too large: {line}"));
                }
            }
            "BITMAP" => in_bitmap = true,
            row if in_bitmap => {
                let bits = u32::from_str_radix(row, 16)
                    .map_err(|_| format!("unreadable glyph row: {line}"))?;
                // Rows are padded to whole bytes on the right.
                glyph.rows.push(bits << (32 - 4 * row.len() as u32));
            }
            _ => {}
        }
    }
    Err("the font ends in the middle of a glyph".into())
}

/// The fonts of the client.
#[derive(Debug, Clone)]
pub struct Fonts {
    pub regular: Font,
    pub bold: Font,
}

impl Fonts {
    /// The fonts compiled into the client.
    pub fn builtin() -> Self {
        Fonts {
            regular: Font::parse(REGULAR).expect("the built-in regular font is valid"),
            bold: Font::parse(BOLD).expect("the built-in bold font is valid"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{Rect, HEIGHT, WIDTH};

    /// What a text looks like, as rows of `#` and `.`.
    fn rendered(font: &Font, text: &str, scale: i32) -> Vec<String> {
        let mut frame = Frame::new();
        let baseline = 20 * scale;
        let end = font.draw_scaled(&mut frame, 0, baseline, text, 7, scale);
        (baseline - CAP_HEIGHT * scale..baseline)
            .map(|y| {
                (0..end).map(|x| if frame.pixel(x, y) == Some(7) { '#' } else { '.' }).collect()
            })
            .collect()
    }

    #[test]
    fn the_built_in_fonts_cover_french() {
        let fonts = Fonts::builtin();
        for font in [&fonts.regular, &fonts.bold] {
            assert_eq!((font.ascent, font.descent), (9, 2));
            for character in "àâäçéèêëîïôöùûüÿœæÀÂÇÉÈÊËÎÏÔÖÙÛÜŒÆ«»’…€".chars()
            {
                assert!(font.glyphs.contains_key(&character), "missing {character}");
            }
        }
    }

    #[test]
    fn a_capital_is_seven_pixels_tall() {
        let fonts = Fonts::builtin();
        assert_eq!(
            rendered(&fonts.regular, "F", 1),
            ["#####.", "#.....", "#.....", "####..", "#.....", "#.....", "#....."]
        );
        // The bold font has stems two pixels wide.
        assert!(rendered(&fonts.bold, "F", 1).iter().all(|row| row.starts_with("##")));
    }

    #[test]
    fn text_is_measured_as_it_is_drawn() {
        let fonts = Fonts::builtin();
        let mut frame = Frame::new();
        let end = fonts.regular.draw(&mut frame, 10, 20, "Société", 7);
        assert_eq!(end - 10, fonts.regular.width("Société"));
        assert!(fonts.bold.width("Société") > fonts.regular.width("Société"));
        assert_eq!(fonts.regular.width(""), 0);
    }

    #[test]
    fn accents_and_tails_stay_inside_the_line() {
        let fonts = Fonts::builtin();
        let mut frame = Frame::new();
        fonts.regular.draw(&mut frame, 0, 20, "Éjgçy", 7);
        let inked: Vec<i32> =
            (0..HEIGHT).filter(|y| (0..WIDTH).any(|x| frame.pixel(x, *y) == Some(7))).collect();
        assert_eq!(inked.first(), Some(&(20 - fonts.regular.ascent)));
        assert_eq!(inked.last(), Some(&(20 + fonts.regular.descent - 1)));
    }

    #[test]
    fn enlarged_text_is_made_of_larger_pixels() {
        let fonts = Fonts::builtin();
        let small = rendered(&fonts.regular, "A", 1);
        let large = rendered(&fonts.regular, "A", 2);
        assert_eq!(large.len(), 2 * small.len());
        for (row, line) in small.iter().enumerate() {
            let doubled: String = line.chars().flat_map(|c| [c, c]).collect();
            assert_eq!(large[2 * row], doubled);
            assert_eq!(large[2 * row + 1], doubled);
        }
    }

    #[test]
    fn a_missing_glyph_is_drawn_as_a_question_mark() {
        let fonts = Fonts::builtin();
        assert_eq!(rendered(&fonts.regular, "\u{10FFFF}", 1), rendered(&fonts.regular, "?", 1));
    }

    #[test]
    fn fitting_counts_the_characters_that_fit() {
        let font = Fonts::builtin().regular;
        let width = font.width("Lov");
        assert_eq!(font.fitting("Lovelace", width), 3);
        assert_eq!(font.fitting("Lovelace", width - 1), 2);
        assert_eq!(font.fitting("Lovelace", 1000), 8);
        assert_eq!(font.fitting("", 10), 0);
    }

    #[test]
    fn text_is_cut_by_the_clip() {
        let font = Fonts::builtin().regular;
        let mut frame = Frame::new();
        frame.clip_to(Rect::new(0, 0, 3, HEIGHT));
        font.draw(&mut frame, 0, 20, "MMMM", 7);
        assert!((3..WIDTH).all(|x| (0..HEIGHT).all(|y| frame.pixel(x, y) == Some(0))));
    }

    #[test]
    fn a_broken_font_is_refused() {
        assert!(Font::parse("STARTFONT 2.1\nENDFONT\n").is_err());
        assert!(
            Font::parse("STARTCHAR a\nENCODING 63\nBBX 1 2 0 0\nBITMAP\n80\nENDCHAR\n").is_err()
        );
        assert!(Font::parse("STARTCHAR a\nENCODING 63\nBBX 1 1 0 0\nBITMAP\n80\n").is_err());
    }
}
