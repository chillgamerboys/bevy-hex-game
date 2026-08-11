//! Deterministic in-memory multi-app harness for session protocol tests.

use std::{fmt, time::Duration};

use aeronet::io::server::{Server, ServerEndpoint};
use aeronet_channel::{ChannelIo, ChannelIoPlugin};
use aeronet_replicon::{client::AeronetRepliconClient, server::AeronetRepliconServer};
use bevy_app::{App, Update};
use bevy_ecs::{
    hierarchy::ChildOf,
    message::MessageReader,
    prelude::{Entity, Resource},
};
use bevy_replicon::prelude::{ClientState, ProtocolHash};
use bevy_state::{app::StatesPlugin, state::State};
use bevy_time::{TimePlugin, TimeUpdateStrategy};
use hex_core::SimulationRole;

use crate::{
    AdmissionAccepted, AdmissionCredential, AdmissionRefusal, AdmissionSetupError,
    AuthenticatedCommandRequest, ClientHello, CommandResult, InviteToken, LiveSessionSnapshotV1,
    LobbySnapshot, MultiplayerPlugin, PlayerKnowledgeSnapshotV1, ReconnectCredential,
    SessionAdmissionAuthority, SessionClosed, SessionControlResult, SessionManifestV1,
    WorldDeltaV1,
};

/// Messages captured from one deterministic client app.
#[derive(Resource, Debug, Default)]
pub struct ClientProbe {
    /// Successful custom admission responses.
    pub accepted: Vec<AdmissionAccepted>,
    /// Typed pre-authorization refusals.
    pub refused: Vec<AdmissionRefusal>,
    /// Ordered lobby projections.
    pub lobbies: Vec<LobbySnapshot>,
    /// Frozen manifests received after authorization.
    pub manifests: Vec<SessionManifestV1>,
    /// Final or duplicate command results.
    pub command_results: Vec<CommandResult>,
    /// Authorized remembered-player knowledge projections.
    pub player_knowledge: Vec<PlayerKnowledgeSnapshotV1>,
    /// Restart-capable reconnect baselines.
    pub live_snapshots: Vec<LiveSessionSnapshotV1>,
    /// Ordered world mutations newer than a reconnect baseline.
    pub world_deltas: Vec<WorldDeltaV1>,
    /// Typed seatless-lobby request results.
    pub control_results: Vec<SessionControlResult>,
    /// Typed session termination notifications.
    pub closed: Vec<SessionClosed>,
}

/// Authenticated requests captured before gameplay authority reduction.
#[derive(Resource, Debug, Default)]
pub struct HostProbe {
    /// Exactly-once authority ingress messages.
    pub commands: Vec<AuthenticatedCommandRequest>,
}

struct HarnessClient {
    app: App,
    server_connection: Entity,
    client_connection: Entity,
}

/// One listen-host app plus separately scheduled in-memory client apps.
pub struct ChannelSessionHarness {
    host: App,
    server: Entity,
    manifest: SessionManifestV1,
    clients: Vec<HarnessClient>,
}

impl ChannelSessionHarness {
    /// Creates a running in-memory server with deterministic initial session secrets.
    pub fn new(
        manifest: SessionManifestV1,
        host_identity: crate::SessionPeerId,
        invite_token: InviteToken,
    ) -> Result<Self, HarnessError> {
        let mut host = session_app(false);
        let protocol_hash = *host.world().resource::<ProtocolHash>();
        let authority = SessionAdmissionAuthority::with_session_secrets(
            protocol_hash,
            manifest.clone(),
            host_identity,
            invite_token,
        )?;
        host.world_mut().insert_resource(authority);
        let server = host
            .world_mut()
            .spawn((
                ServerEndpoint,
                Server::new(std::time::Instant::now()),
                AeronetRepliconServer,
            ))
            .id();
        Ok(Self {
            host,
            server,
            manifest,
            clients: Vec::new(),
        })
    }

    /// Creates and physically connects a fresh remote replica app.
    pub fn add_client(&mut self) -> usize {
        let mut client = session_app(true);
        let (client_io, server_io) = ChannelIo::new();
        let server_connection = self
            .host
            .world_mut()
            .spawn((ChildOf(self.server), server_io))
            .id();
        let client_connection = client
            .world_mut()
            .spawn((client_io, AeronetRepliconClient))
            .id();
        self.clients.push(HarnessClient {
            app: client,
            server_connection,
            client_connection,
        });
        self.clients.len().saturating_sub(1)
    }

    /// Advances host and clients in a stable order for the requested number of frames.
    pub fn pump(&mut self, frames: usize) {
        for _ in 0..frames {
            self.host.update();
            for client in &mut self.clients {
                client.app.update();
            }
        }
    }

    /// Whether a client backend has reached Replicon's connected state.
    #[must_use]
    pub fn client_connected(&self, index: usize) -> bool {
        self.clients.get(index).is_some_and(|client| {
            client
                .app
                .world()
                .get_resource::<State<ClientState>>()
                .is_some_and(|state| *state.get() == ClientState::Connected)
        })
    }

