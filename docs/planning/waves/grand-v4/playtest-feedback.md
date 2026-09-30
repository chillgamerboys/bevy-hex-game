# Grand V4 — terrain correction for the next session

Development was paused for the night at the user's request. On September 26 the
user explicitly resumed it and authorized continuing after the weekly reset until
the approved project is complete. The failed terrain review below remains the
first corrective priority.

## User playtest result: terrain traversal fails

The current final4 landmasses are much too steep and rounded. The user found them
essentially impossible to traverse: the offshore island was almost impossible to
step onto, and slopes beside the river rose so quickly that the user could not
leave the area close to the river. This is a fundamental terrain-shape problem,
not a small movement-tuning issue or a request for a few isolated paths.

The required traversal character is:

- **Valleys:** close to fully traversable, with broad usable ground.
- **Hills:** easily traversable across most of their surface, with only a few
  deliberately non-traversable spots.
- **Mountains:** still provide understandable, usable routes through and up them.
- **Coasts and rivers:** allow ordinary landings and movement away from the water;
  avoid trapping the player between water and immediately steep banks/hills.

Use the original Grand V3 terrain as a reference, and especially the prior V4 map
with Dragons and Goblins for successful traversal. Recover and inspect those exact
references before redesigning; do not substitute an assumed map or another rounded
heightfield. Woods should also resemble the forests in that V4 map.

## Intended result

Transpose Grand V3's more authored look and composition into the larger V4 world,
with more beautiful detail and lush forests. The enlargement is not an instruction
to inflate rounded hills or make most of the landscape inaccessible. Preserve the
agreed geography and scale while giving the terrain purposeful shapes, generous
walkable areas, varied slopes, natural transitions and convincing forest structure.

Correct the broad landforms and traversal first, then revisit forest character and
detail. Do not treat one narrow valid route or increasingly restrictive route
validators as sufficient: the surrounding valley and hill surfaces should themselves
be pleasant to explore. Use terrain iterations, fresh views and actual ordinary
movement to review this correction, including landing on the island and leaving
both sides of the river corridor. Do not compensate by assuming upgraded mobility,
teleport, or gliding is necessary for ordinary exploration.

The recorded sevenfold area measurements, save/restart tests and paused streaming
circuits do not establish terrain usability. Keep their evidence, but the current
candidate has **failed the user's terrain/traversal review** and must not be called
an accepted integrated expedition.

At the overnight final4 checkpoint, the steady downhill river-wave request remained queued. Its separate source
checkpoint is `bf6f4f63997d9e4246711714a5decc4386649ac6` on
`feat/grand-river-flow`; one authored-link test passed, but renderer compilation,
package/graph validation and motion review remain pending. It is not integrated
into the current final4 playtest candidate.


## September 26 corrections under validation

The revised source uses the recovered `b314a9d` Forest expedition as its traversal
and tree reference. Broad lowland envelopes replace the inflated rounded slopes,
with continuous coast/river shoulders and meaningful mountain approaches. The
first real-controller walking run passed five routes and failed six: it exposed
two terrain joins at the island and Shadow outlet, encounter-interest loading
stalls, and local tree-avoidance limitations in the test driver. Those failures
are preserved; source repairs require a new complete walking run, not a changed
claim on the old receipt.

The downhill river shader and exact flow graph are integrated. A focused fresh
windowless frame renders its bands but exposed unsupported regular reaches. Exact
columns showed a later valley cap cutting away already-composed beds and banks;
that ordering is now repaired, retaining shallow reaches, broad bank shoulders
and localized waterfall drops. The moving-wave appearance remains under review.

Forest recipes, persistent distant forest publication, garden courtyard and
library architecture are integrated. Full-package admission caught two overlapping
fountain-rim/shrine voxels missed by the earlier local dressing tests. The rim now
respects authored occupancy; a full-composition overlap regression and both affected
chunks pass. A fresh immutable package, all eleven walking routes, measured sailing,
36-view static matrix, temporal sequences, combined CI and native play remain
required. This entry records corrective work, not acceptance of the expedition.

## September 26 — first actual revision02 plain render

The user found the general mountain shapes too steep, with bases that appear
untraversable. This applies beyond the square summit and raised tunnel strips
caused by cave-cover inflation. Broaden the mountain feet into connected gentle
aprons, retain steep upper scenic faces and the peaks enclosing the hidden lake,
and test cross-country access across the entire dry foothill belt. Forest-only
grade statistics and isolated passing route centerlines do not clear this concern.

The first actual package (`compiled-r02-plain-01`, source4ad2b82d) preserves broad
landmark relationships but fails visual transfer: cave cover distorts the massif,
water-adjacent exact terrain differs sharply from the smooth distant proxy, and
the fall camera cannot show the complete plunge. These are diagnostic renders,
not an accepted terrain or completed presentation checkpoint.

The first repair compressed lower elevations and restored the upper profile. A
whole-footprint source survey found that it merely moved the steep band inward:
only 14.85% of neighboring edges in the 65–70-unit band met the ordinary one-level
step limit, and the seaward feet still contained broad steep belts. That repair is
not accepted. The next study reshapes the mountain bases spatially, with broad
lower slopes, setback upper faces and varied coastal headlands. Its source views
omit layered caves, stairs, objects and the controller; they guide implementation
and cannot stand in for rebuilt game screenshots or ordinary walking.

