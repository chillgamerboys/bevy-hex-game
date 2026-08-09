# Wave protocol

**Audience:** contributors and coding agents.

**Owner:** both owners, jointly.

[parallel-development.md](parallel-development.md) answers *what shape should this work
take* — independent, stacked, or wave. This document answers the next question: **given a
wave, what is its artifact, its ownership algebra, and its merge order.**

It is the tool-neutral source of truth. Claude's `/plan-epic`, `/dispatch`, and `/inject`
and Codex's `$plan-epic` all route here rather than carrying their own copy of the policy.
Skills own mechanics; this document owns the rules.

## 1. Vocabulary

| Term | Meaning |
|---|---|
| Wave | One release candidate assembled from several lanes on a `wave/<slug>` branch |
| Lane | One unit of ownership inside a wave. A lane is a concept area, not a size |
| Order | The self-contained brief a lane is built from, under `orders/` |
| Map | Banked exploration a lane's order cites, under `maps/` |
| Dispatch queue | The machine-readable list of lanes. §3 |
| Coordinator | The single integration owner. Writes to the wave, reviews, merges |
| Worker | An agent building exactly one lane in an isolated worktree |
| **Authority** | *Crate* ownership: `world`, `gameplay`, or `shared`. Who decides the behavior |
| **Builder** | *Who types it*: `worker`, or `@<login>` for a human's own lane |

**Authority and builder are different axes and must never be collapsed into one `owner`
field.** A `gameplay`-authority lane can be built by a worker or by a human; a `@human`
lane holds real territory but never occupies a dispatch slot. A tool that cannot tell
those apart will dispatch a worker into a human's open edit.

## 2. The wave artifact

One wave, one directory, committed to the repository:

```
docs/planning/waves/<slug>/
  manifest.md          the survivor — plan, queue, ownership, acceptance
  orders/<id>-<slug>.md  transient — one per lane, deleted at close-out
  maps/<name>.md         banked exploration — spent, deleted at close-out
```

**The artifact is committed, never `.context/`.** `.context/` holds per-workspace scratch
and is not tracked, so anything stored there is invisible inside a fresh worktree, in a
fresh clone, and on the other owner's machine — including to every dispatched worker, which
is exactly the audience the orders exist for. That is a property of being untracked, not of
any one ignore rule, so it holds even where `.gitignore` says nothing about the path.
`docs/planning/spell-resolution-wave.md` is the existing precedent for a committed wave
record.

There is **exactly one wave artifact format**. A lane plan that lives anywhere else, in any
other shape, is not a wave — it is a note.

`manifest.md` carries, in this order:

1. **Header** — status (`planning` / `dispatching` / `integrating` / `landed` / `closed`),
   wave branch, base `origin/dev` SHA, coordinator, epic reference, the one shippable
   outcome, and explicit exclusions.
2. **Why this wave exists.**
3. **Locked decisions** — numbered, quoted verbatim into every order they bind.
4. **Shared foundation** — the live contracts the wave builds on, each with its authority;
   and every contract change the wave *requires*, with its owner and a behavior-neutral
   landing plan. A cross-owner contract change with no named owner and no landing plan is a
   stop condition, not a lane detail.
5. **Dispatch queue** — §3. One block, one truth.
6. **Ownership map** — per lane: verbatim paths, regions, composed end-states, hotspot
   rules, and a row for every file a teammate's in-flight branch also touches.
7. **Territory** — the teammate and other-owner PR and branch sweep, with measured
   footprints.
8. **Integration order** — the readiness graph: what runs in parallel, what lands last.
9. **Combined acceptance** — the `wave/* → dev` gate, enumerated rather than named: the
   runtime path the wave delivers; the composition and failure paths no single lane owns;
   regeneration and return-to-title or re-entry where relevant; the affected static
   camera/UI/rendered-map frames to inspect, or a verified-maintainer N/A; the affected
   video or human motion, input, and feel route, or a verified-maintainer N/A; and the typed
   hooks that prove every logical claim.
