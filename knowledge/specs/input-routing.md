---
type: Product Specification
title: Input routing
description: Defines how a frame's focus and overlay ownership decide which surface receives an event, and why delivery is a toolkit concern rather than host code.
---

# Input routing

## Why

tuika already models everything routing needs and stops one step short of it.
`FocusRegistry` records the base focus ring and an overlay's exclusive input
ownership; `Scene::sync_focus` resolves overlay z-order into that owner;
component states consume an `Event` and answer an `InputOutcome`. The step from
"the registry knows the overlay owns input" to "the overlay's state receives
this event" was host code — written once per host, per event kind, per surface.

That gap has a characteristic failure. Keys travel one host function and pastes
another; only the key path knows about a newly added overlay; a paste therefore
reaches the surface *behind* an open modal. It is not a bug in ownership, focus,
or the overlay — each was correct — but in the route, which existed twice. The
same shape recurs for any second event kind, and it pushes hosts toward
hand-rolled fields, because adopting a tuika input state buys nothing while
delivery is still hand-written.

Delivery is therefore a toolkit concern, on the same grounds as layout, focus,
and painting: given a frame's ownership state, tuika decides which state
receives an event.

## What routing owns

- **Ownership gates delivery.** The active surface — the overlay owner if there
  is one, else the focus ring's holder — is the only one a `target` stage
  reaches. `set_owner` stops being advisory for a host that routes.
- **One route for every event kind.** A stage receives the whole `Event`, so a
  new variant cannot create a per-host hole.
- **Precedence is declared, not re-derived.** It comes from the registry a scene
  already synchronized; the host names surfaces, not an ordering rule per event.
- **Global chords compose with routing.** A chord that must outrank the active
  surface, and one that only runs when nothing claimed the event, are distinct
  declared stages rather than positions in an `if` chain.
- **Fallthrough is explicit.** A `Delivery` names the receiver, the stage, and
  the outcome — including "nothing received this", the state that hides a hole.
- **It is testable without a terminal**, the way layout is.

## Boundaries

- Routing learns no host modes. Which surfaces exist, what owns input, and what
  a surface then does stay in the host; the toolkit resolves delivery only.
- It is not a retained widget tree with view-level bubbling. Stages are a flat,
  ordered list a host writes per update, and views remain ephemeral.
- The runner boundary is unchanged: routing happens inside `Application::update`
  and needs no signal, capability, or callback from the loop.
- A surface reachable while it does *not* own input is a declared exception
  (`always`), not an implicit one. Making the exception nameable is what keeps
  ownership meaningful; a transcript that scrolls under a modal, or a picker
  that claims its navigation keys and leaves typing to the input beneath it, are
  the cases that earn it. Such a surface refuses first, so it can take the narrow
  set of events it exists for without swallowing the rest.
- The pointer keeps resolving by geometry through `HitMap`. Keys route by
  ownership and the pointer by position; a host that wants a click to change
  focus resolves the id and calls `FocusRegistry::focus`, which keeps one
  meaning per mechanism.

## Design consequences

The router reads the registry at construction and does not hold it, so a stage
closure may take `&mut` on the same host struct the registry lives on. That is
what keeps host state in plain fields instead of `Rc<RefCell<_>>` — the property
that decides whether a routing API is adoptable at all.

Stages run eagerly and in a fixed order (global, declared exceptions, the active
surface, then last-chance globals); calling them out of order is a host bug and
trips a debug assertion rather than silently reordering precedence.

A stage decides; the host applies afterwards. Quitting, submitting, or opening
another surface from inside a stage would require a borrow of the whole
application mid-route, so the recommended shape is a stage that records intent
and a host that acts on it once the route is finished.

## Related

- [Architecture](architecture.md) — focus registry, overlays, and the host boundary.
- [Keymap](keymap.md) — what a global stage dispatches.
- [Public API surface](api-surface.md) — why `routing` sits on the crate root.
