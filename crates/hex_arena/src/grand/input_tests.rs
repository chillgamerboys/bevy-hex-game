//! Exercise the actual ArenaTick input boundary, including deferred Grand steps.
use super::*;
use crate::{plugin, ArenaInput, ArenaOutcome};
use bevy_app::App;
use bevy_math::Vec2;
use hex_core::arena::{
    ArenaExpeditionSites, ArenaMap, ArenaMaterials, ArenaPackageIdentity, ArenaReset,
    ArenaResidency, ArenaSelection, ArenaSolidSpan, ArenaTick,
};
use hex_core::ocean::{
    OceanEnvironmentSampler, OceanEnvironmentView, OceanSurfaceSample, OceanWaterColumn,
    OceanWindProfile,
};
use hex_core::{ElementId, HexCoord, SubstanceId, TilePos};
use std::sync::Arc;

#[derive(Debug)]
struct StillSea;
impl OceanEnvironmentSampler for StillSea {
    fn surface_at(
        &self,
        _xz: Vec2,
        _phase: f32,
        column: OceanWaterColumn,
    ) -> Option<OceanSurfaceSample> {
        Some(OceanSurfaceSample {
            height: column.mean_height,
            normal: Vec3::Y,
            vertical_velocity: 0.0,
            mean_height: column.mean_height,
            bed_height: column.bed_height,
            water_id: column.water_id,
        })
    }
}

fn fixture(wet: bool) -> App {
    let geometry = ArenaVoxelGeometry {
        level_height: 1.0,
        radius: 32,
        min_level: -20,
        ..Default::default()
    };
    let land = HexCoord::from_axial(-10, 0);
    let dry = land.within_radius(3);
    let chunks = (-3..=3)
        .flat_map(|q| (-3..=3).map(move |r| (q, r)))
        .collect();
    let mut terrain = ArenaTerrainView {
        revision: 1,
        selection: ArenaSelection {
            map: ArenaMap::GrandV4,
            ..Default::default()
        },
        spawns: [land.to_world(SKIN), Vec3::ZERO],
        residency: Some(ArenaResidency {
            ready: chunks,
            catalogue: (-3..=3)
                .flat_map(|q| (-3..=3).map(move |r| (q, r)))
                .collect(),
        }),
        expedition: Some(ArenaExpeditionSites::default()),
        package_identity: Some(ArenaPackageIdentity {
            world_id: "grand-input-test".into(),
            manifest_fingerprint: 7,
            sites_fingerprint: None,
        }),
        ..Default::default()
    };
    for coord in HexCoord::ORIGIN.within_radius(32) {
        let water = wet && !dry.contains(&coord);
        terrain.columns.insert(
            coord,
            vec![ArenaSolidSpan {
                bottom: TilePos::new(coord, -20),
                top_level: if water { -10 } else { 0 },
                substance: SubstanceId(1),
            }],
        );
        if water {
            terrain.liquids.push(ArenaSolidSpan {
                bottom: TilePos::new(coord, -9),
                top_level: 0,
                substance: SubstanceId(2),
            });
        }
    }
    terrain.liquids.sort_by_key(|span| span.bottom);
    let mut app = App::new();
    app.insert_resource(terrain)
        .insert_resource(geometry)
        .insert_resource(ArenaMaterials {
            stone: SubstanceId(1),
            reinforced_stone: None,
            bedrock: SubstanceId(3),
            grass: SubstanceId(4),
            dirt: SubstanceId(5),
            fire: ElementId(1),
        })
        .insert_resource(ArenaReset::default())
        .insert_resource(OceanEnvironmentView {
            package_fingerprint: 7,
            sampler: Arc::new(StillSea),
            wind: OceanWindProfile::default(),
        })
        .add_plugins(plugin);
    app.world_mut().run_schedule(ArenaTick);
    let mut session = app.world_mut().resource_mut::<ArenaSession>();
    assert!(session.encounter.initialized && session.is_grand_run());
    session.bot_enabled = false;
    app
}

fn player(app: &App) -> &Actor {
    app.world()
        .resource::<ArenaSession>()
        .actors
        .first()
        .expect("Grand player")
}

fn set_ready(app: &mut App, ready: bool) {
    let mut world = app.world_mut().resource_mut::<ArenaTerrainView>();
    let residency = world.residency.as_mut().expect("finite terrain");
    residency.ready = if ready {
        residency.catalogue.clone()
    } else {
        Default::default()
    };
    world.revision += 1;
    world.full_rebuild = true;
}