10. **Stop conditions.**
11. **Injection log** — one line per `/inject`.
12. **Close-out.**

**The manifest is updated as the wave runs, not only when it is written.** Each combined
checkpoint records its findings and the fixes made on the wave. A builder records its PR and
`in-review`; the coordinator alone records `merged-to-wave` after the merge is verified. A
manifest that still describes the plan rather than the territory is the input to every later
collision check, and it will be wrong.

**Decisions are amendable, never edited.** When one is retired mid-wave, keep the original
text and append an AMENDMENT naming what changed, who ratified it, and when. Orders quote
decisions verbatim, so a silent edit desynchronizes every copy: a locked decision that
changes its text quietly is worse than one that visibly ends.

**Maps are spent once the wave lands.** A map whose territory no longer exists is not
reference material, it is a trap for its next reader.

## 3. The lane field table

**This table is versioned here.** `/plan-epic`, `/inject`, `/dispatch`, and `$plan-epic`
point at it and do not restate it. One copy and four pointers, rather than four copies that
drift.

Encode the queue as a single fenced YAML block in `manifest.md` — sixteen fields do not
survive as a Markdown table, and YAML diffs per field.

| Field | Type | Meaning | Written by |
|---|---|---|---|
| `id` | `L1`, `L2`… | Lane label. The primary key, stable for the wave's life. **Not** the ticket | plan-epic / inject |
| `title` | string | One line | plan-epic / inject |
| `order` | path | `orders/L1-<slug>.md`, relative to `manifest.md`. Empty only for a `@human` lane with no order | plan-epic / inject |
| `ticket` | `HEX-N` or `null` | **Soft.** `null` is legal and never blocks dispatch or merge | plan-epic / inject |
| `authority` | `world \| gameplay \| shared` | Crate authority. A lane that crosses it is a stop condition | plan-epic |
| `builder` | `worker \| @<login>` | `worker` when absent. A `@human` lane is territory: counted in the ownership union and every sweep, never dispatched, never against the slot cap | plan-epic / inject |
| `branch` | `<prefix>/<slug>` | Cut from `wave/<slug>` **by the coordinator** before launch | dispatch |
| `owns` | list | Paths *and named regions*. Every entry must appear in the ownership map | plan-epic / inject |
| `dispatch_blockers` | list | What must be TRUE to START. **Empty is the normal value** | plan-epic / inject |
| `merge_blockers` | list | Lane ids or PR numbers that must land into `wave/*` first | plan-epic / inject |
| `fences` | list of `{path, disposition: retire\|retarget\|keep, reason}` | The fence disposition, machine-readable | plan-epic / inject |
| `selector` | `{concerns: [...], full: bool}` | Which `.config/test-scopes.json` concerns this lane's paths select, and whether it promotes to the complete gate | plan-epic |
| `evidence` | `logic-only \| static-presentation \| motion-or-feel` | Drives whether the lane needs a visual walk, and confirms it defers manual runtime to the wave PR | plan-epic |
| `sizing` | `{model, effort}` | Per-lane agent sizing | plan-epic / inject |
| `state` | `queued \| dispatched \| in-review \| merged-to-wave \| blocked \| deferred` | The builder records `in-review`; only the coordinator records `merged-to-wave` after verifying the merge | worker / coordinator |
| `pr` | int or `null` | Lane PR number, base `wave/<slug>` | worker |

Untagged `builder` means `worker`. **That tag is the only thing standing between a human's
in-progress lane and a worker dispatched onto it.**

## 4. Decomposition and ownership

A lane is a unit of **ownership**, not a unit of size. Decompose along the seams where
files stop being shared, not into equal parts.

### 4.1 Crate authority is the first seam

Every lane declares one authority: `world`, `gameplay`, or `shared`.

