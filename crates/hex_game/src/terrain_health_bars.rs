//! Privacy-safe world-space presentation for partially damaged terrain voxels.

use std::collections::BTreeMap;

use bevy::camera::visibility::VisibilitySystems;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::transform::TransformSystems;
use hex_core::{
    AppSystems, DamagedVoxels, Headroom, HexSpan, HexTile, IlluminationLevel, KnowledgeState,
    PerceptionSystems, PresentationSystems, Screen, TerrainSystems, TerrainVoxelHealth, TilePos,
};
use hex_perception::{FactionMapKnowledge, ResolvedIllumination};
use hex_ui::{DespawnOnExit, GameplayChromeView};
use hex_units::Faction;
use hex_world::{CameraSystems, PanOrbitCamera};

const BAR_LIFT: f32 = 0.52;
const BAR_WIDTH: f32 = 0.84;
const BAR_HEIGHT: f32 = 0.14;
const BAR_DEPTH: f32 = 0.025;
const FILL_INSET: f32 = 0.025;
// The copied camera rotation makes local +Z point toward the viewer. Keep the fill
// just in front of the opaque backing so the normal depth test cannot hide it.
const FILL_FORWARD: f32 = 0.02;

#[derive(Resource, Default)]
struct TerrainHealthBarAssets {
    mesh: Option<Handle<Mesh>>,
    background: Option<Handle<StandardMaterial>>,
    fill: Option<Handle<StandardMaterial>>,
}

impl TerrainHealthBarAssets {
    fn handles(
        &self,
    ) -> Option<(
        Handle<Mesh>,
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
    )> {
        Some((
            self.mesh.clone()?,
            self.background.clone()?,
            self.fill.clone()?,
        ))
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
struct TerrainHealthBar {
    pos: TilePos,
    tile: Entity,
    fill: Entity,
}

#[derive(Component)]
struct TerrainHealthBarPart;

#[derive(Component)]
struct TerrainHealthBarFill;

#[derive(Debug, Clone, Copy)]
struct ExposedTile {
    entity: Entity,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<TerrainHealthBarAssets>()
        .add_systems(Startup, prepare_assets)
        .add_systems(
            Update,
            sync_health_bars
                .in_set(AppSystems::Update)
                .after(TerrainSystems::ApplyWorld)
                .after(PerceptionSystems::PublishKnowledge)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            PostUpdate,
            orient_health_bars
                .after(CameraSystems::FollowPresentation)
                .before(TransformSystems::Propagate)
                .run_if(in_state(Screen::Gameplay)),
        )
        .add_systems(
            PostUpdate,
            compose_health_bar_visibility
                .after(PresentationSystems::ApplyVisibility)
                .before(VisibilitySystems::VisibilityPropagate)
                .run_if(in_state(Screen::Gameplay)),
        );
}

fn prepare_assets(
    mut render_assets: ResMut<TerrainHealthBarAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if render_assets.handles().is_some() {
        return;
    }
    render_assets.mesh = Some(meshes.add(Cuboid::new(1.0, 1.0, 1.0)));
    render_assets.background = Some(materials.add(bar_material(Color::srgb(0.055, 0.06, 0.075))));
    render_assets.fill = Some(materials.add(bar_material(Color::srgb(0.88, 0.16, 0.10))));
}

fn bar_material(color: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        unlit: true,
        perceptual_roughness: 1.0,
        ..default()
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "the adapter deliberately reads each independent authority instead of caching a second presentation truth"
)]
fn sync_health_bars(
    mut commands: Commands,
    damaged: Option<Res<DamagedVoxels>>,
    knowledge: Option<Res<FactionMapKnowledge>>,
    illumination: Option<Res<ResolvedIllumination>>,
    render_assets: Res<TerrainHealthBarAssets>,
    tiles: Query<(Entity, &TilePos, &Headroom), With<HexTile>>,
    bars: Query<(Entity, &TerrainHealthBar)>,
    mut fills: Query<&mut Transform, With<TerrainHealthBarFill>>,
) {
    let mut existing = BTreeMap::new();
    for (entity, bar) in &bars {
        if let Some((duplicate, _)) = existing.insert(bar.pos, (entity, *bar)) {
            commands.entity(duplicate).despawn();
        }
    }

    let Some(damaged) = damaged else {
        despawn_remaining(&mut commands, existing);
        return;
    };
    if damaged.is_empty() {
        despawn_remaining(&mut commands, existing);
        return;
    }
    let (Some(knowledge), Some(illumination), Some(handles)) =
        (knowledge, illumination, render_assets.handles())
    else {
        despawn_remaining(&mut commands, existing);
        return;
    };

    let exposed_tiles = collect_exposed_tiles(
        &damaged,
        tiles
            .iter()
            .map(|(entity, pos, headroom)| (entity, *pos, *headroom)),
    );

    for (pos, health) in damaged.iter() {
        let observed = knowledge.faction(Faction::Player).state(pos) == KnowledgeState::Observed;
        let lit = illumination
            .get(pos)
            .is_some_and(|resolved| resolved.level != IlluminationLevel::Dark);
        let tile = exposed_tiles.get(&pos).copied().flatten();
        let Some(tile) = tile.filter(|_| observed && lit && health.is_damaged()) else {
            continue;
        };

        match existing.remove(&pos) {
            Some((entity, mut bar)) if fills.get_mut(bar.fill).is_ok() => {
                if let Ok(mut transform) = fills.get_mut(bar.fill) {
                    *transform = fill_transform(health);
                }
                if bar.tile != tile.entity {
                    bar.tile = tile.entity;
                    commands.entity(entity).insert(bar);
                }
            }
            Some((entity, _)) => {
                commands.entity(entity).despawn();
                spawn_health_bar(&mut commands, &handles, tile.entity, pos, health);
            }
            None => spawn_health_bar(&mut commands, &handles, tile.entity, pos, health),
        }
    }

    despawn_remaining(&mut commands, existing);
}

fn collect_exposed_tiles(
    damaged: &DamagedVoxels,
    tiles: impl IntoIterator<Item = (Entity, TilePos, Headroom)>,
) -> BTreeMap<TilePos, Option<ExposedTile>> {
    let mut exposed_tiles = BTreeMap::new();
    for (entity, pos, headroom) in tiles {
        if damaged.get(pos).is_none() || headroom.0 == 0 {
            continue;
        }
        match exposed_tiles.entry(pos) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(Some(ExposedTile { entity }));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                entry.insert(None);
            }
        }
    }
    exposed_tiles
}

