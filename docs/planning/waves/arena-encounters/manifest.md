# Arena enemies and authored encounters — local wave

Status: foundation validated; lanes ready. Base: 127d1ce2058de9ba79da9717b7e37df4b9913502. Branch: experiment/spell-combat-arena. Coordinator: root. Ticket: null (Linear reauthentication required).
Outcome: [approved specification](plan.md), grounded in [audited seams](maps/seams.md). World facts, creature authority and presentation form one combined playable outcome. User requires local implementation/commits, excluding remote PR/dev merges. Foundation remains local; contributors edit disjoint authority files in the shared checkout. Root alone commits and serializes builds/captures.

## Locked decisions

1. Preserve accepted Duel and Shadow profile. Fort default Dragon; Seven Regions three independent parties.
2. Literal Dragon body; attack-only transparent barrier; allied actor immunity and projectile pass-through; unoccluded splash and caster self-risk.
3. Party wakes from 12-unit LOS or positive damage using observations only. Species search/return, dead parties stay dead, map-only safe human regeneration.
4. Every attack chips terrain through world authority; physical damage explicit. Water unchanged; required routes dry. Static objects indestructible with conservative protected support.
5. Numeric behavior and acceptance in [plan.md](plan.md) are authoritative.

## Ownership and readiness

Root owns hex_core contracts, mechanical consumer migration, shared contract/delivery docs, wave artifacts and combined review. Domain changes remain separately identifiable commits. Additional paths require explicit coordination.
Foundation first; all three lanes can then start. World/gameplay integrate before final presentation acceptance. Expensive builds and captures are serialized. No source/evidence cleanup.

## Territory

Origin fetched 2026-09-07. Candidate remains based on accepted local 127d1ce. Open PR219 Grand V3 and220 V4 remain outside (474/528-file differences from dev). PR210–213 are unrelated biome branches,196 tactical lattice. Fourteen worktrees audited; no other checkout changed. Reuse intended cargo-target-explore cache; 8 GiB available. No parallel Cargo processes or cache deletion.

## Acceptance and close-out

Typed hooks establish gameplay; fresh windowless captures establish static presentation. Run focused lane checks and selector-chosen combined gate, strict lint and native build. Record performance and known limitations. Human motion/feel and 20–30 encounters remain pending user playtest. End with local committed candidate, launcher and concise guides/report; no automatic remote actions.

## Injection log

- 2026-09-07: user-approved encounter follow-up; three lanes queued behind shared foundation.

## Dispatch queue

```yaml
{
  "lanes": [
    {
      "id": "L1",
      "title": "Authored world and material admission",
      "order": "orders/L1-world.md",
      "ticket": null,
      "authority": "world",
      "builder": "worker",
      "branch": "experiment/spell-combat-arena",
      "owns": [
        "crates/hex_map/",
        "crates/hex_assets/src/terrain_damage.rs",
        "crates/hex_assets/src/content_index.rs",
        "assets/config/terrain_damage.ron"
      ],
      "dispatch_blockers": [
        "foundation-committed"
      ],
      "merge_blockers": [],
      "fences": [],
      "selector": {
        "concerns": [
          "map_unit",
          "map_generation",
          "map_contracts",
          "clippy",
          "docs",
          "shipping"
        ],
        "full": true
      },
      "evidence": "static-presentation",
      "sizing": {
        "model": "inherited",
        "effort": "inherited"
      },
      "state": "queued",
      "pr": null
    },
    {
      "id": "L2",
      "title": "Creature and encounter simulation",
      "order": "orders/L2-gameplay.md",
      "ticket": null,
      "authority": "gameplay",
      "builder": "worker",
      "branch": "experiment/spell-combat-arena",
      "owns": [
        "crates/hex_arena/",
        "assets/config/arena.ron"
      ],
      "dispatch_blockers": [
        "foundation-committed"
      ],
      "merge_blockers": [
        "L1"
      ],
      "fences": [],
      "selector": {
        "concerns": [
          "contracts",
          "simulation",
          "clippy",
          "docs"
        ],
        "full": true
      },
      "evidence": "motion-or-feel",
      "sizing": {
        "model": "inherited",
        "effort": "inherited"
      },
      "state": "queued",
      "pr": null
    },
    {
      "id": "L3",
      "title": "Selection models HUD and launcher",
      "order": "orders/L3-shared.md",
      "ticket": null,
      "authority": "shared",
      "builder": "worker",
      "branch": "experiment/spell-combat-arena",
      "owns": [
        "crates/hex_game/src/arena.rs",
        "crates/hex_game/src/arena/",
        "tools/arena.py"
      ],
      "dispatch_blockers": [
        "foundation-committed"
      ],
      "merge_blockers": [
        "L1",
        "L2"
      ],
      "fences": [],
      "selector": {
        "concerns": [
          "app",
          "clippy",
          "docs",
          "shipping"
        ],
        "full": true
      },
      "evidence": "motion-or-feel",
      "sizing": {
        "model": "inherited",
        "effort": "inherited"
      },
      "state": "queued",
      "pr": null
    }
  ]
}
```

## Checkpoints

- Foundation: existing hex_arena suite and126 core tests passed; bounded sphere and authored vertical-offset regression included. Receipt: task work/encounter-foundation-tests.log. No changes to accepted bot behavior.
