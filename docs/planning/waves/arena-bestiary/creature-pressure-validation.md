# Creature pressure validation — draft

Status: implementation prepared; combined verification and changed-surface native
review are **pending**. This record covers the [2026-09-08 playtest follow-up](creature-pressure.md),
not the earlier calibration checkpoint. Root owns integration and the final receipts.
No merge is authorized.

## Candidate and scope

- Integrated commit and clean source receipt: **PENDING**.
- Test/build commands, log locations and artifact hashes: **PENDING**.
- Accepted human controls, spells, movement and the original Shadow policy remain
  the required reference. No new human balance result is claimed.

Player and observer Goblin recipes now contain ten; Shaman recipes contain a
Shaman plus five Goblins, giving Seven Regions 17 enemies. Goblins spread their
approaches, attempt supported high jumps and use bounded hole recovery. Escorts
stay near the Shaman; its support radius is nine units.

Healthy Dragons can make a brief swept approach burst under ranged pressure.
Wisps may spend two cover shots toward a recent own sighting, then seek another
angle. Neither behavior gains access to a hidden target's current position.

The Golem's two-second laser charge now precedes four seconds of fire, capped at
180 damage per target. Bounded aim follows own sightings; after losing sight during
fire, it extrapolates the same target's last observed velocity. A short frontal
swipe clears obstructing terrain while excluding footing and protected cells.

Activated, physically buried Worms privately sense hostile positions across the
map. Above ground they require sight; hostile damage causes retraction and renewed
pursuit. Movement follows underground approach, exposed attack and burrow/reposition
cycles. Boulder damage 70, HP 320 and the world acknowledgement/full-body admission
rules remain. An impossible shallow footprint can still prevent escape; it cannot
justify fabricated support or above-ground crawling.

Terminal outcomes automatically pause the arena and free the cursor. Reset returns
to the ready screen; completed rounds cannot resume.

## Required evidence

| Check | Result and receipt |
|---|---|
| Gameplay regressions, hidden-history boundaries and unchanged Duel goldens | **PENDING** |
| Actual-map larger-roster admission, actions, independent parties and resets | **PENDING** |
| Application terminal pause, input cancellation and controls | **PENDING** |
| Strict workspace lint and selected combined repository gate | **PENDING** |
| Clean native build and changed ability/static capture matrix | **PENDING** |
| Sustained 17-enemy workload, publications and native tick measurements | **PENDING** |
| Human control feel, motion recognition and revised matchup balance | **PENDING — playtest** |

The [historical bestiary record](../../../systems/arena-bestiary-validation.md)
retains its original source identities, smaller rosters and results. Earlier Worm
6/8 wins against five Goblins, Wisp12 Fort3/4 and Golem matchup figures are not fresh
evidence for this candidate. Timeouts remain unresolved results, and still images
do not establish native input feel or animation quality.
