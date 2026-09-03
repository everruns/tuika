//! Picture the split-footer screen mode **without a recorder** — the offline
//! path to what `scripts/gen-split-footer-demo.sh` records under VHS for
//! `docs/split-footer.gif`.
//!
//! A split footer cannot be photographed from a `Buffer`: the interesting part
//! is the *whole terminal*, and tuika's buffer only ever holds the footer's own
//! rows. What is above them belongs to the terminal, so the picture has to come
//! from one.
//!
//! So this records a real terminal session with no VHS, ttyd, or ffmpeg
//! anywhere: it runs the built [`split_footer`](../split_footer.rs) example
//! under a pseudo-terminal — the same harness `tests/pty_smoke.rs` asserts
//! against, including answering the cursor-position query an inline viewport is
//! anchored with — samples the resulting cell grid through a reference terminal
//! (vt100), and serializes those frames to a self-contained animated SVG. The
//! example is the source of truth, so neither picture can drift from it.
//!
//! The session is started from a real shell, so the prompt line above the
//! footer is genuine terminal output, and the last frame is captured *after*
//! the example exits — that frame is the promise of the mode: the published
//! output is still there, the footer's rows are gone.
//!
//! Nothing here is committed: the doc asset is the VHS recording, and this
//! writes to `target/` unless pointed somewhere else. Reach for it when the
//! recording toolchain is unavailable or when a vector asset is wanted.
//!
//! ```text
//! cargo run --example split_footer_demo                         # real terminal
//! cargo run --example split_footer_demo -- out.svg              # write SVG
//! cargo run --example split_footer_demo -- --theme gruvbox-dark # themed terminal
//! ```

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use tuika::prelude::Theme;
use tuika::ui::Color;

mod support;

/// Recorded grid. Wide enough for the footer's key hints, tall enough to show
/// several published blocks plus the shell line that started the session.
const COLS: u16 = 64;
const ROWS: u16 = 15;

/// The worker in the example publishes every 700ms; sample in step with it so
/// every frame differs from the last by one published block.
const SAMPLE: Duration = Duration::from_millis(700);
const SAMPLES: usize = 5;
/// Let the footer paint (and pin itself) before the first sample.
const SETTLE: Duration = Duration::from_millis(500);

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let dump = cli.args.iter().any(|arg| arg == "--dump");
    let args = cli
        .args
        .iter()
        .filter(|arg| arg.as_str() != "--dump")
        .cloned()
        .collect::<Vec<_>>();
    if dump && !args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--dump cannot be combined with an output path",
        ));
    }
    let output = support::Output::parse(&args)?;
    if !dump && output == support::Output::Terminal {
        return run_interactive(cli.theme_name.as_deref());
    }

    let frames = record(&cli.theme, cli.theme_name.as_deref())?;
    if dump {
        // What the recorder saw as text, for checking the generated asset.
        for (i, frame) in frames.iter().enumerate() {
            println!("── frame {i} {}", "─".repeat(COLS as usize - 10));
            for row in frame.chunks(COLS as usize) {
                let line: String = row.iter().map(|cell| cell.glyph()).collect();
                println!("{}", line.trim_end());
            }
        }
        return Ok(());
    }
    let support::Output::Svg(out) = output else {
        unreachable!("non-dump terminal mode returned before recording")
    };
    let svg = render_svg(&frames, &cli.theme);
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&out, svg.as_bytes())?;
    println!(
        "wrote {} ({} frames, {:.1} KiB)",
        out.display(),
        frames.len(),
        svg.len() as f32 / 1024.0
    );
    Ok(())
}

