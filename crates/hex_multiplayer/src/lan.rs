//! Opt-in mDNS/DNS-SD discovery for open Direct lobbies on one local network.
//!
//! Discovery is deliberately not an authorization boundary. DNS-SD records are
//! unauthenticated local-link metadata and contain the current ephemeral invite for an
//! explicitly open LAN lobby. A discovered peer still connects through the pinned Direct
//! transport and passes the ordinary protocol, build, content, lobby, and seat checks.

use std::{
    collections::BTreeMap,
    fmt,
    net::IpAddr,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use mdns_sd::{
    DaemonEvent, Receiver, ResolvedService, ServiceDaemon, ServiceEvent, ServiceInfo, TryRecvError,
};
use sha2::{Digest as _, Sha256};

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
    daemon: ServiceDaemon,
    monitor: Receiver<DaemonEvent>,
    fullname: String,
    current: LanSessionAdvertisement,
}

impl LanDiscoveryAdvertiser {
    /// Starts multicast advertisement. This is the explicit socket-opening action.
    pub fn start(advertisement: LanSessionAdvertisement) -> Result<Self, LanDiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(LanDiscoveryError::daemon)?;
        let monitor = daemon.monitor().map_err(LanDiscoveryError::daemon)?;
        let info = service_info(&advertisement)?;
        let fullname = info.get_fullname().to_owned();
        daemon.register(info).map_err(LanDiscoveryError::daemon)?;
        Ok(Self {
            daemon,
            monitor,
            fullname,
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
        let info = service_info(&advertisement)?;
        self.daemon
            .register(info)
            .map_err(LanDiscoveryError::daemon)?;
        self.current = advertisement;
        Ok(true)
    }

    /// Surfaces lazy multicast socket failures reported by the daemon thread.
    pub fn poll_health(&self) -> Result<(), LanDiscoveryError> {
        loop {
            match self.monitor.try_recv() {
                Ok(DaemonEvent::Error(error)) => return Err(LanDiscoveryError::daemon(error)),
                Ok(_) => {}
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Err(LanDiscoveryError::DaemonStopped),
            }
        }
    }
}

impl fmt::Debug for LanDiscoveryAdvertiser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LanDiscoveryAdvertiser")
            .field("fullname", &self.fullname)
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl Drop for LanDiscoveryAdvertiser {
    fn drop(&mut self) {
        let _status = self.daemon.unregister(&self.fullname);
        let _status = self.daemon.shutdown();
    }
}

/// Active continuous browser for open LAN lobbies.
pub struct LanDiscoveryBrowser {
    daemon: ServiceDaemon,
    events: Receiver<ServiceEvent>,
    monitor: Receiver<DaemonEvent>,
    sessions: BTreeMap<String, LanDiscoveredSession>,
}

impl LanDiscoveryBrowser {
    /// Starts browsing. This is the explicit socket-opening action.
    pub fn start() -> Result<Self, LanDiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(LanDiscoveryError::daemon)?;
        let monitor = daemon.monitor().map_err(LanDiscoveryError::daemon)?;
        let events = daemon
            .browse(LAN_DISCOVERY_SERVICE_TYPE)
            .map_err(LanDiscoveryError::daemon)?;
        Ok(Self {
            daemon,
            events,
            monitor,
            sessions: BTreeMap::new(),
        })
    }

