//! Spell VFX authoring screen.
//!
//! A caster and a target dummy on a bare plate — no terrain, no combat, no mana, no
//! turn order — with the selected spell's animation parameters editable beside them
//! and a Play button that replays the cast on demand. Reached from Main Menu Tools,
//! beside the Character and Spell Creators, which is the same authoring role.
//!
//! `crate::spell_vfx::spawn_cast_vfx` is called directly here, bypassing
//! `hex_combat` entirely: this screen has no notion of a caster's mana, action
//! economy, or the target's consent. That is the point — waiting for a real turn in
//! a real encounter between two tweaks makes tuning a visual effect impractical.
//!
//! The two dummies are plain colored capsules, not real unit art. This screen only
//! needs to show *where* a cast's motion starts and lands, and building real
//! character rendering would pull most of `hex_units` into a tool.
//!
//! Editing writes straight into the live [`SpellAnimationFile`] resource, which is
//! the same resource the real gameplay trigger reads. Mutating it marks it changed,
//! which drops every cached particle-effect handle (see
//! `spell_vfx::trigger::clear_stale_effect_cache`), so the very next Play rebuilds
//! the effect from the edited values. Save then writes that live resource back to
//! `assets/config/spell_animations.ron`.

use std::io::Write;
use std::path::PathBuf;

use atomicwrites::{AllowOverwrite, AtomicFile};
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use hex_assets::{
    ElementCatalog, MotionArchetype, SpellAnimation, SpellAnimationFile, SpellBook, VfxStyle,
    MAX_ARC_BRANCHES, MAX_ARC_SUBDIVISIONS, MAX_PARTICLE_COUNT, MIN_DURATION_SECONDS, MIN_SIZE,
};
use hex_core::{InputAction, InputBindings, Screen};
use hex_ui::{
    DespawnOnExit, UiIntent, UiSystems, VfxTunerControl, VfxTunerField, VfxTunerIntent,
    VfxTunerRowView, VfxTunerSpellView, VfxTunerView,
};
use ron::ser::PrettyConfig;

use crate::spell_vfx::{resolve_cast_color, spawn_cast_vfx, SpellVfxAssetCache, SpellVfxLifetime};

use super::despawn_screen;

/// World-space gap between the two dummies, in world units. A hex is about 2 units
/// across (`HEX_CIRCUMRADIUS = 1.0`), so this sits inside the shipped spells'
/// authored ranges rather than being an arbitrary distance.
const DUMMY_GAP: f32 = 6.0;

/// The point the tuner camera orbits: between the dummies, at roughly chest height,
/// which is where an impact burst lands.
const ORBIT_FOCUS: Vec3 = Vec3::new(0.0, 0.9, 0.0);

/// How far the camera may pitch toward the poles before `looking_at` degenerates.
const MAX_ORBIT_PITCH: f32 = 1.45;

/// Closest and furthest the tuner camera may be pulled, in world units.
const MIN_ORBIT_RADIUS: f32 = 3.0;
const MAX_ORBIT_RADIUS: f32 = 28.0;

/// Marks the preview's caster dummy.
#[derive(Component)]
struct PreviewCaster;

/// Marks the preview's target dummy.
#[derive(Component)]
struct PreviewTarget;

/// Which spell is being tuned, and how the live values relate to disk.
#[derive(Resource, Default)]
struct VfxTunerSession {
    spell: Option<String>,
    /// The values as they were last read from or written to disk. Revert restores
    /// this, and it is what "unsaved edits" is measured against.
    baseline: Option<SpellAnimationFile>,
    status: Option<String>,
}

/// The shared startup camera's pose, borrowed for the duration of this screen.
///
/// This screen reuses the one camera `hex_world` spawns at startup rather than
/// adding a second: that camera is the render target every UI screen already draws
/// through, and two 3D cameras at the same order render ambiguously.
#[derive(Resource)]
struct TunerCamera {
    camera: Entity,
    restore: Transform,
    yaw: f32,
    pitch: f32,
    radius: f32,
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<VfxTunerSession>();
    app.add_systems(OnEnter(Screen::VfxTuner), (enter_scene, take_camera));
    app.add_systems(
        OnExit(Screen::VfxTuner),
        (
            despawn_screen(Screen::VfxTuner),
            release_camera,
            despawn_live_vfx,
            reset_session,
        ),
    );
    app.add_systems(
        Update,
        (orbit_tuner_camera, handle_cancel, publish_view).run_if(in_state(Screen::VfxTuner)),
    );
    app.add_systems(
        Update,
        handle_intents
            .after(UiSystems::EmitIntents)
            .run_if(in_state(Screen::VfxTuner)),
    );
}

fn reset_session(mut session: ResMut<VfxTunerSession>) {
    *session = VfxTunerSession::default();
}

