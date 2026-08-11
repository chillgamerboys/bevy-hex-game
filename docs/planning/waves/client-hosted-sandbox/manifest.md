# Client-hosted Sandbox wave

- **Status:** dispatching
- **Wave branch:** `wave/client-hosted-sandbox`
- **Refreshed base:** `origin/dev@1dca1065c7681737ce424fa187879ea31974e356`
- **Dev refresh merge:** `e610e26c50398e43ff23bc4db0890ba7463f11ae`
- **Foundation tip:** `02356b00a5f7a9f26ebe788d8afc45ed58d5baa6`
- **Coordinator:** `@shrav-k`
- **Epic:** user-approved Client-Hosted Multiplayer Epic, 2026-08-08 (`ticket: null`)
- **Outcome:** up to six players can host or join one shipped Sandbox encounter over an
  encrypted direct connection, control exclusive party subsets through the existing
  authoritative command reducer, disconnect/restart/rejoin safely, and return through the
  host-owned outcome flow.
- **Exclusions:** Campaign persistence, Steam lobbies/relay, public matchmaking,
  spectators, split-screen, dedicated servers, WASM, non-Steam relay services, UPnP,
  STUN/TURN, host migration, prediction, rollback, simultaneous allied turns, custom
  content transfer, and combat saving.

## Why this wave exists

The deliverable is meaningful only when session transport, gameplay authority, world
restoration/disclosure, and session UI compose in one host-plus-client runtime. Those
concerns have separate crate authorities and can be built in parallel after a shared
foundation, but no leaf is independently shippable. The shared command, world, app, and
UI seams plus one combined native runtime gate make this a wave rather than independent
or stacked work.

Campaign persistence and Steam are deliberately later release units. They depend on a
proven transport-neutral direct protocol and do not belong in this wave.

## Locked decisions

1. **Human and system seats.** “Human seats are `0..=5`; reserve
   `PlayerSeat(u8::MAX)` for host AI/system commands so the host’s player seat cannot
   command hostile units.”
2. **Assignment invariant.** “The host must control at least one party member. Every
   connected non-spectator must own at least one; the host owns unassigned members by
   default and may redistribute them in the lobby.”
3. **Party movement.** “Group movement includes only characters assigned to the issuing
   seat. `MoveParty` validates every included member, not only its anchor.”
4. **Admission.** “New players enter only before launch. A previously admitted player may
   restart the app and rejoin the active encounter using a private rotating reconnect
   credential.”
5. **Disconnect delegation.** “A disconnected seat remains reserved for 30 real-time
   seconds. It then receives a temporary host delegation; canonical `ControlOwner`
   assignments do not change. Reconnection revokes delegation at the next boundary with
   no command, decision, or movement in flight.”
6. **Host loss.** “A host disconnect ends the session. Clients return to the Multiplayer
   screen with a typed reason.”
7. **Host-only operations.** “Only the host may globally pause, save, launch, retry, kick,
   or close. Client Escape menus are local and non-pausing. Any connected player may issue
   `Rest` through one of their assigned party members.”
8. **Content compatibility.** “Multiplayer supports only shipped content with an exact
   accepted-content fingerprint. Custom Creator content and host content transfer are
   rejected in this epic.”
9. **Tempo.** “Existing combat rules remain unchanged: one global `Mode`, one active turn,
   no split-tempo party, and no simultaneous allied turns.”
10. **Saving.** “Campaign saves remain host-owned, manual, and limited to quiescent paused
    exploration. Combat saving is excluded.”
11. **Authority model.** The listen host owns simulation, AI, world mutation, admission,
    global pause, and saves. Remote clients submit intents and apply disclosure-safe
    authoritative projections; there is no lockstep, rollback, prediction, host migration,
    or dedicated server in this epic.
12. **Command identity.** A wire `GameCommandRequest` contains only `request_id` and
    `command`. The host derives seat/delegation from the authenticated connection and
    caches outcomes by seat/request id; a remote payload can never assert a seat.
13. **World secrecy.** `CombatState` remains host-only. Clients receive exact authorized
    unit/session projections and the existing shared player-faction knowledge view, never
    undisclosed hostile lattice facts.
14. **Initial world.** Every peer generates the static map locally from the frozen
    `SessionManifestV1`, reports the complete public map fingerprint, and activates only
    after exact agreement. Reconnect uses a bounded host snapshot plus deltas newer than
    its baseline sequence.
15. **Direct transport.** Direct hosting uses WebTransport on editable UDP port `7777`, a
    per-session self-signed certificate pinned by SPKI SHA-256, and a redacted versioned
    `HEX1.<base64url>` code carrying advertised endpoint, fingerprint, exact certificate
    expiry, and a 128-bit invite token. Production-unsafe certificate bypass is forbidden.
16. **Reconnect secret.** A rotating 256-bit reconnect token is written atomically to
    temporary application storage with its random session instance, endpoint/SPKI,
    certificate expiry, seat, and player identity. It is never included in `Debug` or
    ordinary logs, and is deleted only for the matching typed closure, expiry, or a
    successful replacement.
