//! Test-only helpers shared by this module's submodule tests.
//!
//! The submodules each own the tests for their own concern, but they assert
//! against the same rendered output, so the two render helpers and the stub
//! image resolver live here rather than being copied four times.

use crate::text::Line;

use crate::highlight::CodeHighlighter;
use crate::style::{StyleSheet, Theme};
use crate::term::image::ImageData;

use super::image::ImageResolver;
use super::to_lines;

/// Plain text of a rendered line (spans concatenated).
pub(super) fn text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

/// The whole render as plain lines, for content assertions.
pub(super) fn plain(source: &str, width: u16) -> Vec<String> {
    to_lines(
        source,
        width,
        &Theme::default(),
        &StyleSheet::default(),
        CodeHighlighter::Plain,
    )
    .iter()
    .map(text)
    .collect()
}

/// A resolver that returns a fixed image for any URL containing "ok".
pub(super) struct StubResolver;

impl ImageResolver for StubResolver {
    fn resolve(&self, url: &str) -> Option<ImageData> {
        url.contains("ok")
            .then(|| ImageData::from_rgba(4, 2, vec![0u8; 4 * 2 * 4]).unwrap())
    }
}
