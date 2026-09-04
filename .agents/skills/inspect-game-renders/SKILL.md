---
name: inspect-game-renders
description: Capture and independently inspect fresh Hex Game renders for visible defects. Use after significant changes to map generation, terrain or chunk rendering, materials, liquids, fog, cutaways, lighting, props, vegetation, camera framing, or presentation batching; use before showing a changed map to the user or calling presentation work review-ready.
---

# Inspect Game Renders

Treat pixels as a required presentation test, not decoration. Run this workflow on the
composed candidate after each materially different render. Any later relevant code or
asset change makes the prior visual verdict stale.

## Preserve the evidence boundary

- Use typed tests and runtime hooks for geometry, connectivity, occupancy, lighting tiers,
  picking, determinism, and other world facts.
- Use stills to judge visible coverage, framing, seams, holes, materials, lighting,
  cutaways, occlusion, and composition.
- Use recorded motion or a named live pass to judge flicker, z-fighting, popping, camera
  clipping, animated materials, and temporal visibility. Never clear motion with stills.

## Plan the smallest complete matrix

Read [references/map-render-checklist.md](references/map-render-checklist.md) for map,
terrain, liquid, landmark, or camera work. Before launching, record:

1. exact Git head, scenario, seed, changed presentation surfaces, and review anchors;
2. the authored reference, approved sketch, or written composition contract, with its
   non-negotiable silhouettes and spatial relationships listed in plain language;
3. one full-footprint Map frame with every world boundary visible;
4. close Map, Character, and First Person frames at each materially changed region;
5. at least two useful azimuths for entrances, seams, tall landmarks, or layered geometry;
6. cutaway and illumination diagnostics when those adapters or interiors changed;
7. a motion route through repeated, translucent, emissive, liquid, chunk-boundary, or
   near-coplanar geometry.

Do not accept a cropped overview, a distant landmark hidden by terrain, or several frames
that all show the same face as complete coverage.

## Capture fresh renders

Run source builds through Cargo. Never launch a bare `target/*/hex_game` binary. Put every
run in a new exact-head directory under `.context/`; do not overwrite an older approval
pack.

For a user-requested visible launch, do not shorten the wait by reusing a previously built
`target/*/hex_game`. Launch through Cargo from the exact worktree being described, record the
current head plus dirty digest in the launch log, and confirm startup emits no missing-font or
missing-asset errors before telling the user it is ready. If the current tree cannot build or
its selected world fails setup, report that blocker; never substitute an older playable binary
and call it current.

An approval pack must come from one committed, clean candidate. Record the full output of
`git rev-parse HEAD`, require both tracked and untracked source state to be clean before the
run, and derive the new directory name from that revision. A dirty-worktree capture is useful
scratch evidence only: label it `UNAPPROVABLE-DIRTY` and recapture after commit. Before the
first launch, fail if the exact-head output directory already exists; an old directory with the
same short SHA is never a freshness check.

Automated evidence must not interrupt someone using the workstation. The repository's
`HEX_REVIEW_CAPTURE` path and paired `HEX_WALK_SCRIPT`/`HEX_WALK_OUT` path render to image
targets through a windowless schedule runner; use those paths for routine inspection. Never
launch a visible native game window, activate the application, or start a screen recording
unless the user explicitly asks to play or has agreed to a live motion review. If the headless
path regresses, stop the capture batch and fix it instead of falling back to repeated visible
launches.

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
`HEX_REVIEW_CUTAWAY=full`, and `HEX_REVIEW_ILLUMINATION=overlay` as applicable.
When an exact feature anchor is a poor or obstructed place to stand, use the review-only
free camera instead of accepting a clipped frame: set `HEX_REVIEW_CAMERA=map`,
`HEX_REVIEW_LOOK_AT_ANCHOR=<anchor>`, and
`HEX_REVIEW_LOOK_AT_OFFSET=x,y,z`. The finite world-space offset is applied from the exact
rendered anchor surface, `HEX_REVIEW_VIEW` can rotate it deterministically, and the harness
logs the resolved seed-exact `TilePos`. This path does not move actors or exercise shipped
Character/First Person collision, so label it as feature-composition evidence rather than
gameplay-camera evidence.
For a wider low-angle context frame without changing gameplay camera settings, combine
`HEX_REVIEW_CAMERA=character` with `HEX_REVIEW_CHARACTER_RADIUS_SCALE=<1..=20>`.
Do not use that review-only scale to judge the shipped camera distance.
Use a relevant script under `walks/` with the `visual-walk` feature when movement and
multi-stop coverage are already authored. A mechanically completed capture is still
unreviewed.

