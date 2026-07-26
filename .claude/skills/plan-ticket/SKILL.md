---
name: plan-ticket
description: Read a Linear HEX-* ticket and produce an approved implementation plan before any code is edited. Fetches the ticket, comments, parent, and linked PRs via Linear MCP; surveys the affected crates, docs, and prior art; asks clarifying questions until no blocking ambiguity remains; then presents the plan via plan mode for explicit approval. On approval, posts the plan to the ticket, moves it to In Progress, and creates the feature branch off `dev`. Use when starting work on a ticket ("plan HEX-42", "pick up HEX-42"). Front end of the pipeline — /plan-ticket → implement → /create-pr → /audit-pr → /merge-pr.
---

Produce a plan the operator explicitly approves **before the first
code edit**. Two hard rules govern every step:

1. **Never guess.** An unresolved doubt becomes a question to the
   operator, not an assumption in the plan. A wrong assumption
   compounds through implementation, review, and re-work.
2. **Research before asking.** A question the codebase, docs, git
   history, or the ticket itself can answer must be answered by
   research. Only genuine unknowns — design intent, scope boundaries,
   priority calls — go to the operator.

Copy this checklist and track progress:

```
Plan Progress:
- [ ] Step 0: Resolve ticket + pre-flight
- [ ] Step 1: Fetch full ticket context from Linear
- [ ] Step 2: Route to crate(s)
- [ ] Step 3: Survey code, docs, and prior art
- [ ] Step 4: Resolve ambiguity (clarifying questions)
- [ ] Step 5: Draft plan → plan-mode approval
- [ ] Step 6: Post-approval actions (comment, state, branch)
```

## Step 0 — Resolve ticket + pre-flight

1. **Resolve the ticket ID**, in priority order:
   - Explicit argument: `/plan-ticket HEX-42` → use it.
   - Current branch name contains `HEX-\d+` (case-insensitive) →
     propose that ticket and confirm before proceeding.
   - Neither → list candidates via `mcp__linear__list_issues`
     (`team: "28b8704f-ced3-4884-9601-4ea07b2ca778"`, states
     Backlog/Todo, `orderBy: updatedAt`, `limit: 10`) and ask the
     operator to pick.

2. **Linear MCP loaded.** If `mcp__linear__*` tools are unavailable,
   **STOP**: "Linear MCP not loaded — cannot plan from a ticket."
   Setup is the same as `/update-linear`
   (`claude mcp add --transport http linear https://mcp.linear.app/mcp -s user`,
   authenticate via `/mcp`, restart the session). Do not plan from a
   pasted ticket description as a fallback — comments and links carry
   decisions the description lacks.

3. **Clean-enough worktree.** Uncommitted changes don't block
   planning, but warn: Step 6 creates a branch, and the operator
   should stash or commit WIP first.

## Step 1 — Fetch full ticket context from Linear

The description alone is not the ticket. Fetch all of:

- **The issue**: `mcp__linear__get_issue` with `id: "HEX-N"`. Verify
  it's on the Hex Game team — **STOP** on any other team (same rule
  as `/audit-linear`).
- **Comments**: `mcp__linear__list_comments` — later comments
  routinely override the description (scope cuts, decisions, answers
  to prior questions). Treat the newest decision as authoritative and
  note contradictions for Step 4.
- **Parent / children**: if the ticket has a parent, fetch it for the
  broader goal; list siblings to learn what is deliberately out of
  scope.
- **Attachments / links**: linked PRs and documents. A linked closed
  PR often *is* the prior art.

If the ticket state is `Done`, `Canceled`, or `Duplicate`, warn and
confirm the operator really wants a plan for it before continuing.

## Step 2 — Route to crate(s)

Decide where the change lands **before** surveying code. The crate
graph is enforced by Cargo, so routing is a correctness question, not
a style one — read `docs/ARCHITECTURE.md` for the reasoning and
`CONTRIBUTING.md`'s "Where code goes" table for the short version:

| Adding | Goes in |
|---|---|
| Hex math, voxel positions, substances, shared types, states, ordering sets | `hex_core` |
| Voxels, terrain, tile spawning, map settings | `hex_map` |
| Asset loading, shared settings | `hex_assets` |
| Sky and camera | `hex_world` |
| Rules: input, movement, interaction | `hex_units` |
| The turn order, engagement, AI | `hex_combat` |
| Transform animation | `hex_anim` |
| A debug tool | `hex_dev` |
| A screen, menu, or app wiring | `hex_game` |

