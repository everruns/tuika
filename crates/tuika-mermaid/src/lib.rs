//! Terminal-native Mermaid fenced blocks for
//! [`tuika::components::Markdown`].
//!
//! [`MermaidRenderer`] adapts mmdflux's Unicode text output to tuika's generic
//! [`MarkdownBlockRenderer`] boundary. It
//! handles `mermaid` fences and returns `None` for every other language or for
//! Mermaid input mmdflux cannot render, preserving tuika's ordinary code-block
//! fallback.

use mmdflux::{OutputFormat, RenderConfig, TextColorMode, render_diagram};
use tuika::Theme;
use tuika::components::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};
use tuika::ui::Style;
use tuika::ui::{Line, Span};

/// Upper bound for one Mermaid fence handed to mmdflux.
const MAX_SOURCE_BYTES: usize = 64 * 1024;
/// Upper bound for mmdflux text retained as terminal cells.
const MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Progressively tighter graph separations — `(node_sep, rank_sep, edge_sep)`
/// in mmdflux's pixel units — tried when the default layout is wider than the
/// pane.
///
/// mmdflux's defaults (50/50/20) are tuned for SVG, where a pixel of
/// separation is a fraction of a glyph; on a character grid the same numbers
/// spread a diagram far past any terminal. Tightening `node_sep` is what
/// actually narrows a diagram, so the ladder squeezes it hardest. `rank_sep`
/// is held at or above 25: below that the layered ranker starts collapsing
/// ranks into each other, which trades height for a *much* wider result.
const FIT_LADDER: [(f64, f64, f64); 3] = [(30.0, 30.0, 12.0), (20.0, 25.0, 8.0), (12.0, 25.0, 5.0)];

/// Renders `mermaid` fenced blocks as Unicode terminal diagrams.
///
/// A diagram wider than the available columns is re-laid out with tighter
/// node separation until it fits, or until mmdflux's text renderer has no
/// slack left — a graph can be irreducibly wider than the pane, and the
/// narrowest layout is used in that case.
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

impl MarkdownBlockRenderer for MermaidRenderer {
    fn render(
        &self,
        block: MarkdownBlock<'_>,
        context: MarkdownBlockContext<'_>,
    ) -> Option<Vec<Line<'static>>> {
        let MarkdownBlock::Fenced { language, source } = block else {
            return None;
        };
        if !language.eq_ignore_ascii_case("mermaid") {
            return None;
        }
        if source.len() > MAX_SOURCE_BYTES {
            return None;
        }

        let rendered = fitted_diagram(source, context.width)?;
        Some(
            rendered
                .lines()
                .map(|line| styled_line(line, context.theme))
                .collect(),
        )
    }
}

/// The narrowest layout of `source` that fits `width`, or the narrowest one
/// mmdflux can produce when none does.
fn fitted_diagram(source: &str, width: u16) -> Option<String> {
    let budget = usize::from(width);
    // The default layout is the reference: a diagram that already fits keeps
    // mmdflux's own spacing, so nothing that renders well today changes.
    let mut best = render_layout(source, None)?;
    if diagram_width(&best) <= budget {
        return Some(best);
    }

    for separations in FIT_LADDER {
        // A tighter layout that fails where the default succeeded is not a
        // reason to lose the diagram — keep the widest-but-working one.
        let Some(candidate) = render_layout(source, Some(separations)) else {
            continue;
        };
        if diagram_width(&candidate) < diagram_width(&best) {
            best = candidate;
        }
        if diagram_width(&best) <= budget {
            break;
        }
    }
    Some(best)
}

/// One mmdflux text render, containing any failure as `None`.
fn render_layout(source: &str, separations: Option<(f64, f64, f64)>) -> Option<String> {
    let mut config = RenderConfig {
        text_color_mode: TextColorMode::Plain,
        ..RenderConfig::default()
    };
    if let Some((node_sep, rank_sep, edge_sep)) = separations {
        config.layout.node_sep = node_sep;
        config.layout.rank_sep = rank_sep;
        config.layout.edge_sep = edge_sep;
    }

    // Backstop: no panic in mmdflux is known to be reachable from here, but a
    // `View::render` must not be able to take the host application down
    // mid-frame over a diagram, so contain one if it appears. The panic hook is
    // deliberately left alone — `Markdown` re-renders every frame, and swapping
    // a global hook that often would swallow unrelated panics from other
    // threads for as long as the window is open.
    let rendered = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_diagram(source, OutputFormat::Text, &config)
    }));

    let rendered = rendered.ok()?.ok()?;
    (rendered.len() <= MAX_OUTPUT_BYTES).then_some(rendered)
}

