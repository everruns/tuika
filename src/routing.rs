//! Input routing: turn the frame's focus and overlay ownership into delivery.
//!
//! [`FocusRegistry`] already answers *who is active* — an overlay owner beats
//! the base focus ring, and [`Scene::sync_focus`](crate::scene::Scene::sync_focus)
//! already resolves overlay z-order into that owner. What was missing is the
//! step after it: handing the event to that surface's state. Written by hand,
//! that step is one `if focus.is_active(id)` per surface **per event kind**, and
//! a second event kind added later silently skips the check — a paste reaching
//! the composer behind an open prompt, for instance.
//!
//! [`Router`] is that step, as a value scoped to one `update` call:
//!
//! - **Ownership gates delivery.** [`target`](Router::target) delivers only to
//!   the [`active`](FocusRegistry::active) surface. Reaching a surface that is
//!   *not* active is [`always`](Router::always) — explicit and greppable.
//! - **One route for every event kind.** Stages take the whole [`Event`], so a
//!   new variant reaches every registered surface without a host edit.
//! - **Precedence is declared, not re-derived.** Ownership comes from the
//!   registry the scene already synchronized; stages run in one fixed order.
//! - **Global chords compose.** [`pre_fn`](Router::pre_fn) wins over the active
//!   surface, [`fallback_fn`](Router::fallback_fn) runs only if nothing consumed
//!   the event — instead of a chord's position in an `if` chain.
//! - **Fallthrough is observable.** [`Delivery`] reports which surface received
//!   the event, at which [`RouteStage`], and what it answered.
//!
//! # Stages
//!
//! Stages are applied eagerly, in call order, and each is skipped once the event
//! has been consumed. Calls must follow the canonical order below; a host that
//! calls them out of order trips a debug assertion.
//!
//! | Order | RouteStage | Reaches |
//! |---|---|---|
//! | 1 | [`RouteStage::Pre`] | global chords, before any surface |
//! | 2 | [`RouteStage::Always`] | a named surface regardless of focus |
//! | 3 | [`RouteStage::Target`] | the active surface only |
//! | 4 | [`RouteStage::Fallback`] | global chords, only if nothing consumed |
//!
//! # Example
//!
//! An overlay owns input, so the composer behind it never sees the paste:
//!
//! ```
//! use tuika::prelude::*;
//! use tuika::routing::{Router, RouteStage};
//!
//! let mut composer = SingleLineInputState::new();
//! let mut prompt = SingleLineInputState::new();
//!
//! let mut focus = FocusRegistry::new();
//! focus.begin_frame();
//! focus.register("composer");
//! focus.set_owner("prompt"); // what `Scene::sync_focus` does for an overlay.
//!
//! let event = Event::Paste("hunter2".into());
//! let mut router = Router::new(&focus, &event);
//! router.target("prompt", &mut prompt);
//! router.target("composer", &mut composer);
//! let delivery = router.finish();
//!
//! assert_eq!(delivery.target, Some("prompt"));
//! assert_eq!(delivery.stage, RouteStage::Target);
//! assert_eq!(prompt.text(), "hunter2");
//! assert!(composer.text().is_empty());
//! ```
//!
//! Mouse events still resolve by geometry through
//! [`HitMap`](crate::mouse::HitMap): keys route by ownership, the pointer routes
//! by position, and a host that wants both feeds the resolved id into
//! [`FocusRegistry::focus`] before routing.

use crate::event::{Event, InputOutcome};
use crate::focus::FocusRegistry;

/// Which stage of the route delivered an event.
///
/// Ordered: stages must be called in this order, which is also how they are
/// compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum RouteStage {
    /// A global handler that runs before any surface.
    Pre,
    /// A surface reached regardless of focus.
    Always,
    /// The surface that owns input this frame.
    Target,
    /// A global handler that runs only when nothing consumed the event.
    Fallback,
    /// Nothing received the event.
    Undelivered,
}

