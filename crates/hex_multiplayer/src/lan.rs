//! Opt-in mDNS/DNS-SD discovery for open Direct lobbies on one local network.
//!
//! Discovery is deliberately not an authorization boundary. DNS-SD records are
//! unauthenticated local-link metadata and contain the current ephemeral invite for an
//! explicitly open LAN lobby. A discovered peer still connects through the pinned Direct
//! transport and passes the ordinary protocol, build, content, lobby, and seat checks.

use std::{
    collections::{btree_map::Entry, BTreeMap},
    fmt,
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use sha2::{Digest as _, Sha256};

mod platform;

use crate::{
    BuildIdentityV1, CertificateFingerprint, ContentFingerprint, DirectConnectionCode,
    DirectEndpoint, InviteToken, SessionInstanceId, SessionLaunchKindV1, SESSION_PROTOCOL_VERSION,
};

/// DNS-SD service type used only for same-link Direct lobby discovery.
pub const LAN_DISCOVERY_SERVICE_TYPE: &str = "_hexgame._udp.local.";

const LAN_DISCOVERY_SCHEMA: &str = "1";
const PROPERTY_SCHEMA: &str = "v";
const PROPERTY_SESSION: &str = "session";
const PROPERTY_KIND: &str = "kind";
const PROPERTY_COMPATIBILITY: &str = "compat";
const PROPERTY_CERTIFICATE_PIN: &str = "pin";
const PROPERTY_CERTIFICATE_EXPIRY: &str = "expires";
const PROPERTY_INVITE: &str = "invite";
const PROPERTY_CLAIMED_SEATS: &str = "seats";
const PROPERTY_SEAT_CAPACITY: &str = "capacity";
const MAX_SERVICE_ID_BYTES: usize = 512;
const MAX_DISCOVERED_ADDRESSES: usize = 32;
const MAX_TXT_PROPERTIES: usize = 16;
const MAX_TXT_METADATA_BYTES: usize = 4_096;
const MAX_DISCOVERED_LAN_SESSIONS: usize = 64;
const MAX_LAN_SEATS: u8 = 6;

/// Non-secret digest used to mark obviously incompatible discovered builds before connecting.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct LanCompatibilityKey([u8; Self::BYTE_LENGTH]);

impl LanCompatibilityKey {
    /// Encoded digest length. This is a discovery hint; final admission checks exact fields.
    pub const BYTE_LENGTH: usize = 16;

    /// Derives a stable key from the exact build and accepted shipped-content identities.
    #[must_use]
    pub fn from_build_and_content(build: &BuildIdentityV1, content: ContentFingerprint) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"hex-lan-compatibility-v1");
        hasher.update(SESSION_PROTOCOL_VERSION.to_be_bytes());
        hash_text(&mut hasher, build.version.as_str());
        hash_text(&mut hasher, build.revision.as_str());
        hasher.update(content.0.to_be_bytes());
        let digest = hasher.finalize();
        let mut key = [0_u8; Self::BYTE_LENGTH];
        for (target, source) in key.iter_mut().zip(digest.iter()) {
            *target = *source;
        }
        Self(key)
    }

    fn encode(self) -> String {
        URL_SAFE_NO_PAD.encode(self.0)
    }

    fn decode(encoded: &str) -> Result<Self, LanDiscoveryError> {
        Ok(Self(decode_exact::<{ Self::BYTE_LENGTH }>(
            PROPERTY_COMPATIBILITY,
            encoded,
        )?))
    }
}

impl fmt::Debug for LanCompatibilityKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LanCompatibilityKey(")?;
        for byte in self.0.iter().take(4) {
            write!(formatter, "{byte:02x}")?;
        }
        formatter.write_str("…)")
    }
}

fn hash_text(hasher: &mut Sha256, text: &str) {
    hasher.update(u64::try_from(text.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(text.as_bytes());
}

/// Public launch class shown by the LAN browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LanSessionKind {
    /// Disposable shipped Sandbox encounter.
    Sandbox,
    /// Host-owned Campaign checkpoint.
    Campaign,
}

impl LanSessionKind {
    const fn as_property(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Campaign => "campaign",
        }
    }

    fn parse(value: &str) -> Result<Self, LanDiscoveryError> {
        match value {
            "sandbox" => Ok(Self::Sandbox),
            "campaign" => Ok(Self::Campaign),
            _ => Err(LanDiscoveryError::MalformedAnnouncement("kind")),
        }
    }
}

