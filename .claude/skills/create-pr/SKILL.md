---
name: create-pr
description: Open a PR against `dev` (or a `wave/N-*` integration branch when the work belongs to a wave) — push branch if needed, derive title from first-commit subject, auto-populate this repo's PR template (What-and-why / Changes / Checks / Boundaries / Changelog with mechanical sections filled and prose TODOs marked), `gh pr create --base <dev-or-wave>`, then chain `/update-linear` to bind a Linear ticket if one applies. Binding is encouraged, never required. Pairs with `/audit-pr` (gate) and `/merge-pr` (finalize). Closes the front of the PR-lifecycle workflow.
---

When invoked, follow these steps. Pre-flight (Step 0) is hard: STOP
on any failure. Body generation (Step 3) is mechanical. The
`/update-linear` chain (Step 5) is best-effort — an untied PR is
legitimate here, so a bind failure is reported, not fatal.

**Resolve the base first**: `BASE=${BASE_ARG:-dev}` — a wave-bound PR
passes its `wave/N-*` branch in the invocation. Every `origin/dev`
in the steps below means `origin/${BASE}` (ahead-count, title
derivation, diff metrics all measure against the branch the PR will
actually target).

## Step 0 — Pre-flight (5 cheap checks, <2s combined)

1. **On a feature branch, not `dev`.**

   ```bash
   BRANCH=$(git symbolic-ref --short HEAD)
   ```

   STOP if `$BRANCH == "dev"` — `/create-pr` opens PRs from feature
   branches; you can't PR `dev` against itself. (For the `dev`→`main`
   promotion PR, use `/promote`.)

