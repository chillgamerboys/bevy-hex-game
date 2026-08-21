# Procedural world generation V3

V3 is the next map contract, not another compatibility wrapper around V1 or V2.
It generates one validated world from semantic plans, then projects only the exact
facts the rest of the game needs. V1 and V2 remain in the tree temporarily as
visual and behavioral oracles while their recipes are rebuilt. They are removed
after the V3 migration corpus is approved.

This document fixes the boundaries and delivery order. Current implementation status
is maintained in [planning/status.md](../planning/status.md). Recipe algorithms and
tuning remain private to `hex_map`.

## The boundary

`GeneratedWorldPlan` is a private `hex_map` type. It is the only complete semantic
description of a V3 world and contains:

- occupied solid volumes and non-solid fills;
- standing and directed liquid topology plus rendered flow classifications;
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
- presentation occlusion composes named reasons rather than letting fog or an explicit
  review cutaway overwrite one another's `Visibility`; camera-facing tree opacity is
  grouped separately by exact root.

Terrain edits still enter through `TerrainEdit`. V3 may rebuild private derived
layers after an edit, but no consumer receives mutable access to the plan.

The first V3 liquid policy is deliberately conservative. Until `hex_map` can rebuild
liquid occupancy, directed topology, and their runtime projection as one atomic
operation, it requires rejection of an edit to an authored V3 liquid voxel and an
edit to every lower voxel in that column while a retained authored liquid run remains
above it.
The private liquid plan classifies the exact `TilePos` and identifies every stacked
run affected. Waterfall now enforces that classification at the existing
`TerrainEdit` admission point, atomically rejecting edits that would leave stale
occupancy or flow metadata.

This rule does not change `Substance::diggable`. Legacy and non-topological liquids
continue to use their existing material policy. A rejected V3 edit changes neither
occupancy nor flow metadata: liquid does not redistribute, and the map must never
leave stale current or fall descriptors behind.

## Layouts and patches

`generator_version: 3` selects one of four layouts:

- `Single(PatchSpec)` fills one connected world footprint with one recipe.
- `Ring7` fills one radius-33 footprint with a central patch and six surrounding
  connected patches.
- `Ring19` fills one radius-55 footprint with a central patch, six first-ring
  patches, and twelve second-ring patches.
- `Macro(MacroLayoutSettings)` fills a radius-77 footprint from 37
  radius-12-scale atomic cells, then collapses those cells into authored logical
  biome instances.

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
- liquid ports, including dry edges, directed inlet/outlet flow, and exact-level
  standing-water joins;
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

`Ring19` admits fixed profile rosters over one composite geometry. Its deterministic Voronoi masks cover
exactly 9,241 columns around patch centres 22 columns apart, with 42 reciprocal
internal seams and 30 outer boundary sides. Slot order is stable: centre, six
first-ring slots clockwise, then twelve second-ring slots clockwise. The original
selectable **Two Rings** map uses the `TwoRings` profile roster:

1. central Hills confluence;
2. Frozen Hills, Forest A, Prairie A, downstream Hills, Waterfall B, and Waterfall A;
3. Sky Islands, Deep Forest A, Deep Forest B, Forest B, Prairie B, outlet Waterfall,
   Fort, Caves, Volcano, Mountains A, Mountains B, and Mountains C.

Each region also carries an explicit rotation in `0..=5`. Every internal seam has two
width-two walker ports and protected depth-three approaches. Whole-world validation
requires both endpoints of all 42 seams to join the party-reachable physical graph,
then removes each seam in turn and proves that all 19 regions remain reachable.
Optional Sky Islands surfaces remain flight-gated and are not promoted into the
ordinary network.

The fixed water graph has three mountain sources and one confluence:

- Mountains A → Waterfall B → central Hills;
- Mountains B → Waterfall A → central Hills;
- Mountains C → Frozen Hills → central Hills; and
- central Hills → downstream Hills → outlet Waterfall → the south-east world
  boundary.

The first two mountain handoffs are level 29; all remaining internal water handoffs
are level 16. The outlet Waterfall reaches its boundary terminal at level 3.
Volcano owns a separate lava body which exits the western boundary at level 14; lava
never joins the water graph. Every liquid crossing is explicit, directed, acyclic,
level or descending, and checked against the exact seam lanes.

The `DesertOasis` Ring19 profile keeps the same masks, slot order, 42 seams, redundant
ordinary-walker graph, and 5-bit patch namespace while replacing the biome roster:

