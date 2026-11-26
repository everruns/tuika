//! [`FrameBuffer`] — a mutable RGBA canvas with sprite blitting and per-pixel
//! shaders, rendered to cells with half-blocks.
//!
//! This is the drawing-surface analog of OpenTUI's framebuffer/sprite stack,
//! built on the same RGBA pixel path as [`Image`](crate::components::Image) (so a buffer can
//! also be handed to the crisp Kitty/iTerm2/Sixel graphics protocols via
//! [`to_image_data`](FrameBuffer::to_image_data)). The always-available
//! [`FrameBufferView`] needs no graphics protocol: it packs two vertical pixels
//! into each terminal cell with the `▀` half-block, so a game/visualization
//! renders in any terminal.
//!
//! The pieces:
//! - [`FrameBuffer`] — clear, `set`/`blend` pixels, `fill_rect`, `blit` another
//!   buffer (alpha-composited), and `shade` a per-pixel post-pass.
//! - [`Sprite`] — a spritesheet sliced into equal frames; [`Sprite::draw`] blits
//!   one frame onto a target buffer for animation.
//! - [`FrameBufferView`] — a [`View`] that paints a buffer into the
//!   cells it reserves.
//!
//! tuika owns none of the *content* — the host draws into the buffer each frame,
//! exactly as it owns image decoding and syntax highlighting.

use crate::geometry::Rect;
use crate::style::{Color, Style};

use crate::geometry::Size;
use crate::surface::Surface;
use crate::term::image::ImageData;
use crate::view::{RenderCtx, View};

/// An 8-bit RGBA pixel: `[r, g, b, a]`, `a` = 255 opaque, 0 transparent.
pub type Rgba = [u8; 4];

/// A fully transparent pixel.
pub const TRANSPARENT: Rgba = [0, 0, 0, 0];

/// The sixteen quadrant block glyphs, indexed by a four-bit occupancy mask.
///
/// Bit 0 is the top-left quadrant, 1 top-right, 2 bottom-left, 3 bottom-right,
/// so `QUADRANTS[0b0101]` is the left half `▌`. Packing four sub-cells into one
/// glyph doubles the effective resolution of a cell grid in both axes, which is
/// what lets a chart or canvas render without any graphics protocol.
pub const QUADRANTS: [char; 16] = [
    ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
];

/// The sixty-four sextant glyphs, indexed by a six-bit occupancy mask.
///
/// A sextant cell is two columns by three rows; bit `row * 2 + col` is set when
/// that sub-cell is inked, so bit 0 is the top-left and bit 5 the bottom-right.
/// The four masks Unicode already covers elsewhere map to those characters:
/// blank, `▌`, `▐`, and `█`. Everything else is in *Symbols for Legacy
/// Computing* (Unicode 13).
pub const SEXTANTS: [char; 64] = [
    '\u{20}',
    '\u{1FB00}',
    '\u{1FB01}',
    '\u{1FB02}',
    '\u{1FB03}',
    '\u{1FB04}',
    '\u{1FB05}',
    '\u{1FB06}',
    '\u{1FB07}',
    '\u{1FB08}',
    '\u{1FB09}',
    '\u{1FB0A}',
    '\u{1FB0B}',
    '\u{1FB0C}',
    '\u{1FB0D}',
    '\u{1FB0E}',
    '\u{1FB0F}',
    '\u{1FB10}',
    '\u{1FB11}',
    '\u{1FB12}',
    '\u{1FB13}',
    '\u{258C}',
    '\u{1FB14}',
    '\u{1FB15}',
    '\u{1FB16}',
    '\u{1FB17}',
    '\u{1FB18}',
    '\u{1FB19}',
    '\u{1FB1A}',
    '\u{1FB1B}',
    '\u{1FB1C}',
    '\u{1FB1D}',
    '\u{1FB1E}',
    '\u{1FB1F}',
    '\u{1FB20}',
    '\u{1FB21}',
    '\u{1FB22}',
    '\u{1FB23}',
    '\u{1FB24}',
    '\u{1FB25}',
    '\u{1FB26}',
    '\u{1FB27}',
    '\u{2590}',
    '\u{1FB28}',
    '\u{1FB29}',
    '\u{1FB2A}',
    '\u{1FB2B}',
    '\u{1FB2C}',
    '\u{1FB2D}',
    '\u{1FB2E}',
    '\u{1FB2F}',
    '\u{1FB30}',
    '\u{1FB31}',
    '\u{1FB32}',
    '\u{1FB33}',
    '\u{1FB34}',
    '\u{1FB35}',
    '\u{1FB36}',
    '\u{1FB37}',
    '\u{1FB38}',
    '\u{1FB39}',
    '\u{1FB3A}',
    '\u{1FB3B}',
    '\u{2588}',
];

