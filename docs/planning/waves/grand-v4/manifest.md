# Grand V4 integration wave

Status: September 26 revision02 approved; production geography transfer in progress
Wave: `wave/grand-v4`
Verified remote dev: `bb556963632de933b44fb75b1d306aca79258cef`
Playable integration base: `873a2c37eeb8eda23c11d398ce7d68d7965a64db`
Coordinator: root Codex agent. Epic: none; Linear reauthentication required.
Outcome: full-scale Grand remake with shrine progression, combat/marine traversal and durable arena resume.
Exclusions: new species/boss variants, advanced elemental control, tactical/network changes, V3 save compatibility.

## Why a wave
World, gameplay and shared app must be reviewed together; existing #222/#223 form the playable base. No individual leaf proves the expedition or save composition.

## Locked decisions
Latest user direction: [terrain playtest feedback](playtest-feedback.md) is the
first correction for the next session. Current landforms are too steep and rounded;
valleys and hills need broadly traversable surfaces, mountains need usable routes,
and forest character should follow the prior Dragon/Goblin V4 map while preserving
Grand V3's authored composition at larger scale.

1. Grand V4 is a new selectable streamed expedition based on playable 873a2c37; preserve all existing maps.
2. Mainland coastline-enclosed area including inland lakes is seven times canonical Grand V3 mainland, excluding ocean/offshore islands. Crystal Ascent is independently seven times original footprint, never scaled twice.
3. Preserve approved relative layout, intimate garden/lake, connected waterfall/library, separate straight Shadow tunnel, broad forest/world tree and offshore volcano calibrated to 45 seconds favorable unupgraded sailing.
4. Five one-time cumulative shrines: Fire damage/size/explosions; Air projectile/glider; Water swim/boat; Earth new player structures/run and never HP; Plant size/jump. Existing XP remains. Shadow grants teleport instead of Forest HP reward.
5. Teleport is X, visible supported ground within 12 units, 6-second cooldown, gaps allowed, walls/unloaded/occupied destinations refused without cooldown.
6. One dedicated atomic resume slot preserves combat, movement, progression, enemies, clocks, sparse destruction and partial damage. Death respawns at last shrine/start retaining progress. Explicit New Run resets.
7. Existing species/population budget first. New bosses, scent/group ecology and advanced elemental spells are deferred.
8. Accepted sailing/boat/swim/glider baseline is protected; natural wind influence on ordinary gliding stays 65 percent. Water evolution is compared independently.
9. Automated rendering is windowless. Logical claims require typed tests; native feel and 60 FPS remain explicitly unverified until measured.

## Shared foundation
Existing ArenaTick, exact TilePos, ArenaExpeditionSites, ArenaAvailability, finite V4 packages and atomic owner attachments remain authority. Gameplay owns ActorId widening and Grand progression/checkpoint. World owns Grand package/sites, finite overlay codecs and partial damage. Coordinator owns shared map-kind declaration and application adapters. New additive vocabulary lands in wave before consumers; no old-map semantics are changed merely to admit Grand.

## Dispatch queue
- id: L4
  title: Approved geography and terrain transfer
  order: orders/L4.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-r02-geography
  owns:
    - "crates/hex_schematic/src/v4/grand/** (except dressing and ground_cover)"
    - "crates/hex_schematic/src/v4/northern/mod.rs (shared overview producer schema)"
    - "assets/config/v4/grand-v4/world.ron"
    - "assets/config/v4/grand-v4/geography-r02.json"
    - "crates/hex_world_tool/src/grand.rs"
  dispatch_blockers: []
  merge_blockers: []
  fences: [preserve-gameplay-and-saves, approved-r02-relationships]
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null
- id: L5
  title: Landmarks and forest character
  order: orders/L5.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-r02-dressing
  owns:
    - "crates/hex_schematic/src/v4/grand/dressing.rs"
    - "crates/hex_schematic/src/v4/grand/dressing/**"
    - "crates/hex_schematic/src/v4/grand/ground_cover.rs"
    - "assets/config/v4/grand-v4/forest/**"
  dispatch_blockers: []
  merge_blockers: [L4]
  fences: [preserve-gameplay-and-saves, approved-r02-relationships]
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null
- id: L6
  title: Runtime geography admission and presentation
  order: orders/L6.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-r02-runtime
  owns:
    - "crates/hex_map/src/arena/streamed/**"
    - "crates/hex_map/src/v4/river.rs (shared presentation helper only)"
    - "crates/hex_world_runtime/** (envelope and sparse edit query only)"
  dispatch_blockers: []
  merge_blockers: [L4]
  fences: [preserve-gameplay-and-saves, approved-r02-relationships]
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: dispatched
  pr: null

