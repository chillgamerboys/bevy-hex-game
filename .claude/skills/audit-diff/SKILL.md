---
name: audit-diff
description: Correctness walk on the current diff. Standalone — runnable on its own AND used as Step 1 of /audit-pr. Code diffs get the 8 bug-trained lenses (silent failures, DRY, edge values, round-trip consistency, compiles-but-wrong API use, Commands/ordering contracts, test-altitude, feature/config traceability); diffs with no Rust take the cheap doc-only path (4 docs lenses: automation contracts, links+fragments, claims-vs-reality, single-source) with one subagent. Both paths end in the fresh-eyes pass and a mandatory Wave entry. Findings-only — does not auto-fix.
---

When invoked, follow these steps. The goal is to catch correctness
bugs **before** the human reviewer does, so they can spend their
attention on design rather than typos and silent swallowing.

## Workflow

1. **Read the diff scope.**
   - Resolve the diff base FIRST: `BASE=$(gh pr view --json
     baseRefName -q .baseRefName 2>/dev/null || echo dev)` — a wave
     PR audits against its wave branch, not `dev`. Every
     `origin/dev...HEAD` below means `origin/$BASE...HEAD`.
   - `git diff origin/$BASE...HEAD --stat` — file-level overview.
   - `gh pr view --json number,title,body 2>/dev/null` — stated scope.
     If no PR exists yet, `git log --oneline origin/dev...HEAD`.
   - For each changed file, read
     `git diff origin/dev...HEAD -- <file>` so you have the
     actual hunks, not summaries.

   **If the diff contains no Rust** (`git diff --name-only
   origin/dev...HEAD | grep '\.rs$'` is empty), take the **doc-only
   path**: skip straight to the docs lenses (see "Doc-only diffs"
   below) with ONE subagent, then the fresh-eyes pass, then the Wave
   entry. Five of the eight code lenses can only ever report "clean —
   no code" on such a diff; walking them is ceremony, not review.

2. **Spawn one Explore subagent per major surface** (max 3 parallel).
   Each gets:
   - The diffs for files in their surface.
   - The lens checklist below.
   - Instructions to report findings as a list, classified
     SHIP-BLOCKER vs NON-BLOCKER, with file:line references.

   Subagents act as fresh-eyes reviewers — they don't share the
   implementer's context, so they catch the same bugs the human
   reviewer does. Surfaces split per crate ownership (e.g., map /
   gameplay / presentation / binary+config); use the diff's file
   distribution to decide.

