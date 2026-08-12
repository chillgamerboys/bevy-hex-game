# L1 / C1 — complete world checkpoint restore

Read `../manifest.md` and `../maps/world-authority.md` completely. If refreshed source
disagrees with either, stop and escalate to the coordinator.

## Locked decisions binding this lane

- “`HostCampaignCheckpointV2` stores the complete generator-neutral
  `WorldSnapshotV1`; regeneration from a seed or generator plan is not a restore path.”
- “World and gameplay candidates validate completely before activation. Any malformed,
  incompatible, or incomplete candidate fails closed and leaves no partially restored
  world or actor state.”
- “Campaign persistence contains no Direct, EOS, or Steam transport fact. Direct/LAN and
  future EOS sessions consume the same fresh assignment and authoritative checkpoint
  contracts.”

## Work

Implement the map-owned pending Campaign bootstrap/import path and typed result. Reuse the
existing canonical snapshot validator/preparer, keep private map resources private, and
publish the restored world through the ordinary `TerrainReady` path before actors are
restored. Add teardown/import identity and malformed-candidate transactional tests for all
world families named by the manifest.

Do not edit gameplay unit/combat state, save files, UI, session routing, or protocol
registration. Update only L1's manifest row with the PR/state.

Run the full selector-selected gate for this map snapshot seam. Evidence is logic-only;
typed fingerprints/resources establish the claims, not screenshots.