/// What a route did with one event.
///
/// `target` and `stage` describe the last surface or handler the event was
/// *delivered* to, whether or not it recognized the event; `outcome` is what
/// that receiver answered. [`RouteStage::Undelivered`] with `target: None` means the
/// event reached nothing at all — the state a host wants to log rather than
/// silently drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delivery<'a> {
    /// The id of the last receiver, if it had one. Global stages have none.
    pub target: Option<&'a str>,
    /// The stage that delivered the event.
    pub stage: RouteStage,
    /// What the receiver answered.
    pub outcome: InputOutcome,
}

impl Delivery<'_> {
    /// Whether some receiver consumed the event.
    pub fn consumed(&self) -> bool {
        self.outcome.consumed()
    }

    /// Whether `id` received the event.
    pub fn reached(&self, id: &str) -> bool {
        self.target == Some(id)
    }
}

/// Host state that can receive a routed event.
///
/// Implemented for tuika's own single-`Event` input states. A state whose
/// `handle` needs frame context (a scroll extent, a viewport height) or a host
/// type that is not a tuika state routes through the `*_fn` stages instead, so
/// adopting the router never requires rewriting a surface first.
pub trait InputTarget {
    /// Handle one event, reporting whether it was recognized.
    fn handle(&mut self, event: &Event) -> InputOutcome;
}

macro_rules! impl_input_target {
    ($($ty:path),* $(,)?) => {
        $(impl InputTarget for $ty {
            fn handle(&mut self, event: &Event) -> InputOutcome {
                <$ty>::handle(self, event)
            }
        })*
    };
}

impl_input_target!(
    crate::components::CompletionState,
    crate::components::ConfirmDialogState,
    crate::components::InputDialogState,
    crate::components::SingleLineInputState,
    crate::components::SliderState,
    crate::components::TextInputState,
);

impl<T: InputTarget + ?Sized> InputTarget for &mut T {
    fn handle(&mut self, event: &Event) -> InputOutcome {
        (**self).handle(event)
    }
}

/// Routes one event through the frame's focus state.
///
/// Built per `update` call: it reads the [`FocusRegistry`] once, then borrows
/// each surface's state in turn — so a host keeps its state in plain fields
/// instead of `Rc<RefCell<_>>`, and a stage closure is free to take `&mut self`.
/// See the [module docs](self) for the model.
pub struct Router<'a, 'e> {
    /// The active id, copied at construction so the registry borrow ends there
    /// and a stage closure may take `&mut self` on the host.
    active: Option<String>,
    event: &'e Event,
    delivery: Delivery<'a>,
    last_stage: RouteStage,
}

impl<'a, 'e> Router<'a, 'e> {
    /// Start routing `event` against the frame's focus state.
    ///
    /// The registry is read here and not held, so `focus` may live on the same
    /// host struct a stage closure then mutates.
    pub fn new(focus: &FocusRegistry, event: &'e Event) -> Self {
        Self {
            active: focus.active().map(str::to_owned),
            event,
            delivery: Delivery {
                target: None,
                stage: RouteStage::Undelivered,
                outcome: InputOutcome::Ignored,
            },
            last_stage: RouteStage::Pre,
        }
    }