fn run_interactive(theme_name: Option<&str>) -> io::Result<()> {
    let mut command = Command::new(example_bin()?);
    if let Some(name) = theme_name {
        command.args(["--theme", name]);
    }
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "split_footer exited with {status}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Recording
// ---------------------------------------------------------------------------

/// One captured screen: `COLS * ROWS` resolved cells, row-major.
type Frame = Vec<Paint>;

fn example_bin() -> io::Result<PathBuf> {
    let here = std::env::current_exe()?;
    let bin = here
        .parent()
        .map(|dir| dir.join("split_footer"))
        .unwrap_or_default();
    if !bin.is_file() {
        let status = Command::new(env!("CARGO"))
            .args(["build", "--example", "split_footer"])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .status()?;
        if !status.success() || !bin.is_file() {
            return Err(io::Error::other(format!(
                "could not build the `split_footer` example at {}",
                bin.display()
            )));
        }
    }
    Ok(bin)
}

fn record(theme: &Theme, theme_name: Option<&str>) -> io::Result<Vec<Frame>> {
    let bin = example_bin()?;
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(io::Error::other)?;

    // Through a shell, so the command line above the footer is real terminal
    // output rather than something painted into the picture afterwards.
    let mut cmd = CommandBuilder::new("/bin/sh");
    // The prompt's own color comes from the theme too, so the one line the
    // shell contributes does not sit outside the palette of everything below it.
    let prompt = match theme.accent_alt {
        Color::Rgb(r, g, b) => format!("\x1b[38;2;{r};{g};{b}m"),
        _ => String::new(),
    };
    let theme_suffix = theme_name
        .map(|name| format!(" --theme {name}"))
        .unwrap_or_default();
    cmd.args([
        "-c",
        &format!(
            "printf '{prompt}~/src/tuika\\033[0m $ cargo run --example split_footer{theme_suffix}\\n'; \
             exec \"$@\""
        ),
        "split-footer-demo",
    ]);
    cmd.arg(&bin);
    if let Some(name) = theme_name {
        cmd.args(["--theme", name]);
    }
    cmd.env("TERM", "xterm-256color");
    let mut child = pair.slave.spawn_command(cmd).map_err(io::Error::other)?;
    drop(pair.slave);

    let writer = Arc::new(Mutex::new(
        pair.master.take_writer().map_err(io::Error::other)?,
    ));
    // The reference terminal is shared: the reader thread feeds it every byte,
    // the recorder samples it, and cursor-position queries are answered from it
    // (ratatui refuses to start an inline viewport without a truthful reply).
    let parser = Arc::new(Mutex::new(vt100::Parser::new(ROWS, COLS, 0)));
    let reader = pair.master.try_clone_reader().map_err(io::Error::other)?;
    let reader_parser = Arc::clone(&parser);
    let reader_writer = Arc::clone(&writer);
    let reader_handle = thread::spawn(move || {
        let mut reader = reader;
        let mut chunk = [0u8; 8192];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            let bytes = &chunk[..n];
            let mut parser = reader_parser.lock().expect("lock parser");
            parser.process(bytes);
            if bytes.windows(4).any(|w| w == b"\x1b[6n") {
                let (row, col) = parser.screen().cursor_position();
                let mut writer = reader_writer.lock().expect("lock writer");
                let _ = write!(writer, "\x1b[{};{}R", row + 1, col + 1);
                let _ = writer.flush();
            }
        }
    });

    let sample = || -> Frame {
        let parser = parser.lock().expect("lock parser");
        capture(parser.screen(), theme)
    };

    thread::sleep(SETTLE);
    let mut frames = vec![sample()];
    for _ in 0..SAMPLES {
        thread::sleep(SAMPLE);
        frames.push(sample());
    }

    // Quit, then record what the user is left with: the footer's rows released,
    // the published scrollback still on screen.
    {
        let mut writer = writer.lock().expect("lock writer");
        writer.write_all(b"q")?;
        writer.flush()?;
    }
    let _ = child.wait();
    thread::sleep(Duration::from_millis(200));
    frames.push(sample());

    drop(pair.master);
    let _ = reader_handle.join();
    Ok(frames)
}

/// Resolve every cell of `screen` into what the SVG paints.
fn capture(screen: &vt100::Screen, theme: &Theme) -> Frame {
    let mut frame = Vec::with_capacity(COLS as usize * ROWS as usize);
    for row in 0..ROWS {
        for col in 0..COLS {
            frame.push(match screen.cell(row, col) {
                Some(cell) => Paint::of(cell, theme),
                None => Paint::blank(theme),
            });
        }
    }
    frame
}

// ---------------------------------------------------------------------------
// SVG serialization
//
// Each cell becomes an absolutely positioned glyph, so alignment never depends
// on the viewer's font metrics. Cells identical in every frame are written once
// into a static base layer; the rest go into one group per frame, and a CSS
// keyframe animation reveals exactly one group at a time — the same technique
// `examples/screenshot.rs` uses for the README hero.
// ---------------------------------------------------------------------------

const CELL_W: f32 = 9.6;
const CELL_H: f32 = 19.0;
const FONT_SIZE: f32 = 15.5;
const BASELINE: f32 = 14.2;
const SCREEN_MARGIN: f32 = 14.0;
const GRID_PAD: f32 = 12.0;
const TITLE_BAR: f32 = 40.0;