**The authoritative owner map is `CLAUDE.md` §"Two owners, two roles" and
`docs/architecture.md` §Ownership. Read it there and do not copy it into a manifest** — a
second copy of the crate list is exactly the drift this document exists to avoid, and the
map has moved before.

Two properties of that map matter more than the crate list when you are cutting lanes:

- **`hex_assets` is split by concern, not by directory.** Generic loader traits, load
  tracking, registration patterns, and cross-domain reference infrastructure are
  gameplay-owned; a domain's own schema, validation, settings, and content belong to that
  domain's owner. So a lane in `hex_assets` declares the authority of *the concern it
  touches*, and "it is all one crate" is not an argument for calling it `shared`.
- **`shared` means no domain authority, not joint authority.** `hex_game`, `hex_ui`,
  `hex_objects`, and `hex_editor` are wiring, presentation, and tooling. A change there
  that encodes a domain decision is that domain's lane, not a shared one.

**A lane whose `owns` crosses the world/gameplay line is a stop condition.** Re-cut the
seam, or plan a small behavior-neutral foundation lane that lands on `dev` first. A wave
does not make ownership collective; it is an integration boundary, not a new owner. When
two lanes would implement the same authority, that is one concern with one owner.

If a lane's crate is not in the owner map — a new crate, or one the map has not caught up
with — that is a decision for the owners, not a default. Tag it and route it through review
mode rather than guessing.

### 4.2 File and region ownership

Within an authority, ownership is expressed as **paths plus symbols**, and the union across
all lanes must have no *unannotated* overlap — precisely what `/dispatch` pre-flights, so a
failure there is a decomposition bug.

Where a file genuinely must be shared, split it by **region** — a module, a block, a
`register_type` list — and name both each region's owner and the **composed end-state**:
what the file looks like once both lanes land. Two lanes that each drop half of a block and
keep the other half produce a clean-looking auto-merge and a broken file. Add a **hotspot
rule** per shared file: who refreshes after whose changes and how the composed end-state is
verified.

The repo's append-only shared files — `crates/hex_assets/src/lib.rs`,
`crates/hex_units/src/lib.rs` and their siblings — are the canonical regional case. A lane
adds its `mod` / `pub use` / `register_type` line in the existing alphabetical position and
touches nothing else. A one-line addition merges; a reflow does not.

**`manifest.md` is shared by construction, and every wave must annotate it.** Each lane
updates its own `state` and `pr` in its own PR (§3) — that is what makes the queue
trustworthy rather than aspirational — so the manifest belongs in *every* lane's `owns`,
scoped to **that lane's queue row only**, with the standing hotspot rule that a lane merges
the updated wave into its published branch rather than resolving a sibling's row in
isolation. Omitting it makes each lane's
definition-of-done edit an ownership violation; listing it without the region split trips
the disjointness check. It is the one overlap that is expected on every wave, and expected
is not the same as unannotated.

A **new file importing a symbol another lane deletes** is invisible to any path-level map.
Wherever that shape is visible, make it an explicit merge blocker and flag the composed-tree
check in both orders.

**Test fences are ownership too.** Assign every load-bearing fence the wave touches to
exactly one lane, or two builders will edit the same assertion in opposite directions. Never
disposition a fence as `retire` without naming the claim that dies with it, and say in the
order that the fence going red first is it working.

Shared claims-about-now files that nobody owns — the test count in `CLAUDE.md`, type names
quoted in `docs/planning/boundary.md` — belong to the coordinator at integration, not to
three lanes at once.

### 4.3 Selector economics

Read `.config/test-scopes.json` and record each lane's selected concerns and whether it
promotes to the complete gate. This is a decomposition input, not a footnote: **if three
lanes each promote to the full gate, they are usually one foundation lane plus two.**
Splitting work that all pays the same expensive gate multiplies cost without buying
isolation.

### 4.4 Mixed ownership

A wave may carry lanes a human teammate builds themselves. They are tagged `builder:
@<login>`, they count in the ownership union and in every territory sweep, and they never
dispatch and never occupy a slot. A lane whose authority is the other owner's is **offered,
never assigned**.

