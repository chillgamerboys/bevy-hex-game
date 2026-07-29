---
name: test-quick
description: Fastest validation loop — fmt + clippy + the workspace test suite. Fast when warm (incremental after the first build); run during iteration and before commits. Skips deny, doc build, link check, and the ship-shape build. Use during iteration; `/test-local` is the next tier up.
---

When invoked, run this sequence. STOP on first failure — the
downstream tiers are meaningless if quick is red.

## Step 1 — Format

```bash
cargo fmt --all --check
```

Expected: clean. Any error → STOP, surface verbatim.

## Step 2 — Clippy

```bash
cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings
```

Expected: clean. Any error → STOP, surface verbatim. (Rust has no
separate typecheck step — clippy subsumes `cargo check` and runs the
repo's strict lint set: `#[allow]` banned, unwrap/panic/indexing
denied outside tests.)

## Step 3 — Tests

```bash
cargo test --workspace --all-features --profile ci
```

Expected: all pass across all twelve crates plus doctests. Sum the
`test result:` lines and report:

```
✓ /test-quick — fmt clean, clippy clean, <N> tests passed (<elapsed>s)
```

## When to invoke

- **During iteration**, while writing code. Incremental compilation
  makes this cheap once the workspace is warm; the first cold run
  compiles Bevy and takes far longer — that is expected, not a hang.
- **Before commits**, as a pre-push sanity gate.
- **In `/audit-diff`** as the verification step when running
  standalone (not from `/audit-pr` — that uses `/test-full`).

## When NOT to invoke

- **As the merge gate.** `/audit-pr` calls `/test-full`, which adds
  deny, the doc build, the link check, and the ship-shape build.
- **For PRs touching `.github/workflows/`, `Cargo.toml`, `deny.toml`,
  or `rust-toolchain.toml`.** Use `/test-full` — those files change
  what CI itself runs, and quick's subset can be green while CI is not.

## Doc-only short-circuit

If the diff is doc-only (changes restricted to `**/*.md`, `docs/`,
`README*`, `CHANGELOG*`, `.claude/`), report:

```
✓ /test-quick — doc-only diff; skipping fmt/clippy/tests.
```

and exit success. This keeps `/audit-pr` Step 2 fast on doc PRs
that short-circuit to test-quick.

## Self-updating

If a new fast-tier check earns its place, add it as Step 0 ahead of
format. Keep the tier meaningfully faster than `/test-local` — that's
the contract.