Two constraints the plan must respect:

- **`hex_map`, `hex_world` and `hex_units` may not depend on each
  other.** Anything shared goes in `hex_core`. If the plan needs a
  new shared type, say so explicitly — that's a `hex_core` change and
  a cross-owner conversation.
- **Ownership.** The map is one person's; `hex_units` / `hex_combat`
  are the other's. A plan that changes *design* inside someone else's
  crate needs their agreement, not just an approved plan — flag it in
  the plan's risks rather than assuming.

## Step 3 — Survey code, docs, and prior art

Prime the plan with complete information. In rough order:

1. **Docs first**: `CLAUDE.md` (the operational summary and the
   Bevy-0.19 trap list), then `docs/ARCHITECTURE.md` for structure,
   and the area's own doc — `docs/MAP_MODEL.md` for anything touching
   voxels/tiles, `docs/GAMEPLAY_LOOP.md` for turns/modes/movement,
   `docs/DESIGN.md` for what the mechanic is *for*, `docs/CONTENT.md`
   for RON-configurable values. `crates/hex_map/CLAUDE.md` if the
   change is in the map. These state intent; code states reality —
   note any doc/code drift for the plan's risks.
2. **Targeted code exploration**: locate the modules, components,
   systems, and tests the ticket touches. For broad sweeps, fan out
   Explore agents rather than dumping whole files into context.
3. **Prior art**: recent merged PRs and commits in the same area
   (`git log --oneline -- <paths>`,
   `gh pr list --search "<keywords>" --state merged --limit 5`). A
   sibling feature that already shipped usually fixes the pattern to
   follow — and this repo's PR discussions record *why* several rules
   are the way they are.
4. **Tests**: which crate's suite covers the area, whether the change
   needs a headless `App` integration test in `crates/<crate>/tests/`
   or a pure unit test, and what fixtures exist. Remember the
   fixture-realism bar: a fixture too simple to express the bug
   reports a safety it doesn't provide.

Output of this step (kept internal): the file-level change surface,
the pattern to follow, and the docs the change will obligate updating
— **doc updates ship in the same PR**, they are part of the plan.

## Step 4 — Resolve ambiguity (clarifying questions)

This is the load-bearing step. Walk this checklist against the ticket
+ survey; every unresolved row becomes a question:

| Dimension | Must be unambiguous before planning |
|---|---|
| Scope | Which crates/systems/flows are in vs out |
| Behavior | Expected in-game behavior; how the player sees it; what happens on the failure path |
| Design intent | Is this settling a question `docs/DESIGN.md` left open? Whose call is it? |
| Rules interaction | How it interacts with the stacking rule, headroom, one-level steps, turn order, the high-ground rule |
| Data / components | New components or shared `hex_core` types; new RON config fields and when they're read |
| Edge cases | Level 0, buried runs, no-route, empty span, a body that doesn't fit, mid-walk interruption |
| Conflicts | Ticket says X, code/docs say Y — surface it, never silently pick |
| Provisional values | Is the ticket tuning something the design deliberately hasn't decided? Those are meant to be replaced, not tuned |

Question mechanics:

- Ask via `AskUserQuestion` — concrete options with a recommended
  default first, plus the built-in free-text escape.
- Batch up to 4 per round; **multiple rounds are expected**, not a
  failure. Answers often surface new unknowns — loop until the
  checklist has no blocking row.