fn enter_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut session: ResMut<VfxTunerSession>,
    animations: Option<Res<SpellAnimationFile>>,
) {
    session.baseline = animations.map(|animations| animations.clone());

    let despawn = DespawnOnExit(Screen::VfxTuner);
    let ground_mesh = meshes.add(Plane3d::default().mesh().size(24.0, 24.0));
    let ground_material = materials.add(Color::srgb(0.16, 0.17, 0.2));
    commands.spawn((
        Name::new("VFX Tuner Ground"),
        Mesh3d(ground_mesh),
        MeshMaterial3d(ground_material),
        Transform::IDENTITY,
        despawn,
    ));
    commands.spawn((
        Name::new("VFX Tuner Light"),
        DirectionalLight {
            illuminance: 9000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -1.0, 0.4, 0.0)),
        despawn,
    ));

    let dummy_mesh = meshes.add(Capsule3d::new(0.35, 1.1));
    let caster_material = materials.add(Color::srgb(0.25, 0.45, 0.95));
    let target_material = materials.add(Color::srgb(0.95, 0.3, 0.25));
    commands.spawn((
        Name::new("VFX Tuner Caster"),
        PreviewCaster,
        Mesh3d(dummy_mesh.clone()),
        MeshMaterial3d(caster_material),
        Transform::from_xyz(-DUMMY_GAP * 0.5, 0.85, 0.0),
        despawn,
    ));
    commands.spawn((
        Name::new("VFX Tuner Target"),
        PreviewTarget,
        Mesh3d(dummy_mesh),
        MeshMaterial3d(target_material),
        Transform::from_xyz(DUMMY_GAP * 0.5, 0.85, 0.0),
        despawn,
    ));
}

fn take_camera(mut commands: Commands, cameras: Query<(Entity, &Transform), With<Camera3d>>) {
    let Ok((camera, transform)) = cameras.single() else {
        return;
    };
    let state = TunerCamera {
        camera,
        restore: *transform,
        yaw: 0.0,
        pitch: 0.45,
        radius: 11.0,
    };
    commands.entity(camera).insert(orbit_transform(&state));
    commands.insert_resource(state);
}

fn release_camera(
    mut commands: Commands,
    state: Option<Res<TunerCamera>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(state) = state else {
        return;
    };
    if let Ok(mut transform) = cameras.get_mut(state.camera) {
        *transform = state.restore;
    }
    commands.remove_resource::<TunerCamera>();
}

/// Despawns every effect this screen spawned, so leaving mid-cast does not leave a
/// particle emitter alive behind the Main Menu.
fn despawn_live_vfx(mut commands: Commands, effects: Query<Entity, With<SpellVfxLifetime>>) {
    for effect in &effects {
        commands.entity(effect).despawn();
    }
}

fn orbit_transform(state: &TunerCamera) -> Transform {
    let offset = Vec3::new(
        state.yaw.sin() * state.pitch.cos(),
        state.pitch.sin(),
        state.yaw.cos() * state.pitch.cos(),
    ) * state.radius;
    Transform::from_translation(ORBIT_FOCUS + offset).looking_at(ORBIT_FOCUS, Vec3::Y)
}

/// Right-drag orbits, scroll zooms — the same gestures the gameplay camera uses.
///
/// Reads `CursorMoved` rather than `MouseMotion` for the same reason
/// `hex_world::camera::orbit_camera` does: Wayland (and therefore WSLg) does not
/// deliver `MouseMotion` while a button is held.
fn orbit_tuner_camera(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cursor: MessageReader<CursorMoved>,
    mut wheel: MessageReader<MouseWheel>,
    buttons: Res<ButtonInput<MouseButton>>,
    mut last_cursor: Local<Option<Vec2>>,
    state: Option<ResMut<TunerCamera>>,
    mut cameras: Query<&mut Transform, With<Camera3d>>,
) {
    let Some(mut state) = state else {
        return;
    };

    let mut drag = Vec2::ZERO;
    if buttons.pressed(MouseButton::Right) {
        // Establish the baseline on the first frame of the press, so the orbit does
        // not jump from wherever the cursor happened to be last frame.
        if last_cursor.is_none() {
            *last_cursor = windows.single().ok().and_then(Window::cursor_position);
        }
        for moved in cursor.read() {
            if let Some(previous) = *last_cursor {
                drag += moved.position - previous;
            }
            *last_cursor = Some(moved.position);
        }
    } else {
        cursor.clear();
        *last_cursor = None;
    }

    let mut scroll = 0.0;
    for event in wheel.read() {
        scroll += match event.unit {
            MouseScrollUnit::Line => event.y,
            MouseScrollUnit::Pixel => event.y / 20.0,
        };
    }

    if drag.length_squared() <= 0.0 && scroll.abs() <= 0.0 {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let size = window.size().max(Vec2::ONE);
    state.yaw -= drag.x / size.x * std::f32::consts::TAU;
    state.pitch = (state.pitch + drag.y / size.y * std::f32::consts::PI)
        .clamp(-MAX_ORBIT_PITCH, MAX_ORBIT_PITCH);
    state.radius =
        (state.radius - scroll * state.radius * 0.1).clamp(MIN_ORBIT_RADIUS, MAX_ORBIT_RADIUS);

    let pose = orbit_transform(&state);
    if let Ok(mut transform) = cameras.get_mut(state.camera) {
        *transform = pose;
    }
}

fn handle_cancel(
    keys: Res<ButtonInput<KeyCode>>,
    bindings: Res<InputBindings>,
    mut next: ResMut<NextState<Screen>>,
) {
    if bindings.just_pressed(&keys, InputAction::Cancel) {
        next.set(Screen::Title);
    }
}

fn publish_view(
    animations: Option<Res<SpellAnimationFile>>,
    mut session: ResMut<VfxTunerSession>,
    mut view: ResMut<VfxTunerView>,
) {
    let Some(animations) = animations else {
        if view.ready {
            *view = VfxTunerView::default();
        }
        return;
    };

    let mut names: Vec<String> = animations.animations.keys().cloned().collect();
    names.sort();
    if session
        .spell
        .as_ref()
        .is_none_or(|spell| !animations.animations.contains_key(spell))
    {
        session.spell = names.first().cloned();
    }

    let selected = session.spell.clone();
    let spells = names
        .iter()
        .map(|name| VfxTunerSpellView {
            name: name.clone(),
            summary: animations
                .animations
                .get(name)
                .map_or_else(String::new, summarize),
            selected: selected.as_ref() == Some(name),
        })
        .collect();
    let rows = selected
        .as_ref()
        .and_then(|name| animations.animations.get(name))
        .map_or_else(Vec::new, build_rows);

    let next = VfxTunerView {
        ready: true,
        spells,
        rows,
        status: session.status.clone(),
        dirty: session
            .baseline
            .as_ref()
            .is_none_or(|baseline| baseline != &*animations),
    };
    if *view != next {
        *view = next;
    }
}

fn summarize(animation: &SpellAnimation) -> String {
    format!(
        "{} · {}",
        motion_name(animation.motion),
        style_name(animation.style)
    )
}

const fn motion_name(motion: MotionArchetype) -> &'static str {
    match motion {
        MotionArchetype::Projectile { .. } => "Projectile",
        MotionArchetype::Beam { .. } => "Beam",
        MotionArchetype::Arc { .. } => "Arc",
        MotionArchetype::InstantFlash { .. } => "Instant",
    }
}

