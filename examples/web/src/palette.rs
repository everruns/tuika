//! Resolve a ratatui [`Color`] to a concrete RGB triple.
//!
//! A terminal does this itself: it owns the 16-color palette and the 256-color
//! cube, and `Color::Reset` means "whatever the user's profile says". A canvas
//! owns none of that, so the host has to resolve every color to real pixels
//! before it can paint one. Doing it in Rust — rather than shipping the tables
//! to JavaScript — keeps the wasm/JS boundary a flat array of RGB values.

use ratatui_core::style::Color;

/// The xterm 16-color base palette, in ANSI order (`Black` … `LightWhite`).
///
/// Any palette would do; this is the widely used xterm default, so the demo
/// matches what a terminal user would see with a stock profile.
const ANSI16: [u32; 16] = [
    0x00_00_00, 0x80_00_00, 0x00_80_00, 0x80_80_00, 0x00_00_80, 0x80_00_80, 0x00_80_80, 0xc0_c0_c0,
    0x80_80_80, 0xff_00_00, 0x00_ff_00, 0xff_ff_00, 0x00_00_ff, 0xff_00_ff, 0x00_ff_ff, 0xff_ff_ff,
];

/// The 6 levels of the xterm 6×6×6 color cube.
const CUBE: [u32; 6] = [0, 95, 135, 175, 215, 255];

/// Resolve `color` to `0x00RRGGBB`, falling back to `default` for
/// [`Color::Reset`] (which has no value of its own — the terminal substitutes
/// the profile's foreground or background, so the host must say which).
pub fn rgb(color: Color, default: u32) -> u32 {
    match color {
        Color::Reset => default,
        Color::Black => ANSI16[0],
        Color::Red => ANSI16[1],
        Color::Green => ANSI16[2],
        Color::Yellow => ANSI16[3],
        Color::Blue => ANSI16[4],
        Color::Magenta => ANSI16[5],
        Color::Cyan => ANSI16[6],
        Color::Gray => ANSI16[7],
        Color::DarkGray => ANSI16[8],
        Color::LightRed => ANSI16[9],
        Color::LightGreen => ANSI16[10],
        Color::LightYellow => ANSI16[11],
        Color::LightBlue => ANSI16[12],
        Color::LightMagenta => ANSI16[13],
        Color::LightCyan => ANSI16[14],
        Color::White => ANSI16[15],
        Color::Rgb(r, g, b) => u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
        Color::Indexed(i) => indexed(i),
    }
}

/// Resolve one of the 256 palette slots: 0–15 are the base palette, 16–231 the
/// 6×6×6 cube, 232–255 the 24-step grayscale ramp.
fn indexed(i: u8) -> u32 {
    match i {
        0..=15 => ANSI16[i as usize],
        16..=231 => {
            let i = u32::from(i) - 16;
            let (r, g, b) = (i / 36, (i / 6) % 6, i % 6);
            CUBE[r as usize] << 16 | CUBE[g as usize] << 8 | CUBE[b as usize]
        }
        232..=255 => {
            let level = 8 + (u32::from(i) - 232) * 10;
            level << 16 | level << 8 | level
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_takes_the_supplied_default() {
        assert_eq!(rgb(Color::Reset, 0x12_34_56), 0x12_34_56);
    }

    #[test]
    fn indexed_covers_base_cube_and_ramp() {
        assert_eq!(rgb(Color::Indexed(1), 0), 0x80_00_00);
        assert_eq!(rgb(Color::Indexed(196), 0), 0xff_00_00);
        assert_eq!(rgb(Color::Indexed(232), 0), 0x08_08_08);
    }
}
