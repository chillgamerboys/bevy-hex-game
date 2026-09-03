# Crystal Mountain wave

- Status: `integrating`
- Wave branch: `wave/crystal-mountain`
- Original published head: `74deb7f84d92e2088c63eafc1d5988c63171896d`
- Original stack base: `fc55bd5a1c3c0181b6506d5ac59e1189d287838a`
- Refresh base: `origin/dev @ 495a73dcbe7edbab6d993867d91b15979fa6ce81`
- Additive refresh merge: `2e175917201d005b11a8b6a207963acf13bc35bc`
- Prerequisite: the refresh base contains the delivered Crystal Ascent and First Person
  implementation
- Coordinator: Codex / world integration
- Epic: Crystal Mountain and cross-biome tunnel
- Shippable outcome: one selectable radius-77 mountain whose only ordinary foot-to-basin
  route is a lit, four-wide tunnel into the existing Crystal Ascent and out into a wooded
  level-150 summit basin enclosed by higher ridges.
- Exclusions: combat population, doors, branches, water features, destructible tunnel
  walls, save migration, alternate surface ascent, and new runtime movement/perception
  mechanics.

## Why this wave exists

The outcome crosses Macro layout settings, exact mask ownership, global volume composition,
interior/light-domain publication, presentation cutaway, and scenario/catalog wiring. Those
parts are independently testable but only meaningful as one composed route, so they ship as
one wave rather than unrelated PRs.

## Locked decisions

1. **D1:** "Crystal Ascent remains centered at the world origin, rotation zero, base level 6,
   and rise 144; its exact radius-32 site may borrow only the columns it needs from adjacent
   Macro masks."
2. **D2:** "The tunnel is the only ordinary foot-to-basin route: level 6, exactly four lanes,
   six clear levels, at least three solid roof levels, and a rough-hewn body transitioning to
   worked Gothic masonry for its final twelve centerline rows."
3. **D3:** "The tunnel and Crystal Ascent publish one Dark interior and one light domain; only
   the foot and summit thresholds are exterior entrances, and every required floor is at
   least Dim from paired nonblocking crystal lights."
4. **D4:** "The five consecutive upper radius-two cells are temperate Forest basin, the other
   radius-two cells are inner mountain wall, and all radius-three cells are outer slope and
   enclosing ridge; no alpine treeline removes temperate basin or Crystal crown trees."
5. **D5:** "Existing runtime traversal, occupancy, LOS, fog, authored-heart occupancy, camera,
   and save contracts remain authoritative; this wave adds generation and presentation data,
   not parallel gameplay mechanics."
6. **D6:** "Legacy Macro configuration and fingerprints remain byte-identical when the new
   defaulted walker, spanning-feature, and anchor-alias collections are empty."

## Shared foundation

- World authority: `MacroLayoutSettings`, `ResolvedLayoutPlan`, exact patch masks and seam
  contracts in `hex_map` remain the source of layout truth. L1 adds defaulted explicit walker,
  tunnel, alias, and resolved subsurface vocabulary without changing legacy behavior.
- World authority: `GeneratedWorldPlan`, `PlannedInterior`, planned lights, anchors, semantic
  volume, and feature ownership remain the runtime publication inputs. L2 composes the tunnel
  once after fragment merge and before final validation.
- Shared presentation: current cutaway and feature reconciliation consume the world-owned
  interior and roof facts. L3 adds no new visibility authority.
- Shared integration: scenario and Sandbox catalogs select the new world recipe. L4 adds no
  generation policy.
- Landing plan: L1 contracts merge first; L2 consumes them; L3 and L4 remain additive and
  merge after the generator contract is stable. The coordinator owns all shared hotspots and
  the final composed validation.

## Dispatch queue

