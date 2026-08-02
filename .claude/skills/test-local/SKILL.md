---
name: test-local
description: PR-local check loop — fmt + clippy + concern-selected tests + cargo-deny + doc build + markdown link check. Mirrors the affected CI closure on the developer machine. Use before pushing for review; `/test-full` adds the selected ship-shape and visual applicability gates.
---

When invoked, run this sequence. STOP on first failure.

## Step 1 — Format

```bash
cargo fmt --all --check
```

## Step 2 — Clippy

```bash
python3 tools/test_scope.py run clippy
```

## Step 3 — Selected concern closure

```bash
BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || \
  git rev-parse --abbrev-ref '@{upstream}' 2>/dev/null | sed 's#^[^/]*/##')
python3 tools/test_scope.py plan --base "origin/${BASE:-dev}" --head HEAD || exit $?
SELECTED=$(python3 tools/test_scope.py selected-tests \
  --base "origin/${BASE:-dev}" --head HEAD) || exit $?
REMAINING=$SELECTED
while [ -n "$REMAINING" ]; do
  concern=${REMAINING%% *}
  case "$REMAINING" in
    *" "*) REMAINING=${REMAINING#* } ;;
    *) REMAINING= ;;
  esac
  python3 tools/test_scope.py run "$concern" || exit $?
done
case " $SELECTED " in
  *" residual "*) cargo test --workspace --all-features --profile ci --doc ;;
esac
```

Report the decision and each selected concern independently. Do not manually promote
an omitted application/UI, simulation, generation, or residual concern merely because
it exists. Unknown/unclassified paths and empty diffs already fail closed; pushes to
`dev`/`main` and final wave or release candidates force the complete gate.

## Step 4 — Dependency audit

```bash
cargo deny check
```

(Install once with `cargo install cargo-deny --locked` if missing.)

## Step 5 — Doc build

```bash
python3 tools/test_scope.py run docs
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
✓ /test-local — fmt, clippy, concerns <names>, deny, doc, links green (<elapsed>s)
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
