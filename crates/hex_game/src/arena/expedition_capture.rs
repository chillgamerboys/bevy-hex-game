//! Explicit synthetic presentation fixtures; ordinary arena ticks own all rewards.

use bevy::prelude::*;
use hex_arena::{ArenaSession, ExpeditionReward, ExpeditionRole, ExpeditionSnapshot};
use hex_core::arena::{ArenaTerrainView, ArenaVoxelGeometry};

const FOUNTAIN: &str = "forest_fountain_01";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fixture {
    TrollOrb,
    DragonOrb,
    ShadowOrb,
    ShadowCollected,
    FountainSpent,
}

impl Fixture {
    fn parse(view: &str) -> Option<Self> {
        match view {
            "expedition-orb-troll" => Some(Self::TrollOrb),
            "expedition-orb-dragon" => Some(Self::DragonOrb),
            "expedition-orb-shadow" => Some(Self::ShadowOrb),
            "expedition-shadow-collected" => Some(Self::ShadowCollected),
            "expedition-fountain-spent" => Some(Self::FountainSpent),
            _ => None,
        }
    }

    fn milestone(self) -> Option<(ExpeditionRole, ExpeditionReward, &'static str)> {
        match self {
            Self::TrollOrb => Some((
                ExpeditionRole::Troll,
                ExpeditionReward::TrollDamage,
                "forest_troll",
            )),
            Self::DragonOrb => Some((
                ExpeditionRole::Dragon,
                ExpeditionReward::DragonExplosions,
                "dragon_upper",
            )),
            Self::ShadowOrb | Self::ShadowCollected => Some((
                ExpeditionRole::MountainShadow,
                ExpeditionReward::ShadowVitality,
                "mountain_shadow",
            )),
            Self::FountainSpent => None,
        }
    }

    fn ready(self, snapshot: &ExpeditionSnapshot, player_hp: f32, maximum_hp: f32) -> bool {
        if self == Self::FountainSpent {
            return snapshot
                .fountains
                .iter()
                .any(|pool| pool.name == FOUNTAIN && pool.consumed)
                && (player_hp - 100.0).abs() < 0.001;
        }
        let Some((_, reward, _)) = self.milestone() else {
            return false;
        };
        snapshot.milestones.iter().any(|milestone| {
            milestone.reward == reward
                && milestone.defeated
                && if self == Self::ShadowCollected {
                    milestone.collected
                        && milestone.available_position.is_none()
                        && (player_hp - 60.0).abs() < 0.001
                        && (maximum_hp - 125.0).abs() < 0.001
                } else {
                    !milestone.collected && milestone.available_position.is_some()
                }
        })
    }
}

pub(super) fn fixture_view(view: &str) -> bool {
    Fixture::parse(view).is_some()
}

/// Receipt text intentionally separates these staged pixels from combat evidence.
pub(super) fn description(view: &str) -> Option<&'static str> {
    Fixture::parse(view).map(|fixture| match fixture {
        Fixture::FountainSpent => "SYNTHETIC_PRESENTATION: frame 20 moves the wounded player into a published fountain water cell; ordinary ticks consume it. External composition camera; no native movement, XP, or natural-combat claim.",
        Fixture::ShadowCollected => "SYNTHETIC_PRESENTATION: frame 20 sets Shadow HP to zero and player HP to 60; frame 40 moves the player to the public reward-orb position. Ordinary ticks grant maximum HP without healing. External composition camera; no XP or natural-combat claim.",
        _ => "SYNTHETIC_PRESENTATION: frame 20 sets the milestone enemy role HP to zero (all three Dragons for that reward); ordinary ticks publish the reward orb. External composition camera; no XP or natural-combat claim.",
    })
}

