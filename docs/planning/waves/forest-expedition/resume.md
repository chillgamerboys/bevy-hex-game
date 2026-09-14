# Expedition resume checkpoint

September 12, 2026. The approved [combat, glider and world plan](combat-world-upgrade.md) is integrated in the isolated candidate. **The hourly timer remains PAUSED.** Do not create a continuation automation, spend reset credits, merge dev/main, or write to remote services.

## Current candidate and package

- Checkout: `/Users/alberto/Documents/Codex/2026-09-11/i-w/work/hex-expedition`, branch `wave/forest-expedition`.
- Runtime foundation: `19e02f44c95b8c13ba28dbbe342b40f90e659829`, followed by the user-requested bluer sky in this checkpoint, including the clipped-menu repair and observation optimization, rank upgrades, transient health dots, closer discovery, aggressive/summit Dragons, glider, lowland encounters/rewards, sparse solid carving, closed water volumes/tint, sky and crater shading.
- Package: `/Users/alberto/Documents/Codex/2026-09-11/i-w/outputs/forest-combat-package-8f56a9974aaac54b`, fingerprint `8f56a9974aaac54b`. Strict survey: 105469 columns, 444 chunks, 9497 support sites, 49 routes, 25 encounters, 914 objects (807 trees and 107 props), six fountains and 67.0088% forest canopy.
- Exact roster: 107 Goblins in camps 3,3,3,3,3,5,5,5,9,9,11,13,15,20; two Shamans, Troll, three Dragons, Shadow, three Golems and ten Wisps. **127 enemies / 128 actors.** The first 15 Goblins are babies; Shamans support the 13/15 camps.
- Fountains remain the only healing source. Shadow gives +25 maximum HP without healing; Troll adds 25 base Fireball damage; last Dragon unlocks explosions; final Wisp/Golem clears drop collected velocity/guide and Shield rewards. Full credited clearance gives 432 XP: level 8, 109/171 XP and seven points. Victory leaves casting/exploration/pickups active; Restart resets the complete run.
- The prior user-owned runtime `70de964`, PID 1608/window 17025, may still be open. Do not control it, terminate it, or change its recording. Keep its original `assets/config/v4/forest-massif/expedition/compiled` package (`a538263d612e891f`) untouched so that its Restart remains valid. The new launcher must select the new package explicitly.

## Latest validation

Evidence directory: `/Users/alberto/Documents/Codex/2026-09-11/i-w/outputs/expedition-validation`.

- Final gameplay: **437 tests passed, zero failed, two existing ignored** on arena sources byte-identical to root `6407eaa` (tree `adb85564837b4d3fb111feb2151ef8c0682ed98a`). Strict arena library Clippy passes. The arrival fix suppresses travel probes only inside existing arrival tolerance while retaining crowd spacing and physical support refresh; focused regressions cover remote and underfoot carving.
- Fresh combined focused probes validate the exact roster/reset, published routes, camp/rally and destructive combat, plus Duel reset/prediction and Fort navigation. Six object-carving tests, five legacy world tests, the actual package mixed-carve/reset fixture, two closed-water mesh tests and four render-fixture contracts pass.
- All ten menu/layout tests pass at `82d506f` (one optional ignored), including scaled edge clipping. The actual map M-toggle/Restart regression also passes. Final fountain correction: seven focused tests pass, including immediate personal-use memory without revealing unseen pools.
- Actual 128-actor CPU at `82d506f`: camp p95 **3.662 ms**, rally **7.495 ms**, destructive combat **4.913 ms** overall and **7.303 ms** on terrain-changing ticks. Before the idle-arrival fix these were 13.132, 8.727, 16.171 and 18.138 ms respectively. Both fixtures pass. These are simulation CPU measurements, not renderer/HUD/GPU/FPS or recording performance.
- Cargo native build passes, including the Mac recorder helper. The first render pack at `6407eaa` failed for oversized clouds, insufficient crater seam contrast, an overconstrained water-edge fixture and distant summit framing. The corrected `19e02f4` pack captured all 12 views. The user accepted the crater layers and requested a bluer sky; this checkpoint changes only the sky gradient after that review. Fresh source/package identities and the final static outcome are recorded in `outputs/expedition-validation/final-validation.md` and its linked capture receipts. Stills do not establish glider feel, cloud motion, health-dot timing or native performance.
- Repository selector chose the full combined gate. Links, formatting and Cargo deny pass. The gate stopped at strict workspace Clippy: 517 inherited `procedural_v3` findings, plus two new water findings that were subsequently fixed. Scoped presentation library Clippy passes. The remaining broad selected test/doc/shipping stages were not run; do not describe the full gate as passing. Python arena helper tests: 55 passed before the final capture additions; 17 focused launcher/expedition tests pass after them. A zero-test discovery attempt and an incorrect import-root attempt are retained as failed harness attempts, not pass evidence.
- Automated native UI is limited to reproducing concrete user-reported defects. The user owns the short manual playtests in `outputs/expedition-validation/combat-world-playtest.md`. No new native playtest or recording measurement is claimed.

## Handoff and next work

1. Consult `outputs/expedition-validation/final-validation.md` for the latest build, source, package, pixel review and delivery state. The final sky-color capture and independent inspection must complete before a static pass is claimed.
2. The candidate launcher must select the new package explicitly; use `outputs/expedition-validation/combat-world-playtest.md` for the three short checkpoints. Do not take over or close the old game. Address reported defects with focused reproductions; avoid repeated broad UI clicking.
3. Full merge acceptance remains blocked by inherited lint debt; native feel and final performance acceptance belong to the user playtest. No dev/main merge is authorized.
4. The conditional official OpenAI practices review is deferred until game development/validation work is complete with at least 20% usage remaining. Do not edit global instructions or settings as part of that review.

## Build resources

APP target: `/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/work/cargo-target-explore`.
PURE target: `/Users/alberto/Documents/Codex/2026-09-04/i/work/cargo-v4-pure`.
Serialize each target and set `CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2`. Use Cargo for all source builds and launches; no bare executable. Tests must exercise positive expected counts. Preserve capture source/package hashes and distinguish static presentation, simulation CPU and native motion evidence.

Earlier UX/recording and Restart evidence is preserved in [battle-ux.md](battle-ux.md), [restart-feedback.md](restart-feedback.md), [render-review.md](render-review.md), Git history and the validation outputs. The approved injection supersedes their older roster, upgrades, discovery and world-destruction contracts.
