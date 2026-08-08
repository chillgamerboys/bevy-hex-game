//! Versioned direct-connect codes with explicit endpoint and certificate identity.

use std::fmt;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};

use crate::{
    limits::{BoundError, BoundedText, MAX_ADVERTISED_HOST_BYTES, MAX_CONNECTION_CODE_BYTES},
    InviteToken,
};

const CONNECTION_CODE_PREFIX: &str = "HEX1.";

/// SHA-256 digest carried by a connection code to pin the host certificate.
///
/// The 32-byte shape is stable. Whether the digest covers the SPKI or the complete leaf
/// certificate is deliberately not encoded until the audited verifier choice is ratified.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CertificateFingerprint([u8; Self::BYTE_LENGTH]);

impl CertificateFingerprint {
    /// SHA-256 digest length.
    pub const BYTE_LENGTH: usize = 32;

    /// Constructs a fingerprint from exact SHA-256 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; Self::BYTE_LENGTH]) -> Self {
        Self(bytes)
    }

    /// Returns the exact digest bytes.
    #[must_use]
    pub const fn to_bytes(self) -> [u8; Self::BYTE_LENGTH] {
        self.0
    }
}

impl fmt::Debug for CertificateFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("CertificateFingerprint(")?;
        for byte in self.0.iter().take(4) {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str("…)")
    }
}

/// Advertised hostname/IP and UDP port for a direct host.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DirectEndpoint {
    host: BoundedText<MAX_ADVERTISED_HOST_BYTES>,
    port: u16,
}

impl DirectEndpoint {
    /// Validates a DNS name or textual IP literal and non-zero UDP port.
    pub fn new(host: impl Into<String>, port: u16) -> Result<Self, ConnectionCodeError> {
        let host = BoundedText::new(host).map_err(ConnectionCodeError::InvalidHostText)?;
        if !host.as_str().is_ascii()
            || !host.as_str().chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '-' | ':' | '[' | ']' | '%' | '_')
            })
        {
            return Err(ConnectionCodeError::InvalidHostSyntax);
        }
        if port == 0 {
            return Err(ConnectionCodeError::InvalidPort);
        }
        Ok(Self { host, port })
    }

    /// Advertised DNS name or textual IP literal.
    #[must_use]
    pub fn host(&self) -> &str {
        self.host.as_str()
    }

    /// Advertised UDP port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// Decoded direct-connect information.
#[derive(Clone, PartialEq, Eq)]
pub struct DirectConnectionCode {
    /// Advertised host endpoint.
    pub endpoint: DirectEndpoint,
    /// Pinned certificate digest.
    pub certificate_fingerprint: CertificateFingerprint,
    /// One-time lobby admission secret.
    pub invite_token: InviteToken,
}

impl DirectConnectionCode {
    /// Encodes this payload as `HEX1.<base64url>` without padding.
    #[must_use]
    pub fn encode(&self) -> EncodedConnectionCode {
        let host = self.endpoint.host().as_bytes();
        let mut payload = Vec::with_capacity(
            2 + host.len() + 2 + CertificateFingerprint::BYTE_LENGTH + InviteToken::BYTE_LENGTH,
        );
        let host_length = u16::try_from(host.len()).unwrap_or(u16::MAX);
        payload.extend_from_slice(&host_length.to_be_bytes());
        payload.extend_from_slice(host);
        payload.extend_from_slice(&self.endpoint.port().to_be_bytes());
        payload.extend_from_slice(&self.certificate_fingerprint.to_bytes());
        payload.extend_from_slice(&self.invite_token.to_bytes());
        EncodedConnectionCode(format!(
            "{CONNECTION_CODE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(payload)
        ))
    }

    /// Parses and validates a version-1 code from user input.
    pub fn parse(encoded: &str) -> Result<Self, ConnectionCodeError> {
        if encoded.len() > MAX_CONNECTION_CODE_BYTES {
            return Err(ConnectionCodeError::CodeTooLong);
        }
        let body = encoded
            .strip_prefix(CONNECTION_CODE_PREFIX)
            .ok_or(ConnectionCodeError::WrongVersion)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(body)
            .map_err(|_decode_error| ConnectionCodeError::InvalidBase64)?;
        let mut remaining = decoded.as_slice();

        let host_length = usize::from(u16::from_be_bytes(take_array::<2>(&mut remaining)?));
        if host_length == 0 || host_length > MAX_ADVERTISED_HOST_BYTES {
            return Err(ConnectionCodeError::InvalidHostLength);
        }
        let (host_bytes, tail) = remaining
            .split_at_checked(host_length)
            .ok_or(ConnectionCodeError::Truncated)?;
        remaining = tail;
        let host = std::str::from_utf8(host_bytes)
            .map_err(|_utf8_error| ConnectionCodeError::InvalidUtf8)?;
        let port = u16::from_be_bytes(take_array::<2>(&mut remaining)?);
        let certificate_fingerprint =
            CertificateFingerprint::from_bytes(take_array::<32>(&mut remaining)?);
        let invite_token = InviteToken::from_bytes(take_array::<16>(&mut remaining)?);
        if !remaining.is_empty() {
            return Err(ConnectionCodeError::TrailingData);
        }

        Ok(Self {
            endpoint: DirectEndpoint::new(host, port)?,
            certificate_fingerprint,
            invite_token,
        })
    }
}

