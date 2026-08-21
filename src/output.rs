//! Render a view once as ordinary terminal output.

use std::io::{self, Write};

use crate::style::{Color, Style};
use crate::text::{Line, Span};

use crate::testing;
use crate::view::{AvailableSpace, MeasureRequest, RenderCtx, View};
use crate::{Size, Theme};

/// Sizing and whitespace policy for one-shot output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OneShotOptions {
    /// Maximum output width in terminal cells.
    pub width: u16,
    /// Maximum output height in rows.
    pub max_height: u16,
    /// Remove trailing space cells at the end of each row.
    pub trim_end: bool,
    /// Write a final newline after the last row.
    pub trailing_newline: bool,
}

impl Default for OneShotOptions {
    fn default() -> Self {
        Self {
            width: 80,
            max_height: 4096,
            trim_end: true,
            trailing_newline: true,
        }
    }
}

/// Render `view` once to an ANSI-styled UTF-8 string.
///
/// Unlike [`Runner`](crate::Runner), this does not enter raw mode, hide the
/// cursor, capture input, or own a screen. It is suitable for command output,
/// completed status summaries, and static reports that should remain in
/// scrollback.
///
/// ```
/// use tuika::prelude::*;
///
/// let output = render_once(
///     &Text::raw("complete"),
///     &Theme::default(),
///     OneShotOptions::default(),
/// )?;
/// assert!(output.contains("complete"));
/// # Ok::<(), std::io::Error>(())
/// ```
pub fn render_once(view: &dyn View, theme: &Theme, options: OneShotOptions) -> io::Result<String> {
    let mut bytes = Vec::new();
    write_once(&mut bytes, view, theme, options)?;
    Ok(String::from_utf8(bytes).expect("terminal output is UTF-8"))
}

/// Render `view` once and write ANSI-styled output to `out`.
pub fn write_once(
    out: &mut impl Write,
    view: &dyn View,
    theme: &Theme,
    options: OneShotOptions,
) -> io::Result<()> {
    if options.width == 0 || options.max_height == 0 {
        return Ok(());
    }
    let request = MeasureRequest::new(Size::new(options.width, options.max_height))
        .with_available_width(AvailableSpace::Definite(options.width))
        .with_available_height(AvailableSpace::Definite(options.max_height));
    let measured = view.measure_request(request, &RenderCtx::new(theme));
    let size = Size::new(
        measured.width.min(options.width),
        measured.height.min(options.max_height),
    );
    if size.width == 0 || size.height == 0 {
        return Ok(());
    }
    let buffer = testing::render(view, size.width, size.height, theme);
    for row in 0..size.height {
        let mut end = size.width;
        if options.trim_end {
            while end > 0 && buffer[(end - 1, row)].symbol() == " " {
                end -= 1;
            }
        }
        let mut spans = Vec::new();
        let mut text = String::new();
        let mut current = None;
        for column in 0..end {
            let cell = &buffer[(column, row)];
            let style = cell_style(cell.fg, cell.bg, cell.modifier);
            if current.is_some_and(|active| active != style) {
                spans.push(Span::styled(std::mem::take(&mut text), current.unwrap()));
            }
            current = Some(style);
            // A view controls cell symbols; ordinary output must not turn an
            // embedded C0/C1 byte into a second terminal protocol stream.
            text.extend(
                cell.symbol()
                    .chars()
                    .filter(|character| !character.is_control()),
            );
        }
        if let Some(style) = current {
            spans.push(Span::styled(text, style));
        }
        crate::term::hyperlink::write_line(out, &Line::from(spans))?;
        if row + 1 < size.height || options.trailing_newline {
            out.write_all(b"\n")?;
        }
    }
    out.flush()
}

fn cell_style(foreground: Color, background: Color, modifier: crate::style::Modifier) -> Style {
    Style::default()
        .fg(foreground)
        .bg(background)
        .add_modifier(modifier)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::style::{Color, Modifier};

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("sink closed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("sink closed"))
        }
    }

    #[test]
    fn one_shot_output_is_sized_to_content_and_keeps_newline() {
        let output = render_once(
            &Text::raw("hello"),
            &Theme::default(),
            OneShotOptions::default(),
        )
        .unwrap();
        assert!(output.contains("hello"));
        assert!(output.ends_with('\n'));
        assert!(!output.contains(
            "hello                                                                           "
        ));
    }

    #[test]
    fn one_shot_output_drops_embedded_control_characters() {
        let output = render_once(
            &Text::raw("safe\u{1b}[31munsafe"),
            &Theme::default(),
            OneShotOptions::default(),
        )
        .unwrap();
        assert!(!output.contains("\u{1b}[31munsafe"));
    }

    #[test]
    fn one_shot_output_preserves_cell_sgr_without_owning_terminal_state() {
        let style = Style::default()
            .fg(Color::Indexed(201))
            .add_modifier(Modifier::BOLD);
        let view = crate::view::DrawView::new(
            move |area: crate::geometry::Rect, surface: &mut crate::Surface, _: &RenderCtx| {
                surface.set_string(area.x, area.y, "styled", style);
            },
        )
        .intrinsic_size(Size::new(6, 1));
        let output = render_once(&view, &Theme::default(), OneShotOptions::default()).unwrap();
        // Crossterm intentionally suppresses color commands under NO_COLOR;
        // attributes remain deterministic and still prove SGR serialization.
        assert!(output.contains("\x1b[1m"), "{output:?}");
        assert!(
            output.contains("\x1b[0m"),
            "style must be reset: {output:?}"
        );
        assert!(!output.contains("\x1b[?1049h"));
        assert!(!output.contains("\x1b[?25l"));
    }

    #[test]
    fn one_shot_output_propagates_writer_errors() {
        let error = write_once(
            &mut FailingWriter,
            &Text::raw("hello"),
            &Theme::default(),
            OneShotOptions::default(),
        )
        .expect_err("writer failure must reach the caller");
        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(error.to_string(), "sink closed");
    }
}
