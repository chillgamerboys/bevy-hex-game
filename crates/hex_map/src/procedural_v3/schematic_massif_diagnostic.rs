//! Test-only observation of existing Massif fields and final terrain authority.
//! No writer receives a mutable world reference and no terrain is changed.

#![cfg(test)]

use super::*;
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

const OUTPUT_ENV: &str = "HEX_MASSIF_SHOULDER_DIAGNOSTIC_DIR";

thread_local! {
    static RESOLVE_CAPTURE: RefCell<Option<Option<[Level; 6]>>> = const { RefCell::new(None) };
    static GENERATION_INDEX: Cell<u32> = const { Cell::new(0) };
}

pub(in crate::procedural_v3) fn enabled() -> bool {
    std::env::var_os(OUTPUT_ENV).is_some()
}

pub(in crate::procedural_v3) fn capture_resolve(
    resolve: impl FnOnce() -> Level,
) -> (Level, [Level; 6]) {
    RESOLVE_CAPTURE.with(|slot| {
        assert!(
            slot.replace(Some(None)).is_none(),
            "nested Massif diagnostic"
        );
    });
    let result = resolve();
    let components = RESOLVE_CAPTURE.with(|slot| {
        slot.take()
            .expect("Massif diagnostic was armed")
            .expect("Massif resolve reached its final body cap")
    });
    (result, components)
}

pub(in crate::procedural_v3) fn observe_resolve(components: [Level; 6]) {
    RESOLVE_CAPTURE.with(|slot| {
        if let Some(destination) = slot.borrow_mut().as_mut() {
            assert!(
                destination.replace(components).is_none(),
                "duplicate Massif resolve observation"
            );
        }
    });
}

pub(in crate::procedural_v3) fn coords(
    values: impl IntoIterator<Item = HexCoord>,
) -> Vec<[i32; 2]> {
    values
        .into_iter()
        .map(|coord| [coord.x(), coord.y()])
        .collect()
}

pub(in crate::procedural_v3) fn levels(
    values: impl IntoIterator<Item = (HexCoord, Level)>,
) -> Vec<[i32; 3]> {
    values
        .into_iter()
        .map(|(coord, level)| [coord.x(), coord.y(), level])
        .collect()
}

pub(in crate::procedural_v3) fn write(stage: &str, value: &Value) {
    let root =
        PathBuf::from(std::env::var_os(OUTPUT_ENV).expect("explicit diagnostic output path"));
    assert!(
        root.is_absolute(),
        "diagnostic output must use an absolute root-selected path"
    );
    let thread = std::thread::current();
    let test = thread
        .name()
        .expect("diagnostic runs under a named lib test");
    let file_stem = test
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    std::fs::create_dir_all(&root).expect("create explicit diagnostic output directory");
    let sequence = GENERATION_INDEX.with(|index| {
        if stage == "profile" {
            index.set(
                index
                    .get()
                    .checked_add(1)
                    .expect("bounded diagnostic generation count"),
            );
        }
        assert!(
            index.get() > 0,
            "final diagnostic follows its observed foundation profile"
        );
        index.get()
    });
    let path = root.join(format!("{file_stem}.generation-{sequence:03}.{stage}.json"));
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|error| {
            panic!("create fresh Massif diagnostic {}: {error}", path.display())
        });
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, value).expect("write complete Massif diagnostic JSON");
    std::io::Write::flush(&mut writer).expect("flush complete Massif diagnostic JSON");
}

pub(super) struct FinalContext<'a> {
    pub(super) stage: &'a str,
    pub(super) plan: &'a SchematicPlanV1,
    pub(super) world: &'a GeneratedWorldPlan,
    pub(super) art_catalog: &'a RuntimeArtCatalog,
    pub(super) visual: &'a super::super::schematic_highlands::MassifVisualAuthority,
    pub(super) crest: &'a MassifCrestAuthority,
    pub(super) crystal: &'a super::super::schematic_highlands::CrystalMantleAuthority,
    pub(super) crystal_mask: &'a BTreeSet<HexCoord>,
    pub(super) peaks: &'a super::super::schematic_highlands::PeakRidgeAuthority,
    pub(super) hydrology: &'a HydrologyCompilation,
    pub(super) surface_exclusion: &'a BTreeSet<HexCoord>,
    pub(super) connector_exclusion: &'a BTreeSet<HexCoord>,
    pub(super) shared_minimums: &'a BTreeMap<HexCoord, Level>,
    pub(super) shared_maximums: &'a BTreeMap<HexCoord, Level>,
    pub(super) taper: &'a BTreeSet<HexCoord>,
    pub(super) tunnel: &'a TunnelOverburdenAuthority,
    pub(super) scenic_cliff_edges: &'a BTreeSet<(TilePos, TilePos)>,
}

