# Asset Workshop

The contract for the standalone voxel-style and object-authoring tool. The Workshop is
development tooling: it creates durable RON assets for the game, but it is not a game
screen and does not run through the gameplay setup lifecycle.

The contracts, tracked catalogs, and standalone `hex_editor` authoring application
are live. Static runtime object rendering is also live through `hex_objects`. Forest
migration, procedural plant generation, reference-image import, and animated effect
timelines remain separate later work.

## Launch

From anywhere inside the repository, run:

```sh
cargo editor
```

This is a separate binary. `cargo dev` remains pinned to `hex_game` and never opens
the Workshop.

The editor walks upward from the current directory until it finds the palette,
voxel-style, and object catalogs under `assets/art/`. To author a different checkout,
or when the current directory is outside one, provide its repository root explicitly:

```sh
cargo editor -- --project-root /path/to/bevy-hex-game
```

Both `--project-root PATH` and `--project-root=PATH` are accepted. A relative path is
resolved from the current directory. The editor opens an unsaved calibration scene;
tracked files change only after an explicit Save, Save As, or Duplicate action.

## Boundary

`hex_editor` is an isolated Bevy application that depends on `hex_core` and
`hex_assets`. It does not depend on `hex_game`, `hex_map`, `hex_world`, `hex_units`, or
`hex_combat`. The editor owns interactive UI, filesystem scanning and writing, and
editor-only entities, recovery drafts, and deterministic review captures. Reusable
schemas, validation, stable ids, deterministic serialization, and fingerprints live
in `hex_assets`.

The editor uses the game's hex-prism geometry and coordinate vocabulary without
importing terrain storage. An object placement is object-local axial `(q, r)` plus a
signed integer level. It is never a runtime `TilePos`, a generated feature id, or a
numeric `SubstanceId`.

Tracked output lives under `assets/art/`:

| Path | Purpose |
|---|---|
| `palette.ron` | The canonical named colour vocabulary |
| `voxel_styles.ron` | Shared combinations of palette colour and render behavior |
| `object_catalog.ron` | Sorted identities of every tracked runtime object |
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

An `ObjectAssetId` has an additional persistence contract: it is exactly
`<category>/<filename>`, where the singular category is `plant`, `effect`, or `prop`
and `filename` is one id segment. The category must match the blueprint's typed
category, and nested object paths are rejected. For example, `plant/oak` is stored at
`assets/art/objects/plant/oak.ron`; neither `plants/oak` nor
`plant/temperate/oak` is valid.

Catalog maps and placement lists serialize in canonical sorted order. Palette, style,
and object semantics receive independent deterministic fingerprints, unaffected by RON
whitespace or comments. An object fingerprint includes referenced ids, not the
transitive RGB values behind those ids, so changing a shared swatch reports affected
objects without pretending their geometry changed.

`object_catalog.ron` is the packaged-build index; runtime code never scans an
untyped directory of RON files. Its sorted ids must exactly match the tracked object
files. Object Save As, Duplicate, and Delete publish the blueprint and catalog as one
rollback-protected operation, so neither a stale catalog nor an orphan file becomes
the visible project state. Each replacement is atomic, but the pair is not a
journaled transaction across a process crash; a partial pair is rejected as an
incoherent graph the next time the Workshop loads.

Tracked Workshop RON files are machine-written. Explicit saves replace the complete
document and do not preserve comments; contract rationale belongs in the documentation.

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

- Place, Erase, Repaint, Eyedropper, and Select are explicit tools. Repaint applies
  both the active style and active semantic role.
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

### Mouse controls

| Input | Action |
|---|---|
| Left click | Apply the active tool at the addressed cell |
| Left drag | Continue one Place, Erase, or Repaint stroke |
| `Shift` + left click | Add an occupied cell to the selection |
| Right drag | Orbit around the current focus |
| Middle drag | Pan the focus |
| `Space` + left drag | Pan without a middle mouse button |
| `Shift` + right drag | Pan without a middle mouse button |
| Mouse wheel | Zoom |

Place uses the outward face of an occupied voxel when one is clicked. Clicking an
empty guide cell places on the active level. UI panels own the pointer while hovered,
so dragging a control never edits or moves the scene behind it.

Placement strokes clip cells outside the authored radius or level range and report one
summary when the stroke ends; valid cells remain one undoable command. Any other model
failure cancels the complete stroke and restores its starting state.

### Keyboard controls

On macOS, `Command` is the command modifier; on Windows and Linux it is `Ctrl`.
Shortcuts are suspended while a text field or modal dialog owns keyboard input.

