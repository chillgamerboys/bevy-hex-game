---
name: land-development-wave
description: Reconcile related source branches or PRs into one reviewed wave and land it on dev. Use for a batch of three or more dependent PRs, stale stacked branches, duplicate shared logic, or a feature set whose release confidence comes from combined runtime behavior. Do not use for unrelated PRs or a dev-to-main promotion.
---

# Land Development Wave

Turn a branch or PR batch into one coherent candidate while preserving authorship,
owner boundaries, and useful provenance. Apply focused scrutiny to each source diff,
then run the selector-chosen candidate gate and applicable runtime review once on the
combined wave.

Merging PRs and changing remote branches require explicit user authorization. Without
it, prepare and validate the wave but stop before remote mutation.

## Read the plan and contracts

Read `AGENTS.md`, `CLAUDE.md`, and
`docs/development/parallel-development.md`. Read the supplied wave manifest. If none
exists, use `$plan-parallel-work` to classify and record the batch before changing
branches.

Read the relevant architecture, contract, boundary, system, and design docs for every
shared concern. The wave is an integration boundary, not a new owner.

## 1. Inventory before mutation

Fetch `dev` and every source branch. For each source, record:

- PR number and draft/review state, if any;
- declared base and actual merge base;
- head SHA and unique commit range;
- changed files, contracts, and owner;
- parent/child PR relationships;
- current checks; and
- intended residual behavior.

Read `references/reconciliation-playbook.md` when ancestry is stacked, bases differ, or
the source contains parent work.

Do not trust a draft label as a readiness signal. Do not trust a green leaf check as
evidence of combined readiness.

## 2. Create or refresh the wave

Create `wave/<name>` from current `origin/dev`, or inspect the existing wave before
using it. Merge updated `dev` into an already-published wave with an additive commit;
never rewrite shared history.

Only the integration owner writes directly to the wave. Preserve source branches
until landing and cleanup are complete.

## 3. Integrate in semantic order

Apply:

1. shared foundation and contract corrections;
2. owner-local foundations;
3. feature lanes;
4. composition and shared adapters;
5. integration-only fixes.

Use a normal merge only when the source's complete ancestry belongs in the wave. If a
stacked tip carries obsolete parent state, transplant the intended unique commits
from the correct fork point or reproduce its small residual diff on the wave. Audit
the resulting diff against `origin/dev` after every source.

When lanes touch the same concern, determine whether they implement the same feature
or distinct authorities:

- same authority: consolidate on one contract and remove the duplicate;
- distinct authorities: keep implementations separate and connect them through the
  published contract;
- behavior-changing ownership ambiguity: stop for an explicit decision.

Mechanical conflicts and straightforward contract adaptation do not require user
review. Fix them on the wave with additive commits and record the resolution.

## 4. Review at combined checkpoints

After each semantic group:

- inspect source intent and the aggregate diff;
- run affected checks and deterministic scenarios;
- test composition, failure behavior, regeneration, and state re-entry;
- inspect automated visual-walk frames for affected presentation surfaces; and
- update the wave manifest with findings and fixes.

Screenshots/frames may judge static camera, UI, and rendered-map presentation;
video/human checks may judge motion, input response, control feel, and taste. They may
show how hook-established state is rendered, but never prove or corroborate gameplay
or exact world logic when typed hooks, state, messages, logs, snapshots, or
deterministic contracts can prove the claim. If that oracle is missing, add the narrow
hook or contract instead of inferring logic from pixels.

Do not demand a separate full workspace run after every mechanically integrated leaf.
Escalate validation when a semantic group becomes coherent.

Before the final gate, strongly prefer `$reconcile-delivery-state` on the candidate.
Include status, roadmap, design, contract, and ticket-description corrections in the
wave. Keep partial epics open even when a leaf merges. Missing Linear access is a
visible warning and handoff item, never a merge blocker.

## 5. Gate the wave

On the final candidate, run the selector-chosen checks listed in
`docs/development/parallel-development.md`, push the exact reviewed state, and let the
wave PR run its selected platform and coverage jobs. Run the automated visual walk and
human play route only for affected presentation, native-input, motion, or feel claims.
A logic-only wave uses its authoritative hooks and the verified-maintainer N/A process
from the testing contract; wave topology alone does not manufacture a visual gate.

A compiler, lint, test, documentation, coverage, or application failure blocks
landing. Retry a likely infrastructure failure once. A second identical hard timeout
requires a recorded maintainer waiver; do not silently treat it as green.

## 6. Land and clean up

When authorized and all required evidence is green:

1. merge the wave PR to `dev` with a merge commit;
2. verify the resulting `dev` SHA and checks;
3. close leaf PRs as superseded with a link to the wave;
4. retarget every open child before deleting its parent;
5. delete source and wave branches only after nothing depends on them;
6. recommend or apply delivered, partial, and obsolete ticket updates through
   `$reconcile-delivery-state`, re-fetching modified issues when Linear is available;
   and
7. provide reconciliation instructions for protected ongoing branches.

Never merge the wave directly to `main`. Promotion is separate.

## Report

Return:

- wave PR and merge SHA, or the exact remaining blocker;
- source PR disposition and preserved provenance;
- shared concerns consolidated and ownership decisions made;
- checks, presentation evidence, retries, and waivers;
- branches retained or deleted and why; and
- instructions for ongoing branches to absorb updated `dev`.