```yaml
- id: L1
  title: World content and publication
  order: orders/L1.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-v4-world
  owns:
    - "crates/hex_schematic/**"
    - "crates/hex_map/** (except water-field files explicitly coordinated)"
    - "assets/config/v4/grand-v4/**"
    - "tools/grand_*"
    - "assets/art/** (new Grand assets only)"
    - "docs/planning/waves/grand-v4/manifest.md (L1 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: integrated
  pr: null
- id: L2
  title: Gameplay progression and checkpoint
  order: orders/L2.md
  ticket: null
  authority: gameplay
  builder: worker
  branch: feat/grand-v4-gameplay
  owns:
    - "crates/hex_arena/**"
    - "assets/config/arena/grand-v4.ron"
    - "docs/planning/waves/grand-v4/manifest.md (L2 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: integrated
  pr: null
- id: L3
  title: Finite persistence and water field
  order: orders/L3.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/grand-v4-runtime
  owns:
    - "crates/hex_world_runtime/**"
    - "crates/hex_world_contracts/**"
    - "docs/planning/waves/grand-v4/manifest.md (L3 state/pr only)"
  dispatch_blockers: []
  merge_blockers: []
  fences: []
  selector: {concerns: [full], full: true}
  evidence: motion-or-feel
  sizing: {model: inherited, effort: inherited}
  state: integrated
  pr: null
```

## Ownership and hotspots
Lane paths above are exclusive. Root owns hex_core shared arena vocabulary, hex_game, tools/arena.py, shared docs, shader composition and Cargo coordination. Request an explicit seam before touching another lane. All lane commits remain identifiable. No worker stages another worktree. Manifest updates limited to own row; coordinator resolves composition.

## Territory
#223 feat/water-lab -> #222 wave/northern-archipelago -> dev. Both draft PR heads are ancestors of this candidate. #220/#219 and #210-213 are historical overlapping ancestors; preserve and do not merge tips blindly. #196 lattice fusion is separate and untouched. The targeted GitHub dev fetch succeeded after an initial pack failure; the verified base is still `bb556963632de933b44fb75b1d306aca79258cef`. Actual GitHub remote is github; origin is an old local checkout. Published branches and PRs remain untouched.

## Integration order
Source lanes start together. Root lands vocabulary first, then coherent world/runtime/gameplay commits, then shared app adapters, checkpoint UI and water presentation. Shared heavy Cargo target is serialized by root; workers may run pure Python or lightweight tests only with agreed target. Final candidate targets dev, no force-push or automatic old PR closure.

## Combined acceptance
Full-size measurable terrain and independent source scale; all five shrine orders/idempotent reward; valid/refused teleport; old maps unchanged; staged actor admission over loaded terrain; land/boat/glider/combat save restart and partial damage; interrupted save preserves old checkpoint; death/New Run; unload/reload and repeated circuits; windowless full map and named landmark frames; native movement/30-minute profiling remains separate. Focused iteration then full selector gate; failures are recorded, never relabeled green.

## Stop conditions
Resolve source conflicts, invalid shared facts, actual budget failures and failing exact checkpoints before declaring completion. Preserve approved size and behavior. Missing Linear does not block work.

## Injection log
- Sep25: user approved implementation; three isolated source lanes and root integration started.
- Sep25 visual review: the first full-size captures exposed omitted detailed inland water,
  unreadable cave lighting, a blocked Plant approach, insufficient World Tree canopy scale,
  a missing Earth focal object, and a buried Frozen Woods review camera. These are repairs
  within this integration wave. The completed gameplay worker takes a separate world-only
  repair assignment in `hex_map/src/arena/streamed/render.rs` and its liquid tests; the world
  worker retains Grand schematic content and route tests. Root owns `hex_game` lighting and
  review cameras. The water worker investigates the reproduced shoreline stippling. Each
  shared file still has one writer. Failed exact-head frames remain preserved; corrections
  require a new immutable package where authoring changes and fresh captures.

