# Camera presentation

The gameplay camera has two modes with separate authority:

- **Map** is a free pan/orbit view framed by the generated `MapViewHint`.
- **Character** follows the selected `CameraFocusTarget`; the player exclusively owns
  look direction and desired zoom. A deterministic upward-look composition keeps
  ordinary free-look above the supporting floor, while terrain may shorten only the
  rendered boom along that placement ray.

This is presentation only. Camera geometry never grants sight, changes gameplay
targeting legality, brightens darkness, or becomes an occupancy fact. A unit hidden
by the near-camera presentation envelope is also ignored by Bevy picking until it is
shown again; that suppression does not alter gameplay command authority.

## Character motion regression contract

Human motion review of the first adaptive candidate (`9bbaddf`) found defects that
the endpoint screenshot suite could not expose:

| Observed defect | Cause in the first candidate | Required regression guarantee |
|---|---|---|
| The perspective and distance visibly changed while the selected unit walked. | A stateless collision search could choose a different pitch every frame, while any improved radius clearance began restoring immediately. Small terrain changes therefore produced alternating pitch and in/out boom motion. | Following an unobstructed moving unit preserves the exact player-authored rotation and its deterministic placement offset. Collision may retract only the effective radius; blocked frames never move it outward, and release requires continuous full clearance before one monotonic recovery. |
| Upward look stopped at some positions or was immediately undone. | Character pitch was limited to a downward-only arc, and the same-frame collision pass could replace a manual pitch with its own searched pitch. | Character input spans straight up through straight down. A follow/collision pass never writes rotation, including at either vertical pole or while the boom is fully retracted. |
| A fully retracted unusual-angle view could be inside the selected character. | At straight-up look, the supporting floor correctly limits the downward boom to zero; the shipped focus point is inside the current character mesh. | When terrain forces the eye inside the selected character's near-camera envelope, camera-owned presentation occlusion suppresses that character until the monotonic boom recovery carries the eye clear. The reason composes with fog and other visibility owners and is removed on retarget, mode exit, and gameplay exit. |
| Orbiting could suppress zoom input. | Orbit and wheel handling were mutually exclusive within one frame. | A simultaneous right-drag and wheel event authors both rotation and desired radius. |
| Automated review reported green despite the motion defects. | Scripted captures were taken after movement settled and proved route coverage, not temporal continuity or control feel. | Headless regressions cover frame sequences and exact authority; final acceptance also requires a native motion review, not still images alone. |

These are behavior contracts, not tuning preferences. Rotation always matches player
input exactly. For ordinary upward free-look, the placement ray may lag that authored
pitch by at most `15°`; this keeps the long third-person boom above the supporting floor
and the character near the lower portion of the frame. Beyond the first `15°` of upward
look, the placement ray tracks the authored pitch with that fixed composition offset,
while contact with the supporting floor progressively shortens the boom into the close
view. This placement is a pure function of player look, so walking cannot change it.
Terrain then changes position only along that placement ray: it retracts immediately,
remains stable through changing partial clearance, waits for the configured release
delay after complete clearance, and restores at the configured maximum rate.
`PanOrbitCamera.radius` remains the player's requested zoom throughout.

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

`PanOrbitCamera.radius` is always the player's desired zoom, and the camera
`Transform.rotation` is always the player's authored look direction. Character
collision keeps only an independent effective radius:

1. Derive the placement ray from player-authored rotation. Level and downward views use
   the exact orbit ray. Shallow upward free-look retains a horizontal boom; beyond the
   `15°` composition allowance it progressively tilts down behind the target while the
   view rotation itself remains exact.
2. Sweep a conservative camera probe from focus along that placement ray.
3. Retract immediately before the first expanded public hex prism, retaining the
   configured collision margin. Never rotate the view or overwrite desired zoom.
4. While any obstruction remains, accept only safer inward changes. Improved partial
   clearance cannot move the camera outward or accumulate release time.
5. After the complete desired boom remains clear for the release delay, restore at no
   more than the configured world-units-per-second rate. Recovery is one monotonic
   outward run unless a new obstruction requires an immediate retraction.
6. At or below the configured `character_self_hide_radius` (shipped as `1.0`), add the
   camera-owned composable visibility reason to the selected unit root. Restore the
   unit only beyond the threshold plus exit hysteresis, so a near-first-person view
   remains clear without visibility chatter.

The probe and collision margin expand all six prism faces and the vertical span,
conservatively enclosing the close camera's near plane around faces, corners, bridge
undersides, and cave ceilings. While the placement ray exits level or upward, zero-entry
floor spans at the focus coordinate and its immediate ring are ignored when their top is
at or below the focus and within one voxel level of the selected surface. This prevents
the local support floor from collapsing an otherwise clear tangent boom without
ignoring remote or overhead geometry. A downward boom—produced when the player looks
up—may correctly retract to zero against the supporting floor; close-character
occlusion keeps that fully controlled view usable. Validation keeps the probe radius no
larger than the focus height, so the target point remains outside ordinary supporting
material.

