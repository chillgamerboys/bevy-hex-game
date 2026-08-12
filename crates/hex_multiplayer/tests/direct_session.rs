//! Exact in-memory composition contract for custom admission, sequencing, and reconnect.

use bevy_app::App;
use bevy_replicon::prelude::{AuthorizedClient, ClientId, Replicated, SendTargets, ToClients};
use hex_core::{
    CommandRequestId, ControlOwner, Faction, GameCommand, HexCoord, Mode, Pause, PendingDecision,
    PlayerSeat, SimSeeds, TilePos, Turn, UnitId,
};
use hex_lattice::LatticeState;
use hex_multiplayer::{
    AdmissionRefusalReason, ArchetypeIdentityV1, AuthorityCommandResolution, AuthoritySequence,
    AuthorizedSessionClient, BoundedText, BoundedVec, BuildIdentityV1, ChannelSessionHarness,
    ClientLobbyAction, ClientLobbyRequest, ClientMapReady, CommandOutcome, CommandSequencer,
    ContentFingerprint, GameCommandRequest, HostProbe, HostSessionAction,
    HostSessionControlRequest, InviteToken, LiveSessionSnapshotV1, LobbyPhase, MapManifestV1,
    MotionReplicaV1, PlayerKnowledgeSnapshotV1, ProtocolVersion, PublicWorldFingerprint,
    RosterEntryV1, RulesManifestV1, SeatConnectionState, SessionAdmissionAuthority,
    SessionCloseReason, SessionControlOutcome, SessionManifestV1, SessionOutcome, SessionPeerId,
    SessionReplica, UnitDeploymentV1, UnitReplica, WorldColumnSnapshotV1, WorldDamageSnapshotV1,
    WorldDeltaOperationV1, WorldDeltaV1, WorldRunSnapshotV1, WorldSnapshotV1,
    LIVE_SESSION_SNAPSHOT_VERSION_V1, MAX_IDENTITY_BYTES, PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1,
    WORLD_DELTA_VERSION_V1, WORLD_SNAPSHOT_VERSION_V1,
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

#[expect(clippy::expect_used, reason = "static fixture archetype is valid")]
fn archetype(value: &str) -> ArchetypeIdentityV1 {
    ArchetypeIdentityV1::new(value).expect("fixture archetype should fit")
}

fn unit_replica(
    unit: UnitId,
    faction: Faction,
    owner: PlayerSeat,
    position: TilePos,
) -> UnitReplica {
    UnitReplica {
        unit,
        archetype: archetype(if faction == Faction::Player {
            "warrior"
        } else {
            "raider"
        }),
        faction,
        position,
        motion: None,
        owner: ControlOwner(owner),
        lattice: (faction == Faction::Player).then(LatticeState::default),
        downed: false,
        turn: None,
        effects: BoundedVec::default(),
    }
}

fn session_replica(sequence: AuthoritySequence) -> SessionReplica {
    SessionReplica {
        authority_sequence: sequence,
        mode: Mode::Exploring,
        pause: Pause(false),
        initiative: BoundedVec::default(),
        active_turn: None,
        round: 0,
        pending_decision: PendingDecision::None,
        outcome: None,
    }
}

#[expect(clippy::expect_used, reason = "static snapshot fixture is valid")]
fn world_snapshot(fingerprint: PublicWorldFingerprint) -> WorldSnapshotV1 {
    let coord = HexCoord::ORIGIN;
    WorldSnapshotV1 {
        version: WORLD_SNAPSHOT_VERSION_V1,
        public_fingerprint: fingerprint,
        columns: BoundedVec::new(vec![WorldColumnSnapshotV1 {
            coord,
            runs: BoundedVec::new(vec![WorldRunSnapshotV1 {
                position: TilePos::new(coord, 2),
                run_bottom: 0,
                span_bottom_bits: 0.0_f32.to_bits(),
                span_top_bits: 3.0_f32.to_bits(),
                substance: text("stone"),
                headroom: 4,
            }])
            .expect("one run should fit"),
        }])
        .expect("one column should fit"),
        damage: BoundedVec::default(),
        anchors: BoundedVec::default(),
        interior_surfaces: BoundedVec::default(),
        interior_roofs: BoundedVec::default(),
        special_regions: BoundedVec::default(),
        biome_regions: BoundedVec::default(),
        blockers: BoundedVec::default(),
        view_hint: None,
        lights: BoundedVec::default(),
        liquids: BoundedVec::default(),
        objects: BoundedVec::default(),
    }
}

fn player_knowledge() -> PlayerKnowledgeSnapshotV1 {
    PlayerKnowledgeSnapshotV1 {
        version: PLAYER_KNOWLEDGE_SNAPSHOT_VERSION_V1,
        surfaces: BoundedVec::default(),
    }
}

fn projected_units(app: &App) -> Vec<UnitReplica> {
    let mut units = app
        .world()
        .iter_entities()
        .filter_map(|entity| entity.get::<UnitReplica>().cloned())
        .collect::<Vec<_>>();
    units.sort_by_key(|unit| unit.unit);
    units
}

fn projected_session(app: &App) -> Option<SessionReplica> {
    app.world()
        .iter_entities()
        .find_map(|entity| entity.get::<SessionReplica>().cloned())
}

#[expect(
    clippy::expect_used,
    reason = "the harness owns every client index supplied by this fixture"
)]
fn assert_projection_equality(
    harness: &ChannelSessionHarness,
    admitted: &[usize],
    overflow: usize,
) {
    let expected_units = projected_units(harness.host());
    let expected_session = projected_session(harness.host());
    for &client in admitted {
        let client_app = harness.client(client).expect("admitted client app exists");
        assert_eq!(
            projected_units(client_app),
            expected_units,
            "client {client} unit projection must equal the authorized host view"
        );
        assert_eq!(
            projected_session(client_app),
            expected_session,
            "client {client} session projection must equal the authorized host view"
        );
    }
    let overflow_app = harness
        .client(overflow)
        .expect("overflow client app should remain connected");
    assert!(projected_units(overflow_app).is_empty());
    assert_eq!(projected_session(overflow_app), None);
}

