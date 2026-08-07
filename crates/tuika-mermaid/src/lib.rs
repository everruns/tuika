//! Terminal-native Mermaid fenced blocks for
//! [`tuika::components::Markdown`].
//!
//! [`MermaidRenderer`] adapts mmdflux's Unicode text output to tuika's generic
//! [`MarkdownBlockRenderer`] seam. It
//! handles `mermaid` fences and returns `None` for every other language or for
//! Mermaid input mmdflux cannot render, preserving tuika's ordinary code-block
//! fallback.

use mmdflux::{OutputFormat, RenderConfig, TextColorMode, render_diagram};
use ratatui_core::style::Style;
use ratatui_core::text::{Line, Span};
use tuika::Theme;
use tuika::components::{MarkdownBlock, MarkdownBlockContext, MarkdownBlockRenderer};

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

        if trips_diamond_label_bug(source) {
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

/// Whether `source` would hit mmdflux's broken multi-line diamond label.
///
/// Upstream: <https://github.com/kevinswiber/mmdflux/issues/387>. Fixed in no
/// release as of 2.6.0 — drop this guard, and the `catch_unwind` in
/// [`render_layout`], once it lands.
///
/// `render_diamond` renders a fixed three-row shape sized from the label's
/// *widest line*, then centres the *whole* label — line breaks included —
/// against that width. Mermaid accepts `<br/>` in a decision node and it is the
/// natural way to keep a wide one readable, so this is reachable from ordinary
/// documents rather than only from malformed ones, and it fails three different
/// ways: a subtraction underflow that panics under debug overflow checks, the
/// same underflow wrapping in release so the label is dropped and an empty
/// diamond renders, and — for labels short enough not to underflow — the label
/// written across the shape's own borders. Only the first is contained by
/// `catch_unwind`; the other two look like success. So the shape is recognised
/// up front and the fence keeps tuika's code-block fallback, where the diagram
/// is at least readable as source.
///
/// Only `flowchart`/`graph` sources are scanned: `{}` delimits a node label
/// there, while in a class or state diagram it opens a member block that
/// legitimately spans lines.
fn trips_diamond_label_bug(source: &str) -> bool {
    let is_flowchart = source
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("%%"))
        .is_some_and(|line| line.starts_with("flowchart") || line.starts_with("graph"));
    if !is_flowchart {
        return false;
    }

    source.lines().any(|line| {
        let Some(open) = line.find('{') else {
            return false;
        };
        let Some(close) = line.rfind('}') else {
            return false;
        };
        close > open && line[open..close].contains("<br")
    })
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

    // Backstop only: `trips_diamond_label_bug` keeps the one panic we know
    // about (https://github.com/kevinswiber/mmdflux/issues/387) out of this
    // call, but a `View::render` must not be able to take the host application
    // down mid-frame over a malformed diagram, so contain the rest too. The
    // panic hook is deliberately left alone — `Markdown` re-renders every
    // frame, and swapping a global hook that often would swallow unrelated
    // panics from other threads for as long as the window is open.
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

    /// A multi-line diamond label degrades to the code-block fallback rather
    /// than unwinding into the host — both the label wide enough to underflow
    /// mmdflux's centering and the short one that silently paints over the
    /// shape's borders. See <https://github.com/kevinswiber/mmdflux/issues/387>.
    #[test]
    fn multiline_diamond_label_uses_markdown_fallback() {
        for source in [
            "flowchart TD\n A[x] --> B{aaaa<br/>aaaa}\n",
            "flowchart TD\n A[x] --> B{abc<br/>abc}\n",
        ] {
            assert!(
                render_block(MarkdownBlock::Fenced {
                    language: "mermaid",
                    source,
                })
                .is_none(),
                "{source}"
            );
        }
    }

    /// The `catch_unwind` backstop, exercised against the same input with the
    /// pre-check bypassed: even reaching mmdflux, the seam must return rather
    /// than unwind.
    #[test]
    fn panicking_layout_is_contained_rather_than_unwound() {
        assert!(render_layout("flowchart TD\n A[x] --> B{aaaa<br/>aaaa}\n", None).is_none());
    }

    /// Only flowcharts use `{}` for a node label; a class diagram's member
    /// block must not be mistaken for one.
    #[test]
    fn diamond_precheck_is_scoped_to_flowcharts() {
        assert!(trips_diamond_label_bug(
            "flowchart TD\n A --> B{one<br/>two}\n"
        ));
        assert!(trips_diamond_label_bug(
            "%% a comment\ngraph LR\n A --> B{one<br>two}\n"
        ));
        assert!(!trips_diamond_label_bug(
            "flowchart TD\n A[one<br/>two] --> B\n"
        ));
        assert!(!trips_diamond_label_bug("flowchart TD\n A --> B{plain}\n"));
        assert!(!trips_diamond_label_bug(
            "classDiagram\n class Foo {\n +bar()<br/>\n }\n"
        ));
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
