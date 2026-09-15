//! Manifest syntax that the MSRV's Cargo must be able to parse.
//!
//! TOML 1.0 allows a newline inside an inline table only where it is part of a
//! value — an array may wrap, the table itself may not. Newer Cargo accepts the
//! fully multi-line form as an extension, so a manifest written that way builds
//! on the development toolchain `rust-toolchain.toml` pins and fails only on the
//! MSRV, where Cargo rejects it before compiling anything.
//!
//! That break reached `main` once already, because the MSRV CI job was checking
//! the development toolchain: the `rust-toolchain.toml` pin outranks the
//! `rustup default` the install action sets. CI now pins `RUSTUP_TOOLCHAIN` and
//! asserts the active version; this test catches the same mistake locally, on
//! whatever toolchain the contributor happens to be running.

use std::path::{Path, PathBuf};

/// Every manifest in the workspace: the root package plus each member.
fn workspace_manifests() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut found = vec![root.join("Cargo.toml")];

    let members = std::fs::read_dir(root.join("crates")).expect("crates/ must exist");
    let mut member_manifests: Vec<PathBuf> = members
        .map(|entry| {
            entry
                .expect("readable crates/ entry")
                .path()
                .join("Cargo.toml")
        })
        .filter(|manifest| manifest.is_file())
        .collect();
    member_manifests.sort();

    assert!(
        !member_manifests.is_empty(),
        "expected at least one workspace member under crates/"
    );
    found.extend(member_manifests);
    found
}

/// 1-based line numbers where a newline falls directly inside `{ }` rather than
/// inside a value — the construct TOML 1.0 forbids and the MSRV's Cargo refuses.
///
/// Braces and brackets are counted outside of comments and strings; a wrapped
/// array is a value, so a newline while inside `[ ]` is fine.
fn unterminated_inline_tables(toml: &str) -> Vec<usize> {
    #[derive(PartialEq)]
    enum Ctx {
        Code,
        Comment,
        Basic,
        Literal,
        MultiBasic,
        MultiLiteral,
    }

    let mut ctx = Ctx::Code;
    let mut braces: i32 = 0;
    let mut brackets: i32 = 0;
    let mut line = 1;
    let mut offenders = Vec::new();
    let bytes: Vec<char> = toml.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        let ch = bytes[i];
        let rest3 = |i: usize| -> bool {
            bytes.len() >= i + 3 && bytes[i] == bytes[i + 1] && bytes[i + 1] == bytes[i + 2]
        };

        if ch == '\n' {
            if matches!(ctx, Ctx::Comment) {
                ctx = Ctx::Code;
            }
            if matches!(ctx, Ctx::Code) && braces > 0 && brackets == 0 {
                offenders.push(line);
            }
            line += 1;
            i += 1;
            continue;
        }

        match ctx {
            Ctx::Comment => {}
            Ctx::Basic => match ch {
                '\\' => i += 1,
                '"' => ctx = Ctx::Code,
                _ => {}
            },
            Ctx::Literal => {
                if ch == '\'' {
                    ctx = Ctx::Code;
                }
            }
            Ctx::MultiBasic => {
                if ch == '\\' {
                    i += 1;
                } else if ch == '"' && rest3(i) {
                    ctx = Ctx::Code;
                    i += 2;
                }
            }
            Ctx::MultiLiteral => {
                if ch == '\'' && rest3(i) {
                    ctx = Ctx::Code;
                    i += 2;
                }
            }
            Ctx::Code => match ch {
                '#' => ctx = Ctx::Comment,
                '"' if rest3(i) => {
                    ctx = Ctx::MultiBasic;
                    i += 2;
                }
                '\'' if rest3(i) => {
                    ctx = Ctx::MultiLiteral;
                    i += 2;
                }
                '"' => ctx = Ctx::Basic,
                '\'' => ctx = Ctx::Literal,
                '{' => braces += 1,
                '}' => braces -= 1,
                '[' => brackets += 1,
                ']' => brackets -= 1,
                _ => {}
            },
        }
        i += 1;
    }

    offenders
}

#[test]
fn no_manifest_opens_a_multi_line_inline_table() {
    let mut offenders = Vec::new();

    for manifest in workspace_manifests() {
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|err| panic!("reading {}: {err}", manifest.display()));
        let lines: Vec<&str> = text.lines().collect();

        for line_number in unterminated_inline_tables(&text) {
            offenders.push(format!(
                "{}:{}: {}",
                manifest.display(),
                line_number,
                lines[line_number - 1].trim()
            ));
        }
    }

    assert!(
        offenders.is_empty(),
        "an inline table must not span lines — Cargo on tuika's MSRV rejects the \
         multi-line form and the manifest will not parse at all:\n  {}",
        offenders.join("\n  ")
    );
}

#[test]
fn the_scanner_separates_a_wrapped_array_from_a_wrapped_table() {
    // The break this test exists to catch.
    assert_eq!(
        unterminated_inline_tables("dep = {\n  version = \"1\",\n}\n"),
        vec![1, 2]
    );

    // Legal TOML 1.0: the table is one line, the array value inside it wraps.
    assert_eq!(
        unterminated_inline_tables("dep = { features = [\n  \"a\",\n] }\n"),
        Vec::<usize>::new()
    );
    assert_eq!(
        unterminated_inline_tables("dep = { version = \"1\" }\n"),
        Vec::<usize>::new()
    );

    // Braces in comments and strings are not table delimiters.
    assert_eq!(
        unterminated_inline_tables("# a { comment\nname = \"a { brace\"\n"),
        Vec::<usize>::new()
    );
    assert_eq!(
        unterminated_inline_tables("name = \"escaped \\\" { quote\"\n"),
        Vec::<usize>::new()
    );
    assert_eq!(
        unterminated_inline_tables("name = '''\na { brace\n'''\n"),
        Vec::<usize>::new()
    );

    // A table with no key-values still must not be split.
    assert_eq!(unterminated_inline_tables("dep = {\n}\n"), vec![1]);
}
