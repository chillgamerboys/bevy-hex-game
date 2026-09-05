//! Final-world composition checks for the six recovered map fixes.
//!
//! The small solver tests retain their independent corner/width fixtures. This
//! module checks their published geometry after every subsequent generation pass.
//! Natural-pass fixtures were captured independently from the accepted V3 source
//! before these fixes, at construction and again after final vegetation.

use super::*;

#[test]
fn garden_spring_stone_bed_keeps_exact_voxels_in_canonical_runs() {
    let coord = HexCoord::ORIGIN;
    let bed = TilePos::new(coord, 152);
    let mut volume = VolumePlan::new(BTreeSet::from([coord]));
    volume.columns.insert(
        coord,
        water_column(bed.level, 153, SolidMaterialRole::Stone),
    );
    volume.surfaces.insert(
        bed,
        SurfaceMetadata {
            access: SurfaceAccess::NonStandable,
            interior: None,
        },
    );
    volume.validate().expect("spring column is canonical");
    for level in 1..=bed.level {
        assert_eq!(
            solid_material_at(&volume, TilePos::new(coord, level)),
            Some(SolidMaterialRole::Stone)
        );
    }
    assert_eq!(
        volume.fill_runs_by_top().get(&TilePos::new(coord, 153)),
        Some(&NonSolidFill {
            levels: LevelInterval::new(153, 154),
            material: FillMaterialRole::Water,
        })
    );
}

#[test]
fn final_hero_preserves_intake_river_bridges_and_expanded_tunnel() {
    assert_final_map_fixes(1_592_598_566);
}

#[test]
fn final_seed_zero_preserves_intake_river_bridges_and_expanded_tunnel() {
    assert_final_map_fixes(0);
}

fn assert_final_map_fixes(seed: u64) {
    let template = hex_schematic::grand_v3_reference_template().expect("template parses");
    let settings = ProceduralV3Settings {
        layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
            template: V3SchematicTemplate::GrandV3,
            template_revision: V3_GRAND_V3_TEMPLATE_REVISION,
            cell_pitch: 22,
            terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                V3GrandV3BasicTerrainProfile::canonical(),
            ),
        }),
    };
    let schematic = hex_schematic::generate(&template, seed)
        .expect("representative schematic generates")
        .plan;
    let selection = compile_schematic(
        &schematic,
        &settings,
        V3_SCHEMATIC_GRID_RADIUS,
        0.35,
        super::super::vegetation::tests::runtime_art_catalog(),
    )
    .unwrap_or_else(|error| panic!("seed {seed} final generation failed: {error}"));
    let world = &selection.validated.plan;
    assert_original_natural_pass(seed, world);
    assert_final_tunnel(&schematic, world);
    assert_final_hydrology(&schematic, world);
}

#[derive(serde::Deserialize)]
struct OriginalNaturalPass {
    width: u32,
    centerline: Vec<(i32, i32, Level)>,
    surfaces: Vec<(i32, i32, Level)>,
}

fn assert_original_natural_pass(seed: u64, world: &GeneratedWorldPlan) {
    let fixture = match seed {
        0 => include_str!("fixtures/natural-pass-v3-seed-0.ron"),
        1_592_598_566 => include_str!("fixtures/natural-pass-v3-seed-1592598566.ron"),
        _ => panic!("no independent original route fixture for seed {seed}"),
    };
    let expected: OriginalNaturalPass =
        ron::from_str(fixture).expect("original route fixture parses");
    let position = |(q, r, level)| TilePos::new(HexCoord::from_axial(q, r), level);
    let route = world
        .features
        .protected_routes
        .get("grand_v3.natural_pass")
        .expect("final world retains the natural ascent");
    assert_eq!(
        route.centerline,
        expected
            .centerline
            .into_iter()
            .map(position)
            .collect::<Vec<_>>(),
        "adjacent grading and underground expansion must preserve every original centerline grade"
    );
    assert_eq!(
        route.surfaces,
        expected
            .surfaces
            .into_iter()
            .map(position)
            .collect::<BTreeSet<_>>(),
        "the original walking width and exact route footprint must remain unchanged"
    );
    let actual_width =
        validate_natural_pass_physical_width(seed, route, &world.volume, Some(&world.blockers))
            .expect("the final original route retains physical clearance");
    assert_eq!(actual_width, expected.width);
    for surface in &route.surfaces {
        assert_eq!(
            world
                .volume
                .surfaces
                .get(surface)
                .map(|metadata| metadata.access),
            Some(SurfaceAccess::Ordinary),
            "every original route surface remains walkable after final publication"
        );
    }
}

