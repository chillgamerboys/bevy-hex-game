# Golem and Worm response repair — 2026-09-09

Local candidate branch: `fix/arena-golem-worm-response`, based on merged PR #221
(`bb556963632de933b44fb75b1d306aca79258cef`). This repair is not a new balance pass.
The user requested a stop below 20% remaining account usage; checks during this
pass reported 28% remaining.

## Changes

Golem Stone Swipe previously required 48 ticks without progress. Sliding beside
cover and stationary attack holds reset that movement clock, delaying or starving
obstacle clearing. The existing full-body obstruction probe now admits a swipe on
its normal 10 Hz cadence without the separate stall requirement. Empty paths,
protected terrain, cooldowns, attack duration and damage retain their rules.

A Worm could remain in Diving forever when a crater removed the common shallow
travel band beneath its long body. After lowering its head, an activated Worm now
retries stationary emergence after its existing surface interval. Actual head
clearance and line of sight still gate its Boulder, and full-body rise admission
still applies. It cannot teleport, replace missing terrain, move above ground or
claim underground knowledge while exposed. A destroyed travel band can still stop
locomotion; this repair restores a counterattack opportunity in that situation.

## Focused evidence

- 45 Worm-filtered library tests pass, including damaged head/tail support,
  retraction followed by counterfire, hidden-information isolation and ordinary
  retraction/travel between successive shots on intact ground.
- 32 Golem-filtered library tests pass, including a new production-motion check
  proving swipe admission after recent forward progress and the existing check
  for following a published opening. Protected/static cover and empty-lane checks
  remain in the same test scope.
- Strict `hex_arena` Clippy passes with all targets/features and `-D warnings`.
- A new actual-map application check admits a stationary human on dry, visible
  footing near a Worm in Fort and Duel and requires a real Boulder release. The
  initial run passed even before this repair: it is a normal-encounter regression,
  not a reproduction of the damaged-ground deadlock. The final candidate rerun
  also passed on both maps (one test). Formatting and diff checks passed.

Commands and logs are retained in `.context/creature-pressure-checks/`:
`worm-response-tests.log`, `golem-response-tests.log`, `response-lint.log`,
`worm-response-repro.log` and `worm-response-final-map.log`.
No broad workspace gate, new render capture or matchup calibration was run.
Native playtesting must still confirm these fixes cover the reported cases;
other causes of nonresponse or blocked movement have not been ruled out.
