//! From the 480 by 300 screen to the pixels of the window.
//!
//! The screen is enlarged by a whole factor, the largest that fits, and
//! centred. Pixels stay square and sharp at any window size.

use crate::frame::{Frame, HEIGHT, WIDTH};
use crate::palette::{slot, Palette};

/// Where the screen is shown in a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    /// Window pixels per screen pixel.
    pub scale: u32,
    /// Top left corner of the screen in the window.
    pub x: u32,
    pub y: u32,
}

impl Viewport {
    /// The viewport of a window of a given size, in physical pixels.
    pub fn fit(width: u32, height: u32) -> Self {
        let scale = (width / WIDTH as u32).min(height / HEIGHT as u32).max(1);
        Viewport {
            scale,
            x: width.saturating_sub(WIDTH as u32 * scale) / 2,
            y: height.saturating_sub(HEIGHT as u32 * scale) / 2,
        }
    }

    /// The screen pixel under a point of the window, if there is one.
    pub fn to_screen(&self, x: f64, y: f64) -> Option<(i32, i32)> {
        let column = ((x - f64::from(self.x)) / f64::from(self.scale)).floor();
        let row = ((y - f64::from(self.y)) / f64::from(self.scale)).floor();
        let inside =
            column >= 0.0 && row >= 0.0 && column < f64::from(WIDTH) && row < f64::from(HEIGHT);
        inside.then_some((column as i32, row as i32))
    }
}

/// The largest whole factor at which the screen fits in a given size.
pub fn largest_scale(width: f64, height: f64, at_most: u32) -> u32 {
    let fitting = (width / f64::from(WIDTH)).min(height / f64::from(HEIGHT)).floor();
    (fitting as u32).clamp(1, at_most)
}

/// Paints the screen into the pixels of a window, `0x00RRGGBB` each, row by
/// row. What is around the screen is painted in the darkest colour.
pub fn paint(frame: &Frame, palette: &Palette, target: &mut [u32], width: u32, height: u32) {
    let (width, height) = (width as usize, height as usize);
    if width == 0 || target.len() < width * height {
        return;
    }
    let viewport = Viewport::fit(width as u32, height as u32);
    let scale = viewport.scale as usize;
    target.fill(palette.rgb(slot::INK));

    let shown_width = (WIDTH as usize * scale).min(width - viewport.x as usize);
    let mut line = vec![0u32; WIDTH as usize * scale];
    for row in 0..HEIGHT as usize {
        let pixels = &frame.pixels()[row * WIDTH as usize..(row + 1) * WIDTH as usize];
        for (column, slot) in pixels.iter().enumerate() {
            line[column * scale..(column + 1) * scale].fill(palette.rgb(*slot));
        }
        for repeat in 0..scale {
            let y = viewport.y as usize + row * scale + repeat;
            if y >= height {
                return;
            }
            let start = y * width + viewport.x as usize;
            target[start..start + shown_width].copy_from_slice(&line[..shown_width]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::Rect;

    #[test]
    fn the_screen_is_enlarged_by_the_largest_whole_factor_and_centred() {
        assert_eq!(Viewport::fit(1440, 900), Viewport { scale: 3, x: 0, y: 0 });
        assert_eq!(Viewport::fit(1500, 1000), Viewport { scale: 3, x: 30, y: 50 });
        assert_eq!(Viewport::fit(2880, 1800), Viewport { scale: 6, x: 0, y: 0 });
        // Too small a window still shows the screen, cut.
        assert_eq!(Viewport::fit(300, 200), Viewport { scale: 1, x: 0, y: 0 });
    }

    #[test]
    fn a_point_of_the_window_is_a_pixel_of_the_screen() {
        let viewport = Viewport::fit(1500, 1000);
        assert_eq!(viewport.to_screen(30.0, 50.0), Some((0, 0)));
        assert_eq!(viewport.to_screen(32.9, 52.9), Some((0, 0)));
        assert_eq!(viewport.to_screen(33.0, 53.0), Some((1, 1)));
        assert_eq!(viewport.to_screen(1469.9, 949.9), Some((479, 299)));
        assert_eq!(viewport.to_screen(29.9, 60.0), None);
        assert_eq!(viewport.to_screen(1470.0, 60.0), None);
    }

    #[test]
    fn the_starting_size_fits_the_display() {
        assert_eq!(largest_scale(1512.0, 900.0, 3), 3);
        assert_eq!(largest_scale(1280.0, 720.0, 3), 2);
        assert_eq!(largest_scale(3840.0, 2160.0, 3), 3);
        assert_eq!(largest_scale(400.0, 200.0, 3), 1);
    }

    #[test]
    fn each_pixel_becomes_a_square_of_its_colour() {
        let mut frame = Frame::new();
        frame.fill(Rect::SCREEN, slot::CREAM);
        frame.set(0, 0, slot::RED);
        frame.set(479, 299, slot::GREEN);
        let palette = Palette::boot();
        let (width, height) = (480 * 2 + 10, 300 * 2 + 6);
        let mut target = vec![0xdead_beef; (width * height) as usize];
        paint(&frame, &palette, &mut target, width, height);

        let at = |x: u32, y: u32| target[(y * width + x) as usize];
        // The border, then the screen two pixels for one.
        assert_eq!(at(0, 0), palette.rgb(slot::INK));
        assert_eq!(at(4, 3), palette.rgb(slot::INK));
        assert_eq!(at(5, 3), palette.rgb(slot::RED));
        assert_eq!(at(6, 4), palette.rgb(slot::RED));
        assert_eq!(at(7, 3), palette.rgb(slot::CREAM));
        assert_eq!(at(5, 5), palette.rgb(slot::CREAM));
        assert_eq!(at(5 + 959, 3 + 599), palette.rgb(slot::GREEN));
        assert_eq!(at(5 + 960, 3 + 599), palette.rgb(slot::INK));
        assert!(!target.contains(&0xdead_beef));
    }

    #[test]
    fn a_window_smaller_than_the_screen_is_painted_without_overflow() {
        let frame = Frame::new();
        let palette = Palette::boot();
        let mut target = vec![0; 100 * 50];
        paint(&frame, &palette, &mut target, 100, 50);
        paint(&frame, &palette, &mut [], 0, 0);
        paint(&frame, &palette, &mut target, 1000, 1000);
    }
}
