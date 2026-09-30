//! Actual package + world transaction + arena consumer evidence, without a renderer.

use super::*;
use hex_arena::ExpeditionRole;
use hex_core::arena::ArenaMaterials;
use hex_core::{
    TerrainBatchId, TerrainDamageKind, TerrainImpact, TerrainImpactOutcome, TerrainImpactResult,
    TilePos,
};

struct BlastProbe {
    impact: TerrainImpact,
    object: TilePos,
    terrain: TilePos,
    center: Vec3,
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "An explicit populated-world fixture requires an actual liquid and a safe mixed solid sample."
)]
fn mixed_blast(app: &App, sequence: u64, preceding: &[BlastProbe]) -> BlastProbe {
    let view = app.world().resource::<ArenaTerrainView>();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let session = app.world().resource::<ArenaSession>();
    let fire = app.world().resource::<ArenaMaterials>().fire;
    // Public occupancy supplies the sample. Search/fixture setup is outside the
    // measured tick; no private tree blueprint or generator assumptions are used.
    for (coord, spans) in &view.object_columns {
        for span in spans {
            let object = span.bottom;
            if view.voxels.contains_key(&object) {
                continue;
            }
            let center = geometry.center(object);
            if preceding
                .iter()
                .any(|probe| probe.center.distance(center) < 6.0)
            {
                continue;
            }
            if session
                .actors
                .iter()
                .filter(|actor| actor.hp > 0.0)
                .any(|actor| actor.center().distance(center) < 15.0)
            {
                continue;
            }
            let mut volume = geometry.sphere(view, center, 2.5);
            let Some(terrain) = volume
                .iter()
                .copied()
                .find(|pos| view.voxels.contains_key(pos) && pos.coord.distance(*coord) <= 1)
            else {
                continue;
            };
            // A real liquid cell deliberately shares the command: water must
            // survive while nearby solid geometry resolves through one batch.
            let water = view.liquids.first().expect("actual river volume").bottom;
            assert!(view.solid_at(water).is_none(), "water is not a solid");
            volume.push(water);
            volume.sort_unstable();
            volume.dedup();
            return BlastProbe {
                impact: TerrainImpact {
                    batch: TerrainBatchId(9_900_000 + sequence),
                    volume,
                    kind: TerrainDamageKind::Elemental(fire),
                    power: 8,
                },
                object,
                terrain,
                center,
            };
        }
    }
    panic!("actual expedition must provide a safe mixed tree/terrain destruction sample");
}

#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "Missing or rejected authoritative outcomes must fail this composition fixture."
)]
fn verify_blast(app: &mut App, probe: &BlastProbe) {
    let outcome = app
        .world_mut()
        .resource_mut::<Messages<TerrainImpactOutcome>>()
        .drain()
        .find(|outcome| outcome.batch == probe.impact.batch)
        .expect("world must return the exact announced mixed batch");
    assert!(outcome.is_consistent_with(&probe.impact));
    let TerrainImpactResult::Applied(voxels) = outcome.result else {
        panic!("actual expedition must admit finite solid destruction");
    };
    for target in [probe.object, probe.terrain] {
        let result = voxels
            .iter()
            .find(|result| result.pos == target)
            .expect("mixed target outcome");
        assert!(
            result.after.is_none(),
            "finite durability target {target:?}"
        );
    }
    let view = app.world().resource::<ArenaTerrainView>();
    assert!(view.solid_at(probe.object).is_none());
    assert!(view.solid_at(probe.terrain).is_none());
    assert!(!view.static_spans.iter().any(|span| {
        span.bottom.coord == probe.object.coord
            && (span.bottom.level..=span.top_level).contains(&probe.object.level)
    }));
    let desired = probe.center + Vec3::Y * 0.1;
    let session = app.world().resource::<ArenaSession>();
    assert!(
        session
            .camera_position(probe.center, desired)
            .distance(desired)
            < 0.001,
        "same-tick arena collision must consume the carved opening"
    );
}

