---
title: Keymap
description: Declare, dispatch, and query host-agnostic key bindings, chords, sequences, and mode-gated layers.
sidebar:
  order: 5
---

# Keymap

A host-agnostic binding engine: declare an application's key bindings once, as
data, and resolve incoming key presses to named commands. It sits beside the
[component gallery](components.md) and [terminal features](features.md) as a
third kind of tuika primitive — not a widget and not an escape sequence, but the
routing layer between input and behavior.

It follows the **register → dispatch → query** shape of
[OpenTUI's keymap](https://opentui.com/docs/keymap/overview/), adapted to
idiomatic Rust and to tuika's own [`Key`] events. Because it consumes the
already-translated `Key` — never a terminal type — the whole engine runs and
unit-tests without a PTY.
[API](https://docs.rs/tuika/latest/tuika/keymap/index.html)

## The model

Four types, smallest to largest:

- **`Chord`** — one key press plus its modifiers (`ctrl+r`, `alt+shift+tab`,
  `?`, `A`, `space`, the literal `ctrl++`). Character tokens are exact logical
  text, after the active keyboard layout. Parse a chord from a string with
  `Chord::parse`, or build it from a live event with `Chord::from_key`.
- **`KeySequence`** — one *or more* chords typed in order, written
  space-separated (`g g`, `ctrl+x s`). A single chord is a one-element sequence,
  so every binding is a sequence underneath.
- **`Layer`** — a named, prioritized group of bindings. A layer may be *gated*
  on runtime data (`when("mode", "search")`) so it is active only in a given
  application mode; an ungated layer is always active.
- **`Keymap<C>`** — the engine. It owns the layers, a small runtime-data store
  used to gate them, and the pending multi-stroke state. `C` is your command
  type — usually an enum.

## Defining bindings

Bindings are built declaratively and resolve to a command value of your choosing:

```rust
use tuika::keymap::{Keymap, Layer};

#[derive(Clone, Debug, PartialEq)]
enum Action { Search, Quit, Top, Close }

let mut keymap = Keymap::new()
    .layer(
        Layer::new("global")
            .bind_labeled("ctrl+r", "search", Action::Search)
            .bind("ctrl+d", Action::Quit)
            // A two-stroke sequence, vim-style.
            .bind("g g", Action::Top),
    );
```

`bind` (and `bind_labeled`, which also carries a help string) panics on a
malformed spec, because bindings are authored as static literals — a bad spec is
a programmer error caught on first run, like a malformed regex literal. For
config-sourced specs, use `Chord::parse` / `Layer::try_bind`, which return a
`Result`.

## Dispatching

Feed each translated `Key` to `dispatch`. It returns one of three outcomes:

```rust
use tuika::event::{Key, KeyCode};
use tuika::keymap::Dispatch;

let ctrl_r = Key { code: KeyCode::Char('r'), ctrl: true, alt: false, shift: false };
match keymap.dispatch(ctrl_r) {
    Dispatch::Command(action) => { /* run it */ }
    Dispatch::Pending         => { /* a multi-stroke sequence is mid-flight */ }
    Dispatch::Unmatched       => { /* no binding — handle the key yourself */ }
    Dispatch::Consumed        => { /* a key mode used the key — ignore it */ }
}
```

- **Single-stroke** bindings resolve immediately.
- **Multi-stroke** sequences accumulate: the first `g` of `g g` returns
  `Pending`, the second returns `Command(Top)`.
- An **exact** match wins immediately even when it is also the prefix of a
  longer binding — a bound `g` fires without waiting to see whether `g g`
  follows, so author overlapping bindings with that in mind.
- A sequence that **dead-ends** is dropped, and its final stroke is retried on
  its own so it can begin a fresh sequence. `Keymap::reset` abandons a pending
  sequence explicitly (e.g. on a focus change or a timeout tick — the engine
  keeps no clock of its own).

A key that no active binding matches returns `Unmatched`, so the host stays in
control of everything the keymap does not claim (typing into a text field,
say).

### Logical text and modifier normalization

Matching is on a normalized chord, so a binding matches the event a terminal
actually delivers. A character key is the exact logical Unicode character
produced by the active keyboard layout: `?` arrives as `Char('?')`, not
`Shift`+`Char('/')`. Bind `?`, `A`, or `ctrl+R` directly. This is portable to
non-US layouts because the keymap never guesses which physical key plus Shift
produces that text.

Accordingly, `shift+character` specs are rejected instead of silently losing
Shift; `Chord::parse` and `Layer::try_bind` return `KeyParseError`, while static
`Layer::bind` fails fast as usual. Shift remains a distinct modifier for
non-character keys (`Shift+Enter`). `Shift`+`Tab` folds to the distinct
`BackTab` key terminals report. `Chord::from_key` drops any separately reported
Shift flag from character events because its effect is already present in the
character. Parsed bindings and live events therefore share one layout-neutral
identity.

Shortcuts keep working on a non-Latin layout. With a Ukrainian, Russian,
Belarusian, Greek, or Hebrew layout active, Ctrl+C's key arrives as Ctrl+`с`;
when a Ctrl or Alt chord matches no binding, the keymap retries it as the US
QWERTY key in the same position, so `ctrl+c` still fires. An explicit binding
for the non-Latin chord wins, and plain characters are never translated, since
they are text. `Key::base_layout` exposes the same mapping, and
`Key::shortcut` applies it to Ctrl/Alt chords for code that matches keys by
hand.

## Modes: layers gated on runtime data

`set_data` drives which layers are active, mirroring an application's modes. A
layer's `when` clauses must all match the current data for it to be active;
changing data recomputes activation and clears any now-impossible pending
sequence.

```rust
let mut keymap = Keymap::new()
    .layer(Layer::new("global").bind("ctrl+d", Action::Quit))
    .layer(
        Layer::new("panel")
            .when("mode", "panel")
            .bind("q", Action::Close),
    );

// `q` does nothing until the app enters panel mode…
keymap.set_data("mode", "panel");
// …now it closes the panel, while ctrl+d still quits in every mode.
```

When two active layers bind the same sequence, the higher `priority` wins, so an
overlay layer can shadow a global binding while it is up.

## Querying for help

`hints()` lists the currently-active bindings — key label, optional help text,
command, layer name, and layer priority. The component adapters consume the same
declarations directly, so dispatch, a responsive footer, and a complete help
screen cannot drift apart:

```rust
use tuika::components::{KeyHints, KeymapHelp};

let footer = KeyHints::from_keymap(&keymap);
let help = KeymapHelp::from_keymap(&keymap);
```

`KeyHints` fits only complete hints: it keeps higher-priority layer bindings
first on narrow screens and never clips halfway through a key/action pair.
`KeymapHelp::offset` supports a vertically scrollable help overlay. Because both
query active bindings, changing keymap runtime data updates the displayed mode.

## Key modes

A key mode is a named set of bindings that apply only while the mode is
entered: vim's normal mode, tmux's prefix table, a "resize" mode where arrows
resize instead of move. Bind in a mode by prefixing the spec with its name and
a slash, or put a whole layer in one with `Layer::mode`:

```rust
use tuika::keymap::Mode;

let mut keymap = Keymap::new()
    .layer(
        Layer::new("global")
            .bind("ctrl+w", Action::EnterResize)
            .bind("resize/left", Action::Shrink)
            .bind("resize/right", Action::Grow)
            .bind("resize/enter", Action::Done),
    )
    .mode(Mode::new("resize").exclusive().block("q"));

// The host enters and leaves modes while running commands:
// Action::EnterResize => keymap.enter_mode("resize"),
// Action::Done        => { keymap.exit_mode(); }
```

- **A stack.** `enter_mode` pushes; the newest mode is checked first.
  `exit_mode` leaves only that one and `exit_all_modes` leaves them all.
  `current_mode` and `active_modes` feed a mode indicator.
- **Fall-through.** A key the mode does not bind goes to the modes below it and
  then to the base keymap, so typing still works.
- **Exclusive.** With `Mode::exclusive`, only the mode's bindings work while
  it is on top; every other key returns `Dispatch::Consumed`.
- **Escape.** A bare Escape leaves the top mode and returns
  `Dispatch::Consumed`, unless the mode binds Escape itself or sets
  `escape_exits(false)`.
- **Blocked keys.** `Mode::block("q")` makes a key do nothing inside the mode,
  even though the base keymap binds it.
- **Once.** `enter_mode_once` leaves the mode after the next complete key —
  whatever that key did — which is the tmux prefix-key pattern.

Mode names are lowercase letters, digits, `_`, and `-`, so the slash key itself
(`/`, `ctrl+/`) is never read as a mode. A mode needs no `Mode` registration
unless you change its options. `hints()` leads with the top mode's bindings
and drops keys a mode blocks, so a footer follows the mode automatically.

## A leader key

Bind `<leader>` at the start of a spec and set the leader on the keymap. The
leader resolves at dispatch time, so changing it rebinds every leader sequence
at once, and a help surface shows the real chord:

```rust
let mut keymap = Keymap::new()
    .leader("space")
    .layer(Layer::new("global").bind_labeled("<leader> f", "find", Action::Search));
// Space then F runs Search; the hint reads "Space F".
```

Until a leader is set, leader bindings never fire and are left out of `hints()`.

## A command catalog

Bindings say which keys run a command; the catalog says what the command *is*,
bound or not. Register each command once with a stable id and a title people
search for, and a palette or help screen reads it back:

```rust
use tuika::keymap::CommandInfo;

let keymap = keymap
    .command(
        CommandInfo::new("app.search", "Search messages", Action::Search)
            .category("Navigation")
            .keyword("find"),
    )
    .command(CommandInfo::new("app.quit", "Quit", Action::Quit).description("Exit the app"));

for found in keymap.search_commands("fin") {
    // found.info.title == "Search messages", found.keys == ["Ctrl+R", "Space F"]
}
```

`search_commands` uses the same fuzzy ranking as the completion popup: a match
on the title ranks first, then the id, keywords, category, and description.
`keys_for` lists the active keys for one command. Registering an id twice
replaces the earlier entry.

## Diagnostics and the dump

Some binding mistakes only show up as a key that silently does nothing.
`diagnostics()` finds them statically, across every runtime-data value:

- **Duplicate** — the same keys twice in one layer; the later never fires.
- **Shadowed** — a winning layer that is active whenever this one is binds the
  same keys. A *gated* overlay overriding a global key is intended and is not
  reported.
- **Ambiguous** — two layers that can be active together bind the same keys at
  the same priority, so only declaration order picks the winner.
- **Unreachable** — a shorter binding is a prefix (`g` and `g g`), and exact
  matches fire at once.
- **MissingLeader** — a `<leader>` binding with no leader set.

Each diagnostic names both bindings with the source line that declared them
(`bind` records its caller), so a test can assert a keymap is clean:

```rust
assert!(keymap.diagnostics().is_empty(), "{}", keymap.dump());
```

`dump()` prints every binding grouped by layer — keys, label, command, source
line — and then the diagnostics:

```text
leader: Space

layer "global"  priority 0  always active
  Ctrl+R   search  Search  src/app.rs:12:10
  Space F  find    Search  src/app.rs:13:10

layer "panel"  priority 10  when mode=panel  (inactive)
  Q        close   Close   src/app.rs:17:10
```

`bindings()` returns the same information as data.

## Testing

The engine never touches the terminal, so behavior is tested by feeding
synthetic `Key`s and asserting the `Dispatch` outcome — no raw mode, no PTY:

```rust
let mut keymap = Keymap::new().layer(Layer::new("nav").bind("g g", Action::Top));
let g = Key::new(KeyCode::Char('g'));
assert_eq!(keymap.dispatch(g), Dispatch::Pending);
assert_eq!(keymap.dispatch(g), Dispatch::Command(Action::Top));
```

[`Key`]: https://docs.rs/tuika/latest/tuika/event/struct.Key.html
