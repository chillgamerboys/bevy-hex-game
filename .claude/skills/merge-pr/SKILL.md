---
name: merge-pr
description: Finalize a PR after `/audit-pr` is green — strict schema-v3 receipt check with structured findings (no warn-but-proceed paths), cheap pre-flights (base branch, mergeable, worktree-free, branch pushed), then `gh pr merge --merge` (merge commits, never squash) + optional Linear state-sync to Done + `git fetch origin --prune`. Four merge classes by base/head: feature→dev and ticket→wave delete the head branch (into-wave merges skip the Done sync — tickets wait for the wave); a wave→dev landing requires the ticked human-walk box and batch-syncs only complete epics; a dev→main promotion deletes nothing. The receipt is a hard merge contract; a `failed` overall_status STOPs the merge with the failing step names + exact findings.
---

When invoked, follow these steps. STOP on any pre-flight failure
unless `--force "<reason>"` is passed. The merge action itself is
irreversible — once `gh pr merge` succeeds, all subsequent failures
are warn-don't-fail.

**Merge policy for this repo:** merge commits, **never squash**
(`gh pr merge N --merge`), so per-PR history is preserved. Feature
branches are deleted once merged; **`dev` is never deleted**.

## Step 0 — Audit-receipt strict check (hard gate)

Look for `/tmp/audit-pr-receipt-{PR_NUM}.json`. The file is written
by `/audit-pr` after every run (green AND failed). This is the
**hard gate** for merge eligibility — there is no warn-but-proceed
path. Six cases (routed on `schema_version`):

For every schema-v3 receipt, validate `5_visual_walk` before routing on SHA,
status, or age. The entry must exist, `review_policy` must be `blocking`,
`advisory`, or `not_applicable`, and the status/policy pair must be coherent:
`pass` pairs with `blocking` or `advisory`; `warn` pairs only with `advisory`;
`skipped` pairs only with `not_applicable`; and `fail` pairs with `blocking` or
`advisory` and requires `overall_status: "failed"`. Any `fail` inside an
otherwise-green receipt is also invalid. A missing field or invalid pair means
the receipt cannot prove the gate that produced it: **STOP** and rerun
`/audit-pr`. This validation applies equally to fresh and older receipts.

1. **Receipt present, `schema_version=3`, `head_sha` matches `git
   rev-parse HEAD`, `completed_at` within last 60 min,
   `overall_status: "green"`** → ✓ silent. Audit-pr was green
   for *this exact commit* recently. Proceed with full confidence.
   (A `warn` on step `0_audit_linear` — no ticket tie — is part of
   green here; this repo's Linear tie is soft. Echo it in the report,
   don't block on it. A valid advisory warning on `5_visual_walk` is NOT
   silent: print its review-tier findings `{step,
   png_path, check, message}` in the report so the operator sees what the
   agent's eyes flagged before finalizing. A `fail` on
   `5_visual_walk` flips `overall_status` to failed like any other
   step; `skipped` means the diff had no runtime surface.)

