---
title: Input routing
description: Deliver every event kind to the surface that owns input, with ownership, precedence, and fallthrough declared once.
sidebar:
  order: 6
---

# Input routing

A frame usually has more than one place an event could go: a composer, a
picker over it, a modal that just opened, a transcript that scrolls behind
everything. Deciding **which one receives this event** is policy, and tuika owns
it the way it owns layout and focus.

[`FocusRegistry`](https://docs.rs/tuika/latest/tuika/focus/struct.FocusRegistry.html)
already answers *who is active* — an overlay owner beats the base focus ring —
and `Scene::sync_focus` resolves overlay z-order into that owner.
[`Router`](https://docs.rs/tuika/latest/tuika/routing/struct.Router.html) is the
step after it: given that state, it hands the event to the right surface's state,
for **every** event kind at once.
[API](https://docs.rs/tuika/latest/tuika/routing/index.html)

## Why it is a toolkit concern

Written by hand, the step is one `if focus.is_active("composer")` per surface,
per event kind. That scales badly in a specific and dangerous way: keys go
through one function and pastes through another, only the key path learns about
a new overlay, and a pasted secret lands in the composer *behind* an open
prompt. The overlay was correct, the registry was correct, and the event still
went to the wrong place — because the route was host code written twice.

One route removes the second path:

- **Ownership gates delivery.** `target` reaches only the active surface.
- **One route per event kind.** Stages take the whole `Event`, so a new variant
  cannot miss a surface.
- **Precedence is declared, not re-derived.** It comes from the registry the
  scene already synchronized.
- **Global chords compose.** They are stages, not a position in an `if` chain.
- **Fallthrough is observable.** `Delivery` says who received what.
- **It tests without a terminal**, like layout.

## The stages

A `Router` is built per update, and stages run eagerly in this order. Each is
skipped once something has consumed the event; calling them out of order trips a
debug assertion.

| Stage | Reaches | Typical use |
|---|---|---|
| `pre_fn` | everything, first | interrupt and quit chords |
| `always` / `always_fn` | a named surface, focused or not | a transcript that keeps scrolling; a picker that claims its navigation keys and leaves the rest |
| `target` / `target_fn` | the active surface only | the composer, the open dialog |
| `fallback_fn` | everything, last | navigation and help, when nothing claimed the event |

`always` is the deliberate hole in ownership. It is a named call rather than an
implied exception, so "what reaches a surface that does not own input" is
greppable.

## Routing an event

```rust
use tuika::prelude::*;

fn update(app: &mut App, event: &Event) {
    // 1. Declare the frame's ownership. A host whose modal is an overlay gets
    //    this from `Scene::sync_focus`; declare it directly otherwise.
    app.focus.begin_frame();
    app.focus.register("composer");
    if app.dialog.is_some() {
        app.focus.set_owner("dialog");
    } else {
        app.focus.clear_owner();
    }

    // 2. Route. One registration per surface, covering every event kind.
    let mut quit = false;
    let mut router = Router::new(&app.focus, event);
    router.pre_fn(|event| match event {
        Event::Key(key) if key.ctrl && key.code == KeyCode::Char('c') => {
            quit = true;
            InputOutcome::Cancelled
        }
        _ => InputOutcome::Ignored,
    });
    router.always_fn("transcript", |event| {
        app.scroll.handle(event, app.content_h, app.viewport_h)
    });
    router.target_fn("dialog", |event| app.handle_dialog(event));
    router.target("composer", &mut app.composer);
    let delivery = router.finish();

    // 3. Apply what the stages decided.
    if quit { app.quit(); }
    if !delivery.consumed() {
        // Nobody claimed it — a state a host can log instead of losing.
    }
}
```

`Router::new` reads the registry and does not hold it, so a stage closure is free
to take `&mut` on the same host struct the registry lives on.

### Targets

`target` and `always` take any
[`InputTarget`](https://docs.rs/tuika/latest/tuika/routing/trait.InputTarget.html)
— implemented for tuika's single-`Event` input states (`TextInputState`,
`SingleLineInputState`, `CompletionState`, `ConfirmDialogState`,
`InputDialogState`, `SliderState`). A state whose `handle` needs frame context
(`ScrollState` wants the content and viewport heights) or a host type that is not
a tuika state routes through the `*_fn` form instead, so adopting the router
never requires rewriting a surface first.

### Stages decide, the host applies

Keep a stage closure to the surface it routes to, and apply anything wider — quit,
submit, opening another surface — after `finish()`. It keeps a stage from needing
a borrow of the whole application mid-route, and keeps the route readable as a
list of surfaces.

## What a `Delivery` reports

```rust
let delivery = router.finish();
delivery.target;              // Some("dialog") — the last receiver
delivery.stage;               // RouteStage::Target
delivery.outcome;             // InputOutcome::Changed
delivery.consumed();          // true
delivery.reached("composer"); // false
```

`RouteStage::Undelivered` with no target means the event reached nothing at all.
That is the case worth logging: an event that vanishes is how a routing hole
announces itself.

## Testing a route

Routing asserts like layout does — no terminal, no rendering:

```rust
let mut composer = SingleLineInputState::new();
let mut prompt = SingleLineInputState::new();

let mut focus = FocusRegistry::new();
focus.begin_frame();
focus.register("composer");
focus.set_owner("prompt");

let event = Event::Paste("secret".into());
let mut router = Router::new(&focus, &event);
router.target("prompt", &mut prompt);
router.target("composer", &mut composer);

assert!(router.finish().reached("prompt"));
assert_eq!(prompt.text(), "secret");
assert!(composer.text().is_empty());
```

## Mouse

The pointer still resolves by geometry through
[`HitMap`](https://docs.rs/tuika/latest/tuika/mouse/struct.HitMap.html): keys
route by ownership, the pointer routes by position. A host that wants a click to
move focus resolves the id from the hit map, calls `FocusRegistry::focus`, and
routes from there.

## See also

- [Keymap](keymap.md) — turning key presses into named commands, which a `pre_fn`
  or `fallback_fn` stage dispatches.
- [`examples/codex`](https://github.com/everruns/tuika/tree/v0.11.0/examples/codex) —
  a full host: a modal, a picker, a composer, and a transcript on one route.
