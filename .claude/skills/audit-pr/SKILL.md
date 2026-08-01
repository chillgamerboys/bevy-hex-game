---
name: audit-pr
description: THE merge gate. Chains `audit-linear` (soft Linear-tie check) → `audit-diff` (pre-test lens walk; 8 code lenses, or the 4 docs lenses on no-Rust diffs) → `test-full` (fmt/clippy/tests/deny/doc/links + ship build) → `visual-walk` (scripted capture walk, agent-read frames) → `audit-silent-failures` (explicit log) → `update-docs` (mechanical doc fix). Stop on first failure. Writes a schema-v3 receipt at `/tmp/audit-pr-receipt-{PR}.json` on green AND failed runs. Requires a PR. The canonical "I'm ready to merge" command.
---

When invoked, follow these steps. Stop on first failure within any
step. If a step blocks, surface the finding + skill name and do not
proceed.

**What this gate can and cannot see:** since `/visual-walk` (Step 2.5)
it CAN see the window — the game photographs itself along a scripted
walk and the agent reads every frame, so renders-nothing and
renders-broken failures (blue window, black sky, missing panel, dead
screen transition) are mechanical failures here, not surprises for a
human. What it still cannot judge is motion and taste: whether the
thing that renders correctly also looks *good*. That stays the
operator's, via the PR template's "a human ran the game and looked at
it" checkbox, and is why `main` moves only by promotion.

## Pre-flight: Require a PR

Run `gh pr view --json number,title 2>/dev/null`. If no PR, **STOP**
and tell the user to create one with `/create-pr` first. This skill
is the merge gate; running it without a PR is meaningless.

Report: "Audit for PR #N: [title]. Chain starts below."

## Step 0 — `audit-linear` (soft Linear-tie check, ~2s)

**Why first:** it's the cheapest check in the chain (one MCP call), so
it runs before anything slow.

**Soft gate — this repo's departure from the seed.** A PR tied to a
`HEX-N` ticket is traceable on the board, which is worth encouraging;
but plenty of work here is chore-shaped and doesn't earn a ticket.
A missing tie is therefore a **warning, never a block**.

**What it does:** invokes the `/audit-linear` skill, which:

- Parses the branch name and PR body for `HEX-\d+` references
- Resolves the matched ticket via Linear MCP (verifies it exists on
  the Hex Game team, surfaces title + state + URL)
- Warns if the ticket is already terminal (Done / Canceled /
  Duplicate)

**Decision:**
- ✓ tied to `HEX-N` → proceed to Step 1, status `pass`.
- ⚠ no tie found, unresolvable reference, or terminal-state ticket →
  proceed to Step 1 anyway, status `warn`. Mention `/update-linear`
  as the optional fix.
- A `warn` here **must not** flip `overall_status` off green.

<!-- Security scan (seed Step 1.5) was deliberately not ported: the seed's 37 rules are HTTP/web-shaped (auth, CSRF, SQL, webhooks) and this game has no network surface. If that changes, port `audit-security` from the seed and reinstate it here as Step 1.5. -->

## Step 1 — `audit-diff` (pre-test sanity, ~30s)

**Why first among the bulk steps:** every minute spent in audit-diff
saves the multi-minute test cycle that would otherwise surface the
same finding via test failure or a runtime regression. The 8-lens walk
catches contract-level findings (silent fallbacks, duplicated
constants, missing edge cases, compiles-but-wrong API use,
Commands/ordering contract breaks, test-altitude gaps, dead config
wiring) that a green test suite passes cleanly through.

**What it does:** walks the diff through audit-diff's lenses — the 8
code lenses, or the doc-only path (4 docs lenses, one subagent) when
the diff has no Rust. See `/audit-diff` SKILL.md for both catalogs.

**Decision:**
- ✓ `audit-diff` returns "clean across N files" → proceed to Step 2.
- ✗ Any SHIP-BLOCKER surfaces → **STOP**. Report the finding,
  file:line, and suggested action. Do not run tests; fix and re-invoke
  `/audit-pr`.

## Step 2 — `test-full`

**Why second:** tests are the slow gate. Run them only after
audit-diff is clean, to avoid burning the cycle on a diff that would
have failed review anyway.

**What it does:** invokes the `/test-full` skill — `/test-local`
(fmt, clippy, workspace tests, `cargo deny`, doc build, markdown link
check) then the ship-shape build (`cargo build --workspace --profile
ci`, no `--all-features`), then it prints the manual visual walk.
Doc-only diffs short-circuit to `/test-quick`.

See the `test-*` SKILL.mds for current expected counts (these grow as
the suite expands; embedding them here would drift).

**Decision:**
- ✓ `test-full` returns all-green → proceed to Step 3. Its Phase 3
  (visual walk) reports `manual — operator confirms`; that is not a
  failure, and the receipt does not claim the walk happened.
- ✗ Any step fails → **STOP**. Surface the failing suite/step and
  report back. Do not run Step 3 — a silent-failures audit on a
  partially-broken state conflates failures.