1. one central Oasis with a radius-five local pool, a three-column green ring, and
   twelve date palms;
2. six clockwise Dunes patches, rotated once through all six local orientations; and
3. an outer ring alternating six taller Dunes patches with six Desert Plain patches.

Every patch uses the `Arid` environment. Its seams remain Dry and expose the same two
width-two walker ports with depth-three approaches. The oasis is an explicit local-
water exception: its connected Still pool has no cross-region liquid connection and
no boundary outlet, so this profile declares empty `liquid_connections` and
`boundary_outlets`. That is not permission for an omitted hydrology graph in the
`TwoRings` profile or another composite. `DesertOasis` appends a conditional profile
extension to the settings fingerprint; the original defaulted `TwoRings` byte stream
and fingerprint remain unchanged.

`Macro` separates atomic ownership from logical biome identity. Atomic cells provide
exact coverage, adjacency, and masks. Each named biome instance claims one connected
set of cells, takes its logical id and seed namespace from stable authored order, and
runs its recipe once over their union, publishing one opaque `BiomeRegionId`. Edges
between cells owned by the same instance disappear. External seams are resolved from
the complete region-pair boundary, which may contain more than one segment on the
same compass side. Aquatic and scenic
fragments may omit actor anchors; the composed world must still publish its canonical
actor anchors and validate every declared critical land route.

Macro adjacency validation is allowed by default and is not applied retroactively to
Single, Ring7, or Ring19. The initial rules are deliberately narrow:

- Shallow Sea may touch only Beach or Shore.
- Beach and Shore must each touch at least one Shallow Sea instance and at least one
  Forest or Prairie instance.
- Every actual Deep Mountain neighbor must be Mountains; a world boundary is not a
  neighbor.
- Frozen and Volcanic environments may not touch directly.

Diagnostics name the offending logical instance or pair and its atomic cells so an
authored layout can be repaired without reconstructing the resolved masks from a
fingerprint.

The first Macro layout is the selectable **Mountain Range** scenario. Its seven
sea-to-massif diagonal bands contain `4/5/6/7/6/5/4` atomic cells:

1. one four-cell Shallow Sea instance;
2. two Beach and three Shore instances, with Beach at the transverse ends;
3. three Forest and three Prairie instances;
4. five Hills and two Waterfall instances;
5. six first-tier Alpine Mountains instances;
6. three elevated Alpine Mountains instances around two forward Deep Mountain cells;
   and
7. one elevated Alpine Mountains instance beside three rear Deep Mountain cells.

The result is 18,019 columns and 30 logical biome regions. The Shallow Sea recipe runs
once over a four-cell union, while Deep Mountain runs once over a five-cell
three-back/two-front wedge; neither publishes internal seams.
The elevation progression is sea level 8, coast levels 9–13, green terrain 12–18,
Hills approximately 16–24, first-tier mountain seam datums 24–34 with peaks near 44,
second-tier datums 34–48 with peaks near 62, and a Deep Mountain base near 48 rising
toward a broad level-96 summit under a hard cap of 104. The Deep Mountain climate
payload supplies Macro's alpine thresholds: its shipped treeline is level 36 and its
shipped snowline makes Mountain and Deep Mountain surfaces snowy from level 52.

Two directed Waterfall tributaries descend through the green and coastal bands and
join one shared water body at its still sea footprint. Their generated channels
publish current, rapid, and fall stages, while standing-water seams join submerged
coastal lanes to that footprint without creating a current. Prairie instances place
their configured nonblocking authored grass over eligible dry terrain. The required
ordinary-walker route is central Shore → Prairie → central Hills → one instance in
each mountain tier → the landward Deep Mountain base. The summit, massif interior, a
through-route, and global connectivity among all other land instances are
intentionally not required. The party starts on central Shore, the hostile starts in
central Hills, and review anchors cover coast, inland, foothill, massif front, and the
Deep Mountain base.

Crystal Mountain is the second authored Macro layout and the first to use a spanning
feature. Four logical instances own the complete 37-cell graph:

1. `crystal-ascent` owns the centre and all six radius-one cells, then expands as
   needed to contain the complete radius-32 authored landmark footprint around world
   origin while retaining its original central-cell fringe;
2. `summit-forest` owns five consecutive upper radius-two cells and keeps a connected
   temperate basin at levels 149–151;
3. `inner-mountain` owns the other seven radius-two cells; and
4. `outer-mountain` owns all eighteen radius-three cells and rises into the enclosing
   level-178-through-192 ridge.

