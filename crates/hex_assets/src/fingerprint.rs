//! Small deterministic encoder for derived-content source identities.
//!
//! `DefaultHasher` intentionally does not promise stable output across Rust releases.
//! Content revisions need a value that is reproducible across runs and platforms, so
//! the asset pipeline uses domain-separated FNV-1a over an explicitly ordered byte
//! representation instead.

/// Incremental, deterministic semantic fingerprint encoder.
pub(crate) struct FingerprintEncoder {
    hash: u64,
}

impl FingerprintEncoder {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut encoder = Self {
            hash: Self::OFFSET_BASIS,
        };
        encoder.bytes(domain);
        encoder
    }

    pub(crate) fn bytes(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(Self::PRIME);
        }
    }

    pub(crate) fn bool(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.bytes(&[value]);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    pub(crate) fn usize(&mut self, value: usize) {
        self.u64(u64::try_from(value).unwrap_or(u64::MAX));
    }

    pub(crate) fn string(&mut self, value: &str) {
        self.usize(value.len());
        self.bytes(value.as_bytes());
    }

    pub(crate) fn f32(&mut self, value: f32) {
        // Signed zero has identical authored/runtime meaning and should not create a
        // distinct content revision.
        self.u32(if value == 0.0 { 0 } else { value.to_bits() });
    }

    pub(crate) const fn finish(self) -> u64 {
        self.hash
    }
}
