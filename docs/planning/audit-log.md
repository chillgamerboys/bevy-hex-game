# Audit log

Wave entries appended by `/audit-diff` — one per audited PR, the
durable trail of which lenses fired and what was fixed or deferred.
The receipt at `/tmp/audit-pr-receipt-<N>.json` is ephemeral; this
file is the record that travels with the repo.

<!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->

## Wave 10 — feat(editor): add the Asset Workshop (2026-07-28)

- **PR**: #95 — `feat/asset-workshop-editor`
- **Outcome**: green — 4 ship-blockers and 3 non-blockers fixed; 1 path-literal duplication deferred
- **Lenses triggered**: 1, 2, 4, 5, 7, D3, plus the fresh-eyes pass

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 5, 7 | `crates/hex_editor/src/viewport.rs`:503 | SHIP-BLOCKER | fixed in `9c358ac` — style edits now use tracked `Assets::get_mut`; a headless Bevy App regression observes the resulting `AssetEvent::Modified` for the cached material |
| 1, 3 | `crates/hex_editor/src/model.rs`:210 | SHIP-BLOCKER | fixed in `56ad5bb` — unsaved documents use an explicit absent checkpoint, so deleting the only Effect or Prop voxel cannot make new work appear clean and bypass close/document-change protection |
| 1 | `crates/hex_editor/src/app.rs`:696 | SHIP-BLOCKER | fixed in `56ad5bb` — Save As validates and builds a cloned editor before writing, then swaps it into the live draft only after persistence succeeds; rejected writes no longer leave a rename or undo entry behind |
| 4, fresh-eyes | `crates/hex_editor/src/ui.rs`:1968 | SHIP-BLOCKER | fixed in `56ad5bb` — object inspector forms now track the last model values field-by-field, refreshing undo/redo changes without discarding live form input |
| 1 | `crates/hex_editor/src/workshop.rs`:151 | NON-BLOCKER | fixed in `56ad5bb` — history labels are validated before the edit closure can mutate the object |
| 2 | `crates/hex_editor/src/workshop.rs`:13 | NON-BLOCKER | fixed in the Wave 10 follow-up — global and object snapshot histories now share `DEFAULT_HISTORY_LIMIT` |
| D3 | `docs/systems/asset-workshop.md`:157 | NON-BLOCKER | fixed in `56ad5bb` — persistence actions include confirmed Delete, and the document now distinguishes active external-change detection from future recovery drafts |
| 2 | `crates/hex_editor/src/launch.rs`:9 | NON-BLOCKER | deferred — root discovery deliberately checks the two canonical catalog sentinels while persistence owns the broader `assets/art` root; both paths are contract-tested, and consolidating them would couple separate responsibilities without removing a mutable value |

**Notes**: all eight lenses and a silent-failure sweep found no remaining real
candidate. The full gate passes with 792 tests, including the tracked-material App
regression and four state regressions added during review. The game visual walk is
not an applicable editor check: it drives `hex_game`, while this PR adds the
standalone `hex_editor`; the renderer failure is covered at the Bevy asset-event
altitude instead.

## Wave 9 — feat(map): render animated opaque liquids (2026-07-28)

- **PR**: #88 — `feat/v3-liquid-renderer`
- **Outcome**: green — 4 ship-blockers fixed, 2 coverage limits deferred to the first runnable V3 world
- **Lenses triggered**: 2, 3, 4, 7, 8

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 4, 8 | `crates/hex_game/src/review.rs`:222 | SHIP-BLOCKER | fixed in `68d18c4` — captures without an explicit liquid phase now freeze at `0.0`, while non-capture launches retain live animation; focused regression and docs added |
| 2 | `crates/hex_map/src/liquid_render.rs`:29 | SHIP-BLOCKER | fixed in `8d44bf5` — production flow-rate constants now feed both material construction and the common-period test, and the shader source contract pins the secondary phase rate |
| 2 | `crates/hex_map/src/liquid_render.rs`:36 | SHIP-BLOCKER | fixed in `8d44bf5` — cap and curtain geometry now derive the inradius from `hex_core::config::HEX_SMALL_DIAMETER` |
| 3, 4 | `crates/hex_map/src/procedural_v3/materialize.rs`:84 | SHIP-BLOCKER | fixed in `8d44bf5` — edit protection now includes the voxel immediately above authored liquid, preventing topology-breaking burial |
| 7 | `crates/hex_map/tests/spawning.rs`:950 | NON-BLOCKER | deferred to draft #89 — no runnable V3 recipe can publish `MapPresentationProjection` on this branch, so the new V3 edit/rebuild schedule path cannot yet be exercised by an App |
| 7 | `crates/hex_map/src/liquid_render.rs`:844 | NON-BLOCKER | deferred to draft #89 — fall-curtain pixels remain unreachable until the first runnable V3 world; that landing must add `/visual-walk` plus the human motion/feel walk |

