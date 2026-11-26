---
type: Architecture Specification
title: Out-of-band terminal escapes
description: Defines how tuika emits escapes that live outside the cell buffer — hyperlinks, clipboard, native progress — and the rule that separates the safe ones from the ones needing capability detection.
---

# Out-of-band terminal escapes

## Why

A cell carries one grapheme plus a style. Several things a modern
terminal can do are not expressible that way: making a text run clickable,
writing the system clipboard, driving the window's own progress indicator, or
painting pixels. Each needs an escape sequence that lives *outside* the cell
buffer, which the frame diff knows nothing about.

Emitting them naively fights the diff: writes appear at the wrong cursor
position, or get overwritten on the next frame, or leave the cursor somewhere
the renderer did not expect.

## What

Out-of-band capabilities, in three families:

| Capability | Sequence | Module | Emission point |
| --- | --- | --- | --- |
| Hyperlinks | OSC 8 | `term::hyperlink` | embedded by semantic prose components or spliced into arbitrary drawn runs by `HyperlinkBackend` |
| Clipboard | OSC 52 | `term::clipboard` | host- or runner-initiated, any time |
| Desktop notification | OSC 9 / OSC 777 | `term::attention` | host-initiated, opt-in; inbox fallback stays host-owned |
| Focus reporting | CSI ? 1004 | `term::attention::FocusReporting` | host-initiated scope; drop disables reporting |
| Native progress | OSC 9;4 | `term::progress` | host-initiated, any time |
| Program status | OSC 7501 | `term::program_status` | host-initiated, or runner-owned from one status value |
| Program resume | OSC 7577 | `term::program_resume` | host-initiated on state change; only when the terminal provided a token |
| Pointer shape | OSC 22 | `term::pointer` | host-initiated, any time |
| Window title | OSC 2, CSI 22/23 t | `term::title` | host-initiated, or runner-owned from one title value |
| Images | Kitty / iTerm2 / Sixel | `term::image` | after `terminal.draw()` returns |
| Sized text | OSC 66 | `term::text_size` | embedded in cells by `apply_sized_runs_in` |
| Terminal queries | DA1, Kitty keyboard (`CSI ? u`), OSC 10 / 11 / 4, OSC 7501 `?` | `term::capabilities`, `term::palette`, `term::program_status` | host-initiated, once at startup, in raw mode |

They live together under `term` because they are one kind of thing, and a reader
who has understood one has understood the shape of all of them. Each exposes a
pure `encode` that builds the sequence without touching I/O — that is what makes
the wire format unit-testable without a terminal — plus a thin writer over
`impl Write`. A capability that needs per-frame state owns a driver instead
(`term::progress::TerminalProgress`).

The last row is the third family, and the one that breaks the shared shape: the
first five *tell* the terminal something, while a query *asks*, so it is the only
one with a reply — and the reply lands on stdin. `term::capabilities` asks what
the terminal supports; `term::palette` asks what colors it was configured with.

Images are the exception that proves the grouping: the *protocol* half is here,
but the `Image` view is a component like any other, because reserving cells is a
cell-grid concern. The split is deliberate — see [images.md](./images.md).

## Design

### Cursor-neutral escapes can be spliced; graphics cannot

OSC 8, 52, and 9;4 do not move the cursor. That is what lets `HyperlinkBackend`
wrap a URL run *inside* the byte stream the backend is already writing: the diff's
cursor model is unaffected, so nothing downstream needs to know.

Graphics escapes are not cursor-neutral — Kitty places the image at the cursor —
so they cannot be spliced. They are emitted after the frame, wrapped in a cursor
save/restore with an explicit CUP to each image's cell origin, leaving the net
effect on tuika's cursor model at nil. See [images.md](./images.md).

### Program resume is signed, so the token is the capability test

OSC 7577 records a command the terminal may later *run*. Every other escape
here is harmless if forged; this one is not, so each report carries a SHA-256
MAC keyed by a per-pty secret the terminal puts in `OSC7577_TOKEN`. Three
consequences shape `term::program_resume`:

- **No token, no report.** The variable is the protocol's only feature test
  (there is no query), and a report without it could never verify. `Reporter`
  exists only when a token does, so a host cannot emit an unsigned report.
- **The token is a credential.** It is redacted from `Debug`, never written
  anywhere but the MAC input, and `scrub_env` strips it from child processes,
  which the spec requires: a child holding it could report in the host's name.
