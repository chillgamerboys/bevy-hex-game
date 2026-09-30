//! Validation of immutable presentation facts against the admitted finite package.
use super::*;
use hex_world_contracts::WorldManifest;

struct Bounds {
    radius: u32,
    max_height: f32,
    extent: Vec2,
    coverage_margin: Vec2,
}

fn bounds(overview: &NorthernOverview, manifest: &WorldManifest) -> Result<Bounds, String> {
    let [region] = manifest.regions.as_slice() else {
        return Err("Streamed overview requires one finite region".into());
    };
    if region.origin != WorldHex::new(0, 0) || region.radius != overview.radius {
        return Err("Streamed overview region differs from its package".into());
    }
    if manifest.world_id != "grand-v4" {
        if overview.radius != 700 || overview.level_bounds != [0, 1400] {
            return Err("Northern overview differs from its physical profile".into());
        }
        return Ok(Bounds {
            radius: 700,
            max_height: 490.0,
            extent: Vec2::new(1216.0, 1056.0),
            coverage_margin: Vec2::new(3.0, 5.0),
        });
    }
    // These conversion limits protect exact local coordinates and the existing
    // signed-16-bit anchor-height admission. They are representation limits,
    // independent of a particular Grand composition or its highest mountain.
    let radius = u16::try_from(region.radius)
        .map_err(|error| format!("Grand region exceeds its physical coordinate range: {error}"))?;
    let [minimum, maximum] = overview.level_bounds;
    let maximum = i16::try_from(maximum)
        .map_err(|error| format!("Grand height exceeds its physical level range: {error}"))?;
    if radius == 0
        || radius > 4096
        || minimum != 0
        || maximum <= 0
        || f32::from(maximum) * overview.level_height + overview.vertical_offset > 4096.0
    {
        return Err("Grand overview has invalid finite bounds".into());
    }
    let radius_f32 = f32::from(radius);
    Ok(Bounds {
        radius: u32::from(radius),
        // The upper voxel bound is inclusive; its exposed upper face is one
        // level above its index under the retained physical lattice profile.
        max_height: f32::from(maximum) * overview.level_height + overview.vertical_offset,
        extent: Vec2::new((radius_f32 + 0.5) * 3.0_f32.sqrt(), radius_f32 * 1.5 + 1.0),
        coverage_margin: Vec2::ZERO,
    })
}

fn point_in_bounds(point: [f32; 3], bounds: &Bounds) -> bool {
    let point = Vec3::from_array(point);
    point.is_finite()
        && (0.0..=bounds.max_height).contains(&point.y)
        && point.x.abs() <= bounds.extent.x
        && point.z.abs() <= bounds.extent.y
        && world_hex(HexCoord::from_world(point))
            .checked_distance(WorldHex::new(0, 0))
            .is_ok_and(|distance| distance <= u64::from(bounds.radius))
}

