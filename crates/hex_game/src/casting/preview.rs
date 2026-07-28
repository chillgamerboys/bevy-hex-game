//! What an aimed spell would touch, painted on the ground.
//!
//! `hex_units::volumes` has resolved shapes to exact voxel sets since HEX-19a and had
//! no consumer at all; this is it. Three layers go down while a spell is aimed:
//!
//! | | |
//! |---|---|
//! | a faint marker | on every surface the anchor may legally sit on |
//! | a strong cap | on every surface inside the resolved volume |
//! | a bright cap | on the chosen anchor itself |
//!
//! All three take their colour from the spell's element, so what is being aimed is
//! legible without reading the panel.
//!
//! # Only surfaces can be painted
//!
//! A volume is three-dimensional and most of it is usually rock or air. Painting a
//! voxel with no surface would mean knowing how tall a level is in world units, and
//! `level_height` is a renderer fact the world owner owns and gameplay is forbidden to
//! know — that dependency is precisely what the crate split exists to prevent. So the
//! preview marks the surfaces the map published and nothing else, and the panel reports
//! both counts: a sphere that paints four hexes is touching a great deal more than four
//! voxels, and the player should not have to guess that.
//!
//! # A marker is a picking blocker, deliberately
//!
//! Every other overlay in this codebase carries `Pickable::IGNORE`, because three
//! separate bugs here came from an overlay eating the click that drives movement. The
//! anchor markers are the exception, and the exception is the mechanism: an anchor
//! marker sits between the camera and its tile, so a click on a lit surface reaches the
//! marker, aims the spell, and never also reaches `hex_units`'s global click-to-move
//! observer. Off the lit set there is no marker and a click still means "walk there".
//! The other two layers keep `Pickable::IGNORE` so they cannot shadow the markers.

use bevy::picking::events::{Click, Pointer};
use bevy::picking::Pickable;
use bevy::prelude::*;

use hex_assets::{GameAssets, TargetShape};
use hex_core::{Headroom, HexSpan, HexTile, Pause, TilePos};
use hex_units::{volumes, TerrainRevision};

use super::{facing_toward, in_range, Aim, Aiming, CastReadout};

/// Thickness of an overlay cap in world units, matched to `hex_units::selection` so the
/// casting layers read as the same kind of paint as the movement ones.
const CAP_THICKNESS: f32 = 0.02;

/// How much of a tile the anchor marker covers.
///
/// Nearly all of it, and for a reason the other two insets do not share: this layer is
/// the click target, and every uncovered sliver is a pixel where a click meant to aim
/// reaches the tile underneath and walks the caster there instead.
const ANCHOR_INSET: f32 = 0.98;

/// How much of a tile a volume cap covers. Inside the anchor marker, so a surface
/// carrying both still reads as one the anchor could move to.
const VOLUME_INSET: f32 = 0.72;

/// How much of a tile the chosen-anchor cap covers.
const AIM_INSET: f32 = 0.46;

/// How far above a surface the anchor marker floats.
///
/// Above both movement tints (0.01 and 0.05), so aiming reads on top of the range and
/// route the same surfaces may still be carrying.
const ANCHOR_LIFT: f32 = 0.09;

/// How far above a surface a volume cap floats.
const VOLUME_LIFT: f32 = 0.13;

/// How far above a surface the chosen-anchor cap floats.
const AIM_LIFT: f32 = 0.17;

/// Alpha for the anchor markers — a hint that a surface is legal, not a highlight.
const ANCHOR_ALPHA: f32 = 0.16;

/// Alpha for the volume caps: the answer to "what would this touch".
const VOLUME_ALPHA: f32 = 0.5;

/// Alpha for the chosen anchor.
const AIM_ALPHA: f32 = 0.95;