| Input | Action |
|---|---|
| `Command/Ctrl` + `S` | Save the current catalogs or existing object |
| `Command/Ctrl` + `Shift` + `S` | Save the current object under a new immutable id |
| `Command/Ctrl` + `Z` | Undo |
| `Command/Ctrl` + `Shift` + `Z` or `Ctrl` + `Y` | Redo |
| `P`, `E`, `R`, `I`, `V` | Select Place, Erase, Repaint, Eyedropper, or Select |
| `Command/Ctrl` + `C`, `Command/Ctrl` + `V` | Copy or paste the current selection |
| `Delete` or `Backspace` | Delete the current selection |
| `Alt` + `Up`, `Alt` + `Down` | Move the active level by one |
| `F` | Frame the visible authored voxels |

Top, Front, Side, and 3D camera snaps, exact selection nudges, 60-degree rotation, and
overlay controls remain available in the toolbar and object inspector.

## Authoring workflow

1. In **Voxel Styles**, search the canonical palette before adding a swatch. When a
   new color is justified, assign its permanent id, name, and tags, inspect the five
   nearest OKLab matches, and explicitly confirm any near-color warning.
2. Create or edit a voxel style that refers to exact palette swatches. Check opaque,
   cutout, translucent, additive, and emitted treatments in Neutral, Dark, and Unlit
   preview rigs as appropriate.
3. In **Objects**, create a Plant, Effect, or Prop draft. Choose the id and category
   deliberately: the id determines the tracked path and cannot be renamed after the
   first save.
4. Build the form with role-aware voxels, then author the origin, blocker footprint,
   and canopy cells explicitly. Use level isolation and repeatable camera snaps to
   inspect dense or stacked geometry.
5. Resolve every validation error. Inspect semantic and blocker/canopy overlays, save
   explicitly, reload the asset, and verify that transforms and masks survived the
   round trip.
6. Export a review pack from the clean saved object. Review the contact sheet and
   report together; the RON object remains the source of truth.

Do not add calibration objects, experiments, or generated candidates to production
assets merely to exercise the editor. Use an alternate `--project-root` or an
untracked working copy under `.context/asset-workshop/` for disposable work.

## Explicit persistence

Tracked assets change only through explicit persistence actions: Save, Save As,
Duplicate, or a confirmed Delete. Before a write, the tool validates the complete
palette-style-object reference graph and shows the global impact of shared changes.
Invalid data leaves the last valid file intact.

In **Voxel Styles**, Save writes the palette and style catalogs as one validated
operation. In **Objects**, Save updates an already tracked object, Save As assigns a
new id and destination, and Duplicate creates a new tracked asset from an existing
saved object. A new or calibration object therefore requires Save As before ordinary
Save is available.

Saving writes same-directory temporary files and atomically replaces each destination.
When both catalogs change, it chooses an order whose intermediate graph is valid; if
the second replacement fails, it atomically restores the first file and reports any
restore failure without advancing the in-memory project. Referenced palette entries
and styles cannot be deleted until their references are removed or migrated.

The Workshop periodically compares every loaded tracked art file with its exact source
bytes. This catches formatting-only edits as well as semantic changes, additions, and
deletions. When another process changes the project after load, tracked overwrites are
blocked. Reload to accept the disk state as the new baseline. Save As can preserve an
object draft under a new id when the shared catalogs themselves are unchanged. Reload
discards local catalog and object drafts, so it requires confirmation when work is
dirty.

## Recovery and closing

Recovery is deliberately untracked and never substitutes for Save. While work is
dirty, the Workshop writes a recovery snapshot after roughly three idle seconds and
at least every thirty seconds during continuous editing. An open paint or transform
transaction is never captured halfway through. The active file is:

```text
.context/asset-workshop/recovery/workshop-v1.ron
```

The snapshot contains the palette and style drafts, current and last-saved object
checkpoints, document identity, mode, tool, active level/style/part, preview rig,
selection, and the exact tracked-file baseline. Undo history and clipboard contents
are session-only.

At the next launch, the Workshop never restores a draft silently. It offers Restore or
Discard before authoring continues. A malformed or newer recovery schema is left
untouched and autosave pauses until the author explicitly discards it. Recovery accepts
temporarily invalid object geometry so an interrupted stroke can be recovered, but the
normal production deserializer and every tracked save still enforce the full object
contract.

If tracked files changed after the recovery snapshot, Restore preserves the draft but
marks a recovery conflict. **Reconcile** performs a three-way merge between the
recovery checkpoint, recovered catalog edits, and current tracked catalogs. Independent
changes are retained from both sides; same-id conflicts are named and remain blocked
until the author explicitly chooses **Recovered Wins** or **Tracked Wins** (the choice
applies only to same-id conflicts), or reloads. If tracked catalogs change again after
reconciliation, saving blocks until the author reconciles the new baseline. A dirty
recovered tracked object whose source also changed must be preserved through Save As
before its old destination can be overwritten. A clean recovered object instead adopts
the current tracked version.
Reconciled catalogs may be saved first when that new object depends on a recovered
style. Closing a dirty session first flushes recovery, then requires Save All and Quit,
Keep Recovery and Quit, Discard and Quit, or Cancel. A clean explicit save removes
obsolete recovery state.

