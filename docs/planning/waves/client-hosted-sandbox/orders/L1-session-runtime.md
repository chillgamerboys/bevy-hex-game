# L1 order — Session runtime

## Objective

Build the transport-neutral session authority plus direct encrypted transport inside the
shared `hex_multiplayer` crate. Deliver custom admission, lobby state, ordered authority
sequencing, request idempotence, reconnect credentials, and an in-memory multi-app harness.
Do not implement gameplay/world authority or rendered UI.

Read `../manifest.md` and every map it names before editing. The order is banked against
`origin/dev@92662d456746506093e8de61f54f1d619085e1fe`. If reality disagrees with a map,
mark L1 blocked and escalate; do not improvise.

## Locked decisions (verbatim)

- “Human seats are `0..=5`; reserve `PlayerSeat(u8::MAX)` for host AI/system commands so
  the host’s player seat cannot command hostile units.”
- “New players enter only before launch. A previously admitted player may restart the app
  and rejoin the active encounter using a private rotating reconnect credential.”
- “A disconnected seat remains reserved for 30 real-time seconds. It then receives a
  temporary host delegation; canonical `ControlOwner` assignments do not change.
  Reconnection revokes delegation at the next boundary with no command, decision, or
  movement in flight.”
- “A host disconnect ends the session. Clients return to the Multiplayer screen with a
  typed reason.”
- “Multiplayer supports only shipped content with an exact accepted-content fingerprint.
  Custom Creator content and host content transfer are rejected in this epic.”
- “A wire `GameCommandRequest` contains only `request_id` and `command`. The host derives
  seat/delegation from the authenticated connection and caches outcomes by seat/request id;
  a remote payload can never assert a seat.”
- “Direct hosting uses WebTransport on editable UDP port `7777`, a per-session self-signed
  certificate pinned by SPKI SHA-256, and a redacted versioned `HEX1.<base64url>` code
  carrying advertised endpoint, fingerprint, and a 128-bit invite token.
  Production-unsafe certificate bypass is forbidden.”
- “A rotating 256-bit reconnect token is written atomically to temporary application
  storage, is never included in `Debug` or ordinary logs, and is deleted when the session
  ends.”
- “Serialized commands are capped at 64 KiB; decoded strings, vectors, paths, and domain
  values are validated; request bursts are rate limited; and snapshot allocation is capped
  before deserialization.”
- “Single-player uses the same local request ingress and defaults to
  `SimulationRole::Authority` without opening a socket.”

## Banked maps

- `../maps/authority-and-command.md`
- `../maps/world-and-disclosure.md`
- `../maps/territory.md`

## Owned territory

Only the L1 paths and L1 manifest row listed in the queue. Root Cargo, lockfile, selector,
protocol registration order, and `hex_game` composition are coordinator-only. Do not add a
map/unit/combat/perception dependency to `hex_multiplayer`.

## Required implementation

1. Install one deterministic protocol plugin using the foundation's stable registration
   list. Register ordered client/server messages and replicated projections exactly once.
2. Implement pure admission validation for protocol hash, exact build identity, exact
   shipped-content fingerprint, frozen scenario/map identity, lobby phase, capacity,
   invite token, reconnect token, duplicate active seat, and token rotation/reuse.
3. Maintain six stable lobby seats, lowest-free initial allocation, host identity,
   assignment/readiness state, launch close, disconnect reservation, delegation eligibility,
   and typed close/refusal reasons. L1 owns mechanics, not UI policy duplication.
4. Convert an authorized connection and `GameCommandRequest` into an authenticated
   request context. Never accept seat or delegation from serialized input.
5. Allocate a monotonic `authority_sequence`, cache final results by canonical
   `(PlayerSeat, CommandRequestId)`, and return the cached `Duplicate` outcome for retries
   without re-enqueueing authority work.
6. Represent authority boundaries explicitly. Delegation activates after 30 real-time
   seconds; reclamation waits until no request/decision/movement is in flight and takes
   effect before the next command.
7. Encode/decode direct connection codes with strict version, endpoint, port, fixed-size
   fingerprint, and token validation. `Debug`/logs are redacted. Hostname/IP text has a
   small bound and no control characters.
8. Generate a per-session self-signed certificate, derive/publish its SPKI SHA-256 pin, and
   configure WebTransport validation to require that pin. Do not enable any dangerous
   certificate-verification bypass.
9. Persist reconnect credentials through an injected atomic temporary-storage adapter;
   tests use memory/temp storage. Rotate after successful reconnect and delete at session
   end.
10. Add explicit byte/allocation/rate/domain limits before data reaches reducers. Decode
    errors close or refuse the request without panic or large allocation.
11. Add `aeronet_channel` host/client harness helpers capable of advancing several Bevy
    apps deterministically. Keep actual sockets default-off unless Host Direct or Join
    Direct explicitly starts one.

Do not add Steam code, public discovery, NAT traversal, custom-content transfer, prediction,
rollback, or a dedicated server.

## Required evidence

- Unit/property tests for every auth/refusal/secret/code/limit/sequence transition.
- Serde round trips and golden protocol hash/registration-order test.
- Compile-time/serde proof that `GameCommandRequest` has no seat field.
- Duplicate/reconnect test proving one authority ingress for repeated request id.
- Host-plus-six-client in-memory lobby test covering capacity, readiness, launch closure,
  disconnect reservation, delegation eligibility, rotation, reconnect, and host close.
- Fuzz/property-style arbitrary bytes for auth, direct code, command envelope, and bounded
  snapshot header with no panic.
- Selector-chosen L1 concerns. Evidence classification is logic-only; no screenshot or
  human-motion claim is valid for this lane.

## Handoff

Update only the L1 queue row to `in-review` and record the PR targeting
`wave/client-hosted-sandbox`. Report protocol hash, dependency versions, focused commands,
and any API deviation. The coordinator alone records `merged-to-wave`.
