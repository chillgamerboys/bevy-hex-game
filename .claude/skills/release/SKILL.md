---
name: release
description: Cut a version — compute the next semver from Conventional-Commit subjects since the last tag, bump `[workspace.package] version` in the root Cargo.toml (plus Cargo.lock), generate a CHANGELOG.md section, land the bump through the normal PR gate, then tag `vX.Y.Z` on the promoted `main` commit and push it. The tag push triggers `.github/workflows/release.yaml`, which builds the four platform archives and attaches them to the GitHub release. Operator-run; a release is a deliberate aggregated event, never a per-PR action.
---

When invoked, follow these steps. A **release** stamps an immutable
`vX.Y.Z` tag on code that has been promoted to `main`. It is NOT a
per-PR action — individual feature merges do not cut versions (that's
why `/merge-pr` does not tag). The version is single-sourced:
`[workspace.package] version` in the root `Cargo.toml` is the source
of truth, all eleven crates inherit it via `version.workspace = true`,
and the tag must agree with it.

## Invocation

- `--bump <major|minor|patch>`: override the computed bump. Rare — the
  Conventional-Commit analysis (Step 2) is the default and should be
  trusted. Use when history mislabels a change, or to make the
  deliberate `0.x` → `1.0.0` jump.
- `--dry-run`: compute + print the next version, the bump reason, and
  the changelog body, then STOP before writing/committing/tagging.
  Always safe.
- `--tag-only`: skip the bump (the manifest already equals the target
  version) and just tag + push. This is the second half of the flow,
  run after the bump PR has merged and been promoted.

## Step 0 — Pre-flights (STOP on any failure)

1. **Clean tree.** `git status --porcelain` empty — the release commit
   must contain only the manifest + lockfile + changelog bump.
2. **Fetched and current.** `git fetch origin --tags`, and confirm the
   local ref for the branch you're about to work from is not behind
   its remote.
3. **Branch expectations by phase:**
   - *Bump phase* (default) starts from `dev` — that's where all work
     lands, and the bump is ordinary reviewed work.
   - *Tag phase* (`--tag-only`) requires the target commit to be on
     `main`. Verify with
     `git merge-base --is-ancestor <sha> origin/main`. STOP if the
     version has not been promoted — **`main` moves only by promoting
     `dev`**, and a tag on an unpromoted commit would claim a release
     nobody played.

## Step 1 — Resolve the last release

```bash
git fetch origin --tags
LAST_TAG=$(git describe --tags --match "v*" --abbrev=0 2>/dev/null || true)
```

- **A tag exists** → `LAST_VER=${LAST_TAG#v}` is the baseline; the
  commit range is `${LAST_TAG}..origin/main`.
- **No tag yet** (the current state — this repo has never released) →
  the baseline is the manifest version (`0.1.0`). It becomes the FIRST
  tag as-is if the range carries only chore/docs/test commits, or is
  bumped by the range (Step 2). The range is the full history. Report
  "first release — seeding from manifest `0.1.0`".

## Step 2 — Compute the next version (Conventional Commits → semver)

Collect the commit subjects in range and classify each by its
Conventional-Commit prefix:

```bash
git log --format='%h %s%n%b' "${LAST_TAG:+${LAST_TAG}..}origin/main"
```

- **MAJOR** — any commit whose type carries a `!` (`feat!:`, `fix!:`,
  `refactor!:`) OR whose body has a `BREAKING CHANGE:` footer.
- **MINOR** — else, any `feat:` / `feat(scope):`.
- **PATCH** — else (`fix:`, `perf:`, or a range of only
  chore/docs/test/refactor that still warrants a release).

Subjects that aren't Conventional-shaped count as **patch**. History
predating the convention is full of prose subjects, so on a first
release expect the classification to lean on whatever typed commits
exist; `--bump` is the escape hatch. Merge commits
(`Merge pull request #N from chillgamerboys/feat/...`) are ignored by
the classifier — the commits they bring in are already in the range.

Apply the bump to `LAST_VER`:

- **Pre-1.0 rule (`0.y.z`)** — the current state at `0.1.0`: while the
  major is `0` the API is not declared stable, so a **breaking**
  change bumps **MINOR** (`0.1.0` → `0.2.0`), not major; `feat` bumps
  MINOR too; `fix` bumps PATCH. Reserve the `0.x` → `1.0.0` jump for a
  deliberate `--bump major`.
- **≥1.0:** standard semver per the classification above.

Echo the decision before writing anything:
`release: <LAST_VER> → <NEXT_VER>  (<major|minor|patch> — <reason>, <N> commits)`.
If `--dry-run`, print the Step-3 changelog body too, then STOP.

## Step 3 — Write the manifest + lockfile + changelog

1. **Manifest** — root `Cargo.toml`, `[workspace.package]`:

   ```toml
   [workspace.package]
   version = "<NEXT_VER>"
   ```

   **One edit point.** All eleven crates inherit with
   `version.workspace = true` — never edit a per-crate `Cargo.toml`.