impl From<SessionLaunchKindV1> for LanSessionKind {
    fn from(value: SessionLaunchKindV1) -> Self {
        match value {
            SessionLaunchKindV1::Sandbox => Self::Sandbox,
            SessionLaunchKindV1::Campaign => Self::Campaign,
        }
    }
}

/// Complete private input used to publish one explicitly open LAN lobby.
#[derive(Clone, PartialEq, Eq)]
pub struct LanSessionAdvertisement {
    session_instance_id: SessionInstanceId,
    kind: LanSessionKind,
    compatibility: LanCompatibilityKey,
    connection_code: DirectConnectionCode,
    claimed_seats: u8,
    seat_capacity: u8,
}

impl LanSessionAdvertisement {
    /// Validates one open-lobby advertisement before any multicast socket is created.
    pub fn new(
        session_instance_id: SessionInstanceId,
        kind: LanSessionKind,
        compatibility: LanCompatibilityKey,
        connection_code: DirectConnectionCode,
        claimed_seats: u8,
        seat_capacity: u8,
    ) -> Result<Self, LanDiscoveryError> {
        if !session_instance_id.is_valid() {
            return Err(LanDiscoveryError::MalformedAnnouncement("session"));
        }
        if seat_capacity == 0
            || seat_capacity > MAX_LAN_SEATS
            || claimed_seats == 0
            || claimed_seats > seat_capacity
        {
            return Err(LanDiscoveryError::MalformedAnnouncement("seat counts"));
        }
        Ok(Self {
            session_instance_id,
            kind,
            compatibility,
            connection_code,
            claimed_seats,
            seat_capacity,
        })
    }

    /// Concrete host session advertised by this record.
    #[must_use]
    pub const fn session_instance_id(&self) -> SessionInstanceId {
        self.session_instance_id
    }
}

impl fmt::Debug for LanSessionAdvertisement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanSessionAdvertisement")
            .field("session_instance_id", &self.session_instance_id)
            .field("kind", &self.kind)
            .field("compatibility", &self.compatibility)
            .field("connection_code", &"[OPEN LAN INVITE REDACTED]")
            .field("claimed_seats", &self.claimed_seats)
            .field("seat_capacity", &self.seat_capacity)
            .finish()
    }
}

/// One resolved LAN lobby. Its admission credential is never exposed to presentation code.
#[derive(Clone, PartialEq, Eq)]
pub struct LanDiscoveredSession {
    service_id: String,
    session_instance_id: SessionInstanceId,
    kind: LanSessionKind,
    compatibility: LanCompatibilityKey,
    connection_code: DirectConnectionCode,
    claimed_seats: u8,
    seat_capacity: u8,
}

impl LanDiscoveredSession {
    /// Opaque DNS-SD identity used by a typed join intent.
    #[must_use]
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    /// Concrete session identity, suitable for a short non-secret display suffix.
    #[must_use]
    pub const fn session_instance_id(&self) -> SessionInstanceId {
        self.session_instance_id
    }

    /// Public launch class.
    #[must_use]
    pub const fn kind(&self) -> LanSessionKind {
        self.kind
    }

    /// Whether this discovery hint matches the local exact build/content hint.
    #[must_use]
    pub fn is_compatible_with(&self, local: LanCompatibilityKey) -> bool {
        self.compatibility == local
    }

    /// Resolved endpoint chosen from the service's address records.
    #[must_use]
    pub fn endpoint(&self) -> &DirectEndpoint {
        &self.connection_code.endpoint
    }

    /// Currently claimed human seats.
    #[must_use]
    pub const fn claimed_seats(&self) -> u8 {
        self.claimed_seats
    }

    /// Maximum human seats advertised by this build.
    #[must_use]
    pub const fn seat_capacity(&self) -> u8 {
        self.seat_capacity
    }

