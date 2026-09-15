# Water laboratory

The `water-lab` arena map is an isolated experiment for opaque, whole-voxel water,
surface swimming, wind variation, and glider wind tuning. Existing maps retain their
water and controller defaults. This does not replace sailing propulsion.

Launch from this checkout through Cargo:

```sh
python3 tools/arena.py launch --map water-lab
```

Enter starts; Tab pauses and frees the mouse. The lab panel has clickable buttons and matching comparison keys:

| Key | Action |
| --- | --- |
| F1 | Show/hide lab controls |
| F2 | Flat / gentle / regular / swell / crossing / extreme waves |
| F3 | Depth colors / crests / patterns (depth offshore + nearshore crests + moving shimmer) |
| F4 | Calm / steady 9 / steady 20 / gusts / turning / local shelter |
| F5 | Glider wind influence: 100 / 65 / 45 percent; starts at 65 |
| F6 | Freeze/resume waves; wind and player movement stay live |
| F7 | Reset wave phase |
| F8 / F9 / F10 / F11 | Beach / swimming / boat / glider start |
| B / G / F | Toggle boat / glider / exploration flight |
| C | First-/third-person camera |

The map has exactly seven nonoverlapping radius-12 hex regions (3,283 columns),
a walkable central island, a cove, a channel, a headland, and surrounding water.
Its bounding radius is 37; the corners of that bounding disk are not extra biomes.
Boat contact requires exact wet columns. Lab input returns a player leaving the
finite footprint to the beach through the gameplay-owned reset API.

`hex_map::water_lab::LabSurface` owns the wave and local wind functions. It consumes
exact current exposed solid beds, including currently dry shore columns. The
opt-in `inundation_column_at` contract permits wave run-up and retreat without
changing solid occupancy. Current admission is still checked first; failed
sampling remains blocked. Other oceans keep their existing stored-liquid policy. The renderer and gameplay both use the same
quantized height and completed simulation phase. The opaque renderer uses one
bounded disposable mesh batch, rebuilt from the currently inundated footprint; it never
edits stored water voxels to animate a wave. This is a small-map prototype, not a
scalable ocean renderer for the later large island map.

Stepped tops have upward normals and zero continuous vertical velocity. Discrete
height changes update surface contact, not launch impulses. In the lab, neutral
swimming targets two-thirds of body height below the surface and moves at 85% of
the actor's walking speed. The original swim tuning remains on other maps.
Glider wind factors apply only to lab gliding; boat propulsion consumes full local
wind. HUD wind indicates the environment, not the glider's reduced effective wind.

Review-only launch settings:

- `HEX_WATER_LAB_WAVE`: `flat`, `gentle`, `regular`, `swell`, `crossing`, `extreme`.
- `HEX_WATER_LAB_STYLE`: `depth`, `crests`, `patterns`.
- `HEX_WATER_LAB_WIND`: `calm`, `steady`, `strong`, `gusts`, `turning`, `shelter`.
- `HEX_WATER_LAB_PHASE`: a finite phase to freeze; omit for live waves.
- `HEX_WATER_LAB_GLIDER_WIND`: a fraction in `[0, 1]`; the comparison candidates are 1, 0.65, and 0.45.
- `HEX_ARENA_CAPTURE`: PNG destination; uses the existing windowless image target.
- `HEX_ARENA_VIEW`: `water-lab-overview`, `water-lab-shore`, `water-lab-reverse`,
  `water-lab-channel`, `water-lab-swim-first`, `water-lab-swim-third`,
  `water-lab-boat-first`, `water-lab-glider-third`, or `water-lab-controls`.
- `water-lab-motion` and `water-lab-motion-reverse` produce 24 numbered PNG/JSON
  pairs from a continuously running windowless orbit, four frames apart. These
  diagnose visible motion defects; user control feel remains a native playtest.

Capture metadata records the actual selected settings, wave phase, actor state,
source/build identity, and timing. Check returned settings rather than assuming
an environment variable selected the intended case. Frame timing includes CPU
render submission and readback overhead and is not a GPU or native-vsync benchmark.

