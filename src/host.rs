//! Terminal host: screen-mode lifecycle, event translation, compositor.
//!
//! This is the boundary between `tuika` and a real terminal. [`TerminalSession`]
//! owns the complete lifecycle for either [`ScreenMode`] — the alternate screen
//! or a split footer on the main screen (see [`screen`](crate::screen)) —
//! including enhanced keyboard reporting; [`AltScreen`] is the narrower guard
//! for hosts that manage raw mode, keyboard modes, and cursor visibility
//! themselves.
//! [`translate_event`] maps crossterm input to `tuika` [`Event`]s. [`paint`]
//! composites a root view plus any overlays into the frame buffer: background
//! fill, root, then each overlay last (clearing its rect first), which is why
//! overlays never leak surrounding chrome here.

use std::io::{self, Write};
use std::process::{Command as ProcessCommand, Stdio};

use crate::buffer::Buffer;
use crate::geometry::Rect;
use crate::style::Style;
use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    DisableMouseCapture, EnableMouseCapture, Event as CtEvent, KeyCode as CtKeyCode, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, MouseButton as CtMouseButton,
    MouseEventKind as CtMouseKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
    is_raw_mode_enabled,
};

use super::event::{Event, Key, KeyCode, Mouse, MouseButton, MouseKind};
use super::overlay::Overlay;
use super::screen::ScreenMode;
use super::style::{StyleSheet, Theme};
use super::surface::Surface;
use super::view::{RenderCtx, SelectionFrame, View};

/// RAII guard for the alternate screen.
pub struct AltScreen {
    active: bool,
    mouse_capture: bool,
}

impl AltScreen {
    /// Enter the alternate screen, leaving mouse handling to the terminal.
    /// Assumes raw mode is already enabled by the caller.
    pub fn enter() -> io::Result<Self> {
        Self::enter_with(false)
    }

    /// Enter the alternate screen and enable terminal mouse reporting.
    ///
    /// Capture gives the application pointer and wheel events, but prevents
    /// native OSC 8 link activation, selection, and scrolling.
    pub fn enter_with_mouse_capture() -> io::Result<Self> {
        Self::enter_with(true)
    }

    fn enter_with(mouse_capture: bool) -> io::Result<Self> {
        let mut out = io::stdout();
        execute!(out, EnterAlternateScreen)?;
        if mouse_capture {
            execute!(out, EnableMouseCapture)?;
        }
        out.flush()?;
        Ok(Self {
            active: true,
            mouse_capture,
        })
    }

    /// Explicitly leave (also runs on drop; idempotent).
    pub fn leave(&mut self) {
        if self.active {
            let mut out = io::stdout();
            let _ = out.write_all(
                crate::term::pointer::encode(crate::term::pointer::PointerShape::Default)
                    .as_bytes(),
            );
            if self.mouse_capture {
                let _ = execute!(out, DisableMouseCapture);
            }
            let _ = execute!(out, LeaveAlternateScreen);
            let _ = out.flush();
            self.active = false;
        }
    }
}

impl Drop for AltScreen {
    fn drop(&mut self) {
        self.leave();
    }
}

/// Complete RAII ownership of a terminal session, in either [`ScreenMode`].
///
/// Construction enables raw mode and enhanced keyboard reporting, hides the
/// cursor, and — depending on the mode — enters the alternate screen. Mouse
/// capture is explicit; the defaults preserve native terminal links, selection,
/// and scrolling. Enhanced reporting is what lets crossterm distinguish
/// chords such as Shift+Enter from plain Enter on ANSI terminals; Windows
/// console events already carry modifier state through the native input API.
/// Drop restores exactly what it took, including when unwinding from a panic.
/// Pre-existing raw mode and keyboard-reporting stack entries are preserved. If
/// construction fails partway through, it performs the same best-effort
/// rollback before returning.
///
/// A split-footer session touches neither the alternate screen nor (by default)
/// mouse capture, so the terminal's scrollback, selection, and wheel keep
/// working while the footer is up. It does *not* reserve or release the footer's
/// rows — that is the renderer's job, since only it knows where the viewport
/// landed: see [`pin_footer`](crate::screen::pin_footer) and
/// [`close_footer`](crate::screen::close_footer).
pub struct TerminalSession {
    active: bool,
    raw_mode_owned: bool,
    keyboard_enhancement: Option<KeyboardEnhancement>,
    mode: ScreenMode,
    mouse_capture: bool,
    cursor_hidden: bool,
}