## 5. Sequencing: two blocker fields, never one list

Fill in two fields per lane, separately:

| Field | Contains | Typical value |
|---|---|---|
| `dispatch_blockers` | file or region overlap with a RUNNING lane; an unmade decision; a missing artifact, such as a map not yet banked | **empty** |
| `merge_blockers` | the lanes that must LAND first: a deletion whose last importer another lane removes, a handoff contract, a fence another lane writes | one or two |

**Dispatch order is not merge order.** A lane queued behind another *for merging* still
dispatches now. Then audit the output: **a lane whose only blocker is "merges after X" has
an EMPTY `dispatch_blockers`.** If more than a couple of lanes carry dispatch blockers, the
plan is almost certainly a merge chain that has been mislabeled — the exact failure this
representation exists to prevent. If the lanes genuinely form a chain, say so and stop: a
chain is stacked work, not a wave.

Every handoff gets a **verifiable contract**, not a promise — "lane L2 leaves
`crates/hex_units/src/lib.rs` importing nothing from your module; grep-zero that on the
refreshed tree before you delete the file." A late additive refresh from the wave is the
builder default, so the order says *verify*, never *assume*.

## 6. Merge order and the wave branch

One coordinator creates `wave/<slug>` from current `origin/dev` — **after the manifest and
orders have landed on `dev`**, so that every lane worktree cut from the wave contains its
own order. Cutting the wave first produces workers whose briefs are absent from the tree
they check out, which is the same failure as storing the plan in untracked scratch. The
manifest header records the branch and that exact base SHA. Only the coordinator writes
directly to the wave. Lane builders push additive commits to their own branches. Published
and shared branches are never rebased or force-pushed; merge updated `dev` into an
already-published wave with an additive commit.

**Every lane PR targets `wave/<slug>` and merges through the repository's ordinary gate**
— `/audit-pr` then `/merge-pr`, with a merge commit. Nothing in a wave loop targets `dev` or
`main`. The single `wave/* → dev` merge is a separate deliberate act by the coordinator,
carrying the combined head's own exact-head runtime classification (§7), and `dev → main`
is `/promote` only.

Integrate in semantic order:

1. shared foundations and contract corrections;
2. owner-local foundations;
3. feature lanes;
4. composition and adapters;
5. combined fixes discovered by the wave.

Resolve a shared concern once on the wave. Push a correction back to a source branch only
when that branch remains an independently useful review unit.

### The composed-tree check

**After every lane merges into the wave, re-plan the selector against the composed head and
run the concerns it selects:**

```sh
python3 tools/test_scope.py plan --base origin/dev --head origin/wave/<slug>
```

This is mandatory because **CI's cover of the wave branch is stale, not absent, and stale is
the dangerous shape.** `.github/workflows/ci.yaml` has no branch filter on `pull_request`,
so a lane PR into `wave/*` does run the full selector-driven gate against GitHub's merge
commit — but it runs on *push*, not when the base moves underneath it. A lane that went
green before its sibling landed has proven nothing about the tree that now exists, and
`ci.yaml`'s `push` jobs are limited to `main` and `dev`, so nothing re-checks the wave
branch after a merge. Two lanes green in isolation can compose red — most sharply when a new file imports a symbol
another lane deleted, which every path-level ownership map is blind to. Discovering that at
the wave gate, after the builders are gone, costs the wave.

**This is the selector-chosen composed run, not the complete candidate gate**, and the
distinction is the whole point. The
[review budget](parallel-development.md) still applies unchanged: detailed review and
expensive validation escalate when a semantic group becomes coherent, and the full
platform, shipping, and coverage gate runs **once**, on the final wave PR. Repeating that
tier per leaf is the V3/Ring7 failure — it consumed time across more than ten provisional
PRs without increasing confidence in the combined runtime state.

