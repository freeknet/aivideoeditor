# AI Video Editor agent instructions

Read [`CLAUDE.md`](CLAUDE.md) first. It holds the commands, the rules the
build enforces, and the things that are deliberate. Preserve unrelated
dirty-worktree changes; never revert another contributor's edits.

Before handoff, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

`CLAUDE.md` and `CONTRIBUTING.md` contain the remaining project rules.