17. **Untrusted bounds.** Serialized commands are capped at 64 KiB; decoded strings,
    vectors, paths, and domain values are validated; request bursts are rate limited; and
    snapshot allocation is capped before deserialization.
18. **Offline parity.** Single-player uses the same local request ingress and defaults to
    `SimulationRole::Authority` without opening a socket.
19. **Transport neutrality.** Replicon messages, manifests, snapshots, validation, seat
    rules, and saves are transport-neutral. Steam later supplies identity, invite-only
    lobby discovery, and relay traversal behind the same session interface; Direct Connect
    remains available when Steam is absent.

Decisions are amendable, never silently edited. An amendment records its ratifier and date.

## Amendments

### 2026-08-08 — authorized wave-cut exception

The user explicitly authorized creating the wave branch after the planning artifact and
behavior-neutral foundation had already been completed on
`shrav-k/client-hosted-multiplayer`. The coordinator therefore cut
`wave/client-hosted-sandbox` at exact foundation commit
`02356b00a5f7a9f26ebe788d8afc45ed58d5baa6` rather than first landing those commits on
`dev`. The single draft wave PR carries the additive `origin/dev` merge, planning
artifacts, foundation, and subsequent lanes back to `dev`; no rebase or branch rename is
involved. This explicitly supersedes only the “foundation lands on `dev` before wave cut”
sequencing rule below. It does not waive any lane territory, ownership, merge, or evidence
gate. Ratifier: user; recorded by coordinator.

### 2026-08-08 — preserve SPKI pinning with an audited verifier

The coordinator selected the contract-preserving option for decision 15: L1 will use a
custom native rustls `ServerCertVerifier` that hashes the certificate
`SubjectPublicKeyInfo` and compares it to the connection-code pin. The verifier must also
enforce the certificate validity interval, a maximum 14-day total lifetime, an ECDSA
P-256 public key, and real TLS handshake-signature verification through rustls's supported
algorithms. Tests must prove rejection for a wrong SPKI, expired/not-yet-valid or
overlong-lived certificates, and unsupported key algorithms. Aeronet's no-validation
configuration remains forbidden. Ratifier: coordinator, preserving the user-approved SPKI
contract.

### 2026-08-08 — L2 exact-symbol territory remap

The 2026-08-08 pre-dispatch re-sweep found that draft PRs #186, #188, #189, and
stacked #190 still have their previously recorded heads and have not landed on `dev`.
The coordinator re-cut L2 around their exact symbols instead of editing their active
AI, reducer, casting, selection, movement-feedback, HUD, or save hunks:

- a gameplay-owned pre-reducer adapter extracts only legacy direct commands carrying a
  human seat, discards that untrusted seat, and reissues the command through
  `LocalGameCommandRequest` (authority) or seatless `GameCommandRequest` (replica);
- authenticated commands re-enter `CommandQueue` with an internal correlation marker;
  a post-reducer adapter derives acceptance/refusal from the consumed marker and
  structured `CombatEvent`, so `commands/mod.rs` remains untouched;
- AI commands retain their direct `PlayerSeat::AI` route and the whole existing
  `CombatSystems` chain is placed inside `AuthoritativeSystems`, so `ai.rs` remains
  untouched;
- movement gating is applied to `MovementSystems::Reconcile` from the owning plugin
  registration, without editing PR #188's `movement.rs` reach/feedback regions or
  `selection.rs`;
- group movement is re-cut through the untouched `formation.rs`, `units.rs`, and
  `commands/move_party.rs` symbols; the complete owned subset remains atomically
  validated;
- existing click, cast, lattice, turn, Rest, and party-strip emitters therefore need no
  edits in draft-owned hot files, while no direct human command can reach authority
  reduction in the composed application.

This remap resolves only L2's open-PR dispatch condition. It does not waive L3's
world-owner decision or any combined evidence gate. Ratifier: coordinator, preserving
the user-directed 2026-08-08 instruction to proceed through L2–L4.

### 2026-08-09 — L4 exact-symbol remap and shared lobby-control injection

The L4 pre-dispatch sweep found `origin/dev` unchanged at
`92662d456746506093e8de61f54f1d619085e1fe` and the five draft heads unchanged at
#186 `d3ec9e7b`, #187 `244b6b7b`, #188 `3234f060`, #189 `fbf9ee7c`, and stacked
#190 `15102e23`. L4 is re-cut around their exact presentation symbols:

- #186 changes only `publish_hud_view`/`disclosed_actor` and their disclosure fixtures in
  `screens/gameplay.rs`; L4 owns the earlier pause/outcome routing and adds separate tests;
- #189 adds only `CreatorIntent::SetTouch` in `hex_ui::model`; L4 appends dedicated
  multiplayer view/intent types without editing the Creator enum;
- #188 and #190 extend `walk.rs` with `HoverTile` and `AssertCameraMode`; L4 does not edit
  the walk runner and uses its existing generic screen/button/capture steps from a new
  `walks/multiplayer_session.ron` fixture;
- none of #186–#190 changes the Main Menu model/renderer/adapter, the new multiplayer
  modules, `screens/mod.rs`, or `gameplay_app.rs` regions owned by L4.

