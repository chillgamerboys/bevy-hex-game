//! Explicit native WebTransport host/join preparation with SPKI pinning.

use std::{fmt, sync::Arc, time::Duration};

use aeronet_replicon::{client::AeronetRepliconClient, server::AeronetRepliconServer};
use aeronet_webtransport::{
    client::{ClientConfig, WebTransportClient},
    server::{ServerConfig, SessionRequest, SessionResponse, WebTransportServer},
    wtransport::{
        self,
        tls::{self, rustls},
    },
};
use bevy_ecs::{
    prelude::{Entity, On, World},
    system::EntityCommand as _,
};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::WebPkiSupportedAlgorithms,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest as _, Sha256};
use x509_parser::{
    oid_registry::{OID_EC_P256, OID_KEY_TYPE_EC_PUBLIC_KEY},
    prelude::{FromDer as _, X509Certificate},
};

use crate::{CertificateFingerprint, DirectConnectionCode, DirectEndpoint, InviteToken};

/// Default editable UDP port shown by Host Direct.
pub const DEFAULT_DIRECT_PORT: u16 = 7777;
/// Exact WebTransport application path accepted by Direct Connect hosts.
pub const DIRECT_SESSION_PATH: &str = "/hex1";
const KEEP_ALIVE: Duration = Duration::from_secs(1);
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CERTIFICATE_LIFETIME_SECS: i64 = 14 * 24 * 60 * 60;

/// Prepared direct-listen endpoint and its redacted share code.
///
/// Construction generates a fresh P-256 identity for this session. Calling [`Self::open`]
/// is the explicit state-changing action that binds the UDP socket.
pub struct PreparedDirectHost {
    config: ServerConfig,
    connection_code: DirectConnectionCode,
}

impl PreparedDirectHost {
    /// Generates one per-session certificate and WebTransport server configuration.
    pub fn new(
        advertised_endpoint: DirectEndpoint,
        invite_token: InviteToken,
    ) -> Result<Self, DirectTransportError> {
        let (identity, fingerprint) = generate_identity(&advertised_endpoint)?;
        let config = ServerConfig::builder()
            .with_bind_default(advertised_endpoint.port())
            .with_identity(identity)
            .keep_alive_interval(Some(KEEP_ALIVE))
            .max_idle_timeout(Some(IDLE_TIMEOUT))
            .map_err(|_error| DirectTransportError::InvalidIdleTimeout)?
            .build();
        Ok(Self {
            config,
            connection_code: DirectConnectionCode {
                endpoint: advertised_endpoint,
                certificate_fingerprint: fingerprint,
                invite_token,
            },
        })
    }

    /// Direct connection code shown only by the explicit copy/share UI.
    #[must_use]
    pub const fn connection_code(&self) -> &DirectConnectionCode {
        &self.connection_code
    }

    /// Opens the prepared server and marks it as a Replicon backend.
    pub fn open(self, world: &mut World) -> Entity {
        let server = world.spawn(AeronetRepliconServer).id();
        WebTransportServer::open(self.config).apply(world.entity_mut(server));
        server
    }
}

impl fmt::Debug for PreparedDirectHost {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedDirectHost")
            .field("connection_code", &self.connection_code)
            .field("config", &"[WEBTRANSPORT SERVER CONFIG]")
            .finish()
    }
}

/// Prepared pinned direct client connection.
///
/// Calling [`Self::connect`] is the explicit state-changing action that creates the
/// outgoing socket. The invite token is returned separately for the subsequent ordered
/// `ClientHello`; it is never put in a URL or transport header.
pub struct PreparedDirectJoin {
    config: ClientConfig,
    target: String,
    invite_token: InviteToken,
}

