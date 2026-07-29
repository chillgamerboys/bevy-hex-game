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
| `HexTile` + `TilePos`, `HexSpan`, `SubstanceId`, `Headroom` on tile entities | world | gameplay | live | [systems/map.md](systems/map.md) |
| `TraversalProfile` / `TraversalEndpoint` — standability and step predicates | core | both | live | [systems/map.md](systems/map.md) |
| `MapAnchorId` / `MapAnchors` — named exact surfaces | world | gameplay | live | [systems/map.md](systems/map.md) |
| `SpecialMovementRegions` — deliberately opaque region ids | world | gameplay | live | [systems/map.md](systems/map.md) |
| `TerrainReady` — terrain built and validated | world | gameplay | live | [systems/map.md](systems/map.md) |
| `MapViewHint` — generated camera framing | world | presentation | live | [systems/map.md](systems/map.md) |
| `InteriorRegions` / `CutawayOccluder` — interior membership and roof cutaway | world | presentation | live | [systems/map.md](systems/map.md) |
| `ResolvedMapSeed` — the seed a session actually used | game | world | live | [development/config.md](development/config.md) |
| `TerrainEdit::Set` / `::Clear` — the write path | gameplay | world | live | [systems/map.md](systems/map.md) |
| `BiomeRegions` — published by V3; gameplay consumer pending | world | gameplay | **partial** | [systems/world-generation-v3.md](systems/world-generation-v3.md) |
| `TraversalBlockers` — exact feature-occupied surfaces consumed by validation, perception, and movement | world | perception / `hex_units` | live | [systems/world-generation-v3.md](systems/world-generation-v3.md) |
| `RunBottom(Level)` — each run's lowest voxel; prerequisite to terrain casting and obstruction-aware trajectories | world | gameplay | **agreed** | [planning/boundary.md](planning/boundary.md) C |
| `TerrainImpact { batch, canonical_volume, ElementId, power }` — declarative voxel effect | gameplay | world | **agreed** | [planning/boundary.md](planning/boundary.md) G |
| `TerrainImpactOutcome` — explicit, deterministically ordered per-voxel dispositions | world | gameplay | **agreed** | [planning/boundary.md](planning/boundary.md) H |
| `PendingTerrainEdits` — replay before first spawn | gameplay | world | **asked** | [planning/boundary.md](planning/boundary.md) ask D1 |
| `TerrainSnapshot` — generator-independent dump | world | gameplay | **asked** | [planning/boundary.md](planning/boundary.md) ask D2 |

## Perception and presentation

| Contract | Publisher | Consumer | Status | Specified in |
|---|---|---|---|---|
| `IlluminationLevel` / `ExteriorIllumination` — gameplay illumination, never sampled from the renderer | world | perception | live | [systems/perception.md](systems/perception.md) |
| `GameplayLight` + derived `LightDomain` — fixed V3 cave sources published and consumed | world | perception | live | [systems/perception.md](systems/perception.md) |
| `SightProfile` / `SightBand` — sight limits per illumination tier | perception | perception | live | [systems/perception.md](systems/perception.md) |
| `LocalMapKnowledge` — publisher live; movement adapter pending | perception | `hex_units` | **partial** | [systems/perception.md](systems/perception.md) |
| `FactionMapKnowledge` current-unit observations gate lattice views | perception | `hex_combat` | live | [systems/perception.md](systems/perception.md) |
| `KnowledgeSource` / `KnowledgeExpiry` — how a lattice fact was learned and when it stops being true | core | combat | live | [systems/combat.md](systems/combat.md) |
| `CanopyOccluder` — root-keyed generated-feature marker for current camera cutaway | world | presentation | live | [systems/perception.md](systems/perception.md) |
| `PresentationOcclusion` — cave/canopy reasons live; fog reason pending | shared | presentation | **partial** | [systems/perception.md](systems/perception.md) |
| `perception.ron` — sight tunables as designer-facing settings | world | perception | live | [planning/boundary.md](planning/boundary.md) J |

## Ordering

| Contract | Publisher | Consumer | Status | Specified in |
|---|---|---|---|---|
| `GameplaySetup` — `Resources → Terrain → Actors → Perception → View → Finalize` | core | all | live | [`CLAUDE.md`](../CLAUDE.md) |
| `PerceptionSystems` — headless phases through `PublishKnowledge` | core | perception | live | [systems/perception.md](systems/perception.md) |
| `PerceptionSystems::ApplyPresentation` — fog projection phase | core | perception | reserved | [systems/perception.md](systems/perception.md) |
| `AppSystems`, `PausableSystems` | core | all | live | [`CLAUDE.md`](../CLAUDE.md) |
| `CombatSystems` — `Act → Apply → Advance` | gameplay | gameplay | live | [`hex_combat/src/lib.rs`](../crates/hex_combat/src/lib.rs) |

## Content

| Contract | Owner | Consumer | Status | Specified in |
|---|---|---|---|---|
| `palette.ron` + `SwatchId` / `SrgbColor` — canonical authored-content colour vocabulary | shared visual contract | editor and runtime presentation | live | [design/visual-language.md](design/visual-language.md) |
| `voxel_styles.ron` + `VoxelStyleCatalog` — palette-bound reusable surface treatments | shared visual contract | `hex_editor`, `hex_objects` | live | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `object_catalog.ron` + `ObjectBlueprint` — deterministic catalog of validated local hex-voxel plants, effects, and props | shared visual contract | `hex_editor`, `hex_objects` | live | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `ObjectInstance` — exact object id, origin voxel, level height, and six-way rotation | shared visual contract | future world/effect publishers, `hex_objects` | **partial** — renderer live; no world/effect publisher yet | [systems/asset-workshop.md](systems/asset-workshop.md) |
| `substances.ron` — substance names, exact palette references, solidity, diggability | world | both | live | [development/config.md](development/config.md) |
| World files, lighting profiles | world | world | live | [development/config.md](development/config.md) |
| `spells.ron`, `elements.ron` — requirements, axes, targeting, effects | gameplay | gameplay | live | [development/config.md](development/config.md) |
| `combat.ron` — engagement, budgets, policy knobs | gameplay | gameplay | live | [development/config.md](development/config.md) |
| `scenarios.ron` — the scenario list | shared | both | live | [development/config.md](development/config.md) |
| `encounters/*.ron` — rosters by archetype, and where each unit starts | shared | both | live | [development/config.md](development/config.md) |
| Terrain-response table — authored stable names resolved to `(ElementId, power, SubstanceId)` | world | world | **agreed** | [planning/boundary.md](planning/boundary.md) G |
| `Substance::conjurable` plus spell-reference validation | world policy / gameplay loader | gameplay | **agreed** | [planning/boundary.md](planning/boundary.md) L |

Cross-file references between the two content domains are resolved and validated by
[`ContentIndex`](../crates/hex_assets/src/content_index.rs) at load, which is what lets a spell name a substance without either side
guessing.

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
