//! Inheriting the terminal's own colors.
//!
//! A [`Theme`] is normally chosen by the application — one of the bundled
//! presets, or a literal the host writes. This module covers the other case: the
//! user already picked a palette *in their terminal*, and the application should
//! look like it belongs there rather than imposing its own.
//!
//! There are two ways to do that, and they trade accuracy against cost:
//!
//! - [`themes::TERMINAL`](crate::themes::TERMINAL) — a `const` [`Theme`] built
//!   entirely from [`Color::Reset`] and [`Color::Indexed`], so the terminal
//!   resolves every color itself. No I/O, no timeout, no failure mode, works on
//!   every terminal including ones that answer nothing. What it cannot do is
//!   produce a tone *between* two ANSI slots, so panels, rules, and selection
//!   bands are flatter than a designed palette.
//! - **Querying the terminal** for its actual colors — [`TerminalPalette::query`]
//!   — then deriving a full theme from them with [`Theme::from_terminal`]. This
//!   yields real 24-bit values, so the in-between tones can be blended and
//!   contrast-checked. It costs one round-trip at startup and degrades to
//!   `None` on a terminal that stays silent.
//!
//! ```no_run
//! use std::time::Duration;
//! use tuika::{Theme, themes};
//! use tuika::term::palette::TerminalPalette;
//!
//! // In raw mode, before the event loop reads stdin:
//! let theme = Theme::from_terminal(&TerminalPalette::query(Duration::from_millis(100)))
//!     .unwrap_or(themes::TERMINAL); // silent terminal → inherit via ANSI slots
//! ```
//!
//! # The queries
//!
//! The terminal is asked with the xterm color-query escapes — [`QUERY_FOREGROUND`]
//! (OSC 10), [`QUERY_BACKGROUND`] (OSC 11), and OSC 4 per palette entry — each
//! answered with `OSC <n> ; rgb:RRRR/GGGG/BBBB ST`. Ghostty, kitty, WezTerm,
//! foot, alacritty, contour, iTerm2, and xterm all implement them; others answer
//! nothing at all.
//!
//! "Nothing at all" is the whole difficulty: a terminal that does not implement
//! OSC 11 does not say so, it just stays quiet, and a reader cannot tell *not
//! yet* from *never*. So [`query_sequence`] appends
//! [`DA1_REQUEST`](crate::term::capabilities::DA1_REQUEST) — which every terminal
//! answers — as a **fence**: once the DA1 reply arrives, every color reply that
//! was ever coming has already arrived. That makes the probe cost one round-trip
//! rather than one timeout, and it folds into the Device Attributes round-trip
//! tuika already makes (see
//! [`Capabilities::query_with_palette`](crate::term::capabilities::Capabilities::query_with_palette)).

use crate::style::Color;

use crate::style::{CodeTheme, Theme};

/// Query the terminal's default foreground (OSC 10). The reply is
/// `OSC 10 ; rgb:RRRR/GGGG/BBBB ST`.
pub const QUERY_FOREGROUND: &str = "\x1b]10;?\x1b\\";

/// Query the terminal's default background (OSC 11). The reply is
/// `OSC 11 ; rgb:RRRR/GGGG/BBBB ST`.
pub const QUERY_BACKGROUND: &str = "\x1b]11;?\x1b\\";

/// The bytes to write to ask the terminal for its whole palette: the default
/// foreground and background plus all 16 ANSI entries, fenced by
/// [`DA1_REQUEST`](crate::term::capabilities::DA1_REQUEST) so the reply stream has a
/// definite end even on a terminal that answers none of the color queries.
///
/// Feed everything read back to [`TerminalPalette::parse`].
pub fn query_sequence() -> String {
    let mut out = String::with_capacity(24 * 20);
    out.push_str(QUERY_FOREGROUND);
    out.push_str(QUERY_BACKGROUND);
    for i in 0..16 {
        out.push_str(&format!("\x1b]4;{i};?\x1b\\"));
    }
    // The fence must come last: it is what tells the reader the color replies
    // are done (or were never coming).
    out.push_str(crate::term::capabilities::DA1_REQUEST);
    out
}

