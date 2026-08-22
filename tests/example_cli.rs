#[path = "../examples/support/mod.rs"]
mod support;

use support::{Cli, Output};

#[test]
fn theme_is_removed_from_example_specific_arguments() {
    let _environment_entry_point: fn() -> std::io::Result<Cli> = Cli::parse;
    let cli = Cli::parse_from([
        "out.svg".to_owned(),
        "--theme".to_owned(),
        "gruvbox-dark".to_owned(),
        "--dump".to_owned(),
    ])
    .unwrap();

    assert_eq!(cli.theme, tuika::themes::GRUVBOX_DARK);
    assert_eq!(cli.theme_name.as_deref(), Some("gruvbox-dark"));
    assert_eq!(cli.args, ["out.svg", "--dump"]);
}

#[test]
fn equals_syntax_selects_a_theme() {
    let cli = Cli::parse_from(["--theme=light".to_owned()]).unwrap();

    assert_eq!(cli.theme, tuika::themes::LIGHT);
    assert!(cli.args.is_empty());
}

#[test]
fn unknown_and_repeated_themes_are_rejected() {
    assert!(Cli::parse_from(["--theme=nope".to_owned()]).is_err());
    assert!(Cli::parse_from(["--theme=light".to_owned(), "--theme=dracula".to_owned(),]).is_err());
}

#[test]
fn no_output_path_selects_the_terminal_and_a_path_selects_svg() {
    assert_eq!(Output::parse(&[]).unwrap(), Output::Terminal);
    assert_eq!(
        Output::parse(&["docs/hero.svg".to_owned()]).unwrap(),
        Output::Svg("docs/hero.svg".into())
    );
    assert!(Output::parse(&["--dump".to_owned()]).is_err());
    assert!(Output::parse(&["one.svg".to_owned(), "two.svg".to_owned()]).is_err());
}