/// The 256 octant glyphs, indexed by an eight-bit occupancy mask.
///
/// An octant cell is two columns by four rows; bit `row * 2 + col` is set when
/// that sub-cell is inked. Unicode 16 added the octants proper; the 26 masks
/// that already had a character — blank, `█`, the halves and quadrants, and the
/// quarter blocks — map to it instead, so every mask is one glyph.
pub const OCTANTS: [char; 256] = [
    '\u{20}',
    '\u{1CEA8}',
    '\u{1CEAB}',
    '\u{1FB82}',
    '\u{1CD00}',
    '\u{2598}',
    '\u{1CD01}',
    '\u{1CD02}',
    '\u{1CD03}',
    '\u{1CD04}',
    '\u{259D}',
    '\u{1CD05}',
    '\u{1CD06}',
    '\u{1CD07}',
    '\u{1CD08}',
    '\u{2580}',
    '\u{1CD09}',
    '\u{1CD0A}',
    '\u{1CD0B}',
    '\u{1CD0C}',
    '\u{1FBE6}',
    '\u{1CD0D}',
    '\u{1CD0E}',
    '\u{1CD0F}',
    '\u{1CD10}',
    '\u{1CD11}',
    '\u{1CD12}',
    '\u{1CD13}',
    '\u{1CD14}',
    '\u{1CD15}',
    '\u{1CD16}',
    '\u{1CD17}',
    '\u{1CD18}',
    '\u{1CD19}',
    '\u{1CD1A}',
    '\u{1CD1B}',
    '\u{1CD1C}',
    '\u{1CD1D}',
    '\u{1CD1E}',
    '\u{1CD1F}',
    '\u{1FBE7}',
    '\u{1CD20}',
    '\u{1CD21}',
    '\u{1CD22}',
    '\u{1CD23}',
    '\u{1CD24}',
    '\u{1CD25}',
    '\u{1CD26}',
    '\u{1CD27}',
    '\u{1CD28}',
    '\u{1CD29}',
    '\u{1CD2A}',
    '\u{1CD2B}',
    '\u{1CD2C}',
    '\u{1CD2D}',
    '\u{1CD2E}',
    '\u{1CD2F}',
    '\u{1CD30}',
    '\u{1CD31}',
    '\u{1CD32}',
    '\u{1CD33}',
    '\u{1CD34}',
    '\u{1CD35}',
    '\u{1FB85}',
    '\u{1CEA3}',
    '\u{1CD36}',
    '\u{1CD37}',
    '\u{1CD38}',
    '\u{1CD39}',
    '\u{1CD3A}',
    '\u{1CD3B}',
    '\u{1CD3C}',
    '\u{1CD3D}',
    '\u{1CD3E}',
    '\u{1CD3F}',
    '\u{1CD40}',
    '\u{1CD41}',
    '\u{1CD42}',
    '\u{1CD43}',
    '\u{1CD44}',
    '\u{2596}',
    '\u{1CD45}',
    '\u{1CD46}',
    '\u{1CD47}',
    '\u{1CD48}',
    '\u{258C}',
    '\u{1CD49}',
    '\u{1CD4A}',
    '\u{1CD4B}',
    '\u{1CD4C}',
    '\u{259E}',
    '\u{1CD4D}',
    '\u{1CD4E}',
    '\u{1CD4F}',
    '\u{1CD50}',
    '\u{259B}',
    '\u{1CD51}',
    '\u{1CD52}',
    '\u{1CD53}',
    '\u{1CD54}',
    '\u{1CD55}',
    '\u{1CD56}',
    '\u{1CD57}',
    '\u{1CD58}',
    '\u{1CD59}',
    '\u{1CD5A}',
    '\u{1CD5B}',
    '\u{1CD5C}',
    '\u{1CD5D}',
    '\u{1CD5E}',
    '\u{1CD5F}',
    '\u{1CD60}',
    '\u{1CD61}',
    '\u{1CD62}',
    '\u{1CD63}',
    '\u{1CD64}',
    '\u{1CD65}',
    '\u{1CD66}',
    '\u{1CD67}',
    '\u{1CD68}',
    '\u{1CD69}',
    '\u{1CD6A}',
    '\u{1CD6B}',
    '\u{1CD6C}',
    '\u{1CD6D}',
    '\u{1CD6E}',
    '\u{1CD6F}',
    '\u{1CD70}',
    '\u{1CEA0}',
    '\u{1CD71}',
    '\u{1CD72}',
    '\u{1CD73}',
    '\u{1CD74}',
    '\u{1CD75}',
    '\u{1CD76}',
    '\u{1CD77}',
    '\u{1CD78}',
    '\u{1CD79}',
    '\u{1CD7A}',
    '\u{1CD7B}',
    '\u{1CD7C}',
    '\u{1CD7D}',
    '\u{1CD7E}',
    '\u{1CD7F}',
    '\u{1CD80}',
    '\u{1CD81}',
    '\u{1CD82}',
    '\u{1CD83}',
    '\u{1CD84}',
    '\u{1CD85}',
    '\u{1CD86}',
    '\u{1CD87}',
    '\u{1CD88}',
    '\u{1CD89}',
    '\u{1CD8A}',
    '\u{1CD8B}',
    '\u{1CD8C}',
    '\u{1CD8D}',
    '\u{1CD8E}',
    '\u{1CD8F}',
    '\u{2597}',
    '\u{1CD90}',
    '\u{1CD91}',
    '\u{1CD92}',
    '\u{1CD93}',
    '\u{259A}',
    '\u{1CD94}',
    '\u{1CD95}',
    '\u{1CD96}',
    '\u{1CD97}',
    '\u{2590}',
    '\u{1CD98}',
    '\u{1CD99}',
    '\u{1CD9A}',
    '\u{1CD9B}',
    '\u{259C}',
    '\u{1CD9C}',
    '\u{1CD9D}',
    '\u{1CD9E}',
    '\u{1CD9F}',
    '\u{1CDA0}',
    '\u{1CDA1}',
    '\u{1CDA2}',
    '\u{1CDA3}',
    '\u{1CDA4}',
    '\u{1CDA5}',
    '\u{1CDA6}',
    '\u{1CDA7}',
    '\u{1CDA8}',
    '\u{1CDA9}',
    '\u{1CDAA}',
    '\u{1CDAB}',
    '\u{2582}',
    '\u{1CDAC}',
    '\u{1CDAD}',
    '\u{1CDAE}',
    '\u{1CDAF}',
    '\u{1CDB0}',
    '\u{1CDB1}',
    '\u{1CDB2}',
    '\u{1CDB3}',
    '\u{1CDB4}',
    '\u{1CDB5}',
    '\u{1CDB6}',
    '\u{1CDB7}',
    '\u{1CDB8}',
    '\u{1CDB9}',
    '\u{1CDBA}',
    '\u{1CDBB}',
    '\u{1CDBC}',
    '\u{1CDBD}',
    '\u{1CDBE}',
    '\u{1CDBF}',
    '\u{1CDC0}',
    '\u{1CDC1}',
    '\u{1CDC2}',
    '\u{1CDC3}',
    '\u{1CDC4}',
    '\u{1CDC5}',
    '\u{1CDC6}',
    '\u{1CDC7}',
    '\u{1CDC8}',
    '\u{1CDC9}',
    '\u{1CDCA}',
    '\u{1CDCB}',
    '\u{1CDCC}',
    '\u{1CDCD}',
    '\u{1CDCE}',
    '\u{1CDCF}',
    '\u{1CDD0}',
    '\u{1CDD1}',
    '\u{1CDD2}',
    '\u{1CDD3}',
    '\u{1CDD4}',
    '\u{1CDD5}',
    '\u{1CDD6}',
    '\u{1CDD7}',
    '\u{1CDD8}',
    '\u{1CDD9}',
    '\u{1CDDA}',
    '\u{2584}',
    '\u{1CDDB}',
    '\u{1CDDC}',
    '\u{1CDDD}',
    '\u{1CDDE}',
    '\u{2599}',
    '\u{1CDDF}',
    '\u{1CDE0}',
    '\u{1CDE1}',
    '\u{1CDE2}',
    '\u{259F}',
    '\u{1CDE3}',
    '\u{2586}',
    '\u{1CDE4}',
    '\u{1CDE5}',
    '\u{2588}',
];

