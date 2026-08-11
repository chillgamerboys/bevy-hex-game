//! Disclosure-safe reconnect knowledge and Replicon withdrawal contracts.

use std::collections::{BTreeMap, BTreeSet};

use bevy_app::{App, PluginGroup, PostUpdate};
use bevy_platform::collections::HashMap;
use bevy_replicon::{
    prelude::{
        AuthorizedClient, ClientState, Remote, Replicated, RepliconPlugins, ServerPlugin,
        ServerState,
    },
    test_app::{ServerTestAppExt, TestClientEntity},
};
use bevy_state::app::AppExtStates;
use bevy_state::app::StatesPlugin;
use bevy_state::state::State;
use bevy_time::TimePlugin;
use hex_assets::{
    ArtPalette, PaletteSwatch, SrgbColor, Substance, SubstanceFile, SubstanceTable, SwatchId,
};
use hex_core::{
    ControlOwner, Headroom, HexCoord, HexSpan, KnowledgeState, LightDomain, LocalMapKnowledge,
    PlayerSeat, RunBottom, Screen, TilePos, UnitId,
};
use hex_multiplayer::{
    register_protocol, AuthorizedSessionClient, BoundedVec, SessionPeerId, UnitReplica,
};
use hex_perception::{
    apply_observations, export_player_knowledge_snapshot_v1, import_player_knowledge_snapshot_v1,
    FactionMapKnowledge, FactionObservation, FactionObservations, ObservedUnit, SurfaceSnapshot,
    SurfaceSnapshots,
};
use hex_units::Faction;

#[expect(
    clippy::expect_used,
    reason = "a malformed static integration-test catalog must fail during fixture construction"
)]
fn test_table() -> (SubstanceTable, hex_core::SubstanceId) {
    let swatch_id = SwatchId::new("test/gray").expect("fixture swatch id should be valid");
    let swatch = PaletteSwatch::new(
        "Test Gray",
        SrgbColor::new(0.5, 0.5, 0.5).expect("fixture color should be valid"),
        BTreeSet::from(["test".to_owned()]),
    )
    .expect("fixture swatch should be valid");
    let palette = ArtPalette::new(BTreeMap::from([(swatch_id.clone(), swatch)]))
        .expect("fixture palette should be valid");
    let substances = HashMap::from_iter([
        ("air".to_owned(), Substance::invisible(false, false)),
        (
            "stone".to_owned(),
            Substance::from_swatch(swatch_id, true, true),
        ),
    ]);
    let table = SubstanceTable::from_file(&SubstanceFile { substances }, &palette)
        .expect("fixture substances should resolve");
    let stone = table
        .id("stone")
        .expect("fixture table should contain stone");
    (table, stone)
}

fn surface(position: TilePos, substance: hex_core::SubstanceId) -> SurfaceSnapshot {
    SurfaceSnapshot {
        pos: position,
        span: HexSpan::new(0.8, 2.4),
        substance,
        headroom: Headroom(7),
        is_solid: true,
        blocked: true,
        domain: LightDomain::Interior(hex_core::InteriorRegionId(4)),
    }
}

#[expect(
    clippy::expect_used,
    reason = "the integration fixture deliberately requires one unique hostile observation"
)]
fn observation_with_hostile(position: TilePos, hostile: UnitId) -> FactionObservations {
    let mut player = FactionObservation::new();
    player.insert_surface(position);
    player
        .try_insert_unit(ObservedUnit {
            id: hostile,
            faction: Faction::Hostile,
            pos: position,
            provides_sight: true,
        })
        .expect("hostile fixture should be unique");
    FactionObservations::with_faction(Faction::Player, player)
}