pub(super) fn record_final(context: FinalContext<'_>) {
    if !enabled() {
        return;
    }
    let FinalContext {
        stage,
        plan,
        world,
        art_catalog,
        visual,
        crest,
        crystal,
        crystal_mask,
        peaks,
        hydrology,
        surface_exclusion,
        connector_exclusion,
        shared_minimums,
        shared_maximums,
        taper,
        tunnel,
        scenic_cliff_edges,
    } = context;
    let domain = visual
        .visual_mask
        .iter()
        .copied()
        .flat_map(|coord| std::iter::once(coord).chain(coord.neighbors()))
        .filter(|coord| world.volume.mask.contains(coord))
        .collect::<BTreeSet<_>>();
    let clipped_coords = |values: BTreeSet<HexCoord>| {
        coords(values.into_iter().filter(|coord| domain.contains(coord)))
    };
    let clipped_levels = |values: &BTreeMap<HexCoord, Level>| {
        levels(
            values
                .iter()
                .filter(|(coord, _)| domain.contains(coord))
                .map(|(coord, level)| (*coord, *level)),
        )
    };
    let columns = domain.iter().map(|coord| {
        let column = world.volume.columns.get(coord).expect("diagnostic domain retains its source column");
        let (surface, metadata) = world.volume.top_surface_at_coord(*coord)
            .expect("diagnostic domain retains its top surface");
        let elements = column.elements.iter().map(|element| match element {
            VolumeElement::Solid(mass) => json!(["solid", mass.levels.bottom, mass.levels.top,
                format!("{:?}", mass.material), mass.cutaway_for.map(|id| id.0)]),
            VolumeElement::Fill(fill) => json!(["fill", fill.levels.bottom, fill.levels.top,
                format!("{:?}", fill.material), null]),
        }).collect::<Vec<_>>();
        let surfaces = world.volume.surfaces_at_coord(*coord).map(|(position, metadata)| {
            json!([position.level, format!("{:?}", metadata.access), metadata.interior.map(|id| id.0)])
        }).collect::<Vec<_>>();
        json!({"coord": [coord.x(), coord.y()], "top": surface.level,
            "access": format!("{:?}", metadata.access),
            "scenic": metadata.access == SurfaceAccess::SpecialMovement(SCENIC_MOVEMENT_REGION),
            "interior": metadata.interior.map(|id| id.0), "surfaces": surfaces, "elements": elements})
    }).collect::<Vec<_>>();
    let mut masks = BTreeMap::new();
    masks.insert(
        "surface_exclusion",
        clipped_coords(surface_exclusion.clone()),
    );
    masks.insert(
        "connector_exclusion",
        clipped_coords(connector_exclusion.clone()),
    );
    masks.insert("crest", clipped_coords(crest.coords().collect()));
    masks.insert("taper", clipped_coords(taper.clone()));
    masks.insert(
        "visual_connectors",
        clipped_coords(visual.connector_owners.keys().copied().collect()),
    );
    masks.insert("crystal_site", clipped_coords(crystal_mask.clone()));
    masks.insert(
        "crystal_screen",
        clipped_coords(crystal.route_exclusion.clone()),
    );
    masks.insert(
        "crystal_openings",
        clipped_coords(crystal.opening_clearance.clone()),
    );
    masks.insert(
        "crystal_skin",
        clipped_coords(crystal.natural_shell_skin.clone()),
    );
    masks.insert(
        "crystal_exposed_openings",
        clipped_coords(crystal.exposed_shell_openings.clone()),
    );
    masks.insert(
        "crystal_exact_caps",
        clipped_coords(
            crystal
                .expected_uplift_caps
                .as_ref()
                .expect("final Crystal authority has sealed exact caps")
                .keys()
                .copied()
                .collect(),
        ),
    );
    masks.insert(
        "crystal_apron",
        clipped_coords(crystal.shell_concealment_floors.keys().copied().collect()),
    );
    masks.insert(
        "crystal_sector_pins",
        clipped_coords(
            crystal
                .sector_pins
                .values()
                .map(|(coord, _)| *coord)
                .collect(),
        ),
    );
    masks.insert(
        "peak_profile",
        clipped_coords(
            peaks
                .components
                .iter()
                .flat_map(|component| {
                    component
                        .expected_ridge_profile
                        .keys()
                        .copied()
                        .chain(component.feather_owners.keys().copied())
                })
                .collect(),
        ),
    );
    masks.insert("water", clipped_coords(hydrology.water_coords.clone()));
    masks.insert(
        "water_banks",
        clipped_coords(
            recessed_water_bank_minimums(&world.volume)
                .keys()
                .copied()
                .collect(),
        ),
    );
    masks.insert(
        "waterfall",
        clipped_coords(
            hydrology
                .waterfall_cliff
                .gorge_surfaces
                .iter()
                .chain(&hydrology.waterfall_cliff.feather_surfaces)
                .chain(&hydrology.waterfall_cliff.plunge_clearance_surfaces)
                .chain(hydrology.waterfall_cliff.cascade_rows.iter().flatten())
                .map(|surface| surface.coord)
                .collect(),
        ),
    );
    masks.insert(
        "tunnel_overburden",
        clipped_coords(tunnel.columns.keys().copied().collect()),
    );
    masks.insert(
        "scenic_cliff_endpoints",
        clipped_coords(
            scenic_cliff_edges
                .iter()
                .flat_map(|(a, b)| [a.coord, b.coord])
                .collect(),
        ),
    );
    masks.insert(
        "structures",
        clipped_coords(
            world
                .structures
                .by_id
                .values()
                .flat_map(|structure| structure.voxels.iter().map(|voxel| voxel.coord))
                .collect(),
        ),
    );
    masks.insert(
        "blockers",
        clipped_coords(
            world
                .blockers
                .iter()
                .map(|position| position.coord)
                .collect(),
        ),
    );
    masks.insert(
        "clearings",
        clipped_coords(
            world
                .features
                .clearings
                .values()
                .flat_map(|clearing| clearing.surfaces.iter().map(|surface| surface.coord))
                .collect(),
        ),
    );
    masks.insert(
        "anchors",
        clipped_coords(
            world
                .anchors
                .values()
                .chain(world.observation_anchors.values())
                .map(|position| position.coord)
                .collect(),
        ),
    );
    masks.insert(
        "lights",
        clipped_coords(
            world
                .lights
                .values()
                .map(|light| light.origin.coord)
                .collect(),
        ),
    );
    let temperate = TemperateVegetationSet::resolve(art_catalog, "Massif diagnostic")
        .expect("diagnostic resolves existing temperate assets");
    let snowy = SnowyVegetationSet::resolve(art_catalog, "Massif diagnostic")
        .expect("diagnostic resolves existing snowy assets");
    let objects = [
        &temperate.small_broadleaf,
        &temperate.tall_narrow,
        &temperate.old_growth,
        &temperate.grass_tuft,
        &snowy.small_broadleaf,
        &snowy.tall_narrow,
        &snowy.old_growth,
        &snowy.grass_tuft,
    ];
    let mut visual_features = BTreeSet::new();
    for feature in world.features.by_id.values() {
        // All existing Grand tree variants project at most two hexes laterally.
        // The three-cell proximity test includes every potentially intersecting volume.
        if !feature
            .root
            .coord
            .within_radius(3)
            .iter()
            .any(|coord| domain.contains(coord))
        {
            continue;
        }
        let object = objects
            .iter()
            .find(|object| object.id == feature.object_id)
            .expect("Grand diagnostic feature uses an existing surface vegetation variant");
        let projected = object
            .project_visual_volume(feature.root, feature.rotation)
            .expect("existing vegetation retains its full visual projection");
        visual_features.extend(projected.cells.into_iter().map(|voxel| voxel.coord));
        visual_features.insert(feature.root.coord);
        visual_features.extend(feature.blocker_footprint.iter().map(|voxel| voxel.coord));
    }
    masks.insert("feature_visuals", clipped_coords(visual_features));
    let routes = world.features.protected_routes.iter().map(|(name, route)| (name.clone(), json!({
        "centerline": levels(route.centerline.iter().filter(|p| domain.contains(&p.coord)).map(|p| (p.coord,p.level))),
        "surfaces": levels(route.surfaces.iter().filter(|p| domain.contains(&p.coord)).map(|p| (p.coord,p.level))),
        "total_centerline": route.centerline.len(), "total_surfaces": route.surfaces.len(),
    }))).collect::<BTreeMap<_,_>>();
    write(
        stage,
        &json!({"schema": 1, "stage": stage, "seed": plan.provenance.world_seed,
            "crest": [crest.crest.coord.x(), crest.crest.coord.y(), crest.crest.level],
            "domain": coords(domain.iter().copied()), "semantic_mask": coords(visual.semantic_owner_mask.iter().copied()),
            "columns": columns, "fixed_masks": masks, "routes": routes,
            "shared_minimums": clipped_levels(shared_minimums), "shared_maximums": clipped_levels(shared_maximums),
            "shell_floors": clipped_levels(&crystal.shell_concealment_floors),
            "shell_ceilings": clipped_levels(&crystal.shell_concealment_ceilings),
            "forced_low_frozen": clipped_levels(&crystal.forced_low_frozen_halo),
            "forced_low_exit": clipped_levels(&crystal.forced_low_exit_blend),
        }),
    );
}
