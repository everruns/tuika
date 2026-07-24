---
type: Product Specification
title: Keymap engine
description: Defines tuika's declarative key-binding engine — chords, sequences, layers, and dispatch — and why key routing is a toolkit concern.
---

# Keymap engine

## Why

Key handling in a TUI typically grows as a hand-rolled cascade of `match
key.code` blocks in the event loop. Global chord shortcuts end up scattered
inline alongside modal handlers, so *what a key does* is neither declared in one
place nor discoverable for a help surface. Adding a shortcut means editing an
imperative branch and remembering its ordering relative to every modal guard.

Key routing is a toolkit concern in the same way layout, overlays, and focus
are — the problem [OpenTUI's keymap](https://opentui.com/docs/keymap/overview/)
solves for the browser and terminal. tuika owns a declarative binding engine so
any host declares its shortcuts once and dispatches through a single seam.

## What

A host-agnostic engine (`tuika::keymap`) that resolves declared key bindings to
named command values:

- **Chords and sequences.** A `Chord` is one key press plus modifiers, parsed
  from a string (`ctrl+r`, `alt+shift+tab`, `?`, `space`, literal `ctrl++`). A
  `KeySequence` is one or more chords typed in order, written space-separated
  (`g g`, `ctrl+x s`), so multi-stroke bindings are first-class.
- **Layers.** Bindings are grouped into named, prioritized `Layer`s. A layer may
  be *gated* on runtime data (`when("mode", "panel")`) so it is active only in a
  given application mode; an ungated layer is always active. When two active
  layers bind the same sequence, the higher priority wins.
- **Dispatch.** `Keymap::dispatch` takes a translated `Key` and returns
  `Command(c)`, `Pending` (the strokes so far are a live prefix of a longer
  binding), or `Unmatched` (the host should handle the key itself).
- **Query.** `Keymap::hints` lists the currently-active bindings — key label,
  optional help text, command — for a help overlay or `KeyHints` row.

## Design

### Built on translated events, not crossterm

The engine consumes tuika's own `Key` — the terminal-independent event the host
already translates crossterm into — never a crossterm type. That keeps the
engine free of terminal I/O, so it is unit-testable without a PTY: the same
boundary that lets the rest of the widget layer be tested against synthetic
events.

### Modifier normalization mirrors how terminals report input

Matching is on a normalized `Chord`, so a binding matches the event a terminal
actually delivers:

- For a character key, Shift is folded into the character itself (`?` arrives as
  `Char('?')`, not `Shift`+`Char('/')`), so the `shift` flag is dropped for
  characters and kept meaningful only for non-character keys (`Shift`+`Enter`).
- `Shift`+`Tab` folds to the distinct `BackTab` key that terminals report.

Both the string parser and `Chord::from_key` apply the same normalization, so a
parsed binding and a live event agree.

### Sequence resolution: exact wins, dead-ends retry

`dispatch` accumulates strokes into a pending buffer. On each key it resolves the
pending buffer against the active layers: an exact match fires immediately and
clears the buffer, even when the buffer is also the prefix of a longer binding
(so a bound `g` fires without waiting to see whether `g g` follows — overlapping
bindings are authored with that in mind). A live prefix with no exact match
returns `Pending`. A buffer that matches nothing is dropped, and the final stroke
is retried on its own so it can begin a fresh sequence. Changing runtime data
clears the pending buffer, since the bindings that could complete it may no
longer be active.

### Bindings are static, so parse errors panic by construction

`Layer::bind` panics on a malformed key spec, because bindings are authored as
static literals — a bad spec is a programmer error caught on first run, like a
malformed regex literal. `Chord::parse` / `Layer::try_bind` return a `Result` for
the rare dynamic case (config-sourced bindings).

### Precedence is the host's to choose

The engine resolves bindings; it does not decide when a host consults it. A host
that wants global chords to fire in any mode dispatches ahead of its modal
guards; one that wants modals to win dispatches after. Both are correct, and
neither requires the engine to model modality beyond gated layers.

## Non-goals

- No key *decoding* in the engine — the host translates crossterm into tuika
  `Key` events first, exactly as the rest of tuika consumes input.
- No sequence-timeout clock. Pending state is cleared explicitly (a completed or
  dead-ended sequence, or a mode change); a host that wants a timeout drives
  `Keymap::reset` from its own tick.
- No user-facing rebinding or config format. The engine supports dynamically
  sourced bindings (`try_bind`), but parsing a user's keymap file is the host's
  job — see [goal.md](./goal.md).

## Public surface

- [`docs/keymap.md`](../../docs/keymap.md)