For the first review, follow walk → swim → boat → swim → shore; then compare wave
shape, colors, and glider wind influence one category at a time. Retain explicit
pending verdicts for taste, comfort, and control feel until the user has played.

## First playtest revision

The native F2/F3/F4 feedback is preserved in the chat outputs with four original
screenshots. The new fixture widens the channel to about ten world units and
extends its sand bed underwater. Incoming +X waves gain shorter wavelengths and
slower phase speed over shallows. Nearshore crests grow before dissipating; the
channel loses height progressively, more at its banks, and fades at its far end.
This is a stylized depth-dependent wave field, not a fluid or current solver.
Spatial propagation is cached once per settings/terrain publication.

Calm Patterns water uses continuously changing, oblique shimmer ribbons throughout
the sea instead of binary static patches. Depth and Crests remain comparison
styles; Patterns combines offshore depth colors with nearshore crest highlights.
The headland is now roughly twenty units above the sea. F11 starts above its
summit facing out to sea; its footprint is unchanged.

Hands-free swimming and holding Space both target the same 0.8-unit immersion.
A swimmer keeps following a descending wave instead of briefly switching to
gravity. Ctrl still dives, releasing it returns to surface swimming; High Jump
retains its launch. Boat toggles use the same changing shore surface.

`water-lab-motion-cycle` records a fixed inlet view over more than 18 seconds.
`water-lab-motion-swim-first` and `water-lab-motion-swim-third` record the same
interval through gameplay cameras, with typed surface/feet/eye contact receipts.
These 24 samples are 48 frames apart; dense orbit views remain four frames apart.
The sparse cycle proves sampled presentation only, not smoothness or comfort.

## Second playtest revision

The user prefers running Flat, Regular and Extreme, Patterns colors, and 65%
glider wind influence. The lab now starts at 65%; the comparison controls remain.
Boat propulsion is unchanged from the liked native baseline.

Channel delay is slightly reduced. Wave amplitude recovers smoothly over twenty
world units after the exit instead of leaving a permanent calm wake. Patterns
adds white foam to high crests and shallow shore contact.

Lab swimming and boat buoyancy approach the stepped water height with a
critically damped response (frequency parameter 16/s), capped at six vertical
units/s. Velocity eases into each step rather than changing immediately.
Physical collision remains authoritative. The two-thirds swim immersion is an
equilibrium target; rising waves can briefly immerse the eye while the body
catches up. Boat deck, player and camera share physical vertical displacement.

Third-person distance is 2.4 units for the body and 5 units while boating or
gliding, with the existing collision sweep retracting the camera near obstacles.
All aim consumers use that same camera pose. The canopy now uses the flight
direction with world up, avoiding roll introduced by a shortest rotation arc.
These camera and canopy presentation changes apply to the arena generally.

Next native route: inspect the channel exit and foam; compare Space released/held
and boat toggles in Regular/Extreme; repeat full heading turns with the glider
at 65% in both camera modes. Control comfort and animation acceptance remain
pending until that playtest.


## Third playtest: accepted appearance and wave response

The user accepts the current wave appearance and boat experience. Preserve the
water renderer, sailing propulsion, steering and wind tuning.

Lab swimmers and boats now receive a small downhill drift when the sampled water
height changes. The slope uses admitted water samples two units to either side;
dry/unloaded neighbours do not supply a slope. Smoothed surface rise/fall speed
scales the push, capped at 0.8 units/s. A static slope does not itself drive an
idle body. Mode changes reset the surface history so mounting/folding is not
mistaken for a wave. The physical boat velocity includes drift, but that drift
is removed before the next sailing calculation, preserving propulsion behavior.
Ordinary oceans retain their previous marine behavior.

All 19 focused marine tests pass, including ascent/descent push direction, flat
and static-water neutrality, blocked sampling, sailing separation and vertical
step/reversal easing. Native comfort remains pending. Try floating on Regular,
repeat with Extreme, then sail across wave faces and toggle B near shore. The
first 2D Grand V3 draft and its design notes remain separate planning work.