## Historical September 25 integration checkpoint
Source lanes and the first visual repairs are integrated. The
[candidate guide](../../../development/grand-v4.md) records launch, owner contracts,
focused acceptance commands, and the deferred backlog. At
`36eb903487124e953103ed0ee9ea6c789f11984f`, immutable package fingerprint
`13408690208396973052` passes separate-process land, moving-boat and glider resume,
three streaming circuits with 31 stops, and fourteen authored parties containing
127 stable enemies. Save-menu requests, active combat state, sparse destruction and
partial health participate in the restart comparison. The revised package also
passes seven geometry checks and the focused world suite, including persistent
tree cuts and exclusive water-boundary ownership. These are logical and bounded
residency results; they make no native movement, appearance or frame-rate claim.
At that checkpoint, the full gate encountered inherited V3/map lint failures.
The September 26 record below supersedes its validation and repair status.

## Historical September 25 review checkpoint before user play

Combined draft PR: https://github.com/chillgamerboys/bevy-hex-game/pull/224.
The source offered at that checkpoint was `53a66990411ddf01ae0840830c62d2d7e306bb03`
with immutable final4 package fingerprint `13408690208396973052`.
A fresh twelve-view windowless correction pack completed mechanically at that head.
Static inspection confirms restored inland water, a broader dominant tree, a visible
Earth marker and an unburied Frozen Woods camera. It does **not** approve the full
presentation: garden dressing is sparse, library depth/identity remains weak,
the waterfall is rigid, and the ordinary forest has a conspicuous rectangular edge.
Paired bay frames retain visible stepped/striped surface detail; native motion is
still needed. The remaining eighteen matrix views and native route are pending.

At the same source, the required combined workspace Clippy command still fails in
`hex_map` with 517 errors in inherited V3/map files; downstream full-gate checks
remain not run. The preceding dependency audit and format checks pass. Focused
logical evidence above remains useful but does not replace this failed gate.

The user's additional river direction is a relatively uniform travelling wave
moving downhill. A world-only follow-up is being prepared in the runtime lane:
author exact Directed/Waterfall links, keep physical surfaces unchanged, and drive
steady downstream shading from the saved ocean clock within existing mesh ownership.
It is not part of the launchable final4 candidate until compiled and validated.
The user explicitly requests stopping development at 10% weekly allowance remaining
and opening the most recent valid map; this replaces all historical usage floors.

## September 26 resumption and corrective source lanes

The user explicitly resumed the work and authorized continuing after the weekly
reset until the approved Grand V4 project is complete. The live account returned
0% used and a new weekly reset timestamp of 1791052758, replacing 1790450297.
Their new 2% floor applied only before that reset; the prior overnight pause and
10% stop are superseded. The explicitly named `Add late-game large spells` chat
was told to resume once, as requested. An hourly thread heartbeat was initially created for continuation. The user later
asked to remove the hourly check; `finish-grand-v4-expedition` was deleted through
the app on September 26. Development continues in the active chat. Deferred features remain backlog.

The exact traversal/forest reference is the prior Forest expedition at `b314a9d`,
including its authored lowland, one-level grades, shelves and preserved rendered
tree assets. The original Grand V3 remains the composition/footprint reference.
This is an authorized terrain redesign with those references, not a new literal
cell trace awaiting approval.

Corrective ownership within this existing wave:

- Terrain lane: Grand surface/cave/site composition and terrain tests in
  `hex_schematic`, preserving exact footprint and meaningful vertical geography.
- Forest lane: Grand `dressing.rs` and tree recipes/assets; palette additions are
  coordinated through the terrain writer, runtime substance aliases through root.
- River lane: exact directed-liquid graph and shading/material lifecycle in
  `hex_schematic`/`hex_map`, preserving physical surfaces. Coordinate compiler
  hooks and changed datums with the terrain writer.
- Root: combined source, runtime adapters, captures, logical/native acceptance,
  documentation and the single PR into `dev`.

Reference recovery and source preparation start independently. A fresh immutable
package and full visual review depend on all three composed source lanes; ordinary
walking tests depend on the revised terrain, and river shader review depends on the
new directed-liquid package. Shared heavy Cargo work uses the cooperative lock in
the task work directory, also supplied to the large-spells chat. Each source file
still has one writer. No candidate is accepted by a narrow route validator alone.


## September 26 integration status

The terrain, source-lake/headwater containment, forest recipes and distant forest
publication are composed with the garden/library architecture. Exact scale and
water datums remain unchanged. Two material conflicts in the combined garden were
reproduced and repaired before issuing a replacement package. The fresh package
build and all current walking/sailing results must be recorded before declaring
this candidate traversable. The original failed receipts remain preserved.

