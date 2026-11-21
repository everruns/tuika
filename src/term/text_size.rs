//! Large text through Kitty's text sizing protocol (OSC 66).
//!
//! A terminal cell is one fixed size, so a heading has only weight and color to
//! stand out with. Kitty's text sizing protocol lifts that limit: text sent as
//!
//! ```text
//! ESC ] 66 ; s=<scale>:w=<cols> ; <text> ST
//! ```
//!
//! is drawn `scale` times larger, covering a block of `scale × cols` columns by
//! `scale` rows. The terminal owns those cells afterwards — it scrolls, clips,
//! and erases them like ordinary text, which is why this fits tuika's cell grid
//! far better than a picture of the glyphs would.
//!
//! # How it meets the cell grid
//!
//! The escape lives in the buffer, the way an OSC 8 hyperlink does, so the diff,
//! the split-footer scrollback, and one-shot output all carry it without a
//! separate emission step:
//!
//! - each grapheme's origin cell holds its OSC 66 sequence and is marked
//!   [`ForcedWidth`](CellDiffOption::ForcedWidth) of `scale × cols`, so the diff
//!   steps over the columns the large glyph covers;
//! - every other cell of the block is [`Skip`](CellDiffOption::Skip), because a
//!   write to any cell of a multicell glyph makes the terminal erase it.
//!
//! [`apply_sized_runs_in`] does that rewrite for text already painted at normal
//! size, which is also the fallback: when the block does not fit, the run is left
//! exactly as painted. [`LargeText`](crate::components::LargeText) and the
//! markdown heading option are built on it.
//!
//! # Support
//!
//! A terminal that does not implement OSC 66 swallows the whole sequence, text
//! included, so unlike OSC 8 this cannot be emitted blindly: the heading would
//! simply vanish. Rendering is therefore gated on [`TextSizing`], which
//! [`Runner`](crate::Runner) detects and installs on the render context. Only
//! kitty (0.40 and later) implements the protocol today; inside tmux or screen
//! the multiplexer, not kitty, parses the stream, so detection says no there.

use std::num::NonZeroU16;

use crate::buffer::{Buffer, CellDiffOption};
use crate::geometry::Rect;
use crate::style::Style;
use crate::width::grapheme_cols;

/// String terminator for an OSC sequence: `ESC \`.
const ST: &str = "\x1b\\";

/// The largest scale the protocol accepts.
pub const MAX_SCALE: u8 = 7;

/// Whether the terminal draws OSC 66 sized text.
///
/// A plain `bool` would do, but a named type keeps the call sites readable and
/// leaves room for the protocol's other knobs (fractional scale, alignment)
/// without another signature change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextSizing {
    /// The terminal is not known to implement OSC 66; render at normal size.
    #[default]
    Unsupported,
    /// The terminal implements OSC 66 sized text.
    Supported,
}

impl TextSizing {
    /// Detect support from the process environment — instant, no terminal I/O.
    pub fn detect() -> Self {
        let env = |k: &str| std::env::var(k).ok();
        Self::detect_from(
            env("TERM").as_deref(),
            env("KITTY_WINDOW_ID").as_deref(),
            env("TMUX").is_some() || env("STY").is_some(),
        )
    }

    /// The pure core of [`detect`](Self::detect), taking the environment
    /// explicitly so the heuristic is unit-testable.
    ///
    /// Conservative: kitty only, and never under a multiplexer. A false
    /// positive costs the user the text itself, so an unknown terminal is
    /// treated as unsupported.
    pub fn detect_from(
        term: Option<&str>,
        kitty_window_id: Option<&str>,
        multiplexed: bool,
    ) -> Self {
        let kitty = kitty_window_id.is_some() || term == Some("xterm-kitty");
        if kitty && !multiplexed {
            Self::Supported
        } else {
            Self::Unsupported
        }
    }

    /// Whether sized text should be emitted.
    pub fn is_supported(self) -> bool {
        self == Self::Supported
    }
}