```yaml
- id: L1
  title: Macro spanning-feature contracts
  order: orders/L1-macro-contracts.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/crystal-mountain-contracts
  owns:
    - crates/hex_map/src/settings.rs (Macro settings vocabulary and validation)
    - crates/hex_map/src/procedural_v3/layout.rs (resolved walker/subsurface/alias contracts)
    - crates/hex_map/src/procedural_v3/fingerprint.rs (conditional Macro extension)
    - crates/hex_map/src/procedural_v3/seam.rs (explicit four-wide walker seam validation)
    - docs/planning/waves/crystal-mountain/manifest.md (L1 queue row only)
  dispatch_blockers: []
  merge_blockers: []
  fences:
    - path: crates/hex_map/src/procedural_v3/fingerprint.rs
      disposition: keep
      reason: old Mountain Range and Two Rings fingerprints must remain byte-identical
  selector:
    concerns: [map_unit, map_generation, map_contracts, clippy, docs, shipping]
    full: false
  evidence: logic-only
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
- id: L2
  title: Crystal Mountain composition and tunnel
  order: orders/L2-world-composition.md
  ticket: null
  authority: world
  builder: worker
  branch: wave/crystal-mountain
  owns:
    - crates/hex_map/src/procedural_v3/macro_world.rs (merge/finalize and global tunnel)
    - crates/hex_map/src/procedural_v3/macro_spanning.rs (global tunnel planning and carving)
    - crates/hex_map/src/procedural_v3/composition.rs (staged composition seam)
    - crates/hex_map/src/procedural_v3/crystal_ascent.rs (Macro landmark validation seam only)
    - crates/hex_map/src/procedural_v3/mod.rs (Crystal Mountain dispatch/report regions)
    - crates/hex_map/tests/contracts/composed_worlds.rs (Crystal Mountain composed-world contracts and budgets)
    - docs/planning/waves/crystal-mountain/manifest.md (L2 queue row and integration ledger)
  dispatch_blockers: []
  merge_blockers: [L1]
  fences:
    - path: crates/hex_map/src/procedural_v3/macro_world.rs
      disposition: keep
      reason: existing Macro route, generation-budget, and full-world validators remain live
  selector:
    concerns: [map_generation, map_contracts, clippy, docs, shipping]
    full: false
  evidence: static-presentation
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
- id: L3
  title: Interior cutaway and overlying-feature reconciliation
  order: orders/L3-presentation.md
  ticket: null
  authority: world
  builder: worker
  branch: feat/crystal-mountain-presentation
  owns:
    - crates/hex_world/src (Crystal Mountain cutaway and feature-lifecycle regions only)
    - crates/hex_map/src/grid.rs (rendered-run visibility ownership)
    - docs/planning/waves/crystal-mountain/manifest.md (L3 queue row only)
  dispatch_blockers: []
  merge_blockers: [L2]
  fences:
    - path: crates/hex_world/src
      disposition: keep
      reason: Map remains opaque and gameplay camera behavior remains unchanged
  selector:
    concerns: [selector, rules, trajectory_contracts, contracts, simulation, app, map_unit, map_generation, map_contracts, residual, clippy, docs, shipping]
    full: true
  evidence: static-presentation
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
- id: L4
  title: Crystal Mountain selection, review route, and docs
  order: orders/L4-content-docs.md
  ticket: null
  authority: shared
  builder: worker
  branch: feat/crystal-mountain-content
  owns:
    - assets/config/worlds/procedural-crystal-mountain.ron
    - assets/config/encounters/crystal-mountain-showcase.ron
    - assets/config/scenarios.ron (Crystal Mountain entry only)
    - assets/config/sandbox_maps.ron (Crystal Mountain entry only)
    - crates/hex_game/src/scenarios.rs (embedded Crystal Mountain path only)
    - crates/hex_game/src/save.rs (embedded Crystal Mountain path allowlist only)
    - crates/hex_game/src/walk.rs (Crystal Mountain review route only)
    - crates/hex_assets/src/sandbox.rs (Crystal Mountain catalog regression only)
    - .config/test-scopes.json (exactly three Crystal required-ignored identities only)
    - .github/workflows/stress.yaml (Crystal Mountain ignored acceptance jobs only)
    - tools/test_test_scope.py (Crystal Mountain selector regression only)
    - assets/ui/sandbox/crystal-mountain.png
    - walks/camera_crystal_mountain.ron
    - walks/camera_routes.ron (Crystal Mountain route only)
    - CLAUDE.md (authored-map visual-matrix exception only)
    - docs/README.md
    - docs/contracts.md
    - docs/design/game.md
    - docs/development/config.md
    - docs/development/gameplay-testing.md
    - docs/systems/map.md
    - docs/systems/world-generation-v3.md
    - docs/systems/interiors.md
    - docs/systems/lighting.md
    - docs/systems/camera.md
    - docs/systems/perception.md
    - docs/planning/status.md
    - docs/planning/roadmap.md
    - docs/planning/waves/crystal-mountain/manifest.md (L4 queue row only)
  dispatch_blockers: []
  merge_blockers: [L1, L2]
  fences: []
  selector:
    concerns: [selector, rules, trajectory_contracts, contracts, simulation, app, map_unit, map_generation, map_contracts, residual, clippy, docs, shipping]
    full: true
  evidence: motion-or-feel
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
```

