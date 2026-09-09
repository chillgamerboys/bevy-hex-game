//! Shield publication must preserve world-owned material and accumulated damage.

use super::*;
use hex_core::arena::ArenaMaterials;
use hex_core::{
    TerrainBatchId, TerrainEdit, TerrainImpact, TerrainImpactDisposition, TerrainImpactOutcome,
    TerrainImpactResult, TerrainVoxelOutcome, TilePos,
};

fn damage_slots(app: &mut App, batch: u64, slots: &[TilePos]) -> Vec<TerrainVoxelOutcome> {
    let element = app.world().resource::<ArenaMaterials>().fire;
    let mut volume = slots.to_vec();
    volume.sort_unstable();
    app.world_mut().write_message(TerrainImpact {
        batch: TerrainBatchId(batch),
        volume,
        kind: hex_core::TerrainDamageKind::Elemental(element),
        power: 1,
    });
    tick(app);
    let outcomes: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .collect();
    assert_eq!(outcomes.len(), 1);
    let outcome = outcomes.into_iter().next().expect("one world outcome");
    assert_eq!(outcome.batch, TerrainBatchId(batch));
    let TerrainImpactResult::Applied(voxels) = outcome.result else {
        panic!("fixture damage must be admitted by the real world");
    };
    assert_eq!(voxels.len(), slots.len());
    voxels
}

#[test]
fn shield_publication_preserves_existing_material_and_remaining_voxel_health() {
    let mut app = app(120);
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let materials = *app.world().resource::<ArenaMaterials>();
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim: Vec3::new(1.0, -0.2, 0.0).normalize(),
        selected: Some(Spell::Shield),
        ..default()
    };
    tick(&mut app);
    let original = preview(
        app.world().resource::<ArenaSession>(),
        app.world().resource::<ArenaTerrainView>(),
        &geometry,
        app.world().resource::<ArenaTuning>(),
    );
    assert!(original.valid && original.wall_voxels.len() > 2);
    let top_level = original
        .wall_voxels
        .iter()
        .map(|pos| pos.level)
        .max()
        .expect("shield rows");
    let top_row: Vec<_> = original
        .wall_voxels
        .iter()
        .copied()
        .filter(|pos| pos.level == top_level)
        .collect();
    let [stone, dirt, ..] = top_row.as_slice() else {
        panic!("fixture needs two distinct columns in the shield's top row");
    };
    let slots = [*stone, *dirt];
    let eye = app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("human")
        .eye();
    for pos in slots {
        // These occupied cells clip the shield without intercepting its downward
        // seed or moving either actor. They are created through world messages.
        assert!(geometry.top(pos) - geometry.level_height > eye.y + 0.1);
    }
    for (pos, substance) in [
        (slots.first().copied().expect("stone slot"), materials.stone),
        (slots.last().copied().expect("dirt slot"), materials.dirt),
    ] {
        app.world_mut()
            .write_message(TerrainEdit::Set { pos, substance });
    }
    tick(&mut app);
    let first = damage_slots(&mut app, 1_000_001, &slots);
    for (pos, substance) in [(stone, materials.stone), (dirt, materials.dirt)] {
        let voxel = first
            .iter()
            .find(|voxel| voxel.pos == *pos)
            .expect("first damage result");
        assert_eq!(voxel.disposition, TerrainImpactDisposition::Damaged);
        assert_eq!(voxel.before, Some(substance));
        assert_eq!(voxel.after, Some(substance));
        let before = voxel.health_before.expect("initial world health");
        let after = voxel.health_after.expect("partially damaged world health");
        assert!(after.remaining > 0 && after.remaining < before.remaining);
    }
    let clipped = preview(
        app.world().resource::<ArenaSession>(),
        app.world().resource::<ArenaTerrainView>(),
        &geometry,
        app.world().resource::<ArenaTuning>(),
    );
    assert!(clipped.valid);
    assert_eq!(
        clipped.wall_voxels.len() + slots.len(),
        original.wall_voxels.len()
    );
    assert!(slots.iter().all(|pos| !clipped.wall_voxels.contains(pos)));

    {
        let mut input = app.world_mut().resource_mut::<ArenaInput>();
        input.human.cast_pressed = true;
        input.human.cast_released = true;
    }
    for _ in 0..120 {
        tick(&mut app);
    }
    assert_eq!(app.world().resource::<ArenaSession>().shields_raised, 1);
    {
        let terrain = app.world().resource::<ArenaTerrainView>();
        assert_eq!(terrain.voxels.get(stone), Some(&materials.stone));
        assert_eq!(terrain.voxels.get(dirt), Some(&materials.dirt));
        assert!(clipped
            .wall_voxels
            .iter()
            .all(|pos| terrain.voxels.get(pos) == Some(&materials.stone)));
    }
    // Read remaining HP through a second authoritative impact, never through
    // private DamageState or by reconstructing health from the material catalog.
    let second = damage_slots(&mut app, 1_000_002, &slots);
    for prior in first {
        let next = second
            .iter()
            .find(|voxel| voxel.pos == prior.pos)
            .expect("second damage result");
        assert_eq!(next.before, prior.after);
        assert_eq!(
            next.health_before, prior.health_after,
            "shield creation must not heal an occupied voxel"
        );
    }
    assert_eq!(
        second
            .iter()
            .find(|voxel| voxel.pos == *stone)
            .expect("stone result")
            .disposition,
        TerrainImpactDisposition::Damaged
    );
    assert_eq!(
        second
            .iter()
            .find(|voxel| voxel.pos == *dirt)
            .expect("dirt result")
            .disposition,
        TerrainImpactDisposition::Destroyed
    );
}

