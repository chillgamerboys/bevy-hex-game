---
name: test-local
description: PR-local check loop — exact PR-context planning, selector-chosen tests and non-test gates, plus Markdown links. Mirrors only the affected CI closure on the developer machine. Use before pushing for review; `/test-full` adds the selected ship-shape and visual applicability gates.
---

When invoked, run this sequence. STOP on first failure.

## Step 1 — Exact PR-context scope

```bash
BASE=$(gh pr view --json baseRefName --jq .baseRefName 2>/dev/null || printf dev)
HEAD_REF=$(gh pr view --json headRefName --jq .headRefName 2>/dev/null || \
  git branch --show-current)
PR_NUMBER=$(gh pr view --json number --jq .number 2>/dev/null || true)
SCOPE_ARGS=(--base "origin/$BASE" --head HEAD)
if [ -n "$PR_NUMBER" ]; then
  SCOPE_ARGS+=(--event-name pull_request --base-ref "$BASE" \
    --head-ref "$HEAD_REF" --pull-request-number "$PR_NUMBER")
fi
python3 tools/test_scope.py plan "${SCOPE_ARGS[@]}" || exit $?
```

The printed booleans are authoritative for every later phase. An exact tracked waiver
applies only when this context matches; an absent or mismatched context fails closed.
Keep `SCOPE_ARGS` in the same shell through Step 3; repeat the Step 1 setup if the
command runner starts a fresh shell per block.

## Step 2 — Format, dependency policy, and Clippy

For `code: true`, run:

```bash
cargo fmt --all --check
cargo deny check
```

When the decision selects `clippy`, additionally run:

```bash
python3 tools/test_scope.py run clippy
```

Do not run Clippy merely because another test concern exists.

## Step 3 — Selected concern closure

```bash
SELECTED=$(python3 tools/test_scope.py selected-tests \
  "${SCOPE_ARGS[@]}") || exit $?
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
it exists. Unknown/unclassified paths and empty diffs already fail closed. Protected
pushes and final candidates ordinarily promote to the complete gate, while an exact
tracked waiver retains only its authorized narrow closure.

## Step 4 — Selected doc build

Run only when the decision selects `docs`:

```bash
python3 tools/test_scope.py run docs
```

## Step 5 — Markdown relative links

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
✓ /test-local — selected gates <names>, links green (<elapsed>s)
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