- Distinguish **blocking** (changes the plan's shape — ask now) from
  **deferrable** (implementation detail — record in "Open questions"
  with your proposed default).
- Record every answer verbatim for the plan's **Decisions** section so
  the rationale survives into the PR and ticket.
- If an answer contradicts the ticket, note that the ticket needs a
  comment update (Step 6 posts it).

**Do not enter plan mode with a blocking unknown.** If the operator is
unavailable and a blocking question can't be answered, stop and report
the questions — an unapproved half-plan beats a wrong plan.

## Step 5 — Draft plan → plan-mode approval

Enter plan mode (`EnterPlanMode`) and write the plan with these
sections (adapt depth to the ticket, keep the skeleton):

```markdown
# HEX-N — <title>

## Context
What the ticket asks for and why (1 short paragraph, from Step 1).

## Decisions
Q&A from Step 4, verbatim — each answer with who decided it.

## Approach
The chosen approach and, if alternatives were considered, one line
on why they lost.

## Changes
Per crate, file-level: path → what changes. Follow the prior-art
pattern named in Step 3. Name any new shared `hex_core` type
explicitly.

## Documentation
Every doc the change obligates (docs/, CLAUDE.md, the crate's own
CLAUDE.md). Same PR, mandatory.

## Tests
New/changed tests, which crate, unit vs headless integration,
fixtures. Note anything only a human at the window can verify.

## Risks & edge cases
Including doc/code drift found in Step 3, and any cross-owner
design question.

## Out of scope
Explicitly deferred items (feeds the PR body's section later).

## Open questions (non-blocking)
Deferred details with the proposed default for each.
```

Plan rules:

- **No effort/time estimates** — describe scope (files, additions,
  removals, dependencies), never duration.
- **Sizing check**: if Changes spans many crates or mixes concepts,
  propose the split into multiple PRs up front.
- **Visual verification is always in the plan** when the change
  touches rendering, transforms, movement, or state transitions — CI
  cannot see a black sky or a sunken piece.
- Exit plan mode via `ExitPlanMode` for approval. If the operator
  edits or rejects, incorporate and re-present — approval must be
  explicit, not inferred.

## Step 6 — Post-approval actions

Confirm once ("Post plan to HEX-N, move it to In Progress, and create
the branch?"), then:

1. **Post the plan to the ticket** via `mcp__linear__save_comment` —
   the approved plan condensed to Context / Decisions / Approach /
   Out of scope (skip the file-level listing; Linear readers want the
   *what and why*).
2. **Move the ticket to In Progress**: `mcp__linear__save_issue` with
   the In Progress state **ID** `ac061151-a864-440e-907e-60fb4af13378`
   — never a state *name*; resolution is unreliable (see
   `/update-linear`'s constants table, the source of truth). Also set
   `assignee: "me"`.
3. **Create the branch off `origin/dev`** — the branch `/create-pr`
   targets as the PR base and `/merge-pr` merges into. Use this
   repo's prefixes (`feat/`, `fix/`, `perf/`, `chore/`, `docs/`,
   `refactor/`) and include the ticket ID so `/audit-linear` can find
   it:

   ```bash
   git fetch origin dev
   git checkout -b "feat/hex-N-<kebab-slug>" origin/dev
   ```

   Pick the prefix that matches the work, not always `feat/`.

4. **Report and hand off**:
   ```
   ✓ /plan-ticket complete — HEX-N "<title>"
     plan:   approved + posted to ticket
     state:  In Progress (assigned to you)
     branch: feat/hex-N-<slug> (off dev)
   Next: implement per plan, then /create-pr.
   ```

Nothing in this step commits, pushes, or opens a PR — those need their
own explicit go-ahead.

## Troubleshooting

**Linear MCP unavailable** — STOP per Step 0; never plan from a pasted
description while claiming ticket fidelity.

**Ticket is a roadmap epic** (broad, no direct work) — it wants
splitting first. Offer to plan a narrower slice, and note that
roadmap rows become tickets via `/seed-tickets`, not implementation
plans.

**Ticket already has a linked open PR** — someone may be mid-flight.
Surface the PR and ask before planning over it.

**Operator answers change mid-plan** — update the Decisions section
and re-present via plan mode; the posted ticket comment must match the
final approved plan, so post only after final approval.

**Ticket asks to tune a provisional value** — the gameplay docs mark
certain numbers as placeholders standing in for undecided design.
Tuning them into place is usually the wrong move; ask whether the
ticket means to *settle the design question* instead.

## What this skill does NOT do

- **Implement** — the plan's approval ends this skill; edits follow as
  ordinary work under the approved plan.
- **Create tickets** — that's `/seed-tickets` (roadmap→backlog) or
  `/update-linear` (bind-time fallback).
- **Open PRs or verify ties** — `/create-pr` and `/audit-linear`.
- **Estimate effort** — never, in any section.

## Self-updating

- **Workflow state IDs / team ID change** → update Step 6.2 here and
  `/update-linear`'s constants table (the canonical copy).
- **Branch naming convention changes** → update Step 6.3 and keep it
  consistent with what `/audit-linear` greps for.
- **Doc paths change** (the docs restructure moves `MAP_MODEL.md`,
  `GAMEPLAY_LOOP.md`, `CONTENT.md` under a kind-separated tree) →
  update Step 3.1's pointers.
- **New ambiguity class bites in review** (a bug traced to an unasked
  question) → add a row to Step 4's table so the next plan asks it.