## Review output

Review is available only for a clean, saved object whose complete palette, style, and
object dependency graph validates. One Review action renders ten fixed 1024-by-1024
frames and publishes deterministic artifacts beneath
`.context/asset-workshop/reviews/<asset-id>/<fingerprint>/`:

- `01-perspective.png` and `02-top.png`;
- six authored-material turns from 0 through 300 degrees;
- `09-semantic.png` and `10-blocker-canopy.png`;
- a fixed four-by-three `contact-sheet.png`;
- a canonical `report.ron` with identity, bounds, origin, connectivity, occupied-cell
  counts by style and part, exact masks, resolved style and swatch dependencies,
  framing, validation, and independent object, style-catalog, and palette
  fingerprints plus the exact renderer-mesh byte revision.

Review framing, the neutral render rig, file order, and report format are versioned.
The report excludes absolute paths, timings, and machine or GPU details. The composite
review fingerprint changes when the object, style catalog, or palette semantics
change, regardless of RON formatting, and when the shared voxel mesh bytes change.

Frames are rendered to an offscreen target and rejected if they are blank, black, or
the wrong size. The tool stages the complete pack beside its final directory and
publishes it with one rename, so a failure cannot expose a partial review. Exporting
the same bytes again reuses the existing directory; a different pack claiming the
same fingerprint is reported as a collision and never overwrites it. Review output is
derived and untracked. A saved object remains the only source of truth.

At startup, the Workshop copies `assets/meshes/hex.glb` to an untracked,
content-addressed cache and renders that immutable copy. Request creation, renderer
startup, and publication each verify the exact tracked art-source revisions and mesh
bytes used to prepare the review.
Publication checks once before staging and again immediately before the atomic rename.
A source change at any of those points aborts the export, removes its staging
directory, and requires a project reload or fresh review request.

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

## Runtime rendering

`hex_assets` loads and validates the complete palette → style → object graph. Initial
failure keeps the loading screen closed; a bad later edit retains the previous
coherent graph. `hex_objects` consumes `ObjectInstance` components and draws cached
mesh chunks grouped by style and canopy membership. Repeated instances share those
meshes and materials, and object voxels never become one ECS entity each.

Run `cargo object-gallery` to inspect all six rotations of the first production
object under the neutral rig. Set `HEX_OBJECT_GALLERY_OBJECT=<object-id>` to inspect
any tracked object; the camera and layout scale to its authored bounds. Set
`HEX_OBJECT_GALLERY_RIG=dark` for the matching dark render without changing the asset
or its runtime material. Set
`HEX_OBJECT_GALLERY_CAPTURE=<path.png>` to capture the selected rig through an
offscreen target and exit automatically. Set
`HEX_OBJECT_GALLERY_MATERIAL_FIXTURES=1` to add transient opaque, cutout, translucent,
additive, and emissive samples without adding them to the tracked object catalog.

The instance origin is the exact world voxel occupied by the blueprint origin, not
the supporting terrain surface. Its validated level height supplies vertical scale,
and rotation is one of six exact 60-degree turns. The renderer handles material
appearance, shadows, ignored picking, and lifecycle only. It does not apply blocker
footprints or infer gameplay from semantic parts.

Blend chunks use Bevy's native order-independent transparency and remove shared
internal faces before baking. Bevy 0.19 requires OIT cameras to use `Msaa::Off`, so
Cutout styles temporarily render as single-sample threshold masks while any Blend
object is live. The renderer restores each camera's previous MSAA setting after the
last Blend chunk leaves, which restores true alpha-to-coverage for Cutout styles.

The Workshop and renderer still do not replace Forest's temporary vegetation,
synthesize plants, import reference images, animate spell effects, or provide a
runtime construction system. Forest integration is world-side work: the map may
publish `ObjectInstance` for the authored visual while continuing to publish exact
blockers separately. Authored canopy chunks need a separate world/presentation
adapter before they can replace Forest's current root-keyed `CanopyOccluder`.
`hex_objects` must not infer either gameplay fact from `root`, `trunk`, or `foliage`
parts. Procedural plant synthesis follows only after the authored exemplar path is
reviewed; neither stage widens the object schema with renderer- or biome-specific
policy.

Common launch, save, recovery, and review failures are indexed in
[troubleshooting.md](../development/troubleshooting.md#asset-workshop).
