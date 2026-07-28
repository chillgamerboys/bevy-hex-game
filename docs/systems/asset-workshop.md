# Asset Workshop

The contract for the standalone voxel-style and object-authoring tool. The Workshop is
development tooling: it creates durable RON assets for the game, but it is not a game
screen and does not run through the gameplay setup lifecycle.

The first delivery establishes contracts and tracked catalogs only. The planned
`hex_editor` application, runtime object rendering, Forest migration, procedural plant
generation, reference-image import, and animated effect timelines do not become live
merely because this document or the seed files exist.

## Boundary

`hex_editor` is an isolated Bevy application that may depend on `hex_core` and
`hex_assets`. It does not depend on `hex_game`, `hex_map`, `hex_world`, `hex_units`, or
`hex_combat`. The editor owns interactive UI, filesystem scanning and writing,
recovery drafts, captures, and editor-only entities. Reusable schemas, validation,
stable ids, deterministic serialization, and fingerprints live in `hex_assets`.

The editor uses the game's hex-prism geometry and coordinate vocabulary without
importing terrain storage. An object placement is object-local axial `(q, r)` plus a
signed integer level. It is never a runtime `TilePos`, a generated feature id, or a
numeric `SubstanceId`.

Tracked output lives under `assets/art/`:

| Path | Purpose |
|---|---|
| `palette.ron` | The canonical named colour vocabulary |
| `voxel_styles.ron` | Shared combinations of palette colour and render behavior |
| `objects/plant/*.ron` | Grounded plant blueprints |
| `objects/effect/*.ron` | Static effect sculptures, which may float |
| `objects/prop/*.ron` | Grounded or free-standing prop blueprints |

The palette policy and visual rationale are in
[visual-language.md](../design/visual-language.md).

## Stable identity

`SwatchId`, `VoxelStyleId`, and `ObjectAssetId` are validated path-like strings. They
use lowercase ASCII segments separated by `/`; each segment begins with a letter and
then contains letters, digits, or hyphens. Segments cannot be empty. IDs are immutable
after first save. Display names are editable and never resolve references.

Catalog maps and placement lists serialize in canonical sorted order. Palette, style,
and object semantics receive independent deterministic fingerprints, unaffected by RON
whitespace or comments. An object fingerprint includes referenced ids, not the
transitive RGB values behind those ids, so changing a shared swatch reports affected
objects without pretending their geometry changed.

Each file carries an explicit schema version. Unknown versions and unknown fields fail
closed; an older tool must not erase data it does not understand.

## Voxel styles

The **Voxel Styles** mode edits the shared style catalog while previewing one floating
game-sized voxel. A style contains:

- a stable id and editable display name;
- an exact base `SwatchId`;
- one render mode: `Opaque`, `Cutout`, `Translucent`, or `Additive`;
- finite opacity in `0.0 < opacity <= 1.0`;
- optional emission with an exact swatch and finite nonnegative strength.

Opaque styles require full opacity. Cutout maps to alpha-to-coverage. Translucent and
Additive use the corresponding Bevy blend modes. A missing swatch, non-finite number,
or invalid mode/opacity combination invalidates the catalog.

The mode also searches the palette and creates swatches deliberately through RGB,
HSV, or hex entry. Before an addition it shows the five nearest OKLab neighbours and
warns at distance `0.025` or less. Confirmation never merges or rewrites ids
automatically.

## Object blueprints

The **Objects** mode stacks style references on the same hex-prism grid used by the
game. The default canvas has radius 6 and 36 levels; authored bounds may grow to
radius 12 and 64 levels. A blueprint contains:

- stable identity, display name, and `Plant`, `Effect`, or `Prop` category;
- authoring bounds (`radius`, `min_level`, and `height`), an explicit voxel origin,
  and `Grounded` or `Free` connectivity;
- unique placements with a style id and a category-valid semantic part;
- an explicit horizontal blocker footprint;
- exact canopy-occluder cells.

No blueprint may exceed 8,192 occupied cells. Placements cannot overlap or fall
outside their bounds. Masks must refer to the blueprint's local coordinate space and
must satisfy the category contract.

Plants are always grounded with `min_level: 0`, place a root at the level-zero origin,
and require every occupied cell to be face-connected to it. Their blocker footprint is
exactly the horizontal footprint of their level-zero roots, and every canopy occluder
must name an occupied foliage voxel. Effects are always free, place a core at their
origin, may use signed levels and disconnected cells, and carry no blocker or canopy
mask. Props place structure at the origin and never carry canopy cells: grounded props
start at level zero and are connected, while free props may float. Plant parts are
root, trunk, branch, foliage, and accent; effect parts are core, trail, and accent;
prop parts are structure and detail.

Part labels, blocker footprints, and canopy cells are author intent. None is inferred
from colour, opacity, or occupancy, and none automatically creates gameplay damage,
light, traversal, or interaction. Runtime adapters will consume the exact authored
consequences in later work.

## Editing

Voxel Styles and Objects are modes in one application, sharing the viewport, palette,
asset browser, undo stack, dirty state, and save workflow.

- Place, Erase, Repaint, Eyedropper, and Select are explicit tools.
- Placement targets either the clicked voxel face or the active level slice.
- Right drag orbits, middle drag pans, and the wheel zooms.
- Top, front, side, and perspective snaps give repeatable inspection angles.
- Selection supports additive selection, axial and vertical nudging, copy/paste,
  delete, and exact 60-degree rotation around the object origin.
- A continuous paint stroke or one transform is one undoable command.
- UI input suppresses viewport tools; every destructive icon has a tooltip.

The object viewport can isolate an active level and independently overlay semantic
parts, blocker footprint, and canopy cells. Neutral, dark, and unlit preview rigs are
deterministic and independent of gameplay time-of-day.

## Saving and recovery

Tracked assets change only through explicit Save, Save As, or Duplicate. Before a
write, the tool validates the complete palette-style-object reference graph and shows
the global impact of shared changes. Invalid data leaves the last valid file intact.

Saving writes a sibling temporary file, verifies it, then atomically replaces the
destination. The editor records the on-disk fingerprint from load; if it changes
externally, overwrite is blocked until the author reloads or chooses a new id.
Referenced palette entries and styles cannot be deleted until their references are
removed or migrated.

Unsaved recovery is separate from source assets. The editor periodically writes a
draft under `.context/asset-workshop/recovery/`, which is gitignored and never loaded
by the game. Recovery never counts as an explicit save.

## Review output

One Review action writes deterministic artifacts beneath
`.context/asset-workshop/reviews/<asset-id>/<fingerprint>/`:

- perspective, top, and six 60-degree horizontal views;
- semantic-part and blocker/canopy overlays;
- a contact sheet;
- a report with identity, bounds, occupied-cell counts by style and part, mask sizes,
  validation result, dependencies, and fingerprints.

Review output is derived and untracked. A saved object remains the only source of
truth.

## Procedural follow-up

The exact authored object is both an exemplar and a fallback. Later generators emit
the same validated `ObjectBlueprint` shape and their results can be opened or
duplicated in the Workshop.

Regular branching plants may fit parameterized grammatical recipes; irregular mature
trees may fit rooted skeleton and crown-envelope recipes. Semantic parts constrain
those recipes. Illustrations remain human or agent references for authoring a reviewed
3D exemplar; arbitrary image-to-3D inference is outside this contract.

Static spell sculptures fit the object format. Timelines, emitters, moving particles,
and attachment events need a separate future VFX format rather than hidden fields in a
static blueprint.