/// Encode one grapheme cluster as an OSC 66 sequence drawn at `scale`.
///
/// `cols` is the cluster's normal width in cells, sent explicitly as `w` so the
/// terminal's width table cannot disagree with tuika's about how many columns
/// the block covers. `scale` is clamped to `1..=`[`MAX_SCALE`], `cols` to the
/// protocol's `1..=7`.
///
/// Control characters are stripped from `text` — the same breakout defense as
/// the OSC 8 target sanitizer — so caller text can never terminate the sequence
/// early and start one of its own.
pub fn encode(text: &str, scale: u8, cols: u16) -> String {
    let scale = scale.clamp(1, MAX_SCALE);
    let cols = cols.clamp(1, 7);
    let mut out = String::with_capacity(text.len() + 16);
    out.push_str("\x1b]66;s=");
    out.push(char::from(b'0' + scale));
    out.push_str(":w=");
    out.push(char::from(b'0' + cols as u8));
    out.push(';');
    out.extend(text.chars().filter(|c| !c.is_control()));
    out.push_str(ST);
    out
}

/// A run of text already painted at normal size that should be redrawn larger.
///
/// Positions are relative to the area handed to [`apply_sized_runs_in`], the
/// way a [`BufferLink`](crate::term::hyperlink::BufferLink) is: the run is row
/// `line`, columns `start_col..end_col`. Scaled, it covers rows
/// `line..line + scale` and columns `start_col..start_col + scale × width`, so
/// whoever lays out the text reserves that block (blank) beforehand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SizedRun {
    /// Row of the run, relative to the area's top.
    pub line: u16,
    /// First column of the run, relative to the area's left edge.
    pub start_col: u16,
    /// Exclusive end column of the run at normal size.
    pub end_col: u16,
    /// How many times larger to draw it, `2..=`[`MAX_SCALE`].
    pub scale: u8,
}

impl SizedRun {
    /// The cell block this run covers once scaled, relative to its area.
    pub fn scaled_size(&self) -> (u16, u16) {
        let scale = u16::from(self.scale.clamp(1, MAX_SCALE));
        let width = self.end_col.saturating_sub(self.start_col);
        (width.saturating_mul(scale), scale)
    }
}

/// Redraw each [`SizedRun`] inside `area` at its scale.
///
/// The text and its styles are read back from the cells the run already
/// occupies, so inline styling (an inline-code span inside a heading) survives
/// scaling, and a run that cannot be scaled is simply left as painted. A run is
/// left alone when its scaled block would cross `area` or the buffer, or when
/// `scale` is below 2.
///
/// Call this after painting and *before* applying hyperlinks: a sized run reads
/// plain cells, and an OSC 8 marker in one would be carried into the escape.
pub fn apply_sized_runs_in(buf: &mut Buffer, area: Rect, runs: &[SizedRun]) {
    let clip = area.intersection(buf.area);
    for run in runs {
        if run.scale < 2 || run.end_col <= run.start_col {
            continue;
        }
        let scale = run.scale.min(MAX_SCALE);
        let (width, height) = run.scaled_size();
        let x = area.x.saturating_add(run.start_col);
        let y = area.y.saturating_add(run.line);
        let block = Rect::new(x, y, width, height);
        // Only a block that lies wholly inside the clip is drawn; a partial one
        // would either be erased by the cells around it or spill past them.
        if block.is_empty()
            || x < clip.left()
            || y < clip.top()
            || u32::from(x) + u32::from(width) > u32::from(clip.right())
            || u32::from(y) + u32::from(height) > u32::from(clip.bottom())
        {
            continue;
        }
        stamp(buf, x, y, run.end_col - run.start_col, scale);
    }
}

