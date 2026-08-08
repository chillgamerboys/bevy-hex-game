---
name: dispatch
description: "Coordinator loop for a wave: run up to three implementation agents in parallel isolated worktrees, keep every slot full, review each returned diff, and merge each lane into wave/<slug> serially through /audit-pr and /merge-pr. Consumes the dispatch queue /plan-epic commits. Never merges to dev or main."
---

# Dispatch a wave

Run a wave: several implementation agents in parallel worktrees against one repo, with one
coordinator that dispatches, reviews and merges. **The coordinator writes no implementation
code.** Its scarce resource is not tokens — it is **wall-clock with an idle slot in it**.

**`/dispatch` never merges to `dev` or `main`.** Every lane PR targets `wave/<slug>`. The
single `wave/* → dev` merge is the coordinator's separate act, carrying the combined head's
own exact-head runtime classification; `dev → main` is `/promote` only. This is the hardest stop
in the skill.

Four hard rules govern every step:

1. **Dispatch order ≠ merge order.** A lane queued behind another *for merging* still
   dispatches now. The only true dispatch blockers are file-ownership overlap with a
   RUNNING lane, an unmade decision, a missing artifact, and the slot cap.
2. **Every free slot is accounted for.** At each wake-up a free slot names either its next
   lane or its blocker. An idle slot with no named blocker is a protocol violation, not a
   lull.
3. **The worker's report is the only channel.** Anything not in the final report (or an
   on-ticket comment) dies with the agent.
4. **The repo has other authors.** The other owner's PRs open and land while your wave
   runs. Briefs carry the world as of dispatch, wake-ups re-sweep it, and reviews reconcile
   against the world *now* — not the one the order was written against.

Copy this checklist and track progress:

```
Dispatch Progress:
- [ ] Step 0: Load the queue + pre-flight
- [ ] Step 1: Order-first verification (per lane, at dispatch)
- [ ] Step 2: Compose the worker brief
- [ ] Step 3: Create the branch and worktree, launch up to 3 workers
- [ ] Step 4: SLOT AUDIT + territory sweep — every wake-up
- [ ] Step 5: Review the report; reconcile against the world now
- [ ] Step 6: Composed-tree check, then merge serially into wave/<slug>
- [ ] Step 7: Backfill immediately; close the wave
```

## Step 0 — Load the queue + pre-flight

The input is the **dispatch queue** in `docs/planning/waves/<slug>/manifest.md`. Its field
set is defined once, in
[wave-protocol.md §3](../../../docs/development/wave-protocol.md) — read it there. Do not
restate the table here; one copy and four pointers is the whole point.

`/plan-epic` emits exactly that shape alongside the committed wave artifact. An ad-hoc
queue is fine if the two blocker lists stay **separate fields** — one ordered list silently
collapses "must merge after" into "must start after," the under-utilization bug this skill
exists to prevent.

The queue is a **living list**, not a fixed plan: re-read it at each wake-up. Work
discovered mid-wave enters through `/inject`, which appends in the same shape and updates
the manifest to match.

A **mixed-ownership** wave carries lanes a human builds themselves (`builder: @<login>`).
They count in the ownership union and in every sweep — they are territory — and they never
dispatch and never occupy a slot.

Pre-flight, once per wave:

1. Working tree clean; `git fetch origin --prune` fresh — and run the wave's first
   territory sweep (Step 4) here, so t=0 territory is observed rather than inherited from
   the plan.
2. The union of all `owns` entries has no *unannotated* overlap. Where two lanes share a
   file, the queue must name the disjoint regions AND the composed end-state — else STOP
   and fix the queue.
3. Confirm `wave/<slug>` exists on the remote and was branched from a known `origin/dev`
   SHA that the manifest records.
4. Name the **machine-global resources**: the GPU and display (`/visual-walk`, `cargo dev`,
   `cargo editor`), the human playtest, and disk for three cold build directories. They are
   the coordinator's alone.
5. Confirm the **bootstrap block** (Step 2.4) works in a fresh worktree — run it once by
   hand.