/// Check published volume and graph facts in normalized river rows. Geometric
/// centerline order is not a downstream path where a later row reclaims a bend.
fn assert_published_river_flow(
    rows: &[BTreeSet<TilePos>],
    fills: &BTreeMap<TilePos, NonSolidFill>,
    liquids: &LiquidPlan,
    source_level: Level,
    terminal_level: Level,
    source_rapids: usize,
) {
    assert!(rows.iter().all(|row| row.len() == 3));
    let sources = rows.first().expect("river retains its three source lanes");
    let terminals = rows.last().expect("river retains its three terminal lanes");
    assert!(sources
        .iter()
        .all(|position| position.level == source_level));
    assert!(terminals
        .iter()
        .all(|position| position.level == terminal_level));
    let positions = rows.iter().flatten().copied().collect::<BTreeSet<_>>();
    let coords = positions
        .iter()
        .map(|position| position.coord)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        positions.len(),
        coords.len(),
        "normalized rows have one height per coordinate"
    );
    let mut owners = BTreeSet::new();
    for position in &positions {
        let actual = fills
            .range(
                TilePos::new(position.coord, Level::MIN)..=TilePos::new(position.coord, Level::MAX),
            )
            .filter(|(_, fill)| fill.material == FillMaterialRole::Water)
            .map(|(top, _)| *top)
            .collect::<Vec<_>>();
        assert_eq!(
            actual.as_slice(),
            [*position],
            "river water changed at {position:?}"
        );
        let fill = fills.get(position).expect("exact river water is present");
        assert_eq!(fill.levels.top, position.level + 1);
        let exact_owners = liquids
            .bodies
            .iter()
            .filter(|(_, body)| body.nodes.contains_key(position))
            .map(|(id, body)| {
                assert_eq!(body.material, FillMaterialRole::Water);
                *id
            })
            .collect::<Vec<_>>();
        assert_eq!(
            exact_owners.len(),
            1,
            "river water needs one actual body at {position:?}"
        );
        owners.extend(exact_owners);
    }
    assert_eq!(owners.len(), 1, "all river lanes retain one water body");
    let body = liquids
        .bodies
        .get(owners.first().expect("river has an owner"))
        .expect("river owner exists");

    // Trace every node, including side branches that the three source paths do
    // not visit. This proves final reachability and exact flow states in addition
    // to preserving all seven one-level drops on each complete 15-to-8 path.
    for source in &positions {
        let mut cursor = *source;
        let mut seen = BTreeSet::new();
        let mut rapids = 0_usize;
        loop {
            assert!(
                seen.insert(cursor),
                "river path from {source:?} cycles at {cursor:?}"
            );
            let node = body
                .nodes
                .get(&cursor)
                .expect("actual river node retains its body");
            if terminals.contains(&cursor) {
                assert_eq!(cursor.level, terminal_level);
                assert_eq!(node.state, LiquidFlowState::Still);
                assert_eq!(node.downstream, None);
                break;
            }
            let next = node
                .downstream
                .expect("every nonterminal river node must drain");
            assert!(
                positions.contains(&next),
                "river edge leaves its normalized ribbon: {cursor:?} -> {next:?}"
            );
            assert_eq!(cursor.coord.distance(next.coord), 1);
            let drop = cursor.level - next.level;
            assert!(
                [0, 1].contains(&drop),
                "river edge must descend zero or one level: {cursor:?} -> {next:?}"
            );
            let expected_state = if drop == 1 {
                rapids += 1;
                LiquidFlowState::Rapid
            } else {
                LiquidFlowState::Current
            };
            assert_eq!(
                node.state, expected_state,
                "actual river flow at {cursor:?}"
            );
            cursor = next;
        }
        assert_eq!(
            rapids,
            usize::try_from(source.level - terminal_level)
                .expect("river source is at or above its terminal")
        );
        if sources.contains(source) {
            assert_eq!(
                rapids, source_rapids,
                "every inlet lane retains all authored river drops: {source:?}"
            );
        }
    }
}

