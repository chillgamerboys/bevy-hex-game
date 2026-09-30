//! Grand V3's composed one-column edit contract.

use std::collections::{BTreeMap, BTreeSet};

use bevy::asset::AssetId;
use bevy::picking::Pickable;

use super::*;

type ChunkKey = (i32, i32);

fn grand_v3_edit_app() -> App {
    let mut app = unfinished_procedural_gameplay_app("Grand V3 Baseline", false);
    crate::fog::plugin(&mut app);
    finish_test_app(app)
}

fn render_batches_by_chunk(app: &mut App) -> BTreeMap<ChunkKey, BTreeSet<Entity>> {
    let world = app.world_mut();
    let mut batches = world.query::<(Entity, &TerrainRenderBatch)>();
    let mut by_chunk = BTreeMap::<_, BTreeSet<_>>::new();
    for (entity, batch) in batches.iter(world) {
        let chunk = batch.chunk();
        by_chunk
            .entry((chunk.q, chunk.r))
            .or_default()
            .insert(entity);
    }
    by_chunk
}

fn active_grid(app: &mut App) -> Entity {
    let world = app.world_mut();
    let mut grids = world.query_filtered::<Entity, With<HexGrid>>();
    grids
        .single(world)
        .expect("Grand V3 should publish one stable grid")
}

fn liquid_presentation_snapshot(app: &mut App) -> BTreeSet<(Entity, Entity, AssetId<Mesh>)> {
    let world = app.world_mut();
    let mut presentations =
        world.query::<(Entity, &Name, &ChildOf, &Mesh3d, &Pickable, Has<HexTile>)>();
    presentations
        .iter(world)
        .filter(|(_entity, name, _parent, _mesh, _pickable, _tile)| {
            matches!(
                name.as_str(),
                "LiquidCap" | "LiquidSideCurtain" | "LiquidFallCurtain"
            )
        })
        .map(|(entity, _name, parent, mesh, pickable, tile)| {
            assert!(
                !tile,
                "liquid presentation must not become terrain authority"
            );
            assert_eq!(*pickable, Pickable::IGNORE);
            (entity, parent.parent(), mesh.0.id())
        })
        .collect()
}

fn local_knowledge_snapshot(app: &App) -> Vec<(TilePos, hex_core::KnownTraversal)> {
    app.world().resource::<LocalMapKnowledge>().iter().collect()
}

fn assert_affected_chunk_picking(app: &mut App, target: TilePos, affected: ChunkKey) {
    let logical = {
        let world = app.world_mut();
        let mut tiles =
            world.query_filtered::<(Entity, &TilePos, &HexSpan, &SubstanceId), With<HexTile>>();
        tiles
            .iter(world)
            .filter(|(_entity, position, _span, _substance)| {
                terrain_chunk_key(position.coord) == affected
            })
            .map(|(entity, position, span, substance)| (entity, (*position, *span, *substance)))
            .collect::<BTreeMap<_, _>>()
    };
    assert!(
        logical.values().any(|(position, _, _)| *position == target),
        "the edited logical run was not republished"
    );

    let world = app.world_mut();
    let mut batches = world.query::<(&TerrainRenderBatch, &Pickable)>();
    let mut represented = BTreeSet::new();
    let mut resolved_target = false;
    for (batch, pickable) in batches.iter(world).filter(|(batch, _pickable)| {
        let chunk = batch.chunk();
        (chunk.q, chunk.r) == affected
    }) {
        assert_eq!(*pickable, Pickable::default());
        for run in batch.runs() {
            let tuple = logical
                .get(&run.entity())
                .expect("the affected batch referenced an absent logical run");
            assert_eq!((run.position(), run.span(), batch.substance()), *tuple);
            assert!(
                represented.insert(run.entity()),
                "an affected logical run appeared in more than one render batch"
            );

            if run.position() == target {
                let hit = target.coord.to_world(run.span().top);
                assert_eq!(
                    batch.resolve_hit(hit, Some(Vec3::Y)),
                    Some(run.entity()),
                    "the rebuilt Grand batch did not resolve the edited cap to its exact logical run"
                );
                resolved_target = true;
            }
        }
    }
    assert_eq!(
        represented,
        logical.keys().copied().collect(),
        "the rebuilt affected chunk lost exact render-to-run picking coverage"
    );
    assert!(
        resolved_target,
        "no actual affected-batch hit exercised the edited logical run"
    );
}