impl PreparedDirectJoin {
    /// Parses a share code and configures mandatory SPKI validation.
    pub fn new(connection_code: &DirectConnectionCode) -> Result<Self, DirectTransportError> {
        let verifier = Arc::new(SpkiPinVerifier::new(
            connection_code.certificate_fingerprint,
        ));
        let tls_config = tls::client::build_default_tls_config(
            Arc::new(rustls::RootCertStore::empty()),
            Some(verifier),
        );
        let config = ClientConfig::builder()
            .with_bind_default()
            .with_custom_tls(tls_config)
            .keep_alive_interval(Some(KEEP_ALIVE))
            .max_idle_timeout(Some(IDLE_TIMEOUT))
            .map_err(|_error| DirectTransportError::InvalidIdleTimeout)?
            .build();
        Ok(Self {
            config,
            target: direct_target(&connection_code.endpoint),
            invite_token: connection_code.invite_token,
        })
    }

    /// Invitation to place in the first ordered `ClientHello` after connection.
    #[must_use]
    pub const fn invite_token(&self) -> InviteToken {
        self.invite_token
    }

    /// Creates the outgoing WebTransport session and marks it as Replicon's client.
    pub fn connect(self, world: &mut World) -> Entity {
        let client = world.spawn(AeronetRepliconClient).id();
        WebTransportClient::connect(self.config, self.target).apply(world.entity_mut(client));
        client
    }
}

impl fmt::Debug for PreparedDirectJoin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedDirectJoin")
            .field("target", &self.target)
            .field("invite_token", &self.invite_token)
            .field("config", &"[PINNED WEBTRANSPORT CLIENT CONFIG]")
            .finish()
    }
}

/// Rustls verifier for one exact SHA-256 `SubjectPublicKeyInfo` pin.
///
/// Pinning replaces public-CA/name validation for the session's self-signed certificate,
/// but does not disable certificate checks. The verifier retains WebTransport's short-lived
/// certificate constraints and delegates TLS handshake signatures to rustls.
#[derive(Debug)]
pub struct SpkiPinVerifier {
    expected: CertificateFingerprint,
    supported_algorithms: WebPkiSupportedAlgorithms,
}

impl SpkiPinVerifier {
    /// Creates a verifier for one exact connection-code fingerprint.
    #[must_use]
    pub fn new(expected: CertificateFingerprint) -> Self {
        Self {
            expected,
            supported_algorithms: rustls::crypto::ring::default_provider()
                .signature_verification_algorithms,
        }
    }
}

impl ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        verify_pinned_certificate(self.expected, end_entity, intermediates, now)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &self.supported_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &self.supported_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported_algorithms.supported_schemes()
    }
}

fn verify_pinned_certificate(
    expected: CertificateFingerprint,
    end_entity: &CertificateDer<'_>,
    intermediates: &[CertificateDer<'_>],
    now: UnixTime,
) -> Result<(), rustls::Error> {
    if !intermediates.is_empty() {
        return Err(rustls::CertificateError::UnknownIssuer.into());
    }
    let (remaining, certificate) = X509Certificate::from_der(end_entity.as_ref())
        .map_err(|_error| rustls::CertificateError::BadEncoding)?;
    if !remaining.is_empty() {
        return Err(rustls::CertificateError::BadEncoding.into());
    }

    let now =
        i64::try_from(now.as_secs()).map_err(|_error| rustls::CertificateError::BadEncoding)?;
    let not_before = certificate.validity().not_before.timestamp();
    let not_after = certificate.validity().not_after.timestamp();
    if now < not_before {
        return Err(rustls::CertificateError::NotValidYet.into());
    }
    if now > not_after {
        return Err(rustls::CertificateError::Expired.into());
    }
    let lifetime = not_after
        .checked_sub(not_before)
        .ok_or(rustls::CertificateError::BadEncoding)?;
    if !(1..=MAX_CERTIFICATE_LIFETIME_SECS).contains(&lifetime) {
        return Err(rustls::CertificateError::UnknownIssuer.into());
    }

    let public_key = certificate.public_key();
    if public_key.algorithm.algorithm != OID_KEY_TYPE_EC_PUBLIC_KEY {
        return Err(rustls::CertificateError::UnknownIssuer.into());
    }
    if !matches!(
        public_key
            .algorithm
            .parameters
            .as_ref()
            .map(|parameters| parameters.as_oid()),
        Some(Ok(oid)) if oid == OID_EC_P256
    ) {
        return Err(rustls::CertificateError::UnknownIssuer.into());
    }

    let actual = CertificateFingerprint::from_bytes(Sha256::digest(public_key.raw).into());
    if actual != expected {
        return Err(rustls::CertificateError::UnknownIssuer.into());
    }
    Ok(())
}

fn generate_identity(
    endpoint: &DirectEndpoint,
) -> Result<(wtransport::Identity, CertificateFingerprint), DirectTransportError> {
    let san = certificate_san(endpoint.host());
    let identity = wtransport::Identity::self_signed_builder()
        .subject_alt_names([san])
        .from_now_utc()
        .validity_days(14)
        .build()
        .map_err(|_error| DirectTransportError::InvalidCertificateIdentity)?;
    let certificate = identity
        .certificate_chain()
        .as_slice()
        .first()
        .ok_or(DirectTransportError::MissingLeafCertificate)?;
    let fingerprint = spki_fingerprint(certificate.der())?;
    Ok((identity, fingerprint))
}

fn spki_fingerprint(der: &[u8]) -> Result<CertificateFingerprint, DirectTransportError> {
    let (remaining, certificate) = X509Certificate::from_der(der)
        .map_err(|_error| DirectTransportError::InvalidCertificateEncoding)?;
    if !remaining.is_empty() {
        return Err(DirectTransportError::InvalidCertificateEncoding);
    }
    Ok(CertificateFingerprint::from_bytes(
        Sha256::digest(certificate.public_key().raw).into(),
    ))
}

fn certificate_san(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|unwrapped| unwrapped.strip_suffix(']'))
        .unwrap_or(host)
}

