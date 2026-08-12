# C3/C4 shared session and UI source map

Banked at `origin/dev@a0f95e62d02c663902b864cc08a89e831d9ba437`. If refreshed source
disagrees with this map, escalate rather than guessing.

## Current symbols and dispositions

| Anchor | Current role | Disposition |
|---|---|---|
| `crates/hex_game/src/save.rs:332` `CampaignsFile` | Three-slot V1 document | Introduce a versioned decode envelope that preserves V1 records and writes V2 only after successful validation/atomic replacement. |
| `crates/hex_game/src/save.rs:384` `CampaignStore` | Parsed records plus sticky refusals | Keep three-slot behavior and invalid-record preservation; add typed saving/resume state without transport facts. |
| `crates/hex_game/src/save.rs:531` `migrate_legacy` | One-time pre-Campaign import | Keep strict and non-destructive. Legacy translates through V1 before a later successful V2 save. |
| `crates/hex_game/src/save.rs:1078` `handle_campaign_intents` | Local New/Continue flow | Preserve single-player; add an explicit multiplayer-host selection path rather than overloading client intent. |
| `crates/hex_game/src/save.rs:1317` `save_exploration` | Host/local safe-save gate and atomic write | Retain the quiescent paused-exploration predicate; add listen-host role enforcement and client status projection. |
| `crates/hex_gameplay_model/src/multiplayer.rs:8` `MultiplayerRoute` | Direct setup/lobby/loading/reconnect routes | Add Campaign host/browser/resume routes without creating an EOS route yet. |
| `crates/hex_gameplay_model/src/multiplayer.rs:77` `MultiplayerModel` | Pure route/role/seat behavior | Extend with renderer-free Campaign transitions and Back semantics. |
| `crates/hex_ui/src/model.rs:1203` Campaign slot views | Local Campaign slot cards | Reuse presentation shape where possible; add typed multiplayer availability/save status rather than embedding save records. |
| `crates/hex_ui/src/model.rs:1345` `MultiplayerView` | Direct/lobby immutable projection | Add Campaign mode/slots/save progress fields; retain Direct/LAN fields and redaction. |
| `crates/hex_ui/src/model.rs:1413` `MultiplayerIntent` | Typed Direct/lobby actions | Add host-only Campaign select/resume/save actions; no intent carries a path or persistence document. |
| `crates/hex_ui/src/multiplayer.rs:168` onward | Multiplayer screen hierarchy | Add Host Campaign entry, slot browser, saving feedback, resume lobby, and typed refusal copy. Direct/LAN stays visible under Advanced. |
| `crates/hex_game/src/screens/multiplayer.rs:423`/`:465` | Prepared-host handoff and intent adapter | Generalize prepared session origin to Sandbox or Campaign while keeping host controls local and clients seatless. |
| `crates/hex_game/src/screens/multiplayer.rs:1997` `publish_view` | Disclosure-safe immutable view | Project only sanitized Campaign status and never checkpoint data, credentials, or identities. |
| `walks/multiplayer_session.ron` | Existing Direct/lobby presentation walk | Extend with Campaign browser, saving, resume lobby, and refusal frames after L3 is live. |

## Shared-file hotspot rules

- L3 refreshes after L1 and L2, owns the save document and runtime orchestration, and
  calls the two owner adapters. It does not duplicate their validation.
- L4 refreshes after L3 and edits only immutable view/intent/render regions in
  `save.rs`/`screens/multiplayer.rs`. If it needs a new logical state, L3 must publish a
  typed status first; L4 may not infer it from frame timing or file presence.
- `hex_game/src/lib.rs` is L2's one-line module hotspot. Open PR #196's one-line logging
  edit is preserved by merging current `dev` into the lane before review.

## Required end state

The host chooses a Campaign slot, enters a fresh six-seat lobby, assigns the restored
party, and launches through the same transport-neutral session protocol. During active
play only the host sees an enabled Save action at a valid boundary; clients receive
non-authoritative “host is saving” and result status. Resume never restores seats,
credentials, cameras, selections, or transport state.
