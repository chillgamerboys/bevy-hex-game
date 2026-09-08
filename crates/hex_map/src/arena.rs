//! Small deterministic arena producer, independent of the tactical loading pipeline.
//!
//! Authoritative mutation and publication run together in `ArenaTick`. Disposable
//! per-column run meshes catch up in `PostUpdate` without becoming collision truth.

use std::collections::{BTreeMap, BTreeSet};

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use hex_assets::{
    ArtPalette, ElementCatalog, ElementFile, RuntimeArtCatalog, SubstanceFile, SubstanceTable,
    TerrainDamageFile, TerrainDamageTable,
};
use hex_core::arena::{
    ArenaMaterials, ArenaReset, ArenaSelection, ArenaSolidSpan, ArenaSystems, ArenaTerrainView,
    ArenaTick, ArenaVoxelGeometry,
};
use hex_core::{
    DamagedVoxels, HexCoord, SubstanceId, TerrainEdit, TerrainImpact, TerrainImpactOutcome,
    TerrainImpactRejection, TerrainImpactResult, TilePos,
};

use crate::terrain_damage::TerrainDamageState;
use crate::{Column, VoxelMap};

#[cfg(test)]
mod real_world_tests;
mod render;
#[cfg(test)]
mod tests;
mod worlds;

const GROUND_LEVEL: i32 = 8;

#[derive(Resource, Default)]
struct ArenaWorldState {
    generation: u64,
    changed: BTreeSet<HexCoord>,
    render_dirty: BTreeSet<HexCoord>,
    original: Option<worlds::WorldRecipe>,
    full_rebuild: bool,
    presentation_dirty: bool,
}

#[derive(Resource, Default)]
struct ArenaInbox {
    edits: Vec<TerrainEdit>,
    impacts: Vec<TerrainImpact>,
}

/// Registers the standalone arena world, production terrain messages, and rendering.
///
/// Startup publication belongs to `ArenaSystems::PublishTerrain`; actor setup may
/// order after that set or wait for the first `ArenaTick`. The integration owner
/// drives the tick schedule at its chosen fixed rate and pauses it as a whole.
pub fn plugin(app: &mut App) {
    app.init_schedule(ArenaTick)
        .configure_sets(
            ArenaTick,
            (
                ArenaSystems::ApplyTerrain,
                ArenaSystems::PublishTerrain,
                ArenaSystems::Simulate,
            )
                .chain(),
        )
        .init_resource::<ArenaReset>()
        .init_resource::<ArenaSelection>()
        .init_resource::<DamagedVoxels>()
        .init_resource::<TerrainDamageState>()
        .init_resource::<ArenaWorldState>()
        .init_resource::<ArenaInbox>()
        .add_message::<TerrainEdit>()
        .add_message::<TerrainImpact>()
        .add_message::<TerrainImpactOutcome>()
        .add_message::<AppExit>()
        .add_systems(Startup, initialize.in_set(ArenaSystems::PublishTerrain))
        .add_systems(PreUpdate, retain_announcements)
        .add_systems(
            ArenaTick,
            apply_terrain
                .in_set(ArenaSystems::ApplyTerrain)
                .run_if(resource_exists::<VoxelMap>),
        )
        .add_systems(
            ArenaTick,
            publish_terrain
                .in_set(ArenaSystems::PublishTerrain)
                .run_if(resource_exists::<VoxelMap>),
        )
        .add_plugins(render::plugin);
}

/// Preserve the last simulated tick's effects while the complete tick schedule is
/// paused. Reading every frame prevents ordinary Bevy message retention expiring;
/// admission and mutation still happen only inside the authoritative tick.
fn retain_announcements(
    mut inbox: ResMut<ArenaInbox>,
    mut edits: ResMut<Messages<TerrainEdit>>,
    mut impacts: ResMut<Messages<TerrainImpact>>,
) {
    inbox.edits.extend(edits.drain());
    inbox.impacts.extend(impacts.drain());
}

struct Content {
    substances: SubstanceTable,
    elements: ElementCatalog,
    damage: TerrainDamageTable,
    materials: ArenaMaterials,
    art: RuntimeArtCatalog,
}

