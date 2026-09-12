---
name: inspect-game-renders
description: Capture and independently inspect fresh Hex Game renders for visible defects. Use after significant changes to map generation, terrain or chunk rendering, materials, liquids, fog, cutaways, lighting, props, vegetation, camera framing, or presentation batching; use before showing a changed map to the user or calling presentation work review-ready.
---

# Inspect Game Renders

Treat pixels as a required presentation test. A successful capture command proves only
that frames were produced; it does not prove that those frames are complete, current, or
visually correct.

Review the composed candidate. Any later code or asset change that can affect a reviewed
surface makes the affected verdict stale.

Read the [review checklist](references/review-checklist.md), the
[map render checklist](references/map-render-checklist.md) for map, terrain, liquid,
landmark, or camera work, and the
[visual-evidence cases](../../../docs/development/problem-solving-casebook.md) before
reviewing.

## Evidence Boundary

- Typed logic, snapshots, and deterministic contracts prove map/world state.
- Fresh stills prove only visible static presentation: framing, geometry, seams,
  materials, lighting, cutaways, and composition.
- Native motion/video and human checks prove animation, flicker behavior, camera response,
  control feel, and taste.

Do not use a still to clear motion, or pixels to infer exact world logic.

## 1. Define the review matrix

Record the full commit SHA—or, for diagnostic scratch only, a dirty-state identifier—plus
the scenario, seed, authored-source revision, and changed presentation surfaces. For
authored work, name the approved sketch or written composition contract and list its
non-negotiable silhouettes and spatial relationships before capture. Include at minimum:

- one whole-footprint overview that visibly contains the complete map;
- Map, Character, and First Person views when those cameras can expose the change;
- close views of every changed region or landmark;
- at least two azimuths for seams, supports, or translucent surfaces;
- relevant lighting times, liquid phases, cutaways, or illumination overlays; and
- a named native-motion route for emissive, translucent, animated, or camera-sensitive
  geometry.

A cropped “whole map” image is a failed matrix entry, not a partial pass.

## 2. Capture fresh exact-head evidence

Launch source builds through Cargo as required by [AGENTS.md](../../../AGENTS.md). Create a
new, uniquely named directory under `.context/` for the exact head and matrix; never
overwrite an old approved pack in place.

An approval pack must come from one committed candidate whose tracked and untracked source
state is clean. Derive its new output directory from the full HEAD and matrix identifier,
and fail before launch if that exact directory already exists. A dirty-state capture is
scratch evidence only: label it `UNAPPROVABLE-DIRTY` and recapture after committing.

Do not open, activate, or focus a visible native game window or start screen recording
unless the user explicitly requested play or approved a named live review. If a
noninteractive capture path is unavailable or regresses, stop and report the review as
blocked instead of silently substituting a visible launch.

For a user-requested visible launch, do not shorten the wait by reusing a previously built
`target/*/hex_game`. Launch through Cargo from the exact worktree being described, record
the current head plus dirty digest in the launch log, and confirm startup emits no
missing-font or missing-asset errors before telling the user it is ready. If the current
tree cannot build or its selected world fails setup, report that blocker; never substitute
an older playable binary and call it current.

Automated evidence must not interrupt someone using the workstation. The repository's
`HEX_REVIEW_CAPTURE` path and paired `HEX_WALK_SCRIPT`/`HEX_WALK_OUT` path render to image
targets through a windowless schedule runner; use those paths for routine inspection.

For immediate deterministic frames, use the repository's `map-review` harness:

```sh
CARGO_INCREMENTAL=0 \
CARGO_PROFILE_RELEASE_DEBUG=0 \
HEX_REVIEW_SCENARIO='<scenario>' \
HEX_REVIEW_SEED=<seed> \
HEX_REVIEW_CAPTURE="$PWD/.context/map-review/<short-sha>/<name>.png" \
HEX_REVIEW_VIEW=top-down \
HEX_REVIEW_CAMERA=map \
cargo run --release -p hex_game --features 'test-support map-review'
```

Use `HEX_REVIEW_FOCUS_ANCHOR`, `HEX_REVIEW_CAMERA=character|first-person`,
`HEX_REVIEW_VIEW=default|rotated|counter-rotated|rear|top-down`,
`HEX_REVIEW_CUTAWAY=full`, and `HEX_REVIEW_ILLUMINATION=overlay` as applicable. When an
exact feature anchor is a poor or obstructed place to stand, use the review-only free
camera instead of accepting a clipped frame: set `HEX_REVIEW_CAMERA=map`,
`HEX_REVIEW_LOOK_AT_ANCHOR=<anchor>`, and `HEX_REVIEW_LOOK_AT_OFFSET=x,y,z`. The finite
world-space offset is applied from the exact rendered anchor surface,
`HEX_REVIEW_VIEW` can rotate it deterministically, and the harness logs the resolved
seed-exact `TilePos`. This path does not move actors or exercise shipped Character/First
Person collision, so label it as feature-composition evidence rather than gameplay-camera
evidence. For a wider low-angle context frame without changing gameplay camera settings,
combine `HEX_REVIEW_CAMERA=character` with
`HEX_REVIEW_CHARACTER_RADIUS_SCALE=<1..=20>`; do not use that review-only scale to judge
the shipped camera distance.

