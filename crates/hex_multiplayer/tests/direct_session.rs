//! Exact in-memory composition contract for custom admission, sequencing, and reconnect.

use bevy_replicon::prelude::AuthorizedClient;
use hex_core::{CommandRequestId, Faction, GameCommand, SimSeeds, TilePos, UnitId};
use hex_multiplayer::{
    AdmissionRefusalReason, AuthorityCommandResolution, AuthoritySequence, AuthorizedSessionClient,
    BoundedText, BoundedVec, BuildIdentityV1, ChannelSessionHarness, ClientLobbyAction,
    ClientLobbyRequest, ClientMapReady, CommandOutcome, CommandSequencer, ContentFingerprint,
    GameCommandRequest, HostProbe, HostSessionAction, HostSessionControlRequest, InviteToken,
    LobbyPhase, MapManifestV1, ProtocolVersion, PublicWorldFingerprint, RosterEntryV1,
    RulesManifestV1, SeatConnectionState, SessionAdmissionAuthority, SessionCloseReason,
    SessionControlOutcome, SessionManifestV1, SessionPeerId, UnitDeploymentV1, MAX_IDENTITY_BYTES,
};

#[expect(clippy::expect_used, reason = "static fixture identity is valid")]
fn text(value: &str) -> BoundedText<MAX_IDENTITY_BYTES> {
    BoundedText::new(value).expect("fixture identity should fit")
}