#[test]
fn player_knowledge_round_trip_preserves_run_and_clears_private_facts() {
    let (table, stone) = test_table();
    let player_position = TilePos::new(HexCoord::ORIGIN, 5);
    let hostile_position = TilePos::new(HexCoord::from_axial(3, -1), 6);
    let current = SurfaceSnapshots::try_from_projected_iter([
        (RunBottom(2), surface(player_position, stone)),
        (RunBottom(4), surface(hostile_position, stone)),
    ])
    .expect("surface fixtures should be exact");

    let mut player = FactionObservation::new();
    player.insert_surface(player_position);
    player
        .try_insert_unit(ObservedUnit {
            id: UnitId(9),
            faction: Faction::Hostile,
            pos: player_position,
            provides_sight: true,
        })
        .expect("player-observed unit should be unique");
    let mut hostile = FactionObservation::new();
    hostile.insert_surface(hostile_position);
    let observations = FactionObservations::from_factions(player, hostile.clone());
    let mut knowledge = FactionMapKnowledge::new();
    apply_observations(&mut knowledge, &current, &observations);
    apply_observations(
        &mut knowledge,
        &current,
        &FactionObservations::from_factions(FactionObservation::new(), hostile),
    );

    let snapshot = export_player_knowledge_snapshot_v1(&knowledge, &table)
        .expect("player knowledge should export");
    assert_eq!(snapshot.surfaces.len(), 1);
    let exported = snapshot
        .surfaces
        .first()
        .expect("one player surface should be exported");
    assert_eq!(exported.position, player_position);
    assert_eq!(exported.run_bottom, 2);
    assert_eq!(
        exported.state,
        hex_multiplayer::PlayerKnowledgeStateV1::Remembered
    );

    let stale = TilePos::new(HexCoord::from_axial(-4, 0), 1);
    let mut local = LocalMapKnowledge::new();
    local.set(
        KnowledgeState::Observed,
        hex_core::TraversalEndpoint::new(stale, true, Headroom(1)),
        false,
    );
    import_player_knowledge_snapshot_v1(&snapshot, &table, &mut knowledge, &mut local)
        .expect("player knowledge should import");

    assert!(knowledge.faction(Faction::Hostile).is_empty());
    assert_eq!(knowledge.faction(Faction::Player).unit_count(), 0);
    let restored = knowledge
        .faction(Faction::Player)
        .surface(player_position)
        .expect("player surface should be restored");
    assert_eq!(restored.state(), KnowledgeState::Remembered);
    assert_eq!(restored.run_bottom(), RunBottom(2));
    assert_eq!(restored.snapshot(), surface(player_position, stone));
    assert_eq!(local.state(stale), KnowledgeState::Unknown);
    assert_eq!(local.state(player_position), KnowledgeState::Remembered);
    assert_eq!(
        export_player_knowledge_snapshot_v1(&knowledge, &table)
            .expect("restored knowledge should re-export"),
        snapshot
    );
}

fn replication_app(with_disclosure: bool) -> App {
    let mut app = App::new();
    app.add_plugins((
        TimePlugin,
        StatesPlugin,
        RepliconPlugins.set(ServerPlugin::new(PostUpdate)),
    ));
    app.init_state::<Screen>();
    register_protocol(&mut app);
    if with_disclosure {
        app.add_plugins(hex_perception::plugin);
    }
    app.finish();
    app
}

fn replica(unit: UnitId, faction: Faction, position: TilePos, owner: PlayerSeat) -> UnitReplica {
    UnitReplica {
        unit,
        faction,
        position,
        motion: None,
        owner: ControlOwner(owner),
        lattice: None,
        downed: false,
        turn: None,
        effects: BoundedVec::default(),
    }
}

fn replicated_units(app: &mut App) -> BTreeSet<UnitId> {
    let world = app.world_mut();
    let mut replicas = world.query::<&UnitReplica>();
    replicas.iter(world).map(|replica| replica.unit).collect()
}

fn exchange_replication(server: &mut App, client: &mut App) {
    for _frame in 0..3 {
        server.update();
        server.exchange_with_client(client);
        client.update();
    }
}