/// How a cell is divided into sub-cells when pixels or plotted points are
/// packed into glyphs.
///
/// Finer blitters resolve more detail per cell but need newer glyphs in the
/// terminal's font. `HalfBlock` and `Quadrant` are in every font a terminal
/// ships; `Sextant` (Unicode 13) is widely drawn, and `Octant` (Unicode 16) is
/// drawn natively by recent terminals — Ghostty, kitty, WezTerm, foot — but may
/// fall back to a missing-glyph box elsewhere. `Braille` is universally
/// available but draws separated dots rather than solid blocks, so it suits
/// scatter points better than filled pictures.
///
/// A cell holds one foreground and one background color, so a mosaic finer
/// than `HalfBlock` approximates each cell with the two colors that best split
/// its pixels: more spatial detail, less color detail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Blitter {
    /// 1×2 sub-cells with `▀`: full color per sub-cell. The default.
    #[default]
    HalfBlock,
    /// 2×2 sub-cells from the quadrant blocks ([`QUADRANTS`]).
    Quadrant,
    /// 2×3 sub-cells from the sextant blocks ([`SEXTANTS`]).
    Sextant,
    /// 2×4 sub-cells from the octant blocks ([`OCTANTS`]).
    Octant,
    /// 2×4 Braille dots (U+2800–U+28FF).
    Braille,
}

impl Blitter {
    /// Every blitter, coarsest first.
    pub const ALL: [Blitter; 5] = [
        Blitter::HalfBlock,
        Blitter::Quadrant,
        Blitter::Sextant,
        Blitter::Octant,
        Blitter::Braille,
    ];

    /// Sub-cells per terminal cell, as `(columns, rows)`.
    pub const fn subcells(self) -> (u32, u32) {
        match self {
            Blitter::HalfBlock => (1, 2),
            Blitter::Quadrant => (2, 2),
            Blitter::Sextant => (2, 3),
            Blitter::Octant | Blitter::Braille => (2, 4),
        }
    }