pub(super) fn validate_overview(
    overview: &NorthernOverview,
    manifest: &WorldManifest,
) -> Result<(), String> {
    if overview.version != 1
        || overview.world_id != manifest.world_id
        || overview.package_fingerprint != manifest.fingerprint
        || overview.source_fingerprint != manifest.source_fingerprint
        || overview.materials != manifest.materials
        || overview.hex_radius.to_bits() != 1.0_f32.to_bits()
        || overview.level_height.to_bits() != 0.35_f32.to_bits()
        || overview.vertical_offset.to_bits() != 0.35_f32.to_bits()
        || overview.sea_level.to_bits() != 140.0_f32.to_bits()
    {
        return Err("Streamed overview identity, palette or physical pitch differs from its package/profile".into());
    }
    let bounds = bounds(overview, manifest)?;
    if overview.sea_level > bounds.max_height {
        return Err("Streamed ocean lies outside the finite height bounds".into());
    }
    validate_cameras(overview, &bounds)?;
    use hex_schematic::v4::northern::terrain_surface::TERRAIN_SURFACE_KEY;
    match (
        &overview.terrain_surface,
        manifest.presentation_fingerprints.get(TERRAIN_SURFACE_KEY),
    ) {
        (None, None) => {}
        (Some(surface), Some(expected)) if manifest.world_id == "grand-v4" => {
            let builder = hex_schematic::v4::northern::terrain_surface::SurfaceBuilder::new(
                surface,
                overview.radius,
                overview.level_bounds,
                &overview.materials,
                f64::from(overview.level_height),
            )
            .map_err(|error| error.to_string())?;
            for chunk in &surface.chunks {
                if let Some(patch) = &chunk.surface {
                    builder
                        .certify_patch(chunk.coordinate, patch)
                        .map_err(|error| error.to_string())?;
                }
            }
            if surface.fingerprint().map_err(|error| error.to_string())? != *expected {
                return Err("Grand terrain surface differs from its sealed package".into());
            }
        }
        _ => return Err("Grand terrain surface is missing or has no package binding".into()),
    }
    super::grand_water::validate(overview)?;
    if let Some(forest) = &overview.forest {
        if manifest.world_id != "grand-v4" {
            return Err("Forest companion is only supported for Grand".into());
        }
        forest
            .validate_catalog_in_bounds(&manifest.features, overview.radius, overview.level_bounds)
            .map_err(|error| error.to_string())?;
    }
    if let Some(cover) = &overview.ground_cover {
        if manifest.world_id != "grand-v4" {
            return Err("Ground cover companion is only supported for Grand".into());
        }
        cover
            .validate_in_bounds(overview.radius, overview.level_bounds)
            .map_err(|error| error.to_string())?;
    }
    let samples = overview
        .width
        .checked_mul(overview.height)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or("Streamed overview dimensions overflow")?;
    if overview.width < 2
        || overview.height < 2
        || samples > 1_048_576
        || overview.bed_heights.len() != samples
        || overview.surface_materials.len() != samples
        || !overview.spacing.is_finite()
        || !(0.5..=8.0).contains(&overview.spacing)
        || !overview.origin_xz.iter().all(|value| value.is_finite())
        || !point_in_bounds(overview.player_spawn, &bounds)
        || overview.anchors.values().any(|point| {
            if manifest.world_id == "grand-v4" {
                !point_in_bounds(*point, &bounds)
            } else {
                !point.iter().all(|value| value.is_finite())
            }
        })
        || overview
            .bed_heights
            .iter()
            .any(|height| !height.is_finite() || !(0.0..=bounds.max_height).contains(height))
        || overview
            .surface_materials
            .iter()
            .any(|index| overview.materials.get(usize::from(*index)).is_none())
    {
        return Err(
            "Streamed overview grid, palette index or observation position is invalid".into(),
        );
    }
    let width = u16::try_from(overview.width - 1)
        .map_err(|error| format!("Streamed overview width exceeds its grid bound: {error}"))?;
    let height = u16::try_from(overview.height - 1)
        .map_err(|error| format!("Streamed overview height exceeds its grid bound: {error}"))?;
    let start = Vec2::from_array(overview.origin_xz);
    let end = start + Vec2::new(f32::from(width), f32::from(height)) * overview.spacing;
    if !end.is_finite()
        || (start.cmpgt(-bounds.extent + bounds.coverage_margin)).any()
        || (end.cmplt(bounds.extent - bounds.coverage_margin)).any()
    {
        return Err("Streamed overview does not cover the finite region".into());
    }
    Ok(())
}

fn validate_cameras(overview: &NorthernOverview, bounds: &Bounds) -> Result<(), String> {
    if overview.review_cameras.len() > 256 {
        return Err("Streamed overview has too many authored cameras".into());
    }
    for (name, camera) in &overview.review_cameras {
        let eye = Vec3::from_array(camera.eye);
        let target = Vec3::from_array(camera.target);
        let direction = target - eye;
        if name.is_empty()
            || name.len() > 128
            || !eye.is_finite()
            || !target.is_finite()
            || !direction.is_finite()
            || !direction.length_squared().is_finite()
            || direction.length_squared() <= 0.000_001
            || !point_in_bounds(camera.interest, bounds)
            || camera
                .orthographic_span
                .is_some_and(|span| !span.is_finite() || span <= 0.0 || camera.ground)
        {
            return Err(format!("Invalid authored streamed camera: {name}"));
        }
    }
    Ok(())
}