Alpine treeline filtering applies only to alpine vegetation. It does not remove the
temperate Forest basin's trees at levels 149–151 or Crystal Ascent's authored crown
trees; both remain valid above the surrounding alpine recipes' normal treeline.

The initial seven-cell union is not itself a radius-32 disk: it both misses a small
part of the authored site and protrudes beyond it. Resolution transfers only the
missing disk columns from adjacent atomic masks and does not trim the central seven's
pre-existing ownership. Those transfers happen before seam construction; complete
coverage, disjoint ownership, and every donor mask's remaining connectivity are then
revalidated. In composite mode every retained landmark column outside radius 32
receives level-150 summit terrain instead of the standalone world's base-level grass.

Macro has three defaulted, orthogonal collections for this composition.
`walker_connections` declares explicit ordinary surface ports independently from the
legacy `critical_route`; Crystal Mountain resolves one exact four-wide connection
whose Crystal-side approach is the authored upper terminal trail and whose other side
opens into the Forest basin. `spanning_features` declares a
stable name, ordered instance route, world-boundary terminal, destination anchor,
floor, width, clearance, and roof thickness. `anchor_aliases` maps instance-local
anchors to stable world names. An empty `critical_route` is admitted only when exactly
one spanning feature is marked as the canonical route.

The `crystal_mountain.tunnel` route follows `outer-mountain` → `inner-mountain` →
`crystal-ascent`, entering from the outer instance's west world boundary and ending at
`crystal_ascent.lower_entry`. It remains flat at level 6, has exactly four lanes, six
clear levels, and at least three solid roof levels. Its eight-wide, twelve-level
exterior mouth narrows to the four-lane passage. Its eight boundary apron floors are
open to the exterior; the first roofed row is the four-wide foot threshold. Resolved
seam facts record exact subsurface lane pairs and never masquerade as surface walker
ports. After carving, every incidental ordinary level-6 contact across a crossed
shared edge is changed to a world-owned special-movement closure, leaving exactly the
declared four lane pairs traversable. Routing minimizes length and then turns within
the declared instance order, with stable coordinate tie-breaking. The final twelve
centerline rows transition from rough stone to worked Gothic masonry. The tunnel is
the only ordinary foot-to-basin route; the ridge and surface masks expose no
alternative ascent.

The tunnel floor retains the `BiomeRegionId` of its horizontal owner. One Dark
interior and light domain spans every roofed passage floor and Crystal Ascent. The
four foot-threshold and four summit-threshold surfaces are the interior's exact
entrance set; the eight open boundary-apron floors remain exterior, and the lower
Crystal aperture is an internal connection. Stable world anchors cover the foot
apron, mouth, midpoint, Gothic transition, Ascent threshold, summit exit, basin
clearing, and ridge while retaining every `crystal_ascent.*` review anchor. The world
reserves Macro namespace prefix 63 across feature, light, interior, and special
movement ids; the tunnel consumes it for its lights, unified interior, and exact seam
closures.

Single and Ring7 retain their shipped 4-bit patch / 28-bit local numeric namespace.
Ring19 uses a layout-specific 5-bit patch / 27-bit local namespace, so patch ids
16–18 cannot alias local feature, structure, liquid, light, interior, or
special-movement identities. Macro uses a 6-bit instance / 26-bit local namespace,
leaving room for all 30 Mountain Range instances without changing legacy ids or
fingerprints.

## Determinism and selection

One top-level candidate represents the complete output. In `Ring7`, `Ring19`, and
`Macro`, patches or instances are not selected independently: a locally strong
fragment cannot win if its seams make the world invalid.

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

Macro's walker, spanning-feature, and anchor-alias settings extend the settings
fingerprint only when at least one collection is nonempty. A legacy Macro file whose
three defaulted collections are empty retains its exact prior byte stream and
fingerprint; adding the schema with empty defaults cannot rewrite Mountain Range.
Ring19 follows the same compatibility principle for its defaulted profile: `TwoRings`
retains its prior settings bytes, while `DesertOasis` carries an explicit versioned
profile suffix.

Reports record generator version, resolved seed, candidate, repair actions, fallback
use, the three fingerprints, metrics, and timings. Diagnostic collections are sorted
before reporting or hashing.