const fn style_name(style: VfxStyle) -> &'static str {
    match style {
        VfxStyle::Spark => "Spark",
        VfxStyle::Flame => "Flame",
        VfxStyle::FireballConcentrated => "Fireball (dense)",
        VfxStyle::FireballWispy => "Fireball (wispy)",
    }
}

/// Formats a float for a value box.
///
/// Three decimals everywhere rather than per-field precision: at two, a `spread` of
/// `0.001` displayed as `0.00`, so the box showed zero for a non-zero value and
/// typing could not round-trip through it.
fn number(value: f32) -> String {
    format!("{value:.3}")
}

fn nudge_row(field: VfxTunerField, label: &str, value: String) -> VfxTunerRowView {
    VfxTunerRowView {
        field,
        label: label.to_owned(),
        value,
        control: VfxTunerControl::Nudge,
    }
}

fn cycle_row(field: VfxTunerField, label: &str, value: String) -> VfxTunerRowView {
    VfxTunerRowView {
        field,
        label: label.to_owned(),
        value,
        control: VfxTunerControl::Cycle,
    }
}

/// Rows are built per motion archetype, so a timing that archetype does not have is
/// simply absent rather than present and inert. An instant flash has no impact hold
/// and no trail; only a projectile has a trail.
fn build_rows(animation: &SpellAnimation) -> Vec<VfxTunerRowView> {
    let mut rows = vec![
        cycle_row(
            VfxTunerField::Motion,
            "Motion",
            motion_name(animation.motion).to_owned(),
        ),
        cycle_row(
            VfxTunerField::Style,
            "Style",
            style_name(animation.style).to_owned(),
        ),
    ];
    match animation.motion {
        MotionArchetype::InstantFlash { hold_seconds } => {
            rows.push(nudge_row(
                VfxTunerField::TimingPrimary,
                "Hold",
                number(hold_seconds),
            ));
        }
        MotionArchetype::Beam {
            flash_seconds,
            impact_hold_seconds,
            thickness,
        } => {
            rows.push(nudge_row(
                VfxTunerField::TimingPrimary,
                "Flash",
                number(flash_seconds),
            ));
            rows.push(nudge_row(
                VfxTunerField::TimingImpact,
                "Impact Hold",
                number(impact_hold_seconds),
            ));
            rows.push(nudge_row(
                VfxTunerField::BeamThickness,
                "Beam Thickness",
                number(thickness),
            ));
        }
        MotionArchetype::Arc {
            flash_seconds,
            impact_hold_seconds,
            thickness,
            displacement,
            subdivisions,
            branches,
        } => {
            rows.push(nudge_row(
                VfxTunerField::TimingPrimary,
                "Flash",
                number(flash_seconds),
            ));
            rows.push(nudge_row(
                VfxTunerField::TimingImpact,
                "Impact Hold",
                number(impact_hold_seconds),
            ));
            rows.push(nudge_row(
                VfxTunerField::BeamThickness,
                "Bolt Thickness",
                number(thickness),
            ));
            rows.push(nudge_row(
                VfxTunerField::ArcDisplacement,
                "Jaggedness",
                number(displacement),
            ));
            rows.push(nudge_row(
                VfxTunerField::ArcSubdivisions,
                "Subdivisions",
                subdivisions.to_string(),
            ));
            rows.push(nudge_row(
                VfxTunerField::ArcBranches,
                "Branches",
                branches.to_string(),
            ));
        }
        MotionArchetype::Projectile {
            travel_seconds,
            trail,
            impact_hold_seconds,
        } => {
            rows.push(nudge_row(
                VfxTunerField::TimingPrimary,
                "Travel",
                number(travel_seconds),
            ));
            rows.push(nudge_row(
                VfxTunerField::TimingImpact,
                "Impact Hold",
                number(impact_hold_seconds),
            ));
            rows.push(cycle_row(
                VfxTunerField::Trail,
                "Trail",
                if trail { "On" } else { "Off" }.to_owned(),
            ));
        }
    }
    rows.push(nudge_row(
        VfxTunerField::ParticleCount,
        "Particles",
        animation.particle_count.to_string(),
    ));
    rows.push(nudge_row(
        VfxTunerField::ParticleSpeed,
        "Speed",
        number(animation.particle_speed),
    ));
    rows.push(nudge_row(
        VfxTunerField::ParticleLifetime,
        "Lifetime",
        number(animation.particle_lifetime_seconds),
    ));
    rows.push(nudge_row(
        VfxTunerField::Scale,
        "Scale",
        number(animation.scale),
    ));
    rows.push(nudge_row(
        VfxTunerField::Spread,
        "Spread",
        number(animation.spread),
    ));
    rows.push(cycle_row(
        VfxTunerField::ColorOverride,
        "Color Override",
        if animation.color_override.is_some() {
            "On"
        } else {
            "Element Tint"
        }
        .to_owned(),
    ));
    if let Some([red, green, blue, _]) = animation.color_override {
        rows.push(nudge_row(VfxTunerField::ColorRed, "Red", number(red)));
        rows.push(nudge_row(VfxTunerField::ColorGreen, "Green", number(green)));
        rows.push(nudge_row(VfxTunerField::ColorBlue, "Blue", number(blue)));
    }
    rows
}

