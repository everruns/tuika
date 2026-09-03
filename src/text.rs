//! Styled text: [`Span`], [`Line`], and [`Alignment`].
//!
//! A [`Span`] is a run of text with one [`Style`]; a [`Line`] is a row of spans
//! with an optional line-level style and alignment. Components take `Line` in
//! their constructors because it is the smallest unit that can carry mixed
//! styling — a markdown paragraph with a bold word in it is one `Line` of three
//! spans, not three separate strings the caller has to place.
//!
//! Both borrow: `Span<'a>` holds a [`Cow<'a, str>`], so building a line from
//! `&str` allocates nothing while `Line<'static>` can own its content when a
//! component needs to keep it across frames.
//!
//! Widths here are counted in tuika's grapheme-aware display columns
//! ([`width::str_cols`](crate::width::str_cols)), not per `char` — the whole
//! toolkit measures the same way, so a line's reported width matches what
//! [`Surface`](crate::Surface) will actually paint.

use std::borrow::Cow;

use crate::style::Style;
use crate::width::str_cols;

/// Horizontal placement of a line within the area it is drawn into.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// Against the left edge.
    #[default]
    Left,
    /// Centered, with any odd cell going to the right.
    Center,
    /// Against the right edge.
    Right,
}

/// A run of text sharing one style.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Span<'a> {
    /// The text itself.
    pub content: Cow<'a, str>,
    /// How to paint it. Patched over the owning [`Line`]'s style at render.
    pub style: Style,
}

impl<'a> Span<'a> {
    /// An unstyled span.
    pub fn raw<T: Into<Cow<'a, str>>>(content: T) -> Self {
        Self {
            content: content.into(),
            style: Style::default(),
        }
    }

    /// A span with `style` applied.
    pub fn styled<T: Into<Cow<'a, str>>, S: Into<Style>>(content: T, style: S) -> Self {
        Self {
            content: content.into(),
            style: style.into(),
        }
    }

    /// Replace the style, returning the span.
    #[must_use = "returns a new Span"]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// Layer `style` on top of the current one.
    #[must_use = "returns a new Span"]
    pub fn patch_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = self.style.patch(style);
        self
    }

    /// Display width in cells.
    pub fn width(&self) -> u16 {
        str_cols(self.content.as_ref())
    }

    /// Detach from the borrowed source, cloning the content if needed.
    pub fn into_owned(self) -> Span<'static> {
        Span {
            content: Cow::Owned(self.content.into_owned()),
            style: self.style,
        }
    }
}

impl<'a, T> From<T> for Span<'a>
where
    T: Into<Cow<'a, str>>,
{
    fn from(content: T) -> Self {
        Self::raw(content)
    }
}

impl std::fmt::Display for Span<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.content.as_ref())
    }
}

/// Split content into one span per line, dropping the breaks.
///
/// Borrows through when there is exactly one line, so the common case still
/// allocates nothing.
fn split_lines(content: Cow<'_, str>) -> Vec<Span<'_>> {
    match content {
        Cow::Borrowed(text) => text.lines().map(Span::raw).collect(),
        Cow::Owned(text) => {
            let mut lines = text.lines();
            match (lines.next(), lines.next()) {
                (None, _) => Vec::new(),
                (Some(only), None) if only.len() == text.len() => {
                    vec![Span::raw(Cow::Owned(text))]
                }
                _ => text
                    .lines()
                    .map(|line| Span::raw(Cow::Owned(line.to_owned())))
                    .collect(),
            }
        }
    }
}

/// A row of styled spans.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line<'a> {
    /// The spans, left to right.
    pub spans: Vec<Span<'a>>,
    /// A style applied beneath every span's own style.
    pub style: Style,
    /// Where to place the line horizontally. `None` lets the drawing component
    /// decide — which is what makes a `Line` reusable across a left-aligned
    /// list and a centered dialog without rebuilding it.
    pub alignment: Option<Alignment>,
}

impl<'a> Line<'a> {
    /// An unstyled line.
    ///
    /// Content is split on line breaks into one span per line, and the breaks
    /// themselves are dropped — a `Line` is a single row, so an embedded
    /// newline cannot be honored and silently keeping it would put a control
    /// character into a cell. Empty input therefore yields *no* spans.
    pub fn raw<T: Into<Cow<'a, str>>>(content: T) -> Self {
        Self {
            spans: split_lines(content.into()),
            style: Style::default(),
            alignment: None,
        }
    }

    /// Like [`raw`](Self::raw), with `style` applied at the line level.
    pub fn styled<T: Into<Cow<'a, str>>, S: Into<Style>>(content: T, style: S) -> Self {
        Self {
            spans: split_lines(content.into()),
            style: style.into(),
            alignment: None,
        }
    }

    /// Replace the line-level style.
    #[must_use = "returns a new Line"]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// Layer `style` on top of the line-level style.
    #[must_use = "returns a new Line"]
    pub fn patch_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = self.style.patch(style);
        self
    }

    /// Set the alignment.
    #[must_use = "returns a new Line"]
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }

    /// Align left.
    #[must_use = "returns a new Line"]
    pub fn left_aligned(self) -> Self {
        self.alignment(Alignment::Left)
    }

    /// Center.
    #[must_use = "returns a new Line"]
    pub fn centered(self) -> Self {
        self.alignment(Alignment::Center)
    }

    /// Align right.
    #[must_use = "returns a new Line"]
    pub fn right_aligned(self) -> Self {
        self.alignment(Alignment::Right)
    }

    /// Append a span.
    pub fn push_span<S: Into<Span<'a>>>(&mut self, span: S) {
        self.spans.push(span.into());
    }

    /// Total display width in cells, saturating.
    pub fn width(&self) -> u16 {
        self.spans
            .iter()
            .map(Span::width)
            .fold(0, u16::saturating_add)
    }

    /// The spans, left to right.
    pub fn iter(&self) -> std::slice::Iter<'_, Span<'a>> {
        self.spans.iter()
    }

    /// Detach from the borrowed source, cloning content as needed.
    pub fn into_owned(self) -> Line<'static> {
        Line {
            spans: self.spans.into_iter().map(Span::into_owned).collect(),
            style: self.style,
            alignment: self.alignment,
        }
    }
}