The public composite metrics summarize the admitted whole world rather than
concatenating patch reports. Ring7 records ordinary and reachable surfaces,
reachable elevation diversity and relief, the critical route, macro edges and
redundant regions, directed liquid seams, and exact counts of feature instances,
structures, gameplay lights, and interiors. Ring19 additionally records its exact
world columns, biome-region count, reciprocal seams, outer boundary sides, and
boundary liquid outlets. These fields are deterministic semantic measurements;
timing and presentation-only entity counts remain outside them.

Mountain Range metrics additionally record its 37 atomic cells, 42 outer macro sides,
30 logical regions, resolved reciprocal, standing, and directed seams, critical-route
steps, liquid coverage, elevation extrema and relief, summit level, and broad
high-massif coverage. Layout validation separately fixes the raw atomic adjacency
count at 90.

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
a calm elevated inlet, rapids, a contiguous thirteen-level fall, an extended plunge
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
visual-only. All features publish renderer-neutral `ObjectInstance`s. Every rendered
tree chunk retains the exact stack-safe root used for whole-tree camera fading, while
only authored canopy chunks retain canopy art metadata. Tree roots cover roughly
20-24% of the woodland, while non-blocking tall grass covers 65-75% of the prairie.
Tall grass has no concealment rule. Trees cannot be chopped in this milestone.

Forest likewise uses candidate rejection rather than semantic repair: its bounded
repair hook returns `NoChange`, selection advances to the next deterministic
candidate, and the canonical fallback remains the final hard-valid result.
Recipe-specific repair actions will be added only when they can preserve the
validated topology instead of disguising regeneration as repair.

The recipe requires `party_start`, `hostile_start`, `forest_clearing`, and
`prairie_overlook` while preserving the open generated-anchor vocabulary. The two
review anchors are bound to the primary clearing and the recipe's exact prairie
overlook surface. `walks/forest.ron` pins the shipped hero seed and captures map and
character-camera presentation. The walk DSL can click an exact stack-safe `TilePos`
and wait for party movement to become idle, but the current Forest script has no
authored waypoint route. It therefore remains a capture walk rather than traversal
evidence; exact graph validation and the recorded manual traversal still cover its
topology.

### Deep Forest and Prairie

Deep Forest and Prairie are distinct additive recipes backed by the same private
vegetation placement vocabulary as Forest. Deep Forest covers the complete patch
with updated authored trees, keeps blocking-root coverage in the 28–32% band, and
protects one winding trail plus three irregular clearings. It has no prairie grass
zone. Prairie uses rolling Forest-style ground without trees or an authored road and
covers 65–75% of eligible surfaces with nonblocking authored grass.

Both recipes retain exact object ids, six-way rotations, complete rotated bounds,
blocker footprints, and deterministic semantic fingerprints. Their standalone
selectable maps are **Deep Forest** and **Prairie**, both pinned to seed
`1592598566`.

### Arid recipes

`Arid` is the sand, sparse-vegetation, dune, and isolated-oasis environment. Its four
recipes are deliberately separate so a focused `Single` map does not need to pretend
to be a complete desert region:

- Desert Transition assigns connected grass, dirt-ecotone, and sand bands from exact
  slices of the recipe-local frame. Rotation turns the complete transition. The band
  boundaries and requested dry coverage are exact consequences of settings; the
  modest rolling relief varies through its named landform stream.
- Desert Plain is connected, low-relief sand without authored structures or
  vegetation. Its rolling surface varies by seed while its material coverage,
  relief bound, seams, routes, and anchors remain exact.
- Dunes authors the requested number, spacing, and height of parallel ridges. A named
  seed stream varies only their bounded lateral warp; the resulting sand surface
  remains one-level-Lipschitz and ordinary-walkable from trough to crest.
- Oasis authors one exact local Still-water pool, a grass-and-soil shore ring, dry
  sand approaches, and the requested exact count of `plant/date-palm` instances.
  Pool and shore geometry do not vary with the seed. Palm priority and rotation do,
  while their visual volumes, reserved-route clearance, and single-root traversal
  blockers remain exact. Oasis water never participates in a composite seam.

All four reject non-Arid environments and overlays. The selectable **Desert
Transition**, **Desert Plain**, and **Dunes** scenarios are radius-12 `Single` maps;
**Desert Oasis Rings** is the radius-55 `DesertOasis` Ring19 profile described above.

### Volcano

Volcanic Hills keeps its scenario name for compatibility but now dispatches the V3
Volcano recipe. An off-centre crater massif occupies roughly one quarter of the
patch and rises at least 20 levels above its base. A directed lava body descends from
the crater to the boundary with distinct static, current, fall, and deterministic
landing presentation. There is no ford. The only ordinary crossing is an elevated
bridge at least four levels above lava, reached by one-level stair approaches.