#[expect(
    clippy::too_many_arguments,
    reason = "one handler owns the tuner's complete intent surface: editing needs \
              the animation content, replaying needs the spell/element catalogs, \
              both dummy positions, and every VFX asset cache independently"
)]
fn handle_intents(
    mut intents: MessageReader<UiIntent>,
    mut commands: Commands,
    mut session: ResMut<VfxTunerSession>,
    mut animations: Option<ResMut<SpellAnimationFile>>,
    spells: Option<Res<SpellBook>>,
    elements: Option<Res<ElementCatalog>>,
    casters: Query<&Transform, With<PreviewCaster>>,
    targets: Query<&Transform, With<PreviewTarget>>,
    live: Query<Entity, With<SpellVfxLifetime>>,
    mut next: ResMut<NextState<Screen>>,
    mut cache: ResMut<SpellVfxAssetCache>,
    mut effects: ResMut<Assets<bevy_hanabi::EffectAsset>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    asset_server: Res<AssetServer>,
) {
    let mut play_requested = false;
    let mut back_requested = false;

    for intent in intents.read() {
        match intent {
            UiIntent::Back => back_requested = true,
            UiIntent::VfxTuner(intent) => match intent {
                VfxTunerIntent::Select(name) => {
                    session.spell = Some(name.clone());
                    session.status = None;
                }
                VfxTunerIntent::Play => play_requested = true,
                VfxTunerIntent::Decrement(field) | VfxTunerIntent::Increment(field) => {
                    let up = matches!(intent, VfxTunerIntent::Increment(_));
                    apply_edit(animations.as_mut(), session.spell.as_deref(), |animation| {
                        nudge(animation, *field, up);
                    });
                    session.status = None;
                }
                VfxTunerIntent::Cycle(field) => {
                    apply_edit(animations.as_mut(), session.spell.as_deref(), |animation| {
                        cycle(animation, *field);
                    });
                    session.status = None;
                }
                VfxTunerIntent::Revert => {
                    if let (Some(animations), Some(baseline)) =
                        (animations.as_mut(), session.baseline.clone())
                    {
                        **animations = baseline;
                    }
                    session.status = Some("reverted to the values on disk".to_owned());
                }
                VfxTunerIntent::Save => {
                    let status = animations.as_ref().map(|animations| {
                        save_spell_animations(animations).map(|path| (path, (*animations).clone()))
                    });
                    session.status = Some(match status {
                        Some(Ok((path, saved))) => {
                            session.baseline = Some(saved);
                            format!("saved to {}", path.display())
                        }
                        Some(Err(error)) => format!("save failed: {error}"),
                        None => "nothing to save".to_owned(),
                    });
                }
            },
            _ => {}
        }
    }

    if back_requested {
        next.set(Screen::Title);
        return;
    }
    if !play_requested {
        return;
    }

    let (Some(animations), Some(spells), Some(elements)) = (animations, spells, elements) else {
        return;
    };
    let (Some(spell_name), Ok(caster), Ok(target)) =
        (session.spell.clone(), casters.single(), targets.single())
    else {
        return;
    };
    let Some(animation) = animations.animations.get(&spell_name) else {
        return;
    };
    let Some(spell_def) = spells.id(&spell_name).and_then(|id| spells.spell(id)) else {
        return;
    };

    // A replay starts clean: without this, hammering Play stacks emitters and the
    // effect being judged is several casts overlaid rather than the one just tuned.
    for effect in &live {
        commands.entity(effect).despawn();
    }

    // Rebuilt from the values on screen right now, every time. Without this the
    // spawn races `clear_stale_effect_cache` — the two systems have no ordering
    // between them, so a replay could pick up either the current tuning or the
    // cached build from before the last edit, and visibly alternate between them.
    cache.clear_authored_content();
    let color = resolve_cast_color(spell_def, animation, &elements);
    spawn_cast_vfx(
        &mut commands,
        &spell_name,
        animation,
        color,
        caster.translation,
        target.translation,
        &mut cache,
        &mut effects,
        &mut meshes,
        &mut materials,
        &mut images,
        &asset_server,
    );
}

