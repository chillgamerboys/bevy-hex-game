//! Canonical byte encoding and independent fingerprint domains for V3.
//!
//! Fingerprints are schema-driven: callers write fields in their documented
//! semantic order and sort unordered collections before writing them. The
//! encoder owns the byte representation of primitives, while the three final
//! domains prevent equivalent bytes at different pipeline stages from sharing
//! an identity.

use hex_core::{HexCoord, IlluminationLevel, MapViewHint, TilePos};
use xxhash_rust::xxh3::xxh3_64;

use crate::settings::{
    EdgeLiquidSettings, MacroAccessSettings, MacroAxisSettings, MacroBoundarySideSettings,
    MacroHeadwaterSettings, MacroLayoutSettings, MacroLiquidConnectionSettings,
    MacroSpanningFeatureSettings, NamedOverlaySettings, PatchEdgeContractSettings,
    PatchEdgesSettings, PatchMaskSettings, PatchSpec, ProceduralV3Settings, Ring19BoundarySide,
    Ring19RegionSettings, SharedEdgeSettings, V3EnvironmentSettings, V3LayoutSettings,
    V3OverlaySettings, V3RecipeSettings, V3Ring19ProfileSettings, V3Ring19Settings,
    V3Ring7Settings, V3SchematicLayoutSettings, V3SchematicTemplate, V3SchematicTerrainProfile,
};

use super::layout::{
    HexSide, LayoutKind, PatchId, ResolvedEdgeContract, ResolvedEdgeReference, ResolvedLayoutPlan,
    ResolvedLiquidElevation, ResolvedLiquidPort, ResolvedPort,
};
use super::liquid::{LiquidFlowState, LiquidPlan};
use super::volume::{
    FillMaterialRole, SolidMaterialRole, SurfaceAccess, SurfaceMetadata, VolumeElement, VolumePlan,
};
#[cfg(test)]
use super::world::CaveCrystalPresentation;
use super::world::{
    CaveCrystalKind, CaveCrystalSiteKind, FeatureKind, FeaturePlan, GeneratedWorldPlan,
    PlannedGameplayLight, PlannedInterior, PlannedLightPresentation, PlannedStructure,
    StructureKind,
};

const SETTINGS_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/settings";
const SEMANTIC_PLAN_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/semantic-plan";
const MATERIALIZED_WORLD_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/materialized-world";
const MACRO_EXTENSION_TAG: u8 = 128;
const MACRO_EXTENSION_VERSION: u8 = 1;
const RING19_EXTENSION_TAG: u8 = 129;
const RING19_EXTENSION_VERSION: u8 = 1;

/// Canonical V3 fingerprint payload.
///
/// The encoder intentionally has no support for maps or sets. Callers must sort
/// those collections by their semantic keys, write the collection count, and
/// then write each entry. This makes ordering decisions visible at the schema
/// boundary instead of depending on a container's iteration implementation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct FingerprintEncoder {
    bytes: Vec<u8>,
}

impl FingerprintEncoder {
    /// Starts an empty canonical payload.
    #[must_use]
    pub(crate) const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Writes an enum or union variant tag.
    pub(crate) fn tag(&mut self, value: u8) {
        self.u8(value);
    }