/// Derive the terminal defaults and window chrome from the selected theme so
/// the real-terminal and SVG paths are the same scene in the same palette.
fn term_bg(theme: &Theme) -> String {
    hex(theme.background)
}

fn term_fg(theme: &Theme) -> String {
    hex(theme.text)
}

/// The fully resolved appearance of one cell in one frame.
#[derive(Clone, PartialEq)]
struct Paint {
    sym: String,
    fg: String,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
}

impl Paint {
    /// The character this cell shows, for the text dump.
    fn glyph(&self) -> char {
        self.sym.chars().next().unwrap_or(' ')
    }

    /// Whether two cells can share one `<text>` element: same styling, and
    /// neither carries a background rect the other would have to repeat.
    fn same_run(&self, other: &Self) -> bool {
        self.fg == other.fg
            && self.bg == other.bg
            && self.bold == other.bold
            && self.dim == other.dim
            && self.italic == other.italic
            && self.underline == other.underline
    }

    fn blank(theme: &Theme) -> Self {
        Self {
            sym: String::new(),
            fg: term_fg(theme),
            bg: None,
            bold: false,
            dim: false,
            italic: false,
            underline: false,
        }
    }

    fn of(cell: &vt100::Cell, theme: &Theme) -> Self {
        let raw_fg = color_hex(cell.fgcolor());
        let raw_bg = color_hex(cell.bgcolor());
        // Reverse video swaps the pair, defaulting each side to the terminal's
        // own colors — otherwise a reversed cell with a default background
        // would paint invisible text.
        let (fg, bg) = if cell.inverse() {
            (
                raw_bg.unwrap_or_else(|| term_bg(theme)),
                Some(raw_fg.unwrap_or_else(|| term_fg(theme))),
            )
        } else {
            (raw_fg.unwrap_or_else(|| term_fg(theme)), raw_bg)
        };
        Self {
            sym: cell.contents().to_string(),
            fg,
            bg,
            bold: cell.bold(),
            dim: cell.dim(),
            italic: cell.italic(),
            underline: cell.underline(),
        }
    }
}