The inherited V3/map Clippy block has been repaired in identifiable maintenance
commits. Scoped combined map checks passed 58/59 tests; the remaining fountain
fixture assumed opaque topology while requesting translucent water. Its corrected
dual-style lifecycle and closed-water tests pass, as do the affected terrain
contracts. These scoped checks do not replace the final selector-chosen gate.

The static matrix now contains 36 views, including admitted ground-level forest
and fountain cameras and a reverse library view. Capture admission rejects stale
authoring signatures and labels subsets as partial diagnostics. Continuous
windowless river/forest sequences are implemented separately; native control feel
and the 30-minute session remain pending.

Disk capacity is a separate validation constraint. Package-scoped development-cache
cleanup was explicitly approved by the user after a Cargo dry run and exact
filesystem snapshot. Revalidation under the shared lock succeeded; Cargo removed
only `hex_game` and `hex_map` development artifacts, recovering about 26 GiB.
Source, packages, saves and captures were preserved. No other cache was cleaned.
The large-spells chat received the requested post-reset resumption message; its
prototype work remains separate from Grand’s retained advanced-spell backlog.

### Package04 physical checkpoint

Clean source `c8f1b8e200442621137c1182f4b65e81f3594fb8` strictly compiled immutable
package04, fingerprint `4216780698166138326`, preserving the 653,282-column mainland
and 22,183-column Crystal footprint. Full-width mountain grading removes the
polyline turn discontinuities; the upper western route remains on the existing
coastline. An outer library collar closes unintended roof-side openings, and an
irregular elevation contour replaces the straight mountain material boundary.

The actual-package production-controller run passes all eleven walking routes:
both riverbank exits, four western cross-country hill directions, the eastern
valley, garden ascent, island landing/ascent, western massif and Crystal shoulder.
Each uses ordinary walking without jump, glider, flight, teleport or upgrades;
static-object collision remains active. Local obstacle steering belongs only to
the test driver. The unupgraded authored sailing route measures 43.9 simulation
seconds, against the approximate 45-second separation request.

Separate-process land/boat/air restoration passes with exact owner state, active
projectiles, destruction and partial health retained. All fourteen authored parties
admit their 127 stable enemies. Three streaming circuits complete 31 stops, with
high waters of 48 source chunks, 17 detailed chunks and two jobs. These paused
circuits establish bounded residency and edit revisits, not native FPS or memory.

Independent final-column inspection finds all 24,125 audited western/Crystal
mountain-approach tread columns connected through dry terrain and cave floors/stairs
with four levels of clearance. The 236 steep directed top-view edges are intentional cave-mouth drops; upper-hall access
continues through the covered library. This graph excludes objects/body width and
does not replace the controller run. All 9,768 directed river columns have exact
neighbor targets, descend or stay level, and terminate at the lake or sea without
cycles. Topology does not establish the moving-wave appearance.

A shared Cargo cache initially linked old gameplay into a new app test. Refreshing
all local dependency entrypoints under the shared build lock reproduced a current
build, whose linked recovery markers and staged-respawn, ground-camera and motion
fixture tests pass. The mixed-cache result is preserved as diagnostic evidence.
Future cross-worktree builds must establish local dependency freshness explicitly.

Five package04 diagnostic views at `ea4347837d5acb7381b5bdbff0f1e545a1898fb7`
were inspected at full resolution and independently reviewed. They show closure
of the library aperture, the irregular mountain material boundary, and useful
forward/reverse forest framing. Library room contrast remains weak. A local warm
stone floor/pier finish preserves every tested occupied interval, liquid and stone
policy; all 32 Grand schematic tests and its map material-alias test pass. A fresh
immutable package and recapture are required before approving the finish.

Water inspection found a 0.261-cycle phase jump at an actual waterfall lip. The
Grand-only shader now uses one continuous horizontal/height phase chart; geometry,
hard hex normals, physics and ocean materials remain unchanged. Focused shader
input tests and temporal presentation review remain pending at this checkpoint.

The gameplay audit found that live death-recovery waits could advance combat.
Recovery now holds simulation clocks, enemies, projectiles and effects while the
safe destination loads. The new shrine/start waiting regression and all 40 Grand
gameplay tests pass at `8092f7e`. Staged Continue retains its existing pause.

The workspace/all-target/all-feature Clippy diagnostic passes at `8e2b121`.
Cloud checks from the older `828c144` exposed a stale Sandbox row count, a stale
legacy vegetation catalog fixture and a redundant rustdoc link; narrow repairs
are integrated. Independent legacy V3 generation failures are being classified
against the starting baseline, without reviving terrain-validator redesign.

