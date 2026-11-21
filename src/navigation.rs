//! Multi-screen navigation: a host-owned stack of screens with back-history.
//!
//! An application with more than one screen — a list that opens a detail
//! view, a settings page over a main view, a wizard — keeps a stack: open a
//! screen on top, go back to the one beneath, or swap the current one. Every
//! such app writes that stack by hand, and the part that is easy to get wrong
//! is focus: coming back to a screen should land on the control you left, not
//! on its first one.
//!
//! [`ScreenStack`] is that state. Screens are the host's own type — usually an
//! enum whose variants hold each screen's state (scroll offsets, selections,
//! inputs), so a screen beneath the top keeps everything while it is covered.
//! The stack records which control held focus when a screen was covered and
//! asks [`FocusRegistry`] to restore it when the screen is uncovered. It knows
//! nothing about rendering or keys: the host builds its view from
//! [`current`](ScreenStack::current) and decides which key means "back".

use crate::focus::FocusRegistry;

#[derive(Clone, Debug)]
struct Entry<S> {
    screen: S,
    /// The focused id when this screen was covered by another.
    focus: Option<String>,
}

/// A non-empty stack of host-defined screens; the top one is current.
///
/// The bottom screen is the root and cannot be popped, so there is always a
/// current screen to render. [`push`](Self::push) covers the current screen
/// and remembers what held focus on it; [`pop`](Self::pop) uncovers it and
/// restores that focus; [`replace`](Self::replace) swaps the current screen
/// without adding history.
///
/// ```
/// use tuika::prelude::*;
///
/// #[derive(Debug, PartialEq)]
/// enum Screen {
///     Inbox,
///     Message(u32),
/// }
///
/// let mut focus = FocusRegistry::new();
/// let mut screens = ScreenStack::new(Screen::Inbox);
///
/// // The inbox frame registers two regions; the user tabs to the list.
/// focus.begin_frame();
/// focus.register("search");
/// focus.register("list");
/// focus.focus("list");
///
/// screens.push(Screen::Message(7), &focus);
/// assert_eq!(screens.current(), &Screen::Message(7));
/// focus.begin_frame();
/// focus.register("reply");
///
/// // Back: the inbox is current again, and its next frame focuses the list.
/// assert_eq!(screens.pop(&mut focus), Some(Screen::Message(7)));
/// focus.begin_frame();
/// focus.register("search");
/// focus.register("list");
/// assert_eq!(focus.focused(), Some("list"));
/// assert_eq!(screens.pop(&mut focus), None, "the root stays");
/// ```
#[derive(Clone, Debug)]
pub struct ScreenStack<S> {
    /// Never empty; the last entry is the current screen.
    entries: Vec<Entry<S>>,
}

impl<S> ScreenStack<S> {
    /// A stack holding only `root`.
    pub fn new(root: S) -> Self {
        Self {
            entries: vec![Entry {
                screen: root,
                focus: None,
            }],
        }
    }

    /// The screen on top: the one to render and route input to.
    pub fn current(&self) -> &S {
        &self.top().screen
    }

    /// Mutable access to the current screen's state.
    pub fn current_mut(&mut self) -> &mut S {
        &mut self.top_mut().screen
    }

    /// The bottom screen, which [`pop`](Self::pop) never removes.
    pub fn root(&self) -> &S {
        &self.entries[0].screen
    }

    /// The screen [`pop`](Self::pop) would return to, if any.
    pub fn previous(&self) -> Option<&S> {
        let len = self.entries.len();
        (len > 1).then(|| &self.entries[len - 2].screen)
    }

    /// How many screens are stacked; `1` when only the root is open.
    pub fn depth(&self) -> usize {
        self.entries.len()
    }

    /// Whether there is a screen to go back to.
    pub fn can_go_back(&self) -> bool {
        self.entries.len() > 1
    }

