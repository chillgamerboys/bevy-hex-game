# Docs restructure — implementation spec

A handoff document. It describes one change: reorganising `docs/` from a
flat pile into a kind-separated tree, and updating the things that point
into it. **Delete this file as the last step of the work it describes.**

## Why

`docs/` is six flat UPPERCASE files with no index. Four problems, in the
order they bite:

1. **No orientation layer.** Nothing says who each doc is for or what to
   read first. A designer, a new contributor and an AI agent each need a
   different two of the six, and none of them can tell which two.
2. **Audiences are mixed.** Game design, code contracts, a designer's
   how-to guide and a status report sit as identical-looking siblings.
3. **Stable contracts are mixed with drift-prone status.** "What's built
   vs. placeholder" lives in `GAMEPLAY_LOOP.md` *and* `DESIGN.md`'s tail
   *and* `CLAUDE.md`'s Current state. Three copies, rotting at three
   different rates. This is the one that costs real time, and it is why
   `/update-docs` currently has only one anchor it can trust.
4. **Names don't say what's inside.** `MAP_MODEL.md` is the voxel
   contract. `CONTENT.md` is a RON tuning manual. `GAMEPLAY_LOOP.md` vs
   `DESIGN.md` doesn't tell you which is reality and which is intent.

Two files also mix *kinds* internally: `ARCHITECTURE.md` is a crate-graph
reference with a 100-line sky-shader explainer and a troubleshooting table
inside it; `CONTENT.md` is a tutorial with its own troubleshooting section.

## Hard precondition — do not start until PR #52 has merged

PR #52 ("Add deterministic procedural map pipeline") edits **five of the
files this change moves**: `CLAUDE.md`, `crates/hex_map/CLAUDE.md`,
`docs/ARCHITECTURE.md`, `docs/CONTENT.md`, `docs/MAP_MODEL.md`.

Moving them first would hand #52 a conflict in every one. Wait for it to
land on `dev`, branch from `dev` afterwards, and re-read those five files
before splitting them — #52 may have added sections this spec doesn't
mention.

## Target tree

```
docs/
  README.md              NEW — the index. Every doc: audience, purpose,
                         who keeps it fresh. Doubles as /update-docs' map.
  architecture.md        from ARCHITECTURE.md — code organisation ONLY
  systems/               one doc per built system: model, rules, contract
    map.md               from MAP_MODEL.md (content unchanged)
    combat.md            GAMEPLAY_LOOP.md's stable half
    sky.md               extracted from ARCHITECTURE.md
  design/
    game.md              from DESIGN.md, minus its status tail
  development/
    onboarding.md        from ONBOARDING.md
    config.md            from CONTENT.md, minus its troubleshooting
    troubleshooting.md   NEW — the single source for failure modes
  planning/
    status.md            NEW — the one doc allowed to drift
    docs-restructure.md  this file; delete when done
    audit-log.md         already exists — DO NOT MOVE
    roadmap.md           not created here; /seed-tickets expects this path
CHANGELOG.md             stays at root
CONTRIBUTING.md          stays at root
```

**Why `systems/`.** "Map" is not a peer of "architecture" — it is a peer
of combat, spells, lattices, movement. Each built system gets a doc there
when it lands; `sky.md` proves the directory holds presentation systems
too. Design intent lives in `design/`, the built thing in `systems/`, and
the gap between them in `planning/status.md`.

**Naming:** lowercase-kebab throughout. Keeping UPPERCASE would have been
equally defensible; mixing the two is the only wrong answer.

## Moves (pure `git mv`, no content change)

```sh
git mv docs/MAP_MODEL.md  docs/systems/map.md
git mv docs/ONBOARDING.md docs/development/onboarding.md
git mv docs/CONTENT.md    docs/development/config.md
git mv docs/DESIGN.md     docs/design/game.md
git mv docs/ARCHITECTURE.md docs/architecture.md
```

Do these as their own commit, before any editing. `git mv` then edit in a
later commit keeps `git log --follow` working; a delete-plus-create loses
the history of files that carry a lot of hard-won reasoning.