### Coastal island recipes

The Coastal environment also admits two island recipes. `SandyIslets` authors an
exact requested count of separated low sand components inside one level-8 still sea;
only its designated primary component owns actor anchors and a protected ordinary
route. `WoodedIsland` authors one broad connected dry component with an exact
two-column sand fringe, grass-and-soil interior, protected route, and deterministic
eligible broadleaf placement. Its configured tree percentage describes canopy
coverage; exact roots use half that density because every accepted silhouette spans
multiple columns and visual volumes may not overlap. Both keep material topology and
component count stable while named streams vary bounded relief and feature placement.

Their focused scenarios are radius-24 **Sandy Islets** with exactly five dry
components and radius-40 **Wooded Island** with one connected island. The radius-77
**Ocean Archipelagoes** Macro profile composes one 24-cell Shallow Sea, three two-cell
scenic Sandy Islets instances, one one-cell sandy landing, and one six-cell Wooded
Island heart. Ten Standing seams form one ocean; one width-four landing-to-heart
walker aperture excludes its exact protected causeway footprint from the otherwise wet coast and
forms the entire ordinary inter-instance route. Its seven dry
components are intentional: the landing/heart component is playable and six satellite
components are scenic. Sandy Islets and Wooded Island currently reject Macro spanning
features because their specialized coastline construction does not yet consume global
feature reservations.

Island metrics retain exact world columns, water and dry coverage, dry-component and
reachable-surface counts, shoreline coverage, relief, route length, and trees where
applicable. The Macro report additionally fixes 37 cells, six logical regions, ten
standing-water seams, seven dry components, and six scenic dry components.

### Coastal and alpine Macro recipes

Shallow Sea uses an exact deliberately simple column profile: Bedrock at level 0,
Stone at levels 1–2, Dirt at level 3, Sand at level 4, and Still water at levels 5–8.
Sand is a first-class palette-backed solid used by materialization and semantic
fingerprints; soil continues to use Dirt.

Beach keeps 60–75% of its footprint submerged, exposes a narrow sand edge, and places
sparse broadleaf trees on 2–5% of eligible dry columns. Shore keeps 20–40% submerged,
raises 3–6-level voxel cliffs above the water, retains a broader dry top, and places
trees on 8–12% of eligible dry columns. Both reuse the existing tree assets and exact
blocker projection. Their still-water portions connect through Standing seams rather
than receiving synthetic downstream directions.

Alpine Mountains apply the instance's authored low-to-high grade before adding rocky
interior peaks. Adjacent same-tier instances share seam datums, while the protected
route apertures retain the ordinary one-level movement constraint. The existing
Frozen Mountains recipe remains compatible and unchanged in presentation.

Deep Mountain consumes one connected multi-cell union mask. A low-frequency height
field, boundary falloff, broad shoulders, and one dominant summit create one massif
instead of four stitched peaks. Validation requires a reachable landward base, a
summit near the authored target, and substantial high-elevation coverage; it does not
require ordinary access to the summit or interior.

### Caves

Plan one varied rocky exterior and one rooted underground network in the same stacked
volume. The native V3 recipe creates six through twelve chambers on three flat floor
tiers at relative levels `+0/+2/+4`, connects the critical network with one-level
two-wide corridors, and descends through an open two-wide one-level entrance ramp.
Corridors preserve at least three clear levels, chambers at least four, and every
covered cell retains at least three solid cutaway roof levels. Exact interior floors
and roof voxels remain the source of truth for perception domains and presentation.
Sparse nonblocking authored moss and lichen stay outside required connectors and
crystal reservations.

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

Before candidate construction, Caves preflights all three possible authored assets:
`prop/crystal-low-cluster`, `prop/crystal-branched`, and `prop/crystal-spire`. Their
radius, height, origin, style modes, and empty blocker/canopy masks must match the
reserved geometry. A missing or incompatible dependency fails setup before any map
entities are published.

The authoritative `GameplayLight` and `TilePos` remain together on the exact cave
floor. A separate renderer-neutral `ObjectInstance` starts one voxel above it with
the planned six-way rotation, and owns a restrained non-shadow-casting point light.
The authored emission and physical light are presentation only; neither carries
`GameplayLight` nor determines gameplay illumination.

### Crystal Ascent

