# Map render inspection checklist

Use this checklist to choose views and inspect the resulting pixels. It is deliberately
symptom-oriented: several important renderer failures produce clean logs.

## Coverage and freshness

- Confirm the scenario, seed, Git head, and changed assets match the candidate.
- Record any review-only camera-radius scale and do not mistake that framing for shipped
  camera behavior.
- Reject an overview unless the complete map boundary is visible with margin on every side.
- Confirm all expected chunks and landmark silhouettes appear; look for rectangular or axial
  gaps that suggest failed chunk publication.
- Reject captures copied from an earlier build, proxy, seed, camera pose, or pre-fix run.

## Ground and structures

- Inspect entrances, aprons, bridges, stairs, cliffs, supports, landmark bases, and biome seams
  from a low angle and from the reverse side.
- If a feature anchor itself produces a buried or clipped camera, recapture it with the
  review-only anchor look-at offset. Keep separate Character and First Person frames at safe
  route positions so free-camera evidence is never mistaken for shipped-camera proof.
- Look for empty space below floors, void halos, isolated black wedges, paper-thin stairs,
  detached skirts, floating trees or props, buried objects, and unexpected straight coarse-cell
  borders.
- Inspect tall or contracting structures from below, midway, and above; a top-down frame alone
  hides missing foundations and roofs.
- Check edited/chunk-boundary areas for duplicated surfaces, missing faces, stale geometry, and
  visible rebuild boundaries.

## Authored composition and terrain language

- Put the approved sketch or written composition beside the full-footprint render. Verify
  landmark positions, route directions, openings, water flow, and biome adjacency explicitly;
  a technically complete map with the wrong composition fails.
- Compare the important vertical silhouettes from at least two low azimuths. Check that enclosing
  ridges actually hide what they are meant to hide, dominant peaks remain dominant, central crests
  are not accidentally peripheral, and landmarks do not protrude above their intended enclosure.
- Reject procedural mountains that read as isolated cones, pyramids, cylinders, or repeated stamps
  when the intent is a connected ridge or massif. Look for shared shoulders, varied ridgelines,
  nonuniform slopes, saddles, and a coherent base.
- Trace every authored watercourse visually from source to outlet. A waterfall needs a clear lip,
  vertical drop, and receiving basin; a river needs intentional lateral bends and changing context,
  not merely correct blue cells along a straight corridor.
- Inspect secret or recessed entrances from the ordinary approach and from both oblique sides.
  Reject surface trenches, material stripes, straight guide-lines, or silhouette gaps that reveal
  a route intended to be concealed.
- Read vegetation and snow as gradients rather than cell labels: dense low growth should taper
  through foothills, trees should stop below exposed high terrain, and connected high-altitude
  surfaces should share a coherent snow language unless an authored magical exception is explicit.
- Inspect scenic exceptions in context. A magical garden island, oasis, or landmark must visibly
  contrast with its surroundings while its supports, trees, materials, and access still look
  intentional from multiple heights.

## Liquids, translucent, and emissive content

- Confirm water sits visibly below its banks and does not read as terrain painted blue.
- Orbit past water edges, crystal faces, fog caps, and other close surfaces to expose z-fighting,
  transparency sorting errors, and one-frame disappearance.
- Inspect repeated lights and emissive props for flicker, inconsistent exposure, floating light
  sources, and lighting that vanishes at chunk or domain boundaries.

## Visibility, cutaway, and cameras

- Compare ordinary opaque Map view with any full cutaway; roofs, props, fog, and vegetation must
  disappear and return together without floating remnants.
- Check Map, Character, and First Person modes. Look for near-plane clipping, camera entry into
  terrain, shaded geometry visible from an impossible angle, lost focus, and a changed FOV.
- Capture both sides of doors, tunnel mouths, walls, steps, and vertical transitions. Include
  enough sky or void behind the silhouette to expose missing geometry.
- For illumination diagnostics, confirm the overlay aligns with surfaces and does not alter the
  underlying gameplay presentation after teardown.

## Motion pass

- Walk and orbit slowly across every changed seam and around repeated geometry for at least one
  complete camera arc.
- For nonintrusive workstation review, use the enforced seed-exact sequence
  `ClickAnchor` → `CaptureWhileMoving` → `AwaitPartyIdle` → matching `AssertSelectedAt`.
  Inspect every numbered PNG in order and as a contact sheet; the completed review index must
  list the entire sequence.
- Choose `every_frames` densely enough that a one-frame disappearance cannot hide between
  samples, but keep the exact `capture_count` short enough that movement remains pending through
  the last request. A sequence that ends early is a failed route plan, not partial evidence.
- Watch for flickering blocks, z-fighting, chunk popping, stale fog or cutaway entities, liquid
  phase jumps, visibility oscillation, and camera collision breathing.
- Revisit the same route in reverse with a different explicit prefix. Direction-dependent
  artifacts are failures, and partial files from an aborted sequence must not be reviewed.
- If an anomaly appears, preserve a short clip or exact reproduction route and inspect the
  relevant frames individually.
- Frame sequences can clear discrete renderer defects without opening a native window. They do
  not establish input feel, smoothness between sampled frames, or camera comfort; retain a named,
  explicitly approved live pass for those experiential claims.

## Report vocabulary

- `PASS`: the artifact visibly covers its named criterion and has no observed defect.
- `FAIL`: a concrete defect, stale artifact, inadequate framing, or incomplete coverage exists.
- `BLOCKED`: capture could not be produced or inspected; never treat this as a pass.
- `HUMAN-MOTION-PENDING`: static artifacts passed, but no durable or named live motion pass has
  occurred.