    /// Sends a first hello with the supplied one-time invitation.
    pub fn send_invite_hello(&mut self, index: usize, invite_token: InviteToken) {
        self.send_hello(index, AdmissionCredential::Invite(invite_token));
    }

    /// Sends a first hello with a previously issued reconnect credential.
    pub fn send_reconnect_hello(&mut self, index: usize, credential: ReconnectCredential) {
        self.send_hello(index, AdmissionCredential::Reconnect(credential));
    }

    /// Despawns the server-side transport entity for one client.
    pub fn disconnect_client(&mut self, index: usize) -> bool {
        let Some(client) = self.clients.get(index) else {
            return false;
        };
        self.host.world_mut().despawn(client.server_connection)
    }

    /// Destroys and recreates one client app, attaching a fresh physical connection.
    ///
    /// Callers retain the previously captured reconnect credential outside the app, as a
    /// real process restart would reload it through the injected temporary store.
    pub fn restart_client(&mut self, index: usize) -> bool {
        let Some(previous) = self.clients.get(index) else {
            return false;
        };
        let _despawned = self.host.world_mut().despawn(previous.server_connection);
        let mut app = session_app(true);
        let (client_io, server_io) = ChannelIo::new();
        let server_connection = self
            .host
            .world_mut()
            .spawn((ChildOf(self.server), server_io))
            .id();
        let client_connection = app
            .world_mut()
            .spawn((client_io, AeronetRepliconClient))
            .id();
        let replacement = HarnessClient {
            app,
            server_connection,
            client_connection,
        };
        let Some(slot) = self.clients.get_mut(index) else {
            return false;
        };
        *slot = replacement;
        true
    }

    /// Current one-time host invite after any successful rotations.
    #[must_use]
    pub fn current_invite(&self) -> InviteToken {
        self.host
            .world()
            .resource::<SessionAdmissionAuthority>()
            .invite_token()
    }

    /// Immutable host app access for typed assertions.
    pub const fn host(&self) -> &App {
        &self.host
    }

    /// Mutable host app access for host-only transitions and reducer fixtures.
    pub fn host_mut(&mut self) -> &mut App {
        &mut self.host
    }

    /// Immutable remote app access.
    #[must_use]
    pub fn client(&self, index: usize) -> Option<&App> {
        self.clients.get(index).map(|client| &client.app)
    }

    /// Mutable remote app access for sending protocol messages.
    #[must_use]
    pub fn client_mut(&mut self, index: usize) -> Option<&mut App> {
        self.clients.get_mut(index).map(|client| &mut client.app)
    }

    /// Captured client messages.
    #[must_use]
    pub fn client_probe(&self, index: usize) -> Option<&ClientProbe> {
        self.client(index)
            .map(App::world)
            .and_then(|world| world.get_resource::<ClientProbe>())
    }

    /// Server-side connection entity corresponding to a client app.
    #[must_use]
    pub fn server_connection(&self, index: usize) -> Option<Entity> {
        self.clients
            .get(index)
            .map(|client| client.server_connection)
    }

    /// Client-side session entity corresponding to a remote app.
    #[must_use]
    pub fn client_connection(&self, index: usize) -> Option<Entity> {
        self.clients
            .get(index)
            .map(|client| client.client_connection)
    }

    fn send_hello(&mut self, index: usize, credential: AdmissionCredential) {
        let Some(client) = self.clients.get_mut(index) else {
            return;
        };
        let protocol_hash = *client.app.world().resource::<ProtocolHash>();
        client.app.world_mut().write_message(ClientHello {
            protocol_hash,
            build: self.manifest.build.clone(),
            content_fingerprint: self.manifest.content_fingerprint,
            credential,
        });
    }
}

impl fmt::Debug for ChannelSessionHarness {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChannelSessionHarness")
            .field("server", &self.server)
            .field("manifest", &self.manifest)
            .field("client_count", &self.clients.len())
            .finish_non_exhaustive()
    }
}

fn session_app(replica: bool) -> App {
    let mut app = App::new();
    app.add_plugins((TimePlugin, StatesPlugin, MultiplayerPlugin, ChannelIoPlugin))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            50,
        )))
        .init_resource::<ClientProbe>()
        .init_resource::<HostProbe>()
        .add_systems(Update, (capture_client_messages, capture_host_commands));
    app.finish();
    app.cleanup();
    if replica {
        app.world_mut().insert_resource(SimulationRole::Replica);
    }
    app
}