    /// Writes a boolean as exactly zero or one.
    #[cfg(test)]
    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    /// Writes an unsigned byte.
    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Writes a little-endian unsigned 16-bit integer.
    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian unsigned 32-bit integer.
    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian unsigned 64-bit integer.
    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian signed 32-bit integer.
    pub(crate) fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a little-endian signed 64-bit integer.
    #[cfg(test)]
    pub(crate) fn i64(&mut self, value: i64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    /// Writes a finite 32-bit float by its exact IEEE-754 bits.
    ///
    /// NaN and infinity are rejected because neither is a valid generation
    /// setting or semantic value. Positive and negative zero retain their exact
    /// bits rather than being normalized.
    pub(crate) fn finite_f32(&mut self, value: f32) -> Result<(), String> {
        if !value.is_finite() {
            return Err(format!(
                "fingerprint values must be finite, received {value}"
            ));
        }
        self.u32(value.to_bits());
        Ok(())
    }

    /// Writes a byte slice preceded by its little-endian `u64` length.
    #[cfg(test)]
    pub(crate) fn bytes(&mut self, value: &[u8]) -> Result<(), String> {
        self.length(value.len(), "byte slice")?;
        self.bytes.extend_from_slice(value);
        Ok(())
    }

    /// Writes UTF-8 bytes preceded by their little-endian `u64` length.
    pub(crate) fn str(&mut self, value: &str) -> Result<(), String> {
        self.length(value.len(), "string")?;
        self.bytes.extend_from_slice(value.as_bytes());
        Ok(())
    }

    /// Writes all three cube components of an exact horizontal coordinate.
    pub(crate) fn hex_coord(&mut self, coord: HexCoord) {
        self.i32(coord.x());
        self.i32(coord.y());
        self.i32(coord.z());
    }

    /// Writes an exact stacked voxel position.
    pub(crate) fn tile_pos(&mut self, pos: TilePos) {
        self.hex_coord(pos.coord);
        self.i32(pos.level);
    }

    /// Writes a collection count as a little-endian `u64`.
    pub(crate) fn collection_count(&mut self, count: usize) -> Result<(), String> {
        self.length(count, "collection")
    }

    /// Finalizes this payload in the V3 settings domain.
    #[must_use]
    pub(crate) fn finish_settings(&self) -> u64 {
        fingerprint(SETTINGS_DOMAIN, &self.bytes)
    }

    /// Finalizes this payload in the V3 semantic-plan domain.
    #[must_use]
    pub(crate) fn finish_semantic_plan(&self) -> u64 {
        fingerprint(SEMANTIC_PLAN_DOMAIN, &self.bytes)
    }

    /// Finalizes this payload in the V3 materialized-world domain.
    #[must_use]
    pub(crate) fn finish_materialized_world(&self) -> u64 {
        fingerprint(MATERIALIZED_WORLD_DOMAIN, &self.bytes)
    }

    fn length(&mut self, length: usize, kind: &str) -> Result<(), String> {
        let encoded = u64::try_from(length).map_err(|error| {
            format!("{kind} length {length} exceeds the V3 fingerprint format: {error}")
        })?;
        self.u64(encoded);
        Ok(())
    }
}

/// Fingerprints every V3 setting which can affect generated output.
///
/// Ring patches use their fixed semantic slot order. Overlay names and explicit
/// mask coordinates are sorted before encoding, so authored list order cannot
/// perturb generation identity.
pub(crate) fn settings_fingerprint(
    grid_radius: u32,
    level_height: f32,
    settings: &ProceduralV3Settings,
) -> Result<u64, String> {
    let mut encoder = FingerprintEncoder::new();
    encoder.u32(3);
    encoder.u32(grid_radius);
    encoder.finite_f32(level_height)?;
    match &settings.layout {
        V3LayoutSettings::Single(patch) => {
            encoder.tag(0);
            encode_patch_settings(&mut encoder, patch)?;
        }
        V3LayoutSettings::Ring7(ring) => {
            encoder.tag(1);
            encode_ring_settings(&mut encoder, ring)?;
        }
        V3LayoutSettings::Ring19(ring) => {
            encoder.tag(2);
            encode_ring19_settings(&mut encoder, ring)?;
        }
        V3LayoutSettings::Macro(macro_layout) => {
            encoder.tag(3);
            encode_macro_settings(&mut encoder, macro_layout)?;
        }
        V3LayoutSettings::Schematic(schematic) => {
            encoder.tag(4);
            encode_schematic_settings(&mut encoder, schematic);
        }
    }
    Ok(encoder.finish_settings())
}

fn encode_schematic_settings(
    encoder: &mut FingerprintEncoder,
    settings: &V3SchematicLayoutSettings,
) {
    encoder.tag(match settings.template {
        V3SchematicTemplate::GrandV3 => 0,
    });
    encoder.u32(settings.template_revision);
    encoder.u32(settings.cell_pitch);
    match settings.terrain_profile {
        V3SchematicTerrainProfile::GrandV3BasicV1(profile) => {
            encoder.tag(0);
            for level in [
                profile.sea_level,
                profile.island_level,
                profile.beach_level,
                profile.shore_level,
                profile.valley_level,
                profile.plateau_level,
                profile.hill_level,
                profile.mountain_floor,
                profile.massif_floor,
                profile.high_core_level,
                profile.high_gradient_per_cell,
                profile.mountain_lake_level,
                profile.frozen_woods_level,
                profile.lake_island_min_level,
                profile.lake_island_max_level,
                profile.sharp_peak_bench_min,
                profile.sharp_peak_bench_max,
                profile.sharp_peak_min,
                profile.sharp_peak_max,
                profile.valley_lake_level,
                profile.crystal_base_level,
                profile.crystal_rise_levels,
            ] {
                encoder.i32(level);
            }
        }
    }
}

fn encode_macro_settings(
    encoder: &mut FingerprintEncoder,
    settings: &MacroLayoutSettings,
) -> Result<(), String> {
    encoder.u32(settings.macro_radius);
    encoder.u32(settings.approach_depth);
    encoder.collection_count(settings.instances.len())?;
    for instance in &settings.instances {
        encoder.str(&instance.name)?;
        let mut cells = instance
            .cells
            .iter()
            .map(|cell| (cell.x, cell.y, cell.z))
            .collect::<Vec<_>>();
        cells.sort_unstable();
        encoder.collection_count(cells.len())?;
        for (x, y, z) in cells {
            encoder.i32(x);
            encoder.i32(y);
            encoder.i32(z);
        }
        encoder.tag(environment_tag(instance.environment));
        encode_recipe_settings(encoder, &instance.recipe);
        encoder.u8(instance.rotation_turns);
        encoder.tag(match instance.access {
            MacroAccessSettings::Aquatic => 0,
            MacroAccessSettings::Land => 1,
            MacroAccessSettings::Scenic => 2,
        });
        encoder.i32(instance.elevation.low);
        encoder.i32(instance.elevation.high);
        encoder.tag(macro_axis_tag(instance.elevation.grade_axis));
    }
    let mut liquids = settings.liquid_connections.clone();
    liquids.sort_unstable();
    encoder.collection_count(liquids.len())?;
    for liquid in liquids {
        match liquid {
            MacroLiquidConnectionSettings::Standing {
                first_instance,
                second_instance,
                width,
                level,
            } => {
                encoder.tag(0);
                encoder.str(&first_instance)?;
                encoder.str(&second_instance)?;
                encoder.u32(width);
                encoder.i32(level);
            }
            MacroLiquidConnectionSettings::Directed {
                source_instance,
                sink_instance,
                width,
                level,
            } => {
                encoder.tag(1);
                encoder.str(&source_instance)?;
                encoder.str(&sink_instance)?;
                encoder.u32(width);
                encoder.i32(level);
            }
        }
    }
    let mut headwaters = settings.headwaters.clone();
    headwaters.sort_unstable();
    encoder.collection_count(headwaters.len())?;
    for headwater in headwaters {
        match headwater {
            MacroHeadwaterSettings::CaveFall {
                instance,
                source_level,
                overhang_depth,
            } => {
                encoder.tag(0);
                encoder.str(&instance)?;
                encoder.i32(source_level);
                encoder.u8(overhang_depth);
            }
            MacroHeadwaterSettings::RivuletConfluence {
                instance,
                source_level,
                branch_count,
            } => {
                encoder.tag(1);
                encoder.str(&instance)?;
                encoder.i32(source_level);
                encoder.u8(branch_count);
            }
        }
    }
    encoder.collection_count(settings.critical_route.len())?;
    for instance in &settings.critical_route {
        encoder.str(instance)?;
    }
    if !settings.walker_connections.is_empty()
        || !settings.spanning_features.is_empty()
        || !settings.anchor_aliases.is_empty()
    {
        encoder.tag(MACRO_EXTENSION_TAG);
        encoder.u8(MACRO_EXTENSION_VERSION);

        let mut walkers = settings.walker_connections.clone();
        for walker in &mut walkers {
            if walker.second_instance < walker.first_instance {
                std::mem::swap(&mut walker.first_instance, &mut walker.second_instance);
            }
        }
        walkers.sort_unstable();
        encoder.collection_count(walkers.len())?;
        for walker in walkers {
            encoder.str(&walker.first_instance)?;
            encoder.str(&walker.second_instance)?;
            encoder.u32(walker.width);
            encoder.i32(walker.level);
        }

        let mut features = settings.spanning_features.clone();
        features.sort_unstable();
        encoder.collection_count(features.len())?;
        for feature in features {
            match feature {
                MacroSpanningFeatureSettings::Tunnel(tunnel) => {
                    encoder.tag(0);
                    encoder.str(&tunnel.name)?;
                    encoder.u8(u8::from(tunnel.canonical_route));
                    encoder.collection_count(tunnel.instance_route.len())?;
                    for instance in tunnel.instance_route {
                        encoder.str(&instance)?;
                    }
                    encoder.str(&tunnel.boundary_terminal.instance)?;
                    encoder.tag(macro_boundary_side_tag(tunnel.boundary_terminal.side));
                    encoder.str(&tunnel.destination_anchor.instance)?;
                    encoder.str(&tunnel.destination_anchor.anchor)?;
                    encoder.i32(tunnel.floor_level);
                    encoder.u32(tunnel.width);
                    encoder.u32(tunnel.clearance);
                    encoder.u32(tunnel.roof_thickness);
                }
            }
        }

        let mut aliases = settings.anchor_aliases.clone();
        aliases.sort_unstable();
        encoder.collection_count(aliases.len())?;
        for alias in aliases {
            encoder.str(&alias.alias)?;
            encoder.str(&alias.instance)?;
            encoder.str(&alias.anchor)?;
        }
    }
    Ok(())
}

const fn macro_boundary_side_tag(side: MacroBoundarySideSettings) -> u8 {
    match side {
        MacroBoundarySideSettings::East => 0,
        MacroBoundarySideSettings::SouthEast => 1,
        MacroBoundarySideSettings::SouthWest => 2,
        MacroBoundarySideSettings::West => 3,
        MacroBoundarySideSettings::NorthWest => 4,
        MacroBoundarySideSettings::NorthEast => 5,
    }
}

const fn macro_axis_tag(axis: MacroAxisSettings) -> u8 {
    match axis {
        MacroAxisSettings::East => 0,
        MacroAxisSettings::SouthEast => 1,
        MacroAxisSettings::SouthWest => 2,
        MacroAxisSettings::West => 3,
        MacroAxisSettings::NorthWest => 4,
        MacroAxisSettings::NorthEast => 5,
    }
}

/// Fingerprints the complete private semantic output of one V3 candidate.
///
/// All plan collections currently use ordered containers. The explicit counts
/// and keys below remain part of the contract so adding, removing, or moving an
/// entry always changes identity.
pub(crate) fn semantic_plan_fingerprint(plan: &GeneratedWorldPlan) -> Result<u64, String> {
    let mut encoder = FingerprintEncoder::new();
    encoder.u32(3);
    // This conditional extension deliberately writes no bytes for established V3
    // recipes. Grand V3 can therefore bind its complete schematic input without
    // changing any legacy semantic fingerprint.
    if let Some(source) = plan.source_schematic_fingerprint {
        encoder.u8(255);
        encoder.u64(source);
    }
    encode_layout_plan(&mut encoder, &plan.layout)?;
    encode_volume_plan(&mut encoder, &plan.volume)?;
    encode_liquids(&mut encoder, &plan.liquids)?;
    encode_features(&mut encoder, &plan.features)?;
    encode_structures(&mut encoder, &plan.structures.by_id)?;
    encode_tile_set(&mut encoder, &plan.blockers)?;
    encode_lights(&mut encoder, &plan.lights)?;
    encoder.collection_count(plan.biome_regions.len())?;
    for (position, region) in &plan.biome_regions {
        encoder.tile_pos(*position);
        encoder.u32(region.0);
    }
    encode_interiors(&mut encoder, &plan.interiors.by_id)?;
    encoder.collection_count(plan.anchors.len())?;
    for (name, position) in &plan.anchors {
        encoder.str(name)?;
        encoder.tile_pos(*position);
    }
    encode_view_hint(&mut encoder, plan.view_hint)?;
    Ok(encoder.finish_semantic_plan())
}

fn encode_ring_settings(
    encoder: &mut FingerprintEncoder,
    ring: &V3Ring7Settings,
) -> Result<(), String> {
    for patch in [
        &ring.center,
        &ring.mountains,
        &ring.waterfall,
        &ring.forest,
        &ring.fort,
        &ring.caves,
        &ring.sky_islands,
    ] {
        encode_patch_settings(encoder, patch)?;
    }
    Ok(())
}

fn encode_ring19_settings(
    encoder: &mut FingerprintEncoder,
    ring: &V3Ring19Settings,
) -> Result<(), String> {
    encoder.collection_count(ring.regions.len())?;
    for region in &ring.regions {
        encode_ring19_region(encoder, region)?;
    }
    encode_shared_edge_settings(encoder, &ring.seam_defaults);

    let mut connections = ring.liquid_connections.clone();
    connections.sort_unstable();
    encoder.collection_count(connections.len())?;
    for connection in connections {
        encoder.u8(connection.source_region);
        encoder.u8(connection.sink_region);
        encoder.u32(connection.width);
        encoder.i32(connection.level);
    }

    let mut outlets = ring.boundary_outlets.clone();
    outlets.sort_unstable();
    encoder.collection_count(outlets.len())?;
    for outlet in outlets {
        encoder.u8(outlet.source_region);
        encoder.tag(ring19_boundary_side_tag(outlet.side));
        encoder.u32(outlet.width);
        encoder.i32(outlet.level);
    }
    if ring.profile != V3Ring19ProfileSettings::TwoRings {
        encoder.tag(RING19_EXTENSION_TAG);
        encoder.u8(RING19_EXTENSION_VERSION);
        encoder.tag(ring19_profile_tag(ring.profile));
    }
    Ok(())
}

fn encode_ring19_region(
    encoder: &mut FingerprintEncoder,
    region: &Ring19RegionSettings,
) -> Result<(), String> {
    encoder.tag(environment_tag(region.environment));
    encode_recipe_settings(encoder, &region.recipe);
    encode_overlays(encoder, &region.overlays)?;
    encoder.u8(region.rotation_turns);
    Ok(())
}

fn encode_patch_settings(
    encoder: &mut FingerprintEncoder,
    patch: &PatchSpec,
) -> Result<(), String> {
    encoder.tag(environment_tag(patch.environment));
    encode_recipe_settings(encoder, &patch.recipe);

    encode_overlays(encoder, &patch.overlays)?;

    match &patch.mask {
        PatchMaskSettings::WholeWorld => encoder.tag(0),
        PatchMaskSettings::GeneratedRegion => encoder.tag(1),
        PatchMaskSettings::Explicit(coords) => {
            encoder.tag(2);
            let mut sorted: Vec<_> = coords
                .iter()
                .map(|coord| (coord.x, coord.y, coord.z))
                .collect();
            sorted.sort_unstable();
            encoder.collection_count(sorted.len())?;
            for (x, y, z) in sorted {
                encoder.i32(x);
                encoder.i32(y);
                encoder.i32(z);
            }
        }
    }
    encode_edge_settings(encoder, &patch.edges);
    Ok(())
}

fn encode_overlays(
    encoder: &mut FingerprintEncoder,
    overlays: &[NamedOverlaySettings],
) -> Result<(), String> {
    let mut overlays: Vec<_> = overlays.iter().collect();
    overlays.sort_by_key(|overlay| (overlay.name.as_str(), overlay_tag(overlay.kind)));
    encoder.collection_count(overlays.len())?;
    for overlay in overlays {
        encode_overlay_settings(encoder, overlay)?;
    }
    Ok(())
}

fn encode_recipe_settings(encoder: &mut FingerprintEncoder, recipe: &V3RecipeSettings) {
    match recipe {
        V3RecipeSettings::Hills(settings) => {
            encoder.tag(0);
            encode_hills_fields(
                encoder,
                settings.valley_level,
                settings.max_relief,
                settings.hills_per_bank,
            );
        }
        V3RecipeSettings::SkyIslands(settings) => {
            encoder.tag(1);
            encode_hills_fields(
                encoder,
                settings.ground.valley_level,
                settings.ground.max_relief,
                settings.ground.hills_per_bank,
            );
            encoder.i32(settings.min_clearance);
            encoder.u8(settings.upper_coverage_percent);
        }
        V3RecipeSettings::Mountains(settings) => {
            encoder.tag(2);
            encoder.i32(settings.base_level);
            encoder.i32(settings.relief);
            encoder.u8(settings.peak_count);
        }
        V3RecipeSettings::Caves(settings) => {
            encoder.tag(3);
            encoder.i32(settings.surface_level);
            encoder.i32(settings.cave_floor_level);
            encoder.u8(settings.chamber_count);
        }
        V3RecipeSettings::Waterfall(_) => encoder.tag(4),
        V3RecipeSettings::Forest(_) => encoder.tag(5),
        V3RecipeSettings::Fort(_) => encoder.tag(6),
        V3RecipeSettings::Volcano(settings) => {
            encoder.tag(7);
            encoder.i32(settings.base_level);
            encoder.i32(settings.summit_relief);
            encoder.u8(settings.massif_coverage_percent);
            encoder.i32(settings.bridge_clearance);
        }
        V3RecipeSettings::DeepForest(settings) => {
            encoder.tag(8);
            encoder.i32(settings.base_level);
            encoder.i32(settings.max_relief);
            encoder.u8(settings.blocker_coverage_percent);
            encoder.u8(settings.clearing_count);
        }
        V3RecipeSettings::Prairie(settings) => {
            encoder.tag(9);
            encoder.i32(settings.base_level);
            encoder.i32(settings.max_relief);
            encoder.u8(settings.grass_coverage_percent);
        }
        V3RecipeSettings::ShallowSea(settings) => {
            encoder.tag(11);
            encoder.i32(settings.sea_level);
        }
        V3RecipeSettings::Beach(settings) => {
            encoder.tag(12);
            encoder.u8(settings.water_coverage_percent);
            encoder.u8(settings.tree_coverage_percent);
        }
        V3RecipeSettings::Shore(settings) => {
            encoder.tag(13);
            encoder.u8(settings.water_coverage_percent);
            encoder.u8(settings.tree_coverage_percent);
            encoder.i32(settings.cliff_height);
        }
        V3RecipeSettings::DeepMountain(settings) => {
            encoder.tag(14);
            encoder.i32(settings.summit_level);
            encoder.i32(settings.hard_cap);
            encoder.i32(settings.treeline);
            encoder.i32(settings.snowline);
        }
        V3RecipeSettings::CrystalAscent(settings) => {
            // Tags 10 and 15 are already reserved by the parallel Outpost and
            // Garden recipe contracts; recipe tags are append-only.
            encoder.tag(16);
            encoder.i32(settings.base_level);
            encoder.i32(settings.rise_levels);
        }
        V3RecipeSettings::DesertTransition(settings) => {
            encoder.tag(17);
            encoder.i32(settings.base_level);
            encoder.i32(settings.max_relief);
            encoder.u8(settings.transition_width);
            encoder.u8(settings.dry_coverage_percent);
        }
        V3RecipeSettings::DesertPlain(settings) => {
            encoder.tag(18);
            encoder.i32(settings.base_level);
            encoder.i32(settings.max_relief);
        }
        V3RecipeSettings::Dunes(settings) => {
            encoder.tag(19);
            encoder.i32(settings.base_level);
            encoder.i32(settings.ridge_height);
            encoder.u8(settings.ridge_spacing);
            encoder.u8(settings.ridge_count);
        }
        V3RecipeSettings::Oasis(settings) => {
            encoder.tag(20);
            encoder.i32(settings.base_level);
            encoder.u8(settings.pool_radius);
            encoder.u8(settings.palm_count);
            encoder.u8(settings.grass_ring_width);
        }
        V3RecipeSettings::SandyIslets(settings) => {
            // Coastal-island tags append after the accepted Arid recipe range.
            encoder.tag(21);
            encoder.i32(settings.sea_level);
            encoder.u8(settings.land_coverage_percent);
            encoder.u8(settings.islet_count);
            encoder.i32(settings.max_relief);
        }
        V3RecipeSettings::WoodedIsland(settings) => {
            encoder.tag(22);
            encoder.i32(settings.sea_level);
            encoder.u8(settings.land_coverage_percent);
            encoder.i32(settings.max_relief);
            encoder.u8(settings.tree_coverage_percent);
        }
    }
}

fn encode_hills_fields(
    encoder: &mut FingerprintEncoder,
    valley_level: i32,
    max_relief: i32,
    hills_per_bank: u8,
) {
    encoder.i32(valley_level);
    encoder.i32(max_relief);
    encoder.u8(hills_per_bank);
}

fn encode_overlay_settings(
    encoder: &mut FingerprintEncoder,
    overlay: &NamedOverlaySettings,
) -> Result<(), String> {
    encoder.str(&overlay.name)?;
    encoder.tag(overlay_tag(overlay.kind));
    Ok(())
}

const fn environment_tag(environment: V3EnvironmentSettings) -> u8 {
    match environment {
        V3EnvironmentSettings::TemperateGrassland => 0,
        V3EnvironmentSettings::Frozen => 1,
        V3EnvironmentSettings::Volcanic => 2,
        V3EnvironmentSettings::Rocky => 3,
        V3EnvironmentSettings::Coastal => 4,
        V3EnvironmentSettings::Alpine => 5,
        V3EnvironmentSettings::Arid => 6,
    }
}

const fn ring19_profile_tag(profile: V3Ring19ProfileSettings) -> u8 {
    match profile {
        V3Ring19ProfileSettings::TwoRings => 0,
        V3Ring19ProfileSettings::DesertOasis => 1,
    }
}

const fn overlay_tag(overlay: V3OverlaySettings) -> u8 {
    match overlay {
        V3OverlaySettings::Liquid => 0,
        V3OverlaySettings::Vegetation => 1,
        V3OverlaySettings::Structure => 2,
        V3OverlaySettings::Lighting => 3,
    }
}

const fn ring19_boundary_side_tag(side: Ring19BoundarySide) -> u8 {
    match side {
        Ring19BoundarySide::East => 0,
        Ring19BoundarySide::SouthEast => 1,
        Ring19BoundarySide::SouthWest => 2,
        Ring19BoundarySide::West => 3,
        Ring19BoundarySide::NorthWest => 4,
        Ring19BoundarySide::NorthEast => 5,
    }
}

fn encode_edge_settings(encoder: &mut FingerprintEncoder, edges: &PatchEdgesSettings) {
    for edge in [
        &edges.east,
        &edges.south_east,
        &edges.south_west,
        &edges.west,
        &edges.north_west,
        &edges.north_east,
    ] {
        match edge {
            PatchEdgeContractSettings::WorldBoundary => encoder.tag(0),
            PatchEdgeContractSettings::Shared(shared) => {
                encoder.tag(1);
                encode_shared_edge_settings(encoder, shared);
            }
        }
    }
}

fn encode_shared_edge_settings(encoder: &mut FingerprintEncoder, shared: &SharedEdgeSettings) {
    encoder.i32(shared.elevation.preferred);
    encoder.i32(shared.elevation.min);
    encoder.i32(shared.elevation.max);
    encoder.u8(shared.walker.count);
    encoder.u32(shared.walker.width);
    match shared.liquid {
        EdgeLiquidSettings::Dry => encoder.tag(0),
        EdgeLiquidSettings::Inlet(port) => {
            encoder.tag(1);
            encoder.u32(port.width);
        }
        EdgeLiquidSettings::Outlet(port) => {
            encoder.tag(2);
            encoder.u32(port.width);
        }
        EdgeLiquidSettings::Standing(port) => {
            encoder.tag(3);
            encoder.u32(port.width);
        }
    }
    encoder.u32(shared.approach_depth);
}

fn encode_layout_plan(
    encoder: &mut FingerprintEncoder,
    layout: &ResolvedLayoutPlan,
) -> Result<(), String> {
    encoder.tag(match layout.kind {
        LayoutKind::Single => 0,
        LayoutKind::Ring7 => 1,
        LayoutKind::Ring19 => 2,
        LayoutKind::Macro => 3,
        LayoutKind::Schematic => 4,
    });
    encoder.u32(layout.grid_radius);
    encode_coord_set(encoder, &layout.footprint)?;
    encoder.collection_count(layout.patches.len())?;
    for (id, patch) in &layout.patches {
        encoder.u32(id.0);
        encoder.u32(patch.biome_region.0);
        if matches!(layout.kind, LayoutKind::Ring19 | LayoutKind::Macro) {
            encoder.u8(patch.rotation_turns);
        }
        encode_coord_set(encoder, &patch.mask)?;
        encoder.collection_count(patch.edges.len())?;
        for (side, reference) in &patch.edges {
            encoder.tag(hex_side_tag(*side));
            match reference {
                ResolvedEdgeReference::WorldBoundary => encoder.tag(0),
                ResolvedEdgeReference::Shared(edge) => {
                    encoder.tag(1);
                    encoder.u32(edge.0);
                }
            }
        }
    }
    encoder.collection_count(layout.shared_edges.len())?;
    for (id, edge) in &layout.shared_edges {
        encoder.u32(id.0);
        encode_resolved_edge(encoder, edge)?;
    }
    if layout.kind == LayoutKind::Ring19 {
        encoder.collection_count(layout.boundary_liquid_outlets.len())?;
        for ((source, side), outlet) in &layout.boundary_liquid_outlets {
            encoder.u32(source.0);
            encoder.tag(hex_side_tag(*side));
            encoder.u32(outlet.source.0);
            encoder.tag(hex_side_tag(outlet.side));
            encoder.collection_count(outlet.lanes.len())?;
            for (inside, outside) in &outlet.lanes {
                encoder.hex_coord(*inside);
                encoder.hex_coord(*outside);
            }
            encode_coord_set(encoder, &outlet.inward_approach)?;
            encoder.u32(outlet.approach_depth);
            encoder.i32(outlet.level);
        }
    }
    Ok(())
}

fn encode_resolved_edge(
    encoder: &mut FingerprintEncoder,
    edge: &ResolvedEdgeContract,
) -> Result<(), String> {
    encode_patch_side(encoder, edge.first);
    encode_patch_side(encoder, edge.second);
    encoder.i32(edge.elevation.preferred);
    encoder.i32(edge.elevation.min);
    encoder.i32(edge.elevation.max);
    encoder.u8(edge.walker.count);
    encoder.u32(edge.walker.width);
    encoder.collection_count(edge.walker.ports.len())?;
    for port in &edge.walker.ports {
        encode_resolved_port(encoder, port)?;
    }
    match &edge.liquid {
        ResolvedLiquidPort::Dry => encoder.tag(0),
        ResolvedLiquidPort::Directed {
            source,
            sink,
            port,
            elevation: ResolvedLiquidElevation::EdgeBand,
        } => {
            encoder.tag(1);
            encoder.u32(source.0);
            encoder.u32(sink.0);
            encode_resolved_port(encoder, port)?;
        }
        ResolvedLiquidPort::Directed {
            source,
            sink,
            port,
            elevation: ResolvedLiquidElevation::Exact(level),
        } => {
            encoder.tag(2);
            encoder.u32(source.0);
            encoder.u32(sink.0);
            encode_resolved_port(encoder, port)?;
            encoder.i32(*level);
        }
        ResolvedLiquidPort::Standing {
            port,
            elevation: ResolvedLiquidElevation::EdgeBand,
        } => {
            encoder.tag(3);
            encode_resolved_port(encoder, port)?;
        }
        ResolvedLiquidPort::Standing {
            port,
            elevation: ResolvedLiquidElevation::Exact(level),
        } => {
            encoder.tag(4);
            encode_resolved_port(encoder, port)?;
            encoder.i32(*level);
        }
    }
    encoder.u32(edge.approach_depth);
    encoder.collection_count(edge.boundary_pairs.len())?;
    for (first, second) in &edge.boundary_pairs {
        encoder.hex_coord(*first);
        encoder.hex_coord(*second);
    }
    encoder.collection_count(edge.protected_approaches.len())?;
    for (patch, cells) in &edge.protected_approaches {
        encoder.u32(patch.0);
        encode_coord_set(encoder, cells)?;
    }
    Ok(())
}

fn encode_resolved_port(
    encoder: &mut FingerprintEncoder,
    port: &ResolvedPort,
) -> Result<(), String> {
    encoder.collection_count(port.lanes.len())?;
    for (first, second) in &port.lanes {
        encoder.hex_coord(*first);
        encoder.hex_coord(*second);
    }
    encode_coord_set(encoder, &port.first_approach)?;
    encode_coord_set(encoder, &port.second_approach)?;
    Ok(())
}

fn encode_patch_side(encoder: &mut FingerprintEncoder, (patch, side): (PatchId, HexSide)) {
    encoder.u32(patch.0);
    encoder.tag(hex_side_tag(side));
}

const fn hex_side_tag(side: HexSide) -> u8 {
    match side {
        HexSide::East => 0,
        HexSide::SouthEast => 1,
        HexSide::SouthWest => 2,
        HexSide::West => 3,
        HexSide::NorthWest => 4,
        HexSide::NorthEast => 5,
    }
}

fn encode_volume_plan(encoder: &mut FingerprintEncoder, volume: &VolumePlan) -> Result<(), String> {
    encode_coord_set(encoder, &volume.mask)?;
    encoder.collection_count(volume.columns.len())?;
    for (coord, column) in &volume.columns {
        encoder.hex_coord(*coord);
        encoder.collection_count(column.elements.len())?;
        for element in &column.elements {
            match element {
                VolumeElement::Solid(mass) => {
                    encoder.tag(0);
                    encoder.i32(mass.levels.bottom);
                    encoder.i32(mass.levels.top);
                    encoder.tag(solid_material_tag(mass.material));
                    encode_optional_region(encoder, mass.cutaway_for.map(|region| region.0));
                }
                VolumeElement::Fill(fill) => {
                    encoder.tag(1);
                    encoder.i32(fill.levels.bottom);
                    encoder.i32(fill.levels.top);
                    encoder.tag(fill_material_tag(fill.material));
                }
            }
        }
    }
    encoder.collection_count(volume.surfaces.len())?;
    for (position, metadata) in &volume.surfaces {
        encoder.tile_pos(*position);
        encode_surface_metadata(encoder, *metadata);
    }
    Ok(())
}

fn encode_surface_metadata(encoder: &mut FingerprintEncoder, metadata: SurfaceMetadata) {
    match metadata.access {
        SurfaceAccess::Ordinary => encoder.tag(0),
        SurfaceAccess::SpecialMovement(region) => {
            encoder.tag(1);
            encoder.u32(region.0);
        }
        SurfaceAccess::NonStandable => encoder.tag(2),
    }
    encode_optional_region(encoder, metadata.interior.map(|region| region.0));
}

fn encode_optional_region(encoder: &mut FingerprintEncoder, region: Option<u32>) {
    match region {
        None => encoder.tag(0),
        Some(region) => {
            encoder.tag(1);
            encoder.u32(region);
        }
    }
}

const fn solid_material_tag(material: SolidMaterialRole) -> u8 {
    match material {
        SolidMaterialRole::Bedrock => 0,
        SolidMaterialRole::Stone => 1,
        SolidMaterialRole::Dirt => 2,
        SolidMaterialRole::Grass => 3,
        SolidMaterialRole::Gravel => 4,
        SolidMaterialRole::Metal => 5,
        SolidMaterialRole::Snow => 6,
        SolidMaterialRole::Ice => 7,
        SolidMaterialRole::Basalt => 8,
        SolidMaterialRole::WorkedStone => 9,
        // Tags 10..=13 are reserved by the complete Outpost draft for
        // Limestone, Slate, Timber, and Terracotta. Keeping Sand after that
        // range makes either wave additive whichever one lands first.
        SolidMaterialRole::Sand => 14,
    }
}

const fn fill_material_tag(material: FillMaterialRole) -> u8 {
    match material {
        FillMaterialRole::Water => 0,
        FillMaterialRole::Lava => 1,
    }
}

fn encode_liquids(encoder: &mut FingerprintEncoder, liquids: &LiquidPlan) -> Result<(), String> {
    encoder.collection_count(liquids.bodies.len())?;
    for (body_id, body) in &liquids.bodies {
        encoder.u32(body_id.0);
        encoder.tag(fill_material_tag(body.material));
        encoder.collection_count(body.nodes.len())?;
        for (position, node) in &body.nodes {
            encoder.tile_pos(*position);
            encoder.tag(match node.state {
                LiquidFlowState::Still => 0,
                LiquidFlowState::Current => 1,
                LiquidFlowState::Rapid => 2,
                LiquidFlowState::Fall => 3,
            });
            match node.downstream {
                None => encoder.tag(0),
                Some(downstream) => {
                    encoder.tag(1);
                    encoder.tile_pos(downstream);
                }
            }
        }
    }
    Ok(())
}

fn encode_features(encoder: &mut FingerprintEncoder, features: &FeaturePlan) -> Result<(), String> {
    encoder.collection_count(features.by_id.len())?;
    for (id, feature) in &features.by_id {
        encoder.u32(id.0);
        encoder.tile_pos(feature.root);
        encoder.tag(match feature.kind {
            FeatureKind::Tree => 0,
            FeatureKind::TallGrass => 1,
            FeatureKind::CaveVegetation => 2,
        });
        encoder.str(feature.object_id.as_str())?;
        encoder.u8(feature.rotation.steps());
        encode_tile_set(encoder, &feature.blocker_footprint)?;
    }
    encoder.collection_count(features.protected_routes.len())?;
    for (name, route) in &features.protected_routes {
        encoder.str(name)?;
        encoder.collection_count(route.centerline.len())?;
        for position in &route.centerline {
            encoder.tile_pos(*position);
        }
        encode_tile_set(encoder, &route.surfaces)?;
    }
    encoder.collection_count(features.clearings.len())?;
    for (name, clearing) in &features.clearings {
        encoder.str(name)?;
        encode_tile_set(encoder, &clearing.surfaces)?;
    }
    Ok(())
}

fn encode_structures(
    encoder: &mut FingerprintEncoder,
    structures: &std::collections::BTreeMap<super::world::StructureId, PlannedStructure>,
) -> Result<(), String> {
    encoder.collection_count(structures.len())?;
    for (id, structure) in structures {
        encoder.u32(id.0);
        encoder.tag(structure_kind_tag(structure.kind));
        encode_tile_set(encoder, &structure.voxels)?;
    }
    Ok(())
}

const fn structure_kind_tag(kind: StructureKind) -> u8 {
    match kind {
        StructureKind::Bridge => 0,
        StructureKind::Wall => 1,
        StructureKind::Stair => 2,
        StructureKind::Tower => 3,
        StructureKind::Gate => 4,
        StructureKind::Keep => 5,
    }
}

fn encode_lights(
    encoder: &mut FingerprintEncoder,
    lights: &std::collections::BTreeMap<super::world::LightId, PlannedGameplayLight>,
) -> Result<(), String> {
    encoder.collection_count(lights.len())?;
    for (id, light) in lights {
        encoder.u32(id.0);
        encoder.tile_pos(light.origin);
        encoder.tag(illumination_tag(light.level));
        encoder.u32(light.radius);
        encode_light_presentation(encoder, light.presentation);
    }
    Ok(())
}

pub(super) fn encode_light_presentation(
    encoder: &mut FingerprintEncoder,
    presentation: Option<PlannedLightPresentation>,
) {
    match presentation {
        None => encoder.tag(0),
        Some(PlannedLightPresentation::CaveCrystal(crystal)) => {
            encoder.tag(1);
            encoder.tag(match crystal.site {
                CaveCrystalSiteKind::InteriorAlcove => 0,
                CaveCrystalSiteKind::EntranceLanding => 1,
            });
            encoder.tag(match crystal.kind {
                CaveCrystalKind::LowCluster => 0,
                CaveCrystalKind::Branched => 1,
                CaveCrystalKind::Spire => 2,
            });
            encoder.tag(crystal.rotation);
        }
        Some(PlannedLightPresentation::CrystalAscent(crystal)) => {
            // Additive tag: the established cave-crystal bytes above remain
            // unchanged for all existing V3 fingerprints.
            encoder.tag(2);
            match crystal.kind {
                super::world::CrystalAscentCrystalKind::Landing(kind) => {
                    encoder.tag(0);
                    encoder.tag(match kind {
                        CaveCrystalKind::LowCluster => 0,
                        CaveCrystalKind::Branched => 1,
                        CaveCrystalKind::Spire => 2,
                    });
                }
                super::world::CrystalAscentCrystalKind::Heart => encoder.tag(1),
            }
            encoder.tag(crystal.rotation);
        }
    }
}

const fn illumination_tag(level: IlluminationLevel) -> u8 {
    match level {
        IlluminationLevel::Dark => 0,
        IlluminationLevel::Dim => 1,
        IlluminationLevel::Bright => 2,
    }
}

fn encode_interiors(
    encoder: &mut FingerprintEncoder,
    interiors: &std::collections::BTreeMap<hex_core::InteriorRegionId, PlannedInterior>,
) -> Result<(), String> {
    encoder.collection_count(interiors.len())?;
    for (id, interior) in interiors {
        encoder.u32(id.0);
        encode_tile_set(encoder, &interior.floors)?;
        encode_tile_set(encoder, &interior.entrances)?;
        encode_tile_set(encoder, &interior.roof_voxels)?;
    }
    Ok(())
}

fn encode_coord_set(
    encoder: &mut FingerprintEncoder,
    coords: &std::collections::BTreeSet<HexCoord>,
) -> Result<(), String> {
    encoder.collection_count(coords.len())?;
    for coord in coords {
        encoder.hex_coord(*coord);
    }
    Ok(())
}

fn encode_tile_set(
    encoder: &mut FingerprintEncoder,
    positions: &std::collections::BTreeSet<TilePos>,
) -> Result<(), String> {
    encoder.collection_count(positions.len())?;
    for position in positions {
        encoder.tile_pos(*position);
    }
    Ok(())
}

fn encode_view_hint(
    encoder: &mut FingerprintEncoder,
    view_hint: MapViewHint,
) -> Result<(), String> {
    for value in [
        view_hint.eye.0,
        view_hint.eye.1,
        view_hint.eye.2,
        view_hint.focus.0,
        view_hint.focus.1,
        view_hint.focus.2,
    ] {
        encoder.finite_f32(value)?;
    }
    Ok(())
}

fn fingerprint(domain: &[u8], payload: &[u8]) -> u64 {
    let domain_len = u64::try_from(domain.len()).unwrap_or(u64::MAX);
    let payload_len = u64::try_from(payload.len()).unwrap_or(u64::MAX);
    let capacity = 8_usize
        .saturating_add(domain.len())
        .saturating_add(8)
        .saturating_add(payload.len());
    let mut framed = Vec::with_capacity(capacity);
    framed.extend_from_slice(&domain_len.to_le_bytes());
    framed.extend_from_slice(domain);
    framed.extend_from_slice(&payload_len.to_le_bytes());
    framed.extend_from_slice(payload);
    xxh3_64(&framed)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use hex_core::{BiomeRegionId, InteriorRegionId};

    use super::*;
    use crate::procedural_v3::layout::{
        ResolvedBoundaryLiquidOutlet, ResolvedEdgeReference, ResolvedElevationBand, ResolvedPatch,
        ResolvedWalkerPorts,
    };
    use crate::procedural_v3::liquid::{LiquidBodyId, LiquidBodyPlan, LiquidNode};
    use crate::procedural_v3::volume::{LevelInterval, SolidMass, SurfaceMetadata, VolumeColumn};
    use crate::procedural_v3::world::{
        FeatureClearing, FeatureId, InteriorPlan, LightId, PlannedFeature, ProtectedFeatureRoute,
        StructureId, StructurePlan,
    };
    use crate::settings::{
        CubeCoord, EdgeElevationSettings, MacroAnchorAliasSettings, MacroAnchorReferenceSettings,
        MacroBoundaryTerminalSettings, MacroSpanningFeatureSettings, MacroTunnelSettings,
        MacroWalkerConnectionSettings, MapSettings, ProceduralSettings,
        Ring19BoundaryOutletSettings, Ring19LiquidConnectionSettings, SharedEdgeSettings,
        TerrainSettings, V3CrystalAscentSettings, V3DesertPlainSettings,
        V3DesertTransitionSettings, V3DunesSettings, V3GrandV3BasicTerrainProfile, V3HillsSettings,
        V3OasisSettings, V3PrairieSettings, V3SandyIsletsSettings, V3SchematicLayoutSettings,
        V3SchematicTemplate, V3SchematicTerrainProfile, V3WoodedIslandSettings, WalkerPortSettings,
    };

    const MOUNTAIN_RANGE_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-mountain-range.ron");
    const TWO_RINGS_RON: &str =
        include_str!("../../../../assets/config/worlds/procedural-two-rings.ron");

    fn shipped_v3_settings(source: &str) -> (u32, f32, ProceduralV3Settings) {
        let settings: MapSettings = ron::from_str(source).expect("shipped V3 settings parse");
        let TerrainSettings::Procedural(ProceduralSettings::V3(v3)) = settings.terrain else {
            panic!("shipped fixture must use procedural V3");
        };
        (settings.grid_radius, settings.level_height, v3)
    }

    fn world_edges() -> PatchEdgesSettings {
        PatchEdgesSettings {
            east: PatchEdgeContractSettings::WorldBoundary,
            south_east: PatchEdgeContractSettings::WorldBoundary,
            south_west: PatchEdgeContractSettings::WorldBoundary,
            west: PatchEdgeContractSettings::WorldBoundary,
            north_west: PatchEdgeContractSettings::WorldBoundary,
            north_east: PatchEdgeContractSettings::WorldBoundary,
        }
    }

    fn hills_patch(marker: i32) -> PatchSpec {
        PatchSpec {
            environment: V3EnvironmentSettings::TemperateGrassland,
            recipe: V3RecipeSettings::Hills(V3HillsSettings {
                valley_level: marker,
                max_relief: 8,
                hills_per_bank: 3,
            }),
            overlays: Vec::new(),
            mask: PatchMaskSettings::GeneratedRegion,
            edges: world_edges(),
        }
    }

    fn compact_world() -> GeneratedWorldPlan {
        let coord = HexCoord::ORIGIN;
        let surface = TilePos::new(coord, 0);
        let mask = BTreeSet::from([coord]);
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let layout = ResolvedLayoutPlan {
            kind: LayoutKind::Single,
            grid_radius: 12,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    rotation_turns: 0,
                    mask: mask.clone(),
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::new(),
        };
        let volume = VolumePlan {
            mask,
            columns: BTreeMap::from([(
                coord,
                VolumeColumn {
                    elements: vec![VolumeElement::Solid(SolidMass {
                        levels: LevelInterval::new(0, 1),
                        material: SolidMaterialRole::Stone,
                        cutaway_for: None,
                    })],
                },
            )]),
            surfaces: BTreeMap::from([(
                surface,
                SurfaceMetadata {
                    access: SurfaceAccess::Ordinary,
                    interior: None,
                },
            )]),
        };
        GeneratedWorldPlan {
            source_schematic_fingerprint: None,
            layout,
            volume,
            liquids: LiquidPlan::default(),
            features: FeaturePlan::default(),
            structures: StructurePlan::default(),
            blockers: BTreeSet::new(),
            lights: BTreeMap::new(),
            biome_regions: BTreeMap::from([(surface, BiomeRegionId(0))]),
            interiors: InteriorPlan::default(),
            anchors: BTreeMap::from([("party_start".to_owned(), surface)]),
            view_hint: MapViewHint::new((0.0, 8.0, 8.0), (0.0, 0.0, 0.0)),
        }
    }

    #[test]
    fn final_domains_are_independent() {
        let mut encoder = FingerprintEncoder::new();
        encoder.str("same payload").expect("the string fits");

        let settings = encoder.finish_settings();
        let semantic = encoder.finish_semantic_plan();
        let materialized = encoder.finish_materialized_world();

        assert_ne!(settings, semantic);
        assert_ne!(settings, materialized);
        assert_ne!(semantic, materialized);
    }

    #[test]
    fn conditional_schematic_source_identity_changes_semantics_without_shifting_legacy() {
        let legacy = compact_world();
        let legacy_fingerprint =
            semantic_plan_fingerprint(&legacy).expect("the compact legacy world fingerprints");

        let mut first = legacy.clone();
        first.source_schematic_fingerprint = Some(7);
        let mut second = legacy.clone();
        second.source_schematic_fingerprint = Some(8);

        assert_eq!(
            semantic_plan_fingerprint(&legacy).expect("legacy identity remains encodable"),
            legacy_fingerprint
        );
        assert_ne!(
            semantic_plan_fingerprint(&first).expect("first source identity fingerprints"),
            semantic_plan_fingerprint(&second).expect("second source identity fingerprints")
        );
    }

    #[test]
    fn length_prefixes_resist_variable_field_ambiguity() {
        let mut split_after_a = FingerprintEncoder::new();
        split_after_a.bytes(b"a").expect("the bytes fit");
        split_after_a.bytes(b"bc").expect("the bytes fit");

        let mut split_after_ab = FingerprintEncoder::new();
        split_after_ab.bytes(b"ab").expect("the bytes fit");
        split_after_ab.bytes(b"c").expect("the bytes fit");

        assert_ne!(split_after_a.bytes, split_after_ab.bytes);
        assert_ne!(
            split_after_a.finish_semantic_plan(),
            split_after_ab.finish_semantic_plan()
        );

        let mut as_string = FingerprintEncoder::new();
        as_string.str("a/bc").expect("the string fits");
        let mut other_string = FingerprintEncoder::new();
        other_string.str("ab/c").expect("the string fits");
        assert_ne!(as_string.bytes, other_string.bytes);
    }

    #[test]
    fn encoding_is_little_endian_and_matches_the_v3_golden() {
        let mut encoder = FingerprintEncoder::new();
        encoder.tag(9);
        encoder.bool(true);
        encoder.u16(0x1234);
        encoder.u32(0x1234_5678);
        encoder.u64(0x0123_4567_89ab_cdef);
        encoder.i32(-0x0123_4567);
        encoder.i64(-0x0123_4567_89ab_cdef);
        encoder.finite_f32(12.5).expect("the float is finite");
        encoder.bytes(b"V3").expect("the bytes fit");
        encoder.str("hex").expect("the string fits");
        encoder.hex_coord(HexCoord::from_axial(-2, 5));
        encoder.tile_pos(TilePos::new(HexCoord::from_axial(4, -7), 13));
        encoder.collection_count(3).expect("the count fits");

        let mut expected = Vec::new();
        expected.extend_from_slice(&[9, 1]);
        expected.extend_from_slice(&0x1234_u16.to_le_bytes());
        expected.extend_from_slice(&0x1234_5678_u32.to_le_bytes());
        expected.extend_from_slice(&0x0123_4567_89ab_cdef_u64.to_le_bytes());
        expected.extend_from_slice(&(-0x0123_4567_i32).to_le_bytes());
        expected.extend_from_slice(&(-0x0123_4567_89ab_cdef_i64).to_le_bytes());
        expected.extend_from_slice(&12.5_f32.to_bits().to_le_bytes());
        expected.extend_from_slice(&2_u64.to_le_bytes());
        expected.extend_from_slice(b"V3");
        expected.extend_from_slice(&3_u64.to_le_bytes());
        expected.extend_from_slice(b"hex");
        for component in [-2_i32, 5, -3, 4, -7, 3, 13] {
            expected.extend_from_slice(&component.to_le_bytes());
        }
        expected.extend_from_slice(&3_u64.to_le_bytes());

        assert_eq!(encoder.bytes, expected);
        assert_eq!(
            encoder.finish_settings(),
            1_560_625_848_665_618_143,
            "update only with an explicit V3 fingerprint-encoding decision"
        );
    }

    #[test]
    fn crystal_ascent_uses_the_reserved_additive_recipe_tag_and_both_fields() {
        let mut encoder = FingerprintEncoder::new();
        encode_recipe_settings(
            &mut encoder,
            &V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
                base_level: 6,
                rise_levels: 144,
            }),
        );

