---
name: audit-linear
description: Check whether the current PR is tied to a Linear HEX-* ticket (a HEX-N reference in the branch name or PR body) on the Hex Game team. Read-only — does not create or modify tickets. Soft gate: a missing tie is a warning, never a block; the fix is `/update-linear`. Used as audit-pr Step 0 so merges are traceable to the board when a ticket exists.
---

When invoked, follow these steps.

**Soft-gate contract (this repo's departure from the seed):** a
missing or unresolvable Linear tie produces a **warning**, not a
STOP. This is a two-person game repo where plenty of work is
chore-shaped and doesn't earn a ticket. Binding is encouraged, never
required. The only hard failures here are infrastructure ones (no PR
to inspect, MCP unreachable while a reference exists to verify).

1. **Pre-flight: Require a PR.** Run
   `gh pr view --json number,title,headRefName,body 2>/dev/null`.
   If no PR, **STOP** and tell the user to create one with
   `/create-pr`. This skill inspects a PR; without one there is
   nothing to check.

2. **Search for a `HEX-\d+` reference in the standard locations.**
   Either signal is a valid tie — a branch named `HEX-42-foo-bar`, or
   a `HEX-N` reference under a `## Linear` section in the PR body.
   Both are first-class.

   ```bash
   BRANCH=$(echo "$PR_JSON" | jq -r '.headRefName')
   BODY=$(echo "$PR_JSON" | jq -r '.body')
   REF=$(echo "$BRANCH $BODY" | grep -oE 'HEX-[0-9]+' | head -1)
   ```

   - **Match found** → proceed to step 3.
   - **No match** → report a **warning** and return `warn` (not a
     failure):

     ```
     ⚠ No Linear tie found for PR #N.
       Searched: branch name '<branch>' and PR body.
       Optional: run `/update-linear` to bind an existing HEX-*
       ticket or create one. Not required — proceeding.
     ```

3. **Verify the ticket actually exists on the Hex Game team.** A stale
   reference (deleted ticket, wrong prefix, typo) is worse than no
   reference, because it *looks* traceable. Resolve via Linear MCP:

   - Tool: `mcp__linear__get_issue` with `id: "<REF>"` (Linear
     accepts `HEX-42`-style identifiers directly here — the schema
     field is `id`, not `query`).
   - Required team match: the Hex Game team, id
     `28b8704f-ced3-4884-9601-4ea07b2ca778` (key `HEX`).
   - **Not found / wrong team** → **warn**:

     ```
     ⚠ PR references <REF> but Linear can't resolve it on the
       Hex Game team (deleted? typo?). Treating as untied.
       Fix: run `/update-linear` to bind to a real ticket.
     ```

4. **Report.** On success, surface enough state for the operator to
   sanity-check the tie before the slow audit-pr steps run:

   ```
   ✓ Tied to <REF> — "<title>"
     state: <state name>   (id: <state id>)
     assignee: <name or "unassigned">
     url: <ticket url>
   ```

   If the state is `Done` / `Canceled` / `Duplicate`, **warn** but
   proceed — there are legitimate reasons to push a follow-up fix to
   an already-closed ticket:

   ```
   ⚠ HEX-42 is in state 'Done' — proceeding, but verify this is
      the intended tie. To rebind, run `/update-linear`.
   ```

## When invoked from `/audit-pr`

This skill is Step 0 of the merge-gate chain. Because it is a soft
gate, its result is recorded but **never blocks the chain** — a
`warn` must not flip the receipt's `overall_status` off green. The
audit-pr report row is:

| Step | Result |
|---|---|
| 0 audit-linear | ✓ HEX-42 (In Review) / ⚠ no tie / ⚠ HEX-42 (Done) |

## Standalone invocation

Useful before opening the PR to confirm a branch-name convention will
be picked up:

```
$ /audit-linear
✓ Tied to HEX-5 — "Port jxp-skills pipeline into the repo"
  state: In Progress (id: ac061151-a864-440e-907e-60fb4af13378)
  assignee: Shravan Kumaran
  url: https://linear.app/hex-game/issue/HEX-5
```

## Troubleshooting

**Linear MCP unavailable** — step 3's verification needs
`mcp__linear__get_issue`. If the MCP server isn't loaded in the
current session:

- **No `HEX-N` reference present** → report the step-2 warning and
  move on; nothing needed verification.
- **A reference IS present** → say so plainly: "Linear MCP not loaded
  — cannot verify HEX-N exists." Do **not** report a verified tie on
  the strength of a text grep; a claimed verification that never ran
  is exactly the silent-pass class `/audit-silent-failures` hunts.

One-time setup:

```bash
claude mcp add --transport http linear https://mcp.linear.app/mcp -s user
```

The URL is a positional argument (not `--url`). Use `--transport http`
with the `/mcp` endpoint — the older `/sse` transport was removed and
rejects calls. After adding, run `/mcp` → `linear` → authenticate,
then **restart the session** so the `mcp__linear__*` tools load.

**Multiple HEX-N references** — if the branch *and* body each carry a
different ticket ID, pick the **branch-name** match (the strongest
declared signal) and warn about the body mismatch. Operator can run
`/update-linear --force-rebind` to reconcile.

**Cross-team references** — only Hex Game tickets count. Rebind via
`/update-linear` if a lookalike from another team wandered in from a
copy-pasted branch name.

## What this skill does NOT do

- **Bind tickets** — that's `/update-linear`. This skill is read-only.
- **Validate PR contents** — that's `/audit-diff` / `/audit-pr`.
- **Block a merge over a missing ticket** — deliberately. If this
  repo ever wants a hard tie, change step 2's no-match branch from
  `warn` to STOP and update audit-pr's Step 0 contract to match.

## Self-updating

If a new "valid tie" channel emerges (e.g., a GitHub label mapping
1:1 to a ticket), add it to step 2 as an additional signal — keep the
priority order explicit so multiple-match resolution stays
deterministic. If the Hex Game team's ID or key changes, update the
constants in step 3 (and in `/update-linear`, which holds the full
table).