/// Embed the accepted source files together so an arena executable never mixes
/// catalogs from another checkout or starts with asynchronously missing content.
fn load_content() -> Result<Content, String> {
    let palette: ArtPalette = ron::from_str(include_str!("../../../assets/art/palette.ron"))
        .map_err(|error| format!("Arena palette: {error}"))?;
    let substance_file: SubstanceFile =
        ron::from_str(include_str!("../../../assets/config/substances.ron"))
            .map_err(|error| format!("Arena substances: {error}"))?;
    let element_file: ElementFile =
        ron::from_str(include_str!("../../../assets/config/elements.ron"))
            .map_err(|error| format!("Arena elements: {error}"))?;
    element_file.validate()?;
    let damage_file: TerrainDamageFile =
        ron::from_str(include_str!("../../../assets/config/terrain_damage.ron"))
            .map_err(|error| format!("Arena terrain damage: {error}"))?;
    let substances = SubstanceTable::from_file(&substance_file, &palette)
        .map_err(|error| format!("Arena substance catalog: {error}"))?;
    let elements = ElementCatalog::from_file(&element_file);
    let damage = TerrainDamageTable::from_file(&damage_file, &elements, &substances)
        .map_err(|errors| format!("Arena damage catalog: {errors:?}"))?;
    let id = |name| {
        substances
            .id(name)
            .ok_or_else(|| format!("Arena missing substance {name}"))
    };
    let materials = ArenaMaterials {
        stone: id("stone")?,
        bedrock: id("bedrock")?,
        grass: id("grass")?,
        dirt: id("dirt")?,
        fire: elements
            .id("Fire")
            .ok_or_else(|| "Arena missing Fire element".to_owned())?,
    };
    Ok(Content {
        substances,
        elements,
        damage,
        materials,
        art: worlds::load_art(&palette)?,
    })
}

fn initialize(world: &mut World) {
    let content = match load_content() {
        Ok(content) => content,
        Err(error) => {
            error!("{error}");
            world.write_message(AppExit::error());
            return;
        }
    };
    let selection = *world.resource::<ArenaSelection>();
    let recipe = match worlds::build(
        selection,
        content.materials,
        &content.substances,
        &content.art,
    ) {
        Ok(recipe) => recipe,
        Err(error) => {
            error!("{error}");
            world.write_message(AppExit::error());
            return;
        }
    };
    let generation = world.resource::<ArenaReset>().generation;
    world.insert_resource(ArenaWorldState {
        generation,
        render_dirty: recipe.map.columns().map(|(coord, _)| coord).collect(),
        original: Some(recipe.clone()),
        presentation_dirty: true,
        ..default()
    });
    world.insert_resource(content.substances);
    world.insert_resource(content.elements);
    world.insert_resource(content.damage);
    world.insert_resource(content.materials);
    world.insert_resource(content.art);
    world.insert_resource(recipe.geometry);
    world.insert_resource(recipe.map);
    world.insert_resource(recipe.view);
    world.insert_resource(recipe.presentation);
}

fn spawn_positions(geometry: ArenaVoxelGeometry) -> [Vec3; 2] {
    [-8, 8].map(|q| {
        let pos = TilePos::new(HexCoord::from_axial(q, 0), GROUND_LEVEL);
        pos.coord.to_world(geometry.top(pos))
    })
}