    /// Clones the private pinned Direct join material for an explicit selected session.
    #[must_use]
    pub fn connection_code(&self) -> DirectConnectionCode {
        self.connection_code.clone()
    }
}

impl fmt::Debug for LanDiscoveredSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanDiscoveredSession")
            .field("service_id", &self.service_id)
            .field("session_instance_id", &self.session_instance_id)
            .field("kind", &self.kind)
            .field("compatibility", &self.compatibility)
            .field("endpoint", &self.connection_code.endpoint)
            .field("connection_code", &"[OPEN LAN INVITE REDACTED]")
            .field("claimed_seats", &self.claimed_seats)
            .field("seat_capacity", &self.seat_capacity)
            .finish()
    }
}

/// Active publisher for one explicitly open LAN lobby.
pub struct LanDiscoveryAdvertiser {
    backend: Option<platform::Advertiser>,
    current: LanSessionAdvertisement,
}

impl LanDiscoveryAdvertiser {
    /// Starts multicast advertisement. This is the explicit socket-opening action.
    pub fn start(advertisement: LanSessionAdvertisement) -> Result<Self, LanDiscoveryError> {
        let backend = platform::Advertiser::start(&advertisement)?;
        Ok(Self {
            backend: Some(backend),
            current: advertisement,
        })
    }

    /// Re-announces changed open-lobby fields, including the rotated one-time invite.
    pub fn refresh(
        &mut self,
        advertisement: LanSessionAdvertisement,
    ) -> Result<bool, LanDiscoveryError> {
        self.poll_health()?;
        if advertisement.session_instance_id != self.current.session_instance_id {
            return Err(LanDiscoveryError::SessionChanged);
        }
        if advertisement == self.current {
            return Ok(false);
        }
        // Native Bonjour cannot update TXT metadata in place. Drop the old registration before
        // replacing it so conflict auto-renaming cannot leave this session under a stale alias.
        self.backend = None;
        self.backend = Some(platform::Advertiser::start(&advertisement)?);
        self.current = advertisement;
        Ok(true)
    }

    /// Polls the native discovery service and surfaces lazy registration failures.
    pub fn poll_health(&mut self) -> Result<(), LanDiscoveryError> {
        self.backend
            .as_mut()
            .ok_or(LanDiscoveryError::DaemonStopped)?
            .poll_health()
    }

    /// Whether the operating system has confirmed the current service registration.
    #[must_use]
    pub fn is_announced(&self) -> bool {
        self.backend
            .as_ref()
            .is_some_and(platform::Advertiser::is_announced)
    }
}

impl fmt::Debug for LanDiscoveryAdvertiser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanDiscoveryAdvertiser")
            .field("announced", &self.is_announced())
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

/// Active continuous browser for open LAN lobbies.
pub struct LanDiscoveryBrowser {
    backend: platform::Browser,
    sessions: BTreeMap<String, LanDiscoveredSession>,
}

impl LanDiscoveryBrowser {
    /// Starts browsing. This is the explicit socket-opening action.
    pub fn start() -> Result<Self, LanDiscoveryError> {
        let backend = platform::Browser::start()?;
        Ok(Self {
            backend,
            sessions: BTreeMap::new(),
        })
    }

    /// Drains currently available events without blocking the Bevy frame.
    pub fn poll(&mut self) -> Result<bool, LanDiscoveryError> {
        self.backend.poll_health()?;
        let mut changed = false;
        let now = current_unix_seconds();
        let previous_count = self.sessions.len();
        self.sessions.retain(|_service_id, session| {
            session.connection_code.certificate_expires_unix_seconds > now
        });
        changed |= self.sessions.len() != previous_count;
        loop {
            match self.backend.try_recv()? {
                Some(platform::BrowserEvent::Resolved(service)) => {
                    match discovered_session(&service, now) {
                        Ok(discovered) => {
                            changed |= cache_discovered_session(&mut self.sessions, discovered);
                        }
                        Err(_untrusted_record) => {
                            changed |= self.sessions.remove(&service.service_id).is_some();
                        }
                    }
                }
                Some(platform::BrowserEvent::Removed(fullname)) => {
                    changed |= self.sessions.remove(&fullname).is_some();
                }
                None => return Ok(changed),
            }
        }
    }

