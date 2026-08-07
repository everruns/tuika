//! Single-component demo harness.
//!
//! It renders one component per scene inside a small labeled frame, and is the
//! source of truth for the gallery: the scene registry drives the CLI, the tape
//! generator, and the integrity check.
//!
//! ```text
//! cargo run --example demo -- spinner          # interactive, records a GIF
//! cargo run --example demo -- spinner --dump    # print one frame as text
//! cargo run --example demo -- list              # list scene names
//! cargo run --example demo -- check             # verify the docs assets
//! cargo run --example demo -- tapes <dir>       # emit VHS tapes (used by the generator)
//! ```
//!
//! The GIFs under `docs/demos/` are recorded by `scripts/gen-demos.sh`,
//! which asks this example to emit the VHS tapes and records each — the tapes
//! are generated, not committed. See `AGENTS.md`.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crossterm::event::{self, Event as CtEvent, KeyCode as CtKeyCode, KeyEventKind};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::{Terminal, TerminalOptions, Viewport as RatatuiViewport};

use tuika::framebuffer::{FrameBuffer, FrameBufferView};
use tuika::prelude::*;
use tuika::view::DrawView;

/// A tiny self-contained Rust highlighter for the `markdown`/`code_block` scenes.
///
/// The examples deliberately avoid a grammar dependency (that would drag
/// tree-sitter into tuika's dev/MSRV builds); this shows how little it takes to
/// satisfy the [`Highlighter`] seam. For production-grade highlighting across
/// many languages, use the `tuika-codeformatters` crate.
struct DemoHighlighter;

/// A `'static` instance so `Markdown`/`CodeBlock` views (which borrow one) can be
/// boxed into `Element` in the scene builders.
static HL: DemoHighlighter = DemoHighlighter;

impl Highlighter for DemoHighlighter {
    fn highlight(
        &self,
        lang: &str,
        lines: &[&str],
        theme: &Theme,
    ) -> Option<Vec<Vec<Span<'static>>>> {
        if lang != "rust" {
            return None;
        }
        Some(
            lines
                .iter()
                .map(|l| highlight_rust(l, &theme.code))
                .collect(),
        )
    }
}

/// Split one line of Rust into styled spans, reconstructing it exactly.
fn highlight_rust(line: &str, code: &CodeTheme) -> Vec<Span<'static>> {
    const KW: &[&str] = &[
        "fn", "let", "mut", "pub", "use", "struct", "enum", "impl", "match", "return", "for", "in",
        "if", "else", "while", "loop", "const", "as", "mod", "trait", "self", "true", "false",
    ];
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            let rest: String = chars[i..].iter().collect();
            out.push(Span::styled(
                rest,
                Style::default()
                    .fg(code.comment)
                    .add_modifier(Modifier::ITALIC),
            ));
            break;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += if chars[i] == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(chars.len());
            let s: String = chars[start..i].iter().collect();
            out.push(Span::styled(s, Style::default().fg(code.string)));
            continue;
        }
        if c.is_alphanumeric() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let style = if KW.contains(&word.as_str()) {
                Style::default()
                    .fg(code.keyword)
                    .add_modifier(Modifier::BOLD)
            } else if word.chars().all(|d| d.is_ascii_digit()) {
                Style::default().fg(code.constant)
            } else {
                Style::default().fg(code.text)
            };
            out.push(Span::styled(word, style));
            continue;
        }
        let start = i;
        i += 1;
        while i < chars.len() {
            let d = chars[i];
            if d.is_alphanumeric()
                || d == '_'
                || d == '"'
                || (d == '/' && chars.get(i + 1) == Some(&'/'))
            {
                break;
            }
            i += 1;
        }
        let s: String = chars[start..i].iter().collect();
        let style = if s.trim().is_empty() {
            Style::default().fg(code.text)
        } else {
            Style::default().fg(code.punctuation)
        };
        out.push(Span::styled(s, style));
    }
    out
}

type Build = fn(u64, &Theme) -> Element;

/// A single gallery scene. The registry below is the one place a component's
/// demo is declared — name, blurb, recording size, and builder.
struct Demo {
    name: &'static str,
    /// Public-facing component name shown in the recording.
    title: &'static str,
    blurb: &'static str,
    /// Frame height in rows, chrome included. The scene is pinned to exactly this
    /// many rows when recorded, so anything taller is cut off — see [`check`].
    rows: u16,
    /// Motion scenes hold longer and record at a higher frame rate.
    animated: bool,
    /// The scene fills its frame edge to edge on purpose — a viewport or log tail
    /// whose whole point is that content runs past the bottom. Exempts it from
    /// the clipping check in [`check`].
    fills_frame: bool,
    build: Build,
}

impl Demo {
    const fn asset_extension(&self) -> &'static str {
        if self.animated { "gif" } else { "png" }
    }

    fn asset_filename(&self) -> String {
        format!("{}.{}", self.name, self.asset_extension())
    }
}

const fn demo(
    name: &'static str,
    title: &'static str,
    blurb: &'static str,
    rows: u16,
    animated: bool,
    build: Build,
) -> Demo {
    Demo {
        name,
        title,
        blurb,
        rows,
        animated,
        fills_frame: false,
        build,
    }
}

/// A scene that runs to the bottom of its frame by design.
const fn filling_demo(
    name: &'static str,
    title: &'static str,
    blurb: &'static str,
    rows: u16,
    animated: bool,
    build: Build,
) -> Demo {
    Demo {
        fills_frame: true,
        ..demo(name, title, blurb, rows, animated, build)
    }
}

