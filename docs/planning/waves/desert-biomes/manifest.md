# Desert biomes wave

- Status: `review-ready`
- Wave branch: `wave/desert-biomes`
- Base `origin/dev`: `fc55bd5a1c3c0181b6506d5ac59e1189d287838a`
- Required stacked dependency: Crystal Mountain exact head
  `74deb7f84d92e2088c63eafc1d5988c63171896d` (draft PR #210)
- Composed implementation commit: `7d9047947acbdbf101b29ca45da8c87ad0931f7f`
- Coordinator: Codex / world integration
- Epic: Arid biomes and Desert Oasis Rings; no Linear ticket is assigned
- Shippable outcome: three focused radius-12 Arid maps and one radius-55 Ring19 desert
  composition, all selectable in Sandbox with permanent deterministic world contracts and
  review routes.
- Exclusions: islands and archipelagoes, combat populations, dynamic weather, moving sand,
  water routes between biomes, boats, new movement rules, and Crystal Mountain changes.

## Why this wave exists

The Arid recipes share serialized tags, fingerprint rules, exact terrain generation,
Ring19 composition, one authored palm asset, shipping catalogs, and a common presentation
review. They are useful independently but must compose without changing legacy V3
fingerprints or Ring19 topology, so they form one candidate rather than unrelated changes.

This manifest reconciles work that was already composed directly into implementation commit
`7d904794`. It does not invent source-lane PRs. `merged-to-wave` below means the coordinator
verified each lane's implementation in that one composed commit; it does not claim a lane PR
or a source-branch merge.

## Locked decisions

1. **D1:** "`Arid` owns Desert Transition, Desert Plain, Dunes, and Oasis; the first three
   ship as radius-12 `Single` maps."
2. **D2:** "Desert Oasis Rings is the existing radius-55 Ring19 topology: one central Oasis,
   six rotated inner Dunes, and an outer ring alternating six taller Dunes with six Desert
   Plain regions."
3. **D3:** "All 42 reciprocal Ring19 seams remain Dry. Oasis water is local Still water and
   never becomes composite seam hydrology."
4. **D4:** "The default `TwoRings` encoding and all existing recipe/environment tags remain
   byte-identical; Arid and `DesertOasis` use new nonconflicting tags and a conditional
   fingerprint extension."
5. **D5:** "The date palm has one exact blocking root; palm crowns may overhang protected
   ground only where exact terrain and authored-object overlap validation allow it."
6. **D6:** "Screenshots prove the rendered review surface only. Typed tests remain the oracle
   for topology, materials, blockers, determinism, and connectivity; named human visual and
   play approval remains required."

## Shared foundation

- World authority: V3 settings, layout, generation, semantic fingerprints, volumes,
  vegetation, anchors, and Ring19 masks remain authoritative in `hex_map`. L1 extends those
  contracts without changing legacy defaults.
- World authority: authored-object catalog data and scenario/world configuration publish the
  palm and four selectable maps. L2 adapts existing loaders and UI catalogs; it adds no new
  runtime authority.
- Shared presentation: the existing camera and visual-walk runner consume published anchors.
  L3 adds review scripts and assertions, not generation policy.
- Landing plan: Crystal Mountain head `74deb7f` must land or be reconciled first. Within this
  wave L1 precedes L2, L3 verifies the composed tree, and the coordinator owns any final
  integration-only repairs and the single candidate gate.

## Dispatch queue

```yaml
- id: L1
  title: Arid contracts and deterministic generation
  order: ""
  ticket: null
  authority: world
  builder: "@codex"
  branch: wave/desert-biomes
  owns:
    - crates/hex_map/src/lib.rs (Arid exports only)
    - crates/hex_map/src/procedural.rs (Arid report fields only)
    - crates/hex_map/src/settings.rs (Arid environment, recipes, and Ring19 profile)
    - crates/hex_map/src/procedural_v3/arid_landform.rs
    - crates/hex_map/src/procedural_v3/desert_plain.rs
    - crates/hex_map/src/procedural_v3/desert_transition.rs
    - crates/hex_map/src/procedural_v3/desert_vegetation.rs
    - crates/hex_map/src/procedural_v3/dunes.rs
    - crates/hex_map/src/procedural_v3/oasis.rs
    - crates/hex_map/src/procedural_v3/fingerprint.rs (Arid tags and conditional profile suffix)
    - crates/hex_map/src/procedural_v3/layout.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/composite_patch.rs (Arid patch dispatch only)
    - crates/hex_map/src/procedural_v3/mod.rs (Arid generation dispatch only)
    - crates/hex_map/src/procedural_v3/ring19.rs (DesertOasis generation, excluding L3 tests)
    - crates/hex_map/src/procedural_v3/ring7.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/vegetation.rs (date-palm identifier only)
    - crates/hex_map/src/procedural_v3/caves.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/deep_forest.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/forest.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/fort.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/hills.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/mountains.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/prairie.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/sky.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/volcano.rs (Arid exhaustive dispatch only)
    - crates/hex_map/src/procedural_v3/waterfall.rs (Arid exhaustive dispatch only)
    - docs/planning/waves/desert-biomes/manifest.md (L1 queue row only)
  dispatch_blockers: []
  merge_blockers: ["Crystal Mountain @ 74deb7f"]
  fences:
    - path: crates/hex_map/src/procedural_v3/fingerprint.rs
      disposition: keep
      reason: legacy TwoRings and existing V3 fingerprints remain byte-identical
  selector:
    concerns: [map_unit, map_generation, map_contracts, clippy, docs, shipping]
    full: false
  evidence: logic-only
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
- id: L2
  title: Selectable content, palm asset, and Sandbox presentation
  order: ""
  ticket: null
  authority: world
  builder: "@codex"
  branch: wave/desert-biomes
  owns:
    - assets/art/object_catalog.ron (date-palm entry only)
    - assets/art/objects/plant/date-palm.ron
    - assets/config/worlds/procedural-desert-transition.ron
    - assets/config/worlds/procedural-desert-plain.ron
    - assets/config/worlds/procedural-dunes.ron
    - assets/config/worlds/procedural-desert-oasis-rings.ron
    - assets/config/scenarios.ron (four desert entries only)
    - assets/config/sandbox_maps.ron (four desert entries only)
    - assets/ui/sandbox/desert-transition.png
    - assets/ui/sandbox/desert-plain.png
    - assets/ui/sandbox/dunes.png
    - assets/ui/sandbox/desert-oasis-rings.png
    - crates/hex_assets/src/object_catalog.rs (date-palm catalog assertions only)
    - crates/hex_assets/src/sandbox.rs (desert catalog assertions only)
    - crates/hex_assets/src/scenario.rs (desert scenario count/assertions only)
    - crates/hex_game/src/save.rs (desert dependency inputs and digest assertion only)
    - crates/hex_game/src/scenarios.rs (desert embedded worlds and assertions only)
    - crates/hex_game/testdata/example_resume_elemental_grid.ron (dependency digest only)
    - crates/hex_ui/src/lib.rs (shipping scenario count only)
    - docs/planning/waves/desert-biomes/manifest.md (L2 queue row only)
  dispatch_blockers: []
  merge_blockers: [L1]
  fences:
    - path: crates/hex_game/testdata/example_resume_elemental_grid.ron
      disposition: keep
      reason: the shipped-content dependency digest remains fail-closed
  selector:
    concerns: [selector, rules, trajectory_contracts, contracts, simulation, app, map_unit, map_generation, map_contracts, residual, clippy, docs, shipping]
    full: true
  evidence: static-presentation
  sizing: { model: inherited, effort: high }
  state: merged-to-wave
  pr: null
- id: L3
  title: Acceptance routes, documentation, and candidate review
  order: ""
  ticket: null
  authority: world
  builder: "@codex"
  branch: wave/desert-biomes
  owns:
    - crates/hex_map/src/procedural_v3/ring19.rs (DesertOasis acceptance tests only)
    - crates/hex_game/src/walk.rs (desert review-route regions only)
    - walks/camera_desert_transition.ron
    - walks/camera_desert_plain.ron
    - walks/camera_dunes.ron
    - walks/camera_desert_oasis_rings.ron
    - walks/camera_routes.ron (four desert entries only)
    - docs/design/game.md (Arid section only)
    - docs/design/visual-language.md (Arid additions only)
    - docs/development/config.md (Arid configuration only)
    - docs/development/map-testing.md (Arid acceptance only)
    - docs/planning/roadmap.md (Arid row only)
    - docs/planning/status.md (Arid status only)
    - docs/systems/asset-workshop.md (date-palm note only)
    - docs/systems/creator-and-sandbox.md (desert catalog note only)
    - docs/systems/world-generation-v3.md (Arid and DesertOasis sections only)
    - docs/planning/waves/desert-biomes/manifest.md (L3 queue row and acceptance ledger)
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

`@codex` records coordinator-built retrospective lanes, so no missing worker order or lane
PR is implied. The common branch is existing territory, not evidence of a lane merge.

## Ownership map

- L1 owns all world behavior: tags, validation, exact terrain, vegetation placement,
  composition, fingerprints, and deterministic typed tests colocated with each recipe.
- L2 owns the concrete content graph and additive loader/catalog wiring. Its shared-crate
  edits may expose world content but may not define generation behavior.
- L3 owns only the named Ring19 acceptance-test region, four visual routes, review registry,
  and Arid documentation. It may not infer logical acceptance from captures.
- `ring19.rs` is the sole code hotspot: L1 owns production behavior; L3 owns only the
  DesertOasis test functions. L3 refreshes after L1 and runs those tests on the composed
  source.
- `manifest.md` is the expected overlap. The coordinator reconciles all three rows and owns
  final integration fixes; a fix that changes world behavior returns to L1's contract.

## Territory

- The creation sweep measured 41 modified tracked files (`+2376/-176`) and 19 new files
  before this manifest. Every one is assigned above.
- The branch is stacked exactly on Crystal Mountain `74deb7f`; that prerequisite remains a
  draft and the desert wave must not land independently on `dev`.
- `origin/dev @ fc55bd5` contains no `Arid`, `DesertTransition`, `DesertPlain`, or
  `DesertOasis` implementation. The branch-name sweep found no competing desert branch.
- Existing older biome waves and Crystal branches are read-only territory. Untracked
  `.context/` captures and run data are scratch, not part of any lane or delivery.

## Integration order and current state

1. Reconcile or land Crystal Mountain exact head `74deb7f` without rewriting its contracts.
2. Implementation commit `7d904794` composes L1's generation contract, L2's serialized
   content, and L3's tests/routes directly on the wave. No source-lane merge or PR is claimed.
3. The composed selector plan is full; run every selected concern on that exact head, then
   run the one final candidate gate required by the full-selecting paths.
4. Publish one wave PR to `dev`, retain draft state until named human presentation/play
   approval, and never merge this wave directly to `main`.

## Combined acceptance

| Claim | Required evidence | Current evidence |
|---|---|---|
| Four Arid recipes validate bounds, exact materials, anchors, fallback, and determinism | Typed recipe tests | Focused desert filter: 13 passing; Dunes: 5 passing; Oasis: 8 passing; full `cargo test -p hex_map` passed |
| DesertOasis keeps 19 regions, 9,241 columns, 42 Dry seams, 12 central palms, seven aliases, connectivity, and repeated fingerprint/metrics | Typed Ring19 hero-seed test | Ring19 module: 11 passing, one release benchmark ignored; full `hex_map` test command passed |
| Legacy `TwoRings` and existing tags/fingerprints remain unchanged | Fingerprint regression tests on composed head | Full `hex_map` unit, generation, and contract profiles pass, including the legacy fingerprint fixtures |
| Palm catalog, blocking root, four scenarios, four Sandbox cards, embedded worlds, and save dependency digest compose | Asset/app/shipping tests plus release build | Asset/configuration contracts, 169 application tests, 11 application postflight tests, and the optimized shipping build pass |
| Regeneration, teardown, return-to-title, and gameplay re-entry leave no stale map/object state | Headless lifecycle hooks | Expanded 15-scenario lifecycle test and the selected application/residual lifecycle coverage pass |
| Full selector-chosen CI-equivalent candidate is green | Committed-head selector plan and every selected command | Full plan passes: selector 60/60; rules 180/180; trajectory contracts 93/93; contracts 420/420 plus 5 spell-resolution checks; simulation 29/29; app 169/169 plus 11 postflight; map unit 113/113; map generation 493/493; map contracts 95/95; residual 1,047/1,047; workspace doctests; formatting; dependency policy; panic-free clippy; documentation; and optimized shipping build |
| Presentation is legible and camera movement feels correct | Scripted static frames plus named human play review | All four real walks passed exact pointer movement and arrival proofs and produced 22 frames plus four card previews; human taste/play review pending |

The review matrix is:

- Desert Transition: six frames covering the map bands, grass front/reverse, ecotone, and
  sand front/reverse.
- Desert Plain: four frames covering the map relief and front/side/rear overlook.
- Dunes: five frames covering the ridge field, crest front/side, and trough front/ridge wall.
- Desert Oasis Rings: seven frames covering the oasis overview, palms, inner dune in both
  directions, outer dune, and open plain.

The automated candidate and scripted walks are complete. They do not substitute for visual
taste, native camera/control feel, or a named-human `PASS`; the wave remains review-ready
rather than delivered until that review and publication to `dev` occur.

## Stop conditions

- Crystal Mountain does not land at or reconcile cleanly from exact head `74deb7f`.
- Any legacy settings fingerprint or existing numeric tag changes.
- Any DesertOasis seam becomes wet, local oasis water escapes its patch, or ordinary Ring19
  connectivity/redundancy regresses.
- Any selected composed concern is red, or a full gate is described as passed from focused
  tests.
- A presentation-changing wave PR lacks an exact-head named-human `PASS`.
- Island work overlaps this candidate before the desert wave is committed and reviewable.

## Injection log

- 2026-08-21: retroactive wave artifact added for the already composed desert working tree;
  no behavior or acceptance state changed by this documentation step.
- 2026-08-21: the full selector-chosen CI-equivalent gate, doctests, lint/policy/docs gates,
  optimized shipping build, four pointer-driven walks, and 22-frame review matrix passed;
  named-human presentation/play approval and delivery remain intentionally open.

## Close-out

After the wave lands on `dev`, record the wave PR and exact merge SHA, confirm post-merge
checks, change status to `closed`, and remove any transient lane material. No source branch
or ticket deletion is authorized by this integrating manifest.
