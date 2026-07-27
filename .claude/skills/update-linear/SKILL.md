---
name: update-linear
description: Bind the current PR to a Linear HEX-* ticket — branch-name parse first, then prompt-pick from open HEX tickets, then create new as fallback. Also syncs ticket state via `--state` (in-progress|in-review|done), single-ticket or batched over a commit range via `--range`. Pass `--confirm` after a state-sync to re-fetch the ticket and verify the transition took. Writes to Linear via MCP and updates the PR body via `gh pr edit`. Pair with `/audit-linear` for the read-only side.
---

When invoked, follow these steps. The skill has three modes: **bind**
(default, no `--state` flag), **state-sync** (`--state=<name>`), and
**batch state-sync** (`--state=<name> --range <rev-range>`).

## Precondition — Linear MCP must be connected

This skill writes to Linear via the `mcp__linear__*` tools. If they
aren't loaded in the current session (check with `/mcp`, or notice a
"Linear MCP not loaded" failure), connect Linear's hosted MCP server,
then **restart the session** (MCP servers load at session start):

```bash
claude mcp add --transport http linear https://mcp.linear.app/mcp -s user
```

- `linear` is the server name; the URL is a **positional** argument
  (not `--url`, which older docs showed — that flag was removed).
- `-s user` registers it for all your projects. Drop it for repo-local
  scope.
- After adding, run `/mcp` → select `linear` → authenticate in the
  browser (the server uses OAuth; adding it does not log you in).
- **Use `--transport http` with the `/mcp` endpoint.** The older
  `--transport sse https://mcp.linear.app/sse` form is deprecated and
  rejected. To replace an old SSE entry:
  `claude mcp remove linear -s user && claude mcp add --transport http linear https://mcp.linear.app/mcp -s user`.

Verify with `claude mcp list` (should show `linear` on the `/mcp`
URL). The `mcp__linear__*` tools appear only after a session restart.

## Constants — Hex Game team

**Critical:** Linear MCP state-name resolution is fuzzy and
unreliable — passing `state="canceled"` to a save call can resolve to
the state literally named "Duplicate" (a documented quirk).
**Always pass `stateId`, not `state`.**

| State | ID |
|---|---|
| Backlog | `89aef568-612f-4e33-b5d6-6225be57ed67` |
| Todo | `6520524c-ff59-462f-abaf-1e263228c5fb` |
| In Progress | `ac061151-a864-440e-907e-60fb4af13378` |
| In Review | `bcf79ffe-5846-4fe4-93c5-8da4bf2237a6` |
| Done | `291e0ff7-7f8a-4e67-bc9a-cc72f0163ba8` |
| Canceled | `a4d4ce0f-b1fc-4ea9-b2ee-9943888a5684` |
| Duplicate | `32f14b4d-7bef-4506-a7c7-8b51714745c1` |

Other constants:

| Resource | ID |
|---|---|
| Hex Game team | `28b8704f-ced3-4884-9601-4ea07b2ca778` (key `HEX`) |
| Lead / operator | `b1d79196-d236-4c6a-bff8-ef45f911679f` (Shravan Kumaran) |
| Project | none — issues live directly on the team |

| Label | ID |
|---|---|
| Feature | `98aba74e-f357-40ec-953a-01f06405ecb0` |
| Improvement | `7c221608-f39a-403e-b76b-9bc683f7cf7d` |
| Bug | `2b409def-f070-4ef5-a9b5-d99ee363d51f` |

**There is no "Ready For Release" state on this team.** The pipeline's
state mapping is therefore: PR merged into `dev` → **Done**; the
`dev`→`main` promotion does **not** touch tickets. If someone adds a
Ready-For-Release state later, add the row here and change
`/merge-pr`'s feature-merge sync to use it, with `/promote` closing to
Done on the terminal hop.

These IDs are baked in because this repo hand-ported the skills (no
`project.json` render step). When Linear admins change a state name or
ID, **update the table here** — this SKILL.md is the canonical source.

## Mode A — Bind flow (no `--state` flag)

1. **Pre-flight: Require a PR.** Run
   `gh pr view --json number,title,headRefName,body,url`. If no PR,
   **STOP** with "no PR — create one with `/create-pr` first."

2. **Heuristic 1: Branch-name parse.** Extract `HEX-\d+` from
   `headRefName`. Example: `HEX-42-rename-skills` → `HEX-42`.
   If matched → skip directly to step 5 (Link & report).

3. **Heuristic 2: PR-body parse.** If the branch didn't match, grep
   the PR body for `HEX-\d+`. If matched:

   - **Default**: respect the existing tie, no-op (idempotent). Skip
     to step 5.
   - **`--force-rebind`**: ignore the existing reference and continue
     to step 4.