const DEMOS: &[Demo] = &[
    demo(
        "spinner",
        "Spinner",
        "frame-cycled activity glyphs",
        10,
        true,
        scene_spinner,
    ),
    demo(
        "progress_bar",
        "ProgressBar",
        "determinate & indeterminate bars",
        12,
        true,
        scene_progress,
    ),
    demo(
        "activity_list",
        "ActivityList",
        "task lifecycle with optional per-step progress",
        15,
        true,
        scene_activity_list,
    ),
    demo(
        "loader",
        "Loader",
        "spinner + message + hint row",
        9,
        true,
        scene_loader,
    ),
    demo(
        "text",
        "Text",
        "styled lines and word-wrapped prose",
        11,
        false,
        scene_text,
    ),
    demo(
        "markdown",
        "Markdown",
        "CommonMark streamed in, only the tail re-parses",
        18,
        true,
        scene_markdown,
    ),
    demo(
        "markdown_table",
        "Markdown table",
        "GFM table: alignment, emoji, and links",
        13,
        false,
        scene_markdown_table,
    ),
    demo(
        "markdown_html",
        "Markdown inline HTML",
        "tags styled through the markdown roles",
        18,
        false,
        scene_markdown_html,
    ),
    demo(
        "code_block",
        "CodeBlock",
        "syntax-highlighted code with line numbers",
        12,
        false,
        scene_code_block,
    ),
    demo(
        "diff",
        "Diff",
        "unified line diff with +/- gutters and line numbers",
        14,
        false,
        scene_diff,
    ),
    demo(
        "ascii_font",
        "AsciiFont",
        "large figlet-style block-letter banners",
        12,
        false,
        scene_ascii_font,
    ),
    demo(
        "qr",
        "QrCode",
        "QR code encoded and drawn with half-blocks",
        24,
        false,
        scene_qr,
    ),
    demo(
        "rule",
        "Rule",
        "titled horizontal separators",
        12,
        false,
        scene_rule,
    ),
    demo(
        "boxed",
        "Boxed",
        "borders, titles, and padding",
        12,
        false,
        scene_boxed,
    ),
    demo(
        "flex",
        "Flex",
        "flexbox grow / fixed distribution",
        10,
        false,
        scene_flex,
    ),
    filling_demo(
        "app_shell",
        "AppShell",
        "responsive tool chrome around growing content",
        15,
        false,
        scene_app_shell,
    ),
    filling_demo(
        "selection_screen",
        "SelectionScreen",
        "responsive borrowed action picker",
        15,
        false,
        scene_selection_screen,
    ),
    demo(
        "flow",
        "Flow",
        "intrinsic-width items wrapping into flex lines",
        12,
        false,
        scene_flow,
    ),
    demo(
        "grid",
        "Grid",
        "a small equal-column terminal grid",
        15,
        false,
        scene_grid,
    ),
    filling_demo(
        "scroll",
        "Scroll",
        "viewport + scrollbar over long content",
        13,
        true,
        scene_scroll,
    ),
    filling_demo(
        "item_scroll",
        "ItemScroll",
        "the same viewport over laid-out items, not lines",
        13,
        true,
        scene_item_scroll,
    ),
    demo(
        "scrollbar",
        "Scrollbar + VirtualWindow",
        "one range, vertical or horizontal",
        12,
        false,
        scene_scrollbar,
    ),
    demo(
        "primitives",
        "Primitives",
        "owned dialog + form + panning custom viewport",
        17,
        true,
        scene_primitives,
    ),
    demo(
        "dialog_presets",
        "Dialog presets",
        "confirm, choice, multi-choice, and input flows",
        17,
        true,
        scene_dialog_presets,
    ),
    demo(
        "select",
        "SelectList",
        "keyboard-navigable selection list",
        11,
        true,
        scene_select,
    ),
    demo(
        "keyed_table",
        "KeyedTable",
        "projected borrowed rows with stable keyed selection",
        11,
        true,
        scene_keyed_table,
    ),
    demo(
        "completion_palette",
        "CompletionPalette",
        "filter-ranked commands with host-owned selection",
        13,
        true,
        scene_completion_palette,
    ),
    demo("tabs", "Tabs", "host-state tab strip", 9, true, scene_tabs),
    demo(
        "tab_select",
        "TabSelect",
        "value-selecting segmented control",
        8,
        true,
        scene_tab_select,
    ),
    demo(
        "slider",
        "Slider",
        "value picker over a numeric range",
        10,
        true,
        scene_slider,
    ),
    demo(
        "timeline",
        "Timeline",
        "keyframed easing tracks sampled from the frame counter",
        12,
        true,
        scene_timeline,
    ),
    demo(
        "toast",
        "Toasts",
        "level-colored notification stack",
        9,
        false,
        scene_toast,
    ),
    filling_demo(
        "console",
        "Console",
        "captured stdout/log tail overlay",
        11,
        false,
        scene_console,
    ),
    demo(
        "framebuffer",
        "FrameBuffer",
        "RGBA canvas + sprite drawn with half-blocks",
        14,
        true,
        scene_framebuffer,
    ),
    demo(
        "status_bar",
        "StatusBar",
        "left / right status segments",
        7,
        false,
        scene_status_bar,
    ),
    demo(
        "key_hints",
        "KeyHints",
        "priority-aware fitting at constrained widths",
        10,
        false,
        scene_key_hints,
    ),
    demo(
        "keymap_help",
        "KeymapHelp",
        "complete help generated from active bindings",
        12,
        false,
        scene_keymap_help,
    ),
    demo(
        "textinput",
        "TextInput",
        "multi-line edit model",
        9,
        true,
        scene_textinput,
    ),
    demo(
        "hyperlink",
        "Hyperlink",
        "OSC 8 links — clickable URLs in the transcript",
        12,
        false,
        scene_hyperlink,
    ),
    demo(
        "mouse",
        "Mouse",
        "drag-to-select, highlight, and clickable regions",
        11,
        true,
        scene_mouse,
    ),
    demo(
        "overlay",
        "Overlay",
        "target-relative popover with edge-aware flipping",
        18,
        false,
        scene_overlay,
    ),
];

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("list");

    match cmd {
        "list" | "--list" | "-h" | "--help" => {
            println!("scenes:");
            for d in DEMOS {
                println!("  {:<14} {}", d.name, d.blurb);
            }
            Ok(())
        }
        "check" => check(),
        "tapes" => {
            let Some(dir) = args.get(1) else {
                eprintln!("usage: demo tapes <output-dir>");
                std::process::exit(2);
            };
            emit_tapes(Path::new(dir))
        }
        name => {
            let Some(d) = DEMOS.iter().find(|d| d.name == name) else {
                eprintln!("unknown scene {name:?}; run `list` to see the options");
                std::process::exit(2);
            };
            if args.iter().any(|a| a == "--dump") {
                dump(d)
            } else {
                run(d)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// `tapes` — emit a VHS tape per scene from the registry, so the tapes are
// generated rather than committed. Recorded at ~2× pixel density and displayed
// at `width="880"`, so the assets stay crisp on HiDPI screens. Motion scenes
// record as GIF; settled scenes use a full-color PNG screenshot. `Output` is
// relative to the tuika crate dir (the generator cds there); the command is an
// absolute path to this very binary.
// ---------------------------------------------------------------------------

/// Recorded scene width in columns. Every scene records at the same width so the
/// GIFs line up in the gallery; only the height varies, per `Demo::rows`.
const RECORD_COLS: u16 = 66;
/// Upper bounds on the cell size, in pixels, that VHS's default monospace at
/// `Set FontSize 40` yields in `xterm.js` on the reference capture host. A tape
/// is sized in *pixels* and the emulator divides by its own cell size, so these
/// are deliberately rounded up: the terminal is then never smaller than
/// `RECORD_COLS × rows`, and the leftover row or column is absorbed by the scene
/// pinning in [`run`].
const CELL_W: u32 = 26;
const CELL_H: u32 = 55;
/// The tape's `Set Padding`, applied on every side.
const PADDING: u32 = 40;

fn emit_tapes(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let bin = std::env::current_exe()?;
    let width = u32::from(RECORD_COLS) * CELL_W + PADDING * 2;
    for d in DEMOS {
        let height = u32::from(d.rows) * CELL_H + PADDING * 2;
        let (output, ending, fps) = if d.animated {
            (
                format!("Output docs/demos/{}.gif", d.name),
                "Sleep 4s".to_owned(),
                24,
            )
        } else {
            // VHS only renders screenshots after the next captured frame. Its
            // ordinary output is disposable but keeps that frame stream alive.
            (
                format!("Output \"{}/{}.gif\"", dir.display(), d.name),
                format!(
                    "Sleep 300ms\nScreenshot docs/demos/{}.png\nSleep 100ms",
                    d.name
                ),
                12,
            )
        };
        let tape = format!(
            "# Generated by `demo -- tapes`; recorded by scripts/gen-demos.sh.\n\
             # Do not edit or commit — regenerate from the scene registry instead.\n\
             {output}\n\
             \n\
             Set Shell bash\n\
             Set FontSize 40\n\
             Set CursorBlink false\n\
             Set Width {width}\n\
             Set Height {height}\n\
             Set Padding {PADDING}\n\
             Set Framerate {fps}\n\
             \n\
             Hide\n\
             Type \"{bin} {name}\"\n\
             Enter\n\
             Sleep 900ms\n\
             Show\n\
             {ending}\n",
            name = d.name,
            bin = bin.display(),
        );
        fs::write(dir.join(format!("{}.tape", d.name)), tape)?;
    }
    println!("wrote {} tapes to {}", DEMOS.len(), dir.display());
    Ok(())
}

// ---------------------------------------------------------------------------
// `check` — the gallery integrity guard. The scene registry is the source of
// truth: every scene needs a non-empty recording in its declared format, no
// orphan asset may linger, and every referenced component asset must map to a
// real scene. Runs in CI (the MSRV job) and at the end of the generator, so
// drift fails loudly instead of shipping a broken image to docs.rs.
// ---------------------------------------------------------------------------

fn check() -> io::Result<()> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let demos = dir.join("docs/demos");
    let mut errors: Vec<String> = Vec::new();

    // Every scene has a non-empty recording.
    for d in DEMOS {
        let asset = demos.join(d.asset_filename());
        match fs::metadata(&asset) {
            Ok(m) if m.len() > 0 => {}
            Ok(_) => errors.push(format!("recording {} is empty", asset.display())),
            Err(_) => errors.push(format!(
                "scene `{}` has no recording at {} (run scripts/gen-demos.sh)",
                d.name,
                asset.display()
            )),
        }
    }

    // Every scene fits its recorded frame. `rows` is hand-picked, so a scene that
    // outgrows it records a *silently* clipped GIF — which is how a QR code, a
    // banner, and a diff shipped with their bottoms cut off. Rendering the scene
    // again with room to spare surfaces that: any line the taller frame shows and
    // the recorded one does not is a line the recording loses. Scenes that overflow
    // by design (`fills_frame`) are exempt.
    for d in DEMOS.iter().filter(|d| !d.fills_frame) {
        let lost = lost_to_clipping(d, d.rows);
        if !lost.is_empty() {
            errors.push(format!(
                "scene `{}` is clipped by its {}-row frame; {} line(s) never make it \
                 into the recording, starting with `{}` — raise `rows` and re-record",
                d.name,
                d.rows,
                lost.len(),
                lost[0].trim()
            ));
        }
    }

    // Guard the guard. A comparison that stops finding anything would let the
    // loop above pass vacuously, so require it to still have teeth: squeezing a
    // scene by a row has to register somewhere.
    if !DEMOS
        .iter()
        .any(|d| !lost_to_clipping(d, d.rows.saturating_sub(1)).is_empty())
    {
        errors.push(
            "the clipping check no longer detects a scene squeezed by a row — it is \
             passing vacuously"
                .to_owned(),
        );
    }

    // No orphan or stale-format recording without a matching scene declaration.
    for extension in ["gif", "png"] {
        for stem in stems(&demos, extension) {
            match DEMOS.iter().find(|d| d.name == stem.as_str()) {
                None => errors.push(format!(
                    "docs/demos/{stem}.{extension} has no matching scene in DEMOS"
                )),
                Some(d) if d.asset_extension() != extension => errors.push(format!(
                    "docs/demos/{stem}.{extension} is stale; scene `{stem}` uses .{}",
                    d.asset_extension()
                )),
                Some(_) => {}
            }
        }
    }

    // Every demo asset referenced by a component doc or gallery page maps to a
    // scene and uses its declared format.
    let mut sources: Vec<PathBuf> = vec![
        dir.join("docs/components.md"),
        dir.join("docs/features.md"),
        dir.join("docs/markdown.md"),
    ];
    // A component is a file *or* a directory of submodules (markdown is split
    // into one), and the embed usually rides the view's own file — so descend,
    // or a component that outgrows a single file silently leaves the check.
    let mut dirs = vec![dir.join("src/components")];
    while let Some(next) = dirs.pop() {
        for entry in fs::read_dir(next)?.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                sources.push(path);
            }
        }
    }
    // Module-level docs outside `components/` may embed a demo too (e.g. the
    // `overlay` GIF on `OverlaySpec`).
    sources.push(dir.join("src/overlay.rs"));
    for path in sources {
        // A doc source may not exist yet (e.g. features.md before it lands);
        // skip a missing one rather than failing the whole check.
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for (referenced, extension) in referenced_assets(&text) {
            match DEMOS.iter().find(|d| d.name == referenced.as_str()) {
                None => errors.push(format!(
                    "{} references demos/{referenced}.{extension} but there is no such scene",
                    path.display()
                )),
                Some(d) if d.asset_extension() != extension => errors.push(format!(
                    "{} references demos/{referenced}.{extension}, but scene `{referenced}` uses .{}",
                    path.display(),
                    d.asset_extension()
                )),
                Some(_) => {}
            }
        }
    }

    if errors.is_empty() {
        println!(
            "ok: {} scenes, recordings, and references in sync",
            DEMOS.len()
        );
        Ok(())
    } else {
        for e in &errors {
            eprintln!("error: {e}");
        }
        std::process::exit(1);
    }
}

