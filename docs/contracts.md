# Contracts

Every fact that crosses the boundary between the **world owner** and the **gameplay
owner**, in one table. [architecture.md](architecture.md) describes the structure;
this describes the interfaces across it, and who may change each one.

The two of us work independently, so this page exists to answer one question quickly:
*is this thing something I can rely on today, something accepted and scheduled,
something reserved for later, or something still being asked for?*

| Status | Meaning |
|---|---|
| **live** | Published and consumed in the shipped build |
| **partial** | One side is live, while the row names the required producer or consumer still pending |
| **agreed** | Both owners accept the contract and sequencing, but it is not live yet |
| **reserved** | Type and ordering exist; nothing produces or consumes it yet |
| **asked** | Proposed in [planning/boundary.md](planning/boundary.md), not yet agreed |

Detail lives in the doc named in the last column. Where a contract is *asked* rather
than agreed, the fallback the gameplay side ships without it is in
[boundary.md](planning/boundary.md).

## Terrain and geometry

| Contract | Publisher | Consumer | Status | Specified in |
|---|---|---|---|---|
| `HexTile` + `TilePos`, `RunBottom`, `HexSpan`, `SubstanceId`, `Headroom` on every material-run entity | world | gameplay | live | [systems/map.md](systems/map.md) |
| `TraversalProfile` / `TraversalEndpoint` — standability and step predicates | core | both | live | [systems/map.md](systems/map.md) |
| `MapAnchorId` / `MapAnchors` — named exact surfaces | world | gameplay | live | [systems/map.md](systems/map.md) |
| `SpecialMovementRegions` — deliberately opaque region ids | world | gameplay | live | [systems/map.md](systems/map.md) |
| `TerrainReady` — terrain built and validated | world | gameplay | live | [systems/map.md](systems/map.md) |
| `MapViewHint` — generated camera framing | world | presentation | live | [systems/map.md](systems/map.md) |
| `InteriorRegions` / `CutawayOccluder` — interior membership plus review-only roof metadata | world | presentation tooling | live | [systems/map.md](systems/map.md) |
| `ResolvedMapSeed` — the seed a session actually used | game | world | live | [development/config.md](development/config.md) |
| `TerrainEdit::Set` / `::Clear` — the write path | gameplay | world | live | [systems/map.md](systems/map.md) |
| `BiomeRegions` — published by V3; gameplay consumer pending | world | gameplay | **partial** | [systems/world-generation-v3.md](systems/world-generation-v3.md) |
| `TraversalBlockers` — exact feature-occupied surfaces consumed by validation, perception, and movement | world | perception / `hex_units` | live | [systems/world-generation-v3.md](systems/world-generation-v3.md) |
| `RunBottom(Level)` — each run's lowest voxel; exact occupancy for terrain casting, trajectories, and paired seven-ray sight | world | gameplay / perception | **live** | [planning/boundary.md](planning/boundary.md) C |
| `TerrainImpact { batch, volume, ElementId, power }` — declarative canonical-volume voxel damage | gameplay | world | **live** — #175 owns map admission/resolution; #180 adds the paid gameplay spell publisher and monotonic batch ledger | [planning/boundary.md](planning/boundary.md) G |
| `TerrainImpactOutcome` — one applied or rejected answer with exact per-voxel health transitions | world | gameplay | **live** — #175 publishes the answer, #178 validates it exhaustively, and #180 correlates it under the authority hold before settlement and release | [planning/boundary.md](planning/boundary.md) H |
| `DamagedVoxels` — exact partial-health projection, never a visibility grant | world | shared presentation | live | [planning/boundary.md](planning/boundary.md) H |
| `PendingTerrainEdits` — replay before first spawn | gameplay | world | **asked** | [planning/boundary.md](planning/boundary.md) ask D1 |
| `TerrainSnapshot` — generator-independent dump | world | gameplay | **asked** | [planning/boundary.md](planning/boundary.md) ask D2 |

## Perception and presentation

