// The browser half of the host: a canvas "terminal emulator" for tuika's cell
// buffer, plus DOM-event translation.
//
// It never sees an escape sequence. A terminal host serializes the cell buffer
// into ANSI and the emulator parses it back into cells; here the cells cross the
// wasm boundary directly, as a flat array in linear memory, and this file just
// paints them. That is the whole reason the port is small: everything a
// terminal emulator does that tuika depends on — a monospace grid, colors,
// bold/italic/underline, key and mouse events — is a few dozen lines against a
// 2D context.

const WASM_URL = "tuika_web_demo.wasm";

// Must match `PackedCell` in src/lib.rs.
const CELL_STRIDE = 16;
const FLAG_BOLD = 1;
const FLAG_DIM = 2;
const FLAG_ITALIC = 4;
const FLAG_UNDERLINE = 8;
const FLAG_STRIKE = 16;

// Must match `key_code` in src/lib.rs.
const KEYS = {
  Enter: 1,
  Escape: 2,
  Backspace: 3,
  Delete: 4,
  Tab: 5,
  ArrowUp: 7,
  ArrowDown: 8,
  ArrowLeft: 9,
  ArrowRight: 10,
  Home: 11,
  End: 12,
  PageUp: 13,
  PageDown: 14,
};

const FONT_STACK =
  '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';
const FONT_PX = 15;
// Cell height as a multiple of the font size. Terminals pick something in this
// range; it decides how much air sits between rows.
const LINE_HEIGHT = 1.32;

// Box-drawing glyphs as arm weights `[up, right, down, left]` — 0 absent,
// 1 light, 2 heavy — plus a flag for the rounded corners.
//
// These are drawn as geometry, not text. A terminal emulator guarantees that a
// box-drawing glyph fills its whole cell, so borders tile seamlessly down a
// column; a proportional-ish glyph rendered into a canvas cell with line spacing
// leaves a gap between rows and the border comes out dashed. This is the one
// piece of terminal behavior a canvas host has to reimplement rather than
// inherit, and it is the difference between "renders" and "looks right".
const BOX = {
  0x2500: [0, 1, 0, 1],
  0x2501: [0, 2, 0, 2],
  0x2502: [1, 0, 1, 0],
  0x2503: [2, 0, 2, 0],
  0x250c: [0, 1, 1, 0],
  0x250f: [0, 2, 2, 0],
  0x2510: [0, 0, 1, 1],
  0x2513: [0, 0, 2, 2],
  0x2514: [1, 1, 0, 0],
  0x2517: [2, 2, 0, 0],
  0x2518: [1, 0, 0, 1],
  0x251b: [2, 0, 0, 2],
  0x251c: [1, 1, 1, 0],
  0x2524: [1, 0, 1, 1],
  0x252c: [0, 1, 1, 1],
  0x2534: [1, 1, 0, 1],
  0x253c: [1, 1, 1, 1],
  0x256d: [0, 1, 1, 0, true],
  0x256e: [0, 0, 1, 1, true],
  0x256f: [1, 0, 0, 1, true],
  0x2570: [1, 1, 0, 0, true],
};

/**
 * Paint a box-drawing or block glyph as rectangles, filling the cell edge to
 * edge. Returns false for anything not handled, which the caller draws as text.
 */