## Ownership map

- L1 exclusively owns the serialized and resolved vocabulary. L2 consumes those types but
  does not edit their definitions or fingerprint encoding.
- L2 exclusively owns world construction, tunnel routing/carving, unified interiors/lights,
  route/path validators, and generation benchmarks.
- L3 exclusively owns review-cutaway presentation behavior and lifecycle tests. It may read
  semantic roof/interior facts but may not infer them from rendered entities.
- L4 exclusively owns selectable content, embedded paths, scripted review actions, and the
  listed documentation. It may not add generation policy in configuration consumers.
- `manifest.md` is the expected overlap: each worker changes only its queue row; the
  coordinator reconciles all rows after integrating commits.
- Hotspot rule: L2 refreshes after L1 before compiling. L3 refreshes after L2 before its
  final tests. L4 refreshes after the final config schema and generated anchor names exist.
- The coordinator owns post-lane final-candidate repairs and their narrow regression hooks.
  Current territory is the exact Macro framing/selection/composition performance path in
  `local_frame.rs`, `selection.rs`, `composition.rs`, and `macro_world.rs`; rendered-run
  cutaway ownership in `hex_map/src/grid.rs`; test-only fog snapshots and the composed
  Crystal Mountain runtime lifecycle in `hex_game/src/fog.rs` and `scenarios.rs`; plus the
  additive `Cargo.lock` base refresh and final authored-map acceptance repairs in
  `CLAUDE.md`, `docs/design/game.md`, `docs/systems/perception.md`, and the Crystal Mountain
  walk. These edits remain identifiable integration work and do not retroactively widen a
  source lane's crate authority.

## Territory

- PR #197 / `wave/crystal-ascent` owns the prerequisite Crystal Ascent and first-person
  implementation. This wave carries that exact head and does not rewrite its approved route.
- `origin/dev` contains no competing Crystal Mountain implementation at wave creation.
- Untracked `.context/` is scratch and outside every lane.

## Integration order

1. L1 contracts and legacy fingerprint closure.
2. L2 mask claim, terrain composition, tunnel, interior, lights, anchors, and validators.
3. L3 presentation reconciliation and lifecycle coverage.
4. L4 selectable content, visual walk, documentation, and benchmark records.
5. Coordinator aggregate review, selector plan, full CI-equivalent gate, captures, and named
   First/Third/Map camera review.

## Integration ledger

| Lane | Integrated commits | Result |
|---|---|---|
| L1 | `1ae8712`, `c264d93`, `82df844` | defaulted spanning, walker, alias, fingerprint, and seam contracts |
| L2 | `cbac20c` through `90ac1bf`, `fcfa603`, `98c9fce` | staged composition, exact tunnel, landscape, validation, lighting containment, rotation stability, and benchmarks |
| L3 | `e6a638c` | review cutaway hides and restores exact overlying roof trees |
| L4 | `b9d7361`, `044b088`, `5686748` | selectable content, docs, preview, and deterministic camera route |
| Refresh | `b887ecb`, `8bd1fe3` (`origin/dev @ fc55bd5`) | multiplayer/VFX additions retained additively before final gates and lockfile refreshed |
| Post-reconciliation refresh | `2e175917201d005b11a8b6a207963acf13bc35bc` (`origin/dev @ 495a73dc`) | identity selection retained; all prior local, hosted, capture, and human evidence reset |

`merged-to-wave` records only that each lane's commits are present on the composed branch.
It does not record candidate acceptance. The release build, selector-chosen complete gate,
visual captures/review, and named-human runtime review remain release blockers and are not
recorded as passed by this manifest.

## Combined acceptance

- Load Crystal Mountain and publish exactly 18,019 disjoint radius-77 columns.
- Prove the exact seven-cell logical Crystal core, exact radius-32 authored claim, connected
  neighboring masks, five-cell high Forest basin, and enclosing inner/outer mountain ridges.
