# Procedural world generation V3

V3 is the next map contract, not another compatibility wrapper around V1 or V2.
It generates one validated world from semantic plans, then projects only the exact
facts the rest of the game needs. V1 and V2 remain in the tree temporarily as
visual and behavioral oracles while their recipes are rebuilt. They are removed
after the V3 migration corpus is approved.

This document fixes the boundaries and delivery order before implementation starts.
Recipe algorithms and tuning remain private to `hex_map`.

## The boundary

`GeneratedWorldPlan` is a private `hex_map` type. It is the only complete semantic
description of a V3 world and contains:

- occupied solid volumes and non-solid fills;
- directed liquid topology and rendered flow classifications;
- surface features such as trees and tall grass;
- structures such as walls, stairs, towers, and bridges;
- exact traversal blockers;
- gameplay light-source placements;
- exact biome, interior, and ambient-domain metadata;
- anchors and generated camera hints.

These layers are planned before voxelization. A recipe must not hide semantics in
material names, infer a river from water voxels after the fact, or hand-edit the
materialized map to rescue a seed.

No crate may import `GeneratedWorldPlan`, a patch planner, a liquid graph, a feature
plan, or a structure plan from `hex_map`. At the runtime boundary the map publishes
only the shared, exact consequences required by consumers:

- rendered footing keeps the existing `TilePos`, `HexSpan`, `SubstanceId`, and
  `Headroom` contract;
- anchors, interiors, and view hints keep their existing shared contracts;
- biome membership and traversal blockers are keyed by exact `TilePos`, never by
  horizontal coordinate alone;
- generated light entities publish an exact origin plus `GameplayLight` radius and
  level; perception derives their current `LightDomain` from that position and the
  interior metadata rather than caching a domain on the source;
- presentation occlusion composes named reasons rather than letting fog, cave
  cutaway, or canopy cutaway overwrite one another's `Visibility`.

Terrain edits still enter through `TerrainEdit`. V3 may rebuild private derived
layers after an edit, but no consumer receives mutable access to the plan.

The first V3 liquid policy is deliberately conservative. Until `hex_map` can rebuild
liquid occupancy, directed topology, and their runtime projection as one atomic
operation, it requires rejection of an edit to an authored V3 liquid voxel and an
edit to every lower voxel in that column while a retained authored liquid run remains
above it.
The private liquid plan classifies the exact `TilePos` and identifies every stacked
run affected. The classifier lands before a runtime hook because no runnable V3
recipe exists yet; the first runnable recipe must enforce it at the existing
`TerrainEdit` admission point.

This rule does not change `Substance::diggable`. Legacy and non-topological liquids
continue to use their existing material policy. A rejected V3 edit changes neither
occupancy nor flow metadata: liquid does not redistribute, and the map must never
leave stale current or fall descriptors behind.

## Layouts and patches

`generator_version: 3` selects one of two layouts:

- `Single(PatchSpec)` fills one connected world footprint with one recipe.
- `Ring7` fills one radius-33 footprint with a central patch and six surrounding
  connected patches.

A `PatchSpec` contains an environment, a typed recipe, named overlays, one connected
mask, and six directional edge contracts. A mask is a set of horizontal columns
inside the world footprint. Masks are disjoint, cover the footprint exactly, and
must not collapse vertically stacked surfaces. Explicit masks may have arbitrary
connected interiors, but each declared shared side must expose exactly one oriented,
simple, contiguous seam. Every seam lane must also retain an independent inward
approach corridor for the contract's full `approach_depth`; branched, disjoint, or
pinched seams are rejected while settings are validated, before candidate generation.

An edge contract describes what neighboring patches must agree on:

- the boundary elevation profile and permitted transition band;
- ordinary-walker route ports and their required width;
- liquid ports, including direction and whether the edge is an inlet, outlet, or
  dry;
- protected approach cells that either recipe must preserve.

The world planner resolves shared edges once. Both patches consume the same resolved
contract; they do not each generate a border and blend incompatible results later.
The planner establishes the macro route graph, elevation datums, and hydrology DAG
before any recipe fills its interior.

The first `Ring7` roster and clockwise order are fixed:

1. Hills in the center;
2. Mountains;
3. Waterfall;
4. Forest;
5. Fort;
6. Caves;
7. Sky Islands.

