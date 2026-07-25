//! Terminal-native Mermaid fenced blocks for
//! [`tuika::components::Markdown`].
//!
//! [`MermaidRenderer`] adapts mmdflux's Unicode text output to tuika's generic
//! [`FencedBlockRenderer`] seam. It
//! handles `mermaid` fences and returns `None` for every other language or for
//! Mermaid input mmdflux cannot render, preserving tuika's ordinary code-block
//! fallback.

use mmdflux::{OutputFormat, RenderConfig, TextColorMode, render_diagram};
use ratatui_core::style::Style;
use ratatui_core::text::{Line, Span};
use tuika::Theme;
use tuika::components::FencedBlockRenderer;

/// Upper bound for one Mermaid fence handed to mmdflux.
const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// Upper bound for mmdflux text retained as terminal cells.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Renders `mermaid` fenced blocks as Unicode terminal diagrams.
///
/// Fences larger than 64 KiB, invalid Mermaid, and unsupported diagram types
/// return `None` so tuika keeps the visible source-code fallback.
#[derive(Clone, Copy, Debug, Default)]
pub struct MermaidRenderer;

impl MermaidRenderer {
    /// A Mermaid fenced-block renderer.
    pub const fn new() -> Self {
        Self
    }
}

impl FencedBlockRenderer for MermaidRenderer {
    fn render(
        &self,
        language: &str,
        source: &str,
        _width: u16,
        theme: &Theme,
    ) -> Option<Vec<Line<'static>>> {
        if !language.eq_ignore_ascii_case("mermaid") {
            return None;
        }
        if source.len() > MAX_SOURCE_BYTES {
            return None;
        }

        let config = RenderConfig {
            text_color_mode: TextColorMode::Plain,
            ..RenderConfig::default()
        };
        let rendered = render_diagram(source, OutputFormat::Text, &config).ok()?;
        if rendered.len() > MAX_OUTPUT_BYTES {
            return None;
        }
        Some(
            rendered
                .lines()
                .map(|line| styled_line(line, theme))
                .collect(),
        )
    }
}

fn styled_line(line: &str, theme: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    let mut run = String::new();
    let mut run_is_structure = None;

    for ch in line.chars() {
        // mmdflux color is disabled above; strip any remaining control bytes
        // from user labels so a diagram can never smuggle terminal escapes into
        // ratatui's cell stream.
        if ch.is_control() {
            continue;
        }
        let is_structure = diagram_glyph(ch);
        if run_is_structure.is_some_and(|current| current != is_structure) {
            push_run(&mut spans, &mut run, run_is_structure.unwrap(), theme);
        }
        run_is_structure = Some(is_structure);
        run.push(ch);
    }
    if let Some(is_structure) = run_is_structure {
        push_run(&mut spans, &mut run, is_structure, theme);
    }

    Line::from(spans)
}

fn push_run(spans: &mut Vec<Span<'static>>, run: &mut String, structure: bool, theme: &Theme) {
    let foreground = if structure { theme.dim } else { theme.text };
    spans.push(Span::styled(
        std::mem::take(run),
        Style::default().fg(foreground),
    ));
}

fn diagram_glyph(ch: char) -> bool {
    matches!(
        ch,
        '\u{2500}'..='\u{259f}' | '▲' | '▶' | '▼' | '◀' | '◆' | '◇' | '●' | '○'
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::components::Markdown;
    use tuika::testing::{grid, render};

    fn text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn renders_mermaid_fence_inside_markdown() {
        let theme = Theme::default();
        let renderer = MermaidRenderer::new();
        let markdown = Markdown::new(
            "before\n\n```mermaid\nflowchart LR\n  A[Parse] --> B[Paint]\n```\n\nafter",
        )
        .block_renderer(&renderer);
        let output = grid(&render(&markdown, 80, 12, &theme));

        assert!(output.contains("Parse"), "{output}");
        assert!(output.contains("Paint"), "{output}");
        assert!(!output.contains("flowchart LR"), "{output}");
        assert!(output.contains("before"), "{output}");
        assert!(output.contains("after"), "{output}");
    }

    #[test]
    fn ignores_other_fence_languages() {
        assert!(
            MermaidRenderer::new()
                .render("rust", "fn main() {}", 80, &Theme::default())
                .is_none()
        );
    }

    #[test]
    fn invalid_mermaid_uses_markdown_fallback() {
        assert!(
            MermaidRenderer::new()
                .render("mermaid", "this is not Mermaid", 80, &Theme::default())
                .is_none()
        );
    }

    #[test]
    fn oversized_source_uses_markdown_fallback() {
        let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert!(
            MermaidRenderer::new()
                .render("mermaid", &oversized, 80, &Theme::default())
                .is_none()
        );
    }

    #[test]
    fn strips_control_bytes_from_rendered_labels() {
        let theme = Theme::default();
        let line = styled_line("safe\x1b]8;;https://example.com\x07label", &theme);
        let rendered = text(&line);

        assert!(!rendered.contains('\x1b'));
        assert!(!rendered.contains('\x07'));
        assert!(rendered.contains("safe"));
        assert!(rendered.contains("label"));
    }

    #[test]
    fn uses_theme_for_structure_and_labels() {
        let theme = Theme::default();
        let line = styled_line("┌─ Node ─┐", &theme);

        assert!(
            line.spans
                .iter()
                .any(|span| span.style.fg == Some(theme.dim))
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.content.contains("Node") && span.style.fg == Some(theme.text))
        );
    }
}
