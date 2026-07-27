//! Canonical byte encoding and independent fingerprint domains for V3.
//!
//! Fingerprints are schema-driven: callers write fields in their documented
//! semantic order and sort unordered collections before writing them. The
//! encoder owns the byte representation of primitives, while the three final
//! domains prevent equivalent bytes at different pipeline stages from sharing
//! an identity.

use hex_core::{HexCoord, TilePos};
use xxhash_rust::xxh3::xxh3_64;

const SETTINGS_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/settings";
const SEMANTIC_PLAN_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/semantic-plan";
const MATERIALIZED_WORLD_DOMAIN: &[u8] = b"bevy-hex-game/procedural-v3/materialized-world";

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
        let encoded = u64::try_from(length)
            .map_err(|_| format!("{kind} length {length} exceeds the V3 fingerprint format"))?;
        self.u64(encoded);
        Ok(())
    }
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
    use std::collections::BTreeMap;

    use super::*;

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
}