fn render_svg(frames: &[Frame], theme: &Theme) -> String {
    let cols = COLS as usize;
    let rows = ROWS as usize;
    let grid_w = COLS as f32 * CELL_W;
    let grid_h = ROWS as f32 * CELL_H;
    let screen_w = grid_w + 2.0 * GRID_PAD;
    let screen_h = grid_h + 2.0 * GRID_PAD;
    let width = screen_w + 2.0 * SCREEN_MARGIN;
    let height = screen_h + TITLE_BAR + SCREEN_MARGIN;
    let gx = SCREEN_MARGIN + GRID_PAD;
    let gy = TITLE_BAR + GRID_PAD;

    // A cell is *static* when it is identical in every frame; those are painted
    // once. The rest are painted per frame. Splitting each row into runs of
    // like-styled cells keeps the file to a few thousand elements instead of
    // one per cell per frame.
    let static_cell: Vec<bool> = (0..cols * rows)
        .map(|idx| frames.iter().all(|frame| frame[idx] == frames[0][idx]))
        .collect();
    let mut base = String::new();
    let mut anim: Vec<String> = vec![String::new(); frames.len()];
    for y in 0..rows {
        let cy = gy + y as f32 * CELL_H;
        emit_row(
            &mut base,
            &frames[0][y * cols..(y + 1) * cols],
            &static_cell[y * cols..(y + 1) * cols],
            true,
            gx,
            cy,
        );
        for (f, frame) in frames.iter().enumerate() {
            emit_row(
                &mut anim[f],
                &frame[y * cols..(y + 1) * cols],
                &static_cell[y * cols..(y + 1) * cols],
                false,
                gx,
                cy,
            );
        }
    }

    // One shared timeline; each group gets a disjoint window on it via a boxcar
    // keyframe plus a *positive* stagger, so the frames play in the order they
    // were recorded (a negative delay runs the sequence backwards, which for a
    // recording of output accumulating would show it disappearing). The last
    // frame — the terminal after the example exited — holds twice as long,
    // because it is the point.
    let per_frame = 1.0_f32;
    let dur = per_frame * (frames.len() as f32 + 1.0);
    let slot = 100.0 / (frames.len() as f32 + 1.0);
    let slot_end = slot + 0.02;
    let last_slot = 2.0 * slot;
    let last_slot_end = last_slot + 0.02;

    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" \
viewBox=\"0 0 {width:.0} {height:.0}\" font-family=\"ui-monospace,'SFMono-Regular',\
'Cascadia Code',Menlo,Consolas,'Liberation Mono',monospace\" role=\"img\" \
aria-label=\"A tuika split footer: a pinned status box on the last rows of a terminal \
while published output accumulates above it as ordinary scrollback, and remains after \
the program exits.\">\n"
    ));
    s.push_str("<style>\n");
    s.push_str(&format!(
        ".t{{font-size:{FONT_SIZE}px;dominant-baseline:alphabetic}}\n"
    ));
    s.push_str(&format!(
        ".fr{{opacity:0;animation:cyc {dur:.2}s steps(1,end) infinite}}\n"
    ));
    s.push_str(&format!(
        ".fl{{opacity:0;animation:cycl {dur:.2}s steps(1,end) infinite}}\n"
    ));
    s.push_str(&format!(
        "@keyframes cyc{{0%,{slot:.3}%{{opacity:1}}{slot_end:.3}%,100%{{opacity:0}}}}\n"
    ));
    s.push_str(&format!(
        "@keyframes cycl{{0%,{last_slot:.3}%{{opacity:1}}{last_slot_end:.3}%,100%{{opacity:0}}}}\n"
    ));
    s.push_str("</style>\n");

    // Window chrome.
    s.push_str(
        "<defs><filter id=\"sh\" x=\"-20%\" y=\"-20%\" width=\"140%\" height=\"140%\">\
<feDropShadow dx=\"0\" dy=\"10\" stdDeviation=\"16\" flood-color=\"#000\" flood-opacity=\"0.45\"/>\
</filter></defs>\n",
    );
    s.push_str(&format!(
        "<rect x=\"{m:.1}\" y=\"{m:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"12\" \
fill=\"{chrome}\" stroke=\"{stroke}\" stroke-width=\"1\" filter=\"url(#sh)\"/>\n",
        m = SCREEN_MARGIN / 2.0,
        w = width - SCREEN_MARGIN,
        h = height - SCREEN_MARGIN,
        chrome = hex(theme.surface),
        stroke = hex(theme.border),
    ));
    let ty = SCREEN_MARGIN / 2.0 + TITLE_BAR / 2.0;
    for (i, color) in ["#ff5f56", "#ffbd2e", "#27c93f"].iter().enumerate() {
        let dot_x = SCREEN_MARGIN + 6.0 + i as f32 * 20.0;
        s.push_str(&format!(
            "<circle cx=\"{dot_x:.1}\" cy=\"{ty:.1}\" r=\"6.5\" fill=\"{color}\"/>\n"
        ));
    }
    s.push_str(&format!(
        "<text x=\"{cx:.1}\" y=\"{cy:.1}\" text-anchor=\"middle\" font-size=\"13\" \
fill=\"{muted}\">tuika · split-footer screen mode</text>\n",
        cx = width / 2.0,
        cy = ty + 4.5,
        muted = hex(theme.muted),
    ));
    s.push_str(&format!(
        "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" rx=\"6\" \
fill=\"{bg}\"/>\n",
        bg = term_bg(theme),
        x = SCREEN_MARGIN,
        y = TITLE_BAR,
        w = screen_w,
        h = screen_h,
    ));
    s.push_str(&format!(
        "<clipPath id=\"scr\"><rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" \
height=\"{h:.1}\" rx=\"6\"/></clipPath>\n",
        x = SCREEN_MARGIN,
        y = TITLE_BAR,
        w = screen_w,
        h = screen_h,
    ));

    s.push_str("<g clip-path=\"url(#scr)\">\n");
    s.push_str("<g>");
    s.push_str(&base);
    s.push_str("</g>\n");
    let last = frames.len() - 1;
    for (f, group) in anim.iter().enumerate() {
        let delay = f as f32 * per_frame;
        let class = if f == last { "fl" } else { "fr" };
        s.push_str(&format!(
            "<g class=\"{class}\" style=\"animation-delay:{delay:.3}s\">"
        ));
        s.push_str(group);
        s.push_str("</g>\n");
    }
    s.push_str("</g>\n</svg>\n");
    s
}