## Step 2.5 — `visual-walk` (the gate grows eyes)

**Why here:** it needs the binary Step 2 just built, and its verdict
should exist before the cheap textual steps wrap up.

**What it does:** invokes the `/visual-walk` skill — builds with the
`visual-walk` feature, drives the game through `walks/*.ron` (real
button wiring, injected input), captures a PNG per scripted step, and
the agent READS every frame. Two tiers: mechanical (stall, black
frame, wrong/missing screen) and review (layout, overflow, contrast).
Review findings block when UI or presentation is the changed surface and remain
advisory for other runtime changes.

**Decision:**
- ✓ all frames ok → status `pass`, proceed to Step 3.
- ⚠ review-tier findings in a non-UI runtime diff only → status `warn`, proceed.
  Findings ride the receipt into the merge report; the human judges them.
- ✗ any review-tier finding in a UI/presentation diff → **STOP**, status `fail`.
  Usability is part of the changed surface and cannot be downgraded to advisory.
- ✗ any mechanical failure → **STOP**, status `fail`. A game that
  cannot walk its own screens is not merge-ready.
- — no runtime surface in the diff → status `skipped`, proceed. Same
  trigger rule as audit-diff's visual flag.

## Step 3 — `audit-silent-failures` (explicit log, ~5s)

**Why explicit re-run:** `audit-diff` Step 1 already covered the
silent-failures lens, but a focused `/audit-silent-failures`
invocation here leaves an explicit record in the PR transcript.
Reviewers see "audit-silent-failures: 0 real candidates (N
FP-suppressed)" alongside the test results — clearer than
"audit-diff Lens 1 passed."

**What it does:** greps for the 7 Rust/Bevy anti-patterns on the diff
scope + FP-context output.

**Decision:**
- ✓ 0 real candidates → proceed to Step 4.
- ✗ Real findings → **STOP**. Real findings here mean Step 1 missed
  them — flag the regression to `audit-diff`'s Lens 1 logic. Do not
  run update-docs (its commit would land on top of unfinished work).

## Step 4 — `update-docs` (mechanical doc fix)

**Why last:** the test count should match what the suite just
reported. Step 2 ran it; this step propagates the number into the
Documentation Map anchors. Updating by hand drifts.

**What it does:** sums the `test result:` lines across the workspace,
compares to CLAUDE.md's anchored count, commits if drift. Atomic —
human-observed numbers (frame rate, entity counts) and historical
references are never touched.

**Decision:**
- ✓ No drift → done.
- ✓ Drift found + committed → **PUSH** the commit before declaring
  the PR ready for merge. The PR diff now includes the doc fix, and
  the receipt's `head_sha` must reflect the new HEAD (Step 5 reads it
  fresh).
- ✗ Test command failed (shouldn't happen if Step 2 was green — but
  defensive) → STOP, do not propagate broken counts.

## Step 5 — Write audit receipt (always, on green AND failed runs)

After steps 0-4 complete (regardless of pass/fail), write a JSON
receipt at `/tmp/audit-pr-receipt-{PR_NUM}.json` using the
**schema v3** format:

```json
{
  "schema_version": 3,
  "pr_number": 53,
  "head_sha": "8accfdf...",
  "branch": "feat/whatever",
  "base_branch": "dev",
  "completed_at": "2026-07-26T17:30:42Z",
  "overall_status": "green",
  "steps": {
    "0_audit_linear":          {"status": "pass", "summary": "HEX-5 (In Progress)", "findings": []},
    "1_audit_diff":            {"status": "pass", "summary": "clean (3 files, 8 lenses)", "findings": []},
    "2_test_full":             {"status": "pass", "summary": "local green, ship build green, visual walk manual", "findings": []},
    "3_audit_silent_failures": {"status": "pass", "summary": "0 candidates", "findings": []},
    "4_update_docs":           {"status": "pass", "summary": "no drift", "findings": []},
    "5_visual_walk":           {"status": "pass", "review_policy": "blocking", "summary": "11 frames across 2 walks, 0 mechanical, 0 review findings", "findings": []}
  },
  "environment": {
    "is_conductor_workspace": true,
    "workspace_path": "<pwd at audit time>"
  }
}
```

**Per-step `status` values:**

- `"pass"` — step completed cleanly (✓ in the report table).
- `"warn"` — step completed with non-blocking caveats (⚠ in the
  report table; e.g. audit-linear found no ticket tie, or matched a
  Done-state ticket). Counts as pass for `overall_status`.
- `"fail"` — step found blocking issues (✗ in the report table;
  audit-diff findings, failing tests, etc.).
- `"skipped"` — step did not apply to the diff or a prior step failed.

`5_visual_walk` additionally requires `review_policy`: `"blocking"` for a
UI/presentation diff, `"advisory"` for another runtime diff, or
`"not_applicable"` when skipped. A `warn` is valid only with `"advisory"`.

**`overall_status` values:**