/// Lines `d` would paint with room to spare that a `rows`-tall frame never
/// shows — that is, the content a recording of that height silently loses.
fn lost_to_clipping(d: &Demo, rows: u16) -> Vec<String> {
    /// Extra rows granted to the reference render. Wide enough to reveal a
    /// caption or a couple of trailing lines, narrow enough that a scene sized
    /// against the terminal does not reflow into something unrecognizable.
    const SLACK: u16 = 8;

    let theme = Theme::default();
    let at = |rows: u16| {
        let root = framed(d.title, d.blurb, (d.build)(24, &theme), &theme);
        let buffer = tuika::testing::render(root.as_ref(), RECORD_COLS, rows, &theme);
        tuika::testing::grid(&buffer)
            .lines()
            .map(|l| l.trim_end().to_owned())
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
    };
    let shown = at(rows);
    at(rows + SLACK)
        .into_iter()
        .filter(|l| !shown.contains(l))
        .collect()
}

/// File stems (without extension) of every `*.ext` entry in `dir`.
fn stems(dir: &Path, ext: &str) -> BTreeSet<String> {
    let Ok(read) = fs::read_dir(dir) else {
        return BTreeSet::new();
    };
    read.filter_map(Result::ok)
        .filter_map(|e| {
            let path = e.path();
            (path.extension().and_then(|s| s.to_str()) == Some(ext))
                .then(|| path.file_stem().and_then(|s| s.to_str()).map(str::to_owned))
                .flatten()
        })
        .collect()
}

/// Every `demos/<name>.{gif,png}` reference in `text`, relative or absolute.
fn referenced_assets(text: &str) -> BTreeSet<(String, String)> {
    const MARKER: &str = "demos/";
    let mut found = BTreeSet::new();
    for (idx, _) in text.match_indices(MARKER) {
        let rest = &text[idx + MARKER.len()..];
        let Some((end, extension)) = ["gif", "png"]
            .into_iter()
            .filter_map(|extension| {
                rest.find(&format!(".{extension}"))
                    .map(|end| (end, extension))
            })
            .min_by_key(|(end, _)| *end)
        else {
            continue;
        };
        let stem = &rest[..end];
        if !stem.is_empty() && stem.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            found.insert((stem.to_owned(), extension.to_owned()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_assets_finds_static_and_animated_recordings() {
        let found =
            referenced_assets("demos/text.png and https://example.test/docs/demos/spinner.gif");

        assert_eq!(
            found,
            BTreeSet::from([
                ("spinner".to_owned(), "gif".to_owned()),
                ("text".to_owned(), "png".to_owned()),
            ])
        );
    }

    #[test]
    fn scene_format_tracks_whether_motion_is_part_of_the_demo() {
        for demo in DEMOS {
            assert_eq!(
                demo.asset_extension(),
                if demo.animated { "gif" } else { "png" }
            );
            assert!(!demo.title.contains('_'));
        }
    }
}

/// Common chrome: a title/blurb header, a rule, then the scene body.
fn framed(name: &str, blurb: &str, body: Element, theme: &Theme) -> Element {
    let bg = Style::default().bg(theme.background);
    let header = Text::new(vec![Line::from(vec![
        Span::styled(
            name.to_string(),
            theme.accent_style().add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {blurb}"), theme.muted_style()),
    ])]);
    view! {
        col(padding = Padding::all(1), gap = 0, background = bg) {
            fixed(1) { node(header) }
            fixed(1) { node(Rule::new().style(theme.muted_style())) }
            fixed(1) { spacer() }
            grow(1) { node(body) }
        }
    }
}

/// Render one frame into an in-memory buffer and print it — no terminal needed.
///
/// Uses the scene's recorded geometry, so a dump is a faithful preview of the
/// GIF: content that would be clipped out of the recording is clipped here too.
fn dump(d: &Demo) -> io::Result<()> {
    let theme = Theme::default();
    let root = framed(d.title, d.blurb, (d.build)(24, &theme), &theme);
    let buffer = tuika::testing::render(root.as_ref(), RECORD_COLS, d.rows, &theme);
    println!("{}", tuika::testing::grid(&buffer));
    Ok(())
}

/// Interactive loop: animate from a frame counter until `q`/`Esc`.
///
/// The scene is pinned to `RECORD_COLS × d.rows` rather than filling the
/// terminal. A recorder sizes its window in pixels and the emulator divides by
/// whatever cell size the font happens to give it, so "how many rows the scene
/// gets" is otherwise a property of the recording host — which is how demos ended
/// up clipped. Pinning makes the registry the authority instead, and the surplus
/// (the tape asks for a little more room than the scene needs) is painted in the
/// theme background, so it reads as margin.
fn run(d: &Demo) -> io::Result<()> {
    let _session = tuika::TerminalSession::enter()?;
    let mut terminal = Terminal::with_options(
        ratatui::backend::CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: RatatuiViewport::Fullscreen,
        },
    )?;
    let theme = Theme::default();
    let mut frame = 0u64;
    loop {
        terminal.draw(|f| {
            let area = f.area();
            let (w, h) = (area.width.min(RECORD_COLS), area.height.min(d.rows));
            let scene = Rect {
                x: area.x + (area.width - w) / 2,
                y: area.y + (area.height - h) / 2,
                width: w,
                height: h,
            };
            f.buffer_mut()
                .set_style(area, Style::default().bg(theme.background));
            let root = framed(d.title, d.blurb, (d.build)(frame, &theme), &theme);
            paint(f.buffer_mut(), scene, &theme, root.as_ref(), &[]);
        })?;
        if event::poll(Duration::from_millis(80))?
            && let CtEvent::Key(key) = event::read()?
            && key.kind != KeyEventKind::Release
            && matches!(key.code, CtKeyCode::Char('q') | CtKeyCode::Esc)
        {
            break;
        }
        frame = frame.wrapping_add(1);
    }
    let _ = terminal.clear();
    drop(terminal);
    Ok(())
}

// ---------------------------------------------------------------------------
// Scenes. Each is a pure function of the frame counter and theme.
// ---------------------------------------------------------------------------

fn labeled_row(glyph: Element, label: &str, theme: &Theme) -> Element {
    let text = Text::new(vec![Line::from(Span::styled(
        label.to_string(),
        theme.text_style(),
    ))]);
    view! {
        row(gap = 1) {
            fixed(2) { node(glyph) }
            grow(1) { node(text) }
        }
    }
}

fn scene_spinner(frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Braille)), "Braille — the smooth default", theme)) }
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Line)), "Line — ASCII fallback", theme)) }
            fixed(1) { node(labeled_row(element(Spinner::new(frame).style(SpinnerStyle::Dots)), "Dots — bouncing", theme)) }
        }
    }
}

