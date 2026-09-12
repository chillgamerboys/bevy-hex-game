# Expedition site admission contract

The world publishes `ArenaExpeditionSites` only after validating its exact terrain
and static occupancy. These facts describe geometry; gameplay still admits each
actor's physical pose and owns roster, rally orders, healing and rewards.

Each route declares a positive `clearance_levels`, bounded by the published world
height. Every support in the complete ribbon must retain that many clear levels
above it. Adjoining steps differ by at most one level and both columns must retain
the full aperture above the higher support. The centerline and shoulder flood use
the same step predicate. Terrain, movement-blocking static geometry and liquid
all obstruct clearance. Forest's 0.35-unit voxel height needs at least three clear
levels for a 0.8-unit body; gameplay remains responsible for radius and actual
body dimensions.

Deployment surfaces remain generic candidates with one free level. When a camp
names `rally_entry`, that node's exact supporting voxel must belong to the camp's
published deployment surfaces. This establishes the geometric join to the route
network without granting any movement order or knowledge of the player.

Fountain cells must lie inside the published geometry and match a liquid span of
the catalog's known, non-Air, non-solid `water` material. They cannot overlap solid
terrain, movement-blocking static geometry or another fountain's claimed cells.
The published spans, rather than visual glow or a guessed material, establish
water membership. Healing amounts and one-use consumption remain gameplay facts.

Validation tests cover low ceilings, static crowns, elevated water, stair
apertures in both directions, blocked shoulder joins, remote rally nodes, fake
water materials and world bounds. Cargo execution is coordinated on the combined
candidate; this contract does not claim native actor movement was playtested.
