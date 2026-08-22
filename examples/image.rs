//! Terminal images via the Kitty graphics protocol. Run with
//! `cargo run --example image` (q or esc to quit).
//!
//! Builds a synthetic RGBA gradient (no image-decoding dependency needed) and
//! shows it centered on screen. Capability is auto-detected: on Kitty, Ghostty,
//! WezTerm, or Konsole the real pixels are painted; anywhere else the same
//! [`Image`](tuika::Image) view degrades to its alt-text placeholder.
//!
//! [`Runner`](tuika::Runner) detects graphics support, collects image placements,
//! and emits them after each cell frame. The application only describes the
//! image and its fallback.

use std::io;

#[path = "support/image_app.rs"]
mod image_app;
mod support;

fn main() -> io::Result<()> {
    let cli = support::Cli::parse()?;
    if !cli.args.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: cargo run --example image [-- --theme NAME]",
        ));
    }
    image_app::run(&cli.theme)
}