    /// The glyph for an occupancy `mask`, bit `row * columns + col` set for
    /// each inked sub-cell (see [`subcells`](Self::subcells)). Bits beyond
    /// the blitter's sub-cell count are ignored.
    ///
    /// ```
    /// use tuika::framebuffer::Blitter;
    ///
    /// assert_eq!(Blitter::HalfBlock.glyph(0b01), '▀');
    /// assert_eq!(Blitter::Quadrant.glyph(0b0101), '▌');
    /// assert_eq!(Blitter::Sextant.glyph(0b11_1111), '█');
    /// assert_eq!(Blitter::Braille.glyph(0b1000_0001), '⢁');
    /// ```
    pub const fn glyph(self, mask: u8) -> char {
        match self {
            Blitter::HalfBlock => [' ', '▀', '▄', '█'][(mask & 0b11) as usize],
            Blitter::Quadrant => QUADRANTS[(mask & 0b1111) as usize],
            Blitter::Sextant => SEXTANTS[(mask & 0b11_1111) as usize],
            Blitter::Octant => OCTANTS[mask as usize],
            Blitter::Braille => {
                // Braille numbers its dots down the left column first, with
                // the bottom row added last: 1 2 3 7 left, 4 5 6 8 right.
                const DOTS: [u8; 8] = [0, 3, 1, 4, 2, 5, 6, 7];
                let mut bits = 0u32;
                let mut i = 0;
                while i < 8 {
                    if mask & (1 << i) != 0 {
                        bits |= 1 << DOTS[i];
                    }
                    i += 1;
                }
                match char::from_u32(0x2800 + bits) {
                    Some(ch) => ch,
                    None => ' ',
                }
            }
        }
    }
}

/// Paint a `cols × rows` cell mosaic at `origin` from `sample`, which returns
/// the opaque color of virtual pixel `(px, py)` in a grid of
/// `cols·sub_w × rows·sub_h` (see [`Blitter::subcells`]).
///
/// `HalfBlock` keeps both pixel colors exactly. The other blitters split each
/// cell's pixels into two clusters around its most distant pair of colors and
/// paint the brighter cluster's mean as the glyph's foreground over the other
/// cluster's mean, so edges stay sharp and flat regions become plain cells.
pub(crate) fn paint_mosaic(
    surface: &mut Surface,
    origin: (u16, u16),
    cols: u16,
    rows: u16,
    blitter: Blitter,
    sample: impl Fn(u32, u32) -> [u8; 3],
) {
    let (sw, sh) = blitter.subcells();
    let rgb = |c: [u8; 3]| Color::Rgb(c[0], c[1], c[2]);
    let mut pixels = [[0u8; 3]; 8];
    for row in 0..rows {
        for col in 0..cols {
            let n = (sw * sh) as usize;
            for (i, px) in pixels.iter_mut().take(n).enumerate() {
                let i = i as u32;
                *px = sample(col as u32 * sw + i % sw, row as u32 * sh + i / sw);
            }
            let (x, y) = (origin.0 + col, origin.1 + row);
            if blitter == Blitter::HalfBlock {
                let style = Style::default().fg(rgb(pixels[0])).bg(rgb(pixels[1]));
                surface.set(x, y, '▀', style);
                continue;
            }
            let (mask, fg, bg) = split(&pixels[..n]);
            let style = Style::default().fg(rgb(fg)).bg(rgb(bg));
            surface.set(x, y, blitter.glyph(mask), style);
        }
    }
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    let d = |i: usize| (i32::from(a[i]) - i32::from(b[i])).unsigned_abs();
    d(0) * d(0) + d(1) * d(1) + d(2) * d(2)
}

fn luma(c: [u8; 3]) -> u32 {
    299 * u32::from(c[0]) + 587 * u32::from(c[1]) + 114 * u32::from(c[2])
}

/// Two-color quantization of one cell's pixels: the occupancy mask of the
/// foreground cluster, then the foreground and background means.
fn split(pixels: &[[u8; 3]]) -> (u8, [u8; 3], [u8; 3]) {
    let (mut a, mut b, mut far) = (0, 0, 0);
    for i in 0..pixels.len() {
        for j in i + 1..pixels.len() {
            let d = distance(pixels[i], pixels[j]);
            if d > far {
                (a, b, far) = (i, j, d);
            }
        }
    }
    if far == 0 {
        return (0, pixels[0], pixels[0]);
    }
    let (ink, paper) = if luma(pixels[a]) >= luma(pixels[b]) {
        (pixels[a], pixels[b])
    } else {
        (pixels[b], pixels[a])
    };
    let mut mask = 0u8;
    let (mut fg, mut bg) = ([0u32; 3], [0u32; 3]);
    let (mut nf, mut nb) = (0u32, 0u32);
    for (i, &px) in pixels.iter().enumerate() {
        let (sum, count) = if distance(px, ink) < distance(px, paper) {
            mask |= 1 << i;
            (&mut fg, &mut nf)
        } else {
            (&mut bg, &mut nb)
        };
        for c in 0..3 {
            sum[c] += u32::from(px[c]);
        }
        *count += 1;
    }
    let mean = |sum: [u32; 3], n: u32| sum.map(|c| ((c + n / 2) / n) as u8);
    (mask, mean(fg, nf), mean(bg, nb))
}