| Contract | Publisher | Consumer | Status | Specified in |
|---|---|---|---|---|
| `UnitId` + `Faction` on unit entities — stable identity and shared side vocabulary | gameplay | perception / combat / presentation | live | [systems/combat.md](systems/combat.md) |
| `MovingTo` + `Busy` — bounded exact-surface domain movement and legality; never reconstructed from animation components | gameplay | combat / AI / presentation | live | [systems/combat.md](systems/combat.md) |
| `IlluminationLevel` / `ExteriorIllumination` — gameplay illumination, never sampled from the renderer | world | perception | live | [systems/perception.md](systems/perception.md) |
| `GameplayLight` + derived `LightDomain` — fixed V3 cave sources published and consumed | world | perception | live | [systems/perception.md](systems/perception.md) |
| `SightProfile` / `SightBand` — sight limits per illumination tier | perception | perception | live | [systems/perception.md](systems/perception.md) |
| Exact paired sight bundle — head center to target top center plus six matching standing-body-top-corner to target-corner strict-interior rays; one observer owns the three-of-six threshold; character LOS alone omits a run's exposed top voxel when its top is within one level of that observer's support and the run continues below it, while the raw symmetric kernel retains full runs | core / gameplay | perception | live | [systems/perception.md](systems/perception.md) |
| `LocalMapKnowledge` — faction-generic Observed/Remembered traversal projection; AI consumer live, player movement adapter pending | perception | `hex_combat` / `hex_units` | **partial** | [systems/perception.md](systems/perception.md) |
| `FactionMapKnowledge` — current observations gate hostile lattice views, cast anchors, and AI identities | perception | `hex_combat` | live | [systems/perception.md](systems/perception.md) |
| `KnowledgeSource` / `KnowledgeExpiry` — how a lattice fact was learned and when it stops being true | core | combat | live | [systems/combat.md](systems/combat.md) |
| `InspectionCameraSubject` / `CenterInspectionCamera` — disclosure-authorized Third Person / First Person follow target plus one-shot Map-camera centering; never gameplay selection or command authority | gameplay adapter | world presentation | live | [systems/camera.md](systems/camera.md) |
| `TargetReticleRequest` / `WorldMarkerSuppression` — one disclosed target marker and phase-level suppression over unit-owned, non-pickable world markers | gameplay adapter | unit presentation | live | [systems/combat.md](systems/combat.md) |
| `CanopyOccluder` — exact authored canopy membership, separate from whole-tree behavior; runtime consumer pending | shared art / `hex_objects` | pending | **partial** | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `TreeOccluder` / `TreeFadeAmount` — stack-safe whole-tree identity and renderer-neutral camera opacity | world | presentation | live | [systems/camera.md](systems/camera.md) |
| `PresentationOcclusion` — faction fog, review-roof, character-camera proximity/full First Person model hiding, and Sandbox-deployment reasons compose through one visibility owner | shared | presentation | **live** | [systems/camera.md](systems/camera.md), [systems/perception.md](systems/perception.md) |
| `perception.ron` — sight tunables as designer-facing settings | world | perception | live | [planning/boundary.md](planning/boundary.md) J |

## Ordering

| Contract | Publisher | Consumer | Status | Specified in |
|---|---|---|---|---|
| `GameplaySetup` — `Resources → Terrain → Actors → Restore → Perception → View → Finalize` | core | all | live | [`CLAUDE.md`](../CLAUDE.md) |
| `PerceptionSystems` — headless phases through `PublishKnowledge` | core | perception | live | [systems/perception.md](systems/perception.md) |
| `PerceptionSystems::ApplyPresentation` — tactical-shroud and hostile-fog projection phase | core | perception / presentation | live | [systems/perception.md](systems/perception.md) |
| `PresentationSystems` — camera obstruction → renderer-owned materials → composed visibility | core | world / presentation | live | [systems/camera.md](systems/camera.md) |
| `TerrainSystems` — `ApplyWorld → RefreshProjections → ReconcileActors → ConsumeOutcomes` before perception and later combat authority | core | world / gameplay | **live** — #175 supplies map `ApplyWorld`, #178 supplies the shared ordering, and #180 wires occupancy/movement refresh, deterministic actor settlement and authority adoption, then matching-outcome consumption | [planning/boundary.md](planning/boundary.md) H |
| `AppSystems`, `PausableSystems` | core | all | live | [`CLAUDE.md`](../CLAUDE.md) |
| Same-frame combat knowledge — `PublishKnowledge → spatial lattice sync → Act → Apply → Resolve → Advance` | perception / gameplay | combat / AI | live | [systems/ai.md](systems/ai.md) |