/// Combine actual lowland AI with explicit mixed terrain/object damage commands.
/// Extra human HP and supported visits are declared fixture changes. This is a
/// CPU/state probe; it does not establish glider feel, water appearance or FPS.
#[test]
#[ignore = "requires HEX_FOREST_WORLD pointing at the 127-enemy expedition; explicit populated destructive CPU probe"]
fn authored_expedition_lowland_combat_and_carving_tick_profile_resets() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(ArenaSelection {
            map: ArenaMap::ForestMassif,
            ..default()
        })
        .add_plugins((hex_map::arena::plugin, hex_arena::plugin))
        .init_resource::<TickPhaseClock>()
        .add_systems(
            ArenaTick,
            (
                phase_begin.before(ArenaSystems::ApplyTerrain),
                phase_applied
                    .after(ArenaSystems::ApplyTerrain)
                    .before(ArenaSystems::PublishTerrain),
                phase_published
                    .after(ArenaSystems::PublishTerrain)
                    .before(ArenaSystems::Simulate),
                phase_simulated.after(ArenaSystems::Simulate),
            ),
        );
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .set_cpu_profiling(true);
    app.update();
    tick(&mut app);
    let (original_water, identity, original_progress, original_actors, representatives) = {
        let view = app.world().resource::<ArenaTerrainView>();
        let session = app.world().resource::<ArenaSession>();
        assert_eq!(session.actors.len(), 128, "{}", session.notice);
        assert_eq!(session.encounter_summary().living_enemies, 127);
        assert_lowland_sites(session, view);
        let representatives = session
            .parties()
            .iter()
            .filter_map(|party| {
                let actor = session.actors.iter().find(|actor| {
                    actor.party == Some(party.id)
                        && matches!(
                            actor.expedition_role(),
                            Some(ExpeditionRole::PlainGolem | ExpeditionRole::PlainWisp)
                        )
                })?;
                Some((actor.id, party.id, party.home))
            })
            .collect::<Vec<_>>();
        assert_eq!(representatives.len(), 6);
        (
            view.liquids.clone(),
            view.package_identity
                .clone()
                .expect("accepted package identity"),
            session.progress().expect("initial progression"),
            session
                .actors
                .iter()
                .map(|actor| (actor.id, actor.feet, actor.hp.to_bits()))
                .collect::<Vec<_>>(),
            representatives,
        )
    };
    // Capture original material truth before any combat can place a Shield or
    // carve terrain. The 24 disjoint areas avoid actor spawn/support reservations.
    let mut probes = Vec::new();
    let mut originals = BTreeMap::new();
    for sequence in 0..24 {
        let probe = mixed_blast(&app, sequence, &probes);
        let view = app.world().resource::<ArenaTerrainView>();
        for pos in &probe.impact.volume {
            if let Some(material) = view.solid_at(*pos) {
                originals.entry(*pos).or_insert(material);
            }
        }
        probes.push(probe);
    }
    let mut baseline = Vec::new();
    for _ in 0..120 {
        let revision = app.world().resource::<ArenaTerrainView>().revision;
        let began = Instant::now();
        tick(&mut app);
        baseline.push(phase_sample(
            &app,
            began.elapsed().as_secs_f64() * 1000.0,
            app.world().resource::<ArenaTerrainView>().revision != revision,
        ));
    }
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        session.bot_enabled = true;
        let player = session.actors.first_mut().expect("player");
        player.max_hp = 100_000.0;
        player.hp = player.max_hp;
    }
    let mut activated = BTreeSet::new();
    let mut samples = Vec::new();
    let mut batches = 0_u64;
    let mut peak_projectiles = 0;
    for (actor, party, home) in representatives {
        visit(&mut app, actor, home, None);
        for step in 0_u16..120 {
            let probe = if step > 0 && step.is_multiple_of(24) {
                let probe = probes
                    .get(usize::try_from(batches).expect("bounded batch count"))
                    .expect("pre-surveyed original blast volume");
                app.world_mut().write_message(probe.impact.clone());
                batches += 1;
                Some(probe)
            } else {
                None
            };
            let revision = app.world().resource::<ArenaTerrainView>().revision;
            let began = Instant::now();
            tick(&mut app);
            samples.push(phase_sample(
                &app,
                began.elapsed().as_secs_f64() * 1000.0,
                app.world().resource::<ArenaTerrainView>().revision != revision,
            ));
            if let Some(probe) = probe {
                verify_blast(&mut app, probe);
                assert_eq!(
                    app.world().resource::<ArenaTerrainView>().liquids,
                    original_water
                );
            }
            let session = app.world().resource::<ArenaSession>();
            assert!(!session.is_finished());
            peak_projectiles = peak_projectiles.max(session.projectiles.len());
            if session
                .parties()
                .iter()
                .any(|candidate| candidate.id == party && candidate.phase == PartyPhase::Active)
            {
                activated.insert(party);
            }
        }
    }
    assert_eq!(batches, 24);
    assert_eq!(
        activated.len(),
        6,
        "all authored lowland encounters must activate"
    );
    assert!(
        samples
            .iter()
            .filter(|sample| sample.terrain_changed)
            .count()
            >= 24
    );
    assert!(!originals.is_empty());
    app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent::default();
    app.world_mut().resource_mut::<ArenaReset>().generation += 1;
    tick(&mut app);
    let view = app.world().resource::<ArenaTerrainView>();
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(view.package_identity.as_ref(), Some(&identity));
    assert_eq!(view.liquids, original_water);
    for (pos, material) in &originals {
        assert_eq!(
            view.solid_at(*pos),
            Some(*material),
            "restart restores {pos:?}"
        );
    }
    assert_eq!(session.progress(), Some(original_progress));
    assert_eq!(
        session
            .actors
            .iter()
            .map(|actor| (actor.id, actor.feet, actor.hp.to_bits()))
            .collect::<Vec<_>>(),
        original_actors
    );
    println!(
        "EXPEDITION_DESTRUCTIVE_RECEIPT {}",
        serde_json::json!({
            "package_identity":identity,
            "actors_at_reset":128,"enemies_at_reset":127,"parties":25,"authored_routes":49,
            "baseline_ticks":120,"lowland_combat_ticks":720,"mixed_batches":batches,
            "activated_lowland_parties":activated.len(),"restored_sampled_solid_cells":originals.len(),
            "peak_projectiles":peak_projectiles,
            "baseline":phase_distributions(&baseline),"lowland_combat_and_carving":phase_distributions(&samples),
            "scope":"Actual populated package, normal lowland AI and ArenaTick ApplyTerrain/PublishTerrain/Simulate. Extra player HP, six supported player visits and 24 explicit power-8 mixed object/terrain commands are synthetic fixture inputs. Sampling/search/reporting outside timers. Water occupancy and complete sampled reset are typed state assertions. No renderer, HUD, recorder, GPU, native feel or FPS claim."
        })
    );
}
