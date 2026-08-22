use std::io;
use std::path::PathBuf;

use tuika::Theme;

/// Common command-line options shared by the runnable examples.
pub struct Cli {
    pub theme: Theme,
    #[allow(dead_code)]
    pub theme_name: Option<String>,
    #[allow(dead_code)]
    pub args: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
#[allow(dead_code)]
pub enum Output {
    Terminal,
    Svg(PathBuf),
}

impl Output {
    #[allow(dead_code)]
    pub fn parse(args: &[String]) -> io::Result<Self> {
        match args {
            [] => Ok(Self::Terminal),
            [path] if !path.starts_with('-') => Ok(Self::Svg(PathBuf::from(path))),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected at most one SVG output path",
            )),
        }
    }
}

impl Cli {
    pub fn parse() -> io::Result<Self> {
        Self::parse_from(std::env::args().skip(1))
    }

    pub(crate) fn parse_from(args: impl IntoIterator<Item = String>) -> io::Result<Self> {
        let mut args = args.into_iter();
        let mut rest = Vec::new();
        let mut theme_name = None;

        while let Some(arg) = args.next() {
            if arg == "--theme" {
                let name = args
                    .next()
                    .ok_or_else(|| invalid_theme("missing theme name"))?;
                set_theme_name(&mut theme_name, name)?;
            } else if let Some(name) = arg.strip_prefix("--theme=") {
                set_theme_name(&mut theme_name, name.to_owned())?;
            } else {
                rest.push(arg);
            }
        }

        let theme = match theme_name.as_deref() {
            Some(name) => tuika::themes::by_name(name)
                .ok_or_else(|| invalid_theme(&format!("unknown theme {name:?}")))?,
            None => Theme::default(),
        };
        Ok(Self {
            theme,
            theme_name,
            args: rest,
        })
    }
}

fn set_theme_name(current: &mut Option<String>, name: String) -> io::Result<()> {
    if current.replace(name).is_some() {
        return Err(invalid_theme("--theme may only be supplied once"));
    }
    Ok(())
}

fn invalid_theme(detail: &str) -> io::Error {
    let names = tuika::themes::PRESETS
        .iter()
        .map(|preset| preset.name)
        .collect::<Vec<_>>()
        .join(", ");
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{detail}; available themes: {names}"),
    )
}
