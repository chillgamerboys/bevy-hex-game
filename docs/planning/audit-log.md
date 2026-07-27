# Audit log

Wave entries appended by `/audit-diff` — one per audited PR, the
durable trail of which lenses fired and what was fixed or deferred.
The receipt at `/tmp/audit-pr-receipt-<N>.json` is ephemeral; this
file is the record that travels with the repo.

<!-- /audit-diff appends below this line. Don't insert content between this comment and Wave entries; the skill anchors on this marker. -->

## Wave 3 — feat(hex_lattice): add the pure lattice rules engine (HEX-8) (2026-07-26)

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

**Notes**: two deferrals are latent, out-of-practical-domain integer edges (mana and lattice coordinates are small by construction). Lenses 2 (deps — serde/ron/rand match the workspace, hexx correctly absent, `--all-features` unifies cleanly), 6 & 8 (N/A — no ECS systems, no RON/features in this crate), and docs lenses D1–D4 on the four changed docs all verified clean. Determinism (BTree/sorted, no float/RNG/HashMap), backtracking undo, `break_enchant` clearing all locks, and `satisfy` termination were confirmed correct. The fresh-eyes pass caught what the eight lenses missed: the SHIP-BLOCKER fix sat at plan time while the invariant-violating write is at apply time. This PR carries two rebase-time bridges (`hex_core::elements::ElementId` owned by HEX-7; inline `serde` owned by HEX-6) and must not merge until rebased onto both.

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

