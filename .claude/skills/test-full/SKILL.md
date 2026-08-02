---
name: test-full
description: Pre-merge gate — chains `/test-local` (fmt/clippy/tests/deny/doc/links) → ship-shape build (default features, what CI's matrix builds) → manual visual-verification reminder. Used as Step 2 of `/audit-pr`. Doc-only diffs short-circuit to `/test-quick`. Stop on first failure.
---

When invoked, run this sequence. STOP on first failure.

## Pre-flight — Doc-only short-circuit

If the diff is doc-only:

```bash
BASE=$(gh pr view --json baseRefName -q .baseRefName 2>/dev/null || echo dev)
CHANGED=$(git diff --name-only "origin/${BASE}...HEAD")
if printf '%s\n' "$CHANGED" | grep -vqE '\.md$|^docs/|^README|^CHANGELOG|^\.claude/'; then
    # Has non-doc changes; proceed.
    :
else
    # Doc-only — delegate to /test-quick.
    /test-quick
    exit $?
fi
```

(`printf '%s\n'` rather than `echo` — zsh's builtin `echo` interprets
backslash escapes, which mangles any path containing one and breaks
JSON re-fed into `jq`. Same reason `/audit-linear` avoids it.)

This keeps `/audit-pr` fast on doc-fixup PRs without skipping the
link-check baseline entirely (CI still runs it; so does `/test-local`).

## Phase 1 — Local (`/test-local`)

```
/test-local
```

If `/test-local` fails → STOP. Surface the failing step + report.

## Phase 2 — Ship-shape build

```bash
cargo build --package hex_game --release
```

No `--all-features`, and only the shipping package: this is exactly what CI's
three-platform matrix builds and what ships — workspace-only binaries, `hex_dev`,
and the `dev` feature are excluded. It
catches "builds with the inspector but not without", which Phase 1
cannot see because every other command runs `--all-features`.

## Phase 3 — Visual verification (two tiers)

**The automated tier is `/visual-walk`** — as `/audit-pr` Step 2.5 it
drives the game through `walks/*.ron`, photographs every screen, and
the agent reads the frames. Blue windows, black skies, dead screens
and missing panels are caught there mechanically. When running
`/test-full` standalone on a runtime-surface change, invoke
`/visual-walk` here rather than skipping straight to the reminder.

**The human tier must not be skipped silently.** Stills are not play:
motion, feel, hairline seams and taste are still found by a person
looking at the window — as every serious bug here was before the walk
existed. Print the walk for the operator:

```
Manual walk (PR checkbox: "A human ran the game and looked at it"):
  cargo dev
  splash → title → New Game → Party Trial
  orbit (right-drag), WASD pan, zoom
  select and move the party; range tint + route preview draw
  ESC pause, F5 save, BACKSPACE to title, Continue
  Settings change → restart → verify; launch affected fixtures separately
```

Report this phase as `automated walk: <verdict>; human walk: operator
confirms`. The human box in the PR template belongs to the operator,
not this skill.

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