    /// Current resolved lobbies in deterministic DNS-SD identity order.
    pub fn sessions(&self) -> impl Iterator<Item = &LanDiscoveredSession> {
        self.sessions.values()
    }

    /// Resolves one opaque selection produced by [`Self::sessions`].
    #[must_use]
    pub fn session(&self, service_id: &str) -> Option<&LanDiscoveredSession> {
        self.sessions.get(service_id)
    }
}

fn cache_discovered_session(
    sessions: &mut BTreeMap<String, LanDiscoveredSession>,
    discovered: LanDiscoveredSession,
) -> bool {
    let id = discovered.service_id.clone();
    let at_capacity = sessions.len() >= MAX_DISCOVERED_LAN_SESSIONS;
    match sessions.entry(id) {
        Entry::Occupied(mut current) => {
            if current.get() == &discovered {
                false
            } else {
                current.insert(discovered);
                true
            }
        }
        Entry::Vacant(vacant) if !at_capacity => {
            vacant.insert(discovered);
            true
        }
        Entry::Vacant(_) => false,
    }
}

impl fmt::Debug for LanDiscoveryBrowser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanDiscoveryBrowser")
            .field("sessions", &self.sessions)
            .finish_non_exhaustive()
    }
}

/// Failure to create or operate the opt-in LAN discovery adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LanDiscoveryError {
    /// A locally created or remotely received record violates the bounded schema.
    MalformedAnnouncement(&'static str),
    /// A publisher refresh attempted to replace a different concrete session.
    SessionChanged,
    /// The underlying daemon stopped unexpectedly.
    DaemonStopped,
    /// The operating system or DNS-SD daemon refused an operation.
    ServiceUnavailable(String),
}

impl LanDiscoveryError {
    fn daemon(error: impl fmt::Display) -> Self {
        Self::ServiceUnavailable(error.to_string())
    }
}

impl fmt::Display for LanDiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedAnnouncement(field) => {
                write!(formatter, "LAN discovery metadata has an invalid {field}")
            }
            Self::SessionChanged => {
                formatter.write_str("a LAN advertisement cannot change concrete sessions")
            }
            Self::DaemonStopped => formatter.write_str("the LAN discovery service stopped"),
            Self::ServiceUnavailable(reason) => {
                write!(formatter, "LAN discovery is unavailable: {reason}")
            }
        }
    }
}

impl std::error::Error for LanDiscoveryError {}

fn service_instance_name(advertisement: &LanSessionAdvertisement) -> String {
    let session_hex = encode_hex(&advertisement.session_instance_id.to_bytes());
    let short_session = session_hex.chars().take(8).collect::<String>();
    format!("Hex {} {}", kind_label(advertisement.kind), short_session)
}

fn announcement_properties(advertisement: &LanSessionAdvertisement) -> Vec<(String, String)> {
    vec![
        (PROPERTY_SCHEMA.to_owned(), LAN_DISCOVERY_SCHEMA.to_owned()),
        (
            PROPERTY_SESSION.to_owned(),
            URL_SAFE_NO_PAD.encode(advertisement.session_instance_id.to_bytes()),
        ),
        (
            PROPERTY_KIND.to_owned(),
            advertisement.kind.as_property().to_owned(),
        ),
        (
            PROPERTY_COMPATIBILITY.to_owned(),
            advertisement.compatibility.encode(),
        ),
        (
            PROPERTY_CERTIFICATE_PIN.to_owned(),
            URL_SAFE_NO_PAD.encode(
                advertisement
                    .connection_code
                    .certificate_fingerprint
                    .to_bytes(),
            ),
        ),
        (
            PROPERTY_CERTIFICATE_EXPIRY.to_owned(),
            advertisement
                .connection_code
                .certificate_expires_unix_seconds
                .to_string(),
        ),
        (
            PROPERTY_INVITE.to_owned(),
            URL_SAFE_NO_PAD.encode(advertisement.connection_code.invite_token.to_bytes()),
        ),
        (
            PROPERTY_CLAIMED_SEATS.to_owned(),
            advertisement.claimed_seats.to_string(),
        ),
        (
            PROPERTY_SEAT_CAPACITY.to_owned(),
            advertisement.seat_capacity.to_string(),
        ),
    ]
}

