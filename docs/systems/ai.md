# Enemy intelligence

Enemy behavior is deterministic and replaceable. The runtime does not treat one
strategy as “the AI”: content selects a profile, a profile selects a registered
algorithm, and every algorithm receives the same authorized request contract.

`hex_ai` is pure and headless. It may hold algorithm-private deterministic state, but
it cannot define legality or mutate the simulation. `hex_combat` enumerates every
legal action in canonical semantic order, fingerprints that exact set, invokes the
profile's mutable algorithm instance, validates the returned request-scoped key, and
queues the matched `GameCommand` through the ordinary applier. An unknown or stale key
mutates nothing and falls back only to the enumerated End Turn command.

Observations contain stable `UnitId` and `TilePos` values, never `Entity`. Allied
information is complete. Hostile identity and position require observation, and
hostile lattice facts come only from the existing faction-knowledge projection.
Traversal input is likewise an authorized projection rather than hidden world truth.
It contains directed surface-to-surface edges, so algorithms can compare complete
terrain routes without learning any map fact outside the authorized projection.

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