Crystal Ascent is a deterministic standalone landmark recipe over a radius-40 world.
Its authored site occupies radius 32, begins at `base_level`, and accepts an exact rise
from 100 through 200 levels. The shipped showcase uses base level 6 and a 144-level
rise. Its monumental geometry is seed-independent; seed streams vary only the landing
crystal silhouettes and rotations plus the summit tree placement and rotations.

A twelve-hex-wide, eighteen-level-high pointed lower aperture opens into a radius-23
worked-stone chamber. The radius-four cathedral-heart reservation contains a
30-level asymmetric crystal: a broad fractured base, long irregular-prism body, and
off-centre crown. It blocks the chamber centre while the shaft above it remains open
and contracts to a radius-11 summit oculus. Exactly three clockwise stair circuits
climb around that void. Their four-wide
radial bands are `24..=27`, `21..=24`, and `18..=21`; each circuit contains six
flights and six corner landings. Flight boundary `i` is
`base_level + floor(i * rise_levels / 18)`, and rises are distributed along each
flight so every ordinary transition is flat or one level. Flights retain at least
four clear levels, corner landings retain at least eight, and every contraction's two
four-by-four turning pads retain eight-level clearance across their 28 unique cells.
Each circuit is carved into a radial worked-stone haunch: its four lanes thicken from
two through eight voxels toward the wall while retaining the exact same walkable top.
The exact allowlisted handoff graph is the only connection between consecutive
circuits: an exact four-cell ownership seam, four pairwise-disjoint radial transfer
lanes, their immediate entry and exit edges, and one unavoidable adjacent hex-corner
diagonal. Every other cross-circuit edge is invalid. This prevents a single
articulation tile without pretending that a flat hex-grid corner is a square-grid
vertex cut. Validation also rejects narrower coverage, void crossings, wall clipping,
insufficient headroom, or a lower-to-upper route whose elevation differs from the
requested rise.

The summit is a soil-and-grass crown around the oculus. Radius 18 remains an open
clearing; existing broadleaf trees become denser outside it while an exact four-wide
trail stays clear. Stable `crystal_ascent.lower_entry`,
`crystal_ascent.bottom_chamber`, and `crystal_ascent.upper_exit` anchors identify the
landmark. Stable `crystal_ascent.mid_flight`, `crystal_ascent.corner_landing`, and
`crystal_ascent.upper_contraction` anchors provide exact interior review positions,
and exact four-wide lower and upper terminal pads remain protected. The
radius-32 lower terminal is an exterior apron; the Dark interior begins at the exact
four-wide radius-31 threshold. The upper terminal is opposite the lower aperture.
Landmark architecture and fixtures remain inside the radius-32 site, while the
standalone recipe fills the surrounding radius-40 world with ordinary grass terrain.
A real Macro patch context constructs and round-trips its volume, blockers, regions,
lights, and anchors after translation and any of the six rotations; recipe-owned
decorative rotations compose from recipe-local orientation into world space and
round-trip exactly. Arbitrary Macro placement remains invalid. Crystal Mountain is the
first admitted composite: it fixes the landmark at world origin with rotation zero,
requires its mask to contain the complete radius-32 site, and retains any central-cell
fringe without resizing the architecture.

Each of the eighteen outward landing alcoves reuses one accepted cave-crystal asset.
Every alcove origin publishes exactly one Bright radius-4 source and one Dim radius-18
source, with only the Bright source owning the visual object and its non-shadow-casting
4,500-lumen point light. The cathedral-heart origin similarly owns exactly one Bright
radius-8 source and one Dim radius-24 source, one visual object, and four vertically
distributed point lights. The complete chamber, radius-28-through-31 entrance
approach, and stairs share one Dark interior domain and resolve to at least Dim. The
lower apron and non-stair summit crown are exterior; the stair footprint stays
Interior through its upper terminal and transitions only after the route exits onto
the crown. Physical light and emissive materials communicate these rules but never
establish gameplay illumination.

The heart is the first authored prop to opt into exact gameplay occupancy. Its
preflighted structural voxels are rotated and compacted into
`AuthoredObjectVoxelRuns`; the runtime publishes their union before movement and
perception. Standing-body intersections derive the landmark's exact traversal
blockers, and the same complete volume blocks strict-interior sight without terrain's
low-cover exception. Small crystals and summit vegetation retain their existing
contracts, and authored-object casting obstruction remains later work.

### Fort

