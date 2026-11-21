//! Focus registry and input ownership.
//!
//! Focusable regions register a stable string id each frame. The registry
//! tracks which id currently holds focus and cycles through them with
//! Tab/BackTab. Overlays claim *input ownership*: while an overlay is open, the
//! registry reports its id as focused and the host routes events there,
//! regardless of the base tree's focus ring — the missing "input ownership
//! across overlay and non-overlay components" piece from Pi.

use super::event::{Event, EventFlow, KeyCode};

/// Tracks the focus ring and the currently focused region.
#[derive(Clone, Debug, Default)]
pub struct FocusRegistry {
    order: Vec<String>,
    focused: Option<String>,
    /// When set, this id owns all input (an open overlay).
    owner: Option<String>,
    /// A focus request waiting for its id to register; `true` once a frame
    /// has begun since the request, so an unmet request lapses after one frame.
    pending: Option<(String, bool)>,
}

impl FocusRegistry {
    /// Create an empty registry with no registered or focused regions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a frame: reconcile focus against the preceding frame, then clear
    /// the per-frame focus ring. Registrations for the new frame follow.
    pub fn begin_frame(&mut self) {
        self.pending = match self.pending.take() {
            Some((id, false)) => Some((id, true)),
            // A whole frame registered without this id: the request lapses.
            _ => None,
        };
        if self.order.is_empty() {
            self.focused = None;
        } else if !self
            .focused
            .as_ref()
            .is_some_and(|focused| self.order.contains(focused))
        {
            self.focused = self.order.first().cloned();
        }
        self.order.clear();
    }

    /// Register a focusable region for this frame, in tab order.
    pub fn register(&mut self, id: impl Into<String>) {
        let id = id.into();
        if self
            .pending
            .as_ref()
            .is_some_and(|(pending, _)| *pending == id)
        {
            self.pending = None;
            self.focused = Some(id.clone());
        } else if self.focused.is_none() {
            self.focused = Some(id.clone());
        }
        self.order.push(id);
    }

    /// Give exclusive input ownership to `id` (e.g. an overlay) until cleared.
    pub fn set_owner(&mut self, id: impl Into<String>) {
        self.owner = Some(id.into());
    }

    /// Release input ownership.
    pub fn clear_owner(&mut self) {
        self.owner = None;
    }

    /// The overlay holding exclusive input, if any.
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    pub(crate) fn owner(&self) -> Option<&str> {
        self.owner.as_deref()
    }

    /// This frame's registrations, in tab order.
    #[cfg_attr(not(feature = "devtools"), allow(dead_code))]
    pub(crate) fn ring(&self) -> &[String] {
        &self.order
    }

    fn ring_focus(&self) -> Option<&str> {
        self.focused
            .as_deref()
            .filter(|focused| self.order.iter().any(|id| id == focused))
            .or_else(|| self.order.first().map(String::as_str))
    }

    /// The id that should receive input: the owner if any, else the focused id.
    /// If a focused region disappeared this frame, the first current
    /// registration is used immediately.
    pub fn active(&self) -> Option<&str> {
        self.owner.as_deref().or_else(|| self.ring_focus())
    }

    /// Whether `id` is the active input target.
    pub fn is_active(&self, id: &str) -> bool {
        self.active() == Some(id)
    }

    /// Whether `id` holds focus in the base ring (ignores overlay ownership).
    pub fn is_focused(&self, id: &str) -> bool {
        self.ring_focus() == Some(id)
    }

    /// Focus a registered base-ring region, returning whether the request was
    /// accepted.
    ///
    /// Requests for unknown ids and requests made while an overlay owns input
    /// are ignored without disturbing the existing ring or its Tab order. A
    /// host commonly calls this after resolving a pane id through
    /// [`HitMap`](crate::mouse::HitMap), then renders each pane through
    /// [`FocusScope`](crate::components::FocusScope).
    pub fn focus(&mut self, id: &str) -> bool {
        if self.owner.is_some() || !self.order.iter().any(|registered| registered == id) {
            return false;
        }
        self.focused = Some(id.to_owned());
        true
    }

    /// Focus `id` as soon as it is registered: immediately if it is in the
    /// current ring, otherwise when the next frame registers it.
    ///
    /// This is how focus comes back to a screen that is not on screen yet —
    /// after [`ScreenStack::pop`](crate::navigation::ScreenStack::pop), the
    /// previous screen's regions register only when it next renders, so a
    /// plain [`focus`](Self::focus) would be refused. A request the next frame
    /// does not satisfy lapses, so a stale id cannot steal focus later. Unlike
    /// [`focus`](Self::focus) it is accepted while an overlay owns input; it
    /// sets the base ring underneath.
    pub fn request_focus(&mut self, id: impl Into<String>) {
        let id = id.into();
        if self.order.contains(&id) {
            self.focused = Some(id);
            self.pending = None;
        } else {
            self.pending = Some((id, false));
        }
    }