function drawGraphic(ctx, symbol, px, py, cw, ch, dpr) {
  if (symbol.length !== 1) return false;
  const cp = symbol.codePointAt(0);

  // Cell width is fractional, so adjacent filled cells would leave a hairline
  // seam wherever a rect edge lands mid-device-pixel. Snapping both edges to
  // the device grid makes a run of full blocks one solid bar, the way a
  // terminal's glyph cells tile.
  const snap = (v) => Math.round(v * dpr) / dpr;
  const fill = (x, y, w, h) => ctx.fillRect(snap(x), y, snap(x + w) - snap(x), h);

  // Block elements: full, upper half, and the eighth-width left blocks the
  // progress bar and scrollbar use.
  if (cp === 0x2588) {
    fill(px, py, cw, ch);
    return true;
  }
  if (cp === 0x2580) {
    fill(px, py, cw, ch / 2);
    return true;
  }
  if (cp === 0x2584) {
    fill(px, py + ch / 2, cw, ch / 2);
    return true;
  }
  if (cp >= 0x2589 && cp <= 0x258f) {
    // U+2589 is 7/8 wide and each step down loses an eighth.
    fill(px, py, (cw * (0x2590 - cp)) / 8, ch);
    return true;
  }
  if (cp >= 0x2591 && cp <= 0x2593) {
    // Shade blocks: a terminal stipples; alpha reads the same at cell size.
    const alpha = ctx.globalAlpha;
    ctx.globalAlpha = alpha * [0.25, 0.5, 0.75][cp - 0x2591];
    fill(px, py, cw, ch);
    ctx.globalAlpha = alpha;
    return true;
  }

  const arms = BOX[cp];
  if (!arms) return false;

  const [up, right, down, left, rounded] = arms;
  const weight = (arm) => (arm === 2 ? 2 : 1);
  const midX = px + Math.round(cw / 2);
  const midY = py + Math.round(ch / 2);

  if (rounded) {
    // One quarter-turn through the cell center, joining the two present arms.
    const thickness = weight(up || down);
    const endX = right ? px + cw : px;
    const endY = down ? py + ch : py;
    ctx.beginPath();
    ctx.lineWidth = thickness;
    ctx.strokeStyle = ctx.fillStyle;
    ctx.moveTo(midX, endY);
    ctx.quadraticCurveTo(midX, midY, endX, midY);
    ctx.stroke();
    return true;
  }

  if (left) {
    const t = weight(left);
    ctx.fillRect(px, midY - t / 2, midX - px + t / 2, t);
  }
  if (right) {
    const t = weight(right);
    ctx.fillRect(midX - t / 2, midY - t / 2, px + cw - midX + t / 2, t);
  }
  if (up) {
    const t = weight(up);
    ctx.fillRect(midX - t / 2, py, t, midY - py + t / 2);
  }
  if (down) {
    const t = weight(down);
    ctx.fillRect(midX - t / 2, midY - t / 2, t, py + ch - midY + t / 2);
  }
  return true;
}

/** Bit set matching the `mods` argument of `tk_key` / `tk_mouse`. */
function mods(event) {
  return (
    (event.ctrlKey ? 1 : 0) |
    (event.altKey ? 2 : 0) |
    (event.shiftKey ? 4 : 0) |
    (event.metaKey ? 8 : 0)
  );
}

class CanvasHost {
  constructor(canvas, wasm, status) {
    this.canvas = canvas;
    this.wasm = wasm;
    this.status = status;
    this.label = "";
    this.ctx = canvas.getContext("2d", { alpha: false });
    this.cols = 0;
    this.rows = 0;
    this.cellWidth = 0;
    this.cellHeight = 0;
    this.dpr = 1;
    this.pressed = false;
    // u32 → "#rrggbb", so a frame of one theme does a handful of string builds.
    this.cssCache = new Map();
    this.decoder = new TextDecoder();
    this.measure();
  }

  /** Measure the monospace advance width — the browser's `winsize` query. */
  measure() {
    this.ctx.font = `${FONT_PX}px ${FONT_STACK}`;
    // A long run divided by its length averages out sub-pixel rounding.
    const sample = "M".repeat(64);
    this.cellWidth = this.ctx.measureText(sample).width / 64;
    this.cellHeight = Math.round(FONT_PX * LINE_HEIGHT);
  }

  /** Re-fit the grid to the element box. This is the browser's SIGWINCH. */
  resize() {
    const rect = this.canvas.getBoundingClientRect();
    this.dpr = window.devicePixelRatio || 1;
    const cols = Math.max(1, Math.floor(rect.width / this.cellWidth));
    const rows = Math.max(1, Math.floor(rect.height / this.cellHeight));

    this.canvas.width = Math.floor(rect.width * this.dpr);
    this.canvas.height = Math.floor(rect.height * this.dpr);
    // Resetting the backing store clears every context property, so the
    // transform and font have to be reapplied here rather than once at startup.
    this.ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    this.ctx.textBaseline = "alphabetic";

    if (cols !== this.cols || rows !== this.rows) {
      this.cols = cols;
      this.rows = rows;
      this.wasm.tk_resize(cols, rows);
      if (this.status) {
        this.status.textContent = `${this.label}${cols}×${rows} cells`;
      }
    }
  }

