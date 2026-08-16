# L2 / C2 — authoritative gameplay checkpoint

Read `../manifest.md` and `../maps/gameplay-authority.md` completely. If refreshed source
disagrees with either, stop and escalate to the coordinator.

## Locked decisions binding this lane

- “`HostCampaignCheckpointV2` stores authoritative units, positions, factions, shipped
  archetypes, lattice state, downed state, persistent-effect ledger state, party
  formation, scenario/content/rules/seeds, and active play time.”
- “A Campaign checkpoint contains no reconnect or invite credential, online/store
  identity, transport endpoint or entity id, camera, local UI state, or selection.”
- “World and gameplay candidates validate completely before activation. Any malformed,
  incompatible, or incomplete candidate fails closed and leaves no partially restored
  world or actor state.”

## Work

Create the gameplay-owned Campaign authority adapter and exact persistent-effect ledger
snapshot/restore API. Export authoritative actor state without querying map-private state.
Validate roster, faction, archetype, lattice, position/footing input, effect ids and
references, downed state, and formation completely before mutating anything. Keep replica
projection and authority checkpoint APIs distinct.

Do not own the Campaign document, file I/O, world import, lobby/session flow, or UI. If
extracting existing V1 unit helpers, touch only the region listed in the manifest and leave
the legacy schema decodable. Update only L2's manifest row.

Run the selector-selected full gate. Evidence is logic-only and must include exact
export/teardown/import equality plus transactional refusals.
