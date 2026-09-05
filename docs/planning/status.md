# Status

**This is the doc that is allowed to be out of date, and the only one.** Everything
else under `docs/` describes a contract; this describes a moment. If it disagrees
with the code, the code is right and this needs an edit.

What is *planned* is [roadmap.md](roadmap.md). What the game is *for* is
[the design](../design/game.md). This is the gap between them.

## In delivery

V4 foundation is implemented on the unmerged `wave/v4-foundation` review branch in
[draft PR #220](https://github.com/chillgamerboys/bevy-hex-game/pull/220), stacked on
Grand reference PR #219 at `bc06a89`. It is **not delivered on `dev`**. Runtime-loaded
regional authoring, one/two/seven full-sized fixtures, availability-aware residency,
local edits, fresh partition saves, resident stock art and private exploration have
concrete consumers in the opt-in `hex_v4` explorer. Exact checks, capture provenance,
remaining acceptance and scope limits are tracked in the
[V4 wave](waves/v4-foundation/manifest.md) and
[platform contract](../systems/world-platform-v4.md). The strict inherited V3 gate,
human motion review and active-authoring-hour target remain open. No legacy save
migration, complete online game, encounter merging or infinite generator is claimed.

Catch-up enablers are now delivered to `dev`: [PR #214](https://github.com/chillgamerboys/bevy-hex-game/pull/214)
established the biome delivery ledger, [PR #216](https://github.com/chillgamerboys/bevy-hex-game/pull/216)
restored the locked dependency graph and 45-minute macOS shipping budget,
[PR #215](https://github.com/chillgamerboys/bevy-hex-game/pull/215) replaced numeric
partition pins with identity-based fail-closed selection, and
[PR #217](https://github.com/chillgamerboys/bevy-hex-game/pull/217) published the isolated
case-backed development skills. This reconciliation is based on post-#217 `dev` at
`4c97b75151b1a6f4e1ea1972976e1d9512ed8c45`. The #217 source-head matrix and exact-head
no-runtime-change `N/A` passed; post-merge run `33708389913` was still running at this
cutoff. These tooling deliveries do not deliver biome content.

Four map candidates remain one draft dependency stack, and no published head contains
current `dev`. At their published heads, #210 is green except for a cancelled macOS build
and now conflicts with `dev`; #211 has a failed Map partitions job and cancelled macOS
build; #212's historical hosted matrix is green; and #213 has a failed Map partitions job.
#215 repairs the count-based selector on `dev`, but none of those old check rollups proves a
refreshed head. Their successful manual-sign-off jobs are draft/source-lane deferrals: all
four still require named-human exact-head presentation, motion/control-feel, and play
findings. They must land #210 → #211 → #212 → #213 under the
[biome stack reconciliation](waves/biome-stack-reconciliation/manifest.md).

At the earlier `4c97b75` delivery snapshot, the sixteen Grand V3 checkpoints were
packageable only after #213, and Garden still needed its integration repairs and gates.
Generic review/provenance, capture-sequence tooling, Grand structural review, route
revision 3, time-cycle and subtle-geometry work were then unpublished. This is historical
context: the selected V3 reference `bc06a89` already contains `tools/review.py`,
`tools/test_review.py` and `tools/run_grand_v3_structural_review.sh`. Those earlier dirty
snapshots remain unsuitable for whole-tree commits; their old publication labels do not
describe the later #219 reference.
Rejected Outpost checkpoint `f4f0e4c` is preserved only as
`archive/outpost-rejected-f4f0e4c`; the replacement design has not started.

The **Grand V3 schematic planner** has completed its revision-2 correction and received
cell-for-cell visual approval on `wave/grand-v3-schematic`, stacked temporarily on the
exact corrective biome head while draft PRs #210–#213 await delivery to `dev`. The
implementation adds a standalone world-owned
library and CLI that turn one seed plus a strict radius-eight template into a complete
217-cell semantic plan, validation metrics, semantic fingerprint, and review-only
SVG/HTML projections. Alberto approved the corrected cell-for-cell source transcription:
revision 2 uses the fixed flat-top orientation, places twelve peaks in two six-cell chains
around the north-eastern mountain lake, and restores the exact lake island, frozen core and
shore contact, waterfall opening, straight `q = 1` tunnel, and land-overlay river route.

The previous revision-1 package and gallery are stale and must not be used as evidence.
Revision 2 now passes the complete package suite, the 256-seed normal corpus, and the
10,000-seed release corpus with zero invalid outputs, fallbacks, or duplicate semantic
plans. Deterministic four-worker candidate evaluation preserves byte-identical serial
results and brings sampled release p95 to 36.6–37.7 ms below the 50 ms budget. A fresh
revision-2 reference plus twelve-seed gallery has been generated and machine-validated.
The exact source transcription has Alberto's named visual approval; the seeded gallery's
variation quality remains a separate pending human review. Local peak resident memory measured 8.7 MiB, while
the scheduled Linux `VmHWM` run remains the authoritative 64 MiB platform gate.

The **Grand V3 schematic-to-map compiler** has passed its required representative proxy
decision and is now in final implementation on the same branch. Alberto approved the exact
pitch-22/radius-187 scale, the recorded budgets, combined solid-terrain chunk meshes with
exact picking, and retention of the 50 ms perception plus 100 ms local-edit targets. The
approved proxy remains the before-state: 105,469 columns, 217 stable biome identities, 444
resident chunks, a 10,861,429-byte snapshot, 1.794 s setup, and 1.137 s combined semantic
compile plus voxel materialization. Its 56.707 ms perception and 192.377 ms local-edit p95
measurements remain misses to close, not relaxed budgets.

Final implementation now includes bounded terrain batches with exact picking, authoritative
three-lane hydrology, exactly two bridges, ordinary hubs/routes, the tunnel and embedded
Crystal Ascent, one shared interior/light domain, vegetation, and the complete anchor set.
The 256-seed fine-topology corpus passes and exercises all three natural-pass width buckets;
reference, zero, hero, and maximum-seed complete worlds also pass the final physical pass,
route-cut, seam, vegetation, Crystal, anchor, and export validators.
Exact release runtime gates also pass for one-chunk edit locality, complete Crystal
occupancy/perception/fog/cutaway teardown and re-entry, and 10,000 unchanged updates with
zero projection or presentation rebuild churn. The final-content comparison now meets all
approved deterministic latency and snapshot budgets: 3.210 s setup, 2.099 s semantic
compile plus materialization, 25.535 ms perception p95, and 77.638 ms one-column edit p95.
Full-renderer measurements remain over the approved memory targets at 3.84 GB
maximum RSS and 7.63 GB peak footprint. A same-process phase trace now isolates the current
actionable miss to terrain publication/extraction: RSS peaks near 2.94 GB during upload,
then settles around 0.50--0.72 GB and reaches about 0.78 GB during PNG capture. Renderer
publication-memory optimization therefore remains open. The 32-world fully materialized
release corpus passes in 64.65 s, including a permanent seed-14 review-anchor reachability
regression. The real preview, camera-route capture, CI-equivalent gate, native
motion review, and named visual/play approval remain outstanding. The fail-closed delivery
state is tracked in
[the Grand V3 map manifest](waves/grand-v3-map/manifest.md); the original quantitative
baseline remains in [the proxy checkpoint](waves/grand-v3-map/proxy-checkpoint.md), and
the final measurements are recorded in
[the final runtime checkpoint](waves/grand-v3-map/final-runtime-checkpoint.md).

The **Coastal islands** wave is review-ready on its delivery branch and is not yet a
claim about `dev`. Exact implementation head
`6f78a9cae681d9675e67daf6bd98b20e4a058475` contains focused **Sandy Islets** and
**Wooded Island** maps plus the radius-77 **Ocean Archipelagoes** Macro world. The focused
profiles reuse the Coastal environment at sea level 8: five separated sandy
components in a radius-24 world, and one broad radius-40 island with a two-column
beach and broadleaf interior. The Macro roster assigns 24 of 37 atomic cells to one
continuous ocean, six cells to three scenic sandy clusters, one to a playable sandy
landing, and six to the wooded heart. Its landing-heart seam is the only ordinary dry
cross-instance route; six remote dry components remain scenery.

Validation close-out head `45b0704c409474abf86c273b70fef3cac981751d`
refreshes the exhaustive map-partition pins to the delivered 119/506/97 split. Typed
settings, metrics, generation, Macro composition, three selectable maps, compact
party placement, dependency invalidation, lifecycle checks, semantic review routes, and
real Sandbox previews are complete. The exact-head selector-chosen CI-equivalent gate is
green, including 119 map-unit, 506 map-generation, 97 map-contract, and 1,050 residual
tests; doctests, strict clippy, formatting, dependency policy, generated documentation,
tracked documentation links, and the optimized shipping build also pass. Three deterministic
scripted walks produced 17 captures. The shipped large-map camera benchmark records Ocean
Archipelagoes at 2.583 µs query p95 against its 2 ms budget. Named-human visual/play
approval and publication to `dev` remain pending; neither the automated gate nor the
captures substitute for that review.

The 2026-08-21 corrective pass increased Wooded Island's central relief allowance by two
voxels in both the focused recipe and Ocean Archipelagoes' wooded heart. Focused and Macro
tests retain shoreline, protected-route, dry-connectivity, and height-bound contracts while
replacement frames show the raised center; named-human approval remains pending.

The **Crystal Mountain** wave is implemented on its delivery branch and is not yet a
claim about `dev`. Its selectable radius-77 Macro candidate places the existing
base-6/rise-144 Crystal Ascent at world origin, surrounds its level-150 summit with a
five-cell temperate Forest basin and higher inner/outer ridges, and makes one
four-wide level-6 tunnel the only ordinary route from the mountain foot to that basin.
The landmark mask contains its complete radius-32 site while retaining the protruding
central-cell fringe as summit terrain. The tunnel crosses the outer and inner mountain
instances before joining the landmark, then shares one Dark interior and light domain
with the complete Ascent; of the tunnel route floors, only its eight-cell boundary
apron remains exterior.

The branch now contains the defaulted surface-walker, spanning-feature, and anchor
contracts; authored summit-port alignment; reserve/merge/carve/finalize composition;
exact subsurface seam closures; unified interior, tunnel crystals, anchors and
validators; selectable non-combat content; and review-cutaway feature reconciliation.
Exact delivery head `74deb7f` is published as
[draft PR #210](https://github.com/chillgamerboys/bevy-hex-game/pull/210). The complete
selector-chosen CI-equivalent gate passes at that head, including release rotations
and corpora, runtime lifecycle, materialization and re-entry, fog and heart occupancy,
camera collision, and perception. Its deterministic review pack contains 28 captures
under `.context/visual-walks/crystal-mountain/74deb7f`.

Release profiling records Crystal Mountain generation at 7.10 s p95 versus 3.04 s
for Ring19, or 2.33× inside the 2.5× budget. Materialization takes 7.187 s for 61,450
entities, compared with Mountain Range's 3.854 s for 68,650. Camera collision is
2.917 µs p95 and terrain-index rebuilding is 4.935 ms. Perception is 33.768 ms p95,
the dense six-observer case is 101.655 ms p95, and 10,000 idle frames perform zero
recomputation. Crystal Mountain's measured RSS/peak memory is
940,670,976/1,481,492,232 bytes versus Mountain Range's
764,608,512/1,249,576,544, an increase of 23.0%/18.6%.

Human visual, camera-motion, control-feel, and named play approval remain pending.
The PR deliberately remains draft and the candidate has not merged to `dev`; neither
the generated frames nor the automated gate substitutes for that human review.

The 2026-08-21 corrective pass found and closed two Crystal defects at their world and mesh
sources. The complete Crystal Ascent stair annulus now has a stratified foundation through
base level with a worked-stone cap, and final Crystal Mountain composition revalidates that exact plinth after merge
and tunnel carving. Crystal mesh chunks now give opaque geometry sole ownership of a shared
opaque-to-translucent/additive face, removing the coincident surfaces that caused movement
flicker while retaining a closed backing. Typed geometry and mesh tests are green; native
camera-motion review remains intentionally pending.

The stacked **Arid biomes** wave is implemented on its delivery branch and is not yet
a claim about `dev`. It adds the `Arid` environment, Desert Transition, Desert Plain,
Dunes, and Oasis recipes, the blocking `plant/date-palm` object, and four selectable
maps: **Desert Transition**, **Desert Plain**, **Dunes**, and **Desert Oasis Rings**.
The first three are focused radius-12 `Single` worlds. The last is a radius-55
`DesertOasis` Ring19 profile with one local-water oasis, six inner dune regions, and
an alternating outer ring of six taller dune and six plain regions. It retains 42
Dry reciprocal seams, redundant ordinary reachability, and the original defaulted
`TwoRings` fingerprint.

The complete selector-chosen CI-equivalent candidate is green: 60 selector checks,
180 rules checks, 93 trajectory contracts, 420 cross-crate contracts plus five spell
resolution checks, 29 simulation tests, 180 application and postflight tests, 701 map
unit/generation/contract tests, and 1,047 residual workspace tests all pass. Workspace
doctests, formatting, dependency policy, panic-free clippy, generated documentation,
and the optimized shipping build also pass. Four pointer-driven camera walks produced
22 deterministic frames and four Sandbox previews. Human visual/play approval and
publication to `dev` remain pending; the automated pack does not substitute for either.

The 2026-08-21 corrective pass lowered Oasis water exactly one voxel below its surrounding
base-level grass, with the sand bed one further voxel down. Exact generation tests and
replacement review frames verify the recessed-basin contract; named-human approval remains
pending.

Corrective visual evidence now fails closed across these review packs. Each run invalidates
any stale review index before setup, records scenario/seed/script provenance and successful
capture names, and may report completion only with the exact expected frame set. Every frame
starts `UNREVIEWED` and must receive an explicit `PASS` or `FAIL`; static evidence is never
used to claim native-motion flicker or control-feel approval.

## What is built

A playable skeleton. Workspace boundaries enforced by Cargo, CI on three platforms,
a strict lint wall, dependency auditing, a state machine, and a RON content pipeline
that refuses to start on a bad file rather than defaulting past it.

The world is a voxel map with substances, destruction, and a deterministic
procedural generator: seeded recipes with validated crossings, anchors that encounters
place units on by name, architecture probes for frozen and volcanic Hills, and
dedicated Sky Islands, Mountains, and Caves biomes. Sky Islands preserves a complete
playable Hills map below a higher, multi-band flight-gated upper network. Mountains
covers most of the map with sharp frozen massifs, deliberate cliffs, a
high-pass/low-bypass route pair, snow caps, and a peak-fed river and fall. Caves
places a varied rocky surface above a two-wide entrance and a dense,
height-validated underground chamber network with exact opaque cutaway roofs.

The selectable **Crystal Ascent** landmark occupies one radius-40 world. Its
monumental lower aperture opens into a crystal chamber and central shaft; three exact
four-wide clockwise stair circuits, joined by four independent lanes at each
contraction, climb 144 levels through eighteen crystal-lit landings before emerging
around a radius-11 oculus into a protected woodland
clearing. A 30-level irregular-prism heart dominates the chamber, and each circuit's
four stair lanes sit on an exact two-/four-/six-/eight-voxel radial stone haunch. The
recipe accepts rises from 100 through 200, keeps its architecture
seed-independent, publishes stable lower/chamber/upper anchors, an upper
corner-landing review anchor, and terminal pads, and
validates exact handoff edges, ordinary traversal, one-level transitions, turning-pad
headroom, per-crystal light pairs, and the absence of non-handoff cross-loop shortcuts.
The standalone party starts on the exterior apron facing inward. Arbitrary Macro
placement remains rejected; the in-delivery Crystal Mountain candidate is the one
specialized composition that constructs the landmark at world origin, aligns its
authored summit approach, and joins its lower aperture to the global tunnel.

V3 now has twenty-one recipe variants: Hills, Sky Islands, Mountains, Caves, Waterfall,
Forest, Fort, Volcano, Deep Forest, Prairie, Shallow Sea, Beach, Shore, Deep Mountain,
Crystal Ascent, Desert Transition, Desert Plain, Dunes, Oasis, Sandy Islets, and
Wooded Island. Ring7 places its fixed seven-recipe roster in one connected radius-33
world. Ring19 powers two
selectable radius-55, 9,241-column profiles. **Two Rings** retains its 19 fixed mixed
regions, 42 reciprocal seams, 30 outer boundary sides, and physical ordinary-walker
graph that keeps all regions reachable after any one seam is removed. Its three
mountain-fed water branches meet in central Hills before flowing through downstream
Hills and an outlet Waterfall; the western Volcano owns a separate lava outlet.
**Desert Oasis Rings** reuses the masks and redundant dry walker graph around one
local Still pool. Single and Ring7 retain their 4-bit patch namespace, while Ring19
uses 5 patch bits so slots 16–18 remain collision-free.

The new authored Macro path powers the selectable **Mountain Range** map. It covers a
radius-77, 18,019-column world with 37 atomic radius-12-scale cells collapsed into 30
logical biome regions; the four-cell Shallow Sea and five-cell Deep Mountain are each
generated once over a union mask and publish one region id. Macro uses a six-bit
instance namespace without changing legacy fingerprints. Its allowed-by-default
adjacency check rejects direct Frozen–Volcanic contact, non-coastal Shallow Sea
neighbors, Deep Mountain neighbors other than Mountains, and Beach or Shore without
both sea and inland context. These rules gate Macro only.

Mountain Range progresses through Shallow Sea, Beach/Shore, alternating Forest and
Prairie, Hills and two Waterfalls, two graded Alpine mountain tiers, and one broad
Deep Mountain massif. Sand backs the exact sea bed and coastal surfaces, Prairie
instances place nonblocking authored grass, and the two Waterfall instances retain
rapid, fall, and current stages. Standing seams join the submerged coast to the
shared water body's still sea footprint, while the two directed tributaries descend
and merge without an uphill edge or cycle. The required ordinary route runs from
central Shore through Prairie, central Hills, both mountain tiers, and the landward
Deep Mountain base; summit and through-massif access are deliberately optional. The
party and hostile anchors live on central Shore and central Hills, with additional
coast-to-massif review anchors.

Waterfall authors deterministic directed liquid topology from calm inlet through
rapids, a contiguous thirteen-level fall, plunge basin, outlet, redundant land
routes, and sparse vegetation; an opaque animated renderer consumes the same exact
flow facts. Forest plans rolling terrain and clearings, places its denser woodland,
then bends a mostly two-wide road around exact authored tree footprints with short
one-wide constraints and a three-cell prairie taper. Deep Forest extends blocking
woodland across a complete patch around a winding trail and three clearings; Prairie
ships the complementary tree-free grassland. Volcano replaces the old volcanic-Hills
geometry behind the stable scenario name with a crater massif, descending lava, and
an elevated stair-served bridge.

Small broadleaf, tall narrow, and seven-root old-growth trees vary the canopy and
height profile; most prairie surfaces carry nonblocking authored grass tufts. Object
ids, exact rotations, and rotated blocker footprints are fingerprinted before
routing. Map validation, movement previews, click routing, command validation,
spawning, review relocation, enemy pathfinding, terrain-edit protection, and the
object renderer consume the same world-owned projection. Native V3 Caves plans the
rocky exterior and stacked underground network together: six through twelve chambers
on three `+0/+2/+4` floor tiers, one-level two-wide connectors, exact cutaway roofs,
sparse authored moss and lichen, and deterministic Bright gameplay lights that cover
the required network while leaving optional branches dark.

Two Rings is mechanically selectable and covered by deterministic generation,
spawning, regeneration, and re-entry checks. Alberto approved its visual, motion,
and play feel at the exact reviewed head before the development wave landed. Mountain
Range is mechanically selectable and its generator validates the authored geometry,
coast, watershed, elevations, massif, anchors, and critical route. The complete
selector-chosen functional closure passes, including map generation, real-plugin
publication, regeneration, and re-entry. Its four-view deterministic capture pack and
45-step, eight-frame automated walk cover the overview, rear silhouette, coast,
watershed, foothills, both massif azimuths, and the Deep Mountain base.

The Mountain Range walk removes Hostile rosters once, before actor setup and only
behind the default-off `visual-walk` feature, so combat cannot interrupt terrain and
camera evidence. Normal launches retain the authored encounter; typed map and scenario
contracts, not the rendered frames, remain authoritative for connectivity, spawning,
and gameplay. On 2026-08-03, `@shrav-k` approved the overview and rear-silhouette
static presentation. The reviewed frames have SHA-256 hashes
`3b36aaff163828b762b23d65fc3c8bcfea00b47e06bc551c44d4c60c558ab8ab` and
`a8e36f44370f4b3ccfd5fbd49d7d6739310f9e33899a649ee2636626a5afddb9`.
That approval does not claim human motion or control-feel evidence.

To unblock unrelated pull requests, `@shrav-k` explicitly waived and cancelled the
remaining release-only 128-seed corpus, 10,000-seed stress corpus, generation
benchmark, large-map camera performance diagnostic, and native human motion/control-
feel replay on 2026-08-03. Those gates are **WAIVED**, not passed; no fallback-rate,
timing, or human-motion result is inferred from the functional tests, screenshots, or
automated walk.

Authoritative spatial perception now runs headlessly every gameplay frame.
`hex_world` publishes a renderer-independent Bright or Dim exterior tier;
`hex_perception` derives exact exterior/interior domains, maximum-tier public local
lights, exact obstruction-aware pooled faction sight, and independent faction memory
over stacked `TilePos` surfaces. The target's illumination chooses a 36/12/1
Bright/Dim/Dark upper-dome radius; every in-range observer-target pair then traces one
head-center to target-top-center ray plus six standing-body-top corners to their
matching target corners through compact `RunBottom` terrain occupancy. A blocked
center requires three clear paired perimeter rays from one observer, never cross-pairs
or cross-observer pooling. The bundle applies globally and never exceeds seven rays.
For character LOS, only the exposed top voxel of a run topped within one level of the
observer's support is low cover, and only when that run continues into material
directly below the top. Deeper run cores, disconnected one-voxel platforms, two-level
walls, and vertically remote roofs or decks remain blockers. The raw strict-interior
segment kernel still tests complete runs symmetrically, but observer-relative
low-cover classification can make the resulting visibility directional. Material
interior crossings block, exact tangencies remain clear, and a physically open cave
mouth permits cross-domain sight. Downed units can remain visible but cannot provide
sight, and changing `Downed`, a unit position, a light, a sight profile, or terrain
occupancy republishes observation in the same frame. Three validated hot-reloadable
sight profiles live in `perception.ron`. V3 cave sources publish fixed local gameplay
lights directly into this headless pipeline.

The tactical shroud keeps current terrain visible and pickable, but places one dark
navy cap over every current surface the player does not observe. Unknown and
Remembered terrain intentionally look alike because live map geometry is public in
this design. Unobserved hostile roots receive only the composable Fog occlusion
reason, suppressing their models, picking, shadows, markers, targeting, inspection,
health bars, and identifying HUD details while combat retains an anonymous initiative
entry. Unknown, Remembered, and Observed knowledge still gates gameplay facts:
remembered snapshots do not leak hidden edits, and unseen units disappear immediately.
The faction-generic traversal projection is rebuilt from that same knowledge.
World observation gates the gameplay-owned hostile lattice view, every cast anchor,
and AI identities, effects, turn order, traversal, and legal commands. AI can traverse
only Observed or Remembered terrain and cannot use Unknown truth. Unknown-frontier
routing, engagement, ordinary-attack targeting, and lost-contact search are not wired
yet. Authored emissive cave crystals and restrained physical point lights now present
every fixed cave gameplay-light source without becoming gameplay authority. The cap
renderer deliberately shades top faces rather than every cliff side or tall prop;
full-scene shading and fades remain presentation refinements.

Authored-object occupancy is live as an opt-in exact-volume contract. The cathedral
heart projects its rotated structural voxel runs before movement and perception,
blocks the standing two-voxel body and strict-interior sight, and rebuilds or
withdraws the authoritative resource in the same update when its source changes.
Terrain low-cover handling never applies to that volume. The eighteen smaller
landing crystals remain nonblocking presentation objects. Their paired Bright/Dim
gameplay lights and the heart's four physical point lights remain independent of
rendered emission.

Fort adds the first complete V3 structure recipe and the canonical worked-stone
substance. A five-level, two-wide curtain surrounds a gravel courtyard and offset
keep, with six small accessible turrets, two lintelled gates, two broad stair
terraces, and alternating outer battlements. Exact graph validation proves that
closing both gates seals the courtyard, either gate independently reconnects it, and
every usable wall or tower surface remains ordinary-walker accessible.

Movement is level-based over stacked surfaces, with body size decided by headroom and
a breadth-first pathfinder that cannot collapse a stack. A movement preview draws the
reachable set and the route before a click commits to either.
Combat has two tempos, a turn order, engagement with hysteresis, and surface-aware
targeting where height buys range. Its tuning values are designer-facing knobs in
`assets/config/combat.ron`. Player and AI intent flows through one **command funnel**:
clicks, the end-turn key, and the AI emit `GameCommand`s into a queue, and a single
applier in `hex_combat` validates each against seat, turn, reach, and budget before
applying it. Passive effects and derived consequences such as downing run at their
own deterministic schedule points. The queue is consumed rather than persisted;
recording its command stream is future replay work.

Who stands on a map is an **encounter**: `assets/config/encounters/*.ron`, a roster of
units per side, each naming an archetype and one placement — an authored coordinate, a
generated anchor, or a formation that spreads a group over the surfaces walkable from one
centre. A scenario names its encounter by path exactly as it names its world and its sky,
so several scenarios share one file, and every rostered unit is either placed or setup
fails naming the entry and the reason. It replaced a two-coordinate scaffold that could
express one player and one enemy and nothing else. **The archetype is looked up in
`lattices.ron`**, so a roster line is most of what a unit is. The shipped encounters are
no longer limited to one unit a side. Party Trial fields matching three-member
hedge-mage, raider, and wolf parties. Ability Lab and Raider Mirror retain focused
ability and identity checks behind default-off deterministic test support.

The element wheel and spells load as **validated content**: `elements.ron` (the
six-basic wheel, opposition, and fusion recipes, checked acyclic and feedable) and
`spells.ron` (requirements as an element multiset with tier ≤ 6, casting and mana axes,
targeting, and a closed effect enum). A `ContentIndex` resolves every element and
substance name a spell references; a dangling reference is logged and the last valid
content kept. Canonical source fingerprints prevent that retained index or lattice
library from being paired with newer raw catalogs: Loading requires one
`AcceptedContentRevision` spanning elements, substances, the terrain-damage matrix,
spells, and lattices. A test opens everything shipped so a broken reference cannot
ship.
`ElementId` and `SpellId` are opaque `hex_core` ids assigned from sorted names.
`SubstanceId` instead preserves the frozen original vocabulary and an additive reserved
compatibility tail, so independently landing terrain catalogs cannot renumber existing
materials. A dev-feature content dump remains available for inspecting the resolved
spell list, while gameplay now consumes the same catalogs through its lattices and cast
panel. Every externally authored archetype must also form one contiguous lattice;
disconnected islands fail with the archetype named in the error.

> **Elemental-grid foundation E0 (PR #184; HEX-37/HEX-56/HEX-57) is delivered.**
> Packaged content uses the canonical Air, Fire, Metal, Earth, Life, Water wheel; six
> direct pairs; six direct triples; and a neutral 18-element × 10-substance
> terrain-damage table. The current single-target Scrying Eye content belongs to
> Divination, Daylight is removed, and the presentation-only radius-two Creator chart
> uses editable vector masters plus checked-in runtime glyph assets. Light is removed
> and Life is newly authored; no Light-to-Life save or Creator-draft rewrite exists.
> Old-revision Campaign records remain preserved and incompatible, while Creator
> drafts remain preserved with unresolved-name diagnostics. E0 ships none of the
> later basic/pair/triple school mechanics.

**Damage exists.** The lattice engine (`hex_lattice`) is joined to the game at last:
`lattices.ron` authors the three archetypes the design names — a wolf of four hexes and
a bite, a raider of eight around a metal shield, a hedge-mage of thirteen with fused
elements and Scrying Eye — and units spawn carrying them, keyed by the
archetype their encounter rostered. A cast goes through the command funnel and the
legality ladder, and drains the lattice that paid for it. Damage names a count; **the defender
chooses which hexes go down**, answering through a `ChooseDisables` command so the choice
is replayable rather than made inside the applier. A unit whose every hex is disabled
leaves the turn order and is **downed** — retained with its lattice rather than
despawned. Heal restores one chosen cell on the caster or a mutually touch-adjacent
Observed unit; a hostile also requires complete current lattice knowledge. Renewal
remains the stronger ranged two-cell restoration. Either removes `Downed` after a
successful restore and returns the unit at the next round boundary; exploration Rest
recovers the party immediately. A strike deals damage the same way, through the same
decision.

**Channel is live.** An active, non-downed combatant can spend its one action to
restore each element by that unit's Channelling value, capped by Attunement capacity.
The lattice engine skips disabled and enchantment-locked cells in deterministic
element/coordinate order and reports only mana actually restored. Human input and
baseline AI use the same command/refusal/event seam; the summary attributes Channel
actions and restored mana under stable element names.

**And casting has an interface.** A spell panel lists what the acting unit inscribes,
each row carrying its live blocked reason from `castable` and, above the list, whichever
of the applier's own refusals is standing in the way — not this unit's turn, action
already spent, a decision still open. Choosing a spell starts aiming: every legal anchor
takes a clickable marker, `hex_units::volumes` resolves the shape, and the surfaces
inside that volume are painted in the spell's element colour. The anchor moves by
clicking a lit surface or by cycling the units in range; `ENTER` casts and `Q` puts the
spell down. Only *surfaces* are painted — gameplay cannot know how tall a level is in
world units — so the panel reports the whole voxel count beside the number it could
show. Preview, target cycling, AI enumeration, and the authoritative applier all
require the exact anchor to be Observed. An authorized area may still spill into
hidden space without revealing the result. The `1`-casts-something placeholder that
made the damage loop playable before any of this existed is gone.

> **Spell-resolution delivery:** PR #180 makes radial volume clipping, stable
> friendly-fire area Disable/Burn, explicit Fire/power-2 Fireball terrain Impact, one
> paid monotonic batch transaction, the combat-authority hold, deterministic
> unsupported-actor settlement, and typed fatal freeze live. HEX-19 and HEX-24 remain
> partial / In Progress for the residual work named in the roadmap; landing this
> coherent gameplay slice does not complete either epic.

Bodies are one hex wide; there is no footprint for anything larger. Exact `TilePos`
occupancy now makes those bodies real: movement preview, path construction, command
validation, party routes, baseline AI, encounter placement, and Sandbox deployment
all prevent occupied endpoints and pass-through routes without collapsing stacked
elevations. In-flight paths reserve their surfaces, command refusals distinguish route
from endpoint conflicts, and downed bodies retain their surface for revival.

**Complete-party combat is live.** The compact Party component presents up to six
stable members. A first card or number-key activation inspects and centers without
changing command authority; repeating it opens Character Main View, while combat
keeps gameplay selection on the acting ally. Exploration can switch between Solo
movement and atomic Group movement through Formation Main View;
authored formations rotate by route segment, compress through the Crossing bottleneck,
and reform when space returns. Algorithm-neutral AI consumes canonical legal actions
through the same command funnel as the player. Exact-cell damage and restoration use
a compact fingerprinted eligible set instead of allocating every cell combination;
the host validates count, uniqueness, eligibility, and fingerprint before building
the same replayable command. Movement scoring shares one authorized graph, one actor
reach/predecessor projection, and one reverse distance map per live observed hostile.
Victory and Defeat retain the
battlefield, Retry rebuilds the same resolved seed, Heal and Renewal revive at the next
round boundary, and exploration Rest recovers the whole party. The minimalist tactical
HUD keeps Party, Initiative, Activity, and Action Bar independently configurable, hosts
Character/Formation/Required Decision in one typed Main View, and keeps actor,
selected ally, decision owner, aimed target, and retained target as explicit roles.
Required decisions remain forced while ordinary components are hidden. Party and
disclosed Initiative inspection can center Map camera or feed Character follow without
mutating selection, turn, caster, command ownership, or formation.
Party Trial is the 3v3 integration and human regression case; Ability Lab and Raider
Mirror remain its focused automated companions behind default-off stable fixture IDs.

The **Campaign/Sandbox/Multiplayer application shell is live**. The Main Menu exposes
exactly Campaign, Sandbox, Multiplayer, Tools, and Settings. Campaign projects exactly
three indexed local records as Empty, Available, or Invalid. A new canonical Party
Trial is bound to the chosen empty slot and occupies it only on its first safe manual
save. Available cards show their party and accumulated active-play time; invalid
records remain preserved and visibly refused. `campaigns.ron` is replaced atomically.
When it is absent, one structurally valid legacy `resume.ron` is copied to slot 1
without modifying the legacy file, then checked against the current semantic content.
Mountain Range changes those digest-bound shipped world inputs, so the narrow PR #175
legacy translation is intentionally no longer compatible: the imported record remains
preserved as Invalid with a visible scenario-changed refusal. Only active, unpaused,
non-terminal Campaign gameplay accrues time. Manual saving instead requires paused,
safe, quiescent Campaign exploration.

Sandbox is the sole player-facing authority for a temporary map, two ordered fixed
six-slot rosters, character picks, deployment, and launch. Its in-memory default is
Flat Arena with one Hedge Mage and one Raider. A selected map is pending until Use
Map; generated maps may regenerate only that pending resolved seed, and Back discards
it. Sparse slots and duplicates are valid, while launch flattens occupied slots in
stable order. The draft survives child routes, Main Menu and Creator excursions, and
gameplay return. Guided deployment places occupied Party slots and then Enemy slots
one at a time on any canonical legal, unoccupied exact surface. The ordinary gameplay
HUD is fully suppressed during that phase, leaving a compact task card; the final
placement enters Review with Undo, Return to Sandbox, and Start Combat. Catalog side
regions remain only as hidden actor-staging compatibility metadata. Start freezes
shipped combat rules plus exact map/seed, ordered rosters, content revision, and
deployment for Loading and Retry Exact. Terminal Sandbox play shows only
Victory/Defeat, Retry Exact, and Return to Sandbox.

Direct client-hosted Sandbox multiplayer is live for one listen host and up to five
guests across the six human seats. Host Direct freezes one shipped Sandbox setup,
opens an encrypted WebTransport endpoint, and shares a redacted `HEX1` connection
code; Join Direct verifies the exact protocol, build, shipped content, certificate
SPKI pin, and generated-world fingerprint before activation. The host assigns the six
party members, every connected guest must own at least one, assignment changes clear
readiness, and no new player is admitted after launch. Offline single-player uses the
same seatless command ingress without opening a socket.

Same-network Sandbox testing also has an explicit zero-configuration path. **Host LAN
Sandbox** uses the existing Sandbox and deployment flow, then advertises the open
assignment lobby with mDNS/DNS-SD. **Find LAN Games** continuously lists compatible
lobbies on the same multicast link and joins the chosen host through the same pinned
Direct transport; no IP address or copied code is needed. Advertisement stops at launch
and resumes only if the host returns to the lobby. Because the current ephemeral invite
is necessarily visible in unauthenticated LAN metadata, this mode is for trusted local
networks; exact admission and authority checks remain unchanged.

The listen host owns simulation, AI, world mutation, pause, admission, and outcome
actions. Clients submit intents and interpolate exact authoritative motion; they do not
run rollback, lockstep, prediction, or private combat authority. A disconnected seat is
reserved for 30 real-time seconds, then temporarily delegated to the host without
changing canonical ownership. An admitted player can restart and rejoin with a rotating
credential bound to the session, endpoint, and SPKI pin; reconnect restores a complete
generator-neutral world and authorized player-knowledge/unit/session baseline before
ordered later deltas. Host loss ends the session, while client Escape opens a local
non-pausing menu.

The same Direct/LAN session can now host a Campaign from one of the host's three local
slots. An empty slot starts a new host-owned Campaign; an occupied compatible slot
transactionally restores its complete generator-neutral world, units, lattices, effects,
formation, rules, seeds, and active time before opening a fresh assignment lobby. Resume
never restores old seats, credentials, cameras, selections, or transport state. Only the
listen host can save, and only during paused, quiescent exploration; clients receive an
ordered, non-blocking save-status projection and never read or write the checkpoint.
Legacy/V1 records retain their strict compatibility behavior and upgrade only after a
successful next save.

Direct multiplayer still requires shipped content. LAN discovery does not cross
routers, VLANs, guest-network isolation, or most VPNs. Internet hosts must arrange UDP
forwarding themselves or use the documented temporary Tailscale test route and may
still fail behind CGNAT; there is no UPnP, STUN/TURN, public matchmaking, cross-store
relay, host migration, spectator mode, or dedicated server. Universal EOS Internet
sessions remain a later milestone.

The default-off online feasibility foundation now fixes transport-neutral EOS identity,
lobby, reconnect, join-code, and streamed-snapshot vocabulary plus a safe mock backend.
Its isolated `hex_eos_ffi` crate is the sole audited unsafe boundary and loads only an
explicit checksum-staged official runtime path. This foundation does **not** make Play
Online functional: no EOS platform, identity, lobby, packet connection, or socket is
created by ordinary builds. Protected official headers/runtime, a configured development
deployment, and live Device ID/Steam/lobby/P2P evidence remain required before the
Universal Online wave dispatches. Steam is planned as identity and native invitation
integration into the same EOS lobby, not as a second gameplay transport.

Tools contains Character Creator, Spell Creator, and a disabled Map Creator marked
Coming Soon. Creator origins and destinations are typed. Creating from a character
picker returns to that exact side/slot and highlights without applying. Open in
Sandbox requires a saved clean Map-ready character, preserves the map and Enemies,
replaces Party with that character in slot 1, and returns to its Creator owner when
the flow is left. The local lattice mechanics test remains Creator-only.

Scenario definitions remain the internal world + lighting + encounter launch
contract for Campaign, Sandbox, saves, Retry, review, and tests. Category metadata is
temporarily inert for legacy-resume compatibility. Stable Ability Lab, Raider Mirror,
and Tempo Matrix definitions, optional rules-profile injection, `CombatSummary`, and
deterministic run snapshots remain behind test support. The default plugin graph has
no standalone browser for internal launch inputs, deterministic-case selector, rule
picker, live experiment statistics, local result history, comparison, tuning/copy,
or result deletion.
`combat-reports.ron` is never read, modified, migrated, or deleted.

### Historical Waves 5–8 organization (superseded)

Waves 5–8 originally presented a title grid, one resume, separate Map Scenarios and
Demos catalogs, and a player-facing Combat Lab with Sandbox, fixtures, alternate rule
profiles, live statistics, and saved reports. Those releases established creation
persistence, exact-surface occupancy, deployment, Channel, frozen launches,
deterministic simulation, `CombatSummary`, and the pure model boundary. The current
Campaign/Sandbox shell above supersedes that player-facing organization; the retained
gameplay authority and deterministic evidence do not imply those historical routes
still ship. The bounded Wave 7 tempo decision remains recorded in the
[decision audit](../development/wave-7-tempo-decision.md).

`hex_combat_core` remains the sole renderer-free, serializable authority for the
commands it reduces, exact positions, turns, lattices, summaries, and transcripts.
Bevy combat resources, movement, animation, and UI are projections or validated
content adapters over that authority rather than parallel mutation paths. The bounded
simulation target proves canonical state, occupancy, turn/action accounting, optional
test profile propagation, fingerprints, spell/effect composition, and typed command,
turn, no-progress, or outcome termination. It consumes exact per-unit scripts or a
deterministic non-random baseline controller. This is a regression workbench, not a
claim that the baseline is optimal or balance is fun.

Pure `hex_gameplay_model` transitions own Main Menu, Campaign, Sandbox, Multiplayer,
and Creator navigation, map/draft edits, slot identity, launch blockers, Retry identity,
re-entry, and edit history without exposing mutable widget state.

Gameplay validation is split by oracle into pure rules, focused ECS contracts,
deterministic simulation, and model/headless-app partitions. One fail-closed concern
map selects exact packages, targets, and features for narrow pull requests. Map
validation uses the same authority for unit, deterministic generation, and real-plugin
publication contracts, with all PR seeds preserved under an optimized test-only
profile. Map partition preflight compares enumerated test identities, requires an
exhaustive and disjoint ordinary set plus exactly one match for every declared
ignored-test pattern, and fails closed when a canonical command executes no tests.
Unknown paths, unclassified shared core/assets, other world crates, or
selector-command/CI-topology changes promote to the complete gate. The combined
terrain-impact source stays full because it also carries an application-consumed
health projection. Trajectory/volume-only changes instead run their pure/direct
contract modules and casting consumers; they do not select application/UI partitions
that cannot exercise those authorities. The residual workspace corpus ordinarily runs
on its owning changes, protected-branch pushes, schedules, and candidates whose exact
combined diff selects it.
Screenshots are valid for static camera/UI/rendered-map presentation; video/human
checks are valid for motion, input response, control feel, and taste. They may judge
how hook-established state is rendered, but must never be collected, requested, or
cited to establish gameplay behavior, state transitions, ordering, settlement,
authority release, or any other logic when a gameplay hook, canonical snapshot,
renderer-free contract, or headless composition test can prove the claim. When such a
hook exists, it is the authoritative evidence and visual observation is inadmissible
for that logic. The dependency ceilings, commands, budgets, and anti-patterns are recorded in the
[gameplay](../development/gameplay-testing.md) and
[map](../development/map-testing.md) testing contracts.

PR #180's spell-resolution work now lives under the ordinary fail-closed concern
graph. Trajectory and volume rules use `trajectory_contracts`; rules, ECS, map seam,
and application consumers use their owning concerns; and the renderer-free
`hex_game/tests/spell_resolution.rs` composition target runs as the `contracts`
postflight. The
temporary delivery-only routing used while that wave was in review has been retired.
Screenshots neither satisfy nor supplement those gameplay-logic gates.

The **knowledge seam is live** as `hex_combat::knowledge`:
`FactionLatticeKnowledge::view` is the one read path for a hostile lattice.
World-owned `FactionMapKnowledge` gates which subjects currently exist to each viewer;
the gameplay adapter publishes only existence and faction, while capacity and cells
remain opaque until Reveal.
Scrying Eye's current single-target Reveal writes a complete, expiring projection for
an already-observed subject. While the subject remains observable, its known cells
refresh from live mana and disabled state without extending the lifetime; loss of
ordinary observation hides the view. The later Divination release owns the proposed
readable off-sight live feed. The HUD renders the current projection, retains a valid
aimed hostile, and freezes legal disclosure when each typed combat event enters the
bounded log. The dev reveal-all toggle remains `K` under the `dev` feature.

Around the game sits its own verification tooling. The Creator's **local lattice
test** isolates the magic ruleset and shared lattice renderer from a full fight. A
default-off
**`visual-walk`** build drives the whole game through scripted RON walks — screens,
named UI clicks, exact stack-safe terrain clicks, bounded party-idle waits, keys, and
scenario launches — photographing every step through an offscreen render target so
an agent can read the frames; `/audit-pr` runs it as a
structural and mechanical gate, with usability findings also blocking changes to UI
or presentation. Campaign reaches the 3v3 Party Trial through a selected slot, while
default-off test-support requests launch Ability Lab and Raider Mirror by stable ID.
The menus wear vendored
Cinzel/Inter type over a
design-token widget set; scenarios carry optional per-scenario lighting, and cyclic
time-of-day is available to those that opt in. The Main Menu shows the workspace
version, sessions write a `hex_game.log` beside the executable (fresh per launch),
and a panic hook puts the last words in it.

The 2026-07-29 foundation inventory contains 1,363 tests: 1,338 ordinary tests in the
complete all-feature workspace gate and 25 explicitly ignored stress/benchmark
entries. The exact list, measurements, branch matrix, and exclusions are recorded in
[foundation-hardening.md](foundation-hardening.md) rather than repeated as a brittle
project-wide constant.

The 2026-07-30 dev-integrated biome-wave checkpoint extends that complete gate to
1,583 passing ordinary tests and 32 deliberate ignored stress/benchmark entries.
Its release-mode Ring19 generation gate measures Two Rings at 3.250 seconds p95
against Seven Regions at 1.234 seconds p95, or 2.63× inside the 3.5× budget.
Automated final-SHA captures and the mandatory human visual/play review remain
separate release gates.

The standalone **Asset Workshop** is available through `cargo editor`. It loads the
canonical palette, voxel-style, and object catalogs, starts with an unsaved
calibration object, and provides palette/style editing plus hex-voxel object authoring
with semantic parts, masks, level slicing, deterministic preview rigs, camera
controls, grouped undo/redo, explicit validated saves, external-change guards, and
untracked crash recovery. A clean saved object can export a deterministic ten-view
review pack, contact sheet, and semantic report under `.context/asset-workshop/`.

The runtime resolves that complete art graph atomically and retains its last valid
revision across a bad hot reload. `hex_objects` renders static instances from cached
mesh chunks using the game prism and exact palette-backed material modes. Production
review exemplars cover nine-, sixteen-, and twenty-one-level trees, their snowy
variants, a nonblocking grass tuft and snowy variant, cave moss and lichen, and three
nonblocking emissive crystal silhouettes. Terrain substances, liquids, construction
metal, and unit presentation resolve exact palette swatches. Forest and Deep Forest
publish generated vegetation as shared `ObjectInstance`s while retaining exact
rotated blockers and stack-safe tree roots. Third Person fades an entire obstructing
tree through isolated per-tree material clones; authored canopy masks remain art
metadata. Prairie publishes nonblocking grass.
Caves publishes authored crystal `ObjectInstance`s with presentation-only
point-light children at its gameplay-light sites.
Oasis publishes exact `plant/date-palm` instances with a single blocking root each;
their seed-selected rotations and positions cannot overlap the pool, reserved routes,
or one another.

The camera action now cycles Map → Third Person → First Person → Map. Third Person
gives the player exclusive ownership of yaw, full-range pitch, and desired zoom. A
conservative probe retracts only its effective boom radius against the public
stacked-terrain projection, waits for continuous full clearance, then restores outward
monotonically. First Person instead follows the same disclosed subject at the
configured `0.6`-unit eye height with a `60°` vertical lens, a horizon entry pitch, and
no boom sweep. It keeps the tactical cursor/right-drag/click-to-move controls; it does
not capture the mouse, zoom the fixed eye, or introduce WASD character locomotion.

Near third-person retraction and First Person hide the resolved followed model through
the same composable camera-owned visibility reason. Retargeting and mode/lifecycle
transitions restore the complete model without removing fog or other owners. Returning
from either character view restores the exact saved Map pose and projection. Ordinary
gameplay keeps cave roofs intact, while explicit map-review capture may still request a
complete interior cutaway. Automated geometry, control-authority, motion-continuity,
lifecycle, idle-churn, and release-performance gates are live. Seed-exact
multi-azimuth walks exercise ordinary pointer movement to a proved destination on every
standalone selectable map and every Two Rings region. Alberto approved the corrected
third-person camera's motion and readability in a native Two Rings release walk at
runtime head `2397d8e` on 2026-08-01. On 2026-08-10, `shrav-k` approved First Person's
native three-state cycle, look and movement feel, retargeting, model restoration, and
exact Map-pose restoration on the combined `dev` head `8a8e45e4`. Map remains
available without a scenario restriction. A generated `MapViewHint` may extend its
zoom ceiling with ten percent headroom, so a large initial frame such as Mountain
Range does not snap inward on the first scroll; Third Person retains its authored
ceiling.

## What is provisional

Everything in this table is a guess standing in for a decision that
[the design](../design/game.md) explicitly has not taken. **Do not tune these into
place** — they are meant to be replaced.

| Thing | Now | What it is waiting for |
|---|---|---|
| **Initiative** | a number on a component, high to low, ties by stable `UnitId` | The initiative question; derived-from-lattice is one candidate and could also address boss action economy |
| **A turn** | 4 hexes of movement and one action; retained after the Wave 7 bounded tempo audit | Broader human playtesting and future initiative/action-economy work; Tactical two-step lengthened the fixed 3v3 fixture without a clear compensating benefit |
| **Damage** | disables lattice hexes; a player defender chooses and confirms live cells in the HUD, while non-player defenders use a deterministic cheapest-first policy | The fight-length question — how many hexes a spell should take is a feel question nobody has played with yet |
| **Enemy behaviour** | deterministic `baseline-v1`: revive, reveal, direct-damage cast, self-enchant, strike, then approach an observed live hostile | A rout threshold to know when to stop and a broader tactical policy; this remains a deliberately small baseline rather than a balance decision |
| **Engage range** | 4 hexes, 6 to disengage; perception will gate the reach trigger on observation | The numbers remain a feel question. The disengage margin stays spatial hysteresis; the separate lost-contact rule searches for one round |
| **What height is worth** | +1 hex of range per 5 levels above the target | The value remains provisional; engagement and spell targeting now share the rule |
| **How the tints look** | pale warm white, 0.22 alpha for range and 0.6 for the affordable route; a rose × marks a connected hovered destination beyond the current budget | Nothing but taste. The constants are at the top of `hex_units::selection`; change the numbers rather than the structure |

**No randomness** is *not* provisional. The design is explicit that uncertainty comes
from hidden information rather than dice, so the turn order is deterministic: ties
break by the stable `UnitId` dealt at spawn, and the same units always
produce the same order across runs and saves.

### What damage does not settle

It disables hexes and it can put a unit down, and that is deliberately as far as it
goes. **Downed is provisional**: the design leaves both functional death — a threshold
arriving before zero — and permadeath open, and a unit whose lattice is spent simply
leaves the turn order while retaining its lattice for restoration. Heal or Renewal can
reactivate it for the next round, and exploration Rest recovers it. How many hexes a
spell disables, how long a fight runs, and what a strike costs are all knobs rather
than answers; `strike_disables`
sits in `combat.ron` beside the rest precisely so it can be moved without touching code.
Further damage against an already downed target is refused before spending the action
or mana, while non-damaging inspection such as Reveal can still reach the retained
lattice.

Permanent construction now reaches terrain through exact inclusive `TilePos` and
`RunBottom` occupancy. Evocations using `Single` or `Column` publish atomic
`TerrainEdit::Set` batches for map-approved conjurable substances. Hidden material or
units suppress an unsafe batch without changing acceptance or payment. World-owned
toughness content, damage admission, ordered impact resolution/outcomes, sparse health,
terrain consequences, and observation-gated health bars are live. PR #180 makes
gameplay elemental announcement, matching-outcome consumption, refreshed
occupancy/movement, deterministic unsupported-actor settlement, authority adoption,
and exact release live as one held transaction.

**A cast can now outlast itself.** `Burn` runs through the persistent-effect runtime
(`hex_combat::effects`, vocabulary in `hex_core::effects`): a cast books a countdown in
the effect ledger, and one hex goes down at the start of each of that target's own turns.
The countdown lives **only** there. An earlier shape parked a `Vec<Burn>` inside
`LatticeState` and it was pulled back out before anything persisted it — a burn has a
source the lattice has no vocabulary for, and a tick point a rules engine with no turn
order cannot see. The two settled rules hold — the tick point is **personal, not the round
boundary**, and burn **ignores armour** while still going through the defender's choice,
so the nondeterministic choice is captured as a replayable command. No replay log is
persisted yet. What that does *not* settle is anything about the negative spiral it
accelerates: fight length, functional death, and the brakes the design names (rout,
surrender) are all still deferred, and burn deliberately ships without one. See
[systems/combat.md](../systems/combat.md#effects-that-outlast-their-cast).

## Not built, and not next

Everything in [the design](../design/game.md#open-questions)'s open questions, plus:

- **Terrain that costs something to cross.** `Reach` charges one per step, so the
  shortest route is the one taken and breadth-first order is enough to find it. Mud,
  ice or a climb would each need a priority queue, and none of them are designed.
- **A way out of a stalemate.** A melee-only enemy separated by terrain it cannot cross
  stays in the fight forever: `approach` finds no route, so it spends its turn doing
  nothing, every round. Height makes this easier to fall into, since a fight now starts
  from further away when one side is above the other. Nothing is stuck — the player can
  still walk out past the disengage margin — but the enemy should give up rather than
  wait to be left. That is the rout threshold the design names and the enemy-behaviour
  row above is waiting on. **Rout was deferred deliberately on 2026-07-27**, not
  overlooked: the threshold is a number nobody can pick honestly before fights have been
  played. `rout_policy` stays an unbuilt knob, and this stalemate is the known cost of
  waiting.
- **Multi-hex bodies.** `Body` has room for a footprint; the rule for whether a wide
  body may straddle a one-level step has not been decided.

## Casting: the playable slice and its remaining boundary

[casting.md](../systems/casting.md) records the built 0.3 path, live world-side terrain
durability, and #180's live gameplay integration. `GameCommand::Cast` is
authoritative, pays through the acting lattice, emits typed outcomes, and applies the
implemented unit effects: direct and area disables, Burn, and observed-subject Reveal.
The panel and aiming flow described above are the ordinary player path into that
command.

**The shape vocabulary resolves to exact voxels**
(`hex_units::volumes`): `SelfCast`, `Single`, `Sphere`, `Column`, `Line`, `Cone` and
`Path`, over `TilePos` in the grid-space metric where hexes and levels count equally,
handing back the sorted, deduplicated form an announcement requires. `spells.ron`'s
`TargetShape` carries the matching extents — `Blast` is now `Sphere(radius: N)` — and
validation caps them. The casting preview resolves the aimed shape and paints every
surface in it, and the cast applier refuses a shape that cannot resolve. PR #180 makes
the wider half live: Impact casts announce exact terrain volumes, area Disable/Burn
iterates every snapshotted occupant, and supported effect volumes clip radially to
obstruction. Area Restore/Reveal remains fail-closed pending its hidden-information
policy.

The live transaction preflights monotonic session-local batch ids, pays once, and keeps
a separate combat-authority hold until every queued defender answer, terrain outcome,
settlement, and exact-position adoption completes. Valid applied/rejected answers may
arrive out of order and complete independently. Foreign, duplicate, reused,
mismatched, or structurally inconsistent evidence freezes resolution instead of
timing out or advancing a turn between answers.

The binding parts are:

- **Friendly fire remains enabled.** Unit effects include allies, enemies, and the
  caster whenever the resolved volume includes them.
- **Every positional anchor must be Observed.** An area may extend into Remembered or
  Unknown positions, but presentation and logs do not reveal hidden impact outcomes.
- **Evocation terrain persists for multiple turns.** The initial implementation makes
  applied terrain edits permanent rather than keeping an expiry ledger.
- **Generated feature effects are deferred.** Trees, tall grass, and other feature
  entities ignore impacts until a feature-response and outcome contract lands.
- **Conjuration is map-approved content.** A spell may name only a substance marked
  `conjurable`; the generic `TerrainEdit::Set` path remains available for authored
  restoration and other non-spell uses.

The live implementation retains these explicit limitations:

- **Trajectories and supported effect volumes are obstruction-aware.** `Direct` and
  authored-rise `Arc` casts test exact material occupancy with one direction-symmetric
  integer supercover, while `None` deliberately bypasses it. After an anchor is legal,
  the canonical effect volume is filtered radially from that anchor over the same
  direct supercover: radial endpoints stay hittable, `None` stays byte-for-byte
  unchanged, and preview/AI use only faction-known occupancy. Authoritative casting
  uses complete `RunBottom` occupancy, so Unknown terrain cannot change
  faction-facing choices while full truth can still clip the applied volume. Authored
  range and arc rise are technically capped at 16. The same rational intersection
  foundation now serves sight through a separate strict-interior wrapper; casting's
  closed-contact supercover and endpoint policy are unchanged.
- **A breached cave roof will not admit daylight.** Terrain edits already keep the
  interior *roof* projection current, but interior **membership** is never re-derived,
  so a chamber you blow open still counts as inside. Live perception therefore
  continues to classify the chamber as Interior and does not admit daylight
  ([boundary.md](boundary.md) I).
- **Casting is provisionally combat-only.** Recovery between fights is intended to be
  a rest action, but real-time casting still needs an interaction and rest flow.
  **Rituals remain deferred** — `co_castable` parses and labels rituals in the demo,
  but has no mechanical effect.
- **Paid-on-resistance is provisional.** The gameplay adapter charges mana and the
  action once after a legal announcement even if every material resists, and retains
  payment for every valid rejection, including unavailable terrain. Playtesting may
  still change that policy without moving material authority out of the world.
- **No-undermining is provisional.** Permanent evocation construction checks its
  complete volume and emits no edits when it intersects existing material, a unit body,
  or a unit's supporting surface. The cast remains accepted and paid so hidden blockers
  are not an oracle. Destructive terrain impacts refresh footing and movement, settle
  unsupported actors deterministically, adopt their exact positions into combat
  authority, and freeze with a typed diagnostic if no legal landing exists.
- **Downed-first death is provisional.** A fully disabled unit initially leaves the
  turn order and retains its lattice. Heal or Renewal restores it into the next round
  and Rest recovers it after combat; functional death and permadeath remain open.
- **Supported area unit effects reach every exact occupant in the clipped volume.**
  `volumes::resolve` produces the full voxel list, and the transaction snapshots exact
  occupants at payment, orders authored effect then stable `UnitId`, queues one public
  defender decision at a time, includes caster/allies/enemies, and skips incidental
  downed spill targets without disclosure. Area Disable/Burn and Impact are live. Area
  Restore/Reveal stay refused pending a separately accepted hidden-information policy.
  Fireball is the first playable Impact consumer (`Fire`, power 2) and drops its
  deferred `Displace`.
- **Burn attributes one source per tick.** Several burns on one target come due as a
  single count and therefore a single decision, which has room for one `source`. The
  earliest-lit fire fills it. The rules never read `source`, so the imprecision is
  confined to the combat log.

## Not yet done, at the toolchain level

- **`bevy_lint`** is wired (`cfg(bevy_lint)` is declared, the `register_tool`
  attribute is in place) but unusable: it supports Bevy 0.18 at most, and this is
  0.19. Adopting it later costs no source changes.
- **Bevy feature trimming.** `default-features = true` still. The `3d` collection
  would cut compile time and binary size but risks silently dropping capability.
- **The animation system** is still `Box<dyn Transformer>` trait objects, which is
  why `Transformation` cannot derive `Reflect` and is invisible in the inspector.
  It works and is correctly frame-timed; it is the most likely thing to be
  rewritten when real gameplay lands.

## The production gap

Most of what makes this a product does not exist yet: no long-term-compatible save
contract, audio content, controller support, signing, or store packaging. The current
shell provides three atomic build-bound Campaign slots with one-time legacy-resume
import and invalid-record preservation, a persistent Settings menu, categorized
configurable keyboard actions, HUD visibility preferences, empty audio buses,
normalized release artifacts, and retained symbol material. The first hygiene
slice has landed — a per-session log file beside the executable, a panic hook that
writes into it, and the version on the Main Menu — but full crash *reporting*
(symbolication, upload, a dialog) has not. These replaceable seams do not close the
production gap or promise compatibility. The full checklist and evidence remain
frozen in [production-audit.md](production-audit.md); the sequenced scaffold is in
[roadmap.md](roadmap.md).