- Prove the only ordinary route is foot apron -> four-wide level-6 tunnel -> Crystal lower
  threshold -> approved 144-level ascent -> summit -> every basin section.
- Prove tunnel clearance/roof/seam lane pairs, unified Dark interior/domain, Dim route
  coverage, exact Bright pools, and physical occupancy/LOS/fog. No liquid or traversal
  blocker may intersect the tunnel, apron, or reserved clearance; basin trees remain valid.
- Regenerate representative seeds and all six rotations; teardown and re-enter gameplay and
  review cutaway without stale roofs, lights, features, anchors, or floating trees.
- Inspect Map, First Person, and Third Person frames for foot portal, natural tunnel, Gothic
  transition, chamber, ascent, summit, basin, ridge, lighting overlay, and review cutaway.
- Run selector-chosen CI, full candidate gate, the radius-77 camera budget, preserved
  radius-40 perception budgets, the 10,000-idle-frame perception gate, and Macro
  generation/materialization/entity/peak-memory comparisons.

The approved authored-map walk exception is exactly these 23 deterministic frames:

| # | Scripted capture name |
|---:|---|
| 1 | `01-crystal-mountain-opaque-massif-map` |
| 2 | `02-crystal-mountain-rear-ridge-basin-map` |
| 3 | `03-crystal-mountain-foot-portal-map` |
| 4 | `04-crystal-mountain-foot-portal-character` |
| 5 | `05-crystal-mountain-foot-portal-first-person` |
| 6 | `06-crystal-mountain-natural-tunnel-map` |
| 7 | `07-crystal-mountain-natural-tunnel-character` |
| 8 | `08-crystal-mountain-natural-tunnel-first-person` |
| 9 | `09-crystal-mountain-gothic-transition-map` |
| 10 | `10-crystal-mountain-gothic-transition-character` |
| 11 | `11-crystal-mountain-gothic-transition-first-person` |
| 12 | `12-crystal-mountain-crystal-chamber-map` |
| 13 | `13-crystal-mountain-crystal-chamber-character` |
| 14 | `14-crystal-mountain-crystal-chamber-first-person` |
| 15 | `15-crystal-mountain-mid-ascent-map` |
| 16 | `16-crystal-mountain-mid-ascent-character` |
| 17 | `17-crystal-mountain-mid-ascent-first-person` |
| 18 | `18-crystal-mountain-summit-exit-map` |
| 19 | `19-crystal-mountain-summit-exit-character` |
| 20 | `20-crystal-mountain-summit-exit-first-person` |
| 21 | `21-crystal-mountain-wooded-basin-map` |
| 22 | `22-crystal-mountain-wooded-basin-character` |
| 23 | `23-crystal-mountain-wooded-basin-first-person` |

Five separately launched review captures complete the approved matrix without changing the
ordinary movement walk: `24-crystal-mountain-ridge-map`,
`25-crystal-mountain-ridge-character`, `26-crystal-mountain-ridge-first-person`,
`27-crystal-mountain-illumination-overlay-map`, and
`28-crystal-mountain-full-cutaway-map`.

All five use scenario `Crystal Mountain`, seed `1592598566`, a release build with the
default-off `map-review` feature, and the named output file below. Empty cells mean the
environment variable is omitted.

| Capture | Focus anchor | View | Camera | Cutaway | Illumination |
|---|---|---|---|---|---|
| `24-crystal-mountain-ridge-map` | `crystal_mountain.ridge` | `default` | `map` |  |  |
| `25-crystal-mountain-ridge-character` | `crystal_mountain.ridge` | `rotated` | `character` |  |  |
| `26-crystal-mountain-ridge-first-person` | `crystal_mountain.ridge` | `rotated` | `first-person` |  |  |
| `27-crystal-mountain-illumination-overlay-map` | `crystal_mountain.midpoint` | `top-down` | `map` | `full` | `overlay` |
| `28-crystal-mountain-full-cutaway-map` | `crystal_mountain.midpoint` | `top-down` | `map` | `full` |  |

Map each non-empty column to `HEX_REVIEW_FOCUS_ANCHOR`, `HEX_REVIEW_VIEW`,
`HEX_REVIEW_CAMERA`, `HEX_REVIEW_CUTAWAY`, and `HEX_REVIEW_ILLUMINATION`; map the capture
name to an absolute `HEX_REVIEW_CAPTURE` path ending in `.png`.