The upper lake remains at 205 units above sea level and must remain screened by
the connected mountain complex. Original individual summit heights are adjustable
within the user's requested shape correction. Any lowered summit requires the Air
shrine and library to fit the actual new rock cover, without rebuilding an exterior
wall to preserve an old endpoint. Review the broad foothill belt and mountain joins
before another full package, then verify the real composed columns and movement.

The spatial repair is now in source: wider low mountain aprons, setback steep
cores, continuous Crystal shoulders and a broad backing landform beneath the lake.
Source measurements show substantial connected foothill ground, but neither the
coarse MODEL pictures nor the graph clears the user's concern. The next review
uses a new actual game package, matching whole-world angles and four fixed walking
height views across and up the western and lake-front bases. Keep this feedback
open until those images and ordinary controller movement have been reviewed.

The plain02 review keeps this feedback open. Eleven fresh views show broader feet
but still expose upper walls and abrupt distant/detail terrain boundaries. Actual
controller checks pass one of the four fixed probes: both western probes stop on
two-level lower-apron ledges, while the lake crossing hits a riverbank descent.
The next correction must address the regional source profiles and usable banks;
do not grade narrow strips along the test lines or weaken the fall limits. The
automatic ridges' lost saddle factor is separately restored so enclosing peaks
remain distinct. High mountain screening and broad lower traversal are separate
requirements, and neither is waived to satisfy the other.

The lake-crossing endpoints straddle a real river. They now have a separate
walk–swim–walk check, retaining the same endpoints and strict dry-segment fall
limits. Every tick still requires loaded terrain and clear body space; a real
swimming entry, wet travel, exit and settled dry endpoint are required. Plain02
still fails this corrected check at the same bank, before swimming activates.
Reclassifying the crossing did not clear the defect. The other three foothill
probes remain ordinary dry walking.

The next candidate includes irregular dry shelves around the upper lake and
shallower margins on ordinary river reaches (`cb29841`); the plunge and receiving
pools retain their separate geometry. Focused emitted-bank and nonuphill-flow
checks pass, but only a new compiled package and the unchanged mixed crossing
can establish the actual controller result. The western repair composes the
coastal foot and mountain body rather than adding their slopes, and broadens the
western summit. Its library and Air route must be refitted inside the new rock;
the old room elevations are not a reason to inflate the mountain again.

Plain03 now passes all three unchanged dry foothill controller probes. The mixed
crossing enters and swims but stalls at the opposite bank; the emitted step is
three voxel levels. New bank source reduces that specific edge and the measured
northwest lower-lake cliff to one level. The separate shallow-water handoff keeps
the normal walking step limit. Both changes need a new compiled package and the
same mixed crossing before calling the river access fixed.

The new game pictures still fail general mountain presentation: the Crystal
exterior reads as a regular drum, its Frozen connection as a raised ribbon, and
several enclosing peaks as thin spires. Detailed/distant boundary wedges are a
separate unresolved rendering defect. A proposed wider mountain profile improved
the source silhouette but destroyed much of a previously connected low-ground
region east of Crystal; it has not been integrated. Lower-ground continuity takes
priority while refining that profile. Preserve the steep inner ascent and hidden
lake screens, while joining their outer bodies to usable lower terrain.

## September 27 — plain04 keeps the broad-traversability concern open

Five fresh actual-game views still fail the mountain composition. Crystal has a
regular steep exterior, the Frozen connection reads as a raised shelf, and the
upper lake is surrounded by narrow spires instead of substantial connected rock
shoulders. The low valley is wider, but that does not settle the user's concern.
Capture stopped after these failures were established; the remaining eleven
requested views were not produced and the diagnostic pipeline remains failed.

The whole-domain plain03-to-plain04 comparison found 6,681 columns lost from the
largest ordinary-edge dry component, against 384 gains. The largest regression
comes from Crystal's outer rock rising across previously usable hills; another
comes from the shoreline lowering ground below the root room's old ceiling.
Source corrections now confine the Crystal enclosure and fit the room beneath
its actual natural roof. A broader dry riverbank also addresses the remaining
descent after the now-successful swimming exit. New package evidence is pending;
the old failed receipts remain unchanged.

Garden, Crystal Ascent and Frozen Woods pass their focused actual-controller
checks on unchanged plain04 after fixing the review driver to visit every stair
support. An independent audit confirms all 815 grounded arrivals in exact order.
No terrain, movement limit or fall threshold was loosened to get those passes.
These routes cannot establish broad hillside movement or final presentation.

The Grand V3 falls reference shows broad overlapping mountain shoulders with
unequal connected crests around the cleft. A separate exact-source upper-body
study will carry that shape into the current design while preserving lower
foothills, the continuous Crystal/Frozen route and the hidden lake. Compare the
entire affected belt, including the transition into steep upper rock; do not
revive narrow route validators or move the fixed cross-country endpoints.
