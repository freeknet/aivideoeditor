# AI Video Editor agent instructions

Read [`CLAUDE.md`](CLAUDE.md) first. It holds the commands, the rules the
build enforces, and the things that are deliberate. Preserve unrelated
dirty-worktree changes; never revert another contributor's edits.

## Unsafe Rust is a leaf boundary, without exceptions

- First try to remove the unsafe operation. Prefer safe standard-library
  APIs, checked slices, and RAII owners.
- Required unsafe code belongs only under `src/win/`.
  Everything else calls safe wrappers.
- Put each FFI call in its own small, named, private **safe** function. That
  leaf contains one unsafe operation and nothing else.
- Every unsafe operation needs an immediately relevant `// SAFETY:` proof.
- Do not expose `unsafe fn`. Never add unsafe to a test.
- Any resource with a matching close/free/destroy API gets one owning type
  whose `Drop` releases it exactly once.
- Never silence an unsafe lint. `Cargo.toml` and `src/unsafe_check.rs`
  enforce this policy.

Before handoff, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

`CLAUDE.md` and `CONTRIBUTING.md` contain the remaining project rules.
