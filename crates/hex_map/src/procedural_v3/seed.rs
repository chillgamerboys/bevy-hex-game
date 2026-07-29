//! Deterministic named streams for procedural generator V3.
//!
//! V3 owns this encoding. It deliberately does not reuse the frozen V1/V2 hash
//! helpers, so evolving V3 cannot perturb compatibility output from either older
//! generator. Streams are indexed hashes rather than mutable random-number
//! generators; sampling one stage or index therefore cannot advance another.

use hex_core::HexCoord;
use xxhash_rust::xxh3::xxh3_64;

const STREAM_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/stage-stream";
const INDEX_SAMPLE: u8 = 0;
const COORD_SAMPLE: u8 = 1;

/// Root of every deterministic V3 stream for one world candidate and patch.
///
/// Patch IDs are assigned by the resolved layout rather than by generation
/// order: `Single` uses zero, while `Ring7` uses its fixed semantic slots. This
/// keeps patch streams stable if planning or evaluation order changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeedStreams {
    world_seed: u64,
    candidate: u8,
    patch_id: u32,
}

impl SeedStreams {
    /// Creates the stream namespace for one world seed, candidate, and stable patch.
    #[must_use]
    pub(crate) const fn new(world_seed: u64, candidate: u8, patch_id: u32) -> Self {
        Self {
            world_seed,
            candidate,
            patch_id,
        }
    }

    /// Derives an independently indexed semantic stage.
    #[must_use]
    pub(crate) const fn stage(self, name: &str) -> SeedStream<'_> {
        SeedStream {
            world_seed: self.world_seed,
            candidate: self.candidate,
            patch_id: self.patch_id,
            name,
        }
    }
}

/// One named, call-order-independent source of deterministic V3 values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SeedStream<'a> {
    world_seed: u64,
    candidate: u8,
    patch_id: u32,
    name: &'a str,
}