The sweep also found that L1 intentionally exposes host-owned pure lobby mechanics but
does not yet register a seatless client readiness/leave request or a trusted local
host-control ingress for assignment, kick, launch, retry, return-to-lobby, and close.
Allowing L4 to query or mutate admission internals directly would create a second session
authority. Before L4 dispatch, the coordinator therefore injects one transport-neutral
shared control contract into `hex_multiplayer`, its deterministic registration order,
runtime adapters, and the in-memory harness. The injection must derive every remote seat
from `AuthorizedSessionClient`, return typed outcomes, broadcast one canonical
`LobbySnapshot`, and retain secret-redaction and message bounds.

L4 is reclassified from `shared` to `gameplay`: it owns gameplay-facing screen behavior
and adapts that behavior through shared `hex_game`/`hex_ui` presentation files, but crosses
no world-owned boundary. The coordinator injection remains separately shared authority.
This amendment resolves L4's open-PR territory condition only; L3 remains owner-blocked
and L4 remains merge-blocked on L3. Ratifier: coordinator.

After L4's first compile, adding `Screen::Multiplayer` exposed two exhaustive shared-tooling
consumers that were not named by the banked map: the review timeout classifier and visual
walk root validator. The coordinator moved `Screen::Multiplayer` plus those neutral match
arms to the wave in `d942d5c7`; L4 still does not edit `walk.rs`, and the new arm is outside
#188/#190's `HoverTile`/`AssertCameraMode` hunks. This is an additive clarification of the
same remap, not lane territory. Ratifier: coordinator.

Composed L4 work then exposed two more coordinator-owned integration gaps. A host-only
lobby entered `Loading` after accepting the host fingerprint but had no remote
`ClientMapReady` event capable of activating it. The shared authority now activates
immediately when the host is the only claimed seat, while still waiting for every claimed
guest in an ordinary lobby. The first L4 visual-walk fixture also required the generic
screen parser and checked-in script registry to recognize the already-reserved Multiplayer
screen. Commit `943b497c` fixes both without touching #188/#190's camera-step variants;
`f2da0c71` carries the identical L4-owned setup-frame script onto the wave so the combined
all-feature test remains green before the L4 merge. Ratifier: coordinator.

### 2026-08-10 — temporary world authority and generator-neutral restore contract

World owner `trova97` is unavailable for two weeks and asked the team to review and land
their ready work, then place multiplayer on top. The user explicitly delegated temporary
world authority to the coordinator, authorized additive changes/review reconciliation,
and supplied the required exact-head human verdicts. PRs #186–#190 are now represented on
`dev`; the delivery-state reconciliation finished at
`origin/dev@1dca1065c7681737ce424fa187879ea31974e356`. The coordinator merged that exact
`dev` additively into the wave as
`e610e26c50398e43ff23bc4db0890ba7463f11ae`. No Trova source branch was deleted.

The temporary authority ratifies the following L3 contract:

- reconnect and Campaign restore import a complete generator-neutral `WorldSnapshotV1`;
  regenerating `SessionManifestV1` is not a restore path;
- canonical stable-name columns retain every non-air voxel run and the complete published
  tile tuple: exact `TilePos`, integer `RunBottom`, `HexSpan` bit patterns, stable-name
  `SubstanceId`, and integer `Headroom`;
- the snapshot also retains partial voxel health, anchors, interior floors/roof voxels,
  special and biome memberships, traversal blockers, exact view hint, gameplay lights,
  liquid material/flow/downstream state, and current feature/crystal object consequences
  required for rendering, blockers, and edit protection;
- generator plans, private recipe identities, transient `PlannedStructure` descriptors
  whose only runtime consequence is already in voxels, entities, handles, materials,
  cameras, transport state, and hostile knowledge are excluded. The reserved #187
  surface-feature vocabulary remains absent until a live producer exists;
- `PublicWorldFingerprintV1` covers canonical stable-name state and every public semantic
  projection. It is distinct from the generator-owned
  `GenerationReport::map_fingerprint` and changes after authoritative terrain mutation;
- collection limits are derived from twice the largest shipped configured map measurement,
  rounded up to a power of two: radius 77 is 18,019 columns, producing a 65,536-column and
  flat-projection envelope. The radius-four 61-surface authored footprint produces a
  128-surface per-object envelope. Names remain 128 bytes, existing coordinate/level
  bounds remain authoritative, and every live snapshot/delta frame is capped at 64 MiB
  before deserialization.

Before L3 dispatch, the coordinator-owned shared protocol defines canonical snapshot
entries, ordered `WorldDeltaV1` upserts/removals with base/target fingerprints,
`PlayerKnowledgeSnapshotV1`, and `LiveSessionSnapshotV1`. The live payload carries the
manifest, current world, authorized shared-player knowledge, unit/session replicas, and a
baseline equal to both the fixed allocation header and `SessionReplica`. Deltas are
authority-boundary ordered and must apply transactionally and idempotently in L3.

