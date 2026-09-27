# Grand V4 candidate

Grand V4 is a selectable streamed expedition built on `feat/water-lab` at
`873a2c37eeb8eda23c11d398ce7d68d7965a64db`. The integration branch preserves the
Forest, Northern Archipelago and Water Lab work in draft PRs #222 and #223 and
targets `dev`; it does not merge unrelated historical branch tips.

## Launch and package identity

Use the existing shared Cargo target directory when one is available:

```sh
python3 tools/grand_package.py ensure --target-dir /absolute/cargo-target
python3 tools/arena.py launch --map grand-v4 --target-dir /absolute/cargo-target
```

`HEX_GRAND_WORLD` or `--grand-world` selects a different compiled package. A package
is immutable: a changed authoring source requires a fresh output directory, never
overwriting an existing one. The [package guide](../../assets/config/v4/grand-v4/README.md)
documents mainland measurement, the independent Crystal enlargement, source
contracts, and the distinction between terrain relief and native rendered evidence.

The exact package manifest, site companion, biome companion and gameplay content
bind the resume slot. There is no Grand V3 migration. A rejected or corrupt save
remains preserved; explicitly confirmed New Run archives its head before the new
checkpoint replaces it. Generated packages, captures and save data are not committed.

## Playing and resuming

R activates each elemental shrine once. Shrine identities derive cumulative
bonuses alongside XP upgrades. Configuration lives in
`assets/config/arena/grand-v4.ron`; Earth changes running and the durability of new
player structures, never player health. Defeating Shadow earns X teleport to visible
supported ground within 12 units, with a six-second cooldown. Invalid destinations
do not start cooldown.

Continue restores the saved expedition. Save & Quit completes an atomic checkpoint
before exit. New Run requires an explicit confirmation. Death returns the player
to the last activated shrine, or the starting beach, preserving enemies, progress
and edits. Nearby valid support is used when the old support has been destroyed.
If that loaded area has no usable ground, recovery stages and loads the starting
beach. Pending recovery survives saving; it does not heal or advance the session
while required terrain is unavailable. If both bounded areas are unusable, the run
is retained with an explicit recovery notice.

The dedicated `grand-v4-resume` slot is beside application preferences and honors
`HEX_GAME_DATA_DIR`. It preserves the exact movement mode, boat/glider velocity,
charges and attacks, live/dead enemies and AI, progression, discoveries, clocks,
sparse destruction and partially damaged voxels. Ordinary autosaves use completed
simulation ticks; reward acquisition and respawn also trigger saves.

## Ownership and transaction boundary

- World publishes exact sites, compact collision, loading state and biome facts. Its
  checkpoint partitions contain sparse edits, health and transaction counters.
- Gameplay uses stable authored `u32` actor identities and an explicit Grand policy.
  Its checkpoint retains active and dormant encounter state. Rebinding waits for all
  required body and transient collision, then rejects invalid embedded poses.
- The application settles pending world acknowledgements without advancing the
  simulation, snapshots all owners at the same boundary, and commits through the
  existing atomic storage infrastructure. Encoding partition records and writing
  them run on a worker. Restore stages and validates every owner before play resumes.
- Water presentation, boats and swimmers consume the same wind and full environment
  clock. Inland water retains its local level. The accepted 65% glider wind influence
  and ordinary sailing momentum are preserved.

Source residency is bounded at 512 chunks and detailed presentation at 256 chunks.
The distant World Tree mesh comes from compact authored columns, yields to the
complete detailed object, and applies persistent removals after edits or restore.
Ground damage cannot erase the tree. Updating this mesh never pins source chunks;
it is bounded to 20,000 columns, 25,000 runs and 600,000 indexed vertices. The current
265-by-223.5-unit canopy uses 496,932 vertices across an 85-chunk footprint. It is
not collision or seabed authority. Local shrine and library lights illuminate the
opaque caves without changing global daylight or gameplay visibility. Final cave
contrast and readability remain part of fresh presentation review.

Ordinary forest trees also publish authored distant silhouettes, independent of
source-chunk residency. Shared crown/trunk meshes yield atomically to detailed
objects, retain persistent cuts and never pin source chunks. The bounded overview
contains 773 ordinary trees from 13 recipe families; its full intact geometry is about
1.41 million vertices against a 1.6-million limit. Ground edits do not erase trees.
The recipes reuse the earlier Dragon/Goblin expedition’s curved timber and layered
foliage, with irregular grove edges and understory. The garden now has an open
courtyard, planted beds and a shallow fountain rim; the library has arched wall
bays with colored books. Fresh composed-package review remains the acceptance
authority for their appearance.

The detailed terrain renderer owns water above mean sea level. The animated ocean
owns sea-level water; raised neighbors still occlude its boundary faces. This keeps
the garden, fountain, falls, valley lake and river visible without duplicate ocean
bottoms or sides.

