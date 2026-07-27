---
name: promote
description: Open and finalize the deliberate `dev`→`main` promotion PR. Runs light pre-flights (clean tree, pushed, non-empty range), then the load-bearing gate — explicit operator confirmation that a human ran the game from `dev` and looked at it — then creates the PR with a per-feature body and finalizes via `/merge-pr` (merge-commit, `dev` never deleted). Single hop: every promotion is terminal. Pairs with `/release`, which tags the promoted commit.
---

When invoked, follow these steps. A **promotion** advances `dev` to
`main`. Unlike a feature PR (one change, into `dev`), a promotion PR
bundles **many** merged features and lands as a merge commit so their
individual history survives.

**Why this skill exists at all.** `main` moves only by merging `dev`
into it, as a deliberate act once the work has been played and looked
at. That gap is the point: **CI cannot see a black sky, a gap between
tiles, or a piece sunk into the terrain**, and every serious bug in
this codebase so far was found by a person clicking. `dev` is where
things are allowed to be wrong. This skill is the checkpoint where
that stops being true.

There is no deploy step — the artifact-building release workflow fires
from a `v*` tag, which is `/release`'s job, not this one's.

## Step 0 — The chain is fixed

The chain is exactly **`dev` → `main`**. There is no staging branch and
nothing to infer.

- `--source`: defaults to `dev`; any other value → **STOP**.
- `--target`: defaults to `main`; any other value → **STOP**.

Every promotion is **terminal** by definition.

Echo the plan before doing anything: `promote: dev → main (TERMINAL)`.

If the operator wants some other branch pair, they want `/create-pr`
(feature work into `dev`) — say so and stop.

## Step 1 — Pre-flights (STOP on any failure)

1. **Clean tree.** `git status --porcelain` empty — STOP otherwise (a
   promotion must reflect exactly what's on `dev`).
2. **`dev` is pushed.** `git fetch origin dev`, then confirm
   `origin/dev` has no commits missing from the local `dev` ref you're
   promoting. STOP if the remote is behind — the promotion PR diffs
   the *remote* branches, so unpushed work simply wouldn't ship.
3. **Non-empty promotion.**

   ```bash
   git fetch origin main dev
   git log --oneline "origin/main..origin/dev"
   ```

   If empty → STOP "`dev` has no commits ahead of `main`; nothing to
   promote."
4. **Back-merge warning (not a STOP).** If `origin/main` has commits
   `origin/dev` lacks (`git log origin/dev..origin/main`), warn: the
   branches diverged, so the PR may surface unexpected diffs or
   conflicts. Proceed only if the operator confirms. (This normally
   means someone committed to `main` directly — worth understanding
   before promoting over it.)

## Step 1.5 — The played-and-looked-at gate (hard)

**This is the gate. Do not skip it, and do not infer it from a green
receipt — including a green `5_visual_walk`.** The agent's scripted
walk photographs the screens and catches the renders-broken class, but
stills are not play; this gate is about a human having *played* it.
Before asking, pre-fill the operator's context: if a recent
`/visual-walk` ran, surface its PNG paths and any review-tier findings
so the human walk can start where the agent's eyes stopped.

Ask the operator explicitly (via `AskUserQuestion`) whether a human has
run the game from this `dev` and walked it:

```
cargo dev
splash → title → click a scenario → gameplay
orbit (right-drag), WASD pan, zoom
click a tile — the piece walks there; range tint + route preview draw
click the tile you're standing on
ESC pause and resume
BACKSPACE to the title, then click a scenario to rebuild the world
```

Also worth a look, because these fail silently: the sky is not black
and clouds aren't smeared (check from the *gameplay* camera, which
looks down); tiles have no hairline gaps; no piece is floating or sunk;
movement speed looks right.

- **Confirmed** → proceed to Step 2. Record who confirmed and what they
  saw; it goes in the PR body.
- **Not confirmed / unsure** → **STOP**:

  ```
  ✗ Promotion blocked: nobody has confirmed playing this dev build.

    main only moves once the work has been played and looked at.
    Run `cargo dev`, walk the flow above, then re-invoke /promote.
  ```

A promotion is cheap to redo and expensive to get wrong — `main` is
what a release tags.

## Step 2 — Create the promotion PR

```bash
COMMITS=$(git log --format='- %s' "origin/main..origin/dev" --reverse)
N=$(git rev-list --count "origin/main..origin/dev")
cat > /tmp/promote-body.md <<EOF
## promote: dev → main ($N commits)

Advancing **dev** to **main**.

### Included
$COMMITS

### Played and looked at
<who> walked splash → title → scenario → gameplay: orbit, tile click,
route preview, pause/resume, BACKSPACE rebuild. Sky, tile seams and
piece placement checked.
EOF

gh pr create --base main --head dev \
    --title "promote: dev → main ($N commits)" \
    --body-file /tmp/promote-body.md
```

Merge-commit subjects are kept verbatim in the body — they name the
feature branches, which is the most readable summary of what's
shipping.

A promotion PR is **not** bound to a single Linear ticket: it carries
many features, each already at `Done` from its own merge into `dev`.
Do not run `/update-linear` bind here.

## Step 3 — Finalize via `/merge-pr`

Hand the merge over:

```
/merge-pr
```

`/merge-pr` classifies base `main` + head `dev` as a **promotion
merge** and therefore:

- merges with `gh pr merge --merge` (merge commit, preserving per-PR
  history), and
- **does NOT pass `--delete-branch`** — **`dev` is permanent** and must
  never be deleted, and
- **skips** the single-ticket Linear state-sync (tickets reached `Done`
  when their PRs landed on `dev`; promotion touches no tickets).

The receipt gate still applies: `/merge-pr` needs a green
`/audit-pr` receipt for the promotion PR's HEAD. Run `/audit-pr` on it
first — on a promotion the diff is usually large but already-audited
per-PR, so the run is mostly confirmation that `dev` is coherent as a
whole.

## Step 4 — Release pointer (never automatic)

Changelog-worthy work has just reached `main`. If a version should be
cut, that is `/release` — a separate, deliberate act:

```
/release
```

It computes the next semver from the Conventional-Commit subjects,
lands the `Cargo.toml` + `Cargo.lock` + `CHANGELOG.md` bump through the
normal `dev` PR gate, and tags the promoted `main` commit — which
triggers the four-platform build workflow.

**Print this as a pointer; never auto-run it.** Not every promotion is
a release.

## Report

```
| Step | Result |
|---|---|
| 0 resolve | ✓ dev → main (TERMINAL) |
| 1 pre-flights | ✓ clean tree, dev pushed, <N> commits to promote |
| 1.5 played-and-looked-at | ✓ confirmed by <who> / ✗ STOP |
| 2 create PR | ✓ PR #<N> opened |
| 3 merge | ✓ merge-commit <sha> → main (dev intact) |
| 4 release | — pointer only: run /release to cut vX.Y.Z |
```

## When NOT to invoke

- **Feature work.** That's `/create-pr` into `dev`.
- **Nobody has played the build.** Step 1.5 will stop you; save the
  round trip.
- **To tag a version.** That's `/release`. This skill never tags.

## Self-updating

- **A staging branch appears** → the chain stops being a single hop.
  Restore the seed's source→target inference and the
  intermediate-vs-terminal split, and note that only the terminal hop
  carries the visual gate.
- **The visual walk changes** (new screens, new controls) → update Step
  1.5's script and keep it in sync with
  `.github/pull_request_template.md`, which carries the same walk.
- **A deploy step appears** (a published build, a server) → add a
  verify gate after Step 3 and STOP the release pointer on a red
  verify.
