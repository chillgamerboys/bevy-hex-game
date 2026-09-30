//! Grand V3's composed Crystal Ascent runtime contract.

use super::*;
use hex_core::MapObservationAnchors;

const GRAND_V3_SCENARIO: &str = "Grand V3 Baseline";
const CRYSTAL_FOCUS: &str = "crystal_ascent.corner_landing";
const CRYSTAL_HEART: &str = "prop/crystal-cathedral-heart";
const CRYSTAL_SITE_RADIUS: u32 = 32;

#[derive(Debug, PartialEq)]
struct GrandCrystalRuntimeSnapshot {
    map_fingerprint: u64,
    observation_anchors: Vec<(String, TilePos)>,
    heart: (TilePos, u8),
    heart_runs: AuthoredObjectVoxelRuns,
    authored_occupancy: AuthoredObjectOccupancy,
    heart_supports: BTreeSet<TilePos>,
    blocked_sight_pair: (TilePos, TilePos),
    fixture_lighting: Vec<(String, TilePos, u8, u32, LightDomain)>,
    observations: FactionObservations,
    knowledge: FactionMapKnowledge,
    local_knowledge: Vec<(TilePos, hex_core::KnownTraversal)>,
    fog_surfaces: BTreeSet<TilePos>,
    cutaway_roof_runs: BTreeSet<(TilePos, i32)>,
    tree_roots: BTreeSet<TilePos>,
    cutaway_tree_roots: BTreeSet<TilePos>,
}

fn grand_v3_crystal_app() -> App {
    let mut app = unfinished_procedural_gameplay_app(GRAND_V3_SCENARIO, false);
    let settings: CameraSettings = ron::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/config/camera.ron"
    )))
    .expect("the shipped camera settings should deserialize");
    settings
        .validate()
        .expect("the shipped camera settings should remain valid");
    app.insert_resource(settings);
    app.add_plugins(bevy::window::WindowPlugin {
        primary_window: None,
        ..default()
    });
    app.add_plugins(bevy::transform::TransformPlugin);
    app.add_plugins((
        hex_world::test_support::headless_camera_plugin,
        crate::fog::plugin,
    ));
    hex_world::install_full_cutaway_review_override(&mut app);
    finish_test_app(app)
}

fn focus_crystal_interior(app: &mut App) -> TilePos {
    let target = app
        .world()
        .resource::<MapAnchors>()
        .get(&MapAnchorId::from(CRYSTAL_FOCUS))
        .expect("Grand V3 should publish the Crystal corner landing");
    let surface = app
        .world()
        .resource::<SurfaceSnapshots>()
        .get(target)
        .expect("the Crystal corner landing should be an exposed runtime surface");
    let standing = Standing {
        pos: target,
        span: surface.span,
    };
    let player = {
        let world = app.world_mut();
        let mut players = world.query_filtered::<(Entity, &UnitId), With<Player>>();
        players
            .iter(world)
            .min_by_key(|(_, unit)| **unit)
            .map(|(entity, _)| entity)
            .expect("Grand V3 should retain its exploration party")
    };
    app.world_mut().entity_mut(player).insert((
        StandsOn(standing),
        Transform::from_translation(standing.world_position()),
        CameraFocusTarget::new(target),
    ));
    app.update();

    assert!(
        app.world()
            .resource::<FactionObservations>()
            .faction(Faction::Player)
            .observes(target),
        "the relocated player should authoritatively observe the Crystal landing"
    );
    target
}