Every manifest/admission/closure now carries a random 128-bit `SessionInstanceId`.
Reconnect storage binds that id to endpoint/SPKI, exact verified certificate expiry,
seat/player identity, and the rotating credential. An unrelated endpoint refusal or
closure cannot delete the retained session. Host/client protocol registration remains one
deterministic order and its golden hash changes with this amendment. Ratifier: user under
the temporary world-owner delegation; recorded and implemented by coordinator.

## Shared foundation

Live contracts this wave builds on:

- **Gameplay:** `GameCommand`, `IssuedCommand`, `CommandQueue`, stable `UnitId`,
  `PlayerSeat`, `ControlOwner`, `SimSeeds`, and the one authority reducer at
  `crates/hex_core/src/commands.rs:41` and `crates/hex_core/src/unit_ids.rs:26`.
- **Gameplay:** the reducer already validates each included `MoveParty` member at
  `crates/hex_combat/src/commands/move_party.rs:46`.
- **World:** stack-safe terrain components/resources, `DamagedVoxels`,
  `GenerationReport::map_fingerprint`, and `TerrainReady` publication at
  `crates/hex_core/src/terrain.rs:57`,
  `crates/hex_core/src/terrain_impact.rs:258`, and
  `crates/hex_map/src/procedural.rs:36`.
- **Shared loader:** `AcceptedContentRevision::fingerprint` at
  `crates/hex_assets/src/content_index.rs:252`.
- **Shared app:** `AppSystems`, `PausableSystems`, `GameplaySetup`, one global `Mode`,
  and `GameplayPhase` at `crates/hex_core/src/app.rs:89` and
  `crates/hex_core/src/app.rs:115`–`231`.

Required behavior-neutral foundation, originally specified to land on `dev` before the
wave cut and now carried at the wave base under the recorded exception:

1. **Joint architecture decision:** declare new `hex_multiplayer` as shared protocol and
   session infrastructure. It may depend on shared domain types but may not query map,
   unit, combat, or perception implementations. Add it to `CLAUDE.md`,
   `docs/architecture.md`, `docs/contracts.md`, and `.config/test-scopes.json`.
2. **Gameplay-owned shared vocabulary:** add `SimulationRole::{Authority, Replica}` and
   `AuthoritativeSystems`; reserve `PlayerSeat::AI == PlayerSeat(u8::MAX)` while keeping
   human seats `0..=5`; add `CommandRequestId` and `LocalGameCommandRequest` without
   changing offline behavior.
3. **Shared wire vocabulary:** add the versioned protocol/session/replica types, redacted
   secret wrappers, deterministic registration order, structural limits, and protocol
   hash in `hex_multiplayer`. No socket opens merely because its plugin is installed.
4. **World-owned snapshot contract:** the temporary delegated world authority ratified
   the complete fields, generator-neutral presentation consequences, and round-trip
   `PublicWorldFingerprintV1` in the 2026-08-10 amendment. Regeneration alone does not
   meet reconnect or Campaign restore. The shared type is a data contract; only `hex_map`
   exports/imports it.
5. **Coordinator-only dependency/composition:** pin the Bevy-0.19-compatible Replicon and
   Aeronet `0.21` stack, add fail-closed Cargo features/selectors, and keep `steam`
   optional and absent from Milestone A. Root Cargo and plugin composition remain
   coordinator territory.

### Certificate-pin implementation decision

The dependency audit found a concrete mismatch between the locked connection-code
contract and the available safe convenience API. `aeronet_webtransport 0.21.0` exposes
`digest_from_spki_fingerprint`, but its pinned `wtransport 0.6.1` verifier compares the
configured digest with SHA-256 of the complete leaf-certificate DER, not the SPKI bytes.
Passing the documented SPKI digest to that verifier would therefore reject a valid host;
using its disable-verification path remains forbidden.

The transport-neutral foundation retains a typed 32-byte `CertificateFingerprint`. The
amendment above selects an audited custom SPKI verifier that preserves decision 15 and the
built-in verifier's validity, maximum-lifetime, and key-algorithm constraints. L1 must
retain real handshake-signature verification and add negative tests for every constraint.

The 2026-08-10 temporary world-authority amendment is the explicit sign-off required by
item 4. L3 must still stop on any round-trip mismatch, private-information leak, or need
to serialize generator-private state.

## Dispatch queue