fn scene_progress(frame: u64, theme: &Theme) -> Element {
    let animated = tuika::anim::ping_pong(frame, 140);
    let _ = theme;
    view! {
        col(gap = 1) {
            fixed(1) { node(ProgressBar::determinate(0.25).percent(true)) }
            fixed(1) { node(ProgressBar::determinate(0.60).label("0:42 / 3:07").percent(true)) }
            fixed(1) { node(ProgressBar::determinate(animated).percent(true)) }
            fixed(1) { node(ProgressBar::indeterminate(frame)) }
        }
    }
}

fn scene_activity_list(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let progress = tuika::anim::ping_pong(frame, 180);
    element(
        ActivityList::new(vec![
            ActivityItem::new("Resolve dependencies", ActivityStatus::Succeeded)
                .detail("42 crates"),
            ActivityItem::new("Compile workspace", ActivityStatus::Running)
                .detail("tuika")
                .progress(progress),
            ActivityItem::new("Run tests", ActivityStatus::Queued),
            ActivityItem::new("Publish artifacts", ActivityStatus::Skipped),
        ])
        .frame(frame)
        .gap(1),
    )
}

fn scene_loader(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    view! {
        col(gap = 1) {
            fixed(1) { node(Loader::new(frame, "compiling crate…").hint("esc to cancel")) }
            fixed(1) { node(Loader::new(frame, "fetching dependencies…").spinner_style(SpinnerStyle::Line)) }
        }
    }
}

fn scene_text(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let styled = Text::new(vec![
        Line::from(vec![
            Span::styled(
                "error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::styled(": unresolved import ", theme.text_style()),
            Span::styled("`tuika::Widget`", theme.accent_style()),
        ]),
        Line::from(Span::styled(
            "  perhaps you meant `tuika::View`?",
            theme.muted_style(),
        )),
    ]);
    let prose = tuika::components::Paragraph::new(
        "Paragraph word-wraps plain text to the render width in a single style, \
         re-flowing every frame so a resize just re-wraps.",
        theme.text_style(),
    );
    view! {
        col(gap = 1) {
            fixed(2) { node(styled) }
            grow(1) { node(prose) }
        }
    }
}

/// The document the `markdown` scene streams in, one glyph at a time.
const MARKDOWN_DOC: &str = "\
# Streaming Markdown

Renders **CommonMark** as it *streams* — only the in-flight tail
re-parses, so settled blocks and `code` never re-tokenize.

- headings, **bold**, *italic*, `inline code`
- nested lists and tables

```rust
fn greet(name: &str) {
    println!(\"hello, {name}!\");
}
```
";

/// Animated: reveal `MARKDOWN_DOC` progressively through a `MarkdownState`, the
/// same way a host feeds an assistant message as it streams. Holds on the full
/// document, then restarts.
fn scene_markdown(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let total = MARKDOWN_DOC.chars().count();
    // Reveal briskly so the whole document — including the highlighted code
    // block — lands within a short recording, then hold before the cycle repeats.
    let pos = (frame as usize * 6) % (total + 120);
    let revealed: String = MARKDOWN_DOC.chars().take(pos.min(total)).collect();

    let mut state = MarkdownState::new();
    state.set(revealed);
    // Rendered at a fixed width for the demo frame; a real host passes the live
    // viewport width so prose re-wraps on resize.
    let sheet = tuika::StyleSheet::from_theme(theme);
    let lines = state
        .lines(
            64,
            theme,
            &sheet,
            tuika::highlight::CodeHighlighter::With(&HL),
        )
        .to_vec();
    element(Text::new(lines))
}

/// The GFM table the `markdown_table` scene renders.
///
/// Deliberately exercises everything a table cell can carry: per-column
/// alignment from the `:---:` markers, inline code and bold, wide emoji (which
/// must be measured grapheme-aware or the borders drift), and links whose label
/// is what gets painted while the destination rides along as OSC 8.
const MARKDOWN_TABLE_DOC: &str = "\
| Component   |  Status   |                          Docs |
| :---------- | :-------: | ----------------------------: |
| `Markdown`  | ✅ stable | [docs.rs](https://docs.rs/tuika) |
| `CodeBlock` | ✅ stable | \
[gallery](https://github.com/everruns/tuika/blob/main/docs/components.md) |
| **Image**   |  🚧 beta  | \
[features](https://github.com/everruns/tuika/blob/main/docs/features.md) |
";

/// A GFM table rendered by the one-shot `Markdown` view.
///
/// Column widths come from the content and are fitted to the area, so the same
/// source reflows on resize rather than being pre-formatted by the host.
fn scene_markdown_table(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    element(Markdown::new(MARKDOWN_TABLE_DOC))
}

const MARKDOWN_HTML_DOC: &str = "\
### Inline HTML

Markdown in the wild carries HTML, so the presentational inline
tags render instead of being dropped — each resolving the same
`StyleSheet` role as the markdown it mirrors.

- <b>strong</b> and <em>emphasis</em>, <s>struck</s>, <u>underlined</u>
- <mark>highlighted</mark>, <kbd>Ctrl</kbd>+<kbd>C</kbd>, <a href=\"https://docs.rs/tuika\">a link</a>
- H<sub>2</sub>O and 3&times;10<sup>8</sup> m/s

A line<br>broken by a tag.
";

/// Inline HTML inside markdown: every tag resolves a `StyleSheet` role, so
/// `<b>` cannot look different from `**bold**`.
fn scene_markdown_html(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    element(Markdown::new(MARKDOWN_HTML_DOC))
}

/// A single themed, syntax-highlighted fenced block via `CodeBlock`.
fn scene_code_block(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let source = "pub fn fib(n: u64) -> u64 {\n    match n {\n        0 | 1 => n,\n        _ => fib(n - 1) + fib(n - 2),\n    }\n}";
    element(
        CodeBlock::new("rust", source)
            .highlighter(&HL)
            .line_numbers(true),
    )
}

fn scene_rule(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    view! {
        col(gap = 1) {
            fixed(1) { node(Rule::new().style(theme.muted_style())) }
            fixed(1) { node(Rule::new().title(Line::from(Span::styled(" Section ", theme.accent_style()))).style(theme.muted_style())) }
            fixed(1) { node(Rule::new().glyph('┈').style(theme.muted_style())) }
            fixed(1) { node(Rule::new().title(Line::from(Span::styled(" dotted ", theme.accent_style()))).glyph('·').style(theme.muted_style())) }
        }
    }
}

fn scene_boxed(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let inner = Text::new(vec![Line::from(Span::styled(
        "border + padding + title",
        theme.text_style(),
    ))]);
    let plain = Text::new(vec![Line::from(Span::styled(
        "rounded border",
        theme.text_style(),
    ))]);
    view! {
        col(gap = 1) {
            fixed(3) {
                boxed(title = Line::from(Span::styled(" thick ", theme.accent_style())), border = BorderStyle::Thick, padding = Padding::symmetric(1, 0)) {
                    node(inner)
                }
            }
            fixed(3) {
                boxed(title = Line::from(Span::styled(" rounded ", theme.accent_style())), border = BorderStyle::Rounded) {
                    node(plain)
                }
            }
        }
    }
}

fn scene_flex(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let cell = |label: &str, color: Color| -> Element {
        let text = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            Style::default()
                .fg(theme.background)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ))]);
        element(
            tuika::components::Boxed::new(element(text))
                .border(BorderStyle::Plain)
                .background(Style::default().bg(color)),
        )
    };
    view! {
        col(gap = 1) {
            fixed(3) {
                row(gap = 1) {
                    grow(1) { node(cell("grow 1", theme.accent)) }
                    grow(2) { node(cell("grow 2", theme.accent_alt)) }
                    fixed(12) { node(cell("fixed 12", theme.muted)) }
                }
            }
            fixed(1) { node(Text::new(vec![Line::from(Span::styled("row · gap 1 · grow shares leftover width", theme.muted_style()))])) }
        }
    }
}

