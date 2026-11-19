## Summary

What this change does and why. One or two sentences: the *why* matters more
than the *what*. If this fixes an issue, reference it here (e.g. `Closes #12`).

## Test plan

What you ran, and on what. The checks below must be clean before this is
mergeable; a warning is a build failure in CI.

Say whether you ran it **on Windows**, the platform the app ships for. A
change to anything drawn wants a screenshot.

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --all-features` passes
- [ ] No new `unwrap` / `expect` / `panic!`, not even in tests
- [ ] New `.rs` files carry the module header banner (`Module:` / `Description:` /
      `Dependencies:`) and the matching `mod` declaration
- [ ] Every new `unsafe` block is inside a safe leaf wrapper under `src/win/`,
      contains only the one call, and carries a `// SAFETY:` comment
- [ ] UI changes include a screenshot
- [ ] Commit messages use the `feat:` / `fix:` / `perf:` / `refactor:` / `docs:` /
      `test:` / `build:` / `ci:` prefixes

Full guidelines, including how to build and what CI enforces, are in
[`CONTRIBUTING.md`](https://github.com/lnorton89/aivideoeditor/blob/main/CONTRIBUTING.md).
