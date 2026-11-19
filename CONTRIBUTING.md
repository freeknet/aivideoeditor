# Contributing to AI Video Editor

Thanks for considering a contribution to AI Video Editor: Index footage, plan beat-synced edits, render and export timelines.

The [README](README.md) covers installation and use; this file is about
contributing.

## Reporting bugs and requesting features

Open an issue on the [issue tracker](https://github.com/lnorton89/aivideoeditor/issues).
Security vulnerabilities are *not* reported there; see
[`SECURITY.md`](SECURITY.md) for the private channel.

For a bug report, include your operating system, the version (or commit)
you ran, what you did, and what happened. The smallest reproduction you can
manage is worth more than a long description.

## Building and running

You need a Rust toolchain. The version is pinned in `rust-toolchain.toml`,
so `rustup` fetches the right one automatically.

```bash
cargo run                        # debug build
cargo run --profile release-fast # optimised, incremental: the one to iterate with
cargo run --release              # what a user gets: fat LTO, panic = "abort"
```

`--release` rebuilds slowly by design. Fat LTO over one codegen unit
re-optimises every dependency after any edit. `release-fast` is the same
optimisation level and `panic = "abort"` without the LTO, and rebuilds in
seconds. It is not the shipped binary, so build `--release` before
measuring performance.

`cargo check` is faster still when you only need to know whether the code
compiles. On Windows, most of a debug rebuild after a small edit is the
linker. Two things help there, and both are your call rather than the
repository's:

- **`rust-lld`**, which ships with the toolchain: set
  `CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER=rust-lld.exe` in your
  environment, or the same key in your user-level `~/.cargo/config.toml`.
  It is not set in the repository because CI and the release build use
  `link.exe`, and a linker change there would be a change to what ships.
- **Keep `target/` out of real-time antivirus scanning**: a Dev Drive, or a
  Defender exclusion for the checkout.

## Before you submit

Three commands, all clean:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

CI runs all three on Windows and on Linux, plus `cargo deny check`. A
warning is a build failure there.

### What a Linux run proves

The app ships for Windows, and the platform layer under `src/win/` only
compiles there. Off Windows, type-check it without linking:

```bash
rustup target add x86_64-pc-windows-msvc
cargo clippy --all-targets --all-features --target x86_64-pc-windows-msvc -- -D warnings
```

That catches a broken `extern` block or an egui API that moved. It cannot
run a test, and it cannot show you the window: **anything visible still
needs a Windows run, and a visible change wants a screenshot in the pull
request.**

## Rules the codebase enforces

These are enforced by lints and tests, so a violation fails the build
rather than getting caught in review.
[`CLAUDE.md`](CLAUDE.md) has the full list with the reasoning; the short
version:

- **No `unwrap`, `expect`, `panic!`, `todo!` or `unreachable!`, including
  in tests.** Use `let ... else`, `?`, or an explicit fallback. Tests return
  `anyhow::Result<()>`, and every `assert!` carries a message.
- **`match` arms list every variant** (`wildcard_enum_match_arm` is denied),
  and suppressions are `#[expect(lint, reason = "...")]`, never `#[allow]`,
  so they cannot outlive what they suppressed.
- **Every public item is documented** (`missing_docs` is denied), and
  `unreachable_pub` is forbidden: what is `pub` is reachable from outside.
- **Every source file opens with a module header banner**: a ruled block
  naming `Module:`, `Description:` and `Dependencies:`, then the `//!` docs.
  `src/header_check.rs` walks `src/`, `tests/` and `examples/` and fails the
  build for a file missing one, checking `Module:` against the path the file
  actually sits at.
- **Every `unsafe` block is a safe leaf wrapper.** One named, private, safe
  function per FFI call; the `unsafe` block contains the call and nothing
  else; a `// SAFETY:` comment stating the reasoning; an owning `Drop` type
  wherever there is a matching close/free/destroy. Required unsafe lives
  only under `src/win/`, and `src/unsafe_check.rs` enforces it.
- **One toolchain.** `rust-toolchain.toml` pins it, every workflow's
  `toolchain:` line names the same version, and `tests/toolchain_pin.rs`
  fails the build if they drift.
- **Every dependency passes `cargo deny check`**: an allowed licence, no
  known advisory, crates.io as the only source.
- **Constants live with what they govern**, and there is deliberately no
  `constants.rs`.

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).
Add a line under `## [Unreleased]` in the same pull request as the change,
in the section its commit prefix implies: `feat:` under Added, `fix:` under
Fixed, and so on.

## Cutting a release

1. On a release branch off `main`: bump `version` in `Cargo.toml`, move the
   `Unreleased` entries under a new `## [x.y.z] - YYYY-MM-DD` heading in
   `CHANGELOG.md`, and commit.
2. Open the pull request and let the checks pass.
3. Merge (a merge commit, so the history the changelog describes survives),
   then tag the merge commit, annotated:
   `git tag -a v0.2.0 -m "aivideoeditor v0.2.0"`, and push the tag.
4. The push triggers `.github/workflows/release.yml`: it checks the tag
   against `Cargo.toml`, builds the release archive with an SBOM and
   checksums, attests them, and publishes the GitHub release.

**Never amend or rebase a commit after tagging it.** The tag keeps pointing
at the copy you discarded, which then sits on no branch at all.

## Dependency updates

Dependabot opens version-update PRs weekly for both crates and GitHub
Actions (`.github/dependabot.yml`). CI runs against every one, so the check
marks are the first thing to read.

One thing about that config is deliberate: **only patch bumps are
grouped.** Dependabot classifies an update by its literal version segments,
so `0.36 -> 0.37` reaches it as a *minor*, while under cargo that is
exactly the breaking one. Anything above patch therefore arrives as its own
PR by design. If you are batching upgrades by hand, batch them the same way.

`rust-toolchain.toml` and the `dtolnay/rust-toolchain` pin in the workflows
move by hand, together, in their own commit.

## Commit hygiene

- Keep each commit focused on one change, with a message that says *why*.
  Prefixes: `feat:`, `fix:`, `perf:`, `refactor:`, `docs:`, `test:`,
  `build:`, `ci:`.
- Stage only the files you actually changed, by name, not `git add -A`.
  `git commit` commits the whole index, so use `git commit -- path/to/file`
  when the tree holds work that is not yours.
- Line endings are handled by `.gitattributes`: every text file is stored
  and checked out as LF, including on Windows. Don't convert endings by
  hand.

## Opening a pull request

Against `main`, with a title that summarises the change and a body that
says what and why. The template asks what you ran and on what; please
answer it honestly.
UI changes want a screenshot.

Small, focused PRs are easier to review and land faster than large ones.