## Content

| Contract | Owner | Consumer | Status | Specified in |
|---|---|---|---|---|
| `palette.ron` + `SwatchId` / `SrgbColor` — canonical authored-content colour vocabulary | shared visual contract | editor and runtime presentation | live | [design/visual-language.md](design/visual-language.md) |
| `voxel_styles.ron` + `VoxelStyleCatalog` — palette-bound reusable surface treatments | shared visual contract | `hex_editor`, `hex_objects` | live | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `object_catalog.ron` + `ObjectBlueprint` — deterministic catalog of validated local hex-voxel plants, effects, and props | shared visual contract | `hex_editor`, `hex_objects` | live | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `ObjectInstance` — exact object id, origin voxel, level height, and six-way rotation | shared visual contract | world publishers, `hex_objects`; future effects | **partial** — world publishers live for Forest vegetation and cave crystals; effect publishers pending | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `substances.ron` — stable name/id registry, exact palette references, solidity, diggability, and toughness | world | both | live | [development/config.md](development/config.md) |
| `Substance::toughness` — optional voxel HP on the fixed 1/2/4/8 scale | world | world | live | [planning/boundary.md](planning/boundary.md) G |
| World files, lighting profiles | world | world | live | [development/config.md](development/config.md) |
| `spells.ron`, `elements.ron` — requirements, axes, targeting, effects | gameplay | gameplay | live | [development/config.md](development/config.md) |
| `AcceptedContentRevision` — one deterministic semantic identity across elements, substances, terrain damage, spells, and lattices; Loading requires it | shared loader boundary | game setup | live | [planning/foundation-hardening.md](planning/foundation-hardening.md) |
| `lattices.ron` — every authored archetype is one contiguous hex arrangement; errors name the archetype | gameplay | lattice spawning | live | [development/config.md](development/config.md#writing-a-lattice) |
| `combat.ron` — engagement, budgets, policy knobs | gameplay | gameplay | live | [development/config.md](development/config.md) |
| `scenarios.ron` / `ScenarioToLoad` — internal world + lighting + encounter launch identity for Campaign, Sandbox, saves, retry, review, and tests | shared | both | live | [development/config.md](development/config.md) |
| `sandbox_maps.ron` — stable player-facing map IDs and scenario/seed identity, plus Party/Enemy regions retained only as hidden actor-staging compatibility metadata | shared | both | live | [systems/creator-and-sandbox.md](systems/creator-and-sandbox.md) |
| `encounters/*.ron` — rosters by archetype, and where each unit starts | shared | both | live | [development/config.md](development/config.md) |
| `terrain_damage.ron` / `TerrainDamageTable` — stable-name Boolean element × substance damage allow-list | world | world | live | [planning/boundary.md](planning/boundary.md) G |
| `Substance::conjurable` plus spell-reference validation | world policy / gameplay loader | gameplay | live | [planning/boundary.md](planning/boundary.md) L |

Cross-file references between the two content domains are resolved and validated by
[`ContentIndex`](../crates/hex_assets/src/content_index.rs) at load, which is what lets
a spell name a substance without either side guessing. `TerrainDamageTable`,
`ContentIndex`, and `LatticeLibrary` retain their last valid values across a rejected
edit, but
deterministic canonical source fingerprints prevent those retained values from
masquerading as the new revision. Loading proceeds only when every raw catalog,
direct catalog, and all derived tables match one published
`AcceptedContentRevision`; resource presence and Bevy change ticks are not readiness
signals.

## What each side commits to

Contracts are only half of it. The rest is what we each promise *not* to do.

**Gameplay will not**: sample renderer lights, shadows, exposure, or pixels for a
gameplay fact; import map-generator internals; divide by `level_height` or otherwise
reconstruct world units; or land a change to an owned crate's behavior without that
owner's review.

**The world will not**: publish generator plans, patch masks, edge contracts, or repair
metadata as consumable facts; key a published projection by `HexCoord` in a way that
collapses stacked surfaces; infer gameplay policy from a rendered object or semantic
part; or make one presentation system the sole owner of `Visibility`.

Both sides: a shared-type change lands in its own commit before either side depends on
it.
