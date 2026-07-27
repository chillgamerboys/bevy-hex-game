# Audit log

Wave entries appended by `/audit-diff` — one per audited PR, the
durable trail of which lenses fired and what was fixed or deferred.
The receipt at `/tmp/audit-pr-receipt-<N>.json` is ephemeral; this
file is the record that travels with the repo.

<!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->

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

