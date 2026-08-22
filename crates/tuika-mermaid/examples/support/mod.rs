use std::io;

use tuika::Theme;

pub fn theme_and_args() -> io::Result<(Theme, Vec<String>)> {
    let mut source = std::env::args().skip(1);
    let mut args = Vec::new();
    let mut theme = None;
    while let Some(arg) = source.next() {
        let name = if arg == "--theme" {
            Some(source.next().ok_or_else(|| invalid("missing theme name"))?)
        } else {
            arg.strip_prefix("--theme=").map(str::to_owned)
        };
        if let Some(name) = name {
            if theme.is_some() {
                return Err(invalid("--theme may only be supplied once"));
            }
            theme = Some(
                tuika::themes::by_name(&name)
                    .ok_or_else(|| invalid(&format!("unknown theme {name:?}")))?,
            );
        } else {
            args.push(arg);
        }
    }
    Ok((theme.unwrap_or_default(), args))
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, detail)
}
