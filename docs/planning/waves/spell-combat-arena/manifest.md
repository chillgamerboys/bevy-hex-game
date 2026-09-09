# Spell Combat Arena — local experiment

Status: implemented locally, native playtest pending. Branch: experiment/spell-combat-arena. Base: dev 495a73dcbe7edbab6d993867d91b15979fa6ce81. Coordinator: root. Ticket: null (Linear reauthentication required). Original combined candidate 6c89d48 passed all 19 required checks and static inspection of 22 windowless captures; later follow-ups retain separate validation receipts in task outputs.

Outcome: native offline first-person spell duel against a machine opponent. The user initially chose a local experiment branch until playtesting, then authorized one combined draft PR on 2026-09-08, including the encounter and bestiary continuations. The existing branch targets `dev`; no merge or production promotion is authorized.

## Locked decisions

1. First person default; C toggles tightly constrained third person. WASD, mouse look, Space jump, Shift sprint, 1/2/3 select. All spells release on mouse-up; Shield/Fireball gain launch speed over a 0.75-second hold and Area Blast stays fixed. T toggles selected projectile preview. Enter or Start begins from a frozen ready screen. Escape/Tab pauses and releases the mouse; no live HUD menu button. The paused menu includes Resume, Reset, Fullscreen/Windowed, Quit, and tuning. R fully resets to the ready screen.
2. Shield is a physical ballistic seed whose first terrain or actor impact anchors upright stone cover, persistent until destroyed. No support requirement: filter terrain, bounds, and both bodies per cell at emergence completion. Actor hits add a gentle horizontal impulse with zero direct HP damage. Partial/no-space results still consume the release cooldown; existing terrain and its damage remain untouched.
3. Fireball is ballistic, detonates once at earliest body/terrain hit, damages its caster. Area Blast is self-centered and excludes its caster. Radial damage and knockback ignore cover. All explosive damage uses world-space spheres.
4. HP100; shield/fireball/area cooldowns5/1.25/7 seconds; max actor damage0/35/45; terrain power2. Projectile launch speed32, gravity12. Three independently chosen size presets: shield3x4/5x5/7x6 at thickness1; fireball radius1.5/2.5/3.5; blast radius2.5/4/5.5. Standard default.
5. Accepted M01 walk3.5/run7/body2 levels/radius.25/step1/jump3.25/gravity17.333334/coyote and buffer.1; no flight, no automatic terrain recovery teleport for ordinary falling. External impulse velocity survives input.
6. Radius12, .4 level height. Two elevated platforms with ramps, cover, destructible soil, protected bedrock. No V4, multiplayer, progression, or visual-lab imports.

## Foundation and public interfaces

`hex_core::arena` defines ArenaTick, ArenaSystems::{ApplyTerrain,PublishTerrain,Simulate}, ArenaVoxelGeometry, ArenaTerrainView (complete sorted occupancy, revision, two spawn positions), ArenaMaterials (catalog identities), ArenaReset (generation). Only world writes terrain facts. Voxel top is level*.4 and bottom is (level-1)*.4 in this isolated producer.