#[test]
fn one_dry_grand_v3_edit_is_chunk_local_and_preserves_composed_runtime_contracts() {
    let mut app = grand_v3_edit_app();
    enter_screen(&mut app, Screen::Gameplay);
    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "Grand V3 setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );

    let topology_before = observe_terrain_runtime_topology(&mut app, "Grand V3 Baseline");
    assert_eq!(topology_before.resident_chunks, 444);
    let target = checkpoint_edit_target(&mut app, "Grand V3 Baseline");
    let baseline_column = app
        .world()
        .resource::<VoxelMap>()
        .column(target.position.coord)
        .expect("the selected Grand edit column should remain resident")
        .clone();
    let replacement = [
        "dirt",
        "stone",
        "gravel",
        "grass",
        "snow",
        "sand",
        "ice",
        "basalt",
        "worked_stone",
        "metal",
    ]
    .into_iter()
    .filter_map(|name| app.world().resource::<SubstanceTable>().id(name))
    .find(|substance| {
        let table = app.world().resource::<SubstanceTable>();
        if *substance == target.original
            || !table.is_solid(*substance)
            || !table.is_diggable(*substance)
        {
            return false;
        }
        let mut predicted = baseline_column.clone();
        predicted.set(target.position.level, *substance);
        hex_map::runs(&predicted).len() == target.baseline_column_runs
    })
    .expect("Grand V3 should expose a dry solid-for-solid edit that preserves run cardinality");

    let roots_before = checkpoint_chunk_roots(&mut app);
    let batches_before = render_batches_by_chunk(&mut app);
    let grid_before = active_grid(&mut app);
    let liquids_before = liquid_presentation_snapshot(&mut app);
    assert!(
        !liquids_before.is_empty(),
        "Grand V3 should publish liquids"
    );
    let occupancy_before = app.world().resource::<TerrainOccupancy>().clone();
    let illumination_before = app.world().resource::<ResolvedIllumination>().clone();
    let observations_before = app.world().resource::<FactionObservations>().clone();
    let local_knowledge_before = local_knowledge_snapshot(&app);
    let fog_before = crate::fog::fog_overlay_positions(app.world_mut());
    assert!(!fog_before.is_empty(), "Grand V3 should publish fog caps");
    let snapshot_before = app
        .world()
        .resource::<CurrentWorldSnapshotV1>()
        .snapshot()
        .clone();
    assert!(snapshot_before
        .liquids
        .as_slice()
        .iter()
        .all(|liquid| liquid.position.coord != target.position.coord));
    let report_before = {
        let report = app.world().resource::<GenerationReport>();
        (
            report.settings_fingerprint,
            report.semantic_plan_fingerprint,
            report.map_fingerprint,
        )
    };
    assert!(report_before.1.is_some());
    let perception_before = *app
        .world()
        .resource::<hex_perception::PerceptionRuntimeStats>();

    app.world_mut().write_message(TerrainEdit::Set {
        pos: target.position,
        substance: replacement,
    });
    app.update();

    assert!(
        !app.world().contains_resource::<GameplaySetupFailure>(),
        "the dry Grand V3 edit failed"
    );
    assert_eq!(
        app.world().resource::<VoxelMap>().get(target.position),
        replacement
    );
    let topology_after = observe_terrain_runtime_topology(&mut app, "Grand V3 Baseline");
    assert_eq!(
        topology_after.resident_chunks,
        topology_before.resident_chunks
    );
    assert_eq!(topology_after.grid_entities, topology_before.grid_entities);
    assert_eq!(topology_after.tile_entities, topology_before.tile_entities);
    assert_eq!(
        topology_after.terrain_render_batches, topology_before.terrain_render_batches,
        "a dry solid-for-solid edit should preserve Grand render-batch cardinality"
    );
    assert_eq!(
        topology_after.terrain_batched_runs,
        topology_before.terrain_batched_runs
    );

    let roots_after = checkpoint_chunk_roots(&mut app);
    let grid_after = active_grid(&mut app);
    assert_eq!(grid_after, grid_before, "the edit replaced the stable grid");
    assert_eq!(
        roots_after.keys().collect::<Vec<_>>(),
        roots_before.keys().collect::<Vec<_>>()
    );
    let changed_roots = roots_before
        .iter()
        .filter(|(chunk, entity)| roots_after.get(chunk) != Some(*entity))
        .map(|(chunk, _entity)| *chunk)
        .collect::<Vec<_>>();
    assert_eq!(changed_roots, vec![target.chunk]);
    let retired_root = roots_before
        .get(&target.chunk)
        .copied()
        .expect("the edited Grand column should have an original chunk root");
    assert!(
        app.world().get_entity(retired_root).is_err(),
        "the replaced Grand chunk root remained alive"
    );

    let batches_after = render_batches_by_chunk(&mut app);
    assert!(batches_before.iter().all(|(chunk, entities)| {
        *chunk == target.chunk || batches_after.get(chunk) == Some(entities)
    }));
    let retired_batches = batches_before
        .get(&target.chunk)
        .expect("the original affected root should own render batches");
    let rebuilt_batches = batches_after
        .get(&target.chunk)
        .expect("the rebuilt affected root should own render batches");
    assert!(
        retired_batches.is_disjoint(rebuilt_batches),
        "the replaced root retained stale render batches"
    );
    assert!(retired_batches
        .iter()
        .all(|entity| app.world().get_entity(*entity).is_err()));
    assert_eq!(
        rebuilt_batches.len(),
        retired_batches.len(),
        "the dry solid-for-solid edit changed the affected chunk's batch count"
    );
    assert_affected_chunk_picking(&mut app, target.position, target.chunk);

    assert_eq!(
        app.world().resource::<TerrainOccupancy>(),
        &occupancy_before,
        "solid-for-solid presentation batching changed exact terrain occupancy"
    );
    assert_eq!(
        app.world().resource::<ResolvedIllumination>(),
        &illumination_before,
        "the occupancy-equivalent edit changed exact gameplay illumination"
    );
    assert_eq!(
        app.world().resource::<FactionObservations>(),
        &observations_before,
        "the occupancy-equivalent edit changed exact faction LOS"
    );
    assert_eq!(
        local_knowledge_snapshot(&app),
        local_knowledge_before,
        "the edit changed the local traversal-facing knowledge publication"
    );
    assert_eq!(
        crate::fog::fog_overlay_positions(app.world_mut()),
        fog_before,
        "the occupancy-equivalent edit changed the exact fog publication"
    );
    assert_eq!(
        liquid_presentation_snapshot(&mut app),
        liquids_before,
        "the dry edit rebuilt or changed Grand liquid presentation"
    );

    let snapshot_after = app
        .world()
        .resource::<CurrentWorldSnapshotV1>()
        .snapshot()
        .clone();
    assert_eq!(
        export_world_snapshot_v1(app.world()).expect("the edited Grand world should export"),
        snapshot_after,
        "the current Grand snapshot cache drifted from save/export truth"
    );
    assert_ne!(
        snapshot_after.public_fingerprint, snapshot_before.public_fingerprint,
        "the accepted edit retained the pre-edit public snapshot identity"
    );
    assert_eq!(snapshot_after.version, snapshot_before.version);
    assert_eq!(snapshot_after.columns.len(), snapshot_before.columns.len());
    let changed_columns = snapshot_before
        .columns
        .as_slice()
        .iter()
        .zip(snapshot_after.columns.as_slice())
        .filter(|(before, after)| before != after)
        .map(|(before, after)| (before.coord, after.coord))
        .collect::<Vec<_>>();
    assert_eq!(
        changed_columns,
        vec![(target.position.coord, target.position.coord)],
        "the one-column edit changed more than its canonical saved column"
    );
    assert_eq!(snapshot_after.damage, snapshot_before.damage);
    assert_eq!(snapshot_after.liquids, snapshot_before.liquids);
    assert_eq!(snapshot_after.anchors, snapshot_before.anchors);
    assert_eq!(
        snapshot_after.interior_surfaces,
        snapshot_before.interior_surfaces
    );
    assert_eq!(
        snapshot_after.interior_roofs,
        snapshot_before.interior_roofs
    );
    assert_eq!(
        snapshot_after.special_regions,
        snapshot_before.special_regions
    );
    assert_eq!(snapshot_after.biome_regions, snapshot_before.biome_regions);
    assert_eq!(snapshot_after.blockers, snapshot_before.blockers);
    assert_eq!(snapshot_after.view_hint, snapshot_before.view_hint);
    assert_eq!(snapshot_after.lights, snapshot_before.lights);
    assert_eq!(snapshot_after.objects, snapshot_before.objects);

    let report_after = {
        let report = app.world().resource::<GenerationReport>();
        (
            report.settings_fingerprint,
            report.semantic_plan_fingerprint,
            report.map_fingerprint,
        )
    };
    assert_eq!(
        report_after, report_before,
        "a live save-state edit must not rewrite the selected Grand semantic world identity"
    );
    let perception_after = *app
        .world()
        .resource::<hex_perception::PerceptionRuntimeStats>();
    assert_eq!(
        perception_after.surface_rebuilds - perception_before.surface_rebuilds,
        1
    );
    assert_eq!(
        perception_after.illumination_resolutions - perception_before.illumination_resolutions,
        1
    );
    assert_eq!(
        perception_after.observation_resolutions - perception_before.observation_resolutions,
        1
    );
    assert_eq!(
        perception_after.knowledge_publications - perception_before.knowledge_publications,
        1
    );
}