6. If the wave artifact is still in **review mode** — an open draft PR carrying unruled
   decisions — the wave does not start. Those decisions are dispatch blockers on every lane
   they bind, and N workers reading an argued doc produce N readings of it.

## Step 1 — Order-first verification (at dispatch, per lane)

**The work order is the binding artifact, not the ticket.** Before launching a lane's
worker, verify its order exists under `docs/planning/waves/<slug>/orders/`, that its map's
`file:line` anchors still resolve, and that its preconditions hold against the tree as it
is *now*, not as the plan assumed: the lanes it claims are merged, the symbols it claims
exist, the counts it quotes. A worker should not have to escalate anything you could have
checked in thirty seconds.

Verify branch assumptions against the wave, not the workspace. A type introduced by the
wave's foundation lane exists on `origin/wave/<slug>` and not on `dev`, so
`git grep <symbol> origin/wave/<slug>` is the check that answers "does this exist"; asking
the workspace checkout produces a false "I'm blocked."

When a Linear connector is available, also verify the lane's `HEX-N` exists, names this
order, and move it to the live In Progress equivalent, resolving the state by returned name
and type rather than a stored identifier. Re-read it against current reality: text written
days earlier names modules a merged PR has moved or deleted. Corrections ride the brief as
explicit **correction notes**, never silent omissions. **A missing or unavailable ticket is
a visible warning and never blocks dispatch** — a lane keyed on its lane id is a normal
lane. A ticket assigned to the other owner is stronger than one of ours: its banked design
is LAW for the worker, quoted verbatim, and gets an on-ticket heads-up with a stand-down
offer before you dispatch over it.

## Step 2 — Compose the worker brief

A brief is **self-contained** — the worker has none of your context and cannot ask a
follow-up cheaply. Every brief carries:

1. **Context and the banked map.** The exploration already done, with `file:line`
   references, as an authoritative map — plus the rule: *if reality disagrees with the map,
   that is an escalation, not a judgment call.*
2. **The decisions that bind it**, quoted verbatim from the manifest — not summarized, not
   linked.
3. **The WORLD-STATE block**: the repo as of *this* dispatch, not the plan's — the
   `origin/wave/<slug>` tip SHA, what has merged into the wave since the map was banked,
   the other owner's PRs in flight with their footprints, siblings' region splits in shared
   files, and which handoff contract to verify rather than assume. Without it a late rebase
   reads to the worker as a conflict, not a design interaction.
4. **The bootstrap block**, run in the fresh worktree:

   ```sh
   git remote get-url origin          # must name bevy-hex-game — STOP on mismatch
   cat rust-toolchain.toml            # 1.97.1, pinned; an older stable fails the tree
   cat .cargo/config.toml             # BEVY_ASSET_ROOT must resolve to THIS worktree
   python3 tools/test_scope.py plan --base origin/wave/<slug> --head HEAD
   ```

   Everything runs through cargo. **Do not set a shared `CARGO_TARGET_DIR`**: cargo takes
   an exclusive lock on a build directory, so sharing one across worktrees serializes the
   whole wave. Each worker pays its own cold Bevy build; that cost is why the cap is three.

   The last command is the self-check, and it replaces the seed's break-one-line test: it
   must name *this lane's* paths and its selected concerns. If it reports
   `fail-closed-empty-diff` or someone else's paths, the worktree or the base is wrong and
   the gate the worker is about to trust is measuring the wrong tree.
5. **File and region ownership** — which files, and where two lanes share one, which hunks
   and what the composed end-state is.
6. **Test and fence dispositions** — per load-bearing test the lane touches: retire,
   retarget or keep, with the reason. Without it a worker either breaks a fence or refuses
   to touch one it should. Never disposition a fence as retire without naming the claim
   that dies with it, and say in the order that the fence going red first is it working.
7. **The evidence class** for this lane, and the deferral: a lane's manual runtime
   sign-off belongs to the combined wave PR.
8. **[The worker prompt contract](references/worker-contract.md)**, verbatim.

## Step 3 — Create the branch and worktree, launch up to 3 workers