#[expect(clippy::expect_used, reason = "fixture sequence cannot exhaust u64")]
fn advance_system_boundary(harness: &mut ChannelSessionHarness) -> AuthoritySequence {
    harness
        .host_mut()
        .world_mut()
        .resource_mut::<CommandSequencer>()
        .advance_system_boundary()
        .expect("fixture sequence should advance")
}

#[test]
fn host_plus_six_clients_admit_play_retry_restart_reconnect_and_close() {
    let manifest = manifest();
    let expected_world = manifest.map.expected_public_fingerprint;
    let mut harness = ChannelSessionHarness::new(
        manifest.clone(),
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
    let admitted = clients.iter().take(5).copied().collect::<Vec<_>>();
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
    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(20_000),
            action: HostSessionAction::ReportHostMapReady {
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
        LobbyPhase::Loading,
        "the host report cannot activate while claimed guests are still loading"
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

    let lobby = harness
        .host()
        .world()
        .resource::<SessionAdmissionAuthority>()
        .lobby()
        .snapshot()
        .clone();
    let player_entities = (0_u64..6)
        .map(|unit| {
            let unit = UnitId(unit);
            let owner = lobby
                .seats
                .iter()
                .find(|seat| seat.assigned_units.contains(&unit))
                .map(|seat| seat.seat)
                .expect("every shipped party member has one canonical owner");
            let entity = harness
                .host_mut()
                .world_mut()
                .spawn((
                    Replicated,
                    unit_replica(unit, Faction::Player, owner, TilePos::ORIGIN),
                ))
                .id();
            (unit, entity)
        })
        .collect::<Vec<_>>();
    let session_entity = harness
        .host_mut()
        .world_mut()
        .spawn((Replicated, session_replica(AuthoritySequence(1))))
        .id();
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    let acting_entity = player_entities
        .iter()
        .find(|(unit, _entity)| *unit == acting_unit)
        .map(|(_unit, entity)| *entity)
        .expect("acting unit owns a replicated entity");
    let destination = TilePos::new(HexCoord::from_axial(1, 0), 0);
    let moving_sequence = advance_system_boundary(&mut harness);
    {
        let mut replica = harness
            .host_mut()
            .world_mut()
            .get_mut::<UnitReplica>(acting_entity)
            .expect("acting replica exists");
        replica.motion = Some(MotionReplicaV1 {
            route: BoundedVec::new(vec![TilePos::ORIGIN, destination])
                .expect("fixture route should fit"),
            speed_bits: 2.0_f32.to_bits(),
            elapsed_bits: 0.25_f64.to_bits(),
            started: true,
            reconciled_step: 0,
        });
    }
    harness
        .host_mut()
        .world_mut()
        .get_mut::<SessionReplica>(session_entity)
        .expect("session replica exists")
        .authority_sequence = moving_sequence;
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    let settled_sequence = advance_system_boundary(&mut harness);
    {
        let mut replica = harness
            .host_mut()
            .world_mut()
            .get_mut::<UnitReplica>(acting_entity)
            .expect("acting replica exists");
        replica.position = destination;
        replica.motion = None;
    }
    harness
        .host_mut()
        .world_mut()
        .get_mut::<SessionReplica>(session_entity)
        .expect("session replica exists")
        .authority_sequence = settled_sequence;
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    let hostile_unit = UnitId(100);
    let hostile_entity = harness
        .host_mut()
        .world_mut()
        .spawn((
            Replicated,
            unit_replica(
                hostile_unit,
                Faction::Hostile,
                PlayerSeat::AI,
                TilePos::new(HexCoord::from_axial(2, 0), 0),
            ),
        ))
        .id();
    let combat_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        session.authority_sequence = combat_sequence;
        session.mode = Mode::Combat;
        session.initiative =
            BoundedVec::new(vec![acting_unit, hostile_unit]).expect("initiative should fit");
        session.active_turn = Some(acting_unit);
        session.round = 1;
    }
    harness
        .host_mut()
        .world_mut()
        .get_mut::<UnitReplica>(acting_entity)
        .expect("acting replica exists")
        .turn = Some(Turn {
        movement_left: 4,
        acted: false,
    });
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    let decision_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        session.authority_sequence = decision_sequence;
        session.active_turn = Some(hostile_unit);
        session.pending_decision = PendingDecision::ChooseDisables {
            decider: acting_unit,
            count: 1,
            source: hostile_unit,
        };
    }
    harness
        .host_mut()
        .world_mut()
        .get_mut::<UnitReplica>(acting_entity)
        .expect("acting replica exists")
        .turn = None;
    harness
        .host_mut()
        .world_mut()
        .get_mut::<UnitReplica>(hostile_entity)
        .expect("hostile replica exists")
        .turn = Some(Turn {
        movement_left: 3,
        acted: true,
    });
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    assert!(harness.host_mut().world_mut().despawn(hostile_entity));
    let withdrawn_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        session.authority_sequence = withdrawn_sequence;
        session.initiative = BoundedVec::new(vec![acting_unit]).expect("initiative should fit");
        session.active_turn = Some(acting_unit);
        session.pending_decision = PendingDecision::None;
    }
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

    let hostile_entity = harness
        .host_mut()
        .world_mut()
        .spawn((
            Replicated,
            unit_replica(
                hostile_unit,
                Faction::Hostile,
                PlayerSeat::AI,
                TilePos::new(HexCoord::from_axial(2, 0), 0),
            ),
        ))
        .id();
    let observed_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        session.authority_sequence = observed_sequence;
        session.initiative =
            BoundedVec::new(vec![acting_unit, hostile_unit]).expect("initiative should fit");
        session.active_turn = Some(hostile_unit);
    }
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

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

    let baseline_session = projected_session(harness.host()).expect("host session exists");
    let baseline_units = projected_units(harness.host());
    let knowledge = player_knowledge();
    let live_snapshot = LiveSessionSnapshotV1 {
        version: LIVE_SESSION_SNAPSHOT_VERSION_V1,
        manifest: manifest.clone(),
        world: world_snapshot(expected_world),
        player_knowledge: knowledge.clone(),
        units: BoundedVec::new(baseline_units).expect("authorized units should fit"),
        session: baseline_session,
        baseline_sequence: observed_sequence,
    };
    let reconnected_connection = harness
        .server_connection(first_client)
        .expect("reconnected server-side session exists");
    harness.host_mut().world_mut().write_message(ToClients {
        targets: SendTargets::Single(ClientId::Client(reconnected_connection)),
        message: knowledge.clone(),
    });
    harness.host_mut().world_mut().write_message(ToClients {
        targets: SendTargets::Single(ClientId::Client(reconnected_connection)),
        message: live_snapshot.clone(),
    });
    harness.pump(16);
    let reconnect_probe = harness
        .client_probe(first_client)
        .expect("reconnected client has a probe");
    assert_eq!(reconnect_probe.player_knowledge.last(), Some(&knowledge));
    assert_eq!(reconnect_probe.live_snapshots.last(), Some(&live_snapshot));
    assert_projection_equality(&harness, &admitted, overflow);

    let delta_sequence = advance_system_boundary(&mut harness);
    assert!(delta_sequence > observed_sequence);
    harness
        .host_mut()
        .world_mut()
        .get_mut::<SessionReplica>(session_entity)
        .expect("session replica exists")
        .authority_sequence = delta_sequence;
    let world_delta = WorldDeltaV1 {
        version: WORLD_DELTA_VERSION_V1,
        authority_sequence: delta_sequence,
        base_fingerprint: expected_world,
        target_fingerprint: PublicWorldFingerprint(201),
        operations: BoundedVec::new(vec![WorldDeltaOperationV1::UpsertDamage(
            WorldDamageSnapshotV1 {
                position: TilePos::new(HexCoord::ORIGIN, 1),
                remaining: 1,
                maximum: 2,
            },
        )])
        .expect("one delta operation should fit"),
    };
    harness.host_mut().world_mut().write_message(ToClients {
        targets: SendTargets::CLIENTS_ONLY,
        message: world_delta.clone(),
    });
    harness.pump(16);
    for &client in &admitted {
        assert_eq!(
            harness
                .client_probe(client)
                .and_then(|probe| probe.world_deltas.last()),
            Some(&world_delta),
            "client {client} should receive the ordered terrain delta"
        );
    }
    assert!(harness
        .client_probe(overflow)
        .is_some_and(|probe| probe.world_deltas.is_empty()));
    assert_projection_equality(&harness, &admitted, overflow);

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
    let duplicate_sequence = harness
        .host()
        .world()
        .resource::<CommandSequencer>()
        .last_sequence();
    assert!(duplicate_sequence > delta_sequence);
    harness
        .host_mut()
        .world_mut()
        .get_mut::<SessionReplica>(session_entity)
        .expect("session replica exists")
        .authority_sequence = duplicate_sequence;
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

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
    let outcome_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        session.authority_sequence = outcome_sequence;
        session.active_turn = None;
        session.pending_decision = PendingDecision::None;
        session.outcome = Some(SessionOutcome::Victory);
    }
    {
        let mut hostile = harness
            .host_mut()
            .world_mut()
            .get_mut::<UnitReplica>(hostile_entity)
            .expect("hostile replica exists");
        hostile.downed = true;
        hostile.turn = None;
    }
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);
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
    harness
        .host_mut()
        .world_mut()
        .write_message(HostSessionControlRequest {
            request_id: CommandRequestId(20_001),
            action: HostSessionAction::ReportHostMapReady {
                public_world_fingerprint: expected_world,
            },
        });
    harness.pump(4);
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
    assert!(harness.host_mut().world_mut().despawn(hostile_entity));
    let retry_sequence = advance_system_boundary(&mut harness);
    {
        let mut session = harness
            .host_mut()
            .world_mut()
            .get_mut::<SessionReplica>(session_entity)
            .expect("session replica exists");
        *session = session_replica(retry_sequence);
    }
    {
        let mut acting = harness
            .host_mut()
            .world_mut()
            .get_mut::<UnitReplica>(acting_entity)
            .expect("acting replica exists");
        acting.position = TilePos::ORIGIN;
        acting.motion = None;
        acting.turn = None;
    }
    harness.pump(16);
    assert_projection_equality(&harness, &admitted, overflow);

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