        let mut expected = vec![16];
        expected.extend_from_slice(&6_i32.to_le_bytes());
        expected.extend_from_slice(&144_i32.to_le_bytes());
        assert_eq!(encoder.bytes, expected);

        let baseline = encoder.finish_settings();
        let mut changed_base = FingerprintEncoder::new();
        encode_recipe_settings(
            &mut changed_base,
            &V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
                base_level: 7,
                rise_levels: 144,
            }),
        );
        let mut changed_rise = FingerprintEncoder::new();
        encode_recipe_settings(
            &mut changed_rise,
            &V3RecipeSettings::CrystalAscent(V3CrystalAscentSettings {
                base_level: 6,
                rise_levels: 145,
            }),
        );
        assert_ne!(baseline, changed_base.finish_settings());
        assert_ne!(baseline, changed_rise.finish_settings());
    }

    #[test]
    fn arid_environment_and_desert_recipe_tags_are_append_only_and_complete() {
        assert_eq!(environment_tag(V3EnvironmentSettings::Arid), 6);

        let recipes = [
            V3RecipeSettings::DesertTransition(V3DesertTransitionSettings {
                base_level: 15,
                max_relief: 3,
                transition_width: 8,
                dry_coverage_percent: 55,
            }),
            V3RecipeSettings::DesertPlain(V3DesertPlainSettings {
                base_level: 15,
                max_relief: 2,
            }),
            V3RecipeSettings::Dunes(V3DunesSettings {
                base_level: 15,
                ridge_height: 6,
                ridge_spacing: 12,
                ridge_count: 5,
            }),
            V3RecipeSettings::Oasis(V3OasisSettings {
                base_level: 15,
                pool_radius: 5,
                palm_count: 12,
                grass_ring_width: 3,
            }),
        ];
        let expected = [
            {
                let mut bytes = vec![17];
                bytes.extend_from_slice(&15_i32.to_le_bytes());
                bytes.extend_from_slice(&3_i32.to_le_bytes());
                bytes.extend_from_slice(&[8, 55]);
                bytes
            },
            {
                let mut bytes = vec![18];
                bytes.extend_from_slice(&15_i32.to_le_bytes());
                bytes.extend_from_slice(&2_i32.to_le_bytes());
                bytes
            },
            {
                let mut bytes = vec![19];
                bytes.extend_from_slice(&15_i32.to_le_bytes());
                bytes.extend_from_slice(&6_i32.to_le_bytes());
                bytes.extend_from_slice(&[12, 5]);
                bytes
            },
            {
                let mut bytes = vec![20];
                bytes.extend_from_slice(&15_i32.to_le_bytes());
                bytes.extend_from_slice(&[5, 12, 3]);
                bytes
            },
        ];

        for (recipe, expected) in recipes.iter().zip(expected) {
            let mut encoder = FingerprintEncoder::new();
            encode_recipe_settings(&mut encoder, recipe);
            assert_eq!(encoder.bytes, expected);
        }
    }

    #[test]
    fn coastal_island_recipe_tags_are_append_only_and_every_field_is_sensitive() {
        assert_eq!(environment_tag(V3EnvironmentSettings::Coastal), 4);

        fn recipe_fingerprint(recipe: &V3RecipeSettings) -> u64 {
            let mut encoder = FingerprintEncoder::new();
            encode_recipe_settings(&mut encoder, recipe);
            encoder.finish_settings()
        }

        let sandy = V3RecipeSettings::SandyIslets(V3SandyIsletsSettings {
            sea_level: 8,
            land_coverage_percent: 28,
            islet_count: 5,
            max_relief: 3,
        });
        let wooded = V3RecipeSettings::WoodedIsland(V3WoodedIslandSettings {
            sea_level: 8,
            land_coverage_percent: 68,
            max_relief: 6,
            tree_coverage_percent: 26,
        });

        let mut sandy_encoder = FingerprintEncoder::new();
        encode_recipe_settings(&mut sandy_encoder, &sandy);
        let mut sandy_expected = vec![21];
        sandy_expected.extend_from_slice(&8_i32.to_le_bytes());
        sandy_expected.extend_from_slice(&[28, 5]);
        sandy_expected.extend_from_slice(&3_i32.to_le_bytes());
        assert_eq!(sandy_encoder.bytes, sandy_expected);

        let mut wooded_encoder = FingerprintEncoder::new();
        encode_recipe_settings(&mut wooded_encoder, &wooded);
        let mut wooded_expected = vec![22];
        wooded_expected.extend_from_slice(&8_i32.to_le_bytes());
        wooded_expected.push(68);
        wooded_expected.extend_from_slice(&6_i32.to_le_bytes());
        wooded_expected.push(26);
        assert_eq!(wooded_encoder.bytes, wooded_expected);

        let sandy_baseline = recipe_fingerprint(&sandy);
        for changed in [
            V3SandyIsletsSettings {
                sea_level: 9,
                land_coverage_percent: 28,
                islet_count: 5,
                max_relief: 3,
            },
            V3SandyIsletsSettings {
                sea_level: 8,
                land_coverage_percent: 29,
                islet_count: 5,
                max_relief: 3,
            },
            V3SandyIsletsSettings {
                sea_level: 8,
                land_coverage_percent: 28,
                islet_count: 6,
                max_relief: 3,
            },
            V3SandyIsletsSettings {
                sea_level: 8,
                land_coverage_percent: 28,
                islet_count: 5,
                max_relief: 4,
            },
        ] {
            assert_ne!(
                sandy_baseline,
                recipe_fingerprint(&V3RecipeSettings::SandyIslets(changed))
            );
        }

        let wooded_baseline = recipe_fingerprint(&wooded);
        for changed in [
            V3WoodedIslandSettings {
                sea_level: 9,
                land_coverage_percent: 68,
                max_relief: 6,
                tree_coverage_percent: 26,
            },
            V3WoodedIslandSettings {
                sea_level: 8,
                land_coverage_percent: 69,
                max_relief: 6,
                tree_coverage_percent: 26,
            },
            V3WoodedIslandSettings {
                sea_level: 8,
                land_coverage_percent: 68,
                max_relief: 7,
                tree_coverage_percent: 26,
            },
            V3WoodedIslandSettings {
                sea_level: 8,
                land_coverage_percent: 68,
                max_relief: 6,
                tree_coverage_percent: 27,
            },
        ] {
            assert_ne!(
                wooded_baseline,
                recipe_fingerprint(&V3RecipeSettings::WoodedIsland(changed))
            );
        }
    }

    #[test]
    fn empty_macro_extensions_preserve_shipped_settings_fingerprints() {
        let (radius, level_height, mountain_range) = shipped_v3_settings(MOUNTAIN_RANGE_RON);
        let V3LayoutSettings::Macro(layout) = &mountain_range.layout else {
            unreachable!("Mountain Range is Macro");
        };
        assert!(layout.walker_connections.is_empty());
        assert!(layout.spanning_features.is_empty());
        assert!(layout.anchor_aliases.is_empty());
        assert_eq!(
            settings_fingerprint(radius, level_height, &mountain_range)
                .expect("Mountain Range settings encode"),
            2_843_243_527_997_079_402,
            "defaulted empty Macro extensions append no bytes"
        );

        let (radius, level_height, two_rings) = shipped_v3_settings(TWO_RINGS_RON);
        assert_eq!(
            settings_fingerprint(radius, level_height, &two_rings)
                .expect("Two Rings settings encode"),
            2_347_243_186_379_186_390,
            "the unrelated Ring19 settings identity remains byte-identical"
        );
    }

    #[test]
    fn schematic_layout_uses_additive_tag_four_and_encodes_every_numeric_field() {
        let schematic = V3SchematicLayoutSettings {
            template: V3SchematicTemplate::GrandV3,
            template_revision: 2,
            cell_pitch: 22,
            terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                V3GrandV3BasicTerrainProfile::canonical(),
            ),
        };
        let mut encoder = FingerprintEncoder::new();
        encoder.tag(4);
        encode_schematic_settings(&mut encoder, &schematic);

        let mut expected = vec![4, 0];
        expected.extend_from_slice(&2_u32.to_le_bytes());
        expected.extend_from_slice(&22_u32.to_le_bytes());
        expected.push(0);
        for level in [
            8_i32, 12, 10, 12, 16, 20, 20, 28, 48, 150, 18, 150, 152, 151, 158, 150, 166, 178, 192,
            15, 6, 144,
        ] {
            expected.extend_from_slice(&level.to_le_bytes());
        }
        assert_eq!(
            encoder.bytes, expected,
            "the additive layout tag and BasicV1 field order are wire contracts"
        );
    }

    #[test]
    fn schematic_settings_fingerprint_changes_with_every_numeric_field() {
        fn canonical() -> ProceduralV3Settings {
            ProceduralV3Settings {
                layout: V3LayoutSettings::Schematic(V3SchematicLayoutSettings {
                    template: V3SchematicTemplate::GrandV3,
                    template_revision: 2,
                    cell_pitch: 22,
                    terrain_profile: V3SchematicTerrainProfile::GrandV3BasicV1(
                        V3GrandV3BasicTerrainProfile::canonical(),
                    ),
                }),
            }
        }

        let baseline = settings_fingerprint(187, 0.4, &canonical())
            .expect("canonical Schematic settings encode");

        let mut changed_revision = canonical();
        let V3LayoutSettings::Schematic(schematic) = &mut changed_revision.layout else {
            unreachable!("the fixture is Schematic");
        };
        schematic.template_revision += 1;
        assert_ne!(
            settings_fingerprint(187, 0.4, &changed_revision)
                .expect("changed revision settings encode"),
            baseline
        );

        let mut changed_pitch = canonical();
        let V3LayoutSettings::Schematic(schematic) = &mut changed_pitch.layout else {
            unreachable!("the fixture is Schematic");
        };
        schematic.cell_pitch += 1;
        assert_ne!(
            settings_fingerprint(187, 0.4, &changed_pitch).expect("changed pitch settings encode"),
            baseline
        );

        macro_rules! assert_profile_field_changes_fingerprint {
            ($($field:ident),+ $(,)?) => {
                $(
                    let mut changed = canonical();
                    let V3LayoutSettings::Schematic(schematic) = &mut changed.layout else {
                        unreachable!("the fixture is Schematic");
                    };
                    let V3SchematicTerrainProfile::GrandV3BasicV1(profile) =
                        &mut schematic.terrain_profile;
                    profile.$field += 1;
                    assert_ne!(
                        settings_fingerprint(187, 0.4, &changed)
                            .expect("changed profile settings encode"),
                        baseline,
                        concat!(stringify!($field), " must affect the settings fingerprint")
                    );
                )+
            };
        }
        assert_profile_field_changes_fingerprint!(
            sea_level,
            island_level,
            beach_level,
            shore_level,
            valley_level,
            plateau_level,
            hill_level,
            mountain_floor,
            massif_floor,
            high_core_level,
            high_gradient_per_cell,
            mountain_lake_level,
            frozen_woods_level,
            lake_island_min_level,
            lake_island_max_level,
            sharp_peak_bench_min,
            sharp_peak_bench_max,
            sharp_peak_min,
            sharp_peak_max,
            valley_lake_level,
            crystal_base_level,
            crystal_rise_levels,
        );
    }

    #[test]
    fn ring19_profile_extension_is_conditional_and_preserves_two_rings() {
        let (radius, level_height, two_rings) = shipped_v3_settings(TWO_RINGS_RON);
        let baseline = settings_fingerprint(radius, level_height, &two_rings)
            .expect("defaulted Two Rings settings encode");
        assert_eq!(baseline, 2_347_243_186_379_186_390);

        let explicit_source = TWO_RINGS_RON.replacen(
            "layout: Ring19((",
            "layout: Ring19((\n            profile: TwoRings,",
            1,
        );
        let (_, _, explicit) = shipped_v3_settings(&explicit_source);
        assert_eq!(
            settings_fingerprint(radius, level_height, &explicit)
                .expect("explicit TwoRings settings encode"),
            baseline,
            "the default profile appends no bytes"
        );

        let V3LayoutSettings::Ring19(default_ring) = &two_rings.layout else {
            unreachable!("the fixture is Ring19");
        };
        let mut base_encoder = FingerprintEncoder::new();
        encode_ring19_settings(&mut base_encoder, default_ring).expect("Ring19 encodes");
        let mut desert_ring = default_ring.clone();
        desert_ring.profile = V3Ring19ProfileSettings::DesertOasis;
        let mut desert_encoder = FingerprintEncoder::new();
        encode_ring19_settings(&mut desert_encoder, &desert_ring).expect("Ring19 encodes");
        assert_eq!(
            desert_encoder
                .bytes
                .strip_suffix(&[RING19_EXTENSION_TAG, RING19_EXTENSION_VERSION, 1,]),
            Some(base_encoder.bytes.as_slice()),
            "DesertOasis appends exactly the versioned profile suffix"
        );
        assert_eq!(RING19_EXTENSION_TAG, 129);
        assert_eq!(RING19_EXTENSION_VERSION, 1);
        assert_eq!(
            [
                V3Ring19ProfileSettings::TwoRings,
                V3Ring19ProfileSettings::DesertOasis,
            ]
            .map(ring19_profile_tag),
            [0, 1]
        );
    }

    #[test]
    fn macro_extension_identity_is_conditional_canonical_and_complete() {
        let (radius, level_height, mut settings) = shipped_v3_settings(MOUNTAIN_RANGE_RON);
        let V3LayoutSettings::Macro(layout) = &mut settings.layout else {
            unreachable!("Mountain Range is Macro");
        };
        layout.walker_connections = vec![
            MacroWalkerConnectionSettings {
                first_instance: "hills-center".to_owned(),
                second_instance: "hills-lower".to_owned(),
                width: 4,
                level: 150,
            },
            MacroWalkerConnectionSettings {
                first_instance: "hills-upper".to_owned(),
                second_instance: "hills-upper-outer".to_owned(),
                width: 2,
                level: 24,
            },
        ];
        layout.spanning_features = vec![
            MacroSpanningFeatureSettings::Tunnel(MacroTunnelSettings {
                name: "crystal_mountain.tunnel".to_owned(),
                canonical_route: false,
                instance_route: ["hills-lower-outer", "hills-lower", "hills-center"]
                    .map(str::to_owned)
                    .to_vec(),
                boundary_terminal: MacroBoundaryTerminalSettings {
                    instance: "hills-lower-outer".to_owned(),
                    side: MacroBoundarySideSettings::NorthWest,
                },
                destination_anchor: MacroAnchorReferenceSettings {
                    instance: "hills-center".to_owned(),
                    anchor: "crystal_ascent.lower_entry".to_owned(),
                },
                floor_level: 6,
                width: 4,
                clearance: 6,
                roof_thickness: 3,
            }),
            MacroSpanningFeatureSettings::Tunnel(MacroTunnelSettings {
                name: "review.secondary".to_owned(),
                canonical_route: false,
                instance_route: ["hills-upper-outer", "hills-upper"]
                    .map(str::to_owned)
                    .to_vec(),
                boundary_terminal: MacroBoundaryTerminalSettings {
                    instance: "hills-upper-outer".to_owned(),
                    side: MacroBoundarySideSettings::SouthEast,
                },
                destination_anchor: MacroAnchorReferenceSettings {
                    instance: "hills-upper".to_owned(),
                    anchor: "review.destination".to_owned(),
                },
                floor_level: 7,
                width: 2,
                clearance: 3,
                roof_thickness: 2,
            }),
        ];
        layout.anchor_aliases = vec![
            MacroAnchorAliasSettings {
                alias: "crystal_mountain.ascent_threshold".to_owned(),
                instance: "hills-center".to_owned(),
                anchor: "crystal_ascent.lower_entry".to_owned(),
            },
            MacroAnchorAliasSettings {
                alias: "crystal_mountain.ridge".to_owned(),
                instance: "hills-upper".to_owned(),
                anchor: "ridge".to_owned(),
            },
        ];
        let baseline = settings_fingerprint(radius, level_height, &settings)
            .expect("extended Macro settings encode");

        let mut reordered = settings.clone();
        let V3LayoutSettings::Macro(reordered_layout) = &mut reordered.layout else {
            unreachable!("reordered fixture remains Macro");
        };
        reordered_layout.walker_connections.reverse();
        for walker in &mut reordered_layout.walker_connections {
            std::mem::swap(&mut walker.first_instance, &mut walker.second_instance);
        }
        reordered_layout.spanning_features.reverse();
        reordered_layout.anchor_aliases.reverse();
        assert_eq!(
            baseline,
            settings_fingerprint(radius, level_height, &reordered)
                .expect("reordered Macro settings encode"),
            "unordered extension collections and unordered walker endpoints canonicalize"
        );

        let mut changed_walker = settings.clone();
        let V3LayoutSettings::Macro(layout) = &mut changed_walker.layout else {
            unreachable!();
        };
        layout
            .walker_connections
            .first_mut()
            .expect("fixture has explicit walkers")
            .level += 1;
        assert_ne!(
            baseline,
            settings_fingerprint(radius, level_height, &changed_walker).expect("settings encode")
        );

        let mut changed_tunnel = settings.clone();
        let V3LayoutSettings::Macro(layout) = &mut changed_tunnel.layout else {
            unreachable!();
        };
        let MacroSpanningFeatureSettings::Tunnel(tunnel) = layout
            .spanning_features
            .first_mut()
            .expect("fixture has spanning features");
        tunnel.roof_thickness += 1;
        assert_ne!(
            baseline,
            settings_fingerprint(radius, level_height, &changed_tunnel).expect("settings encode")
        );

        let mut changed_alias = settings.clone();
        let V3LayoutSettings::Macro(layout) = &mut changed_alias.layout else {
            unreachable!();
        };
        layout
            .anchor_aliases
            .first_mut()
            .expect("fixture has anchor aliases")
            .anchor
            .push_str(".changed");
        assert_ne!(
            baseline,
            settings_fingerprint(radius, level_height, &changed_alias).expect("settings encode")
        );

        assert_eq!(MACRO_EXTENSION_TAG, 128);
        assert_eq!(MACRO_EXTENSION_VERSION, 1);
        assert_eq!(
            [
                MacroBoundarySideSettings::East,
                MacroBoundarySideSettings::SouthEast,
                MacroBoundarySideSettings::SouthWest,
                MacroBoundarySideSettings::West,
                MacroBoundarySideSettings::NorthWest,
                MacroBoundarySideSettings::NorthEast,
            ]
            .map(macro_boundary_side_tag),
            [0, 1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn solid_material_tags_are_append_only() {
        let roles = [
            SolidMaterialRole::Bedrock,
            SolidMaterialRole::Stone,
            SolidMaterialRole::Dirt,
            SolidMaterialRole::Grass,
            SolidMaterialRole::Gravel,
            SolidMaterialRole::Metal,
            SolidMaterialRole::Snow,
            SolidMaterialRole::Ice,
            SolidMaterialRole::Basalt,
            SolidMaterialRole::WorkedStone,
            SolidMaterialRole::Sand,
        ];

        assert_eq!(
            roles.map(solid_material_tag),
            [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 14]
        );
    }

    #[test]
    fn sorted_caller_input_is_insertion_order_independent() {
        fn encode(entries: impl IntoIterator<Item = (&'static str, i32)>) -> u64 {
            let sorted = entries.into_iter().collect::<BTreeMap<_, _>>();
            let mut encoder = FingerprintEncoder::new();
            encoder
                .collection_count(sorted.len())
                .expect("the count fits");
            for (name, level) in sorted {
                encoder.str(name).expect("the string fits");
                encoder.i32(level);
            }
            encoder.finish_semantic_plan()
        }

        let forward = encode([("bridge", 16), ("ford", 14), ("summit", 30)]);
        let reverse = encode([("summit", 30), ("ford", 14), ("bridge", 16)]);

        assert_eq!(forward, reverse);
    }

    #[test]
    fn changing_each_primitive_changes_the_payload() {
        fn payload(changed_field: Option<usize>) -> FingerprintEncoder {
            let mut encoder = FingerprintEncoder::new();
            encoder.tag(u8::from(changed_field == Some(0)));
            encoder.bool(changed_field == Some(1));
            encoder.u8(u8::from(changed_field == Some(2)));
            encoder.u16(u16::from(changed_field == Some(3)));
            encoder.u32(u32::from(changed_field == Some(4)));
            encoder.u64(u64::from(changed_field == Some(5)));
            encoder.i32(i32::from(changed_field == Some(6)));
            encoder.i64(i64::from(changed_field == Some(7)));
            encoder
                .finite_f32(if changed_field == Some(8) { 1.0 } else { 0.0 })
                .expect("the floats are finite");
            encoder
                .bytes(if changed_field == Some(9) { b"x" } else { b"" })
                .expect("the bytes fit");
            encoder
                .str(if changed_field == Some(10) { "x" } else { "" })
                .expect("the string fits");
            encoder.hex_coord(if changed_field == Some(11) {
                HexCoord::from_axial(1, -1)
            } else {
                HexCoord::ORIGIN
            });
            encoder.tile_pos(TilePos::new(
                HexCoord::ORIGIN,
                i32::from(changed_field == Some(12)),
            ));
            encoder
                .collection_count(usize::from(changed_field == Some(13)))
                .expect("the count fits");
            encoder
        }

        let baseline = payload(None);
        for field in 0..14 {
            let changed = payload(Some(field));
            assert_ne!(changed.bytes, baseline.bytes, "primitive field {field}");
            assert_ne!(
                changed.finish_semantic_plan(),
                baseline.finish_semantic_plan(),
                "primitive field {field}"
            );
        }
    }

    #[test]
    fn non_finite_floats_are_rejected_and_signed_zero_is_exact() {
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut encoder = FingerprintEncoder::new();
            assert!(encoder.finite_f32(value).is_err());
        }

        let mut positive = FingerprintEncoder::new();
        positive.finite_f32(0.0).expect("zero is finite");
        let mut negative = FingerprintEncoder::new();
        negative.finite_f32(-0.0).expect("zero is finite");
        assert_ne!(positive.bytes, negative.bytes);
    }

    #[test]
    fn settings_identity_sorts_overlays_and_explicit_coordinates() {
        let mut patch = hills_patch(15);
        patch.overlays = vec![
            NamedOverlaySettings {
                name: "vegetation".to_owned(),
                kind: V3OverlaySettings::Vegetation,
            },
            NamedOverlaySettings {
                name: "liquid".to_owned(),
                kind: V3OverlaySettings::Liquid,
            },
        ];
        patch.mask = PatchMaskSettings::Explicit(vec![
            CubeCoord { x: 1, y: -1, z: 0 },
            CubeCoord { x: 0, y: 0, z: 0 },
        ]);
        let forward = ProceduralV3Settings {
            layout: V3LayoutSettings::Single(patch),
        };
        let mut reversed = forward.clone();
        {
            let V3LayoutSettings::Single(reversed_patch) = &mut reversed.layout else {
                unreachable!("the fixture is Single");
            };
            reversed_patch.overlays.reverse();
            let PatchMaskSettings::Explicit(coords) = &mut reversed_patch.mask else {
                unreachable!("the fixture has an explicit mask");
            };
            coords.reverse();
        }

        assert_eq!(
            settings_fingerprint(12, 0.4, &forward).expect("the settings encode"),
            settings_fingerprint(12, 0.4, &reversed).expect("the settings encode")
        );
        assert_ne!(
            settings_fingerprint(12, 0.4, &forward).expect("the settings encode"),
            settings_fingerprint(12, 0.5, &forward).expect("the settings encode")
        );
        assert!(settings_fingerprint(12, f32::NAN, &forward).is_err());

        let V3LayoutSettings::Single(reversed_patch) = &mut reversed.layout else {
            unreachable!("the fixture is Single");
        };
        reversed_patch
            .overlays
            .first_mut()
            .expect("the fixture has overlays")
            .kind = V3OverlaySettings::Lighting;
        assert_ne!(
            settings_fingerprint(12, 0.4, &forward).expect("the settings encode"),
            settings_fingerprint(12, 0.4, &reversed).expect("the settings encode")
        );
    }

    #[test]
    fn ring_settings_use_fixed_semantic_patch_order() {
        let original = ProceduralV3Settings {
            layout: V3LayoutSettings::Ring7(V3Ring7Settings {
                center: hills_patch(10),
                mountains: hills_patch(11),
                waterfall: hills_patch(12),
                forest: hills_patch(13),
                fort: hills_patch(14),
                caves: hills_patch(15),
                sky_islands: hills_patch(16),
            }),
        };
        let mut swapped = original.clone();
        let V3LayoutSettings::Ring7(ring) = &mut swapped.layout else {
            unreachable!("the fixture is Ring7");
        };
        std::mem::swap(&mut ring.mountains, &mut ring.waterfall);

        assert_ne!(
            settings_fingerprint(33, 0.4, &original).expect("the settings encode"),
            settings_fingerprint(33, 0.4, &swapped).expect("the settings encode")
        );
    }

    #[test]
    fn ring19_identity_fixes_region_order_and_canonicalizes_graph_lists() {
        let region = Ring19RegionSettings {
            environment: V3EnvironmentSettings::TemperateGrassland,
            recipe: V3RecipeSettings::Prairie(V3PrairieSettings {
                base_level: 12,
                max_relief: 5,
                grass_coverage_percent: 70,
            }),
            overlays: Vec::new(),
            rotation_turns: 0,
        };
        let original = ProceduralV3Settings {
            layout: V3LayoutSettings::Ring19(V3Ring19Settings {
                profile: V3Ring19ProfileSettings::TwoRings,
                regions: vec![region; 19],
                seam_defaults: SharedEdgeSettings {
                    elevation: EdgeElevationSettings {
                        preferred: 15,
                        min: 14,
                        max: 16,
                    },
                    walker: WalkerPortSettings { count: 2, width: 2 },
                    liquid: EdgeLiquidSettings::Dry,
                    approach_depth: 3,
                },
                liquid_connections: vec![
                    Ring19LiquidConnectionSettings {
                        source_region: 0,
                        sink_region: 1,
                        width: 3,
                        level: 16,
                    },
                    Ring19LiquidConnectionSettings {
                        source_region: 1,
                        sink_region: 7,
                        width: 3,
                        level: 16,
                    },
                ],
                boundary_outlets: vec![
                    Ring19BoundaryOutletSettings {
                        source_region: 7,
                        side: Ring19BoundarySide::NorthEast,
                        width: 3,
                        level: 16,
                    },
                    Ring19BoundaryOutletSettings {
                        source_region: 9,
                        side: Ring19BoundarySide::East,
                        width: 2,
                        level: 16,
                    },
                ],
            }),
        };
        let mut reordered = original.clone();
        {
            let V3LayoutSettings::Ring19(ring) = &mut reordered.layout else {
                unreachable!("the fixture is Ring19");
            };
            ring.liquid_connections.reverse();
            ring.boundary_outlets.reverse();
        }
        assert_eq!(
            settings_fingerprint(55, 0.4, &original).expect("the settings encode"),
            settings_fingerprint(55, 0.4, &reordered).expect("the settings encode"),
            "connection and outlet declaration order is not semantic"
        );

        let mut changed_connection_level = original.clone();
        let V3LayoutSettings::Ring19(ring) = &mut changed_connection_level.layout else {
            unreachable!("the fixture is Ring19");
        };
        ring.liquid_connections
            .first_mut()
            .expect("the fixture has internal liquid")
            .level += 1;
        assert_ne!(
            settings_fingerprint(55, 0.4, &original).expect("the settings encode"),
            settings_fingerprint(55, 0.4, &changed_connection_level).expect("the settings encode"),
            "exact internal liquid levels are semantic"
        );

        let mut changed_outlet_level = original.clone();
        let V3LayoutSettings::Ring19(ring) = &mut changed_outlet_level.layout else {
            unreachable!("the fixture is Ring19");
        };
        ring.boundary_outlets
            .first_mut()
            .expect("the fixture has a boundary outlet")
            .level += 1;
        assert_ne!(
            settings_fingerprint(55, 0.4, &original).expect("the settings encode"),
            settings_fingerprint(55, 0.4, &changed_outlet_level).expect("the settings encode"),
            "exact boundary liquid levels are semantic"
        );

        let V3LayoutSettings::Ring19(ring) = &mut reordered.layout else {
            unreachable!("the fixture is Ring19");
        };
        ring.regions.swap(0, 1);
        ring.regions
            .first_mut()
            .expect("the fixture has nineteen regions")
            .rotation_turns = 1;
        assert_ne!(
            settings_fingerprint(55, 0.4, &original).expect("the settings encode"),
            settings_fingerprint(55, 0.4, &reordered).expect("the settings encode"),
            "fixed slot order and rotation are semantic"
        );
    }

    #[test]
    fn resolved_port_identity_covers_exact_lanes_and_approaches() {
        fn fingerprint_port(port: &ResolvedPort) -> u64 {
            let mut encoder = FingerprintEncoder::new();
            encode_resolved_port(&mut encoder, port).expect("the port encodes");
            encoder.finish_semantic_plan()
        }

        let first = HexCoord::ORIGIN;
        let second = HexSide::East.neighbor(first);
        let baseline = ResolvedPort {
            lanes: BTreeSet::from([(first, second)]),
            first_approach: BTreeSet::from([first]),
            second_approach: BTreeSet::from([second]),
        };
        let baseline_fingerprint = fingerprint_port(&baseline);

        let mut moved_lane = baseline.clone();
        let moved_first = HexCoord::from_axial(0, 1);
        moved_lane.lanes = BTreeSet::from([(moved_first, HexSide::East.neighbor(moved_first))]);
        assert_ne!(fingerprint_port(&moved_lane), baseline_fingerprint);

        let mut changed_first_approach = baseline.clone();
        changed_first_approach
            .first_approach
            .insert(HexCoord::from_axial(-1, 0));
        assert_ne!(
            fingerprint_port(&changed_first_approach),
            baseline_fingerprint
        );

        let mut changed_second_approach = baseline.clone();
        changed_second_approach
            .second_approach
            .insert(HexCoord::from_axial(2, 0));
        assert_ne!(
            fingerprint_port(&changed_second_approach),
            baseline_fingerprint
        );
    }

    #[test]
    fn resolved_edge_identity_covers_walker_and_liquid_port_topology() {
        fn fingerprint_edge(edge: &ResolvedEdgeContract) -> u64 {
            let mut encoder = FingerprintEncoder::new();
            encode_resolved_edge(&mut encoder, edge).expect("the edge encodes");
            encoder.finish_semantic_plan()
        }

        let first = HexCoord::ORIGIN;
        let second = HexSide::East.neighbor(first);
        let port = ResolvedPort {
            lanes: BTreeSet::from([(first, second)]),
            first_approach: BTreeSet::from([first]),
            second_approach: BTreeSet::from([second]),
        };
        let mut baseline = ResolvedEdgeContract {
            first: (PatchId(0), HexSide::East),
            second: (PatchId(1), HexSide::West),
            elevation: ResolvedElevationBand {
                preferred: 15,
                min: 14,
                max: 16,
            },
            walker: ResolvedWalkerPorts {
                count: 1,
                width: 1,
                ports: vec![port.clone()],
            },
            liquid: ResolvedLiquidPort::Dry,
            approach_depth: 2,
            boundary_pairs: BTreeSet::from([(first, second)]),
            protected_approaches: BTreeMap::from([
                (PatchId(0), BTreeSet::from([first])),
                (PatchId(1), BTreeSet::from([second])),
            ]),
        };
        let dry_fingerprint = fingerprint_edge(&baseline);

        baseline.walker.count = 2;
        assert_ne!(fingerprint_edge(&baseline), dry_fingerprint);
        baseline.walker.count = 1;
        baseline.walker.width = 2;
        assert_ne!(fingerprint_edge(&baseline), dry_fingerprint);
        baseline.walker.width = 1;

        baseline.liquid = ResolvedLiquidPort::Directed {
            source: PatchId(0),
            sink: PatchId(1),
            port: port.clone(),
            elevation: ResolvedLiquidElevation::EdgeBand,
        };
        let directed_fingerprint = fingerprint_edge(&baseline);
        assert_ne!(directed_fingerprint, dry_fingerprint);

        let mut exact = baseline.clone();
        let ResolvedLiquidPort::Directed { elevation, .. } = &mut exact.liquid else {
            unreachable!("the fixture has a directed port");
        };
        *elevation = ResolvedLiquidElevation::Exact(16);
        let exact_fingerprint = fingerprint_edge(&exact);
        assert_ne!(exact_fingerprint, directed_fingerprint);
        let ResolvedLiquidPort::Directed { elevation, .. } = &mut exact.liquid else {
            unreachable!("the fixture has a directed port");
        };
        *elevation = ResolvedLiquidElevation::Exact(17);
        assert_ne!(fingerprint_edge(&exact), exact_fingerprint);

        let ResolvedLiquidPort::Directed { port, .. } = &mut baseline.liquid else {
            unreachable!("the fixture has a directed port");
        };
        port.first_approach.insert(HexCoord::from_axial(-1, 0));
        assert_ne!(fingerprint_edge(&baseline), directed_fingerprint);
    }

    #[test]
    fn ring19_boundary_outlet_identity_covers_exact_geometry_and_level() {
        fn fingerprint_layout(layout: &ResolvedLayoutPlan) -> u64 {
            let mut encoder = FingerprintEncoder::new();
            encode_layout_plan(&mut encoder, layout).expect("the layout encodes");
            encoder.finish_semantic_plan()
        }

        let inside_a = HexCoord::from_axial(2, -1);
        let inside_b = HexCoord::from_axial(2, 0);
        let outside_a = HexSide::East.neighbor(inside_a);
        let outside_b = HexSide::East.neighbor(inside_b);
        let mask = HexCoord::ORIGIN
            .within_radius(2)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let edges = HexSide::ALL
            .into_iter()
            .map(|side| (side, ResolvedEdgeReference::WorldBoundary))
            .collect();
        let mut layout = ResolvedLayoutPlan {
            kind: LayoutKind::Ring19,
            grid_radius: 55,
            footprint: mask.clone(),
            patches: BTreeMap::from([(
                PatchId(0),
                ResolvedPatch {
                    biome_region: BiomeRegionId(0),
                    rotation_turns: 0,
                    mask,
                    edges,
                },
            )]),
            shared_edges: BTreeMap::new(),
            boundary_liquid_outlets: BTreeMap::from([(
                (PatchId(0), HexSide::East),
                ResolvedBoundaryLiquidOutlet {
                    source: PatchId(0),
                    side: HexSide::East,
                    lanes: BTreeSet::from([(inside_a, outside_a), (inside_b, outside_b)]),
                    inward_approach: BTreeSet::from([
                        inside_a,
                        inside_b,
                        HexSide::West.neighbor(inside_a),
                        HexSide::West.neighbor(inside_b),
                    ]),
                    approach_depth: 2,
                    level: 16,
                },
            )]),
        };
        let baseline = fingerprint_layout(&layout);

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .level += 1;
        assert_ne!(fingerprint_layout(&layout), baseline);
        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .level -= 1;

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .approach_depth += 1;
        assert_ne!(fingerprint_layout(&layout), baseline);
        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .approach_depth -= 1;

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .inward_approach
            .insert(HexCoord::from_axial(0, 1));
        assert_ne!(fingerprint_layout(&layout), baseline);
        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .inward_approach
            .remove(&HexCoord::from_axial(0, 1));

        layout
            .boundary_liquid_outlets
            .values_mut()
            .next()
            .expect("the fixture has one outlet")
            .lanes
            .remove(&(inside_b, outside_b));
        assert_ne!(fingerprint_layout(&layout), baseline);
    }

    #[test]
    fn liquid_identity_covers_body_material_nodes_state_and_downstream() {
        fn fingerprint_liquid(liquid: &LiquidPlan) -> u64 {
            let mut encoder = FingerprintEncoder::new();
            encode_liquids(&mut encoder, liquid).expect("the liquid plan encodes");
            encoder.finish_semantic_plan()
        }

        let upstream = TilePos::new(HexCoord::ORIGIN, 4);
        let downstream = TilePos::new(HexCoord::from_axial(1, 0), 3);
        let baseline = LiquidPlan {
            bodies: BTreeMap::from([(
                LiquidBodyId(7),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: BTreeMap::from([
                        (
                            upstream,
                            LiquidNode {
                                state: LiquidFlowState::Current,
                                downstream: Some(downstream),
                            },
                        ),
                        (
                            downstream,
                            LiquidNode {
                                state: LiquidFlowState::Still,
                                downstream: None,
                            },
                        ),
                    ]),
                },
            )]),
        };
        let baseline_fingerprint = fingerprint_liquid(&baseline);

        let mut changed_id = baseline.clone();
        let body = changed_id
            .bodies
            .remove(&LiquidBodyId(7))
            .expect("the body exists");
        changed_id.bodies.insert(LiquidBodyId(8), body);
        assert_ne!(fingerprint_liquid(&changed_id), baseline_fingerprint);

        let mut changed_material = baseline.clone();
        changed_material
            .bodies
            .get_mut(&LiquidBodyId(7))
            .expect("the body exists")
            .material = FillMaterialRole::Lava;
        assert_ne!(fingerprint_liquid(&changed_material), baseline_fingerprint);

        let mut changed_state = baseline.clone();
        changed_state
            .bodies
            .get_mut(&LiquidBodyId(7))
            .and_then(|body| body.nodes.get_mut(&upstream))
            .expect("the node exists")
            .state = LiquidFlowState::Rapid;
        assert_ne!(fingerprint_liquid(&changed_state), baseline_fingerprint);

        let mut changed_downstream = baseline.clone();
        changed_downstream
            .bodies
            .get_mut(&LiquidBodyId(7))
            .and_then(|body| body.nodes.get_mut(&upstream))
            .expect("the node exists")
            .downstream = None;
        assert_ne!(
            fingerprint_liquid(&changed_downstream),
            baseline_fingerprint
        );

        let mut changed_downstream_level = baseline.clone();
        changed_downstream_level
            .bodies
            .get_mut(&LiquidBodyId(7))
            .and_then(|body| body.nodes.get_mut(&upstream))
            .expect("the node exists")
            .downstream = Some(TilePos::new(downstream.coord, downstream.level - 1));
        assert_ne!(
            fingerprint_liquid(&changed_downstream_level),
            baseline_fingerprint
        );
    }

    #[test]
    fn semantic_identity_covers_every_top_level_layer() {
        fn mutate_layout(plan: &mut GeneratedWorldPlan) {
            plan.layout.grid_radius = 13;
        }
        fn mutate_volume(plan: &mut GeneratedWorldPlan) {
            let Some(column) = plan.volume.columns.get_mut(&HexCoord::ORIGIN) else {
                return;
            };
            let Some(VolumeElement::Solid(mass)) = column.elements.first_mut() else {
                return;
            };
            mass.material = SolidMaterialRole::Dirt;
        }
        fn mutate_liquids(plan: &mut GeneratedWorldPlan) {
            plan.liquids.bodies.insert(
                LiquidBodyId(1),
                LiquidBodyPlan {
                    material: FillMaterialRole::Water,
                    nodes: BTreeMap::from([(
                        TilePos::new(HexCoord::ORIGIN, 1),
                        LiquidNode {
                            state: LiquidFlowState::Still,
                            downstream: None,
                        },
                    )]),
                },
            );
        }
        fn mutate_features(plan: &mut GeneratedWorldPlan) {
            plan.features.by_id.insert(
                FeatureId(1),
                PlannedFeature {
                    root: TilePos::ORIGIN,
                    kind: FeatureKind::TallGrass,
                    object_id: hex_assets::ObjectAssetId::new("prop/grass-tuft")
                        .expect("fixture id should be valid"),
                    rotation: hex_assets::HexObjectRotation::ZERO,
                    blocker_footprint: BTreeSet::new(),
                },
            );
        }
        fn mutate_structures(plan: &mut GeneratedWorldPlan) {
            plan.structures.by_id.insert(
                StructureId(1),
                PlannedStructure {
                    kind: StructureKind::Bridge,
                    voxels: BTreeSet::from([TilePos::ORIGIN]),
                },
            );
        }
        fn mutate_blockers(plan: &mut GeneratedWorldPlan) {
            plan.blockers.insert(TilePos::ORIGIN);
        }
        fn mutate_lights(plan: &mut GeneratedWorldPlan) {
            plan.lights.insert(
                LightId(1),
                PlannedGameplayLight {
                    origin: TilePos::ORIGIN,
                    level: IlluminationLevel::Bright,
                    radius: 4,
                    presentation: None,
                },
            );
        }
        fn mutate_biomes(plan: &mut GeneratedWorldPlan) {
            plan.biome_regions.insert(TilePos::ORIGIN, BiomeRegionId(1));
        }
        fn mutate_interiors(plan: &mut GeneratedWorldPlan) {
            plan.interiors.by_id.insert(
                InteriorRegionId(1),
                PlannedInterior {
                    floors: BTreeSet::from([TilePos::ORIGIN]),
                    entrances: BTreeSet::new(),
                    roof_voxels: BTreeSet::new(),
                },
            );
        }
        fn mutate_anchors(plan: &mut GeneratedWorldPlan) {
            plan.anchors
                .insert("enemy_start".to_owned(), TilePos::ORIGIN);
        }
        fn mutate_view(plan: &mut GeneratedWorldPlan) {
            plan.view_hint.eye.0 = 1.0;
        }

        let baseline = compact_world();
        assert!(
            baseline.validate().is_empty(),
            "the compact baseline should satisfy common world contracts"
        );
        let baseline_fingerprint =
            semantic_plan_fingerprint(&baseline).expect("the baseline encodes");
        let mutations: [(&str, fn(&mut GeneratedWorldPlan)); 11] = [
            ("layout", mutate_layout),
            ("volume", mutate_volume),
            ("liquids", mutate_liquids),
            ("features", mutate_features),
            ("structures", mutate_structures),
            ("blockers", mutate_blockers),
            ("lights", mutate_lights),
            ("biome memberships", mutate_biomes),
            ("interiors", mutate_interiors),
            ("anchors", mutate_anchors),
            ("view hint", mutate_view),
        ];

        for (name, mutate) in mutations {
            let mut changed = baseline.clone();
            mutate(&mut changed);
            assert_ne!(
                semantic_plan_fingerprint(&changed).expect("the changed plan encodes"),
                baseline_fingerprint,
                "mutating {name} must change semantic identity"
            );
        }
    }

    #[test]
    fn semantic_identity_covers_every_crystal_presentation_choice() {
        let mut fingerprints = BTreeSet::new();
        for site in [
            CaveCrystalSiteKind::InteriorAlcove,
            CaveCrystalSiteKind::EntranceLanding,
        ] {
            for kind in [
                CaveCrystalKind::LowCluster,
                CaveCrystalKind::Branched,
                CaveCrystalKind::Spire,
            ] {
                for rotation in 0..6 {
                    let mut plan = compact_world();
                    plan.lights.insert(
                        LightId(1),
                        PlannedGameplayLight {
                            origin: TilePos::ORIGIN,
                            level: IlluminationLevel::Bright,
                            radius: 4,
                            presentation: Some(PlannedLightPresentation::CaveCrystal(
                                CaveCrystalPresentation {
                                    kind,
                                    site,
                                    rotation,
                                },
                            )),
                        },
                    );
                    fingerprints.insert(
                        semantic_plan_fingerprint(&plan)
                            .expect("every crystal presentation choice should encode"),
                    );
                }
            }
        }
        assert_eq!(fingerprints.len(), 36);
    }

    #[test]
    fn semantic_identity_covers_named_feature_memberships() {
        let baseline = compact_world();
        let baseline_fingerprint =
            semantic_plan_fingerprint(&baseline).expect("the baseline encodes");

        let mut with_route = baseline.clone();
        with_route.features.protected_routes.insert(
            "main_route".to_owned(),
            ProtectedFeatureRoute {
                centerline: vec![TilePos::ORIGIN],
                surfaces: BTreeSet::from([TilePos::ORIGIN]),
            },
        );
        assert_ne!(
            semantic_plan_fingerprint(&with_route).expect("the route plan encodes"),
            baseline_fingerprint
        );

        let mut with_clearing = baseline.clone();
        with_clearing.features.clearings.insert(
            "meadow".to_owned(),
            FeatureClearing {
                surfaces: BTreeSet::from([TilePos::ORIGIN]),
            },
        );
        assert_ne!(
            semantic_plan_fingerprint(&with_clearing).expect("the clearing plan encodes"),
            baseline_fingerprint
        );
        assert_ne!(
            semantic_plan_fingerprint(&with_route).expect("the route plan encodes"),
            semantic_plan_fingerprint(&with_clearing).expect("the clearing plan encodes")
        );
    }

    #[test]
    fn named_feature_membership_insertion_order_is_not_semantic() {
        let first_position = TilePos::ORIGIN;
        let second_position = TilePos::new(HexCoord::ORIGIN, 1);
        let first_entries = [
            (
                "secondary".to_owned(),
                ProtectedFeatureRoute {
                    centerline: vec![second_position, first_position],
                    surfaces: BTreeSet::from([second_position, first_position]),
                },
            ),
            (
                "primary".to_owned(),
                ProtectedFeatureRoute {
                    centerline: vec![first_position],
                    surfaces: BTreeSet::from([first_position]),
                },
            ),
        ];

        let mut first = compact_world();
        for (name, route) in first_entries.clone() {
            first.features.protected_routes.insert(name, route);
        }
        let mut second = compact_world();
        for (name, route) in first_entries.into_iter().rev() {
            second.features.protected_routes.insert(name, route);
        }

        assert_eq!(
            semantic_plan_fingerprint(&first).expect("the first plan encodes"),
            semantic_plan_fingerprint(&second).expect("the second plan encodes")
        );
    }

    #[test]
    fn protected_route_centerline_order_is_semantic() {
        let first_position = TilePos::ORIGIN;
        let second_position = TilePos::new(HexCoord::ORIGIN, 1);
        let surfaces = BTreeSet::from([first_position, second_position]);
        let mut forward = compact_world();
        forward.features.protected_routes.insert(
            "road".to_owned(),
            ProtectedFeatureRoute {
                centerline: vec![first_position, second_position],
                surfaces: surfaces.clone(),
            },
        );
        let mut reverse = compact_world();
        reverse.features.protected_routes.insert(
            "road".to_owned(),
            ProtectedFeatureRoute {
                centerline: vec![second_position, first_position],
                surfaces,
            },
        );

        assert_ne!(
            semantic_plan_fingerprint(&forward).expect("the forward route encodes"),
            semantic_plan_fingerprint(&reverse).expect("the reverse route encodes")
        );
    }
}