/// Mouse-capture policy for a [`TerminalSessionConfig`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseCapture {
    /// Follow the selected [`ScreenMode`].
    #[default]
    ModeDefault,
    /// Enable mouse reporting regardless of the mode default. This gives mouse
    /// events to the application instead of native terminal links and selection.
    Enabled,
    /// Leave mouse reporting disabled, preserving native terminal handling.
    Disabled,
}

/// Independently configurable terminal lifecycle policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSessionConfig {
    /// Screen ownership and viewport mode.
    pub screen_mode: ScreenMode,
    /// Enable raw input mode for the lifetime of the session.
    pub raw_mode: bool,
    /// Negotiate enhanced keyboard reporting where supported.
    pub enhanced_keyboard: bool,
    /// Mouse reporting policy.
    pub mouse_capture: MouseCapture,
    /// Hide the cursor while the session is active.
    pub hide_cursor: bool,
}

impl Default for TerminalSessionConfig {
    fn default() -> Self {
        Self::for_mode(ScreenMode::default())
    }
}

impl TerminalSessionConfig {
    /// The established lifecycle defaults for `screen_mode`.
    pub const fn for_mode(screen_mode: ScreenMode) -> Self {
        Self {
            screen_mode,
            raw_mode: true,
            enhanced_keyboard: true,
            mouse_capture: MouseCapture::ModeDefault,
            hide_cursor: true,
        }
    }

    pub(crate) fn captures_mouse(self) -> bool {
        match self.mouse_capture {
            MouseCapture::ModeDefault => self.screen_mode.captures_mouse(),
            MouseCapture::Enabled => true,
            MouseCapture::Disabled => false,
        }
    }
}

impl TerminalSession {
    /// Enter a full-screen session on the alternate screen, rolling back on
    /// failure. Shorthand for `enter_with(ScreenMode::Alternate)`.
    pub fn enter() -> io::Result<Self> {
        Self::enter_with(ScreenMode::Alternate)
    }

    /// Enter a session in `mode`, rolling back on failure.
    pub fn enter_with(mode: ScreenMode) -> io::Result<Self> {
        Self::enter_config(TerminalSessionConfig::for_mode(mode))
    }

    /// Enter a session using independently configurable lifecycle policies.
    pub fn enter_config(config: TerminalSessionConfig) -> io::Result<Self> {
        let raw_mode_owned = config.raw_mode && !is_raw_mode_enabled()?;
        if raw_mode_owned {
            enable_raw_mode()?;
        }
        let mut out = io::stdout();
        let keyboard = KeyboardEnhancement::detect();
        let keyboard_active = if config.enhanced_keyboard {
            match keyboard.enable(&mut out) {
                Ok(active) => active,
                Err(error) => {
                    if raw_mode_owned {
                        let _ = disable_raw_mode();
                    }
                    return Err(error);
                }
            }
        } else {
            false
        };
        let mode = config.screen_mode;
        let mouse_capture = config.captures_mouse();
        let entered = (|| -> io::Result<()> {
            if mode.is_alternate() {
                execute!(out, EnterAlternateScreen)?;
            }
            if mouse_capture {
                execute!(out, EnableMouseCapture)?;
            }
            if config.hide_cursor {
                execute!(out, Hide)?;
            }
            out.flush()
        })();
        if let Err(error) = entered {
            if config.hide_cursor {
                let _ = execute!(out, Show);
            }
            if mouse_capture {
                let _ = execute!(out, DisableMouseCapture);
            }
            if mode.is_alternate() {
                let _ = execute!(out, LeaveAlternateScreen);
            }
            if keyboard_active {
                keyboard.disable(&mut out);
            }
            if raw_mode_owned {
                let _ = disable_raw_mode();
            }
            return Err(error);
        }
        Ok(Self {
            active: true,
            raw_mode_owned,
            keyboard_enhancement: keyboard_active.then_some(keyboard),
            mode,
            mouse_capture,
            cursor_hidden: config.hide_cursor,
        })
    }

    /// The mode this session was entered in.
    pub fn mode(&self) -> ScreenMode {
        self.mode
    }