/// The colors a terminal reported for itself.
///
/// Every field is optional because a terminal may implement some queries and not
/// others — or none. Build one from a reply stream with [`parse`](Self::parse),
/// or from a live terminal with [`query`](Self::query), then turn it into a
/// theme with [`Theme::from_terminal`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TerminalPalette {
    /// The default foreground (OSC 10), if reported.
    pub foreground: Option<Color>,
    /// The default background (OSC 11), if reported.
    pub background: Option<Color>,
    /// The 16 ANSI palette entries (OSC 4), each if reported. Index 0–7 are the
    /// normal colors, 8–15 the bright ones.
    pub ansi: [Option<Color>; 16],
}

impl TerminalPalette {
    /// Parse every color reply found in `bytes`, ignoring anything else.
    ///
    /// Tolerant by design: the buffer is whatever the terminal sent, which may
    /// interleave the Device Attributes reply, hold only some of the answers, or
    /// be truncated mid-sequence. Unrecognized, malformed, and partial replies
    /// are skipped rather than failing the whole parse, so a terminal that
    /// answers three queries out of eighteen still contributes three colors.
    pub fn parse(bytes: &[u8]) -> Self {
        let mut out = Self::default();
        let mut i = 0;
        while i + 1 < bytes.len() {
            if bytes[i] != 0x1b || bytes[i + 1] != b']' {
                i += 1;
                continue;
            }
            let body_start = i + 2;
            let Some((body, next)) = osc_body(bytes, body_start) else {
                // An unterminated OSC is the last thing in the buffer; there is
                // nothing after it to parse.
                break;
            };
            out.absorb(body);
            i = next;
        }
        out
    }

    /// Ask the terminal directly: write [`query_sequence`] and parse the reply.
    ///
    /// **Call once at startup, after entering raw mode and before the event loop
    /// reads stdin** — the replies arrive on stdin and would otherwise be
    /// delivered to the application as key events. On a non-tty, a non-Unix
    /// platform, or a terminal that answers nothing, this returns an empty
    /// palette, which [`Theme::from_terminal`] turns into `None`.
    ///
    /// `timeout` bounds the wait for the fence reply; a real tty answers Device
    /// Attributes immediately, so this normally returns at once.
    pub fn query(timeout: std::time::Duration) -> Self {
        match crate::term::capabilities::probe_terminal(&query_sequence(), timeout) {
            Some(bytes) => Self::parse(&bytes),
            None => Self::default(),
        }
    }

    /// Whether the terminal reported nothing at all.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Record one OSC body (the bytes between `ESC ]` and the terminator).
    fn absorb(&mut self, body: &[u8]) {
        let mut parts = body.split(|&b| b == b';');
        let Some(code) = parts.next() else { return };
        match code {
            b"10" => self.foreground = parts.next().and_then(parse_color_spec),
            b"11" => self.background = parts.next().and_then(parse_color_spec),
            b"4" => {
                // `4;<index>;<spec>` — and a terminal may pack several pairs into
                // one reply, so keep consuming them.
                while let (Some(index), Some(spec)) = (parts.next(), parts.next()) {
                    let Some(index) = std::str::from_utf8(index)
                        .ok()
                        .and_then(|s| s.parse::<usize>().ok())
                    else {
                        return;
                    };
                    if let Some(slot) = self.ansi.get_mut(index) {
                        *slot = parse_color_spec(spec);
                    }
                }
            }
            _ => {}
        }
    }

    /// ANSI entry `index`, falling back to xterm's default for that slot when
    /// the terminal did not report one.
    fn ansi_or_default(&self, index: usize) -> Rgb {
        self.ansi
            .get(index)
            .copied()
            .flatten()
            .and_then(rgb_of)
            .unwrap_or(XTERM_DEFAULTS[index.min(15)])
    }
}

/// Find the body of an OSC sequence starting at `start`, plus the index just
/// past its terminator. Accepts both terminators a terminal may use: `ESC \`
/// (ST) and `BEL`. Returns `None` if the sequence is unterminated.
fn osc_body(bytes: &[u8], start: usize) -> Option<(&[u8], usize)> {
    let mut j = start;
    while j < bytes.len() {
        match bytes[j] {
            0x07 => return Some((&bytes[start..j], j + 1)),
            0x1b if bytes.get(j + 1) == Some(&b'\\') => return Some((&bytes[start..j], j + 2)),
            // A bare ESC that is not ST starts a *different* sequence: the OSC
            // was truncated. Stop rather than swallowing the next sequence.
            0x1b => return None,
            _ => j += 1,
        }
    }
    None
}

