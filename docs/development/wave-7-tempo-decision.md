# Wave 7 tempo decision audit

Wave 7 retains the shipped rule of four movement hexes and one action per turn.
This is a bounded development decision, not a claim that the final action economy is
settled. No value in `assets/config/combat.ron` changes.

## Evidence

The authoritative evidence is the renderer-, viewport-, wall-clock-, and
map-generator-independent `hex_combat` simulation target. Every case starts from
fresh synthetic shared surfaces, runs twice, and requires complete
`CombatRunSnapshot` equality before checking named metrics.

| Case | Opening movement budget | Roster | Completed turns | Successful / refused | Typed termination |
|---|---:|---:|---:|---:|---|
| Shipped | 4 | 3v3 | 12 | 12 / 0 | bounded no-progress, streak 12 |
| Tactical two-step | 2 | 3v3 | 12 | 12 / 0 | bounded no-progress, streak 12 |
| Custom three-step | 3 | 3v3 | 12 | 12 / 0 | bounded no-progress, streak 12 |
| Shipped dense run | 4 | 6v6 | 24 | 24 / 0 | bounded no-progress, streak 24 |

These cases intentionally end turns without inventing a balance-policy oracle. They
prove that all three profiles enter the real turn loop with their exact frozen
budgets, stable turn state, canonical summary/command/transcript fingerprints,
bounded telemetry, and exact unique `TilePos` occupancy. The 6v6 run retains twelve
unique positions and non-zero canonical command and transcript fingerprints across
both runs.

Separate cases in the same target prove an occupied chokepoint rejects the exact move
without changing position, and that Channel restores two named Fire mana, consumes
exactly one action, and refuses the repeated action. Focused command/effect/AI tests
remain in their owning crate partition.

No screenshot, viewport timing, input heuristic, animation frame, or map generator
result contributes to these claims. The earlier visual-run round/turn/distance
numbers are withdrawn as balance evidence because the solver was frame-sensitive.

## Decision

Keep Shipped movement at four. The deterministic matrix proves the presets are
faithfully applied but deliberately does not pretend a bounded no-progress script is
a balance preference. With no new deterministic or human evidence strong enough to
justify changing authored policy, retaining the shipped value is the conservative
decision.

Future changes need broader human playtesting plus goal-directed deterministic cases
with an agreed policy oracle. They must revisit initiative, rout/stalemate policy,
and action economy together rather than treating viewport-dependent completion time
as a mechanical result.

## Presentation review

Wave 7's presentation evidence was consolidated again during the UI foundation into
the sole gameplay-owned `walks/gameplay_ui.ron`. The historical logical claims remain
in deterministic simulation and app snapshots. The walk opens presentation states
without solving combat and its frames are evidence only for layout and legibility.
