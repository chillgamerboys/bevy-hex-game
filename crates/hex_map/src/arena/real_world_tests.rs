use std::sync::OnceLock;

use hex_core::arena::{ArenaEncounter, ArenaMap};
use hex_core::{TerrainBatchId, TerrainDamageKind, TerrainImpactDisposition};

use super::*;

fn recipe(map: ArenaMap) -> worlds::WorldRecipe {
    let content = load_content().expect("accepted arena content");
    worlds::build(
        ArenaSelection { map, ..default() },
        content.materials,
        &content.substances,
        &content.art,
    )
    .expect("accepted real world builds")
}

fn seven() -> &'static worlds::WorldRecipe {
    static RECIPE: OnceLock<worlds::WorldRecipe> = OnceLock::new();
    RECIPE.get_or_init(|| recipe(ArenaMap::SevenRegions))
}

#[test]
fn fort_uses_accepted_geometry_and_publishes_exact_solid_runs() {
    let first = recipe(ArenaMap::Fort);
    let second = recipe(ArenaMap::Fort);
    assert_eq!(first.map.len(), 469);
    assert_eq!(first.view.voxels, second.view.voxels);
    assert!(first.view.liquids.is_empty());
    assert!(first.view.static_spans.is_empty());
    assert!(first.view.edit_protected.is_empty());
    assert!(first.view.anchors.contains_key("fort_courtyard"));
    for span in first.view.columns.values().flatten() {
        for level in span.bottom.level..=span.top_level {
            assert_eq!(
                first
                    .view
                    .voxels
                    .get(&TilePos::new(span.bottom.coord, level)),
                Some(&span.substance)
            );
        }
    }
    for feet in first.view.spawns {
        let pos = first
            .geometry
            .voxel_at(feet - Vec3::Y * 0.001)
            .expect("spawn voxel");
        assert!(first.view.voxels.contains_key(&pos));
        assert!(!first.view.voxels.contains_key(&pos.above()));
        #[expect(
            clippy::cast_precision_loss,
            reason = "small validated map voxel levels"
        )]
        let expected_top = (pos.level + 1) as f32 * first.geometry.level_height;
        assert!((feet.y - expected_top).abs() < 0.0001);
    }
}

#[test]
fn seven_publishes_three_dry_encounter_anchors_and_distinct_static_geometry() {
    let recipe = seven();
    assert_eq!(recipe.map.len(), 3367);
    assert_eq!(recipe.geometry.radius, 33);
    assert!(!recipe.view.liquids.is_empty());
    assert!(!recipe.view.static_spans.is_empty());
    for name in [
        "mountains_high_pass",
        "fort_fort_courtyard",
        "caves_cave_entrance",
    ] {
        let feet = *recipe
            .view
            .anchors
            .get(name)
            .expect("accepted encounter anchor");
        let support = recipe
            .geometry
            .voxel_at(feet - Vec3::Y * 0.001)
            .expect("anchor support");
        assert!(
            recipe.view.voxels.contains_key(&support),
            "{name} has solid support"
        );
        assert!(
            !recipe.view.voxels.contains_key(&support.above()),
            "{name} is exposed"
        );
        assert!(
            !recipe.view.liquids.iter().any(|span| {
                span.bottom.coord == support.coord
                    && span.top_level > support.level
                    && span.bottom.level <= support.level + 2
            }),
            "{name} remains dry"
        );
    }
    for span in &recipe.view.liquids {
        assert!(!recipe.view.voxels.contains_key(&span.bottom));
        assert!(recipe
            .view
            .edit_protected
            .get(&span.bottom.coord)
            .is_some_and(|intervals| {
                intervals
                    .iter()
                    .any(|(bottom, top)| *bottom <= span.bottom.level && *top > span.top_level)
            }));
    }
    assert!(recipe
        .view
        .static_spans
        .iter()
        .any(|span| span.blocks_movement));
    assert!(recipe
        .view
        .static_spans
        .iter()
        .any(|span| span.blocks_sight && !span.blocks_movement));
    for span in &recipe.view.static_spans {
        assert!(recipe
            .view
            .edit_protected
            .get(&span.bottom.coord)
            .is_some_and(|intervals| {
                intervals
                    .iter()
                    .any(|(bottom, top)| *bottom <= span.bottom.level && *top >= span.top_level)
            }));
    }
}