**The coordinator creates the lane branch off `wave/<slug>` and the isolated worktree
before launching.** The worker then never runs checkout, switch, branch creation, or
rename — which is what makes `/create-pr`'s Conductor rule satisfiable inside a dispatched
lane. Record the branch in the lane's `branch` field.

**Root the worktree deliberately**: an isolated dispatch resolves its repo from the
coordinator's CWD, not from the order's prose, so verify the target repo in the same turn
as the launch. Size model and effort per lane from its `sizing` field, and send independent
launches in **one batch** so they actually run concurrently.

Three is the cap. Past it, workers finish faster than the one serial reviewer-merger lands
them and the queue converts into rebase debt — and three cold Bevy build directories is
already tens of gigabytes. Lower it when the machine says so; never raise it to hide a slow
first build.

## Step 4 — SLOT AUDIT + territory sweep (every wake-up)

A wake-up is **every** worker report and **every** merge. Print:

```
slots: <in-flight>/3
  slot 1: <id> running (<n>m)
  slot 2: FREE → dispatching <id>
  slot 3: FREE → blocked: <named blocker>
```

- A free slot with no named blocker is a **protocol violation**. Fix it in that turn — same
  fail-loud discipline as a test fence.
- "Blocked for merge" is almost never "blocked for dispatch." Check a candidate blocker
  against `dispatch_blockers` only; merge order is Step 6's problem.
- **Legit-defer** is a real category and naming it satisfies the audit: a lane that is
  *cheap* AND *input-unstable* AND *off the critical path* defers with that reason stated.
  Anything else needs a blocker, not a preference.
- **New lanes appear here legitimately.** A lane the queue did not have at t=0 is not
  queue-jumping — it is `/inject`'s output, and it arrives probed, collision-checked and
  artifact-recorded. Audit it like any other: read its `dispatch_blockers` in isolation.
- **`builder: @<login>` lanes are not slots.** They appear in this audit as territory —
  footprint and merge relationship — and never as in-flight work. Counting one against the
  cap silently shrinks the wave; dispatching one puts a worker inside a human's open edit.
- **Territory sweep, same turn.** Re-run `gh pr list --state open` across **all** authors
  and `git fetch origin --prune`, reading the new-branch lines rather than discarding them.
  A plan-time sweep goes stale within hours, and a branch shows up before its PR does —
  either can change a merge order you already reasoned about. The other owner runs the game
  constantly against `dev`; their map PRs land there while your wave runs.
- Review and merge duties never block dispatch. They interleave.

## Step 5 — Review the report; reconcile against the world now

Per returned worker, before anything merges:

1. Read the **diff**, not the summary — the report is the claim, the diff is the evidence.
2. Check the lane's diff against its declared `owns`. A path outside it is a blocker even
   when the change is correct; `/audit-diff` lens 9 covers this, but you own the
   disjointness union.
3. Check the **deviations**. A worker correcting a stale claim and reporting the delta is a
   good worker; one that silently obeyed a wrong instruction is the expensive one.
4. Triage **escalations** yourself — most are real findings a guessing worker would have
   shipped as bugs — and file the **out-of-scope debt** now, while it still has a finder.
5. **Broadcast hazards the same turn.** A report surfacing an environment or gate-validity
   hazard (a scope plan measuring the wrong tree, a fixture reading the clock) goes to every
   in-flight sibling immediately via `SendMessage`, not into the close-out.

Then **reconcile**: the order was written against an older world, and its green is a
statement about that one. Between the report and the merge —

(a) **What landed since.** If the worker's HEAD lacks the newest merge touching ANY file it
    shares — a sibling's or the other owner's — send it back for a rebase round and fresh
    gates. `MERGEABLE` is a claim about text, not about composed behaviour.

(b) **Read every removed line** in the diff, not just the added ones:

```sh
git diff origin/wave/<slug>...HEAD | awk '/^--- a\//{f=$2} /^-[^-]/{print f": "$0}'
```

A worker on a stale base can rewrite a file wholesale and revert a deliberate default
inside it — additive-looking, green on every gate, and invisible in any summary.

(c) **Make the worker introspect a composed same-file change** — explain the *interaction*,
    not re-run green. A registration list one lane empties while another's loader adds
    entries to it is green either way, and the pair changed what the fence tolerates.

