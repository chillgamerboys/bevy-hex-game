---
name: test-local
description: Full local check loop — fmt + clippy + workspace tests + cargo-deny + doc build + markdown link check. Everything CI's quick and test jobs run, on the developer machine. Use before pushing for review; `/test-full` adds the ship-shape build on top.
---

When invoked, run this sequence. STOP on first failure.

## Step 1 — Format

```bash
cargo fmt --all --check
```

## Step 2 — Clippy

```bash
cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings
```

## Step 3 — Full workspace suite

```bash
cargo test --workspace --all-features --profile ci
```

Sum all `test result:` lines across the eleven workspace crates plus
doctests. The count drifts, and `/update-docs` owns the exact number
in CLAUDE.md.

## Step 4 — Dependency audit

```bash
cargo deny check
```

(Install once with `cargo install cargo-deny --locked` if missing.)

## Step 5 — Doc build

```bash
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features
```

## Step 6 — Markdown relative links

The exact loop CI runs (keep in sync with `.github/workflows/ci.yaml`):

```bash
set -u
broken=$(mktemp)
while IFS= read -r file; do
  dir=$(dirname "$file")
  grep -oE '\]\([^)#][^)]*\)' "$file" 2>/dev/null \
    | sed -E 's/^\]\(//; s/\)$//; s/#.*$//' \
    | grep -vE '^[a-z]+:' \
    | while IFS= read -r link; do
        [ -z "$link" ] && continue
        [ -e "$dir/$link" ] || echo "BROKEN: $file -> $link"
      done
done < <(git ls-files '*.md') > "$broken"
if [ -s "$broken" ]; then cat "$broken"; exit 1; fi
echo "all relative links resolve"
```

## Output

```
✓ /test-local — fmt, clippy, <N> tests, deny, doc, links all green (<elapsed>s)
```

## When to invoke

- **Before pushing** to remote for review.
- **In `/audit-diff`** as the verification step when running
  standalone.
- **In `/test-full`** as Phase 1 (local) — chained automatically.

## When NOT to invoke

- **As the merge gate.** `/audit-pr` calls `/test-full`, which adds
  the ship-shape build (no `--all-features` — what the CI matrix
  actually ships).
- **In the tight inner loop.** Prefer `/test-quick` while iterating;
  promote to `/test-local` when an iteration completes.

## Self-updating

- When CI's job list changes (`.github/workflows/ci.yaml`) → mirror
  the change here so local == CI stays true.
- When the suite grows past a comfortable local-runtime budget,
  consider splitting heavier suites out into `/test-full` only.
