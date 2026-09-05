# V4 world platform

V4 has an explicit `hex_v4` explorer and fresh formats. It opens compiled world
packages directly; it does not install the V3 scenario, whole-world readiness,
snapshot, save or combat plugins. The frozen V3 reference is
`bc06a8969532b807ec677928eee304bc28399386` (PR #219). Its outstanding checks are
recorded separately in the [wave manifest](../planning/waves/v4-foundation/manifest.md).

## Authority and data flow

| Owner | Implementation | Responsibility |
| --- | --- | --- |
| Shared contract | `hex_world_contracts` | Exact integer positions, immutable products, availability, edit commands and shared aperture predicates; no Bevy or gameplay dependency. |
| World | `hex_schematic::v4` | Strict runtime-loaded RON intentions, shared boundary resolution, deterministic operators, constraints, compiled terrain and summaries. |
| World | `hex_world_runtime` | Chunk residency, exact queries, atomic terrain/object edits, fresh partition saves, history, knowledge storage and disclosure products. |
| World | `hex_map::v4`, `hex_perception::v4` | Disposable resident terrain meshes; local revision-dependent sight, illumination and explicit visible absence. |
| Gameplay | `hex_units::v4` | Bounded route planning and continuous step queries through the same exact terrain contract. |
| Shared integration | `hex_game::v4`, `hex_objects::v4` | Runtime wiring, actor-local controllers/checkpoints, private exploration, stock-art fragments, picking, cameras and atlas. |
| World tooling | `hex_world_tool`, `tools/world.py` | Prebuilt validate/compile/preview workflow, strict provenance and executed-work measurements. |

The pipeline is editable `WorldSpec`/`RegionSpec` → resolved shared boundaries →
regional compilation → immutable `ChunkPackage` products → resident authority →
independent gameplay and presentation consumers. A recipe, placed region, storage
chunk, render batch, interest owner and encounter are different identities.

Editable sources, compiled products and runtime modifications are separate records.
Generated chunk files must never be edited as authoring sources. Ordinary source
changes do not require Rust edits or a compiler rebuild. See the executable commands
and authoring vocabulary in [AUTHORING.md](../../assets/config/v4/AUTHORING.md).

## Compilation and composition

Region instances retain unique stable identities even when they share a recipe.
Integer `WorldHex` coordinates use checked i64 addressing; presentation rebases into
bounded local coordinates. Geometry retains canonical material runs, empty volumes,
liquids, complete object footprints, stacked supports and exact headroom.

One world resolver owns every touching-region boundary before either neighbor
compiles. Shared terrain and water datums, directed flow, openings and dry crossings
become both regions' inputs. Cross-boundary caves/bridges use explicit canonical
opening constraints. Source constraints remain named through the compiler; conflicting
hard overrides or obstructed required routes fail instead of being silently repaired.
Scenic observation anchors do not impose traversability.

The first cache deliberately uses whole-region and pre-feature geometry stages.
Boundary dependencies participate in region keys. Independent clean compilation must
equal cached compilation after source or seam edits. Current compilation is serial;
input-order tests do not imply a worker-count benchmark. Compilation still assembles
the full output in memory. Fine stage caching, distributed compilation and a universal
procedural geography provider are future extensions, not hidden current capabilities.

The radius-187 fixtures contain 105,469, 210,938 and 738,283 columns. The seven-region
fixture combines caldera, desert, wooded upland and snowy spire recipes using the same
operators. Existing stock tree, crystal and scrub exports carry blueprint provenance.
The procedural limestone tower intentionally uses exact proxy geometry.

## Residency and local changes

Storage remains 16×16 axial chunks. `WorldRuntime` holds the union of actor/operation
interests, retains a reversal buffer, bounds workers and publications, and rejects
late jobs by source epoch and revision. Cancellation continues to occupy a worker
slot until that worker finishes. Route and transaction pins keep dependencies alive.
Load timing measures successful job launch through queryable admission; the explorer
uses it, queue depth and travel speed to prioritize a bounded area ahead of a route.

`WorldQuery` returns `Ready`, `Unloaded` or `OutsideWorld`. Unavailable chunks never
mean air. A `WorldRoute` proves every waypoint partition revision; interpolation uses
the same surface and lateral-aperture predicates as discrete steps. It is a small
continuous-motion adapter, not a second physics engine or a new terrain format.

Terrain edits and object edits are separate atomic command types. An object command
carries complete before/after records once and checks all old/new footprint revisions.
Identity-tagged clipped influences preserve overlapping objects even with unloaded
roots. Runtime-created objects use transaction-derived IDs in a reserved namespace.
The caller protects actor supports and body volumes; the runtime protects world
semantic constraints. No turn or global encounter clock governs these operations.

Stock art is a disposable projection. Terrain proxies remain until compatible art
and its exact occupancy suppression can publish together. Changed object sources
retire old fragments before new fragments are admitted. Unsupported art stays an
explicit proxy with diagnostics. Mesh preparation and publication use bounded queues;
unloading and rebasing retire only the owning presentation resources.

Terrain side occlusion uses an immutable one-hex halo from actually published
neighbors, including their exact object suppression. Loading, editing or retiring a
chunk prepares that chunk and at most six existing neighbors asynchronously, then
publishes the complete preflighted transaction in one exclusive operation. A
neighbor that is only resident cannot remove a visible boundary wall. Halo changes
never create logical occupancy or picking identities, and do not recursively
invalidate other halos. This bounds each preparation/upload transaction to seven
chunks; its measured frame cost must be assessed separately from steady rendering.
An origin change drains obsolete roots before admitting the new view.

## Persistence, knowledge and disclosure

A base manifest references independently addressable immutable chunks. Runtime saves
store changed partitions, transaction history and an atomic current head. Terrain and
object edits can include opaque owner attachments in that same durable commit. The
explorer binds actor positions, selection and step mode to the world checkpoint with
compare-and-swap protection. A restarted controller resumes at its exact saved
support; it does not reconstruct authority from an animation pose.

Fresh saves are bound to world ID and exact compiled base. No V3 migration is provided.
Save deltas remain ordered/idempotent and preserve unchanged partitions. Historical
transaction bodies and fine knowledge are paged. The loopback acceptance tool runs
two receiver processes, kills the first after its durable ACK, and verifies exact
restart/replay. It is a protocol integration test, not a production V4 lobby,
authentication service or live entity replication implementation.

Sight consumes local revision-tagged terrain, object occupancy and light influences.
Each principal owns observation and memory. The selected renderer view cannot grant
knowledge, and an unrelated party awaiting a turn cannot block another observer.
Visible missing supports/landmarks explicitly invalidate memory; hidden remembered
facts remain until their absence is observed. Knowledge writes run through bounded
background jobs. Save completion and automated capture wait for them to settle.

The atlas reads coarse geographic summaries and the selected principal's permitted
exploration/landmarks. Geography is public in this authoring explorer. It is not the
production gameplay fog/disclosure UI. Dormant fine terrain is not loaded to pan the
atlas. Historical landmark metadata is paged; the UI labels an incomplete catalogue.

## Current operational limits and evidence

Fine residency, query work, mesh preparation, observer caches and unsaved work have
explicit budgets. Exceeding them is a named refusal, never truncated accepted output.
The finite catalogue and its indexes still grow with world metadata. Save/knowledge
heads still grow with modified partition and transaction descriptors. V5 will need
paged catalogue metadata and sharded or compacted durable heads before unlimited
worlds or indefinitely growing sessions. These are distinct from resident terrain.

Source replacement cannot retain old consumers while changing material or shared
boundary policy. Such a change requires a fresh runtime and consumer construction;
the explorer opens one immutable revision per process. Source RON hot reload and
cross-base save migration are not installed.

Use `tools/v4_review.py` for windowless, source-bound captures. It checks actual work,
exact package/script identities, completion, PNG integrity/coverage and bounded
settled frames. It records uncapped real frame intervals and samples only its owned
game process for RSS; RSS is not a separate GPU allocation measurement. Dirty or
map-test captures are diagnostic, never promotion approval. Static review and typed
motion evidence do not clear `HUMAN-MOTION-PENDING`.

The four-hour active-authoring target and 60 FPS target require measurements; neither
is a promise inferred from tests. Generation, validation/publication/preview, build
waiting and human design/review time must be reported separately. Cold process launch
does not imply cold OS storage caches. Detailed encounter scheduling/joining/merging,
infinite drainage and unlimited world simulation remain separate workstreams.