2. **Lockfile** — bumping the workspace version rewrites the eleven
   crate entries in `Cargo.lock`. Refresh and commit it:

   ```bash
   cargo check --workspace
   ```

   A release commit that bumps `Cargo.toml` without `Cargo.lock`
   leaves the tree dirty for the next person and can fail CI's
   `--locked` expectations.

3. **Changelog** — `CHANGELOG.md`: prepend a new section under the
   `# Changelog` header. Group the range's commits by type:

   ```markdown
   ## v<NEXT_VER> — <YYYY-MM-DD>

   ### Breaking
   - <subject> (<short-sha>)
   ### Features
   - <subject> (<short-sha>)
   ### Fixes
   - <subject> (<short-sha>)
   ```

   Omit empty groups. Strip the Conventional-Commit prefix for
   readability (`feat(units): add X` → `add X`). Enrich from the
   `## Changelog` bullets in the range's PR bodies where they say
   something the subject doesn't. Keep a trailing `(#PR)` when present
   — it's a useful backlink.

## Step 4 — Land the bump, then tag

The bump is a real change to tracked files, so it goes through the
normal gate. **Never commit it directly to `main`** — `main` moves
only by promotion.

**Phase 1 — bump PR (default invocation):**

```bash
git fetch origin dev
git checkout -b "chore/release-v<NEXT_VER>" origin/dev
# edit Cargo.toml, refresh Cargo.lock, write CHANGELOG.md
git add Cargo.toml Cargo.lock CHANGELOG.md
git commit -m "chore(release): v<NEXT_VER>"
```

Then run `/create-pr` → `/audit-pr` → `/merge-pr` on that branch
(base `dev`), exactly like any other change.

**Phase 2 — promote:** run `/promote` to open and merge the
`dev`→`main` PR. Its played-and-looked-at gate is the real release
gate — a human confirms the game runs before anything is tagged.

**Phase 3 — tag (`--tag-only`):** once the promotion merge is on
`main`:

```bash
git fetch origin main --tags
MERGE_SHA=$(git rev-parse origin/main)
git tag -a "v<NEXT_VER>" -m "bevy-hex-game v<NEXT_VER>" "$MERGE_SHA"
git push origin "v<NEXT_VER>"
```

Tagging the promoted merge commit is what keeps `Cargo.toml` and the
tag in agreement — the parity contract. Do not tag from `dev`.

**On the ceremony:** a release costs a dev PR plus a promotion here.
That's the price of "main only moves by promotion", and releases are
rare enough that it's the right trade. The alternative — tagging
`main` without a manifest bump — breaks parity.

## Step 5 — GitHub Release

```bash
gh release create "v<NEXT_VER>" \
    --title "bevy-hex-game v<NEXT_VER>" \
    --notes-file <(sed -n '/^## v<NEXT_VER>/,/^## /p' CHANGELOG.md | sed '$d')
```

**How this interacts with CI:** pushing the `v*` tag in Step 4 triggers
`.github/workflows/release.yaml`, which builds `hex_game` for
linux-x86_64, windows-x86_64, macos-aarch64 and macos-x86_64, archives
each with `assets/` and the README, and attaches them to the release
for this tag. Creating the release here (with real notes) means the
workflow attaches artifacts to a release that already has a changelog,
rather than an auto-created bare one. Either order works — the action
attaches to an existing release for the tag or creates one — but
notes-first reads better.

Skip on `--dry-run`.

## Report

```
| Step | Result |
|---|---|
| 0 pre-flights | ✓ clean, fetched, branch OK for phase |
| 1 last release | ✓ <LAST_TAG> / first release (seed 0.1.0) |
| 2 compute | ✓ <LAST_VER> → <NEXT_VER> (<bump> — <reason>) |
| 3 write | ✓ Cargo.toml + Cargo.lock + CHANGELOG.md |
| 4 land | ✓ bump PR opened (tag after promote) / ✓ tagged v<NEXT_VER> pushed |
| 5 gh release | ✓ created (workflow will attach 4 platform archives) / — skipped |
```

Echo `NEXT_VER` on its own line at the end.

## When NOT to invoke

- **On a feature merge.** Releases aggregate many merges; they are not
  per-PR. `/merge-pr` deliberately does not tag.
- **Before anyone has played the promoted build.** `/promote`'s visual
  gate is the release gate. CI cannot see a black sky.
- **To re-tag an existing version.** **Tags are immutable.** To fix a
  bad release, cut the next patch — never move a tag. The release
  workflow has already built artifacts against the old one.

## Self-updating

- **Per-crate versions diverge** (a crate gets published to crates.io
  on its own cadence) → this skill assumes one workspace version. It
  would need a `--package` scope and a `<pkg>-v` tag prefix; the
  compute machinery is unchanged, only the prefix + range scoping
  differ.
- **New Conventional-Commit type needs a bump rule** → update Step 2's
  classification, keeping the pre-1.0 rule intact.
- **`release.yaml` changes its trigger or artifact set** → update Step
  5's description so the two don't drift.
- **`1.0.0` is declared** → drop the pre-1.0 paragraph from Step 2 and
  note the date; from then on breaking means MAJOR.