/// Flat open firing lane at r=0, mirrored side platforms and five one-level
/// ramp steps, plus isolated cover that leaves both spawns and side routes open.
fn build_arena(geometry: ArenaVoxelGeometry, materials: ArenaMaterials) -> VoxelMap {
    let mut map = VoxelMap::new();
    for coord in HexCoord::ORIGIN.within_radius(geometry.radius) {
        let mut column = Column::new();
        for level in 0..=GROUND_LEVEL {
            let substance = match level {
                0 => materials.bedrock,
                1..=3 => materials.stone,
                GROUND_LEVEL => materials.grass,
                _ => materials.dirt,
            };
            column.set(level, substance);
        }
        map.insert_column(coord, column);
    }
    for side in [-1, 1] {
        let center = HexCoord::from_axial(-5 * side, 6 * side);
        for coord in center.within_radius(2) {
            fill_above_ground(&mut map, geometry, coord, 5, materials.stone);
        }
        for step in 1..=5 {
            let coord = HexCoord::from_axial((3 - step) * side, 6 * side);
            fill_above_ground(&mut map, geometry, coord, step, materials.stone);
        }
        for q in -3..=-1 {
            let coord = HexCoord::from_axial(q * side, 2 * side);
            fill_above_ground(&mut map, geometry, coord, 4, materials.stone);
        }
        for r in 2..=3 {
            let coord = HexCoord::from_axial(7 * side, r * side);
            fill_above_ground(&mut map, geometry, coord, 3, materials.stone);
        }
    }
    map
}

fn fill_above_ground(
    map: &mut VoxelMap,
    geometry: ArenaVoxelGeometry,
    coord: HexCoord,
    height: i32,
    substance: SubstanceId,
) {
    if geometry.contains_column(coord) {
        for level in GROUND_LEVEL + 1..=GROUND_LEVEL + height {
            map.set(TilePos::new(coord, level), substance);
        }
    }
}

#[derive(SystemParam)]
struct ArenaContentRefs<'w> {
    substances: Res<'w, SubstanceTable>,
    elements: Res<'w, ElementCatalog>,
    damage_table: Res<'w, TerrainDamageTable>,
    art: Res<'w, RuntimeArtCatalog>,
}

fn apply_terrain(
    reset: Res<ArenaReset>,
    selection: Res<ArenaSelection>,
    mut geometry: ResMut<ArenaVoxelGeometry>,
    materials: Res<ArenaMaterials>,
    content: ArenaContentRefs,
    mut map: ResMut<VoxelMap>,
    mut state: ResMut<ArenaWorldState>,
    mut presentation: ResMut<crate::procedural_v3::MapPresentationProjection>,
    mut inbox: ResMut<ArenaInbox>,
    mut damage: ResMut<TerrainDamageState>,
    mut damaged: ResMut<DamagedVoxels>,
    mut edits: ResMut<Messages<TerrainEdit>>,
    mut impacts: ResMut<Messages<TerrainImpact>>,
    mut outcomes: ResMut<Messages<TerrainImpactOutcome>>,
    mut exit: MessageWriter<AppExit>,
) {
    let ArenaContentRefs {
        substances,
        elements,
        damage_table,
        art,
    } = content;
    if state.generation != reset.generation {
        let cached = state
            .original
            .as_ref()
            .filter(|recipe| recipe.view.selection.map == selection.map);
        let mut recipe = match cached
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| worlds::build(*selection, *materials, &substances, &art))
        {
            Ok(recipe) => recipe,
            Err(error) => {
                error!("{error}");
                exit.write(AppExit::error());
                return;
            }
        };
        recipe.view.selection = *selection;
        state.changed.extend(map.columns().map(|(coord, _)| coord));
        state.generation = reset.generation;
        *map = recipe.map.clone();
        *geometry = recipe.geometry;
        *presentation = recipe.presentation.clone();
        state.original = Some(recipe);
        state.full_rebuild = true;
        state.presentation_dirty = true;
        damage.reset(&mut damaged);
        edits.clear();
        impacts.clear();
        outcomes.clear();
        inbox.edits.clear();
        inbox.impacts.clear();
        state.changed.extend(map.columns().map(|(coord, _)| coord));
        return;
    }
    for edit in std::mem::take(&mut inbox.edits)
        .into_iter()
        .chain(edits.drain())
    {
        let pos = edit.pos();
        if !geometry.contains_column(pos.coord)
            || !(geometry.min_level..=geometry.max_level).contains(&pos.level)
            || protected(&state, pos)
        {
            continue;
        }
        let current = map.get(pos);
        let replacement = match edit {
            TerrainEdit::Set { substance, .. } => substance,
            TerrainEdit::Clear { .. } => SubstanceId::AIR,
        };
        if current == replacement
            || (!current.is_air() && !substances.is_diggable(current))
            || substances.get(replacement).is_none()
        {
            continue;
        }
        map.set(pos, replacement);
        damage.forget_voxel(pos, &mut damaged);
        state.changed.insert(pos.coord);
    }
    for impact in std::mem::take(&mut inbox.impacts)
        .into_iter()
        .chain(impacts.drain())
    {
        let rejection = if !damage.consume_batch(impact.batch) {
            Some(TerrainImpactRejection::ReusedBatch)
        } else if let Some(reason) = impact.structural_rejection() {
            Some(reason)
        } else if impact
            .kind
            .element()
            .is_some_and(|element| elements.name(element).is_none())
        {
            Some(TerrainImpactRejection::UnknownElement)
        } else {
            None
        };
        if let Some(reason) = rejection {
            outcomes.write(TerrainImpactOutcome {
                batch: impact.batch,
                result: TerrainImpactResult::Rejected(reason),
            });
            continue;
        }
        let resolved = damage.apply(
            impact,
            &mut map,
            &substances,
            &damage_table,
            &mut damaged,
            |pos| {
                !geometry.contains_column(pos.coord)
                    || !(geometry.min_level..=geometry.max_level).contains(&pos.level)
                    || protected(&state, pos)
            },
        );
        state
            .changed
            .extend(resolved.destroyed.iter().map(|pos| pos.coord));
        outcomes.write(resolved.outcome);
    }
    if !state.changed.is_empty() {
        let before = presentation.features().len();
        presentation.retain_features(|feature| {
            feature.kind == crate::procedural_v3::FeatureKind::Tree
                || !state.changed.contains(&feature.root.coord)
                || (substances.is_solid(map.get(feature.root))
                    && map.get(feature.root.above()).is_air())
        });
        state.presentation_dirty |= presentation.features().len() != before;
    }
}

