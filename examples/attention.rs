//! Run in a real terminal; notifications depend on terminal and OS settings.
use std::io;
use tuika::term::attention::{self, FocusReporting, Protocol};

fn main() -> io::Result<()> {
    let _focus = FocusReporting::enable(io::stdout())?;
    attention::write(
        &mut io::stdout(),
        "Tuika",
        "Attention protocol smoke test",
        Protocol::Osc777,
    )?;
    println!("Focus reporting enabled; notification sent. Cleanup restores focus reporting.");
    Ok(())
}
