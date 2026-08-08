# VHS renderer comparison — xterm.js vs Ghostty

Evidence for evaluating the experimental `Set Renderer Ghostty` path in the
[`everruns/vhs`](https://github.com/everruns/vhs) fork against the default
xterm.js path the committed demos use today. Nothing here is referenced by the
docs or the `demo -- check` invariant; it exists so the choice can be reviewed
by looking at frames instead of adjectives.

Three scenes, recorded twice from the same generated tape (`demo -- tapes`), same
geometry and framerate, the Ghostty pass adding only `Set Renderer Ghostty`:

| scene      | xterm.js          | Ghostty           |
| ---------- | ----------------- | ----------------- |
| `spinner`  | `spinner_x.gif`   | `spinner_g.gif`   |
| `markdown` | `markdown_x.gif`  | `markdown_g.gif`  |
| `timeline` | `timeline_x.gif`  | `timeline_g.gif`  |

## What the frames show

- Ghostty's glyphs are crisper and the theme background is exact; xterm.js
  softens glyph edges. Cell metrics differ slightly between the two, so a scene
  lands at a different column offset for the same pixel width.
- Ghostty drops frames. For the same `Sleep 4s`, xterm.js captured 96 frames
  (3.84 s) while Ghostty captured 76 (`spinner`), 67 (`timeline`), and 53
  (`markdown`) — 2.1–3.0 s. A Ghostty recording therefore plays back faster than
  the session ran, the same failure mode the showcase notes in `AGENTS.md` warn
  about. This is the open blocker for adopting it in the generators.

## Reproducing

The Ghostty renderer is opt-in at build time and needs Go 1.26, Zig 0.15.2,
pkg-config, `ttyd`, and `ffmpeg`:

```bash
git clone https://github.com/everruns/vhs && cd vhs
export PKG_CONFIG_PATH="$(scripts/build-libghostty-vt.sh)"
go build -tags ghostty -o vhs-ghostty .
```

Zig 0.15.2 has no HTTP proxy support, so behind an egress proxy its dependency
fetches fail; seed `~/.cache/zig/p` first by downloading each tarball out of band
and handing the local file to `zig fetch`. As root, xterm.js recordings also need
`VHS_NO_SANDBOX=1` so Chromium will launch.