fn scene_app_shell(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let query = "view";
    let header_text = theme.text_style();
    let header_accent = theme.accent_style();
    let header = view_fn(
        |available, _ctx| Size::new(available.width, available.height.min(2)),
        move |area, surface, _ctx| {
            surface.set_string(
                area.x.saturating_add(2),
                area.bottom().saturating_sub(1),
                "Search: ",
                header_text,
            );
            surface.set_string(
                area.x.saturating_add(10),
                area.bottom().saturating_sub(1),
                query,
                header_text,
            );
            surface.set_string(
                area.right().saturating_sub(10),
                area.bottom().saturating_sub(1),
                " 3 matches ",
                header_accent,
            );
        },
    );
    let content = Boxed::new(element(Text::new(vec![
        Line::from(Span::styled("app_shell.rs", theme.text_style())),
        Line::from(Span::styled("flex.rs", theme.text_style())),
        Line::from(Span::styled("responsive.rs", theme.text_style())),
    ])))
    .title(" Files ");
    let status_ready = theme.selection_style();
    let status_muted = theme.muted_style();
    let status = view_fn(
        |available, _ctx| Size::new(available.width, available.height.min(1)),
        move |area, surface, _ctx| {
            surface.set_string(area.x, area.y, " READY ", status_ready);
            surface.set_string(
                area.right().saturating_sub(15),
                area.y,
                "borrowed state ",
                status_muted,
            );
        },
    );

    element(
        AppShell::new(content)
            .header(header)
            .top_rule()
            .status(status)
            .bottom_rule()
            .footer(KeyHints::new([
                ("↑/↓", "move"),
                ("enter", "open"),
                ("q", "quit"),
            ])),
    )
}

fn scene_selection_screen(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let rows = vec![
        Line::from("Run the requested command"),
        Line::from("Delegate to a specialist agent"),
        Line::from("Allow access for this command"),
        Line::from("Allow access for this session"),
        Line::from("Resume the interrupted task"),
        Line::from("Start a fresh task"),
    ];
    let mut state = SelectState::new();
    state.select(Some(3));

    element(
        SelectionScreen::new("Choose how to continue", rows, &state)
            .leading_rule()
            .trailing_rule()
            .footer(KeyHints::new([
                ("↑/↓", "move"),
                ("enter", "choose"),
                ("esc", "cancel"),
            ])),
    )
}

fn scene_flow(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let labels = [
        "build",
        "test",
        "docs",
        "release-ready",
        "terminal UI",
        "Rust",
    ];
    let mut flow = tuika::components::Flow::new().gap(1);
    for label in labels {
        flow = flow.item(element(
            tuika::components::Boxed::new(element(Text::raw(label)))
                .border_color(theme.accent)
                .padding(Padding::symmetric(1, 0)),
        ));
    }
    element(flow)
}

fn scene_grid(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let mut grid = tuika::components::Grid::new(3).gap(1);
    for (label, value) in [
        ("jobs", "12"),
        ("passed", "12"),
        ("failed", "0"),
        ("time", "4.2s"),
        ("target", "all"),
        ("status", "ready"),
    ] {
        grid = grid.cell(element(
            tuika::components::Boxed::new(element(Text::new(vec![
                Line::from(Span::styled(label, theme.muted_style())),
                Line::from(Span::styled(value, theme.accent_style())),
            ])))
            .padding(Padding::symmetric(1, 0)),
        ));
    }
    element(grid)
}

fn scene_scroll(frame: u64, theme: &Theme) -> Element {
    let lines: Vec<Line<'static>> = (1..=24)
        .map(|i| {
            Line::from(Span::styled(
                format!("  line {i:>2} — content that overflows the viewport"),
                theme.text_style(),
            ))
        })
        .collect();
    let viewport_h = 8usize;
    let content_h = lines.len();
    let mut state = ScrollState::new();
    let reach = tuika::anim::ping_pong(frame, 200);
    let steps = (reach * (content_h.saturating_sub(viewport_h) as f32 / 3.0 + 1.0)) as u32;
    let down = Event::Mouse(Mouse::at(MouseKind::ScrollDown, 0, 0));
    for _ in 0..steps {
        let _ = state.handle(&down, content_h, viewport_h);
    }
    view! {
        col {
            fixed(8) { node(Scroll::new(lines, &state)) }
        }
    }
}

/// A transcript of *components* — bordered panels beside plain rows — scrolling
/// by row, so a panel taller than the remaining space clips at the edge instead
/// of snapping. This is what `Scroll` cannot do: its content is pre-wrapped
/// lines, so anything laid out has to be flattened by hand first.
fn scene_item_scroll(frame: u64, theme: &Theme) -> Element {
    let items: Vec<Element> = (1..=6)
        .map(|i| {
            if i % 2 == 0 {
                element(
                    Boxed::new(element(Text::new(vec![
                        Line::from(Span::styled(format!("panel {i}"), theme.text_style())),
                        Line::from(Span::styled("laid out, not drawn", theme.muted_style())),
                    ])))
                    .title(Line::from(Span::styled(" note ", theme.accent_style()))),
                )
            } else {
                element(Text::new(vec![Line::from(Span::styled(
                    format!("  row {i} — an ordinary line of text"),
                    theme.text_style(),
                ))]))
            }
        })
        .collect();
    let viewport_h = 8usize;
    let content_h = ItemScroll::measure_height(&items, 60, 1, true, &RenderCtx::new(theme));
    let mut state = ScrollState::new();
    let reach = tuika::anim::ping_pong(frame, 200);
    let steps = (reach * (content_h.saturating_sub(viewport_h) as f32 / 3.0 + 1.0)) as u32;
    let down = Event::Mouse(Mouse::at(MouseKind::ScrollDown, 0, 0));
    for _ in 0..steps {
        let _ = state.handle(&down, content_h, viewport_h);
    }
    view! {
        col {
            fixed(8) { node(ItemScroll::new(items, &state).gap(1)) }
        }
    }
}