fn capture_client_messages(
    mut accepted: MessageReader<AdmissionAccepted>,
    mut refused: MessageReader<AdmissionRefusal>,
    mut lobbies: MessageReader<LobbySnapshot>,
    mut manifests: MessageReader<SessionManifestV1>,
    mut command_results: MessageReader<CommandResult>,
    mut player_knowledge: MessageReader<PlayerKnowledgeSnapshotV1>,
    mut live_snapshots: MessageReader<LiveSessionSnapshotV1>,
    mut world_deltas: MessageReader<WorldDeltaV1>,
    mut control_results: MessageReader<SessionControlResult>,
    mut closed: MessageReader<SessionClosed>,
    mut probe: bevy_ecs::prelude::ResMut<ClientProbe>,
) {
    probe.accepted.extend(accepted.read().copied());
    probe.refused.extend(refused.read().copied());
    probe.lobbies.extend(lobbies.read().cloned());
    probe.manifests.extend(manifests.read().cloned());
    probe
        .command_results
        .extend(command_results.read().copied());
    probe
        .player_knowledge
        .extend(player_knowledge.read().cloned());
    probe.live_snapshots.extend(live_snapshots.read().cloned());
    probe.world_deltas.extend(world_deltas.read().cloned());
    probe
        .control_results
        .extend(control_results.read().copied());
    probe.closed.extend(closed.read().copied());
}

fn capture_host_commands(
    mut commands: MessageReader<AuthenticatedCommandRequest>,
    mut probe: bevy_ecs::prelude::ResMut<HostProbe>,
) {
    probe.commands.extend(commands.read().cloned());
}

/// Harness setup failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessError {
    /// Host custom-admission state could not be initialized.
    Admission(AdmissionSetupError),
}

impl From<AdmissionSetupError> for HarnessError {
    fn from(error: AdmissionSetupError) -> Self {
        Self::Admission(error)
    }
}

impl fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("in-memory session harness setup failed")
    }
}

impl std::error::Error for HarnessError {}

#[cfg(test)]
mod tests {
    use aeronet::io::Session;
    use hex_core::{Faction, SimSeeds, TilePos, UnitId};

    use super::*;
    use crate::{
        BoundedText, BoundedVec, BuildIdentityV1, ContentFingerprint, MapManifestV1,
        ProtocolVersion, PublicWorldFingerprint, RosterEntryV1, RulesManifestV1, UnitDeploymentV1,
        MAX_IDENTITY_BYTES,
    };

    fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
        BoundedText::new(value).expect("fixture identity should fit")
    }

    fn manifest() -> SessionManifestV1 {
        SessionManifestV1 {
            session_instance_id: crate::SessionInstanceId::from_bytes([3; 16]),
            protocol: ProtocolVersion::default(),
            build: BuildIdentityV1::new("0.4.0", "harness").expect("valid build"),
            content_fingerprint: ContentFingerprint(1),
            scenario_identity: text("sandbox"),
            map: MapManifestV1 {
                catalog_identity: text("small"),
                seed: 1,
                generator_identity: text("v3"),
                generator_version: 3,
                expected_public_fingerprint: PublicWorldFingerprint(2),
            },
            rules: RulesManifestV1 {
                profile_identity: text("default"),
                fingerprint: 3,
            },
            shipped_roster: BoundedVec::new(
                (0_u64..6)
                    .map(|unit| RosterEntryV1 {
                        unit: UnitId(unit),
                        archetype_identity: text("warrior"),
                        character_identity: text(&format!("hero-{unit}")),
                        faction: Faction::Player,
                    })
                    .collect(),
            )
            .expect("six roster entries fit"),
            deployment: BoundedVec::new(
                (0_u64..6)
                    .map(|unit| UnitDeploymentV1 {
                        unit: UnitId(unit),
                        position: TilePos::ORIGIN,
                    })
                    .collect(),
            )
            .expect("six deployments fit"),
            simulation_seeds: SimSeeds::default(),
        }
    }

    #[test]
    fn channel_pair_reaches_replicon_connected_state() {
        let mut harness = ChannelSessionHarness::new(
            manifest(),
            crate::SessionPeerId::from_bytes([1; 16]),
            InviteToken::from_bytes([2; 16]),
        )
        .expect("harness should initialize");
        let client = harness.add_client();
        harness.pump(8);
        assert!(harness.client_connected(client));
        let server_connection = harness
            .server_connection(client)
            .expect("server connection should exist");
        assert!(harness
            .host()
            .world()
            .get::<Session>(server_connection)
            .is_some());
    }

    #[test]
    fn admitted_client_maps_transport_loss_to_typed_host_disconnect() {
        let mut harness = ChannelSessionHarness::new(
            manifest(),
            crate::SessionPeerId::from_bytes([1; 16]),
            InviteToken::from_bytes([2; 16]),
        )
        .expect("harness should initialize");
        let client = harness.add_client();
        harness.pump(8);
        let invite = harness.current_invite();
        harness.send_invite_hello(client, invite);
        harness.pump(8);
        assert!(harness
            .client_probe(client)
            .is_some_and(|probe| !probe.accepted.is_empty()));

        assert!(harness.disconnect_client(client));
        harness.pump(8);
        assert!(harness.client_probe(client).is_some_and(|probe| {
            probe
                .closed
                .iter()
                .any(|closed| closed.reason == crate::SessionCloseReason::HostDisconnected)
        }));
    }
}
