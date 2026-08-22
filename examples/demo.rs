//! Single-component demo harness.
//!
//! It renders one component per scene inside a small labeled frame, and is the
//! source of truth for the gallery: the scene registry drives the CLI, the tape
//! generator, and the integrity check.
//!
//! ```text
//! cargo run --example demo                    # spinner in a real terminal
//! cargo run --example demo -- spinner          # interactive, records a GIF
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
use std::io::{self, Read};
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

#[path = "demo_scenes/mod.rs"]
mod demo_scenes;
mod support;
use demo_scenes::*;

/// A tiny self-contained Rust highlighter for the `markdown`/`code_block` scenes.
///
/// The examples deliberately avoid a grammar dependency (that would drag
/// tree-sitter into tuika's dev/MSRV builds); this shows how little it takes to
/// satisfy the [`Highlighter`] boundary. For production-grade highlighting across
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
    filling_demo(
        "tree_list",
        "TreeList",
        "stable selection, expansion, and persistent scrolling",
        12,
        true,
        scene_tree_list,
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
    let cli = support::Cli::parse()?;
    let args = cli.args;
    let cmd = args.first().map(String::as_str).unwrap_or("spinner");

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
                dump(d, &cli.theme)
            } else {
                run(d, &cli.theme)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// `tapes` — emit a VHS tape per scene from the registry, so the tapes are
// generated rather than committed. Recorded at exactly 2× the `width="880"`
// display size, so the assets stay crisp on HiDPI screens. Motion scenes
// record as GIF; settled scenes use a full-color PNG screenshot. `Output` is
// relative to the tuika crate dir (the generator cds there); the command is an
// absolute path to this very binary.
// ---------------------------------------------------------------------------

/// Recorded scene width in columns. Every scene records at the same width so the
/// GIFs line up in the gallery; only the height varies, per `Demo::rows`.
const RECORD_COLS: u16 = 66;
/// Every gallery embed is displayed at 880 px. Keeping the capture at an exact
/// integer multiple avoids browser resampling across fractional source pixels.
const RECORD_WIDTH: u32 = 1_760;
/// Documentation uses one stable palette so assets from different generators
/// read as one gallery. The theme comparison gallery is the sole local exception.
const RECORD_THEME: &str = "solarized-dark";
const RECORD_BACKGROUND: &str = "#002b36";
const RECORD_FOREGROUND: &str = "#839496";
/// Upper bounds on the cell size, in pixels, that VHS's default monospace at
/// `Set FontSize 40` yields in `xterm.js` on the reference capture host. A tape
/// is sized in *pixels* and the emulator divides by its own cell size, so these
/// are deliberately rounded up: the terminal is then never smaller than
/// `RECORD_COLS × rows`, and the leftover row or column is absorbed by the scene
/// pinning in [`run`].
const CELL_W: u32 = 26;
const CELL_H: u32 = 55;
/// The tape's `Set Padding`, applied on every side. This leaves exactly 66
/// upper-bound cell widths inside the 1760 px recording.
const PADDING: u32 = (RECORD_WIDTH - RECORD_COLS as u32 * CELL_W) / 2;

fn emit_tapes(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let bin = std::env::current_exe()?;
    let width = RECORD_WIDTH;
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
        let tape = tape_for(d, &bin, width, height, output, ending, fps);
        fs::write(dir.join(format!("{}.tape", d.name)), tape)?;
    }
    println!("wrote {} tapes to {}", DEMOS.len(), dir.display());
    Ok(())
}