2. **Branch prefix (warn, don't STOP).** This repo uses `chore/`,
   `fix/`, `perf/`, `feat/`, `docs/`, `refactor/`:

   ```bash
   echo "$BRANCH" | grep -qE '^(chore|fix|perf|feat|docs|refactor)/' \
     || echo "⚠ branch '$BRANCH' doesn't use a standard prefix"
   ```

   A non-standard prefix is a nudge, not a blocker — plenty of
   existing branches predate the convention.

3. **At least one commit ahead of `origin/dev`.**

   ```bash
   AHEAD=$(git rev-list --count origin/dev..HEAD)
   ```

   STOP if `$AHEAD -eq 0` — nothing to PR. The operator should
   commit first, or rebase if commits landed upstream already.

4. **No uncommitted changes.**

   ```bash
   git status --porcelain
   ```

   STOP if non-empty — the diff in the PR wouldn't include those
   uncommitted edits, leaving them as silent state. Operator should
   `git stash` or commit before re-running.

5. **No existing PR for this branch.**

   ```bash
   gh pr view --json number 2>/dev/null
   ```

   STOP if a PR already exists — `/create-pr` is for opening PRs.
   To refresh the body of an existing PR, use
   `gh pr edit --body-file <file>`. To re-bind a Linear ticket, use
   `/update-linear`.

Each STOP returns a clear remediation. Combined check is <2s.

## Step 1 — Push branch if unpushed (or fast-forward push)

```bash
if ! git ls-remote --heads origin "$BRANCH" | grep -q .; then
    git push -u origin "$BRANCH"
elif [ "$(git rev-list --count "origin/$BRANCH..HEAD")" -gt 0 ]; then
    git push
fi
```

- **Branch absent on origin** → `git push -u` creates + tracks.
- **Branch present but local has new commits** → `git push` fast-forwards.
- **Branch present + synced** → no-op.

If push fails (auth issue, force-push needed, etc.), surface the
git error verbatim and STOP. The PR can't open against a branch the
remote can't see.

## Step 2 — Derive title from first-commit subject

```bash
TITLE=$(git log --format=%s "origin/dev..HEAD" | tail -1)
```

`tail -1` gives the **oldest** commit on the branch — typically the
load-bearing "thesis" commit; later commits are usually polish or
audit fixups.

Commit subjects follow **Conventional Commits** (`feat:`, `fix:`,
`chore:`, `docs:`, `refactor:`, `perf:`), because `/release` computes
the version bump from them. If the derived title isn't
Conventional-shaped, warn — the release classifier will treat it as a
patch.

**Length warning:** if `${#TITLE} > 70`, log a warning but proceed.
The operator can `gh pr edit --title "<better>"` afterward.

## Step 3 — Generate body from this repo's template + git introspection

Capture metrics:

```bash
FILES_COUNT=$(git diff --name-only origin/dev...HEAD | wc -l | tr -d ' ')
LINES_STAT=$(git diff --shortstat origin/dev...HEAD)
COMMITS_COUNT=$(git rev-list --count origin/dev..HEAD)
COMMIT_LIST=$(git log --format='- %s' "origin/dev..HEAD" --reverse)
```

Write `/tmp/pr-body-${BRANCH//\//-}.md`. **This reproduces
`.github/pull_request_template.md`** — `--body-file` suppresses
GitHub's auto-template, so the skill must supply it rather than fight
it. Keep the checklist items verbatim; they mirror every CI gate:

```markdown
## What and why

<!-- TODO: the diff says what changed. Say why. Operator writes this; the skill leaves this TODO marker so it can't be silently shipped. -->

## Changes

${COMMIT_LIST}

<details>
<summary>Sizing — ${FILES_COUNT} files, ${COMMITS_COUNT} commits</summary>

${LINES_STAT}

</details>

## Checks

- [ ] All relative links in tracked Markdown resolve
- [ ] `cargo fmt --all --check` (unless Markdown-only)
- [ ] `cargo deny check` (unless Markdown-only)
- [ ] `cargo clippy --workspace --all-targets --all-features --profile ci -- -D warnings` (unless Markdown-only)
- [ ] `cargo test --workspace --all-features --profile ci` (unless Markdown-only)
- [ ] `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --all-features` (unless Markdown-only)
- [ ] `cargo build --package hex_game --release` (unless Markdown-only; CI builds the shipping package on all three platforms)
- [ ] Automated visual walk green — `/visual-walk` captured the screens and the agent read every frame
- [ ] **A human ran the game and looked at it**

<!--
Those last two are different gates, not a formality and not each other's
substitute. Several failure modes here produce a clean log and a wrong window:
missing assets render a plain blue screen, a missed skybox event renders a black
sky, a wrong speed unit just looks slightly off, and a tile whose transform
disagrees with its span floats or sinks. All of them pass CI. The automated walk
(receipt key 5_visual_walk) catches the renders-nothing/renders-broken class and
lists layout findings; motion, feel, and taste still need human eyes — /promote
gates on the human box, never the automated one.

If the change touches rendering, movement, persistence, or state transitions, walk it:
splash -> title -> New Game -> Party Trial, orbit, move the party, ESC to pause,
save with F5, BACKSPACE to the title, Continue, then open Settings and persist one
change across restart. Launch an affected Map or focused Demo separately.
-->

## Boundaries

- [ ] I stayed inside my crate, **or** I have said below why a shared crate needed changing

<!--
`hex_core` and `hex_game` are shared. Changing them is fine and sometimes
necessary — it is worth a sentence so the people who depend on them know.
-->

## Changelog

<!-- TODO: one line, in the player's voice, for the release notes. Delete this section if the PR has no user-visible change (internal refactor, test-only, docs). -->

- 
```

The first eight Checks boxes are what `/audit-pr` verifies
mechanically — the operator can tick them once its receipt is green,
including the automated visual walk (receipt key `5_visual_walk`).
**The last box is the operator's alone**: stills are not play, and
motion, feel, and taste still need a human at the window.

**TODO-marker design:** the literal `TODO: ... the skill leaves this
TODO marker` text is intentionally obvious in the rendered PR. A
reviewer scanning the body sees it immediately; so does an operator
about to merge, via `gh pr view`.

**Changelog section design:** the bullet is the one-line release-notes
entry. `/release` compiles `CHANGELOG.md` primarily from the
**Conventional-Commit subjects** in the released range, and may enrich
from these per-PR bullets. Delete the section (with its TODO) for a
pure refactor / test-only / docs change.

No `## Linear` section yet — Step 5's `/update-linear` appends it if a
tie is made.

## Step 4 — `gh pr create`

```bash
BASE="${BASE_ARG:-dev}"   # `--base wave/N-*` when the work belongs to a wave
gh pr create \
    --base "$BASE" \
    --title "$TITLE" \
    --body-file "/tmp/pr-body-${BRANCH//\//-}.md"
```

Capture the returned PR number and URL:

```bash
PR_NUM=$(gh pr view --json number --jq .number)
PR_URL=$(gh pr view --json url --jq .url)
```

The explicit `--base` is load-bearing. **Everything lands on `dev`;
nothing is merged straight to `main`** — but gameplay-ticket work
grouped into a wave targets its `wave/N-*` integration branch, and the
wave reaches `dev` in one walked merge (see CONTRIBUTING.md's wave
section). Accept a base via the invocation ("create-pr onto
wave/2-command-flow"); default `dev`. An explicit base also defends
against GitHub picking a parent feature branch as the default when the
local branch was forked from one. Valid bases are `dev` and `wave/*`
only — anything else, STOP and ask.

If `gh pr create` fails (auth, no-changes-to-PR, branch protection),
surface verbatim and STOP. The branch is pushed but no PR is open —
the operator can investigate.

## Step 5 — Invoke `/update-linear` (best-effort bind)

```
Run /update-linear (no --state flag → bind mode).
```

`/update-linear` follows its own bind flow: branch-name `HEX-\d+`
first, then a prompt-pick from open HEX tickets, then create-new — and
crucially, **"leave untied" is a first-class answer**. This repo's tie
is soft: chore and fix work often doesn't earn a ticket.

On success it appends `## Linear / HEX-N` to the PR body and attaches
the PR URL to the ticket.

On **failure** (Linear MCP outage, network blip), report and continue
— do not STOP:

```
✓ PR #${PR_NUM} opened: ${PR_URL}
⚠ /update-linear failed: <error>

  PR is open and untied. Run `/update-linear` later if this work
  should appear on the board. /audit-pr will note the missing tie
  as a warning, not a block.
```

Never close the PR over a bind failure.

## Step 6 — Combined report

```
✓ /create-pr complete

  PR:     #${PR_NUM} — ${TITLE}
  URL:    ${PR_URL}
  Linear: ${LINEAR_REF} — ${LINEAR_URL}   (or "untied")
  Title:  derived from first commit (${COMMITS_COUNT} commits on branch)

Next: /audit-pr when ready to gate for merge.

Reminder: fill the `<!-- TODO -->` markers (What-and-why, Changelog)
and run the game before ticking the last Checks box.
```

## Troubleshooting

**Branch has no commits ahead of `origin/dev`:** verify via
`git log origin/dev..HEAD`. If the branch was based on a feature that
since merged, rebase: `git rebase origin/dev`.

**PR opened against `main` by mistake** (manual `gh pr create` without
`--base`): retarget rather than merging —
`gh pr edit <N> --base dev`.

**Existing PR for this branch:** `/create-pr` won't overwrite. Refresh
the body with `gh pr edit --body-file <file>`; re-bind with
`/update-linear --force-rebind`.

**Title >70 chars:** the skill warns but proceeds. `gh pr edit
--title "<better>"` post-create.

**`gh pr create` exits non-zero with "must first push":** Step 1
should have caught this. Verify with
`git ls-remote --heads origin "$BRANCH"`; GitHub's index may need a
few seconds.

**Linear MCP not loaded:** Step 5 reports the miss and continues. Bind
later from a session that has it.

**Conductor workspace:** the branch is checked out in this worktree
while `dev` is held by the parent. That's expected — nothing here
needs to check out `dev`.

## When NOT to invoke

- **Pre-existing PR you want to refresh** — use `gh pr edit` or
  `/update-linear --force-rebind`.
- **The `dev`→`main` promotion** — that's `/promote`, which opens the
  PR with `--base main --head dev` and gates on the visual walk.
- **Stacked PRs** — this skill assumes a flat branch off `dev`.

## Self-updating

- **`.github/pull_request_template.md` changes** → mirror it in Step
  3's body template. The two must not drift: `--body-file` means the
  skill's copy is what ships.
- **CI's check list changes** (`.github/workflows/ci.yaml`) → update
  the Checks block, and the `test-*` skills that run those commands.
- **Branch prefixes change** → update Step 0.2's regex.
- **`gh pr create` flags evolve** (e.g. `--draft` becomes useful) →
  expand Step 4.
- **TODO-marker text triggers false positives downstream** → revise
  the literal string; keep it HTML-comment-shaped (invisible in
  rendered Markdown, greppable in source).
