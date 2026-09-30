# Visual Language

The game's authored colour vocabulary: what belongs in it, how it grows, and how it
stays legible as more terrain, plants, units, props, and effects arrive.

This is a design contract, not a renderer settings guide. Sky colour, time-of-day
lighting, fog, selection feedback, interface colour, and debug overlays deliberately
live outside it. Those systems change how authored colours are perceived; they do not
define the colour of an object.

## One named palette

[`assets/art/palette.ron`](../../assets/art/palette.ron) is the canonical inventory of
authored content colours. Every entry has:

- an immutable, path-like id used by assets and code;
- an editable display name;
- one sRGB colour with channels in the inclusive range `0.0..=1.0`;
- sorted tags for search and review.

The id names the visual role, not the current RGB value: `terrain/grass`, not
`green-35-62-30`. A colour may be tuned without rewriting every object that uses it.
Two swatches may have the same RGB value when they serve genuinely independent roles;
the editor reports that relationship instead of silently collapsing it.

Palette order is deterministic. Each id segment begins with a lowercase ASCII letter;
the remainder may contain lowercase letters, digits, or hyphens. Tags use lowercase
ASCII letters, digits, hyphens, or underscores without path separators. Renaming a
display name is harmless; changing an id is a reference migration, not an ordinary
edit.

The catalogs are machine-written documents. Workshop saves replace their complete
serialized contents, so comments inside the RON files are not durable; visual policy
and migration notes belong in this document. A `legacy` tag means the live renderer
still owns that colour literal and is removed when the renderer resolves the swatch
directly. Cataloged object styles do not implicitly promote runtime substances or
temporary-feature materials into reviewed Workshop styles.

## Strict for new work, staged for old work

The palette has two adoption rules on purpose:

1. **Strict:** every asset saved by the Asset Workshop refers to an existing swatch by
   id. Object files never embed an arbitrary base colour, and procedural object
   generation may choose between swatches but may not invent per-instance tints.
2. **Staged:** existing renderers retain their current colour sources until their own
   migration. Terrain substances, liquid bodies and foam, construction metal, unit
   presentation, and Forest's authored object instances now resolve palette swatches
   directly.

This keeps the palette useful immediately without turning its introduction into a
cross-cutting visual rewrite. During the staged period, a palette entry can be an
inventory of a live literal rather than its authority. Its tag includes `legacy` until
the corresponding renderer resolves the swatch directly. The current catalog has no
remaining legacy-tagged live literals.

`liquid/foam` stores the nearest f32-representable sRGB encoding of the liquid
shader's former linear blend target (within two ULP per channel after conversion).
Moving that colour into the palette therefore changes its ownership without a
visible change to the rendered Waterfall appearance.

## Adding a colour

Reusing a nearby colour is the default, but similarity is evidence rather than a
prohibition.

1. Search by id, display name, and tags.
2. Inspect the five nearest swatches measured in OKLab.
3. At a distance of `0.025` or less, the editor warns and requires explicit
   confirmation.
4. Add a stable id, clear display name, and meaningful tags.
5. Review the affected asset under neutral, dark, and unlit preview rigs.

The editor never merges close swatches automatically. A new colour is justified when
it carries a distinct visual role or when reusing the nearest colour makes the form
less readable. Convenience, random variation, and procedural noise are not reasons to
widen the palette.

Changing a shared swatch is intentionally visible. Tooling reports every referring
style and object before the change is saved so a local adjustment cannot quietly
recolour unrelated art.

Launch instructions, editor controls, explicit-save behavior, recovery, and the
review workflow live in the
[Asset Workshop contract](../systems/asset-workshop.md#authoring-workflow).

## Colour and surface are separate

A palette swatch answers **which colour** an authored part uses. A voxel style answers
**how that surface renders**. Shared styles in
[`voxel_styles.ron`](../../assets/art/voxel_styles.ron) combine:

- one base swatch;
- `Opaque`, `Cutout`, `Translucent`, or `Additive` rendering;
- opacity;
- optional emission with its own palette swatch and nonnegative strength.

Foliage cutouts, translucent magic, and glowing crystals can therefore share colours
without pretending to be the same material. Emission is authored independently from
base colour because a blue stone may cast a pale cyan glow.

Transparency does not excuse hidden geometry or unclear silhouettes. Opaque is the
default. Cutout is for binary-edged coverage such as foliage; Translucent is for
surfaces that genuinely reveal what is behind them; Additive is for light-like effects
rather than physical matter.

## Current inventory

The initial catalog records the currently rendered content vocabulary:

| Group | Swatches | Current authority |
|---|---|---|
| Terrain | grass, dirt, sand, stone, gravel, snow, ice, basalt, bedrock | `palette.ron`, referenced by `substances.ron` |
| Liquids and construction | water, lava, water foam, metal | `palette.ron`; bodies and metal are referenced by `substances.ron`, while the liquid shader resolves foam directly |
| Authored vegetation | trunk, three foliage values, and two grass values | `palette.ron`, referenced by the small broadleaf, tall narrow, old-growth, blocking date-palm, and nonblocking grass-tuft objects rendered by generated biomes |
| Emissive props | cyan crystal body and glow | `palette.ron`, referenced by the low, branched, and spire crystal objects |
| Units | player red, hostile blue | `palette.ron`, resolved during actor setup |

Air is absent because it is never drawn. Sky, celestial light, atmosphere, fog,
interface panels, text, movement overlays, selection rings, and debug colours are
also absent because they are presentation state rather than authored object colour.

The inventory will contract as well as grow. Art refinement may deliberately merge
roles after reviewing every reference; the point is to make that choice explicit and
auditable rather than to maximize the number of swatches.

## Coastal island language

Island maps introduce no new palette roles. Still water, sand, grass, dirt, stone,
tree trunks, and broadleaf foliage retain their existing swatches and rendering
contracts. Their identity comes from land-water silhouette and material succession:
open water must clearly separate sandy components, while a wooded island reads from
sea to sand fringe to grass-and-soil interior before tree density supplies the crown.

At Macro scale, scenic satellites need strong negative-water gaps so they do not read
as a broken land bridge. The playable sandy landing and wooded heart should remain
visually close enough to advertise their single dry connection. Random shoreline
detail, tree placement, and tree rotation may soften repeated shapes, but they must
not obscure the exact component count or make an unreachable gap look traversable.

## Shape vocabulary

The grid is part of the visual language. Workshop objects use the same hexagonal prism
as terrain and the same default `0.4` world-units-per-level proportion. A placement is
an object-local axial coordinate plus an integer level, so authored plants and effects
align with the map instead of approximating it with unrelated cubes.

Semantic parts describe why a voxel exists:

- plants use root, trunk, branch, foliage, and accent;
- effects use core, trail, and accent;
- props use structure and detail.

These labels are generation and review vocabulary, not permission to infer gameplay
behavior. Blocking, canopy occlusion, damage, light, and interaction remain explicit
contracts rather than consequences of a colour or part name.