    /// The stacked screens from the root to the current one — the history a
    /// breadcrumb trail shows.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &S> + ExactSizeIterator {
        self.entries.iter().map(|entry| &entry.screen)
    }

    /// Mutable access to every stacked screen, root first; for updating state
    /// that covered screens hold (a refreshed list, a finished download).
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut S> + ExactSizeIterator {
        self.entries.iter_mut().map(|entry| &mut entry.screen)
    }

    /// Open `screen` on top of the current one.
    ///
    /// What held focus on the current screen is recorded from `focus`, so
    /// [`pop`](Self::pop) can bring it back. The new screen starts with
    /// whatever its first frame registers, unless the host requests a focus.
    pub fn push(&mut self, screen: S, focus: &FocusRegistry) {
        self.top_mut().focus = focus.focused().map(str::to_owned);
        self.entries.push(Entry {
            screen,
            focus: None,
        });
    }

    /// Close the current screen and return it, uncovering the one beneath and
    /// asking `focus` to restore what held focus there when it renders next.
    ///
    /// Returns `None` and changes nothing when only the root is open, so a
    /// "back" key can call this unconditionally and quit (or ignore it) when
    /// it yields nothing.
    pub fn pop(&mut self, focus: &mut FocusRegistry) -> Option<S> {
        if self.entries.len() == 1 {
            return None;
        }
        let closed = self.entries.pop().map(|entry| entry.screen);
        self.restore_focus(focus);
        closed
    }

    /// Pop screens until `keep` accepts the current one (or only the root is
    /// left), returning the closed screens with the most recent first, and
    /// restoring focus on the screen that is uncovered.
    pub fn pop_until(
        &mut self,
        mut keep: impl FnMut(&S) -> bool,
        focus: &mut FocusRegistry,
    ) -> Vec<S> {
        let mut closed = Vec::new();
        while self.entries.len() > 1 && !keep(self.current()) {
            if let Some(entry) = self.entries.pop() {
                closed.push(entry.screen);
            }
        }
        if !closed.is_empty() {
            self.restore_focus(focus);
        }
        closed
    }

    /// Swap the current screen for `screen` without adding history, returning
    /// the one replaced. Going back from the new screen returns to the screen
    /// that was beneath the replaced one.
    pub fn replace(&mut self, screen: S) -> S {
        let top = self.top_mut();
        top.focus = None;
        std::mem::replace(&mut top.screen, screen)
    }

    /// Discard all history and start again from `root`, returning the screens
    /// that were open, root first.
    pub fn reset(&mut self, root: S) -> Vec<S> {
        std::mem::replace(
            &mut self.entries,
            vec![Entry {
                screen: root,
                focus: None,
            }],
        )
        .into_iter()
        .map(|entry| entry.screen)
        .collect()
    }

    fn restore_focus(&mut self, focus: &mut FocusRegistry) {
        if let Some(id) = self.top_mut().focus.take() {
            focus.request_focus(id);
        }
    }

    fn top(&self) -> &Entry<S> {
        self.entries.last().expect("a screen stack is never empty")
    }

    fn top_mut(&mut self) -> &mut Entry<S> {
        self.entries
            .last_mut()
            .expect("a screen stack is never empty")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    enum Screen {
        Home,
        List { scroll: usize },
        Detail(u8),
        Settings,
    }

    fn frame(focus: &mut FocusRegistry, ids: &[&str]) {
        focus.begin_frame();
        for id in ids {
            focus.register(*id);
        }
    }

    #[test]
    fn covered_screens_keep_their_state_and_focus() {
        let mut focus = FocusRegistry::new();
        let mut screens = ScreenStack::new(Screen::Home);
        frame(&mut focus, &["nav", "feed"]);
        assert!(focus.focus("feed"));

        screens.push(Screen::List { scroll: 0 }, &focus);
        frame(&mut focus, &["filter", "rows"]);
        assert!(focus.focus("rows"));
        if let Screen::List { scroll } = screens.current_mut() {
            *scroll = 40;
        }

        screens.push(Screen::Detail(3), &focus);
        frame(&mut focus, &["body"]);
        assert_eq!(screens.depth(), 3);
        assert_eq!(screens.previous(), Some(&Screen::List { scroll: 40 }));
        assert_eq!(
            screens.iter().collect::<Vec<_>>(),
            [
                &Screen::Home,
                &Screen::List { scroll: 40 },
                &Screen::Detail(3)
            ]
        );

        assert_eq!(screens.pop(&mut focus), Some(Screen::Detail(3)));
        assert_eq!(screens.current(), &Screen::List { scroll: 40 });
        frame(&mut focus, &["filter", "rows"]);
        assert_eq!(focus.focused(), Some("rows"));

        assert_eq!(screens.pop(&mut focus), Some(Screen::List { scroll: 40 }));
        frame(&mut focus, &["nav", "feed"]);
        assert_eq!(focus.focused(), Some("feed"));
        assert!(!screens.can_go_back());
        assert_eq!(screens.pop(&mut focus), None);
        assert_eq!(screens.current(), &Screen::Home);
    }

    #[test]
    fn replace_swaps_without_history_and_pop_until_unwinds() {
        let mut focus = FocusRegistry::new();
        let mut screens = ScreenStack::new(Screen::Home);
        frame(&mut focus, &["feed"]);
        screens.push(Screen::List { scroll: 0 }, &focus);
        assert_eq!(
            screens.replace(Screen::Settings),
            Screen::List { scroll: 0 }
        );
        assert_eq!(screens.depth(), 2);
        screens.push(Screen::Detail(1), &focus);
        screens.push(Screen::Detail(2), &focus);

        let closed = screens.pop_until(|screen| *screen == Screen::Home, &mut focus);
        assert_eq!(
            closed,
            [Screen::Detail(2), Screen::Detail(1), Screen::Settings]
        );
        assert_eq!(screens.current(), &Screen::Home);
        frame(&mut focus, &["nav", "feed"]);
        assert_eq!(focus.focused(), Some("feed"));
        // Nothing matches: unwinding stops at the root.
        screens.push(Screen::Settings, &focus);
        assert_eq!(screens.pop_until(|_| false, &mut focus), [Screen::Settings]);
        assert_eq!(screens.depth(), 1);
    }

    #[test]
    fn reset_returns_the_old_history() {
        let focus = FocusRegistry::new();
        let mut screens = ScreenStack::new(Screen::Home);
        screens.push(Screen::Settings, &focus);
        assert_eq!(
            screens.reset(Screen::Detail(9)),
            [Screen::Home, Screen::Settings]
        );
        assert_eq!(screens.root(), &Screen::Detail(9));
        assert_eq!(screens.current(), &Screen::Detail(9));
        for screen in screens.iter_mut() {
            *screen = Screen::Home;
        }
        assert_eq!(screens.current(), &Screen::Home);
    }
}
