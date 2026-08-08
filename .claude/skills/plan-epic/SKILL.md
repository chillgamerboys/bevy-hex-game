---
name: plan-epic
description: "Decompose a wave into lanes with disjoint crate authority and file ownership, bank the exploration as maps, sequence each lane with separate dispatch and merge blockers, take explicit approval, then commit the manifest and work orders under docs/planning/waves/ and emit the dispatch queue /dispatch consumes. Use when a wave has about three or more lanes that can run in parallel."
---

# Plan an epic into a wave

An epic becomes a **wave**: a manifest of lanes, one self-contained work order per lane,
and a dispatch queue `/dispatch` can execute without re-deriving anything. This skill
spends the judgment; `/dispatch` spends the wall-clock.

Four hard rules govern every step:

1. **Dispatch order ≠ merge order.** Every lane carries two separate fields —
   `dispatch_blockers` (what must be true to START) and `merge_blockers` (what must LAND
   first). Most lanes have merge blockers and zero dispatch blockers. One ordered list
   collapses the two, and a coordinator then executes your *merge* order as a *work* order:
   slots idle for hours with nothing named as blocking them.
2. **An order is self-contained or it is not an order.** The worker has none of your
   context and cannot ask a follow-up cheaply. Exploration is banked INTO the order as a
   map with `file:line` anchors; decisions are quoted verbatim, not linked.
3. **Ownership is disjoint, or regional with a named composed end-state.** There is no
   third option. Two orders editing "the same file, different parts" without stated regions
   and a stated end-state compose into nonsense that both PRs call green.
4. **The repo has other authors.** A lane's territory must be disjoint from the other lanes
   *and* from the other owner's in-flight branches and assigned tickets. A map drawn over
   your own wave only is a map of half the repo.

Copy this checklist and track progress:

```
Plan Epic Progress:
- [ ] Step 0: Resolve the epic + pre-flight
- [ ] Step 1: Read the epic, its children, its comments
- [ ] Step 2: Survey, BANK the maps, sweep territory
- [ ] Step 3: Decompose into lanes with disjoint ownership
- [ ] Step 4: Sequence — dispatch_blockers vs merge_blockers
- [ ] Step 5: Resolve ambiguity → locked decisions
- [ ] Step 6: Draft manifest + orders → approval (plan mode OR review mode)
- [ ] Step 7: Tickets, comment, commit, emit the queue
```

## Step 0 — Resolve the epic + pre-flight

1. **Resolve the input**, in priority order: an explicit `/plan-epic HEX-42`; or a prose
   goal plus a banked audit the operator names; or neither, in which case list recent
   candidates and ask which epic to plan.

2. **Read [wave-protocol.md](../../../docs/development/wave-protocol.md) first.** It owns
   the artifact layout, the lane field table (§3), the ownership algebra, and the merge
   order. This skill's output must satisfy that contract, and the contract is versioned
   there, not here. `/dispatch` consumes what you emit and `/inject` mutates it once the
   wave is running — so write both artifacts as things a later author will *mutate*, not as
   a finished plan.

3. **Confirm the topology.** `$plan-parallel-work` and
   [parallel-development.md](../../../docs/development/parallel-development.md) own the
   independent/stacked/wave choice. A chain of strictly dependent lanes is stacked work,
   not a wave; do not decompose one into a wave to make it look parallel.

4. **Linear is soft.** When a connector is available, use it. When it is not, emit a
   visible warning, plan from repository and GitHub evidence, key every lane on its lane
   id, put the recommended child set in the handoff, and continue. **Missing Linear access
   never blocks planning or dispatch** — see
   [delivery-state.md](../../../docs/development/delivery-state.md).

5. **Clean-enough worktree on a branch cut from `origin/dev`.** Step 7 commits the manifest
   and orders from the current workspace branch; warn the operator to stash or commit work
   in progress. In Conductor, use the current branch exactly as provided and never switch or
   create one — if it is the wrong branch, stop and ask for the right workspace.

## Step 1 — Read the epic, its children, its comments

When Linear is connected:

- **The epic**: fetch it by identifier and verify it belongs to the Hex Game team by
  returned key and name. Stop on any other team. Never use a stored identifier.
- **Children**: a child already In Progress or carrying an open PR is **not yours to
  plan** — it becomes a merge blocker for whatever touches its files.
- **Comments**, on the epic *and* on every child: later comments routinely override the
  description. Newest decision wins; note contradictions for Step 5.