    /// The id holding focus in the base ring, ignoring overlay ownership:
    /// what a screen saves before another covers it.
    pub fn focused(&self) -> Option<&str> {
        self.ring_focus()
    }

    fn advance(&mut self, delta: isize) {
        if self.order.is_empty() {
            return;
        }
        let len = self.order.len() as isize;
        let current = self
            .ring_focus()
            .and_then(|focused| self.order.iter().position(|id| id == focused))
            .map(|p| p as isize)
            .unwrap_or(0);
        let next = ((current + delta) % len + len) % len;
        self.focused = Some(self.order[next as usize].clone());
    }

    /// Route Tab/BackTab to move focus. Ignored while an overlay owns input.
    pub fn handle(&mut self, event: &Event) -> EventFlow {
        if self.owner.is_some() {
            return EventFlow::Ignored;
        }
        let Event::Key(k) = event else {
            return EventFlow::Ignored;
        };
        match k.code {
            KeyCode::Tab if k.plain() => {
                self.advance(1);
                EventFlow::Consumed
            }
            KeyCode::BackTab => {
                self.advance(-1);
                EventFlow::Consumed
            }
            _ => EventFlow::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, EventFlow, Key, KeyCode};

    #[test]
    fn focus_tab_cycles_registered_regions() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("a");
        f.register("b");
        f.register("c");
        assert!(f.is_focused("a"));
        let tab = Event::Key(Key::new(KeyCode::Tab));
        assert_eq!(f.handle(&tab), EventFlow::Consumed);
        assert!(f.is_focused("b"));
        let back = Event::Key(Key::new(KeyCode::BackTab));
        f.handle(&back);
        assert!(f.is_focused("a"));
        // Wrap backwards.
        f.handle(&back);
        assert!(f.is_focused("c"));
    }

    #[test]
    fn overlay_owner_takes_input_and_blocks_tab() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("composer");
        f.set_owner("dialog");
        assert!(f.is_active("dialog"));
        assert!(!f.is_active("composer"));
        // Tab is swallowed while an overlay owns input.
        let tab = Event::Key(Key::new(KeyCode::Tab));
        assert_eq!(f.handle(&tab), EventFlow::Ignored);
        f.clear_owner();
        assert!(f.is_active("composer"));
    }

    #[test]
    fn removed_focus_target_falls_back_to_the_current_ring() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("old");
        assert_eq!(f.active(), Some("old"));

        f.begin_frame();
        f.register("new");
        assert_eq!(f.active(), Some("new"));
        assert!(f.is_focused("new"));

        // Starting another frame commits the fallback, so reintroducing the old
        // target cannot steal focus back.
        f.begin_frame();
        f.register("old");
        f.register("new");
        assert_eq!(f.active(), Some("new"));
    }

    #[test]
    fn requested_focus_applies_when_the_id_registers_and_then_lapses() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("dialog-ok");
        // The id belongs to a screen that renders next frame.
        f.request_focus("list");
        assert_eq!(f.focused(), Some("dialog-ok"));

        f.begin_frame();
        f.register("search");
        f.register("list");
        assert_eq!(f.focused(), Some("list"));

        // A request that the next frame does not satisfy lapses.
        f.request_focus("gone");
        f.begin_frame();
        f.register("search");
        f.register("list");
        f.begin_frame();
        f.register("gone");
        f.register("list");
        assert_eq!(f.focused(), Some("list"));

        // A registered id is focused at once, even under an overlay owner.
        f.set_owner("modal");
        f.request_focus("gone");
        assert_eq!(f.focused(), Some("gone"));
        assert_eq!(f.active(), Some("modal"));
    }

    #[test]
    fn programmatic_focus_accepts_only_registered_base_targets() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("left");
        f.register("right");
        assert!(f.focus("right"));
        assert!(f.is_active("right"));
        assert!(!f.focus("missing"));
        assert!(f.is_active("right"));

        let tab = Event::Key(Key::new(KeyCode::Tab));
        assert_eq!(f.handle(&tab), EventFlow::Consumed);
        assert!(
            f.is_active("left"),
            "registration order remains the tab ring"
        );
    }

    #[test]
    fn overlay_ownership_rejects_click_to_focus_requests() {
        let mut f = FocusRegistry::new();
        f.begin_frame();
        f.register("left");
        f.register("right");
        f.set_owner("dialog");
        assert!(!f.focus("right"));
        assert_eq!(f.active(), Some("dialog"));
        f.clear_owner();
        assert_eq!(f.active(), Some("left"));
    }
}
