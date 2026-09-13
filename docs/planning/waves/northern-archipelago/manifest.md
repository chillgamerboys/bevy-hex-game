# Northern Archipelago implementation

Status: playable marine checkpoint; native acceptance pending and combined CI blocked by inherited lint failures.
Base: user-selected expedition `13fdb5aca0948e6364aaa193a9bc7b846161460e`.
Candidate: `wave/northern-archipelago`. Local work only; no dev/main or remote writes.
Topology: one wave, because the new world, shared surface renderer and exploration controller have one meaningful combined runtime checkpoint. Root is the integration owner. Existing island-biomes PR212 is frozen V3; PR220 is the earlier V4 foundation already incorporated. Neither branch tip is imported. PR213/219 remain separate. Linear reconciliation unavailable; no tickets created.

## Locked decisions

The user's approved Northern Archipelago plan is authoritative: approximately2400 world units across, exactly11 islands in3 clusters separated by300–500 water units; crater~300 high; cold snowy Viking landscape;4 buildings and1 field; dry bay spawn. The first checkpoint has no enemies/objectives/victory and no fluid simulation. The later user-authorized marine extension below adds surface sailing and swimming, with optional enemies only after that checkpoint. Normal expedition player and spells, G glider, F collision-aware freeflight80/160. Ocean waves amplitudes1.2/.6/.2, wavelengths110/180/60, periods18/25/12; shared static liquid levels and dynamic visual surface. Water is indestructible; all solids carve; unloaded is never air. Original Forest package and running game stay untouched. No native automation or launch without user request; user owns motion/feel checks.

## Foundation

`ArenaMap::NorthernArchipelago`, `ArenaMapCapabilities`, compact-run `solid_at`, `ArenaResidency` and `ArenaAvailability`, `ArenaStreamInterest` establish shared vocabulary. Ocean presentation profile is world-owned and published by the water lane. Current legacy maps retain full-publication behavior. New map never constructs a whole-world VoxelMap or expanded voxel BTreeMap. Resident max512, rendered max256,2 IO workers/products; async V4 preparation/halos; coarse distant terrain and ocean are not collision authority. Sparse session edits survive unload and clear Restart.

## Queue

```yaml
lanes:
  - id: L1
    title: Terrain authoring and package
    order: orders/L1.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/northern-geography
    owns:
      - "crates/hex_schematic/src/v4/ (new ocean/northern authoring only)"
      - "crates/hex_world_tool/ (northern authoring command only)"
      - "assets/config/v4/northern-archipelago/"
      - "tools/northern_*.py"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: integrated-awaiting-combined-validation
    pr: null
  - id: L2
    title: Ocean surface presentation
    order: orders/L2.md
    ticket: null
    authority: world
    builder: worker
    branch: feat/northern-ocean
    owns:
      - "crates/hex_map/src/ocean/"
      - "crates/hex_map/src/liquid_render/ (shared surface conversion)"
      - "crates/hex_world/src/battle_sky.rs (profile configuration only)"
      - "assets/shaders/ocean.wgsl"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: integrated-awaiting-combined-validation
    pr: null
  - id: L3
    title: Exploration controller
    order: orders/L3.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: feat/northern-flight
    owns:
      - "crates/hex_arena/"
      - "crates/hex_core/src/arena/exploration.rs (availability helpers only)"
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: integrated-awaiting-combined-validation
    pr: null
  - id: L4
    title: Streamed world adapter
    order: orders/L4.md
    ticket: null
    authority: world
    builder: worker
    branch: wave/northern-archipelago
    owns:
      - "crates/hex_map/src/arena/streamed/"
      - "crates/hex_world_runtime/ (stream-safe carve overlay)"
      - "crates/hex_map/src/arena.rs (stream admission wiring)"
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: integrated-awaiting-combined-validation
    pr: null
  - id: L5
    title: Application composition
    order: orders/L5.md
    ticket: null
    authority: shared
    builder: worker
    branch: wave/northern-archipelago
    owns:
      - "crates/hex_game/"
      - "tools/arena.py"
      - "docs/planning/waves/northern-archipelago/"
    dispatch_blockers: []
    merge_blockers: [L1, L2, L3, L4]
    fences: []
    selector: {concerns: [map_unit, map_generation, map_contracts, residual, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: integrated-awaiting-combined-validation
    pr: null
```

## Acceptance

