---
name: seed-tickets
description: Turn `docs/planning/roadmap.md` into Linear HEX-* tickets, idempotently. Each roadmap row becomes a ticket; inline `<!-- linear: HEX-N -->` markers are the idempotency key, with a Linear-side `seed-key` backstop for lost markers. Re-running never duplicates. Human-run, never automated. Pair with `/update-linear` (per-PR binding) — this is the roadmap-to-backlog front end.
---

When invoked, follow these steps. This skill is **human-run and
idempotent**: it only ever *creates* tickets for roadmap entries that
don't already have one, and a re-run on an unchanged repo creates
nothing. It never edits or closes existing tickets (see **Scope**).

## Precondition — the roadmap must exist

This skill reads **`docs/planning/roadmap.md`**. It exists, and its
`## Upcoming` table is the parse target. The roadmap is human-authored
— this skill will not invent epics, and it will not rewrite a row's
scope once a ticket exists for it.

If it is ever missing (a fresh branch predating it, say), source
material for rebuilding one: `docs/planning/status.md`'s "not built,
and not next", the open questions in `docs/design/game.md`, and the
production checklist in `docs/planning/production-audit.md`. The
things that are genuinely next are already written down in prose — the
roadmap is where they become rows.

Suggested row shape (a table the skill can parse):

```markdown
## Upcoming

| Epic | Scope | Owner |
|---|---|---|
| Lattices | The hex grid a character casts from; damage disables hexes | units |
| Multi-hex bodies | Footprint on `Body`; the straddle rule for a one-level step | units |
| Unit obstruction | Occupancy over unit positions so routes can't pass through pieces | units |
```

If the file is missing, **STOP** and say so plainly, with the above as
the suggestion. Do not scaffold a roadmap full of guesses.

## Precondition — Linear MCP must be connected

This skill writes to Linear via the `mcp__linear__*` tools. If they
aren't loaded in the current session (check with `/mcp`), connect
Linear's hosted MCP server, then **restart the session** (MCP servers
load at session start):

```bash
claude mcp add --transport http linear https://mcp.linear.app/mcp -s user
```

- `linear` is the server name; the URL is a **positional** argument.
- `-s user` registers it for all your projects; drop it for repo-local.
- After adding, run `/mcp` → select `linear` → authenticate in the
  browser (OAuth; adding the server does not log you in).
- **Use `--transport http` with the `/mcp` endpoint** — the older
  `/sse` transport was removed and is rejected.

If MCP is unavailable, **STOP** — do not write markers into the
roadmap without real tickets on the other side. A doc claiming a
ticket that doesn't exist is the silent-pass class.

## Constants — Hex Game team

**Critical:** Linear MCP state-name resolution is fuzzy — passing
`state="..."` can resolve to the wrong workflow state. **Always pass
the state ID.**

| State | ID |
|---|---|
| Backlog | `89aef568-612f-4e33-b5d6-6225be57ed67` |
| Todo | `6520524c-ff59-462f-abaf-1e263228c5fb` |
| In Progress | `ac061151-a864-440e-907e-60fb4af13378` |
| In Review | `bcf79ffe-5846-4fe4-93c5-8da4bf2237a6` |
| Done | `291e0ff7-7f8a-4e67-bc9a-cc72f0163ba8` |
| Canceled | `a4d4ce0f-b1fc-4ea9-b2ee-9943888a5684` |
| Duplicate | `32f14b4d-7bef-4506-a7c7-8b51714745c1` |

| Resource | ID |
|---|---|
| Hex Game team | `28b8704f-ced3-4884-9601-4ea07b2ca778` (key `HEX`) |
| Lead (assignee fallback) | `b1d79196-d236-4c6a-bff8-ef45f911679f` |
| Project | none — issues live directly on the team |

| Label | ID |
|---|---|
| Feature | `98aba74e-f357-40ec-953a-01f06405ecb0` |
| Improvement | `7c221608-f39a-403e-b76b-9bc683f7cf7d` |
| Bug | `2b409def-f070-4ef5-a9b5-d99ee363d51f` |

`/update-linear` holds the canonical copy of this table — keep them in
sync when Linear admins change anything.

## Identity and ownership

- `DEV` = a slug of `git config user.name` (lowercase, spaces → `-`);
  fall back to the local-part of `git config user.email`.
- `ASSIGNEE` = the runner's Linear user. Resolve best-effort via
  `mcp__linear__list_users` matched on `git config user.email`; if no
  match, fall back to the Lead ID above and note it in the report.

This repo's work splits by **crate ownership** — the map is one
person's, `hex_units`/`hex_combat` the other's. A roadmap row's
`Owner` column should name that area, and the claim marker records who
actually took it.

## Flow

### Step 1 — Sync first (collision layer 1)

Run `git pull --rebase` on the branch holding the roadmap. This kills
the common collision: a stale local copy that doesn't show the other
person's already-written markers. If the rebase conflicts, **STOP**
and tell the operator to resolve — never auto-resolve roadmap markers.

### Step 2 — Load the roadmap

Read `docs/planning/roadmap.md`. If missing → the precondition STOP
above.

### Step 3 — Parse rows + show claim status

Parse the rows of the roadmap's epic table. For each row, read any
trailing marker and classify:

- **mine** — `owner: <DEV>` matches the runner.
- **claimed** — `owner:` is someone else. Off-limits.
- **unclaimed** — no marker.

Present the list (epic · status · linked `HEX-N` if any) via
`mcp__conductor__AskUserQuestion` and let the operator pick an
**unclaimed** or **mine** row. If they pick a **claimed** one, refuse:
"claimed by <owner> — pick another or coordinate with them."

Multi-select is fine here — seeding several rows in one run is the
common case for a fresh roadmap.

### Step 4 — Backstop dedup (collision layer 2)

For each candidate row, guard against a lost marker (manual ticket,
fresh clone, dropped commit) before creating. Compute
`seed-key = HEX/<epic-slug>` (lowercase, non-alphanumerics → `-`,
collapse repeats), then:

1. Query team tickets via `mcp__linear__list_issues`
   (`team: "28b8704f-ced3-4884-9601-4ea07b2ca778"`, reasonable `limit`).
2. For likely title matches, `mcp__linear__get_issue` and check the
   description for the candidate's `seed-key` footer.
   - **Match** → **adopt**: write the existing `HEX-N` marker back
     onto the row; do not create.
   - **No match** → leave as a create-candidate.

### Step 5 — Dry-run plan + confirm (collision layer 4)

Present the plan via `mcp__conductor__AskUserQuestion` before any
writes:

```
Roadmap → HEX tickets
  create: <N>   skip (already marked): <M>   adopt (matched): <K>
  to create:
    - "<epic>"  (owner: <DEV>, label: <label>)
    - ...
Proceed?
```

Options: **Create all** / **Let me pick** (multi-select) / **Cancel**.
Nothing is written to Linear until the operator confirms.

### Step 6 — Create tickets

For each confirmed candidate, call `mcp__linear__save_issue` (no `id`
→ create) with:

- `team`: `28b8704f-ced3-4884-9601-4ea07b2ca778`
- `title`: the epic name (verbatim from the roadmap row)
- `description`: the row's scope cell, plus a final line
  `<!-- seed-key: HEX/<epic-slug> -->` (the Step-4 backstop key)
- `state`: the **Backlog** state ID — roadmap work is planned, not in
  flight. (`/plan-ticket` moves it to In Progress when someone starts.)
- `assignee`: `ASSIGNEE`
- `labels`: best-effort — a new capability is `Feature`, a known gap
  in existing behavior is `Bug`, tooling/refactor/docs work is
  `Improvement`.

Estimates are **not** set: estimation is off on this team. If it is
enabled later, add an `estimate` here and document the scale.

Capture each returned `HEX-N`.

### Step 7 — Write markers back + commit

For each created or adopted row, write
`<!-- linear: HEX-N owner: <DEV> -->` onto the roadmap row (after the
trailing `|`). Then:

```sh
git add docs/planning/roadmap.md
git commit -m "chore(roadmap): seed HEX tickets"
git push
```

**If the push is rejected** (non-fast-forward), the other person
committed first. Run `git pull --rebase`, re-read the rows, and:

- a row now shows another `owner:` → it was claimed under you; re-run
  from Step 3 and steer elsewhere. Its ticket, if you created one,
  is a duplicate — say so in the report so the operator can cancel it
  in Linear (this skill never closes tickets).
- rows still unclaimed → re-push.

`git push` is the serializer: the first person to land a claim owns
the row. Markers are HTML comments — invisible in rendered Markdown,
tracked in git, no external state file.

### Step 8 — Report

```
✓ Seeded roadmap → HEX (owner: <DEV>)
    created: <N>   adopted: <K>   skipped (already marked): <M>
    assignee: <resolved Linear user | Lead fallback>
    tickets:
      HEX-<n>  "<epic>"
      ...
```

Re-running now reports `created: 0` for unchanged rows — proof the run
is idempotent.

## Idempotency & collision — why each layer exists

| Layer | Mechanism | Catches |
|---|---|---|
| 1 | `git pull --rebase` first (Step 1) | Stale local copy missing the other person's markers. |
| 2 | `seed-key` backstop query (Step 4) | Lost marker — manual ticket, fresh clone, dropped commit. |
| 3 | Push-to-claim (Step 7) | Both people grabbing the same row at once. |
| 4 | Dry-run `AskUserQuestion` (Step 5) | Bulk-create mistakes; the human is the final gate. |

Inline markers are the primary idempotency key; the `seed-key` is
defense-in-depth for when git state and Linear state drift apart.

## Scope (deliberately bounded)

- **Create-only.** A marker means *leave it alone*. Editing a roadmap
  row after its ticket exists does **not** update the ticket.
- **No auto-close.** Deleting a row leaves its ticket open (orphan);
  the skill ignores it.
- **Single tier.** The seed's per-developer breakdown files were
  dropped in this port — two people and crate ownership make the extra
  tier ceremony rather than structure. If the roadmap ever needs
  epic→child nesting, add it here with `parentId` on the child create.

## Self-updating

- **New Linear state/label** → add a row to the constants table (and
  `/update-linear`'s copy).
- **Estimates enabled on the team** → add `estimate` to Step 6 and
  document the scale used.
- **Roadmap format changes** → keep Step 3's parser and the
  suggested-shape example in sync with the real file.
- **MCP tool renamed** → update the tool references in Steps 4 and 6.
