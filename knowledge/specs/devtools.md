---
type: Product Specification
title: Devtools inspector
description: Defines the feature-gated inspector overlay — what it shows, who supplies each part, and the limits it keeps.
---

# Devtools inspector

## Why

Most of what goes wrong in a tuika application is invisible from outside it: a
view laid out into a rect one row shorter than intended, an overlay that still
owns input after it closed, a key that fell through every route stage, a
multi-stroke sequence left pending, a frame that repaints the whole screen on
every tick. tuika already computes every one of these — the solver assigns the
rects, `FocusRegistry` resolves ownership, `Router` reports a `Delivery`, the
keymap holds its pending chords, the terminal diff knows what it wrote — and
then throws them away. Textual's devtools showed that putting that state on
screen, live, beside the application, shortens the loop from "something is
wrong" to "this is why" more than any amount of logging.

## What

`tuika::devtools` (behind `feature = "devtools"`) provides an `Inspector`: a
shareable handle that collects the frame's state and paints it as a panel over
the frame, docked right on wide screens and to the bottom on narrow ones, with
an optional outline mode that frames and labels every traced view.

Ownership of each part follows who has the data:

| Shown | Supplied by |
| --- | --- |
| frame paint time, history, damaged cells | the runner (or the host's own loop through `record_frame` and `Terminal::last_damage`) |
| event log with each `UpdateResult` | the runner (or `log_event`) |
| toggle chord | the runner gives the inspector first refusal of every event |
| layout rects | the host wraps views with `trace`, like a `RectProbe` |
| focus owner, overlay owner, ring | the host calls `record_focus` |
| route of the event being handled | the host calls `record_route` with the `Delivery` |
| pending key sequence | the host calls `record_keymap` |

Tracing is explicit rather than automatic because tuika views have no identity
(see [Architecture](architecture.md)): an automatic tree dump would list
anonymous rects, and naming them is the useful part. It also costs nothing for
views nobody asked about.

## Constraints

- **Feature-gated.** The module and both runners' `with_inspector` exist only
  under `devtools`, and the feature has no dependencies. Runner integration goes
  through one private hook type whose methods are no-ops without the feature, so
  neither loop carries `cfg` branches.
- **Never alters the application's frame contract.** The inspector paints after
  the application's tree and after the runner's text selection, so it never
  reaches copied text, and it is timed out of the paint figure it reports. Its
  own cells do count as damage, because they are cells the terminal wrote.
- **A clean update still repaints while it is open**, so the log shows the
  event just delivered; while hidden the runner's redraw policy is unchanged.
- **Bounded.** The event log keeps a fixed number of entries (configurable) and
  coalesces pointer and scroll bursts; frame history is a fixed window.
- **No payloads.** A paste is logged by length only. An inspector is what ends up
  in screenshots and shared screens, and a paste is where secrets travel.
- **Debugging data, not state.** The handle shares data behind a mutex and
  recovers from poisoning instead of propagating it, because a half-written log
  line is not an invariant worth crashing a frame for.

## Related

- [Input routing](input-routing.md)
- [Keymap](keymap.md)
- [Architecture](architecture.md)
- [Testing](../processes/testing.md)