  css(rgb) {
    let hit = this.cssCache.get(rgb);
    if (hit === undefined) {
      hit = `#${rgb.toString(16).padStart(6, "0")}`;
      this.cssCache.set(rgb, hit);
    }
    return hit;
  }

  /** Render one frame: ask wasm to composite, then paint the cells it packed. */
  frame() {
    const count = this.wasm.tk_render();
    if (count === 0) return;

    // Every pointer must be re-read after a call into wasm: growing the Rust
    // heap replaces the `WebAssembly.Memory` buffer and detaches old views.
    const memory = new DataView(this.wasm.memory.buffer);
    const bytes = new Uint8Array(this.wasm.memory.buffer);
    const cells = this.wasm.tk_cells();
    const atlas = this.wasm.tk_atlas();
    const atlasLen = this.wasm.tk_atlas_len();
    const atlasBytes = bytes.subarray(atlas, atlas + atlasLen);

    const ctx = this.ctx;
    const cw = this.cellWidth;
    const chh = this.cellHeight;
    const baseline = Math.round(chh * 0.76);
    const symbols = new Map();

    // The context is scaled by the device-pixel ratio, so clear in CSS pixels.
    // This also paints the margin below/right of the last whole cell, so the
    // grid's remainder matches the screen tuika composited.
    ctx.fillStyle = this.css(this.wasm.tk_background());
    ctx.fillRect(0, 0, this.canvas.width / this.dpr, this.canvas.height / this.dpr);

    for (let y = 0; y < this.rows; y++) {
      const rowBase = y * this.cols;

      // Backgrounds first, batched into runs: a full-width status bar is one
      // fillRect instead of one per cell.
      let runStart = 0;
      let runColor = -1;
      for (let x = 0; x <= this.cols; x++) {
        const bg =
          x < this.cols
            ? memory.getUint32(cells + (rowBase + x) * CELL_STRIDE + 12, true)
            : -1;
        if (bg !== runColor) {
          if (runColor >= 0 && x > runStart) {
            ctx.fillStyle = this.css(runColor);
            ctx.fillRect(runStart * cw, y * chh, (x - runStart) * cw, chh);
          }
          runStart = x;
          runColor = bg;
        }
      }

      for (let x = 0; x < this.cols; x++) {
        const at = cells + (rowBase + x) * CELL_STRIDE;
        const len = memory.getUint16(at + 4, true);
        if (len === 0) continue; // blank, or a wide glyph's trailing cell

        const offset = memory.getUint32(at, true);
        let symbol = symbols.get(offset);
        if (symbol === undefined) {
          symbol = this.decoder.decode(
            atlasBytes.subarray(offset, offset + len),
          );
          symbols.set(offset, symbol);
        }

        const flags = memory.getUint16(at + 6, true);
        const fg = memory.getUint32(at + 8, true);
        const px = x * cw;
        const py = y * chh;

        ctx.font =
          (flags & FLAG_ITALIC ? "italic " : "") +
          (flags & FLAG_BOLD ? "bold " : "") +
          `${FONT_PX}px ${FONT_STACK}`;
        // DIM has no font-weight equivalent on a canvas; alpha is what a
        // terminal does to it anyway.
        ctx.globalAlpha = flags & FLAG_DIM ? 0.6 : 1;
        ctx.fillStyle = this.css(fg);
        // Box-drawing and block glyphs are painted geometrically rather than as
        // text — see `drawGraphic`.
        if (!drawGraphic(ctx, symbol, px, py, cw, chh, this.dpr)) {
          ctx.fillText(symbol, px, py + baseline);
        }

        if (flags & FLAG_UNDERLINE) {
          ctx.fillRect(px, py + baseline + 2, cw, 1);
        }
        if (flags & FLAG_STRIKE) {
          ctx.fillRect(px, py + Math.round(chh * 0.5), cw, 1);
        }
        ctx.globalAlpha = 1;
      }
    }
  }

