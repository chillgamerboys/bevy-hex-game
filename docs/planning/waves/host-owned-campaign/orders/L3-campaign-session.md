# L3 / C3 — host-owned Campaign session lifecycle

Read `../manifest.md` and `../maps/session-and-ui.md` completely. Merge the refreshed wave
containing L1 and L2 before implementation review. If refreshed source disagrees with a
banked map, stop and escalate.

## Locked decisions binding this lane

- “The listen host owns simulation, world mutation, and Campaign saves. Clients submit
  intents and render authoritative projections; a client never writes or owns the
  Campaign record.”
- “Campaign saving is manual, host-only, and permitted only during quiescent paused
  exploration. Combat saving remains excluded.”
- “Resuming creates a fresh session instance, lobby, reconnect credentials, and seat
  assignments. It does not assume the same people return.”
- “Legacy and V1 Campaign records remain preserved through the existing strict
  compatibility path and upgrade only after the next successful V2 save; invalid data is
  never silently overwritten.”
- “Campaign persistence contains no Direct, EOS, or Steam transport fact. Direct/LAN and
  future EOS sessions consume the same fresh assignment and authoritative checkpoint
  contracts.”

## Work

Add the V2 save envelope and atomic next-write path, preserve strict legacy/V1 reads, and
orchestrate L1 world plus L2 gameplay export/import without reconstructing their facts.
Add host Campaign slot selection, fresh six-seat assignment lobby handoff, host-only safe
save, client status events, process-restart restore, and typed refusal paths. Ensure no
transport/session secret or prior seat survives persistence.

Do not render UI or edit owner-private world/gameplay implementation. Update only L3's
manifest row.

Run the selector-selected app/lint/docs/shipping closure plus focused process-teardown,
legacy migration, host/client authority, and fresh-assignment tests. Evidence is
logic-only.
