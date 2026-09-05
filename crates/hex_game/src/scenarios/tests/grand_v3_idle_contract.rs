//! Grand V3's unchanged-frame cache-reuse contract.

use bevy::asset::AssetId;

use super::*;

type ChunkKey = (i32, i32);

fn grand_v3_idle_app() -> App {
    let mut app = unfinished_procedural_gameplay_app("Grand V3 Baseline", false);
    crate::fog::plugin(&mut app);
    finish_test_app(app)
}

fn terrain_roots(app: &mut App) -> Vec<(Entity, ChunkKey, Entity)> {
    let world = app.world_mut();
    let mut roots = world.query::<(Entity, &TerrainChunkRoot, &ChildOf)>();
    let mut snapshot = roots
        .iter(world)
        .map(|(entity, root, parent)| (entity, (root.q, root.r), parent.parent()))
        .collect::<Vec<_>>();
    snapshot.sort_by_key(|(entity, _chunk, _parent)| *entity);
    snapshot
}

fn terrain_batches(app: &mut App) -> Vec<(Entity, ChunkKey, SubstanceId, usize, AssetId<Mesh>)> {
    let world = app.world_mut();
    let mut batches = world.query::<(Entity, &TerrainRenderBatch, &Mesh3d)>();
    let mut snapshot = batches
        .iter(world)
        .map(|(entity, batch, mesh)| {
            let chunk = batch.chunk();
            (
                entity,
                (chunk.q, chunk.r),
                batch.substance(),
                batch.runs().len(),
                mesh.0.id(),
            )
        })
        .collect::<Vec<_>>();
    snapshot.sort_by_key(|(entity, _chunk, _substance, _runs, _mesh)| *entity);
    snapshot
}

fn fog_batches(app: &mut App) -> Vec<(Entity, String, AssetId<Mesh>, AssetId<StandardMaterial>)> {
    let world = app.world_mut();
    let mut batches = world.query::<(Entity, &Name, &Mesh3d, &MeshMaterial3d<StandardMaterial>)>();
    let mut snapshot = batches
        .iter(world)
        .filter(|(_entity, name, _mesh, _material)| name.as_str().starts_with("FogOverlayBatch["))
        .map(|(entity, name, mesh, material)| {
            (
                entity,
                name.as_str().to_owned(),
                mesh.0.id(),
                material.0.id(),
            )
        })
        .collect::<Vec<_>>();
    snapshot.sort_by_key(|(entity, _name, _mesh, _material)| *entity);
    snapshot
}

fn local_knowledge(app: &App) -> Vec<(TilePos, hex_core::KnownTraversal)> {
    app.world().resource::<LocalMapKnowledge>().iter().collect()
}

#[test]
#[ignore = "manual release-mode 10,000-frame Grand V3 lifecycle stress gate"]
fn grand_v3_ten_thousand_idle_frames_reuse_every_runtime_projection() {
    const IDLE_FRAMES: u64 = 10_000;

    let mut app = grand_v3_idle_app();
    enter_screen(&mut app, Screen::Gameplay);
    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "Grand V3 setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );

    let topology = observe_terrain_runtime_topology(&mut app, "Grand V3 Baseline");
    assert_eq!(topology.resident_chunks, 444);

    let perception_before = *app
        .world()
        .resource::<hex_perception::PerceptionRuntimeStats>();
    let surfaces_before = app.world().resource::<SurfaceSnapshots>().clone();
    let illumination_before = app.world().resource::<ResolvedIllumination>().clone();
    let observations_before = app.world().resource::<FactionObservations>().clone();
    let faction_knowledge_before = app.world().resource::<FactionMapKnowledge>().clone();
    let local_knowledge_before = local_knowledge(&app);
    let roots_before = terrain_roots(&mut app);
    let terrain_batches_before = terrain_batches(&mut app);
    let fog_batches_before = fog_batches(&mut app);
    let fog_positions_before = crate::fog::fog_overlay_positions(app.world_mut());
    let mesh_count_before = app.world().resource::<Assets<Mesh>>().len();
    let entity_count_before = app.world().entities().len();
    let snapshot_fingerprint_before = app
        .world()
        .resource::<CurrentWorldSnapshotV1>()
        .fingerprint();

    assert_eq!(roots_before.len(), 444);
    assert!(!terrain_batches_before.is_empty());
    assert!(!fog_batches_before.is_empty());
    assert!(!fog_positions_before.is_empty());

    for _ in 0..IDLE_FRAMES {
        app.update();
    }

    let perception_after = *app
        .world()
        .resource::<hex_perception::PerceptionRuntimeStats>();
    assert_eq!(
        perception_after.frames_checked,
        perception_before.frames_checked + IDLE_FRAMES
    );
    assert_eq!(
        perception_after.surface_rebuilds, perception_before.surface_rebuilds,
        "unchanged Grand frames rebuilt exact terrain surfaces"
    );
    assert_eq!(
        perception_after.illumination_resolutions, perception_before.illumination_resolutions,
        "unchanged Grand frames resolved illumination again"
    );
    assert_eq!(
        perception_after.observation_resolutions, perception_before.observation_resolutions,
        "unchanged Grand frames resolved faction observations again"
    );
    assert_eq!(
        perception_after.knowledge_publications, perception_before.knowledge_publications,
        "unchanged Grand frames republished map knowledge"
    );

    assert_eq!(app.world().resource::<SurfaceSnapshots>(), &surfaces_before);
    assert_eq!(
        app.world().resource::<ResolvedIllumination>(),
        &illumination_before
    );
    assert_eq!(
        app.world().resource::<FactionObservations>(),
        &observations_before
    );
    assert_eq!(
        app.world().resource::<FactionMapKnowledge>(),
        &faction_knowledge_before
    );
    assert_eq!(local_knowledge(&app), local_knowledge_before);

    assert_eq!(
        terrain_roots(&mut app),
        roots_before,
        "unchanged Grand frames replaced terrain chunk roots"
    );
    assert_eq!(
        terrain_batches(&mut app),
        terrain_batches_before,
        "unchanged Grand frames rebuilt terrain batch entities or meshes"
    );
    assert_eq!(
        fog_batches(&mut app),
        fog_batches_before,
        "unchanged Grand frames rebuilt fog batch entities, meshes, or materials"
    );
    assert_eq!(
        crate::fog::fog_overlay_positions(app.world_mut()),
        fog_positions_before,
        "unchanged Grand frames changed the exact fog-cap projection"
    );
    assert_eq!(
        app.world().resource::<Assets<Mesh>>().len(),
        mesh_count_before,
        "unchanged Grand frames leaked or replaced presentation meshes"
    );
    assert_eq!(
        app.world().entities().len(),
        entity_count_before,
        "unchanged Grand frames spawned or retired runtime entities"
    );
    assert_eq!(
        app.world()
            .resource::<CurrentWorldSnapshotV1>()
            .fingerprint(),
        snapshot_fingerprint_before,
        "unchanged Grand frames changed save-state terrain identity"
    );
}
