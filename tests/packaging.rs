//! Guards the published `.crate` contents.
//!
//! Two things must stay out of the tarball, and `Cargo.toml`'s `exclude` is the
//! only thing keeping them out:
//!
//! - The demo/showcase/theme/styling assets under `docs/`, and the `codex`
//!   example's own recording, are ~14 MiB and are read only from repository
//!   docs or absolute rustdoc URLs; docs.rs builds the crate front page from the
//!   hand-written `//!` header in `lib.rs`, which references no images.
//! - Repository machinery. tuika is the *root* package of its repo, so the
//!   internal knowledge bundle, agent skills, CI definitions, and
//!   asset-generation scripts all sit beside it and would otherwise ship.
//!   `tuika-codeformatters` has one GitHub-only recording of its own
//!   (`docs/languages.gif`) under the same rule.
//!
//! This test drives the real packaging path (`cargo package --list`) so a stray
//! asset — or a regression that drops an image the crates.io README needs —
//! fails loudly instead of silently re-inflating the crate.
//!
//! It guards all five published crates. The packaging rule is repository-wide,
//! so the root package checks every companion in one place rather than
//! scattering equivalent subprocess tests. Which way it falls for a given
//! recording is decided by how that crate's
//! README embeds it: an absolute `raw.githubusercontent.com` URL means the
//! packaged copy is unreachable and must not ship (`tuika-codeformatters` and
//! `tuika-charts`), while a relative path means crates.io renders from the
//! packaged copy and it must (`tuika-mermaid`, `tuika-html`, and tuika's four
//! README assets).

use std::process::Command;

/// Ask cargo which files it would put in `package`'s `.crate`, exactly as
/// `publish` would. `--list` resolves the manifest's `include`/`exclude` without
/// building, so this stays cheap even for the tree-sitter-heavy member.
fn packaged_files(package: &str) -> Vec<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "package",
            "--list",
            "--quiet",
            "--allow-dirty",
            "-p",
            package,
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run `cargo package --list`");
    assert!(
        output.status.success(),
        "`cargo package --list` failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 file list")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn heavy_doc_assets_are_excluded_from_the_package() {
    let files = packaged_files("tuika");

    // No demo/showcase/theme/styling asset, and no example recording, may ship:
    // those are GitHub-only assets. The two recordings the crates.io README
    // embeds by relative path — the hero and the split footer — sit directly
    // under `docs/` for exactly this reason, and are asserted below.
    let bundled_heavy_assets: Vec<&String> = files
        .iter()
        .filter(|f| {
            (f.ends_with(".gif") || f.ends_with(".png"))
                && (f.starts_with("docs/demos/")
                    || f.starts_with("docs/showcases/")
                    || f.starts_with("examples/codex/")
                    || f.starts_with("docs/styling/")
                    || f.starts_with("docs/themes/"))
        })
        .collect();
    assert!(
        bundled_heavy_assets.is_empty(),
        "these assets must not ship in the crate (see Cargo.toml `exclude`): {bundled_heavy_assets:?}"
    );
}