/// Emit one row's cells, taking either the static ones or the animated ones,
/// as runs of like-styled cells.
fn emit_row(
    out: &mut String,
    row: &[Paint],
    static_cell: &[bool],
    want_static: bool,
    gx: f32,
    cy: f32,
) {
    let mut x = 0usize;
    while x < row.len() {
        if static_cell[x] != want_static {
            x += 1;
            continue;
        }
        let mut end = x + 1;
        while end < row.len() && static_cell[end] == want_static && row[end].same_run(&row[x]) {
            end += 1;
        }
        emit_run(out, &row[x..end], gx + x as f32 * CELL_W, cy);
        x = end;
    }
}

/// Emit one run of like-styled cells: a single background rect and a single
/// text element, both spanning the run.
fn emit_run(out: &mut String, run: &[Paint], cx: f32, cy: f32) {
    let p = &run[0];
    if let Some(bg) = &p.bg {
        // Slight overdraw removes hairline gaps between adjacent filled cells.
        out.push_str(&format!(
            "<rect x=\"{cx:.1}\" y=\"{cy:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" \
fill=\"{bg}\" shape-rendering=\"crispEdges\"/>",
            w = run.len() as f32 * CELL_W + 0.6,
            h = CELL_H + 0.6,
        ));
    }
    let text: String = run.iter().map(|cell| cell.glyph()).collect();
    if text.trim().is_empty() {
        return;
    }
    let mut style = format!("fill:{}", p.fg);
    if p.bold {
        style.push_str(";font-weight:700");
    }
    if p.dim {
        style.push_str(";opacity:0.6");
    }
    if p.italic {
        style.push_str(";font-style:italic");
    }
    if p.underline {
        style.push_str(";text-decoration:underline");
    }
    // `textLength` pins the run to the cell grid, so a viewer whose monospace
    // font has a different advance still lands every glyph in its own column.
    out.push_str(&format!(
        "<text class=\"t\" x=\"{x:.1}\" y=\"{y:.1}\" style=\"{style}\" \
textLength=\"{len:.1}\" lengthAdjust=\"spacingAndGlyphs\" xml:space=\"preserve\">{}</text>",
        xml_escape(text.trim_end()),
        x = cx,
        y = cy + BASELINE,
        len = text.trim_end().chars().count() as f32 * CELL_W,
    ));
}

#[allow(dead_code)]
fn emit_cell(out: &mut String, p: &Paint, cx: f32, cy: f32) {
    if let Some(bg) = &p.bg {
        // Slight overdraw removes hairline gaps between adjacent filled cells.
        out.push_str(&format!(
            "<rect x=\"{cx:.2}\" y=\"{cy:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
fill=\"{bg}\" shape-rendering=\"crispEdges\"/>",
            w = CELL_W + 0.6,
            h = CELL_H + 0.6,
        ));
    }
    let sym = p.sym.trim_end();
    if sym.is_empty() {
        return;
    }
    let mut style = format!("fill:{}", p.fg);
    if p.bold {
        style.push_str(";font-weight:700");
    }
    if p.dim {
        style.push_str(";opacity:0.6");
    }
    if p.italic {
        style.push_str(";font-style:italic");
    }
    if p.underline {
        style.push_str(";text-decoration:underline");
    }
    out.push_str(&format!(
        "<text class=\"t\" x=\"{x:.2}\" y=\"{y:.2}\" style=\"{style}\" \
xml:space=\"preserve\">{}</text>",
        xml_escape(sym),
        x = cx,
        y = cy + BASELINE,
    ));
}

/// xterm color index → hex. `Default` is `None`, so the cell keeps whatever the
/// terminal's own colors are (which is exactly what an unstyled published block
/// wants).
/// A `Theme` slot as an SVG color. The bundled themes are all truecolor; an
/// indexed or named slot falls back to the same table a cell's own color uses.
fn hex(color: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(i) => indexed_hex(i),
        // No bundled theme uses a named color; `currentColor` keeps a custom
        // one from silently painting the chrome black.
        _ => "currentColor".to_string(),
    }
}

fn color_hex(color: vt100::Color) -> Option<String> {
    match color {
        vt100::Color::Default => None,
        vt100::Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
        vt100::Color::Idx(i) => Some(indexed_hex(i)),
    }
}

fn indexed_hex(i: u8) -> String {
    const BASE: [&str; 16] = [
        "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
        "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
    ];
    match i {
        0..=15 => BASE[i as usize].to_string(),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let n = i - 16;
            let (r, g, b) = (n / 36, (n % 36) / 6, n % 6);
            format!(
                "#{:02x}{:02x}{:02x}",
                LEVELS[r as usize], LEVELS[g as usize], LEVELS[b as usize]
            )
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            format!("#{v:02x}{v:02x}{v:02x}")
        }
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
