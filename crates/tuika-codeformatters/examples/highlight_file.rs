//! View a source file with tree-sitter syntax highlighting.
//!
//! ```text
//! cargo run -p tuika-codeformatters --example highlight_file
//! cargo run -p tuika-codeformatters --example highlight_file -- path/to/file.rs
//! ```

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tuika::prelude::*;
use tuika_codeformatters::TreeSitterHighlighter;

mod support;

static HIGHLIGHTER: TreeSitterHighlighter = TreeSitterHighlighter;

struct FileViewer {
    path: PathBuf,
    language: &'static str,
    lines: Vec<String>,
    scroll: ScrollState,
    viewport_rows: usize,
}

impl FileViewer {
    fn open(path: PathBuf) -> io::Result<Self> {
        let source = fs::read_to_string(&path)?;
        let (_, height) = crossterm::terminal::size().unwrap_or((80, 24));
        let mut scroll = ScrollState::new();
        scroll.jump_to_top();
        Ok(Self {
            language: language_for_path(&path),
            path,
            lines: source.split('\n').map(str::to_owned).collect(),
            scroll,
            viewport_rows: source_rows(height),
        })
    }

    fn move_by(&mut self, delta: isize) -> UpdateResult {
        let current = self.scroll.offset();
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current
                .saturating_add(delta as usize)
                .min(ScrollState::max_offset(
                    self.lines.len(),
                    self.viewport_rows,
                ))
        };
        self.scroll.set_offset(next);
        if next == current {
            UpdateResult::Clean
        } else {
            UpdateResult::Dirty
        }
    }
}

impl Application for FileViewer {
    fn update(&mut self, signal: Signal) -> UpdateResult {
        let Signal::Event(event) = signal else {
            return UpdateResult::Clean;
        };

        if let Event::Resize { height, .. } = event {
            self.viewport_rows = source_rows(height);
            self.scroll.clamp(self.lines.len(), self.viewport_rows);
            return UpdateResult::Dirty;
        }

        if let Event::Key(key) = &event
            && key.plain()
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return UpdateResult::Exit,
                KeyCode::Up | KeyCode::Char('k') => return self.move_by(-1),
                KeyCode::Down | KeyCode::Char('j') => return self.move_by(1),
                _ => {}
            }
        }

        if self
            .scroll
            .handle(&event, self.lines.len(), self.viewport_rows)
            == InputOutcome::Changed
        {
            UpdateResult::Dirty
        } else {
            UpdateResult::Clean
        }
    }

    fn view(&self, _frame: u64) -> ScopedElement<'_> {
        let start = self.scroll.offset();
        let end = start
            .saturating_add(self.viewport_rows)
            .min(self.lines.len());
        let source = self.lines[start..end].join("\n");
        let language = if self.language.is_empty() {
            "plain"
        } else {
            self.language
        };
        let header = format!("{language} · {}", self.path.display());
        let status = format!(
            "lines {}-{} of {} · drag select/copy · ↑/↓, j/k, PgUp/PgDn scroll · q quit",
            start.saturating_add(1).min(self.lines.len()),
            end,
            self.lines.len()
        );
        let code = CodeBlock::new(self.language, source)
            .highlighter(&HIGHLIGHTER)
            .label(false)
            .start_line(start + 1);

        view! {
            col(padding = Padding::all(1), gap = 1) {
                fixed(1) { node(Text::raw(header)) }
                grow(1) { node(code) }
                fixed(1) { node(Text::raw(status)) }
            }
        }
    }
}

fn source_rows(terminal_height: u16) -> usize {
    // Two padding rows, two gaps, and the header/footer remain outside the code.
    usize::from(terminal_height.saturating_sub(6).max(1))
}

fn language_for_path(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("rs") => "rust",
        Some("py") => "python",
        Some("ts" | "mts" | "cts") => "typescript",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("tsx") => "tsx",
        Some("jsx") => "jsx",
        Some("go") => "go",
        Some("java") => "java",
        Some("rb") => "ruby",
        Some("css") => "css",
        Some("html" | "htm") => "html",
        Some("cs") => "csharp",
        Some("php") => "php",
        Some("zig") => "zig",
        Some("scala") => "scala",
        Some("sql") => "sql",
        _ => "",
    }
}

fn input_path_from(args: impl IntoIterator<Item = OsString>) -> io::Result<PathBuf> {
    let mut args = args.into_iter();
    let Some(path) = args.next() else {
        return Err(invalid_usage(None));
    };
    if let Some(extra) = args.next() {
        return Err(invalid_usage(Some(extra)));
    }
    Ok(PathBuf::from(path))
}

fn invalid_usage(extra: Option<OsString>) -> io::Error {
    let detail = extra.map_or_else(String::new, |arg| {
        format!(" (unexpected argument: {})", arg.to_string_lossy())
    });
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("usage: highlight_file <path>{detail}"),
    )
}

fn main() -> io::Result<()> {
    let (theme, args) = support::theme_and_args()?;
    let path = if args.is_empty() {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs")
    } else {
        input_path_from(args.into_iter().map(OsString::from))?
    };
    let mut viewer = FileViewer::open(path)?;
    Runner::new(RunnerConfig::default()).run(&theme, &mut viewer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuika::testing::{grid, render};

    #[test]
    fn accepts_exactly_one_path() {
        assert_eq!(
            input_path_from([OsString::from("source.rs")]).unwrap(),
            PathBuf::from("source.rs")
        );
        assert_eq!(
            input_path_from([]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            input_path_from([OsString::from("one.rs"), OsString::from("two.rs")])
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn opens_detects_and_renders_a_real_source_file() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        let mut viewer = FileViewer::open(path).unwrap();
        viewer.viewport_rows = 6;

        assert_eq!(viewer.language, "rust");
        let view = viewer.view(0);
        let output = grid(&render(view.as_ref(), 80, 12, &Theme::default()));
        assert!(output.contains("rust · "));
        assert!(output.contains("Tree-sitter syntax highlighting"));
        assert!(output.contains("lines 1-6 of"));
    }

    #[test]
    fn unknown_extensions_render_as_plain_code() {
        assert_eq!(language_for_path(Path::new("notes.unknown")), "");
        assert_eq!(language_for_path(Path::new("main.tsx")), "tsx");
    }
}
