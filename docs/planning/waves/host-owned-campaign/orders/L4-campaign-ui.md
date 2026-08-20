# L4 / C4 — multiplayer Campaign UI

Read `../manifest.md` and `../maps/session-and-ui.md` completely. Merge the refreshed wave
containing L3 before implementation review. If refreshed source disagrees with a banked
map, stop and escalate.

## Locked decisions binding this lane

- “The listen host owns simulation, world mutation, and Campaign saves. Clients submit
  intents and render authoritative projections; a client never writes or owns the
  Campaign record.”
- “Campaign saving is manual, host-only, and permitted only during quiescent paused
  exploration. Combat saving remains excluded.”
- “Resuming creates a fresh session instance, lobby, reconnect credentials, and seat
  assignments. It does not assume the same people return.”
- “A Campaign checkpoint contains no reconnect or invite credential, online/store
  identity, transport endpoint or entity id, camera, local UI state, or selection.”

## Work

Add Host Campaign browsing, slot selection, save status, client “host is saving” feedback,
resume lobby, and typed refusal surfaces to the existing Multiplayer hierarchy. Preserve
Direct/LAN as a visible advanced path. Extend pure models, immutable views, intents, UI
tests, and the multiplayer visual walk. UI reads only L3's typed status and never infers
save success from pixels, time, or disk presence.

Do not create save/session authority or inspect checkpoint data. Update only L4's manifest
row.

Run the selector-selected app/lint/docs/shipping closure and automated presentation walk.
Static frames prove hierarchy/layout only; defer interaction feel and the named human
`PASS` to the combined wave candidate.
