# Client-hosted Sandbox wave

- **Status:** dispatching
- **Wave branch:** `wave/client-hosted-sandbox`
- **Base:** `origin/dev@92662d456746506093e8de61f54f1d619085e1fe`
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
    `HEX1.<base64url>` code carrying advertised endpoint, fingerprint, and a 128-bit invite
    token. Production-unsafe certificate bypass is forbidden.
16. **Reconnect secret.** A rotating 256-bit reconnect token is written atomically to
    temporary application storage, is never included in `Debug` or ordinary logs, and is
    deleted when the session ends.
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
  `crates/hex_assets/src/content_index.rs:235`.
- **Shared app:** `AppSystems`, `PausableSystems`, `GameplaySetup`, one global `Mode`,
  and `GameplayPhase` at `crates/hex_core/src/app.rs:54`.

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
4. **World-owned review draft:** the world owner must explicitly ratify the fields and
   round-trip fingerprint of `WorldSnapshotV1` before L3 dispatch, including whether
   stable generator-neutral presentation consequences are snapshotted (recommended) or
   regenerated. Regeneration alone does not meet the Campaign snapshot contract. The
   shared type is a data contract; only `hex_map` exports/imports it.
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

The current user approval ratifies the product behavior, but it is not recorded as the
separate world-owner sign-off required by item 4. That sign-off remains a dispatch
condition for L3.

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
    dispatch_blockers:
      - PRs 186, 188, 189, and stacked PR 190 have landed or every overlapping symbol is remapped in an amended manifest
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
      - crates/hex_map/src/world_snapshot.rs
      - crates/hex_map/src/grid.rs#snapshot-import-export-and-terrain-deltas
      - crates/hex_map/src/lib.rs#world-snapshot-publication
      - crates/hex_map/tests/contracts/world_snapshot.rs
      - crates/hex_perception/src/runtime.rs#multiplayer-player-faction-disclosure
      - crates/hex_perception/tests/multiplayer_disclosure.rs
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L3-row
    dispatch_blockers:
      - explicit world-owner ratification of WorldSnapshotV1 fields and complete public fingerprint
      - PRs 186, 187, and stacked PR 190 have landed or every overlapping symbol is remapped in an amended manifest
    merge_blockers: [L1]
    fences: []
    selector:
      concerns: [map_unit, map_contracts, app, clippy, docs, shipping]
      full: true
    evidence: static-presentation
    sizing:
      model: gpt-5.6-sol
      effort: high
    state: queued
    pr: null

  - id: L4
    title: Session UI and application adapters
    order: orders/L4-session-ui.md
    ticket: null
    authority: shared
    builder: worker
    branch: worker/client-hosted-session-ui
    owns:
      - crates/hex_gameplay_model/src/multiplayer.rs
      - crates/hex_gameplay_model/src/main_menu.rs#multiplayer-route-only
      - crates/hex_gameplay_model/src/lib.rs#multiplayer-module-export
      - crates/hex_ui/src/multiplayer.rs
      - crates/hex_ui/src/main_menu.rs#fifth-product-route
      - crates/hex_ui/src/model.rs#multiplayer-view-and-intents
      - crates/hex_ui/src/lib.rs#multiplayer-registration-only
      - crates/hex_game/src/screens/multiplayer.rs
      - crates/hex_game/src/screens/main_menu.rs#multiplayer-intent-adapter
      - crates/hex_game/src/screens/mod.rs#multiplayer-plugin-and-screen-teardown
      - crates/hex_game/tests/gameplay_app.rs#multiplayer-session-journey
      - walks/multiplayer_session.ron
      - docs/planning/waves/client-hosted-sandbox/manifest.md#L4-row
    dispatch_blockers:
      - PRs 186, 188, 189, and stacked PR 190 have landed or every overlapping symbol is remapped in an amended manifest
    merge_blockers: [L1, L2, L3]
    fences: []
    selector:
      concerns: [app, clippy, docs, shipping]
      full: false
    evidence: motion-or-feel
    sizing:
      model: gpt-5.6-sol
      effort: high
    state: queued
    pr: null
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
| #186 visibility | `wave/visibility` / `dev` | 34 files, +3597/−518 | perception, core exports, save, gameplay UI | blocks L2/L3/L4 regions until landed or remapped |
| #187 surface contract | `wave/hex-81-surface-feature-contract` / `dev` | 4 files, +852/−6 | `hex_core` public world contract and boundary docs | blocks WorldSnapshotV1 ratification/remap |
| #188 movement feedback | `wave/hex-87-movement-feedback` / `dev` | 8 files, +881/−85 | unit movement/selection and gameplay walk | blocks L2/L4 movement regions |
| #189 Heal | `wave/hex-79-heal` / `dev` | 37 files, +3018/−244 | combat authority, save, UI, content identity | blocks L2/L4 authority/UI regions |
| #190 first person | `wave/hex-89-first-person` / `wave/visibility` | 52 files, +5366/−775 | inherited visibility plus world camera and gameplay walk | blocks L2/L3/L4 until stack lands or exact regions are remapped |

No open PR touches the new `crates/hex_multiplayer/**` namespace. Re-sweep immediately
before foundation landing, wave creation, every lane dispatch, and integration.

The pre-dispatch re-sweep on 2026-08-08 fetched and pruned `origin`, found
`origin/dev` unchanged at `92662d456746506093e8de61f54f1d619085e1fe`, and found PRs
186–190 at the same heads, bases, and measured footprints. L1 remains uncontested. The
listed L2–L4 overlap blockers remain active. A complete scan of non-completed Hex Game
Linear work found no existing multiplayer ticket, so the deliberately sparse lane
`ticket: null` fields remain reconciled.

## Integration order

1. Under the recorded user-authorized exception, carry the behavior-neutral foundation
   at wave base `02356b00a5f7a9f26ebe788d8afc45ed58d5baa6`; retain explicit
   world-owner `WorldSnapshotV1` agreement as an L3 dispatch condition.
2. Dispatch L1 immediately. Dispatch L2–L4 only when their territory blockers are true;
   merge blockers do not serialize their construction.
3. Merge L1 first. Run the selector-chosen composed-tree checks.
4. Merge L2 and L3 in either order. After each merge, refresh the other branch, inspect
   removed lines, re-plan the selector, and run its composed concerns.
5. Merge L4 last, then let the coordinator apply only root Cargo/plugin composition and
   combined fixes.
6. Run the exact-head combined gate before the single wave PR targets `dev`.

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

## Stop conditions

- Authority cannot be gated without moving presentation-only systems behind authority.
- `WorldSnapshotV1` cannot reproduce the complete public world contract or lacks explicit
  world-owner agreement.
- Client exploration requires simulation prediction for acceptable feel.
- Disclosure filtering leaks private combat/lattice facts.
- A refreshed open-PR footprint disagrees with a banked map or creates unowned overlap.
- The protocol requires map, unit, combat, or perception implementation queries from
  `hex_multiplayer`.
- Direct certificate pinning would require Aeronet’s dangerous validation bypass.

On any stop condition, do not improvise in a lane: mark it blocked, bank the evidence, and
amend this manifest after owner review.

## Injection log

- None.

## Close-out

Not started. At landing, retain the manifest as the durable outcome record, delete spent
orders/maps, record the exact `dev` SHA and named runtime sign-off, close/retarget lane PRs,
and remove the wave/source branches only after no open PR uses them as a base.
