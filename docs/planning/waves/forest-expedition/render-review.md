# Expedition render and native review matrix

Written composition authority: `manifest.md`, including amendments A1–A3.
This is the planned acceptance matrix, not a passed review. Exact full source HEAD,
compiler identity, package byte hashes, loaded manifest and companion fingerprints,
commands and per-frame hashes are recorded in the fresh capture pack receipt.
The source and package must remain unchanged across a pack. Initial authored package
fingerprint is `f06ba29a0bdfa9b0`; later geometry changes require a new candidate.

Use `tools/arena.py capture --expedition-review --forest-world ABSOLUTE_PACKAGE
--output NEW_ABSOLUTE_DIRECTORY --timeout 1200`. This calls Cargo and renders to
windowless Bevy image targets at 1600×900, scale1, fixed afternoon light and water
phase0. A clean committed candidate is required for acceptance. Output starts
UNREVIEWED; it becomes static evidence only after every full-resolution frame and
one whole-matrix contact sheet have specific independent review notes.

## Required views

| Entries | Visible criteria |
| --- | --- |
| expedition-overview, expedition-rear | Entire radius187 footprint with margin; layered western forest, bent central river, one bridge and eastern connected massif all visible. No missing chunks. |
| expedition-start, expedition-upgrades | Legible roster, contact-fireball start, three reward objectives, XP/upgrade controls and fountains-only healing; no clipping. |
| bridge-first, bridge-third | Actual starting cameras on deck; safe-looking approaches, gates and parapets. |
| bridge-arch, bridge-arch-rear | Arch rises through center; exact supported banks, rail endings and open portals from both directions. |
| woodland-ground, pine-ground | Smaller trees create depth and obstruct some sightlines; walkable/shootable local clearings and hills remain readable. |
| ancient-ground, ancient-ground-rear | Irregular tapered bases, supporting roots and larger local spaces; layers of canopy, rocks and crystals from low opposite angles. |
| heart-tree, heart-tree-rear | Unique dominant central tree with tall crown and space for several camps; roots meet terrain and crown connects to trunk. |
| dragon-lower, dragon-middle, dragon-summit | Distinct shelf elevations, variable-width ascent, rest openings, connected mountain shoulders and snow progression. |
| shadow-arena, shadow-gate | Duel-sized interior, tall decorated enclosure, massive open gate and supported approach. |
| forest-fountain, mountain-fountain | Recessed discoverable pools, visible charged water, supported rim and entry; no opaque surface replacing water physics. |
| troll-reward-orb, dragon-reward-orb, shadow-reward-orb | Gold, blue and violet spheres visibly hover near valid ground with restrained light/halo. Explicit synthetic defeat fixtures. |
| shadow-reward-collected | Sphere absent after collection; HUD shows60/125 without healing. Explicit synthetic pickup fixture. |
| fountain-spent | Charged material/glimmers absent but the physical water/rim remains. Explicit synthetic wounded-player placement. |

Landmark/ground/reward views use external composition cameras and are not evidence
of native player movement or camera comfort. Reward and spent-pool states are
established by ordinary gameplay ticks and typed readiness checks after explicitly
recorded staging; pixels only review their appearance. CPU profiles and exact
roster/support/canopy/XP/pickup assertions are separate evidence.

## Native route

Start on the bridge, aim contact shots past railings and tree trunks, walk the
winding forest approaches and enter a large camp, reach the Heart clearing, enter
and leave the Shadow gate, climb between the lower and middle Dragon shelves and
try the summit approach. Observe animated water/crystals, use a hidden fountain,
collect a reward, pause/resume and restart. Judge aiming, input response, camera
collision, shadows, attack readability, firing space, crowd movement and overall
navigation. Repeat close water/tree/structure passes in the reverse direction.

Native access currently reports that macOS is locked; do not bypass the lock or
substitute OS automation. Continue windowless work, and retain HUMAN-MOTION-PENDING
until the Mac is manually unlocked and the actual candidate can be addressed.

## Scale reference inspected

