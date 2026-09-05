# Recovered Hex map fixes and Crystal crown snow

## Integration and source

One stacked follow-up from accepted V3 snapshot `125e2df`, on `fix/hex-six-map-fixes`. The snapshot was supplied by the V3 owner after the user accepted the original water tint at alpha 0.85 and combined voxel choices. Archive SHA-256: `a2c9e75a0220d8a1aad184289396b2d1838dcb0a3ffe16def9c9275f5e8bc155`; 762 recorded files and modes verified. Parent source commit: `bc06a8969532b807ec677928eee304bc28399386`.

Generation edits are sequential under one writer. Independent renderer work and read-only audits share this isolated integration checkout; no concurrent writers to shared generation files. This is one implementation and review unit, not six branches or PRs. No new save, network, or shared gameplay schema is planned. Private generation metadata owns bounded exceptions.

## Scope ledger

| Item | Status | Contract |
|---|---|---|
| Waterfall intake | Candidate authored; focused intake tests passed again in candidate07 | Four existing sections 9→7→5→3; preserve core elevations and downstream cascade; directed drainage for added water. |
| Garden Island | Candidate authored; validation pending | Reserve complete courtyard against roots and crowns; retain surrounding trees; small supported voxel spring connected to upper lake. |
| Natural pass | Candidate authored; independent baseline regression fixtures added | Feather adjacent shoulders; preserve exact walking centerline, grades, width, endpoints, and two-entrance topology. |
| Rear shelf | Context matched; three bounded geometry tests passed in candidate07; full generation pending | Eastern rear apron connected to coarse cell128, outside the Peak ring, dry natural levels121..<200 with neighbor steps≤2. Record the exact contiguous mask before sparse deterministic snowy-tree clusters; preserve routes, water, summits, complete crowns and terrain. |
| River | Candidate authored; final generation and animation review pending | Final lower-lake15→sea8 descent and both bridges; accepted water color/alpha in ordinary gameplay; directional Rapid movement and restrained foam. |
| Tunnel | Candidate authored; shell-side expansion preference pending | Repair the bend, retain floor and tapered joins, add two ceiling voxels, and keep complete crystals clear. Unobstructed sections add two cells per side; beside the immutable Crystal shell, the current eight-wide candidate adds four outward. True symmetric widening would enter two protected shell columns. |
| Crystal Ascent crown snow | Three focused geometry/distribution tests passed in candidate07; full generation/captures pending | Existing summit cap voxels only: bare radius11, full snow radius27 and existing outer summit trail; deterministic radial smoothstep with coherent patches and fine dither. Existing crown trees use accepted snowy-small-broadleaf with unchanged occupied voxels, roots, rotations and blockers. |
| Minor rear peaks | Deferred by approved plan | Optional terrain reconstruction increases scope; sparse trees address the requested empty shelf. |

## Validation and evidence

Focused tests during changes, then repository-selected combined validation from baseline125e2df. Validate final generated geometry, complete vegetation/crystal footprints, water graph and elevations, bridges, access and roof cover. Preserve seed0 overlap and representative-seed contracts. Fingerprints change only after independent geometry checks.

Capture windowless before/after frames for all five original references, whole map, and tunnel first-person views with fog-of-war off; inspect full frames and combined contact sheet. Check flowing water with a separate phase sequence. Native control/camera feel remains a separate user review unless requested.

Inherited baseline caveats: ordinary water alpha0.85 was only a review replacement and needs production promotion; non-draft Massif-above-Crystal majority is 5628/10315 above182, below the existing55% contract. Diagnose actual terrain stages without weakening the threshold. Acceptance of V3 visuals did not claim shipping, motion, or alternate-light validation.

The user added the Crystal Ascent crown snow transition while implementation was active. Include crown close-ups and a top view in the same final visual review.

## Crystal crown implementation handoff

The private `schematic_crystal_snow.rs` pass runs after global alpine cap reconciliation and before general vegetation. Its authority is the claimed radius32 Crystal patch, exact existing exterior summit surfaces at the authored upper-exit level (canonical150), radii11..27 plus the exact four-wide shell summit trail at radii28..32. It changes only existing one-voxel Grass/Snow material intervals and the existing crown tree object IDs. The radius-below11 hole, stair cutouts, all lower strata, worked-stone architecture, surface positions and metadata, cutaway tags, feature IDs, roots, rotations, complete visual occupancy, canopy membership and blockers remain unchanged. The accepted snowy tree retains green undersides beneath white canopy tops; no new asset is introduced.

The post-ecology Crystal column, metadata, feature and blocker authority is checked after vegetation and at final generation. The baseline diagnostic comparison proved the corrective validator still enforced the obsolete146 threshold despite the accepted ecology’s mean200 snowline. Final snow admission now follows the accepted organic cap policy and existing Frozen/summit rules, and permits only exact Grass cap positions sealed by the crown transition. This changes validation to agree with the accepted visuals; it does not alter the broad snowline or terrain. Neither broad Crystal membership nor another level of the same column is exempt. The existing outer-opening Snow requirement remains intact.