#[test]
fn repository_machinery_is_excluded_from_the_package() {
    let files = packaged_files("tuika");

    // Internal-only trees that live beside the root package in the repo.
    const INTERNAL_PREFIXES: [&str; 5] =
        [".agents/", ".claude/", ".github/", "knowledge/", "scripts/"];
    const INTERNAL_FILES: [&str; 6] = [
        "AGENTS.md",
        "CLAUDE.md",
        "CODE_OF_CONDUCT.md",
        "CONTRIBUTING.md",
        "SECURITY.md",
        // Pins the development toolchain; it contradicts the declared MSRV.
        "rust-toolchain.toml",
    ];
    let leaked: Vec<&String> = files
        .iter()
        .filter(|f| {
            INTERNAL_PREFIXES.iter().any(|p| f.starts_with(p))
                || INTERNAL_FILES.iter().any(|n| f == n)
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "repository-only files must not ship in the crate (see Cargo.toml `exclude`): {leaked:?}"
    );

    // The workspace member is a package of its own; cargo must not fold it in.
    let nested: Vec<&String> = files.iter().filter(|f| f.starts_with("crates/")).collect();
    assert!(
        nested.is_empty(),
        "nested workspace packages must not ship inside tuika's crate: {nested:?}"
    );
}

#[test]
fn crates_io_readme_assets_and_source_are_kept() {
    let files = packaged_files("tuika");
    let has = |p: &str| files.iter().any(|f| f == p);

    // The assets the crates.io README embeds by relative path.
    assert!(has("logo.svg"), "README logo must ship");
    assert!(has("docs/hero.gif"), "README hero image must ship");
    assert!(
        has("docs/demos/image.svg"),
        "README image-protocol asset must ship"
    );
    assert!(
        has("docs/split-footer.gif"),
        "README split-footer recording must ship"
    );
    // Sanity: the crate still carries its source and manifest.
    assert!(has("src/lib.rs"), "library source must ship");
    assert!(has("Cargo.toml"), "manifest must ship");
    assert!(has("README.md"), "README must ship");
}

#[test]
fn alternate_logo_assets_are_repository_only() {
    let files = packaged_files("tuika");

    for asset in [
        "logo-dark.svg",
        "logo-mono.svg",
        "logo.png",
        "logo-dark.png",
        "logo-mono.png",
    ] {
        assert!(
            !files.iter().any(|f| f == asset),
            "non-README logo asset must not ship: {asset}"
        );
    }
}

#[test]
fn codeformatters_ships_source_but_not_its_demo_recording() {
    let files = packaged_files("tuika-codeformatters");

    // `docs/languages.gif` is embedded by absolute raw.githubusercontent.com URL
    // in the member's README, so no crate consumer can reach the packaged copy.
    let gifs: Vec<&String> = files.iter().filter(|f| f.ends_with(".gif")).collect();
    assert!(
        gifs.is_empty(),
        "demo recordings must not ship in tuika-codeformatters (see its Cargo.toml `exclude`): {gifs:?}"
    );

    let has = |p: &str| files.iter().any(|f| f == p);
    assert!(has("src/lib.rs"), "library source must ship");
    assert!(
        has("examples/highlight_file.rs"),
        "the file-viewer example documented in the README must ship"
    );
    assert!(has("Cargo.toml"), "manifest must ship");
    assert!(has("README.md"), "README must ship");
}

#[test]
fn html_keeps_the_demo_its_readme_embeds() {
    let files = packaged_files("tuika-html");
    let has = |p: &str| files.iter().any(|f| f == p);

    // Same rule as `tuika-mermaid`: the README embeds the demo by *relative*
    // path, so the packaged copy is what crates.io renders.
    for asset in [
        "examples/html_markdown/html.png",
        "examples/html_view/html_view.png",
    ] {
        assert!(
            has(asset),
            "the demos the crates.io README embeds by relative path must ship: {asset}"
        );
    }
    assert!(has("src/lib.rs"), "library source must ship");
    assert!(has("Cargo.toml"), "manifest must ship");
    assert!(has("README.md"), "README must ship");
}

#[test]
fn mermaid_keeps_the_recording_its_readme_embeds() {
    let files = packaged_files("tuika-mermaid");
    let has = |p: &str| files.iter().any(|f| f == p);

    // The inverse of the case above, and the reason this crate has no `exclude`:
    // its README reaches the recording by *relative* path, so crates.io renders
    // from the packaged copy and dropping it would break that page.
    assert!(
        has("examples/mermaid_markdown/mermaid.gif"),
        "the recording the crates.io README embeds by relative path must ship"
    );
    assert!(has("src/lib.rs"), "library source must ship");
    assert!(has("Cargo.toml"), "manifest must ship");
    assert!(has("README.md"), "README must ship");
}

#[test]
fn charts_ships_source_and_readme_without_repository_demo() {
    let files = packaged_files("tuika-charts");
    let has = |p: &str| files.iter().any(|f| f == p);

    assert!(has("src/lib.rs"), "library source must ship");
    assert!(has("examples/charts.rs"), "runnable example must ship");
    assert!(
        !files.iter().any(|path| path.starts_with("docs/charts/")),
        "repository-hosted chart demos must not ship"
    );
    assert!(has("Cargo.toml"), "manifest must ship");
    assert!(has("README.md"), "README must ship");

    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for kind in ["line", "area", "bar", "scatter", "step"] {
        for renderer in ["cells", "graphics"] {
            let path = root.join(format!("docs/charts/{kind}-{renderer}.png"));
            let metadata = path
                .metadata()
                .unwrap_or_else(|_| panic!("missing generated chart demo: {}", path.display()));
            assert!(
                metadata.len() > 0,
                "generated chart demo is empty: {}",
                path.display()
            );
        }
    }
}