/// Applies `edit` to the selected spell, touching the resource **only** if the edit
/// actually changed something.
///
/// The guard matters more than it looks. Reaching for `ResMut` marks
/// `SpellAnimationFile` as changed whether or not a value moved, and
/// `clear_stale_effect_cache` dumps every built particle effect whenever it is
/// marked. Since a text box republishes its contents continuously, the unguarded
/// version dropped and rebuilt the whole cache every frame.
fn apply_edit(
    animations: Option<&mut ResMut<SpellAnimationFile>>,
    spell: Option<&str>,
    edit: impl FnOnce(&mut SpellAnimation),
) {
    let (Some(animations), Some(spell)) = (animations, spell) else {
        return;
    };
    // Read through the immutable deref so an unchanged edit never marks the resource.
    let Some(current) = animations.animations.get(spell).copied() else {
        return;
    };
    let mut updated = current;
    edit(&mut updated);
    if updated == current {
        return;
    }
    if let Some(slot) = animations.animations.get_mut(spell) {
        *slot = updated;
    }
}

/// Reads the field's current value, for a stepper that moves relative to it.
fn read(animation: &SpellAnimation, field: VfxTunerField) -> Option<f32> {
    Some(match field {
        VfxTunerField::TimingPrimary => match animation.motion {
            MotionArchetype::InstantFlash { hold_seconds } => hold_seconds,
            MotionArchetype::Beam { flash_seconds, .. }
            | MotionArchetype::Arc { flash_seconds, .. } => flash_seconds,
            MotionArchetype::Projectile { travel_seconds, .. } => travel_seconds,
        },
        VfxTunerField::TimingImpact => match animation.motion {
            MotionArchetype::Beam {
                impact_hold_seconds,
                ..
            }
            | MotionArchetype::Arc {
                impact_hold_seconds,
                ..
            }
            | MotionArchetype::Projectile {
                impact_hold_seconds,
                ..
            } => impact_hold_seconds,
            MotionArchetype::InstantFlash { .. } => return None,
        },
        VfxTunerField::BeamThickness => match animation.motion {
            MotionArchetype::Beam { thickness, .. } | MotionArchetype::Arc { thickness, .. } => {
                thickness
            }
            _ => return None,
        },
        VfxTunerField::ArcDisplacement => match animation.motion {
            MotionArchetype::Arc { displacement, .. } => displacement,
            _ => return None,
        },
        VfxTunerField::ArcSubdivisions => match animation.motion {
            MotionArchetype::Arc { subdivisions, .. } => f32_from_count(subdivisions),
            _ => return None,
        },
        VfxTunerField::ArcBranches => match animation.motion {
            MotionArchetype::Arc { branches, .. } => f32_from_count(branches),
            _ => return None,
        },
        VfxTunerField::ParticleCount => f32_from_count(animation.particle_count),
        VfxTunerField::ParticleSpeed => animation.particle_speed,
        VfxTunerField::ParticleLifetime => animation.particle_lifetime_seconds,
        VfxTunerField::Scale => animation.scale,
        VfxTunerField::Spread => animation.spread,
        VfxTunerField::ColorRed | VfxTunerField::ColorGreen | VfxTunerField::ColorBlue => {
            *animation
                .color_override
                .as_ref()?
                .get(color_index(field)?)?
        }
        VfxTunerField::Motion
        | VfxTunerField::Style
        | VfxTunerField::Trail
        | VfxTunerField::ColorOverride => return None,
    })
}

const fn color_index(field: VfxTunerField) -> Option<usize> {
    match field {
        VfxTunerField::ColorRed => Some(0),
        VfxTunerField::ColorGreen => Some(1),
        VfxTunerField::ColorBlue => Some(2),
        _ => None,
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "particle_count is capped well inside f32's exact integer range"
)]
fn f32_from_count(count: u32) -> f32 {
    count as f32
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped into [1.0, MAX_PARTICLE_COUNT] immediately before \
              the cast, so it always lands in u32 range"
)]
fn count_from_f32(value: f32) -> u32 {
    value.clamp(1.0, f32_from_count(MAX_PARTICLE_COUNT)).round() as u32
}

/// A whole-number field that is allowed to be zero, unlike a particle count.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the value is clamped into [0.0, maximum] immediately before the cast"
)]
fn clamped_count(value: f32, maximum: u32) -> u32 {
    value.clamp(0.0, f32_from_count(maximum)).round() as u32
}