- **Both halves live in tuika.** `verify` implements the terminal's side —
  strict grammar, MAC before anything else, the time window, and the
  ignored/rejected split — so the encoder is tested against a strict decoder
  and the spec's published vectors rather than against itself, and the PTY
  suite can play a restoring terminal end to end: verify every report, then
  relaunch the recorded argv on a fresh pty and check the program came back
  where it was. Hosts get the same check for their own tests.

SHA-256 is implemented in-crate (FIPS vectors in the unit tests) rather than
taken as a dependency: it is one small function, and the dependency set stays
small.

### A query is answered on stdin, so it must be fenced and timed

The escapes tuika *tells* the terminal are fire-and-forget. A query is not: the
answer arrives on stdin, in band with the user's keystrokes, which creates two
hazards a one-way escape does not have.

The first is that a terminal implementing none of the query says nothing, and
silence is indistinguishable from *not yet*. Every probe therefore ends with the
Device Attributes request, which every terminal answers, as a **fence**: its
reply terminates the read, so an unsupported query costs one round-trip instead
of one timeout. This is also why the capability probe and the palette probe are
one request — the fence they need is the same byte sequence.

The fence is DA1's *final byte*, not any `c` in the stream: palette answers are
OSC strings full of hex `c`s, and the Kitty keyboard answer (`CSI ? flags u`) is
itself a CSI frame. The reader therefore tracks CSI frames and stops only on one
that closes with `c`, and the parsers look for their own frame anywhere in the
reply rather than assuming theirs comes first. Kitty keyboard support has no
reliable environment signal under multiplexers, so this probe is the
authoritative answer to "is Shift+Enter real here?".

The second is that a reply read late is a reply delivered to the application as
input. A probe must therefore run once, at startup, after raw mode is entered and
before the event loop reads stdin — and it must stop reading exactly at the
fence, so anything typed behind the replies still reaches the application. The
PTY suite asserts that boundary directly, by sending a keystroke immediately
after the replies and requiring the application to act on it.

A probe is always host-initiated. tuika does not query a terminal, or vary a
default by what terminal it finds itself in, unless a host asks it to.

### "Unknown escapes are swallowed" holds for three of the four

A terminal that does not understand OSC 8, 52, 9;4, 7501, or 7577 ignores it. That is why
those five need no capability detection and are safe to emit everywhere — the
worst case is that nothing happens.

Graphics protocols break that assumption: an unsupported terminal may render the
payload as visible garbage. Images are therefore the one capability gated on
detection, defaulting to a text fallback when unsure.

Sized text (OSC 66) breaks it the other way: the escape carries the text
itself, so a terminal that swallows it shows *less* than nothing — the words are
gone. It is gated on `TextSizing` detection (kitty only, never under a
multiplexer, since tmux would parse the stream rather than kitty) and every
sized view keeps a normal-size fallback.

### Sized text lives in the cell grid, not after it

OSC 66 moves the cursor and covers a `scale × cols` by `scale` block the
terminal then owns, which looks like the graphics case. It is handled the OSC 8
way instead: the escape sits in each glyph's origin cell with a `ForcedWidth`
of the scaled width, and every other cell of the block is `Skip`, because a
write to any of them makes the terminal erase the whole glyph. Keeping it in the
grid means the diff, split-footer scrollback, and one-shot output carry it with
no emission step, and the terminal scrolls and clips it like text. The text is
read back from cells already painted at normal size, so the unsized paint is
both the fallback and the source, and a block the clip would cut is simply left
unsized.

Any *new* out-of-band feature must be classified against this rule before it
ships: if an unsupporting terminal would show garbage rather than nothing, it
needs detection and a fallback, not an opt-out flag.

### Opt-in where the escape writes styled spans

OSC 9;4 and OSC 52 affect surfaces outside the text area. OSC 8 is different: it
wraps styled runs *in* the text, so a defect damages surrounding text, color, or
wrapping. Hosts are expected to keep it behind a switch until they have walked
the terminal matrix for their own UI, and the repository's own PTY smoke asserts
both that the escape is emitted and that the footer text around it survives
intact.

That opt-in applies to backend-wide inference. Components that own prose can
retain link destinations before painting and emit them without reconstructing
meaning from the backend's incremental draw runs. `Markdown` does this for explicit
and bare links; `Paragraph` does it for bare URLs, under `LinkPolicy::WEB` by
default. Literal components (`Text`, `Wrap`, `CodeBlock`, `Console`) do not infer
links. `HyperlinkBackend` remains the host-controlled escape hatch for arbitrary
rendered content.