3. **Run the verification stack read-only** (don't fix anything yet):
   - `cargo fmt --all --check`
   - `/test-quick` (fmt + clippy + workspace tests)

   When called from `/audit-pr`, this verification is supplied by
   `/test-full` as Step 2; you can skip the manual runs.

4. **Fresh-eyes finalization pass.** After the per-lens subagents
   return findings (and before triage), spawn ONE more Explore
   subagent with this prompt:

   > The lens audit on PR #N reported the following findings:
   > {summarize}. The diff is attached below. Read the diff fresh —
   > don't redo the lens checks. Look for one bug class the lenses
   > didn't surface. The training set is retrospective; new bugs by
   > definition are unseen variants. Report `file:line — code
   > excerpt — description — severity` if you find one, else
   > "looked, found nothing."

   The under-specification is the point — the open-ended framing is
   what catches blind spots in the lens definitions. If this pass
   surfaces a class twice across PRs, it earns a 9th lens.

5. **Triage the findings — fix-by-default:**
   - **SHIP-BLOCKERS**: any finding that matches a lens below. Fix
     in a follow-up commit before declaring done.
   - **NON-BLOCKERS**: fix them too, unless the cost is
     disproportionate to the value. Only defer when the fix needs
     design discussion the audit can't unilaterally resolve — and
     remember the ownership rule: a *design* question inside someone
     else's crate is the owner's call, not the audit's. Write the
     *reason* into the deferral.

6. **Append a Wave entry to `docs/planning/audit-log.md`** (mandatory; runs on green AND failed audits).

   Read `docs/planning/audit-log.md` and locate the anchor comment:

   ```
   <!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->
   ```

   Find the highest existing `## Wave N` number; increment to determine `N` for the new entry. If `audit-log.md` doesn't exist, create it with this skeleton:

   ```markdown
   # Audit log

   Wave entries appended by `/audit-diff` — one per audited PR, the
   durable trail of which lenses fired and what was fixed or deferred.

   <!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->
   ```

   Append below the anchor:

   ```markdown
   ## Wave <N> — <PR title> (<YYYY-MM-DD>)

   - **PR**: #<num> — <branch>
   - **Outcome**: green | failed
   - **Lenses triggered**: <comma-list of lens numbers that surfaced findings>

   | Lens | File:line | Severity | Status |
   |---|---|---|---|
   | <N> | <file>:<line> | SHIP-BLOCKER \| NON-BLOCKER | fixed in <sha> \| deferred — <reason> |

   **Notes**: <one-line rationale for any deferred non-blockers, or empty>
   ```

   **Idempotency**: re-running `/audit-diff` on the same `(PR number, branch)` pair updates the existing Wave entry rather than creating a new one. Match by the first line (`## Wave N — <PR title>`); if found, replace the block down to the next `## Wave` heading.

   The Wave log is the durable audit trail — the receipt at `/tmp/audit-pr-receipt-<N>.json` is ephemeral (lost on reboot), but `docs/planning/audit-log.md` is committed to the repo and travels with the PR. Reviewers can trace which lenses fired across the PR's life.

   **Commit the Wave entry** as part of the audit fix-up commit when SHIP-BLOCKERs were fixed, or as a standalone commit (`chore(audit): wave <N> — <one-line>`) when the audit was green.

7. **Report status** in the conversation. When called standalone:

   ```
   audit-diff — PR #N
   Lenses checked: 8/8
   SHIP-BLOCKERS: M (all fixed in commit <sha>)
   NON-BLOCKERS fixed: K_fixed
   NON-BLOCKERS deferred: K_deferred (with rationale)
   Wave entry: docs/planning/audit-log.md Wave <N>
   Verification: fmt clean, clippy clean, tests pass
   ```

   When called from `/audit-pr`, return findings shaped for the v3
   receipt: `{file, line, lens, message, severity}` per finding.

## The lenses

Each lens catches a specific bug class. Run them all on every audit;
track which fired in the receipt's summary. Reference-bug examples
illustrate the bug shape — add this repo's own as new bug classes are
discovered.

### Lens 1 — Silent failures

For every new or modified error path in changed code, ask: would
this hide a failure that should be loud?

| Anti-pattern | Concern |
|---|---|
| `.unwrap_or_default()` / `.unwrap_or(...)` on a `Result` from parsing, IO, or asset loading | A bad file silently becomes a default — the settings pipeline exists to prevent exactly this |
| `Err(_) => {}` / `Err(_) => continue` arms | Error dropped with no record; `map_err_ignore` is denied, so these survived clippy by shape, not by intent |
| Bare `.ok();` statement discarding a `Result` | Same drop, different spelling |
| `if let Ok(...)` with no `else` on a path that must not silently skip | Vacuous pass when the resource is genuinely missing |
| Log-and-continue in a *startup/initial-load* path | Allowed only for hot reload (retain last valid value and report — the documented contract); startup must stall loudly |
| Test asserts only "did not panic" instead of the corrected behavior | Test passes even when the fix is wrong |

**Sibling sweep:** when one of these fires, grep the same pattern
across the diff. Implementers add the same defensive shape
consistently — if one is hiding a bug, the others probably are too.

**See also `/audit-silent-failures`** — a standalone grep harness
for the broader 7-pattern set, including pre-existing instances in
unmodified files within the same module.

### Lens 2 — DRY / single source of truth

For every literal constant introduced in the diff (string, number,
path, RON field name, magic geometry value), grep the repo. If the
same value already exists elsewhere, flag the duplication.

**Sub-check (fires when a dependency file is in the diff):** if
`Cargo.toml` (workspace or crate) is in the diff, also verify:
- New deps are declared in `[workspace.dependencies]` and inherited
  with `workspace = true`, not pinned separately per crate.
- Declared deps appear in `use` statements in the diff (declared ≠
  imported is a smell).
- The dep doesn't gate a future Bevy upgrade (the hexx rule: no Bevy
  features, pin only glam).

Reference bug shape: a constant hardcoded in a handler that was
already defined elsewhere; when the constant changed, half the
codebase saw the new value and half didn't. The repo's standing
example: hex geometry constants must match `hex.glb` — duplicating
them anywhere breaks the "change it safely" contract.