First prove full scale and final ocean mesh with all silhouettes, tallest peak and one dressed sector before further decoration. Exact island/cluster/shore/bed/spawn facts come from typed generation output. Streaming queries, carve reload/reset, movement boundaries, pause and legacy regression checks are logic evidence. Fresh windowless whole-map/bay/settlement/snow/waterline captures establish static visuals. User's three short checks establish waves, fast crossing/reversal and walking/water/freeflight feel. Measure3 circuits; no memory growth after warmup; bounded queues; streaming/publication target<2ms CPU p95, wave-on/off frame regression<10%. Repository-selected combined gate once assembled; inherited V3 lint remains explicit.

## Current integration checkpoint

The combined code is under validation. L1 full-scale proxy package has 5,852 chunks, 11 islands, a 303.1-unit peak above sea, and a deepest bed 138.95 units below sea. The proxy has 63 trees, four buildings and a field; full dressing follows visual/runtime review. L2 wave CPU tests and L3 flight/collision tests passed. L4 sparse carve unload/reload tests passed.

The first exact-head overview at `d1c4406` mechanically captured but failed independent static review: framing was too distant, mirrored lower-sky clouds dominated transparent water, the outer ocean edge was visible, and proxy/ocean shoreline interpolation disagreed. Those surfaces have targeted repairs awaiting fresh captures; the failed image is not acceptance evidence. Native motion and GPU performance remain unverified.

The first actual-package three-circuit authority benchmark passed residency, queue, edit-retention and reset checks; all-pump p95 was 0.72–0.74 ms and the source count settled at 180 across each parked lap. A stronger receipt now measures active-pump p95 separately and process RSS after each lap; replay remains required. This benchmark does not include the renderer or player controller. The same-frame menu switch regression passes: selecting a streamed map must switch adapters before applying the reset, without waiting for the following PreUpdate.

User steering during implementation: retain at least 20% account allowance; no automatic reset or timers. Increase ordinary expedition starting walking speed by 25% to 5.90625 units/s, in the current expedition and this candidate. Normal jump rise is now 1.38 units; actual controller tests verify three-voxel ledge traversal and four-voxel rejection from both directions. The user requested a restart of the current expedition once these small movement changes are ready; that authorizes its native relaunch independently of the archipelago review.

The deferred ground-glider request is implemented in this candidate: G opens on land without lift or changed walking/jumping, and landing leaves the canopy open. The isolated original expedition checkout remains an earlier source checkpoint; the current candidate launcher uses this implementation.


## Offline marine extension — September 12

User priority: finish the basic sea map, improve additive waves/crest foam/shore
interference, then portable sailing, swimming with finite oxygen, wind for sailing
and gliding; island enemies are optional after those features if allowance permits.
Keep at least **20% remaining**. No reset credits, timers, or unattended native UI
checks. The user is offline; use these initial playtest defaults without waiting.

- **B** deploys/folds a portable timber sailboat near admitted, sufficiently deep
  ocean water. Momentum survives toggling; tailwind adds propulsion, crosswind and
  headwind lose speed. W provides a slow paddling fallback; S brakes. Boat hull and
  player sweep against exact solids and unloaded boundaries.
- **Space / Ctrl** rise/dive while swimming. Oxygen lasts **90 seconds**, refills
  over six seconds when breathing, and empty oxygen costs 10 HP/s. Use physical
  eye submersion, not the third-person camera. No passive healing.
- Prevailing wind starts at approximately **10 units/s** with slow modest gusts.
  Glider lift and stall consume air-relative speed; collision and prefetch consume
  ground velocity. Opening equipment supplies no momentum or altitude.
- World publishes one immutable analytic surface sampler through a core contract;
  authoritative current wet spans/residency gate gameplay sampling. Decorative
  ocean never authorizes movement. The same pause/reset-aware run clock drives
  surface sampling, wind and rendering. No currents or volume solver.
- Keep three additive swells and their two-unit envelope. Weak cached shoreline
  reflection and crest-driven foam share the surface phase and derivatives.

Implementation remains a wave. First land the behavior-neutral core surface/wind
contract. The existing world ocean lane owns surface math, its renderer and adapter;
existing gameplay lane owns boat/swimming/oxygen/glider wind; root owns input, HUD,
boat presentation, run clock wiring and combined checks. No lane imports another
owner's implementation. Geography finishes dressing after the basic six-view static
checkpoint. Optional enemies require a separate bounded roster/reward decision and
must not hold the marine checkpoint open.