#[expect(clippy::expect_used, reason = "static fixture manifest is valid")]
fn manifest() -> SessionManifestV1 {
    SessionManifestV1 {
        session_instance_id: hex_multiplayer::SessionInstanceId::from_bytes([4; 16]),
        protocol: ProtocolVersion::default(),
        build: BuildIdentityV1::new("0.4.0", "direct-session-test").expect("valid build"),
        content_fingerprint: ContentFingerprint(100),
        scenario_identity: text("sandbox"),
        map: MapManifestV1 {
            catalog_identity: text("small"),
            seed: 77,
            generator_identity: text("v3"),
            generator_version: 3,
            expected_public_fingerprint: PublicWorldFingerprint(200),
        },
        rules: RulesManifestV1 {
            profile_identity: text("default"),
            fingerprint: 300,
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
        simulation_seeds: SimSeeds {
            world: 10,
            ai_flavor: 20,
            cosmetic: 30,
        },
    }
}

#[test]
fn host_plus_six_clients_admit_play_retry_restart_reconnect_and_close() {
    let manifest = manifest();
    let expected_world = manifest.map.expected_public_fingerprint;
    let mut harness = ChannelSessionHarness::new(
        manifest,
        SessionPeerId::from_bytes([1; SessionPeerId::BYTE_LENGTH]),
        InviteToken::from_bytes([2; InviteToken::BYTE_LENGTH]),
    )
    .expect("harness should initialize");

    let clients = (0..6).map(|_| harness.add_client()).collect::<Vec<_>>();
    harness.pump(12);
    assert!(clients
        .iter()
        .all(|&client| harness.client_connected(client)));

    for &client in clients.iter().take(5) {
        let invite = harness.current_invite();
        harness.send_invite_hello(client, invite);
        harness.pump(12);
        assert_eq!(
            harness
                .client_probe(client)
                .map(|probe| probe.accepted.len()),
            Some(1)
        );
        let server_connection = harness
            .server_connection(client)
            .expect("server-side connection should exist");
        assert!(harness
            .host()
            .world()
            .get::<AuthorizedClient>(server_connection)
            .is_some());
        assert!(harness
            .host()
            .world()
            .get::<AuthorizedSessionClient>(server_connection)
            .is_some());
    }

    let overflow = clients.get(5).copied().expect("sixth client exists");
    let overflow_invite = harness.current_invite();
    harness.send_invite_hello(overflow, overflow_invite);
    harness.pump(12);
    let overflow_probe = harness
        .client_probe(overflow)
        .expect("overflow client has a probe");
    assert!(overflow_probe.accepted.is_empty());
    assert_eq!(
        overflow_probe.refused.last().map(|refusal| refusal.reason),
        Some(AdmissionRefusalReason::LobbyFull)
    );

    for (index, &client) in clients.iter().take(5).enumerate() {
        harness
            .client_mut(client)
            .expect("admitted client exists")
            .world_mut()
            .write_message(ClientLobbyRequest {
                request_id: CommandRequestId(100 + u64::try_from(index).unwrap_or(u64::MAX)),
                action: ClientLobbyAction::SetReady(true),
            });
    }
    harness.pump(16);
    for &client in clients.iter().take(5) {
        assert!(harness.client_probe(client).is_some_and(|probe| {
            probe
                .control_results
                .iter()
                .any(|result| result.outcome == SessionControlOutcome::Accepted)
        }));
    }
    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(200),
            action: HostSessionAction::BeginLoading {
                public_world_fingerprint: expected_world,
            },
        });
    harness.pump(4);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .phase,
        LobbyPhase::Loading
    );
    for &client in clients.iter().take(5) {
        harness
            .client_mut(client)
            .expect("admitted client exists")
            .world_mut()
            .write_message(ClientMapReady {
                public_world_fingerprint: expected_world,
            });
    }
    harness.pump(16);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .phase,
        LobbyPhase::Active
    );

    let first_client = clients.first().copied().expect("first client exists");
    let first_acceptance = harness
        .client_probe(first_client)
        .and_then(|probe| probe.accepted.first())
        .copied()
        .expect("first client was admitted");
    assert!(harness
        .client_probe(first_client)
        .is_some_and(|probe| { !probe.manifests.is_empty() && !probe.lobbies.is_empty() }));
    let acting_unit = harness
        .host()
        .world()
        .resource::<SessionAdmissionAuthority>()
        .lobby()
        .snapshot()
        .seats
        .get(1)
        .and_then(|seat| seat.assigned_units.first())
        .copied()
        .expect("seat one owns a unit");
    let request = GameCommandRequest {
        request_id: CommandRequestId(42),
        command: GameCommand::Rest { unit: acting_unit },
    };
    harness
        .client_mut(first_client)
        .expect("first client exists")
        .world_mut()
        .write_message(request.clone());
    harness.pump(12);
    let ingress_count = harness
        .host()
        .world()
        .resource::<HostProbe>()
        .commands
        .iter()
        .filter(|command| command.request_id == CommandRequestId(42))
        .count();
    assert_eq!(ingress_count, 1);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .active_connection(first_acceptance.seat),
        harness.server_connection(first_client)
    );
    harness
        .host_mut()
        .world_mut()
        .write_message(AuthorityCommandResolution {
            source_seat: first_acceptance.seat,
            request_id: CommandRequestId(42),
            outcome: CommandOutcome::Accepted,
        });
    harness.pump(12);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<CommandSequencer>()
            .last_sequence(),
        AuthoritySequence(1),
        "host should finalize the gameplay outcome"
    );
    let first_probe = harness
        .client_probe(first_client)
        .expect("first client has a probe");
    assert!(
        first_probe.command_results.iter().any(|result| {
            result.request_id == CommandRequestId(42)
                && result.outcome == CommandOutcome::Accepted
                && result.authority_sequence == AuthoritySequence(1)
        }),
        "client results after authority resolution: {:?}",
        first_probe.command_results
    );

    assert!(harness.restart_client(first_client));
    harness.pump(12);
    {
        let mut authority = harness
            .host_mut()
            .world_mut()
            .resource_mut::<SessionAdmissionAuthority>();
        assert!(matches!(
            authority
                .lobby()
                .snapshot()
                .seats
                .get(1)
                .map(|seat| seat.connection),
            Some(SeatConnectionState::Reserved { .. })
        ));
        authority.lobby_mut().advance_reservations(30_000);
        assert!(authority.lobby().host_can_delegate(first_acceptance.seat));
    }
    harness.send_reconnect_hello(first_client, first_acceptance.reconnect_credential);
    harness.pump(16);
    let rotated = harness
        .client_probe(first_client)
        .and_then(|probe| probe.accepted.first())
        .copied()
        .expect("restarted client should reconnect");
    assert!(rotated.seat == first_acceptance.seat);
    assert!(!rotated
        .reconnect_credential
        .matches(first_acceptance.reconnect_credential));
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .seats
            .get(1)
            .map(|seat| seat.connection),
        Some(SeatConnectionState::Connected),
        "quiescent boundary should revoke temporary delegation"
    );

    harness
        .client_mut(first_client)
        .expect("restarted client exists")
        .world_mut()
        .write_message(request);
    harness.pump(12);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<HostProbe>()
            .commands
            .iter()
            .filter(|command| command.request_id == CommandRequestId(42))
            .count(),
        1,
        "reconnect retry must not re-enter gameplay authority"
    );
    assert!(harness.client_probe(first_client).is_some_and(|probe| probe
        .command_results
        .iter()
        .any(|result| matches!(
            result.outcome,
            CommandOutcome::Duplicate {
                original_sequence: AuthoritySequence(1)
            }
        ))));

    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(201),
            action: HostSessionAction::EnterOutcome,
        });
    harness.pump(4);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .phase,
        LobbyPhase::Outcome
    );
    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(202),
            action: HostSessionAction::RetryExact {
                public_world_fingerprint: expected_world,
            },
        });
    harness.pump(4);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .phase,
        LobbyPhase::Loading
    );
    for &client in clients.iter().take(5) {
        harness
            .client_mut(client)
            .expect("admitted client exists")
            .world_mut()
            .write_message(ClientMapReady {
                public_world_fingerprint: expected_world,
            });
    }
    harness.pump(16);
    assert_eq!(
        harness
            .host()
            .world()
            .resource::<SessionAdmissionAuthority>()
            .lobby()
            .snapshot()
            .phase,
        LobbyPhase::Active
    );

    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(203),
            action: HostSessionAction::EnterOutcome,
        });
    harness.pump(4);
    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(204),
            action: HostSessionAction::ReturnToLobby,
        });
    harness.pump(4);
    let returned_lobby = harness
        .host()
        .world()
        .resource::<SessionAdmissionAuthority>()
        .lobby()
        .snapshot();
    assert_eq!(returned_lobby.phase, LobbyPhase::Open);
    assert!(returned_lobby.seats.iter().skip(1).all(|seat| !seat.ready));

    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(205),
            action: HostSessionAction::CloseSession,
        });
    harness.pump(12);
    assert!(harness.client_probe(first_client).is_some_and(|probe| {
        probe
            .closed
            .iter()
            .any(|closed| closed.reason == SessionCloseReason::HostClosed)
    }));
}