fn direct_target(endpoint: &DirectEndpoint) -> String {
    let host = endpoint.host();
    let authority = if host.contains(':') && !(host.starts_with('[') && host.ends_with(']')) {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    format!(
        "https://{authority}:{}{DIRECT_SESSION_PATH}",
        endpoint.port()
    )
}

pub(crate) fn respond_to_direct_session(mut request: On<SessionRequest>) {
    let response = if request.path == DIRECT_SESSION_PATH {
        SessionResponse::Accepted
    } else {
        SessionResponse::NotFound
    };
    request.respond(response);
}

/// Failure while preparing a direct encrypted endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirectTransportError {
    /// The advertised hostname/IP could not form a short-lived self-signed identity.
    InvalidCertificateIdentity,
    /// Generated identity unexpectedly omitted a leaf certificate.
    MissingLeafCertificate,
    /// Generated certificate DER could not be parsed exactly.
    InvalidCertificateEncoding,
    /// A fixed idle timeout was rejected by the transport builder.
    InvalidIdleTimeout,
}

impl fmt::Display for DirectTransportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidCertificateIdentity => "could not generate the direct host identity",
            Self::MissingLeafCertificate => "direct host identity has no leaf certificate",
            Self::InvalidCertificateEncoding => "direct host certificate encoding is invalid",
            Self::InvalidIdleTimeout => "direct transport idle timeout is invalid",
        })
    }
}

impl std::error::Error for DirectTransportError {}

#[cfg(test)]
mod tests {
    use rcgen::{CertificateParams, KeyPair, PKCS_ED25519};
    use rustls::pki_types::UnixTime;
    use wtransport::tls::self_signed::time::OffsetDateTime;

    use super::*;

    fn identity_between(not_before: i64, not_after: i64) -> wtransport::Identity {
        let not_before = OffsetDateTime::from_unix_timestamp(not_before)
            .expect("test timestamp should be representable");
        let not_after = OffsetDateTime::from_unix_timestamp(not_after)
            .expect("test timestamp should be representable");
        wtransport::Identity::self_signed_builder()
            .subject_alt_names(["localhost"])
            .validity_period(not_before, not_after)
            .build()
            .expect("test identity should build")
    }

    fn verify_at(
        identity: &wtransport::Identity,
        expected: CertificateFingerprint,
        timestamp: u64,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let certificate = identity
            .certificate_chain()
            .as_slice()
            .first()
            .expect("test identity has a leaf");
        let der = CertificateDer::from(certificate.der().to_vec());
        let name = ServerName::try_from("localhost")
            .map_err(|error| rustls::Error::General(error.to_string()))?;
        SpkiPinVerifier::new(expected).verify_server_cert(
            &der,
            &[],
            &name,
            &[],
            UnixTime::since_unix_epoch(Duration::from_secs(timestamp)),
        )
    }

