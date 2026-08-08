# L4 order — Session UI and application adapters

## Objective

Add the fifth Main Menu route and the complete Direct Host/Join, six-seat lobby,
verification, reconnect, host/client pause, and outcome flows. Compose the three earlier
lanes without introducing another session authority or leaking secrets.

Read the manifest and maps. Do not start until the L4 dispatch blockers are true; do not
merge until L1/L2/L3 have landed on the wave. If refreshed source differs, escalate.

## Locked decisions (verbatim)

- “The host must control at least one party member. Every connected non-spectator must own
  at least one; the host owns unassigned members by default and may redistribute them in
  the lobby.”
- “New players enter only before launch. A previously admitted player may restart the app
  and rejoin the active encounter using a private rotating reconnect credential.”
- “A disconnected seat remains reserved for 30 real-time seconds. It then receives a
  temporary host delegation; canonical `ControlOwner` assignments do not change.
  Reconnection revokes delegation at the next boundary with no command, decision, or
  movement in flight.”
- “A host disconnect ends the session. Clients return to the Multiplayer screen with a
  typed reason.”
- “Only the host may globally pause, save, launch, retry, kick, or close. Client Escape
  menus are local and non-pausing. Any connected player may issue `Rest` through one of
  their assigned party members.”
- “Multiplayer supports only shipped content with an exact accepted-content fingerprint.
  Custom Creator content and host content transfer are rejected in this epic.”
- “Direct hosting uses WebTransport on editable UDP port `7777`, a per-session self-signed
  certificate pinned by SPKI SHA-256, and a redacted versioned `HEX1.<base64url>` code
  carrying advertised endpoint, fingerprint, and a 128-bit invite token.
  Production-unsafe certificate bypass is forbidden.”
- “A rotating 256-bit reconnect token is written atomically to temporary application
  storage, is never included in `Debug` or ordinary logs, and is deleted when the session
  ends.”

## Banked maps

- `../maps/app-and-session-ui.md`
- `../maps/authority-and-command.md`
- `../maps/world-and-disclosure.md`
- `../maps/territory.md`

## Owned territory

Only L4 paths/regions and its manifest row. The coordinator owns root Cargo/plugin
composition and protocol registration. L4 consumes L1–L3 views/intents; it does not reach
into transport internals, `CombatState`, `VoxelMap`, or reducer state.

## Required implementation

1. Add `Screen::Multiplayer`, `MainMenuRoute::Multiplayer`, and exactly one fifth root
   action “Multiplayer”. Preserve Campaign/Sandbox/Tools/Settings behavior and back/focus.
2. Implement pure model transitions and immutable views for Multiplayer home, Host Direct,
   Join Direct, lobby, loading verification, mismatch/refusal, reconnect/delegated, and
   session-ended states.
3. Host Direct reuses existing shipped Sandbox map/roster/deployment screens. At confirm,
   reject Creator content, freeze `SessionManifestV1`, explicitly start direct hosting,
   then show only a redacted/share-intent connection code surface.
4. Join Direct validates a bounded `HEX1` code locally, starts one explicit connection,
   stores any received reconnect secret only through the credential store, and renders
   typed refusal categories without echoing secrets.
5. Render six stable seats with connection/reservation/delegation, assignments, ready,
   host badge, and launch summary. Allocate lowest free seat; only host may assign/kick/
   launch. Assignment changes clear affected readiness.
6. Disable Launch until every connected seat has at least one member, host has at least
   one, every non-host is ready, and all peers report exact map/content/build/protocol
   verification. New admission closes atomically with launch.
7. On reconnect, render reserved/delegated/catching-up states and keep local camera/
   selection disposable. On host loss/kick/close, clean network/session/credentials and
   return to Multiplayer with a typed reason.
8. Route Escape/Resume by role. Offline/listen host controls global `Pause`; remote client
   opens/closes a local overlay without changing `Pause` or authority systems.
9. At outcome, expose Retry Exact, Return to Lobby, and Close Session only to host.
   Returning to lobby retains frozen session setup but clears readiness; clients render
   host action progress and may leave.
10. Include precise direct-connect help: UDP port, forwarding, CGNAT limitation, no relay,
    and Steam as the later traversal option. Do not imply a join-code service exists.
11. Add structural focus/layout tests and a deterministic visual-walk route for every
    required frame. Never use a screenshot to infer session logic.

## Required evidence

- Pure model matrix for all host/client actions, forbidden transitions, readiness reset,
  launch gate, reconnect/host-loss, retry/lobby/close, and secret redaction.
- Headless app journey from fifth root route through Host Direct and Join Direct into
  gameplay and back; client attempts at host-only actions remain typed no-ops/refusals.
- Host Escape pauses globally; client Escape leaves authority running and only changes its
  local menu.
- Static visual frames: Multiplayer home, Host Direct, Join Direct, six-seat lobby,
  mismatch, reconnect/delegation, host pause, client local menu.
- Selector-chosen app/UI/shipping checks. Motion, input feel, local cameras, interpolation,
  and reconnect experience are deferred to the named exact-head combined runtime PASS.

## Handoff

Refresh from the wave after L1/L2/L3, perform the composed-state audit, and update only the
L4 manifest row to `in-review`. Report typed journey results, visual frame paths, unresolved
manual checks, and every coordinator composition change required. The coordinator alone
records `merged-to-wave`.