Restart now requires **Shift+R**; the Esc button remains. The original expedition
checkout has the same source fix. The native run was left untouched as promised.


Marine integration checkpoint: the shared core sampler tests (4), ocean tests
(13), gameplay filters (48) and three focused transition regressions pass. Strict
arena library Clippy passes. Combined application checks pass; fresh final renders
and user motion/feel checks are still pending. These are focused checks, not the
selector's complete combined gate.

The package02 six-view review passed overview, settlement and summit. It rejected
close coarse/fine terrain edges and partial tree fragments. Exact package terrain
matches the authored surface throughout 110,080 surveyed bay columns; local
coarse interpolation differs by less than about three units near the shore. A
boundary cut was admitting six crown columns without the rest of their tree.
Atomic object publication and a true hex-edge transition are under validation.
The underwater bright strip was independently traced to downward rays reaching
an unfogged sky beyond the finite seabed; camera-water sky attenuation repairs
that presentation path without adding walls or modifying water occupancy.

The optional enemy pass is deferred from this marine checkpoint. A proposed
follow-up is three distant parties (3 baby Goblins, 3 Wisps, 3 Goblins), validated
world-owned deployment regions and small exact-collision residency interests. It
requires an explicit XP-only policy: current progression presence selects Forest
presentation and can engage its clear rewards. No Northern enemies or Forest
rewards are enabled by the marine change.