fn despawn_remaining(
    commands: &mut Commands,
    remaining: BTreeMap<TilePos, (Entity, TerrainHealthBar)>,
) {
    for (entity, _) in remaining.into_values() {
        commands.entity(entity).despawn();
    }
}

fn spawn_health_bar(
    commands: &mut Commands,
    handles: &(
        Handle<Mesh>,
        Handle<StandardMaterial>,
        Handle<StandardMaterial>,
    ),
    tile: Entity,
    pos: TilePos,
    health: TerrainVoxelHealth,
) {
    let root = commands
        .spawn((
            Name::new("TerrainHealthBar"),
            Transform::default(),
            Visibility::Hidden,
            Pickable::IGNORE,
            DespawnOnExit(Screen::Gameplay),
        ))
        .id();
    commands.spawn((
        Name::new("TerrainHealthBarBackground"),
        Mesh3d(handles.0.clone()),
        MeshMaterial3d(handles.1.clone()),
        Transform::from_scale(Vec3::new(BAR_WIDTH, BAR_HEIGHT, BAR_DEPTH)),
        Pickable::IGNORE,
        NotShadowCaster,
        NotShadowReceiver,
        TerrainHealthBarPart,
        ChildOf(root),
    ));
    let fill = commands
        .spawn((
            Name::new("TerrainHealthBarFill"),
            Mesh3d(handles.0.clone()),
            MeshMaterial3d(handles.2.clone()),
            fill_transform(health),
            Pickable::IGNORE,
            NotShadowCaster,
            NotShadowReceiver,
            TerrainHealthBarPart,
            TerrainHealthBarFill,
            ChildOf(root),
        ))
        .id();
    commands
        .entity(root)
        .insert(TerrainHealthBar { pos, tile, fill });
}

fn fill_transform(health: TerrainVoxelHealth) -> Transform {
    let inner_width = BAR_WIDTH - 2.0 * FILL_INSET;
    let ratio = f32::from(health.remaining) / f32::from(health.maximum);
    let width = inner_width * ratio;
    Transform {
        translation: Vec3::new(-(inner_width - width) * 0.5, 0.0, FILL_FORWARD),
        scale: Vec3::new(width, BAR_HEIGHT - 2.0 * FILL_INSET, BAR_DEPTH),
        ..default()
    }
}