impl<'a> From<Span<'a>> for Line<'a> {
    fn from(span: Span<'a>) -> Self {
        Self {
            spans: vec![span],
            style: Style::default(),
            alignment: None,
        }
    }
}

impl<'a> From<Vec<Span<'a>>> for Line<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self {
        Self {
            spans,
            style: Style::default(),
            alignment: None,
        }
    }
}

impl From<String> for Line<'_> {
    fn from(content: String) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<&'a str> for Line<'a> {
    fn from(content: &'a str) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<Cow<'a, str>> for Line<'a> {
    fn from(content: Cow<'a, str>) -> Self {
        Self::raw(content)
    }
}

impl<'a> FromIterator<Span<'a>> for Line<'a> {
    fn from_iter<I: IntoIterator<Item = Span<'a>>>(iter: I) -> Self {
        Self::from(iter.into_iter().collect::<Vec<_>>())
    }
}

impl<'a> IntoIterator for Line<'a> {
    type Item = Span<'a>;
    type IntoIter = std::vec::IntoIter<Span<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.spans.into_iter()
    }
}

impl<'a, 'b> IntoIterator for &'b Line<'a> {
    type Item = &'b Span<'a>;
    type IntoIter = std::slice::Iter<'b, Span<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.spans.iter()
    }
}

impl std::fmt::Display for Line<'_> {
    /// The visible text, spans concatenated and styling dropped. This is what
    /// makes `line.to_string()` usable for assertions and for the plain-text
    /// [`render_once`](crate::render_once) path.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for span in &self.spans {
            f.write_str(span.content.as_ref())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Modifier};

    #[test]
    fn a_str_becomes_a_one_span_line() {
        let line = Line::from("hello");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.to_string(), "hello");
        assert_eq!(line.alignment, None);
    }

    /// A `Line` is one row, so embedded breaks become separate spans rather
    /// than control characters in a cell — and empty content is no spans at all.
    #[test]
    fn raw_splits_on_line_breaks() {
        assert!(Line::raw("").spans.is_empty());
        assert!(Line::raw(String::new()).spans.is_empty());
        assert_eq!(Line::raw("one").spans.len(), 1);

        let split = Line::raw("a\nb");
        assert_eq!(split.spans.len(), 2);
        assert_eq!(split.to_string(), "ab");
        assert_eq!(Line::raw(String::from("a\nb")).spans.len(), 2);
    }

    #[test]
    fn display_concatenates_spans_and_drops_styling() {
        let line = Line::from(vec![
            Span::styled("bo", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw("ld"),
        ]);
        assert_eq!(line.to_string(), "bold");
    }

    /// Width is grapheme-aware, not per-`char`: a ZWJ family is one cluster two
    /// columns wide, and counting its `char`s would report far more.
    #[test]
    fn width_counts_display_columns_not_chars() {
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert_eq!(Span::raw(family).width(), 2);
        assert_eq!(Span::raw("abc").width(), 3);
        assert_eq!(Line::from(family).width(), 2);
    }

    #[test]
    fn line_width_sums_its_spans() {
        let line = Line::from(vec![Span::raw("ab"), Span::raw("cde")]);
        assert_eq!(line.width(), 5);
    }

    #[test]
    fn width_saturates_rather_than_overflowing() {
        let wide = "x".repeat(u16::MAX as usize);
        let line = Line::from(vec![Span::raw(wide.clone()), Span::raw(wide)]);
        assert_eq!(line.width(), u16::MAX);
    }

    #[test]
    fn alignment_helpers_set_the_field() {
        assert_eq!(Line::raw("x").centered().alignment, Some(Alignment::Center));
        assert_eq!(
            Line::raw("x").right_aligned().alignment,
            Some(Alignment::Right)
        );
        assert_eq!(
            Line::raw("x").left_aligned().alignment,
            Some(Alignment::Left)
        );
    }

    /// A span's own style layers *over* the line's, which is what lets a themed
    /// line carry a differently-colored word.
    #[test]
    fn span_style_patches_over_line_style() {
        let line =
            Line::styled("x", Style::new().fg(Color::Red)).style(Style::new().fg(Color::Red));
        let span = Span::styled("x", Style::new().fg(Color::Blue));
        assert_eq!(line.style.patch(span.style).fg, Some(Color::Blue));
    }

    #[test]
    fn into_owned_detaches_from_the_borrow() {
        let owned = {
            let text = String::from("borrowed");
            Line::raw(text.as_str()).into_owned()
        };
        assert_eq!(owned.to_string(), "borrowed");
    }

    #[test]
    fn collecting_spans_builds_a_line() {
        let line: Line = vec![Span::raw("a"), Span::raw("b")].into_iter().collect();
        assert_eq!(line.to_string(), "ab");
    }
}