### Lens 3 — Edge values

For every new helper / parser / rule added in the diff, enumerate
the input space:
- **Hex/voxel:** level 0 (bedrock), negative cube coordinates, the
  origin, coordinates that don't sum to zero, an empty `HexSpan`,
  headroom 0 (buried), headroom exactly `levels_tall`.
- **Numbers:** 0, negative, very large, NaN in float terrain values.
- **Strings/RON:** empty, missing field, unknown substance name.
- **Time:** zero duration, first frame (`Time` delta 0).
- **Collections:** empty route, single-tile route, zero-length line.

"Don't ship code where the test set covers only the happy path."

### Lens 4 — Round-trip consistency

When a value becomes configurable (RON field, cargo feature, env
var), every place it surfaces must read from the same source. Flag
hardcoded defaults that don't match the configured value.

Reference bug shape: a value made configurable while one consumer
kept the old hardcoded default — the two silently diverge the first
time someone edits the file. Watch especially for values read both
every-frame and at-spawn (the hot-reload table in the architecture
doc): a new consumer must land in the right row.

### Lens 5 — Compiles-but-wrong API use

The compiler already catches nonexistent paths — this lens hunts
code that type-checks and is still wrong. For every new Bevy/hexx
API use in the diff, check against the known trap list:

| Pattern | Check |
|---|---|
| Reading events | `MessageReader<T>`, never `EventReader`; `AssetEvent<T>` is a `Message` — `add_observer` on it compiles into a dead observer |
| `Pointer<Click>` | Still an `Event` for observers — the one exception |
| Ambient light | `GlobalAmbientLight` resource, not `AmbientLight` (a per-camera component in 0.19) |
| `StandardMaterial::from(Color)` vs struct literal | `from` infers `AlphaMode::Blend` when alpha < 1; a struct literal leaves `Opaque` and silently discards the alpha |
| Cursor deltas | `CursorMoved`, never `MouseMotion` (Wayland/WSLg drops it while a button is held) |
| `Assets::get_mut` | Returns an `AssetMut` wrapper — bindings need `mut`, and no reads inside the mutably-borrowing call's argument list |
| hexx `a_star` / `field_of_movement` | Key on `Hex` alone — they collapse stacked surfaces. Pathfinding must go through `hex_units::movement::Reach` over `TilePos` |
| Any map keyed by `HexCoord` carrying height/surface data | The forbidden collapse — a `HashMap<HexCoord, _>` that keeps one surface per coordinate makes every lower surface unreachable |

**Sibling sweep:** when one of these fires, grep the diff for other
uses of the same API — the implementer usually misuses it
consistently across call sites.

### Lens 6 — Deferred-commands and ordering contracts

For every new system, observer, or spawn path in the diff, ask:

- (a) Does anything query entities spawned via `Commands` in the
  same schedule pass? Spawns are not queryable until the queue
  applies — the reader needs a system-set boundary (`GameplaySetup`:
  `Resources → Terrain → Actors`), not just `.after()`.
- (b) Does ordering cross a crate boundary? It must go through a
  shared set in `hex_core` — `.chain()` cannot express it, and a
  local chain that looks correct will race.
- (c) Is a new observer state-safe? Observers are global and fire in
  every state; one touching a gameplay-only resource must take
  `Option<Res<T>>` — Bevy validates parameters before the body runs,
  so an internal guard won't save it.
- (d) Does gameplay work opt into `PausableSystems` where it should
  freeze under `Pause`?

Reference bug shape: the player read tile entities in the same set
that created them and spawned *inside* the terrain — ordering alone
would not have fixed it; the set boundary's sync point is what makes
the tiles visible. And `on_tile_clicked` took `Res<HeightMap>` and
crashed the title screen because observers fire everywhere.

### Lens 7 — Test-altitude check

For rule- or world-touching code, is there a headless `App`
integration test (`crates/*/tests/`) that runs the schedule and
inspects the resulting world state — or only a pure-function unit
test / no test? Flag unit-only coverage of scheduling-sensitive
behavior.

Two extra repo-specific bars:
- **Fixture realism**: a fixture too simple to express the bug
  reports safety it doesn't provide (the buried-run bug passed green
  because the fake terrain spawned one tile per coordinate). New
  fixtures should resemble the real map — stacked runs, varying
  headroom.