fn scene_scrollbar(_frame: u64, theme: &Theme) -> Element {
    let window = VirtualWindow::new(100, 24, 38);
    let labels = Text::new(vec![
        Line::from(Span::styled("item 38", theme.text_style())),
        Line::from(Span::styled("   ⋮", theme.muted_style())),
        Line::from(Span::styled("item 61", theme.text_style())),
        Line::from(Span::styled("24 visible of 100", theme.muted_style())),
    ]);
    view! {
        col(gap = 1) {
            fixed(4) {
                row(gap = 2) {
                    fixed(1) { node(Scrollbar::vertical(window)) }
                    grow(1) { node(labels) }
                }
            }
            fixed(1) { node(Scrollbar::horizontal(window)) }
        }
    }
}

fn scene_primitives(frame: u64, theme: &Theme) -> Element {
    let mut scroll = ScrollState::default();
    let pan = (tuika::anim::ping_pong(frame, 160) * 14.0).round() as usize;
    scroll.set_x_offset(pan);
    let mut form_state = FormState::default();
    form_state.focus(((frame / 28) % 2) as usize);

    let canvas = DrawView::new(
        |area: Rect, surface: &mut tuika::Surface<'_>, ctx: &tuika::RenderCtx<'_>| {
            for y in area.y..area.bottom() {
                surface.set_string(
                    area.x,
                    y,
                    "界  custom grid  0123456789  →",
                    ctx.theme.info_style(),
                );
            }
        },
    )
    .intrinsic_size(Size::new(31, 2));
    let preview = Viewport::new(element(canvas), Size::new(31, 2), &scroll)
        .vertical_scrollbar(false)
        .horizontal_scrollbar(true);
    let form = Form::new(
        vec![
            FormField::new("Name", element(Text::raw("Ada"))).help("host-owned value"),
            FormField::new("Preview", element(preview)).error("semantic validation row"),
        ],
        &form_state,
    )
    .stack_below(36);
    let dialog = Dialog::new("Reusable primitives", element(form))
        .size(56, 10)
        .key_hints([("enter", "submit"), ("esc", "cancel")])
        .dim_backdrop(true)
        .focus_owner("primitives");
    element(
        Scene::new(element(Text::new(vec![Line::from(Span::styled(
            "base screen · overlay placement resolves at render time",
            theme.muted_style(),
        ))])))
        .dialog(dialog),
    )
}

fn scene_dialog_presets(frame: u64, theme: &Theme) -> Element {
    let phase = (frame / 48) % 4;
    let dialog: Dialog = match phase {
        0 => ConfirmDialog::new(
            "Apply changes?",
            "This will update three files in the workspace.",
            &ConfirmDialogState::new(),
        )
        .confirm_label("Apply")
        .into(),
        1 => {
            let mut state = ChoiceDialogState::new();
            let down = Event::Key(Key::new(KeyCode::Down));
            let _ = state.handle(&down, 3);
            ChoiceDialog::new(
                "Choose model",
                "Select the model for the next turn.",
                vec!["Fast".into(), "Balanced".into(), "Deep".into()],
                &state,
            )
            .into()
        }
        2 => {
            let mut state = MultiChoiceDialogState::new();
            let space = Event::Key(Key::new(KeyCode::Char(' ')));
            let _ = state.handle(&space, 3);
            MultiChoiceDialog::new(
                "Include context",
                "Choose the sources sent with the request.",
                vec![
                    "Current file".into(),
                    "Diagnostics".into(),
                    "Git diff".into(),
                ],
                &state,
            )
            .into()
        }
        _ => InputDialog::new(
            "Rename task",
            "Give this activity a concise name.",
            &InputDialogState::from_text("Review parser changes"),
        )
        .placeholder("Task name")
        .into(),
    };
    element(
        Scene::new(element(Text::new(vec![Line::from(Span::styled(
            "Agent workspace · host content remains independent",
            theme.muted_style(),
        ))])))
        .dialog(dialog),
    )
}

fn scene_select(frame: u64, theme: &Theme) -> Element {
    let items: Vec<Line<'static>> = ["Open file…", "Save", "Save As…", "Toggle theme", "Quit"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = SelectState::new();
    let target = (frame / 10) % items.len() as u64;
    let down = Event::Key(Key::new(KeyCode::Down));
    for _ in 0..target {
        let _ = state.handle(&down, items.len());
    }
    view! {
        col {
            grow(1) { node(SelectList::new(items, &state)) }
        }
    }
}

struct KeyedDemoRow {
    id: u64,
    name: &'static str,
    state: &'static str,
    age: u16,
}

static KEYED_ROWS: [KeyedDemoRow; 6] = [
    KeyedDemoRow {
        id: 101,
        name: "index workspace",
        state: "running",
        age: 4,
    },
    KeyedDemoRow {
        id: 102,
        name: "review patch",
        state: "queued",
        age: 12,
    },
    KeyedDemoRow {
        id: 103,
        name: "run checks",
        state: "running",
        age: 19,
    },
    KeyedDemoRow {
        id: 104,
        name: "refresh graph",
        state: "queued",
        age: 27,
    },
    KeyedDemoRow {
        id: 105,
        name: "stream logs",
        state: "running",
        age: 34,
    },
    KeyedDemoRow {
        id: 106,
        name: "publish report",
        state: "queued",
        age: 41,
    },
];

static KEYED_REORDERED: [usize; 6] = [4, 0, 3, 2, 5, 1];
static KEYED_ORIGINAL: [usize; 6] = [0, 1, 2, 3, 4, 5];
static KEYED_FILTERED: [usize; 2] = [0, 4];
static KEYED_SELECTION: KeyedSelectState<u64> = KeyedSelectState::with_selected(103);

struct KeyedDemoRows {
    visible: &'static [usize],
}

impl KeyedRowSource<u64> for KeyedDemoRows {
    type Row = KeyedDemoRow;

    fn len(&self) -> usize {
        self.visible.len()
    }

    fn row(&self, index: usize) -> Option<&Self::Row> {
        self.visible
            .get(index)
            .and_then(|&index| KEYED_ROWS.get(index))
    }

    fn key_eq(&self, _index: usize, row: &Self::Row, key: &u64) -> bool {
        row.id == *key
    }
}

fn keyed_demo_name(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(row.name)
}

fn keyed_demo_state(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(row.state)
}

fn keyed_demo_age(row: &KeyedDemoRow) -> Line<'_> {
    Line::from(format!("{}s", row.age))
}

fn scene_keyed_table(frame: u64, _theme: &Theme) -> Element {
    let phase = (frame / 18) % 3;
    let visible: &'static [usize] = match phase {
        0 => &KEYED_REORDERED,
        1 => &KEYED_FILTERED,
        _ => &KEYED_ORIGINAL,
    };
    let label = match phase {
        0 => "reordered · key 103 stays selected",
        1 => "filtered out · key 103 remains in host state",
        _ => "stream refreshed · key 103 restored",
    };
    view! {
        col {
            fixed(1) { node(Text::raw(label)) }
            grow(1) {
                node(KeyedTable::source(
                    vec![
                        KeyedColumn::fixed("id", 5, |row: &KeyedDemoRow| Line::from(row.id.to_string())).right(),
                        KeyedColumn::flex("task", 2, keyed_demo_name),
                        KeyedColumn::fixed("state", 8, keyed_demo_state).hide_below(34),
                        KeyedColumn::fixed("age", 5, keyed_demo_age).right().optional(),
                    ],
                    KeyedDemoRows { visible },
                    &KEYED_SELECTION,
                ))
            }
        }
    }
}