fn orient_health_bars(
    cameras: Query<(&Camera, &Transform), (With<PanOrbitCamera>, Without<TerrainHealthBar>)>,
    tiles: Query<&HexSpan, With<HexTile>>,
    mut bars: Query<
        (&TerrainHealthBar, &mut Transform),
        (With<TerrainHealthBar>, Without<PanOrbitCamera>),
    >,
) {
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    if !camera.is_active {
        return;
    }
    for (bar, mut transform) in &mut bars {
        let Ok(span) = tiles.get(bar.tile) else {
            continue;
        };
        transform.translation = bar.pos.coord.to_world(span.top + BAR_LIFT);
        transform.rotation = camera_transform.rotation;
        transform.scale = Vec3::ONE;
    }
}

fn compose_health_bar_visibility(
    chrome: Res<GameplayChromeView>,
    cameras: Query<&Camera, With<PanOrbitCamera>>,
    tiles: Query<&Visibility, (With<HexTile>, Without<TerrainHealthBar>)>,
    mut bars: Query<
        (&TerrainHealthBar, &mut Visibility),
        (With<TerrainHealthBar>, Without<HexTile>),
    >,
) {
    let camera_ready = cameras.single().is_ok_and(|camera| camera.is_active);
    let chrome_visible = chrome.shown && !chrome.encounter_complete;
    for (bar, mut visibility) in &mut bars {
        let tile_visible = tiles
            .get(bar.tile)
            .is_ok_and(|visibility| *visibility != Visibility::Hidden);
        let next = if chrome_visible && camera_ready && tile_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *visibility != next {
            *visibility = next;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::{ExteriorIllumination, HexCoord, LightDomain, SubstanceId};
    use hex_perception::{
        apply_observations, FactionObservation, FactionObservations, SurfaceSnapshot,
        SurfaceSnapshots,
    };

    fn pos(q: i32, level: i32) -> TilePos {
        TilePos::new(HexCoord::from_axial(q, 0), level)
    }

    fn health(remaining: u8, maximum: u8) -> TerrainVoxelHealth {
        TerrainVoxelHealth::new(remaining, maximum).expect("fixture health is valid")
    }

    fn snapshots(positions: &[TilePos]) -> SurfaceSnapshots {
        SurfaceSnapshots::try_from_iter(positions.iter().copied().map(|position| SurfaceSnapshot {
            pos: position,
            span: HexSpan::new(0.0, 1.0),
            substance: SubstanceId(3),
            headroom: Headroom(4),
            is_solid: true,
            blocked: false,
            domain: LightDomain::Exterior,
        }))
        .expect("fixture surfaces are unique")
    }

    fn knowledge(positions: &[TilePos], observed: bool) -> FactionMapKnowledge {
        let surfaces = snapshots(positions);
        let mut current = FactionObservation::new();
        if observed {
            for position in positions {
                current.insert_surface(*position);
            }
        }
        let observations = FactionObservations::with_faction(Faction::Player, current);
        let mut knowledge = FactionMapKnowledge::new();
        apply_observations(&mut knowledge, &surfaces, &observations);
        knowledge
    }

    fn illumination(positions: &[TilePos], level: IlluminationLevel) -> ResolvedIllumination {
        ResolvedIllumination::try_resolve(
            positions
                .iter()
                .copied()
                .map(|position| (position, LightDomain::Exterior)),
            ExteriorIllumination::new(level),
            &[],
        )
        .expect("fixture illumination positions are unique")
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<TerrainHealthBarAssets>()
            .init_resource::<GameplayChromeView>()
            .add_systems(Startup, prepare_assets)
            .add_systems(
                Update,
                (
                    sync_health_bars,
                    orient_health_bars,
                    compose_health_bar_visibility,
                )
                    .chain(),
            );
        app
    }

    fn install_authority(
        app: &mut App,
        positions: &[TilePos],
        observed: bool,
        light: IlluminationLevel,
    ) {
        app.insert_resource(knowledge(positions, observed));
        app.insert_resource(illumination(positions, light));
    }

    fn damage(app: &mut App, entries: &[(TilePos, TerrainVoxelHealth)]) {
        let mut damaged = DamagedVoxels::new();
        for (position, health) in entries {
            damaged.publish(*position, *health);
        }
        app.insert_resource(damaged);
    }

    fn spawn_tile(app: &mut App, position: TilePos, headroom: Headroom) -> Entity {
        app.world_mut()
            .spawn((
                HexTile,
                position,
                HexSpan::new(0.0, 1.0),
                headroom,
                Visibility::Inherited,
                InheritedVisibility::VISIBLE,
                ViewVisibility::VISIBLE,
            ))
            .id()
    }

    fn spawn_camera(app: &mut App, rotation: Quat) {
        app.world_mut().spawn((
            Camera::default(),
            Transform::from_rotation(rotation),
            PanOrbitCamera::default(),
        ));
    }

    fn settle(app: &mut App) {
        app.update();
        app.update();
    }

    #[test]
    fn exact_stacked_surface_gets_one_bar_without_collapsing_the_column() {
        let mut app = test_app();
        let lower = pos(0, 2);
        let upper = pos(0, 8);
        install_authority(&mut app, &[lower], true, IlluminationLevel::Bright);
        damage(&mut app, &[(lower, health(2, 4))]);
        let lower_tile = spawn_tile(&mut app, lower, Headroom(4));
        spawn_tile(&mut app, upper, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);

        settle(&mut app);

        let mut bars = app.world_mut().query::<&TerrainHealthBar>();
        let bars = bars.iter(app.world()).copied().collect::<Vec<_>>();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars.first().map(|bar| bar.pos), Some(lower));
        assert_eq!(bars.first().map(|bar| bar.tile), Some(lower_tile));
    }

    #[test]
    fn exposed_candidates_store_only_exact_damaged_positions() {
        let target = pos(0, 2);
        let unrelated_low = pos(4, 2);
        let unrelated_high = pos(9, 8);
        let mut damaged = DamagedVoxels::new();
        damaged.publish(target, health(2, 4));

        let mut world = World::new();
        let target_entity = world.spawn_empty().id();
        let unrelated_low_entity = world.spawn_empty().id();
        let unrelated_high_entity = world.spawn_empty().id();
        let candidates = collect_exposed_tiles(
            &damaged,
            [
                (unrelated_low_entity, unrelated_low, Headroom(4)),
                (target_entity, target, Headroom(4)),
                (unrelated_high_entity, unrelated_high, Headroom(4)),
            ],
        );

        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates
                .get(&target)
                .copied()
                .flatten()
                .map(|tile| tile.entity),
            Some(target_entity)
        );
        assert!(!candidates.contains_key(&unrelated_low));
        assert!(!candidates.contains_key(&unrelated_high));
    }

    #[test]
    fn empty_projection_creates_no_bar_and_cleans_existing_parts() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            1
        );

        app.insert_resource(DamagedVoxels::new());
        app.update();

        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBarPart>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn darkness_unknown_knowledge_and_burial_fail_closed() {
        let mut app = test_app();
        let position = pos(0, 2);
        damage(&mut app, &[(position, health(2, 4))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);

        install_authority(&mut app, &[position], true, IlluminationLevel::Dark);
        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );

        install_authority(&mut app, &[position], false, IlluminationLevel::Bright);
        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );

        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        let tile = app
            .world_mut()
            .query_filtered::<Entity, With<HexTile>>()
            .single(app.world())
            .expect("one tile");
        app.world_mut().entity_mut(tile).insert(Headroom(0));
        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn observed_to_remembered_transition_removes_the_bar() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            1
        );

        let surfaces = snapshots(&[position]);
        let observations =
            FactionObservations::with_faction(Faction::Player, FactionObservation::new());
        {
            let mut map_knowledge = app.world_mut().resource_mut::<FactionMapKnowledge>();
            apply_observations(&mut map_knowledge, &surfaces, &observations);
            assert_eq!(
                map_knowledge.faction(Faction::Player).state(position),
                KnowledgeState::Remembered
            );
        }

        settle(&mut app);
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn composed_tile_visibility_ignores_stale_aggregates_and_cleanup_removes_every_part() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        let tile = spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<TerrainHealthBar>>()
            .single(app.world())
            .expect("one health bar");
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited)
        );

        app.world_mut()
            .entity_mut(tile)
            .insert((InheritedVisibility::HIDDEN, ViewVisibility::HIDDEN));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited),
            "tile visibility aggregates are prior-frame state; the bar receives its own current-frame propagation and culling"
        );

        app.world_mut().entity_mut(tile).insert((
            Visibility::Hidden,
            InheritedVisibility::VISIBLE,
            ViewVisibility::VISIBLE,
        ));
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Hidden)
        );

        app.world_mut().remove_resource::<DamagedVoxels>();
        app.update();
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBar>()
                .iter(app.world())
                .count(),
            0
        );
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBarPart>()
                .iter(app.world())
                .count(),
            0
        );
    }

    #[test]
    fn gameplay_chrome_hides_and_restores_world_space_health_bars() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<TerrainHealthBar>>()
            .single(app.world())
            .expect("one health bar");
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited)
        );

        app.world_mut().resource_mut::<GameplayChromeView>().shown = false;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Hidden)
        );

        app.world_mut().resource_mut::<GameplayChromeView>().shown = true;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn encounter_outcome_hides_world_space_health_bars_until_it_clears() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);

        let root = app
            .world_mut()
            .query_filtered::<Entity, With<TerrainHealthBar>>()
            .single(app.world())
            .expect("one health bar");
        app.world_mut()
            .resource_mut::<GameplayChromeView>()
            .encounter_complete = true;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Hidden)
        );

        app.world_mut()
            .resource_mut::<GameplayChromeView>()
            .encounter_complete = false;
        app.update();
        assert_eq!(
            app.world().get::<Visibility>(root),
            Some(&Visibility::Inherited)
        );
    }

    #[test]
    fn grid_respawn_relinks_one_bar_without_leaking_entities() {
        let mut app = test_app();
        let position = pos(0, 2);
        install_authority(&mut app, &[position], true, IlluminationLevel::Bright);
        damage(&mut app, &[(position, health(2, 4))]);
        let original = spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, Quat::IDENTITY);
        settle(&mut app);

        app.world_mut().entity_mut(original).despawn();
        let replacement = spawn_tile(&mut app, position, Headroom(4));
        settle(&mut app);

        let mut bars = app.world_mut().query::<&TerrainHealthBar>();
        let bars = bars.iter(app.world()).copied().collect::<Vec<_>>();
        assert_eq!(bars.len(), 1);
        assert_eq!(bars.first().map(|bar| bar.tile), Some(replacement));
        assert_eq!(
            app.world_mut()
                .query::<&TerrainHealthBarPart>()
                .iter(app.world())
                .count(),
            2
        );
    }

    #[test]
    fn render_parts_are_pick_ignored_shadowless_and_camera_facing() {
        let mut app = test_app();
        let position = pos(1, 3);
        let rotation = Quat::from_rotation_y(0.73) * Quat::from_rotation_x(-0.28);
        install_authority(&mut app, &[position], true, IlluminationLevel::Dim);
        damage(&mut app, &[(position, health(3, 8))]);
        spawn_tile(&mut app, position, Headroom(4));
        spawn_camera(&mut app, rotation);
        settle(&mut app);

        let mut roots = app
            .world_mut()
            .query_filtered::<(&TerrainHealthBar, &Transform), With<TerrainHealthBar>>();
        let (_, transform) = roots.single(app.world()).expect("one health bar");
        assert!(transform.rotation.angle_between(rotation) < 0.000_01);
        assert!((transform.translation.y - (1.0 + BAR_LIFT)).abs() < 0.000_01);

        let mut parts = app.world_mut().query_filtered::<
            (
                &Pickable,
                Has<NotShadowCaster>,
                Has<NotShadowReceiver>,
            ),
            With<TerrainHealthBarPart>,
        >();
        let parts = parts.iter(app.world()).collect::<Vec<_>>();
        assert_eq!(parts.len(), 2);
        assert!(parts.iter().all(|(pickable, caster, receiver)| {
            **pickable == Pickable::IGNORE && *caster && *receiver
        }));

        let mut fills = app
            .world_mut()
            .query_filtered::<&Transform, With<TerrainHealthBarFill>>();
        let fill = fills.single(app.world()).expect("one health-bar fill");
        assert!(
            fill.translation.z > 0.0,
            "local +Z faces the camera, so the fill must sit ahead of the backing"
        );
    }
}
