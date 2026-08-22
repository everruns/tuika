//! Guards the published `.crate` contents.
//!
//! Three things must stay out of the tarball, and `Cargo.toml`'s `exclude` is the
//! only thing keeping them out:
//!
//! - The root public `docs/` tree and the in-repo application showcase recordings
//!   are read only from the tagged repository; docs.rs builds the crate front
//!   page from the hand-written `//!` header in `lib.rs`, which references neither.
//! - Repository machinery. tuika is the *root* package of its repo, so the
//!   internal knowledge bundle, agent skills, CI definitions, and
//!   asset-generation scripts all sit beside it and would otherwise ship. The
//!   generated documentation site is repository-only too; crates.io renders
//!   `README.md` directly and cannot use the site bundle.
//!   `tuika-codeformatters` has one GitHub-only recording of its own
//!   (`docs/languages.gif`) under the same rule.
//! - Benchmark output. Cargo naturally leaves generated `target/` reports out,
//!   while manifest exclusions keep committed IAI result snapshots out. The
//!   benchmark sources still ship and remain reproducible.
//!
//! This test drives the real packaging path (`cargo package --list`) so a stray
//! repository file or stale README tag pin fails loudly instead of silently
//! re-inflating the crate or breaking a published guide link.
//!
//! It guards all five published crates. The packaging rule is repository-wide,
//! so the root package checks every companion in one place rather than
//! scattering equivalent subprocess tests. Which way it falls for a given
//! recording is decided by how that crate's
//! README embeds it: an absolute `raw.githubusercontent.com` URL means the
//! packaged copy is unreachable and must not ship (`tuika`,
//! `tuika-codeformatters`, and `tuika-charts`), while a relative path means
//! crates.io renders from the packaged copy and it must (`tuika-mermaid` and
//! `tuika-html`).

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
fn public_docs_are_tag_pinned_and_excluded_from_the_package() {
    let files = packaged_files("tuika");
    let bundled_docs: Vec<&String> = files.iter().filter(|f| f.starts_with("docs/")).collect();
    assert!(
        bundled_docs.is_empty(),
        "public docs must stay in the tagged repository, not the crate: {bundled_docs:?}"
    );

    let readme = std::fs::read_to_string("README.md").expect("read root README");
    let version = env!("CARGO_PKG_VERSION");
    let expected_prefix = format!("https://github.com/everruns/tuika/blob/v{version}/docs/");
    let doc_links: Vec<&str> = readme
        .split("](")
        .skip(1)
        .filter_map(|rest| rest.split(')').next())
        .filter(|destination| destination.contains("docs/"))
        .collect();
    assert!(!doc_links.is_empty(), "README must link to the public docs");
    assert!(
        doc_links
            .iter()
            .all(|destination| destination.starts_with(&expected_prefix)),
        "README doc links must be pinned to v{version}: {doc_links:?}"
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
fn generated_site_is_excluded_from_the_package() {
    let files = packaged_files("tuika");
    let leaked: Vec<&String> = files.iter().filter(|f| f.starts_with("site/")).collect();

    assert!(
        leaked.is_empty(),
        "the generated documentation site must not ship in the crate: {leaked:?}"
    );
}

#[test]
fn in_repo_showcase_recordings_are_excluded_from_the_package() {
    let files = packaged_files("tuika");

    for asset in [
        "examples/codex/codex.gif",
        "examples/workbench_demo/workbench-demo.gif",
    ] {
        assert!(
            !files.iter().any(|file| file == asset),
            "repository-hosted showcase recording must not ship: {asset}"
        );
    }
}

#[test]
fn benchmark_results_are_excluded_from_every_package() {
    for package in [
        "tuika",
        "tuika-charts",
        "tuika-codeformatters",
        "tuika-html",
        "tuika-mermaid",
    ] {
        let files = packaged_files(package);
        let leaked: Vec<&String> = files
            .iter()
            .filter(|path| {
                path.starts_with("target/")
                    || (path.starts_with("benches/") && path.ends_with(".json"))
            })
            .collect();
        assert!(
            leaked.is_empty(),
            "benchmark results must not ship in {package}: {leaked:?}"
        );
    }
}

#[test]
fn root_readme_assets_use_their_intended_refs_and_are_excluded() {
    let files = packaged_files("tuika");
    let has = |p: &str| files.iter().any(|f| f == p);

    let readme = std::fs::read_to_string("README.md").expect("read root README");
    let version = env!("CARGO_PKG_VERSION");
    let logo_url = format!("https://raw.githubusercontent.com/everruns/tuika/v{version}/logo.svg");
    assert!(
        readme.contains(&logo_url),
        "README must pin logo.svg to v{version}"
    );
    for asset in ["docs/hero.gif", "docs/demos/image.svg"] {
        let url = format!("https://raw.githubusercontent.com/everruns/tuika/main/{asset}");
        assert!(readme.contains(&url), "README must track main for {asset}");
    }
    assert!(
        !readme.contains("docs/split-footer.gif"),
        "split-footer recording belongs in the focused guide, not the root README"
    );

    for asset in [
        "logo.svg",
        "docs/hero.gif",
        "docs/demos/image.svg",
        "docs/split-footer.gif",
    ] {
        assert!(!has(asset), "absolute README asset must not ship: {asset}");
    }

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