fn tape_for(
    d: &Demo,
    bin: &Path,
    width: u32,
    height: u32,
    output: String,
    ending: String,
    fps: u8,
) -> String {
    format!(
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
             Set Theme {{ \"background\": \"{RECORD_BACKGROUND}\", \"foreground\": \"{RECORD_FOREGROUND}\" }}\n\
             Set Framerate {fps}\n\
             \n\
             Hide\n\
             Type \"{bin} {name} --theme {RECORD_THEME}\"\n\
             Enter\n\
             Sleep 900ms\n\
             Show\n\
             {ending}\n",
        name = d.name,
        bin = bin.display(),
    )
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

    check_capture_theme(&dir, &mut errors);

    // Every scene has a non-empty recording at the exact 2× display width. A
    // near miss still forces the browser to resample fractional source pixels,
    // which makes terminal glyphs visibly softer.
    for d in DEMOS {
        let asset = demos.join(d.asset_filename());
        match fs::metadata(&asset) {
            Ok(m) if m.len() > 0 => match image_dimensions(&asset) {
                Ok((width, _)) if width == RECORD_WIDTH => {}
                Ok((width, _)) => errors.push(format!(
                    "recording {} is {width}px wide; expected {RECORD_WIDTH}px for exact 2x display",
                    asset.display()
                )),
                Err(error) => errors.push(format!(
                    "could not read recording dimensions for {}: {error}",
                    asset.display()
                )),
            },
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
    for entry in fs::read_dir(dir.join("docs/components"))?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            sources.push(path);
        }
    }
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

fn check_capture_theme(dir: &Path, errors: &mut Vec<String>) {
    let palette = dir.join("scripts/demo-theme.sh");
    match fs::read_to_string(&palette) {
        Ok(text)
            if text.contains("TUIKA_DEMO_THEME=\"solarized-dark\"")
                && text.contains("TUIKA_DEMO_BACKGROUND=\"#002b36\"")
                && text.contains("TUIKA_DEMO_FOREGROUND=\"#839496\"") => {}
        Ok(_) => errors.push(format!(
            "{} must define the Solarized Dark documentation palette",
            palette.display()
        )),
        Err(error) => errors.push(format!("could not read {}: {error}", palette.display())),
    }

    for relative in [
        "scripts/gen-all-demos.sh",
        "scripts/gen-chart-demo.sh",
        "scripts/gen-codex-demo.sh",
        "scripts/gen-hero.sh",
        "scripts/gen-html-demo.sh",
        "scripts/gen-language-demo.sh",
        "scripts/gen-mermaid-demo.sh",
        "scripts/gen-split-footer-demo.sh",
        "scripts/gen-styling-demos.sh",
    ] {
        let path = dir.join(relative);
        match fs::read_to_string(&path) {
            Ok(text)
                if text.contains("source \"${repo_root}/scripts/demo-theme.sh\"")
                    && text.contains("TUIKA_DEMO_THEME") => {}
            Ok(_) => errors.push(format!(
                "{relative} must select the shared documentation capture theme"
            )),
            Err(error) => errors.push(format!("could not read {}: {error}", path.display())),
        }
    }

    let workbench = dir.join("scripts/gen-workbench-demo.sh");
    match fs::read_to_string(&workbench) {
        Ok(text)
            if text.contains(
                "Set Theme { \"background\": \"#161215\", \"foreground\": \"#deccc7\" }",
            ) && text.contains("Type \"TERM=xterm-256color ${bin}\"")
                && !text.contains("TUIKA_DEMO_THEME") => {}
        Ok(_) => errors.push(format!(
            "{} must record the Workbench example's original palette",
            workbench.display()
        )),
        Err(error) => errors.push(format!("could not read {}: {error}", workbench.display())),
    }
}

/// Dimensions from the fixed header shared by all generated gallery formats.
fn image_dimensions(path: &Path) -> io::Result<(u32, u32)> {
    let mut header = [0_u8; 24];
    fs::File::open(path)?.read_exact(&mut header)?;

    if &header[..6] == b"GIF87a" || &header[..6] == b"GIF89a" {
        return Ok((
            u16::from_le_bytes([header[6], header[7]]).into(),
            u16::from_le_bytes([header[8], header[9]]).into(),
        ));
    }
    if &header[..8] == b"\x89PNG\r\n\x1a\n" && &header[12..16] == b"IHDR" {
        return Ok((
            u32::from_be_bytes(header[16..20].try_into().expect("four-byte width")),
            u32::from_be_bytes(header[20..24].try_into().expect("four-byte height")),
        ));
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "expected a GIF or PNG header",
    ))
}

/// Lines `d` would paint with room to spare that a `rows`-tall frame never
/// shows — that is, the content a recording of that height silently loses.
fn lost_to_clipping(d: &Demo, rows: u16) -> Vec<String> {
    /// Extra rows granted to the reference render. Wide enough to reveal a
    /// caption or a couple of trailing lines, narrow enough that a scene sized
    /// against the terminal does not reflow into something unrecognizable.
    const SLACK: u16 = 8;

    let theme = tuika::themes::SOLARIZED_DARK;
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

    #[test]
    fn generated_tapes_select_the_documentation_theme() {
        let tape = tape_for(
            &DEMOS[0],
            Path::new("/tmp/demo"),
            RECORD_WIDTH,
            900,
            "Output docs/demos/example.gif".to_owned(),
            "Sleep 4s".to_owned(),
            24,
        );

        assert!(
            tape.contains("Set Theme { \"background\": \"#002b36\", \"foreground\": \"#839496\" }")
        );
        assert!(tape.contains("--theme solarized-dark"));
    }

    #[test]
    fn repository_generators_follow_the_capture_palette_policy() {
        let mut errors = Vec::new();
        check_capture_theme(Path::new(env!("CARGO_MANIFEST_DIR")), &mut errors);
        assert_eq!(errors, Vec::<String>::new());
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
fn dump(d: &Demo, theme: &Theme) -> io::Result<()> {
    let root = framed(d.title, d.blurb, (d.build)(24, theme), theme);
    let buffer = tuika::testing::render(root.as_ref(), RECORD_COLS, d.rows, theme);
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
fn run(d: &Demo, theme: &Theme) -> io::Result<()> {
    let _session = tuika::TerminalSession::enter()?;
    let mut terminal = Terminal::with_options(
        ratatui::backend::CrosstermBackend::new(io::stdout()),
        TerminalOptions {
            viewport: RatatuiViewport::Fullscreen,
        },
    )?;
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
            let root = framed(d.title, d.blurb, (d.build)(frame, theme), theme);
            paint(f.buffer_mut(), scene, theme, root.as_ref(), &[]);
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
