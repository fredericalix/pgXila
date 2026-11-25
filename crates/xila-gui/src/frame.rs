//! The screen of pgXila: 480 by 300 pixels, one byte per pixel.
//!
//! Each byte is a palette slot. Everything the graphical client shows is drawn
//! here first, by the client itself, pixel by pixel; the window only enlarges
//! the result by a whole factor.

use xila_core::api::{Image, TRANSPARENT};

pub const WIDTH: i32 = 480;
pub const HEIGHT: i32 = 300;

/// A rectangle of the screen. `w` and `h` may be zero or negative: the
/// rectangle is then empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const SCREEN: Rect = Rect { x: 0, y: 0, w: WIDTH, h: HEIGHT };

    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect { x, y, w, h }
    }

    /// The first column to the right of the rectangle.
    pub const fn right(&self) -> i32 {
        self.x + self.w
    }

    /// The first row below the rectangle.
    pub const fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub const fn is_empty(&self) -> bool {
        self.w <= 0 || self.h <= 0
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The rectangle shrunk by `by` pixels on each side.
    pub const fn inset(&self, by: i32) -> Rect {
        Rect { x: self.x + by, y: self.y + by, w: self.w - 2 * by, h: self.h - 2 * by }
    }

    /// The rectangle moved by an offset.
    pub const fn moved(&self, dx: i32, dy: i32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, w: self.w, h: self.h }
    }

    /// The part common to two rectangles; empty when they do not meet.
    pub fn intersection(&self, other: &Rect) -> Rect {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        Rect { x, y, w: (right - x).max(0), h: (bottom - y).max(0) }
    }

    /// The smallest rectangle that holds both. An empty one counts for nothing.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect {
            x,
            y,
            w: self.right().max(other.right()) - x,
            h: self.bottom().max(other.bottom()) - y,
        }
    }
}

/// How densely a dither covers a surface, in sixteenths.
///
/// A dither mixes two colours on a fixed 4 by 4 pattern, which is how pixel
/// art gets shades and gradients out of a small palette.
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

/// A small picture with transparent pixels, built into the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    pub width: i32,
    pub height: i32,
    /// One entry per pixel, row by row. None is transparent.
    pub pixels: Vec<Option<u8>>,
}

impl Sprite {
    /// Builds a sprite from rows of characters. Each character is looked up
    /// in `key`; a character that is not in the key is transparent.
    pub fn from_art(rows: &[&str], key: &[(char, u8)]) -> Self {
        let width = rows.iter().map(|row| row.chars().count()).max().unwrap_or(0);
        let mut pixels = Vec::with_capacity(width * rows.len());
        for row in rows {
            let mut drawn = 0;
            for c in row.chars() {
                pixels.push(key.iter().find(|(symbol, _)| *symbol == c).map(|(_, slot)| *slot));
                drawn += 1;
            }
            pixels.extend(std::iter::repeat_n(None, width - drawn));
        }
        Sprite { width: width as i32, height: rows.len() as i32, pixels }
    }
}

pub struct Frame {
    pixels: Vec<u8>,
    clip: Rect,
}

impl Default for Frame {
    fn default() -> Self {
        Frame::new()
    }
}

impl Frame {
    pub fn new() -> Self {
        Frame { pixels: vec![0; (WIDTH * HEIGHT) as usize], clip: Rect::SCREEN }
    }

    /// The pixels, row by row: `WIDTH * HEIGHT` palette slots.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The palette slot of a pixel, if it is on the screen.
    pub fn pixel(&self, x: i32, y: i32) -> Option<u8> {
        Rect::SCREEN.contains(x, y).then(|| self.pixels[(y * WIDTH + x) as usize])
    }

    /// Restricts drawing to a rectangle, and returns the previous restriction
    /// so that it can be put back with [`Frame::set_clip`].
    pub fn clip_to(&mut self, area: Rect) -> Rect {
        let previous = self.clip;
        self.clip = previous.intersection(&area);
        previous
    }

    pub fn set_clip(&mut self, clip: Rect) {
        self.clip = clip.intersection(&Rect::SCREEN);
    }

    pub fn set(&mut self, x: i32, y: i32, slot: u8) {
        if self.clip.contains(x, y) {
            self.pixels[(y * WIDTH + x) as usize] = slot;
        }
    }

