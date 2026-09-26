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

The steady downhill river-wave request remains queued. Its separate source
checkpoint is `bf6f4f63997d9e4246711714a5decc4386649ac6` on
`feat/grand-river-flow`; one authored-link test passed, but renderer compilation,
package/graph validation and motion review remain pending. It is not integrated
into the current final4 playtest candidate.
