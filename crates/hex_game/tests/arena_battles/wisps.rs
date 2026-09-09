//! Layered creature deployment through both actual world publishers.

use super::*;
use hex_arena::Species;

#[test]
fn wisps_and_single_goblin_admit_on_both_maps_with_twenty_four_body_capacity() {
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        for (left, right) in [
            (BattlePreset::Wisp, BattlePreset::Goblin),
            (BattlePreset::Goblin, BattlePreset::Wisp),
            (BattlePreset::Wisps12, BattlePreset::Wisps12),
        ] {
            let mut fixture = app(map, ArenaBattleSetup::spectator(left, right, 31));
            let session = fixture.world().resource::<ArenaSession>();
            let world = fixture.world().resource::<ArenaTerrainView>();
            let geometry = *fixture.world().resource::<ArenaVoxelGeometry>();
            assert_eq!(session.human_actor_id(), None);
            assert!(
                battle(&fixture).result.is_none(),
                "{map:?} {left:?}/{right:?}"
            );
            assert_eq!(
                session.actors.len(),
                left.members().len() + right.members().len()
            );
            for actor in &session.actors {
                assert!(session.actor_pose_valid(actor.id, world, geometry));
                if actor.species == Species::Wisp {
                    assert!(actor.flying && !actor.grounded);
                    let parts: Vec<_> = actor.body_hex_prisms().collect();
                    assert_eq!(parts.len(), 1);
                    assert!(parts.iter().all(|part| part.offset.length() < 0.0001
                        && (part.height - geometry.level_height).abs() < 0.0001));
                    let configuration = &fixture.world().resource::<ArenaTuning>().encounters;
                    assert!(
                        world
                            .battle_deployment
                            .as_ref()
                            .expect("published deployment")
                            .iter()
                            .flat_map(|region| &region.surfaces)
                            .any(|surface| {
                                let planar = surface.coord.to_world(actor.feet.y);
                                let elevation = actor.feet.y - geometry.top(*surface);
                                planar.distance(actor.feet) < 0.1
                                    && [
                                        configuration.wisp_cruise_height,
                                        configuration.wisp_cruise_height
                                            + configuration.wisp_layer_spacing,
                                    ]
                                    .iter()
                                    .any(|height| (elevation - height).abs() < 0.01)
                            }),
                        "{map:?} Wisp{} must start in an admitted finite flying layer",
                        actor.id
                    );
                }
            }
            // Exercise ordinary goals and physical movement after admission. A setup-only
            // fixture would miss flying flags or a layer preference being overwritten.
            for _ in 0..60 {
                fixture.world_mut().run_schedule(ArenaTick);
            }
            let session = fixture.world().resource::<ArenaSession>();
            let world = fixture.world().resource::<ArenaTerrainView>();
            for actor in session.actors.iter().filter(|actor| actor.hp > 0.0) {
                assert!(
                    session.actor_volume_valid(actor.id, world, geometry),
                    "{map:?} {left:?}/{right:?} moved actor{} {:?}",
                    actor.id,
                    actor.feet
                );
                if actor.species == Species::Wisp {
                    assert!(actor.flying);
                    assert!(session.actor_pose_valid(actor.id, world, geometry));
                }
                // Ground combatants may be airborne after Ember knockback or lost
                // footing; their occupied volume must still remain clear and dry.
            }
        }
    }
}

#[test]
fn fort_player_wisp_party_resets_back_to_the_accepted_duel() {
    let setup = ArenaBattleSetup {
        player_recipe: Some(BattlePreset::Wisps4),
        ..Default::default()
    };
    let mut fixture = app(ArenaMap::Fort, setup.clone());
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.accepted_battle_setup(), &setup);
    assert_eq!(session.human_actor_id(), Some(0));
    assert!(!session.is_finished());
    assert_eq!(session.actors.len(), 5);
    assert_eq!(
        session
            .actors
            .iter()
            .filter(|actor| actor.species == Species::Wisp)
            .count(),
        4
    );
    let human = session
        .actors
        .iter()
        .find(|actor| actor.id == 0)
        .expect("human body");
    let radius = fixture
        .world()
        .resource::<ArenaTuning>()
        .encounters
        .activation_radius;
    for wisp in session
        .actors
        .iter()
        .filter(|actor| actor.species == Species::Wisp)
    {
        assert!(human.eye().distance(wisp.eye()) > radius);
    }
    *fixture.world_mut().resource_mut::<ArenaBattleSetup>() = ArenaBattleSetup::default();
    fixture.world_mut().resource_mut::<ArenaSelection>().map = ArenaMap::Duel;
    fixture.world_mut().resource_mut::<ArenaReset>().generation += 1;
    fixture.world_mut().run_schedule(ArenaTick);
    let session = fixture.world().resource::<ArenaSession>();
    assert_eq!(session.human_actor_id(), Some(0));
    assert!(session.battle_summary().is_none());
    assert!(!session.is_finished());
    assert_eq!(session.actors.len(), 2);
    assert!(session
        .actors
        .iter()
        .all(|actor| matches!(actor.species, Species::Human | Species::Shadow)));
}