Fort resolves an unobstructed radius-nine site inside its arbitrary patch mask and
keeps every shared-edge approach outside the structure footprint. Worked-stone
volumes form a five-level, two-column-thick curtain, six small accessible corner
turrets, two opposite two-wide gates, two independent two-wide stair terraces, a
gravel courtyard, and an offset keep. Three-level gate apertures preserve the normal
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

Ring7, Ring19, and Macro first resolve global routes, elevation profiles, liquid
ports, spanning-feature reservations, and protected seam approaches. When a feature
ends at an authored landmark, that destination fragment is built first so planning
can consume its exact terminal, upper threshold, and interior identity. The resulting
per-patch footprint is supplied to the remaining recipes before they plan liquids or
decoration. Each fragment still validates against its resolved mask and contracts.

Composition then follows four distinct stages: reserve, merge, carve, and finalize.
Normal fragments are merged without publishing a final world; each spanning feature
atomically rewrites the combined volume exactly once; Crystal Mountain publishes its
authored Forest approach, basin clearing, ridge, and scenario anchors; and only then
does ordinary finalization classify materials and decorative boundaries and admit the
exact `TilePos` graph.

This stage split is load-bearing for cross-biome passages. Liquids and decoration must
observe the reserved corridor, while no patch may independently carve its idea of one
side of a tunnel. The global pass preserves overlying surfaces, carries horizontal
biome ownership onto the new floor, closes undeclared ordinary subsurface seam
contacts, and publishes one continuous interior, lighting, and cutaway projection.
Missing seam lanes, an undeclared opening, a roof or bedrock breach, a blocker
collision, or a second ordinary foot-to-destination path rejects the complete
candidate.

Directed liquid ports are realized during checked composition, not by a later blend
pass. Every declared lane must resolve to exactly one terminal source node and one
sink node in the shared elevation band. The endpoints must use the same material, the
source may not already flow, and the crossing may be level or descend by one level.
Composition deterministically unifies the two liquid bodies and installs the exact
cross-patch downstream edge. Missing, ambiguous, uphill, mismatched, duplicate, or
undeclared crossings reject the complete world candidate.

A Standing crossing instead resolves broad lanes at one exact surface level and
unifies still-water bodies without installing a downstream edge. It is distinct from
a level directed current and from a Waterfall handoff. Missing contacts, mismatched
levels or materials, non-still endpoints, and undeclared standing joins reject the
complete world candidate.

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

The normative delivery order is:

1. contracts and shared vocabulary;
2. V3 foundation;
3. directed steady-state liquid topology and headless perception;
4. the opaque animated flow renderer;
5. Waterfall;
6. isolated perception and gameplay adapters;
7. Forest;
8. Fort;
9. `Ring7`;
10. V3 rebuilds of Hills, Frozen, Volcanic, Sky Islands, Mountains, and Caves;
11. complete scenario and review-tool migration;
12. additive Volcano, Deep Forest, and Prairie recipes;
13. `Ring19` and the selectable Two Rings map;
14. Macro layout, adjacency, coastal/alpine recipes, and the selectable Mountain
    Range map;
15. Macro spanning features and the selectable Crystal Mountain map;
16. Arid recipes and the selectable Desert Transition, Desert Plain, Dunes, and
    Desert Oasis Rings maps;
17. Coastal island recipes and the selectable Sandy Islets, Wooded Island, and Ocean
    Archipelagoes maps;
18. V1/V2 removal.

See [planning/status.md](../planning/status.md) for progress through this sequence.

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
and critical-route traversal pass. Scripts may express exact stack-safe tile clicks
and bounded waits for party movement; routes still require validated authored
waypoints. Until those waypoints exist, exact graph validation plus a recorded manual
traversal supplies that gate, and capture-only scripts must not be described as route
walks. Once every shipped scenario and review tool uses V3, archive one migration
report and remove V1/V2 parsing, dispatch, generator code, assets, and runtime tests
together. Do not leave a permanent three-version matrix. Migration is not a literal
geometry port: once the supporting V3 layers exist, each recipe also receives the
appropriate liquid, vegetation, structure, and gameplay lighting semantics.

## Verification

V3 foundation tests cover connected masks, exact coverage, six-way edge agreement,
volume overlap rejection, named-stream independence, ordered fingerprints, bounded
repair, forced fallback, setup failure, teardown, and re-entry.