fn protected(state: &ArenaWorldState, pos: TilePos) -> bool {
    state
        .original
        .as_ref()
        .and_then(|recipe| recipe.view.edit_protected.get(&pos.coord))
        .is_some_and(|intervals| {
            intervals
                .iter()
                .any(|(bottom, top)| (*bottom..=*top).contains(&pos.level))
        })
}

fn publish_terrain(
    map: Res<VoxelMap>,
    substances: Res<SubstanceTable>,
    mut state: ResMut<ArenaWorldState>,
    mut view: ResMut<ArenaTerrainView>,
) {
    if state.changed.is_empty() {
        return;
    }
    let changed = std::mem::take(&mut state.changed);
    let revision = view.revision.saturating_add(1);
    if std::mem::take(&mut state.full_rebuild) {
        if let Some(recipe) = &state.original {
            *view = recipe.view.clone();
        }
    } else {
        view.full_rebuild = false;
        for coord in &changed {
            let start = TilePos::new(*coord, i32::MIN);
            let end = TilePos::new(*coord, i32::MAX);
            let old: Vec<_> = view
                .voxels
                .range(start..=end)
                .map(|(pos, _)| *pos)
                .collect();
            for pos in old {
                view.voxels.remove(&pos);
            }
            view.columns.remove(coord);
            if let Some(column) = map.column(*coord) {
                publish_column(&mut view, *coord, column, &substances);
            }
        }
    }
    view.revision = revision;
    view.dirty_columns.clone_from(&changed);
    state.render_dirty.extend(changed);
}

fn publish_column(
    view: &mut ArenaTerrainView,
    coord: HexCoord,
    column: &Column,
    substances: &SubstanceTable,
) {
    for (level, substance) in column.iter().enumerate() {
        let Ok(level) = i32::try_from(level) else {
            continue;
        };
        if substances.is_solid(substance) {
            view.voxels.insert(TilePos::new(coord, level), substance);
        }
    }
    let spans = crate::runs(column)
        .into_iter()
        .filter(|run| substances.is_solid(run.substance))
        .map(|run| ArenaSolidSpan {
            bottom: TilePos::new(coord, run.bottom),
            top_level: run.top - 1,
            substance: run.substance,
        })
        .collect::<Vec<_>>();
    if !spans.is_empty() {
        view.columns.insert(coord, spans);
    }
}