fn stage(app: &mut App, feet: Vec3) {
    let mut session = app.world_mut().resource_mut::<ArenaSession>();
    let actor = session.actors.first_mut().expect("Grand player");
    actor.feet = feet;
    actor.previous_feet = feet;
    actor.body = Default::default();
    actor.grounded = false;
}

fn wait_with(app: &mut App, intent: ActorIntent) -> u64 {
    set_ready(app, false);
    app.world_mut().resource_mut::<ArenaInput>().human = intent;
    let tick = app.world().resource::<ArenaSession>().tick;
    for _ in 0..3 {
        // No new edge is supplied on the second and third attempted ticks.
        app.world_mut().run_schedule(ArenaTick);
        let session = app.world().resource::<ArenaSession>();
        assert_eq!(session.tick, tick);
        assert_eq!(session.notice, "Loading nearby encounter terrain…");
    }
    tick
}

#[test]
fn boat_tap_survives_grand_wait_and_deploys_exactly_once() {
    let mut app = fixture(true);
    stage(&mut app, Vec3::Y * 0.1);
    let tick = wait_with(
        &mut app,
        ActorIntent {
            boat_toggle: true,
            ..Default::default()
        },
    );
    assert!(app.world().resource::<ArenaInput>().human.boat_toggle);
    assert!(!player(&app).boat().unwrap().active);
    set_ready(&mut app, true);
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(app.world().resource::<ArenaSession>().tick, tick + 1);
    assert!(player(&app).boat().unwrap().active);
    assert!(!app.world().resource::<ArenaInput>().human.boat_toggle);
    app.world_mut().run_schedule(ArenaTick);
    assert!(
        player(&app).boat().unwrap().active,
        "one deferred tap must not fold the boat again"
    );
}

#[test]
fn glider_tap_survives_grand_wait_and_opens_exactly_once() {
    let mut app = fixture(false);
    let feet = player(&app).feet + Vec3::Y * 8.0;
    stage(&mut app, feet);
    let tick = wait_with(
        &mut app,
        ActorIntent {
            glider_toggle: true,
            ..Default::default()
        },
    );
    assert!(app.world().resource::<ArenaInput>().human.glider_toggle);
    assert!(!player(&app).glider().unwrap().open);
    set_ready(&mut app, true);
    app.world_mut().run_schedule(ArenaTick);
    assert_eq!(app.world().resource::<ArenaSession>().tick, tick + 1);
    assert!(player(&app).glider().unwrap().open);
    assert!(!app.world().resource::<ArenaInput>().human.glider_toggle);
    app.world_mut().run_schedule(ArenaTick);
    assert!(
        player(&app).glider().unwrap().open,
        "one deferred tap must not fold the glider again"
    );
    assert!(player(&app).feet.y < feet.y);
}

#[test]
fn jump_taps_survive_grand_wait_without_repeated_launches() {
    for high_jump in [false, true] {
        let mut app = fixture(false);
        let start = player(&app).feet;
        let tick = wait_with(
            &mut app,
            ActorIntent {
                jump: !high_jump,
                high_jump,
                ..Default::default()
            },
        );
        let pending = app.world().resource::<ArenaInput>().human;
        assert_eq!(pending.jump, !high_jump);
        assert_eq!(pending.high_jump, high_jump);
        set_ready(&mut app, true);
        app.world_mut().run_schedule(ArenaTick);
        assert_eq!(app.world().resource::<ArenaSession>().tick, tick + 1);
        assert!(player(&app).feet.y > start.y);
        assert!(!app.world().resource::<ArenaInput>().human.jump);
        assert!(!app.world().resource::<ArenaInput>().human.high_jump);
        for _ in 0..300 {
            app.world_mut().run_schedule(ArenaTick);
        }
        assert!((player(&app).feet.y - start.y).abs() < 0.01);
        let summary = app.world().resource::<ArenaSession>().round_summary();
        assert_eq!(
            summary.actors.first().unwrap().casts,
            [0, 0, u32::from(high_jump)]
        );
    }
}