```yaml
lanes:
  - id: L1
    title: Session runtime
    order: orders/L1-session-runtime.md
    ticket: null
    authority: shared
    builder: worker
    branch: worker/client-hosted-session-runtime
    owns:
      - crates/hex_multiplayer/src/auth.rs
      - crates/hex_multiplayer/src/direct.rs
      - crates/hex_multiplayer/src/lobby.rs
      - crates/hex_multiplayer/src/runtime.rs
      - crates/hex_multiplayer/src/sequence.rs
      - crates/hex_multiplayer/src/testing.rs
      - crates/hex_multiplayer/tests/direct_session.rs
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L1-row
    dispatch_blockers: []
    merge_blockers: []
    fences: []
    selector:
      concerns: [app, residual, clippy, docs, shipping]
      full: false
    evidence: logic-only
    sizing:
      model: gpt-5.6-sol
      effort: high
    state: merged-to-wave
    pr: 193

  - id: L2
    title: Gameplay authority
    order: orders/L2-gameplay-authority.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: worker/client-hosted-gameplay-authority
    owns:
      - Cargo.lock#hex-game-multiplayer-dependency-edges
      - crates/hex_core/src/commands.rs#direct-versus-authenticated-queue-origin
      - crates/hex_combat/src/authority_host.rs#authority-role-gates
      - crates/hex_combat/src/commands/move_party.rs#per-seat-party-subset
      - crates/hex_combat/src/commands/spell_resolution.rs#authority-role-gates
      - crates/hex_combat/src/effects.rs#replica-projection-and-authority-role-gates
      - crates/hex_combat/src/knowledge.rs#authority-role-gates
      - crates/hex_combat/src/lib.rs#authoritative-system-set-gate
      - crates/hex_combat/src/turns.rs#replica-turn-projection
      - crates/hex_combat/tests/contracts.rs#multiplayer-authority-module
      - crates/hex_combat/tests/contracts/damage.rs#non-host-rest-contract
      - crates/hex_combat/tests/contracts/funnel.rs#per-seat-party-subset-contract
      - crates/hex_units/src/formation.rs#explicit-seat-subset-anchor
      - crates/hex_units/src/lib.rs#formation-and-motion-exports
      - crates/hex_units/src/movement.rs#authority-gate-registration
      - crates/hex_units/src/units.rs#seat-subset-planning-and-motion-projection
      - crates/hex_units/tests/contracts.rs#multiplayer-authority-module
      - crates/hex_anim/src/lib.rs#replica-animation-clock
      - crates/hex_game/Cargo.toml#multiplayer-gameplay-dependencies
      - crates/hex_game/src/lib.rs#multiplayer-gameplay-module
      - crates/hex_game/src/multiplayer_gameplay.rs
      - crates/hex_game/src/screens/mod.rs#multiplayer-gameplay-composition
      - crates/hex_combat/tests/contracts/multiplayer_authority.rs
      - crates/hex_units/tests/contracts/multiplayer_authority.rs
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L2-row
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector:
      concerns: [rules, contracts, simulation, app, clippy, docs, shipping]
      full: false
    evidence: motion-or-feel
    sizing:
      model: gpt-5.6-sol
      effort: high
    state: merged-to-wave
    pr: 194

  - id: L3
    title: World replication and disclosure
    order: orders/L3-world-replication.md
    ticket: null
    authority: world
    builder: worker
    branch: worker/client-hosted-world-replication
    owns:
      - crates/hex_map/Cargo.toml#hex-multiplayer-world-dto-dependency
      - crates/hex_map/src/world_snapshot.rs
      - crates/hex_map/src/grid.rs#snapshot-import-export-and-terrain-deltas
      - crates/hex_map/src/lib.rs#world-snapshot-publication
      - crates/hex_map/src/terrain_damage.rs#snapshot-hydration
      - crates/hex_map/src/procedural_v3/mod.rs#snapshot-internal-reexports
      - crates/hex_map/src/procedural_v3/materialize.rs#generator-neutral-snapshot-adapter
      - crates/hex_map/tests/contracts/world_snapshot.rs
      - crates/hex_perception/Cargo.toml#multiplayer-knowledge-and-visibility-dependencies
      - crates/hex_perception/src/knowledge.rs#player-knowledge-snapshot-hydration
      - crates/hex_perception/src/runtime.rs#multiplayer-player-faction-disclosure
      - crates/hex_perception/src/snapshots.rs#remembered-run-bottom-projection
      - crates/hex_perception/src/lib.rs#multiplayer-knowledge-publication
      - crates/hex_perception/tests/multiplayer_disclosure.rs
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L3-row
    dispatch_blockers: []
    merge_blockers: [L1]
    fences: []
    selector:
      concerns: [map_unit, map_contracts, app, clippy, docs, shipping]
      full: true
    evidence: static-presentation
    sizing:
      model: gpt-5.6-sol
      effort: high
    # Handoff (2026-08-10, temporary world authority ratified by the user):
    # PublicWorldFingerprintV1 covers every canonical world collection under the
    # 524,288-entry envelope, 128-byte stable names, and 64 MiB frame cap. Exact
    # teardown/import passed for Perlin, V1, V2, six V3 configurations, caves, Crystal
    # Ascent, mutation, and partial damage. Replicon observe/withdraw/re-observe passed
    # without hostile lattice disclosure. Static presentation and human experience
    # evidence remain deferred to the exact combined wave head; the user supplied an
    # additional native L3-candidate visual sanity PASS without a retained frame.
    state: merged-to-wave
    pr: 200

  - id: L4
    title: Session UI and application adapters
    order: orders/L4-session-ui.md
    ticket: null
    authority: gameplay
    builder: worker
    branch: worker/client-hosted-session-ui
    owns:
      - crates/hex_gameplay_model/src/multiplayer.rs
      - crates/hex_gameplay_model/src/main_menu.rs#multiplayer-route-only
      - crates/hex_gameplay_model/src/sandbox.rs#multiplayer-entry-and-destination
      - crates/hex_gameplay_model/src/lib.rs#multiplayer-module-export
      - crates/hex_ui/src/multiplayer.rs
      - crates/hex_ui/src/main_menu.rs#fifth-product-route
      - crates/hex_ui/src/model.rs#multiplayer-view-and-intents
      - crates/hex_ui/src/lib.rs#multiplayer-registration-only
      - crates/hex_game/src/screens/multiplayer.rs
      - crates/hex_game/src/screens/main_menu.rs#multiplayer-intent-adapter
      - crates/hex_game/src/screens/mod.rs#multiplayer-plugin-and-screen-teardown
      - crates/hex_game/src/screens/sandbox.rs#multiplayer-host-deployment-handoff
      - crates/hex_game/src/screens/gameplay.rs#multiplayer-pause-and-outcome-routing
      - crates/hex_game/tests/gameplay_app.rs#multiplayer-session-journey
      - walks/multiplayer_session.ron
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L4-row
    dispatch_blockers: []
    merge_blockers: [L1, L2, L3]
    fences: []
    selector:
      concerns: [app, clippy, docs, shipping]
      full: false
    evidence: motion-or-feel
    sizing:
      model: gpt-5.6-sol
      effort: high
    state: dispatched
    pr: 195
```