    /// Drains currently available events without blocking the Bevy frame.
    pub fn poll(&mut self) -> Result<bool, LanDiscoveryError> {
        self.poll_health()?;
        let mut changed = false;
        let now = current_unix_seconds();
        let previous_count = self.sessions.len();
        self.sessions.retain(|_service_id, session| {
            session.connection_code.certificate_expires_unix_seconds > now
        });
        changed |= self.sessions.len() != previous_count;
        loop {
            match self.events.try_recv() {
                Ok(ServiceEvent::ServiceResolved(service)) => {
                    match discovered_session(&service, now) {
                        Ok(discovered) => {
                            let id = discovered.service_id.clone();
                            changed |= self.sessions.get(&id) != Some(&discovered);
                            self.sessions.insert(id, discovered);
                        }
                        Err(_untrusted_record) => {
                            changed |= self.sessions.remove(&service.fullname).is_some();
                        }
                    }
                }
                Ok(ServiceEvent::ServiceRemoved(_service_type, fullname)) => {
                    changed |= self.sessions.remove(&fullname).is_some();
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => return Ok(changed),
                Err(TryRecvError::Disconnected) => return Err(LanDiscoveryError::DaemonStopped),
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

    fn poll_health(&self) -> Result<(), LanDiscoveryError> {
        loop {
            match self.monitor.try_recv() {
                Ok(DaemonEvent::Error(error)) => return Err(LanDiscoveryError::daemon(error)),
                Ok(_) => {}
                Err(TryRecvError::Empty) => return Ok(()),
                Err(TryRecvError::Disconnected) => return Err(LanDiscoveryError::DaemonStopped),
            }
        }
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

impl Drop for LanDiscoveryBrowser {
    fn drop(&mut self) {
        let _status = self.daemon.stop_browse(LAN_DISCOVERY_SERVICE_TYPE);
        let _status = self.daemon.shutdown();
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

fn service_info(advertisement: &LanSessionAdvertisement) -> Result<ServiceInfo, LanDiscoveryError> {
    let session_hex = encode_hex(&advertisement.session_instance_id.to_bytes());
    let short_session = session_hex.chars().take(8).collect::<String>();
    let instance = format!("Hex {} {}", kind_label(advertisement.kind), short_session);
    let hostname = format!("hex-{session_hex}.local.");
    let properties = announcement_properties(advertisement);
    ServiceInfo::new(
        LAN_DISCOVERY_SERVICE_TYPE,
        &instance,
        &hostname,
        "",
        advertisement.connection_code.endpoint.port(),
        properties.as_slice(),
    )
    .map(ServiceInfo::enable_addr_auto)
    .map_err(LanDiscoveryError::daemon)
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

fn discovered_session(
    service: &ResolvedService,
    now_unix_seconds: u64,
) -> Result<LanDiscoveredSession, LanDiscoveryError> {
    if service.ty_domain != LAN_DISCOVERY_SERVICE_TYPE
        || service.fullname.is_empty()
        || service.fullname.len() > MAX_SERVICE_ID_BYTES
        || service.port == 0
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
        service_id: service.fullname.clone(),
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
    service: &'a ResolvedService,
    key: &'static str,
) -> Result<&'a str, LanDiscoveryError> {
    service
        .get_property_val_str(key)
        .ok_or(LanDiscoveryError::MalformedAnnouncement(key))
}

fn parse_seat_count(service: &ResolvedService, key: &'static str) -> Result<u8, LanDiscoveryError> {
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

fn preferred_address(service: &ResolvedService) -> Option<IpAddr> {
    let mut addresses = service
        .get_addresses()
        .iter()
        .map(mdns_sd::ScopedIp::to_ip_addr)
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

    #[test]
    fn resolved_record_replaces_the_hosts_placeholder_with_a_private_lan_address() {
        let advertisement = advertisement();
        let properties = announcement_properties(&advertisement);
        let service = ServiceInfo::new(
            LAN_DISCOVERY_SERVICE_TYPE,
            "Hex Sandbox fixture",
            "hex-fixture.local.",
            "100.64.0.9,192.168.1.42",
            7_777,
            properties.as_slice(),
        )
        .expect("valid fixture service")
        .as_resolved_service();

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
    fn discovery_debug_output_never_contains_the_open_lobby_invite() {
        let advertisement = advertisement();
        let encoded_invite = URL_SAFE_NO_PAD.encode([5; 16]);
        assert!(!format!("{advertisement:?}").contains(&encoded_invite));

        let properties = announcement_properties(&advertisement);
        let service = ServiceInfo::new(
            LAN_DISCOVERY_SERVICE_TYPE,
            "Hex Sandbox fixture",
            "hex-fixture.local.",
            "192.168.1.42",
            7_777,
            properties.as_slice(),
        )
        .expect("valid fixture service")
        .as_resolved_service();
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
        let missing = ServiceInfo::new(
            LAN_DISCOVERY_SERVICE_TYPE,
            "Hex Sandbox fixture",
            "hex-fixture.local.",
            "192.168.1.42",
            7_777,
            properties.as_slice(),
        )
        .expect("the DNS record itself is syntactically valid")
        .as_resolved_service();
        assert_eq!(
            discovered_session(&missing, 1_900_000_000),
            Err(LanDiscoveryError::MalformedAnnouncement(PROPERTY_INVITE))
        );

        let properties = announcement_properties(&advertisement);
        let expired = ServiceInfo::new(
            LAN_DISCOVERY_SERVICE_TYPE,
            "Hex Sandbox fixture",
            "hex-fixture.local.",
            "192.168.1.42",
            7_777,
            properties.as_slice(),
        )
        .expect("the DNS record itself is syntactically valid")
        .as_resolved_service();
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
    #[ignore = "requires real local multicast sockets"]
    fn advertiser_and_browser_exchange_on_the_local_link() {
        let advertisement = advertisement();
        let expected_session = advertisement.session_instance_id();
        let _advertiser =
            LanDiscoveryAdvertiser::start(advertisement).expect("start local advertiser");
        let mut browser = LanDiscoveryBrowser::start().expect("start local browser");
        let deadline = Instant::now() + Duration::from_secs(5);

        while Instant::now() < deadline {
            _advertiser.poll_health().expect("poll advertiser health");
            browser.poll().expect("poll local browser");
            if browser
                .sessions()
                .any(|session| session.session_instance_id() == expected_session)
            {
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }

        panic!("the local browser did not resolve its advertised Hex lobby within five seconds");
    }
}