#[test]
fn shared_input_clear_reset_and_terminal_discard_deferred_travel() {
    for boundary in ["input_clear", "reset", "terminal"] {
        let mut app = fixture(false);
        wait_with(
            &mut app,
            ActorIntent {
                jump: true,
                high_jump: true,
                glider_toggle: true,
                boat_toggle: true,
                flight_toggle: true,
                ..Default::default()
            },
        );
        match boundary {
            // Native pause/focus loss clears this resource. There is no second
            // hidden pending-input state that could survive that boundary.
            "input_clear" => {
                app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent::default()
            }
            "reset" => app.world_mut().resource_mut::<ArenaReset>().generation += 1,
            "terminal" => {
                app.world_mut().resource_mut::<ArenaSession>().outcome = Some(ArenaOutcome::Draw)
            }
            _ => unreachable!(),
        }
        app.world_mut().run_schedule(ArenaTick);
        let pending = app.world().resource::<ArenaInput>().human;
        assert!(
            !pending.jump
                && !pending.high_jump
                && !pending.boat_toggle
                && !pending.glider_toggle
                && !pending.flight_toggle
        );
        app.world_mut().resource_mut::<ArenaSession>().outcome = None;
        set_ready(&mut app, true);
        app.world_mut().run_schedule(ArenaTick);
        assert!(!player(&app).boat().unwrap().active);
        assert!(!player(&app).glider().unwrap().open);
        assert!(player(&app).free_flight().is_none());
        assert!(player(&app).feet.y < 0.01);
    }
}

#[test]
fn grand_wait_does_not_queue_contextual_actions_or_duplicate_cast_ownership() {
    let mut app = fixture(false);
    let feet = player(&app).feet;
    app.world_mut()
        .resource_mut::<ArenaTerrainView>()
        .anchors
        .insert(ShrineId::Water.anchor().into(), feet);
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .grand
        .as_mut()
        .unwrap()
        .teleport_unlocked = true;
    wait_with(
        &mut app,
        ActorIntent {
            teleport: true,
            interact: true,
            cast_pressed: true,
            cast_released: true,
            selected: Some(crate::Spell::Fireball),
            aim: Vec3::Y,
            ..Default::default()
        },
    );
    let pending = app.world().resource::<ArenaInput>().human;
    assert!(
        !pending.teleport && !pending.interact && !pending.cast_pressed && !pending.cast_released
    );
    assert!(pending.selected.is_none());
    // A new aim after loading must not resurrect the old X/R gesture. Managed
    // spell input may be replayed only by the existing application-owned queue.
    app.world_mut().resource_mut::<ArenaInput>().human.aim =
        (Vec3::X * 6.0 - Vec3::Y * player(&app).eye().y).normalize();
    set_ready(&mut app, true);
    app.world_mut().run_schedule(ArenaTick);
    let session = app.world().resource::<ArenaSession>();
    assert!((player(&app).feet - feet).length() < 0.01);
    let progress = session.grand_progress().unwrap();
    assert!(progress.shrines.is_empty());
    assert_eq!(progress.teleport_cooldown.to_bits(), 0.0_f32.to_bits());
    assert!(session.projectiles.is_empty() && player(&app).charge().is_none());
}

#[test]
fn unloaded_teleport_target_refusal_consumes_x_without_spending_cooldown() {
    let mut app = fixture(false);
    let feet = HexCoord::from_axial(-4, 4).to_world(SKIN);
    stage(&mut app, feet);
    app.world_mut()
        .resource_mut::<ArenaSession>()
        .grand
        .as_mut()
        .unwrap()
        .teleport_unlocked = true;
    {
        let mut world = app.world_mut().resource_mut::<ArenaTerrainView>();
        world.residency.as_mut().unwrap().ready.remove(&(0, 0));
        world.revision += 1;
        world.full_rebuild = true;
    }
    let tick = app.world().resource::<ArenaSession>().tick;
    app.world_mut().resource_mut::<ArenaInput>().human = ActorIntent {
        teleport: true,
        aim: (Vec3::X * 9.0 - Vec3::Y * player(&app).eye().y).normalize(),
        ..Default::default()
    };
    app.world_mut().run_schedule(ArenaTick);
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(
        session.tick,
        tick + 1,
        "only the target is unloaded, so X must be evaluated"
    );
    assert!(session
        .notice
        .contains("Teleport needs visible, clear ground"));
    assert!((player(&app).feet - feet).length() < 0.01);
    assert_eq!(
        session
            .grand_progress()
            .unwrap()
            .teleport_cooldown
            .to_bits(),
        0.0_f32.to_bits()
    );
    assert!(!app.world().resource::<ArenaInput>().human.teleport);
    set_ready(&mut app, true);
    app.world_mut().run_schedule(ArenaTick);
    assert!(
        (player(&app).feet - feet).length() < 0.01,
        "loading the target must not retry a refused X"
    );
}