struct LanResolvedRecord {
    service_id: String,
    service_type: String,
    addresses: Vec<IpAddr>,
    port: u16,
    properties: BTreeMap<String, String>,
}

fn discovered_session(
    service: &LanResolvedRecord,
    now_unix_seconds: u64,
) -> Result<LanDiscoveredSession, LanDiscoveryError> {
    let txt_bytes = service
        .properties
        .iter()
        .try_fold(0_usize, |total, (key, value)| {
            total.checked_add(key.len())?.checked_add(value.len())
        });
    if service.service_type != LAN_DISCOVERY_SERVICE_TYPE
        || service.service_id.is_empty()
        || service.service_id.len() > MAX_SERVICE_ID_BYTES
        || service.port == 0
        || service.addresses.len() > MAX_DISCOVERED_ADDRESSES
        || service.properties.len() > MAX_TXT_PROPERTIES
        || txt_bytes.is_none_or(|bytes| bytes > MAX_TXT_METADATA_BYTES)
    {
        return Err(LanDiscoveryError::MalformedAnnouncement("service identity"));
    }
    if property(service, PROPERTY_SCHEMA)? != LAN_DISCOVERY_SCHEMA {
        return Err(LanDiscoveryError::MalformedAnnouncement("schema"));
    }
    let session_instance_id = SessionInstanceId::from_bytes(decode_exact::<16>(
        PROPERTY_SESSION,
        property(service, PROPERTY_SESSION)?,
    )?);
    if !session_instance_id.is_valid() {
        return Err(LanDiscoveryError::MalformedAnnouncement("session"));
    }
    let kind = LanSessionKind::parse(property(service, PROPERTY_KIND)?)?;
    let compatibility = LanCompatibilityKey::decode(property(service, PROPERTY_COMPATIBILITY)?)?;
    let certificate_fingerprint = CertificateFingerprint::from_bytes(decode_exact::<32>(
        PROPERTY_CERTIFICATE_PIN,
        property(service, PROPERTY_CERTIFICATE_PIN)?,
    )?);
    let certificate_expires_unix_seconds = property(service, PROPERTY_CERTIFICATE_EXPIRY)?
        .parse::<u64>()
        .map_err(|_error| LanDiscoveryError::MalformedAnnouncement("certificate expiry"))?;
    if certificate_expires_unix_seconds <= now_unix_seconds {
        return Err(LanDiscoveryError::MalformedAnnouncement(
            "certificate expiry",
        ));
    }
    let invite_token = InviteToken::from_bytes(decode_exact::<16>(
        PROPERTY_INVITE,
        property(service, PROPERTY_INVITE)?,
    )?);
    let claimed_seats = parse_seat_count(service, PROPERTY_CLAIMED_SEATS)?;
    let seat_capacity = parse_seat_count(service, PROPERTY_SEAT_CAPACITY)?;
    if claimed_seats == 0 || claimed_seats > seat_capacity {
        return Err(LanDiscoveryError::MalformedAnnouncement("seat counts"));
    }
    let address = preferred_address(service).ok_or(LanDiscoveryError::MalformedAnnouncement(
        "reachable address",
    ))?;
    let endpoint = DirectEndpoint::new(address.to_string(), service.port)
        .map_err(|_error| LanDiscoveryError::MalformedAnnouncement("reachable address"))?;
    Ok(LanDiscoveredSession {
        service_id: service.service_id.clone(),
        session_instance_id,
        kind,
        compatibility,
        connection_code: DirectConnectionCode {
            endpoint,
            certificate_fingerprint,
            certificate_expires_unix_seconds,
            invite_token,
        },
        claimed_seats,
        seat_capacity,
    })
}

fn property<'a>(
    service: &'a LanResolvedRecord,
    key: &'static str,
) -> Result<&'a str, LanDiscoveryError> {
    service
        .properties
        .get(key)
        .map(String::as_str)
        .ok_or(LanDiscoveryError::MalformedAnnouncement(key))
}

