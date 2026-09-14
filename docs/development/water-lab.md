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
| F2 | Flat / regular / swell / crossing waves |
| F3 | Depth colors / crests / patterns |
| F4 | Calm / steady 9 / steady 20 / gusts / turning / local shelter |
| F5 | Glider wind influence: 100 / 65 / 45 percent |
| F6 | Freeze/resume waves while retaining player movement |
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
exact current wet-column bounds. The renderer and gameplay both use the same
quantized height and completed simulation phase. The opaque renderer uses one
bounded disposable mesh batch, rebuilt from the small wet footprint; it never
edits stored water voxels to animate a wave. This is a small-map prototype, not a
scalable ocean renderer for the later large island map.

Stepped tops have upward normals and zero continuous vertical velocity. Discrete
height changes update surface contact, not launch impulses. In the lab, neutral
swimming targets two-thirds of body height below the surface and moves at 85% of
the actor's walking speed. The original swim tuning remains on other maps.
Glider wind factors apply only to lab gliding; boat propulsion consumes full local
wind. HUD wind indicates the environment, not the glider's reduced effective wind.

Review-only launch settings:

- `HEX_WATER_LAB_WAVE`: `flat`, `regular`, `swell`, `crossing`.
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