/// Called only by the existing explicit windowless capture branch, before ArenaTick.
pub(super) fn stage(world: &mut World, frame: u32, view: &str) -> Result<(), String> {
    let Some(fixture) = Fixture::parse(view) else {
        return Ok(());
    };
    if frame == 20 {
        if world
            .resource::<ArenaSession>()
            .expedition_progress()
            .is_none()
        {
            return Err(
                "Expedition presentation fixture requires the admitted expedition package.".into(),
            );
        }
        if fixture == Fixture::FountainSpent {
            let geometry = *world.resource::<ArenaVoxelGeometry>();
            let point = fountain_cell(world.resource::<ArenaTerrainView>(), geometry).ok_or(
                "Presentation fixture requires a published fountain cell with real water.",
            )?;
            let mut session = world.resource_mut::<ArenaSession>();
            let human = session
                .human_actor_id()
                .ok_or("Presentation fixture needs a player.")?;
            let player = session
                .actors
                .iter_mut()
                .find(|actor| actor.id == human)
                .ok_or("Missing player.")?;
            player.hp = 60.0;
            player.feet = point;
            player.previous_feet = point;
        } else if let Some((role, _, _)) = fixture.milestone() {
            let mut session = world.resource_mut::<ArenaSession>();
            let expected = if role == ExpeditionRole::Dragon { 3 } else { 1 };
            if session
                .actors
                .iter()
                .filter(|actor| actor.expedition_role() == Some(role))
                .count()
                != expected
            {
                return Err(format!(
                    "Presentation fixture requires {expected} admitted {role:?} actors."
                ));
            }
            let human = session.human_actor_id();
            for actor in &mut session.actors {
                if actor.expedition_role() == Some(role) {
                    actor.hp = 0.0;
                }
                if fixture == Fixture::ShadowCollected && Some(actor.id) == human {
                    actor.hp = 60.0;
                }
            }
        }
        info!(
            fixture = description(view),
            "Staged expedition presentation fixture"
        );
    }
    if frame == 40 && fixture == Fixture::ShadowCollected {
        let point = world
            .resource::<ArenaSession>()
            .expedition_progress()
            .and_then(|snapshot| {
                snapshot
                    .milestones
                    .into_iter()
                    .find(|milestone| milestone.reward == ExpeditionReward::ShadowVitality)
            })
            .and_then(|milestone| milestone.available_position)
            .map(Vec3::from_array)
            .ok_or("Shadow orb was not published before the collection fixture.")?;
        let mut session = world.resource_mut::<ArenaSession>();
        let human = session
            .human_actor_id()
            .ok_or("Presentation fixture needs a player.")?;
        let player = session
            .actors
            .iter_mut()
            .find(|actor| actor.id == human)
            .ok_or("Missing player.")?;
        // Public orb position only: no reconstruction of private pickup offsets.
        player.feet = point;
        player.previous_feet = point;
    }
    Ok(())
}

fn fountain_cell(view: &ArenaTerrainView, geometry: ArenaVoxelGeometry) -> Option<Vec3> {
    let pool = view.expedition.as_ref()?.fountains.get(FOUNTAIN)?;
    let focus = super::encounter::forest_focus(view, geometry, FOUNTAIN)?;
    pool.cells
        .iter()
        .filter(|cell| {
            view.liquids.iter().any(|run| {
                run.bottom.coord == cell.coord
                    && run.bottom.level <= cell.level
                    && run.top_level >= cell.level
            })
        })
        .map(|cell| geometry.center(*cell))
        .min_by(|a, b| {
            a.distance_squared(focus)
                .total_cmp(&b.distance_squared(focus))
        })
}

pub(super) fn ready(session: &ArenaSession, view: &str) -> bool {
    let Some(fixture) = Fixture::parse(view) else {
        return false;
    };
    let Some(snapshot) = session.expedition_progress() else {
        return false;
    };
    let Some(player) = session
        .actors
        .iter()
        .find(|actor| Some(actor.id) == session.human_actor_id() && actor.hp > 0.0)
    else {
        return false;
    };
    fixture.ready(&snapshot, player.hp, player.max_hp)
}

