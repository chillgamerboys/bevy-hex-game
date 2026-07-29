# Enemy intelligence

Enemy behavior is deterministic and replaceable. The runtime does not treat one
strategy as “the AI”: content selects a profile, a profile selects a registered
algorithm, and every algorithm receives the same authorized request contract.

`hex_ai` is pure and headless. It may hold algorithm-private deterministic state, but
it cannot define legality or mutate the simulation. For an ordinary turn,
`hex_combat` enumerates every legal command in canonical semantic order, fingerprints
that exact set, invokes the profile's mutable algorithm instance, validates the
returned request-scoped key, and queues the matched `GameCommand` through the ordinary
applier. An unknown or stale key mutates nothing and falls back only to the enumerated
End Turn command.

Exact-cell damage and restoration choices use a separate compact
`CellChoiceSet`. The request carries the subject, exact quota, canonical eligible
coordinates, and a fingerprint; it never materializes the `n choose k` command
combinations. `AiAlgorithm::select` returns `AiSelection::Action` for an ordinary
turn or `AiSelection::Cells` for one of these choices. The host checks the
fingerprint, count, uniqueness, and eligibility before constructing the existing
replayable `ChooseDisables` or `ChooseRestores` command.

That boundary does not transfer lattice ownership to AI: `hex_ai` chooses from an
authorized request, `hex_combat` owns command legality and decision sequencing, and
`hex_lattice` alone owns the pure state transition.

Observations contain stable `UnitId` and `TilePos` values, never `Entity`. Allied
information is complete. Hostile identity, position, effects, and turn-order presence
require current observation, while hostile lattice facts come only from the existing
faction-knowledge projection. Downed hostiles may remain visible, but they are not
offensive movement goals.

Traversal input is likewise an authorized projection rather than hidden world truth.
Observed and Remembered terrain may contribute directed surface-to-surface edges;
Unknown terrain contributes nothing. A decision builds this graph once, builds one
reachable/predecessor projection for the actor, and builds one reverse distance map
for each live observed hostile. Algorithms can therefore compare complete authorized
routes without either repeated breadth-first searches or access to hidden geometry.

The shipped `baseline-v1` policy is intentionally small and deterministic. It prefers
Renewal for the lowest-id downed ally, Scrying Eye for the lowest-id opaque hostile,
the strongest single-target direct-damage cast, a missing self-enchantment, the
lowest-id adjacent strike, and movement along the shortest complete route to an
observed hostile, in that order. It re-enters the host after movement presentation
instead of pre-queuing End Turn, preserving move-then-act turns.

AI-owned disable decisions sacrifice Blank, Gem (least held mana), Fusion, then Spell
cells. Restoration uses the reverse utility order: Spell, Fusion, Gem, then Blank.
Coordinates break every remaining tie. Exact choices are still commands in the replay
stream; replay runs with AI dispatch disabled and consumes those recorded commands.

## Publication and diagnostics

The authorization-critical prefix of the same-frame order is:

`PerceptionSystems::PublishKnowledge` → combat spatial-knowledge synchronization →
`CombatSystems::Act` → `CombatSystems::Apply`. Normal combat processing then continues
through `Resolve` and `Advance`.

An AI decision therefore cannot observe a unit that lost sight earlier in the same
frame, including when the sole sight-providing unit was just downed. The command
applier remains authoritative and repeats positional cast-anchor validation rather
than trusting enumeration.

Detailed AI inspection retains only the latest 64 decision traces. `CombatSummary`
keeps compact totals and rolling fingerprints for the complete session while capping
its detailed event and AI-decision windows at 4,096 entries each. Tests and tooling
that genuinely need every record must explicitly enable `CombatTranscriptRecorder`;
normal gameplay does not accumulate an unbounded transcript. Existing serialized
summary records still decode through the compatibility defaults.

## Future planning model

Search algorithms may later use a `PlanningModel`, but the model is not part of Wave 4
Part 1. Its contract must provide:

- a cloneable authorized planning state;
- deterministic legal-action enumeration;
- application of an enumerated action through authoritative combat rules;
- discovery of the next decision point;
- terminal-state detection; and
- evaluation hooks owned by the calling algorithm.

The model must reuse the command applier rather than duplicate combat legality.
Alpha-beta, MCTS, or another search implementation may construct or receive the model
while continuing to implement `AiAlgorithm`.

Imperfect-information planning receives no hidden truth. Belief states,
determinization, search policy, and evaluation belong inside the algorithm; none may
widen the observation the host authorized.
