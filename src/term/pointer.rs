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
    /// Text insertion cursor.
    Text,
    /// Horizontal splitter or resize handle.
    EwResize,
    /// Vertical splitter or resize handle.
    NsResize,
    /// An object that can be grabbed.
    Grab,
    /// An object currently being dragged.
    Grabbing,
    /// An unavailable action.
    NotAllowed,
}

/// Encode an OSC 22 pointer-shape sequence. Pure and unit-testable — no I/O.
pub fn encode(shape: PointerShape) -> &'static str {
    match shape {
        PointerShape::Default => "\x1b]22;default\x1b\\",
        PointerShape::Pointer => "\x1b]22;pointer\x1b\\",
        PointerShape::Text => "\x1b]22;text\x1b\\",
        PointerShape::EwResize => "\x1b]22;ew-resize\x1b\\",
        PointerShape::NsResize => "\x1b]22;ns-resize\x1b\\",
        PointerShape::Grab => "\x1b]22;grab\x1b\\",
        PointerShape::Grabbing => "\x1b]22;grabbing\x1b\\",
        PointerShape::NotAllowed => "\x1b]22;not-allowed\x1b\\",
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
        for (shape, name) in [
            (PointerShape::Text, "text"),
            (PointerShape::EwResize, "ew-resize"),
            (PointerShape::NsResize, "ns-resize"),
            (PointerShape::Grab, "grab"),
            (PointerShape::Grabbing, "grabbing"),
            (PointerShape::NotAllowed, "not-allowed"),
        ] {
            let mut output = Vec::new();
            write(&mut output, shape).unwrap();
            assert_eq!(output, format!("\x1b]22;{name}\x1b\\").as_bytes());
        }
    }
}