fn parse_seat_count(
    service: &LanResolvedRecord,
    key: &'static str,
) -> Result<u8, LanDiscoveryError> {
    let count = property(service, key)?
        .parse::<u8>()
        .map_err(|_error| LanDiscoveryError::MalformedAnnouncement(key))?;
    if count > MAX_LAN_SEATS {
        return Err(LanDiscoveryError::MalformedAnnouncement(key));
    }
    Ok(count)
}

fn decode_exact<const LENGTH: usize>(
    field: &'static str,
    encoded: &str,
) -> Result<[u8; LENGTH], LanDiscoveryError> {
    let decoded = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_error| LanDiscoveryError::MalformedAnnouncement(field))?;
    decoded
        .try_into()
        .map_err(|_length_error| LanDiscoveryError::MalformedAnnouncement(field))
}

fn preferred_address(service: &LanResolvedRecord) -> Option<IpAddr> {
    let mut addresses = service
        .addresses
        .iter()
        .copied()
        .filter(|address| usable_address(*address))
        .collect::<Vec<_>>();
    addresses.sort_by_key(|address| (address_rank(*address), address_bytes(*address)));
    addresses.into_iter().next()
}

fn usable_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => !address.is_unspecified() && !address.is_multicast(),
        IpAddr::V6(address) => {
            !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_loopback()
                && !address.is_unicast_link_local()
        }
    }
}

fn address_rank(address: IpAddr) -> u8 {
    match address {
        IpAddr::V4(address) if address.is_private() => 0,
        IpAddr::V4(address) if !address.is_link_local() && !address.is_loopback() => 1,
        IpAddr::V6(_) => 2,
        IpAddr::V4(address) if address.is_link_local() => 3,
        IpAddr::V4(_) => 4,
    }
}

fn address_bytes(address: IpAddr) -> [u8; 16] {
    match address {
        IpAddr::V4(address) => address.to_ipv6_mapped().octets(),
        IpAddr::V6(address) => address.octets(),
    }
}