The shipped defaults in `camera.ron` are:

| Setting | Value |
|---|---:|
| probe radius | `0.1` world units |
| collision margin | `0.35` world units |
| release delay | `0.2` seconds |
| radius restoration | `8.0` world units/second |
| self-hide radius | `1.0` world units |
| initial pitch | `0.3` quarter-turn fractions |
| Character pitch arc | `-1.0..=1.0` (straight up through straight down) |

Manual orbit input changes rotation directly, including at both vertical poles. The
full Character arc is a code-level contract rather than a configurable narrower range.
Simultaneous wheel input still changes desired zoom. Switching back to Map restores the
exact saved map pose.

## Trees and interiors

Generated trees publish one exact stack-safe root. `hex_objects` copies
`TreeOccluder(root)` and an opaque `TreeFadeAmount` to every trunk, branch, foliage,
and canopy render chunk. `hex_world` intersects the final camera-focus corridor with
transformed chunk bounds; one blocking chunk fades every chunk at that exact root to
20%, holds for 0.2 seconds after clearance, then restores over 0.3 seconds. A lone
tree retains that exact opacity regardless of how its renderer chunks are split. When
several exact trees intersect the corridor at once, their intersecting chunks share
the 20% opacity budget. This avoids overlapping translucent foliage compounding into
a dark veil over the unit and forward route. The split is a conservative, stable
per-intersecting-chunk multiplier rather than an exact screen-alpha promise: OIT still
composites mesh fragments and whole-tree identity also fades chunks outside the direct
corridor.

Material authority stays in `hex_objects`. It lazily clones each actively fading
tree's shared source materials, participates in OIT while those clones are blended,
and restores the exact handles before deleting the clones. A neighboring tree using
the same catalog style is never mutated. Authored `CanopyOccluder` metadata remains a
separate art boundary and does not create camera behavior by itself.

Ordinary gameplay never removes cave roofs. Those roof runs remain visible collision
geometry, allowing the collision-limited camera to stay inside a tight interior. Only explicit
`map-review` tooling may install the full-cutaway override, which hides the complete
roof of the selected exact `InteriorRegionId` for one deterministic capture.

## Ordering and lifecycle

Character follow runs in `PostUpdate` after unit animation and before transform
propagation. Camera-driven presentation then uses the shared order:

`ResolveCameraOcclusion → ApplyMaterials → ApplyVisibility`

Tree intersection observes final propagated camera/object transforms, renderer-owned
material changes settle before composed visibility, and fog/review reasons remain
independent. Near-character hiding adds and removes only its own composable reason.
Gameplay exit clears collision indexes, effective-radius recovery state, proximity
ownership, fade timelines, temporary material clones, and OIT ownership. Retargeting
also discards the previous unit's collision history and resolves the new unit's own
clear or obstructed corridor in the same frame.

Focused tests cover prism faces/corners and stacked spans, exact player-rotation
authority, both vertical poles, simultaneous orbit/zoom input, 120 open-motion frames,
blocked-clearance chatter, delayed monotonic recovery, proximity occlusion composition,
a clear and obstructed focus retarget, a synthetic flat radius-55 lower-level benchmark,
a 2,048-render-chunk tree-fade
performance gate, 10,000 unchanged frames, whole-tree/material isolation, review-only
roofs, and 100 gameplay lifecycles. An
ignored release composition diagnostic generates the pinned shipped Two Rings
scenario, builds and repeatedly rebuilds the camera index from its public
`HexTile`/`TilePos`/`HexSpan` projection, and keeps steady Character collision below
1 ms p95 across its exact published anchors and six yaws.

The tracked route manifest pins 15 camera-walk Sandbox catalog maps—every entry except
the deployment-only Flat Arena—to their exact scenario seed and representative
stack-safe destinations. Each has an executable multi-azimuth Character walk using
ordinary pointer movement and bounded party-idle waiting, followed by an exact check
that the selected unit's authoritative footing and the camera-focus surface both equal
the requested destination. Sky Islands exercises only its reachable ordinary ground
bridge.

Five seed-pinned Two Rings groups cover one ordinary-network destination in all 19
regions, require at least two captured azimuths after exact selected-unit and
camera-focus proof, and keep each review card to at most ten frames. The woodland group
restarts the same exact scenario and follows the Waterfall A/Frozen Hills detour because
the direct route legitimately enters combat; it never suppresses that combat.

Standalone and Two Rings upper Sky Island surfaces remain flight-gated, so the
evidence proves only their grounded bridges. The harness does not invent movement
capabilities or treat static frames as play-feel approval. Alberto completed the human
motion/readability gate on 2026-08-01 in the shipped release path on Two Rings at
runtime head `2397d8e` and approved the corrected player-controlled camera for merge.
Future camera-behavior changes must repeat a native motion review, including blocked
yaw cases that confirm immediate safe retraction and smooth recovery after the player
rotates clear.