## Exact-head evidence ledger

This table is the release ledger, not a list of focused development runs. Every result must
name the exact `wave/crystal-mountain` candidate SHA it exercised. A later commit invalidates
every applicable row until it is rerun. Static frames may establish only rendered geometry,
lighting, cutaway, and camera composition; the typed map/runtime rows establish all exact
world and gameplay facts.

| Evidence axis | Current refreshed state |
|---|---|
| Local evidence | `PENDING` on the final refreshed head; the old full gate and 28 captures at `74deb7f` are historical |
| Hosted CI | `PENDING`; the old rollup was green except for a cancelled macOS build and cannot validate the refresh |
| Exact-head human | `PENDING`; the successful draft check was a deferral, not human approval |
| Delivered to `dev` | `NO`; the refreshed head is not yet an ancestor of `origin/dev` |

The base and planning-document changes in this refresh invalidate all 28 old captures as
current-head evidence. Regenerate them against the final candidate before readiness.

| Evidence | Required record | Candidate head / result |
|---|---|---|
| Selector and CI-equivalent gate | selector plan, every selected concern, workspace doctests, formatting, dependency policy, strict Clippy, docs, shipping build, rules/simulation graph closure, map partition closure, relative links, and terminology scan | `PENDING` |
| Deterministic world contracts | all six rotations, representative/32-seed corpus, exact four-wide route and seam closures, unified interior/light domain, basin reachability, and teardown/re-entry | `PENDING` |
| Runtime authority contracts | terrain, occupancy, authored-heart LOS, perception/fog, cutaway/feature lifecycle, and clean gameplay re-entry from typed snapshots | `PENDING` |
| Release budgets | Macro generation, materialization/entity/peak-memory comparison, radius-77 camera, radius-40 perception, dense six-observer perception, and 10,000 idle frames | `PENDING` |
| Static presentation | bounded automated walk plus Map/First Person/Third Person review matrix, illumination overlay, and full cutaway; retain artifact paths and mechanical verdict | `PENDING` |
| Human runtime | named human, native route, date, exact SHA, and explicit `PASS`; required before the PR leaves draft | `PENDING` |

The named-human route starts at `crystal_mountain.foot_apron` and uses native pointer input
through tunnel mouth, midpoint, Gothic transition, Ascent threshold, bottom chamber,
mid-flight, summit exit, the exact level-150 summit walker connection, basin clearing, and
one adjacent Forest basin section. At the portal, tunnel, ascent, summit seam, basin, and
ridge, the reviewer cycles Map -> Character -> First Person -> Map, exercises native orbit
and forward/back movement, and checks camera collision, first-person roof/subject occlusion,
ordinary opaque-Map restoration, route responsiveness, and motion/feel. The record names the
reviewer, date, exact SHA, and explicit `PASS`; any later push makes it stale.

Focused tests and benchmark samples run before this ledger receives an exact candidate SHA are
useful development evidence only. They must not be promoted to combined acceptance by wording
in status, roadmap, a PR description, or a screenshot caption.

## Stop conditions

- A required mask subtraction disconnects an adjacent biome.
- Tunnel routing requires an undeclared seam, surface bypass, bedrock breach, or a second
  runtime traversal/perception authority.
- Legacy Macro settings or fingerprints change with empty extensions.
- A presentation lane begins reconstructing world truth from rendering state.
- The prerequisite Crystal Ascent/first-person head changes incompatibly before this wave
  lands; refresh additively and rerun the exact route gates.

## Injection log

- 2026-08-10: initial four-lane implementation wave approved by the user.
- 2026-08-20: coordinator reconciled the live queue metadata, selector vocabulary, lane
  ownership, and pending-evidence state after integration; no locked decision was amended.
- 2026-09-02: merged post-#218 `dev` additively at `2e175917`; retained exactly three
  Crystal required-ignored identities and reset every current-head evidence axis.

## Close-out

Keep the `wave/crystal-mountain -> dev` PR in draft while combined acceptance is pending.
It must not become non-draft until a named human records `PASS` against the exact candidate
head; any later commit invalidates that classification. After landing, record the exact merge
SHA, close superseded source PRs, update delivery state, and remove transient `orders/` files
in the close-out PR.