Added tests: `crystal_crown_snow_preserves_final_geometry_strata_and_foliage` compares an actual merged Crystal fragment before/after, including another owner's grass/tree sentinel, exact strata, full tree projections, idempotence and mutation rejection; `crystal_snowy_tree_counterpart_changes_only_existing_foliage_styles` rejects occupancy, blocker or trunk-style drift; `crystal_crown_radial_snow_is_bare_at_hole_and_full_at_edge` checks exact endpoint coverage and increasing broad-band coverage over seeds0,1,7,14,175,9999. All three focused crown tests passed in focused-candidate-04; combined final-world validation and visual captures remain pending. Rustfmt parsed the new module with edition2021 and `git diff --check` passed.

Visual witnesses to capture: the crown centered on its claimed Crystal center (seed0 world axial22,-132, summit150), with the unchanged central oculus and `crystal_ascent.upper_exit` visible; a top view must show bare inner radius11 and white outer radius27, and a close oblique view must show snowy foliage tops with retained green undersides. Review paths and observed runtime cap/tree counts will be recorded after capture; no unrendered frame is claimed as evidence.

## Combined Grand V3 completion

The user requested that this task finish the Grand V3 PR after both this candidate and `Add walk mode to Hex game` are complete, and prepare easy morning-test launches with and without grounded exploration. PR219 targets `dev` from `wave/grand-v3-schematic`; its inspected head is `bc06a896`. This candidate stacks accepted visual snapshot125e2df on that head. The walking task owns `feat/grand-v3-grounded-exploration`, separately based onbc06a896, and has been asked to send an exact committed readiness handoff directly to this task. This task owns combined integration, conflict resolution, final validation and PR reconciliation. No polling automation is active.

Inspect and merge only the walking task's intended unique changes after its readiness message, preserve the accepted visuals and this candidate's camera OIT contract, and revalidate the combined behavior. Prepare separate Cargo-based launchers for ordinary play and walk/fly exploration on the same final map. Do not open a native game window during unattended preparation. Current PR checks contain inherited failures; their exact causes and current-candidate status must be recorded before merge.

## Independent ascent and final tunnel validation

The accepted-source diagnostic captured the original natural pass both at construction and after vegetation for generated seeds0 and1592598566; those stages were identical. The checked-in RON fixtures record all centerline coordinates and levels, every reserved walking surface, and physical widths4 and3 respectively. Final-world tests compare against those independent fixtures, including Ordinary access, so recomputing a changed solver cannot silently bless a moved route.

Early tunnel expansion changed the two-dimensional route exclusion footprint and disconnected the lower natural ascent even though the added tunnel ceiling remained more than130 levels below the route. The candidate now retains the original tunnel footprint during surface terrain planning and performs the wider physical carve after those routes are fixed. Final publication preserves original named niches, replaces recessed crystal lights, adds expanded interior floors and roofs, and refreshes the graph. Both focused phase/publication tests passed in candidate07. Full-world validation stopped at the broadened shoulder field before it reached late expansion, so final-route and whole-world clearance checks remain pending.

## Candidate07 focused result and bounded approach repair

Candidate07 executed43 focused tests:40 passed and3 full-world tests failed. The successful
set includes original-water seam/picking, all23 liquid-render tests, review material
restoration, three crown tests, three rear-shelf tests, both tunnel-phase tests, bend width,
complete crystal bounds and hero/zero intake fixtures. The full hero, generated zero and
reference Garden tests stopped in the added broad shoulder field: immutable high-cliff
bounds conflicted at(-31,-105) for the hero and(-31,-111) for zero/reference.

The original Peak shoulder solver is restored. A separate bounded fill-only lower-approach
extension is being integrated: it raises only lower adjacent terrain toward the route's
existing nine-level falloff, preserves higher cliffs, pins the exact route and prior terrain
authorities, and refuses to create or worsen steep boundary edges. No passing full-world
result is claimed until that candidate is compiled and admitted.

## Committed walking handoff integrated

The walking owner completed and froze `95b2274de9c9be23a2aaf75634c5a1cb03b1281b`,
based on `bc06a896`, and explicitly yielded its caches. All nine intended commits are
integrated through a merge that preserves their history. The one status-document conflict
was resolved around the combined candidate. Automatic source merges retain the candidate's
OIT/depth/MSAA camera setup and capture-plan detection alongside exploration and windowless
inspector suppression. Deferred visibility retirement uses the walking owner's
`try_insert`/`try_remove` fix. The `dev,map-review` combination still needs validation.

Exploration starts in Fly; F selects Walk/Fly. Its focused tests and static author/reviewer
captures are source-scoped evidence from the walking task. Native feel/flicker remains
pending the user's morning test. Same-map local launchers are prepared but remain disabled
until the combined candidate is validated. Warm development and ci/map-test caches were
APFS-cloned after the owner yielded them; the original caches are preserved.
