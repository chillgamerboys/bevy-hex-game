---
name: inject
description: "Add newly discovered work to a wave that is already dispatching: probe the claim, collision-check it against the ownership map, in-flight lanes, and the other owner's territory, author a self-contained work order, append it to the dispatch queue, and either dispatch it into a free slot or name its blocker — without stopping the wave. Add-only."
---

# Inject work into a running wave

A running wave meets reality: a bug surfaces, a review turns up a finding, a probe
disconfirms an assumption. The work belongs to the wave, and the wave is already executing.
`/inject` adds it **without stopping the wave** — and without letting the manifest quietly
stop describing the territory.

Three hard rules:

1. **A probe comes before a ticket.** Injected work is born from a report, and reports name
   symptoms. The probe is what turns "movement is broken" into the actual defect — and it is
   routinely a different one. An injection without a probe is a rumour with a branch name.
2. **The queue is a living list.** New work appears in it mid-wave by design; `/dispatch`
   re-reads it at every slot audit. What must not change is its *shape* — the
   [§3 field set](../../../docs/development/wave-protocol.md), the two blocker lists still
   separate.
3. **The artifact absorbs the lane, in the same PR.** A manifest that omits an injected lane
   is a map lying about the territory, and every later collision check reads that map.

Copy this checklist and track progress:

```
Inject Progress:
- [ ] Step 0: Resolve the wave + locate the artifact
- [ ] Step 1: The grounding probe
- [ ] Step 2: Collision check — lanes, workers, territory, selector
- [ ] Step 3: Author the order (and the ticket, when Linear is up)
- [ ] Step 4: Append to the queue; dispatch or name the blocker
- [ ] Step 5: Artifact truth
```

## Step 0 — Resolve the wave + locate the artifact

Input is a wave reference plus a prose description of the new work. Resolve both before
anything else:

1. **The wave.** An explicit `/inject <slug> "<prose>"`, or the wave currently being
   coordinated. If the work does not belong to a running wave, stop — see
   [When NOT to invoke](#when-not-to-invoke).
2. **The artifact** — `docs/planning/waves/<slug>/manifest.md` (the durable half: locked
   decisions, the dispatch queue, the ownership map) and `orders/` (the transient half, one
   order per lane). Both are inputs to Step 2 and outputs of Step 5.

**If the wave is running artifact-light** — planned in a session, in a plan file outside the
repo, or under `.context/` — say so explicitly and **create the manifest as part of this
injection**. A wave planned into `.context/` is the likeliest case here, and it is
artifact-light by definition: it is untracked scratch, so nothing in it exists inside a
worker's worktree. The artifact is not optional: Step 2's collision check
has nothing authoritative to read without it, and each injection makes the unbanked state
more expensive. Seed it from what the wave already has — the live lanes, their builders,
their PRs — rather than reconstructing the original plan.

## Step 1 — The grounding probe

Before a ticket exists, **verify the claim yourself** — small and cheap: one run of the
narrowest test concern, one `rg` over the changed authority, one deterministic snapshot
diff, one visual-walk frame. Then put the probe and its output **verbatim** into the order
and the ticket.

Three things the probe decides, none of which prose can: **whether the reported thing is the
real thing** — a tile rendering wrong and a tile *being* wrong are different defects with
different owners, and this repo's whole evidence boundary exists because those two get
confused; **whether it is in scope for this wave**, which decides `/inject` versus a
standalone ticket; and **the evidence a worker inherits**, since the order quotes the probe
and the worker then starts from a verified fact rather than a summary.

Prove logical claims with typed hooks, state, messages, logs, snapshots, or deterministic
contracts. A frame may show you *where* to look; it never establishes the transition. If the
oracle is missing, the injection may well be "add the hook".

**If the probe disconfirms the report, do not inject.** The output is a comment on the
originating ticket recording what was actually observed. A lane is an expensive way to say
"not reproducible".

## Step 2 — Collision check: lanes, workers, territory, selector

Injected work enters a territory that is already claimed. Check the proposed ownership
against all four claimants:

1. **The manifest's ownership map** — the planned lanes, including ones not yet dispatched.
2. **Every in-flight worker's region.** These are the live ones; an overlap here is a real
   `dispatch_blocker`, not a merge blocker.
3. **Territory.** Re-sweep it — do not reuse the plan-time table. `gh pr list --state open`
   across **all** authors plus `git fetch origin --prune`, reading the new-branch lines
   rather than discarding them. A wave outlives its own sweep by hours, which is exactly the
   interval an injection lands in. The table's shape, the measured-footprint rule, and the
   region-relationship column are `/plan-epic` Step 2's; `/dispatch` Step 4 re-runs the same
   sweep at every wake-up. This step adds only *when*.
4. **The selector.** Read `.config/test-scopes.json` for the proposed paths. An injected
   lane touching a path that promotes to the complete gate changes the wave's gate
   economics — it can turn a cheap tail into a serial full-gate leg, and it is worth knowing
   that before the lane is queued rather than after.

**Crate authority is checked first.** An injected lane whose ownership crosses the
world/gameplay boundary is the same stop condition it would be at plan time. Discovered work
is *more* likely to cross it, not less, because it is scoped against a symptom rather than a
seam.

The result is either disjoint ownership, or a region split with a named **composed
end-state** and a hotspot rule. There is no third option, and an injection is the likeliest
place to skip it: the new lane is defined against an already-full map.

**A ticket authored or assigned to the other owner is territory too.** Injected work
frequently binds to one — a finding they filed and deliberately did not fix in flight. Their
banked design is **LAW** for the order, quoted verbatim including the shape they proposed
and the fences they specified; an on-ticket heads-up with a stand-down offer goes out
**before** dispatch, not after the PR opens.

## Step 3 — Author the order (and the ticket, when Linear is up)

**Order first.** The work order is the binding artifact; the ticket is coordination. Author
it at `docs/planning/waves/<slug>/orders/<id>-<slug>.md`, in the same shape every other
order in this wave uses — `/plan-epic` Step 6's section table is the spec. Four things
injection-specific:

- **Separate `dispatch_blockers` and `merge_blockers`.** Fresh work discovered late *feels*
  like it comes last; usually its dispatch blockers are empty and only its merge order is
  constrained. Audit the two fields independently.
- **Verified-current inputs.** Every fact the order lifts from ticket or report prose — a
  type name, a system set, a path, a count — is re-checked against the code **now**, and
  against `origin/wave/<slug>` rather than `dev` where the wave has already moved. Injected
  work is written against the freshest reality in the wave; do not let it inherit aged
  prose.
- **The world-state block**, as of *this* dispatch: the `origin/wave/<slug>` tip, what has
  merged into the wave since it opened, the siblings still in flight and their regions. The
  order is written mid-wave, so the world it names is several merges past the plan's.
- **A fence disposition table and named gates**, same as any other order. Discovered work
  often arrives without a written verification story; "found during the wave" is a reason to
  fence it harder, not a licence to skip.

Quote into it, verbatim, every locked decision from the manifest that binds it — including
any ruling the operator makes while scoping the injection. A ruling that stays in the
conversation binds nobody.

When Linear is connected, mint the child under the epic, carrying Step 1's probe verbatim
and saying plainly that it was **injected mid-wave**, with the reason: an epic's child list
is read later by people reconstructing why the scope moved. Resolve the team and the In
Progress state from live connector data and pass the resolved state identifier, never a
name or a stored UUID. When Linear is unavailable, leave `ticket: null`, key the lane on its
id, warn visibly, and continue — the injection proceeds.

## Step 4 — Append to the queue; dispatch or name the blocker

Append one row to the dispatch queue in the manifest, in exactly the
[§3 field set](../../../docs/development/wave-protocol.md), so the lane survives this
session.

Then, immediately:

- **A free slot and empty `dispatch_blockers` → dispatch now**, through `/dispatch` Step 3:
  the coordinator cuts the lane branch off `wave/<slug>` and creates the isolated worktree
  *before* launching, which is what makes `/create-pr`'s Conductor rule satisfiable inside
  the lane. Then send
  [the worker contract](../dispatch/references/worker-contract.md) unchanged. This is the
  backfill rule. An injection that waits for the next check-in is the under-utilization
  failure `/dispatch` exists to prevent, and it is *worse* here than in a planned wave: the
  coordinator is already awake, and the lane is the freshest-verified thing in the queue.
- **Otherwise the lane appears in the next slot audit with its blocker NAMED** — three full
  slots, an overlapping running region, an unmade decision. "Just injected" is not a
  blocker.
- **An injected lane may be a human's.** Where the work sits in the other owner's territory
  (Step 2), the lane is **offered, not assigned**: tag the row `builder: @<login>`, post the
  heads-up on the ticket, and let them claim it. Until they do it is neither dispatched nor
  a slot. Re-tagging it `builder: worker` after a stand-down is a one-line edit; re-doing
  their work is not.

## Step 5 — Artifact truth

The manifest lives on the wave branch once `/dispatch` Step 0 has cut it, so an injected
lane's artifact update is a commit on `wave/<slug>` by the coordinator — not a second PR to
`dev`, which would leave the new order absent from every lane worktree already checked out.
It gains:

- the **queue row** from Step 4, with its `state` and `builder`;
- its **ownership map** bullet, plus any hotspot rule Step 2 produced;
- its **territory** row, if the collision check moved one; and
- a one-line **injection note** in the injection log: the date, and what caused it.

The injection note is the row that pays off later. A wave whose scope grew twice, with no
record of why, cannot be reviewed at close-out and cannot be learned from — and the close-out
lane is the one that reads this document hardest.

## Report

```
=== /inject complete — wave/<slug> ===
Probe:     <the command + what it showed>
Lane:      <id> · authority <world|gameplay|shared> · ticket <HEX-N|none>
Order:     docs/planning/waves/<slug>/orders/<file>
Ownership: <paths/regions> — collision: none | <region split + end-state>
Selector:  <concerns> · full gate: <yes|no>
Queue:     appended · dispatch_blockers: <empty|named>
Status:    dispatched into slot <n> | queued (blocker: <named>)
Artifact:  manifest updated (queue row + ownership + injection note)
```

## When NOT to invoke

- **No running wave.** A wave that has not been decomposed goes to `/plan-epic`; a wave that
  has closed does not get reopened by an injection — plan the next one.
- **The work is a one-off unrelated to the wave** → `/plan-ticket`. Binding an unrelated fix
  to a wave buys it a work order it does not need and pollutes the close-out.
- **The probe disconfirmed the report** (Step 1) → a comment, not a lane.
- **A UI defect that has not been captured yet** → `/linear-ui-bug-intake` first. Intake owns
  reproduction, deduplication, and evidence; injection owns scheduling the fix.
- **Re-scoping or cancelling an existing lane** — explicitly out of scope. Killing a
  dispatched lane, re-cutting ownership under a running worker, or retiring a locked
  decision are manual acts; do them deliberately and record the amendment in the manifest.
- **The "injection" is really a second wave.** Three or more new lanes arriving at once is
  not an injection — stop the wave's growth and run `/plan-epic` on the new body of work.

## Troubleshooting

**The wave has no manifest.** Artifact-light waves are common when the wave was planned in a
session or under `.context/`. Create the manifest as part of the injection (Step 0), seeded
from the live lanes rather than reconstructed. Do not inject into an unbanked wave twice.

**No free slot.** That is a named blocker, and naming it satisfies the slot audit. Queue the
lane; `/dispatch` Step 7 backfills the moment a slot frees. Do not raise the cap to make
room — it exists because one serial reviewer-merger is the real ceiling, and because three
cold Bevy build directories is already what the machine will bear.

**The ticket is the other owner's.** Their design is LAW (Step 2), quoted verbatim into the
order, with the heads-up and stand-down offer posted before dispatch. Do not re-derive a
shape they already ruled on; if you disagree, that is a conversation on the ticket, before
the worker starts.

**The probe disconfirms the report.** Not an injection. Comment on the originating ticket
with what you actually observed — that comment is the deliverable, and it is worth more than
the lane would have been.

**The new lane overlaps a running worker's region.** A genuine `dispatch_blocker` — one of
the few. Queue it behind that worker rather than splitting a region under someone mid-edit,
and name the composed end-state now, while both scopes are in view.

**The injected lane crosses crate authority.** Re-cut it, exactly as at plan time. A
symptom-scoped lane crosses the world/gameplay boundary more often than a seam-scoped one.

## Self-updating

- **The queue's field set changes** → change it in
  [wave-protocol.md §3](../../../docs/development/wave-protocol.md) and nowhere else. This
  skill, `/plan-epic`, `/dispatch`, and `$plan-epic` all read it from there.
- **An injection shape recurs** (a probe pattern, a collision class) → add it to Step 1 or
  Step 2 rather than to a single order.

## Provenance

Ported from the jxp-skills seed's `/inject`, adapted to this repository: the order rather
than the ticket is the binding artifact, the world-state keys on the wave branch, the
collision check gains crate authority and the test selector as claimants, and probes are
repo-shaped and bound by the evidence boundary. Add-only, as upstream.