#[test]
fn mature_shield_waits_for_real_blast_then_fills_the_destroyed_original_candidate() {
    let mut app = app(120);
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let materials = *app.world().resource::<ArenaMaterials>();
    let aim = Vec3::new(1.0, -0.2, 0.0).normalize();
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim,
        selected: Some(Spell::Shield),
        ..default()
    };
    tick(&mut app);
    let original = preview(
        app.world().resource::<ArenaSession>(),
        app.world().resource::<ArenaTerrainView>(),
        &geometry,
        app.world().resource::<ArenaTuning>(),
    );
    assert!(original.valid);
    let actor = app
        .world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("human");
    let center = actor.center();
    let eye = actor.eye();
    let radius = app.world().resource::<ArenaTuning>().blast_radius();
    let top = original
        .wall_voxels
        .iter()
        .map(|pos| pos.level)
        .max()
        .expect("wall height");
    let destroyed = *original
        .wall_voxels
        .iter()
        .find(|pos| pos.level == top && geometry.center(**pos).distance(center) < radius - 0.25)
        .expect("high candidate inside the real blast sphere");
    let retained = *original
        .wall_voxels
        .iter()
        .find(|pos| pos.level == top && geometry.center(**pos).distance(center) > radius + 0.25)
        .expect("high candidate outside the real blast sphere");
    for (pos, substance) in [(destroyed, materials.grass), (retained, materials.dirt)] {
        assert!(geometry.top(pos) - geometry.level_height > eye.y + 0.1);
        app.world_mut()
            .write_message(TerrainEdit::Set { pos, substance });
    }
    tick(&mut app);
    let clipped = preview(
        app.world().resource::<ArenaSession>(),
        app.world().resource::<ArenaTerrainView>(),
        &geometry,
        app.world().resource::<ArenaTuning>(),
    );
    assert!(clipped.valid);
    assert_eq!(clipped.wall_voxels.len() + 2, original.wall_voxels.len());
    assert!(!clipped.wall_voxels.contains(&destroyed) && !clipped.wall_voxels.contains(&retained));
    {
        let mut input = app.world_mut().resource_mut::<ArenaInput>();
        input.human.cast_pressed = true;
        input.human.cast_released = true;
    }
    let mut ready = false;
    for _ in 0..120 {
        tick(&mut app);
        let session = app.world().resource::<ArenaSession>();
        if session
            .emerging_shields()
            .next()
            .is_some_and(|(_, progress)| progress >= 0.97)
        {
            ready = true;
            break;
        }
    }
    assert!(ready, "stop one tick before the ordinary emergence commits");
    assert_eq!(app.world().resource::<ArenaSession>().shields_raised, 0);
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        aim,
        selected: Some(Spell::AreaBlast),
        cast_pressed: true,
        cast_released: true,
        ..default()
    };
    tick(&mut app);
    assert_eq!(app.world().resource::<ArenaSession>().shields_raised, 0);
    assert!(
        app.world().resource::<Messages<TerrainEdit>>().is_empty(),
        "mature shield must wait for the announced blast"
    );
    assert_eq!(
        app.world()
            .resource::<ArenaTerrainView>()
            .voxels
            .get(&destroyed),
        Some(&materials.grass)
    );

    // This tick applies the real blast, publishes the changed terrain, and only
    // then admits the mature shield. Its edits remain queued until the next tick.
    tick(&mut app);
    let outcomes: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .collect();
    assert_eq!(outcomes.len(), 1);
    let outcome = outcomes
        .into_iter()
        .next()
        .expect("real Area Blast outcome");
    let TerrainImpactResult::Applied(voxels) = outcome.result else {
        panic!("Area Blast must reach the candidate through the real world consumer");
    };
    let removed = voxels
        .iter()
        .find(|voxel| voxel.pos == destroyed)
        .expect("blast hits original occupied slot");
    assert_eq!(removed.disposition, TerrainImpactDisposition::Destroyed);
    assert_eq!(removed.before, Some(materials.grass));
    assert_eq!(removed.after, None);
    assert!(voxels.iter().all(|voxel| voxel.pos != retained));
    assert!(!app
        .world()
        .resource::<ArenaTerrainView>()
        .voxels
        .contains_key(&destroyed));
    assert_eq!(app.world().resource::<ArenaSession>().terrain_outcomes, 1);
    assert_eq!(app.world().resource::<ArenaSession>().shields_raised, 1);
    assert!(app
        .world()
        .resource::<ArenaSession>()
        .emerging_shields()
        .next()
        .is_none());

    tick(&mut app);
    let terrain = app.world().resource::<ArenaTerrainView>();
    assert_eq!(terrain.voxels.get(&destroyed), Some(&materials.stone));
    assert_eq!(terrain.voxels.get(&retained), Some(&materials.dirt));
    assert!(
        original
            .wall_voxels
            .iter()
            .filter(|pos| **pos != retained)
            .all(|pos| terrain.voxels.get(pos) == Some(&materials.stone)),
        "the published shield must keep the original footprint after destruction"
    );
}
