use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::plugins::world_2d::WorldCoord;

pub struct MousePlugin;

impl Plugin for MousePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, init_mouse_pos)
            .add_systems(Update, update_mouse_pos);
    }
}

#[derive(Resource)]
pub struct MousePos(WorldCoord);

impl MousePos {
    pub fn get_world_coords(&self) -> WorldCoord {
        self.0
    }
}

fn init_mouse_pos(
    mut commands: Commands,
    windows: Query<&Window, With<PrimaryWindow>>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
) {
    let mouse_pos = get_mouse_pos(&windows, &q_camera);
    commands.insert_resource(MousePos(mouse_pos));
}

fn update_mouse_pos(
    mut cur_mouse_pos: ResMut<MousePos>,
    windows: Query<&Window, With<PrimaryWindow>>,
    q_camera: Query<(&Camera, &GlobalTransform)>,
) {
    cur_mouse_pos.0 = get_mouse_pos(&windows, &q_camera);
}

fn get_mouse_pos(
    windows: &Query<&Window, With<PrimaryWindow>>,
    q_camera: &Query<(&Camera, &GlobalTransform)>,
) -> WorldCoord {
    let Ok((camera, camera_transform)) = q_camera.single() else {
        return (0.0, 0.0);
    };
    let Ok(window) = windows.single() else {
        return (0.0, 0.0);
    };
    let Some(cursor) = window.cursor_position() else {
        return (0.0, 0.0);
    };
    match camera.viewport_to_world_2d(camera_transform, cursor) {
        Ok(world_pos) => (world_pos.x, world_pos.y),
        Err(_) => (0.0, 0.0),
    }
}