impl fmt::Debug for DirectConnectionCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DirectConnectionCode")
            .field("endpoint", &self.endpoint)
            .field("certificate_fingerprint", &self.certificate_fingerprint)
            .field("invite_token", &self.invite_token)
            .finish()
    }
}

/// An encoded connection code whose ordinary formatting is always redacted.
#[derive(Clone, PartialEq, Eq)]
pub struct EncodedConnectionCode(String);

impl EncodedConnectionCode {
    /// Borrows the complete code for the explicit copy/share UI only.
    #[must_use]
    pub fn expose_for_sharing(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for EncodedConnectionCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EncodedConnectionCode([REDACTED])")
    }
}

/// Why a direct connection code was rejected before any network action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionCodeError {
    /// The input does not use the supported `HEX1` format.
    WrongVersion,
    /// The encoded input exceeds the defensive size cap.
    CodeTooLong,
    /// The payload is not unpadded URL-safe Base64.
    InvalidBase64,
    /// The decoded payload ended before a required field.
    Truncated,
    /// The advertised host length is empty or too large.
    InvalidHostLength,
    /// Host bytes are not UTF-8.
    InvalidUtf8,
    /// The host failed bounded-text validation.
    InvalidHostText(BoundError),
    /// The host contains syntax outside conservative DNS/IP-literal characters.
    InvalidHostSyntax,
    /// UDP port zero is never a joinable advertised endpoint.
    InvalidPort,
    /// Bytes remained after the complete version-1 payload.
    TrailingData,
}

impl fmt::Display for ConnectionCodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::WrongVersion => "unsupported direct connection code version",
            Self::CodeTooLong => "direct connection code exceeds its size limit",
            Self::InvalidBase64 => "direct connection code has invalid base64url data",
            Self::Truncated => "direct connection code is truncated",
            Self::InvalidHostLength => "direct connection code has an invalid host length",
            Self::InvalidUtf8 => "direct connection code host is not UTF-8",
            Self::InvalidHostText(_) => "direct connection code host violates its text bound",
            Self::InvalidHostSyntax => "direct connection code host has invalid syntax",
            Self::InvalidPort => "direct connection code port must be non-zero",
            Self::TrailingData => "direct connection code contains trailing data",
        })
    }
}

impl std::error::Error for ConnectionCodeError {}

fn take_array<const LENGTH: usize>(bytes: &mut &[u8]) -> Result<[u8; LENGTH], ConnectionCodeError> {
    let (head, tail) = bytes
        .split_at_checked(LENGTH)
        .ok_or(ConnectionCodeError::Truncated)?;
    *bytes = tail;
    head.try_into()
        .map_err(|_length_error| ConnectionCodeError::Truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> DirectConnectionCode {
        DirectConnectionCode {
            endpoint: DirectEndpoint::new("example.test", 7777).expect("valid endpoint"),
            certificate_fingerprint: CertificateFingerprint::from_bytes([3; 32]),
            invite_token: InviteToken::from_bytes([5; 16]),
        }
    }

    #[test]
    fn version_one_code_round_trips_and_debug_is_redacted() {
        let original = fixture();
        let encoded = original.encode();
        let decoded = DirectConnectionCode::parse(encoded.expose_for_sharing())
            .expect("generated code should parse");
        assert_eq!(decoded, original);
        assert_eq!(format!("{encoded:?}"), "EncodedConnectionCode([REDACTED])");
        assert!(!format!("{original:?}").contains("05050505"));
    }

    #[test]
    fn malformed_payloads_are_rejected_without_panicking() {
        for length in 0..128 {
            let bytes = vec![u8::try_from(length).unwrap_or(u8::MAX); length];
            let candidate = format!("HEX1.{}", URL_SAFE_NO_PAD.encode(bytes));
            assert!(DirectConnectionCode::parse(&candidate).is_err());
        }
        assert_eq!(
            DirectConnectionCode::parse("HEX2.invalid"),
            Err(ConnectionCodeError::WrongVersion)
        );
    }
}
