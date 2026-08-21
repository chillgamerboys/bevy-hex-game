# Island biomes wave

- Status: `review-ready` (exact implementation head and automated gate complete;
  named-human visual/play review and delivery to `dev` pending)
- Wave branch: `wave/island-biomes`
- Exact implementation head: `6f78a9cae681d9675e67daf6bd98b20e4a058475`
- Validation close-out head: `45b0704c409474abf86c273b70fef3cac981751d`
- Base `origin/dev`: `fc55bd5a1c3c0181b6506d5ac59e1189d287838a`
- Required stacked dependency: Desert biomes exact review-ready head
  `441c22cc6968478993e920a1a575fa086edc05ee`
- Coordinator: Codex / world integration
- Epic: Coastal islands and Ocean Archipelagoes; no Linear ticket is assigned
- Shippable outcome: two selectable focused Coastal island maps and one selectable
  radius-77 ocean Macro composition, with deterministic typed contracts and a visual review
  pack.
- Exclusions: swimming, boats, bridges between remote islets, combat populations, tides,
  waves with gameplay authority, underwater traversal, dynamic shorelines, and changes to
  the accepted Desert or Crystal Mountain candidates.

## Why this wave exists

Small islands, one larger wooded island, and the final archipelago share exact shoreline,
ocean, island-footprint, vegetation, fingerprint, catalog, and camera-review contracts.
The focused recipes must be independently useful, but the final value comes from proving
that they compose into one continuous ocean without pretending disconnected scenic islands
are ordinarily walkable. That makes this one candidate with four ownership lanes.

The four lanes are composed at the exact implementation head above. `merged-to-wave` below
means their implementation has been reconciled there; it does not claim a source-lane PR,
named-human presentation approval, or delivery to `dev`.

## Locked decisions

1. **I1:** "Reuse the existing `Coastal` environment. `SandyIslets` and `WoodedIsland`
   append recipe tags 21 and 22; no existing environment, recipe, settings, or material
   fingerprint may change."
2. **I2:** "Sandy Islets is a radius-24 focused map with five separated sandy land
   components and one playable primary component. Wooded Island is a radius-40 focused map
   with one broad island, a two-column sand fringe, grass-and-soil interior, existing
   broadleaf trees, and one complete ordinary route."
3. **I3:** "Ocean Archipelagoes is a radius-77 Macro world with one connected level-8
   ocean, three scenic two-cell islet clusters, and one connected landing/heart pair that
   owns the playable ordinary route. Remote islands remain intentionally unreachable until
   a future swimming or boat mechanic exists."
4. **I4:** "The final Macro uses exactly 37 atomic cells: 24 open-sea cells, three two-cell
   sandy clusters, one sandy landing cell, and a six-cell wooded heart. It declares ten
   exact Standing-water seams; the landing-to-heart coast excludes exactly one four-lane
   walker causeway, and every other dry component is scenic."
5. **I5:** "Seed changes shoreline detail, relief, and vegetation placement but never map
   radius, sea level, requested island count, land-component count, route ownership,
   terminal identity, or Macro cell roster."
6. **I6:** "Map captures prove presentation only. Typed hooks own ocean connectivity,
   water levels, island counts and coverage, strata, ordinary reachability, blockers,
   seams, fingerprints, lifecycle, and deterministic regeneration."

## Shared foundation

- **World authority:** V3 `Single` and `Macro`, exact `V3Volume` runs, `LiquidPlan`,
  `OrdinaryGraph`, `FeaturePlan`, resolved ports, and conditional fingerprint extensions.
- **World authority:** existing `Coastal`, Shallow Sea, Beach, Shore, broadleaf vegetation,
  and palm assets remain the complete material and presentation vocabulary.
- **World authority change / L1:** append the two recipe settings and fingerprint tags,
  admit them only in supported Single/Macro contexts, and extend exhaustive dispatch and
  Coastal adjacency without changing legacy fingerprints.
- **World authority change / L3:** exact Ocean Archipelagoes validation distinguishes the
  one playable dry network from intentional scenic land components while retaining one
  continuous ocean. This is profile-local and does not relax generic Macro worlds.
