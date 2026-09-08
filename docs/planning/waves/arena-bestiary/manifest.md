# Arena spectator battles and sequential bestiary — local continuation wave

Status: spectator and bounded original-group calibration complete; Golem geometry/combat/presentation dispatched; Wisp and Worm queued.
Coordinator: root. Branch: experiment/spell-combat-arena.
Accepted local base: 127d1ce2058de9ba79da9717b7e37df4b9913502. Parent encounter
checkpoint: 30467790c6ae0c6ac5839b57f70bef3d209d5620. Epic/tickets: null;
previous Linear read required reauthentication. User approved this continuation and
corrections on 2026-09-07. Outcome: playable original enemies and real maps, an
observer mode for opposing creature groups, and three sequentially calibrated additions.
[Approved details and hypotheses](plan.md) distinguish locked behavior from tuning.

## Why this wave exists

World deployment, generalized gameplay targets and observer presentation share one
runtime checkpoint. Shared vocabulary lands first. Golem/Wisp/Worm phases form a
sequence inside this same local candidate; each phase reuses these ownership lanes.
The user explicitly overrides normal remote/dev landing and branch isolation: one
local branch with disjoint shared-checkout edits and coordinator-only commits.

## Locked decisions

1. Preserve accepted human controls, movement, spells and Shadow policy; Goblins match the player body.
2. Spectator has two autonomous monster teams and no dummy human; observations alone feed decisions, and arbitrary stable team IDs retain their meaning.
3. World publishes finite deployment surfaces; gameplay admits the entire roster atomically against actual body shapes, dry support and separation.
4. Finish and roughly calibrate original groups, then add Golem, Ember Wisp and Worm in that order; no Boar.
5. Golem uses a round seven-hex base and five levels of height, slow movement, spherical short slam and long charged laser with a medium-range gap.
6. Wisp is one voxel, slow flying, conspicuously glowing and long ranged; Worm burrows, converts eligible earth to dirt, and must expose its head before boulders.
7. Everything remains local on experiment/spell-combat-arena; no PR, merge, visible launch, multiplayer, adaptive difficulty or unrelated visual work.

## Shared foundation

Root commits gameplay-owned ArenaBattleSetup, ArenaControl, roster recipes, typed
setup refusal, terminal result and summary contracts. Requested setup is accepted
only at reset against published world.selection. Runtime retains that accepted
snapshot. ArenaOutcome keeps ordinary player semantics; optional human ID and
is_finished/battle_summary drive observer consumers.

Root adds world-published ArenaDeploymentRegion in hex_core and an optional two-side
projection in ArenaTerrainView. Surfaces identify supporting voxels, never guaranteed
spawn poses. World lane publishes Duel/Fort sets without changing adventure spawns;
gameplay validates oriented bodies and fails atomically if a roster cannot fit.
Worm mutation vocabulary will receive a separate behavior-neutral foundation before
that phase; no world mutation is implemented in presentation or private gameplay.

## Dispatch queue

```yaml
{
  "lanes": [
    {
      "id": "L1",
      "title": "World deployment and later burrow publication",
      "order": "orders/L1-world.md",
      "ticket": null,
      "authority": "world",
      "builder": "worker",
      "branch": "experiment/spell-combat-arena",
      "owns": [
        "crates/hex_map/",
        "crates/hex_assets/src/terrain_damage.rs",
        "assets/config/terrain_damage.ron"
      ],
      "dispatch_blockers": [
        "foundation-committed",
        "original-focused-milestone-recorded"
      ],
      "merge_blockers": [],
      "fences": [],
      "selector": {
        "concerns": [
          "contracts",
          "simulation",
          "app",
          "map_contracts",
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
      "state": "dispatched",
      "pr": null
    },
    {
      "id": "L2",
      "title": "Monster teams and sequential creature authority",
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
        "foundation-committed",
        "original-focused-milestone-recorded"
      ],
      "merge_blockers": [
        "L1"
      ],
      "fences": [],
      "selector": {
        "concerns": [
          "contracts",
          "simulation",
          "app",
          "map_contracts",
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
      "state": "dispatched",
      "pr": null
    },
    {
      "id": "L3",
      "title": "Observer controls creature effects and launcher",
      "order": "orders/L3-presentation.md",
      "ticket": null,
      "authority": "shared",
      "builder": "worker",
      "branch": "experiment/spell-combat-arena",
      "owns": [
        "crates/hex_game/src/arena/",
        "tools/arena.py"
      ],
      "dispatch_blockers": [
        "foundation-committed",
        "original-focused-milestone-recorded"
      ],
      "merge_blockers": [
        "L1",
        "L2"
      ],
      "fences": [],
      "selector": {
        "concerns": [
          "contracts",
          "simulation",
          "app",
          "map_contracts",
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
      "state": "dispatched",
      "pr": null
    }
  ]
}
```

## Ownership map

