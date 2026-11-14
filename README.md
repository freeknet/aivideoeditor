<div align="center">

# AI Video Editor

**Index footage, plan beat-synced edits, render and export timelines.**

[![CI](https://github.com/lnorton89/aivideoeditor/actions/workflows/ci.yml/badge.svg)](https://github.com/lnorton89/aivideoeditor/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![MSRV](https://img.shields.io/badge/rust-1.98%2B-orange.svg)](Cargo.toml)

</div>

AI Video Editor is a command-line tool. Describe here what it does and who
it is for, in two or three sentences, before the first command.

## Quick start

```bash
cargo run -- --help              # every subcommand and flag
cargo install --path .           # puts `aivideoeditor` on your PATH
```

The toolchain is pinned in `rust-toolchain.toml`, so `rustup` fetches the
right one automatically.

## What's inside

- **A real command line.** `clap` derive, `--help` and `--version` for free,
  and exit code 2 for arguments it refuses.
- **Logging** through `log` + `env_logger`: set `RUST_LOG=debug` to see more.
- **A forgiving config file.** Every field is optional and parsed
  independently, so one bad value costs that setting, not the file; writes
  are atomic (temp file, then rename).
- **Strict by default.** `unwrap`, `expect` and `panic!` are denied, every
  `match` names its variants, and a warning is a build failure.
- **Every source file says what it is.** A header banner on each `.rs` file,
  checked against the file's path by a test.

## Development

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo deny check                                         # licences, advisories, sources
```

All of these are clean on `main` and enforced by CI
(`.github/workflows/ci.yml`). A warning is a build failure there.
See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the rest.

## Documentation

- [`CONTRIBUTING.md`](CONTRIBUTING.md): how to build, test, and what the build enforces
- [`CLAUDE.md`](CLAUDE.md): the rules this codebase enforces, for people and agents
- [`CHANGELOG.md`](CHANGELOG.md): what changed, release by release
- [`SECURITY.md`](SECURITY.md): how to report a vulnerability privately

## Licence

MIT. See [`LICENSE`](LICENSE).

---

<sub>Scaffolded with [create-rust-app](https://github.com/lnorton89/create-rust-app).</sub>
