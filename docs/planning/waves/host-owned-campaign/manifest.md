# Host-owned Campaign multiplayer wave

- **Status:** dispatching; L1/L2/L3 are merged and L4 is the active lane
- **Wave branch:** `wave/host-owned-campaign`
- **Planning base:** `origin/dev@a0f95e62d02c663902b864cc08a89e831d9ba437`
- **Wave base:** `origin/dev@c8506a71166a23777d31cc8504a53e61966bb069`
- **Coordinator:** `@shrav-k`
- **Epic:** user-approved Seamless Cross-Store Multiplayer plan, 2026-08-12
  (`ticket: null`)
- **Outcome:** a listen host can persist and resume a complete authoritative Campaign,
  open a fresh six-seat assignment lobby, and save manually during safe paused
  exploration; clients never own the save.
- **Exclusions:** EOS identity/lobby/P2P behavior, Steam integration, combat saves, host
  migration, transport secrets/ids in saves, cameras, selections, and preserving prior
  seat assignments across a resume.

## Why this is a wave

Complete Campaign persistence spans world-owned generator-neutral restoration,
gameplay-owned unit/lattice/effect authority, shared persistence/session orchestration,
and shared presentation. Those authorities can be implemented independently against a
shared checkpoint contract, but only the combined tree can prove process teardown,
restore identity, a fresh multiplayer lobby, and host-only save behavior. This is a wave,
not four independent PRs. The lanes dispatch from the same stable foundation and merge in
semantic order.

The later Universal Online wave is intentionally not mapped here. It requires a fresh
territory sweep after this Campaign wave lands. The optional Steam adapter remains a
stack after Universal Online.

## Locked decisions

1. **Authority and save owner.** “The listen host owns simulation, world mutation, and
   Campaign saves. Clients submit intents and render authoritative projections; a client
   never writes or owns the Campaign record.”
2. **Save boundary.** “Campaign saving is manual, host-only, and permitted only during
   quiescent paused exploration. Combat saving remains excluded.”
3. **Complete world.** “`HostCampaignCheckpointV2` stores the complete generator-neutral
   `WorldSnapshotV1`; regeneration from a seed or generator plan is not a restore path.”
4. **Complete gameplay state.** “`HostCampaignCheckpointV2` stores authoritative units,
   positions, factions, shipped archetypes, lattice state, downed state, persistent-effect
   ledger state, party formation, scenario/content/rules/seeds, and active play time.”
5. **Explicit exclusions.** “A Campaign checkpoint contains no reconnect or invite
   credential, online/store identity, transport endpoint or entity id, camera, local UI
   state, or selection.”
6. **Fresh session on resume.** “Resuming creates a fresh session instance, lobby,
   reconnect credentials, and seat assignments. It does not assume the same people
   return.”
7. **Compatibility.** “Legacy and V1 Campaign records remain preserved through the
   existing strict compatibility path and upgrade only after the next successful V2
   save; invalid data is never silently overwritten.”
8. **Transactional restore.** “World and gameplay candidates validate completely before
   activation. Any malformed, incompatible, or incomplete candidate fails closed and
   leaves no partially restored world or actor state.”
9. **Transport neutrality.** “Campaign persistence contains no Direct, EOS, or Steam
   transport fact. Direct/LAN and future EOS sessions consume the same fresh assignment
   and authoritative checkpoint contracts.”
10. **Current rules.** “One global exploration/combat `Mode`, one active turn, no
   prediction, no rollback, and no host migration remain unchanged.”

### Coordinator amendment — Campaign launch and save status (2026-08-12)

Ratified by `@shrav-k` while activating L3:

- “`SessionManifestV1` explicitly distinguishes a reproducible Sandbox launch from a
  Campaign launch. A Campaign client waits for and transactionally imports the host's
  complete live baseline; it never regenerates the Campaign world.”
- “Campaign save progress is an ordered, disclosure-safe server event scoped to the
  session and a monotonic operation id. It carries no checkpoint, slot, path, transport
  fact, credential, or player/store identity.”

Decisions are amendable, never silently edited. An amendment records its ratifier and
date, and every affected order receives the exact new text.

## Source maps

- [World authority](maps/world-authority.md)
- [Gameplay authority](maps/gameplay-authority.md)
- [Session and UI](maps/session-and-ui.md)
- [Territory](maps/territory.md)

If refreshed source disagrees with a banked map, stop and escalate to the coordinator;
do not reinterpret the map inside a lane.

## Ownership map

- **L1 / C1 world checkpoint:** map-owned validation, bootstrap import, publication, and
  complete world round-trip fences only.
- **L2 / C2 gameplay checkpoint:** gameplay-owned unit/lattice/effect/formation export and
  transactional restore adapters only; it never queries `VoxelMap` or map-private types.
- **L3 / C3 Campaign session:** shared save document, migration, fresh multiplayer lobby,
  host-only save/resume orchestration, and typed status; it consumes owner exports rather
  than reconstructing their facts.
