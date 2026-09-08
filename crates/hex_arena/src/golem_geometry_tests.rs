//! Full-actor acceptance of the exact compound body, independent of Golem AI.

use super::*;
use crate::spells::{forecast_spell, ForecastBody};
use crate::{ArenaSession, ArenaTuning, EncounterTuning, Spell};
use hex_core::arena::{ArenaSolidSpan, ArenaTerrainView};
use hex_core::{HexCoord, SubstanceId};

const WIDTH: f32 = hex_core::config::HEX_SMALL_DIAMETER;

fn golem(feet: Vec3) -> Actor {
    let mut actor = Actor::spawn(1, feet, Vec3::X);
    actor.configure_species(Species::Golem, &EncounterTuning::default());
    actor
}

fn put(view: &mut ArenaTerrainView, coord: HexCoord, bottom: i32, top: i32) {
    for level in bottom..=top {
        view.voxels
            .insert(TilePos::new(coord, level), SubstanceId(1));
    }
    view.columns.entry(coord).or_default().push(ArenaSolidSpan {
        bottom: TilePos::new(coord, bottom),
        top_level: top,
        substance: SubstanceId(1),
    });
}

fn world(view: &ArenaTerrainView) -> CollisionWorld {
    let mut world = CollisionWorld::default();
    world.refresh(view, ArenaVoxelGeometry::default());
    world
}

fn floor() -> ArenaTerrainView {
    let mut view = ArenaTerrainView::default();
    for coord in HexCoord::ORIGIN.within_radius(8) {
        put(&mut view, coord, 0, 0);
    }
    view
}

#[test]
fn exact_seven_cell_support_and_physical_footprint_ignore_face_aim() {
    let geometry = ArenaVoxelGeometry::default();
    let mut actor = golem(Vec3::NEG_Y * (SKIN * 3.0));
    let footprint = |actor: &Actor| {
        HexCoord::ORIGIN
            .within_radius(3)
            .into_iter()
            .filter(|coord| voxel_overlap(TilePos::new(*coord, 0), geometry, actor))
            .collect::<Vec<_>>()
    };
    let before = footprint(&actor);
    assert_eq!(before.len(), 7);
    assert!(HexCoord::ORIGIN
        .within_radius(1)
        .into_iter()
        .all(|coord| before.contains(&coord)));
    actor.aim = Vec3::Z;
    turn(&CollisionWorld::default(), &mut actor, 1.2, 3.0);
    assert_eq!(footprint(&actor), before);
    assert!(actor.body_yaw.abs() < SKIN);
}

#[test]
fn every_outer_prism_blocks_terrain_and_part_support_respects_floor_and_ceiling() {
    let mut view = floor();
    let actor = golem(Vec3::ZERO);
    for coord in HexCoord::ORIGIN.within_radius(1) {
        let mut blocked = view.clone();
        put(&mut blocked, coord, 1, 1);
        assert!(
            !clear(&world(&blocked), &actor, actor.feet, 0.0),
            "missed body column {coord:?}"
        );
    }
    for coord in HexCoord::ORIGIN.within_radius(3) {
        put(&mut view, coord, 6, 6);
    }
    let tunnel = world(&view);
    assert!(clear(&tunnel, &actor, actor.feet, 0.0));
    assert!(!clear(&tunnel, &actor, actor.feet + Vec3::Y * 0.01, 0.0));
    assert!(!clear(&tunnel, &actor, actor.feet - Vec3::Y * 0.01, 0.0));
    assert!(sweep(&tunnel, &actor, actor.feet, Vec3::X * 0.1).is_none());
    let roof = sweep(&tunnel, &actor, actor.feet, Vec3::Y).expect("actual five-level roof contact");
    assert!(roof.fraction < SKIN && roof.normal.y < -0.9);
    let mut one_support = ArenaTerrainView::default();
    put(&mut one_support, HexCoord::from_axial(1, 0), 0, 0);
    let landing = ground(&world(&one_support), &actor, Vec3::Y, 2.0)
        .expect("one outer component supports runtime movement");
    assert!((landing.y - SKIN).abs() < SKIN);
}