What changed, and why it is worth saying plainly: the earlier guidance was "do not demand a
separate full workspace run after every mechanically integrated leaf," written when the
integration branch had CI watching it. It does not. So the *cheap* composed check became
mandatory per lane while the *expensive* candidate gate stayed once-per-wave. If a single
lane's paths promote the composed plan to the complete gate, that is a signal about the
decomposition (§4.3), not a licence to skip the run.

`MERGEABLE` is a claim about text, not about composed behavior. A clean auto-merge is not
evidence that two changes compose.

Before merging a lane, also read what it *removed*:

```sh
git diff origin/wave/<slug>...HEAD | awk '/^--- a\//{f=$2} /^-[^-]/{print f": "$0}'
```

A builder on a stale base can rewrite a file wholesale and revert a deliberate default
inside it — additive-looking, green on every gate, and invisible in any summary.

## 7. Evidence for lanes and for the candidate

Screenshots and rendered frames prove static presentation — camera framing and occlusion, UI
hierarchy, layout, legibility, focus, contrast and reflow, and rendered-map geometry,
materials, lighting, cutaways, seams, and composition. Video and a human prove motion,
native-input response, animation, control feel, and taste. **Neither ever proves or
corroborates gameplay or exact world logic that a typed hook, message, log, snapshot, or
deterministic contract can express.** If that oracle is missing, add the narrow hook rather
than infer logic from pixels.

Each lane declares its `evidence` class, and that class drives its gate:

- `logic-only` — hook-backed closure, no visual route.
- `static-presentation` — the affected frames of the automated visual walk.
- `motion-or-feel` — deferred to the combined candidate; a still frame cannot establish
  motion, and a lane cannot buy this evidence alone.

**Manual runtime sign-off is deferred for source lanes.**
`.github/workflows/manual-runtime-signoff.yaml` already exempts a PR whose base matches
`wave/*`, on the grounds that exact-head sign-off belongs to the combined wave PR into
`dev`. `/audit-pr` records that deferral rather than a hook closure for such a lane. The
combined `wave/* → dev` PR carries the real classification at its own exact head, under the
repository's ordinary two-way rule: a wave whose changed surface includes rendered
presentation, native input, motion, seams, control feel, or taste needs a **named human's
`PASS`**; a logic-only wave records a **verified-maintainer `N/A`** naming its authoritative
hook closure. Wave topology alone does not manufacture a visual gate. No lane's evidence may
be copied onto the wave PR, and any subsequent commit invalidates either classification.

The display is a **machine-global resource**, along with the human playtest and the disk for
each worktree's own build directory. The coordinator serializes every visual walk and every
interactive run on the composed head; parallel builders never invoke them.

## 8. Reconciling pre-existing branches into a wave

`/dispatch` covers lanes it created. This section covers the other case: a batch of branches
or PRs that already exists and now needs to become one candidate — stale stacked branches,
duplicate shared logic, or a feature set whose release confidence only comes from combined
runtime behavior.

**Start by classifying and recording the batch.** If there is no manifest, use
`$plan-parallel-work` to confirm the batch really is a wave, then write the manifest before
changing any branch — with the existing branches as its lanes, their real owners as their
builders, and their measured footprints as the ownership map. Every step below records into
that artifact, and the inventory is worth nothing if it lives only in a session.

### Inventory before mutation

Fetch `dev` and every source branch. For each source, record the PR number and draft/review
state, the declared base and the actual merge base, the head SHA and unique commit range,
the changed files, contracts and authority, parent/child PR relationships, current checks,
and the intended residual behavior.

**Do not trust a draft label as a readiness signal. Do not trust a green leaf check as
evidence of combined readiness.**

```sh
git fetch origin --prune
git branch -r --contains <sha>
git merge-base origin/dev origin/<source>
git log --graph --oneline --decorate origin/dev origin/<source>
git diff --stat <merge-base>..origin/<source>
git log --reverse --format='%H %s' <merge-base>..origin/<source>
git cherry -v <intended-base> origin/<source>
```