    /// Restore the terminal immediately. Calling this more than once is safe.
    pub fn leave(&mut self) {
        if !self.active {
            return;
        }
        let mut out = io::stdout();
        let _ = out.write_all(
            crate::term::pointer::encode(crate::term::pointer::PointerShape::Default).as_bytes(),
        );
        if self.cursor_hidden {
            let _ = execute!(out, Show);
        }
        if self.mouse_capture {
            let _ = execute!(out, DisableMouseCapture);
        }
        if self.mode.is_alternate() {
            let _ = execute!(out, LeaveAlternateScreen);
        }
        if let Some(keyboard) = self.keyboard_enhancement.take() {
            keyboard.disable(&mut out);
        }
        let _ = out.flush();
        if self.raw_mode_owned {
            let _ = disable_raw_mode();
        }
        self.active = false;
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        self.leave();
    }
}

/// Keyboard-reporting policy for one terminal session.
///
/// Kitty's protocol is a stack: push exactly one level on entry and pop exactly
/// one on exit, preserving any mode the caller had already installed. iTerm2
/// and tmux's xterm key format must not report event types: both can lose or
/// leak modified key events in that mode. tmux's CSI-u format instead needs
/// xterm modifyOtherKeys mode 2 so modified Enter reaches the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KeyboardEnhancement {
    flags: KeyboardEnhancementFlags,
    modify_other_keys: bool,
}

impl KeyboardEnhancement {
    fn detect() -> Self {
        let iterm2 = std::env::var("TERM_PROGRAM")
            .is_ok_and(|value| value.eq_ignore_ascii_case("iTerm.app"))
            || std::env::var("LC_TERMINAL").is_ok_and(|value| value.eq_ignore_ascii_case("iTerm2"));
        let in_tmux = std::env::var_os("TMUX").is_some() || std::env::var_os("TMUX_PANE").is_some();
        let tmux_format = in_tmux.then(read_tmux_extended_keys_format).flatten();
        Self::for_terminal(iterm2, in_tmux, tmux_format.as_deref())
    }

    fn for_terminal(iterm2: bool, in_tmux: bool, tmux_format: Option<&str>) -> Self {
        let mut flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS;
        if !iterm2 && tmux_format != Some("xterm") {
            flags |= KeyboardEnhancementFlags::REPORT_EVENT_TYPES;
        }
        Self {
            flags,
            modify_other_keys: in_tmux && tmux_format == Some("csi-u"),
        }
    }

    #[cfg(not(windows))]
    fn enable(self, out: &mut impl Write) -> io::Result<bool> {
        self.write_enable_ansi(out)?;
        out.flush()?;
        Ok(true)
    }

    // The Windows console input API already reports modifier state, while
    // crossterm deliberately does not implement Kitty keyboard negotiation for
    // the legacy WinAPI backend.
    #[cfg(windows)]
    fn enable(self, _out: &mut impl Write) -> io::Result<bool> {
        Ok(false)
    }

    #[cfg(not(windows))]
    fn disable(self, out: &mut impl Write) {
        let _ = self.write_disable_ansi(out);
    }

    #[cfg(windows)]
    fn disable(self, _out: &mut impl Write) {}

    #[cfg(any(not(windows), test))]
    fn write_enable_ansi(self, out: &mut impl Write) -> io::Result<()> {
        write!(out, "\x1b[>4;0m\x1b[>{}u", self.flags.bits())?;
        if self.modify_other_keys {
            out.write_all(b"\x1b[>4;2m")?;
        }
        Ok(())
    }

    #[cfg(any(not(windows), test))]
    fn write_disable_ansi(self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(b"\x1b[<1u\x1b[>4;0m")
    }
}