2. **Receipt present, SHA and green overall status match, but `completed_at`
   > 60 min old**
   → ⚠ warn (don't STOP):

   ```
   ⚠ audit-pr receipt is <N> minutes old. The audit was green
     (overall_status: green) for HEAD, but you stepped away. If
     the environment has changed (dependency updates, a toolchain
     bump, MCP outages), consider re-running /audit-pr before
     merging.
   ```

3. **Receipt present, SHA matches, `overall_status: "failed"` (regardless of
   receipt age)** →
   ✗ STOP. **Print the failing step's `findings` array verbatim** —
   v3 captures structured findings so the operator sees the exact
   bugs without re-running the audit:

   ```
   ✗ audit-pr completed for this HEAD but found blocking issues.

     overall_status: failed
     Failing steps:
       2 test_full: hex_units 2 failures
         - suite=hex_units test=reach::crawlspace_refused → assertion failed
         - suite=hex_units test=route::no_route_is_none   → panicked at index
       3 audit_silent_failures: skipped — prior step failed
       4 update_docs: skipped — prior step failed

     Fix the failing step(s) and re-run /audit-pr. If you believe
     the failures are pre-existing flakes, confirm they are
     unrelated to this PR's diff before forcing — see /merge-pr
     --force semantics.
   ```

   **No silent override.** `--force` with reason can override
   SHA-mismatch (case 5), but NOT a failed audit-pr — the contract
   is "audit-pr said no, merge says no." If a failing step is
   genuinely a flake, the path is: re-run /audit-pr and get a green
   receipt, then merge cleanly.

4. **Receipt present but `schema_version < 3`** → ✗ STOP:

   ```
   ✗ audit-pr receipt uses schema v<N> (pre-strict).
     Re-run /audit-pr to write a v3 receipt with structured
     per-step status before merging.
   ```

   This repo has only ever written v3 — a v1/v2 receipt means a
   stale file from another tool. Re-run rather than interpret it.

5. **Receipt present, SHA does NOT match HEAD** → ✗ STOP
   unless `--force "<reason>"`:

   ```
   ✗ audit-pr receipt is for SHA <old> but HEAD is now <new>.
     New commits since the audit — the gate did not see them.
     Re-run /audit-pr from the top before merging. To override
     (e.g., the new commits are doc-only fixup), pass:
     /merge-pr --force "doc-only fixup post-audit"
   ```

   SHA-mismatch with `--force` is the only override that survives
   strict mode: the operator commits after a green audit and wants to
   ship without burning another full cycle. They take responsibility
   via the `--force "<reason>"` audit-trail comment.

6. **No receipt at all** → ✗ STOP (HARD — no warn-but-proceed):

   ```
   ✗ No /tmp/audit-pr-receipt-<N>.json found.
     /audit-pr has not run on this branch (or its receipt was
     cleared — /tmp doesn't survive reboot). Run /audit-pr
     before /merge-pr — the receipt is the merge contract.
   ```

`--force "<reason>"` requires a non-empty reason string and applies
**only to case 5 (SHA mismatch)**. It does NOT override cases 3
(failed audit), 4 (old schema), or 6 (no receipt) — those are gate
violations, not gate-age violations.

## Step 1 — Cheap pre-flights (run regardless of receipt state)

State can change between audit and merge — these checks are fast
(<5s combined) and defend against the obvious last-mile failures.

1. **PR exists.** Run
   `gh pr view --json number,title,state,mergeable,mergeStateStatus,baseRefName,headRefName,headRefOid,body`.
   - If no PR → STOP "no PR open; run `/create-pr` first."
   - If `state == "MERGED"` → short-circuit to step 4 (idempotent
     path: merge is done, just run the state-sync if Linear still
     says In Review).
   - If `state == "CLOSED"` → STOP "PR is closed (not merged).
     Reopen with `gh pr reopen` or open a new PR."

2. **Base branch → classify the merge.** Read `baseRefName` and
   `headRefName` from the same JSON. **First matching case wins — the
   order below is load-bearing** (a wave→dev landing also matches the
   plain feature case; it must be classified as a landing):

   - `baseRefName == "dev"` AND `headRefName` matches `wave/*` →
     **wave landing** (the one walked merge): `--merge
     --delete-branch` (the wave branch dies here — never `dev`), and
     Step 4 becomes the **batch** state-sync: Done for every wave
     ticket whose epic is COMPLETE; partially-delivered epics stay In
     Review (wave 1's HEX-6 precedent). Pre-flight extra: the PR's
     human-walk checkbox must be ticked — a wave landing without the
     walk is exactly what the model forbids.
   - `baseRefName == "dev"` → **feature merge** (the normal path):
     `--merge --delete-branch` (Step 2) + single-ticket state-sync to
     Done (Step 4).
   - `baseRefName` matches `wave/*` → **into-wave merge** (a ticket
     PR joining its wave): `--merge --delete-branch`, and **SKIP**
     the state-sync — the ticket stays In Review until the wave
     lands. **Watch Linear's GitHub integration**: it auto-closes a
     ticket the moment its linked PR merges; after an into-wave merge,
     verify the ticket state and revert an auto-close by hand.
     Pre-flight extra: `gh pr list --base <headRefName>` must be
     empty before `--delete-branch` — GitHub CLOSES (not retargets)
     any open PR whose base branch is API-deleted, and a closed PR
     cannot be retargeted or reopened onto a dead base; the recovery
     is a successor PR.
   - `baseRefName == "main"` AND `headRefName == "dev"` →
     **promotion merge** (opened by `/promote`): `--merge` with **no**
     `--delete-branch`, and **SKIP** the state-sync (Step 4). Tickets
     already reached Done when their waves landed on `dev`.
   - Otherwise → STOP:

   ```
   ✗ PR #<N> targets `<baseRefName>` from `<headRefName>`.
     Everything lands on `dev` (directly or through a wave);
     `main` moves only by promoting `dev`.

     Fix: retarget via `gh pr edit <N> --base <dev-or-wave>`, then
     re-invoke /merge-pr. For the deliberate dev→main hop, use
     /promote.
   ```

   Cheapest check in the chain (already in the JSON step 1 fetched).
   It prevents the failure mode where GitHub picks a parent feature
   branch as the default base.

3. **Worktree conflict pre-check (Conductor workspaces).** This repo
   is developed in Conductor worktrees, so run:

   ```bash
   git worktree list
   ```

   If `dev` is held by a different worktree path, surface a warning:

   ```
   ⚠ `dev` is held by another worktree at <path>.
     `gh pr merge --delete-branch` will fail at the post-merge
     local-checkout step. The merge itself will succeed on GitHub,
     and step 5 below recovers by deleting the remote branch via
     the gh API. No action needed; this is informational.
   ```

   Detection-and-recovery beats a surprise mid-flight failure.

4. **Mergeable per GitHub.** Same JSON:
   - `mergeable: MERGEABLE` required; `CONFLICTING` → STOP "resolve
     conflicts via `git pull origin dev --rebase`."
   - `mergeStateStatus: CLEAN` is the happy path. `UNSTABLE` (checks
     pending) → ⚠ warn but proceed. `BLOCKED` → STOP "branch
     protection blocks merge (failing checks / required reviews)."

5. **Local branch fully pushed.** `git status -sb` — if local has
   `[ahead N]` → STOP "local has <N> unpushed commits. Push first
   so the merge picks them up."

6. **Linear tie (soft — warn, never STOP).** *Feature merges only;
   skip for a promotion merge.* Grep the PR body for `HEX-\d+`. If
   found, call `mcp__linear__get_issue id=<HEX-N>` to verify it still
   resolves on the Hex Game team.
   - **No `HEX-N` reference** → ⚠ warn "PR has no Linear tie; nothing
     to transition after merge." **Proceed** — untied PRs are
     legitimate here.
   - **Reference doesn't resolve / wrong team** → ⚠ warn and treat as
     untied; suggest `/update-linear --force-rebind` for next time.
   - **Already Done / Canceled / Duplicate** → ⚠ warn; the Step-4
     sync will detect and skip the transition.

## Step 2 — Merge

**Feature merge** (base == `dev`):

```bash
gh pr merge "$PR_NUM" --merge --delete-branch
```

`--merge` (a merge commit, **never `--squash`**) preserves per-PR
history — the repo's documented policy. `--delete-branch` deletes the
**remote** feature branch, which is what "delete feature branches once
merged" means in practice; local cleanup is step 5.

**Promotion merge** (base == `main`, head == `dev`):

```bash
gh pr merge "$PR_NUM" --merge
```

Merge commit, and **no `--delete-branch`** — **never delete `dev`**.
It is the permanent integration branch, not a release branch that gets
tidied up afterwards.

If the operator passed `--force "<reason>"` in step 0, log it
alongside the merge: `gh pr comment <N> --body "merged via
/merge-pr with --force: <reason>"`. The comment lives on the PR
so the override is durably attributed.

If `gh pr merge` fails: **distinguish "merge didn't happen" from
"merge happened but local-cleanup failed"** before deciding what to
do. The two look identical at the terminal (non-zero exit + an error)
but need opposite responses.

### Worktree-conflict failure mode (Conductor)

`gh pr merge` may succeed on the GitHub API side (the merge commit
lands on `dev`), then try `git checkout dev` locally, which fails
with:

```
failed to run git: fatal: 'dev' is already used by worktree at '<path>'
```

This is **not a merge failure** — it's gh's post-merge local cleanup
failing because Conductor uses worktrees. When the checkout fails, gh
bails before deleting the remote branch, so `--delete-branch` silently
no-ops.

**Detection:** after any `gh pr merge` non-zero exit, run:

```bash
gh pr view <N> --json state,mergeCommit
```

- `state: MERGED` with a `mergeCommit.oid` → the merge already
  succeeded; the gh error was local-cleanup noise. Proceed to steps 3
  and 4 normally, and use step 5's API fallback to delete the branch.
- `state: OPEN` → the merge genuinely didn't happen. STOP and surface
  the gh error verbatim.

## Step 3 — Capture the merge commit SHA

Run `gh pr view <N> --json mergeCommit` and capture
`mergeCommit.oid` — the merge commit on `dev` (or on `main` for a
promotion). Pass it into the report so the operator can `git log <sha>`
it later. For a promotion, `/release` tags this commit.

## Step 4 — Post-merge Linear state-sync (feature merges only)

**Skip entirely** if Step 2 classified this as a **promotion merge**
(base `main`) — tickets reached Done when their PRs landed on `dev`,
and promotion touches no tickets. Also skip if Step 1.6 found no tie.

With a tie, invoke:

```
/update-linear --state=done --confirm
```

`--confirm` is non-negotiable: it makes `/update-linear` re-fetch the
ticket after the save and verify the transition actually took. The
point of the sync is observability — a silent "save returned OK but
state didn't change" leaves the board lying about what shipped.

**Failure handling: hard error on confirmation failure (not warn).**

```
✗ Merge complete (<merge_sha>), BUT Linear state-sync failed
  verification:
    saved state: Done
    actual state: <whatever Linear shows>
  Manual fix required: run
    /update-linear --state=done --confirm
  again from a session with Linear MCP. The merge cannot be
  reverted; ticket state must be corrected manually.
```

The merge already happened — reverting to "undo" a state-sync failure
would be worse than the temporary inconsistency. But the operator MUST
see it, not have it buried as a warning.

**State-already-terminal skip:** `/update-linear` detects a ticket
already in Done / Canceled / Duplicate and no-ops. If the operator
transitioned it manually first, the skip is correct.

## Step 5 — Local cleanup

```bash
git fetch origin --prune
```

- Drops the now-deleted remote branch from local refs.
- Marks the local tracking branch as `[gone]` in `git branch -vv`.

**If gh's `--delete-branch` no-op'd** (the worktree-conflict failure
mode): the remote branch is still there, so `--prune` won't drop the
tracking ref. Verify and clean up:

```bash
# Check: does the remote branch still exist?
git ls-remote --heads origin <branch-name>

# If yes, delete the remote ref via the gh API (replaces the failed
# gh pr merge --delete-branch). Then re-prune.
gh api -X DELETE repos/chillgamerboys/bevy-hex-game/git/refs/heads/<branch-name>
git fetch origin --prune
```

Without this fallback, operators see a non-zero exit from gh and
assume the merge didn't happen — the opposite of reality.

**DO NOT run `git checkout dev`** — `dev` is held by the parent
worktree, and the checkout will fail with "already used by worktree".
To inspect the merged state, `cd` to the main worktree.

**DO NOT run `git branch -D <branch>`** — the local branch is often
this very worktree's checked-out branch (undeletable), and the
operator may want it as reference. `--prune` cleans the tracking ref;
the remote deletion is what the policy actually asks for.

**Never delete `dev`**, locally or remotely, under any code path.

## Step 6 — Combined report

```
| Step | Result |
|---|---|
| 0 audit-receipt | ✓ matches HEAD (<age>) / ⚠ stale (<age>) / ✗ failed (with failing steps + findings) / ✗ old schema / ✗ SHA mismatch (unless --force) / ✗ none (hard STOP) |
| 1 pre-flights | ✓ base=dev, mergeable, tied to <REF or "untied">, branch synced, worktree noted |
| 2 merge | ✓ merged as <merge_sha> (merge commit) / ✗ <gh error> |
| 3 state-sync | ✓ <REF> → Done (confirmed) / — skipped (untied) / — skipped (promotion merge) / ✗ confirmation failed |
| 4 local prune | ✓ remote branch pruned (dev untouched) |
```

PR URL + Linear URL get echoed alongside the table for one-click
navigation.

## Releasing — this skill does not tag

`/merge-pr` finalizes one PR. It deliberately does NOT cut a version:
a release is a deliberate, aggregated event over many merges, and
tagging on every merge is the anti-pattern `/release` exists to avoid.
Version-stamping lives in `/release`, which bumps
`[workspace.package] version` and tags a promoted `main` commit.

**Pointer:** after a **promotion merge** (base `main`), the report
echoes a one-line reminder — "main advanced; run `/release` to cut a
version when ready." Informational only; it never blocks or auto-tags.

## Idempotency / PR-already-merged path

If step 1 detects `state == "MERGED"`:

- Skip steps 2 + 3 (merge already done).
- Run step 4 (state-sync) — useful if the operator merged via the
  GitHub UI and forgot to transition Linear.
- Run step 5 (local prune).
- Report with `2 merge: — (already merged as <sha>)`.

This makes `/merge-pr` safe to re-invoke after a manual merge without
double-firing anything.

## When NOT to invoke

- **Audit-pr was not run.** Run it first. `/merge-pr` is the
  finalize-after-gate step, not the gate itself.
- **Drafts / WIP PRs.** Mark `gh pr ready` first.
- **A PR nobody has played.** The receipt now carries the agent's
  scripted walk (`5_visual_walk`) — stills of the real screens, read
  by the agent — but stills are not play. If the change touches
  motion, feel, or anything the walk scripts don't photograph, and no
  human walked it, the merge is premature even with a green receipt —
  that's what `dev` is for, and what `/promote` gates on.

## Self-updating

- **Receipt format / fields change** → update Step 0's schema here
  AND the writer in `audit-pr/SKILL.md` together; a mismatch silently
  breaks the strict gate. The step keys (`0_audit_linear` …
  `4_update_docs`, plus `5_visual_walk` — numbered past the legacy
  keys although it runs as Step 2.5) are the contract.
- **Merge policy changes** → this file hardcodes `--merge` because
  CONTRIBUTING.md and CLAUDE.md both document merge commits. Change
  all three together or not at all.
- **A "Ready For Release" state gets added in Linear** → Step 4's
  feature-merge target becomes that state, and `/promote` takes over
  closing to Done on the terminal hop. Update `/update-linear`'s
  constants table first.
- **`gh pr merge --auto`** could be useful once branch protection
  requires checks; document if adopted.
