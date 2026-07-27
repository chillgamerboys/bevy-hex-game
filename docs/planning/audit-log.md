# Audit log

Wave entries appended by `/audit-diff` — one per audited PR, the
durable trail of which lenses fired and what was fixed or deferred.
The receipt at `/tmp/audit-pr-receipt-<N>.json` is ephemeral; this
file is the record that travels with the repo.

<!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->

## Wave 6 — feat(map): add procedural V2 Hills parity (2026-07-27)

- **PR**: #58 — `feat/procedural-v2-hills`
- **Outcome**: green — 0 ship-blockers, 4 non-blockers, all left with the crate owner
- **Lenses triggered**: 2, 7, plus the fresh-eyes pass

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 2 | `crates/hex_map/src/procedural_v2/hills.rs`:331 | NON-BLOCKER | deferred to owner — `covered_by_solid` surface rule and `element_levels` duplicate `volume.rs` logic; drift fails loudly (`InvalidVolume` at load), never silently |
| 2 | `crates/hex_map/src/settings.rs`:635 | NON-BLOCKER | deferred to owner — V2 bounds duplicate `HillsSettings::validate`, and `canonical_v1_settings` bypasses V1 parse-time validation, so the V2 copy is the only fence keeping the frozen selector in its tested domain (verified identical today) |
| 7 | `crates/hex_map/src/procedural_v2/hills.rs`:679 | NON-BLOCKER | deferred to owner — `_candidate_diagnostics` binds `notes` without asserting; a notes-pollution regression would pass the parity suite |
| fresh-eyes | `crates/hex_map/src/grid.rs`:182 | NON-BLOCKER | **walk-checklist item** — the three shipped worlds now frame the opening camera from the generated `MapViewHint` instead of `camera.ron`; terrain is byte-identical, the first view is not; only a human can judge it |

**Notes**: the deep walk corrected two characterizations from the initial review
comment: a future substance whose palette role disagrees with `is_solid` fails
**loudly** (`voxelize_plan` re-checks solidity per element → `MaterialContract` →
`GameplaySetupFailure`), and success-path `GenerationReport.notes` stay empty on
the shipped Hills path (V1 discards rejected-candidate notes on non-fallback
success; the `scenarios.rs` relaxation is future-proofing for `run_recipe`
consumers). Silent-failures pass: 0 real candidates. Numbering: Waves 3–5 were
recorded on `wave/1-foundations` and land with the wave merge; this entry takes 6
so the merged log stays monotonic.

## Wave 2 — docs(planning): sequence the roadmap into waves around the V2 work (2026-07-26)

- **PR**: #57 — `docs/roadmap-waves`
- **Outcome**: green
- **Lenses triggered**: D3 (claims vs reality), plus the fresh-eyes pass
- **Path**: doc-only (first run of the docs lenses D1–D4)

| Lens | File:line | Severity | Status |
|---|---|---|---|
| D3 | `docs/planning/roadmap.md`:56 | SHIP-BLOCKER | fixed in `23c3103` — wave 1 claimed every file clear of #56 while its own sim-seams section names `Turn` (app.rs) and `Body` (movement.rs), both #56-held and both on the doc's own avoid-list; resolved by scoping wave 1 precisely (new lines merge cleanly; the two one-line derives trail the gate) |
| D3 | `docs/planning/roadmap.md`:59 | NON-BLOCKER | fixed in `23c3103` — `targeting.rs` attributed to #56; it is contested by our own knobs ticket instead |
| D3 | `docs/planning/roadmap.md`:76 | NON-BLOCKER | fixed in `23c3103` — the held-file list omitted `hex_core/lib.rs` and five other files #56 actually holds |
| fresh-eyes | `docs/planning/roadmap.md`:78 | NON-BLOCKER | fixed in `23c3103` — instructed building against the five-phase `GameplaySetup` before #56 merges the `View` phase; rephrased to age with the gate |

**Notes**: nothing deferred. D1 (seed-tickets parse contract), D2 (links and
fragment anchors), D4 (single source), row↔section coverage, and the four
#56 contract-type claims all verified clean. This audit motivated the
doc-only path itself: five of the eight code lenses had nothing to say
about a roadmap edit, and the four that remained are now the documented
docs lenses — the skill change ships in this same PR.

## Wave 1 — docs: add planning docs and restructure the tree (2026-07-26)

- **PR**: #55 — `docs/planning-seed`
- **Outcome**: green
- **Lenses triggered**: 2, 4, 8, plus the fresh-eyes finalization pass

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 2 | `CLAUDE.md`:241 | NON-BLOCKER | fixed in `2c93d34` — "Known gaps" duplicated `planning/status.md`'s toolchain list; CLAUDE.md now keeps only the constraints an agent must obey |
| 2 | `docs/planning/map-asks.md`:17 | NON-BLOCKER | fixed in `6c1315f` — framed PR #52's contributions as delivered while it was still open; corrected when it merged |
| 4 | `.claude/skills/update-docs/SKILL.md`:76 | NON-BLOCKER | fixed in `2c93d34` — the new index drift check claimed both directions but tested one, and matched the whole README rather than the index table |
| 4 | `docs/README.md`:35 | NON-BLOCKER | fixed in `2c93d34` — credited `/update-docs` with maintaining `status.md`, which it only reads and reports on |
| 8 | `.claude/skills/seed-tickets/SKILL.md`:13 | NON-BLOCKER | fixed in `2c93d34` — precondition still told the reader to create the roadmap this PR added |
| 8 | `docs/planning/map-asks.md`:48 | NON-BLOCKER | fixed in `6c1315f` — `footing_for` sketch promised a conditional default `#[serde(default)]` cannot produce; a literal implementation would have left every substance unwalkable |
| fresh-eyes | `docs/architecture.md`:194 | NON-BLOCKER | fixed in `2c93d34` — shed three sections without leaving a pointer to any of them |

**Notes**: nothing deferred. Content integrity was verified line-by-line against
`origin/dev` — no prose lost, no section silently duplicated, and the merged
troubleshooting doc is strictly richer than each of its three sources. The two
findings dated `6c1315f` were raised against the planning docs before the tree
moved; the rest against the restructure itself. Four cosmetic blank lines at the
`sed` extraction seams were tidied in the same commit. The visual walk does not
apply: the diff is documentation plus four Rust doc comments, with no runtime
surface.