fn read_tmux_extended_keys_format() -> Option<String> {
    for args in [
        ["display-message", "-p", "#{extended-keys-format}"],
        ["show-options", "-gqv", "extended-keys-format"],
    ] {
        let output = ProcessCommand::new("tmux")
            .args(args)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .ok()?;
        if output.status.success()
            && let Ok(value) = String::from_utf8(output.stdout)
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Composite `root` and `overlays` into `buffer` over `area`, using `theme`'s
/// default [`StyleSheet`]. To centralize styling with a custom stylesheet, use
/// [`paint_with_sheet`]. A [`ScopedScene`](crate::ScopedScene) can be passed as
/// `root` to combine a borrowed application view with owned scene overlays.
pub fn paint(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    root: &dyn View,
    overlays: &[Overlay],
) {
    paint_with_sheet(
        buffer,
        area,
        theme,
        StyleSheet::from_theme(theme),
        root,
        overlays,
    );
}

/// Paint an owned [`Scene`](crate::Scene) with no borrowed overlay plumbing.
pub fn paint_scene(buffer: &mut Buffer, area: Rect, theme: &Theme, scene: &crate::Scene) {
    paint(buffer, area, theme, scene, &[]);
}

/// [`paint`] with an explicit [`StyleSheet`], so a host installs one styling
/// policy that the whole component tree resolves against.
pub fn paint_with_sheet(
    buffer: &mut Buffer,
    area: Rect,
    theme: &Theme,
    sheet: StyleSheet,
    root: &dyn View,
    overlays: &[Overlay],
) {
    let ctx = RenderCtx::new(theme).with_sheet(sheet);
    paint_with_context(buffer, area, &ctx, root, overlays);
}

/// Composite a frame with a fully configured render context.
///
/// This is the host boundary for an application that installs a
/// [`StyleResolver`](crate::StyleResolver) in addition to a theme and
/// stylesheet. Simpler hosts can keep using [`paint`] or [`paint_with_sheet`].
pub fn paint_with_context(
    buffer: &mut Buffer,
    area: Rect,
    ctx: &RenderCtx,
    root: &dyn View,
    overlays: &[Overlay],
) {
    let selection = SelectionFrame::default();
    paint_with_context_and_selection(buffer, area, ctx, root, overlays, &selection);
}

pub(crate) fn paint_with_context_and_selection(
    buffer: &mut Buffer,
    area: Rect,
    ctx: &RenderCtx,
    root: &dyn View,
    overlays: &[Overlay],
    selection: &SelectionFrame,
) {
    selection.clear();
    let ctx = ctx.clone().with_selection(selection);
    // Base background so the alternate screen is fully owned (no stale cells).
    {
        let mut surface = Surface::new(buffer, area);
        surface.fill(Style::default().bg(ctx.theme.background));
    }
    {
        let mut surface = Surface::new(buffer, area);
        root.render(area, &mut surface, &ctx);
    }
    for overlay in overlays {
        let mut surface = Surface::new(buffer, overlay.area);
        if overlay.clear {
            surface.fill(Style::default().bg(ctx.theme.surface));
        }
        // An open overlay owns input, so render it focused.
        overlay
            .view
            .render(overlay.area, &mut surface, &ctx.with_focus(true));
    }
}

/// Translate a crossterm event into a `tuika` event, dropping ones `tuika`
/// does not model (key-release, unmapped codes).
pub fn translate_event(event: CtEvent) -> Option<Event> {
    match event {
        CtEvent::Key(k) => {
            if k.kind == KeyEventKind::Release {
                return None;
            }
            let code = match k.code {
                CtKeyCode::Char(c) => KeyCode::Char(c),
                CtKeyCode::Enter => KeyCode::Enter,
                CtKeyCode::Esc => KeyCode::Esc,
                CtKeyCode::Backspace => KeyCode::Backspace,
                CtKeyCode::Delete => KeyCode::Delete,
                CtKeyCode::Tab => KeyCode::Tab,
                CtKeyCode::BackTab => KeyCode::BackTab,
                CtKeyCode::Up => KeyCode::Up,
                CtKeyCode::Down => KeyCode::Down,
                CtKeyCode::Left => KeyCode::Left,
                CtKeyCode::Right => KeyCode::Right,
                CtKeyCode::Home => KeyCode::Home,
                CtKeyCode::End => KeyCode::End,
                CtKeyCode::PageUp => KeyCode::PageUp,
                CtKeyCode::PageDown => KeyCode::PageDown,
                _ => return None,
            };
            Some(Event::Key(Key {
                code,
                ctrl: k.modifiers.contains(KeyModifiers::CONTROL),
                alt: k.modifiers.contains(KeyModifiers::ALT),
                shift: k.modifiers.contains(KeyModifiers::SHIFT),
            }))
        }
        CtEvent::Mouse(m) => {
            let kind = match m.kind {
                CtMouseKind::Down(b) => MouseKind::Down(button(b)),
                CtMouseKind::Up(b) => MouseKind::Up(button(b)),
                CtMouseKind::Drag(b) => MouseKind::Drag(button(b)),
                CtMouseKind::Moved => MouseKind::Moved,
                CtMouseKind::ScrollUp => MouseKind::ScrollUp,
                CtMouseKind::ScrollDown => MouseKind::ScrollDown,
                CtMouseKind::ScrollLeft => MouseKind::ScrollLeft,
                CtMouseKind::ScrollRight => MouseKind::ScrollRight,
            };
            Some(Event::Mouse(Mouse {
                kind,
                column: m.column,
                row: m.row,
                shift: m.modifiers.contains(KeyModifiers::SHIFT),
                ctrl: m.modifiers.contains(KeyModifiers::CONTROL),
                alt: m.modifiers.contains(KeyModifiers::ALT),
                super_key: m.modifiers.contains(KeyModifiers::SUPER),
            }))
        }
        CtEvent::Paste(text) => Some(Event::Paste(text)),
        CtEvent::Resize(width, height) => Some(Event::Resize { width, height }),
        _ => None,
    }
}

fn button(b: CtMouseButton) -> MouseButton {
    match b {
        CtMouseButton::Left => MouseButton::Left,
        CtMouseButton::Right => MouseButton::Right,
        CtMouseButton::Middle => MouseButton::Middle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::components::{Boxed, Flex, ProgressBar, Scroll, ScrollState, StatusBar, Text};
    use crate::event::{Event, MouseButton, MouseKind};
    use crate::geometry::Rect;
    use crate::style::Theme;
    use crate::tests::support::{buffer, rainbow_theme, row};
    use crate::text::{Line, Span};
    use crate::view::{Element, element};

    #[test]
    fn session_mouse_policy_can_override_screen_mode_defaults() {
        let alternate = TerminalSessionConfig {
            mouse_capture: MouseCapture::Disabled,
            ..TerminalSessionConfig::for_mode(ScreenMode::Alternate)
        };
        assert!(!alternate.captures_mouse());

        let footer = TerminalSessionConfig {
            mouse_capture: MouseCapture::Enabled,
            ..TerminalSessionConfig::for_mode(ScreenMode::split_footer(3))
        };
        assert!(footer.captures_mouse());
    }

    /// A representative tree: a scrollable bordered body, a progress bar, and a
    /// status bar — the shapes the full-screen renderer actually composes.
    fn demo_tree(frame: u64) -> Element {
        let lines: Vec<Line<'static>> = (0..40).map(|i| Line::from(format!("row {i}"))).collect();
        let mut st = ScrollState::new();
        st.clamp(40, 8);
        element(
            Flex::column()
                .grow(
                    1,
                    element(Boxed::new(element(Scroll::new(lines, &st))).title(" body ")),
                )
                .fixed(1, element(ProgressBar::indeterminate(frame)))
                .fixed(1, element(StatusBar::new().left(vec![Span::raw("status")]))),
        )
    }

    #[test]
    fn translate_maps_button_modifiers_and_drag() {
        use crossterm::event::{
            Event as CtEvent, KeyModifiers, MouseButton as CtButton, MouseEvent, MouseEventKind,
        };
        let ct = CtEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(CtButton::Right),
            column: 7,
            row: 3,
            modifiers: KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::SUPER,
        });
        let Some(Event::Mouse(m)) = translate_event(ct) else {
            panic!("expected a mouse event");
        };
        assert_eq!(m.kind, MouseKind::Down(MouseButton::Right));
        assert_eq!((m.column, m.row), (7, 3));
        assert!(m.shift && m.ctrl && !m.alt && m.super_key);
        assert!(!m.plain());
    }

    #[test]
    fn translate_preserves_shift_enter() {
        let ct = CtEvent::Key(crossterm::event::KeyEvent::new(
            CtKeyCode::Enter,
            KeyModifiers::SHIFT,
        ));
        let Some(Event::Key(key)) = translate_event(ct) else {
            panic!("expected a key event");
        };
        assert_eq!(key.code, KeyCode::Enter);
        assert!(key.shift && !key.ctrl && !key.alt);
    }

    #[test]
    fn keyboard_enhancement_uses_event_reporting_by_default() {
        let keyboard = KeyboardEnhancement::for_terminal(false, false, None);
        let mut output = Vec::new();
        keyboard.write_enable_ansi(&mut output).unwrap();
        keyboard.write_disable_ansi(&mut output).unwrap();
        assert_eq!(output, b"\x1b[>4;0m\x1b[>7u\x1b[<1u\x1b[>4;0m");
    }

    #[test]
    fn keyboard_enhancement_avoids_event_types_in_iterm_and_xterm_tmux() {
        for keyboard in [
            KeyboardEnhancement::for_terminal(true, false, None),
            KeyboardEnhancement::for_terminal(false, true, Some("xterm")),
        ] {
            let mut output = Vec::new();
            keyboard.write_enable_ansi(&mut output).unwrap();
            assert_eq!(output, b"\x1b[>4;0m\x1b[>5u");
        }
    }

    #[test]
    fn keyboard_enhancement_enables_modify_other_keys_for_csi_u_tmux() {
        let keyboard = KeyboardEnhancement::for_terminal(false, true, Some("csi-u"));
        let mut output = Vec::new();
        keyboard.write_enable_ansi(&mut output).unwrap();
        assert_eq!(output, b"\x1b[>4;0m\x1b[>7u\x1b[>4;2m");
    }

    #[test]
    fn paint_composites_background_root_and_overlay() {
        let theme = Theme::default();
        let mut buf = buffer(20, 5);
        let area = buf.area;
        let root = Text::new(vec![Line::from(Span::raw("base layer"))]);
        let dialog = Text::new(vec![Line::from("MODAL")]);
        let overlay_area = Rect::new(5, 2, 7, 1);
        let overlays = [Overlay {
            area: overlay_area,
            view: &dialog,
            clear: true,
        }];
        paint(&mut buf, area, &theme, &root, &overlays);
        // Root text on the top row.
        assert!(row(&buf, 0).starts_with("base layer"));
        // Background fill applied everywhere.
        assert_eq!(buf[(0, 0)].bg, theme.background);
        // Overlay painted last on its row with a surface background.
        assert!(row(&buf, 2).contains("MODAL"));
        assert_eq!(buf[(5, 2)].bg, theme.surface);
    }

    #[test]
    fn paint_survives_degenerate_and_swept_sizes() {
        let theme = Theme::default();
        // The test passing (no panic, no out-of-bounds index) is the assertion.
        for &w in &[0u16, 1, 2, 3, 5, 10, 17, 40, 200] {
            for &h in &[0u16, 1, 2, 3, 5, 8, 40] {
                let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
                let area = buf.area;
                paint(&mut buf, area, &theme, demo_tree(w as u64).as_ref(), &[]);
                assert_eq!(buf.area, Rect::new(0, 0, w, h));
            }
        }
    }

    #[test]
    fn resize_reflows_body_and_status() {
        let theme = Theme::default();

        // Wide + tall: the body box title is visible on the top border.
        let mut big = buffer(80, 24);
        let a = big.area;
        paint(&mut big, a, &theme, demo_tree(0).as_ref(), &[]);
        assert!(
            row(&big, 0).contains("body"),
            "title at 80x24: {:?}",
            row(&big, 0)
        );

        // Then a small resize: must still render the status row at the bottom and
        // must not panic or leave the previous size's content behind (fresh buffer).
        let mut small = buffer(20, 6);
        let a2 = small.area;
        paint(&mut small, a2, &theme, demo_tree(0).as_ref(), &[]);
        assert!(
            row(&small, 5).contains("status"),
            "status row at 20x6: {:?}",
            row(&small, 5)
        );
    }

    #[test]
    fn compositor_uses_theme_background_and_overlay_surface() {
        let t = rainbow_theme();
        let mut buf = buffer(12, 4);
        let area = buf.area;
        let root = Text::raw("base");
        let dialog = Text::raw("hi");
        let overlays = [Overlay {
            area: Rect::new(4, 1, 4, 1),
            view: &dialog,
            clear: true,
        }];
        paint(&mut buf, area, &t, &root, &overlays);
        assert_eq!(
            buf[(0, 3)].bg,
            t.background,
            "base fill uses theme.background"
        );
        assert_eq!(
            buf[(4, 1)].bg,
            t.surface,
            "overlay clear uses theme.surface"
        );
    }
}
