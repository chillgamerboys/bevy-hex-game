//! Small event cues consume already-visible gameplay snapshots.
use super::super::{ArenaCamera, ViewState, hud};
use bevy::prelude::*;
use hex_arena::{ArenaSession, ArenaTuning};

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PresentCombatCues;

#[derive(Component)]
pub(in crate::arena) struct EnemyDots(pub usize);
#[derive(Component)]
pub(in crate::arena) struct PlayerLevel;
#[derive(Component)]
pub(in crate::arena) struct GliderStatus;
#[derive(Resource, Default)]
struct LevelPulse {
    previous: Option<u32>,
    remaining: f32,
}

pub(super) fn install(app: &mut App) {
    app.init_resource::<LevelPulse>().add_systems(
        Update,
        (enemy_dots, player_level, glider_status)
            .after(hud::update)
            .in_set(PresentCombatCues)
            .in_set(super::super::ArenaFrame::Present),
    );
}

fn enemy_dots(
    session: Res<ArenaSession>,
    tuning: Res<ArenaTuning>,
    state: Res<ViewState>,
    camera: Query<(&Camera, &Transform), With<ArenaCamera>>,
    mut dots: Query<(&EnemyDots, &mut Node, &mut Text, &mut TextColor)>,
) {
    let feedback = session.combat_feedback(&tuning);
    let camera = camera.single().ok();
    for (slot, mut node, mut text, mut color) in &mut dots {
        let point = if state.started && !state.paused {
            feedback.health_cues.get(slot.0).and_then(|cue| {
                let (camera, transform) = camera?;
                let rect = camera.logical_viewport_rect()?;
                let point = camera
                    .world_to_viewport(&GlobalTransform::from(*transform), cue.position)
                    .ok()?;
                rect.contains(point)
                    .then_some((cue.health_pips, (point - rect.min) / rect.size()))
            })
        } else {
            None
        };
        let Some((pips, position)) = point else {
            super::set_display(&mut node, Display::None);
            continue;
        };
        super::set_display(&mut node, Display::Flex);
        node.left = percent(position.x * 100.0);
        node.top = percent(position.y * 100.0);
        let (glyph, tint) = match pips {
            3 => ("● ● ●", Color::WHITE),
            2 => ("● ●", Color::srgb(1.0, 0.64, 0.13)),
            _ => ("●", Color::srgb(1.0, 0.20, 0.16)),
        };
        if text.0 != glyph {
            text.0 = glyph.into();
        }
        color.set_if_neq(TextColor(tint));
    }
}

fn player_level(
    session: Res<ArenaSession>,
    time: Res<Time>,
    state: Res<ViewState>,
    mut pulse: ResMut<LevelPulse>,
    mut labels: Query<(&mut Text, &mut TextColor, &mut TextFont, &mut Node), With<PlayerLevel>>,
) {
    let progress = session.progress();
    let level = progress.map(|p| p.level);
    if pulse
        .previous
        .is_some_and(|previous| level.is_some_and(|next| next > previous))
    {
        pulse.remaining = 1.2;
    } else if level < pulse.previous {
        pulse.remaining = 0.0;
    }
    pulse.previous = level;
    if !state.paused {
        pulse.remaining = (pulse.remaining - time.delta_secs()).max(0.0);
    }
    for (mut text, mut color, mut font, mut node) in &mut labels {
        let Some(progress) = progress else {
            super::set_display(&mut node, Display::None);
            continue;
        };
        super::set_display(&mut node, Display::Flex);
        let next = format!("Lv {} · +{}", progress.level, progress.available_upgrades);
        if text.0 != next {
            text.0 = next;
        }
        color.set_if_neq(TextColor(if progress.available_upgrades > 0 {
            Color::srgb(1.0, 0.79, 0.26)
        } else {
            Color::WHITE
        }));
        let wave = (pulse.remaining * std::f32::consts::TAU * 2.0).sin().abs();
        let size = FontSize::Px(
            24.0 + if pulse.remaining > 0.0 {
                3.0 * wave
            } else {
                0.0
            },
        );
        if font.font_size != size {
            font.font_size = size;
        }
    }
}

fn glider_status(
    session: Res<ArenaSession>,
    mut labels: Query<(&mut Text, &mut TextColor, &mut Node), With<GliderStatus>>,
) {
    let flight = session
        .human_actor_id()
        .and_then(|id| session.actors.iter().find(|actor| actor.id == id))
        .and_then(|actor| {
            actor
                .glider()
                .filter(|flight| flight.open && !actor.grounded)
        });
    for (mut text, mut color, mut node) in &mut labels {
        let Some(flight) = flight else {
            super::set_display(&mut node, Display::None);
            continue;
        };
        super::set_display(&mut node, Display::Flex);
        let next = format!(
            "GLIDE {:.0} u/s{}",
            flight.airspeed,
            if flight.stall_fraction > 0.0 {
                " · LOW LIFT"
            } else {
                ""
            }
        );
        if text.0 != next {
            text.0 = next;
        }
        color.set_if_neq(TextColor(if flight.stall_fraction > 0.0 {
            Color::srgb(1.0, 0.65, 0.16)
        } else {
            Color::srgb(0.68, 0.88, 1.0)
        }));
    }
}