### Encoders live in tuika, payloads come from the host

Desktop notifications follow the same explicit-input rule. Their encoders bound
text, remove terminal control characters, and replace protocol delimiters.
Unknown OSC notifications are ignored; hosts retain a visible in-app fallback.
Hosts choose the protocol and attention policy, including deduplication and
whether focus is known. Enabling focus reports does not reveal current focus
until an event arrives. The focus scope restores reporting on early returns and
failed enable flushes as well as normal exit.

Every one of these sequences is produced only by tuika's own encoder, from data
the host passed explicitly. Host text is never interpolated into an escape
unescaped. This is the property that keeps the "arbitrary caller text reaching
the terminal as a control sequence" class of bug out of scope by construction —
see [`SECURITY.md`](../../SECURITY.md).

Runner-provided mouse selection is the one framework-initiated clipboard path.
It is active only after a host explicitly opts into mouse capture, reads only
the cells the runner just rendered, and passes that text through the same
base64 OSC 52 encoder on release. Native terminal mouse handling remains the
default because capture also disables emulator-owned OSC 8 activation,
selection, and scrolling. `ctrl_click_url` is an opt-in application fallback
for captured sessions; opening its result remains host policy.

The clipboard writer detects tmux and GNU Screen and wraps the same validated
base64 OSC 52 payload in their passthrough envelopes. Screen's inner OSC ends
with BEL so ST cannot close DCS early, and its payloads stay below the older
Screen buffer limit. Pure encoders and explicit transport writers remain
available; nested transport order is host policy rather than guessed from
ambiguous environment markers. No clipboard path negotiates emulator permission.

OSC 8 environment hints recognize editor terminals and versioned VTE support,
but an outer terminal's identity does not prove multiplexer link support.
Pointer shapes are a fixed vocabulary of OSC 22 CSS names, never caller text.

### Program status is a value, not a stream of reports

OSC 7501 records outlive the program on purpose: the terminal drops `working`
and `blocked` when the process exits, but keeps `done` and `error` so the user
finds the result later. A host that ends with `clear` defeats that, and a host
that re-sends on every frame floods the terminal. So the runners own the
protocol around a single value the host sets (`Runner::program_status`): report
on change, clear on exit only what is still live, and leave finished records
alone. OSC 9;4 can be driven from the same value so the two indicators cannot
disagree. The logic lives in `term::program_status::StatusReporter` so a host
with its own loop gets the same rules.

Sub-tasks are records too. A host hands the runner its task tree and each task
becomes an `id=parent/child` record; the runner diffs the set, clears tasks that
left it, and applies the same exit rule per record. Ids come from host keys made
valid rather than rejected, and a subtree past the spec's id limits is left out
whole, since a truncated id would address the wrong record.

The cell-grid side uses the same vocabulary: `components::BadgeState` has
exactly the five reportable states and converts into `State`, so the badge on an
app's tab and the record in the terminal come from one value and cannot drift.

A terminal may accept only some states. The detection reply lists them, and a
report in a missing state is dropped whole, so a host that probed adapts each
report to the nearest state with the same meaning (`blocked` → `working`,
`error` → `done`, then `idle`) rather than going silent.

### A title is borrowed, so it is given back

A window title outlives the program that set it: a shell tab left named
"build: compiling" misleads the user long after the build ended. Titles are
therefore pushed onto the xterm title stack before the first change and popped
on exit, and the runner owns that pair the way it owns other enter/exit pairs.
Host text is sanitized rather than rejected (controls become spaces, the
length is capped), because an unrenamed window is worse than a slightly
altered title. Rex is recognized from its `REX_BLOCK` environment variable
only; renaming a tab through Rex's own CLI is host policy.

## Constraints

- Terminal replies must be suppressed where a protocol acknowledges commands on
  the tty (Kitty's `q=2`), or a full-screen event loop reads them as bogus input.
- Anything emitted on entry must have a matching teardown on exit — alternate
  screen, cursor visibility, mouse capture, progress state — and the PTY smoke
  asserts the pairs, not just the entries.

## Related

- [images.md](./images.md)
- [architecture.md](./architecture.md)
- [testing.md](../processes/testing.md)