## Ownership map

The queue paths are verbatim ownership. The only intentional overlap is this manifest:
each builder owns only its own YAML row and merges the latest wave branch into its branch
before updating that row. No builder resolves or rewrites a sibling row.

Coordinator-only hotspots:

- `Cargo.toml`, `Cargo.lock`, `.config/test-scopes.json`, `CLAUDE.md`,
  `docs/architecture.md`, and `docs/contracts.md` during the behavior-neutral foundation.
- `crates/hex_game/Cargo.toml` and `crates/hex_game/src/lib.rs` plugin composition.
- Protocol registration order in `crates/hex_multiplayer/src/protocol.rs` after the
  foundation. Lanes consume it; additions require coordinator injection.

Shared-file composed end states and hotspot rules:

| File | Regions | Composed end state | Hotspot rule |
|---|---|---|---|
| `crates/hex_gameplay_model/src/main_menu.rs` | L4 owns only `MainMenuRoute::Multiplayer` and matching route test arms | Existing Campaign/Sandbox/Tools/Settings behavior is unchanged and Multiplayer is the fifth root route | Refresh after any UI branch; no reflow outside the enum/test arms |
| `crates/hex_game/src/screens/main_menu.rs` | L4 owns Multiplayer intent handling only | Existing four adapters remain byte-for-byte equivalent; new intent selects `Screen::Multiplayer` | Refresh after #186/#190; coordinator checks complete match block |
| `crates/hex_map/src/grid.rs` | L3 owns snapshot import/export and delta hooks only | Ordinary generation/edit/impact lifecycle remains the default; import reaches the same `TerrainReady` projection | Refresh after every world branch; run complete map lifecycle contracts |
| `crates/hex_perception/src/runtime.rs` | L3 owns authorized shared-player disclosure projection only | Existing authoritative publication remains; network visibility consumes its public projection and cannot reconstruct private facts | #186/#190 land or map is amended before dispatch |
| `crates/hex_combat/src/commands/mod.rs` | L2 owns authenticated request correlation/result emission and delegation lookup only | Existing reducer remains the sole legality authority and emits one typed result per request | #186/#189 land or map is amended before dispatch |
| `crates/hex_units/src/selection.rs` | L2 owns seat-filtered local emission only | Presentation selection may inspect replicas; only owned units generate requests | #188 lands or map is amended before dispatch |

Any disagreement between these banked regions and refreshed source is an escalation, not
a builder judgment call.

## Territory

Sweep performed 2026-08-08 after fetching all remotes. The measurement command was
`git diff --numstat origin/dev...origin/<branch>`; PR 190 is stacked on PR 186 but is also
measured against `origin/dev` to expose its complete inherited footprint.

| PR | Branch / base | Measured footprint | Multiplayer relationship | Disposition |
|---|---|---:|---|---|
| #186 visibility | `wave/visibility` / `dev` | 34 files, +3597/−518 | perception, core exports, save, gameplay UI | landed as `3f2f6dc4`; exact-head runtime PASS recorded |
| #187 surface contract | `wave/hex-81-surface-feature-contract` / `dev` | 4 files, +852/−6 | `hex_core` public world contract and boundary docs | landed as `0e14e89d`; verified behavior-neutral |
| #188 movement feedback | `wave/hex-87-movement-feedback` / `dev` | 8 files, +881/−85 | unit movement/selection and gameplay walk | landed as `9267d9f8`; exact-head runtime PASS recorded |
| #189 Heal | `wave/hex-79-heal` / `dev` | 37 files, +3018/−244 | combat authority, save, UI, content identity | landed as `b6ac0455`; targeting repair and exact-head runtime PASS recorded |
| #190 first person | `wave/hex-89-first-person` / `wave/visibility` | 52 files, +5366/−775 | inherited visibility plus world camera and gameplay walk | unique work landed through composed `32577c26`; delivery reconciled at `1dca1065` after exact-head runtime PASS |