/// Tiles as the preview reads them.
///
/// Terrain is read off the entities rather than through `hex_map`, exactly as
/// `hex_units` does it: however the map is generated or stored, this keeps working.
/// [`Headroom`] comes along because it is what separates a surface somebody could look
/// at from a run buried inside a column.
type TileQuery<'w, 's> =
    Query<'w, 's, (&'static TilePos, &'static HexSpan, &'static Headroom), With<HexTile>>;

/// Marks a clickable marker over one legal anchor.
#[derive(Component, Debug)]
pub(super) struct AnchorMarker;

/// Marks a cap over one surface inside the resolved volume.
#[derive(Component, Debug)]
pub(super) struct VolumeMarker;

/// Marks the cap over the anchor currently chosen.
#[derive(Component, Debug)]
pub(super) struct AimMarker;

/// Everything this module draws, for clearing in one pass.
pub(super) type DrawnPreview = Or<(With<AnchorMarker>, With<VolumeMarker>, With<AimMarker>)>;

/// What the aimed spell would touch.
///
/// Published for the panel rather than recomputed there: the volume is resolved here
/// anyway, and two resolutions of one shape is two places for the number on screen to
/// stop matching the paint on the ground.
#[derive(Resource, Default, Debug, PartialEq, Eq)]
pub struct AimVolume {
    /// Every voxel the shape resolves to.
    pub voxels: usize,
    /// How many of those are surfaces the map published, and therefore painted.
    pub painted: usize,
}

/// What the drawn preview was drawn for.
///
/// `from` and `terrain` are in the key for the same reasons the movement preview keeps
/// them: the caster can walk while aiming, and an accepted terrain edit despawns and
/// respawns the entire grid, so every surface a marker was placed on may be gone.
///
/// `range` and `levels_per_bonus` are in it because **both come from hot-reloadable
/// content** — `spells.ron` and `combat.ron` — and neither changes the aim, the caster's
/// position or the terrain. Edit `levels_per_bonus_range` mid-fight and without them the
/// key still matches: the markers stay where the old rule put them while the applier
/// measures with the new one, so the interface offers a cast that is then refused. That
/// is the one direction of disagreement this module's header calls a bug.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DrawKey {
    aim: Aim,
    from: TilePos,
    terrain: u64,
    range: u32,
    levels_per_bonus: u32,
    shape: TargetShape,
}

/// What is currently on the ground.
#[derive(Resource, Default)]
pub(super) struct DrawnPreviewKey(Option<DrawKey>);