- **L4 / C4 Campaign UI:** pure models, UI hierarchy, immutable view projection, and
  app-facing typed intentions; it creates no save or session authority.

`manifest.md` is the expected shared hotspot. Every lane owns only its own queue row and
must merge the refreshed wave before changing that row. `crates/hex_game/src/save.rs` is a
named regional hotspot: L2 may extract the existing unit restore helpers into its new
gameplay adapter; L3 owns the save document, migration, I/O, and orchestration after
refreshing from merged L2; L4 may edit only immutable notice/view projection regions after
L3. The composed end state has one V2 persistence path calling, not duplicating, the two
owner adapters.

## Dispatch queue

```yaml
- id: L1
  title: C1 complete world checkpoint restore
  order: orders/L1-world-checkpoint.md
  ticket: null
  authority: world
  builder: worker
  branch: worker/campaign-world-checkpoint
  owns:
    - crates/hex_map/src/world_snapshot.rs (Campaign validation/export API only)
    - crates/hex_map/src/grid.rs (OnEnter Campaign bootstrap restore only)
    - crates/hex_map/src/lib.rs (alphabetical checkpoint exports only)
    - crates/hex_map/tests/contracts/world_snapshot.rs (Campaign round-trip fences)
    - docs/planning/waves/host-owned-campaign/manifest.md (L1 queue row only)
  dispatch_blockers:
    - EOS feasibility/shared-contract foundation is merged to dev and wave base is recorded
    - refreshed source agrees with maps/world-authority.md
  merge_blockers: []
  fences:
    - path: crates/hex_map/tests/contracts/world_snapshot.rs
      disposition: keep
      reason: proves complete generator-neutral world identity and transactional refusal
  selector:
    concerns: [selector, rules, trajectory_contracts, contracts, simulation, app, map_unit, map_generation, map_contracts, residual, clippy, docs, shipping]
    full: true
  evidence: logic-only
  sizing: { model: gpt-5.6-sol, effort: high }
  state: merged
  pr: 203

- id: L2
  title: C2 authoritative gameplay checkpoint
  order: orders/L2-gameplay-checkpoint.md
  ticket: null
  authority: gameplay
  builder: worker
  branch: worker/campaign-gameplay-checkpoint
  owns:
    - crates/hex_game/src/campaign_authority.rs
    - crates/hex_game/src/lib.rs (campaign_authority module declaration only)
    - crates/hex_combat/src/effects.rs (authority ledger snapshot/restore API and tests only)
    - crates/hex_game/src/save.rs (existing unit restore helper extraction only)
    - crates/hex_game/tests/gameplay_app.rs (gameplay checkpoint fences only)
    - docs/planning/waves/host-owned-campaign/manifest.md (L2 queue row only)
  dispatch_blockers:
    - EOS feasibility/shared-contract foundation is merged to dev and wave base is recorded
    - refreshed source agrees with maps/gameplay-authority.md
  merge_blockers: [L1]
  fences:
    - path: crates/hex_game/tests/gameplay_app.rs
      disposition: keep
      reason: proves roster/lattice/effect/formation identity and fail-closed restore
  selector:
    concerns: [selector, rules, trajectory_contracts, contracts, simulation, app, map_unit, map_generation, map_contracts, residual, clippy, docs, shipping]
    full: true
  evidence: logic-only
  sizing: { model: gpt-5.6-sol, effort: high }
  state: merged
  pr: 204

- id: L3
  title: C3 host-owned Campaign session lifecycle
  order: orders/L3-campaign-session.md
  ticket: null
  authority: shared
  builder: worker
  branch: worker/campaign-session
  owns:
    - crates/hex_game/src/save.rs (V2 document, migration, I/O, owner-adapter orchestration)
    - crates/hex_game/src/screens/multiplayer.rs (Campaign host/resume handoff only)
    - crates/hex_game/src/storage.rs (Campaign storage path/atomic I/O only if required)
    - crates/hex_game/tests/gameplay_app.rs (Campaign save/resume/session fences only)
    - docs/planning/waves/host-owned-campaign/manifest.md (L3 queue row only)
  dispatch_blockers:
    - EOS feasibility/shared-contract foundation is merged to dev and wave base is recorded
    - refreshed source agrees with maps/session-and-ui.md
  merge_blockers: [L1, L2]
  fences:
    - path: crates/hex_game/src/save.rs
      disposition: keep
      reason: preserves strict legacy/V1 compatibility while making V2 the next-write format
    - path: crates/hex_game/tests/gameplay_app.rs
      disposition: keep
      reason: proves host-only quiescent save, process restart, and fresh lobby assignment
  selector:
    concerns: [app, clippy, docs, shipping]
    full: false
  evidence: logic-only
  sizing: { model: gpt-5.6-sol, effort: high }
  state: merged
  pr: 205

- id: L4
  title: C4 multiplayer Campaign UI
  order: orders/L4-campaign-ui.md
  ticket: null
  authority: shared
  builder: worker
  branch: worker/campaign-ui
  owns:
    - crates/hex_gameplay_model/src/multiplayer.rs (Campaign routes/transitions only)
    - crates/hex_ui/src/model.rs (Campaign multiplayer view/intent fields only)
    - crates/hex_ui/src/lib.rs (alphabetical Campaign view exports only)
    - crates/hex_ui/src/multiplayer.rs (Campaign browser/save/resume rendering only)
    - crates/hex_game/src/screens/multiplayer.rs (immutable Campaign view/intent adapter only)
    - crates/hex_game/src/save.rs (immutable save-status projection only)
    - crates/hex_ui/src/review.rs (Campaign multiplayer fixtures only)
    - crates/hex_game/src/walk.rs (Campaign multiplayer fixture-name registration only)
    - walks/multiplayer_session.ron (Campaign frames only)
    - docs/planning/waves/host-owned-campaign/manifest.md (L4 queue row only)
  dispatch_blockers:
    - EOS feasibility/shared-contract foundation is merged to dev and wave base is recorded
    - refreshed source agrees with maps/session-and-ui.md
  merge_blockers: [L3]
  fences:
    - path: walks/multiplayer_session.ron
      disposition: keep
      reason: extends the existing multiplayer presentation walk with Campaign states
    - path: crates/hex_ui/src/multiplayer.rs
      disposition: keep
      reason: retains Direct/LAN as a visible advanced path while adding Campaign flow
  selector:
    concerns: [app, clippy, docs, shipping]
    full: false
  evidence: static-presentation
  sizing: { model: gpt-5.6-sol, effort: high }
  state: in-review
  pr: null
```