No open PR touched the coordinator-owned `crates/hex_multiplayer/**` namespace during
this landing train. Re-sweep immediately before each remaining lane integration.

The post-train re-sweep on 2026-08-10 fetched `origin/dev@1dca1065`, confirmed all five
footprints represented there, and resolved every L3 dispatch blocker through the explicit
temporary-authority amendment above. A complete scan had found no existing multiplayer
ticket, so the deliberately sparse lane `ticket: null` fields remain reconciled.
The same sweep found new PR #196 (`feat/lattice-fusion-gem-sharing@25d0be5d`): its three
lattice files do not overlap L3 or the shared protocol, while its one-line
`crates/hex_game/src/lib.rs` edit touches a coordinator-only composition hotspot. Re-sweep
and compose it if it lands before L4/final integration; it does not block L3 dispatch.

## Integration order

1. Under the recorded user-authorized exception, carry the behavior-neutral foundation
   at wave base `02356b00a5f7a9f26ebe788d8afc45ed58d5baa6`.
2. L1 and L2 are merged. The 2026-08-10 amendment satisfies L3's world decision and all
   five former territory blockers are represented on the refreshed wave.
3. Commit and gate the coordinator-owned session/snapshot protocol amendment, then cut L3
   from that exact wave head.
4. Merge L3. Refresh L4 on the new wave, inspect removed lines, re-plan the selector, and
   run its composed concerns.
5. Merge L4 last, then let the coordinator apply only root Cargo/plugin composition and
   combined fixes.
6. Run the exact-head combined gate before the single wave PR merges to `dev`.

Milestone B (`client-hosted-campaign`) is a fresh wave after A lands. Milestone C is a
two-level Steam stack after Campaign. Neither is injected into this wave.

## Combined acceptance

Automated contracts on the exact combined head must prove:

- serde round trips and one host/client protocol hash for every wire/snapshot type;
- rejection of wrong protocol/build/content/map, invalid/reused credentials, full/closed
  lobbies, duplicate active seats, malformed/oversized payloads, and non-human claims;
- a wire request cannot carry a seat and a remote connection cannot command another seat
  or `PlayerSeat::AI`;
- `MoveParty` contains only issuing-seat members and `Rest` accepts any owned issuing
  party member;
- `WorldSnapshotV1` export → teardown → import reproduces the complete public fingerprint,
  partial damage, anchors/regions, knowledge inputs, and actor footing;
- offline single-player and listen-host commands traverse the same request ingress and
  preserve existing command transcripts/fingerprints;
- untrusted auth, command, and snapshot decoding is bounded and does not panic.

The headless `aeronet_channel` composition drives one host plus six clients through join,
assignment, ready, local map verification, exploration, combat turns and defender choices,
terrain mutation, outcome, retry, and return to title. It compares each client projection
to its authorized host view after every authority sequence. It also covers disconnect at
each authority boundary, 30-second delegation, safe reclamation, client process destruction
and recreation, snapshot/delta catch-up, duplicate retry idempotence, and typed host loss.

The selector-chosen CI-equivalent suite, strict Clippy, docs, dependency/license audit,
shipping build, and macOS/Windows/Linux direct-transport compilation run on the combined
head. Static frames cover Multiplayer home, Host Direct, Join Direct, six-seat lobby,
mismatch refusal, reconnect/delegation, host pause, and client local menu. A named human
records an exact-head PASS using two native processes or machines for movement feel, local
cameras, combat decisions, disconnect/restart/rejoin, outcome, retry, and return to title.
Typed hooks—not pixels—prove all logical claims.

## Integration checkpoints

- L1 merged through PR #193 and is recorded at the wave base before gameplay authority.
- L2 merged through PR #194 at wave commit
  `28aec80e218309586f6763657427ff2b66d8b107`. Combined-diff audit before merge found and
  fixed foreign-seat formation slots entering subset planning, ambiguous same-unit request
  correlation, stale teardown boundaries/request-id reuse, and restarted-client collisions
  with the retained idempotence cache. GitHub merged when auto-merge was requested because
  the repository does not require the full matrix; the already-running exact-head CI is
  therefore still mandatory evidence, and any failure is repaired additively on the wave.
- Final temporary-owner delivery state is represented by
  `origin/dev@1dca1065c7681737ce424fa187879ea31974e356`; the additive wave refresh is
  `e610e26c50398e43ff23bc4db0890ba7463f11ae`. The only forecast conflicts were the
  additive `hex_core` export seam and `hex_units` seat-subset/object-occupancy composition;
  focused core/unit/multiplayer checks passed after resolution.

## Stop conditions

- Authority cannot be gated without moving presentation-only systems behind authority.
- `WorldSnapshotV1` cannot reproduce the complete public world contract ratified by the
  temporary delegated world authority.
- Client exploration requires simulation prediction for acceptable feel.
- Disclosure filtering leaks private combat/lattice facts.
- A refreshed open-PR footprint disagrees with a banked map or creates unowned overlap.
- The protocol requires map, unit, combat, or perception implementation queries from
  `hex_multiplayer`.
