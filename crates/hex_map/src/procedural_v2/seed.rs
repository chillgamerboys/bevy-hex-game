//! Deterministic named streams for procedural generator V2.
//!
//! Streams are indexed hashes rather than mutable random-number generators. Sampling
//! one stage can therefore never shift another stage's output, and adding a sample at
//! index N cannot change any existing index.

use hex_core::HexCoord;
use xxhash_rust::xxh3::{xxh3_64, xxh3_64_with_seed};

const STREAM_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v2/stage";

/// Root of every deterministic stream for one candidate.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeedStreams {
    seed: u64,
    candidate: u8,
}

impl SeedStreams {
    /// Creates the stream namespace for one world seed and candidate.
    #[must_use]
    pub(crate) const fn new(seed: u64, candidate: u8) -> Self {
        Self { seed, candidate }
    }

    /// Derives an independently indexed stage.
    #[must_use]
    pub(crate) const fn stage(self, name: &str) -> SeedStream<'_> {
        SeedStream {
            seed: self.seed,
            candidate: self.candidate,
            name,
        }
    }
}

/// One named, call-order-independent source of deterministic values.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeedStream<'a> {
    seed: u64,
    candidate: u8,
    name: &'a str,
}

impl SeedStream<'_> {
    /// Samples one stable index.
    ///
    /// This is deliberately the frozen V1 named-hash contract. V2 Hills can therefore
    /// reproduce an approved V1 candidate exactly while every new recipe remains
    /// independent by using its own stage names.
    #[must_use]
    pub(crate) fn sample(self, index: u64) -> u64 {
        crate::procedural::named_hash(self.seed, self.candidate, self.name, index)
    }

    /// Samples one exact horizontal coordinate and an optional local salt.
    #[must_use]
    pub(crate) fn sample_coord(self, coord: HexCoord, salt: u64) -> u64 {
        let mut bytes = [0_u8; 16];
        bytes[..4].copy_from_slice(&coord.x().to_le_bytes());
        bytes[4..8].copy_from_slice(&coord.y().to_le_bytes());
        bytes[8..].copy_from_slice(&salt.to_le_bytes());
        self.sample(xxh3_64(&bytes))
    }

    /// Maps a sample into an inclusive integer range.
    pub(crate) fn range_i32(self, index: u64, min: i32, max: i32) -> Result<i32, String> {
        if min > max {
            return Err(format!("invalid deterministic range {min}..={max}"));
        }
        let span = u64::from(min.abs_diff(max)).saturating_add(1);
        let offset = u32::try_from(self.sample(index) % span).unwrap_or_default();
        Ok(min.saturating_add_unsigned(offset))
    }
}

/// Hashes deterministic V2 settings/report bytes without borrowing V1's hash domain.
#[must_use]
pub(crate) fn fingerprint(bytes: &[u8]) -> u64 {
    let domain = xxh3_64(STREAM_DOMAIN);
    xxh3_64_with_seed(bytes, domain)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_stages_and_candidates_are_independent() {
        let first = SeedStreams::new(77, 3);
        let river = first.stage("hills.river").sample(4);

        for index in 0..100 {
            let _unused = first.stage("hills.relief").sample(index);
        }

        assert_eq!(first.stage("hills.river").sample(4), river);
        assert_ne!(
            SeedStreams::new(77, 4).stage("hills.river").sample(4),
            river
        );
        assert_ne!(first.stage("hills.river").sample(5), river);
    }

    #[test]
    fn seed_bits_cannot_alias_candidate_identity() {
        let shifted_seed = SeedStreams::new(1_u64 << 56, 0)
            .stage("hills.river")
            .sample(0);
        let shifted_candidate = SeedStreams::new(0, 1).stage("hills.river").sample(0);

        assert_ne!(shifted_seed, shifted_candidate);
    }

    #[test]
    fn coordinate_samples_are_order_independent() {
        let stream = SeedStreams::new(19, 2).stage("caves.chambers");
        let a = HexCoord::from_axial(-3, 7);
        let b = HexCoord::from_axial(4, -1);

        let first = (stream.sample_coord(a, 0), stream.sample_coord(b, 0));
        let second = (stream.sample_coord(b, 0), stream.sample_coord(a, 0));

        assert_eq!(first, (second.1, second.0));
        assert_ne!(stream.sample_coord(a, 0), stream.sample_coord(a, 1));
    }

    #[test]
    fn inclusive_ranges_validate_and_reach_singletons() {
        let stream = SeedStreams::new(1, 0).stage("range");

        assert_eq!(stream.range_i32(0, 5, 5), Ok(5));
        assert!(stream.range_i32(0, 6, 5).is_err());
        for index in 0..100 {
            assert!((-4..=9).contains(&stream.range_i32(index, -4, 9).expect("the range is valid")));
        }
    }

    #[test]
    fn indexed_stage_samples_match_the_frozen_v1_contract() {
        let seed = 1_592_598_566;
        let candidate = 7;
        let stage = "hills.lobe.axis";
        let index = 11;
        let sample = SeedStreams::new(seed, candidate).stage(stage).sample(index);

        assert_eq!(
            sample,
            crate::procedural::named_hash(seed, candidate, stage, index),
            "V2 Hills must be able to reproduce V1 stage samples exactly"
        );
        assert_eq!(
            sample, 6_609_271_780_027_420_460,
            "update only with an explicit V1 compatibility-version decision"
        );
    }
}
