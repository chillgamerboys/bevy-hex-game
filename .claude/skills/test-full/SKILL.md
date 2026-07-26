---
name: test-full
description: Pre-merge gate — chains `/test-local` (fmt/clippy/tests/deny/doc/links) → ship-shape build (default features, what CI's matrix builds) → manual visual-verification reminder. Used as Step 2 of `/audit-pr`. Doc-only diffs short-circuit to `/test-quick`. Stop on first failure.
---

When invoked, run this sequence. STOP on first failure.

## Pre-flight — Doc-only short-circuit

If the diff is doc-only:

```bash
CHANGED=$(git diff --name-only origin/dev...HEAD)
if echo "$CHANGED" | grep -vqE '\.md$|^docs/|^README|^CHANGELOG|^\.claude/'; then
    # Has non-doc changes; proceed.
    :
else
    # Doc-only — delegate to /test-quick.
    /test-quick
    exit $?
fi
```

This keeps `/audit-pr` fast on doc-fixup PRs without skipping the
link-check baseline entirely (CI still runs it; so does `/test-local`).

## Phase 1 — Local (`/test-local`)

```
/test-local
```

If `/test-local` fails → STOP. Surface the failing step + report.

## Phase 2 — Ship-shape build

```bash
cargo build --workspace --profile ci
```

No `--all-features`: this is exactly what CI's three-platform matrix
builds and what ships — `hex_dev` and the `dev` feature excluded. It
catches "builds with the inspector but not without", which Phase 1
cannot see because every other command runs `--all-features`.

## Phase 3 — Visual verification (manual)

**This phase cannot be automated and must not be skipped silently.**
Several failure modes in this repo produce a clean log, green tests,
and a wrong window — a black sky, a plain blue screen, a piece sunk
into the terrain. Every serious bug so far was found by a person
looking at the window.

Print the walk for the operator:

```
Manual walk (PR checkbox: "I ran the game and looked at it"):
  cargo dev
  splash → title → click a scenario → gameplay
  orbit (right-drag), WASD pan, zoom
  click a tile — piece walks there; range tint + route preview draw
  ESC pause/resume, BACKSPACE to title, re-enter a scenario (rebuild)
```

Report this phase as `manual — operator confirms`. It maps to the
"I ran the game and looked at it" checkbox in the PR template; the
operator ticks that box, not this skill.

## Output

```
✓ /test-full — local green, ship build green, visual walk: manual — operator confirms
```

## Findings shape (for audit-pr receipt v3)

When invoked from `/audit-pr`, return findings as a list shaped:

```json
{
  "suite": "hex_units",
  "test": "reach::two_level_body_refused_crawlspace",
  "message": "assertion failed: expected surface to be unreachable"
}
```

`suite` is the crate name (`hex_core`, `hex_map`, `hex_units`, …) or
`build`/`deny`/`doc`/`links` for the non-test steps. `/audit-pr`
Step 2 propagates these into the receipt's `2_test_full.findings`
array. `/merge-pr` prints them verbatim in the STOP message so the
operator sees the exact failures without re-running.

## When invoked from `/audit-pr`

Automatic as Step 2. The receipt's `2_test_full` step records the
outcome with per-suite findings. The visual walk stays the operator's
to confirm via the PR checkbox — the receipt does not claim it.

## When invoked standalone

Useful when the operator wants to validate locally before opening
a PR, or to reproduce a CI failure.

## Self-updating

- When a new test tier earns its place (e.g., a rendered-frame
  comparison if screenshot testing ever lands) → add a Phase 4 here.
- When CI's build matrix changes → update Phase 2 to match.