    /// The event being routed, for a stage closure that needs it.
    pub fn event(&self) -> &'e Event {
        self.event
    }

    /// What the route has done so far.
    pub fn delivery(&self) -> Delivery<'a> {
        self.delivery
    }

    /// Finish the route and report what happened.
    pub fn finish(self) -> Delivery<'a> {
        self.delivery
    }

    /// A global handler that sees the event before any surface — an interrupt
    /// or quit chord that must win over whatever owns input.
    ///
    /// ```
    /// use tuika::prelude::*;
    /// use tuika::keymap::{Dispatch, Keymap, Layer};
    /// use tuika::routing::{Router, RouteStage};
    ///
    /// #[derive(Clone, Debug, PartialEq)]
    /// enum Action { Quit }
    ///
    /// let mut keys = Keymap::new().layer(Layer::new("global").bind("ctrl+d", Action::Quit));
    /// let mut focus = FocusRegistry::new();
    /// focus.begin_frame();
    /// focus.register("composer");
    /// let mut composer = SingleLineInputState::new();
    ///
    /// let event = Event::Key(Key { code: KeyCode::Char('d'), ctrl: true, alt: false, shift: false });
    /// let mut command = None;
    /// let mut router = Router::new(&focus, &event);
    /// router.pre_fn(|event| match event {
    ///     Event::Key(key) => match keys.dispatch(*key) {
    ///         Dispatch::Command(action) => {
    ///             command = Some(action);
    ///             InputOutcome::Submitted
    ///         }
    ///         _ => InputOutcome::Ignored,
    ///     },
    ///     _ => InputOutcome::Ignored,
    /// });
    /// router.target("composer", &mut composer);
    ///
    /// assert_eq!(command, Some(Action::Quit));
    /// assert_eq!(router.finish().stage, RouteStage::Pre);
    /// assert!(composer.text().is_empty());
    /// ```
    pub fn pre_fn(&mut self, handler: impl FnOnce(&Event) -> InputOutcome) -> &mut Self {
        self.stage_fn(RouteStage::Pre, None, handler)
    }

    /// Deliver to `id` when it is the frame's active surface.
    pub fn target(&mut self, id: &'a str, target: &mut impl InputTarget) -> &mut Self {
        self.target_fn(id, |event| target.handle(event))
    }

    /// Deliver to `id` when it is active, through a closure — for a state whose
    /// `handle` needs frame context, or a host type that is not a tuika state.
    pub fn target_fn(
        &mut self,
        id: &'a str,
        handler: impl FnOnce(&Event) -> InputOutcome,
    ) -> &mut Self {
        if self.active.as_deref() != Some(id) {
            self.enter(RouteStage::Target);
            return self;
        }
        self.stage_fn(RouteStage::Target, Some(id), handler)
    }

    /// Deliver to `id` whether or not it is active — a transcript that scrolls
    /// while a dialog owns the keyboard, or a picker that takes precedence over
    /// the input under it without owning input outright.
    ///
    /// This is the deliberate hole in ownership: name it, so it is greppable
    /// rather than implied by an event kind that forgot to check focus. Such a
    /// surface gets its refusal *before* the active one, so it can claim the
    /// narrow set of events it exists for and leave the rest.
    pub fn always(&mut self, id: &'a str, target: &mut impl InputTarget) -> &mut Self {
        self.always_fn(id, |event| target.handle(event))
    }

    /// [`always`](Self::always) through a closure.
    pub fn always_fn(
        &mut self,
        id: &'a str,
        handler: impl FnOnce(&Event) -> InputOutcome,
    ) -> &mut Self {
        self.stage_fn(RouteStage::Always, Some(id), handler)
    }

    /// A global handler that runs only when nothing consumed the event.
    pub fn fallback_fn(&mut self, handler: impl FnOnce(&Event) -> InputOutcome) -> &mut Self {
        self.stage_fn(RouteStage::Fallback, None, handler)
    }

    fn stage_fn(
        &mut self,
        stage: RouteStage,
        id: Option<&'a str>,
        handler: impl FnOnce(&Event) -> InputOutcome,
    ) -> &mut Self {
        self.enter(stage);
        if self.delivery.consumed() {
            return self;
        }
        let outcome = handler(self.event);
        // A global stage that ignored the event delivered nothing, so it must
        // not mask a later `Undelivered`; a named surface did receive it.
        if id.is_some() || outcome.consumed() {
            self.delivery = Delivery {
                target: id,
                stage,
                outcome,
            };
        }
        self
    }

    fn enter(&mut self, stage: RouteStage) {
        debug_assert!(
            stage >= self.last_stage,
            "routing stages run in order: {:?} was called after {:?}",
            stage,
            self.last_stage
        );
        self.last_stage = stage;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::SingleLineInputState;
    use crate::event::{Key, KeyCode};

    fn frame(ids: &[&str]) -> FocusRegistry {
        let mut focus = FocusRegistry::new();
        focus.begin_frame();
        for id in ids {
            focus.register(*id);
        }
        focus
    }

    fn paste(text: &str) -> Event {
        Event::Paste(text.to_string())
    }

    fn ctrl(ch: char) -> Event {
        Event::Key(Key {
            code: KeyCode::Char(ch),
            ctrl: true,
            alt: false,
            shift: false,
        })
    }

    /// The yolop incident: an extension prompt owns input, and both a paste and
    /// the paste chord must reach it rather than the composer behind it.
    #[test]
    fn overlay_owner_receives_every_event_kind() {
        for event in [
            paste("hunter2"),
            ctrl('v'),
            Event::Key(Key::new(KeyCode::Char('x'))),
        ] {
            let mut composer = SingleLineInputState::new();
            let mut prompt = SingleLineInputState::new();
            let mut focus = frame(&["composer"]);
            focus.set_owner("prompt");

            let mut router = Router::new(&focus, &event);
            router.target("prompt", &mut prompt);
            router.target("composer", &mut composer);
            let delivery = router.finish();

            assert_eq!(delivery.target, Some("prompt"), "{event:?}");
            assert_eq!(delivery.stage, RouteStage::Target, "{event:?}");
            assert!(composer.text().is_empty(), "{event:?}");
        }
    }

    #[test]
    fn paste_reaches_the_owning_prompt_intact() {
        let mut composer = SingleLineInputState::new();
        let mut prompt = SingleLineInputState::new();
        let mut focus = frame(&["composer"]);
        focus.set_owner("prompt");

        let event = paste("secret-token");
        let mut router = Router::new(&focus, &event);
        router.target("prompt", &mut prompt);
        router.target("composer", &mut composer);
        let delivery = router.finish();

        assert!(delivery.consumed());
        assert_eq!(delivery.outcome, InputOutcome::Changed);
        assert_eq!(prompt.text(), "secret-token");
        assert!(composer.text().is_empty());
    }

    #[test]
    fn without_an_owner_the_focused_surface_receives() {
        let mut composer = SingleLineInputState::new();
        let mut prompt = SingleLineInputState::new();
        let focus = frame(&["composer"]);

        let event = paste("draft");
        let mut router = Router::new(&focus, &event);
        router.target("prompt", &mut prompt);
        router.target("composer", &mut composer);

        assert!(router.finish().reached("composer"));
        assert_eq!(composer.text(), "draft");
        assert!(prompt.text().is_empty());
    }

    #[test]
    fn nothing_recognized_the_event_is_observable() {
        let mut composer = SingleLineInputState::new();
        let focus = frame(&["composer"]);

        // A resize is routed like any other kind; no surface claims it.
        let event = Event::Resize {
            width: 10,
            height: 4,
        };
        let mut router = Router::new(&focus, &event);
        router.pre_fn(|_| InputOutcome::Ignored);
        router.target("composer", &mut composer);
        let delivery = router.finish();

        assert!(!delivery.consumed());
        assert_eq!(delivery.target, Some("composer"));
        assert_eq!(delivery.stage, RouteStage::Target);
    }

    #[test]
    fn an_unregistered_route_delivers_nothing() {
        let focus = frame(&["composer"]);
        let event = ctrl('v');
        let mut router = Router::new(&focus, &event);
        router.pre_fn(|_| InputOutcome::Ignored);
        router.fallback_fn(|_| InputOutcome::Ignored);
        let delivery = router.finish();

        assert_eq!(delivery.target, None);
        assert_eq!(delivery.stage, RouteStage::Undelivered);
    }

    #[test]
    fn pre_stage_wins_over_the_owning_surface() {
        let mut prompt = SingleLineInputState::new();
        let mut focus = frame(&["composer"]);
        focus.set_owner("prompt");

        let event = ctrl('c');
        let mut interrupted = false;
        let mut router = Router::new(&focus, &event);
        router.pre_fn(|_| {
            interrupted = true;
            InputOutcome::Cancelled
        });
        router.target("prompt", &mut prompt);
        let delivery = router.finish();

        assert!(interrupted);
        assert_eq!(delivery.stage, RouteStage::Pre);
        assert_eq!(delivery.target, None);
        assert!(prompt.text().is_empty());
    }

    #[test]
    fn fallback_runs_only_when_nothing_consumed() {
        let mut composer = SingleLineInputState::new();
        let focus = frame(&["composer"]);

        // Consumed by the composer: the fallback never runs.
        let typed = Event::Key(Key::new(KeyCode::Char('a')));
        let mut fell_through = false;
        let mut router = Router::new(&focus, &typed);
        router.target("composer", &mut composer);
        router.fallback_fn(|_| {
            fell_through = true;
            InputOutcome::Consumed
        });
        assert_eq!(router.finish().stage, RouteStage::Target);
        assert!(!fell_through);

        // Ignored by the composer: the fallback gets it.
        let page = Event::Key(Key::new(KeyCode::PageDown));
        let mut router = Router::new(&focus, &page);
        router.target("composer", &mut composer);
        router.fallback_fn(|_| {
            fell_through = true;
            InputOutcome::Consumed
        });
        assert_eq!(router.finish().stage, RouteStage::Fallback);
        assert!(fell_through);
    }

    #[test]
    fn always_reaches_a_surface_the_owner_left_inactive() {
        let mut prompt = SingleLineInputState::new();
        let mut focus = frame(&["transcript"]);
        focus.set_owner("prompt");

        let event = Event::Key(Key::new(KeyCode::PageDown));
        let mut scrolled = false;
        let mut router = Router::new(&focus, &event);
        router.always_fn("transcript", |_| {
            scrolled = true;
            InputOutcome::Changed
        });
        router.target("prompt", &mut prompt);
        let delivery = router.finish();

        assert!(scrolled);
        assert_eq!(delivery.target, Some("transcript"));
        assert_eq!(delivery.stage, RouteStage::Always);
    }

    #[test]
    fn a_consumed_event_stops_at_the_first_receiver() {
        let mut first = SingleLineInputState::new();
        let mut second = SingleLineInputState::new();
        let focus = frame(&["first"]);

        let event = paste("once");
        let mut router = Router::new(&focus, &event);
        router.always("second", &mut second);
        router.target("first", &mut first);

        assert!(router.finish().reached("second"));
        assert_eq!(second.text(), "once");
        assert!(first.text().is_empty());
    }

    /// A picker that takes precedence without owning input: it refuses first,
    /// and what it does not claim still reaches the active surface.
    #[test]
    fn an_always_surface_that_ignores_leaves_the_active_one_its_event() {
        let mut composer = SingleLineInputState::new();
        let focus = frame(&["composer"]);

        let typed = paste("filter");
        let mut picker_saw = 0;
        let mut router = Router::new(&focus, &typed);
        router.always_fn("picker", |_| {
            picker_saw += 1;
            InputOutcome::Ignored
        });
        router.target("composer", &mut composer);
        let delivery = router.finish();

        assert_eq!(picker_saw, 1);
        assert!(delivery.reached("composer"));
        assert_eq!(composer.text(), "filter");
    }

    #[test]
    #[should_panic(expected = "routing stages run in order")]
    fn stages_called_out_of_order_are_a_bug() {
        let focus = frame(&["composer"]);
        let event = ctrl('v');
        let mut router = Router::new(&focus, &event);
        router.fallback_fn(|_| InputOutcome::Ignored);
        router.pre_fn(|_| InputOutcome::Ignored);
    }
}
