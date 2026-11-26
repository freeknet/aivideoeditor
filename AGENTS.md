# AI Video Editor agent instructions

Read [`CLAUDE.md`](CLAUDE.md) first. It holds the commands, the rules the
build enforces (the unsafe boundary among them), and the things that are
deliberate. [`CONTRIBUTING.md`](CONTRIBUTING.md) has the rest.

Before you hand off, run the gates:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Preserve unrelated changes in a dirty worktree, and never revert another
contributor's edits.
