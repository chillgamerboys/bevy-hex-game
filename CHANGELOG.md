# Changelog

Release sections are prepended here and dated when their promoted `main` commit is
tagged. `v0.3.0` is the first tagged build.

## v0.4.0 — 2026-07-30

### Features

- Replace Lattice Demo with separate Character Creator and Spell Creator screens,
  saved draft/ready lifecycles, immutable templates, and local lattice testing.
- Add Combat Lab Sandbox roster composition, terrain-backed exact-surface deployment,
  frozen retry snapshots, and four isolated deterministic fixture families.
- Offer every shipped map through thirteen described renderer previews, including the
  V3 Fort and connected Seven Regions world.
- Replace the Combat scenario lane with Maps, focused Demos, and Actions; New Game now
  launches Party Trial as the hidden integrated default.
- Add one atomic, build-bound exploration resume slot, persistent display/volume
  settings, centralized fixed input actions, and empty audio-bus seams.
- Normalize release artifacts under the Hex Game identity, retain symbol material,
  and document future signing, Steam, and crash-reporting credential boundaries.

## v0.3.0 — 2026-07-28

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
- Add the deterministic V3 Waterfall showcase with directed water stages, an upstream
  bridge, and two independent high-to-low land routes.
- Move the shipped Hills, Frozen, Volcanic, Sky Islands, and Mountains maps to native
  V3 recipes with exact crossing, stacked-volume, route, and environment contracts.
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
