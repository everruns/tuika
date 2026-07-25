//! OSC 22 mouse pointer shapes.
//!
//! Another out-of-band escape in the [`term`](crate::term) family: it changes
//! the shape of the *terminal's own* mouse pointer, which lives outside the cell
//! grid entirely. Interactive regions use it to signal that something under the
//! pointer is clickable — see [`crate::term::hyperlink`] for the link half.

use std::io::{self, Write};

/// Mouse pointer shapes used by interactive terminal regions.
///
/// These are CSS cursor names, understood by Ghostty, Kitty, Foot, and recent
/// iTerm2/xterm releases through OSC 22. Other terminals ignore the sequence.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PointerShape {
    /// Restore the terminal's configured pointer.
    #[default]
    Default,
    /// Show the pointing-hand cursor used for links.
    Pointer,
}

/// Encode an OSC 22 pointer-shape sequence. Pure and unit-testable — no I/O.
pub fn encode(shape: PointerShape) -> &'static str {
    match shape {
        PointerShape::Default => "\x1b]22;default\x1b\\",
        PointerShape::Pointer => "\x1b]22;pointer\x1b\\",
    }
}

/// Set the terminal's mouse pointer shape.
pub fn write(out: &mut impl Write, shape: PointerShape) -> io::Result<()> {
    out.write_all(encode(shape).as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc_pointer_shape_encoding() {
        assert_eq!(encode(PointerShape::Default), "\x1b]22;default\x1b\\");
        assert_eq!(encode(PointerShape::Pointer), "\x1b]22;pointer\x1b\\");
    }
}