An authored ground-cover gap remains: current understory is small trees and a few
floor props, with no grass tufts. The world lane is adding bounded nonblocking
voxel tufts through the existing overview companion, rendered only over matching
published detailed support, with no new streaming requests or collision facts.

The full static/temporal matrix, exact combined CI gate, native movement and
30-minute measured session remain open. This checkpoint is physical validation,
not final terrain taste or presentation acceptance. Linear remains disconnected.

### September 26 approved revision02 transfer

The user explicitly approved the corrected shared topology and authorized the actual
map. This lifts the mockup-only stop. The frozen oracle and camera/source identities
are recorded in `approved-r02/`; the production authoring document owns subsequent
measurable geometry, rather than scattering unrelated coordinates through consumers.

Keep one connected massif, Earth temple at the bottom of a real Crystal climb, the
continuous Frozen Woods route from its upper exit to the hidden mountain lake shore,
a garden island within the lake, and enclosing steep peaks that screen the lake from
the valley. The lake outlet uses a dominant near-vertical fall and connected shorter
drops. World Tree and small camps compose the lower lake/outlet. No walled fort.
Volcano keeps a broad landing-to-crater route and substantial sea separation.

L4 owns geographic authoring, terrain/voids/exact sites/water/biomes. L5 owns dressing
and ground cover through L4's common frames and supports. L6 owns runtime admission
and distant/detail presentation. Root owns package tooling, game camera/adapters,
combined source integration, evidence and docs. Independent work starts at once;
L5/L6 merge after the shared L4 facts exist. Heavy Cargo runs remain serialized.

Before dispatch, pending ground-cover commit a8004de and UI fixture a792c19 were
integrated as 3a60e5f and 890c1e4. Their original refs remain. Legacy V3 orientation
repair 5605bda stays banked and is not a map-rebuild prerequisite. Existing f4824b7
source, immutable packages and saves remain recoverable; none is overwritten.

GitHub dev was independently confirmed via API at bb556963632de933b44fb75b1d306aca79258cef.
Broad fetch reported unresolved pack deltas; no successful broad fetch is claimed.
Open PR heads and the original stack are unchanged; do not merge historical tips.
The combined historical candidate has 750 changed paths relative to dev before this
amendment. Linear reconciliation remains unavailable. Draft #224 remains the delivery
PR targeting dev, and published history is preserved.

First checkpoint: complete plain terrain, actual caves/stairs, water and exact sites
at measured scale; fresh matching cameras establish transfer fidelity before dense
dressing. Final combined acceptance is still required. Prior package04 passes are
historical, not evidence for this changed geography. No native play window is opened
without the user's play request or approval of a named live review.

L6 additionally owns the narrow shared river-material helper in `hex_map/src/v4/river.rs`
and a read-only sparse edit range query in the finite session. These reuse detailed
water presentation and invalidate far faces without changing water physics or the
persistence format. The common authored-camera/inland-water schema landed first as
7262bf3; application consumers follow that producer vocabulary.

Independent continuous-oracle calibration sampled the actual hex lattice and flood
filled its largest six-neighbor mainland component. Scale0.8838784715190828 gives
exactly653282 mainland columns, excluding a53-column detached islet, with no boundary
trimming. Vertical scale stays1. The island is shifted to retain an800-unit geometric
shore gap; actual sailing remains pending. Integer axial recentering[361,87] keeps
the exact measurement while limiting the proposed full ocean disk to radius1052.
The hash-bound numerical receipt is `approved-r02/production-calibration.json`.
The Rust compiler must independently reproduce these figures before package admission.

Root owns the narrow explicit-bound validators in Northern forest/ground-cover data
(284bc7d). Existing no-argument validation retains its legacy limits; current Grand
producer/consumers pass the admitted radius and levels. Five current-source app camera
and ground-support tests pass at42aec8e under the serialized fresh-workspace CI build;
23 Python capture/motion/package provenance tests pass. These are focused checks, not
new-world compilation or final combined acceptance.

The approved topology puts the starting bay farther from the volcano than the
nearest western mainland shore. The sailing harness now measures both published
starts (`sailing_start` and `sailing_start_bay`) independently with identical
production wind and unupgraded input. The 800-unit geometric shore separation is
not a claim that a bay departure takes 45 seconds; record both actual timings.
The western start must have ordinary shore access. Sailing results remain pending
until the new immutable package exists.

### Revision02 first game transfer and foothill correction

