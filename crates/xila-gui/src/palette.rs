//! Colours: palette slots, and what they look like.
//!
//! The client draws with palette slots only. What a slot looks like comes
//! from the `xila.palette` table once a session is open. Before that, on the
//! connection screen, the client uses its own copy of the default palette.

use xila_core::api::{Colour, PaletteColour};

/// The slots of the system palette the client draws its own decor with.
pub mod slot {
    pub const INK: u8 = 0;
    pub const NIGHT: u8 = 1;
    pub const NAVY: u8 = 2;
    pub const BLUE: u8 = 3;
    pub const AZURE: u8 = 4;
    pub const ICE: u8 = 5;
    pub const CREAM: u8 = 6;
    pub const WHITE: u8 = 7;
    pub const SAND: u8 = 8;
    pub const OCHRE: u8 = 9;
    pub const COCOA: u8 = 11;
    pub const GREY: u8 = 13;
    pub const SILVER: u8 = 14;
    pub const PEARL: u8 = 15;
    pub const FERN: u8 = 17;
    pub const GREEN: u8 = 18;
    pub const TURQUOISE: u8 = 21;
    pub const YELLOW: u8 = 22;
    pub const AMBER: u8 = 23;
    pub const RED: u8 = 25;
    pub const VIOLET: u8 = 29;
}

/// The default system palette, for the connection screen only. It is a copy
/// of what the extension writes in `xila.palette`; a test compares the two.
pub const BOOT: [u32; 32] = [
    0x14121f, 0x232a4d, 0x2f4a8a, 0x3f73c4, 0x6fb0e8, 0xbfe3f5, 0xf6ecd2, 0xfffdf5, 0xe3cf9f,
    0xc2a06a, 0x8a6240, 0x573a2e, 0x3d3f52, 0x6c7086, 0xa4a8b8, 0xd6d8e0, 0x1f5a48, 0x2f9a5c,
    0x7fd06a, 0xc9f0a8, 0x1f8a8f, 0x4fd0c2, 0xf7d046, 0xf0a030, 0xe8703a, 0xd0414a, 0x8a2846,
    0xf08aa0, 0xf8c8a8, 0x6a4aa8, 0xa88ae0, 0x3c2452,
];

/// What each of the 256 slots looks like, as `0x00RRGGBB`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    rgb: [u32; 256],
}

impl Palette {
    /// The palette of the connection screen.
    pub fn boot() -> Self {
        let mut rgb = [BOOT[0]; 256];
        rgb[..BOOT.len()].copy_from_slice(&BOOT);
        Palette { rgb }
    }

    /// The palette of a session: the system colours of the server, then the
    /// extra colours of the stack on screen. A slot nobody defines keeps the
    /// colour it has on the connection screen.
    pub fn of_session(system: &[PaletteColour], stack: &[Colour]) -> Self {
        let mut palette = Palette::boot();
        for colour in system {
            if let Some((r, g, b)) = colour.components() {
                palette.rgb[usize::from(colour.slot)] = pack(r, g, b);
            }
        }
        for colour in stack {
            if let Some(rgb) = parse(&colour.rgb) {
                palette.rgb[usize::from(colour.slot)] = rgb;
            }
        }
        palette
    }

    pub fn rgb(&self, slot: u8) -> u32 {
        self.rgb[usize::from(slot)]
    }
}

fn pack(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

/// Reads `#rrggbb`.
fn parse(text: &str) -> Option<u32> {
    let hex = text.strip_prefix('#').filter(|hex| hex.len() == 6)?;
    u32::from_str_radix(hex, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_boot_palette_has_the_colours_of_the_mockups() {
        let palette = Palette::boot();
        assert_eq!(palette.rgb(slot::INK), 0x14121f);
        assert_eq!(palette.rgb(slot::CREAM), 0xf6ecd2);
        assert_eq!(palette.rgb(31), 0x3c2452);
        // Slots of a stack that are not defined are drawn as ink.
        assert_eq!(palette.rgb(200), 0x14121f);
    }

    #[test]
    fn the_server_decides_what_a_slot_looks_like() {
        let system = [
            PaletteColour { slot: 0, name: "black".into(), rgb: "#000000".into() },
            PaletteColour { slot: 6, name: "cream".into(), rgb: "not a colour".into() },
        ];
        let stack = [Colour { slot: 40, rgb: "#102030".into(), name: None }];
        let palette = Palette::of_session(&system, &stack);

        assert_eq!(palette.rgb(0), 0x000000);
        assert_eq!(palette.rgb(6), 0xf6ecd2, "a colour that cannot be read is ignored");
        assert_eq!(palette.rgb(40), 0x102030);
        assert_eq!(palette.rgb(7), 0xfffdf5);
    }
}
