//! Canonical semantic fingerprints for validated schematic plans.
//!
//! The encoder is deliberately schema-driven. Collections have no implicit
//! representation: callers write their semantic count and canonical order so a
//! container implementation cannot change plan identity.

use xxhash_rust::xxh3::xxh3_64;

use crate::model::{
    AccessIntent, ClimateKind, FeatureClaim, FeatureKind, LandformKind, Network, NetworkKind,
    NetworkNodeKind, SchematicPlanV1, SurfaceKind, VegetationDensity,
};

const FINGERPRINT_DOMAIN: &[u8] = b"bevy-hex-game/schematic/semantic-plan/v1";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct FingerprintEncoder {
    bytes: Vec<u8>,
}

impl FingerprintEncoder {
    const fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn tag(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn u32(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn i32(&mut self, value: i32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn coord(&mut self, q: i32, r: i32, s: i32) {
        self.i32(q);
        self.i32(r);
        self.i32(s);
    }

    fn string(&mut self, value: &str) {
        self.length(value.len());
        self.bytes.extend_from_slice(value.as_bytes());
    }

    fn collection(&mut self, count: usize) {
        self.length(count);
    }

    fn length(&mut self, value: usize) {
        self.u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    fn finish(self) -> u64 {
        let mut framed = Vec::with_capacity(
            FINGERPRINT_DOMAIN
                .len()
                .saturating_add(self.bytes.len())
                .saturating_add(16),
        );
        framed.extend_from_slice(
            &u64::try_from(FINGERPRINT_DOMAIN.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        framed.extend_from_slice(FINGERPRINT_DOMAIN);
        framed.extend_from_slice(
            &u64::try_from(self.bytes.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        framed.extend_from_slice(&self.bytes);
        xxh3_64(&framed)
    }
}

/// Computes the canonical semantic fingerprint of one schematic plan.
///
/// Candidate choice, world seed, and all layer provenance are deliberately
/// omitted. Two plans which resolve to the same geography and semantic
/// networks therefore have the same fingerprint even when they were reached
/// by different attempts or by the reference fallback.
#[must_use]
pub fn semantic_fingerprint(plan: &SchematicPlanV1) -> u64 {
    let mut encoder = FingerprintEncoder::new();
    encoder.u32(u32::from(plan.schema_version));
    encoder.string(plan.template_id.as_str());
    encoder.u32(plan.template_revision);
    encoder.u8(plan.radius);

    let mut cells = plan.cells.iter().collect::<Vec<_>>();
    cells.sort_unstable_by_key(|cell| cell.id);
    encoder.collection(cells.len());
    for cell in cells {
        encoder.u32(u32::from(cell.id.get()));
        encoder.coord(cell.coord.q(), cell.coord.r(), cell.coord.s());
        encoder.tag(surface_tag(cell.facts.surface));
        encoder.tag(landform_tag(cell.facts.landform));
        encoder.tag(climate_tag(cell.facts.climate));
        encoder.tag(vegetation_tag(cell.facts.vegetation));
        encoder.tag(access_tag(cell.facts.access));
        let mut overlays = cell.facts.overlays.clone();
        overlays.sort_unstable();
        encoder.collection(overlays.len());
        for overlay in overlays {
            encoder.tag(feature_tag(overlay));
        }
    }

    let mut features = plan.features.iter().collect::<Vec<_>>();
    features.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    encoder.collection(features.len());
    for feature in features {
        encode_feature(&mut encoder, feature);
    }

    let mut networks = plan.networks.iter().collect::<Vec<_>>();
    networks.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    encoder.collection(networks.len());
    for network in networks {
        encode_network(&mut encoder, network);
    }
    encoder.finish()
}

fn encode_feature(encoder: &mut FingerprintEncoder, feature: &FeatureClaim) {
    encoder.string(feature.id.as_str());
    encoder.tag(feature_tag(feature.kind));
    let mut cells = feature.cells.clone();
    cells.sort_unstable();
    encoder.collection(cells.len());
    for cell in cells {
        encoder.coord(cell.q(), cell.r(), cell.s());
    }
}

fn encode_network(encoder: &mut FingerprintEncoder, network: &Network) {
    encoder.string(network.id.as_str());
    encoder.tag(network_tag(network.kind));

    let mut nodes = network.nodes.iter().collect::<Vec<_>>();
    nodes.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    encoder.collection(nodes.len());
    for node in nodes {
        encoder.string(node.id.as_str());
        encoder.tag(node_tag(node.kind));
        encoder.coord(node.coord.q(), node.coord.r(), node.coord.s());
    }

    let mut edges = network.edges.iter().collect::<Vec<_>>();
    edges.sort_unstable_by(|left, right| left.id.cmp(&right.id));
    encoder.collection(edges.len());
    for edge in edges {
        encoder.string(edge.id.as_str());
        encoder.string(edge.from.as_str());
        encoder.string(edge.to.as_str());
        encoder.collection(edge.path.len());
        for cell in &edge.path {
            encoder.coord(cell.q(), cell.r(), cell.s());
        }
    }
}

const fn surface_tag(value: SurfaceKind) -> u8 {
    match value {
        SurfaceKind::Land => 0,
        SurfaceKind::OpenWater => 1,
    }
}

const fn landform_tag(value: LandformKind) -> u8 {
    match value {
        LandformKind::None => 0,
        LandformKind::Island => 1,
        LandformKind::Beach => 2,
        LandformKind::Shore => 3,
        LandformKind::Valley => 4,
        LandformKind::Plateau => 5,
        LandformKind::Hill => 6,
        LandformKind::Mountain => 7,
        LandformKind::Massif => 8,
        LandformKind::SharpPeak => 9,
    }
}

const fn climate_tag(value: ClimateKind) -> u8 {
    match value {
        ClimateKind::Marine => 0,
        ClimateKind::Temperate => 1,
        ClimateKind::Alpine => 2,
        ClimateKind::Frozen => 3,
    }
}

const fn vegetation_tag(value: VegetationDensity) -> u8 {
    match value {
        VegetationDensity::None => 0,
        VegetationDensity::Sparse => 1,
        VegetationDensity::Light => 2,
        VegetationDensity::Moderate => 3,
        VegetationDensity::Dense => 4,
    }
}

const fn access_tag(value: AccessIntent) -> u8 {
    match value {
        AccessIntent::Ordinary => 0,
        AccessIntent::Scenic => 1,
        AccessIntent::Inaccessible => 2,
    }
}

const fn feature_tag(value: FeatureKind) -> u8 {
    match value {
        FeatureKind::Coastline => 0,
        FeatureKind::River => 1,
        FeatureKind::Waterfall => 2,
        FeatureKind::ValleyLake => 3,
        FeatureKind::MountainLake => 4,
        FeatureKind::LakeIsland => 5,
        FeatureKind::FrozenWoods => 6,
        FeatureKind::PeakRing => 7,
        FeatureKind::CrystalAscent => 8,
        FeatureKind::Tunnel => 9,
        FeatureKind::SeaIsland => 10,
    }
}

const fn network_tag(value: NetworkKind) -> u8 {
    match value {
        NetworkKind::Hydrology => 0,
        NetworkKind::Tunnel => 1,
    }
}

const fn node_tag(value: NetworkNodeKind) -> u8 {
    match value {
        NetworkNodeKind::Source => 0,
        NetworkNodeKind::Junction => 1,
        NetworkNodeKind::Sink => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        canonical_coordinates, AccessIntent, CellFacts, CellId, CellPlan, CellProvenance,
        LayerProvenance, PlanProvenance, SchematicPlanParts, StableId,
    };

    #[test]
    fn primitive_encoding_is_explicit_and_stable() {
        let mut encoder = FingerprintEncoder::new();
        encoder.tag(7);
        encoder.u8(9);
        encoder.u32(0x1234_5678);
        encoder.u64(0x0123_4567_89ab_cdef);
        encoder.i32(-41);
        encoder.coord(2, -3, 1);
        encoder.string("grand-v3");
        encoder.collection(217);

        let first = encoder.clone().finish();
        assert_eq!(first, encoder.finish());
    }

    #[test]
    fn boundaries_prevent_concatenation_aliases() {
        let mut split_after_first = FingerprintEncoder::new();
        split_after_first.string("a");
        split_after_first.string("bc");

        let mut split_after_second = FingerprintEncoder::new();
        split_after_second.string("ab");
        split_after_second.string("c");

        assert_ne!(split_after_first.finish(), split_after_second.finish());
    }

    #[test]
    fn semantic_identity_excludes_seed_candidate_and_provenance(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let first = sample_plan(7, 0, VegetationDensity::Light, "stream/landforms")?;
        let second = sample_plan(99, 31, VegetationDensity::Light, "stream/coastline")?;
        if semantic_fingerprint(&first) != semantic_fingerprint(&second) {
            return Err("seed, candidate, or provenance leaked into semantic identity".into());
        }

        let changed = sample_plan(7, 0, VegetationDensity::Dense, "stream/landforms")?;
        if semantic_fingerprint(&first) == semantic_fingerprint(&changed) {
            return Err("semantic vegetation change did not affect plan identity".into());
        }
        Ok(())
    }

    fn sample_plan(
        world_seed: u64,
        candidate: u8,
        vegetation: VegetationDensity,
        stream: &str,
    ) -> Result<SchematicPlanV1, Box<dyn std::error::Error>> {
        let source = LayerProvenance::Seeded {
            stream: StableId::new(stream)?,
        };
        let cells = canonical_coordinates()
            .into_iter()
            .enumerate()
            .map(|(index, coord)| {
                Ok(CellPlan {
                    id: CellId::new(u16::try_from(index)?)?,
                    coord,
                    facts: CellFacts {
                        surface: SurfaceKind::Land,
                        landform: LandformKind::Hill,
                        climate: ClimateKind::Temperate,
                        vegetation,
                        access: AccessIntent::Ordinary,
                        overlays: Vec::new(),
                    },
                    provenance: CellProvenance {
                        surface: source.clone(),
                        landform: source.clone(),
                        climate: source.clone(),
                        vegetation: source.clone(),
                        access: source.clone(),
                        overlays: Vec::new(),
                    },
                })
            })
            .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
        Ok(SchematicPlanV1::new(SchematicPlanParts {
            template_id: StableId::new("template/test")?,
            template_revision: 1,
            provenance: PlanProvenance::candidate(world_seed, candidate, 32)?,
            cells,
            features: Vec::new(),
            networks: Vec::new(),
            semantic_fingerprint: 0,
        })?)
    }
}