/// Writes `value` into `field`, floored only where a non-positive number would not
/// render at all.
///
/// There is deliberately no upper limit on any of these: the tuner exists to find
/// out what a value looks like, and an earlier revision's ceilings were reached
/// while tuning real effects rather than catching real mistakes.
fn set_value(animation: &mut SpellAnimation, field: VfxTunerField, value: f32) {
    if !value.is_finite() {
        return;
    }
    let duration = value.max(MIN_DURATION_SECONDS);
    let size = value.max(MIN_SIZE);
    match field {
        VfxTunerField::TimingPrimary => match &mut animation.motion {
            MotionArchetype::InstantFlash { hold_seconds } => *hold_seconds = duration,
            MotionArchetype::Beam { flash_seconds, .. }
            | MotionArchetype::Arc { flash_seconds, .. } => *flash_seconds = duration,
            MotionArchetype::Projectile { travel_seconds, .. } => *travel_seconds = duration,
        },
        VfxTunerField::TimingImpact => match &mut animation.motion {
            MotionArchetype::Beam {
                impact_hold_seconds,
                ..
            }
            | MotionArchetype::Arc {
                impact_hold_seconds,
                ..
            }
            | MotionArchetype::Projectile {
                impact_hold_seconds,
                ..
            } => *impact_hold_seconds = duration,
            MotionArchetype::InstantFlash { .. } => {}
        },
        VfxTunerField::BeamThickness => match &mut animation.motion {
            MotionArchetype::Beam { thickness, .. } | MotionArchetype::Arc { thickness, .. } => {
                *thickness = size;
            }
            MotionArchetype::InstantFlash { .. } | MotionArchetype::Projectile { .. } => {}
        },
        VfxTunerField::ArcDisplacement => {
            if let MotionArchetype::Arc { displacement, .. } = &mut animation.motion {
                *displacement = size;
            }
        }
        VfxTunerField::ArcSubdivisions => {
            if let MotionArchetype::Arc { subdivisions, .. } = &mut animation.motion {
                *subdivisions = clamped_count(value, MAX_ARC_SUBDIVISIONS);
            }
        }
        VfxTunerField::ArcBranches => {
            if let MotionArchetype::Arc { branches, .. } = &mut animation.motion {
                *branches = clamped_count(value, MAX_ARC_BRANCHES);
            }
        }
        VfxTunerField::ParticleCount => animation.particle_count = count_from_f32(value),
        VfxTunerField::ParticleSpeed => animation.particle_speed = value.max(0.0),
        VfxTunerField::ParticleLifetime => animation.particle_lifetime_seconds = duration,
        VfxTunerField::Scale => animation.scale = size,
        VfxTunerField::Spread => animation.spread = size,
        VfxTunerField::ColorRed | VfxTunerField::ColorGreen | VfxTunerField::ColorBlue => {
            if let (Some(color), Some(index)) =
                (animation.color_override.as_mut(), color_index(field))
            {
                if let Some(channel) = color.get_mut(index) {
                    *channel = value.clamp(0.0, 1.0);
                }
            }
        }
        VfxTunerField::Motion
        | VfxTunerField::Style
        | VfxTunerField::Trail
        | VfxTunerField::ColorOverride => {}
    }
}

/// One stepper press, in units a designer reaches for on that particular field.
const fn step_of(field: VfxTunerField) -> f32 {
    match field {
        VfxTunerField::ParticleCount => 5.0,
        VfxTunerField::ParticleSpeed => 0.25,
        VfxTunerField::Scale | VfxTunerField::BeamThickness => 0.005,
        VfxTunerField::ArcDisplacement => 0.05,
        VfxTunerField::ArcSubdivisions | VfxTunerField::ArcBranches => 1.0,
        VfxTunerField::Spread => 0.02,
        VfxTunerField::ColorRed | VfxTunerField::ColorGreen | VfxTunerField::ColorBlue => 0.05,
        _ => 0.05,
    }
}

fn nudge(animation: &mut SpellAnimation, field: VfxTunerField, up: bool) {
    let Some(current) = read(animation, field) else {
        return;
    };
    let step = if up { step_of(field) } else { -step_of(field) };
    set_value(animation, field, current + step);
}

