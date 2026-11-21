//! Following the OS dark/light preference while the app runs (mode 2031).
//!
//! A theme read once at startup goes stale the moment the user flips their
//! desktop to dark mode at sunset. Terminals that implement the color-palette
//! update notifications — kitty, Ghostty, Contour, foot, recent WezTerm — will
//! tell the application instead:
//!
//! - [`SUBSCRIBE`] (`CSI ? 2031 h`) asks for an unsolicited report on every
//!   change, and [`UNSUBSCRIBE`] (`CSI ? 2031 l`) stops it;
//! - [`QUERY`] (`CSI ? 996 n`) asks for the current preference once;
//! - each answer is `CSI ? 997 ; 1 n` for dark or `CSI ? 997 ; 2 n` for light,
//!   decoded by [`ColorScheme::parse_report`].
//!
//! A terminal that does not implement the mode ignores all three, so subscribing
//! is safe everywhere; the app simply keeps its startup theme.
//!
//! The reports arrive on stdin among keystrokes, at any moment, which is why
//! [`Runner::follow_color_scheme`](crate::Runner::follow_color_scheme) reads
//! input with tuika's own decoder while it is on: crossterm does not recognise
//! the report and would hold every following key while waiting for it to end.

/// Subscribe to color-scheme change reports (`CSI ? 2031 h`).
pub const SUBSCRIBE: &str = "\x1b[?2031h";

/// Stop color-scheme change reports (`CSI ? 2031 l`).
pub const UNSUBSCRIBE: &str = "\x1b[?2031l";

/// Ask for the current color scheme once (`CSI ? 996 n`).
pub const QUERY: &str = "\x1b[?996n";

/// The OS-level appearance preference a terminal reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorScheme {
    /// Dark mode.
    Dark,
    /// Light mode.
    Light,
}

impl ColorScheme {
    /// Parse a report, `CSI ? 997 ; 1 n` (dark) or `CSI ? 997 ; 2 n` (light).
    ///
    /// `bytes` must be exactly one report; anything else, including an unknown
    /// value, is `None`.
    pub fn parse_report(bytes: &[u8]) -> Option<Self> {
        match bytes {
            b"\x1b[?997;1n" => Some(Self::Dark),
            b"\x1b[?997;2n" => Some(Self::Light),
            _ => None,
        }
    }

    /// The scheme a terminal's background implies: dark when its relative
    /// luminance is below one half. Useful to seed the scheme from
    /// [`TerminalPalette`](crate::term::palette::TerminalPalette) on a terminal
    /// that answers OSC 11 but not mode 2031.
    pub fn from_background(color: crate::style::Color) -> Option<Self> {
        let crate::style::Color::Rgb(r, g, b) = color else {
            return None;
        };
        let luminance = 0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b);
        Some(if luminance < 128.0 {
            Self::Dark
        } else {
            Self::Light
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Color;

    #[test]
    fn reports_parse_to_their_scheme() {
        assert_eq!(
            ColorScheme::parse_report(b"\x1b[?997;1n"),
            Some(ColorScheme::Dark)
        );
        assert_eq!(
            ColorScheme::parse_report(b"\x1b[?997;2n"),
            Some(ColorScheme::Light)
        );
        for other in [
            &b"\x1b[?997;3n"[..],
            b"\x1b[?997;1",
            b"\x1b[?996n",
            b"x\x1b[?997;1n",
        ] {
            assert_eq!(ColorScheme::parse_report(other), None, "{other:?}");
        }
    }

    #[test]
    fn background_luminance_picks_a_scheme() {
        assert_eq!(
            ColorScheme::from_background(Color::Rgb(0x1e, 0x1e, 0x2e)),
            Some(ColorScheme::Dark)
        );
        assert_eq!(
            ColorScheme::from_background(Color::Rgb(0xfd, 0xf6, 0xe3)),
            Some(ColorScheme::Light)
        );
        assert_eq!(ColorScheme::from_background(Color::Reset), None);
    }
}
