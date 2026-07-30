# Documentation

The root [README](../README.md) is the front door: a short, human-readable
orientation to the game and its current maturity. This directory owns the detail.
It is separated by *kind*: what the game is meant to be (`design/`), what is
actually built (`systems/`), how to work on it (`development/`), and where the two
diverge (`planning/`).

Every doc below records who it is for and who owns it — or, where no single owner
applies, who keeps it fresh.

**One doc is allowed to be out of date: [planning/status.md](planning/status.md).**
Everything else describes a contract, so a disagreement with the code is a bug in the
doc or the code, not drift.

## Start here

| If you are… | Read |
|---|---|
| Seeing the game for the first time | The root [README](../README.md), then the full [game design](design/game.md) |
| New to the codebase | [development/setup.md](development/setup.md), then [architecture.md](architecture.md) |
| Dividing related work across branches or agents | [development/parallel-development.md](development/parallel-development.md) |
| Taking ownership of the map | [development/onboarding.md](development/onboarding.md) |
| Changing game values without writing Rust | [development/config.md](development/config.md) |
| Deciding what a mechanic should do | [design/game.md](design/game.md) |
| Writing code that touches terrain | [systems/map.md](systems/map.md) and [`crates/hex_map/CLAUDE.md`](../crates/hex_map/CLAUDE.md) |
| Building V3 recipes or composing biome patches | [systems/world-generation-v3.md](systems/world-generation-v3.md) |
| Authoring colours, voxel styles, plants, props, or static effects | [design/visual-language.md](design/visual-language.md), then [systems/asset-workshop.md](systems/asset-workshop.md) |
| Writing code that touches turns or movement | [systems/combat.md](systems/combat.md) |
| Changing enemy decision policy or AI diagnostics | [systems/ai.md](systems/ai.md) |
| Writing code that touches party selection or formations | [systems/party.md](systems/party.md) |
| Writing code that casts a spell or reshapes terrain | [systems/casting.md](systems/casting.md) |
| Wondering who owns a fact that crosses the boundary | [contracts.md](contracts.md) |
| Writing code that reveals terrain or units | [systems/perception.md](systems/perception.md) |
| Looking at a wrong window | [development/troubleshooting.md](development/troubleshooting.md) |
| Producing or extending release artifacts | [development/release.md](development/release.md) |
| Picking up the next piece of work | [planning/roadmap.md](planning/roadmap.md) |
| Reviewing the 2026 foundation-hardening evidence | [planning/foundation-hardening.md](planning/foundation-hardening.md) |

## The index

| Doc | Audience | Purpose | Owner |
|---|---|---|---|
| [architecture.md](architecture.md) | Contributors, agents | Crate graph, ownership, conventions, states, settings, testing philosophy — and the reasoning behind each | Both, jointly |
| [contracts.md](contracts.md) | Both owners | Every fact crossing the world/gameplay boundary, with its status: live, agreed, reserved, or asked | Both, jointly |
| [systems/map.md](systems/map.md) | Anyone touching terrain | The voxel model: columns, runs, surfaces, headroom, and the rules everything else depends on | World owner |
| [systems/world-generation-v3.md](systems/world-generation-v3.md) | Anyone building V3 terrain | Patch and edge contracts, private semantic layers, recipe order, determinism, migration, and removal of V1/V2 | World owner |
| [systems/asset-workshop.md](systems/asset-workshop.md) | Artists and tooling contributors | Voxel-style and object schemas, editing behavior, persistence, review output, and the isolated editor boundary | Both, jointly |
| [systems/combat.md](systems/combat.md) | Anyone touching turns or movement | The turn loop as built: two tempos, what a turn costs, committing a move, what height buys | Gameplay owner |
| [systems/ai.md](systems/ai.md) | Anyone touching enemy decisions | Authorized observations, compact legal choices, deterministic algorithm dispatch, and bounded diagnostics | Gameplay owner |
| [systems/party.md](systems/party.md) | Anyone touching selection or formations | Stable party controls, Group/Solo mode, preset editing, and pre-alpha resume lifetime | Gameplay owner |
| [systems/casting.md](systems/casting.md) | Anyone touching spells or terrain magic | What makes a cast legal, the volume it affects, who decides what the material does, and persistent effects | Gameplay owner |
| [systems/perception.md](systems/perception.md) | Anyone touching sight, fog, AI, or hidden information | Illumination, faction sight, remembered terrain, presentation, and the boundary between them | World owner (gameplay adapters: gameplay owner) |
| [systems/sky.md](systems/sky.md) | Anyone touching presentation | How the sky is drawn, and the four choices in the shader that are not obvious | World owner |
| [design/game.md](design/game.md) | Everyone | The game this is heading toward: lattices, elements, spells, damage, and the questions deliberately left open | The designer; open questions close only on purpose |
| [design/visual-language.md](design/visual-language.md) | Artists, designers, rendering contributors | The canonical art palette, how it grows, and the boundary between colour, material, lighting, and UI | The designer; tooling is shared |
| [development/setup.md](development/setup.md) | New contributors | Prerequisites, running the game, first-run verification, and platform setup | Whoever changes build or platform requirements |
| [development/parallel-development.md](development/parallel-development.md) | Contributors, agents, integrators | Choosing independent, stacked, or wave topology; ownership lanes; combined review and cleanup | Both, jointly |
| [development/onboarding.md](development/onboarding.md) | New map contributors | Map vocabulary, ownership boundaries, and first checks | World owner |
| [development/config.md](development/config.md) | Designers, non-programmers | Changing the game through `assets/config/*.ron` without recompiling | Whoever adds or renames a setting |
| [development/release.md](development/release.md) | Release contributors | Pre-alpha artifact shape, application identity, retained symbols, and reserved future credential boundaries | Whoever changes packaging |
| [development/troubleshooting.md](development/troubleshooting.md) | Everyone | The single list of failure modes, including the ones that log nothing at all | Whoever hits a new one |
| [planning/status.md](planning/status.md) | Everyone | What is built, what is a placeholder, what each placeholder waits on — **the one doc allowed to drift** | Whoever lands a feature; `/update-docs` reports what a diff falsified |
| [planning/roadmap.md](planning/roadmap.md) | Both devs | The epic table `/seed-tickets` turns into Linear tickets, plus the detail behind each | Whoever claims or finishes a row |
| [planning/production-audit.md](planning/production-audit.md) | Both devs | Dated snapshot: the July 2026 production-readiness audit and the architecture it recommends — **frozen; not updated as code moves** | Nobody. It is a record |
| [planning/boundary.md](planning/boundary.md) | Both owners | The open asks in both directions, each with a signature and a fallback if deferred, plus what each side commits to | Whoever adds or retires an ask |
| [planning/audit-log.md](planning/audit-log.md) | Reviewers | The durable trail of `/audit-diff` waves, one per audited PR | `/audit-diff`, automatically |
| [planning/foundation-hardening.md](planning/foundation-hardening.md) | Reviewers and maintainers | July 2026 correctness, scalability, branch, stress, and handoff evidence for the foundation cleanup | Foundation hardening owner |

Outside this directory: the root [`README.md`](../README.md) introduces the game,
[`AGENTS.md`](../AGENTS.md) is Codex's repository instruction surface,
[`CLAUDE.md`](../CLAUDE.md) is the operational summary loaded into Claude sessions,
[`CONTRIBUTING.md`](../CONTRIBUTING.md) is house style and the PR workflow,
and [`crates/hex_map/CLAUDE.md`](../crates/hex_map/CLAUDE.md) is the map crate's own
contract, read automatically when working in that directory.
