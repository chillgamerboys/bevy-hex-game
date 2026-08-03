---
name: test-full
description: Pre-merge gate — chains `/test-local` (fmt/clippy/selected tests/deny/doc/links) → ship-shape build when selected → scoped visual applicability. Used as Step 2 of `/audit-pr`. Doc-only diffs short-circuit to `/test-quick`. Stop on first failure.
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

Run this phase when the scope decision selects `shipping`:

```bash
cargo build --package hex_game --release
```

No `--all-features`, and only the shipping package: this is exactly what CI's
three-platform matrix builds and what ships — workspace-only binaries, `hex_dev`,
and the `dev` feature are excluded. It
catches "builds with the inspector but not without", which Phase 1
cannot see because every other command runs `--all-features`.

If `shipping` is omitted, record `ship build: not applicable (scope decision)` rather
than compiling it anyway.

## Phase 3 — Visual verification (two tiers)

**The automated tier is `/visual-walk`** — as `/audit-pr` Step 2.5 it
drives the game through `walks/*.ron`, photographs every screen, and
the agent reads the frames. It judges static camera framing/occlusion, UI
presentation, and rendered-map geometry/materials/lighting/cutaways/seams/composition.
Blue windows, black skies, dead rendered surfaces, and missing panels are caught there
mechanically. Typed hooks, not images, own all gameplay/world logic. When running
`/test-full` standalone on a presentation-surface change, invoke
`/visual-walk` here rather than skipping straight to the reminder.

**The human tier must not be skipped silently when it applies.** Stills are not play:
video or a human check owns camera/movement/animation motion, native-input response,
control feel, and taste. Human observation is presentation/experience evidence only
and must never prove or corroborate logic available through hooks or contracts. Print
the scoped walk for the operator:

```
Manual presentation/experience walk (record exact-head result, commit, reviewer, date, and route in the PR):
  cargo dev
  splash → title → New Game → Party Trial
  orbit (right-drag), WASD pan, zoom
  select and move the party; judge motion, control feel, range tint, and route rendering
  inspect only affected presentation routes; cite hooks separately for state/persistence
```

Report this phase as `automated presentation walk: <verdict>; human presentation walk:
operator records structured evidence`, or as an exact-head verified-maintainer N/A
naming the renderer-free hook closure. The PR fields belong to the operator, not this
skill.

If the diff changes no rendered presentation, native-input experience, motion, control
feel, seams, taste, or visual script, record both visual tiers as `not applicable` with
that reason even when renderer-free runtime logic changed. This applicability decision
is independent of a fail-closed selector choosing `app`: automated logic closure and
visual evidence answer different questions. Record an exact-head verified-maintainer
waiver naming the authoritative hooks/contracts. Do not launch the UI merely to turn
an unrelated logic test into a checkbox. Combined waves use the same classification;
release promotions retain their broader human presentation gate.

## Output

```
✓ /test-full — local green, ship build green, visual applicability: <human presentation evidence | exact-head hook-backed N/A reason>
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

Automatic as Step 2. The receipt's `2_test_full` step records the outcome with per-suite
findings. Applicable human presentation review stays the operator's to record in the
structured PR fields; logic-only changes instead name the exact hook-backed N/A. The
receipt does not invent either result.

## When invoked standalone

Useful when the operator wants to validate locally before opening
a PR, or to reproduce a CI failure.

## Self-updating

- When a new test tier earns its place (e.g., a presentation-only rendered-frame
  comparison if screenshot testing ever lands) → add a Phase 4 here.
- When CI's build matrix changes → update Phase 2 to match.