#[test]
fn real_map_reset_restores_partial_hp_terrain_selection_and_clears_old_batches() {
    let mut app = App::new();
    app.insert_resource(ArenaSelection {
        map: ArenaMap::Fort,
        ..default()
    })
    .add_plugins(MinimalPlugins)
    .add_plugins(plugin);
    app.update();
    let original = app.world().resource::<ArenaTerrainView>().voxels.clone();
    let stone = app
        .world()
        .resource::<SubstanceTable>()
        .id("worked_stone")
        .expect("masonry");
    let pos = *original
        .iter()
        .find(|(_, id)| **id == stone)
        .expect("fort masonry")
        .0;
    let hit = TerrainImpact {
        batch: TerrainBatchId(3),
        volume: vec![pos],
        kind: TerrainDamageKind::Physical,
        power: 2,
    };
    app.world_mut().write_message(hit.clone());
    app.world_mut().run_schedule(ArenaTick);
    let first = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .next()
        .expect("outcome");
    let TerrainImpactResult::Applied(outcomes) = first.result else {
        panic!("physical batch admitted")
    };
    let damaged = outcomes.first().expect("damaged voxel");
    assert_eq!(damaged.disposition, TerrainImpactDisposition::Damaged);
    assert_eq!(damaged.health_after.expect("remaining HP").remaining, 6);
    assert!(app
        .world()
        .resource::<DamagedVoxels>()
        .iter()
        .next()
        .is_some());
    app.world_mut().write_message(TerrainEdit::Clear { pos });
    app.world_mut().run_schedule(ArenaTick);
    let view = app.world().resource::<ArenaTerrainView>();
    assert!(!view.full_rebuild);
    assert_eq!(view.dirty_columns, BTreeSet::from([pos.coord]));
    assert!(!view.voxels.contains_key(&pos));
    app.world_mut().write_message(hit.clone());
    app.world_mut().resource_mut::<ArenaSelection>().encounter = ArenaEncounter::Goblins;
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.world_mut().run_schedule(ArenaTick);
    let reset = app.world().resource::<ArenaTerrainView>();
    assert_eq!(reset.voxels, original);
    assert_eq!(reset.selection.encounter, ArenaEncounter::Goblins);
    assert!(reset.full_rebuild);
    assert_eq!(reset.columns.len(), 469);
    assert!(app
        .world()
        .resource::<DamagedVoxels>()
        .iter()
        .next()
        .is_none());
    assert!(app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .next()
        .is_none());
    app.world_mut().write_message(hit);
    app.world_mut().run_schedule(ArenaTick);
    let TerrainImpactResult::Applied(outcomes) = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .next()
        .expect("fresh ledger outcome")
        .result
    else {
        panic!("reset frees batch IDs")
    };
    assert_eq!(
        outcomes
            .first()
            .expect("voxel")
            .health_after
            .expect("HP")
            .remaining,
        6
    );
    app.world_mut().resource_mut::<ArenaSelection>().map = ArenaMap::Duel;
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().selection.map,
        ArenaMap::Duel
    );
    assert!(
        app.world()
            .resource::<ArenaVoxelGeometry>()
            .vertical_offset
            .abs()
            < f32::EPSILON
    );
    assert_eq!(
        app.world().resource::<ArenaTerrainView>().spawns,
        spawn_positions(ArenaVoxelGeometry::default())
    );
}

#[test]
fn physical_damage_respects_real_world_object_and_liquid_protection() {
    let content = load_content().expect("content");
    let recipe = seven();
    let mut map = recipe.map.clone();
    let mut ledger = TerrainDamageState::default();
    let mut health = DamagedVoxels::default();
    let pos = recipe
        .view
        .voxels
        .keys()
        .find(|pos| {
            content.substances.is_diggable(map.get(**pos))
                && recipe
                    .view
                    .edit_protected
                    .get(&pos.coord)
                    .is_some_and(|intervals| {
                        intervals
                            .iter()
                            .any(|(bottom, top)| (*bottom..=*top).contains(&pos.level))
                    })
        })
        .copied()
        .expect("protected authored support");
    let before = map.get(pos);
    let applied = ledger.apply(
        TerrainImpact {
            batch: TerrainBatchId(1),
            volume: vec![pos],
            kind: TerrainDamageKind::Physical,
            power: 8,
        },
        &mut map,
        &content.substances,
        &content.damage,
        &mut health,
        |candidate| {
            recipe
                .view
                .edit_protected
                .get(&candidate.coord)
                .is_some_and(|intervals| {
                    intervals
                        .iter()
                        .any(|(bottom, top)| (*bottom..=*top).contains(&candidate.level))
                })
        },
    );
    let TerrainImpactResult::Applied(outcomes) = applied.outcome.result else {
        panic!("structural admission")
    };
    assert_eq!(
        outcomes.first().expect("voxel").disposition,
        TerrainImpactDisposition::Resisted
    );
    assert_eq!(map.get(pos), before);
    assert!(health.iter().next().is_none());
}