/// Parse one X11 color spec into a [`Color::Rgb`].
///
/// Handles the `rgb:R/G/B` form every terminal replies with — components are 1
/// to 4 hex digits each and are scaled to 8 bits — plus the `#RGB`-style forms
/// and `rgba:` variants some terminals use.
fn parse_color_spec(spec: &[u8]) -> Option<Color> {
    let spec = std::str::from_utf8(spec).ok()?.trim();
    if let Some(rest) = spec
        .strip_prefix("rgb:")
        .or_else(|| spec.strip_prefix("rgba:"))
    {
        // `rgba:` puts alpha first (`rgba:A/R/G/B`); tuika has no alpha, so drop it.
        let mut fields: Vec<&str> = rest.split('/').collect();
        if fields.len() == 4 {
            fields.remove(0);
        }
        if fields.len() != 3 {
            return None;
        }
        let r = scale_hex(fields[0])?;
        let g = scale_hex(fields[1])?;
        let b = scale_hex(fields[2])?;
        return Some(Color::Rgb(r, g, b));
    }
    if let Some(hex) = spec.strip_prefix('#') {
        // `#RGB`, `#RRGGBB`, `#RRRGGGBBB`, `#RRRRGGGGBBBB` — equal-width thirds.
        if hex.len() % 3 != 0 || hex.is_empty() || hex.len() > 12 {
            return None;
        }
        let w = hex.len() / 3;
        let r = scale_hex(&hex[0..w])?;
        let g = scale_hex(&hex[w..2 * w])?;
        let b = scale_hex(&hex[2 * w..])?;
        return Some(Color::Rgb(r, g, b));
    }
    None
}

/// Scale a 1–4 digit hex component to 8 bits, the way X11 widens color specs:
/// the value is a fraction of its own maximum (`ff` → 255, `f` → 255, `8` → 136).
fn scale_hex(field: &str) -> Option<u8> {
    if field.is_empty() || field.len() > 4 || !field.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(field, 16).ok()?;
    let max = (1u32 << (4 * field.len() as u32)) - 1;
    // Round to nearest rather than truncating, so `rgb:8/8/8` is not off by one.
    Some(((value * 255 + max / 2) / max) as u8)
}

// --- Color math -----------------------------------------------------------
//
// Blending is a plain lerp in sRGB rather than a perceptual space. The tones
// tuika derives are all blends between two colors the *user* already chose to
// sit together (their foreground, background, and accents), so the extra
// machinery of Oklab buys accuracy the eye cannot spend here — and it would put
// a color-science dependency into a toolkit that deliberately has none.

/// A 24-bit color, used for the arithmetic between parsing and the [`Theme`].
type Rgb = (u8, u8, u8);

const BLACK: Rgb = (0, 0, 0);
const WHITE: Rgb = (255, 255, 255);

/// xterm's default 16-color palette, used for any ANSI slot the terminal did not
/// report so a terminal that answers OSC 10/11 but not OSC 4 still yields a
/// complete theme.
const XTERM_DEFAULTS: [Rgb; 16] = [
    (0, 0, 0),
    (205, 0, 0),
    (0, 205, 0),
    (205, 205, 0),
    (0, 0, 238),
    (205, 0, 205),
    (0, 205, 205),
    (229, 229, 229),
    (127, 127, 127),
    (255, 0, 0),
    (0, 255, 0),
    (255, 255, 0),
    (92, 92, 255),
    (255, 0, 255),
    (0, 255, 255),
    (255, 255, 255),
];

/// The RGB triple behind a [`Color`], or `None` for anything that is not a
/// literal 24-bit color (an indexed or named color has no value tuika can do
/// arithmetic on without knowing the terminal's palette).
fn rgb_of(color: Color) -> Option<Rgb> {
    match color {
        Color::Rgb(r, g, b) => Some((r, g, b)),
        _ => None,
    }
}

