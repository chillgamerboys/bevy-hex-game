// Hex
pub const HEX_INNER_RADIUS: f32 = 0.88;
pub const HEX_CIRCUMRADIUS: f32 = HEX_INNER_RADIUS * 1.1547005; // sqrt(4/3)
pub const HEX_SMALL_DIAMETER: f32 = 2.0 * HEX_INNER_RADIUS;
pub const HEX_LARGE_DIAMETER: f32 = 2.0 * HEX_CIRCUMRADIUS;
pub const HEX_GRID_RADIUS: i32 = 20;
pub const HEX_HEIGHT_SCALE: f32 = 0.4;


// Camera
pub const CAMERA_SPEED: f32 = 0.4;
pub const CAMERA_SPEED_OFFSET: f32 = 10.;
pub const MAX_PITCH: f32 = 0.95;
pub const MIN_PITCH: f32 = 0.25;
pub const MAX_ZOOM_IN: f32 = 5.;
pub const MAX_ZOOM_OUT: f32 = 50.;


// Sun & sky brightness (Bevy 0.18 uses physical units)
// - Illuminance is in lux: ~100_000 = direct noon sun, ~10_000 = overcast.
// - Ambient brightness is in lux as well; a small fill light below the sun.
// - Skybox brightness is in cd/m². The cubemap PNG already encodes a bright sky,
//   so this stays low to avoid blowing the scene out.
pub const SUN_INTENSITY: f32 = 10_000.;
pub const SUN_ROTATION: (f32, f32, f32) = (11.4, 0.3, 0.);
pub const SUN_AMBIENT_LIGHT: f32 = 80.;
pub const SKYBOX_BRIGHTNESS: f32 = 300.;

// Player
pub const PLAYER_SCALE: f32 = 0.25;
/// World units per second. Was 0.005 units/ms back when animation ran off a
/// wall-clock in milliseconds; 5.0 units/s is the same speed in the units the
/// `Res<Time>`-driven transformation system now uses.
pub const PLAYER_SPEED: f32 = 5.0;