- **Adjacent lanes.** Work in flight from the other owner never becomes one of your
  orders — Step 2's territory sweep turns it into a merge blocker plus a hotspot rule
  instead.

Reconcile all of it against the repository using
[delivery-state.md](../../../docs/development/delivery-state.md) before planning from it. A
merged leaf PR does not complete a partial epic, and an old open ticket does not prove its
behavior is still absent.

## Step 2 — Survey, BANK the maps, sweep territory

The survey is not for you — it is for workers who will not repeat it. Delegate breadth to
Explore agents, one per candidate lane, rather than reading trees into this context. Then
**write the result down** as a map file under `docs/planning/waves/<slug>/maps/`. A map that
lives only in this session dies with it, and the next re-dispatch re-explores at full cost.

A useful map is specific: `file:line` anchors, per-symbol dispositions (delete / move /
keep, with the reason), importer counts, and the field-level answer to "what does this
surface actually read". Anchors go stale as PRs land, so every order citing a map carries
the rule: **if reality disagrees with the map, that is an escalation, not a judgment
call** — and re-derive anchors after each rebase.

Maps are **spent** once the wave lands and are deleted with the orders at close-out: a map
whose territory no longer exists is not reference material, it is a trap for its next
reader.

### Territory

Sweep the humans *before* you decompose. `gh pr list --state open` across **all** authors,
plus `git fetch origin --prune` — and read the new-branch lines rather than discarding them;
a branch surfaces hours before its PR does. Bank one row per PR or branch:

| PR/branch | Author | Footprint | Lanes sharing files | Relationship |
|---|---|---|---|---|

Footprint is measured, not guessed — `git diff --stat origin/dev...origin/<branch>`. A
shared file is **not** a blocker by itself; the relationship column is the point. Disjoint
hunks compose on their own; same-region needs a sequencing rule or a heads-up before
dispatch.

The world owner runs the game constantly against `dev` and lands map PRs there while a wave
is open. Their work is territory in exactly this sense, and it moves.

**Tickets are territory too.** A lane binding to a ticket authored or assigned to the other
owner inherits that ticket's banked design as **LAW** — quoted verbatim into the order, with
an on-ticket heads-up and a stand-down offer posted before dispatch. And because tickets
age, every fact an order lifts from ticket prose — a type name, a system set, a count — is
re-checked **against the code at order-writing time**. Prose does not get renamed when the
code does.

## Step 3 — Decompose into lanes with disjoint ownership

A lane is a unit of **ownership**, not a unit of size. Decompose along the seams where
files stop being shared, not into equal parts.
[wave-protocol.md §4](../../../docs/development/wave-protocol.md) is the full algebra; this
step produces its output.

**Crate authority is the first seam.** Every lane declares `world`, `gameplay`, or
`shared`, and **a lane whose `owns` crosses the world/gameplay line is a STOP**: re-cut the
seam, or plan a small behavior-neutral foundation lane that lands on `dev` before the wave
depends on it. Cargo enforces the crate graph, but it cannot see a lane's declared
ownership, so this is a planning-time check or it is nothing.

Within an authority:

- One lane, one concept area, one PR.
- Ownership is **paths plus symbols**. The union across all lanes must have no *unannotated*
  overlap — precisely what `/dispatch` pre-flights, so a failure there is this step's bug.
- Where a file genuinely must be shared, split it by **region** and name both each region's
  owner and the **composed end-state**. Two orders that each drop half of a block and keep
  the other half produce a clean-looking auto-merge and a broken file. The repo's
  append-only shared files — `crates/hex_assets/src/lib.rs`, `crates/hex_units/src/lib.rs`
  and their siblings — are the canonical case: one line in its existing alphabetical
  position, nothing else.
- Add a **hotspot rule** per shared file: who rebases over whom.
- A **new file importing a symbol another lane deletes** is invisible to any path-level
  map. Wherever you can see that shape, make it an explicit merge blocker and flag the
  composed-tree check in both orders.
- **Test fences are ownership too.** Assign every load-bearing fence the wave touches to
  exactly one lane, or two workers will edit the same assertion in opposite directions.
- **Tag every lane's builder** — `worker`, or `@<login>` for a lane a human builds
  themselves. A `@human` lane is never dispatched, but it belongs in this map and in the
  queue because it holds real territory: its files count in the disjointness union and its
  merges go through the hotspot rule. Untagged means `worker`, so the tag is the only thing
  standing between a human's in-progress lane and a worker dispatched onto it.