/// Source-over alpha composite of `src` onto `dst`.
fn over(src: Rgba, dst: Rgba) -> Rgba {
    let sa = src[3] as u32;
    if sa == 255 {
        return src;
    }
    if sa == 0 {
        return dst;
    }
    let da = dst[3] as u32;
    let da_out = da * (255 - sa) / 255;
    let out_a = sa + da_out;
    if out_a == 0 {
        return TRANSPARENT;
    }
    let ch = |s: u8, d: u8| ((s as u32 * sa + d as u32 * da_out) / out_a) as u8;
    [
        ch(src[0], dst[0]),
        ch(src[1], dst[1]),
        ch(src[2], dst[2]),
        out_a as u8,
    ]
}

/// A mutable RGBA pixel canvas.
#[derive(Clone, Debug)]
pub struct FrameBuffer {
    width: u32,
    height: u32,
    /// Row-major RGBA, `width * height * 4` bytes.
    pixels: Vec<u8>,
}

impl FrameBuffer {
    /// A `width × height` buffer, initially fully transparent.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Buffer width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Buffer height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The raw RGBA bytes (row-major), for the graphics protocols.
    pub fn as_rgba(&self) -> &[u8] {
        &self.pixels
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height).then(|| ((y * self.width + x) * 4) as usize)
    }

    /// The pixel at `(x, y)`, or [`TRANSPARENT`] if out of bounds.
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        match self.index(x, y) {
            Some(i) => [
                self.pixels[i],
                self.pixels[i + 1],
                self.pixels[i + 2],
                self.pixels[i + 3],
            ],
            None => TRANSPARENT,
        }
    }

    /// Overwrite the pixel at `(x, y)` (no blending); out-of-bounds is ignored.
    pub fn set(&mut self, x: u32, y: u32, color: Rgba) {
        if let Some(i) = self.index(x, y) {
            self.pixels[i..i + 4].copy_from_slice(&color);
        }
    }

    /// Alpha-composite `color` over the pixel at `(x, y)`.
    pub fn blend(&mut self, x: u32, y: u32, color: Rgba) {
        if let Some(i) = self.index(x, y) {
            let dst = [
                self.pixels[i],
                self.pixels[i + 1],
                self.pixels[i + 2],
                self.pixels[i + 3],
            ];
            self.pixels[i..i + 4].copy_from_slice(&over(color, dst));
        }
    }

    /// Set every pixel to `color` (overwrite).
    pub fn clear(&mut self, color: Rgba) {
        for px in self.pixels.chunks_exact_mut(4) {
            px.copy_from_slice(&color);
        }
    }

    /// Alpha-composite `color` over a rectangle (clipped to the buffer).
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, color: Rgba) {
        for py in y..y.saturating_add(h).min(self.height) {
            for px in x..x.saturating_add(w).min(self.width) {
                self.blend(px, py, color);
            }
        }
    }

    /// Alpha-composite the whole of `src` onto this buffer at `(dx, dy)`.
    pub fn blit(&mut self, src: &FrameBuffer, dx: i32, dy: i32) {
        self.blit_region(src, 0, 0, src.width, src.height, dx, dy);
    }

    /// Alpha-composite a `sw × sh` region of `src` (from `(sx, sy)`) onto this
    /// buffer at `(dx, dy)`. Everything is clipped to both buffers.
    #[allow(clippy::too_many_arguments)]
    pub fn blit_region(
        &mut self,
        src: &FrameBuffer,
        sx: u32,
        sy: u32,
        sw: u32,
        sh: u32,
        dx: i32,
        dy: i32,
    ) {
        for row in 0..sh {
            let ty = dy + row as i32;
            if ty < 0 || ty as u32 >= self.height {
                continue;
            }
            for col in 0..sw {
                let tx = dx + col as i32;
                if tx < 0 || tx as u32 >= self.width {
                    continue;
                }
                let color = src.pixel(sx + col, sy + row);
                if color[3] != 0 {
                    self.blend(tx as u32, ty as u32, color);
                }
            }
        }
    }

    /// Apply a per-pixel shader post-pass: `f(x, y, current)` returns the new
    /// pixel. This is the terminal analog of a fragment shader — invert, tint,
    /// fade, scanlines, etc.
    pub fn shade(&mut self, f: impl Fn(u32, u32, Rgba) -> Rgba) {
        for y in 0..self.height {
            for x in 0..self.width {
                let i = ((y * self.width + x) * 4) as usize;
                let cur = [
                    self.pixels[i],
                    self.pixels[i + 1],
                    self.pixels[i + 2],
                    self.pixels[i + 3],
                ];
                let out = f(x, y, cur);
                self.pixels[i..i + 4].copy_from_slice(&out);
            }
        }
    }

    /// Snapshot as [`ImageData`] for the Kitty/iTerm2/Sixel graphics path, or
    /// `None` for a zero-sized buffer.
    pub fn to_image_data(&self) -> Option<ImageData> {
        ImageData::from_rgba(self.width, self.height, self.pixels.clone())
    }
}