impl SeedStream<'_> {
    /// Samples one stable index.
    ///
    /// The hashed bytes are, in order: the fixed V3 domain, world seed,
    /// candidate, patch ID, stage byte length, stage bytes, sample kind, and
    /// sample index. Every integer wider than one byte is encoded little-endian.
    #[must_use]
    pub(crate) fn sample(self, index: u64) -> u64 {
        xxh3_64(&self.encode_index_sample(index))
    }

    /// Samples one exact horizontal coordinate and an optional local salt.
    ///
    /// Coordinate samples have their own discriminant and encode all three cube
    /// coordinates explicitly. They cannot alias an indexed sample with the same
    /// trailing bytes.
    #[must_use]
    pub(crate) fn sample_coord(self, coord: HexCoord, salt: u64) -> u64 {
        let mut bytes = self.encode_prefix(COORD_SAMPLE, 20);
        bytes.extend_from_slice(&coord.x().to_le_bytes());
        bytes.extend_from_slice(&coord.y().to_le_bytes());
        bytes.extend_from_slice(&coord.z().to_le_bytes());
        bytes.extend_from_slice(&salt.to_le_bytes());
        xxh3_64(&bytes)
    }

    /// Maps an indexed sample into an inclusive signed integer range.
    pub(crate) fn range_i32(self, index: u64, min: i32, max: i32) -> Result<i32, String> {
        if min > max {
            return Err(format!("invalid deterministic range {min}..={max}"));
        }

        let span = u64::from(min.abs_diff(max)).saturating_add(1);
        let sampled = self.sample(index) % span;
        let offset = u32::try_from(sampled).map_err(|error| {
            format!("deterministic range {min}..={max} exceeds i32 capacity: {error}")
        })?;
        Ok(min.saturating_add_unsigned(offset))
    }

    fn encode_index_sample(self, index: u64) -> Vec<u8> {
        let mut bytes = self.encode_prefix(INDEX_SAMPLE, 8);
        bytes.extend_from_slice(&index.to_le_bytes());
        bytes
    }

    fn encode_prefix(self, sample_kind: u8, payload_len: usize) -> Vec<u8> {
        let stage_len = u64::try_from(self.name.len()).unwrap_or(u64::MAX);
        let capacity = STREAM_DOMAIN
            .len()
            .saturating_add(8)
            .saturating_add(1)
            .saturating_add(4)
            .saturating_add(8)
            .saturating_add(self.name.len())
            .saturating_add(1)
            .saturating_add(payload_len);
        let mut bytes = Vec::with_capacity(capacity);

        bytes.extend_from_slice(STREAM_DOMAIN);
        bytes.extend_from_slice(&self.world_seed.to_le_bytes());
        bytes.push(self.candidate);
        bytes.extend_from_slice(&self.patch_id.to_le_bytes());
        bytes.extend_from_slice(&stage_len.to_le_bytes());
        bytes.extend_from_slice(self.name.as_bytes());
        bytes.push(sample_kind);
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V2_FROZEN_SAMPLE: u64 = 6_609_271_780_027_420_460;

    #[test]
    fn every_stream_identity_field_is_independent() {
        let baseline = SeedStreams::new(77, 3, 5).stage("hills.relief").sample(11);

        assert_ne!(
            SeedStreams::new(78, 3, 5).stage("hills.relief").sample(11),
            baseline
        );
        assert_ne!(
            SeedStreams::new(77, 4, 5).stage("hills.relief").sample(11),
            baseline
        );
        assert_ne!(
            SeedStreams::new(77, 3, 6).stage("hills.relief").sample(11),
            baseline
        );
        assert_ne!(
            SeedStreams::new(77, 3, 5).stage("hills.river").sample(11),
            baseline
        );
        assert_ne!(
            SeedStreams::new(77, 3, 5).stage("hills.relief").sample(12),
            baseline
        );
    }

    #[test]
    fn samples_do_not_depend_on_call_order() {
        let stream = SeedStreams::new(91, 2, 4).stage("forest.features");
        let expected_zero = stream.sample(0);
        let expected_seven = stream.sample(7);
        let expected_large = stream.sample(u64::MAX);

        let reverse = (stream.sample(u64::MAX), stream.sample(7), stream.sample(0));

        assert_eq!(
            reverse,
            (expected_large, expected_seven, expected_zero),
            "indexed hashes must not carry mutable stream state"
        );
    }

    #[test]
    fn coordinate_and_salt_samples_are_independent_and_order_free() {
        let stream = SeedStreams::new(19, 2, 6).stage("caves.chambers");
        let a = HexCoord::from_axial(-3, 7);
        let b = HexCoord::from_axial(4, -1);

        let first = (stream.sample_coord(a, 0), stream.sample_coord(b, 0));
        let second = (stream.sample_coord(b, 0), stream.sample_coord(a, 0));

        assert_eq!(first, (second.1, second.0));
        assert_ne!(stream.sample_coord(a, 0), stream.sample_coord(a, 1));
        assert_ne!(stream.sample_coord(a, 0), stream.sample_coord(b, 0));
        assert_ne!(stream.sample_coord(a, 0), stream.sample(0));
    }

    #[test]
    fn length_prefixed_stage_names_resist_boundary_ambiguity() {
        let streams = SeedStreams::new(11, 1, 3);
        let a_bc = streams.stage("a/bc");
        let ab_c = streams.stage("ab/c");

        assert_ne!(a_bc.encode_index_sample(9), ab_c.encode_index_sample(9));
        assert_ne!(a_bc.sample(9), ab_c.sample(9));

        let mut expected = Vec::new();
        expected.extend_from_slice(STREAM_DOMAIN);
        expected.extend_from_slice(&11_u64.to_le_bytes());
        expected.push(1);
        expected.extend_from_slice(&3_u32.to_le_bytes());
        expected.extend_from_slice(&4_u64.to_le_bytes());
        expected.extend_from_slice(b"a/bc");
        expected.push(INDEX_SAMPLE);
        expected.extend_from_slice(&9_u64.to_le_bytes());
        assert_eq!(a_bc.encode_index_sample(9), expected);
    }

    #[test]
    fn inclusive_ranges_validate_bounds_and_extremes() {
        let stream = SeedStreams::new(1, 0, 0).stage("range");

        assert_eq!(stream.range_i32(0, 5, 5), Ok(5));
        assert!(stream.range_i32(0, 6, 5).is_err());
        for index in 0..100 {
            assert!((-4..=9).contains(&stream.range_i32(index, -4, 9).expect("the range is valid")));
            assert!(
                stream.range_i32(index, i32::MIN, i32::MAX).is_ok(),
                "the full i32 range is valid"
            );
        }
    }

    #[test]
    fn indexed_stage_samples_match_the_v3_golden() {
        let sample = SeedStreams::new(1_592_598_566, 7, 4)
            .stage("hills.lobe.axis")
            .sample(11);

        assert_eq!(
            sample, 14_655_107_024_619_543_112,
            "update only with an explicit V3 stream-encoding decision"
        );
        assert_ne!(
            sample, V2_FROZEN_SAMPLE,
            "V3 must remain isolated from the frozen V1/V2 stream contract"
        );
    }
}