/// Rewrite the `cols` normal-size cells at (`x`, `y`) as one sized block. The
/// caller has checked the block fits.
fn stamp(buf: &mut Buffer, x: u16, y: u16, cols: u16, scale: u8) {
    // Read the clusters first: the block overwrites the cells they live in.
    let mut clusters: Vec<(u16, String, Style)> = Vec::new();
    let mut col = 0;
    while col < cols {
        let cell = &buf[(x + col, y)];
        let symbol = cell.symbol();
        let w = grapheme_cols(symbol).max(1);
        // A cell that already carries an escape (a sized glyph, a link marker)
        // is not plain text; scaling it again would nest sequences.
        if symbol.contains('\x1b') {
            return;
        }
        clusters.push((col, symbol.to_string(), cell.style()));
        col = col.saturating_add(w);
    }

    let factor = u16::from(scale);
    for (offset, symbol, style) in clusters {
        let w = grapheme_cols(&symbol).max(1);
        let origin_x = x + offset * factor;
        let block_width = w * factor;
        for dy in 0..factor {
            for dx in 0..block_width {
                let cell = &mut buf[(origin_x + dx, y + dy)];
                cell.reset();
                cell.set_style(style);
                cell.set_diff_option(CellDiffOption::Skip);
            }
        }
        if symbol.trim().is_empty() {
            // A space needs no escape: the block is blank either way, and a
            // plain cell keeps the diff simple. Mark the rest as ordinary too.
            for dy in 0..factor {
                for dx in 0..block_width {
                    buf[(origin_x + dx, y + dy)].set_diff_option(CellDiffOption::None);
                }
            }
            continue;
        }
        let origin = &mut buf[(origin_x, y)];
        origin.set_symbol(&encode(&symbol, scale, w));
        origin.set_diff_option(CellDiffOption::ForcedWidth(
            NonZeroU16::new(block_width).expect("block width is at least 2"),
        ));
    }
}