For frame-discrete motion evidence, put `CaptureWhileMoving` immediately after the exact
`ClickAnchor` that starts the route:

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
contract shown above; runtime sampling watches the selected actor specifically, so unrelated
party motion cannot authorize evidence. Keep `capture_count <= 48`,
`every_frames * capture_count <= 900`, and all such sequences in one walk at or below 192
files; the parser rejects larger plans. Movement must become pending within eight frames and
remain pending through the final request. Early completion, a black/write failure, a filename
collision, or an interrupted run fails the walk, removes that sequence's exact partial files,
and leaves `review-index.md` incomplete. Even a fully written sequence remains provisional
until the contiguous `AwaitPartyIdle` and matching `AssertSelectedAt` succeed; either failure
removes the sequence instead of indexing it. Choose the count to sample the suspect portion.

Calibrate `every_frames` from the seed-exact endpoints and shipped movement speed; never leave
it at `1` merely because that is dense. At the fixed 60 Hz timestep, use horizontal hex distance
times `HEX_SMALL_DIAMETER / speed` as a conservative earliest-arrival bound. Keep the final
request at least eight updates before that bound, and make each direction span at least half of
it so the paired reverse leg samples the other end. Elevation and detours only increase the
arrival margin; use the authored centerline or measured path when claiming complete spatial
coverage. If the 900-frame per-sequence cap prevents meaningful coverage, split the route at a
stable intermediate anchor rather than presenting the opening fraction as whole-route evidence.

Temporal sequences deliberately reuse the continuously rendered image target so sampling
does not pause movement to replace it. They do not alter ordinary `Capture` or
`ReviewCapture`: each later acceptance still receives a fresh target, four settle frames, and
the existing structural stale-evidence guard. Exercise each suspected surface in both
directions. Dense offscreen sequences can clear visible flicker, z-fighting, and popping;
native control feel or camera smoothness still requires an explicitly approved live pass.
Report `HUMAN-MOTION-PENDING` for that experiential remainder rather than taking over the
workstation.

## Inspect independently

Prefer a fresh-eyes agent that did not implement the change. Give it the raw frames or
video, scenario/seed, and changed surface list—not the expected verdict.

1. Open every still at original resolution.
2. Verify the overview includes the complete footprint before judging local composition.
3. Compare the overview and local views directly with every authored spatial relationship;
   report mismatches even when geometry is internally valid and renders without artifacts.
4. Inspect each close view against the checklist and record concrete observations.
5. Scan a contact sheet for systematic framing, missing-chunk, palette, or exposure errors.
6. Scrub or replay motion at normal speed and frame-by-frame around any flicker.
7. Mark each artifact `PASS` or `FAIL`; never infer a pass from capture count or clean logs.

If the change is visually meaningful and no independent agent is available, perform the
first pass yourself and leave the final verdict explicitly awaiting fresh eyes.

## Close the loop

Any hole, void halo, floating object, coplanar flicker, missing chunk, truncated overview,
wrong cutaway, severe camera clipping, stale artifact, or unexplained temporal change fails
the review. Fix the narrow root cause, add a typed invariant when possible, then recapture
the affected matrix on the new exact head.

Report the exact head, scenario/seed, commands, output directory, artifact list, motion
method, per-artifact verdicts, findings, and remaining human checks. Do not call the map
review-ready while any required view is missing or failed.
