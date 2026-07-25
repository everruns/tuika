//! A browser host for tuika: the wasm ABI between the Rust cell buffer and the
//! JavaScript canvas renderer in `tuika.js`.
//!
//! There is no `wasm-bindgen` here, on purpose. The whole interface is a
//! handful of `extern "C"` functions plus one flat array the JavaScript side
//! reads straight out of linear memory, so the demo builds with nothing but
//! `cargo build --target wasm32-unknown-unknown` — no `wasm-pack`, no npm, no
//! generated glue. That also keeps the *cost* of the seam visible: one packed
//! `PackedCell` per grid cell per frame, and a byte blob of the distinct
//! symbols it references.
//!
//! Two things are worth noticing about what is *not* here:
//!
//! - No ANSI. A terminal host serializes the buffer into escape sequences and
//!   the emulator parses them back into cells; a canvas host skips the round
//!   trip and paints the cells it already has.
//! - No layout, no components, no input semantics. Those all live in `tuika`,
//!   unchanged and unaware, on the other side of `app.rs`.
//!
//! # Contract
//!
//! Call [`tk_start`] once, then per frame: [`tk_render`], read [`tk_cells`] /
//! [`tk_atlas`]. Input goes in through [`tk_key`], [`tk_mouse`], [`tk_paste`].
//! Pointers must be re-read after every call — growing the Rust heap detaches
//! JavaScript's view of `WebAssembly.Memory`.

mod app;
mod palette;

use std::cell::RefCell;
use std::collections::HashMap;

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::style::Modifier;

use tuika::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind};

/// One grid cell, as the JavaScript renderer sees it.
///
/// 16 bytes, `#[repr(C)]`, no padding — the JS side indexes it with a
/// `DataView` at a fixed stride. Colors arrive pre-resolved to RGB (see
/// [`palette`]) and reverse-video is already applied, so painting a cell is a
/// `fillRect` plus a `fillText` with no palette logic in JavaScript.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PackedCell {
    /// Byte offset of this cell's symbol in the atlas.
    symbol_offset: u32,
    /// Byte length of the symbol; `0` for a wide character's trailing cell,
    /// which the renderer fills but does not draw into.
    symbol_len: u16,
    /// Bit flags: 1 bold, 2 dim, 4 italic, 8 underlined, 16 struck through.
    flags: u16,
    /// Foreground `0x00RRGGBB`.
    fg: u32,
    /// Background `0x00RRGGBB`.
    bg: u32,
}

/// Everything the module owns between calls.
struct Host {
    app: app::App,
    buffer: Buffer,
    cells: Vec<PackedCell>,
    /// Concatenated distinct cell symbols for one frame, deduplicated so a
    /// screen of spaces costs one entry rather than one per cell.
    atlas: Vec<u8>,
    /// Text handed in by [`tk_paste`], awaiting the next event dispatch.
    incoming: Vec<u8>,
    cols: u16,
    rows: u16,
}