Macro coverage additionally fixes the radius-77 / 18,019-column geometry, 37-cell
ownership, 90 raw adjacencies, 42 outer sides, 30 logical ids, connected multi-cell
instances, erased internal seams, six-bit namespace, permissive and contextual
adjacency behavior, exact sea strata, one shared water body with a continuous still
coastal/sea footprint, acyclic descending tributaries, coastal coverage, massif
shape, and the Shore-to-massif-base route.

Crystal Mountain coverage additionally fixes the exact seven-cell logical landmark
core, containment of the radius-32 site plus its retained central-cell fringe and
connected donor remainders; five-cell Forest basin; enclosing inner/outer ridges; the
authored summit approach and exact four-wide walker port; ordered subsurface instance
route; four level-6 lanes and exact seam closures; six-level clearance; three-level
roof; eight exterior apron floors; one continuous interior and light domain with two
four-wide entrance sets; complete Dim coverage; and the absence of a surface bypass,
undeclared seam opening, liquid collision, bedrock breach, or shortcut.
Representative seeds and all six landmark rotations exercise the patch constructor
even though the shipped world fixes rotation zero.

Arid coverage fixes connected and rotated transition bands, exact sand coverage,
bounded low relief, dune ridge count/spacing/height, one-level dune transitions, the
local oasis pool and green ring, exact date-palm count and blocker projection, and
Dry Ring19 seams around the local-water exception. Composite tests retain all 9,241
columns, 19 region ids, 42 reciprocal seams, single-seam-removal reachability, stable
world aliases, and the original `TwoRings` fingerprint.

Island coverage fixes Sandy Islets' exact requested component count, primary ordinary
route, level-8 connected water, relief bound, and two-column sand fringes; Wooded
Island's single connected dry component, grass-and-soil interior, tree exclusions,
and complete ordinary route; and Ocean Archipelagoes' 18,019 columns, 37 cells, six
logical instances, ten Standing seams, seven dry components, six scenic components,
and sole width-four landing-heart connection. Focused and Macro tests also prove
stable semantic anchors, deterministic re-entry, and that no ordinary cross-water
route appears.

Recipe tests must enforce each runnable recipe's topology and protected routes.
Fast fixed corpora run in CI; ignored 10,000-seed recipe corpora must produce 100%
valid final maps including fallback and target less than 1% fallback use.

Recipe-level benchmarks cover runnable patches at radii 12, 20, and 40. Composite
coverage measures Ring7 at radius 33 and Ring19 at radius 55 on the same machine,
including generation time, entity count, terrain-edit projection, and physical seam
traversal; Ring19 generation p95 may not exceed 3.5× Ring7. Perception benchmarks
separately cover fog recomputation. Mountain Range's release benchmark compares it to
Ring19 on the same runner and budgets generation p95 at no more than 2.5× Ring19;
character-camera collision remains below 2 ms p95. Review packs must include
deterministic reports and default, rotated, top-down, and character-camera captures.
Mountain Range additionally requires coast, watershed, both mountain tiers,
front-massif, and rear-silhouette views. Crystal Mountain requires Map, First Person,
and Third Person frames of the foot portal, natural tunnel, Gothic transition,
Crystal chamber, ascent, summit, Forest basin, and ridge, plus review-only illumination
and full-cutaway frames. Manual review must traverse every critical recipe route and
every open composite seam before that surface ships. Desert acceptance adds Map and
character-camera views of all three Desert Transition bands, Desert Plain's long
sightlines, Dunes from both a crest and trough, and the central oasis plus both
surrounding rings. Those frames establish presentation only;
Island acceptance adds Map and character-camera views of Sandy Islets' complete
component roster, Wooded Island's beach-to-ridge progression, and Ocean
Archipelagoes' open-water gaps, sandy clusters, landing, and wooded heart. Those
frames likewise establish presentation only;
typed recipe and composite tests remain the authority for coverage, elevation,
liquid isolation, date-palm blockers, seams, and reachability. The landed Two
Rings surface received its final visual and play approval at the reviewed wave head;
Mountain Range's 2026-08-03 delivery record contains its four-view deterministic pack
and a 45-step, eight-frame feature-only walk with exact arrival and focus assertions.
`@shrav-k` approved the overview and rear-silhouette static presentation. Hostile
suppression in that walk is presentation-only and cannot establish spawning or
gameplay. To unblock unrelated work, the same maintainer explicitly waived and
cancelled the release-only 128-seed and 10,000-seed corpora, generation and camera
performance diagnostics, and native human motion/control-feel replay. Those gates are
WAIVED, not passed, and this one-delivery exception does not weaken the evidence
requirements for later world or camera behavior changes.

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