#[test]
fn replication_fixture_delivers_a_registered_unit() {
    let mut server = replication_app(false);
    let mut client = replication_app(false);
    server.connect_client(&mut client);
    let client_entity = **client.world().resource::<TestClientEntity>();
    assert!(server
        .world()
        .entity(client_entity)
        .contains::<AuthorizedClient>());
    assert_eq!(
        *server.world().resource::<State<ServerState>>().get(),
        ServerState::Running
    );
    assert_eq!(
        *client.world().resource::<State<ClientState>>().get(),
        ClientState::Connected
    );
    let position = TilePos::new(HexCoord::ORIGIN, 5);
    server.world_mut().spawn((
        Replicated,
        replica(UnitId(1), Faction::Player, position, PlayerSeat::HOST),
    ));

    exchange_replication(&mut server, &mut client);
    let remote_count = {
        let world = client.world_mut();
        let mut remotes = world.query::<&Remote>();
        remotes.iter(world).count()
    };
    assert_eq!(remote_count, 1);
    assert_eq!(replicated_units(&mut client), BTreeSet::from([UnitId(1)]));
}

#[test]
fn hostile_replica_is_observed_withdrawn_and_reobserved() {
    let mut server = replication_app(true);
    let mut client = replication_app(true);
    server.connect_client(&mut client);
    let client_entity = **client.world().resource::<TestClientEntity>();
    assert!(server
        .world()
        .entity(client_entity)
        .contains::<AuthorizedClient>());
    server
        .world_mut()
        .entity_mut(client_entity)
        .insert(AuthorizedSessionClient {
            seat: PlayerSeat::HOST,
            player_identity: SessionPeerId::from_bytes([7; 16]),
        });

    let position = TilePos::new(HexCoord::ORIGIN, 5);
    let player = UnitId(1);
    let hostile = UnitId(2);
    let player_entity = server
        .world_mut()
        .spawn((
            Replicated,
            replica(player, Faction::Player, position, PlayerSeat::HOST),
        ))
        .id();
    let hostile_entity = server
        .world_mut()
        .spawn((
            Replicated,
            replica(hostile, Faction::Hostile, position, PlayerSeat::AI),
        ))
        .id();

    exchange_replication(&mut server, &mut client);
    assert_eq!(replicated_units(&mut client), BTreeSet::from([player]));

    let (_table, stone) = test_table();
    let current =
        SurfaceSnapshots::try_from_projected_iter([(RunBottom(2), surface(position, stone))])
            .expect("visibility surface should be valid");
    let mut knowledge = FactionMapKnowledge::new();
    apply_observations(
        &mut knowledge,
        &current,
        &observation_with_hostile(position, hostile),
    );
    server.world_mut().insert_resource(knowledge);
    exchange_replication(&mut server, &mut client);
    assert_eq!(
        replicated_units(&mut client),
        BTreeSet::from([player, hostile])
    );

    let mut knowledge = server
        .world_mut()
        .remove_resource::<FactionMapKnowledge>()
        .expect("server should retain player knowledge");
    apply_observations(&mut knowledge, &current, &FactionObservations::default());
    server.world_mut().insert_resource(knowledge);
    exchange_replication(&mut server, &mut client);
    assert_eq!(replicated_units(&mut client), BTreeSet::from([player]));
    assert!(server.world().get_entity(player_entity).is_ok());
    assert!(server.world().get_entity(hostile_entity).is_ok());

    let mut knowledge = server
        .world_mut()
        .remove_resource::<FactionMapKnowledge>()
        .expect("server should retain remembered knowledge");
    apply_observations(
        &mut knowledge,
        &current,
        &observation_with_hostile(position, hostile),
    );
    server.world_mut().insert_resource(knowledge);
    exchange_replication(&mut server, &mut client);
    assert_eq!(
        replicated_units(&mut client),
        BTreeSet::from([player, hostile])
    );
    let hostile_replica = client
        .world_mut()
        .query::<&UnitReplica>()
        .iter(client.world())
        .find(|replica| replica.unit == hostile)
        .expect("re-observed hostile should have one replica");
    assert!(hostile_replica.lattice.is_none());
}