## Territory sweep

The full measured sweep is banked in [maps/territory.md](maps/territory.md). At planning
time, `origin/dev` is `a0f95e62d02c663902b864cc08a89e831d9ba437`. The only foreign open
PR is #196 at `25d0be5d9a492d5c3ef679c087c126b51db722a9`: four files,
`+528/-41`. Its substantive lattice files overlap no Campaign lane; its one-line
`crates/hex_game/src/lib.rs` composition edit is an annotated L2 hotspot. Refresh before
foundation landing, wave cut, every lane merge, and the final wave PR.

The live Linear sweep found no non-terminal Campaign multiplayer, EOS, or Steam issue.
Lane tickets therefore remain deliberately `null`; the manifest is the durable queue.
`HEX-95` is an independent Main Menu heading-inset bug and is not absorbed into this wave.

## Integration order

1. Land the behavior-neutral EOS/shared checkpoint foundation on `dev` through its own PR.
2. Record the exact resulting `origin/dev` SHA, cut `wave/host-owned-campaign`, and cut all
   four lane branches from that exact wave head.
3. Dispatch all lanes whose source maps still agree. Merge L1, then L2, then refresh and
   merge L3, then refresh and merge L4.
4. After each lane merge, re-plan and run the selector-chosen composed concerns against
   `origin/dev`; inspect removed lines before accepting the merge.
5. Reconcile implementation/status/roadmap/contracts and run the final combined gate once
   on the wave PR to `dev`.

## Combined acceptance

- Authored, V1, V2, V3, stacked/cave, and mutated/damaged worlds survive export → process
  teardown → import with identical complete public fingerprints and private damage state.
- Malformed, oversized, duplicate, unsorted, unknown-material, impossible-health,
  dangling-region, wrong-version, and wrong-fingerprint inputs fail transactionally.
- Unit identity, faction, shipped archetype, exact footing, lattice, downed state,
  persistent-effect ledger, formation, scenario/content/rules/seeds, and active time are
  identical after process teardown and restore.
- Legacy/V1 saves remain available or visibly Invalid under their existing strict
  compatibility rules; a successful next save writes V2 without silently touching another
  slot.
- Only the listen host can save, and only during paused quiescent exploration. A client
  receives typed “host is saving”/success/refusal status and never writes the file.
- Resume creates a new session instance and fresh assignment lobby with no persisted
  credential, online identity, transport id, camera, selection, or prior seat assignment.
- Direct/LAN single-player and multiplayer regressions remain green.
- The final exact head passes the complete selector-generated gate, strict Clippy,
  warning-denied docs, dependency/license checks, shipping build, and the supported
  platform matrix. Campaign UI frames are captured; logical save/restore claims use typed
  hooks. Any changed interaction/experience receives a named exact-head human `PASS`.

## Stop conditions

- A lane needs to query another owner's private implementation rather than consume a
  published snapshot/adapter.
- World import cannot reproduce complete public identity without regeneration.
- Gameplay restore mutates any actor before every candidate and footing check passes.
- A checkpoint contains a credential, online/store principal, transport id, camera,
  selection, or prior seat assignment.
- A client can initiate persistence or a save can occur outside quiescent paused
  exploration.
- Legacy/V1 data would be silently destroyed or an invalid record would be overwritten.
- Refreshed PR territory disagrees with a banked map or introduces unowned overlap.

On a stop condition, mark the lane blocked and amend this manifest after owner review;
do not improvise inside the lane.