- **Coordinator-only resource:** visual walks, native game runs, display access, build-cache
  cleanup, final selector planning, and combined acceptance are serialized by the
  coordinator.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Coastal recipe contracts and append-only fingerprints
    order: orders/L1-island-contracts.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/island-contracts
    owns:
      - crates/hex_map/src/settings.rs (island recipe settings, validation, and adjacency only)
      - crates/hex_map/src/procedural_v3/fingerprint.rs (tags 21/22 and island tests only)
      - crates/hex_map/src/procedural_v3/mod.rs (island module declarations and dispatch only)
      - crates/hex_map/src/procedural_v3/composite_patch.rs (island exhaustive dispatch only)
      - crates/hex_map/src/procedural_v3/layout.rs (island exhaustive dispatch only)
      - crates/hex_map/src/procedural_v3/ring7.rs (island rejection/exhaustive arms only)
      - crates/hex_map/src/procedural_v3/ring19.rs (island rejection/exhaustive arms only)
      - crates/hex_map/src/procedural_v3/caves.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/deep_forest.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/forest.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/fort.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/hills.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/mountains.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/prairie.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/sky.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/volcano.rs (island exhaustive environment arms only)
      - crates/hex_map/src/procedural_v3/waterfall.rs (island exhaustive environment arms only)
      - docs/planning/waves/island-biomes/manifest.md (L1 queue row only)
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector:
      concerns: [map_unit, map_generation, clippy, docs]
      full: true
    evidence: logic-only
    sizing: {model: inherited, effort: high}
    state: merged-to-wave
    pr: null

  - id: L2
    title: Deterministic Sandy Islets and Wooded Island generators
    order: orders/L2-island-recipes.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/island-recipes
    owns:
      - crates/hex_map/src/procedural_v3/coastal_island.rs
      - crates/hex_map/src/procedural_v3/sandy_islets.rs
      - crates/hex_map/src/procedural_v3/wooded_island.rs
      - crates/hex_map/src/procedural_v3/vegetation.rs (island helper region only)
      - docs/planning/waves/island-biomes/manifest.md (L2 queue row only)
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector:
      concerns: [map_unit, map_generation, clippy]
      full: true
    evidence: static-presentation
    sizing: {model: inherited, effort: high}
    state: merged-to-wave
    pr: null

  - id: L3
    title: Radius-77 Ocean Archipelagoes composition
    order: orders/L3-archipelago-composition.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/ocean-archipelagoes
    owns:
      - crates/hex_map/src/procedural_v3/macro_world.rs (Ocean Archipelagoes profile only)
      - crates/hex_map/src/procedural_v3/composition.rs (ocean liquid-union tests only)
      - crates/hex_map/src/procedural.rs (focused-island and Ocean Archipelagoes report metrics only)
      - crates/hex_map/src/lib.rs (island report exports only)
      - crates/hex_map/src/grid.rs (island report reflection registration only)
      - crates/hex_map/tests/contracts/composed_worlds.rs (Ocean Archipelagoes tests only)
      - assets/config/worlds/procedural-ocean-archipelagoes.ron
      - docs/planning/waves/island-biomes/manifest.md (L3 queue row only)
    dispatch_blockers: []
    merge_blockers: [L1, L2]
    fences: []
    selector:
      concerns: [map_generation, map_contracts, clippy]
      full: true
    evidence: static-presentation
    sizing: {model: inherited, effort: high}
    state: merged-to-wave
    pr: null

  - id: L4
    title: Selectable content, lifecycle, walks, documentation, and review pack
    order: orders/L4-island-shipping-review.md
    ticket: null
    authority: world
    builder: worker
    branch: lane/island-shipping
    owns:
      - assets/config/worlds/procedural-sandy-islets.ron
      - assets/config/worlds/procedural-wooded-island.ron
      - assets/config/encounters/island-showcase.ron
      - assets/config/scenarios.ron (three island entries only)
      - assets/config/sandbox_maps.ron (three island entries only)
      - assets/ui/sandbox/sandy-islets.png
      - assets/ui/sandbox/wooded-island.png
      - assets/ui/sandbox/ocean-archipelagoes.png
      - crates/hex_assets/src/sandbox.rs (island catalog assertions only)
      - crates/hex_assets/src/scenario.rs (island scenario assertions only)
      - crates/hex_game/src/save.rs (island dependency digest only)
      - crates/hex_game/src/scenarios.rs (island worlds and lifecycle only)
      - crates/hex_game/src/walk.rs (island routes only)
      - crates/hex_ui/src/lib.rs (island Sandbox count/order only)
      - walks/camera_sandy_islets.ron
      - walks/camera_wooded_island.ron
      - walks/camera_ocean_archipelagoes.ron
      - walks/camera_routes.ron (three island entries only)
      - docs/design/game.md (island section only)
      - docs/design/visual-language.md (island section only)
      - docs/development/config.md (island configuration only)
      - docs/development/map-testing.md (island acceptance only)
      - docs/planning/roadmap.md (island row only)
      - docs/planning/status.md (island status only)
      - docs/systems/creator-and-sandbox.md (island catalog note only)
      - docs/systems/world-generation-v3.md (island contracts only)
      - docs/planning/waves/island-biomes/manifest.md (L4 queue row and acceptance ledger)
    dispatch_blockers: []
    merge_blockers: [L1, L2, L3]
    fences: []
    selector:
      concerns: [app, map_contracts, residual, clippy, docs, shipping]
      full: true
    evidence: motion-or-feel
    sizing: {model: inherited, effort: high}
    state: merged-to-wave
    pr: null
