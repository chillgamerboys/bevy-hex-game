//! Multiplayer authority contracts owned by the combat reducer.

use bevy::prelude::*;
use hex_core::{
    CommandQueue, GameCommand, IssuedCommand, Mode, PlayerSeat, Screen, SimulationRole, UnitId,
};
use hex_test_support::TestAppBuilder;

#[test]
fn replica_role_never_consumes_the_authoritative_command_queue() {
    let mut builder = TestAppBuilder::new();
    let app = builder.app_mut();
    app.insert_resource(hex_assets::CombatSettings::default())
        .insert_resource(SimulationRole::Replica)
        .add_plugins(hex_combat::plugin);
    let mut app = builder.build();
    app.world_mut()
        .resource_mut::<NextState<Screen>>()
        .set(Screen::Gameplay);
    app.update();
    app.update();
    app.world_mut()
        .resource_mut::<CommandQueue>()
        .push(IssuedCommand {
            seat: PlayerSeat::HOST,
            command: GameCommand::EndTurn { unit: UnitId(1) },
        });

    app.update();

    assert!(
        !app.world().resource::<CommandQueue>().is_empty(),
        "a replica may present and forward intent, but cannot run the authority reducer"
    );

    app.world_mut()
        .resource_mut::<NextState<Mode>>()
        .set(Mode::Combat);
    app.update();
    app.update();
    assert!(
        hex_combat::authority_snapshot(app.world()).is_err(),
        "a replica must never construct the host-only CombatState"
    );
}