#[test]
fn ray_and_sphere_contacts_include_side_hexes_and_leave_the_notch_empty() {
    let actor = golem(Vec3::ZERO);
    let start = Vec3::new(-5.0, 1.0, 1.5);
    let hit = sweep_actor(start, Vec3::X * 10.0, &actor, true, 0.06).expect("upper side hex");
    assert!((hit.fraction - (5.0 - WIDTH - 0.06) / 10.0).abs() < SKIN);
    assert!(hit.normal.distance(Vec3::NEG_X) < SKIN);
    let notch = Vec3::new(0.0, 3.0, 2.3);
    assert!(sweep_actor(notch, Vec3::NEG_Y * 4.0, &actor, true, 0.06).is_none());
    assert!(distance(Vec3::new(0.0, 1.0, 2.3), &actor) > 0.25);
    assert!((distance(Vec3::new(WIDTH * 1.5 + 0.2, 1.0, 0.0), &actor) - 0.2).abs() < SKIN);
}

#[test]
fn relative_projectile_sweep_catches_a_crossing_union_at_the_first_outer_face() {
    let mut actor = golem(Vec3::X * 5.0);
    actor.previous_feet = Vec3::NEG_X * 5.0;
    let hit = sweep_actor(Vec3::Y, Vec3::ZERO, &actor, false, 0.06).expect("crossing body");
    assert!((hit.fraction - (5.0 - WIDTH * 1.5 - 0.06) / 10.0).abs() < SKIN);
    assert!(hit.normal.distance(Vec3::X) < SKIN);
    assert!(sweep_actor(Vec3::Y, Vec3::ZERO, &actor, true, 0.06).is_none());
}

#[test]
fn mouth_uses_radial_boundary_and_locked_direction_without_rotating_the_body() {
    let mut actor = golem(Vec3::new(2.0, 4.0, 1.0));
    let center_eye = actor.feet + Vec3::Y * 1.4;
    actor.aim = Vec3::Z;
    assert!(actor.eye().distance(center_eye + Vec3::Z * (2.0 - SKIN)) < SKIN);
    let locked = golem_mouth(&actor, Vec3::Z);
    actor.aim = Vec3::X;
    assert!(
        actor
            .eye()
            .distance(center_eye + Vec3::X * (WIDTH * 1.5 - SKIN))
            < SKIN
    );
    assert!(golem_mouth(&actor, Vec3::Z).distance(locked) < SKIN);
    assert!(distance(actor.eye(), &actor) < SKIN);
    assert!(actor.body_yaw.abs() < SKIN);
}

#[test]
fn camera_forecast_and_owner_reentry_use_the_same_union() {
    let target = golem(Vec3::ZERO);
    let mut caster = Actor::spawn(0, Vec3::new(-8.0, 0.0, 1.5), Vec3::X);
    caster.selected = Spell::Fireball;
    let mut tuning = ArenaTuning {
        projectile_gravity: 0.0,
        ..Default::default()
    };
    let observed = ForecastBody {
        id: target.id,
        feet: target.feet,
        velocity: Vec3::ZERO,
        predict_seconds: 0.5,
        species: target.species,
        team: target.team,
        dimensions: target.dimensions,
        yaw: 1.5,
        yaw_velocity: 3.0,
    };
    let view = ArenaTerrainView::default();
    let forecast = forecast_spell(
        &caster,
        &[observed],
        &world(&view),
        &view,
        ArenaVoxelGeometry::default(),
        &tuning,
        18.0,
    );
    let hit = forecast
        .impact
        .expect("observed side hex participates without live pose reads");
    assert_eq!(hit.actor, Some(target.id));
    assert!((hit.point.x + WIDTH + 0.06).abs() < 0.001);
    let session = ArenaSession {
        actors: vec![caster.clone(), target.clone()],
        ..Default::default()
    };
    let origin = Vec3::new(-8.0, 2.8, 1.5);
    let direction = Vec3::new(1.0, -0.2, 0.0).normalize();
    let expected_hit =
        sweep_actor(origin, direction * 80.0, &target, true, 0.0).expect("camera body ray");
    let expected = (origin + direction * 80.0 * expected_hit.fraction - caster.eye()).normalize();
    assert!(
        crate::spells::aim_from_camera(&session, caster.id, origin, direction).distance(expected)
            < SKIN
    );

    let mut owner = golem(Vec3::ZERO);
    owner.aim = Vec3::Y;
    owner.selected = Spell::Fireball;
    tuning.projectile_gravity = 20.0;
    let returning = forecast_spell(
        &owner,
        &[],
        &world(&view),
        &view,
        ArenaVoxelGeometry::default(),
        &tuning,
        12.0,
    );
    assert_eq!(
        returning
            .impact
            .expect("vertical shot exits and returns to its real owner")
            .actor,
        Some(owner.id)
    );
}