`GAMEPLAY_LOOP.md` has no single destination — it is split below.

## Splits

This is the part that needs judgment. **Preserve prose verbatim** wherever
it moves — this is a reorganisation, not a rewrite. Where a section leaves
a doc, leave a one-line pointer to where it went if the surrounding text
depended on it.

### `GAMEPLAY_LOOP.md` → `systems/combat.md` + `planning/status.md`

| Section | Goes to |
|---|---|
| Two modes, one map | combat.md |
| A turn | combat.md |
| Saying no out loud | combat.md |
| **What is provisional** | **status.md** |
| **Why there is no damage** | **status.md** |
| Vocabulary (lattice, not core) | combat.md |
| Where it lives | combat.md |
| Trying it out | combat.md |
| Where a unit is, and where it is going | combat.md |
| The high ground | combat.md |
| **Not built, and not next** | **status.md** |

### `ARCHITECTURE.md` → keeps most; two sections leave

| Section | Goes to |
|---|---|
| The sky is a shader on a camera-following dome | **systems/sky.md** |
| Things that fail silently | **development/troubleshooting.md** |
| Not yet done | **planning/status.md** |

Everything else — crate graph, ownership, positions-are-voxels,
conventions, states, settings, hot reload, testing philosophy — stays and
becomes the whole of `architecture.md`.

### `DESIGN.md` → `design/game.md`

Move `## What exists in code today` (the final section) to
`planning/status.md`. Everything above it is the design target and stays.

### `CONTENT.md` → `development/config.md`

Move `## If something goes wrong` to `development/troubleshooting.md`.

## New files

### `docs/development/troubleshooting.md`

**The single source of truth for failure modes** — merge and deduplicate
three existing near-copies:

- `CLAUDE.md`'s Traps table
- `ARCHITECTURE.md`'s "Things that fail silently"
- `CONTENT.md`'s "If something goes wrong"

The first two overlap heavily and have already drifted apart; reconcile
them rather than concatenating. Keep the framing that earns its place:
**several failure modes here produce no log output at all, so a clean log
is not evidence — look at the window.**

`CLAUDE.md` then keeps a ~6-line short version plus a link here. It is
auto-loaded into every agent session, so it should keep the *sharpest*
symptoms inline (plain blue window, black sky, appears frozen) and defer
the long tail.

### `docs/planning/status.md`

The consolidation target, and the point of the whole exercise. Assembled
from the four sources above plus the current-state facts in `CLAUDE.md`.
Open it with a line saying what it is: *the doc that is allowed to be out
of date, and the only one — everything else describes contracts.*

### `docs/README.md`

The index. One row per doc: path, audience, purpose, and who keeps it
fresh. `/update-docs` will check that this table matches the files
actually present, so keep it complete.

## Skills that must be updated

The skills in `.claude/skills/` have doc paths **baked into their text**
(this repo hand-ported them; there is no config to re-render). Moving a
file without updating these leaves a skill quietly pointing at nothing.

### `plan-ticket/SKILL.md` — the big one

Step 3.1 ("Docs first") names six paths, all of which move:

| Current | New |
|---|---|
| `docs/ARCHITECTURE.md` | `docs/architecture.md` |
| `docs/MAP_MODEL.md` | `docs/systems/map.md` |
| `docs/GAMEPLAY_LOOP.md` | `docs/systems/combat.md` |
| `docs/DESIGN.md` | `docs/design/game.md` |
| `docs/CONTENT.md` | `docs/development/config.md` |
| `crates/hex_map/CLAUDE.md` | unchanged |

Step 2 also references `docs/ARCHITECTURE.md`, and Step 4's table
references `docs/DESIGN.md`. The skill's own "Self-updating" section
already flags this restructure as a thing to fix — that note comes out
once it is done.

### `update-docs/SKILL.md` — activate the two dormant anchors