/// Redraws the preview when what it shows has changed, and not otherwise.
pub(super) fn redraw_preview(
    mut commands: Commands,
    mut drawn_key: ResMut<DrawnPreviewKey>,
    mut volume: ResMut<AimVolume>,
    readout: Res<CastReadout>,
    aiming: Res<Aiming>,
    assets: Option<Res<GameAssets>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    revision: Res<TerrainRevision>,
    tiles: TileQuery,
    drawn: Query<Entity, DrawnPreview>,
) {
    let Some(assets) = assets else { return };

    let wanted = aiming
        .0
        .as_ref()
        .zip(readout.caster)
        .map(|(aim, caster)| DrawKey {
            aim: aim.clone(),
            from: caster.standing,
            terrain: revision.0,
            range: readout.row(&aim.spell).map_or(0, |row| row.range),
            levels_per_bonus: readout.levels_per_bonus,
            shape: readout
                .row(&aim.spell)
                .map_or(TargetShape::Single, |row| row.shape.clone()),
        });
    if drawn_key.0 == wanted {
        return;
    }
    for marker in &drawn {
        commands.entity(marker).despawn();
    }
    drawn_key.0 = wanted.clone();

    let Some(key) = wanted else {
        set_volume(&mut volume, AimVolume::default());
        return;
    };
    let Some(row) = readout.row(&key.aim.spell) else {
        set_volume(&mut volume, AimVolume::default());
        return;
    };

    // One material per layer per redraw, shared by every cap in that layer. Three
    // materials for a hundred caps, following `hex_units::selection`'s rule; they are
    // dropped along with the caps on the next redraw, so nothing accumulates in
    // `Assets<StandardMaterial>` however long a fight runs.
    let anchor_material = materials.add(cap_material(row.color.with_alpha(ANCHOR_ALPHA), 3.0));
    let volume_material = materials.add(cap_material(row.color.with_alpha(VOLUME_ALPHA), 4.0));
    let aim_material = materials.add(cap_material(row.color.with_alpha(AIM_ALPHA), 5.0));

    // Only surfaces with room above them: a run buried inside a column has a `TilePos`
    // and no visible face, so a marker there would be paint inside rock. This makes the
    // preview *stricter* than the applier, which is the safe direction — the applier
    // would accept a buried anchor, and there is simply no way to offer one.
    let surfaces: Vec<(TilePos, f32)> = tiles
        .iter()
        .filter(|(_, _, headroom)| headroom.0 > 0)
        .map(|(pos, span, _)| (*pos, span.top))
        .collect();

    let anchors = legal_anchors(
        &surfaces,
        key.from,
        row.range,
        readout.levels_per_bonus,
        &row.shape,
    );
    for (pos, top) in &anchors {
        commands.spawn((
            Name::new("Cast Anchor"),
            AnchorMarker,
            // Keyed on the position and never on the tile entity: an accepted terrain
            // edit despawns and respawns the entire grid, invalidating every tile id.
            *pos,
            cap(
                &assets,
                &anchor_material,
                *pos,
                *top,
                ANCHOR_INSET,
                ANCHOR_LIFT,
            ),
        ));
    }

    let facing = volumes::needs_facing(&row.shape)
        .then(|| facing_toward(key.from.coord, key.aim.anchor.coord));
    let voxels = volumes::resolve(&row.shape, key.from, key.aim.anchor, facing).unwrap_or_default();
    let mut painted = 0;
    for (pos, top) in &surfaces {
        // A resolver hands back its volume sorted and deduplicated — the canonical form
        // an announcement requires — so membership is a binary search rather than a
        // scan over every surface on the map.
        if voxels.binary_search(pos).is_err() {
            continue;
        }
        painted += 1;
        commands.spawn((
            Name::new("Cast Volume"),
            VolumeMarker,
            *pos,
            cap(
                &assets,
                &volume_material,
                *pos,
                *top,
                VOLUME_INSET,
                VOLUME_LIFT,
            ),
            // Above the anchor markers, and so the layer that would shadow them. Deaf
            // to picking, or the click that should aim would land on a cap that has
            // nothing to say and the whole interaction would go quiet.
            Pickable::IGNORE,
        ));
    }

    if let Some((pos, top)) = anchors.iter().find(|(pos, _)| *pos == key.aim.anchor) {
        commands.spawn((
            Name::new("Cast Aim"),
            AimMarker,
            *pos,
            cap(&assets, &aim_material, *pos, *top, AIM_INSET, AIM_LIFT),
            Pickable::IGNORE,
        ));
    }

    set_volume(
        &mut volume,
        AimVolume {
            voxels: voxels.len(),
            painted,
        },
    );
}

/// Writes the volume counts only when they moved, so the panel's change detection stays
/// honest.
fn set_volume(volume: &mut ResMut<AimVolume>, next: AimVolume) {
    if **volume != next {
        **volume = next;
    }
}

/// Every surface this spell's anchor may legally sit on, with the height to paint it at.
///
/// `SelfCast` is the one shape whose range is not a question — the applier skips the
/// check for it — so it offers exactly the caster's own surface rather than a field of
/// anchors that would all resolve to the same volume.
fn legal_anchors(
    surfaces: &[(TilePos, f32)],
    from: TilePos,
    range: u32,
    levels_per_bonus: u32,
    shape: &TargetShape,
) -> Vec<(TilePos, f32)> {
    surfaces
        .iter()
        .filter(|(pos, _)| match shape {
            TargetShape::SelfCast => *pos == from,
            _ => in_range(from, *pos, range, levels_per_bonus),
        })
        .copied()
        .collect()
}