- Direct certificate pinning would require Aeronet’s dangerous validation bypass.

On any stop condition, do not improvise in a lane: mark it blocked, bank the evidence, and
amend this manifest after owner review.

## Injection log

- `2f4015a79dcd0cc1b772b1aa5688694ceb5f1462` — coordinator-owned shared
  lobby-control seam required by the 2026-08-09 L4 amendment. Added seatless
  `ClientLobbyRequest`, non-wire `HostSessionControlRequest`, typed
  `SessionControlResult`, one ordered runtime authority path, open-lobby kick/leave,
  exact retry/return/close transitions, protocol hash `9148828228917372281`, and
  in-memory host-plus-six-client coverage. Focused evidence: 43 unit tests + 1 direct
  session contract, strict `hex_multiplayer` Clippy, and warning-denied rustdoc all PASS.
- `d942d5c726db549acd8e92a46afe5ef0a6774f21` — coordinator-owned
  `Screen::Multiplayer` reservation plus exhaustive
  map-review timeout and visual-walk root-classification arms. This keeps the new app
  state and neutral shared-tooling fallout outside L4's lane queue and compiles with every
  feature while preserving #188/#190's active walk hunks.
- `943b497cfb3a6fdeb4080b4f7119d10a6b8266cd` — coordinator-owned composed fix for
  host-only activation plus the generic visual-walk Multiplayer parser/registry. Focused
  evidence: 42 `hex_multiplayer` unit tests, the host-plus-six-client contract, and the
  all-feature walk screen/script validators PASS. `f2da0c71dc0825b822f822c72cd8790186393382`
  mirrors L4's direct-setup script onto the wave so the registry never points at an absent
  file while L4 remains merge-blocked on L3.
- `bd1d0800` — composed-head repair after the landed Heal command vocabulary extended
  `CommandRefusal`: maps occupied/touch/restoration target details to the existing
  disclosure-safe `InvalidTarget` wire reason and locks that privacy boundary with
  regression assertions.
- `25af4c99` — coordinator protocol amendment adding canonical bounded `WorldSnapshotV1`,
  ordered `WorldDeltaV1`, `PlayerKnowledgeSnapshotV1`, allocation-header-bound
  `LiveSessionSnapshotV1`, random `SessionInstanceId`, exact endpoint/SPKI/certificate
  expiry reconnect binding, matching-session deletion, deterministic message order, and
  protocol hash `9839260687359081537`. Exact-head evidence: 59 selector tests; 180 rules;
  93 trajectory; 416 gameplay contracts plus 5 spell-resolution postflight tests; 29
  simulation; 147 app plus 11 UI postflight; 106 map-unit, 440 map-generation, 81
  map-contract, and 923 residual tests; workspace doctests; strict workspace Clippy;
  warning-denied docs; shipping release; formatting; dependency/license policy; deprecated
  UI terminology; and relative links all PASS before L3 dispatch. The residual partition
  includes the all-feature host-plus-six-client direct session contract.
- `50aadfb03e05c8b2db85bba17b50b3ff90a4e6ad` — coordinator-owned protocol repair after
  L3's V3 round-trip exposed that liquid flow is authored once per material run and copied
  to each occupied voxel. The shared untrusted-input boundary now retains coordinate
  bounds and horizontal adjacency while map authority validates downstream topology
  against the live run; the protocol tag and golden hash advance to
  `4042159340786758443`. Ratified under the user's 2026-08-10 temporary world-authority
  delegation. Focused evidence: 47 multiplayer unit tests, the all-feature
  host-plus-six-client direct-session contract, strict all-target/all-feature Clippy, and
  warning-denied rustdoc all PASS.
- `0e146862c223aadc0449a7d69a69f708d21eb7d0` — coordinator-owned bound correction after
  L3 measured the shipped radius-40 Crystal Ascent configuration at 135,739 exact
  interior-roof entries, disproving the banked 65,536 flat-projection estimate. Applying
  the locked twice-largest-measurement/next-power-of-two rule yields a 524,288-entry
  projection and delta envelope; the independent 64 MiB pre-deserialization frame cap is
  unchanged. The protocol tag and golden hash advance to `4077301579023059970`.
  Ratified under the user's 2026-08-10 temporary world-authority delegation. Focused
  evidence: the 47-test multiplayer suite, all-feature host-plus-six-client direct-session
  contract, strict all-target/all-feature Clippy, and warning-denied rustdoc all PASS.
- `d1d71b89` — coordinator-owned selector classification for the new map-owned
  `world_snapshot.rs` seam. A standalone change to that adapter now selects the complete
  gate without relying on unknown-path fallback, because it crosses generation,
  publication, multiplayer protocol, reconnect, and disclosure consumers. All 60 selector
  regression tests PASS, and the exact path reports every concern with no unknown files.

## Close-out

Not started. At landing, retain the manifest as the durable outcome record, delete spent
orders/maps, record the exact `dev` SHA and named runtime sign-off, close/retarget lane PRs,
and remove the wave/source branches only after no open PR uses them as a base.
