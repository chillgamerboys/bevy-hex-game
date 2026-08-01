# Camera presentation

The gameplay camera has two modes with separate authority:

- **Map** is a free pan/orbit view framed by the generated `MapViewHint`.
- **Character** follows the selected `CameraFocusTarget` and adapts its effective pose
  to public world geometry without changing player-authored yaw or desired zoom.

This is presentation only. Camera geometry never grants sight, changes picking
legality, brightens darkness, or becomes a gameplay occupancy fact.

## Boundary

`hex_world` cannot depend on `hex_map` or `hex_units`. Character collision therefore
builds a cached index exclusively from public `HexTile`, `TilePos`, and `HexSpan`
components. The index preserves every stacked run at one `HexCoord`, sorts spans
canonically, rebuilds only when a published tile is added, changed, or removed, and is
cleared on gameplay exit.

The horizontal candidate ring grows with the configured probe radius. Authored probes
are bounded to `0.0 < radius <= 2.0` world units; an invalid runtime value fails closed
instead of shrinking the candidate set and tunnelling through terrain.

No physics engine or private voxel storage participates. Terrain edits refresh the
same public projection before the next Character-camera resolution.

## Desired and effective pose

`PanOrbitCamera.radius` is always the player's desired zoom. Character collision keeps
an independent effective radius, desired rotation, and effective pitch:

1. Sweep a conservative camera probe from focus along the requested yaw and pitch.
2. Retract immediately before the first expanded public hex prism, retaining the
   configured collision margin.
3. If same-yaw retraction falls below the configured preferred minimum radius, test
   the complete bounded set of progressively higher pitches, then test back toward
   the horizon so low cave ceilings can retain a readable radius. The pitch with the
   greatest true clearance wins; equal clearances prefer the smallest deviation from
   the player-authored pitch, with the upward-first search order as the final stable
   tie-break. Obstructions that leave the preferred minimum intact retract without an
   automatic pitch change.
4. Restore radius and pitch smoothly after clearance, settling exactly inside small
   hysteresis bands.

The probe expands all six prism faces and the vertical span, conservatively enclosing
the close camera's near plane around faces, corners, bridge undersides, and cave
ceilings. The exact `CameraFocusTarget.surface` and coplanar floor tangencies are
ignored while the probe exits upward; a wall, ceiling, raised surface, or different
run genuinely overlapping the probe is an immediate hit. Validation keeps the probe
radius no larger than the focus height, so the configured sweep cannot begin inside
ordinary supporting terrain.

The shipped defaults in `camera.ron` are:

| Setting | Value |
|---|---:|
| probe radius | `0.4` world units |
| collision margin | `0.35` world units |
| preferred minimum effective radius | `1.5` world units |
| adaptive maximum pitch | `0.75` quarter-turn fractions |
| pitch search step | `0.05` quarter-turn fractions |
| radius restoration | `8.0` world units/second |
| pitch restoration | `0.8` quarter-turn fractions/second |

Manual orbit input updates the desired rotation. Automatic collision never searches
or commits a different yaw. Switching back to Map restores the exact saved map pose.

## Trees and interiors

Generated trees publish one exact stack-safe root. `hex_objects` copies
`TreeOccluder(root)` and an opaque `TreeFadeAmount` to every trunk, branch, foliage,
and canopy render chunk. `hex_world` intersects the final camera-focus corridor with
transformed chunk bounds; one blocking chunk fades every chunk at that exact root to
20%, holds for 0.2 seconds after clearance, then restores over 0.3 seconds.

Material authority stays in `hex_objects`. It lazily clones each actively fading
tree's shared source materials, participates in OIT while those clones are blended,
and restores the exact handles before deleting the clones. A neighboring tree using
the same catalog style is never mutated. Authored `CanopyOccluder` metadata remains a
separate art boundary and does not create camera behavior by itself.

Ordinary gameplay never removes cave roofs. Those roof runs remain visible collision
geometry, allowing the adaptive camera to stay inside a tight interior. Only explicit
`map-review` tooling may install the full-cutaway override, which hides the complete
roof of the selected exact `InteriorRegionId` for one deterministic capture.

## Ordering and lifecycle

Character follow runs in `PostUpdate` after unit animation and before transform
propagation. Camera-driven presentation then uses the shared order:

`ResolveCameraOcclusion → ApplyMaterials → ApplyVisibility`

Tree intersection observes final propagated camera/object transforms, renderer-owned
material changes settle before composed visibility, and fog/review reasons remain
independent. Gameplay exit clears collision indexes, adaptive pose state, fade
timelines, temporary material clones, and OIT ownership.

Focused tests cover prism faces/corners and stacked spans, player-yaw preservation,
immediate adaptation and damped recovery, a synthetic flat radius-55 lower-level
benchmark, a 2,048-render-chunk tree-fade performance gate, 10,000 unchanged frames,
whole-tree/material isolation, review-only roofs, and 100 gameplay lifecycles. An
ignored release composition diagnostic generates the pinned shipped Two Rings
scenario, builds and repeatedly rebuilds the camera index from its public
`HexTile`/`TilePos`/`HexSpan` projection, and keeps steady Character collision below
1 ms p95 across its exact published anchors and six yaws.

The tracked route manifest pins all 15 selectable Map scenarios to their exact seed
and representative stack-safe destinations. Every standalone selectable Map has an
executable multi-azimuth Character walk using ordinary pointer movement and bounded
party-idle waiting, followed by an exact check that the selected unit's authoritative
footing and the camera-focus surface both equal the requested destination. Sky Islands
exercises only its reachable ordinary ground bridge.

Five seed-pinned Two Rings groups cover one ordinary-network destination in all 19
regions, require at least two captured azimuths after exact selected-unit and
camera-focus proof, and keep each review card to at most ten frames. The woodland group
restarts the same exact scenario and follows the Waterfall A/Frozen Hills detour because
the direct route legitimately enters combat; it never suppresses that combat.

Standalone and Two Rings upper Sky Island surfaces remain flight-gated, so the
evidence proves only their grounded bridges. Human motion/readability review remains
an explicit presentation gate; the harness does not invent movement capabilities or
treat static frames as play-feel approval. Review cards use route-readable open-side
azimuths; deliberately blocked yaws remain collision/yaw-preservation test cases and
must be swept during the human gate to confirm immediate safe retraction and smooth
recovery after the player rotates clear.