The six outer masks may vary internally but keep that order. Mountains has no liquid
port. Water may cross only matched liquid ports. Cave underground geometry and the
Sky Islands upper layer remain within their owning horizontal masks for the first
composite. The critical ordinary-walker network must connect every region through
redundant macro routes, but every shared boundary need not be open.

## Determinism and selection

One top-level candidate represents the complete output. In `Ring7`, patches are not
selected independently: a locally strong patch cannot win if its seams make the
world invalid.

- Every build evaluates eight deterministic candidates.
- A V3 `SeedStreams` API derives independent named streams from generator version,
  world seed, candidate, patch id, and stage name. Adding `forest.features` must not
  perturb `waterfall.hydrology` or any V1/V2 stream.
- Validation may apply at most four bounded semantic repair rounds. Repair may widen
  a route, clear headroom, move a non-critical feature, or fix a small seam. It may
  not replace the macro topology.
- A candidate requiring a major topology change is rejected. If all eight fail, the
  layout uses its separately validated canonical fallback. A failed fallback is a
  setup failure, never an empty map.
- Candidate selection scores only hard-valid worlds. Tactics and required routes
  outrank visual variety.

Settings, semantic-plan, and materialized-map fingerprints use separate V3 domains.
They exclude timings and unordered iteration, include every field that affects their
respective output, and are stable only within V3. V1/V2 fingerprints are frozen while
those implementations remain; V3 is not required to reproduce them.

Reports record generator version, resolved seed, candidate, repair actions, fallback
use, the three fingerprints, metrics, and timings. Diagnostic collections are sorted
before reporting or hashing.

The public `Ring7` recipe metrics summarize the admitted whole world rather than
concatenating seven recipe reports. They record ordinary and reachable surfaces,
reachable elevation diversity and relief, the critical route, macro edges and
redundant regions, directed liquid seams, and exact counts of feature instances,
structures, gameplay lights, and interiors. These fields are deterministic semantic
measurements; timing and presentation-only entity counts remain outside them.

After an admitted map is edited, `hex_map` keeps its published exact consequences
honest. Edited columns discard buried `BiomeRegions` entries and classify every
newly exposed solid run from the closest prior exact surface in that column.
`TraversalBlockers` remain attached only while their exact footing is still
walker-admitted; newly exposed footing never inherits a feature blocker.

## Recipe stages

Every recipe produces semantics first and voxels last. Shared traversal validation
uses the canonical two-level-tall walker and the same transition-clearance predicate
as live movement.

### Waterfall

Plan a directed, acyclic, steady-state water graph before carving terrain. Its flow
states are `Still`, `Current`, `Rapid`, and edge-aligned `Fall`. The graph establishes
a calm elevated inlet, rapids, a contiguous eleven-level fall, an extended plunge
basin, and an outlet. All three lanes reach both resolved world boundaries, so an
upstream two-wide metal bridge is the only ordinary crossing between the riverbanks.
Terrain is then fitted to that graph.

Water remains an opaque non-solid fill. The renderer animates the authored direction
and flow state, but water does not redistribute after terrain edits, push characters,
slow movement, or deal damage. The escarpment moves laterally by at most two hexes
between neighboring rows and retains a small set of mid-height, special-movement
shelves instead of one straight full-height wall. The critical land network includes
a short two-wide descent and a longer, independently climbable terrace on the
opposite bank. The second route has a broader irregular apron and remains usable if
the critical route is excluded. Until topology-aware rebuilding exists, the
conservative V3 edit policy above protects each authored liquid run and every lower
voxel in its column.

Waterfall candidates do not yet attempt semantic repair. Construction-valid
candidates pass the complete recipe contract unchanged; invalid candidates are
rejected, and the separately validated canonical fallback is the only recovery path.
The dedicated `walks/waterfall.ron` gate captures the default and close-character
views of the same deterministic scenario for liquid-motion and cliff-scale review.

### Forest

Plan the walkable surface and clearings first, then place the blocking woodland before
routing the road through it. Each planned feature carries an exact authored object id,
one of six rotations, and its rotated blocker footprint. A deterministic weighted path
bends between separated non-overlapping clearings and around the complete footprint,
not merely the object origin. Validation requires the four stable clearing names and
rejects any shared surface membership. Its mostly two-wide gravel footprint admits
short one-wide constraints where the existing trees pinch it, then tapers for three
cells into the prairie and stops. Tall grass can therefore reclaim the meadow instead
of preserving a bare feature-free line across it.