pub(super) fn validate_biome_rows(rows: &[(i64, i64, i64)], radius: u32) -> Result<(), String> {
    if rows.windows(2).any(|pair| pair.first() >= pair.get(1))
        || rows.iter().any(|&(r, start, end)| {
            start > end
                || [start, end].into_iter().any(|q| {
                    !WorldHex::new(q, r)
                        .checked_distance(WorldHex::new(0, 0))
                        .is_ok_and(|distance| distance <= u64::from(radius))
                })
        })
    {
        return Err("Grand biome rows are invalid or outside the package region".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_world_contracts::{MaterialSpec, RegionDescriptor};

    fn fixture(grand: bool) -> (NorthernOverview, WorldManifest) {
        let (id, radius, maximum, origin, width, height) = if grand {
            ("grand-v4", 1800, 2400, [-3120.0, -2704.0], 781_u32, 677_u32)
        } else {
            (
                "northern-archipelago",
                700,
                1400,
                [-1216.0, -1056.0],
                305,
                265,
            )
        };
        let materials = vec![MaterialSpec {
            id: "stone".into(),
            solid: true,
            diggable: true,
            color: [120, 120, 120, 255],
        }];
        let samples = usize::try_from(width * height).unwrap();
        let overview = NorthernOverview {
            terrain_surface: None,
            version: 1,
            source_fingerprint: 11,
            package_fingerprint: 22,
            world_id: id.into(),
            hex_radius: 1.0,
            level_height: 0.35,
            vertical_offset: 0.35,
            radius,
            level_bounds: [0, maximum],
            sea_level: 140.0,
            origin_xz: origin,
            spacing: 8.0,
            width,
            height,
            bed_heights: vec![105.0; samples],
            surface_materials: vec![0; samples],
            materials: materials.clone(),
            player_spawn: [0.0, 140.35, 0.0],
            anchors: BTreeMap::new(),
            islands: Vec::new(),
            tree_count: 0,
            forest: None,
            ground_cover: None,
            inland_water: None,
            review_cameras: BTreeMap::new(),
            building_count: 0,
        };
        // This seam receives an already-admitted manifest. Chunk completeness
        // and payload hashes belong to FileChunkSource, not overview validation.
        let manifest = WorldManifest {
            presentation_fingerprints: Default::default(),
            schema_version: hex_world_contracts::SCHEMA_VERSION,
            world_id: id.into(),
            compiler_version: "admission-fixture".into(),
            source_fingerprint: 11,
            materials,
            regions: vec![RegionDescriptor {
                id: "region".into(),
                origin: WorldHex::new(0, 0),
                radius,
                source_fingerprint: 11,
            }],
            chunks: Vec::new(),
            boundaries: Vec::new(),
            summary: Vec::new(),
            features: Vec::new(),
            fingerprint: 22,
        };
        (overview, manifest)
    }

    #[test]
    fn terrain_surface_requires_the_exact_manifest_content_binding() {
        use hex_schematic::v4::northern::terrain_surface::{
            TerrainSurfaceOverview, TERRAIN_SURFACE_KEY,
        };
        let (mut overview, mut manifest) = fixture(true);
        let surface = TerrainSurfaceOverview {
            halo: Vec::new(),
            version: 1,
            tolerance: 2.0,
            profiles: Vec::new(),
            chunks: Vec::new(),
        };
        overview.terrain_surface = Some(surface.clone());
        assert!(
            validate_overview(&overview, &manifest).is_err(),
            "unbound payload"
        );
        manifest
            .presentation_fingerprints
            .insert(TERRAIN_SURFACE_KEY.into(), surface.fingerprint().unwrap());
        validate_overview(&overview, &manifest).unwrap();
        overview.terrain_surface.as_mut().unwrap().tolerance = 1.0;
        assert!(
            validate_overview(&overview, &manifest).is_err(),
            "modified content"
        );
        overview.terrain_surface = None;
        assert!(
            validate_overview(&overview, &manifest).is_err(),
            "missing payload"
        );
        overview.terrain_surface = Some(surface);
        overview.terrain_surface.as_mut().unwrap().version = 2;
        assert!(
            validate_overview(&overview, &manifest).is_err(),
            "unsupported content"
        );
    }

    #[test]
    fn grand_admits_package_envelope_and_heights_beyond_the_previous_composition() {
        let (mut overview, manifest) = fixture(true);
        overview.player_spawn = [-2200.0, 630.0, 0.0];
        overview
            .anchors
            .insert("high-summit".into(), [0.0, 800.0, 0.0]);
        *overview.bed_heights.first_mut().unwrap() = 800.0;
        validate_overview(&overview, &manifest).unwrap();
        validate_biome_rows(&[(0, -1200, 1200)], overview.radius).unwrap();
    }

    #[test]
    fn grand_rejects_mismatched_or_unrepresentable_finite_bounds() {
        let (mut overview, manifest) = fixture(true);
        overview.radius += 1;
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.radius -= 1;
        overview.level_bounds = [0, i32::from(i16::MAX) + 1];
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.level_bounds = [-1, 2400];
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.level_bounds = [0, 398];
        assert!(validate_overview(&overview, &manifest).is_err());
    }

    #[test]
    fn grand_rejects_clipped_grid_and_outside_or_overheight_anchors() {
        let (mut overview, manifest) = fixture(true);
        let original = overview.origin_xz;
        overview.origin_xz = [-3000.0, -2704.0];
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.origin_xz = original;
        overview
            .anchors
            .insert("outside".into(), [4000.0, 140.0, 0.0]);
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.anchors.insert("outside".into(), [0.0, 900.0, 0.0]);
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.anchors.clear();
        overview.level_height = 0.7;
        assert!(validate_overview(&overview, &manifest).is_err());
    }

    #[test]
    fn biome_rows_respect_the_whole_hex_disk_and_canonical_order() {
        validate_biome_rows(&[(0, -1200, 1200), (900, -900, 900)], 1800).unwrap();
        assert!(validate_biome_rows(&[(1000, 900, 1000)], 1800).is_err());
        assert!(validate_biome_rows(&[(0, 2, 1)], 1800).is_err());
        assert!(validate_biome_rows(&[(0, 0, 1), (0, 0, 1)], 1800).is_err());
        assert!(validate_biome_rows(&[(i64::MAX, 0, i64::MAX)], 1800).is_err());
    }

    #[test]
    fn northern_keeps_its_existing_profile_and_identity_checks() {
        let (mut overview, mut manifest) = fixture(false);
        validate_overview(&overview, &manifest).unwrap();
        overview.radius = 1800;
        manifest.regions.first_mut().unwrap().radius = 1800;
        assert!(validate_overview(&overview, &manifest).is_err());
        overview.radius = 700;
        manifest.regions.first_mut().unwrap().radius = 700;
        overview.package_fingerprint ^= 1;
        assert!(validate_overview(&overview, &manifest).is_err());
    }

    #[test]
    fn authored_cameras_allow_distant_framing_but_require_valid_local_interest() {
        use hex_schematic::v4::northern::NorthernReviewCamera;
        let (mut overview, manifest) = fixture(true);
        let camera = NorthernReviewCamera {
            eye: [5000.0, 3000.0, 5000.0],
            target: [0.0, 200.0, 0.0],
            interest: overview.player_spawn,
            orthographic_span: Some(5000.0),
            ground: false,
        };
        overview
            .review_cameras
            .insert("whole-world".into(), camera.clone());
        validate_overview(&overview, &manifest).unwrap();
        for bad in [
            NorthernReviewCamera {
                interest: [5000.0, 140.0, 0.0],
                ..camera.clone()
            },
            NorthernReviewCamera {
                target: camera.eye,
                ..camera.clone()
            },
            NorthernReviewCamera {
                ground: true,
                ..camera.clone()
            },
            NorthernReviewCamera {
                orthographic_span: Some(f32::NAN),
                ..camera.clone()
            },
            NorthernReviewCamera {
                eye: [f32::INFINITY, 0.0, 0.0],
                ..camera.clone()
            },
        ] {
            overview.review_cameras.insert("whole-world".into(), bad);
            assert!(validate_overview(&overview, &manifest).is_err());
        }
    }
}