/// Points the aim at the surface whose marker was clicked.
///
/// Global, like every other picking observer here, and written for it: a marker that is
/// not one of ours falls out at the query, and every resource it touches either exists
/// from plugin build (`Aiming`) or is taken as an `Option` (`Pause` is a sub-state of
/// gameplay and does not exist on the title screen). Bevy validates system parameters
/// *before* the body runs, so a plain `Res` there is a crash this codebase has shipped
/// once already.
///
/// The pause guard is the one `hex_units`'s click observer carries, for the same
/// reason: a click through the pause overlay must mean nothing at all.
///
/// **Left button only.** `Pointer<Click>` fires for every button, including the right one
/// the camera orbits with — and while a spell is aimed these markers blanket the whole
/// near field the player orbits *around*. Without the check, a short right-drag that
/// begins and ends over the same marker silently re-aims the spell, and the next Confirm
/// casts at a voxel nobody chose.
pub(super) fn on_anchor_clicked(
    click: On<Pointer<Click>>,
    markers: Query<&TilePos, With<AnchorMarker>>,
    mut aiming: ResMut<Aiming>,
    pause: Option<Res<State<Pause>>>,
) {
    if click.button != PointerButton::Primary {
        return;
    }
    if pause.is_some_and(|pause| pause.get().0) {
        return;
    }
    let Ok(anchor) = markers.get(click.event_target()) else {
        return;
    };
    let Some(aim) = aiming.0.as_ref() else {
        return;
    };
    if aim.anchor == *anchor {
        return;
    }
    let spell = aim.spell.clone();
    aiming.0 = Some(Aim {
        spell,
        anchor: *anchor,
    });
}

/// An unlit, blended cap material.
///
/// **`alpha_mode` is set explicitly and must stay that way.**
/// `StandardMaterial::from(Color)` infers `Blend` when the alpha is below one; a struct
/// literal like this one does not, and leaves it `Opaque` — which silently discards the
/// alpha and draws a solid slab over the terrain. `unlit` because a highlight that dims
/// as the sun moves reads as a lighting fault rather than as a highlight.
fn cap_material(color: Color, depth_bias: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        depth_bias,
        ..default()
    }
}

/// One cap, sitting on a surface.
///
/// The tile mesh is origin-centred and exactly one unit tall, so scaling Y by the
/// thickness and translating to `top + lift + thickness / 2` puts its underside at
/// `top + lift`. Building a [`HexSpan`] for it would be the obvious thing and is wrong:
/// `HexSpan::new` asserts that the top is above the bottom, and a cap this thin is
/// exactly the degenerate case it refuses.
fn cap(
    assets: &GameAssets,
    material: &Handle<StandardMaterial>,
    pos: TilePos,
    top: f32,
    inset: f32,
    lift: f32,
) -> impl Bundle {
    (
        Mesh3d(assets.hex_tile.clone()),
        MeshMaterial3d(material.clone()),
        Transform {
            translation: pos.coord.to_world(top + lift + CAP_THICKNESS * 0.5),
            scale: Vec3::new(inset, CAP_THICKNESS, inset),
            ..default()
        },
    )
}

/// Clears the preview on leaving gameplay.
///
/// The caps are plain world entities rather than children of anything torn down with
/// the screen, so nothing else would take them with it.
pub(super) fn clear_preview(
    mut commands: Commands,
    mut drawn_key: ResMut<DrawnPreviewKey>,
    mut volume: ResMut<AimVolume>,
    drawn: Query<Entity, DrawnPreview>,
) {
    for marker in &drawn {
        commands.entity(marker).despawn();
    }
    drawn_key.0 = None;
    *volume = AimVolume::default();
}