fn kind_label(kind: LanSessionKind) -> &'static str {
    match kind {
        LanSessionKind::Sandbox => "Sandbox",
        LanSessionKind::Campaign => "Campaign",
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    use fmt::Write as _;

    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _written = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn current_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BoundedText;
    use std::{
        thread,
        time::{Duration, Instant},
    };

    fn advertisement() -> LanSessionAdvertisement {
        LanSessionAdvertisement::new(
            SessionInstanceId::from_bytes([7; 16]),
            LanSessionKind::Sandbox,
            LanCompatibilityKey::from_build_and_content(
                &BuildIdentityV1 {
                    version: BoundedText::new("0.1.0").expect("bounded version"),
                    revision: BoundedText::new("test-build").expect("bounded revision"),
                },
                ContentFingerprint(42),
            ),
            DirectConnectionCode {
                endpoint: DirectEndpoint::new("127.0.0.1", 7_777).expect("valid endpoint"),
                certificate_fingerprint: CertificateFingerprint::from_bytes([3; 32]),
                certificate_expires_unix_seconds: 2_000_000_000,
                invite_token: InviteToken::from_bytes([5; 16]),
            },
            1,
            6,
        )
        .expect("valid LAN advertisement")
    }

    fn resolved_record(
        service_name: &str,
        addresses: &[&str],
        properties: Vec<(String, String)>,
    ) -> LanResolvedRecord {
        LanResolvedRecord {
            service_id: format!("{service_name}.{LAN_DISCOVERY_SERVICE_TYPE}"),
            service_type: LAN_DISCOVERY_SERVICE_TYPE.to_owned(),
            addresses: addresses
                .iter()
                .map(|address| address.parse().expect("valid fixture address"))
                .collect(),
            port: 7_777,
            properties: properties.into_iter().collect(),
        }
    }

    #[test]
    fn resolved_record_replaces_the_hosts_placeholder_with_a_private_lan_address() {
        let advertisement = advertisement();
        let properties = announcement_properties(&advertisement);
        let service = resolved_record(
            "Hex Sandbox fixture",
            &["100.64.0.9", "192.168.1.42"],
            properties,
        );

        let discovered =
            discovered_session(&service, 1_900_000_000).expect("valid announcement should resolve");
        assert_eq!(discovered.endpoint().host(), "192.168.1.42");
        assert_eq!(discovered.endpoint().port(), 7_777);
        assert_eq!(
            discovered.connection_code().certificate_fingerprint,
            advertisement.connection_code.certificate_fingerprint
        );
        assert_eq!(
            discovered.connection_code().invite_token,
            advertisement.connection_code.invite_token
        );
    }

    #[test]
    fn loopback_is_only_selected_when_no_lan_route_is_available() {
        let service = resolved_record(
            "Hex Sandbox fixture",
            &["127.0.0.1", "10.0.0.42"],
            announcement_properties(&advertisement()),
        );
        let discovered =
            discovered_session(&service, 1_900_000_000).expect("private LAN route is preferred");
        assert_eq!(discovered.endpoint().host(), "10.0.0.42");

        let loopback = resolved_record(
            "Hex Sandbox fixture",
            &["127.0.0.1"],
            announcement_properties(&advertisement()),
        );
        let discovered = discovered_session(&loopback, 1_900_000_000)
            .expect("same-machine native processes may discover one another");
        assert_eq!(discovered.endpoint().host(), "127.0.0.1");
    }

    #[test]
    fn resolved_record_collections_are_bounded_before_admission_fields_are_read() {
        let mut too_many_addresses = resolved_record(
            "Hex Sandbox fixture",
            &["192.168.1.42"],
            announcement_properties(&advertisement()),
        );
        too_many_addresses.addresses = (1..=MAX_DISCOVERED_ADDRESSES + 1)
            .map(|suffix| {
                format!("192.168.1.{suffix}")
                    .parse()
                    .expect("fixture address")
            })
            .collect();
        assert_eq!(
            discovered_session(&too_many_addresses, 1_900_000_000),
            Err(LanDiscoveryError::MalformedAnnouncement("service identity"))
        );

        let mut too_many_properties = resolved_record(
            "Hex Sandbox fixture",
            &["192.168.1.42"],
            announcement_properties(&advertisement()),
        );
        for index in 0..=MAX_TXT_PROPERTIES {
            too_many_properties
                .properties
                .insert(format!("extra{index}"), "bounded".to_owned());
        }
        assert_eq!(
            discovered_session(&too_many_properties, 1_900_000_000),
            Err(LanDiscoveryError::MalformedAnnouncement("service identity"))
        );
    }

    #[test]
    fn discovery_debug_output_never_contains_the_open_lobby_invite() {
        let advertisement = advertisement();
        let encoded_invite = URL_SAFE_NO_PAD.encode([5; 16]);
        assert!(!format!("{advertisement:?}").contains(&encoded_invite));

        let properties = announcement_properties(&advertisement);
        let service = resolved_record("Hex Sandbox fixture", &["192.168.1.42"], properties);
        let discovered =
            discovered_session(&service, 1_900_000_000).expect("valid announcement should resolve");
        assert!(!format!("{discovered:?}").contains(&encoded_invite));
    }

    #[test]
    fn announcement_metadata_updates_the_rotating_invite_without_changing_session_identity() {
        let first = advertisement();
        let mut second = first.clone();
        second.connection_code.invite_token = InviteToken::from_bytes([8; 16]);
        let first_properties = announcement_properties(&first)
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let second_properties = announcement_properties(&second)
            .into_iter()
            .collect::<BTreeMap<_, _>>();

        assert_ne!(
            first_properties.get(PROPERTY_INVITE),
            second_properties.get(PROPERTY_INVITE)
        );
        assert_eq!(
            first_properties.get(PROPERTY_SESSION),
            second_properties.get(PROPERTY_SESSION)
        );
        assert_eq!(
            first_properties.get(PROPERTY_CERTIFICATE_PIN),
            second_properties.get(PROPERTY_CERTIFICATE_PIN)
        );
    }

    #[test]
    fn malformed_or_expired_records_fail_closed_without_panicking() {
        let advertisement = advertisement();
        let mut properties = announcement_properties(&advertisement);
        properties.retain(|(key, _value)| key != PROPERTY_INVITE);
        let missing = resolved_record("Hex Sandbox fixture", &["192.168.1.42"], properties);
        assert_eq!(
            discovered_session(&missing, 1_900_000_000),
            Err(LanDiscoveryError::MalformedAnnouncement(PROPERTY_INVITE))
        );

        let properties = announcement_properties(&advertisement);
        let expired = resolved_record("Hex Sandbox fixture", &["192.168.1.42"], properties);
        assert_eq!(
            discovered_session(&expired, 2_000_000_000),
            Err(LanDiscoveryError::MalformedAnnouncement(
                "certificate expiry"
            ))
        );
    }

    #[test]
    fn compatibility_key_changes_with_build_or_content() {
        let first = BuildIdentityV1 {
            version: BoundedText::new("0.1.0").expect("bounded version"),
            revision: BoundedText::new("one").expect("bounded revision"),
        };
        let second = BuildIdentityV1 {
            version: BoundedText::new("0.1.0").expect("bounded version"),
            revision: BoundedText::new("two").expect("bounded revision"),
        };
        let key = LanCompatibilityKey::from_build_and_content(&first, ContentFingerprint(1));
        assert_ne!(
            key,
            LanCompatibilityKey::from_build_and_content(&second, ContentFingerprint(1))
        );
        assert_ne!(
            key,
            LanCompatibilityKey::from_build_and_content(&first, ContentFingerprint(2))
        );
    }

    #[test]
    fn advertisement_rejects_invalid_capacity_or_session_identity() {
        let valid = advertisement();
        assert!(LanSessionAdvertisement::new(
            SessionInstanceId::from_bytes([0; 16]),
            valid.kind,
            valid.compatibility,
            valid.connection_code.clone(),
            1,
            6,
        )
        .is_err());
        assert!(LanSessionAdvertisement::new(
            valid.session_instance_id,
            valid.kind,
            valid.compatibility,
            valid.connection_code,
            7,
            6,
        )
        .is_err());
    }

    #[test]
    fn discovery_cache_is_bounded_but_still_refreshes_known_sessions() {
        let properties = announcement_properties(&advertisement());
        let fixture = |index: usize| {
            let service = resolved_record(
                &format!("Hex Sandbox fixture {index}"),
                &["192.168.1.42"],
                properties.clone(),
            );
            discovered_session(&service, 1_900_000_000).expect("valid announcement should resolve")
        };
        let mut sessions = BTreeMap::new();
        for index in 0..MAX_DISCOVERED_LAN_SESSIONS {
            assert!(cache_discovered_session(&mut sessions, fixture(index)));
        }
        assert!(!cache_discovered_session(
            &mut sessions,
            fixture(MAX_DISCOVERED_LAN_SESSIONS)
        ));
        assert_eq!(sessions.len(), MAX_DISCOVERED_LAN_SESSIONS);

        let mut refreshed = fixture(0);
        refreshed.claimed_seats = 2;
        assert!(cache_discovered_session(&mut sessions, refreshed.clone()));
        assert_eq!(sessions.len(), MAX_DISCOVERED_LAN_SESSIONS);
        assert_eq!(sessions.get(refreshed.service_id()), Some(&refreshed));
    }

    #[test]
    #[ignore = "requires real local multicast sockets"]
    fn advertiser_and_browser_exchange_on_the_local_link() {
        let advertisement = advertisement();
        let expected_session = advertisement.session_instance_id();
        let mut advertiser =
            LanDiscoveryAdvertiser::start(advertisement).expect("start local advertiser");
        let mut browser = LanDiscoveryBrowser::start().expect("start local browser");
        let deadline = Instant::now() + Duration::from_secs(5);

        while Instant::now() < deadline {
            advertiser.poll_health().expect("poll advertiser health");
            browser.poll().expect("poll local browser");
            if browser
                .sessions()
                .any(|session| session.session_instance_id() == expected_session)
            {
                assert!(advertiser.is_announced());
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }

        panic!("the local browser did not resolve its advertised Hex lobby within five seconds");
    }
}