fn scene_completion_palette(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let items = vec![
        CompletionItem::new("status")
            .detail("Show session status")
            .replacement("/status"),
        CompletionItem::new("model")
            .detail("Choose a model")
            .replacement("/model")
            .keyword("engine"),
        CompletionItem::new("approvals")
            .detail("Configure command approvals")
            .replacement("/approvals"),
        CompletionItem::new("review")
            .detail("Review current changes")
            .replacement("/review"),
        CompletionItem::new("init")
            .detail("Create project instructions")
            .replacement("/init"),
    ];
    let queries = ["", "m", "mo", "mod"];
    let query = queries[((frame / 24) as usize) % queries.len()];
    let mut state = CompletionState::new();
    state.sync(query, &items);
    element(
        CompletionPalette::new(&items, &state)
            .title("Commands")
            .viewport(5)
            .show_query(true),
    )
}

fn scene_tabs(frame: u64, theme: &Theme) -> Element {
    let labels: Vec<Line<'static>> = ["Chat", "Diff", "Logs", "Files"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = TabsState::default();
    let target = (frame / 16) % labels.len() as u64;
    let right = Event::Key(Key::new(KeyCode::Right));
    for _ in 0..target {
        let _ = state.handle(&right, labels.len());
    }
    view! {
        col(gap = 1) {
            fixed(1) { node(Tabs::new(labels, &state)) }
            fixed(1) { node(Text::new(vec![Line::from(Span::styled("←/→ or Tab to switch", theme.muted_style()))])) }
        }
    }
}

fn demo_keymap() -> Keymap<&'static str> {
    Keymap::new()
        .layer(
            Layer::new("global")
                .bind_labeled("?", "open help", "help")
                .bind_labeled("q", "quit", "quit"),
        )
        .layer(
            Layer::new("search")
                .priority(10)
                .bind_labeled("enter", "open result", "open")
                .bind_labeled("ctrl+n", "next result", "next")
                .bind_labeled("ctrl+p", "previous result", "previous"),
        )
}

fn scene_key_hints(_frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            node(Text::new(vec![Line::from(Span::styled("Full width", theme.accent_style()))]))
            node(KeyHints::from_keymap(&demo_keymap()))
            row(gap = 2) {
                fixed(12) { node(Text::new(vec![Line::from(Span::styled("28 columns", theme.muted_style()))])) }
                fixed(28) { node(KeyHints::from_keymap(&demo_keymap())) }
            }
        }
    }
}

fn scene_keymap_help(_frame: u64, theme: &Theme) -> Element {
    view! {
        col(gap = 1) {
            node(Text::new(vec![Line::from(Span::styled("Active bindings", theme.accent_style()))]))
            node(KeymapHelp::from_keymap(&demo_keymap()))
        }
    }
}

fn scene_status_bar(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let bar = StatusBar::new()
        .left(vec![
            Span::styled(" NORMAL ", theme.selection_style()),
            Span::styled("  main.rs", theme.text_style()),
        ])
        .right(vec![
            Span::styled("utf-8  ", theme.muted_style()),
            Span::styled("Ln 42, Col 7 ", theme.text_style()),
        ])
        .background(Style::default().bg(theme.surface));
    view! {
        col {
            fixed(1) { node(bar) }
        }
    }
}

/// Typing into a composer: the placeholder while it is empty, and a `@` token
/// colored as it is typed. The trigger table is the host's — tuika finds the
/// token and paints the range it is handed.
fn scene_textinput(frame: u64, theme: &Theme) -> Element {
    let full = "fix @src/parser.rs: trailing commas";
    let n = ((frame / 3) as usize % (full.chars().count() + 12)).min(full.chars().count());
    let typed: String = full.chars().take(n).collect();
    let state = TextInputState::from_text(&typed);
    let mention = Style::default()
        .fg(theme.code.link)
        .add_modifier(Modifier::BOLD);
    let highlights: Vec<TextSpan> = state
        .tokens(&[Trigger::new('@')])
        .iter()
        .map(|t| t.span(mention))
        .collect();
    view! {
        col {
            fixed(3) {
                boxed(title = Line::from(Span::styled(" commit message ", theme.accent_style()))) {
                    node(
                        TextInput::new(&state)
                            .placeholder("describe the change", Style::default().fg(theme.dim))
                            .highlights(highlights)
                    )
                }
            }
        }
    }
}

/// Bare `http(s)` URLs the host wraps in OSC 8, and a markdown link whose label
/// carries the target. Rendered with the theme's link color + underline — the
/// look a supporting terminal makes clickable; others show the text unchanged.
/// The normal paint path draws styled cells; real OSC 8 emission is the job of
/// `HyperlinkBackend` / `write_line`, so this scene shows the *appearance*.
fn scene_hyperlink(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let link = Style::default()
        .fg(theme.code.link)
        .add_modifier(Modifier::UNDERLINED);
    let body = Text::new(vec![
        Line::from(Span::styled(
            "A bare URL is wrapped in place — clickable, text unchanged:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  see ", theme.text_style()),
            Span::styled("https://docs.rs/tuika", link),
            Span::styled(" for the API.", theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "A markdown link shows its label, hiding the target:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  the ", theme.text_style()),
            Span::styled("tuika component gallery", link),
            Span::styled(" demos every widget.", theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Only http(s) links are emitted; anything else stays plain text.",
            theme.muted_style(),
        )),
    ]);
    element(body)
}

/// A left-drag selection growing over a phrase (real, copyable text), plus a
/// row of clickable regions a `HitMap` would resolve to actions.
fn scene_mouse(frame: u64, theme: &Theme) -> Element {
    let phrase = "the quick brown fox jumps over the lazy dog";
    let count = phrase.chars().count();
    let reach = tuika::anim::ping_pong(frame, 200);
    let selected = (reach * count as f32).round() as usize;
    let sel: String = phrase.chars().take(selected).collect();
    let rest: String = phrase.chars().skip(selected).collect();

    let button = |label: &str, active: bool| -> Span<'static> {
        if active {
            Span::styled(
                format!(" {label} "),
                Style::default()
                    .fg(theme.background)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!(" {label} "), theme.muted_style())
        }
    };
    let hot = (frame / 24) % 3;

    let body = Text::new(vec![
        Line::from(Span::styled(
            "Left-drag selects real text — copy it over SSH via OSC 52:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  ", theme.text_style()),
            Span::styled(sel, theme.selection_style()),
            Span::styled(rest, theme.text_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "A HitMap maps screen regions to values — clicks become actions:",
            theme.muted_style(),
        )),
        Line::from(vec![
            Span::styled("  ", theme.text_style()),
            button("Run", hot == 0),
            Span::styled("  ", theme.text_style()),
            button("Diff", hot == 1),
            Span::styled("  ", theme.text_style()),
            button("Cancel", hot == 2),
        ]),
    ]);
    element(body)
}

fn scene_overlay(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    use tuika::overlay::Extent;
    use tuika::probe::RectProbe;

    let target = RectProbe::new();
    let base = |s: &str| {
        Text::new(vec![Line::from(Span::styled(
            s.to_string(),
            theme.muted_style(),
        ))])
    };
    let trigger = target.wrap(Text::new(vec![Line::from(Span::styled(
        "[ Open actions ▾ ]",
        theme.accent_style(),
    ))]));
    let root: Element = view! {
        col(gap = 0) {
            fixed(1) { node(base("base layer stays independently laid out")) }
            fixed(1) { node(base("the popover follows its trigger after layout")) }
            grow(1) { spacer() }
            fixed(1) {
                row {
                    grow(1) { spacer() }
                    fixed(20) { node(trigger) }
                    fixed(2) { spacer() }
                }
            }
            fixed(1) { node(base("preferred below · flipped above at the edge")) }
        }
    };
    let popover = view! {
        boxed(
            title = Line::from(Span::styled(" actions ", theme.accent_style())),
            border = BorderStyle::Rounded,
            padding = Padding::all(1)
        ) {
            col(gap = 1) {
                node(Text::new(vec![Line::from(Span::styled(
                    "Run command",
                    theme.text_style(),
                ))]))
                node(Text::new(vec![Line::from(Span::styled(
                    "Inspect logs",
                    theme.muted_style(),
                ))]))
            }
        }
    };
    let spec = OverlaySpec {
        width: Extent::Cells(28),
        height: Extent::Cells(7),
        ..OverlaySpec::centered(0, 0).margin(1)
    };
    element(
        Scene::new(root).overlay(SceneOverlay::new(popover, spec).target(
            &target,
            TargetPlacement::below().align(TargetAlign::End).gap(1),
        )),
    )
}