Use a relevant script under `walks/` with the `visual-walk` feature when movement and
multi-stop coverage are already authored. Record the commands, scenario, seed, view,
logical canvas, device scale, capture time, and completion status. Hash every matrix file,
and reject unexpected identical hashes across different matrix entries.

Before a run, mark any prior review index stale or use a fresh directory so an aborted
rerun cannot leave old approval authoritative. A complete command exit means the mechanical
capture set exists; it is still `UNREVIEWED` until independent inspection.

## 3. Capture frame-discrete motion

Put `CaptureWhileMoving` immediately after the exact `ClickAnchor` that starts the route:

```ron
ClickAnchor(
    name: "route_destination",
    expected: (q: 0, r: 0, level: 16), // replace from the seed-exact route manifest
),
CaptureWhileMoving(
    prefix: "feature-forward-motion",
    every_frames: 3,
    capture_count: 24,
),
AwaitPartyIdle(max_frames: 1200),
AssertSelectedAt(expected: (q: 0, r: 0, level: 16)),
```

This requests exactly `capture_count` image-target PNGs at fixed runner-frame intervals,
named `<prefix>-0001.png` onward. The parser requires the complete four-step
`ClickAnchor` → `CaptureWhileMoving` → `AwaitPartyIdle` → matching `AssertSelectedAt`
contract shown above; runtime sampling watches the selected actor specifically, so
unrelated party motion cannot authorize evidence. Keep `capture_count <= 48`,
`every_frames * capture_count <= 900`, and all such sequences in one walk at or below
192 files; the parser rejects larger plans. Movement must become pending within eight
frames and remain pending through the final request. Early completion, a black/write
failure, a filename collision, or an interrupted run fails the walk, removes that
sequence's exact partial files, and leaves `review-index.md` incomplete. Even a fully
written sequence remains provisional until the contiguous `AwaitPartyIdle` and matching
`AssertSelectedAt` succeed; either failure removes the sequence instead of indexing it.

Calibrate `every_frames` from the seed-exact endpoints and shipped movement speed; never
leave it at `1` merely because that is dense. At the fixed 60 Hz timestep, use horizontal
hex distance times `HEX_SMALL_DIAMETER / speed` as a conservative earliest-arrival bound.
Keep the final request at least eight updates before that bound, and make each direction
span at least half of it so the paired reverse leg samples the other end. Elevation and
detours only increase the arrival margin; use the authored centerline or measured path
when claiming complete spatial coverage. If the 900-frame per-sequence cap prevents
meaningful coverage, split the route at a stable intermediate anchor rather than
presenting the opening fraction as whole-route evidence.

Temporal sequences reuse the continuously rendered image target so sampling does not pause
movement to replace it. They do not alter ordinary `Capture` or `ReviewCapture`: each later
acceptance still receives a fresh target, four settle frames, and the structural
stale-evidence guard. Exercise each suspected surface in both directions. Dense offscreen
sequences can clear visible flicker, z-fighting, and popping; native control feel or camera
smoothness still requires an explicitly approved live pass. Report
`HUMAN-MOTION-PENDING` for that experiential remainder rather than taking over the
workstation.

## 4. Inspect in two scales

Inspect every full-resolution frame individually, then inspect one contact sheet covering
the entire matrix. Do not rely on thumbnails alone.

For each frame, use the [review checklist](references/review-checklist.md) and write a
specific verdict:

- `PASS`: named visible criteria are present and no relevant defect is seen.
- `FAIL`: name the defect, location, and affected matrix entries.
- `BLOCKED`: the frame is stale, incomplete, cropped, unreadable, or cannot prove the
  requested criterion.
- `HUMAN-MOTION-PENDING`: static review is complete but the required native motion route
  has not been judged.

Give a fresh-eyes reviewer the raw frames or video, scenario and seed, and changed-surface
list—not an expected verdict. Have that reviewer challenge the full-resolution notes and
contact sheet before hero selection. If no independent reviewer is available, perform only
a preliminary pass and leave final approval explicitly pending. A generic repeated PASS
note is not evidence.

## 5. Close defects through the right invariant

When a frame reveals a defect, locate the typed or rendering invariant behind it. Add or
strengthen the narrow automated regression, implement the repair, and recapture every
affected matrix entry from the new exact state.

Examples:

- A base gap needs a support/foundation invariant, not only a prettier camera.
- Flicker between coincident faces needs deterministic face ownership, not blanket face
  removal that can erase legitimate roofs or decks.
- A duplicate screenshot needs capture provenance and uniqueness checks, not reuse as a
  second viewpoint.

Static recapture can confirm visible geometry. Only the named native-motion route can
close flicker, animation, or camera-feel claims.

## Completion Gate

Presentation is review-ready only when the exact-head matrix is complete, each frame has
specific independent notes, the full-resolution and contact-sheet passes agree, failed
entries were recaptured after repair, and every required motion route is explicitly passed
or left visibly pending. A pending route may make the static pack ready for motion review,
but it prevents an overall presentation PASS and every flicker-free, animation, or
camera-feel claim.

Report the full head, scenario and seed, commands, output directory, artifact names and
hashes, motion method, per-artifact verdicts and findings, and every remaining human check.

## Stop Conditions

- The executable was launched directly from `target/` rather than through Cargo.
- The captured state is not the state being claimed.
- The overview crops the footprint or a required camera/region is missing.
- A capture command passed but the pixels have not been inspected.
- A still is being used to prove flicker-free motion, controls, or exact world state.