- `"green"` — all required steps, including `5_visual_walk`, are `pass` or a
  policy-valid `warn`; a no-runtime visual walk may be `skipped`.
- `"failed"` — at least one required step, including `5_visual_walk`, is `fail`.
  Subsequent
  steps are typically `"skipped"`.

**Findings array shape:** each step writes step-specific finding
objects (see each leaf skill's "Findings shape" section). The merge
gate (`/merge-pr` Step 0) reads these to display exact findings
without re-running the audit:

- `1_audit_diff`: `{file, line, lens, message, severity}`
- `2_test_full`: `{suite, test, message}`
- `3_audit_silent_failures`: `{pattern, file, line, snippet, classification}`
- `4_update_docs`: `{file, before, after}`
- `5_visual_walk`: `{step, png_path, check, message}` — `check` is
  `mechanical` or `review`. The key is numbered 5 although the step
  runs as 2.5: keys `0`–`4` predate it and are a read contract with
  `/merge-pr`, so the new step takes the next free key rather than
  renumbering the chain.

`0_audit_linear` carries no findings array beyond `[]` — its result is
a one-line summary.

**Write rules (v3):**

- Write **regardless** of pass/fail outcome.
- Capture `head_sha` from `git rev-parse HEAD` (not from
  `gh pr view --json headRefOid` — the operator may have pushed
  after audit-pr started and we want the SHA the audit actually
  ran against; the latter would race). If Step 4 committed, read
  HEAD *after* that commit.
- Capture `base_branch` from `gh pr view --json baseRefName`
  (defense-in-depth for merge-pr's base-branch check; expected `dev`
  for feature work, `main` only for a `/promote` PR).
- ISO-8601 UTC timestamp. `/merge-pr` uses this to determine
  freshness — it treats receipts older than **60 minutes** as stale
  (warn-but-proceed if the SHA still matches). The 60-min threshold
  lives in `/merge-pr` Step 0 case 2; if it ever changes, update both
  sides together to keep the contract observable from either end.
- Overwrite if a receipt for this `pr_number` already exists
  (latest run wins).
- Skipped steps still get an entry with
  `{"status": "skipped", "summary": "skipped — prior step failed", "findings": []}`
  so the receipt is fully populated and merge-pr can report which
  step blocked.

**Failure handling:** if the write fails (e.g. a `/tmp` permission
anomaly), log loud but do NOT fail the audit. The gate's value is the
gate; the receipt is just the artifact for `/merge-pr` to parse. Loss
of the receipt means `/merge-pr` STOPs — "no receipt" is a hard block.

## Combined report

| Step | Result |
|---|---|
| 0 audit-linear | ✓ HEX-N (state) / ⚠ no tie / ⚠ HEX-N (terminal state) |
| 1 audit-diff | ✓ clean / ✗ N findings |
| 2 test-full | ✓ all green / ✗ failed at [step] |
| 2.5 visual-walk | ✓ N frames ok / ⚠ N non-UI review findings / ✗ UI review or mechanical failure / — skipped (no runtime surface) |
| 3 audit-silent-failures | ✓ 0 real / ✗ N findings |
| 4 update-docs | ✓ current / ✓ N files updated (committed `<sha>`) |
| receipt | ✓ wrote `/tmp/audit-pr-receipt-<N>.json` (overall_status: green / failed) |

If any of steps 1-4 (or either blocking tier in 2.5) is ✗, the PR is not
merge-ready. The skill stops
at the first failure — subsequent rows are reported as `— (skipped,
prior step failed)`. **Step 5 receipt is still written** (with
`overall_status: failed` + the failing step's `status: fail` +
subsequent steps `status: skipped`) so `/merge-pr` can read the
exact failure context.

A ⚠ on Step 0 never blocks — it is reported and the chain continues.

## When NOT to invoke

- **Single-line typo fixes** — over-engineered for the size.
- **Doc-only PRs are NOT a skip** — `/merge-pr` hard-gates on the
  receipt, so run the chain; it is cheap by construction: Step 1 takes
  audit-diff's doc-only path (4 docs lenses, one subagent), Step 2
  short-circuits to `/test-quick`'s doc-only skip.
- **Test-only diffs** — audit-diff's API and round-trip lenses don't
  apply; Lens 1 (silent failures) might still be worth running
  directly.

For these cases the leaf skills (`/audit-diff`,
`/audit-silent-failures`, `/update-docs`, `/test-full`) remain
individually invocable.

## Self-updating

This skill is pure orchestration. Behaviour changes go in the leaf
skills:

- New silent-failure anti-pattern → append to `audit-silent-failures`
- New lens → append to `audit-diff`
- New doc anchor to track → append to `update-docs`
- New test step → append to the relevant `test-*` leaf

If `audit-pr` itself develops an ordering issue (a step needs to
split, or a new step belongs — e.g. reinstating a security scan if the
game ever grows a network surface), update *this* file. The leaf
skills don't know about the chain shape. **Step keys are a contract
with `/merge-pr`**: renaming or renumbering one means updating
merge-pr's receipt reader in the same change.