On September12, root inspected actual Cube World Alpha footage in BebopVoxGaming's
[Crazy Huge Boss Tree, episode18](https://www.youtube.com/watch?v=dY-nHTdZ4a0&t=435s)
at7:15 through the background browser. The Arutar Tree view shows a trunk dwarfing
neighboring ordinary conifers and broad elevated limbs carrying the canopy, with
substantial empty volume below. The crown top is cropped, so this is close-scale
and branching reference, not a complete silhouette or measurable size reference.
The expedition keeps its own voxel blueprints, tapered roots and taller crown form;
no reference asset or distinctive exact tree shape is copied. This observation
supersedes the earlier unsuccessful wiki/reference-search notes.

## First composed pack and repair candidate, September12

Exact source `ccc5847b7d68cb4d0c3c51af46e15bc126872358`, package f06, produced all26 frames. Root and an independent reviewer inspected every full-resolution original and the contact sheet. Static verdict: **FAIL (13PASS,10FAIL,3BLOCKED)**. The full review and hashes are in `.context/expedition-review-ccc5847-first/ccc5847b7d68cb4d0c3c51af46e15bc126872358-forest-expedition-v2-rewards/independent-static-review.md` and the sibling JSON/receipt.

Failures include near-black canopy floors, large-tree trunk/crown regularity, opaque mint fountain caps and clipped feature cameras. Source `028894e` increases forest ambient from240 to1100, reduces directional light7400 to6000, replaces the closed cap prism with one translucent hex surface per water column and revises bridge/arena/fountain cameras. Seven large-tree assets now have taper, staggered bark, flared roots, exposed rising limbs and asymmetric lobed crowns; package **a538263d612e891f** retains67.0088% canopy. The 14-view repair subset below supersedes this unrendered checkpoint; old f06 frames do not validate the new package.

Bridge deck slashes align with actual one-level tread shadows; source audit found one terrain owner and no duplicate deck faces. Do not change arch geometry or blanket-remove legitimate faces without new evidence. Low bridge compositions and brighter fill must be inspected first. Native review remains HUMAN-MOTION-PENDING because the Mac is locked.

## First repair subset, September 12

Exact clean source **cd6e2c08a258de5870e33b3d78ac296ba1aac531**, package **a538263d612e891f**, produced all 14 selected repair views. Root and an independent reviewer inspected all full-resolution originals and the complete contact sheet. Static verdict: **11 PASS / 3 FAIL**. This is a diagnostic subset, not acceptance of the full matrix.

The full-footprint overview, woodland/pine ground views, both ancient ground views, both Heart views, forward bridge arch, Shadow arena/gate and collected Shadow state pass static inspection. Forest shadow floors are readable, large trees have irregular tapered and branched silhouettes, and the Heart remains distinct with a large clearing.

Remaining failures: reverse bridge foreground foliage obscures part of the span/gate; charged forest water resembles ordinary blue water without a distinct glow; the mountain fountain shares that weak charged cue and is framed too closely, cropping its near edge and approach. Independent notes and hashes: `.context/expedition-repair-first/cd6e2c08a258de5870e33b3d78ac296ba1aac531-forest-expedition-v2-rewards-focused/independent-review-actor-broadphase.md`, sibling JSON/contact sheet, and `receipt.json`.

The next repair strengthens the translucent pool tint and adds small drifting lights under the same charged-state parent, shifts both low bridge cameras into river center, and frames the upper spring using its published approach or a higher view of its cliff pocket. Charged/spent forest views share one camera. These changes are not yet compiled or rendered. No terrain or package geometry changed for these framing repairs. Native motion remains pending because the Mac is locked; a manual-unlock request is pending.


## Translucent water omission found before the next pack

A read-only audit confirmed that the Forest renderer still used `AlphaMode::Opaque` and the shared shader forced alpha1. The separate charged-pool cap could not make that water transparent. Historical accepted Grand commit **4e4f93f** supplies the intended alpha0.85 and shader alpha handling. Root **8543b2f** restores these narrowly for Forest Water materials, removes their opaque-overlay depth bias, preserves the standard shader's post-lighting alpha/OIT handling, and keeps grid/non-Forest/Lava materials opaque. Existing underwater terrain already publishes visible solid faces; authoritative water cells and animation remain unchanged. Material regression and fresh shader/pixel review are pending. No new package geometry/fingerprint is introduced.


## Translucent-water repair subset, September 12

Exact clean source **6b455e12e1828c979b034d618c4e4159f0ed7506**, package **a538263d612e891f**, produced all six selected views with source/package hashes unchanged. Root and the independent reviewer inspected every full-resolution original and the entire contact sheet. Static verdict: **3 PASS / 3 FAIL**. Notes and per-frame hashes are under `.context/expedition-repair-water/6b455e12e1828c979b034d618c4e4159f0ed7506-forest-expedition-v2-rewards-focused/independent-review-actor-broadphase.md` and sibling JSON/receipt.

The complete overview, reverse bridge span and spent fountain pass. River bank terraces are visible through the water, and foreground foliage no longer hides the reverse span. Charged glimmers are clearly visible, the mountain basin fits completely, and spent forest water retains its ripples without charged lights. Two charged pools fail because flat cyan hex patches alternate abruptly with dark-blue animated water; the independent translucent cap and water batches have conflicting sort centers. Forward bridge framing also cuts the east portal's outer pier, which the nearly edge-on reverse view does not show coherently. These are presentation repairs; no terrain/package regeneration is needed.

The next candidate widens the river-centered bridge view and repairs charged-water rendering order. All six entries must be freshly recaptured before the full 26-view matrix. Native access was checked again and still reports the Mac locked; the existing manual-unlock request remains pending.


## Actual-water fountain material repair

Foundation **6a68252**, game integration **5595b9d** and map repair **74ac687** remove the competing pool cap and style actual liquid cap/curtain batches from the disposable charged-name snapshot. Existing geometry, UVs, alpha and animation remain unchanged. Charged pools share turquoise surface/fall materials with restrained extension emission; spent or stale-generation bindings select ordinary water. Glimmers retain the gameplay consumption parent. The bridge review eye now uses river-row centering at offset62 instead of50.

Composed25 liquid tests, the actual a538 charged/spent/reset game fixture, and strict game Clippy pass. An independent source review found no concrete issue. These checks do not validate pixels; a fresh six-view subset and then the full26 matrix remain pending. No package geometry changed.


## Final static review, September 12

Exact clean source **e0746f33edd80c67c54854c5b2edd740157db2a3**, runtime **74ac687**, package **a538263d612e891f**, produced a fresh six-view repaired subset and then the complete **26-view** matrix. Both packs completed successfully with unchanged source and package. Root and independent `actor_broadphase` inspected every original at full resolution and each whole contact sheet. The subset passes **6/6**; the final matrix is **26 PASS / 0 FAIL / 0 BLOCKED** for the static criteria above.

Full evidence: `.context/expedition-final-static/e0746f33edd80c67c54854c5b2edd740157db2a3-forest-expedition-v2-rewards`, including `receipt.json`, `source-state.json`, per-frame PNG/state/log/receipt files, `root-static-review.md`, `independent-review-actor-broadphase.md`, the sibling JSON and complete contact sheet. Independent checks verified clean before/after source, all 26 PNG/state/log hashes, a538 manifest/companion identities and all 899 recorded package files. The actual helper command uses Cargo `dev,arena-prototype` with one build job, 1600×900 output, scale 1, 15:00 and water phase 0.

The paired low bridge views now include the formerly cropped east portal and a readable complete span; forest-side foliage remains environmental context. Charged fountain water no longer has alternating flat cyan/dark-blue hex patches: the physical water surface retains its ripple detail with a coherent turquoise material and small glimmers. The consumed pool retains blue physical water and loses the glimmers. The elevated mountain-pool composition shows its whole recessed basin and cliff pocket; it is not evidence of player approach comfort. All three reward colors are distinct, and the collected Shadow state visibly reads **60/125 HP** with the sphere absent. The initial upgrades screen contains the correct 45/12/15/12/0.50 start, locked explosions, all three milestone rewards and fountains-only healing.

The complete contact sheet confirms consistent afternoon palette and readable shaded ground across the forest, connected mountain shoulders and separate shelf elevations, coherent architecture and legible HUD. No further concrete static defect was identified. Earlier failed packs remain historical evidence of the repairs and are superseded for the current static verdict.

**Native review remains HUMAN-MOTION-PENDING.** CUA still reports the Mac locked after this review, and the existing manual-unlock request remains unanswered. No FPS, continuous movement, contact-shot aiming or live combat acceptance is inferred from these windowless images or the separate ArenaTick CPU profiles. No native process is left running.


## Native feedback menu repair, a8559dd

A fresh two-view menu pack at clean **a8559dd4f4263e661860b3f0a8e53e601b55b206** / unchanged **a538263d612e891f** passes **2/2**, all originals and the whole contact sheet inspected by root and independent reviewer. Source/package/per-frame hashes are verified. Evidence: `.context/expedition-restart-speed/a8559dd4f4263e661860b3f0a8e53e601b55b206-forest-expedition-v2-rewards-focused/independent-review.md` and sibling contact sheet/receipts.

The ready menu now places Start beneath Fullscreen/Quit, in the prior Restart screen region; speed help visibly reads 4.725 units/s without clipping. Pause controls remain unchanged and legible. These static findings complement the separate three-size actual-pointer regression and actual controller-distance/408-route tests. The prior 26-view pack remains map/art evidence; the two fresh frames supersede its menu views. User native feedback is partial experience evidence, not completion of the full motion route.