#[test]
fn normalized_river_bend_flows_downstream_despite_raw_centerline_rebound() {
    let centerline = [(-1, 0, 15), (0, 0, 15), (1, 0, 15), (1, -1, 14)]
        .map(|(q, r, level)| TilePos::new(HexCoord::from_axial(q, r), level));
    // Literal three-wide rows use the same transverse axis through the bend.
    // The final row reclaims (0,0), but the previous center (1,0) is still high.
    let authored_rows = centerline
        .iter()
        .map(|center| {
            (-1..=1)
                .map(|offset| {
                    TilePos::new(
                        HexCoord::from_axial(center.coord.x() + offset, center.coord.y() - offset),
                        center.level,
                    )
                })
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    for (index, source) in authored_rows.iter().enumerate().take(3) {
        let target = authored_rows
            .get(index + 1)
            .expect("fixture retains the next row");
        assert!(three_lane_rows_preserve_exact_progression(
            source,
            target,
            authored_rows.get(index + 2),
        ));
    }
    let rows = normalize_three_lane_row_claims(&authored_rows);
    let positions = rows.iter().flatten().copied().collect::<BTreeSet<_>>();
    let actual_centers = centerline
        .iter()
        .map(|center| {
            positions
                .iter()
                .find(|position| position.coord == center.coord)
                .expect("every geometric center retains water")
                .level
        })
        .collect::<Vec<_>>();
    assert_eq!(actual_centers, [15, 14, 15, 14]);
    assert!(actual_centers
        .windows(2)
        .any(|pair| matches!(pair, [before, after] if after > before)));

    let mut volume = VolumePlan::new(positions.iter().map(|position| position.coord).collect());
    for position in &positions {
        volume.columns.insert(
            position.coord,
            water_column(position.level - 1, position.level, SolidMaterialRole::Dirt),
        );
    }
    let fills = volume.fill_runs_by_top();
    let mut nodes = positions
        .iter()
        .map(|position| {
            (
                *position,
                LiquidNode {
                    state: LiquidFlowState::Still,
                    downstream: None,
                },
            )
        })
        .collect();
    apply_directed_watercourse(
        &rows,
        &OutletAuthority {
            edges: BTreeMap::new(),
            downstream_course: BTreeSet::new(),
        },
        &fills,
        &mut nodes,
    )
    .expect("the bend admits actual downstream flow");
    let owner = LiquidBodyId(1);
    let mut liquids = LiquidPlan {
        bodies: BTreeMap::from([(
            owner,
            LiquidBodyPlan {
                material: FillMaterialRole::Water,
                nodes,
            },
        )]),
    };
    assert!(
        liquids.validate(&volume).is_empty(),
        "synthetic flow meets canonical liquid contracts"
    );
    assert_published_river_flow(&rows, &fills, &liquids, 15, 14, 1);

    // The oracle reads the published class, rather than deriving a replacement
    // class and accidentally hiding a lost Rapid in the final liquid plan.
    let rapid = liquids
        .bodies
        .get_mut(&owner)
        .expect("fixture body exists")
        .nodes
        .values_mut()
        .find(|node| node.state == LiquidFlowState::Rapid)
        .expect("fixture contains a real one-level Rapid");
    rapid.state = LiquidFlowState::Current;
    assert!(
        std::panic::catch_unwind(|| {
            assert_published_river_flow(&rows, &fills, &liquids, 15, 14, 1);
        })
        .is_err(),
        "misclassified final flow must fail the oracle"
    );
}

fn assert_final_hydrology(schematic: &SchematicPlanV1, world: &GeneratedWorldPlan) {
    let profile = V3GrandV3BasicTerrainProfile::canonical();
    let ribbon = authoritative_hydrology_centerlines(
        schematic,
        profile,
        schematic.provenance.world_seed,
        &world.layout,
    )
    .expect("locked hydrology resolves");
    let lake = semantic_overlay_coords(schematic, &world.layout, SchematicFeature::MountainLake);
    let intake = resolve_waterfall_intake_rows(
        &ribbon.watercourse_rows,
        ribbon.waterfall_lip_index,
        &world.layout.footprint,
        &lake,
        &BTreeSet::new(),
    )
    .expect("the bounded intake fits the existing spine");
    assert_eq!(
        intake.iter().map(BTreeSet::len).collect::<Vec<_>>(),
        [9, 7, 5, 3]
    );
    let fills = world.volume.fill_runs_by_top();
    let nodes = world
        .liquids
        .bodies
        .values()
        .flat_map(|body| body.nodes.iter().map(|(position, node)| (*position, *node)))
        .collect::<BTreeMap<_, _>>();
    let mut owners = BTreeSet::new();
    for position in intake.iter().flatten() {
        let fill = fills
            .get(position)
            .unwrap_or_else(|| panic!("final generation erased intake water at {position:?}"));
        assert_eq!(fill.material, FillMaterialRole::Water);
        assert_eq!(fill.levels.top, position.level + 1);
        let exact_owners = world
            .liquids
            .bodies
            .iter()
            .filter(|(_, body)| body.nodes.contains_key(position))
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        assert_eq!(
            exact_owners.len(),
            1,
            "intake water must retain one actual owner"
        );
        owners.extend(exact_owners);
    }
    assert_eq!(owners.len(), 1);
    validate_waterfall_intake_flow(&intake, &nodes)
        .expect("the final added water drains through the unchanged handoff");
    for source in intake.first().expect("nine-wide lake intake") {
        assert_eq!(source.level, 150);
        let mut cursor = *source;
        let mut seen = BTreeSet::new();
        while let Some(next) = nodes.get(&cursor).expect("actual liquid node").downstream {
            assert!(seen.insert(cursor), "final water graph contains a cycle");
            assert_eq!(cursor.coord.distance(next.coord), 1);
            assert!(next.level <= cursor.level);
            if nodes.get(&cursor).expect("actual source node").state == LiquidFlowState::Fall {
                assert!(
                    fills
                        .get(&cursor)
                        .expect("actual falling fill")
                        .levels
                        .bottom
                        <= next.level + 1,
                    "the falling intake must physically reach its downstream surface"
                );
            }
            cursor = next;
        }
        assert_eq!(cursor.level, 8, "every intake lane must reach the sea");
    }

    // The authored descent schedule remains seven one-level drops. At bends,
    // actual flow follows normalized rows and published downstream edges below.
    let river_levels = ribbon
        .river_centerline
        .iter()
        .map(|center| center.level)
        .collect::<Vec<_>>();
    assert_eq!(river_levels.first(), Some(&15));
    assert_eq!(river_levels.last(), Some(&8));
    let drops = river_levels
        .windows(2)
        .filter_map(|pair| match pair {
            [before, after] => Some(before - after),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(
        drops.iter().all(|drop| [0, 1].contains(drop)),
        "authored river profile must descend one level at a time: levels={river_levels:?}, drops={drops:?}",
    );
    assert_eq!(drops.iter().filter(|drop| **drop == 1).count(), 7);

    let river_rows = ribbon
        .watercourse_rows
        .get(ribbon.waterfall_centerline.len().saturating_sub(1)..)
        .expect("river rows begin at the waterfall receiving basin");
    assert_eq!(river_rows.len(), ribbon.river_centerline.len());
    let sea = semantic_sea_coords(schematic, &world.layout);
    assert!(
        river_rows
            .last()
            .expect("river has a terminal row")
            .iter()
            .all(|position| sea.contains(&position.coord)),
        "river terminals remain in the actual sea"
    );
    assert_published_river_flow(river_rows, &fills, &world.liquids, 15, 8, 7);

    let bridges = world
        .structures
        .by_id
        .values()
        .filter(|structure| structure.kind == StructureKind::Bridge)
        .collect::<Vec<_>>();
    assert_eq!(bridges.len(), 2);
    for bridge in bridges {
        assert_eq!(bridge.voxels.len(), 10);
        let mut wet_deck = BTreeSet::new();
        for deck in &bridge.voxels {
            assert_eq!(
                solid_material_at(&world.volume, *deck),
                Some(SolidMaterialRole::WorkedStone)
            );
            assert_eq!(
                world
                    .volume
                    .surfaces
                    .get(deck)
                    .map(|metadata| metadata.access),
                Some(SurfaceAccess::Ordinary)
            );
            for (water, fill) in fills
                .range(TilePos::new(deck.coord, Level::MIN)..=TilePos::new(deck.coord, Level::MAX))
            {
                if fill.material == FillMaterialRole::Water {
                    assert!(
                        water.level < deck.level,
                        "the bridge must remain above water"
                    );
                    wet_deck.insert(*deck);
                }
            }
        }
        assert_eq!(
            wet_deck.len(),
            6,
            "each two-wide bridge must span all three river lanes"
        );
    }
}

fn assert_final_tunnel(schematic: &SchematicPlanV1, world: &GeneratedWorldPlan) {
    let floor_level = V3GrandV3BasicTerrainProfile::canonical().crystal_base_level;
    let terminal = &world
        .features
        .protected_routes
        .get("crystal_ascent.lower_terminal_pad")
        .expect("the exact Crystal terminal is published")
        .surfaces;
    let entry = world
        .anchors
        .get("crystal_ascent.lower_entry")
        .expect("the Crystal entry anchor remains present");
    let crystal = &world
        .layout
        .patches
        .values()
        .find(|patch| patch.mask.contains(&entry.coord))
        .expect("the Crystal site keeps its exact mask")
        .mask;
    let coarse = schematic_network_path(schematic, NetworkKind::Tunnel, "edge/tunnel-complete")
        .expect("the authored tunnel route remains present");
    let lane = resolve_exact_terminal_lane(
        terminal,
        crystal,
        &world.layout.footprint,
        fine_network_path(&coarse, 22),
    )
    .expect("the original tunnel lane frame resolves");
    let foot = *lane.centerline.last().expect("tunnel foot");
    let direction = forward_path_direction(&lane.centerline, lane.centerline.len() - 1);
    let approach = foot.line_between(step_in_direction(foot, direction, 32));
    let approach_footprint = approach
        .iter()
        .skip(1)
        .flat_map(|center| {
            lane.lane_offsets
                .into_iter()
                .map(move |offset| step_in_direction(*center, (direction + 2) % 6, offset))
        })
        .collect::<BTreeSet<_>>();
    let carve = plan_tunnel_interior_carve(
        &lane,
        crystal,
        &world.layout.footprint,
        &approach_footprint,
        12,
    )
    .expect("the additive tunnel envelope resolves");
    assert_eq!(
        world.interiors.by_id.len(),
        1,
        "the tunnel and ascent share one interior"
    );
    let (interior_id, interior) = world
        .interiors
        .by_id
        .first_key_value()
        .expect("the tunnel retains its unified interior");
    validate_compiled_tunnel_geometry(&world.volume, *interior_id, floor_level, &carve)
        .expect("all expanded floors, ceilings, and roofs survive final generation");
    assert!(carve.sections.iter().any(|row| row.len() == 8));
    assert!(carve.sections.iter().any(|row| row.len() == 6));
    let route = world
        .features
        .protected_routes
        .get("grand_v3.tunnel")
        .expect("the tunnel route remains protected");
    let mut passage = carve.columns.keys().copied().collect::<BTreeSet<_>>();
    passage.extend(terminal.iter().map(|floor| floor.coord));
    passage.extend(approach_footprint);
    for (coord, shape) in &carve.columns {
        let floor = TilePos::new(*coord, floor_level);
        assert!(route.surfaces.contains(&floor) && interior.floors.contains(&floor));
        assert!((shape.clearance_top..shape.roof_top())
            .all(|level| interior.roof_voxels.contains(&TilePos::new(*coord, level))));
        assert!(
            world
                .volume
                .columns
                .get(coord)
                .expect("expanded tunnel column")
                .elements
                .iter()
                .all(|element| {
                    let VolumeElement::Fill(fill) = element else {
                        return true;
                    };
                    fill.levels.top <= floor_level + 1 || fill.levels.bottom >= shape.clearance_top
                }),
            "expanded tunnel air cannot acquire liquid fill"
        );
    }
    assert_tunnel_crystals_clear_named_targets(world, floor_level);
    let mut crystals = 0;
    for light in world.lights.values() {
        let Some(PlannedLightPresentation::CaveCrystal(presentation)) = light.presentation else {
            continue;
        };
        crystals += 1;
        assert_eq!(light.origin.level, floor_level);
        for coord in light.origin.coord.within_radius(1) {
            assert!(
                !passage.contains(&coord),
                "the complete crystal body enters the passage"
            );
            assert!(interior.floors.contains(&TilePos::new(coord, floor_level)));
            for level in floor_level + 1..=floor_level + presentation.kind.height() {
                assert!(
                    solid_material_at(&world.volume, TilePos::new(coord, level)).is_none(),
                    "the final crystal body clips a solid voxel"
                );
            }
        }
    }
    assert!(crystals > 0);
}

/// Hub centerline entries are one exact target per Ordinary schematic cell;
/// hub surfaces also include connector footing and are a different contract.
fn assert_tunnel_crystals_clear_named_targets(world: &GeneratedWorldPlan, floor_level: Level) {
    let hubs = world
        .features
        .protected_routes
        .get("grand_v3.ordinary_hubs")
        .expect("the final ordinary hub targets remain published");
    let mut targets = hubs
        .centerline
        .iter()
        .enumerate()
        .filter(|(_, target)| target.level == floor_level)
        .map(|(index, target)| (format!("ordinary_hubs target {index}"), *target))
        .collect::<Vec<_>>();
    targets.extend(
        world
            .anchors
            .iter()
            .filter(|(_, target)| target.level == floor_level)
            .map(|(name, target)| (format!("anchor {name}"), *target)),
    );
    let mut overlaps = Vec::new();
    for (id, light) in &world.lights {
        if !matches!(
            light.presentation,
            Some(PlannedLightPresentation::CaveCrystal(_))
        ) || light.origin.level != floor_level
        {
            continue;
        }
        let body = light.origin.coord.within_radius(1);
        for (name, target) in &targets {
            if body.contains(&target.coord) {
                overlaps.push(format!(
                    "light {id:?} at {:?} overlaps {name} at {target:?}",
                    light.origin
                ));
            }
        }
    }
    assert!(
        overlaps.is_empty(),
        "final cave crystal bodies overlap exact floor-level hub targets or anchors: {overlaps:#?}"
    );
}