/// A spritesheet sliced into equal `frame_w × frame_h` frames, left-to-right then
/// top-to-bottom.
#[derive(Clone, Debug)]
pub struct Sprite {
    sheet: FrameBuffer,
    frame_w: u32,
    frame_h: u32,
    cols: u32,
    rows: u32,
}

impl Sprite {
    /// Slice `sheet` into `frame_w × frame_h` frames. Partial trailing frames
    /// (if the sheet isn't an exact multiple) are ignored.
    pub fn new(sheet: FrameBuffer, frame_w: u32, frame_h: u32) -> Self {
        let cols = if frame_w == 0 {
            0
        } else {
            sheet.width / frame_w
        };
        let rows = if frame_h == 0 {
            0
        } else {
            sheet.height / frame_h
        };
        Self {
            sheet,
            frame_w,
            frame_h,
            cols,
            rows,
        }
    }

    /// Number of whole frames in the sheet.
    pub fn frame_count(&self) -> u32 {
        self.cols * self.rows
    }

    /// Pixel dimensions of a single frame.
    pub fn frame_size(&self) -> (u32, u32) {
        (self.frame_w, self.frame_h)
    }

    /// Alpha-composite frame `index` (wrapping — handy to drive from a host frame
    /// counter) onto `target` at `(dx, dy)`.
    pub fn draw(&self, target: &mut FrameBuffer, dx: i32, dy: i32, index: u32) {
        let count = self.frame_count();
        if count == 0 {
            return;
        }
        let index = index % count;
        let (cx, cy) = (index % self.cols, index / self.cols);
        target.blit_region(
            &self.sheet,
            cx * self.frame_w,
            cy * self.frame_h,
            self.frame_w,
            self.frame_h,
            dx,
            dy,
        );
    }
}

/// A [`View`] that paints a [`FrameBuffer`] into the `cols × rows` cells it
/// reserves, packing pixels into glyphs with a [`Blitter`] — by default two
/// vertical pixels per cell with the `▀` half-block.
///
/// The buffer is nearest-neighbor sampled to the blitter's sub-cell grid
/// (`cols × 2·rows` for the half-block), and each pixel is alpha-composited
/// over the theme background so transparency reads as the surrounding UI.
///
/// ![blitters demo](https://raw.githubusercontent.com/everruns/tuika/main/docs/demos/blitters.png)
pub struct FrameBufferView<'a> {
    fb: &'a FrameBuffer,
    cols: u16,
    rows: u16,
    blitter: Blitter,
}

impl<'a> FrameBufferView<'a> {
    /// A view of `fb` reserving `cols × rows` cells.
    pub fn new(fb: &'a FrameBuffer, cols: u16, rows: u16) -> Self {
        Self {
            fb,
            cols,
            rows,
            blitter: Blitter::HalfBlock,
        }
    }

    /// Pack pixels into cells with `blitter` instead of the half-block.
    pub fn blitter(mut self, blitter: Blitter) -> Self {
        self.blitter = blitter;
        self
    }
}

/// Composite `px` over an opaque `bg`.
fn composite(px: Rgba, bg: Color) -> [u8; 3] {
    let (br, bg_, bb) = match bg {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (0, 0, 0),
    };
    let out = over(px, [br, bg_, bb, 255]);
    [out[0], out[1], out[2]]
}

