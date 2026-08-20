# L3 — Interior cutaway and feature reconciliation

## Locked decisions

1. **D3:** "The tunnel and Crystal Ascent publish one Dark interior and one light domain; only
   the foot and summit thresholds are exterior entrances, and every required floor is at
   least Dim from paired nonblocking crystal lights."
2. **D5:** "Existing runtime traversal, occupancy, LOS, fog, authored-heart occupancy, camera,
   and save contracts remain authoritative; this wave adds generation and presentation data,
   not parallel gameplay mechanics."

Keep normal Map view opaque and all existing camera behavior.
For review-only cutaway, consume the authored combined interior/roof ownership and hide the
complete route roof. Reconcile overlying features with hidden roof runs so trees never float.
Prove teardown, gameplay re-entry, regeneration, Map/Character/FirstPerson mode changes,
picking, shadows, and unrelated occlusion-reason composition. Do not infer interior or tunnel
geometry from render transforms and do not change generation policy.