fn assert_heart_blocks_traversal(
    app: &mut App,
    heart: &ObjectInstance,
    runs: &AuthoredObjectVoxelRuns,
    occupancy: &AuthoredObjectOccupancy,
) -> BTreeSet<TilePos> {
    let support_level = heart
        .origin()
        .level
        .checked_sub(1)
        .expect("the Grand heart should stand above a valid floor level");
    let heart_supports = runs
        .iter()
        .map(|run| TilePos::new(run.top.coord, support_level))
        .collect::<BTreeSet<_>>();
    let expected_supports = heart
        .origin()
        .coord
        .within_radius(4)
        .into_iter()
        .map(|coord| TilePos::new(coord, support_level))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        heart_supports, expected_supports,
        "the blueprint-derived heart volume should occupy the exact radius-four support footprint"
    );
    assert!(heart_supports
        .iter()
        .all(|support| { occupancy.blocks_standing_body(*support, TraversalProfile::WALKER) }));
    assert!(
        heart_supports.iter().all(|support| app
            .world()
            .resource::<TraversalBlockers>()
            .contains(*support)),
        "Grand's semantic traversal blockers should retain every exact heart support"
    );

    let substances = app.world().resource::<SubstanceTable>().clone();
    let body = {
        let world = app.world_mut();
        let mut players = world.query_filtered::<(&UnitId, &Body), With<Player>>();
        players
            .iter(world)
            .min_by_key(|(unit, _)| **unit)
            .map(|(_, body)| *body)
            .expect("Grand V3 should retain a player body")
    };
    assert_eq!(body.traversal_profile(), TraversalProfile::WALKER);
    {
        let world = app.world_mut();
        let mut tiles =
            world.query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
        let terrain_only = Footing::from_tiles(tiles.iter(world), &substances, body, None);
        assert!(
            heart_supports
                .iter()
                .all(|support| terrain_only.at(*support).is_some()),
            "the cathedral floor should be standable before the authored heart is composed"
        );
    }
    {
        let world = app.world_mut();
        let mut tiles =
            world.query_filtered::<(&TilePos, &HexSpan, &SubstanceId, &Headroom), With<HexTile>>();
        let object_aware = Footing::from_tiles_with_object_occupancy(
            tiles.iter(world),
            &substances,
            body,
            None,
            occupancy,
        );
        assert!(
            heart_supports
                .iter()
                .all(|support| object_aware.at(*support).is_none()),
            "the exact heart volume should remove every occupied support from runtime footing"
        );
    }
    heart_supports
}

fn crystal_fixture_lighting(
    app: &mut App,
    active_region: hex_core::InteriorRegionId,
    heart: TilePos,
) -> Vec<(String, TilePos, u8, u32, LightDomain)> {
    let mut fixtures = {
        let world = app.world_mut();
        let mut objects = world.query::<&ObjectInstance>();
        objects
            .iter(world)
            .filter(|instance| {
                instance.object_id().as_str().starts_with("prop/crystal-")
                    && instance.origin().coord.distance(heart.coord) <= CRYSTAL_SITE_RADIUS
            })
            .map(|instance| {
                let floor = TilePos::new(
                    instance.origin().coord,
                    instance
                        .origin()
                        .level
                        .checked_sub(1)
                        .expect("a Crystal Ascent fixture should stand above its floor"),
                );
                (
                    instance.object_id().as_str().to_owned(),
                    floor,
                    instance.rotation().steps(),
                )
            })
            .collect::<Vec<_>>()
    };
    fixtures.sort_unstable();
    assert_eq!(
        fixtures.len(),
        19,
        "Grand should retain exactly the heart and eighteen landing visual owners"
    );
    assert_eq!(
        fixtures
            .iter()
            .filter(|(object, _, _)| object == CRYSTAL_HEART)
            .count(),
        1,
        "Grand should publish exactly one cathedral-heart visual owner"
    );
    assert_eq!(
        fixtures
            .iter()
            .map(|(_, floor, _)| *floor)
            .collect::<BTreeSet<_>>()
            .len(),
        fixtures.len(),
        "each embedded Crystal fixture should own one distinct floor"
    );

    let sources = {
        let world = app.world_mut();
        let mut lights = world.query::<(&TilePos, &GameplayLight)>();
        lights
            .iter(world)
            .map(|(position, light)| (*position, *light))
            .collect::<Vec<_>>()
    };
    let interiors = app.world().resource::<InteriorRegions>();
    let illumination = app.world().resource::<ResolvedIllumination>();
    fixtures
        .into_iter()
        .map(|(object, floor, rotation)| {
            let radius = if object == CRYSTAL_HEART { 8 } else { 4 };
            assert_eq!(
                sources
                    .iter()
                    .filter(|(position, light)| {
                        *position == floor
                            && light.level == IlluminationLevel::Bright
                            && light.radius == radius
                    })
                    .count(),
                1,
                "each Crystal visual owner should have one matching Bright gameplay source at {floor:?}"
            );
            assert_eq!(
                interiors.get(floor),
                Some(active_region),
                "Crystal fixture floor {floor:?} should belong to Grand's unified interior"
            );
            let resolved = illumination
                .get(floor)
                .unwrap_or_else(|| panic!("missing resolved illumination at {floor:?}"));
            assert_eq!(resolved.level, IlluminationLevel::Bright);
            assert_eq!(resolved.domain, LightDomain::Interior(active_region));
            (object, floor, rotation, radius, resolved.domain)
        })
        .collect()
}