For a PR, inspect its declared relationship rather than inferring it from the commit graph:

```sh
gh pr view <number> \
  --json number,title,state,isDraft,baseRefName,headRefName,headRefOid,body,mergeable,statusCheckRollup
```

### Select the integration operation

- **Merge the branch** when its full ancestry is current and belongs in the wave.
- **Cherry-pick a unique range** when the tip contains an obsolete parent snapshot but the
  intended child commits are cleanly identifiable.
- **Reimplement the small residual diff on the wave** when conflict resolution is the
  feature and transplanting would preserve a duplicate implementation.
- **Stop for an ownership decision** when choosing a version changes world or gameplay
  authority. Mechanical conflicts alone do not require a stop.

Merging the tip of a branch created from another feature branch can resurrect an obsolete
snapshot of the parent after `dev` has moved. Preserve authorship, and record the source PR
or commit range in the manifest.

After every operation, compare both the file diff and the commit provenance:

```sh
git diff --stat origin/dev...HEAD
git diff --check
git log --oneline origin/dev..HEAD
```

Never rebase or force-push a published source branch. Keep source branches until the wave
has landed and every child PR has been retargeted.

When lanes touch the same concern, decide whether they implement the same feature or
distinct authorities: same authority means consolidating on one contract and removing the
duplicate; distinct authorities means keeping the implementations separate and connecting
them through the published contract; a behavior-changing ownership ambiguity is a stop.

Mechanical conflicts and straightforward contract adaptation do not require user review. Fix
them on the wave with additive commits and record the resolution.

## 9. Land and close out

Before the final gate, reconcile implementation, status/design/roadmap documents, contracts,
and — when available — ticket descriptions using [delivery-state.md](delivery-state.md).
Documentation corrections belong in the candidate. Linear is strongly advised for visibility
but is never a merge gate.

The wave lands on `dev` with a merge commit. Then:

1. confirm the merge commit and post-merge `dev` checks;
2. close source PRs as superseded, linking the wave PR;
3. retarget any still-open child PR before deleting its former base;
4. delete the wave and merged source branches only after no open PR depends on them, and
   never while the manifest lists a lane that is not yet `merged-to-wave` or `deferred`;
5. merge the close-out PR described below so the durable record precedes ticket deletion;
6. reconcile or policy-delete tickets from delivered outcomes rather than incidental
   leaf-PR state, leaving a visible warning if Linear was unavailable; and
7. leave a short reconciliation note for protected or ongoing branches that must now merge
   updated `dev`.

**Close-out is a small post-landing PR to `dev`, not a source lane.** It updates the
manifest to `closed`, records the wave PR and exact `dev` merge SHA, deletes `orders/` and
`maps/`, removes their links in the same commit, and lists any Linear issues eligible for
policy-governed deletion. Merge that durable record before deleting those issues. The
documentation concern's relative-link check is a hard gate, so a manifest still linking a
deleted order fails the close-out PR.

Never merge a wave directly to `main`. Promotion is separate and deliberate.

## 10. Stop conditions

- A lane whose `owns` crosses the world/gameplay authority boundary.
- Two lanes implementing the same authority.
- Any unannotated overlap in the ownership union, including `@human` lanes and teammate
  branches.
- An unresolved cross-owner behavior choice, or an unruled tagged decision — it is a
  dispatch blocker on every lane it binds, and N builders reading an argued document produce
  N readings of it.
- A lane PR whose base is not `wave/<slug>`.
- **Any attempt to merge a wave loop into `dev` or `main` without the coordinator's separate
  human-gated landing.**
- A `wave/* → dev` PR without an exact-head runtime classification of its own — a named
  human's `PASS` for a changed presentation or experience surface, or a
  verified-maintainer `N/A` naming the hook closure for a logic-only wave.
- The composed selector run red after a lane merge, with both lanes green in isolation.
- A source diff that cannot be separated from obsolete parent state.