/// Public orb or authored site/pool framing, never native player-camera evidence.
pub(super) fn camera(
    session: &ArenaSession,
    terrain: &ArenaTerrainView,
    geometry: ArenaVoxelGeometry,
    view: &str,
) -> Option<Transform> {
    let fixture = Fixture::parse(view)?;
    let target = if let Some((_, reward, site)) = fixture.milestone() {
        session
            .expedition_progress()
            .and_then(|snapshot| {
                snapshot
                    .milestones
                    .into_iter()
                    .find(|milestone| milestone.reward == reward)
                    .and_then(|milestone| milestone.available_position)
            })
            .map(Vec3::from_array)
            .or_else(|| {
                let support = terrain
                    .expedition
                    .as_ref()?
                    .encounters
                    .get(site)?
                    .deployment
                    .preferred;
                Some(support.coord.to_world(geometry.top(support) + 0.6))
            })?
    } else {
        super::encounter::forest_focus(terrain, geometry, FOUNTAIN)?
    };
    if fixture == Fixture::FountainSpent {
        return Some(super::encounter::fountain_camera(
            session, terrain, geometry, FOUNTAIN, target,
        ));
    }
    Some(super::encounter::feature_camera(
        session,
        target,
        Vec3::new(5.0, 3.0, 6.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_explicit_expedition_fixture_views_enable_staging() {
        for name in [
            "expedition-orb-troll",
            "expedition-orb-dragon",
            "expedition-orb-shadow",
            "expedition-shadow-collected",
            "expedition-fountain-spent",
        ] {
            assert!(fixture_view(name));
            assert!(description(name).is_some_and(|text| text.contains("SYNTHETIC_PRESENTATION")));
        }
        for name in ["first", "forest-ground", "expedition", "expedition-orb"] {
            assert!(!fixture_view(name));
            assert!(description(name).is_none());
        }
    }

    #[test]
    fn readiness_requires_the_requested_reward_or_pool_and_shadow_never_heals() {
        let mut snapshot = ExpeditionSnapshot {
            enemies_total: 114,
            enemies_defeated: 1,
            forest_total: 109,
            forest_defeated: 0,
            dragons_defeated: 0,
            milestones: [
                ExpeditionReward::TrollDamage,
                ExpeditionReward::DragonExplosions,
                ExpeditionReward::ShadowVitality,
            ]
            .map(|reward| hex_arena::MilestoneSnapshot {
                reward,
                defeated: false,
                available_position: None,
                collected: false,
            }),
            fountains: vec![hex_arena::FountainSnapshot {
                name: FOUNTAIN.into(),
                consumed: false,
            }],
        };
        assert!(!Fixture::ShadowOrb.ready(&snapshot, 60.0, 100.0));
        let shadow = snapshot
            .milestones
            .iter_mut()
            .find(|m| m.reward == ExpeditionReward::ShadowVitality)
            .expect("Shadow");
        shadow.defeated = true;
        shadow.available_position = Some([1.0, 2.0, 3.0]);
        assert!(Fixture::ShadowOrb.ready(&snapshot, 60.0, 100.0));
        assert!(!Fixture::TrollOrb.ready(&snapshot, 60.0, 100.0));
        assert!(!Fixture::ShadowCollected.ready(&snapshot, 60.0, 125.0));
        let shadow = snapshot
            .milestones
            .iter_mut()
            .find(|m| m.reward == ExpeditionReward::ShadowVitality)
            .expect("Shadow");
        shadow.collected = true;
        shadow.available_position = None;
        assert!(Fixture::ShadowCollected.ready(&snapshot, 60.0, 125.0));
        assert!(!Fixture::ShadowCollected.ready(&snapshot, 85.0, 125.0));
        assert!(!Fixture::ShadowOrb.ready(&snapshot, 60.0, 125.0));
        assert!(!Fixture::FountainSpent.ready(&snapshot, 100.0, 100.0));
        snapshot.fountains.first_mut().expect("pool").consumed = true;
        assert!(Fixture::FountainSpent.ready(&snapshot, 100.0, 100.0));
    }

    #[cfg(feature = "test-support")]
    #[test]
    #[ignore = "requires HEX_FOREST_WORLD pointing at the compiled expedition and companion"]
    fn expedition_presentation_fixtures_resolve_through_ordinary_arena_ticks() {
        use hex_core::arena::{ArenaMap, ArenaSelection, ArenaTick};
        for name in [
            "expedition-orb-troll",
            "expedition-orb-dragon",
            "expedition-orb-shadow",
            "expedition-shadow-collected",
            "expedition-fountain-spent",
        ] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(ArenaSelection {
                    map: ArenaMap::ForestMassif,
                    ..default()
                })
                .add_plugins((hex_map::arena::plugin, hex_arena::plugin));
            app.update();
            app.world_mut().resource_mut::<ArenaSession>().bot_enabled = false;
            app.world_mut().run_schedule(ArenaTick);
            assert!(!ready(app.world().resource::<ArenaSession>(), name));
            stage(app.world_mut(), 20, name).expect("stage fixture");
            app.world_mut().run_schedule(ArenaTick);
            if name == "expedition-shadow-collected" {
                assert!(!ready(app.world().resource::<ArenaSession>(), name));
                stage(app.world_mut(), 40, name).expect("stage collection");
                app.world_mut().run_schedule(ArenaTick);
            }
            let session = app.world().resource::<ArenaSession>();
            assert!(
                ready(session, name),
                "{name}: {:?}",
                session.expedition_progress()
            );
            assert_eq!(session.progress().expect("progress").total_xp, 0);
            assert!(
                camera(
                    session,
                    app.world().resource::<ArenaTerrainView>(),
                    *app.world().resource::<ArenaVoxelGeometry>(),
                    name
                )
                .is_some()
            );
        }
    }
}
