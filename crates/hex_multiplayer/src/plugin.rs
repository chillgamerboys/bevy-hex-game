//! Composition of protocol, Replicon, Aeronet, and default-off direct transport.

use aeronet::AeronetPlugins;
use aeronet_replicon::{client::AeronetRepliconClientPlugin, server::AeronetRepliconServerPlugin};
use bevy_app::{App, Plugin, PluginGroup};
use bevy_replicon::prelude::{AuthMethod, RepliconPlugins, RepliconSharedPlugin};
use hex_core::{LocalGameCommandRequest, SimulationRole};

use crate::register_protocol;

/// Installs the transport-neutral protocol plus native direct-transport capability.
///
/// Installing this plugin creates no client/server endpoint entity and therefore opens
/// no socket. A later session runtime must explicitly start Host Direct or Join Direct.
/// Offline single-player retains [`SimulationRole::Authority`] and may use the registered
/// local request message without constructing a network session.
pub struct MultiplayerPlugin;

impl Plugin for MultiplayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SimulationRole>()
            .add_message::<LocalGameCommandRequest>()
            .add_plugins(AeronetPlugins)
            .add_plugins(RepliconPlugins.set(RepliconSharedPlugin {
                auth_method: AuthMethod::Custom,
            }))
            .add_plugins((AeronetRepliconClientPlugin, AeronetRepliconServerPlugin));

        #[cfg(feature = "direct")]
        app.add_plugins((
            aeronet_webtransport::client::WebTransportClientPlugin,
            aeronet_webtransport::server::WebTransportServerPlugin,
        ))
        .add_observer(crate::direct::respond_to_direct_session);

        register_protocol(app);
        crate::runtime::install_runtime(app);
    }
}

#[cfg(test)]
mod tests {
    use aeronet::io::{
        server::{Server, ServerEndpoint},
        SessionEndpoint,
    };
    use bevy_app::App;
    use bevy_ecs::prelude::{Entity, With};
    use bevy_replicon::prelude::ProtocolHash;
    use bevy_state::app::StatesPlugin;

    use super::*;

    fn protocol_app() -> App {
        let mut app = App::new();
        app.add_plugins((StatesPlugin, MultiplayerPlugin));
        app.finish();
        app
    }

    #[test]
    fn protocol_registration_is_identical_and_matches_golden_hash() {
        let first = protocol_app();
        let second = protocol_app();
        assert_eq!(
            first.world().resource::<ProtocolHash>(),
            second.world().resource::<ProtocolHash>()
        );
        assert_eq!(
            format!("{:?}", first.world().resource::<ProtocolHash>()),
            "ProtocolHash(4077301579023059970)"
        );
    }

    #[test]
    fn plugin_defaults_to_authority_without_opening_a_socket() {
        let mut app = protocol_app();
        assert_eq!(
            *app.world().resource::<SimulationRole>(),
            SimulationRole::Authority
        );

        let sessions = app
            .world_mut()
            .query_filtered::<Entity, With<SessionEndpoint>>()
            .iter(app.world())
            .count();
        let servers = app
            .world_mut()
            .query_filtered::<Entity, (With<ServerEndpoint>, With<Server>)>()
            .iter(app.world())
            .count();
        assert_eq!(sessions, 0);
        assert_eq!(servers, 0);
    }
}