4. **Heuristic 3: Prompt-pick from open HEX tickets.** If neither
   heuristic matched (or `--force-rebind` was passed):

   Query Linear via `mcp__linear__list_issues` with:

   - `team: "28b8704f-ced3-4884-9601-4ea07b2ca778"`
   - states Backlog / Todo / In Progress (IDs from the table above)
   - `orderBy: updatedAt` descending, `limit: 10`

   Filter to tickets sharing ≥1 significant word with the PR title
   (drop stop-words like `the`/`a`/`add`/`fix`). Present the **top 5**
   plus a "create new" option and a "leave untied" option via
   `mcp__conductor__AskUserQuestion`:

   ```
   Q: This PR has no Linear tie. Which would you like?
   Options:
   - Bind to HEX-N — "<title>" (<state>)
   - ... (top 5)
   - Create new HEX-N from this PR's title + body
   - Leave untied (binding is optional in this repo)
   ```

   **"Leave untied" is a first-class answer here** — this repo's tie
   is soft (see `/audit-linear`). Chore and fix PRs often don't earn a
   ticket; report and exit cleanly rather than forcing a ticket into
   existence.

   The free-form "Other" input lets the operator paste `HEX-12`
   directly; validate via `mcp__linear__get_issue` before proceeding.

   - **Pick existing** → `REF = <chosen HEX-N>`, go to step 5.
   - **Create new** → step 4a.
   - **Leave untied** → report "left untied" and exit 0.

4a. **Create new HEX-N.** Call `mcp__linear__save_issue` (passing no
    `id` creates) with:

    - `team`: `28b8704f-ced3-4884-9601-4ea07b2ca778`
    - `title`: PR title (verbatim from `gh pr view --json title`)
    - `description`: a short excerpt of the PR body (first ~500 chars)
      plus a final line `**GitHub PR:** <pr url>` so the ticket
      cross-links back even before the attachment lands
    - `state`: `In Progress` — a PR open means work is in flight
    - `assignee`: `"me"` — the authenticated operator
    - `labels`: inferred from diff scope — see step 4b

    Capture the returned identifier (e.g. `HEX-45`), set `REF`, and
    proceed to step 5.

4b. **Label inference** (best-effort, one label per shape). Map the
    diff to the three available labels:

    | Diff shape | Label |
    |---|---|
    | New gameplay/map/presentation capability | Feature |
    | Bug fix, regression, crash, wrong visual | Bug |
    | Refactor, chore, docs, CI, tooling, perf | Improvement |

    Branch prefix is the strongest hint: `feat/` → Feature,
    `fix/` → Bug, `chore/`/`docs/`/`refactor/`/`perf/` →
    Improvement. Extend this table as labels are added to the team.

5. **Link & report.** Two writes happen here, in order (idempotent —
   each is a no-op if already in place):

   1. **Update PR body** via `gh pr edit <PR#> --body "<new body>"`.
      Append a `## Linear` section if missing:

      ```markdown
      ## Linear

      <REF>
      ```

      Linear's GitHub integration parses `HEX-N` references anywhere
      in the body, so the section is the operator-facing affordance,
      not a Linear requirement. If the body already has a `## Linear`
      section with a different REF, replace the value only on
      `--force-rebind`.

   2. **Attach the PR URL to the ticket** via
      `mcp__linear__save_issue` with the `links` parameter (NOT
      `create_attachment` — that tool is for base64 file uploads,
      not URL links):

      ```json
      {
        "id": "HEX-N",
        "links": [{"url": "<github pr url>", "title": "GitHub PR #<N> — <title>"}]
      }
      ```

      `links` is **append-only** — re-running with the same URL adds a
      duplicate. Fetch the issue via `mcp__linear__get_issue` first
      and skip if `attachments[]` already contains a matching URL.

   3. **Report:**

      ```
      ✓ Bound PR #<N> to <REF> — "<title>"
        state: <state>
        url: <ticket url>

      PR body updated; Linear ticket has an attachment to the PR.
      ```

## Mode B — State-sync (`--state=<name>`)

Accepted state names map to the constants-table IDs:

- `in-progress` → In Progress (after a revert, or back to the drawing board)
- `in-review` → In Review
- `done` → Done (what `/merge-pr` sets when a feature PR lands on `dev`,
  or batch-sets when a wave lands)