L1 owns the world paths above; L2 owns hex_arena and arena.ron; L3 owns arena app
presentation and launcher. Root owns hex_core contracts, combined integration and
battle targets under hex_game/tests, shared Cargo target declarations and delivery
docs. Tests may be delegated explicitly without expanding production authority.
No contributor stages/commits. Hot-file changes require coordinator agreement.
The original stress-fixture follow-up temporarily owns prepare_stress_tick and its
fixture in hex_game/tests/arena_encounters.rs; it completes before L3 resumes.

## Territory

This continues the already audited encounter branch. Origin was fetched on2026-09-07;
Grand V3 PR219 (474-file diff) and V4 PR220 (528-file diff), biome PR210–213 and tactical
lattice PR196 remain outside. Fourteen worktrees were audited and left untouched.
No source merging from those branches is planned. A new fetch is unnecessary for
this isolated continuation with no dev/remote landing; refresh before any later merge.

## Integration order

Original focused milestone → shared foundation → L1/L2/L3 spectator work in disjoint
lanes → root combined tests and calibration → Golem → Wisp → Worm → final combined
checks. L1/L2 can implement against committed types together; consumers accept only
published facts. Native capture and performance runs lock source and serialize Cargo.

## Combined acceptance

Duel goldens preserve accepted behavior. Typed tests cover actor0 monsters, arbitrary
teams, observer immunity, target changes, identical hidden histories, allies, support,
whole-team wins/draws/timeouts, physical deployment failure, reset and mode switching.
Actual ArenaTick/world publication drives paired seeded battles with fresh terrain,
ordinary health/brains, both side/initiative orders, timeouts explicit and holdout seeds.
Each new species gets body/attack/cooldown/death/terrain and counter-matchup checks.
All ten Seven Regions enemies receive bounded measured stress; final captures inspect
models, telegraphs, translucency, HUD and unchanged opaque occlusion. Full-resolution
independent notes and contact sheets establish static presentation only. Human controls,
camera motion, animation, feel and20–30 encounters remain HUMAN-MOTION-PENDING.

Run repository-selected full closure (rules, contracts, simulation, app, map unit,
generation, publication, residual and docs tests), trajectory consumers, dependency and
link checks, fmt, strict workspace lint, docs and shipping build; also native arena
build and focused arena integration/routes/battles. Record timings and repeatable stalls.

## Stop conditions and close-out

Escalate unresolved ownership contracts, source changes during captures, hidden-info
leaks, unsafe spawns or failed required checks. Do not label a timeout a win, a frame a
logic proof, or machine balance a human win rate. Keep local commits and all evidence;
no cleanup, visible launch or remote publication. Delete overnight heartbeat only when
all authorized work and final evidence/guide are delivered.

## Injection log

- 2026-09-07: user approved spectator/calibration and sequential Golem/Wisp/Worm work;
  clarified seven-hex body belongs to Golem and Goblins must remain player sized.

## Checkpoints

- Foundation f12b329 committed and independently reviewed;122 gameplay tests and strict arena lint pass. Original focused visual/native performance milestone recorded in [validation](../../../systems/arena-encounters-validation.md). L1/L2/L3 spectator work dispatched; later creature phases remain queued behind original matchup calibration.
- Spectator candidate90e2e4d combines L1 world deployment56db778, L2 runtime469cac0,
  L3 observer6881eb2 and the actual battle harness. Focused world/gameplay/application
  checks and native build pass. The initial12-round pilot exposed movement/sighting
  defects before stat tuning. Repairs and closer observer framing are active;
  Golem/Wisp/Worm remain queued. See the [bestiary validation record](../../../systems/arena-bestiary-validation.md).

- Original tuning checkpoint `27157af` passes 160 arena tests and scoped strict lint.
  Native 120-round calibration, 30-round fresh holdout and 36-round Fort spot corpus
  are complete with timeouts and map effects retained. Final Dragon hypotheses are
  HP220 / breath45 / range6. See the validation record for results and remaining
  Shaman stalemates. Golem body/beam projection vocabulary landed in `d5494a6`;
  seven-prism geometry and Golem production lanes now follow the committed seam.

- Guarded Golem foundation `ddb060d` passes 163 gameplay tests and strict scoped
  lint. Species/ability indices, fixed seven-prism projection, tuning and reset-owned
  Fort player recipes are shared. Golem admission stays refused until combined body
  and attack authority is safe. L2 temporarily delegates exact shape kernels,
  projectile body dispatch, separation and mouth geometry to the geometry worker
  within gameplay authority; L2 retains brain/abilities/motion. L3 integrates
  presentation/selector/launcher and capture recipes. Root owns real-world battle
  deployment/reset tests and combined commits. No world API changes in this phase.

- Golem runtime `2570dce` passes 199 gameplay tests and strict scoped lint. Exact
  compound body/mouth queries, slow motion, spherical slam, locked long laser and
  Fort recipe admission are enabled. Geometry/combat review repaired translated
  shared-edge, point-blank beam and mouth-parallax cases. App/real-world/native
  integration and first Golem matchup calibration remain pending.