Reference review: [Cube World's official travel description](https://www.cubeworld.com/)
treats the boat as portable exploration equipment. Its small voxel hull is also
visible in [archived boating imagery](https://www.timetoloot.com/game/other-games/cube-world-resurfaces-for-full-release/).
This candidate uses a compact stepped timber hull and portable deployment, with
the user's requested sail, directional wind and conserved momentum as separate
mechanics. It copies no game assets.

Fresh combined static pack at 7e4919e (package02) mechanically completed all seven
views. The underwater sky leak is repaired, complete trees replace partial crowns,
and the boat/HUD is visible. Bay angular surface tones and an isolated offshore
zigzag remain under typed diagnosis; the basic visual checkpoint is not yet
approved for full dressing. The boat receipt's phase was hardcoded at zero even
though its renderer uses the shared simulation phase; status now reports the
accepted material uniform. No surface alignment claim comes from that old receipt.

Selector concern: all 116 Python tests pass after registering four exact ignored
package regressions in the canonical map partition. An actual session drowning
regression first failed on oxygen resetting during death, then passed after
cleanup retained the terminal oxygen state; Restart alone replenishes it. Strict
arena library Clippy also passes. Complete selected Cargo gate still pending.


## Full marine validation checkpoint — September 13

Full dressing is enabled. Strict package `9ec5adc635004d97` contains 11 islands,
three clusters, 255 trees, four buildings and one field. All 5,852 chunks validate;
the supported bay spawn and submerged relief remain unchanged. Both candidate
Cargo launchers select the full package when opening Northern.

The seven-view `a9e1df7/full-marine-01` pack passed root and independent static
review. The former offshore reflection zigzag was caused by interpolating
unrelated shoreline anchor positions; weighted distances to the real anchors
remove false offshore reflection. Depth absorption and underwater sky attenuation
remove distant seabed transmission and the bright underwater strip. Exact
published front-facing mesh intersections confirm the remaining bay division is
a physical shelf tangent, with roughly 12 versus 438 units of submerged sight-line
length. Distant crest accents remain visually repetitive; native motion is pending.

The full-package three-circuit authority benchmark at `f80159c` passed. Active-pump
p95 was 0.835 / 0.825 / 0.847 ms; peaks were 182 resident sources, two workers and
185 queued products. Parked sources stayed at 180. Parked RSS was 93,984 / 94,736 /
94,832 KiB. Damage survived retirement/reload, Restart restored it, and Duel/Fort
switches passed. This excludes renderer and player-controller cost.

A final integrated review caught stale ground-step events replaying camera
smoothing during boat/swim/loading ticks. `b6ee4bc` clears them, and the actual-step
transition regression passes. Full-world/render content was unchanged by this fix.
The selector tests pass (116). The canonical combined Clippy run found 532 map
diagnostics: 15 in the new streamed/ocean code and 517 in unchanged V3/preview
files. The new diagnostics are repaired; their rerun and the paired windowless
wave-cost comparison are the remaining automated checkpoint work. The full CI
suite is not claimed passed. Native travel/boat/glider feel remains a user check.


Final automated checkpoint at `32ef89b`: all 15 ocean tests pass after checked
publication/shelter access. Canonical Clippy now reports 517 unchanged map
V3/preview diagnostics plus two unfulfilled lint expectations in unchanged
`hex_units` tests; `git diff` confirms those files match the wave base. No complete
CI pass is claimed, and the remaining broad suites are deferred after that gate
failure under the lean workflow.

Three paired wave-on/flat bay runs each sampled 200 settled Update intervals after
44 warmup frames. Median-run p95 was 23.9755 ms with waves and 23.9707 ms flat
(+0.02%); p50 was 23.7786 / 23.7654 ms (+0.056%). Both modes retain identical
bathymetry, depth prepass and optical absorption. These frozen-phase windowless
Update start-to-start measurements include scheduler/render-submission waits,
not native GPU execution or vsync. No compiler or other owned game run overlapped
the timing samples. All six originals were inspected; the final-source bay still
meets the reviewed composition and water-boundary criteria.

Evidence is retained under the local `outputs/` directory: full-dressing package
audit and geography tests; `actual-circuit-full-02.ron`; marine step and ocean test
logs; `northern-selected-clippy-full-marine-02.log`; and
`northern-wave-comparison-01.json`. Original seven-view images and independent
reviews remain in `.context/northern-review/a9e1df7…/`; paired final-source images
are under `32ef89b…`. Following documentation-only commits do not change runtime
or package content. Optional island encounters remain deferred; no native game
was opened automatically while the user was offline.


## Player feedback: navigation and stronger waves

Bounded follow-up on the same candidate, with two independently reviewable parts.
Root owns the shared UI: repair M on published exploration overviews, add V to
show a north-up wind arrow/speed from the existing core environment/time facts,
and investigate Forest selection from the native start screen. The world lane
owns only the ocean profile, crest presentation and directly affected tests/docs:
raise the three amplitudes by 75% while retaining slow periods and common CPU/GPU
sampling. No new gameplay authority or whole-map package compilation is needed.
Keep the current native run intact; use focused input tests and windowless frames,
then the user's native playtest. Account floor remains 20% remaining.

UI intake: user reports M does nothing in Northern and Forest selection does not
respond. Linear read/search requires reauthentication, so no ticket was written.
The local defect record and implementation can proceed within the user's request.

Forest selection reached its asynchronous loader, but the new empty `seas` field
changed canonical source serialization and rejected the reviewed package identity.
Omitting empty sea lists restores the original Forest fingerprint `8f56a9974aaac54b`;
nonempty Northern seas remain serialized and identity-bearing. Two focused schema
tests pass, and the default Forest package was successfully reproduced and
republished. All 444 original terrain chunks match apart from identity metadata.
The launcher accepts both packages regardless of the initially selected map, and
loading/failure status now appears above the start menu's scrolling content.

M now depends on a published overview and human control rather than Forest-only
progression. V independently toggles the north-up downwind arrow and current
speed, using the same environment and simulation time as sailing and gliding.
Two focused keyboard/layout regressions pass, including 1280×720 at 200% UI scale,
1600×900 and 1920×1080, pause/reset and unavailable-map behavior. The first compile
exposed an obsolete TextLayout constructor; the corrected rerun passes. Seven
launcher/helper tests pass. Native input/feel remains the user's check.

Swells now have amplitudes 2.1 / 1.05 / 0.35 units, a ±3.5-unit envelope, retaining
the existing 18 / 25 / 12-second periods and shared CPU/GPU sampling. Focused ocean
tests and fresh navigation/water captures follow on this combined candidate.

Follow-up validation at `95d608d`: all 16 ocean tests pass. Fresh bay, waterline and
boat/navigation originals pass focused static review in
`.context/northern-review/95d608d…/northern-six-v1-focused-1d4b336f-navigation-taller-waves-01/`.
The wind panel is readable beside the map, and close water still meets the bank
without a visible gap. Existing shallow/deep tonal contrast remains a visual note.
Native movement, input and wave feel remain the user's check; these captures ran
alongside their open game and are not performance evidence. No broad CI rerun or
complete CI pass is claimed beyond the focused checks listed above.