- **The visual limit**: headless tests cannot see a black sky or a
  sunken piece. A finding that touches rendering or transforms must
  be flagged "needs `/visual-walk`" (the scripted capture walk reads
  real frames) — and "needs the human walk" on top for anything about
  motion, feel, or taste — rather than closed on green tests.

### Lens 8 — Feature / config traceability

For every cargo feature, RON field, or env/config value added in the
diff, trace it through ALL code paths:

- (a) Is it honored everywhere it's documented, or a no-op on some
  paths?
- (b) Does a missing/invalid value fail loud (stall loading with a
  named file+line) or silently default? On initial load, resources
  must be absent-until-parsed, never defaulted.
- (c) Does the doc comment / CONTENT-doc entry match what the code
  actually does — including *when* the value takes effect
  (every-frame vs at-interaction vs at-spawn)?
- (d) Are there dead branches? (Gated code the flag can never reach —
  e.g. `dev`-feature code that also needs a runtime condition that's
  never true in dev.)
- (e) Paired values in different files — RON field ↔ reader struct,
  scenario paths ↔ the test that opens every named file — are they
  cross-referenced or just hand-copied?

**Mandatory enumeration before reporting clean.** Run all 5
sub-checks for every flag in the diff. Don't skip based on intuition.

Reference bug shape: a flag added; one of three code paths checked
it, the other two ignored it, and the docstring claimed it covered
the entire run.

## Doc-only diffs — the docs lenses

When the diff has no Rust, the code lenses above are dead weight —
but this repo's docs are load-bearing in ways ordinary prose is not:
skills parse them, CI half-checks them, and the other developer reads
them as instructions. One subagent walks these four instead, then the
fresh-eyes pass (step 4) runs as usual. The Wave entry (step 6)
remains mandatory.

### Lens D1 — Automation contracts

If a changed file is consumed by a skill, verify the skill's parse
expectations against the new content. Current registry (extend as
skills grow):

| File | Consumer | Contract |
|---|---|---|
| `docs/planning/roadmap.md` | `/seed-tickets` | exactly one table, under `## Upcoming`, header `Epic \| Scope \| Owner`, 4 pipes per row, no `\|` inside cells, no hand-written `<!-- linear -->` markers |
| `docs/planning/status.md`, `docs/README.md` | `/update-docs` | index rows ↔ files bidirectional; status claims audit-able |
| `docs/planning/audit-log.md` | `/audit-diff` | the anchor comment intact; Wave entries only below it |
| `CLAUDE.md` | `/update-docs` | exactly one `and NNN tests` clause, inside `## Current state` |

### Lens D2 — Links and fragments

Every relative link resolves from the file's own directory, **and
every `#fragment` targets a real heading in the target file** — CI's
checker strips fragments before testing, so anchors are only ever
verified here.

### Lens D3 — Claims against reality

For every factual claim the diff makes about the repo, code, or an
open PR (a type exists, a file is untouched by #N, a count, a
delivered feature): spot-check it at the source. Reference bugs: a
doc framed an open PR's types as "already delivered" (a fresh `dev`
grep found nothing); a normative type sketch promised a serde default
the attribute cannot produce. Docs read as instructions get
implemented literally.

### Lens D4 — Single source of truth

Does the change duplicate or contradict prose that lives elsewhere —
especially status-shaped claims (`docs/planning/status.md` is the
only doc allowed to carry them) and anything CLAUDE.md summarizes
with a pointer? Three drifting copies of "what's built" is the
disease the docs tree was restructured to cure; don't let a diff
reintroduce copy number two.

## Subagent prompt template

When spawning the per-surface Explore subagents, use a prompt of
this shape:

```
Review the following diff for the {surface} portion of PR #{N}. The
PR's stated scope is: {one-line summary from gh pr view}.

Walk the diff through these 8 bug-class lenses, in order. For each
lens, report findings in this exact format:

- `file:line` — `<one-line code excerpt>` — short description —
  SHIP-BLOCKER | NON-BLOCKER

The code excerpt is mandatory. Without it the user can't verify a
finding without re-reading the file. If a finding spans multiple
lines, quote the most diagnostic 1–3 lines. When in doubt, use Read
to check the line you're claiming, then quote it verbatim.

Lens 1 — Silent failures: every error path in the diff. unwrap_or on
  parse/IO Results, empty Err arms, bare .ok(), if-let-Ok without
  else, log-and-continue on startup paths. Does it mask a failure
  that should be loud?
Lens 2 — DRY / single source of truth: every new constant. Does it
  duplicate something that already exists? Cargo.toml deps via
  workspace inheritance?
Lens 3 — Edge values: every new helper/rule. Enumerate level 0,
  negative coords, empty span, headroom 0, NaN, empty-collection
  cases for its inputs.
Lens 4 — Round-trip consistency: every newly-configurable value.
  Does every consumer read from the configured source? Right
  hot-reload row?
Lens 5 — Compiles-but-wrong API use: every new Bevy/hexx API call.
  Check the trap list: MessageReader vs observers, GlobalAmbientLight,
  StandardMaterial alpha inference, CursorMoved, AssetMut, hexx
  keyed-on-Hex collapse, HexCoord-keyed maps.
Lens 6 — Deferred-commands and ordering: every new system/observer/
  spawn. Sync points for Commands-spawned entities, shared sets
  across crate boundaries, Option<Res<T>> in observers,
  PausableSystems membership.
Lens 7 — Test-altitude: every rule/world-touching change. Headless
  App integration test with a realistic fixture, or unit-only?
  Rendering-adjacent findings flagged "needs /visual-walk" (plus
  the human walk for motion/taste).
Lens 8 — Feature/config traceability: every feature/RON field added.
  Honored on every path? Missing value stalls loudly? Docs match
  the actual read timing? No dead branches? Paired values
  cross-referenced?

Diff:
{paste the per-file diffs here}

Report: numbered findings list with file:line + code excerpt +
severity. If a lens fires zero findings, say "Lens N: clean" (do
not invent findings to fill the slot).
```

## When to run

- **From `/audit-pr`** — automatic as Step 1 of the merge gate. The
  receipt's `1_audit_diff` step records the outcome.
- **Standalone** during iteration, after a review round, or when you
  want a quick lens-check without paying the cost of `/test-full`.
- **User-invoked** via `/audit-diff` at any time.

The audit does NOT replace the human review — it removes the boring
half so the human can focus on design. Nor does it replace the
visual walk: every serious bug in this codebase so far was found by
a person looking at the window.

## Scope

The audit is **diff-anchored, not diff-bounded**. Findings inside the
diff are SHIP-BLOCKERS or NON-BLOCKERS per the triage rules. When a
finding fires, the relevant lens's sibling sweep extends scope to:

- Other instances in the modified file(s).
- Sibling files in the same module the diff touches.
- Pre-existing instances in unmodified files within the same module
  — surfaced as NON-BLOCKERS with rationale "out of scope but same
  class," logged for follow-up.

## What the audit does NOT do

- Generic "code quality" passes — too unbounded.
- Sweep parts of the codebase the diff doesn't touch — the diff is
  the seed; sibling sweep extends from it but doesn't fan out
  indefinitely.
- Re-design existing patterns the PR didn't touch.
- Bless or veto design decisions — that's the crate owner's job
  ("ownership cuts both ways"): flag contract bugs and broken
  boundaries as blockers; surface design questions as questions.

## Self-updating

When a human reviewer flags a bug class not currently in the lens
list, append a new lens (or extend an existing one) to this skill
BEFORE fixing the bug. When a new skill starts parsing a doc, add the
file to Lens D1's registry in the same PR — an unregistered contract
is invisible to the doc-only path. The skill should grow with each new bug class
observed. Keep the reference-bug pointer current — the next maintainer
needs to know when each lens earned its place.

## Troubleshooting

- **Subagent reports a false positive on Lens 5**: the subagent may
  not have checked the actual API signature. Re-grep the dependency
  source (or `cargo doc` output) to confirm, then dismiss the finding.
- **Lens 2 false positive on framework conventions**: some
  duplications are required (e.g., a component struct mirroring a RON
  schema). Note as known FP here.
- **Verification stack flaky**: rerun once. Persistent flakes are a
  separate concern; surface as NON-BLOCKER and move on.
- **No diff yet (pre-commit audit)**: run on staged + unstaged changes
  via `git diff HEAD`. Otherwise identical workflow.