fn perception_and_fog_snapshot(
    app: &mut App,
    focus: TilePos,
) -> (
    FactionObservations,
    FactionMapKnowledge,
    Vec<(TilePos, hex_core::KnownTraversal)>,
    BTreeSet<TilePos>,
) {
    let observations = app.world().resource::<FactionObservations>().clone();
    let knowledge = app.world().resource::<FactionMapKnowledge>().clone();
    let expected_local = knowledge.player_local_map_knowledge();
    let local_knowledge = app
        .world()
        .resource::<LocalMapKnowledge>()
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(
        local_knowledge,
        expected_local.iter().collect::<Vec<_>>(),
        "Grand's movement-facing knowledge should be the exact player-faction projection"
    );

    let current_surfaces = {
        let world = app.world_mut();
        let mut tiles = world.query_filtered::<(&TilePos, &Headroom), With<HexTile>>();
        tiles
            .iter(world)
            .filter_map(|(position, headroom)| (headroom.0 > 0).then_some(*position))
            .collect::<BTreeSet<_>>()
    };
    let player_observation = observations.faction(Faction::Player);
    let observed_surfaces = player_observation.surfaces().collect::<BTreeSet<_>>();
    let knowledge_observed = current_surfaces
        .iter()
        .copied()
        .filter(|position| {
            knowledge.faction(Faction::Player).state(*position) == KnowledgeState::Observed
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        knowledge_observed, observed_surfaces,
        "current Grand observations should be the exact Observed knowledge subset"
    );
    assert_eq!(
        knowledge.faction(Faction::Player).state(focus),
        KnowledgeState::Observed
    );
    assert_eq!(
        app.world().resource::<LocalMapKnowledge>().state(focus),
        KnowledgeState::Observed
    );

    let expected_fog = current_surfaces
        .into_iter()
        .filter(|position| {
            knowledge.faction(Faction::Player).state(*position) != KnowledgeState::Observed
        })
        .collect::<BTreeSet<_>>();
    let fog_surfaces = crate::fog::fog_overlay_positions(app.world_mut());
    assert!(
        !fog_surfaces.is_empty(),
        "Grand should retain a tactical shroud"
    );
    assert_eq!(
        fog_surfaces, expected_fog,
        "fog caps should be exactly the current Unknown and Remembered Grand surfaces"
    );
    assert!(!fog_surfaces.contains(&focus));
    (observations, knowledge, local_knowledge, fog_surfaces)
}

fn cutaway_snapshot(
    app: &mut App,
    active_region: hex_core::InteriorRegionId,
) -> (
    BTreeSet<(TilePos, i32)>,
    BTreeSet<TilePos>,
    BTreeSet<TilePos>,
) {
    let interiors = app.world().resource::<InteriorRegions>().clone();
    assert_eq!(
        interiors
            .surfaces()
            .map(|(_, region)| region)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([active_region]),
        "Grand's tunnel and embedded ascent should share one interior"
    );

    let cutaway_roof_runs = {
        let world = app.world_mut();
        let mut roofs = world.query_filtered::<(
            &TilePos,
            &RunBottom,
            &CutawayOccluder,
            &PresentationOcclusion,
            Option<&Visibility>,
        ), With<HexTile>>();
        roofs
            .iter(world)
            .filter_map(|(position, bottom, cutaway, occlusion, visibility)| {
                if cutaway.0 == active_region {
                    assert!(occlusion.contains(PresentationOcclusionReason::InteriorCutaway));
                    assert_eq!(
                        visibility, None,
                        "logical roof runs must remain outside visibility propagation"
                    );
                    Some((*position, bottom.0))
                } else {
                    assert!(
                        !occlusion.contains(PresentationOcclusionReason::InteriorCutaway),
                        "full cutaway should not hide a roof owned by another interior"
                    );
                    None
                }
            })
            .collect::<BTreeSet<_>>()
    };
    assert!(
        !cutaway_roof_runs.is_empty(),
        "Grand's unified interior should publish cutaway-owned roof runs"
    );
    {
        let world = app.world_mut();
        let mut rendered_roofs =
            world.query::<(&TerrainRenderBatch, &CutawayOccluder, Option<&Visibility>)>();
        let hidden = rendered_roofs
            .iter(world)
            .filter(|(_batch, cutaway, visibility)| {
                cutaway.0 == active_region && *visibility == Some(&Visibility::Hidden)
            })
            .count();
        assert!(
            hidden > 0,
            "the active cutaway did not hide its render batches"
        );
    }

    let (tree_roots, expected_cutaway_roots, actual_cutaway_roots) = {
        let world = app.world_mut();
        let mut trees = world.query::<(
            &TreeOccluder,
            Option<&PresentationOcclusion>,
            Option<&Visibility>,
        )>();
        let mut roots = BTreeSet::new();
        let mut expected = BTreeSet::new();
        let mut actual = BTreeSet::new();
        for (tree, occlusion, visibility) in trees.iter(world) {
            roots.insert(tree.0);
            let should_hide = interiors.roof_region(tree.0) == Some(active_region);
            if should_hide {
                expected.insert(tree.0);
            }
            let is_cut_away = occlusion.copied().is_some_and(|occlusion| {
                occlusion.contains(PresentationOcclusionReason::InteriorCutaway)
            });
            if is_cut_away {
                actual.insert(tree.0);
                assert_eq!(visibility, Some(&Visibility::Hidden));
            }
        }
        (roots, expected, actual)
    };
    assert!(
        !tree_roots.is_empty(),
        "Grand should retain generated trees"
    );
    assert_eq!(
        actual_cutaway_roots, expected_cutaway_roots,
        "full cutaway should hide exactly the trees supported by the active roof"
    );
    (cutaway_roof_runs, tree_roots, actual_cutaway_roots)
}

fn grand_crystal_runtime_snapshot(app: &mut App) -> GrandCrystalRuntimeSnapshot {
    let mut observation_anchors = app
        .world()
        .resource::<MapObservationAnchors>()
        .iter()
        .map(|(id, position)| (id.as_str().to_owned(), position))
        .collect::<Vec<_>>();
    observation_anchors.sort();
    for required in [
        "grand_v3.lake_island",
        "grand_v3.massif_crest",
        "grand_v3.waterfall_base",
        "grand_v3.waterfall_crown",
    ] {
        assert!(
            observation_anchors.iter().any(|(id, _)| id == required),
            "Grand V3 omitted scenic observation anchor {required}"
        );
        assert!(
            app.world()
                .resource::<MapAnchors>()
                .get(&MapAnchorId::from(required))
                .is_none(),
            "scenic observation anchor {required} leaked into gameplay placement"
        );
    }
    let focus = focus_crystal_interior(app);
    let active_region = app
        .world()
        .resource::<InteriorRegions>()
        .get(focus)
        .expect("the Crystal landing should belong to Grand's unified interior");
    let (heart, heart_runs, authored_occupancy) = crystal_heart_occupancy_snapshot(app);
    assert_eq!(
        app.world_mut()
            .query::<&AuthoredObjectVoxelRuns>()
            .iter(app.world())
            .count(),
        1,
        "the cathedral heart should be Grand's only authored occupancy source"
    );
    let heart_supports =
        assert_heart_blocks_traversal(app, &heart, &heart_runs, &authored_occupancy);
    let blocked_sight_pair = crystal_heart_blocked_sight_pair(app, heart.origin());
    {
        let illumination = app.world().resource::<ResolvedIllumination>();
        let terrain = app.world().resource::<TerrainOccupancy>();
        let profile = app
            .world()
            .resource::<PerceptionSettings>()
            .active_profile();
        assert!(can_observe(
            blocked_sight_pair.0,
            blocked_sight_pair.1,
            illumination,
            profile,
            terrain,
        ));
        assert!(!can_observe_with_authored_objects(
            blocked_sight_pair.0,
            blocked_sight_pair.1,
            illumination,
            profile,
            terrain,
            &authored_occupancy,
        ));
    }
    let fixture_lighting = crystal_fixture_lighting(app, active_region, heart.origin());
    let (observations, knowledge, local_knowledge, fog_surfaces) =
        perception_and_fog_snapshot(app, focus);
    let (cutaway_roof_runs, tree_roots, cutaway_tree_roots) = cutaway_snapshot(app, active_region);

    GrandCrystalRuntimeSnapshot {
        map_fingerprint: app.world().resource::<GenerationReport>().map_fingerprint,
        observation_anchors,
        heart: (heart.origin(), heart.rotation().steps()),
        heart_runs,
        authored_occupancy,
        heart_supports,
        blocked_sight_pair,
        fixture_lighting,
        observations,
        knowledge,
        local_knowledge,
        fog_surfaces,
        cutaway_roof_runs,
        tree_roots,
        cutaway_tree_roots,
    }
}

#[test]
fn grand_v3_crystal_contract_rebuilds_identically_across_the_complete_lifecycle() {
    let mut app = grand_v3_crystal_app();
    enter_screen(&mut app, Screen::Gameplay);
    assert!(
        app.world().contains_resource::<TerrainReady>(),
        "Grand V3 setup failed: {:?}",
        app.world()
            .get_resource::<GameplaySetupFailure>()
            .map(|failure| failure.reason.as_str())
    );
    let first = grand_crystal_runtime_snapshot(&mut app);

    enter_screen(&mut app, Screen::Title);
    for (name, present) in [
        ("VoxelMap", app.world().contains_resource::<VoxelMap>()),
        (
            "TerrainOccupancy",
            app.world().contains_resource::<TerrainOccupancy>(),
        ),
        (
            "AuthoredObjectOccupancy",
            app.world().contains_resource::<AuthoredObjectOccupancy>(),
        ),
        (
            "ResolvedIllumination",
            app.world().contains_resource::<ResolvedIllumination>(),
        ),
        (
            "FactionObservations",
            app.world().contains_resource::<FactionObservations>(),
        ),
        (
            "FactionMapKnowledge",
            app.world().contains_resource::<FactionMapKnowledge>(),
        ),
        (
            "LocalMapKnowledge",
            app.world().contains_resource::<LocalMapKnowledge>(),
        ),
        (
            "InteriorRegions",
            app.world().contains_resource::<InteriorRegions>(),
        ),
        (
            "MapObservationAnchors",
            app.world().contains_resource::<MapObservationAnchors>(),
        ),
        (
            "TraversalBlockers",
            app.world().contains_resource::<TraversalBlockers>(),
        ),
        (
            "TerrainReady",
            app.world().contains_resource::<TerrainReady>(),
        ),
    ] {
        assert!(!present, "Grand teardown retained {name}");
    }
    assert!(crate::fog::fog_overlay_positions(app.world_mut()).is_empty());
    {
        let world = app.world_mut();
        assert_eq!(
            world
                .query::<&AuthoredObjectVoxelRuns>()
                .iter(world)
                .count(),
            0
        );
        assert_eq!(world.query::<&GameplayLight>().iter(world).count(), 0);
        assert_eq!(world.query::<&PointLight>().iter(world).count(), 0);
        assert_eq!(world.query::<&ObjectInstance>().iter(world).count(), 0);
        assert_eq!(
            world
                .query_filtered::<Entity, With<HexGrid>>()
                .iter(world)
                .count(),
            0
        );
        assert_eq!(world.query::<&CutawayOccluder>().iter(world).count(), 0);
        assert_eq!(world.query::<&TreeOccluder>().iter(world).count(), 0);
        let mut occlusions = world.query::<&PresentationOcclusion>();
        assert!(occlusions.iter(world).all(|occlusion| {
            !occlusion.contains(PresentationOcclusionReason::Fog)
                && !occlusion.contains(PresentationOcclusionReason::InteriorCutaway)
        }));
    }

    enter_screen(&mut app, Screen::Gameplay);
    assert!(app.world().contains_resource::<TerrainReady>());
    assert!(!app.world().contains_resource::<GameplaySetupFailure>());
    let second = grand_crystal_runtime_snapshot(&mut app);
    assert_eq!(
        second, first,
        "Grand re-entry rebuilt stale or divergent Crystal, perception, fog, or cutaway state"
    );
}