/// Columns the rendered diagram occupies.
fn diagram_width(rendered: &str) -> usize {
    rendered
        .lines()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
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
    use tuika::style::StyleSheet;
    use tuika::testing::{grid, render};

    fn text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn render_block(block: MarkdownBlock<'_>) -> Option<Vec<Line<'static>>> {
        let theme = Theme::default();
        let sheet = StyleSheet::from_theme(&theme);
        MarkdownBlockRenderer::render(
            &MermaidRenderer::new(),
            block,
            MarkdownBlockContext::new(80, &theme, &sheet),
        )
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
            render_block(MarkdownBlock::Fenced {
                language: "rust",
                source: "fn main() {}",
            })
            .is_none()
        );
    }

    #[test]
    fn invalid_mermaid_uses_markdown_fallback() {
        assert!(
            render_block(MarkdownBlock::Fenced {
                language: "mermaid",
                source: "this is not Mermaid",
            })
            .is_none()
        );
    }

    /// A decision node with a `<br/>` label renders as a diagram, with every
    /// label line inside the shape rather than dropped or painted over the
    /// borders. mmdflux sized the diamond from the label's widest line and then
    /// centred the whole label against it, which underflowed for a wide label
    /// and overran the borders for a short one; 2.6.1 grows the shape to the
    /// full label instead. See
    /// <https://github.com/kevinswiber/mmdflux/issues/387>.
    #[test]
    fn multiline_diamond_label_renders_inside_the_shape() {
        for (source, labels) in [
            (
                "flowchart TD\n A[x] --> B{aaaa<br/>aaaa}\n",
                ["aaaa", "aaaa"],
            ),
            ("flowchart TD\n A[x] --> B{abc<br/>abc}\n", ["abc", "abc"]),
        ] {
            let lines = render_block(MarkdownBlock::Fenced {
                language: "mermaid",
                source,
            })
            .expect("a rendered diagram");
            let rendered: Vec<String> = lines.iter().map(text).collect();

            for label in labels {
                // Each label line sits on its own row, fenced by the diamond's
                // own side glyphs — not overlapping a border row.
                assert!(
                    rendered.iter().any(|line| {
                        let trimmed = line.trim_end();
                        trimmed.contains(label)
                            && trimmed.starts_with('<')
                            && trimmed.ends_with('>')
                    }),
                    "{source}: {label} missing from {rendered:#?}"
                );
            }
        }
    }

    /// A branching flowchart whose default layout runs far past a terminal.
    const BRANCHING: &str = "flowchart TD\n\
         A[Gateway model id] --> B{Pi registry provider-exact match?}\n\
         B -- yes --> C[Use registry]\n\
         B -- no --> D{Pi registry model-id fallback?}\n\
         D -- yes --> E[Use registry, minus cost]\n\
         D -- no --> F{models.dev match?}\n\
         F -- yes --> G[Use models.dev]\n\
         F -- no --> H[Safe defaults]\n\
         C --> I[Gateway pricing wins field-by-field]\n\
         E --> I\n\
         G --> I\n\
         H --> I\n";

    #[test]
    fn wide_diagram_is_compressed_toward_the_pane() {
        let relaxed = diagram_width(&render_layout(BRANCHING, None).expect("default layout"));
        let fitted = diagram_width(&fitted_diagram(BRANCHING, 80).expect("fitted layout"));

        assert!(
            fitted <= 80,
            "expected a fit within 80 columns, got {fitted}"
        );
        assert!(
            fitted < relaxed,
            "expected compression below the {relaxed}-column default, got {fitted}"
        );
    }

    /// A width no layout can satisfy settles on the narrowest rung rather than
    /// looping or losing the diagram. Degenerate widths reach this path, so it
    /// is the one that must not misbehave.
    #[test]
    fn unsatisfiable_width_settles_on_the_narrowest_layout() {
        let relaxed = diagram_width(&render_layout(BRANCHING, None).expect("default layout"));
        let floor = diagram_width(
            &render_layout(BRANCHING, Some(FIT_LADDER[FIT_LADDER.len() - 1])).expect("last rung"),
        );

        for width in [0, 1, 4] {
            let fitted = diagram_width(&fitted_diagram(BRANCHING, width).expect("a layout"));
            assert!(
                fitted <= floor && fitted < relaxed,
                "width {width}: expected the {floor}-column floor, got {fitted}"
            );
        }
    }

    /// Compression is only for diagrams that overflow: one that already fits
    /// keeps mmdflux's own spacing.
    #[test]
    fn narrow_diagram_keeps_the_default_layout() {
        let source = "flowchart LR\n A[Parse] --> B[Paint]\n";
        let relaxed = render_layout(source, None).expect("default layout");

        assert_eq!(
            fitted_diagram(source, 80).as_deref(),
            Some(relaxed.as_str())
        );
    }

    #[test]
    fn oversized_source_uses_markdown_fallback() {
        let oversized = "x".repeat(MAX_SOURCE_BYTES + 1);
        assert!(
            render_block(MarkdownBlock::Fenced {
                language: "mermaid",
                source: &oversized,
            })
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