    pub fn fill(&mut self, area: Rect, slot: u8) {
        let area = area.intersection(&self.clip);
        for y in area.y..area.bottom() {
            let start = (y * WIDTH + area.x) as usize;
            self.pixels[start..start + area.w as usize].fill(slot);
        }
    }

    pub fn hline(&mut self, x: i32, y: i32, w: i32, slot: u8) {
        self.fill(Rect::new(x, y, w, 1), slot);
    }

    pub fn vline(&mut self, x: i32, y: i32, h: i32, slot: u8) {
        self.fill(Rect::new(x, y, 1, h), slot);
    }

    /// A one pixel border, inside the rectangle.
    pub fn outline(&mut self, area: Rect, slot: u8) {
        if area.is_empty() {
            return;
        }
        self.hline(area.x, area.y, area.w, slot);
        self.hline(area.x, area.bottom() - 1, area.w, slot);
        self.vline(area.x, area.y, area.h, slot);
        self.vline(area.right() - 1, area.y, area.h, slot);
    }

    /// Paints `slot` over `level` sixteenths of the pixels of a rectangle,
    /// on a regular pattern, and leaves the others as they are.
    pub fn dither(&mut self, area: Rect, slot: u8, level: u8) {
        let area = area.intersection(&self.clip);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if BAYER[(y & 3) as usize][(x & 3) as usize] < level {
                    self.pixels[(y * WIDTH + x) as usize] = slot;
                }
            }
        }
    }

    /// Paints `slot` on every other pixel, as a checkerboard: the shadow of
    /// a window or of a sheet of paper.
    pub fn checker(&mut self, area: Rect, slot: u8) {
        let area = area.intersection(&self.clip);
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                if (x + y) & 1 == 0 {
                    self.pixels[(y * WIDTH + x) as usize] = slot;
                }
            }
        }
    }

    /// A vertical gradient from one colour to another, dithered.
    pub fn gradient(&mut self, area: Rect, top: u8, bottom: u8) {
        self.fill(area, top);
        if area.h <= 1 {
            return;
        }
        let clipped = area.intersection(&self.clip);
        for y in clipped.y..clipped.bottom() {
            let level = ((y - area.y) * 17 / area.h) as u8;
            self.dither(Rect::new(clipped.x, y, clipped.w, 1), bottom, level);
        }
    }

    /// Draws an image of the database, top left corner at (x, y).
    pub fn blit(&mut self, image: &Image, x: i32, y: i32) {
        self.blit_scaled(image, x, y, 1);
    }

    /// Copies an image with each of its pixels enlarged to a square of
    /// `scale` pixels. A transparent pixel leaves what is there.
    pub fn blit_scaled(&mut self, image: &Image, x: i32, y: i32, scale: i32) {
        let scale = scale.max(1);
        let area = Rect::new(x, y, i32::from(image.width) * scale, i32::from(image.height) * scale)
            .intersection(&self.clip);
        for row in area.y..area.bottom() {
            for column in area.x..area.right() {
                let source = (((column - x) / scale) as u16, ((row - y) / scale) as u16);
                match image.pixel(source.0, source.1) {
                    Some(TRANSPARENT) | None => {}
                    Some(slot) => self.pixels[(row * WIDTH + column) as usize] = slot,
                }
            }
        }
    }

    /// Draws a built-in picture, top left corner at (x, y).
    pub fn sprite(&mut self, sprite: &Sprite, x: i32, y: i32) {
        for row in 0..sprite.height {
            for column in 0..sprite.width {
                if let Some(slot) = sprite.pixels[(row * sprite.width + column) as usize] {
                    self.set(x + column, y + row, slot);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(frame: &Frame, slot: u8) -> usize {
        frame.pixels().iter().filter(|p| **p == slot).count()
    }

    #[test]
    fn rectangles_meet_and_join() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 8, 10, 10);
        assert_eq!(a.intersection(&b), Rect::new(5, 8, 5, 2));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 18));
        assert!(a.intersection(&Rect::new(20, 20, 5, 5)).is_empty());
        assert_eq!(a.union(&Rect::default()), a);
        assert_eq!(a.inset(2), Rect::new(2, 2, 6, 6));
        assert!(a.contains(9, 9));
        assert!(!a.contains(10, 9));
    }

    #[test]
    fn drawing_outside_the_screen_is_cut() {
        let mut frame = Frame::new();
        frame.fill(Rect::new(-5, -5, 10, 10), 7);
        frame.fill(Rect::new(WIDTH - 2, HEIGHT - 2, 10, 10), 7);
        frame.set(-1, 0, 7);
        frame.set(WIDTH, 0, 7);
        assert_eq!(count(&frame, 7), 25 + 4);
        assert_eq!(frame.pixel(4, 4), Some(7));
        assert_eq!(frame.pixel(5, 5), Some(0));
        assert_eq!(frame.pixel(WIDTH, 0), None);
    }

    #[test]
    fn a_clip_restricts_drawing_until_it_is_put_back() {
        let mut frame = Frame::new();
        let before = frame.clip_to(Rect::new(10, 10, 4, 4));
        frame.fill(Rect::SCREEN, 3);
        assert_eq!(count(&frame, 3), 16);

        frame.set_clip(before);
        frame.fill(Rect::new(0, 0, 2, 2), 3);
        assert_eq!(count(&frame, 3), 20);
    }

    #[test]
    fn an_outline_is_one_pixel_wide_and_inside() {
        let mut frame = Frame::new();
        frame.outline(Rect::new(2, 2, 5, 4), 1);
        assert_eq!(count(&frame, 1), 14);
        assert_eq!(frame.pixel(2, 2), Some(1));
        assert_eq!(frame.pixel(6, 5), Some(1));
        assert_eq!(frame.pixel(3, 3), Some(0));
        assert_eq!(frame.pixel(7, 2), Some(0));
    }

    #[test]
    fn a_dither_covers_the_asked_share_of_the_pixels() {
        for (level, expected) in [(0, 0), (4, 16), (8, 32), (16, 64)] {
            let mut frame = Frame::new();
            frame.dither(Rect::new(0, 0, 8, 8), 5, level);
            assert_eq!(count(&frame, 5), expected, "level {level}");
        }
    }

    #[test]
    fn a_checker_paints_every_other_pixel() {
        let mut frame = Frame::new();
        frame.checker(Rect::new(0, 0, 4, 2), 9);
        assert_eq!(count(&frame, 9), 4);
        assert_eq!(frame.pixel(0, 0), Some(9));
        assert_eq!(frame.pixel(1, 0), Some(0));
        assert_eq!(frame.pixel(1, 1), Some(9));
    }

    #[test]
    fn a_gradient_goes_from_one_colour_to_the_other() {
        let mut frame = Frame::new();
        frame.gradient(Rect::new(0, 0, 16, 64), 1, 2);
        let share = |row: i32| (0..16).filter(|x| frame.pixel(*x, row) == Some(2)).count();
        assert_eq!(share(0), 0);
        assert!(share(32) > 4 && share(32) < 12);
        assert_eq!(share(63), 16);
    }

    #[test]
    fn an_image_is_drawn_where_asked_and_cut_at_the_edges() {
        let image = Image { width: 2, height: 2, pixels: vec![1, 2, 3, 4] };
        let mut frame = Frame::new();
        frame.blit(&image, 10, 10);
        assert_eq!(frame.pixel(10, 10), Some(1));
        assert_eq!(frame.pixel(11, 11), Some(4));

        frame.blit(&image, WIDTH - 1, 0);
        assert_eq!(frame.pixel(WIDTH - 1, 0), Some(1));
        assert_eq!(frame.pixel(WIDTH - 1, 1), Some(3));
    }

    #[test]
    fn a_sprite_leaves_what_is_under_its_transparent_pixels() {
        let sprite = Sprite::from_art(&["#.", ".#", "#"], &[('#', 4)]);
        assert_eq!((sprite.width, sprite.height), (2, 3));

        let mut frame = Frame::new();
        frame.fill(Rect::new(0, 0, 2, 3), 6);
        frame.sprite(&sprite, 0, 0);
        assert_eq!(frame.pixel(0, 0), Some(4));
        assert_eq!(frame.pixel(1, 0), Some(6));
        assert_eq!(frame.pixel(1, 1), Some(4));
        assert_eq!(frame.pixel(1, 2), Some(6));
    }
}