## Step 6 — Composed-tree check, then merge serially into `wave/<slug>`

Merging is single-owner and serial. Before merging the **second** of two PRs on one
surface: rebase it onto the merged first, then run that surface's own gate on the
**combined** result.

File-ownership maps cannot see a **new** file importing a **deleted** one: each PR is green
in its own worktree and the pair is red on the wave. Opposite-direction edits to the *same
lines* slip file-level ownership the same way; rebase deliberately, to the brief's composed
end-state.

**Audit in the lane's worktree, not yours.** `/audit-pr` requires local `HEAD` to equal the
PR head and a clean tree, which your own workspace cannot satisfy for someone else's lane.
Run `/audit-pr` there, then `/merge-pr` with `--base wave/<slug>`. The coordinator merges;
workers never do.

Read only `overall_status`, `head_sha`, `pr_number`, and `base_branch` from the receipt.
**Do not restate the receipt schema here** — `/audit-pr` is its writer and `/merge-pr` is
its reader of record, and a third copy triples an existing drift hazard.

**After every lane merges, run the composed check on the wave head:**

```sh
python3 tools/test_scope.py plan --base origin/dev --head origin/wave/<slug>
```

then run the concerns it selects. This is **mandatory and has no CI backstop**:
`.github/workflows/ci.yaml` runs its `push` jobs only on `main` and `dev`, so nothing
validates the wave branch between lane merges. It stays mandatory even for a single-lane
merge — worker-green is not coordinator-green for any environment-dependent test. This is
the cheap selector-chosen run, not the complete candidate gate; that tier runs once, on the
final wave PR.

**Ping before you land on the other owner's hotspot.** Shared registration lists, config
manifests and append-only ledgers serialize everyone. Before merging anything that shares
one with their open PR, tell them and state the composed end-state you are creating. The
ping buys the author's review of the composition, which no gate checks.

## Step 7 — Backfill immediately; close the wave

The moment a slot frees and a dispatchable lane exists, dispatch it — never batch
dispatches into check-ins, never wait to be asked. Return to Step 1 for that lane; when
nothing is dispatchable, the slot audit says *why*.

Close the wave with the Report below. Then run the serialized legs workers were forbidden
from running: `/visual-walk` on the composed head for affected presentation, and the
combined selector gate. **Then stop.** Taking the wave to `dev` needs the combined head's own
exact-head runtime classification — a named human's playtest for a changed presentation or
experience surface, a verified-maintainer `N/A` naming the hook closure for a logic-only
wave — and it is not this skill's step.

## Coordinator safety rules

These all live on the coordinator's side of the loop — no worker catches a mistake made
here.

- **Check the receipt in one turn; merge in the next.** A receipt check batched into the
  same shell command as the merge cannot stop it: the gate only gates if its exit code is
  READ before the merge command exists.
- **Receipts carry full 40-char SHAs.** Pass `$(git rev-parse HEAD)`, never an
  abbreviation — a short SHA reads as a mismatch at the gate, which is a green audit
  reported as a stale one. Never hand-author a timestamp.
- **Worktree dispatch roots from the coordinator's CWD**, so verify the target repo
  immediately before launching, and require the worker's bootstrap to assert
  `git remote get-url origin` and STOP on a mismatch. An order for one repo has landed in
  another repo's worktree; only the worker's refusal kept it from being written there.
- **Never merge a wave loop into `dev` or `main`.** Not as a shortcut for a tiny lane, not
  because the wave is nearly done, not because CI is green.
- **The serialized visual and human pass is load-bearing, not ceremony.** No hermetic gate
  in this repo can see a black sky, a gap between tiles, or a piece sunk into terrain.
- **Never `pkill` a running game or rebuild over one.** The operator may have an instance
  open; killing it looks like a crash.
- **Owner etiquette outranks speed.** Heads-up before touching shared work; a "do not merge
  — human review requested" note gets an explicit on-PR retraction citing the ruling
  *before* the merge. A review comment on a design question inside someone else's crate is
  an argument, not a veto — but a contract bug or a broken boundary blocks.