Small broadleaf and tall narrow trees have one-cell blockers. Old-growth trees require
seven same-level grounded supports and publish that exact rotated footprint as
traversal blockers; connectivity validation may deterministically substitute a
one-cell tree where a large footprint would sever ordinary terrain. Grass tufts are
visual-only. All features publish renderer-neutral `ObjectInstance`s, and only
authored canopy chunks receive the composable canopy-cutaway reason. Tree roots cover
roughly 20-24% of the woodland, while non-blocking tall grass covers 65-75% of the
prairie. Tall grass has no concealment rule. Trees cannot be chopped in this milestone.

Forest likewise uses candidate rejection rather than semantic repair: its bounded
repair hook returns `NoChange`, selection advances to the next deterministic
candidate, and the canonical fallback remains the final hard-valid result.
Recipe-specific repair actions will be added only when they can preserve the
validated topology instead of disguising regeneration as repair.

The recipe requires `party_start`, `hostile_start`, `forest_clearing`, and
`prairie_overlook` while preserving the open generated-anchor vocabulary. The two
review anchors are bound to the primary clearing and the recipe's exact prairie
overlook surface. `walks/forest.ron` pins the shipped hero seed and captures map and
character-camera presentation. The current walk DSL cannot address map-space tiles,
so that script is not a route traversal: exact graph validation and recorded manual
traversal cover topology until the tooling gains that capability.

### Caves

Plan one varied rocky exterior and one rooted underground network in the same stacked
volume. The native V3 recipe creates six through twelve chambers at floor levels six
through eight, connects the critical network with two-wide corridors, and descends
through an open two-wide one-level entrance ramp. Corridors preserve at least three
clear levels, chambers at least four, and every covered cell retains at least three
solid cutaway roof levels. Exact interior floors and roof voxels remain the source of
truth for perception domains and presentation.

Generated cave lights are deterministic gameplay semantics. Bright sources with
radii from four through seven cover the entrance, required actor route, and critical
chambers, while at least one optional branch floor remains dark. `hex_map` publishes
each source as an entity carrying its floor `TilePos` and `GameplayLight`;
`hex_perception` derives the interior domain from `InteriorRegions`. Static sources
make their supporting columns map-owned until terrain edits can replan light-bearing
objects.

Every source reserves a flat radius-one presentation footprint. Underground sources
carve a roofed alcove with the required three-level roof thickness; the upper source
uses an open landing beside the entrance rather than flattening its one-level ramp.
The reservation records a deterministic two-, three-, or four-level crystal kind and
one of six rotations in semantic and materialized fingerprints. Exact floor,
clearance, roof, interior, and unoccupied visual-volume checks reject an invalid
candidate.

The V3 recipe does not render a crystal itself. The later stacked integration maps
the ordered kinds to `prop/crystal-low-cluster`, `prop/crystal-branched`, or
`prop/crystal-spire`. Its authoritative `GameplayLight` and `TilePos` remain on the
exact cave floor; the corresponding `ObjectInstance` origin is one voxel above it.
Emission and restrained physical point lights remain presentation and never
determine gameplay illumination. The map-side light entity remains valid without
that optional visual layer.

### Fort

Fort resolves an unobstructed radius-nine site inside its arbitrary patch mask and
keeps every shared-edge approach outside the structure footprint. Worked-stone
volumes form a five-level, two-column-thick curtain, six stepped corner towers, two
opposite two-wide gates, two independent two-wide stair terraces, a gravel
courtyard, and an offset keep. Three-level gate apertures preserve the normal
two-level-tall walker contract. Alternating battlement columns sit outside the usable
wall walk and are tagged as non-ordinary review geometry.

Validation closes both gates to prove the defenses have no accidental shortcut,
then admits each gate separately to prove two independent ordinary routes. It also
checks exact worked-stone structure membership, gate headroom, one-level stairs,
wall-walk and tower access, anchor placement, and whole-network connectivity.
Candidates vary orientation and keep placement through independent named streams;
major structural failures reject the candidate rather than carving the topology,
and a separately authored orientation-zero fallback passes the same checks. The
fort remains generated static geometry, not a player construction system.

### Composite

`Ring7` first resolves the global routes, elevation profiles, liquid ports, and
protected seam approaches. It then runs each recipe against its resolved mask and
contracts, validates patch-local invariants, and finally validates the exact combined
`TilePos` graph. Materials and decorative boundaries are classified only after the
geometry and semantics are accepted.