**Notes**: fresh-eyes found no additional bug class. Reachable legacy/V1/V2 cap
lifecycle remains covered by an App test. The phase default, all 13 liquid-render
unit tests, and the projection edit-protection test pass locally before the full gate.

## Wave 8 — chore(skills): formalize the wave delivery model (2026-07-27)

- **PR**: #72 — `chore/wave-model-skills`
- **Outcome**: green — docs-only path (4 docs lenses + fresh-eyes)
- **Lenses triggered**: D1 (one fix), D2–D4 clean

| Lens | File:line | Severity | Status |
|---|---|---|---|
| D1 | `.claude/skills/merge-pr/SKILL.md` (Step 1.2) | NON-BLOCKER | fixed in-branch — the four merge classes overlap (a wave→dev landing also matches the plain feature case); routing is now explicitly first-match-wins with the order called load-bearing |

**Notes**: encodes wave 1's proven process into the pipeline: create-pr/plan-ticket
learn wave bases, merge-pr gains the four merge classes with the stacked-child
check and the Linear auto-close guard, audit-diff/test-full resolve their diff
base from the PR's `baseRefName`, update-linear states the partial-epic rule, and
CONTRIBUTING.md/CLAUDE.md document the model once each at their own altitude
(full rules vs summary-and-pointer). All historical claims cross-checked against
the wave-1 record (HEX-6 auto-close, #62 closed by base deletion). D2: the
CLAUDE.md → CONTRIBUTING wave-section reference resolves; relative-link sweep
green. No Rust in the diff; test tiers short-circuit per the doc-only path.

## Wave 7 — feat(world): add celestial lighting and time-of-day scrubber (2026-07-27)

- **PR**: #64 — `feat/celestial-lighting`
- **Outcome**: green — 0 ship-blockers, 4 non-blockers, all left with the crate owner
- **Lenses triggered**: 3, 8, plus the fresh-eyes pass; all seven Bevy-0.19 trap checks explicitly cleared

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 3 | `crates/hex_world/src/sky.rs`:31 | NON-BLOCKER | deferred to owner — `ResolvedLighting` derives `Reflect` + `#[reflect(Resource)]` but no plugin registers it; invisible in the inspector, against the register-what-you-introduce convention |
| 8 | `crates/hex_assets/src/scenario.rs`:38 | NON-BLOCKER | deferred — `Scenario` lacks `deny_unknown_fields` just as it gains its first gameplay-visible optional field; a misspelled `starting_time_hours` silently launches at the cycle's noon default |
| 8 | `assets/shaders/sky.wgsl` | NON-BLOCKER | residual — +102 lines of surface on the documented silent-black-sky naga trap; current code verified safe (`fwidth` only under uniform control flow) |
| fresh-eyes | `crates/hex_assets/src/settings.rs`:589 | NON-BLOCKER | deferred — `validate_dark_handoffs` guards the flip keyframe, not the mid-segment elevation zero-crossing where the 180° key flip actually occurs; the shipped RON authors near-zero handoff elevations |

**Notes**: verified on a local merge-with-dev tree (435 tests green, fmt/clippy/deny/doc/links/ship clean). Claim-level checks all held: hot-reload keep-last-valid is real (a failed parse emits no `Modified` event, so the resource is never replaced); exactly one shadow-casting celestial light across scrubbing and gameplay re-entry, pinned by test; the static profile's `exposure_ev100: 9.7` equals Bevy 0.19's `Exposure::default()`, which is what grounds the pixel-identical claim for legacy looks; `TimeOfDay` is genuinely session-static with change-tick-gated resolution (zero per-frame lighting work on a frozen clock); the new `HEX_REVIEW_TIME`/`HEX_REVIEW_CAMERA` overrides stay fully behind `map-review`. Cross-boundary edits: `hex_core/src/view.rs` adds the single shared `CameraFocusTarget` marker (registered by `hex_units::selection` per the convention), and the `hex_units` selection bridge was explicitly ACKed by its crate owner in review. Walk items this merge adds: the actual sky across scrubbed hours, and the close camera's widened 4.5° uphill pitch.

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
## Wave 5 — feat(hex_lattice): add the pure lattice rules engine (HEX-8) (2026-07-27)

- **PR**: #60 — `feat/hex-8-lattice-engine`
- **Outcome**: green
- **Lenses triggered**: 4 (conservation/invariant), 7 (test-altitude), 5 (API/rebase), 1 & 3 (latent, deferred); fresh-eyes pass (plan-time vs mutation-site invariant)

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 4 | `crates/hex_lattice/src/cast.rs` (`satisfy` filter; `state.rs` `locks`/`lock`) | SHIP-BLOCKER | fixed in `8da1907` — a gem with residual mana could fund a second enchantment, and `apply_cast`'s `lock` overwrote the first's entry in the one-per-gem `locks` map, orphaning it (disable→break invariant violated, an unbreakable shield). Fixed by excluding locked gems from `satisfy`'s casting candidates |
| fresh-eyes | `crates/hex_lattice/src/cast.rs` (`apply_cast`) | MEDIUM | fixed in `27b2bb2` — the plan-time filter left the apply-time write reachable via a stale/concurrent plan (two plans on one state, then both applied), re-opening the orphan. `apply_cast` now returns `bool` and rejects a stale plan (a funding gem drained/locked/disabled) atomically; regression test computes two plans before applying |
| 7 | `crates/hex_lattice/tests/engine.rs` | NON-BLOCKER | fixed in `8da1907`/`27b2bb2` — added the shared-gem regression, the stale-plan regression, a two-gem enchantment clearing both locks on break, and channel budget distribution in coordinate order (all previously untested) |
| 5 | `crates/hex_core/src/elements.rs` | NON-BLOCKER | fixed in `8da1907` — bridge doc claimed re-pointing at HEX-7's `ElementId` is a no-op and that ids are "never written to files"; corrected — a `LatticeSpec` serializes the resolved ids, and HEX-7's `ElementId` must derive serde (unlike `SubstanceId`) for the rebase to compile |
| — (sweep) | `crates/hex_lattice/tests/engine.rs` (tier-6 random sweep) | NON-BLOCKER | fixed in `8da1907` — the sweep could pass vacuously; added a guard asserting it exercised ≥1 adjacent pair |
| 1 | `crates/hex_lattice/src/cast.rs` (`apply_cast` locked-mana fold) | NON-BLOCKER | deferred — `u16` `locked_mana` `saturating_add` could under-record a cast draining >65535 mana; out of the practical domain (mana is small by construction) |
| 3 | `crates/hex_core/src/lattice_ids.rs` (`neighbors`/`distance`) | NON-BLOCKER | deferred — overflow at extreme `i32` coords; out of the documented small/character-local domain, matches `HexCoord`'s own lack of a bound |

| wave deep-review | `crates/hex_lattice/src/channel.rs`:23 | SHIP-BLOCKER | fixed in wave follow-up — `channel()` refilled **locked** gems, refunding enchantment-locked mana and collapsing the throughput/capacity distinction; locked gems now skip channelling, with a regression test verified to fail against the bug |
| wave deep-review | `crates/hex_lattice/tests/engine.rs` | NON-BLOCKER | fixed in wave follow-up — `LatticeState` serde round-trip was claimed but untested; drained/disabled staleness modes untested; atomic rejection not pinned by full-state equality; `LatticeCoord`/`CellKind` wire formats unpinned (the Wave-3 HexCoord class). Four tests added |
| wave deep-review | `crates/hex_lattice/src/tables.rs`:20 | NON-BLOCKER | documented — `Requirement.mana` is drained when a gem satisfies it but a fusion substitutes its recipe; the design supports it (recipe complexity, not volume) and the cost question is deferred to HEX-12 |

**Notes**: two deferrals are latent, out-of-practical-domain integer edges (mana and lattice coordinates are small by construction). Lenses 2 (deps — serde/ron/rand match the workspace, hexx correctly absent, `--all-features` unifies cleanly), 6 & 8 (N/A — no ECS systems, no RON/features in this crate), and docs lenses D1–D4 on the four changed docs all verified clean. Determinism (BTree/sorted, no float/RNG/HashMap), backtracking undo, `break_enchant` clearing all locks, and `satisfy` termination were confirmed correct. The fresh-eyes pass caught what the eight lenses missed: the SHIP-BLOCKER fix sat at plan time while the invariant-violating write is at apply time. Both rebase-time bridges were resolved in the wave-integration merge: the placeholder `hex_core/src/elements.rs` was dropped for HEX-7's (which owns `ElementId` **and** `SpellId` — `SpellId` left `lattice_ids` for `elements` as a content id), and `serde` switched to the workspace dependency HEX-6 hoisted. No `hex_lattice` source changes were needed, verified by grep: every import resolves through the root re-exports. (Entry renumbered from Wave 3: two parallel sessions each claimed the next number; sequenced at integration.)

## Wave 4 — feat: elements and spells as content (2026-07-27)

- **PR**: #63 (successor of the auto-closed #62) — `feat/hex-7-elements-and-spells`
- **Outcome**: green
- **Lenses triggered**: 1, 3, 8, doc build, plus the fresh-eyes pass

| Lens | File:line | Severity | Status |
|---|---|---|---|
| doc build | `crates/hex_assets/{elements,spells}.rs` | SHIP-BLOCKER | fixed in `b22db57` — public docs intra-doc-linked the private `Unvalidated*` mirrors; the workspace doc gate runs `-D warnings` |
| 3 | `crates/hex_assets/src/elements.rs`:128 | NON-BLOCKER | fixed in follow-up — fusion recipes had no upper input bound while spells cap at six; same six-neighbour ring, now enforced with a rejection test |
| 8 | `crates/hex_assets/src/spells.rs`:255 | NON-BLOCKER | fixed in follow-up — `SelfCast` with a nonzero range parsed cleanly and meant nothing; rejected at parse with a rejection test |
| 1 (fresh-eyes) | `crates/hex_assets/src/content_index.rs`:9 | NON-BLOCKER | deferred to HEX-12 (recorded on the ticket) — the kept last-valid index can desync from independently rebuilt tables' reassigned ids; needs coupling when a real consumer exists |
| 1 | `crates/hex_assets/src/content_index.rs`:154 | NON-BLOCKER | deferred to HEX-12 (recorded on the ticket) — initial-load dangling cross-references log-and-continue instead of stalling the gate; the gate must wait on the index once something consumes it |

**Notes**: reviewed on the wave-integration model — diffs against `wave/1-foundations`, gate run on the merged state (411 tests green, clippy/deny/ship clean). Schema fidelity to the frozen audit §8 verified: no code matches on an element name, opposition is index arithmetic, ids from byte-order sorted names, both new tables and the index keep the rebuild-deferred-during-Gameplay guard. The deliberate divergence from the audit sketch — flat defense on the `Enchantment` casting axis rather than an effect — is an improvement and is recorded here as accepted.

## Wave 3 — feat(core): serde derives across the domain vocabulary (2026-07-26)

- **PR**: #59 — `feat/hex-6-serde-vocabulary`
- **Outcome**: green
- **Lenses triggered**: 2 (Cargo hoist consolidation), plus the fresh-eyes pass

| Lens | File:line | Severity | Status |
|---|---|---|---|
| 2 | `crates/hex_assets/Cargo.toml`:12 | NON-BLOCKER | deferred — serde/serde_json stay per-crate pins in hex_assets and hex_map, so the workspace hoist is not yet the sole source; hex_map is the colleague's off-limits crate and hex_assets is HEX-7's parallel territory, and all pins match (serde `1.0.229`, serde_json `1`) so there is no active drift |
| fresh-eyes | `crates/hex_core/src/terrain.rs`:21 | NON-BLOCKER | fixed in `5dcc9d4` — the `MapAnchorId` doc justified its newtype pattern by "keeping serialization dependencies out of this bottom-level domain crate"; hoisting serde into hex_core falsified that, so the rationale was rewritten to the reason that still holds (single construction path, not pinned to an on-disk format) |
| fresh-eyes (wave review) | `crates/hex_core/src/hex.rs`:95 | NON-BLOCKER | fixed in follow-up — `HexCoord`'s wire keys were its private field identifiers, so an internal rename would compile clean, pass the symmetric round-trip test, and silently change save files; the wire names are now deliberate axial `q`/`r` via serde renames, pinned by a concrete-string snapshot test |

**Notes**: no ship-blockers. Pure additive data-layer change — serde on `TilePos`, `HexCoord` (axial-only storage keeps the cube invariant by construction), `SubstanceId`, `TerrainEdit`, `TraversalProfile`, `Turn`, `Faction`, `Body`; `HexSpan` deliberately excluded (floats stay out of saves) and the marker / map-measured types (`Headroom`, `HexGrid`, `HexTile`, `TraversalEndpoint`) left for the save work. `Turn` also gained `PartialEq`/`Eq` for the round-trip assertion (rustfmt then wrapped its derive). fmt, clippy (`-D warnings`), workspace tests, and the ship build all green; no rendering / movement / state surface, so no visual walk applies.

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
