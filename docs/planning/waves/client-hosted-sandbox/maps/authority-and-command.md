# Authority and command map

Banked against `origin/dev@92662d456746506093e8de61f54f1d619085e1fe` after the
additive merge into the planning branch. If refreshed source disagrees with this map,
escalate and amend the manifest; do not make a lane-local judgment call.

## Existing flow

```text
UI / click / AI
    -> IssuedCommand { seat, command }
    -> CommandQueue (ordered, single consumer)
    -> hex_combat::commands::apply_commands
    -> validation + authoritative mutation + CombatEvent projection
```

- `GameCommand` is stable domain vocabulary at
  `crates/hex_core/src/commands.rs:41-147`.
- `IssuedCommand` and the exact-once FIFO are at
  `crates/hex_core/src/commands.rs:167-230`.
- `PlayerSeat`, `ControlOwner`, and `SimSeeds` are at
  `crates/hex_core/src/unit_ids.rs:26-90`.
- The one reducer is registered at
  `crates/hex_combat/src/commands/mod.rs:204-230` and drains at
  `crates/hex_combat/src/commands/mod.rs:250-278`.
- AI currently stamps its unit/controller seat at
  `crates/hex_combat/src/ai.rs:169-219` and
  `crates/hex_combat/src/ai.rs:420-452`.
- Exploration movement currently emits directly to `CommandQueue` at
  `crates/hex_units/src/units.rs:377-424` and
  `crates/hex_units/src/units.rs:481-650`.
- Gameplay actions currently emit directly at
  `crates/hex_game/src/screens/gameplay.rs:111-168` and
  `crates/hex_game/src/screens/gameplay.rs:174-277`.
- Other human producers are raw end-turn input at
  `crates/hex_combat/src/turns.rs:465-508`, casting at
  `crates/hex_game/src/casting/panel.rs:113-140` and
  `crates/hex_game/src/casting/mod.rs:772-809`, and defender/restoration decisions at
  `crates/hex_game/src/readouts/lattice.rs:373-478`.

## Per-symbol dispositions

| Symbol / region | Disposition | Reason |
|---|---|---|
| `GameCommand` | keep | It is already deterministic, serde-enabled, entity-free authority vocabulary. Network bounds validate its dynamic fields after decode rather than replacing it. |
| `IssuedCommand` | keep authority-side | It remains the only reducer input and is never accepted directly from the wire. Its seat is derived by the host. |
| `CommandQueue` | keep private ingress target | Local and remote request ingress both push here after authentication/idempotence checks. Reducer ordering stays unchanged. |
| `PlayerSeat` | extend in foundation | Add human bounds/constants and `AI = u8::MAX`; default remains seat 0. Do not deserialize an arbitrary remote seat claim. |
| `ControlOwner` | keep canonical | Lobby launch writes party ownership. Temporary host delegation lives in session state and never rewrites this component. Hostiles use `PlayerSeat::AI`. |
| `SimSeeds` | keep and manifest | Exact seeds enter `SessionManifestV1`; no random source is introduced by networking. |
| `SimulationRole` | add in `hex_core::app` | `Authority` default for offline/listen host; `Replica` only for remote clients. It is independent of connection state. |
| `AuthoritativeSystems` | add in `hex_core::app` | Shared run-condition vocabulary for all mutating simulation sets; presentation/network application stays outside it. |
| `CommandRequestId` | add in `hex_core::commands` | One stable id shared by local ingress and wire requests. |
| `LocalGameCommandRequest` | add as a Bevy message | All human UI/click/casting emitters migrate to it; offline adapter derives the local seat and pushes `IssuedCommand`. |
| `GameCommandRequest` | add in `hex_multiplayer` | Exactly `{ request_id, command }`; deliberately no seat, connection id, token, or authority sequence. |
| `CommandResult` | add in `hex_multiplayer` | Correlates request id with authority sequence and typed accepted/duplicate/refused outcome. |
| reducer legality | keep | Networking authenticates identity and bounds payloads but never duplicates command legality. |

## Ownership and movement findings

The reducer's ordinary unit gate resolves `ControlOwner` and compares it with
`issued.seat`; the multiplayer layer must preserve that check rather than pre-authorize a
command as legal. `MoveParty` is the important special case: it already checks every path
member at `crates/hex_combat/src/commands/move_party.rs:46-92`, but then requires every
canonical party member to be present at lines 123-130. L2 changes the emitter/planner and
that completeness rule to the issuing seat's assigned subset. It must still reject a
foreign included member atomically.

`Rest` validates party membership at `crates/hex_combat/src/commands/rest.rs:12-38`; the
general ownership gate must validate the named issuer first. The effect may still recover
the whole party. This is how “any assigned player may issue Rest” composes without allowing
a remote seat to name an unowned actor.

AI must use `PlayerSeat::AI`, not the host's human seat. Hostile spawn ownership is assigned
at `crates/hex_units/src/units.rs:1251-1308`; changing only the hostile owner there is a
behavior-neutral foundation change because AI already stamps commands from `ControlOwner`.
A host-side delegation lookup may
authorize a host connection to act temporarily for a disconnected human seat, but the
resulting `IssuedCommand` is stamped with the delegated canonical seat so ownership and
idempotence remain truthful.

## Authority gating

`PausableSystems` currently gates many mutating and presentation systems but is not an
authority boundary (`crates/hex_core/src/app.rs:124-130`). Add a separate
`AuthoritativeSystems` set and compose both conditions where needed. At minimum gate:

- AI command generation;
- command reduction and combat progression;
- domain movement clocks and occupancy reconciliation;
- terrain edit/impact mutation;
- encounter/mode transitions and authoritative perception publication;
- Campaign persistence.

Do not gate camera, selection rendering, animation interpolation, UI, networking, or
replica application. A replica never initializes host-only `CombatState` merely to satisfy
an optional query.

`AuthoritativeSystems` must be usable in `OnEnter` schedules as well as `Update`; initial
authoritative setup and replica setup are different paths. Merely gating the update loop
would let a remote replica spawn or initialize authority state once at entry.

Request correlation must not add a required request-id field to `IssuedCommand`, because AI,
replay fixtures, and current deterministic tests are valid authority producers. Add a
sidecar/envelope at ingress and correlate at the sole drain boundary. The reducer already
has typed refusals but no generic accepted event, so `CommandResult` must be emitted from
that boundary rather than inferred from `CombatEvent`.

## Territory

- #186 overlaps core exports, combat AI/commands, gameplay adapters, and perception.
- #188 overlaps unit movement/selection and its walk.
- #189 overlaps combat authority, save, and UI.
- #190 inherits #186 and also overlaps game/unit presentation.

Safe foundation regions now are the new core role/set vocabulary, seat constants/helpers,
hostile AI ownership, new `hex_multiplayer` DTOs, redacted secrets/limits/protocol tests,
and additive Cargo/docs/selector wiring. L2 does not start until the conflicting command,
AI, casting, selection, and movement regions land or a manifest amendment remaps every
symbol.

The direct-transport audit also found that the pinned `wtransport 0.6.1` convenience
verifier hashes the complete leaf-certificate DER while Aeronet names its helper as an
SPKI fingerprint conversion. The foundation may model the fixed-size fingerprint but may
not bind it to that verifier until the explicit SPKI-versus-leaf-DER amendment in the
manifest is ratified. This is an L1 dispatch blocker, not permission to enable dangerous
certificate validation.
