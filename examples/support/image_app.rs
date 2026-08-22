use std::io;

use ratatui::style::Style;
use ratatui::text::Span;
use tuika::prelude::*;
use tuika::term::image::{ImageData, ImageSupport};

pub const ALT: &str = "a red/green gradient";

pub fn rgba(w: u32, h: u32) -> Vec<u8> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            rgba.extend_from_slice(&[r, g, 128, 255]);
        }
    }
    rgba
}

pub fn gradient(w: u32, h: u32) -> ImageData {
    ImageData::from_rgba(w, h, rgba(w, h)).expect("gradient is well-formed")
}

pub fn run(theme: &Theme) -> io::Result<()> {
    const COLS: u16 = 40;
    const ROWS: u16 = 20;

    let data = gradient(320, 320);
    let support = ImageSupport::detect();
    let theme = *theme;
    let mut state = ();

    Runner::new(RunnerConfig::default()).run(
        &theme,
        from_fn(
            &mut state,
            move |_state, _frame| {
                let image = Image::new(data.clone(), COLS, ROWS).alt(ALT);
                let status = match support {
                    ImageSupport::Kitty => " graphics: Kitty protocol detected ",
                    ImageSupport::ITerm2 => " graphics: iTerm2 protocol detected ",
                    ImageSupport::Sixel => " graphics: Sixel protocol detected ",
                    ImageSupport::None => " graphics: none — showing text fallback ",
                };
                let bar = StatusBar::new()
                    .left(vec![Span::styled(status, theme.selection_style())])
                    .right(vec![Span::styled(" q quit ", theme.muted_style())])
                    .background(Style::default().bg(theme.surface));
                view! {
                    col(padding = Padding::all(1), gap = 1) {
                        grow(1) { node(Spacer) }
                        fixed(ROWS) {
                            row {
                                grow(1) { node(Spacer) }
                                fixed(COLS) { node(image) }
                                grow(1) { node(Spacer) }
                            }
                        }
                        grow(1) { node(Spacer) }
                        fixed(1) { node(bar) }
                    }
                }
            },
            |_state, signal| match signal {
                Signal::Event(Event::Key(key))
                    if key.plain() && matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) =>
                {
                    UpdateResult::Exit
                }
                _ => UpdateResult::Clean,
            },
        ),
    )
}
