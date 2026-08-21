---
type: Architecture Specification
title: Screen modes
description: Defines which part of the terminal a tuika frame owns — the alternate screen or a split footer over live scrollback — and the rules that keep the footer's neighbours (the user's scrollback, mouse, and shell prompt) intact.
---

# Screen modes

## Why

A full-screen renderer is one product decision, not the only one. It suits a
dashboard or an editor, and it is wrong for a long-running CLI: entering the
alternate screen hides the session the user was working in, and everything the
tool prints disappears when it exits. Tools of that shape — build watchers,
deploy drivers, coding agents — want the opposite split: a small live region
they repaint, over output that belongs to the terminal and outlives the process.

Doing that by hand is where hosts go wrong. Reserving rows, keeping them pinned
across resizes, and writing above them without corrupting the frame is fiddly,
and the failure mode (a `println!` landing inside the live region) is invisible
until it happens.

## What

`ScreenMode` is the host's first decision:

| Mode | Owns | Scrollback above | Mouse capture |
| --- | --- | --- | --- |
| `Alternate` (default) | the whole window, alternate buffer | n/a — restored on exit | off by default |
| `SplitFooter { height }` | `height` rows at the bottom of the main screen | the terminal's, live | off by default |

Both modes preserve the terminal's native OSC 8 activation, selection, and
scrolling by default. A host that needs pointer or wheel events opts into mouse
capture with `with_mouse_capture`; capture necessarily takes those native
behaviors away from the terminal. When a runner captures, it restores plain
drag selection over its final cell frame while continuing to route wheel events
to the application. Custom-loop hosts opt into capture and selection independently.

A split footer renders into a ratatui `Viewport::Inline`. The pieces around it:

- `TerminalSession::enter_with(mode)` takes only what the mode needs and
  restores exactly that.
- `TerminalSession::enter_config` preserves the same transactional rollback but
  lets a host independently choose raw mode, enhanced keyboard reporting,
  mouse capture, and cursor hiding. Mode defaults remain the ordinary path.
- `pin_footer` pushes the viewport to the bottom rows; `close_footer` gives them
  back and parks the cursor so the shell prompt resumes cleanly.
- `Scrollback` is a cloneable, `Send + Sync` queue of *views*, for publishing
  into the loop from elsewhere; `publish_block` commits one view straight from
  inside the loop, with no `Send` bound.

Both runners drive all of this from `RunnerConfig::screen_mode`; hosts with
their own loop compose the same pieces. The `codex` example runs either way
(`--split-footer`), which is what keeps the hand-rolled path honest.

## Design

### Publishing is a view, committed once

A block is rendered by tuika and then handed to the terminal for good. It is not
a frame: nothing repaints it, so animation, scroll state, or anything derived
from later state belongs in the footer instead. That one-way transfer is what
makes published output the *terminal's* content — selectable, scrollable, and
still on screen after the process exits.

Blocks are painted without the background fill `paint` applies, so untouched
cells keep the terminal's own colors and published output reads as part of the
surrounding session rather than a pasted panel.

Queued blocks are discarded in `Alternate`: there is no scrollback of the host's
to write into, and retaining them would grow without bound.

### Mouse capture is opt-in in both modes

Mouse reporting is global terminal state: there is no portable mode that sends
ordinary clicks to the application while retaining native OSC 8 modifier-clicks.
Both screen modes therefore leave mouse handling to the emulator. A mode that
needs application pointer or wheel input opts in with `with_mouse_capture`,
accepting that native link activation, selection, and scrolling stop until the
session restores the terminal state.

### The footer is pinned, not merely inline

ratatui anchors an inline viewport to the cursor row it was created at, so on a
fresh prompt the footer would float mid-screen with blank space below it. tuika
inserts the gap as blank rows instead, scrolling existing output up so the
footer sits on the last rows — and re-pins after a resize, which is why
`pin_footer` runs before every frame rather than once at startup.

### Every geometry the loop uses is learned before it is used

Both the pin and a published block are written at the geometry the `Terminal`
last observed, not at the terminal's current size — `pin_footer` inserts rows
that wide, and `Scrollback::flush` renders each block that wide. So the loop
calls `Terminal::autoresize` before *both*: before the first pin (a window
dragged between the terminal being constructed and the first frame is otherwise
never observed) and before draining the queue (a block published in response to
a resize — the most ordinary publishing moment there is — would otherwise be
committed at the previous width). The synchronous and async loops share this
order, and `tests/stress_ui.rs` pins it for both.

### Two ways to publish, because a queue cannot carry frame state

`Scrollback` queues a *builder* (`FnOnce(u16) -> Element + Send`), not a built
`Element`: the render width is known only at flush time, and a boxed `View`
cannot cross a thread. That is right for a background producer and wrong for the
loop itself, whose blocks may own things that are deliberately not `Send` — a
transcript entry holding a `MarkdownState` cache, say. `publish_block` is the
same commit without the queue, taking a `&RenderCtx` so a host publishes in its
own stylesheet. Neither is a shortcut for the other: one crosses a thread
boundary, one crosses none.

### Scrolling regions look like the obvious optimization and are the wrong trade

ratatui's portable path for inserting above an inline viewport clears the
viewport and queries the cursor position for every committed block, so the
footer repaints each time. DECSTBM scrolling regions avoid that by scrolling
only the rows above the footer — and lose the point of the mode while doing it:
a terminal discards what scrolls out of a scroll region instead of adding it to
the scrollback buffer. Published output would survive only as long as it stayed
on screen.

So `scrolling-regions` exists as a **compatibility mirror**, not an
optimization. Cargo unifies features one way only, so a host that enables
ratatui's feature would otherwise fail to build tuika — `term::hyperlink::HyperlinkBackend`
implements `Backend`, which the feature gives two more required methods. The
`codex` PTY tests assert the difference under both settings rather than leave it
to be discovered.

This is also why the PTY layer earns its keep: `TestBackend` *does* model
region-scrolled rows as entering its scrollback, so no hermetic test could have
caught this. Only a reference terminal on the other end of a real pty did.

### What a fixed footer height costs, and why it is still fixed

`ScreenMode::SplitFooter { height }` is decided when the terminal is created,
because that is where ratatui fixes an inline viewport; changing it means
rebuilding the `Terminal`. A host whose footer grows — a composer expanding, a
completion popup opening — therefore reserves the tallest state it needs and
lays out inside it, which is what `codex --split-footer` does.

Runtime resizing is a real capability (opentui exposes `footerHeight` as a
setter) and a plausible follow-up, but it needs a boundary for recreating the
backend that does not exist on ratatui's `Terminal` today (there is no
`into_backend`). Reserving the maximum is the honest interim: it costs rows, not
correctness.

## Related

- [architecture.md](./architecture.md)
- [out-of-band.md](./out-of-band.md)
- [testing.md](../processes/testing.md)
