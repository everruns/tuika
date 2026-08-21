//! Public helpers for hermetic view and resize tests.

use crate::buffer::Buffer;
use crate::geometry::Rect;

use crate::{
    Application, Element, Event, RenderCtx, Signal, StyleSheet, Theme, UpdateResult, View, paint,
    paint_with_context, paint_with_sheet,
};

/// A deterministic, terminal-free application harness.
///
/// The harness owns application state, viewport size, and frame numbering. It
/// drives the same `Signal`/`UpdateResult` contract as [`Runner`](crate::Runner)
/// while rendering into an in-memory [`Buffer`]. No raw mode, terminal global,
/// sleeping, or OS event queue is involved.
///
/// ```
/// use tuika::prelude::*;
/// use tuika::testing::{TestHarness, grid};
///
/// let mut app = TestHarness::new(0u8, 3, 1);
/// let (result, frame) = app.step(
///     Signal::Tick,
///     |count, _| { *count += 1; UpdateResult::Dirty },
///     |count, _frame| element(Text::raw(count.to_string())),
/// );
/// assert_eq!(result, UpdateResult::Dirty);
/// assert_eq!(grid(&frame.unwrap()), "1  ");
/// ```
pub struct TestHarness<S> {
    state: S,
    theme: Theme,
    width: u16,
    height: u16,
    frame: u64,
}

impl<S> TestHarness<S> {
    /// Create a harness with the default theme and a fixed viewport.
    pub fn new(state: S, width: u16, height: u16) -> Self {
        Self::with_theme(state, width, height, Theme::default())
    }

    /// Create a harness with an explicit theme.
    pub fn with_theme(state: S, width: u16, height: u16, theme: Theme) -> Self {
        Self {
            state,
            theme,
            width,
            height,
            frame: 0,
        }
    }

    /// Borrow the current application state.
    pub fn state(&self) -> &S {
        &self.state
    }

    /// Mutably borrow the current application state.
    pub fn state_mut(&mut self) -> &mut S {
        &mut self.state
    }

    /// Change the headless viewport for subsequent frames.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Build and render the next application frame.
    pub fn render(&mut self, view: impl FnOnce(&S, u64) -> Element) -> Buffer {
        let root = view(&self.state, self.frame);
        let buffer = render(root.as_ref(), self.width, self.height, &self.theme);
        self.frame = self.frame.wrapping_add(1);
        buffer
    }

    /// Deliver a signal, rendering only when the update marks the app dirty.
    ///
    /// Resize updates also render because layout depends on viewport geometry.
    /// Otherwise, `None` represents a clean/consumed update or exit; inspect
    /// the returned [`UpdateResult`] to distinguish them.
    pub fn step(
        &mut self,
        signal: Signal,
        update: impl FnOnce(&mut S, Signal) -> UpdateResult,
        view: impl FnOnce(&S, u64) -> Element,
    ) -> (UpdateResult, Option<Buffer>) {
        let resized = self.apply_resize_signal(&signal);
        let result = update(&mut self.state, signal);
        let frame = (result != UpdateResult::Exit && (result == UpdateResult::Dirty || resized))
            .then(|| self.render(view));
        (result, frame)
    }

    fn apply_resize_signal(&mut self, signal: &Signal) -> bool {
        if let Signal::Event(Event::Resize { width, height }) = signal {
            self.resize(*width, *height);
            true
        } else {
            false
        }
    }
}

impl<A: Application> TestHarness<A> {
    /// Build and render the application's next frame, including borrowed views.
    pub fn render_app(&mut self) -> Buffer {
        let root = self.state.view(self.frame);
        let buffer = render(root.as_ref(), self.width, self.height, &self.theme);
        drop(root);
        self.frame = self.frame.wrapping_add(1);
        buffer
    }

    /// Deliver a signal through [`Application::update`] and render when needed.
    ///
    /// Resize events always produce a frame unless the application exits,
    /// matching the terminal runners even when `update` returns
    /// [`UpdateResult::Clean`].
    pub fn step_app(&mut self, signal: Signal) -> (UpdateResult, Option<Buffer>) {
        let resized = self.apply_resize_signal(&signal);
        let result = self.state.update(signal);
        let frame = (result != UpdateResult::Exit && (result == UpdateResult::Dirty || resized))
            .then(|| self.render_app());
        (result, frame)
    }
}

/// Render `view` into a zero-origin in-memory buffer.
pub fn render(view: &dyn View, width: u16, height: u16, theme: &Theme) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint(&mut buffer, area, theme, view, &[]);
    buffer
}

/// Render `view` with an explicit stylesheet into a zero-origin in-memory buffer.
pub fn render_with_sheet(
    view: &dyn View,
    width: u16,
    height: u16,
    theme: &Theme,
    sheet: StyleSheet,
) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint_with_sheet(&mut buffer, area, theme, sheet, view, &[]);
    buffer
}

/// Render `view` with a fully configured context into a zero-origin buffer.
pub fn render_with_context(view: &dyn View, width: u16, height: u16, ctx: &RenderCtx) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buffer = Buffer::empty(area);
    paint_with_context(&mut buffer, area, ctx, view, &[]);
    buffer
}

/// Convert a buffer to a stable glyph grid suitable for snapshot assertions.
pub fn grid(buffer: &Buffer) -> String {
    let mut output = String::new();
    for y in buffer.area.y..buffer.area.bottom() {
        for x in buffer.area.x..buffer.area.right() {
            output.push_str(buffer[(x, y)].symbol());
        }
        if y + 1 < buffer.area.bottom() {
            output.push('\n');
        }
    }
    output
}

/// Render the same view at several sizes, useful for resize and degenerate-size tests.
pub fn render_sizes(
    view: &dyn View,
    sizes: impl IntoIterator<Item = (u16, u16)>,
    theme: &Theme,
) -> Vec<Buffer> {
    sizes
        .into_iter()
        .map(|(width, height)| render(view, width, height, theme))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Text;
    use crate::style::Theme;

    #[test]
    fn public_testing_grid_is_stable_and_rectangular() {
        let theme = Theme::default();
        let rendered = render(&Text::raw("hi"), 3, 2, &theme);
        assert_eq!(grid(&rendered), "hi \n   ");
    }

    #[test]
    fn harness_drives_state_and_only_renders_dirty_updates() {
        let mut harness = TestHarness::new(0u8, 2, 1);
        let (result, frame) = harness.step(
            Signal::Tick,
            |state, _| {
                *state += 1;
                UpdateResult::Dirty
            },
            |state, _| crate::element(Text::raw(state.to_string())),
        );
        assert_eq!(result, UpdateResult::Dirty);
        assert_eq!(grid(&frame.unwrap()), "1 ");

        let (_, frame) = harness.step(
            Signal::Tick,
            |_, _| UpdateResult::Clean,
            |_, _| crate::element(Text::raw("unused")),
        );
        assert!(frame.is_none());
    }
}