thread_local! {
    /// wasm32 modules are single-threaded, so one `RefCell` is the whole of the
    /// concurrency story.
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

/// Run `f` against the live host, returning `default` before [`tk_start`].
fn with_host<T>(default: T, f: impl FnOnce(&mut Host) -> T) -> T {
    HOST.with_borrow_mut(|slot| match slot.as_mut() {
        Some(host) => f(host),
        None => default,
    })
}

impl Host {
    fn new(cols: u16, rows: u16) -> Self {
        Self {
            app: app::App::default(),
            buffer: Buffer::empty(Rect::new(0, 0, cols, rows)),
            cells: Vec::new(),
            atlas: Vec::new(),
            incoming: Vec::new(),
            cols,
            rows,
        }
    }

    fn render(&mut self) {
        let area = Rect::new(0, 0, self.cols, self.rows);
        self.buffer.resize(area);
        self.buffer.reset();
        self.app.render(&mut self.buffer, area);
        self.pack(area);
    }

    /// Flatten the painted buffer into [`PackedCell`]s plus the symbol atlas.
    fn pack(&mut self, area: Rect) {
        self.cells.clear();
        self.cells
            .reserve(usize::from(area.width) * usize::from(area.height));
        self.atlas.clear();
        let mut interned: HashMap<String, (u32, u16)> = HashMap::new();

        let theme = self.app.theme();
        let default_fg = palette::rgb(theme.text, 0xff_ff_ff);
        let default_bg = palette::rgb(theme.background, 0x00_00_00);

        for y in 0..area.height {
            for x in 0..area.width {
                let cell = &self.buffer[(x, y)];
                let mods = cell.modifier;
                let mut fg = palette::rgb(cell.fg, default_fg);
                let mut bg = palette::rgb(cell.bg, default_bg);
                // Resolve reverse video here rather than in JavaScript: it is a
                // property of the cell, not of the paint call.
                if mods.contains(Modifier::REVERSED) {
                    std::mem::swap(&mut fg, &mut bg);
                }

                let symbol = cell.symbol();
                // A wide character's second cell carries an empty symbol; it
                // still needs its background painted, so it is packed with a
                // zero-length symbol rather than skipped.
                let (symbol_offset, symbol_len) = if symbol.is_empty() || symbol == " " {
                    (0, 0)
                } else if let Some(&hit) = interned.get(symbol) {
                    hit
                } else {
                    let entry = (
                        u32::try_from(self.atlas.len()).unwrap_or(0),
                        u16::try_from(symbol.len()).unwrap_or(0),
                    );
                    self.atlas.extend_from_slice(symbol.as_bytes());
                    interned.insert(symbol.to_owned(), entry);
                    entry
                };

                let mut flags = 0u16;
                for (modifier, bit) in [
                    (Modifier::BOLD, 1),
                    (Modifier::DIM, 2),
                    (Modifier::ITALIC, 4),
                    (Modifier::UNDERLINED, 8),
                    (Modifier::CROSSED_OUT, 16),
                ] {
                    if mods.contains(modifier) {
                        flags |= bit;
                    }
                }

                self.cells.push(PackedCell {
                    symbol_offset,
                    symbol_len,
                    flags,
                    fg,
                    bg,
                });
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

/// Create the application at `cols` × `rows` cells. Calling it again restarts
/// from a fresh state.
#[unsafe(no_mangle)]
pub extern "C" fn tk_start(cols: u32, rows: u32) {
    let (cols, rows) = clamp_grid(cols, rows);
    HOST.with_borrow_mut(|slot| *slot = Some(Host::new(cols, rows)));
}

/// Resize the grid. This is the browser's `SIGWINCH`: the next frame is laid
/// out against the new size and prose re-wraps, with no other host involvement.
#[unsafe(no_mangle)]
pub extern "C" fn tk_resize(cols: u32, rows: u32) {
    let (cols, rows) = clamp_grid(cols, rows);
    with_host((), |host| {
        if (host.cols, host.rows) == (cols, rows) {
            return;
        }
        host.cols = cols;
        host.rows = rows;
        host.app.handle(&Event::Resize {
            width: cols,
            height: rows,
        });
    });
}

/// Composite one frame. Returns the number of packed cells now readable at
/// [`tk_cells`].
#[unsafe(no_mangle)]
pub extern "C" fn tk_render() -> u32 {
    with_host(0, |host| {
        host.render();
        u32::try_from(host.cells.len()).unwrap_or(0)
    })
}

/// Whether application state changed since the last frame. The demo animates
/// continuously, so this is advisory — a static screen would use it to idle.
#[unsafe(no_mangle)]
pub extern "C" fn tk_dirty() -> u32 {
    with_host(0, |host| u32::from(host.app.dirty()))
}

/// Pointer to the packed cell array (`cols * rows` × 16 bytes).
#[unsafe(no_mangle)]
pub extern "C" fn tk_cells() -> *const u8 {
    with_host(std::ptr::null(), |host| host.cells.as_ptr().cast())
}

/// Pointer to this frame's symbol atlas.
#[unsafe(no_mangle)]
pub extern "C" fn tk_atlas() -> *const u8 {
    with_host(std::ptr::null(), |host| host.atlas.as_ptr())
}

/// Byte length of this frame's symbol atlas.
#[unsafe(no_mangle)]
pub extern "C" fn tk_atlas_len() -> u32 {
    with_host(0, |host| u32::try_from(host.atlas.len()).unwrap_or(0))
}

/// The theme background as `0x00RRGGBB`, so the canvas margin outside the whole
/// cell grid matches the screen tuika paints.
#[unsafe(no_mangle)]
pub extern "C" fn tk_background() -> u32 {
    with_host(0, |host| {
        palette::rgb(host.app.theme().background, 0x00_00_00)
    })
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

/// Feed a key. `code` is the [`KeyCode`] ordinal below, `ch` the Unicode scalar
/// when `code` is `0` (`Char`), `mods` a bit set (1 ctrl, 2 alt, 4 shift,
/// 8 super). Returns `1` when the application wants a redraw.
///
/// This is the browser's [`translate_event`](tuika::translate_event): the same
/// job crossterm's translation does, against a different input vocabulary.
#[unsafe(no_mangle)]
pub extern "C" fn tk_key(code: u32, ch: u32, mods: u32) -> u32 {
    let Some(code) = key_code(code, ch) else {
        return 0;
    };
    let key = Key {
        code,
        ctrl: mods & 1 != 0,
        alt: mods & 2 != 0,
        shift: mods & 4 != 0,
    };
    with_host(0, |host| u32::from(host.app.handle(&Event::Key(key))))
}

/// Feed a mouse event. `kind`: 0 down, 1 up, 2 drag, 3 moved, 4 scroll up,
/// 5 scroll down, 6 scroll left, 7 scroll right. `button`: 0 left, 1 right,
/// 2 middle. `col`/`row` are grid cells, not pixels — the host quantizes.
#[unsafe(no_mangle)]
pub extern "C" fn tk_mouse(kind: u32, button: u32, col: u32, row: u32, mods: u32) -> u32 {
    let button = match button {
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        _ => MouseButton::Left,
    };
    let kind = match kind {
        0 => MouseKind::Down(button),
        1 => MouseKind::Up(button),
        2 => MouseKind::Drag(button),
        3 => MouseKind::Moved,
        4 => MouseKind::ScrollUp,
        5 => MouseKind::ScrollDown,
        6 => MouseKind::ScrollLeft,
        7 => MouseKind::ScrollRight,
        _ => return 0,
    };
    let mouse = Mouse {
        kind,
        column: u16::try_from(col).unwrap_or(u16::MAX),
        row: u16::try_from(row).unwrap_or(u16::MAX),
        shift: mods & 4 != 0,
        ctrl: mods & 1 != 0,
        alt: mods & 2 != 0,
        super_key: mods & 8 != 0,
    };
    with_host(0, |host| u32::from(host.app.handle(&Event::Mouse(mouse))))
}

/// Reserve `len` bytes for JavaScript to write UTF-8 into, then hand back to
/// [`tk_paste`]. The buffer belongs to the module; there is nothing to free.
#[unsafe(no_mangle)]
pub extern "C" fn tk_paste_buffer(len: u32) -> *mut u8 {
    with_host(std::ptr::null_mut(), |host| {
        host.incoming.clear();
        host.incoming.resize(len as usize, 0);
        host.incoming.as_mut_ptr()
    })
}

/// Deliver the bytes previously written into [`tk_paste_buffer`] as a paste
/// event. Invalid UTF-8 is dropped rather than lossily substituted.
#[unsafe(no_mangle)]
pub extern "C" fn tk_paste() -> u32 {
    with_host(0, |host| {
        let Ok(text) = String::from_utf8(std::mem::take(&mut host.incoming)) else {
            return 0;
        };
        u32::from(host.app.handle(&Event::Paste(text)))
    })
}

/// Map the JavaScript key ordinal onto a [`KeyCode`].
fn key_code(code: u32, ch: u32) -> Option<KeyCode> {
    Some(match code {
        0 => KeyCode::Char(char::from_u32(ch)?),
        1 => KeyCode::Enter,
        2 => KeyCode::Esc,
        3 => KeyCode::Backspace,
        4 => KeyCode::Delete,
        5 => KeyCode::Tab,
        6 => KeyCode::BackTab,
        7 => KeyCode::Up,
        8 => KeyCode::Down,
        9 => KeyCode::Left,
        10 => KeyCode::Right,
        11 => KeyCode::Home,
        12 => KeyCode::End,
        13 => KeyCode::PageUp,
        14 => KeyCode::PageDown,
        _ => return None,
    })
}

/// Keep the grid inside `u16` and non-degenerate; a zero-sized canvas is a
/// normal transient state in a browser layout.
fn clamp_grid(cols: u32, rows: u32) -> (u16, u16) {
    (
        u16::try_from(cols).unwrap_or(u16::MAX).max(1),
        u16::try_from(rows).unwrap_or(u16::MAX).max(1),
    )
}
