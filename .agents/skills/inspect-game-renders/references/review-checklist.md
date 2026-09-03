# Hex Game Render Review Checklist

Apply only the rows relevant to the changed surface, but inspect every frame at full
resolution before using the contact sheet.

## Freshness and coverage

- Exact commit or dirty-state identifier, scenario, seed, source revision, camera, view,
  time, phase, logical canvas, and device scale are recorded.
- The whole-map frame contains every edge of the intended footprint with enough margin to
  recognize that it is complete.
- Every required camera, landmark, azimuth, cutaway, and lighting state exists.
- Different matrix entries are actually different where expected; hashes and visible
  framing expose accidental duplicates.

## Ground, supports, and seams

- No void halo, exposed underside, unsupported column, or gap to the foundation.
- Cross-chunk and cross-biome seams are continuous.
- Route, bridge, stair, cliff, and summit connections remain legible.
- No coincident coplanar surfaces or duplicate caps are visible.
- Face ownership did not erase legitimate roofs, decks, ledges, or cave boundaries.

## Materials and effects

- Materials match the intended terrain and remain readable under each required light.
- Translucent and emissive geometry has plausible static layering.
- Liquids meet banks and supports without cracks, clipping, or stray faces.
- Fog, cutaways, and illumination overlays reveal only the intended regions.
- Any animated, translucent, or emissive surface has a separate native-motion verdict.

## Cameras and composition

- Map view proves coverage rather than merely showing a dramatic crop.
- Character and First Person views do not begin inside geometry or hide the changed region.
- Framing has a clear subject, useful scale, and no accidental obstruction.
- Required alternate azimuths expose surfaces hidden in the hero angle.
- UI or debug overlays do not obscure the evidence unless they are the subject.

## Verdict vocabulary

- `PASS — <specific visible criteria>`
- `FAIL — <defect, location, affected frames>`
- `BLOCKED — <missing/stale/cropped/unreadable evidence>`
- `HUMAN-MOTION-PENDING — <named route and behavior>`

Never promote `UNREVIEWED`, `BLOCKED`, or `HUMAN-MOTION-PENDING` to PASS by omission.