/// A muted caption line, reused by several scenes below.
fn caption(text: &str, theme: &Theme) -> Element {
    element(Text::new(vec![Line::from(Span::styled(
        text.to_string(),
        theme.muted_style(),
    ))]))
}

fn scene_diff(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    let old = "fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n\nlet total = add(2, 3);";
    let new = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n\nlet total = add(2, 3);\nprintln!(\"{total}\");";
    view! {
        col(gap = 1) {
            fixed(1) { node(caption("unified · red = removed, green = added", theme)) }
            grow(1) { node(Diff::new(old, new).line_numbers(true)) }
        }
    }
}

fn scene_ascii_font(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    view! {
        col(gap = 1) {
            fixed(5) { node(AsciiFont::new("TUIKA")) }
            fixed(1) { node(caption("embedded 5-row block font · A-Z 0-9 punctuation", theme)) }
        }
    }
}

fn scene_qr(frame: u64, theme: &Theme) -> Element {
    let _ = frame;
    // Byte-mode v1-4 encoder; a short URL fits comfortably in version 1-2.
    const PAYLOAD: &str = "https://everruns.com";
    let qr = QrCode::encode(PAYLOAD, QrEcc::Medium)
        .map(element)
        .unwrap_or_else(|| caption("(payload too large)", theme));
    view! {
        col(gap = 1) {
            grow(1) { node(qr) }
            fixed(1) { node(caption(&format!("QrCode::encode(\"{PAYLOAD}\", QrEcc::Medium)"), theme)) }
        }
    }
}

fn scene_tab_select(frame: u64, theme: &Theme) -> Element {
    let labels: Vec<Line<'static>> = ["Low", "Medium", "High", "Ultra"]
        .iter()
        .map(|s| Line::from(Span::styled((*s).to_string(), theme.text_style())))
        .collect();
    let mut state = TabSelectState::default();
    let target = (frame / 18) % labels.len() as u64;
    let right = Event::Key(Key::new(KeyCode::Right));
    for _ in 0..target {
        let _ = state.handle(&right, labels.len());
    }
    view! {
        col(gap = 1) {
            fixed(1) { node(TabSelect::new(labels, &state)) }
            fixed(1) { node(caption("← → move the selection · enter/space activates", theme)) }
        }
    }
}

fn scene_slider(frame: u64, theme: &Theme) -> Element {
    let swept = tuika::anim::ping_pong(frame, 160);
    let mut volume = SliderState::new(0.0, 100.0, 0.0);
    volume.set_ratio(swept);
    let contrast = SliderState::new(0.0, 10.0, 6.0);
    let mut balance = SliderState::new(0.0, 1.0, 0.0);
    balance.set_ratio(1.0 - swept);

    let row = |label: &str, s: &SliderState, theme: &Theme| -> Element {
        let lbl = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            theme.muted_style(),
        ))]);
        let slider = Slider::new(s).label(s);
        view! {
            row(gap = 1) {
                fixed(9) { node(lbl) }
                grow(1) { node(slider) }
            }
        }
    };
    view! {
        col(gap = 1) {
            fixed(1) { node(row("volume", &volume, theme)) }
            fixed(1) { node(row("contrast", &contrast, theme)) }
            fixed(1) { node(row("balance", &balance, theme)) }
        }
    }
}

fn scene_timeline(frame: u64, theme: &Theme) -> Element {
    // Three tracks over a shared 120-frame loop: a linear ramp, an eased ramp,
    // and a 0→1→0 pulse — each a pure function of the frame counter.
    let linear = Timeline::new()
        .keyframe(0, 0.0)
        .keyframe(120, 1.0)
        .repeat(Repeat::Loop);
    let eased = Timeline::new()
        .keyframe(0, 0.0)
        .ease(120, 1.0, tuika::anim::ease_in_out)
        .repeat(Repeat::Loop);
    let pulse = Timeline::new()
        .keyframe(0, 0.0)
        .keyframe(60, 1.0)
        .keyframe(120, 0.0)
        .repeat(Repeat::Loop);

    let track = |label: &str, value: f32, theme: &Theme| -> Element {
        let lbl = Text::new(vec![Line::from(Span::styled(
            label.to_string(),
            theme.muted_style(),
        ))]);
        view! {
            row(gap = 1) {
                fixed(11) { node(lbl) }
                grow(1) { node(ProgressBar::determinate(value).percent(true)) }
            }
        }
    };
    view! {
        col(gap = 1) {
            fixed(1) { node(track("linear", linear.sample(frame), theme)) }
            fixed(1) { node(track("ease_in_out", eased.sample(frame), theme)) }
            fixed(1) { node(track("pulse", pulse.sample(frame), theme)) }
            fixed(1) { node(caption("Timeline::new().keyframe(..).ease(..).repeat(Loop)", theme)) }
        }
    }
}

fn scene_toast(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    // A representative stack, newest on top. The host ticks these down; here we
    // show the steady-state look of all four levels at once.
    let mut toasts = Toasts::new(4);
    toasts.push(ToastLevel::Info, "Build started");
    toasts.push(ToastLevel::Success, "42 tests passed");
    toasts.push(ToastLevel::Warning, "2 unused imports");
    toasts.push(ToastLevel::Error, "Deploy to prod failed");
    let toasts: &'static Toasts = Box::leak(Box::new(toasts));
    view! {
        col {
            grow(1) { node(ToastList::new(toasts)) }
        }
    }
}

fn scene_console(frame: u64, theme: &Theme) -> Element {
    let _ = (frame, theme);
    let log = ConsoleLog::new(50);
    for line in [
        "INFO  server listening on 127.0.0.1:8080",
        "DEBUG loaded 12 routes in 3ms",
        "INFO  GET /  200  1.2ms",
        "WARN  slow query: users.by_email  214ms",
        "INFO  GET /health  200  0.3ms",
        "ERROR upstream timeout after 5s",
        "INFO  reconnecting to upstream…",
    ] {
        log.line(line);
    }
    let log: &'static ConsoleLog = Box::leak(Box::new(log));
    view! {
        col {
            grow(1) { node(Console::new(log).title(" console ")) }
        }
    }
}

fn scene_framebuffer(frame: u64, theme: &Theme) -> Element {
    let _ = theme;
    let (w, h) = (56u32, 22u32);
    let mut fb = FrameBuffer::new(w, h);
    // A diagonal gradient background.
    fb.shade(|x, y, _| {
        let r = 30 + (x * 120 / w) as u8;
        let g = 20;
        let b = 60 + (y * 120 / h) as u8;
        [r, g, b, 255]
    });
    // A "ball" bouncing over the canvas, drawn as an opaque square sprite.
    let bx = (tuika::anim::ping_pong(frame, 120) * (w - 8) as f32) as u32;
    let by = (tuika::anim::ping_pong(frame.wrapping_add(37), 84) * (h - 8) as f32) as u32;
    fb.fill_rect(bx, by, 8, 8, [240, 210, 90, 255]);
    // A scanline shader post-pass darkens every other row.
    fb.shade(|_x, y, [r, g, b, a]| {
        if y % 2 == 0 {
            [r, g, b, a]
        } else {
            [r / 2, g / 2, b / 2, a]
        }
    });
    let fb: &'static FrameBuffer = Box::leak(Box::new(fb));
    view! {
        col(gap = 1) {
            grow(1) { node(FrameBufferView::new(fb, w as u16, (h / 2) as u16)) }
            fixed(1) { node(caption("FrameBuffer → half-block cells · sprite + scanline shader", theme)) }
        }
    }
}
