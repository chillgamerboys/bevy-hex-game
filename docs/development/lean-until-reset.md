# Lean development until the September 14 usage reset

User-approved temporary workflow for this local game experiment. The last observed
allowance was 18% remaining; the expected reset is September 14, 2026 at 6:26 p.m.
America/Los_Angeles (September 15 at 01:26 UTC). This is a recorded reading, not a
live balance. Apply these rules until the reset is confirmed or the user changes
them. They supersede earlier 16% checkpoint instructions in historical arena plans.

## Budget and scope

- Check usage once per small development task; reuse a recent reading during
  continuous work. Check again only if scope expands unexpectedly.
- Below 10%, accept only small, predictable changes. At 7% or less, stop and
  checkpoint existing work with a brief handoff. Do not spend the buffer on more
  investigation or validation. The user's floor is 5%; other account activity can
  also consume allowance, so this is not an account-wide enforced limit.
- Choose one observable improvement per iteration, or a small batch of related
  tuning values. State the result briefly before implementing it.
- Prefer one agent. Delegate only when it clearly reduces total work. Avoid
  unrelated cleanup, broad research, large file dumps and repeated reports.
- After two unsuccessful fix attempts, pause that investigation and use the
  user's reproduction or playtest feedback to narrow it. Preserve a safe local
  checkpoint and state any known failure; do not call incomplete work tested.

## Verification and playtests

- Balance, speed, size and visual changes: build as needed, inspect the changed
  configuration, then primarily use the user's native playtest.
- Input, cooldown, collision and damage changes: run the smallest relevant
  existing tests. Add a regression only for a concrete failure manual testing
  could easily miss.
- Assemble the change before running checks. Repeat only affected failures or
  checks justified by further changes. Defer broad suites and tournaments.
- Required merge checks still apply before merging. Keep unfinished validation
  explicit in local deliveries rather than claiming a merge-ready candidate.
- For playable changes, hand off two or three specific things to try, with their
  expected behavior. Documentation-only changes need no game build or playtest.

## Delivery

Make small local commits. Maintain a brief note in the existing arena wave record:
what changed, what passed, and what remains untested. Prefer several useful,
playtested improvements over one large feature. No automatic background work or
usage polling is established by this policy.
