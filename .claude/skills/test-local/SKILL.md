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
python3 tools/gameplay_scope.py run clippy
```

## Step 3 — Full workspace suite

```bash
python3 tools/gameplay_scope.py run rules
python3 tools/gameplay_scope.py run contracts
python3 tools/gameplay_scope.py run simulation
python3 tools/gameplay_scope.py run app
python3 tools/gameplay_scope.py run residual
cargo nextest run --workspace --all-features --cargo-profile ci --profile ci -E 'package(hex_map)'
cargo test --workspace --all-features --profile ci --doc
```

Report each concern independently. The explicit map command is the unchanged
world-owned shard enabled beside the residual concern in CI. `/test-local` is
deliberately broad; `/test-quick` uses the scope selector for the edit loop.

## Step 4 — Dependency audit

```bash
cargo deny check
```

(Install once with `cargo install cargo-deny --locked` if missing.)

## Step 5 — Doc build

```bash
python3 tools/gameplay_scope.py run docs
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
✓ /test-local — fmt, clippy, all test concerns, deny, doc, links green (<elapsed>s)
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
