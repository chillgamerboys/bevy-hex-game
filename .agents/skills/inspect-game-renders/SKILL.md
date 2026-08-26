---
name: inspect-game-renders
description: Capture and independently inspect fresh Hex Game renders for visible defects. Use after significant changes to map generation, terrain or chunk rendering, materials, liquids, fog, cutaways, lighting, props, vegetation, camera framing, or presentation batching; use before showing a changed map to the user or calling presentation work review-ready.
---

# Inspect Game Renders

Treat pixels as a required presentation test. A successful capture command proves only
that frames were produced; it does not prove that those frames are complete, current, or
visually correct.

Read `references/review-checklist.md` and the visual-evidence cases in
`docs/development/problem-solving-casebook.md` before reviewing.

## Evidence Boundary

- Typed logic, snapshots, and deterministic contracts prove map/world state.
- Fresh stills prove only visible static presentation: framing, geometry, seams,
  materials, lighting, cutaways, and composition.
- Native motion/video and human checks prove animation, flicker behavior, camera response,
  control feel, and taste.

Do not use a still to clear motion, or pixels to infer exact world logic.

## 1. Define the review matrix

Record the exact commit or dirty-state identifier, scenario, seed, authored-source
revision, and changed presentation surfaces. Include at minimum:

- one whole-footprint overview that visibly contains the complete map;
- Map, Character, and First Person views when those cameras can expose the change;
- close views of every changed region or landmark;
- at least two azimuths for seams, supports, or translucent surfaces;
- relevant lighting times, liquid phases, cutaways, or illumination overlays; and
- a named native-motion route for emissive, translucent, animated, or camera-sensitive
  geometry.

A cropped “whole map” image is a failed matrix entry, not a partial pass.

## 2. Capture fresh exact-head evidence

Launch source builds through Cargo as required by `AGENTS.md`. Create a new, uniquely
named directory under `.context/` for the exact head and matrix; never overwrite an old
approved pack in place.

Use the repository's `map-review` capture hooks for deterministic single frames and
`visual-walk` for scripted routes. Record the commands, scenario, seed, view, logical
canvas, device scale, capture time, and completion status. Hash every matrix file, and
reject unexpected identical hashes across different matrix entries.

At run start, invalidate any prior review index. A complete command exit means the
mechanical capture set exists; it is still `UNREVIEWED` until independent inspection.

## 3. Inspect in two scales

Inspect every full-resolution frame individually, then inspect one contact sheet covering
the entire matrix. Do not rely on thumbnails alone.

For each frame, use `references/review-checklist.md` and write a specific verdict:

- `PASS`: named visible criteria are present and no relevant defect is seen.
- `FAIL`: name the defect, location, and affected matrix entries.
- `BLOCKED`: the frame is stale, incomplete, cropped, unreadable, or cannot prove the
  requested criterion.
- `HUMAN-MOTION-PENDING`: static review is complete but the required native motion route
  has not been judged.

Have a fresh-eyes reviewer challenge the full-resolution notes and contact sheet before
selecting hero images. A generic repeated PASS note is not evidence.

## 4. Close defects through the right invariant

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

## Stop Conditions

- The executable was launched directly from `target/` rather than through Cargo.
- The captured state is not the state being claimed.
- The overview crops the footprint or a required camera/region is missing.
- A capture command passed but the pixels have not been inspected.
- A still is being used to prove flicker-free motion, controls, or exact world state.
