use bevy::prelude::*;
use std::cmp::{max, min};

use crate::plugins::world_2d::WorldCoord;
use crate::plugins::world_2d::config::{HEX_CIRCUMRADIUS, HEX_GRID_RADIUS, HEX_SPRITE_SCALE};
use crate::plugins::world_2d::mouse::MousePos;

pub struct HexPlugin;

impl Plugin for HexPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<HexCoord>()
            .register_type::<HexGrid>()
            .register_type::<HexTile>()
            .register_type::<HighlightedHex>()
            .add_systems(PreStartup, HexGrid::spawn)
            .add_systems(Startup, init_highlighted)
            .add_systems(Update, highlight_on_click);
    }
}

#[derive(Component, Reflect, Default, Debug, Copy, Clone)]
#[reflect(Component)]
pub struct HexCoord(pub i32, pub i32);

impl HexCoord {
    pub fn from_world(world_coord: WorldCoord) -> HexCoord {
        let x = (f32::sqrt(3.0) * world_coord.0 - world_coord.1) / 3.0 / HEX_CIRCUMRADIUS;
        let y = ((2.0 / 3.0) * world_coord.1) / HEX_CIRCUMRADIUS;
        HexCoord::from_floating((x, y))
    }

    pub fn from_floating((fx, fy): (f32, f32)) -> HexCoord {
        let mut x = fx.round();
        let mut y = fy.round();
        let rem_x = fx - x;
        let rem_y = fy - y;
        if rem_x.abs() >= rem_y.abs() {
            x += (rem_x + 0.5 * rem_y).round();
        } else {
            y += (rem_y + 0.5 * rem_x).round();
        }
        HexCoord(x as i32, y as i32)
    }

    pub fn to_world(&self) -> WorldCoord {
        let x = HEX_CIRCUMRADIUS * f32::sqrt(3.0) * ((self.0 as f32) + (self.1 as f32) / 2.0);
        let y = HEX_CIRCUMRADIUS * (3.0 / 2.0) * (self.1 as f32);
        (x, y)
    }

    pub fn within_radius(&self, radius: i32) -> Vec<HexCoord> {
        let mut within = Vec::new();
        for x in -radius..radius + 1 {
            for y in max(-radius, (-x) - radius)..min(radius, (-x) + radius) + 1 {
                within.push(HexCoord(x + self.0, y + self.1));
            }
        }
        within
    }
}

impl PartialEq for HexCoord {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0 && self.1 == other.1
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexGrid;

impl HexGrid {
    fn spawn(mut commands: Commands, assets: Res<AssetServer>) {
        let mut tiles = Vec::new();
        for hex_coord in HexCoord(0, 0).within_radius(HEX_GRID_RADIUS).into_iter() {
            let tile = HexTile::spawn_at(hex_coord, &mut commands, &assets);
            tiles.push(tile);
        }
        commands
            .spawn((
                Transform::default(),
                Visibility::default(),
                Name::new("HexGrid"),
                HexGrid,
            ))
            .add_children(&tiles);
    }
}


#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HexTile;

impl HexTile {
    fn spawn_at(hex_coord: HexCoord, commands: &mut Commands, assets: &Res<AssetServer>) -> Entity {
        let (x, y) = hex_coord.to_world();
        commands
            .spawn((
                Sprite::from_image(assets.load("textures/sprites/hex.png")),
                Transform::from_xyz(x, y, 0.0)
                    .with_scale(Vec3::new(HEX_SPRITE_SCALE, HEX_SPRITE_SCALE, 1.0)),
                Name::new("HexTile"),
                hex_coord,
                HexTile,
            ))
            .id()
    }
}

#[derive(Resource)]
pub struct Highlighted(Option<HexCoord>);

fn init_highlighted(
    mut commands: Commands,
    assets: Res<AssetServer>,
    query: Query<Entity, With<HexGrid>>,
) {
    commands.insert_resource(Highlighted(None));

    let (x, y) = HexCoord(0, 0).to_world();
    let highlighted_hex = commands
        .spawn((
            Sprite::from_image(assets.load("textures/sprites/hex_highlighted.png")),
            Transform::from_xyz(x, y, -1.0)
                .with_scale(Vec3::new(HEX_SPRITE_SCALE, HEX_SPRITE_SCALE, 1.0)),
            Name::new("Highlighted Hex"),
            HighlightedHex,
        ))
        .id();

    if let Ok(grid) = query.single() {
        commands.entity(grid).add_child(highlighted_hex);
    }
}

#[derive(Component, Reflect, Default)]
#[reflect(Component)]
pub struct HighlightedHex;

fn highlight_on_click(
    mut query: Query<&mut Transform, With<HighlightedHex>>,
    mut highlighted: ResMut<Highlighted>,
    buttons: Res<ButtonInput<MouseButton>>,
    mouse_pos: Res<MousePos>,
) {
    if buttons.just_pressed(MouseButton::Left) {
        let Ok(mut high_transform) = query.single_mut() else {
            return;
        };

        let mouse_hex = HexCoord::from_world(mouse_pos.get_world_coords());
        match &highlighted.0 {
            Some(high_coord) => {
                if *high_coord == mouse_hex {
                    highlighted.0 = None;
                } else {
                    highlighted.0 = Some(mouse_hex);
                }
            }
            None => highlighted.0 = Some(mouse_hex),
        }
        match &highlighted.0 {
            Some(high_coord) => {
                let (x, y) = high_coord.to_world();
                high_transform.translation = Vec3::new(x, y, 2.0);
            }
            None => {
                let (x, y) = HexCoord(0, 0).to_world();
                high_transform.translation = Vec3::new(x, y, -1.0);
            }
        }
    }
}
