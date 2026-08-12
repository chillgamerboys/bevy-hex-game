# L2 order — Gameplay authority

## Objective

Route every human command through the shared request ingress, isolate host-only simulation,
apply six-seat ownership/delegation at the existing reducer, and publish disclosure-safe
unit/session projections and typed command results. Preserve offline transcripts and all
existing combat/tempo rules.

Read `../manifest.md` and the maps below first. If refreshed source disagrees, stop and
escalate. Do not edit before every dispatch blocker in the L2 row is true.

## Locked decisions (verbatim)

- “Human seats are `0..=5`; reserve `PlayerSeat(u8::MAX)` for host AI/system commands so
  the host’s player seat cannot command hostile units.”
- “The host must control at least one party member. Every connected non-spectator must own
  at least one; the host owns unassigned members by default and may redistribute them in
  the lobby.”
- “Group movement includes only characters assigned to the issuing seat. `MoveParty`
  validates every included member, not only its anchor.”
- “A disconnected seat remains reserved for 30 real-time seconds. It then receives a
  temporary host delegation; canonical `ControlOwner` assignments do not change.
  Reconnection revokes delegation at the next boundary with no command, decision, or
  movement in flight.”
- “Only the host may globally pause, save, launch, retry, kick, or close. Client Escape
  menus are local and non-pausing. Any connected player may issue `Rest` through one of
  their assigned party members.”
- “Existing combat rules remain unchanged: one global `Mode`, one active turn, no
  split-tempo party, and no simultaneous allied turns.”
- “The listen host owns simulation, AI, world mutation, admission, global pause, and saves.
  Remote clients submit intents and apply disclosure-safe authoritative projections; there
  is no lockstep, rollback, prediction, host migration, or dedicated server in this epic.”
- “A wire `GameCommandRequest` contains only `request_id` and `command`. The host derives
  seat/delegation from the authenticated connection and caches outcomes by seat/request id;
  a remote payload can never assert a seat.”
- “`CombatState` remains host-only. Clients receive exact authorized unit/session
  projections and the existing shared player-faction knowledge view, never undisclosed
  hostile lattice facts.”
- “Single-player uses the same local request ingress and defaults to
  `SimulationRole::Authority` without opening a socket.”

## Banked maps

- `../maps/authority-and-command.md`
- `../maps/app-and-session-ui.md`
- `../maps/territory.md`

## Owned territory

Only the L2 regions/tests and L2 manifest row in the queue. Do not edit world crates,
`hex_multiplayer` protocol/runtime, root Cargo/composition, or shared UI rendering.

## Required implementation

1. Replace direct human `IssuedCommand` emission with `LocalGameCommandRequest` at click,
   casting, lattice decision, turn, Rest, and party-strip sources. One ingress adapter
   handles offline/listen host locally; remote replicas send the seatless wire request.
2. Leave `IssuedCommand` and `CommandQueue` as the reducer boundary. Host ingress derives
   canonical seat/delegation and stamps it immediately before enqueue.
3. Assign all hostile/AI-controlled actors `PlayerSeat::AI` and make AI-generated commands
   use it. Prove the host human seat cannot command a hostile and a remote request cannot
   name AI authority.
4. Apply lobby party-slot ownership on launch. Keep `ControlOwner` canonical through
   disconnect; consult temporary delegation only in authenticated ingress.
5. Change group planning/completeness from the whole Party to only members owned by the
   issuing seat. Preserve atomic validation of every included member, duplicate member,
   start, route, destination, occupancy, and busy checks.
6. Permit Rest when its named issuer is a party unit owned by the issuing seat. The
   existing effect may recover the full party; foreign issuers remain refused.
7. Add `AuthoritativeSystems` gates to AI, command reduction, combat apply/resolve/advance,
   domain movement/occupancy, encounter transitions, authoritative perception hooks, and
   saving adapters owned by gameplay. Compose with `PausableSystems`; do not gate replica
   application or presentation interpolation.
8. Emit one typed `CommandResult` after reducer acceptance/refusal, with the session's
   authority sequence. Adapter-level refusals and duplicate outcomes use the same result
   channel. Do not turn warnings into the network protocol.
9. Build `UnitReplica`/`SessionReplica` from public authority facts: exact `TilePos`, exact
   motion route/progress boundary, owner, lattice view allowed by disclosure, downed/turn/
   effect state, global mode/pause, initiative, pending decision, outcome, and authority
   sequence. Never serialize `CombatState`.
10. On replicas, apply authoritative exact positions/routes and interpolate transforms
    locally. Corrections replace presentation routes without mutating authority facts.
11. Detect the quiescent boundary used by delegation/reclamation and reconnect snapshot:
    empty command ingress/queue, no pending decision belonging to the seat, and no owned
    domain movement in flight.

## Required evidence

- Offline single-player transcript/fingerprint comparison before and after ingress change.
- Listen-host local request follows the identical request-id path without a socket.
- Remote request cannot supply/forge seat, command foreign units, command AI, globally
  pause, save, retry, kick, or close.
- Six-seat assignment tests including host minimum, no empty connected seats, reassignment,
  readiness clearing, per-seat movement subsets, Rest from every seat, and sequential turns.
- Disconnect tests during exploration, movement, combat turn, and pending decision;
  delegation and reclamation happen only at the specified boundary.
- Projection tests compare every authorized field after each sequence and prove hostile
  lattice facts never appear.
- Focused rules/contracts/simulation/app tests plus selector-chosen checks. Motion and feel
  evidence is deferred to the exact combined head.

## Handoff

Update only the L2 manifest row to `in-review`. Report every migrated emitter (grep list),
the exact authority-gated system list, transcript comparison, and any behavior the existing
reducer could not express. The coordinator alone records `merged-to-wave`.