```

## Ownership map

- L1 owns serialized contracts, tags, validation, and exhaustive compiler seams; it does
  not author island geometry.
- L2 owns focused island geometry, water/strata planning, anchors, vegetation placement,
  and its direct tests; it consumes but does not redefine L1 settings.
- L3 owns the final Macro cell roster, sea union, dry route, profile validator, metrics,
  and composed-world tests; it consumes L1/L2 APIs without editing their files.
- L4 owns selectable content and presentation integration. It adds no generation policy
  and cannot infer logical acceptance from captures.
- `manifest.md` is the expected shared hotspot. The coordinator, not parallel workers,
  reconciles queue states in this shared-filesystem execution environment.
- `settings.rs`, `procedural_v3/mod.rs`, and the public metrics files are explicit regional
  hotspots. L1 lands first; L2/L3 refresh against it, and the coordinator verifies every
  exhaustive match after composition.

## Territory

- `origin/dev @ fc55bd5` has Coastal Shallow Sea, Beach, and Shore but no island recipe or
  archipelago scenario.
- Crystal Mountain draft PR #210 and Desert exact head `441c22c` are stacked dependencies;
  this wave must not rewrite their contracts.
- The pre-dispatch branch/PR and working-tree sweep found no competing island implementation
  and no teammate-owned edit in the paths above.
- Generated build directories are outside version control and remain coordinator-owned
  machine resources.

## Integration order

1. Commit this manifest and its four orders before implementation dispatch.
2. Run L1 and L2 in parallel on disjoint regions; verify L2 against the landed L1 contract.
3. Compose L3 after the focused recipes exist, retaining profile-local scenic-component
   policy rather than relaxing generic Macro validation.
4. Add L4 selectable content and typed lifecycle hooks, then derive exact walk waypoints
   from production traversal and run all visual work serially.
5. Re-plan the selector against the final stacked base, run every selected concern on the
   exact candidate head, and prepare a visual report for named-human review.

## Combined acceptance

- Settings boundaries, strict RON, tags 21/22, field sensitivity, and every legacy
  fingerprint are typed and deterministic.
- Sandy Islets proves exact radius, sea level, five separated land components, requested
  land coverage, primary-component anchors/reachability, two-column beaches, strata,
  boundary ocean, no submerged feature roots, and representative-seed determinism.
- Wooded Island proves one broad land component, shoreline fringe, relief bound, protected
  dry route, broadleaf density, unblocked anchors, boundary ocean, and determinism.
- Ocean Archipelagoes proves all 18,019 columns, the exact 37-cell/six-region roster, ten
  Standing-water seams, one connected ocean, seven intentional dry components, one
  connected landing-to-heart ordinary route, scenic satellite isolation, stable aliases,
  unique ids, lifecycle, teardown, and deterministic re-entry.
- Existing movement, terrain occupancy, fog, first-/third-person cameras, and map picking
  remain authoritative; no hidden teleport, swim, or boat path is added.
- Static review covers Map, Third Person, and First Person views of sandy channels, the
  wooded beach/interior/ridge, the whole archipelago, its primary channel, and remote
  silhouettes. Pointer-driven routes use Click → Settle(5) → AwaitPartyIdle → exact arrival.
- The full selector-chosen CI-equivalent gate, formatting, dependency policy, doctests,
  clippy, docs, optimized shipping build, and named-human presentation/play classification
  are required before delivery.

## Evidence ledger

| Acceptance slice | Current evidence | Remaining work |
|---|---|---|
| Focused island contracts and geometry | The final targeted island slice passes all 16 tests, including settings, fingerprints, exact component counts, coastline/strata, protected routes, deterministic construction, and the exact compact roster. | Complete; named-human play classification remains. |
| Ocean Archipelagoes composition | At exact head `6f78a9c`, canonical radius-77 construction, exact aliases, and alias-retarget mutation pass. The report covers 18,019 columns, the 37-cell roster, one ocean, seven dry components, ten wet seams, and the four-wide playable causeway; the complete 119/506/97 map unit/generation/contract gate is green. | Complete; named-human visual classification remains. |
| Runtime lifecycle | Final scenario parsing, dependency digest, island generation, teardown, and deterministic gameplay re-entry pass with the compact encounter included. | Complete. |
| Selectable presentation | All three current visual-walk scripts completed and produced 17 real frames: four Sandy Islets, six Wooded Island, and seven Ocean Archipelagoes captures. The three shipped Sandbox previews are real 640x360 renders derived from this review work. | Named-human visual/play classification remains required. |
| Party formation | `assets/config/encounters/island-showcase.ron` keeps the standard three-character noncombat party but uses compact formation spread `1`, allowing ordinary movement through the authored four-wide coastal routes. Parsing, scenario wiring, save dependency digest, and exact-route assertions pass at the final head. | Complete. |
| Candidate closure | Validation head `45b0704` passes the exhaustive/disjoint 119/506/97 map partition and all selected concerns: 60 selector, 180 rules, 93 trajectory, 425 contract/postflight, 29 simulation, 180 application/postflight, 722 map, and 1,050 residual tests. Doctests, format/diff, strict clippy, dependency policy, generated docs, tracked Markdown links, optimized shipping, and the shipped large-map camera benchmark are green; Ocean Archipelagoes measures 2.583 µs query p95 against 2 ms. | Named-human presentation/play approval and delivery to `dev` remain. |

The local ignored handoff index is `.context/waves/island-biomes-review.md`; it and the
captures do not ship in the source branch. Captures are presentation evidence only; the
typed tests remain authoritative for topology, connectivity, determinism, and lifecycle.

## Stop conditions

- Tags 21/22 conflict with another appended recipe or any legacy fingerprint changes.
- One lane crosses the world/gameplay authority boundary or edits another lane's unlisted
  region.
- A focused island fabricates reachability between separated land components.
- Ocean seams do not union into one body, scenic satellites enter the critical ordinary
  route, or generic Macro connectivity is weakened to admit the profile.
- Trees block a required anchor, route, or seam; water leaves a floating or submerged
  structural root; any boundary column ceases to be ocean.
- A presentation-changing exact head lacks a named-human `PASS`.

## Injection log

- 2026-08-21: initial four-lane island wave recorded from the user's overnight sequence;
  no source work dispatched before the manifest commit.
- 2026-08-21: L3 ownership expanded narrowly to the focused-island public report structs,
  enum variants, exports, and reflection registration. L2 owns private construction metrics,
  while the report boundary remains coordinator-integrated with the Ocean profile; no geometry
  or serialized settings ownership moved.
- 2026-08-21: all four lanes were reconciled into the shared wave tree. Sandy Islets,
  Wooded Island, and Ocean Archipelagoes are selectable; the dedicated compact encounter
  uses formation spread `1`; three successful live walks produced 17 review frames and
  three real 640x360 Sandbox previews.
- 2026-08-21: exact implementation head `6f78a9c` passed the complete selector-chosen
  CI-equivalent source gate, exact parser/digest/lifecycle coverage, doctests, strict clippy,
  formatting, dependency policy, generated documentation, tracked-link validation, and the
  optimized shipping build. Named-human visual/play approval remains deliberately open.
- 2026-08-21: fresh-eyes review found stale map-partition count pins despite every selected
  suite passing. Validation close-out `45b0704` updates them to 119/506/97, proves the
  partition exhaustive and disjoint, and passes the resulting full selector gate plus the
  shipped large-map release camera benchmark. No gameplay or presentation source changed.

## Close-out

After the wave lands on `dev`, record the wave PR and exact merge SHA, confirm post-merge
checks, set status to `closed`, and remove transient order files in a small close-out PR.
No source branch or ticket deletion is authorized before that durable record lands.
