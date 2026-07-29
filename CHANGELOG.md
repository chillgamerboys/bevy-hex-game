# Changelog

Release sections are prepended here and dated when their promoted `main` commit is
tagged. The repository has no earlier release tags; `v0.3.0` is planned as its first
tagged build.

## v0.3.0 — Unreleased

### Features

- Make lattice combat playable end to end: cast from live geometry, choose damage,
  disable cells, break enchantments, and retain downed units for restoration.
- Add Ember Burn on each affected unit turn and full, expiring Scrying Eye knowledge.
- Add knowledge-safe lattice panels, stable initiative, retained hostile focus, damage
  cues, a bounded combat log, and an `H` HUD toggle.
- Make defender choices command-modal, with player confirmation and deterministic
  non-player ownership.
- Categorize all shipped scenarios into scrollable Map, Combat, and Demo menu columns,
  with Close Quarters as the combat showcase and Lattice Demo as the rules sandbox.
- Export serde-capable combat outcomes and refusals using stable replay vocabulary.
- Integrate the canonical runtime art palette and the Asset Workshop recovery and
  deterministic review workflow from `dev`.

### Fixes

- Preserve effectless defensive enchantments and reject ambiguous spells that would
  open more than one defender choice.
- Tick Burn once per actual `Turn`, including same-round downing handoffs.
- Permit only single-cardinality unit-effect shapes until area application is built.
- Keep divined mana and disabled state live without extending knowledge expiry.
- Refuse further damage against downed units before payment while allowing Reveal to
  inspect their retained lattices.
- Prevent `SPACE` from skipping hostile turns and expire unfocused damage pulses on
  schedule.