## Reproducible acceptance

Use an explicit package and a disposable data directory for tests. Coordinate heavy
Cargo jobs against the shared target. The process harness builds current source and
launches separate writer/reader processes for land, boat and glider cases:

```sh
python3 tools/grand_verify.py --cargo-profile ci --circuit --admissions --walking --sailing \
  --package /absolute/grand-package --target-dir /absolute/cargo-target \
  --output /absolute/fresh-restart-evidence
python3 tools/grand_capture.py --package /absolute/grand-package \
  --target-dir /absolute/cargo-target --label fresh-candidate
```

`--circuit` also runs three repeated waypoint loops with actual streamed collision
and CPU-prepared terrain roots. It checks chunk budgets and edit persistence through
eviction/revisit. These paused interest relocations make no travel-speed, pixel,
process-memory or frame-rate claim.
`--admissions` visits all fourteen actual encounter sites, checks their exact
127 stable enemy identities and species, and verifies active/dormant checkpoint
integrity. The restart clock fixture also covers a run longer than sixty hours.
`--walking` exercises the current authored routes and independent broad-area probes through the real controller,
including independent forest crossings, riverbank escapes, and published stacked mountain/cave connections. It requires completed simulation ticks and clear supported endpoints.
`--sailing` measures the authored crossing with one ordinary B input, live wind and
unupgraded boat physics; its reported time is a measurement against the approximate
45-second design reference, not an assumed success.

The 43-view capture matrix is windowless and requires a clean committed candidate
and a package whose authoring signature matches current source. A `--view` subset
is explicitly a partial diagnostic, never a completed full matrix. It
records source and package hashes, renders, native state receipts and an initially
unreviewed index. Inspect every full-resolution frame and the contact sheet.
`--dirty-diagnostic` provides scratch evidence only. The matrix includes the whole
footprint, both cave routes, shrines and major landmarks, two bay directions,
baseline/new water, waterline and underwater views, and gameplay cameras.

Focused tests cover shrine permutations and duplicate rewards, teleport refusals,
compact-column party admission, dormancy, marine modes, checkpoint codecs, atomic
interruption recovery, the full library route, water descent, cave separation and
exact area measurements. The real-package map test
`actual_grand_fresh_burrow_and_owned_checkpoint_round_trip` requires `HEX_GRAND_WORLD`.
Focused results do not replace the exact `dev...HEAD` selector gate in
[CONTRIBUTING](../../CONTRIBUTING.md#before-opening-a-pr).

Native acceptance follows beach → boat → Fire/volcano → forest/tree → garden →
waterfall/library branches → Shadow → Crystal/summit → glider descent. Exercise
shrines, valid/refused teleport, death, and process restart on land/boat/air. Run
repeated circuits and a 30-minute session, report frame-time percentiles and loading,
and inspect vehicle orientation, colored strips and ocean seams in both directions.
Continuous windowless sequences can review temporal presentation. The named native
route still needs an explicitly approved live or user playtest for control feel and
taste; stills and headless timing do not establish those qualities or 60 FPS.

## Retained backlog

1. Golem committed fire into remembered cover, persistent long-range Wisp pursuit,
   and species-specific detection.
2. Library Wisp boss, wall-emerging rock Worms, Yeti, defensive mountain birds,
   Goblin scent/dynamic groups, king/support units and enclosing roots.
3. Advanced elemental control, precise environment editing and independent Shield
   orientation.
4. Selective snow/cloud/fog experiments, vegetation wind, broader time-of-day,
   authoring hot reload and measured compiler improvements.

Rejected visual treatments, the multi-minute island plan and unrelated historical
V3 cleanup remain excluded. Linear ticket reconciliation awaits reconnection.

## Approved revision02 transfer

The September26 user authorization approves the connected highland layout recorded in
[the frozen design oracle](../planning/waves/grand-v4/approved-r02/provenance.json).
The geography companion is an explicit source dependency, included byte-for-byte in
the authoring signature. A plain compile stages that same companion beside its
temporary source. Capture `--plain` explicitly selects plain terrain review; default
captures require the dressed package. Neither mode can borrow the other's identity.

Authored cameras carry final world positions and vertical orthographic spans. The
ground cameras still wait for loaded, supported player-sized clearance and preserve
the authored viewing pitch. Temporal walking endpoints also come from the current
package. Physical route checks use exact XYZ supports so a different floor at the
same horizontal position cannot count as reaching a Crystal stair waypoint.

The revision02 capture matrix adds the connected Crystal/Frozen overview and three
ordinary valley views of the mountain enclosure. They establish game transfer
evidence; the older package04 measurements above remain historical.