/// Cycling motion keeps the timings the new archetype shares with the old one, so
/// switching Beam to Projectile to compare them does not silently reset the impact
/// hold a designer just tuned.
fn cycle(animation: &mut SpellAnimation, field: VfxTunerField) {
    match field {
        VfxTunerField::Motion => {
            animation.motion = match animation.motion {
                MotionArchetype::InstantFlash { hold_seconds } => MotionArchetype::Beam {
                    flash_seconds: hold_seconds,
                    impact_hold_seconds: hold_seconds,
                    // The one value a flash cannot carry over: it has no line.
                    // Seeded from the particle size, which is the closest thing on
                    // screen, and immediately tunable on its own row.
                    thickness: animation.scale,
                },
                // Beam and Arc are the same shape with and without the crackle, so
                // cycling between them keeps every value they share and only seeds
                // what the jagged path additionally needs.
                MotionArchetype::Beam {
                    flash_seconds,
                    impact_hold_seconds,
                    thickness,
                } => MotionArchetype::Arc {
                    flash_seconds,
                    impact_hold_seconds,
                    thickness,
                    displacement: 0.35,
                    subdivisions: 5,
                    branches: 3,
                },
                MotionArchetype::Arc {
                    flash_seconds,
                    impact_hold_seconds,
                    ..
                } => MotionArchetype::Projectile {
                    travel_seconds: flash_seconds,
                    trail: true,
                    impact_hold_seconds,
                },
                MotionArchetype::Projectile {
                    impact_hold_seconds,
                    ..
                } => MotionArchetype::InstantFlash {
                    hold_seconds: impact_hold_seconds,
                },
            };
        }
        VfxTunerField::Style => {
            animation.style = match animation.style {
                VfxStyle::Spark => VfxStyle::Flame,
                VfxStyle::Flame => VfxStyle::FireballConcentrated,
                VfxStyle::FireballConcentrated => VfxStyle::FireballWispy,
                VfxStyle::FireballWispy => VfxStyle::Spark,
            };
        }
        VfxTunerField::Trail => {
            if let MotionArchetype::Projectile { trail, .. } = &mut animation.motion {
                *trail = !*trail;
            }
        }
        VfxTunerField::ColorOverride => {
            animation.color_override = match animation.color_override {
                Some(_) => None,
                // Opens on white rather than the element tint: this crate would have
                // to resolve the tint to seed it, and starting from white makes the
                // three channel rows immediately meaningful in both directions.
                None => Some([1.0, 1.0, 1.0, 1.0]),
            };
        }
        VfxTunerField::TimingPrimary
        | VfxTunerField::TimingImpact
        | VfxTunerField::BeamThickness
        | VfxTunerField::ArcDisplacement
        | VfxTunerField::ArcSubdivisions
        | VfxTunerField::ArcBranches
        | VfxTunerField::ParticleCount
        | VfxTunerField::ParticleSpeed
        | VfxTunerField::ParticleLifetime
        | VfxTunerField::Scale
        | VfxTunerField::Spread
        | VfxTunerField::ColorRed
        | VfxTunerField::ColorGreen
        | VfxTunerField::ColorBlue => {}
    }
}