#[test]
fn finite_cone_queries_each_convex_component_without_filling_the_notch() {
    let actor = golem(Vec3::ZERO);
    assert!(exposed_cone_contact(
        &actor,
        Vec3::new(0.0, 1.0, 3.0),
        Vec3::NEG_Z,
        0.9,
        0.05,
        |_| true
    )
    .is_none());
    let origin = Vec3::new(-5.0, 1.0, 1.5);
    let contact = exposed_cone_contact(&actor, origin, Vec3::X, 4.0, 0.25, |point| point.z > 1.6)
        .expect("an exposed side-hex contact is available around the nearest blocked sample");
    let ray = contact - origin;
    assert!(distance(contact, &actor) < SKIN * 4.0);
    assert!(ray.length() <= 4.0 + SKIN);
    assert!(ray.normalize().dot(Vec3::X) >= 0.25_f32.cos() - SKIN);
    let mut obstruction_queries = 0;
    assert!(
        exposed_cone_contact(&actor, origin, Vec3::X, 4.0, 0.25, |_| {
            obstruction_queries += 1;
            false
        })
        .is_none()
    );
    assert!(obstruction_queries > 0 && obstruction_queries <= 7 * 27);
}

#[test]
fn complete_union_separation_escapes_internal_faces_for_every_body_kind() {
    let base = golem(Vec3::ZERO);
    for species in [Species::Human, Species::Dragon, Species::Golem] {
        let mut other = Actor::spawn(2, Vec3::ZERO, Vec3::NEG_Z);
        other.configure_species(species, &EncounterTuning::default());
        if species == Species::Dragon {
            other.body_yaw = 0.7;
        }
        let mut a = base.clone();
        let push = compound_separation(&a, &other).expect("real initial overlap");
        assert!(push.is_finite() && push.length() > SKIN);
        a.feet += push;
        other.feet -= push;
        assert!(
            compound_separation(&a, &other).is_none(),
            "unresolved internal-face overlap with {species:?}"
        );
    }
    let tiny_outside = Actor::spawn(0, Vec3::new(0.0, 0.6, 2.45), Vec3::X);
    assert!(
        compound_separation(&base, &tiny_outside).is_none(),
        "the external notch is not a filled AABB"
    );
}

#[test]
fn liquid_spans_and_playable_bounds_follow_outer_prisms_and_actual_corners() {
    let mut actor = golem(Vec3::ZERO);
    assert!(hex_span_overlap(
        &actor,
        HexCoord::from_axial(1, 0),
        0.0,
        0.4
    ));
    assert!(!hex_span_overlap(
        &actor,
        HexCoord::from_axial(2, 0),
        0.0,
        0.4
    ));
    assert!(!hex_span_overlap(&actor, HexCoord::ORIGIN, 2.1, 2.5));
    let geometry = ArenaVoxelGeometry {
        radius: 3,
        ..Default::default()
    };
    actor.feet = Vec3::X * 1.5;
    assert!(
        compound_contained(&actor, geometry),
        "actual hex vertices fit where an AABB corner would not"
    );
    actor.feet = Vec3::Z * 2.1;
    assert!(!compound_contained(&actor, geometry));
}

#[test]
fn grounded_golem_steps_one_level_and_rejects_jump_flight_and_a_tall_wall() {
    let mut view = floor();
    put(&mut view, HexCoord::from_axial(2, 0), 1, 1);
    put(&mut view, HexCoord::from_axial(4, 0), 1, 6);
    let world = world(&view);
    let mut actor = golem(Vec3::Y * SKIN);
    for _ in 0..60 {
        crate::motion::tick(
            &mut actor,
            Vec3::X,
            true,
            true,
            true,
            &world,
            &EncounterTuning::default(),
        );
        assert!(clear(&world, &actor, actor.feet, actor.body_yaw));
        assert!(!actor.flying && actor.body_yaw.abs() < SKIN);
    }
    assert!(actor.feet.x > 0.9);
    assert!((actor.feet.y - 0.4 - SKIN).abs() < 0.001);
    for _ in 0..240 {
        crate::motion::tick(
            &mut actor,
            Vec3::X,
            true,
            true,
            true,
            &world,
            &EncounterTuning::default(),
        );
        assert!(clear(&world, &actor, actor.feet, actor.body_yaw));
        assert!(!actor.flying);
    }
    assert!(actor.feet.x <= WIDTH * 2.0 + SKIN);
    assert!(actor.feet.y < 0.401);
}