#[test]
fn submerged_swimming_checkpoint_preserves_oxygen_velocity_and_next_tick() {
    let mut app = fixture(true);
    // Stage only the initial pose. Actual Grand swimming establishes movement
    // and depletes oxygen; neither reserve nor velocity is injected.
    let underwater = Vec3::new(0.0, -2.0, 0.0);
    stage(&mut app, underwater);
    let swimming = ActorIntent {
        aim: Vec3::X,
        movement: Vec2::Y,
        flight_vertical: -1.0,
        ..Default::default()
    };
    let initial_tick = app.world().resource::<ArenaSession>().tick;
    for _ in 0..90 {
        app.world_mut().resource_mut::<ArenaInput>().human = swimming;
        app.world_mut().run_schedule(ArenaTick);
    }
    let identity = GrandCheckpointIdentity {
        world_id: "grand-input-test".into(),
        content_revision: "swimming-resume".into(),
    };
    let session = app.world().resource::<ArenaSession>();
    assert_eq!(session.tick, initial_tick + 90);
    let saved_swim = player(&app).swimming().expect("Grand swimming");
    assert!(saved_swim.active && saved_swim.submerged);
    assert!(saved_swim.oxygen_seconds > 0.0);
    assert!(saved_swim.oxygen_seconds < saved_swim.oxygen_capacity_seconds - 0.5);
    assert!(!player(&app).boat().expect("folded boat").active);
    assert!(!player(&app).glider().expect("folded glider").open);
    assert!(player(&app).free_flight().is_none());
    let saved_feet = player(&app).feet;
    let saved_velocity = session.stream_interest().expect("swim interest").velocity;
    assert!(saved_feet.x > underwater.x + 0.5 && saved_feet.y < underwater.y);
    assert!(saved_velocity.x > 1.0 && saved_velocity.y < -1.0);
    let saved_tick = session.tick;
    let saved_clock = session.ocean_time();
    let bytes = session
        .encode_grand_checkpoint(&identity)
        .expect("swim save");
    let terrain = app.world().resource::<ArenaTerrainView>().clone();
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let generation = app.world().resource::<ArenaReset>().generation;

    // The uninterrupted run is the oracle for the same first ordinary input
    // after adoption, including the internal marine velocity and all clocks.
    app.world_mut().resource_mut::<ArenaInput>().human = swimming;
    app.world_mut().run_schedule(ArenaTick);
    let uninterrupted = app.world().resource::<ArenaSession>();
    assert_eq!(uninterrupted.tick, saved_tick + 1);
    assert!(player(&app).feet.distance(saved_feet) > 0.01);
    let next_swim = player(&app).swimming().expect("continued swimming");
    assert!(next_swim.active && next_swim.submerged);
    assert!(next_swim.oxygen_seconds < saved_swim.oxygen_seconds);
    let expected = uninterrupted
        .encode_grand_checkpoint(&identity)
        .expect("uninterrupted swim");

    let mut restored =
        ArenaSession::decode_grand_checkpoint(&bytes, &identity, &terrain, geometry, generation)
            .expect("decode active swim");
    restored
        .rebind_grand_terrain(&terrain, geometry, generation)
        .expect("admit underwater body without requiring dry ground");
    assert_eq!(restored.tick, saved_tick);
    assert_eq!(restored.ocean_time(), saved_clock);
    assert_eq!(
        restored.stream_interest().expect("restored swim").velocity,
        saved_velocity
    );
    assert_eq!(
        restored
            .encode_grand_checkpoint(&identity)
            .expect("rebound swim"),
        bytes,
        "adoption must not refill oxygen, change mode or discard marine state"
    );
    *app.world_mut().resource_mut::<ArenaSession>() = restored;
    assert_eq!(player(&app).feet, saved_feet);
    assert_eq!(player(&app).swimming(), Some(saved_swim));
    app.world_mut().resource_mut::<ArenaInput>().human = swimming;
    app.world_mut().run_schedule(ArenaTick);
    let resumed = app.world().resource::<ArenaSession>();
    assert_eq!(resumed.tick, saved_tick + 1);
    assert_eq!(player(&app).swimming(), Some(next_swim));
    assert_eq!(
        resumed
            .encode_grand_checkpoint(&identity)
            .expect("resumed swim"),
        expected,
        "the first resumed tick must match uninterrupted submerged movement"
    );
}