/// Serializes `animations` to pretty RON and atomically overwrites the real shipped
/// `assets/config/spell_animations.ron`, resolved the same way Bevy resolves its
/// asset root (`BEVY_ASSET_ROOT`, set for every `cargo` invocation by
/// `.cargo/config.toml`) — so a save lands exactly where hot reload and the shipped
/// build both expect it.
fn save_spell_animations(animations: &SpellAnimationFile) -> Result<PathBuf, String> {
    let text = ron::ser::to_string_pretty(animations, PrettyConfig::new().struct_names(false))
        .map_err(|error| error.to_string())?;
    let root = std::env::var("BEVY_ASSET_ROOT").unwrap_or_else(|_| ".".to_owned());
    let path = PathBuf::from(root).join("assets/config/spell_animations.ron");
    AtomicFile::new(&path, AllowOverwrite)
        .write(|file| file.write_all(text.as_bytes()))
        .map_err(|error| error.to_string())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation() -> SpellAnimation {
        SpellAnimation {
            motion: MotionArchetype::InstantFlash { hold_seconds: 0.4 },
            style: VfxStyle::Flame,
            color_override: None,
            particle_count: 40,
            particle_speed: 1.4,
            particle_lifetime_seconds: 0.5,
            scale: 0.16,
            spread: 0.22,
        }
    }

    /// Nudging is unbounded upward now, but must never step *down* into a value
    /// `hex_assets` rejects — whatever a designer lands on, Save has to produce a
    /// file that loads again.
    #[test]
    fn nudging_a_field_to_either_extreme_stays_valid() {
        for field in [
            VfxTunerField::TimingPrimary,
            VfxTunerField::ParticleCount,
            VfxTunerField::ParticleSpeed,
            VfxTunerField::ParticleLifetime,
            VfxTunerField::Scale,
            VfxTunerField::Spread,
        ] {
            for up in [true, false] {
                let mut tuned = animation();
                for _ in 0..2000 {
                    nudge(&mut tuned, field, up);
                }
                let file = SpellAnimationFile {
                    animations: [("Ember".to_owned(), tuned)].into_iter().collect(),
                };
                let text =
                    ron::ser::to_string_pretty(&file, PrettyConfig::new().struct_names(false))
                        .expect("a tuned animation should serialize");
                ron::from_str::<SpellAnimationFile>(&text).unwrap_or_else(|error| {
                    panic!("{field:?} nudged {up} to its limit must stay loadable: {error}")
                });
            }
        }
    }

    /// The ceilings were removed on purpose: an effect that fills the screen is a
    /// legitimate thing to want to look at.
    #[test]
    fn a_field_can_be_driven_far_past_the_old_ceilings() {
        let mut tuned = animation();
        set_value(&mut tuned, VfxTunerField::Scale, 12.0);
        set_value(&mut tuned, VfxTunerField::Spread, 40.0);
        set_value(&mut tuned, VfxTunerField::TimingPrimary, 90.0);
        assert!((tuned.scale - 12.0).abs() < f32::EPSILON);
        assert!((tuned.spread - 40.0).abs() < f32::EPSILON);

        let file = SpellAnimationFile {
            animations: [("Ember".to_owned(), tuned)].into_iter().collect(),
        };
        let text = ron::ser::to_string_pretty(&file, PrettyConfig::new().struct_names(false))
            .expect("a tuned animation should serialize");
        ron::from_str::<SpellAnimationFile>(&text)
            .expect("a deliberately huge effect must still load");
    }

    /// A beam's line and the sparks it throws are different objects; tying both to
    /// `scale` meant thickening the bolt also inflated every particle.
    #[test]
    fn beam_thickness_is_independent_of_particle_scale() {
        let mut tuned = animation();
        tuned.motion = MotionArchetype::Beam {
            flash_seconds: 0.2,
            impact_hold_seconds: 0.2,
            thickness: 0.1,
        };
        set_value(&mut tuned, VfxTunerField::BeamThickness, 0.9);

        assert!(
            (tuned.scale - 0.16).abs() < f32::EPSILON,
            "scale must not move"
        );
        assert_eq!(
            tuned.motion,
            MotionArchetype::Beam {
                flash_seconds: 0.2,
                impact_hold_seconds: 0.2,
                thickness: 0.9,
            }
        );
    }

    /// An entry that is not yet a number must leave the value alone. (`"0."` is
    /// deliberately absent: Rust parses it as `0.0`, which is why the renderer
    /// additionally refuses to write into whichever box currently has focus.)
    #[test]
    fn a_half_typed_entry_leaves_the_value_untouched() {
        let mut tuned = animation();
        let before = tuned.scale;
        for partial in ["", "-", ".", "1e", "abc"] {
            if let Ok(value) = partial.trim().parse::<f32>() {
                set_value(&mut tuned, VfxTunerField::Scale, value);
            }
        }
        assert!((tuned.scale - before).abs() < f32::EPSILON);
    }

    /// Every nudge row's displayed value has to parse back as a number, or typing
    /// into the box it seeds would immediately be rejected.
    #[test]
    fn every_editable_row_shows_a_parseable_number() {
        let mut tuned = animation();
        cycle(&mut tuned, VfxTunerField::ColorOverride);
        for motion in [
            MotionArchetype::InstantFlash { hold_seconds: 0.4 },
            MotionArchetype::Beam {
                flash_seconds: 0.2,
                impact_hold_seconds: 0.2,
                thickness: 0.1,
            },
            MotionArchetype::Projectile {
                travel_seconds: 0.2,
                trail: true,
                impact_hold_seconds: 0.2,
            },
        ] {
            tuned.motion = motion;
            for row in build_rows(&tuned) {
                if row.control == VfxTunerControl::Nudge {
                    row.value.trim().parse::<f32>().unwrap_or_else(|error| {
                        panic!(
                            "{:?} shows {:?}, which is not a number: {error}",
                            row.field, row.value
                        )
                    });
                }
            }
        }
    }

    /// Cycling has to be a closed loop: a designer comparing archetypes must be
    /// able to get back to the one they started on without reloading.
    #[test]
    fn cycling_motion_visits_every_archetype_and_returns() {
        let mut tuned = animation();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(motion_name(tuned.motion));
            cycle(&mut tuned, VfxTunerField::Motion);
        }
        seen.sort_unstable();
        assert_eq!(seen, ["Arc", "Beam", "Instant", "Projectile"]);
        assert!(matches!(tuned.motion, MotionArchetype::InstantFlash { .. }));
    }

    /// Same for styles, which now include the two authored flipbook sheets.
    #[test]
    fn cycling_style_visits_every_look_and_returns() {
        let mut tuned = animation();
        let mut seen = Vec::new();
        for _ in 0..4 {
            seen.push(style_name(tuned.style));
            cycle(&mut tuned, VfxTunerField::Style);
        }
        seen.sort_unstable();
        assert_eq!(
            seen,
            ["Fireball (dense)", "Fireball (wispy)", "Flame", "Spark"]
        );
        assert_eq!(tuned.style, VfxStyle::Flame);
    }

    /// The flipbook styles are the only ones naming a file; if a path stops matching
    /// what is on disk, the sheet silently fails to load at cast time.
    #[test]
    fn every_flipbook_style_points_at_a_sheet_that_exists() {
        for style in [VfxStyle::FireballConcentrated, VfxStyle::FireballWispy] {
            let sheet = style
                .sprite_sheet()
                .unwrap_or_else(|| panic!("{style:?} is a flipbook style and must name a sheet"));
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../assets")
                .join(sheet.path);
            assert!(
                path.is_file(),
                "missing flipbook sheet at {}",
                path.display()
            );
            assert!(sheet.frames() > 1, "a flipbook needs more than one frame");
        }
        for style in [VfxStyle::Spark, VfxStyle::Flame] {
            assert!(
                style.sprite_sheet().is_none(),
                "{style:?} is generated in code and must not name a sheet"
            );
        }
    }

    #[test]
    fn color_channel_rows_appear_only_once_an_override_exists() {
        let mut tuned = animation();
        assert!(!build_rows(&tuned)
            .iter()
            .any(|row| row.field == VfxTunerField::ColorRed));
        cycle(&mut tuned, VfxTunerField::ColorOverride);
        assert!(build_rows(&tuned)
            .iter()
            .any(|row| row.field == VfxTunerField::ColorRed));
    }

    /// An instant flash has no impact burst to hold, so nudging that timing must be
    /// inert rather than quietly converting the archetype.
    #[test]
    fn impact_hold_is_inert_for_an_instant_flash() {
        let mut tuned = animation();
        nudge(&mut tuned, VfxTunerField::TimingImpact, true);
        assert_eq!(
            tuned.motion,
            MotionArchetype::InstantFlash { hold_seconds: 0.4 }
        );
    }
}
