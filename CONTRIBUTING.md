# Contributing

Thanks for helping improve tuika. Issues and pull requests are welcome at
[everruns/tuika](https://github.com/everruns/tuika).

## Development

This repository is a Cargo workspace: the root package is the `tuika` library
and `crates/tuika-codeformatters/` is its tree-sitter syntax-highlighting
companion. Everything below runs from the repository root and covers both.

```bash
cargo build
cargo test --all-features
```

Format and lint before opening a pull request:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
```

Try a change in a real terminal (press `q` or `Esc` to quit):

```bash
cargo run --example gallery    # motion components + native OSC 9;4 progress
cargo run --example markdown   # streaming markdown + highlighted code blocks
cargo run --example demo -- list
```

### Minimum supported Rust version

tuika supports Rust **1.88** and up. `rust-toolchain.toml` pins a newer
toolchain for development, so an accidental MSRV bump will not fail locally — a
dedicated CI job compiles the crate on 1.88. Avoid newer language features
unless raising the MSRV is intentional and noted in the changelog.

### Benchmarks

Criterion benchmarks live in `benches/`. Compare a change against a saved
baseline rather than against absolute numbers:

```bash
cargo bench --bench markdown -- --save-baseline before
cargo bench --bench markdown -- --baseline before
```

The `*_iai` benchmarks count CPU instructions under Valgrind and are gated in
CI against a committed baseline. They need `valgrind` and a version-matched
`cargo install iai-callgrind-runner` to run locally.

## Documentation and demo assets

`README.md` and `docs/` are the public documentation. The animated demos under
`docs/demos/` are generated from `examples/demo.rs` by `scripts/gen-demos.sh`
(which needs [VHS](https://github.com/charmbracelet/vhs) with `ttyd` and
`ffmpeg`); `cargo run --example demo -- check` verifies that the scenes, the
recordings, and the documentation references stay in sync, and it runs in CI.

## Pull requests

- Commit messages and PR titles follow
  [Conventional Commits](https://www.conventionalcommits.org/)
  (`feat(scroll): …`, `fix(markdown): …`, `docs: …`, `chore: …`), with the title
  under 70 characters.
- Fill in `.github/pull_request_template.md`. For visual changes, include a
  before/after — a recording or a `cargo run --example demo -- <scene> --dump`
  capture.
- Behavior changes need a test that exercises the new behavior. Rendering is
  tested hermetically against an in-memory buffer via `tuika::testing`, so most
  changes are testable without a terminal.
- tuika is a published library: adding, renaming, or removing a `pub` item is an
  API change. Call it out in the pull request so it lands in the changelog.
- Pull requests are squash-merged after CI is green.

## Community

Please report vulnerabilities through [`SECURITY.md`](./SECURITY.md). Project
participation is covered by [`CODE_OF_CONDUCT.md`](./CODE_OF_CONDUCT.md).