**The partial-epic rule (wave model):** a ticket whose PR merged into a
*wave* stays **In Review** until the wave lands on `dev` — and stays In
Review even then if the epic is only partially delivered; it goes Done
when its *scope* is done, not when a PR merges. Linear's GitHub
integration fights this by auto-closing tickets on PR merge — check
after every into-wave merge and revert auto-closes to In Review with a
comment naming the outstanding scope (wave 1's HEX-6 precedent).

The skill **rejects** state names outside this map with a clear
"unknown state" message — never falls back to fuzzy resolution. That
is the explicit defense against the fuzzy-name quirk. (`ready-for-release`
is deliberately absent — no such state on this team.)

Flow:

1. **Pre-flight: PR exists and has a Linear tie.** Run `/audit-linear`
   inline. If it reports no tie, this mode has nothing to sync — report
   "no Linear tie; nothing to transition" and exit 0. **Not an error:**
   untied PRs are legitimate here.

2. **Resolve ticket ID.** Capture `REF` from the audit output.

3. **Transition state.** Call `mcp__linear__save_issue` with:

   - `id`: the ticket identifier or UUID
   - `state`: pass the **state ID** from the constants table

4. **If `--confirm` was passed: verify the transition.** Re-fetch via
   `mcp__linear__get_issue` and assert the state matches the expected
   ID. On mismatch (Linear admin renamed a state, MCP race), surface a
   **hard error** — silently leaving the ticket in the wrong state
   defeats the point of syncing. Without `--confirm`, the assertion is
   skipped.

5. **Report.**

   ```
   ✓ Transitioned HEX-42 → Done
     was: In Review (id: bcf79ffe-…)
     now: Done      (id: 291e0ff7-…)
   ```

   With `--confirm`, the "now" line is read from the re-fetched
   ticket, not assumed from the save call.

## Mode C — Batch state-sync over a commit range (`--range <rev-range> --state=<name>`)

For closing out several tickets at once (e.g. after a batch of merges).
`--range` requires `--state`.

> **Range caveat — `<target>..<source>` is empty AFTER a merge.** It
> captures the commits only *before* the merge; once `<source>` is
> merged it is an ancestor of `<target>`, so
> `origin/<target>..origin/<source>` yields **zero** commits. Capture
> the target tip **before** merging
> (`PRE=$(git rev-parse origin/dev)`, then `--range "$PRE..origin/dev"`).

Flow:

1. **Pre-flight.** `--range` without `--state` → STOP. Resolve the
   state alias via the constants table (reject unknown aliases).

2. **Collect tickets in the range.**

   ```bash
   git log <rev-range> --format='%s%n%b' \
     | grep -oiE 'HEX-[0-9]+' | sort -u
   ```

   Dedupe. A commit with no `HEX-N` (chore, merge commit) is simply
   not in the set — expected here, but counted (step 4) so the delta
   stays visible.

3. **Per ticket: resolve → skip-if-terminal → transition.**
   - Resolve identifier → current state via `mcp__linear__get_issue`.
   - **Skip** if already `Done` / `Canceled` / `Duplicate`
     (idempotent — re-running never churns shipped tickets).
   - Else `mcp__linear__save_issue` with the state **ID**.
   - With `--confirm`: re-fetch + assert. **Collect** per-ticket
     failures — do NOT stop at the first (a 12-ticket batch must not
     abort at ticket 3, leaving 9 unprocessed). After the loop, if any
     failed, surface ONE hard error listing the failed REFs.

4. **Report counts (no silent miss).**

   ```
   ✓ Batch state-sync over <rev-range> → Done
     commits scanned: 14   tickets found: 11
     transitioned: 8   skipped (already terminal): 3   failed: 0
   ```

   The scanned-vs-found delta surfaces untied commits instead of
   silently dropping them.

## Operator invocation patterns

```bash
# Just after `gh pr create` succeeds (chained by /create-pr):
/update-linear                              # Bind (heuristic-driven)
/update-linear --state=in-review --confirm  # Optional state set

# After `gh pr merge` into dev (chained by /merge-pr):
/update-linear --state=done --confirm

# Batch close (capture the range BEFORE merging — see the Mode C caveat):
/update-linear --state=done --range "$PRE..origin/dev" --confirm

# Rebind to a different ticket (rare):
/update-linear --force-rebind
```

`/merge-pr` always passes `--confirm` so a silent state-sync failure
can't ship code with a ticket stuck in "In Review."

## When invoked from `/audit-pr`

`/audit-pr` does **not** invoke this skill — audit-pr is the read-only
gate. `update-linear` is the operator's tool for fixing linkage when
audit-pr Step 0 (`audit-linear`) reports no tie. Same relationship as
`audit-diff` (gate) ↔ `update-docs` (writer): a finding on the gate
points at the writer skill.

## Troubleshooting

**Linear MCP unavailable** — this skill cannot do its job without
MCP. Fail loud; do **not** update the PR body without a real ticket on
the other side. A PR body claiming a tie to a non-existent ticket is
exactly the silent-pass class.

**State transition that doesn't actually fire** — pass `--confirm` so
the post-update fetch catches it. The usual cause is passing a state
*name* instead of the *ID* — re-read the constants table.

**PR body too large** — `gh pr edit --body-file` accepts a path. Write
to `/tmp/pr-body-<N>.md` first if the body is multi-section.

**The `Other` answer came in without a `HEX-` prefix** — validate as
an identifier: a bare integer `42` gets prefixed (`HEX-42`); a UUID
goes to `get_issue` directly. Anything else → reject with "doesn't
look like a Linear identifier."

**Duplicate attachment** — fetch existing attachments before
creating, and skip on match. Idempotency keeps re-invocation safe.

## Self-updating

- **New state added by Linear admins** (notably `Ready For Release`) →
  add a row to the constants table, add the alias to Mode B's map, and
  revisit the merge/promote mapping noted above.
- **New label added** → add a row + extend the step-4b inference table.
- **Estimates enabled on the team** → they're off today, so this skill
  doesn't set them. If enabled, add an `estimate` to the 4a create
  call and note the scale (a scale must include the values used).
- **MCP tool renamed** → update tool references. URL attachments go
  through `save_issue.links` (append-only), not `create_attachment`.