- **`manifest.md` is shared by construction and must be annotated, not omitted.** Every
  lane updates its own `state` and `pr` there in its own PR, so the file belongs in every
  lane's `owns` — scoped to *that lane's queue row only*, with the hotspot rule that a lane
  rebases onto the wave rather than resolving a sibling's row. Omitting it makes the
  definition-of-done edit an ownership violation; listing it without the region split trips
  the disjointness pre-flight.
- Leave the *other* shared claims-about-now files — the test count in `CLAUDE.md`, type
  names quoted in `docs/planning/boundary.md` — to the coordinator at integration, not to
  three lanes at once.

### Selector consequence per lane

Read `.config/test-scopes.json` and record each lane's selected concerns and whether it
promotes to the complete gate. This is a decomposition input, not a footnote: **if three
lanes each promote to the full gate, they are usually one foundation lane plus two.**
Splitting work that all pays the same expensive gate multiplies cost without buying
isolation. Record the result in each lane's `selector` field.

Record each lane's `evidence` class in the same pass — `logic-only`,
`static-presentation`, or `motion-or-feel`. A `motion-or-feel` claim cannot be proven by a
lane at all; it defers to the combined candidate.

## Step 4 — Sequence: `dispatch_blockers` vs `merge_blockers`

Fill in two fields per lane, separately — never one ordered list. What belongs in each is
[wave-protocol.md §5](../../../docs/development/wave-protocol.md), including the verifiable
handoff-contract rule; do not copy that table into the manifest.

The part that is *this step's job* is the self-audit: **a lane whose only blocker is "merges
after X" has an EMPTY `dispatch_blockers`.** If more than a couple of lanes carry dispatch
blockers, you have almost certainly written a merge chain and mislabeled it — the exact
failure this representation exists to prevent. If the lanes genuinely form a chain, say so
and stop; that is stacked work, not a wave.

## Step 5 — Resolve ambiguity → locked decisions

Research before asking; ask only what the repository cannot answer; batch up to four
questions; expect multiple rounds. Every answer becomes a **numbered locked decision** in
the manifest, quoted verbatim into every order it binds.

Wave-specific dimensions that must be unambiguous before decomposing:

| Dimension | Must be unambiguous |
|---|---|
| Delete vs fix | Per surface. "Delete the affordance" is a decision, not a shortcut |
| Boundary | What belongs to this wave versus an adjacent lane; what is deliberately parked, and why |
| Shared-file end-states | For every file two lanes touch (Step 3) |
| Cross-owner behavior | Any change to a fact that crosses the world/gameplay boundary |
| Terminal state | What "this wave is done" means, in checkable terms |
| Gate scope | Which tiers are hermetic (workers run them) versus machine-global (the coordinator serializes them) |

**A decision that is not yours to make gets TAGGED, not guessed.** Write it into the table
with its owner named — `DECISION D5 — @<owner>` — carrying the options and your
recommendation, and route the artifact through **review mode** (Step 6) so the ruling lands
where every order can quote it. Guessing is the expensive version of asking. In this repo
the commonest case is a behavior choice inside the other owner's crates.

**Decisions are amendable, never edited.** When one is retired mid-wave, keep the original
text and append an AMENDMENT naming what changed, who ratified it, and when. Orders quote
decisions verbatim, so a silent edit desynchronizes every copy: a locked decision that
changes its text quietly is worse than one that visibly ends.

**Do not enter plan mode with a blocking unknown.** Report the open questions; an unapproved
decomposition beats a wrong one multiplied by N workers.

## Step 6 — Draft manifest + orders → approval

Approval has two channels. Choose before you draft:

| Channel | Use when | Approval is |
|---|---|---|
| **Plan mode** (default) | the wave is yours end-to-end and every open decision is the operator's to make | an explicit `ExitPlanMode` |
| **Review mode** | the wave crosses the other owner's territory, a lane's authority is theirs, a decision is tagged to another human, or the operator asks for review | a **merged draft PR** |

Either way the approval object is the same — the decomposition and the ownership map, the
part that cannot be cheaply fixed once N workers are running. No effort or time estimates,
in any section.

### Review mode — the draft PR is the approval channel

When a decision belongs to someone who is not in this session, the wave artifact goes up as
a **draft PR**, its open questions carried as named callouts (`DECISION D5 — @<owner>`) and
the rulings coming back as review threads. Three rules, because a planning draft with no
fork ahead of it rots:

- **The draft reviews the wave artifact ITSELF**, never a parallel summary. Threads are the
  discussion; the manifest's Locked decisions section absorbs each ruling verbatim (amend,
  never edit). A summary beside the artifact desynchronizes the moment anyone rules.
- **Dispatch is blocked while the draft is open** — a real `dispatch_blocker` on every lane
  the open decisions bind. Workers reading a doc still being argued produce N readings of
  it.
- **It merges or it closes.** Merging locks the decisions and this skill *finishes*
  (Step 7, on the ruled table); closing leaves a pointer to what superseded it. There is no
  third state — a draft nobody has to resolve is where planning goes to die.

**Decisions-first is a legitimate shape**: the artifact may go up carrying only the decision
table and its options, before lanes or orders exist. The ruled table is then what the rest
of Step 6 plans against — cheaper than decomposing twice.

### The manifest — `docs/planning/waves/<slug>/manifest.md`

The **durable** artifact; it outlives the orders. Its section list is in
[wave-protocol.md §2](../../../docs/development/wave-protocol.md); keep the paths and
section names as written there, because `/dispatch` and `/inject` read them.

The dispatch queue goes in as one fenced YAML block using the §3 field set. Lane `state` and
`pr` are updated by each lane's own builder in its own PR — what makes the table trustworthy
rather than aspirational. Rows still reading `in-review` once the wave closes get reconciled
against `gh pr list` at close-out.

**The close-out convention.** Orders are transient by design — instructions for one worker
on one branch at one moment, full of "if lane L2 has merged, do X instead" conditionals
answered the moment it merges. So the wave ends with a **close-out PR** that absorbs and
deletes the orders and their maps: class rules live in the fences the wave wrote, chosen
debt in the relevant `CLAUDE.md`, descoped work on its own ticket, and the arc's shape in
the manifest. Plan it as a lane from the start, and remove the links in the same commit that
removes the files — the documentation link check is a hard gate.

### A work order — `docs/planning/waves/<slug>/orders/<id>-<slug>.md`

One file per lane, self-contained:

| Section | Contents |
|---|---|
| Context | What this order does, and the pointer to its banked map with the "map disagrees ⇒ escalate" rule |
| Ticket | The real `HEX-N` and verify-before-branch, or an explicit "none — keyed on lane id" |
| Authority | `world`, `gameplay`, or `shared`, and the crates it covers |
| Locked decisions that bind this order | Quoted verbatim from the manifest |
| The map | `file:line` anchors, per-symbol dispositions, per-file tables |
| Sequencing | Ownership and regions, `dispatch_blockers`, `merge_blockers`, the verifiable handoff contract |
| Fences and verification | `/test-quick` with the exact selector construction in [`CONTRIBUTING.md`](../../../CONTRIBUTING.md#before-opening-a-pr), the named fences, the grep-zeros, and the fence disposition table |
| Selector | The concerns this lane selects, and whether it promotes to the complete gate |
| Evidence class | `logic-only`, `static-presentation`, or `motion-or-feel`, plus the wave deferral for manual runtime sign-off |
| Definition of done | Checkable lines, including the lane's own queue-row update |
| Out of scope | The other lanes' surfaces, by name |
| Escalation triggers | The specific conditions that mean "stop and ask" |
| Protocol pointer | One line at the foot, at [the worker contract](../dispatch/references/worker-contract.md) |

The **fence disposition table** (`| Fence | What breaks | Disposition |`) is what lets a
worker touch a load-bearing test safely — per fence: *retire*, *retarget* or *keep*, each
with its reason. Without it a worker either weakens a fence it does not understand or
refuses to touch one it should. Never disposition a fence as retire without naming the claim
that dies with it, and say in the order that the fence going RED first is it working.

Orders name gates by skill and by contract, never as a literal cargo command — the
fail-closed selector owns concern choice, and a hardcoded command silently drifts from it.

## Step 7 — Tickets, comment, commit, emit the queue

In **review mode** this step runs after the draft PR merges — the ruled decision table is
its input, and nothing here should precede it. Confirm once, then, in this order:

1. **Child tickets, when Linear is connected.** Per lane, verify or mint the child under the
   epic. `/plan-epic` is one of exactly two workflows authorized to create issues, because
   it owns both a parent and its own deduplication — one lane, one child, over an ownership
   union that is disjoint by construction. The **approved manifest is the authorization**;
   do not ask a second time. Resolve the team and the In Progress state from live connector
   data by returned name and type, and pass the resolved state identifier rather than a
   name. Never embed a workspace UUID, and never create the epic itself — that is
   `/plan-ticket` and the human.

   Then fill the **real numbers** into the orders: a worker handed a placeholder guesses,
   and a green PR bound to someone else's ticket is expensive to unwind. If the connector is
   unavailable, say so loudly, leave every `ticket` field `null`, key the orders on lane
   ids, and list the recommended children in the report. The wave proceeds.

2. **Post the plan to the epic** as a comment — context, locked decisions, the
   lane-to-ticket mapping, and what is parked and why. The file-level detail is the orders'
   job. Do not paste secrets or local absolute paths.

3. **Commit the manifest and orders in-repo**, on a branch off `origin/dev`, through the
   ordinary `/create-pr` → `/audit-pr` → `/merge-pr` path. This is the one artifact PR that
   targets `dev` rather than the wave. **Do not cut `wave/<slug>` here.** It is cut from
   `origin/dev` *after* this PR lands — that ordering is what guarantees each lane's
   worktree contains its own order, and reversing it leaves every worker reading a tree its
   brief is missing from. Leave the manifest header's wave base SHA blank; `/dispatch`
   Step 0 cuts the branch and fills it in.

   In Conductor this PR uses the current workspace branch. If that branch is not cut from
   `origin/dev`, stop and ask the operator for the right workspace, exactly as `/create-pr`
   preflight 3 requires — never switch or create one here. In-repo is the point: the operator reviews the
   decomposition as a diff, and each worker reads its order out of the tree it checks out.
   A plan under `.context/` is invisible in a worker's worktree.

4. **Emit the dispatch queue** in the
   [§3 field set](../../../docs/development/wave-protocol.md), the two blocker lists as
   separate fields, persisted in the manifest so a re-dispatch after this session does not
   re-derive it. Order the queue by **readiness, not by merge order** — the first three rows
   should be dispatchable at t=0; if they are not, say why in the report. Sizing is yours
   too: a mechanical lane and a delicate refactor are not the same model and effort.

5. **Report and hand off**:

   ```
   ✓ /plan-epic complete — HEX-N "<title>"
     lanes:    N (M dispatchable now)
     tickets:  <minted/verified list, or "none — Linear unavailable">
     manifest: docs/planning/waves/<slug>/manifest.md
     orders:   docs/planning/waves/<slug>/orders/ (N files)
     wave:     wave/<slug> — not yet cut; /dispatch Step 0 cuts it from origin/dev
               once this artifact PR has landed
   Next: /dispatch
   ```

Nothing here launches a worker or merges anything into a wave — `/dispatch` owns the wave
from this point, and work discovered after it starts comes in through `/inject` rather than
a second run of this skill.

## When NOT to invoke

- **A single ticket** → `/plan-ticket`. One agent with full context beats a brief.
- **Backlog seeding** with no decomposition to do.
- **Fewer than about three parallelizable lanes.** That threshold is a recommendation, not
  a gate, but briefs and slot audits cost more than the parallelism wins below it.
- **A genuine dependency chain.** That is stacked work, at most two deep.
- **A wave already dispatching** → `/inject`. Re-planning over a live wave rewrites
  artifacts running workers are reading.
- **Exploration.** A wave executes banked judgment; without a map, dispatching produces N
  contradictory ones.

## Troubleshooting

**Every lane has dispatch blockers.** You wrote a merge chain. Re-read Step 4's audit; if it
really is a chain, this is stacked work.

**The ownership union overlaps and you cannot re-cut it.** The seam is probably crate
authority, not files. Check whether two lanes are really one concern with one owner.

**A lane needs a type that does not exist.** Check `origin/wave/<slug>`, not the workspace
checkout — a foundation lane may already carry it one branch over.

**Three lanes all select the complete gate.** Collapse them. The gate cost, not the file
count, is what parallelism is buying here.

## Self-updating

- **The queue's field set changes** → change it in
  [wave-protocol.md §3](../../../docs/development/wave-protocol.md) and nowhere else.
- **A decomposition failure costs a wave** → add the shape to Step 3 before fixing the
  instance.
- **An order section is repeatedly missing** → add it to the order table, not to prose.

## Provenance

Ported from the jxp-skills seed's `/plan-epic`, adapted to this repository: crate authority
is the first decomposition seam, selector economics are a planning input, Linear is soft and
its identities are resolved live, the artifact is committed under `docs/planning/waves/`,
and the lane field table lives in `wave-protocol.md` rather than being mirrored here. It
supersedes the manifest template `$plan-parallel-work` used to carry.