  /** Pixel position → grid cell, clamped to the grid. */
  cellAt(event) {
    const rect = this.canvas.getBoundingClientRect();
    const col = Math.floor((event.clientX - rect.left) / this.cellWidth);
    const row = Math.floor((event.clientY - rect.top) / this.cellHeight);
    return [
      Math.min(Math.max(col, 0), this.cols - 1),
      Math.min(Math.max(row, 0), this.rows - 1),
    ];
  }

  /** Translate a DOM `KeyboardEvent` — the browser's crossterm translation. */
  key(event) {
    let code = KEYS[event.key];
    let ch = 0;
    if (event.key === "Tab" && event.shiftKey) {
      code = 6; // BackTab, matching how a terminal reports shift+Tab
    } else if (code === undefined) {
      // A single-scalar `key` is a printable character; anything longer is a
      // named key tuika does not model (F-keys, media keys, dead keys).
      const points = Array.from(event.key);
      if (points.length !== 1) return false;
      code = 0;
      ch = points[0].codePointAt(0);
    }
    this.wasm.tk_key(code, ch, mods(event));
    return true;
  }

  paste(text) {
    const encoded = new TextEncoder().encode(text);
    const ptr = this.wasm.tk_paste_buffer(encoded.length);
    new Uint8Array(this.wasm.memory.buffer).set(encoded, ptr);
    this.wasm.tk_paste();
  }
}

async function main() {
  const canvas = document.getElementById("screen");
  const status = document.getElementById("status");

  // Fetched as bytes rather than streamed so the page can report the real
  // transfer size. No import object: the module is plain `extern "C"` Rust with
  // `panic = "abort"`, so it asks the embedder for nothing at all.
  const module = await fetch(WASM_URL).then((r) => r.arrayBuffer());
  const { instance } = await WebAssembly.instantiate(module, {});
  const wasm = instance.exports;

  const host = new CanvasHost(canvas, wasm, status);
  host.label = `${(module.byteLength / 1024) | 0} KiB wasm · no wasm-bindgen · `;
  const rect = canvas.getBoundingClientRect();
  wasm.tk_start(
    Math.max(1, Math.floor(rect.width / host.cellWidth)),
    Math.max(1, Math.floor(rect.height / host.cellHeight)),
  );
  host.resize();

  new ResizeObserver(() => host.resize()).observe(canvas);
  window.addEventListener("keydown", (event) => {
    if (event.metaKey || (event.ctrlKey && event.key === "r")) return;
    if (host.key(event)) event.preventDefault();
  });
  window.addEventListener("paste", (event) => {
    host.paste(event.clipboardData.getData("text"));
    event.preventDefault();
  });

  canvas.addEventListener("mousedown", (event) => {
    const [col, row] = host.cellAt(event);
    host.pressed = true;
    wasm.tk_mouse(0, event.button === 2 ? 1 : event.button, col, row, mods(event));
  });
  canvas.addEventListener("mousemove", (event) => {
    const [col, row] = host.cellAt(event);
    wasm.tk_mouse(host.pressed ? 2 : 3, 0, col, row, mods(event));
  });
  window.addEventListener("mouseup", (event) => {
    if (!host.pressed) return;
    const [col, row] = host.cellAt(event);
    host.pressed = false;
    wasm.tk_mouse(1, event.button === 2 ? 1 : event.button, col, row, mods(event));
  });
  canvas.addEventListener(
    "wheel",
    (event) => {
      const [col, row] = host.cellAt(event);
      wasm.tk_mouse(event.deltaY < 0 ? 4 : 5, 0, col, row, mods(event));
      event.preventDefault();
    },
    { passive: false },
  );

  const tick = () => {
    host.frame();
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

main().catch((error) => {
  const status = document.getElementById("status");
  if (status) status.textContent = `failed: ${error}`;
  throw error;
});
