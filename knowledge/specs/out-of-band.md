---
type: Architecture Specification
title: Out-of-band terminal escapes
description: Defines how tuika emits escapes that live outside the cell buffer — hyperlinks, clipboard, native progress — and the rule that separates the safe ones from the ones needing capability detection.
---

# Out-of-band terminal escapes

## Why

A ratatui cell carries one grapheme plus a style. Several things a modern
terminal can do are not expressible that way: making a text run clickable,
writing the system clipboard, driving the window's own progress indicator, or
painting pixels. Each needs an escape sequence that lives *outside* the cell
buffer, which ratatui's diff knows nothing about.

Emitting them naively fights the diff: writes appear at the wrong cursor
position, or get overwritten on the next frame, or leave the cursor somewhere
the renderer did not expect.

## What

Six out-of-band capabilities, in three families:

| Capability | Sequence | Module | Emission point |
| --- | --- | --- | --- |
| Hyperlinks | OSC 8 | `term::hyperlink` | spliced into the drawn cell run by `HyperlinkBackend` |
| Clipboard | OSC 52 | `term::clipboard` | host-initiated, any time |
| Native progress | OSC 9;4 | `term::progress` | host-initiated, any time |
| Pointer shape | OSC 22 | `term::pointer` | host-initiated, any time |
| Images | Kitty / iTerm2 / Sixel | `term::image` | after `terminal.draw()` returns |
| Terminal queries | DA1, OSC 10 / 11 / 4 | `term::capabilities`, `term::palette` | host-initiated, once at startup, in raw mode |

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
wrap a URL run *inside* the byte stream ratatui is already writing: the diff's
cursor model is unaffected, so nothing downstream needs to know.

Graphics escapes are not cursor-neutral — Kitty places the image at the cursor —
so they cannot be spliced. They are emitted after the frame, wrapped in a cursor
save/restore with an explicit CUP to each image's cell origin, leaving the net
effect on ratatui's cursor model at nil. See [images.md](./images.md).

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

The second is that a reply read late is a reply delivered to the application as
input. A probe must therefore run once, at startup, after raw mode is entered and
before the event loop reads stdin — and it must stop reading exactly at the
fence, so anything typed behind the replies still reaches the application. The
PTY suite asserts that boundary directly, by sending a keystroke immediately
after the replies and requiring the application to act on it.

A probe is always host-initiated. tuika does not query a terminal, or vary a
default by what terminal it finds itself in, unless a host asks it to.

### "Unknown escapes are swallowed" holds for three of the four

A terminal that does not understand OSC 8, 52, or 9;4 ignores it. That is why
those three need no capability detection and are safe to emit everywhere — the
worst case is that nothing happens.

Graphics protocols break that assumption: an unsupported terminal may render the
payload as visible garbage. Images are therefore the one capability gated on
detection, defaulting to a text fallback when unsure.

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

### Encoders live in tuika, payloads come from the host

Every one of these sequences is produced only by tuika's own encoder, from data
the host passed explicitly. Host text is never interpolated into an escape
unescaped. This is the property that keeps the "arbitrary caller text reaching
the terminal as a control sequence" class of bug out of scope by construction —
see [`SECURITY.md`](../../SECURITY.md).

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