The seven actual game views from `4ad2b82d` / `compiled-r02-plain-01` are preserved
as a failed plain-terrain diagnostic. The user rejected the steep mountain bases.
Cave-cover inflation, pre-carve overview tops and inland angular-noise artifacts
are repaired in source. A first elevation-compression approach still creates a
steep inner belt; it is being replaced by a spatial mountain-base study. Keep the
hidden lake enclosure, connected complex and Crystal–Frozen shore relationship.
Do not equate source-model views or isolated route passes with broad traversal.

At `718ff10`, independent and Rust measurements agree on 653,282 mainland columns
with uniform horizontal scale 0.883566890744603. Crystal's actual outer feature
contains 22,201 columns (7.00568 times 3,169), replacing the old unused reservation
count. These source measurements must be repeated if the spatial terrain changes
the coast. The sampled shore gap is approximately 783.812 units; no new controller
sailing result exists. No island repositioning is implied by this measurement.

Physical channel drainage is now a finite reverse graph over actual wet neighbors
leading to receiving lake or ocean water. It is separate from the travelling wave
highlight chart. The composed constructor passes after moving the lower library
away from a plunge pool, while current bank/intake regressions are under repair.
Current rendering work compares bounded stepped Grand distant terrain against the
immutable first package; that renderer-only comparison cannot accept new geography.
The next actual map screenshots require a fresh combined package. Full dressed
presentation, ordinary traversal, restarted saves, residency, combined CI and native
feel/performance remain open; historical package04 results do not clear them.

### Spatial mountain-base implementation

The spatial revision is integrated at `39f5260`: broad lower aprons, smaller upper
cores, varied coastal headlands and continuous Crystal supports replace the failed
height-compression approach. Cave routes fit the actual new rock cover, including
the lowered western summit. All eight route ribbons (42,033 supports), their
centerlines and closed-cave cover pass the focused source checks. These checks do
not establish cross-country controller movement or appearance.

An independent whole-lattice survey reproduces 653,282 mainland and 22,201 Crystal
columns. In the dry front below 70 units, 92.32% of western and 87.21% of lake-front
neighbor edges meet the ordinary one-level step limit. The survey excludes layered
routes, objects and body clearance. Four fixed geographic crossing/uphill probes
are now published as review cameras and consumed by the walking harness; they are
not chosen by searching for successful paths.

The combined Grand unit run on that source passed 39 tests and failed six. Two
failures expose small real ledges at the Shadow well exit and volcano landing;
their repairs are in progress. One test incorrectly required the Shadow exit to
equal the Crystal stair endpoint. Two dressing defects account for the remaining
three failures: a root contacts removed natural terrain, and garden finishes
overlap shrine solids. The dressing and stale-endpoint repairs are integrated as
`9b5859b`; eight focused dressing tests pass. Combined validation remains pending.

The stepped far-terrain prototype is banked, not integrated. Its old-package
east-camera comparison still shows coarse terraces and mixed detail density, at
337.7 MB of raw terrain buffers. The next corrected-geography package uses the
existing renderer. That comparison neither accepts the new terrain nor establishes
performance. First revised actual game pictures remain pending a fresh package.

The follow-up join repair (`42a0d91`) adds a ninth published ground-level gateway
through the Crystal stair foundation, keeps Shadow's final landing level, and
softens the volcano's initial walkout. All 42,247 ribbon supports, centerlines and
cave-cover checks pass; the two remaining combined dressing/Shadow fixtures also
pass. The ordinary-walking harness now observes every simulation tick and bounds
unsupported descent and airtime using production movement facts. Its Python
receipt checks pass; the new Rust controller fixtures still require execution.

Strict emitted-flow validation exposed a sea-level river pool enclosed by its own
terminal bank. A river cell at sea height is now required to reach actual ocean
water, and the seaward bank cap is removed (`c1f6038`, `5e037d8`). This changes the
measured mainland to 653,261 columns: 21 below the nominal 653,282 target, or
6.999775 times the original. The scale is unchanged. The approximate-area contract
now explicitly allows 0.01% (65 columns) for authored coastal openings, while
retained explicit-row worlds still require their original exact count. Receipts
report actual area, target, tolerance and ratio separately; no report may claim
that the new actual count is exactly sevenfold. The full Crystal footprint remains
22,201 columns. The strict emitted-flow acceptance now passes: all authored
channel paths reach their receiving lake or actual ocean through nonuphill wet
neighbors. A fresh plain package and game renders remain pending.