- **Verify-not-rework is a valid brief.** Pointing a worker at the other owner's PR to
  *prove* its load-bearing claims — empirically, both directions — is review-grade evidence
  with no edit to their code.

## Report

```
=== /dispatch wave complete ===
Wave wave/<slug> · slots 3 · dispatched N · merged M · blocked B
Wave head <sha> · composed gate <result>

| Lane | Ticket | PR | Outcome |
|---|---|---|---|

Escalations resolved: <each, with the ruling>
Deferred: <legit-defer lanes + reason>
Debt filed: <issues opened from worker reports>
Serialized legs run by the coordinator: <list>
NOT DONE: wave/<slug> -> dev needs its own exact-head runtime classification at <sha>.
```

## When NOT to invoke

- **A single ticket.** That is `/plan-ticket` → implement → `/create-pr`. One agent with
  your full context beats a brief.
- **Fewer than about three independent lanes.** Briefs, slot audits and composed checks
  cost more than the parallelism wins. That threshold is a **recommendation, not a gate**.
- **One new lane for a wave already running** → `/inject`.
- **Genuinely sequential lanes** — a chain, not a wave. Stacked work, at most two deep.
- **Exploration.** Waves execute banked judgment; without a map, dispatching produces N
  contradictory ones.
- **A wave with no committed manifest.** A plan in `.context/` or in chat is invisible
  inside a worker's worktree.

## Troubleshooting

**Slots keep idling.** The queue encoded merge order as dispatch order. Re-read each
`dispatch_blockers` in isolation; most are empty.

**Two workers touched the same lines.** The ownership map named files, not regions. Rebase
to the composed end-state; fix the queue.

**Composed tree red, both PRs green.** Almost always a new file referencing something the
sibling deleted. Merge the first, rebase the second, re-run the gate.

**A worker's first build takes twenty minutes.** Expected: a fresh worktree has a cold Bevy
build directory. Do not raise the cap to hide it, and do not share `CARGO_TARGET_DIR` to
"fix" it — cargo's exclusive build lock will serialize the wave instead.

**A worker reports a plain blue window.** Assets were not found. Its worktree is missing
`.cargo/config.toml`, or it ran the binary outside cargo, so `BEVY_ASSET_ROOT` never
resolved. Not a content bug.

**A worker's scope plan reports `fail-closed-empty-diff`.** Its base is wrong — usually
`dev` instead of `origin/wave/<slug>`. Every gate it then runs is measuring the wrong tree.

**A worker's report is thin.** It was dispatched without the report contract; that
information is gone. Re-derive from the diff.

**A dispatch mislocated, or a worker cannot deliver mechanically.** Run the **two-worker
relay** instead of re-deriving: the worker that did the analysis ships it as `/tmp`
artifacts plus an `APPLY.md` recipe naming each target path and each file's `sha256`; a
fresh worker, dispatched in the correct root, verifies the hashes and does the apply pass.
The judgment is the expensive half — move it, don't repeat it.

## Self-updating

- **A new failure class costs a wave** (a composition shape, an undeclared shared resource)
  → add it to the safety rules or Troubleshooting *before* fixing the instance.
- **A coordinator action goes wrong that no worker could have caught** → it belongs in the
  safety rules, not in a brief. Workers cannot fix the coordinator's turn.
- **A worker instruction is repeatedly missed** → move it into
  [the fenced contract](references/worker-contract.md); brief prose gets skimmed.
- **The queue's field set changes** → change it in
  [wave-protocol.md §3](../../../docs/development/wave-protocol.md) and nowhere else. This
  skill, `/plan-epic`, `/inject`, and `$plan-epic` all read it from there.

## Provenance

Ported from the jxp-skills seed's `/dispatch`, adapted to this repository's wave model:
lanes merge into `wave/*` rather than the integration branch, the coordinator audits in the
lane's worktree because `/audit-pr` requires an exact-head clean tree, Linear is soft, and
the queue's field table lives in `wave-protocol.md` instead of being mirrored across three
skills. The seed's incidents are not recorded here; its judgment is.