It ships with one live anchor (`CLAUDE.md`'s test count) and two marked
*"activates after the docs restructure"*. Turn them on:

- `docs/planning/status.md` — verify its claims against reality each run.
- `docs/README.md` — index rows must match the files present.

Then add the drift check the Documentation Map promises: every tracked
file under `docs/` has a row in `docs/README.md`.

### Leave alone

- `audit-diff/SKILL.md` → `docs/planning/audit-log.md` **already final.**
- `seed-tickets/SKILL.md` → `docs/planning/roadmap.md` **already final.**
- `merge-pr`, `audit-pr`, `test-local` → reference `CLAUDE.md` /
  `CONTRIBUTING.md`, which do not move.

## Other pointers to update

- **`CONTRIBUTING.md`** — links into `docs/`. Content otherwise untouched.
- **`README.md`** — its Documentation section lists the old tree.
- **`crates/hex_map/CLAUDE.md`** — links to `MAP_MODEL.md` and others.
- **`.github/pull_request_template.md`** — check for doc links.
- **Cross-references between the docs themselves** — these are dense.
  `ONBOARDING.md` alone has a five-row "where to go next" table.

## Verification

1. **The link checker is the main tool.** Run the loop from
   `.github/workflows/ci.yaml` (or `/test-local`, which runs it verbatim)
   until it reports `all relative links resolve`.

2. **⚠ It does NOT check anchors.** The loop strips `#fragment` before
   testing existence, so a link to a heading that no longer exists passes
   silently. Verified: a link to `architecture.md` plus the fragment
   `#does-not-exist`, piped through the checker's `sed`, comes out as bare
   `architecture.md` — the fragment is never tested.

   (Writing that example as a literal Markdown link here would itself fail
   the checker, since the extraction regex does not care that it sits in a
   code span. That is worth knowing before adding link examples to any doc.)

   These two cross-file anchor links **both break in this change** and
   must be fixed by hand — each points at a section that is moving to a
   different file:

   | Link, currently at | Points at | Fix |
   |---|---|---|
   | `docs/ARCHITECTURE.md:59` | `GAMEPLAY_LOOP.md#the-high-ground` | → `systems/combat.md#the-high-ground` |
   | `docs/GAMEPLAY_LOOP.md:178` | `ARCHITECTURE.md#ownership-cuts-both-ways` | → `../architecture.md#ownership-cuts-both-ways` (from `systems/combat.md`) |

   Note the second gains a `../` — most cross-references do, since docs
   move down a directory level. That is the likeliest source of breakage
   in the whole change, and the checker *will* catch those (they're path
   errors, not anchor errors).

   Also `docs/DESIGN.md:8` has a same-file `](#open-questions)`; it stays
   valid, since that heading travels with it into `design/game.md`.

3. **Nothing stranded:**
   ```sh
   grep -rnE 'MAP_MODEL|GAMEPLAY_LOOP|CONTENT\.md|ONBOARDING|ARCHITECTURE\.md|DESIGN\.md' \
     --include='*.md' --include='*.rs' --include='*.yaml' --include='*.toml' .
   ```
   Expect zero hits outside this file's own history.

4. **History survived:** `git log --follow docs/systems/map.md` should
   show the pre-move commits.

5. **`/update-docs`** — run it; all three anchors should be live and green.

6. **`/test-local`** — fmt, clippy, tests, deny, doc build, links. No Rust
   changes here, so this is mostly the link check, but run it anyway.

The docs are prose about a game nobody can see from CI, so also **read the
result**. An index that lies, or a split that leaves a section dangling
mid-argument, passes every check above.

## Landing it

Branch off `dev` after #52 lands. Suggested commits:

1. `docs: move files into the kind-separated tree` — pure `git mv`
2. `docs: split status and troubleshooting out of the stable docs`
3. `docs: add the index and status doc`
4. `docs: repoint skills and cross-references at the new tree`
5. `docs: delete the restructure spec` (this file)

Then `/create-pr` → `/audit-pr` → `/merge-pr`. It is a doc-only diff, so
`/test-full` short-circuits to `/test-quick` and the audit is quick.

Conventional Commit subjects — `/release` computes the version bump from
them.