    #[test]
    fn generated_direct_identity_uses_the_exact_spki_carried_by_the_code() {
        let endpoint =
            DirectEndpoint::new("127.0.0.1", DEFAULT_DIRECT_PORT).expect("valid loopback endpoint");
        let prepared = PreparedDirectHost::new(
            endpoint,
            InviteToken::from_bytes([7; InviteToken::BYTE_LENGTH]),
        )
        .expect("direct host should prepare");
        let code = prepared.connection_code().clone();
        assert!(PreparedDirectJoin::new(&code).is_ok());
        assert!(format!("{prepared:?}").contains("[WEBTRANSPORT SERVER CONFIG]"));
        assert!(!format!("{prepared:?}").contains("07070707"));
    }

    #[test]
    fn verifier_accepts_only_the_exact_short_lived_p256_spki() {
        const START: i64 = 1_735_689_600;
        let identity = identity_between(START, START + 86_400);
        let certificate = identity
            .certificate_chain()
            .as_slice()
            .first()
            .expect("test identity has a leaf");
        let expected = spki_fingerprint(certificate.der()).expect("valid certificate");
        assert!(verify_at(
            &identity,
            expected,
            u64::try_from(START + 1).expect("positive")
        )
        .is_ok());
        assert!(verify_at(
            &identity,
            CertificateFingerprint::from_bytes([9; 32]),
            u64::try_from(START + 1).expect("positive")
        )
        .is_err());
        assert!(verify_at(
            &identity,
            expected,
            u64::try_from(START - 1).expect("positive")
        )
        .is_err());
        assert!(verify_at(
            &identity,
            expected,
            u64::try_from(START + 86_401).expect("positive")
        )
        .is_err());

        let overlong = identity_between(START, START + MAX_CERTIFICATE_LIFETIME_SECS + 1);
        let overlong_certificate = overlong
            .certificate_chain()
            .as_slice()
            .first()
            .expect("test identity has a leaf");
        let overlong_pin = spki_fingerprint(overlong_certificate.der()).expect("valid certificate");
        assert!(verify_at(
            &overlong,
            overlong_pin,
            u64::try_from(START + 1).expect("positive")
        )
        .is_err());
    }

    #[test]
    fn verifier_rejects_a_pinned_non_p256_certificate() {
        const START: i64 = 1_735_689_600;
        let mut params =
            CertificateParams::new(vec!["localhost".to_owned()]).expect("valid certificate params");
        params.not_before =
            OffsetDateTime::from_unix_timestamp(START).expect("valid test timestamp");
        params.not_after =
            OffsetDateTime::from_unix_timestamp(START + 86_400).expect("valid test timestamp");
        let key = KeyPair::generate_for(&PKCS_ED25519).expect("ed25519 generation is supported");
        let certificate = params.self_signed(&key).expect("test certificate builds");
        let der = CertificateDer::from(certificate.der().to_vec());
        let expected = spki_fingerprint(der.as_ref()).expect("valid certificate DER");
        let name = ServerName::try_from("localhost").expect("valid test server name");
        let result = SpkiPinVerifier::new(expected).verify_server_cert(
            &der,
            &[],
            &name,
            &[],
            UnixTime::since_unix_epoch(Duration::from_secs(
                u64::try_from(START + 1).expect("positive timestamp"),
            )),
        );
        assert!(result.is_err());
    }

    #[test]
    fn direct_target_uses_the_fixed_path_and_brackets_ipv6() {
        let ipv4 = DirectEndpoint::new("127.0.0.1", 7777).expect("valid endpoint");
        assert_eq!(direct_target(&ipv4), "https://127.0.0.1:7777/hex1");
        let ipv6 = DirectEndpoint::new("::1", 7777).expect("valid endpoint");
        assert_eq!(direct_target(&ipv6), "https://[::1]:7777/hex1");
    }
}
