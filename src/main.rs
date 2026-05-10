use bevy::prelude::*;
use bevy::picking::mesh_picking::MeshPickingPlugin;

use magic_game::plugins::world_3d::World3dPlugins;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins(MeshPickingPlugin)
        .add_plugins(World3dPlugins)
        .run();
}