impl View for FrameBufferView<'_> {
    fn measure(&self, available: Size, _ctx: &RenderCtx) -> Size {
        Size::new(self.cols, self.rows).clamp_to(available)
    }

    fn render(&self, area: Rect, surface: &mut Surface, ctx: &RenderCtx) {
        if area.is_empty() || self.fb.width == 0 || self.fb.height == 0 {
            return;
        }
        let cols = self.cols.min(area.width);
        let rows = self.rows.min(area.height);
        if cols == 0 || rows == 0 {
            return;
        }
        let bg = ctx.theme.background;
        let (sw, sh) = self.blitter.subcells();
        let (grid_w, grid_h) = (cols as u32 * sw, rows as u32 * sh);
        // Nearest-neighbor sample of the buffer into the sub-cell grid.
        let sample = |px: u32, py: u32| {
            let src_x = (px * self.fb.width) / grid_w;
            let src_y = (py * self.fb.height) / grid_h;
            composite(
                self.fb
                    .pixel(src_x.min(self.fb.width - 1), src_y.min(self.fb.height - 1)),
                bg,
            )
        };
        paint_mosaic(surface, (area.x, area.y), cols, rows, self.blitter, sample);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Theme;

    const RED: Rgba = [255, 0, 0, 255];
    const BLUE: Rgba = [0, 0, 255, 255];
    const HALF_RED: Rgba = [255, 0, 0, 128];

    #[test]
    fn set_get_and_clear() {
        let mut fb = FrameBuffer::new(4, 3);
        assert_eq!(fb.pixel(0, 0), TRANSPARENT);
        fb.set(1, 2, RED);
        assert_eq!(fb.pixel(1, 2), RED);
        // Out-of-bounds reads are transparent, writes are ignored (no panic).
        assert_eq!(fb.pixel(99, 99), TRANSPARENT);
        fb.set(99, 99, BLUE);
        fb.clear(BLUE);
        assert_eq!(fb.pixel(1, 2), BLUE);
    }

    #[test]
    fn blend_composites_alpha() {
        let mut fb = FrameBuffer::new(1, 1);
        fb.set(0, 0, BLUE);
        fb.blend(0, 0, HALF_RED);
        let p = fb.pixel(0, 0);
        // Half-opaque red over blue → a purple, fully opaque.
        assert!(p[0] > 100 && p[2] > 100, "purple-ish: {p:?}");
        assert_eq!(p[3], 255, "opaque result");
    }

    #[test]
    fn fill_rect_clips_to_bounds() {
        let mut fb = FrameBuffer::new(4, 4);
        fb.fill_rect(2, 2, 10, 10, RED); // overflows; must clip
        assert_eq!(fb.pixel(3, 3), RED);
        assert_eq!(fb.pixel(0, 0), TRANSPARENT);
    }

    #[test]
    fn blit_composites_and_clips_offscreen() {
        let mut dst = FrameBuffer::new(4, 4);
        let mut src = FrameBuffer::new(2, 2);
        src.clear(RED);
        dst.blit(&src, 3, 3); // only the top-left source pixel lands in-bounds
        assert_eq!(dst.pixel(3, 3), RED);
        assert_eq!(dst.pixel(0, 0), TRANSPARENT);
        // Negative offset clips the top-left of the source.
        dst.blit(&src, -1, -1);
        assert_eq!(dst.pixel(0, 0), RED);
    }

    #[test]
    fn shade_runs_over_every_pixel() {
        let mut fb = FrameBuffer::new(2, 2);
        fb.clear(RED);
        // Invert RGB.
        fb.shade(|_x, _y, [r, g, b, a]| [255 - r, 255 - g, 255 - b, a]);
        assert_eq!(fb.pixel(0, 0), [0, 255, 255, 255]); // inverted red = cyan
    }

    #[test]
    fn sprite_slices_and_draws_frames() {
        // A 4x2 sheet of two 2x2 frames: frame 0 red, frame 1 blue.
        let mut sheet = FrameBuffer::new(4, 2);
        sheet.fill_rect(0, 0, 2, 2, RED);
        sheet.fill_rect(2, 0, 2, 2, BLUE);
        let sprite = Sprite::new(sheet, 2, 2);
        assert_eq!(sprite.frame_count(), 2);
        assert_eq!(sprite.frame_size(), (2, 2));

        let mut target = FrameBuffer::new(2, 2);
        sprite.draw(&mut target, 0, 0, 1); // frame 1 = blue
        assert_eq!(target.pixel(0, 0), BLUE);
        // Index wraps modulo the frame count.
        sprite.draw(&mut target, 0, 0, 2); // 2 % 2 == 0 = red
        assert_eq!(target.pixel(0, 0), RED);
    }

    #[test]
    fn to_image_data_matches_dimensions() {
        let mut fb = FrameBuffer::new(3, 2);
        fb.clear(RED);
        let img = fb.to_image_data().expect("non-empty");
        assert_eq!((img.pixel_width(), img.pixel_height()), (3, 2));
        assert!(FrameBuffer::new(0, 0).to_image_data().is_none());
    }

    #[test]
    fn view_measures_and_renders_half_blocks() {
        let theme = Theme::default();
        let mut fb = FrameBuffer::new(4, 4);
        fb.clear(RED);
        let view = FrameBufferView::new(&fb, 4, 2);
        assert_eq!(
            view.measure(Size::new(80, 24), &RenderCtx::new(&theme)),
            Size::new(4, 2)
        );
        let buf = crate::testing::render(&view, 4, 2, &theme);
        // Every cell is the half-block glyph; a fully-red buffer paints red fg/bg.
        assert_eq!(buf[(0, 0)].symbol(), "▀");
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(255, 0, 0));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn view_composites_transparency_over_theme_background() {
        let theme = Theme::default();
        // The default theme background is an Rgb color; compositing fully
        // transparent pixels over it yields that exact color back.
        let bg = theme.background;
        assert!(matches!(bg, Color::Rgb(..)));
        // A transparent buffer should read entirely as the theme background.
        let fb = FrameBuffer::new(2, 2);
        let buf = crate::testing::render(&FrameBufferView::new(&fb, 2, 1), 2, 1, &theme);
        assert_eq!(buf[(0, 0)].fg, bg);
        assert_eq!(buf[(0, 0)].bg, bg);
    }

    #[test]
    fn degenerate_sizes_do_not_panic() {
        let theme = Theme::default();
        let mut fb = FrameBuffer::new(8, 8);
        fb.clear(BLUE);
        for (w, h) in [(0u16, 0u16), (1, 1), (3, 2)] {
            let _ = crate::testing::render(&FrameBufferView::new(&fb, 8, 4), w, h, &theme);
        }
        // A zero-sized buffer renders nothing but must not panic.
        let empty = FrameBuffer::new(0, 0);
        let _ = crate::testing::render(&FrameBufferView::new(&empty, 4, 2), 4, 2, &theme);
    }

    #[test]
    fn glyph_tables_are_complete_and_distinct() {
        use std::collections::HashSet;
        assert_eq!(SEXTANTS.iter().collect::<HashSet<_>>().len(), 64);
        assert_eq!(OCTANTS.iter().collect::<HashSet<_>>().len(), 256);
        let braille: HashSet<char> = (0..=255u8).map(|m| Blitter::Braille.glyph(m)).collect();
        assert_eq!(braille.len(), 256);
        assert_eq!(SEXTANTS[0], ' ');
        assert_eq!(SEXTANTS[1], '\u{1FB00}');
        assert_eq!(SEXTANTS[62], '\u{1FB3B}');
        assert_eq!(OCTANTS[4], '\u{1CD00}', "BLOCK OCTANT-3");
        assert_eq!(OCTANTS[255], '█');
    }

    #[test]
    fn finer_blitters_agree_on_shapes_every_table_can_draw() {
        // A shape drawable at quadrant resolution — each quadrant is two
        // sextant/octant rows tall or one — must use the same character.
        for q in 0..16u8 {
            let bit = |b: u8| (q >> b) & 1;
            // Quadrant bits: 0 TL, 1 TR, 2 BL, 3 BR. Octant rows 0-1 are the
            // upper quadrants, rows 2-3 the lower ones.
            let octant = (bit(0) * 0b0000_0101)
                | (bit(1) * 0b0000_1010)
                | (bit(2) * 0b0101_0000)
                | (bit(3) * 0b1010_0000);
            assert_eq!(
                OCTANTS[octant as usize], QUADRANTS[q as usize],
                "quadrant mask {q:#06b}"
            );
        }
        // Left/right halves are the same character at every resolution.
        assert_eq!(SEXTANTS[0b01_0101], '▌');
        assert_eq!(SEXTANTS[0b10_1010], '▐');
        assert_eq!(Blitter::HalfBlock.glyph(0b10), '▄');
    }

    #[test]
    fn braille_maps_row_major_bits_to_dot_numbers() {
        assert_eq!(Blitter::Braille.glyph(0b0000_0001), '⠁'); // dot 1
        assert_eq!(Blitter::Braille.glyph(0b0000_0010), '⠈'); // dot 4
        assert_eq!(Blitter::Braille.glyph(0b0100_0000), '⡀'); // dot 7
        assert_eq!(Blitter::Braille.glyph(0b1000_0000), '⢀'); // dot 8
        assert_eq!(Blitter::Braille.glyph(0xFF), '⣿');
    }

    #[test]
    fn subcells_match_table_sizes() {
        for blitter in Blitter::ALL {
            let (w, h) = blitter.subcells();
            let full = ((1u32 << (w * h)) - 1) as u8;
            assert_ne!(blitter.glyph(full), ' ', "{blitter:?}");
            assert_eq!(
                blitter.glyph(0),
                if blitter == Blitter::Braille {
                    '⠀'
                } else {
                    ' '
                }
            );
        }
    }

    #[test]
    fn split_picks_the_two_colors_and_the_brighter_as_ink() {
        let white = [255, 255, 255];
        let black = [0, 0, 0];
        let (mask, fg, bg) = split(&[white, black, black, white]);
        assert_eq!((mask, fg, bg), (0b1001, white, black));
        let (mask, fg, bg) = split(&[black; 4]);
        assert_eq!((mask, fg, bg), (0, black, black));
    }

    #[test]
    fn view_packs_a_diagonal_into_one_octant_cell() {
        // A 2×4 image: white on the top-left/bottom-right diagonal halves.
        let mut fb = FrameBuffer::new(2, 4);
        fb.clear([0, 0, 0, 255]);
        for (x, y) in [(0, 0), (0, 1), (1, 2), (1, 3)] {
            fb.set(x, y, [255, 255, 255, 255]);
        }
        let theme = Theme::default();
        let view = FrameBufferView::new(&fb, 1, 1).blitter(Blitter::Octant);
        let buf = crate::testing::render(&view, 1, 1, &theme);
        assert_eq!(buf[(0, 0)].symbol(), "▚");
        assert_eq!(buf[(0, 0)].fg, Color::Rgb(255, 255, 255));
        assert_eq!(buf[(0, 0)].bg, Color::Rgb(0, 0, 0));

        let sextant = FrameBufferView::new(&fb, 1, 1).blitter(Blitter::Sextant);
        let buf = crate::testing::render(&sextant, 1, 1, &theme);
        assert!(SEXTANTS.contains(&buf[(0, 0)].symbol().chars().next().unwrap()));
    }

    #[test]
    fn every_blitter_survives_degenerate_sizes() {
        let mut fb = FrameBuffer::new(3, 5);
        fb.clear(RED);
        for blitter in Blitter::ALL {
            for (w, h) in [(0, 0), (1, 1), (7, 3), (40, 20)] {
                let view = FrameBufferView::new(&fb, 7, 3).blitter(blitter);
                let _ = crate::testing::render(&view, w, h, &Theme::default());
            }
        }
    }
}