World owns existing TerrainEdit and TerrainImpact processing and exact TerrainImpactOutcome. Gameplay reads the public terrain view and emits those messages; it must not access VoxelMap. New arena producer is isolated inside hex_map and reuses production private VoxelMap and TerrainDamageState; it avoids importing the pending large-map branch. Gameplay has its own real continuous actors and never uses the inspection ghost/tactical authority. Presentation reads immutable simulation state and submits ActorIntent.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Arena terrain producer
    order: orders/L1-world.md
    ticket: null
    authority: world
    builder: worker
    branch: experiment/arena-world
    owns: [crates/hex_map/src/arena.rs, crates/hex_map/src/arena/, "crates/hex_map/src/lib.rs: arena module export", "crates/hex_map/Cargo.toml: arena feature and ron dependency"]
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector: {concerns: [map_unit, map_contracts, clippy, docs, shipping], full: true}
    evidence: static-presentation
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L2
    title: Continuous actors and spell simulation
    order: orders/L2-gameplay.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: experiment/arena-gameplay
    owns: [crates/hex_arena/]
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector: {concerns: [contracts, simulation, clippy, docs], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
  - id: L3
    title: Native launch, input, camera, HUD, capture and integration
    order: orders/L3-integration.md
    ticket: null
    authority: shared
    builder: worker
    branch: experiment/spell-combat-arena
    owns: [crates/hex_game/, Cargo.toml, Cargo.lock, assets/config/arena.ron, tools/arena.py, docs/planning/waves/spell-combat-arena/, docs/architecture.md, docs/contracts.md, docs/planning/status.md]
    dispatch_blockers: []
    merge_blockers: [L1, L2]
    fences: []
    selector: {concerns: [app, clippy, docs, shipping], full: true}
    evidence: motion-or-feel
    sizing: {model: inherited, effort: inherited}
    state: merged-to-wave
    pr: null
```

## Territory and integration

Current upstream dev remains495a73d. Open PR219 includes unrelated GrandV3/grounded work; PR220 is V4; PR196 is tactical lattice work. All stay outside this candidate. The active visual-lab checkout is read-only source for M01 controller/collision; selectively port those files rather than merging its branch. Root writes foundation core geometry and shared manifest. Worker branches own only their declared crates; root integrates additive commits. No published branches are rebased.

## Acceptance

At 120Hz apply/reset world, publish/flush, refresh collision, simulate actors and casts, then next tick settles effects. Windowless static review: whole-footprint overview, first person, close third person, shield/impact sizes, HUD/tuning, at least two cover angles. Typed tests: round-trip geometry, true radial selection, nearest swept collision, continuous movement and impulses, cooldown and click edges, self effects, terrain damage/outcome, valid/invalid shields, immediate next-tick collision, reset/focus/KO. Native feel remains human pending. Focused lane tests first; combined selector and lint/shipping gates once. Record known baseline failures separately. Inspect every rendered frame before delivering.

## Stop conditions and close-out

No deletion of source/saves/review evidence; disk cleanup authorized separately by user and receipted in task outputs. No native focus-stealing review. Missing authoritative fact requires a shared-contract correction, never private world reconstruction. Finish with committed local prototype, launcher, controls/tuning guide, and exact-head validation report. No automatic merge.

## Local checkpoints

- World producer committed and integrated; eight focused world tests passed.
- Gameplay committed and integrated; first focused run passed 26 arena tests and 124 core tests. Round-ending, moving-body ordering, enclosed launch, and full ballistic-lifetime regressions are included in the combined candidate.
- Native composition, input, two cameras, HUD, tuning, windowless capture matrix, and combined lifecycle tests are authored. Native build, seven lifecycle tests, and strict workspace lint passed before the final layout correction. Initial static review found and corrected paused-panel height, footer readability, and third-person self obstruction. The final source-specific gate and capture receipts are retained in the task outputs; native playtesting remains pending.
- Obsolete generated caches were removed under the user's explicit cleanup instruction; the output receipt records 14,178,758,656 physical bytes recovered. Source checkouts and review evidence were preserved.
- Overnight follow-up was paused after the original local delivery.

## Requested bot follow-up

This bounded follow-up uses one gameplay implementation owner, with independently
reviewed tests and a shared-application integration check. It creates no additional
branch, lane, PR, or world authority. The bot still submits ordinary `ActorIntent`
values and obeys the same movement, spell physics, HP, and cooldown rules.

At five decisions per second it checks terrain line of sight, approaches distant
visible targets, strafes, and retreats from unsafe fireball range. It uses Area
Blast nearby, attempts a validated Shield when hurt, low on health, or facing an
incoming visible fireball, and otherwise fires a low ballistic arc with a small
horizontal lead and seeded error. Shield and fireball admission reuse the human
trajectory preview; the actual shot still resolves against moving bodies and the
current world. Local probes avoid immediate obstacles and unsupported steps. There
is no navigation, learning, intercept search, or tracking through opaque walls.
Bot pressure and motion remain native-playtest questions; typed tests establish
its decisions, actual hits, wall edits, reset, and normal-arena composition.

## Requested start/menu follow-up

One shared-application implementation owner handles the ready gate, keyboard
pause, cursor lifecycle, and menu actions together, with separately authored
regressions in the same local candidate. A setup/reset tick publishes the actors
and terrain with player input cleared and the bot disabled; the ready screen and
paused menu then stop simulation ticks and discard elapsed catch-up time. Enter
and Start explicitly begin combat. Escape or Tab is the menu key; the player's
correction explicitly excludes a live on-screen Menu button. Focus loss pauses;
UI start/resume clicks never cast. Reset returns to ready. Fullscreen toggles the
native window mode while combat remains frozen; Quit requests normal app exit.
Typed input, window, exit, and layout checks cover the transitions. A fresh six-view
windowless menu matrix covers ready, paused, both playable cameras, and both arena
azimuths; native key and window-manager feel remain a playtest route.

## Charged shots and forgiving shields

Approved follow-up from local `ca1b06e`. The existing gameplay and shared-application
lanes form one combined local review unit. Root owns charge contracts and integration;
contributors own disjoint spell, bot, and presentation files. No new world mutation
contract or remote release is introduced. The shared interface is established before
adapting the bot and native input.

Actor-owned charge state advances at 120 Hz, accepts press/release/held intent, and
is cancelled immediately on pause/focus loss, spell switch, reset, or knockout.
Release uses current aim, and a refused cooldown press cannot auto-arm later. A
zero-duration tap and one-second charge use reference range multipliers one-third
and 1.3, linearly interpolated, with launch speed equal to reference speed times
the square root of that multiplier. The bot uses this same path and aims with the
actual release speed. UI shows charge progress or the fixed Area Blast release
prompt; trajectory assistance shares construction and collision with real shots.

Shield footprints stay at the physical contact point and grow upward even on
wall/ceiling/actor hits. Every cell is independently clipped against current
occupancy, bounds, actors, and same-tick shield reservations after 0.18 seconds.
Existing impact outcomes settle before admission; surviving cells publish through
TerrainEdit before the next movement tick. Actor impact applies a single 2 units/s
horizontal impulse, with no teleport, vertical launch, or HP damage. Pinned actors
leave gaps. An empty result reports no room without overwriting terrain or healing
existing stone. Preview outlines the lowest surviving cell in each column.

Acceptance adds real charged-flight range and preview tests, release/cancellation
and render-rate input checks, gentle push and partial-placement races, preservation
of existing voxel damage, and a fresh exact-commit charge/partial-preview capture
matrix. Record focused automated checks and static review separately from native
short-lob and shield-usability playtesting.

### Charged-shot follow-up validation scope

The combined candidate uses the existing 120 Hz world publication lane. Follow-up
checks include the entire `hex_arena` unit suite, arena application tests with
`arena-prototype,test-support`, workspace formatting/link checks, strict all-feature
workspace Clippy, and the native Cargo launcher build. Fresh windowless captures
cover the ready/menu states, partial/full charge meters, release-only Area Blast,
and per-column partial Shield previews in both playable cameras. Runtime receipts
record actual input edges and authoritative charge state; image review remains
separate from gameplay tests. The original prototype's broader gate receipt is
historical evidence, not certification of this follow-up. Native charge feel and
subjective Shield balance remain pending the user's playtest.

## Stronger bot and faster charging

This follow-up supersedes the earlier reactive bot description and one-second
charge default. It remains one gameplay-owned local candidate: root integrates
configuration, cue/forecast contracts, charge timing, and telemetry before the bot
implementation; independent regression and application evaluation work shares the
combined verification. No new branch, remote PR, world API, or separate release is
introduced. The active visual experiment remains untouched.

Both actors reach maximum charge in 0.75 seconds with unchanged range endpoints and
spell rules. The bot uses observed target memory, coarse combat cues, persistent
preparation, limited recent-cover suppression, occasional timed ambushes, and
bounded controller-validated routes around cover. It has a fixed strength profile,
with no access to hidden live actor state in decisions or trajectory forecasts.
The behavior and local comparison procedure are documented in
[the arena bot guide](../../../systems/arena-bot.md).

The combined follow-up gate covers arena unit tests, real-world arena application
checks and paired frozen-baseline evaluations, workspace formatting and strict
all-feature Clippy, documentation links, and a native Cargo build. Charge capture
fixtures scale to the configured duration. Static captures establish presentation
only; beginner and practiced human win rates remain playtest targets.
