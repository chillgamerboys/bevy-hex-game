use super::*;

#[test]
fn elongated_pockets_keep_adventure_and_ordinary_deployment_and_fit_four_cells() {
    let content = load_content().expect("accepted content");
    for map in [ArenaMap::Duel, ArenaMap::Fort] {
        let recipe = build(
            ArenaSelection { map, ..default() },
            content.materials,
            &content.substances,
            &content.art,
        )
        .expect("accepted map has finite elongated deployment");
        let view = &recipe.view;
        let normal = view.battle_deployment.as_ref().expect("ordinary pockets");
        let elongated = view
            .elongated_deployment
            .as_ref()
            .expect("elongated pockets");
        assert_eq!(
            view.spawns,
            [
                *view.anchors.get("party_start").expect("start"),
                *view.anchors.get("hostile_start").expect("hostile")
            ]
        );
        for (normal, long) in normal.iter().zip(elongated) {
            assert_eq!(normal.surfaces.len(), 7);
            assert_eq!(normal.preferred, long.preferred);
            assert!(long.surfaces.len() <= 19);
            assert!(contains_straight_run(long, 4));
            assert!(!contains_straight_run(long, 6));
            assert!(long
                .surfaces
                .iter()
                .all(|surface| { deployment_surface_open(view, recipe.geometry, *surface) }));
        }
        let [left, right] = elongated;
        assert!(left.surfaces.is_disjoint(&right.surfaces));
    }
    let seven = build(
        ArenaSelection {
            map: ArenaMap::SevenRegions,
            ..default()
        },
        content.materials,
        &content.substances,
        &content.art,
    )
    .expect("accepted seven regions");
    assert!(seven.view.battle_deployment.is_none());
    assert!(seven.view.elongated_deployment.is_none());
}

#[test]
fn every_authored_crystal_cell_is_published_without_changing_query_masks() {
    let content = load_content().expect("accepted content");
    let object_id = hex_assets::ObjectAssetId::new("prop/crystal-spire").expect("stable id");
    let object = content.art.object(&object_id).expect("spire catalog id");
    let origin = TilePos::new(HexCoord::from_axial(3, -2), 10);
    let instance =
        ObjectInstance::new(object_id.clone(), origin, 0.4, default()).expect("finite instance");
    let mut view = ArenaTerrainView::default();
    project_instance(&mut view, &content.art, &instance, false).expect("static projection");
    assert_eq!(view.static_spans.len(), object.placements.len());
    assert!(
        view.static_spans.iter().any(|span| {
            !span.blocks_movement && !span.blocks_projectiles && !span.blocks_sight
        }),
        "fixture must contain transparent authored occupancy"
    );
    for span in &view.static_spans {
        if !span.blocks_movement && !span.blocks_projectiles && !span.blocks_sight {
            assert!(!view
                .edit_protected
                .get(&span.bottom.coord)
                .is_some_and(|intervals| {
                    intervals
                        .iter()
                        .any(|(low, high)| (*low..=*high).contains(&span.bottom.level))
                }));
        }
    }
}