Directed liquid ports are realized during checked composition, not by a later blend
pass. Every declared lane must resolve to exactly one terminal source node and one
sink node in the shared elevation band. The endpoints must use the same material, the
source may not already flow, and the crossing may be level or descend by one level.
Composition deterministically unifies the two liquid bodies and installs the exact
cross-patch downstream edge. Missing, ambiguous, uphill, mismatched, duplicate, or
undeclared crossings reject the complete world candidate.

No post-generation blend pass may erase anchors, water direction, traversal blockers,
interior/domain metadata, or protected approaches.

## Parallel delivery

The contracts PR lands before behavioral work. After that, two lanes may proceed from
updated `dev`:

- The map lane owns V3 planning, recipes, voxelization, map-side rendering, and exact
  projections. It does not edit `hex_units` or `hex_combat`.
- The perception lane owns deterministic illumination and faction knowledge in a new
  `hex_perception` crate, then fog presentation and owner-reviewed adapters. It does
  not import map internals.

The intended PR order is:

1. contracts and shared vocabulary, with no behavior change;
2. V3 foundation;
3. directed steady-state liquid topology, in parallel with headless perception;
4. the opaque animated flow renderer;
5. the Waterfall recipe;
6. fog presentation, cave lighting, and isolated gameplay adapters;
7. Forest;
8. Fort;
9. `Ring7`;
10. V3 rebuilds of Hills, Frozen, Volcanic, Sky Islands, Mountains, and Caves;
11. scenario and review-tool migration;
12. V1/V2 removal.

An adapter that changes movement, AI, targeting, engagement, or command validation is
a separate PR reviewed by that crate's owner. A map PR may add shared vocabulary in
an isolated commit, but it must not couple that addition to edits in owner-controlled
gameplay files.

## Migration gate

V1 and V2 are temporary development references, not supported save formats. While
they remain:

- their streams and numeric goldens stay frozen;
- they remain loadable by development scenarios used for side-by-side review;
- V3 reports and captures compare behavior and visual intent, not identical hashes.

Each active recipe migrates only after its V3 fixed corpus, stress corpus, captures,
and critical-route traversal pass. Where review tooling cannot yet address map-space
tiles, exact graph validation plus a recorded manual traversal supplies that gate;
capture-only scripts must not be described as route walks. Once every shipped scenario
and review tool uses V3, archive one migration report and remove V1/V2 parsing,
dispatch, generator code, assets, and runtime tests together. Do not leave a permanent
three-version matrix. Migration is not a literal geometry port: once the supporting V3
layers exist, each recipe also receives the appropriate liquid, vegetation, structure,
and gameplay lighting semantics.

## Verification

V3 foundation tests cover connected masks, exact coverage, six-way edge agreement,
volume overlap rejection, named-stream independence, ordered fingerprints, bounded
repair, forced fallback, setup failure, teardown, and re-entry.

Recipe tests enforce directed Waterfall flow and its bypass, Forest clearings and
protected routes, Fort circulation and headroom, and `Ring7` seam, hydrology, and
macro-route contracts. A fast fixed corpus runs in CI; ignored 10,000-seed corpora
must produce 100% valid final maps including fallback and target less than 1% fallback
use.

Benchmarks cover radius 12, 20, and 40 single patches plus the radius-33 composite,
including generation time, entity count, and terrain-edit projection. Perception
benchmarks separately cover fog recomputation. Review packs include deterministic
reports and default, rotated, top-down, and character-camera captures. Manual review
traverses every critical recipe route and every open composite seam.

## Primary precedents

The stage boundaries follow established techniques without making their
implementations runtime dependencies:

- [Genevaux et al., *Terrain Generation Using Procedural Models Based on
  Hydrology*](https://doi.org/10.1145/2461912.2461996) motivates resolving the
  hydrology graph before fitting terrain around it.
- [Minecraft's feature-generation
  stages](https://learn.microsoft.com/en-us/minecraft/creator/documents/world-generation?view=minecraft-bedrock-experimental)
  motivate keeping landform, liquids, surface features, and structures as ordered
  semantic passes.
- [Bridson's fast Poisson-disk
  sampling](https://www.cs.ubc.ca/~rbridson/docs/bridson-siggraph07-poissondisk.pdf)
  motivates deterministic, naturally spaced vegetation candidates.

V3's exact patch contracts, candidate selection, fallback policy, traversal rules,
and gameplay projections remain authored for this game.
