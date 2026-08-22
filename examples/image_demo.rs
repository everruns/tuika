//! Generate `docs/demos/image.svg` — the demo for the Images section of
//! `docs/features.md`.
//!
//! Why not VHS (the tool the other demos use)? VHS records through `ttyd` +
//! `xterm.js`, which does **not** implement the Kitty graphics protocol, so it
//! can only ever capture this feature's *fallback* — never the pixels. So, like
//! the hero image in `examples/screenshot.rs`, this demo is **generated from the
//! real render** rather than recorded: the picture is the actual RGBA
//! [`ImageData`](tuika::ImageData) the component transmits (embedded as a PNG),
//! and the fallback panel shows the exact placeholder string the component
//! paints. Only the surrounding window chrome is presentational.
//!
//! Regenerate after changing the look:
//!
//! ```text
//! cargo run --example image_demo                         # real terminal
//! cargo run --example image_demo -- out.svg              # write SVG
//! cargo run --example image_demo -- docs/demos/image.svg # write the doc asset
//! cargo run --example image_demo -- --theme gruvbox-dark # themed terminal
//! ```

use std::io;

use ratatui::style::Color;
use tuika::Theme;

#[path = "support/image_app.rs"]
mod image_app;
mod support;

/// Source resolution of the embedded gradient. Small on purpose — it's a smooth
/// gradient, so it upscales cleanly and keeps the committed SVG tiny.
const IMG_W: u32 = 96;
const IMG_H: u32 = 48;

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    let out = match support::Output::parse(&cli.args)? {
        support::Output::Terminal => return image_app::run(&cli.theme),
        support::Output::Svg(out) => out,
    };

    let data = image_app::gradient(IMG_W, IMG_H);
    let png = png_rgba(
        data.pixel_width(),
        data.pixel_height(),
        &image_app::rgba(IMG_W, IMG_H),
    );
    let svg = render_svg(&png, &cli.theme);

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, svg)?;
    println!("wrote {}", out.display());
    // Touch `data` so the real component type stays in the loop (the PNG above is
    // the same pixels it would transmit).
    debug_assert_eq!(data.pixel_width() * data.pixel_height(), IMG_W * IMG_H);
    Ok(())
}

// ---- SVG ------------------------------------------------------------------

fn render_svg(png: &[u8], theme: &Theme) -> String {
    let fallback = Theme::default();
    let palette = SvgPalette {
        bg: hex(theme.background, fallback.background),
        surface: hex(theme.surface, fallback.surface),
        muted: hex(theme.muted, fallback.muted),
        dim: hex(theme.dim, fallback.dim),
        accent: hex(theme.accent, fallback.accent),
    };
    let b64 = base64(png);
    let mut s = String::new();
    s.push_str(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 880 300" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace">
"#,
    );
    // Two terminal "cards": left shows the rendered image, right the fallback.
    card(&mut s, 8.0, "Kitty · Ghostty · WezTerm · Konsole", &palette);
    card(&mut s, 448.0, "every other terminal", &palette);

    // Left card content: the actual gradient pixels, then the app's status line.
    s.push_str(&format!(
        r#"<clipPath id="pic"><rect x="32" y="64" width="368" height="160" rx="4"/></clipPath>
<image x="32" y="64" width="368" height="160" preserveAspectRatio="none" clip-path="url(#pic)" href="data:image/png;base64,{b64}"/>
"#
    ));
    status(
        &mut s,
        32.0,
        &palette.accent,
        " graphics: Kitty protocol detected ",
        &palette.surface,
    );

    // Right card content: the exact fallback placeholder, centered and dimmed.
    s.push_str(&format!(
        r#"<text x="656" y="148" text-anchor="middle" font-size="15" font-style="italic" fill="{muted}">[image: {alt}]</text>
"#
        , muted = palette.muted, alt = image_app::ALT
    ));
    status(
        &mut s,
        472.0,
        &palette.muted,
        " graphics: none — showing text fallback ",
        &palette.surface,
    );

    s.push_str("</svg>\n");
    s
}

struct SvgPalette {
    bg: String,
    surface: String,
    muted: String,
    dim: String,
    accent: String,
}