/// Blend `a` toward `b`; `t` of 0 is `a`, 1 is `b`.
fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| {
        (x as f32 + (y as f32 - x as f32) * t)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

/// WCAG relative luminance.
fn luminance(c: Rgb) -> f32 {
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(c.0) + 0.7152 * channel(c.1) + 0.0722 * channel(c.2)
}

/// WCAG contrast ratio between two colors, from 1.0 (identical) to 21.0.
fn contrast_ratio(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    let (hi, lo) = if la >= lb { (la, lb) } else { (lb, la) };
    (hi + 0.05) / (lo + 0.05)
}

/// Whether text on `bg` reads better in white than in black — the test that
/// decides which pole [`ensure_contrast`] pushes toward, and whether the bright
/// or the normal half of the ANSI palette is the legible one.
fn is_dark(bg: Rgb) -> bool {
    contrast_ratio(WHITE, bg) >= contrast_ratio(BLACK, bg)
}

/// Push `color` away from `bg` until it clears `min` contrast, blending toward
/// whichever pole `bg` is furthest from.
///
/// Colors the terminal *reported* are used verbatim — a user's own foreground is
/// their choice. This guards the colors tuika **derives**, where a blend can
/// otherwise land on top of the background and disappear. On a mid-gray
/// background no color reaches a high ratio; the loop then returns the pole,
/// which is the most legible answer available.
fn ensure_contrast(color: Rgb, bg: Rgb, min: f32) -> Rgb {
    if contrast_ratio(color, bg) >= min {
        return color;
    }
    let pole = if is_dark(bg) { WHITE } else { BLACK };
    for step in 1..=20 {
        let candidate = mix(color, pole, step as f32 / 20.0);
        if contrast_ratio(candidate, bg) >= min {
            return candidate;
        }
    }
    pole
}

impl Theme {
    /// Derive a full theme from the colors a terminal reported, or `None` if it
    /// did not report both a foreground and a background.
    ///
    /// The reported foreground and background are used **verbatim** as `text`
    /// and `background`, so an inherited theme sits in the terminal rather than
    /// beside it. Everything else is derived, because no terminal has a notion
    /// of a raised panel, a rule, or a selection band:
    ///
    /// - The in-between tones (`surface`, `muted`, `dim`, `border`, the code
    ///   background) are blends between the foreground and the background, so
    ///   they follow a light palette and a dark one without a special case.
    /// - The hues (`accent`, and the syntax roles) come from the ANSI palette,
    ///   taking the bright half on a dark background and the normal half on a
    ///   light one. Blue leads and cyan follows — a terminal reports no notion
    ///   of a *brand* color, so tuika picks the conventional interactive hue
    ///   rather than inventing one.
    /// - Every derived color is held to a minimum contrast against the
    ///   background. A user's palette is free to put two colors on top of each
    ///   other; a UI built from it is not.
    ///
    /// Any ANSI slot the terminal did not report falls back to xterm's default
    /// for that slot, so a terminal answering only OSC 10 and OSC 11 still
    /// produces a complete theme.
    pub fn from_terminal(palette: &TerminalPalette) -> Option<Theme> {
        let fg = palette.foreground.and_then(rgb_of)?;
        let bg = palette.background.and_then(rgb_of)?;
        let dark = is_dark(bg);
        // The legible half of the ANSI palette: bright on dark, normal on light.
        let hue = |normal: usize| palette.ansi_or_default(if dark { normal + 8 } else { normal });
        let color = |c: Rgb| Color::Rgb(c.0, c.1, c.2);
        let guard = |c: Rgb, min: f32| ensure_contrast(c, bg, min);

        // Text-weight roles clear WCAG AA for large text (3.0); structural roles
        // that are lines rather than glyphs need only to be visible (1.6).
        let muted = guard(mix(fg, bg, 0.40), 3.0);
        let accent = guard(hue(4), 3.0); // blue
        let accent_alt = guard(hue(6), 3.0); // cyan
        let selection_bg = mix(bg, accent, 0.35);
        // Whichever of the user's two colors reads better on the band we just built.
        let selection_fg = if contrast_ratio(fg, selection_bg) >= contrast_ratio(bg, selection_bg) {
            fg
        } else {
            bg
        };

        Some(Theme {
            background: color(bg),
            surface: color(mix(bg, fg, 0.10)),
            text: color(fg),
            muted: color(muted),
            dim: color(guard(mix(fg, bg, 0.68), 1.6)),
            accent: color(accent),
            accent_alt: color(accent_alt),
            border: color(guard(mix(fg, bg, 0.62), 1.6)),
            border_focused: color(accent),
            selection_bg: color(selection_bg),
            selection_fg: color(ensure_contrast(selection_fg, selection_bg, 4.5)),
            code: CodeTheme {
                heading: color(fg),
                link: color(accent),
                background: color(mix(bg, fg, 0.06)),
                text: color(fg),
                label: color(muted),
                keyword: color(guard(hue(5), 3.0)),   // magenta
                function: color(guard(hue(4), 3.0)),  // blue
                type_name: color(guard(hue(6), 3.0)), // cyan
                constant: color(guard(hue(3), 3.0)),  // yellow
                string: color(guard(hue(2), 3.0)),    // green
                comment: color(guard(palette.ansi_or_default(8), 2.2)),
                punctuation: color(muted),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rgb(c: Option<Color>) -> Rgb {
        rgb_of(c.expect("a color was parsed")).expect("a 24-bit color")
    }

    /// Ghostty's actual reply shape: OSC 11, four hex digits per channel, ST.
    #[test]
    fn parses_a_background_reply() {
        let p = TerminalPalette::parse(b"\x1b]11;rgb:1a1a/1b1b/1e1e\x1b\\");
        assert_eq!(p.background, Some(Color::Rgb(0x1a, 0x1b, 0x1e)));
        assert_eq!(p.foreground, None);
    }

    #[test]
    fn parses_both_terminators_and_all_component_widths() {
        // BEL-terminated, one digit per channel: `f` is full-scale, not 0x0f.
        let p = TerminalPalette::parse(b"\x1b]10;rgb:f/8/0\x07");
        assert_eq!(rgb(p.foreground), (255, 136, 0));
        // Two digits.
        let p = TerminalPalette::parse(b"\x1b]10;rgb:ff/80/00\x1b\\");
        assert_eq!(rgb(p.foreground), (255, 128, 0));
        // Three digits.
        let p = TerminalPalette::parse(b"\x1b]10;rgb:fff/800/000\x1b\\");
        assert_eq!(rgb(p.foreground), (255, 128, 0));
    }

    #[test]
    fn parses_hash_and_rgba_specs() {
        let p = TerminalPalette::parse(b"\x1b]11;#102030\x1b\\");
        assert_eq!(rgb(p.background), (0x10, 0x20, 0x30));
        // `rgba:` leads with alpha, which tuika drops.
        let p = TerminalPalette::parse(b"\x1b]11;rgba:ffff/1111/2222/3333\x1b\\");
        assert_eq!(rgb(p.background), (0x11, 0x22, 0x33));
    }

    #[test]
    fn parses_ansi_entries_by_index() {
        let p = TerminalPalette::parse(
            b"\x1b]4;0;rgb:0000/0000/0000\x1b\\\x1b]4;12;rgb:8888/aaaa/ffff\x1b\\",
        );
        assert_eq!(rgb(p.ansi[0]), (0, 0, 0));
        assert_eq!(rgb(p.ansi[12]), (0x88, 0xaa, 0xff));
        assert_eq!(p.ansi[1], None, "unreported slots stay empty");
    }

    #[test]
    fn parses_multiple_pairs_packed_into_one_osc_4_reply() {
        let p = TerminalPalette::parse(b"\x1b]4;1;rgb:1111/1111/1111;2;rgb:2222/2222/2222\x1b\\");
        assert_eq!(rgb(p.ansi[1]), (0x11, 0x11, 0x11));
        assert_eq!(rgb(p.ansi[2]), (0x22, 0x22, 0x22));
    }

    /// The real shape of a probe buffer: color replies, then the DA1 fence.
    #[test]
    fn parses_a_reply_stream_ending_in_the_da1_fence() {
        let mut stream = Vec::new();
        stream.extend_from_slice(b"\x1b]10;rgb:c5c5/c8c8/c6c6\x1b\\");
        stream.extend_from_slice(b"\x1b]11;rgb:1d1d/1f1f/2121\x1b\\");
        stream.extend_from_slice(b"\x1b[?62;4;22c");
        let p = TerminalPalette::parse(&stream);
        assert_eq!(rgb(p.foreground), (0xc5, 0xc8, 0xc6));
        assert_eq!(rgb(p.background), (0x1d, 0x1f, 0x21));
        // The same buffer still parses as Device Attributes — one round-trip
        // answers both questions.
        let da = crate::term::capabilities::DeviceAttributes::parse(&stream).expect("DA1 parses");
        assert!(da.sixel());
    }

    #[test]
    fn ignores_junk_malformed_and_out_of_range_replies() {
        let p = TerminalPalette::parse(
            b"noise\x1b]11;rgb:zz/00/00\x1b\\\x1b]4;99;rgb:1111/1111/1111\x1b\\\
              \x1b]999;whatever\x1b\\\x1b]10;rgb:0000/1111/2222\x1b\\",
        );
        assert_eq!(p.background, None, "a bad spec is skipped, not fatal");
        assert_eq!(
            rgb(p.foreground),
            (0, 0x11, 0x22),
            "later replies still land"
        );
    }

    /// A truncated stream is the realistic failure of a probe that timed out
    /// mid-reply. It must not panic or read past the end.
    #[test]
    fn truncated_streams_never_panic() {
        let full = b"\x1b]10;rgb:1111/2222/3333\x1b\\\x1b]4;3;rgb:aaaa/bbbb/cccc\x07\x1b[?62c";
        for end in 0..=full.len() {
            let _ = TerminalPalette::parse(&full[..end]);
        }
        // A bare ESC mid-OSC must not swallow the sequence after it.
        let _ = TerminalPalette::parse(b"\x1b]11;rgb:11\x1b]10;rgb:2222/2222/2222\x1b\\");
    }

    #[test]
    fn query_sequence_asks_for_everything_and_ends_with_the_fence() {
        let q = query_sequence();
        assert!(q.starts_with(QUERY_FOREGROUND));
        assert!(q.contains(QUERY_BACKGROUND));
        for i in 0..16 {
            assert!(
                q.contains(&format!("\x1b]4;{i};?")),
                "missing OSC 4 for {i}"
            );
        }
        assert!(
            q.ends_with(crate::term::capabilities::DA1_REQUEST),
            "the fence must be last, or it cannot terminate the read"
        );
    }

    /// A palette built from a full reply stream, so this exercises the real
    /// entry point rather than a hand-built struct.
    fn gruvbox_replies() -> TerminalPalette {
        // Gruvbox dark's background/foreground and its bright ANSI half.
        let mut s =
            String::from("\x1b]11;rgb:2828/2828/2828\x1b\\\x1b]10;rgb:ebeb/dbdb/b2b2\x1b\\");
        let bright: [(usize, &str); 8] = [
            (8, "9292/8383/7474"),
            (9, "fbfb/4949/3434"),
            (10, "b8b8/bbbb/2626"),
            (11, "fafa/bdbd/2f2f"),
            (12, "8383/a5a5/9898"),
            (13, "d3d3/8686/9b9b"),
            (14, "8e8e/c0c0/7c7c"),
            (15, "ebeb/dbdb/b2b2"),
        ];
        for (i, spec) in bright {
            s.push_str(&format!("\x1b]4;{i};rgb:{spec}\x1b\\"));
        }
        s.push_str("\x1b[?62;4;22c");
        TerminalPalette::parse(s.as_bytes())
    }

    #[test]
    fn derives_a_theme_that_keeps_the_terminal_fg_and_bg_verbatim() {
        let theme = Theme::from_terminal(&gruvbox_replies()).expect("fg and bg were reported");
        assert_eq!(theme.background, Color::Rgb(0x28, 0x28, 0x28));
        assert_eq!(theme.text, Color::Rgb(0xeb, 0xdb, 0xb2));
        assert_eq!(theme.code.text, theme.text);
        // Blue leads: bright blue (index 12) is the accent, bright aqua the alt.
        assert_eq!(theme.accent, Color::Rgb(0x83, 0xa5, 0x98));
        assert_eq!(theme.accent_alt, Color::Rgb(0x8e, 0xc0, 0x7c));
        assert_eq!(theme.border_focused, theme.accent);
        // Syntax roles track the reported palette, not a hard-coded one.
        assert_eq!(theme.code.string, Color::Rgb(0xb8, 0xbb, 0x26)); // bright green
        assert_eq!(theme.code.keyword, Color::Rgb(0xd3, 0x86, 0x9b)); // bright purple
    }

    #[test]
    fn requires_both_a_foreground_and_a_background() {
        assert!(Theme::from_terminal(&TerminalPalette::default()).is_none());
        let only_bg = TerminalPalette::parse(b"\x1b]11;rgb:0000/0000/0000\x1b\\");
        assert!(Theme::from_terminal(&only_bg).is_none());
        // An indexed color carries no value tuika can blend, so it is not enough.
        let indexed = TerminalPalette {
            foreground: Some(Color::Indexed(7)),
            background: Some(Color::Rgb(0, 0, 0)),
            ..Default::default()
        };
        assert!(Theme::from_terminal(&indexed).is_none());
    }

    #[test]
    fn fills_unreported_ansi_slots_from_the_xterm_defaults() {
        // OSC 10 and 11 only — the common case for a terminal without OSC 4.
        let p = TerminalPalette::parse(
            b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
        );
        let theme = Theme::from_terminal(&p).expect("a complete theme");
        // Dark background → the bright half; xterm's bright blue is #5c5cff,
        // which is too dark on black and gets lifted by the contrast guard.
        let Color::Rgb(r, g, b) = theme.accent else {
            panic!("accent is 24-bit")
        };
        assert!(b > r && b > g, "accent is still recognizably blue");
        assert!(
            contrast_ratio((r, g, b), (0, 0, 0)) >= 3.0,
            "the guard lifted it clear of the background"
        );
    }

    #[test]
    fn light_and_dark_backgrounds_both_derive_readable_tones() {
        for (name, bg_spec, fg_spec) in [
            ("dark", "1d1d/1f1f/2121", "c5c5/c8c8/c6c6"),
            ("light", "fdfd/f6f6/e3e3", "6565/7b7b/8383"),
        ] {
            let p = TerminalPalette::parse(
                format!("\x1b]11;rgb:{bg_spec}\x1b\\\x1b]10;rgb:{fg_spec}\x1b\\").as_bytes(),
            );
            let theme = Theme::from_terminal(&p).expect("a complete theme");
            let bg = rgb_of(theme.background).unwrap();
            // Every glyph-bearing derived role clears its floor on both.
            for (role, color, min) in [
                ("muted", theme.muted, 3.0),
                ("accent", theme.accent, 3.0),
                ("keyword", theme.code.keyword, 3.0),
                ("string", theme.code.string, 3.0),
                ("comment", theme.code.comment, 2.2),
            ] {
                let c = rgb_of(color).unwrap();
                assert!(
                    contrast_ratio(c, bg) >= min - 0.01,
                    "{name}: {role} at {:.2} is under its {min} floor",
                    contrast_ratio(c, bg)
                );
            }
            // `surface` is a raised panel, so it must differ from the base fill
            // without becoming a second background.
            assert_ne!(
                theme.surface, theme.background,
                "{name}: surface is distinct"
            );
            let surface = rgb_of(theme.surface).unwrap();
            assert!(
                contrast_ratio(surface, bg) < 1.5,
                "{name}: surface stays a subtle raise"
            );
            // Selected text must be readable on the band it sits on.
            let sel_bg = rgb_of(theme.selection_bg).unwrap();
            let sel_fg = rgb_of(theme.selection_fg).unwrap();
            assert!(
                contrast_ratio(sel_fg, sel_bg) >= 4.4,
                "{name}: selection is legible"
            );
        }
    }

    /// The degenerate palette: a terminal reporting the same color for both.
    /// Nothing can be derived well, but nothing may panic or vanish either.
    #[test]
    fn survives_a_foreground_equal_to_the_background() {
        let p = TerminalPalette::parse(
            b"\x1b]11;rgb:0000/0000/0000\x1b\\\x1b]10;rgb:0000/0000/0000\x1b\\",
        );
        let theme = Theme::from_terminal(&p).expect("still derives");
        assert_eq!(theme.text, theme.background, "the reported fg is respected");
        let bg = rgb_of(theme.background).unwrap();
        assert!(
            contrast_ratio(rgb_of(theme.muted).unwrap(), bg) >= 3.0,
            "derived roles are still lifted clear"
        );
    }

    #[test]
    fn scale_hex_widens_the_way_x11_does() {
        assert_eq!(scale_hex("f"), Some(255));
        assert_eq!(scale_hex("0"), Some(0));
        assert_eq!(scale_hex("ffff"), Some(255));
        assert_eq!(scale_hex("8000"), Some(128));
        assert_eq!(scale_hex(""), None);
        assert_eq!(scale_hex("fffff"), None);
        assert_eq!(scale_hex("zz"), None);
    }

    #[test]
    fn contrast_ratio_matches_the_wcag_extremes() {
        assert!((contrast_ratio(WHITE, BLACK) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(WHITE, WHITE) - 1.0).abs() < 0.01);
        assert!(is_dark(BLACK) && !is_dark(WHITE));
    }
}
