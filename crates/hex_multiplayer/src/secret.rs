//! Fixed-size credentials that cannot disclose themselves through `Debug`.

use std::{
    fmt,
    hash::{Hash, Hasher},
};

use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};

/// One-time lobby admission secret carried by a direct connection code.
#[derive(Clone, Copy, Eq, Serialize, Deserialize)]
pub struct InviteToken([u8; Self::BYTE_LENGTH]);

impl InviteToken {
    /// Number of random bytes in an invite token (128 bits).
    pub const BYTE_LENGTH: usize = 16;

    /// Generates a token from the operating system's cryptographic random source.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; Self::BYTE_LENGTH];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Constructs a token from exact bytes, primarily for decoding and tests.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Copies the token bytes for authenticated transport encoding.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::BYTE_LENGTH] {
        self.0
    }

    /// Compares a presented token without data-dependent early return.
    #[must_use]
    pub fn matches(self, presented: Self) -> bool {
        constant_time_equal(&self.0, &presented.0)
    }
}

impl fmt::Debug for InviteToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("InviteToken([REDACTED])")
    }
}

impl PartialEq for InviteToken {
    fn eq(&self, other: &Self) -> bool {
        constant_time_equal(&self.0, &other.0)
    }
}

impl Hash for InviteToken {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

/// Private rotating credential used only by an already-admitted reconnecting player.
#[derive(Clone, Copy, Eq, Serialize, Deserialize)]
pub struct ReconnectCredential([u8; Self::BYTE_LENGTH]);

impl ReconnectCredential {
    /// Number of random bytes in a reconnect credential (256 bits).
    pub const BYTE_LENGTH: usize = 32;

    /// Generates a credential from the operating system's cryptographic random source.
    #[must_use]
    pub fn generate() -> Self {
        let mut bytes = [0_u8; Self::BYTE_LENGTH];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    /// Constructs a credential from exact bytes, primarily for storage adapters and tests.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Copies the credential bytes for encrypted transport or atomic storage.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::BYTE_LENGTH] {
        self.0
    }

    /// Compares a presented credential without data-dependent early return.
    #[must_use]
    pub fn matches(self, presented: Self) -> bool {
        constant_time_equal(&self.0, &presented.0)
    }
}

impl fmt::Debug for ReconnectCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReconnectCredential([REDACTED])")
    }
}

impl PartialEq for ReconnectCredential {
    fn eq(&self, other: &Self) -> bool {
        constant_time_equal(&self.0, &other.0)
    }
}

impl Hash for ReconnectCredential {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (&left_byte, &right_byte) in left.iter().zip(right) {
        difference |= left_byte ^ right_byte;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_output_is_redacted() {
        let invite = InviteToken::from_bytes([7; InviteToken::BYTE_LENGTH]);
        let reconnect = ReconnectCredential::from_bytes([9; ReconnectCredential::BYTE_LENGTH]);
        assert_eq!(format!("{invite:?}"), "InviteToken([REDACTED])");
        assert_eq!(format!("{reconnect:?}"), "ReconnectCredential([REDACTED])");
        assert!(!format!("{invite:?}").contains('7'));
        assert!(!format!("{reconnect:?}").contains('9'));
    }

    #[test]
    fn credential_matching_distinguishes_bytes() {
        let first = ReconnectCredential::from_bytes([1; ReconnectCredential::BYTE_LENGTH]);
        let same = ReconnectCredential::from_bytes([1; ReconnectCredential::BYTE_LENGTH]);
        let different = ReconnectCredential::from_bytes([2; ReconnectCredential::BYTE_LENGTH]);
        assert!(first.matches(same));
        assert!(!first.matches(different));
    }
}