/// A rounded terminal-window card at `x` with three window dots and a `label`.
fn card(s: &mut String, x: f32, label: &str, palette: &SvgPalette) {
    s.push_str(&format!(
        r#"<rect x="{x}" y="8" width="424" height="284" rx="12" fill="{bg}" stroke="{dim}"/>
<rect x="{x}" y="8" width="424" height="40" rx="12" fill="{surface}"/>
<rect x="{x}" y="34" width="424" height="14" fill="{surface}"/>
"#,
        bg = palette.bg,
        dim = palette.dim,
        surface = palette.surface,
    ));
    for (i, c) in [palette.accent.as_str(), "#c9a227", "#3fae6b"]
        .iter()
        .enumerate()
    {
        let cx = x + 24.0 + i as f32 * 18.0;
        s.push_str(&format!(r#"<circle cx="{cx}" cy="28" r="5" fill="{c}"/>"#));
        s.push('\n');
    }
    let lx = x + 212.0;
    s.push_str(&format!(
        r#"<text x="{lx}" y="33" text-anchor="middle" font-size="12" fill="{muted}">{label}</text>
"#,
        muted = palette.muted,
    ));
}

/// The app's status bar line at the bottom of a card, starting at `x`.
fn status(s: &mut String, x: f32, fg: &str, text: &str, surface: &str) {
    let y = 268.0;
    s.push_str(&format!(
        r#"<rect x="{x}" y="{}" width="376" height="22" rx="4" fill="{surface}"/>
<text x="{}" y="{}" font-size="12" fill="{fg}">{text}</text>
"#,
        y - 15.0,
        x + 8.0,
        y - 0.0
    ));
}

fn hex(color: Color, fallback: Color) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Indexed(index) => indexed_hex(index),
        Color::Reset => hex(fallback, Color::Black),
        Color::Black => "#000000".to_owned(),
        Color::Red | Color::LightRed => "#f14c4c".to_owned(),
        Color::Green | Color::LightGreen => "#23d18b".to_owned(),
        Color::Yellow | Color::LightYellow => "#f5f543".to_owned(),
        Color::Blue | Color::LightBlue => "#3b8eea".to_owned(),
        Color::Magenta | Color::LightMagenta => "#d670d6".to_owned(),
        Color::Cyan | Color::LightCyan => "#29b8db".to_owned(),
        Color::Gray | Color::DarkGray | Color::White => "#e5e5e5".to_owned(),
    }
}

fn indexed_hex(index: u8) -> String {
    const BASE: [&str; 16] = [
        "#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5",
        "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff",
    ];
    match index {
        0..=15 => BASE[index as usize].to_owned(),
        16..=231 => {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let n = index - 16;
            format!(
                "#{:02x}{:02x}{:02x}",
                LEVELS[(n / 36) as usize],
                LEVELS[((n % 36) / 6) as usize],
                LEVELS[(n % 6) as usize]
            )
        }
        _ => {
            let value = 8 + 10 * (index - 232);
            format!("#{value:02x}{value:02x}{value:02x}")
        }
    }
}

// ---- Minimal PNG (RFC 2083) + base64, dependency-free ---------------------

/// Encode `rgba` (`w × h`, 8-bit RGBA) as a PNG. Uses stored (uncompressed)
/// DEFLATE blocks — trivial and exact, and the image is small — so tuika keeps
/// its minimal dependency set even in the demo tooling.
fn png_rgba(w: u32, h: u32, rgba: &[u8]) -> Vec<u8> {
    let row = (w * 4) as usize;
    let mut raw = Vec::with_capacity(rgba.len() + h as usize);
    for y in 0..h as usize {
        raw.push(0); // filter type 0 (none)
        raw.extend_from_slice(&rgba[y * row..(y + 1) * row]);
    }

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit, RGBA, default compression/filter/interlace
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &zlib_stored(&raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc_in = Vec::with_capacity(4 + data.len());
    crc_in.extend_from_slice(tag);
    crc_in.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_in).to_be_bytes());
}

/// A zlib stream wrapping `data` in DEFLATE stored blocks (BTYPE 00).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // zlib header (deflate, 32K window, no preset dict)
    let mut i = 0;
    loop {
        let end = (i + 0xffff).min(data.len());
        let block = &data[i..end];
        let last = end == data.len();
        out.push(u8::from(last)); // BFINAL, BTYPE=00
        let len = block.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
        i = end;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn base64(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for g in bytes.chunks(3) {
        let n = (g[0] as u32) << 16
            | (*g.get(1).unwrap_or(&0) as u32) << 8
            | *g.get(2).unwrap_or(&0) as u32;
        out.push(A[(n >> 18) as usize & 63] as char);
        out.push(A[(n >> 12) as usize & 63] as char);
        out.push(if g.len() > 1 {
            A[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if g.len() > 2 {
            A[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
