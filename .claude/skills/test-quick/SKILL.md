---
name: test-quick
description: Fastest validation loop — fmt + strict lint + fail-closed concern-selected tests. Run during iteration and before commits. Skips deny, doc build, link check, and the ship-shape build. Use during iteration; `/test-local` is the next tier up.
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
python3 tools/test_scope.py run clippy
```

Expected: clean. Any error → STOP, surface verbatim. (Rust has no
separate typecheck step — clippy subsumes `cargo check` and runs the
repo's strict lint set: `#[allow]` banned, unwrap/panic/indexing
denied outside tests.)

## Step 3 — Tests

First inspect and record the fail-closed decision:

```bash
BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || \
  git rev-parse --abbrev-ref '@{upstream}' 2>/dev/null | sed 's#^[^/]*/##')
python3 tools/test_scope.py plan --base "origin/${BASE:-dev}" --head HEAD || exit $?
```

Then run the selected test concerns in canonical order:

```bash
BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || \
  git rev-parse --abbrev-ref '@{upstream}' 2>/dev/null | sed 's#^[^/]*/##')
SELECTED=$(python3 tools/test_scope.py selected-tests \
  --base "origin/${BASE:-dev}" --head HEAD) || exit $?
while [ -n "$SELECTED" ]; do
  concern=${SELECTED%% *}
  case "$SELECTED" in
    *" "*) SELECTED=${SELECTED#* } ;;
    *) SELECTED= ;;
  esac
  python3 tools/test_scope.py run "$concern" || exit $?
done
```

Expected: every selected concern passes. Report the selected concerns rather than a
workspace-wide test count:

```
✓ /test-quick — fmt clean, clippy clean, concerns <names> passed (<elapsed>s)
```

## When to invoke

- **During iteration**, while writing code. Exact Cargo package, target, and feature
  selection keeps pure changes out of the renderer graph. An unclassified shared or
  unknown path deliberately selects the full residual concern; classified shared
  contracts use their explicit producer/consumer closure.
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
