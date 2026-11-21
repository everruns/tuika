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
any host declares its shortcuts once and dispatches through a single boundary.

## What

A host-agnostic engine (`tuika::keymap`) that resolves declared key bindings to
named command values:

- **Chords and sequences.** A `Chord` is one key press plus modifiers, parsed
  from a string (`ctrl+r`, `alt+shift+tab`, `?`, `A`, `space`, literal
  `ctrl++`). Character tokens are exact logical text produced by the active
  layout. A `KeySequence` is one or more chords typed in order, written
  space-separated (`g g`, `ctrl+x s`), so multi-stroke bindings are
  first-class.
- **Layers.** Bindings are grouped into named, prioritized `Layer`s. A layer may
  be *gated* on runtime data (`when("mode", "panel")`) so it is active only in a
  given application mode; an ungated layer is always active. When two active
  layers bind the same sequence, the higher priority wins.
- **Dispatch.** `Keymap::dispatch` takes a translated `Key` and returns
  `Command(c)`, `Pending` (the strokes so far are a live prefix of a longer
  binding), `Unmatched` (the host should handle the key itself), or
  `Consumed` (a key mode used the key; the host should ignore it).
- **Query.** `Keymap::hints` lists the currently-active bindings — key label,
  optional help text, command — for a help overlay or `KeyHints` row.
- **Catalog.** `CommandInfo` entries (stable id, title, description, category,
  keywords) describe every command, bound or not, and `search_commands` ranks
  them for a palette with the keys that currently run each one.
- **Diagnostics and dump.** `diagnostics` statically finds bindings that can
  never fire and equal-priority conflicts; `dump` prints every binding with
  the source line that declared it.
- **Leader.** A spec may start with `<leader>`, resolved to the keymap's
  leader chord.
- **Key modes.** A spec may start with `mode/` to bind inside a named key mode.
  The keymap keeps a mode stack (`enter_mode`, `enter_mode_once`,
  `exit_mode`); modes can be exclusive, block keys, and are left with Escape by
  default. `Dispatch::Consumed` reports a key the keymap used itself.

## Design

### Built on translated events, not crossterm

The engine consumes tuika's own `Key` — the terminal-independent event the host
already translates crossterm into — never a crossterm type. That keeps the
engine free of terminal I/O, so it is unit-testable without a PTY: the same
boundary that lets the rest of the widget layer be tested against synthetic
events.

### Character bindings are logical text

Matching is on a normalized `Chord`, so a binding matches the event a terminal
actually delivers:

- A character key is the exact Unicode character produced by the active
  keyboard layout (`?` arrives as `Char('?')`, not `Shift`+`Char('/')`). A
  binding spells that result directly (`?`, `A`, `ctrl+R`) rather than guessing
  a physical Shift combination. This keeps bindings portable across layouts.
- A `shift+character` spec is rejected because its result is layout policy;
  Shift is kept meaningful for non-character keys (`Shift+Enter`). A translated
  character event's Shift flag is dropped because its effect is already present
  in the logical character.
- `Shift`+`Tab` folds to the distinct `BackTab` key that terminals report.

Both the string parser and `Chord::from_key` apply the same normalization, so a
parsed binding and a live event agree.

### Shortcuts survive a non-Latin layout

Logical text has one cost: on a Cyrillic, Greek, or Hebrew layout, Ctrl+C's key
produces Ctrl+`с`, and a `ctrl+c` binding would miss until the user switched
layout. So a Ctrl or Alt chord that resolves to nothing is retried as the US
QWERTY key in the same position (`Key::base_layout`), stroke by stroke inside a
sequence. The logical chord is always tried first, so an explicit `ctrl+с`
binding still wins. Plain characters are never translated: they are text, and
`й` must not fire a `q` binding. The built-in components' fixed Ctrl/Alt
shortcuts (`TextInput` editing, `Ctrl+N`/`Ctrl+P` navigation) use the same rule
through `Key::shortcut`.

The Kitty keyboard protocol reports the true base-layout key for every layout,
and tuika already asks for it (`REPORT_ALTERNATE_KEYS`), but crossterm's
decoder discards that field. The built-in table stands in until tuika decodes
`CSI u` itself; the matching rule does not change when it does.

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

### The catalog is separate from the bindings

A binding answers "what does this key do"; a command palette asks the reverse,
and also needs commands no key reaches. So the catalog is its own list keyed by
a stable string id, joined to bindings by command equality only when a caller
asks for keys (`keys_for`, `search_commands` — the only methods that need
`C: PartialEq`). Search reuses the completion popup's fuzzy ranking so every
searchable surface in tuika orders a query the same way.

### Diagnostics are static, and overrides are not errors

`diagnostics` reasons over gates rather than current data: two layers can be
active together unless they gate one key on different values, and a layer is
active whenever another is when its gates are a subset. That separates the
intended pattern — a gated overlay at higher priority overriding a global key —
from a dead binding, where the winner is active whenever the loser is. Only
the dead case, an equal-priority tie (decided by declaration order alone), a
prefix that starves a longer sequence, and a `<leader>` spec with no leader are
reported. Bindings record `#[track_caller]` locations so each report names the
source lines on both sides, in the spirit of Rex's `rex keymap` source column.

### Modes are a stack of levels, not more gated layers

Gated layers already express "active in mode X", but not what Rex-style modes
need: an order between modes (the newest wins), keys that must *not* reach the
host (exclusive modes, blocked keys), and a mode that ends itself (once). So a
binding carries an optional mode, and dispatch walks levels — the active modes
newest first, then the base keymap — taking the first level with an exact match
or a live prefix. An exclusive mode or a blocked first stroke ends the walk
with `Consumed`. Gates still apply inside every level, so the two compose.

Mode entry is the host's call, made while it runs its own command: the engine
has no built-in "enter mode" command, because `C` is the host's type. Escape
leaving a mode and a once-mode ending are the only stack changes the engine
makes itself, and both are the documented Rex defaults. `Consumed` is a new
`Dispatch` variant rather than a reused `Pending` or `Unmatched`, because the
host must neither wait for more keys nor handle this one.

### The leader resolves at dispatch time

`<leader>` is stored as a flag on the binding, not substituted at parse time,
so layers can be built before the keymap that owns them and changing the leader
rebinds every leader sequence at once. Without a leader those bindings are
inert and hidden from `hints`.

### Static binding parse errors fail fast

`Layer::bind` panics on a malformed or layout-dependent key spec, because
bindings are authored as static literals — a bad spec is a programmer error
caught on first run, like a malformed regex literal. `Chord::parse` /
`Layer::try_bind` return a `Result` for the rare dynamic case (config-sourced
bindings), including an explanation for rejected `shift+character` input.

### Precedence is the host's to choose

The engine resolves bindings; it does not decide when a host consults it. A host
that wants global chords to fire in any mode dispatches ahead of its modal
guards; one that wants modals to win dispatches after. Both are correct, and
neither requires the engine to model modality beyond gated layers and key
modes.

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