/// The visible text of a cell symbol with any OSC 66 wrapper removed.
///
/// Snapshot and selection code reads cell symbols; this recovers the glyph a
/// sized cell draws.
pub fn visible_text(symbol: &str) -> &str {
    let Some(rest) = symbol.strip_prefix("\x1b]66;") else {
        return symbol;
    };
    let Some((_, body)) = rest.split_once(';') else {
        return symbol;
    };
    body.strip_suffix(ST).unwrap_or(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Modifier};

    #[test]
    fn encode_writes_scale_and_width() {
        assert_eq!(encode("H", 2, 1), "\x1b]66;s=2:w=1;H\x1b\\");
        assert_eq!(encode("中", 3, 2), "\x1b]66;s=3:w=2;中\x1b\\");
    }

    #[test]
    fn encode_clamps_parameters() {
        assert_eq!(encode("a", 0, 0), "\x1b]66;s=1:w=1;a\x1b\\");
        assert_eq!(encode("a", 99, 99), "\x1b]66;s=7:w=7;a\x1b\\");
    }

    #[test]
    fn encode_strips_control_characters() {
        let encoded = encode("a\x1b\\\x07\x1b]0;pwn\x07b", 2, 1);
        assert_eq!(encoded, "\x1b]66;s=2:w=1;a\\]0;pwnb\x1b\\");
        // Exactly one ESC opens and one closes: nothing inside can terminate it.
        assert_eq!(encoded.matches('\x1b').count(), 2);
    }

    #[test]
    fn detection_is_kitty_only_and_never_multiplexed() {
        assert!(TextSizing::detect_from(Some("xterm-kitty"), None, false).is_supported());
        assert!(TextSizing::detect_from(Some("xterm-256color"), Some("1"), false).is_supported());
        assert!(!TextSizing::detect_from(Some("xterm-kitty"), Some("1"), true).is_supported());
        assert!(!TextSizing::detect_from(Some("xterm-ghostty"), None, false).is_supported());
        assert!(!TextSizing::detect_from(None, None, false).is_supported());
    }

    fn apply(buf: &mut Buffer, runs: &[SizedRun]) {
        let area = buf.area;
        apply_sized_runs_in(buf, area, runs);
    }

    fn painted(text: &str, width: u16, height: u16, style: Style) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        buf.set_string(0, 0, text, style);
        buf
    }

    #[test]
    fn a_run_becomes_sized_cells_covering_its_block() {
        let style = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let mut buf = painted("Hi", 6, 3, style);
        let run = SizedRun {
            line: 0,
            start_col: 0,
            end_col: 2,
            scale: 2,
        };
        apply(&mut buf, &[run]);

        assert_eq!(buf[(0, 0)].symbol(), "\x1b]66;s=2:w=1;H\x1b\\");
        assert_eq!(buf[(2, 0)].symbol(), "\x1b]66;s=2:w=1;i\x1b\\");
        for (x, y) in [(0, 0), (2, 0)] {
            assert_eq!(
                buf[(x, y)].diff_option,
                CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap())
            );
            assert_eq!(buf[(x, y)].fg, Color::Red);
            assert!(buf[(x, y)].modifier.contains(Modifier::BOLD));
        }
        for (x, y) in [(1, 0), (0, 1), (1, 1), (3, 0), (2, 1), (3, 1)] {
            assert_eq!(buf[(x, y)].diff_option, CellDiffOption::Skip, "({x}, {y})");
        }
        // Outside the block: untouched.
        assert_eq!(buf[(4, 0)].diff_option, CellDiffOption::None);
        assert_eq!(buf[(0, 2)].diff_option, CellDiffOption::None);
        assert_eq!(visible_text(buf[(2, 0)].symbol()), "i");
    }

    #[test]
    fn a_run_that_does_not_fit_is_left_as_painted() {
        let mut buf = painted("Hello", 8, 3, Style::default());
        let before = buf.clone();
        // 10 columns wide scaled: wider than the 8-column buffer.
        let run = SizedRun {
            line: 0,
            start_col: 0,
            end_col: 5,
            scale: 2,
        };
        apply(&mut buf, &[run]);
        assert_eq!(buf, before);
        // Too short as well.
        let mut buf = painted("Hi", 8, 1, Style::default());
        let before = buf.clone();
        apply(&mut buf, &[SizedRun { end_col: 2, ..run }]);
        assert_eq!(buf, before);
    }

    #[test]
    fn wide_clusters_keep_their_width() {
        let mut buf = painted("中a", 6, 2, Style::default());
        apply(
            &mut buf,
            &[SizedRun {
                line: 0,
                start_col: 0,
                end_col: 3,
                scale: 2,
            }],
        );
        assert_eq!(buf[(0, 0)].symbol(), "\x1b]66;s=2:w=2;中\x1b\\");
        assert_eq!(buf[(0, 0)].cell_width(), 4);
        assert_eq!(buf[(4, 0)].symbol(), "\x1b]66;s=2:w=1;a\x1b\\");
    }

    #[test]
    fn spaces_stay_plain_cells() {
        let mut buf = painted("a b", 6, 2, Style::default());
        apply(
            &mut buf,
            &[SizedRun {
                line: 0,
                start_col: 0,
                end_col: 3,
                scale: 2,
            }],
        );
        assert_eq!(buf[(2, 0)].symbol(), " ");
        assert_eq!(buf[(2, 0)].diff_option, CellDiffOption::None);
        assert_eq!(buf[(3, 1)].diff_option, CellDiffOption::None);
    }

    #[test]
    fn applying_twice_does_not_nest() {
        let mut buf = painted("Hi", 6, 3, Style::default());
        let run = SizedRun {
            line: 0,
            start_col: 0,
            end_col: 2,
            scale: 2,
        };
        apply(&mut buf, &[run]);
        let once = buf.clone();
        apply(&mut buf, &[run]);
        assert_eq!(buf, once);
    }

    #[test]
    fn the_diff_writes_only_origin_cells() {
        let mut buf = painted("Hi", 6, 3, Style::default());
        apply(
            &mut buf,
            &[SizedRun {
                line: 0,
                start_col: 0,
                end_col: 2,
                scale: 2,
            }],
        );
        let previous = Buffer::empty(buf.area);
        let updates: Vec<(u16, u16)> = previous
            .diff(&buf)
            .into_iter()
            .map(|(x, y, _)| (x, y))
            .collect();
        assert_eq!(updates, vec![(0, 0), (2, 0)]);
    }
}
