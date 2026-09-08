//! Observer camera and public battle-setup presentation; never creature authority.
//! Input receives filtered CursorMoved displacement, never MouseMotion or ActorIntent.

use super::{ArenaCamera, ViewState};
use bevy::prelude::*;
use hex_arena::{
    ArenaBattleSetup, ArenaControl, ArenaSession, BattlePreset, BattleResult, BattleSummary,
    TeamRoster,
};
use hex_core::arena::{ArenaMap, ArenaReset, ArenaTerrainView, ArenaVoxelGeometry};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) enum ObserverCameraMode {
    #[default]
    Orbit,
    Free,
}

#[derive(Resource, Debug)]
pub(super) struct ObserverCamera {
    pub mode: ObserverCameraMode,
    pub target: Vec3,
    pub position: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub generation: Option<u64>,
}

impl Default for ObserverCamera {
    fn default() -> Self {
        Self {
            mode: ObserverCameraMode::Orbit,
            target: Vec3::ZERO,
            position: Vec3::new(20.0, 20.0, 20.0),
            yaw: 0.0,
            pitch: -0.5,
            distance: 30.0,
            generation: None,
        }
    }
}

impl ObserverCamera {
    /// Call once after a matching world/gameplay reset publishes actual actors.
    /// `pose` is the existing map overview; target is the admitted roster center.
    pub fn reset(&mut self, generation: u64, pose: Transform, target: Vec3) {
        let direction = (target - pose.translation).normalize_or(Vec3::NEG_Z);
        self.mode = ObserverCameraMode::Orbit;
        self.target = target;
        self.position = pose.translation;
        self.yaw = (-direction.x).atan2(-direction.z);
        self.pitch = direction.y.clamp(-1.0, 1.0).asin();
        self.distance = pose.translation.distance(target).clamp(3.0, 120.0);
        self.generation = Some(generation);
    }

    pub fn direction(&self) -> Vec3 {
        Quat::from_euler(EulerRot::YXZ, self.yaw, self.pitch, 0.0) * Vec3::NEG_Z
    }

    pub fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            ObserverCameraMode::Orbit => ObserverCameraMode::Free,
            ObserverCameraMode::Free => {
                // Preserve pose when switching; the next orbit center is along
                // this camera's own look direction, independent of creature AI.
                self.target = self.position + self.direction() * self.distance;
                ObserverCameraMode::Orbit
            }
        };
    }

    /// Camera-only movement. Integration applies the existing public terrain
    /// camera sweep from previous to desired position before accepting the pose.
    pub fn update_pose(
        &mut self,
        look: Vec2,
        movement: Vec3,
        fast: bool,
        wheel: f32,
        dt: f32,
    ) -> Transform {
        self.yaw -= look.x * 0.0025;
        self.pitch = (self.pitch - look.y * 0.0025).clamp(-1.48, 1.48);
        let direction = self.direction();
        let horizontal = direction.with_y(0.0).normalize_or(Vec3::NEG_Z);
        let speed = if fast { 18.0 } else { 7.0 };
        let travel = (horizontal.cross(Vec3::Y) * movement.x
            + Vec3::Y * movement.y
            + horizontal * movement.z)
            .clamp_length_max(1.0)
            * speed
            * dt.clamp(0.0, 0.1);
        match self.mode {
            ObserverCameraMode::Orbit => {
                self.target += travel;
                self.distance = (self.distance * (-wheel * 0.12).exp()).clamp(3.0, 120.0);
                self.position = self.target - direction * self.distance;
            }
            ObserverCameraMode::Free => self.position += travel,
        }
        Transform::from_translation(self.position).looking_to(direction, Vec3::Y)
    }
}

pub(super) fn preset_for(setup: &ArenaBattleSetup, slot: usize) -> Option<BattlePreset> {
    let roster = setup.rosters.get(slot)?;
    if roster.parties.len() != 1 {
        return None;
    }
    BattlePreset::ALL.into_iter().find(|preset| {
        roster
            .parties
            .first()
            .is_some_and(|members| *members == preset.members())
    })
}

/// The setup is requested state; caller increments ArenaReset only on a change.
pub(super) fn choose_preset(
    setup: &mut ArenaBattleSetup,
    slot: usize,
    preset: BattlePreset,
) -> bool {
    let Some(roster) = setup.rosters.get_mut(slot) else {
        return false;
    };
    let replacement = TeamRoster::from_preset(roster.team, preset);
    if *roster == replacement {
        return false;
    }
    *roster = replacement;
    true
}

pub(super) fn launch_setup(
    map: ArenaMap,
    spectator: bool,
    left: Option<&str>,
    right: Option<&str>,
    seed: Option<&str>,
    tick_limit: Option<&str>,
) -> Result<ArenaBattleSetup, String> {
    if !spectator {
        if left.is_some() || right.is_some() || seed.is_some() || tick_limit.is_some() {
            return Err(
                "Team, replay seed and battle limit options require spectator mode.".into(),
            );
        }
        return Ok(ArenaBattleSetup::default());
    }
    let parse_preset = |value: Option<&str>, fallback| match value {
        None => Ok(fallback),
        Some(value) => {
            BattlePreset::from_slug(value).ok_or_else(|| format!("Unknown team preset: {value}"))
        }
    };
    let mut setup = ArenaBattleSetup::spectator(
        parse_preset(left, BattlePreset::Shadow)?,
        parse_preset(right, BattlePreset::Dragon)?,
        seed.unwrap_or("1")
            .parse::<u64>()
            .map_err(|error| format!("Battle seed must be an unsigned 64-bit integer: {error}"))?,
    );
    if let Some(limit) = tick_limit {
        setup.tick_limit =
            Some(limit.parse::<u64>().map_err(|error| {
                format!("Battle tick limit must be a positive integer: {error}")
            })?);
    }
    setup.validate_for(map).map_err(|error| error.to_string())?;
    Ok(setup)
}

pub(super) fn team_status(summary: &BattleSummary) -> String {
    summary
        .teams
        .iter()
        .enumerate()
        .map(|(slot, team)| {
            format!(
                "TEAM {} / {}   {} / {} alive   {:.0} / {:.0} HP",
                team.team,
                if slot == 0 { "CYAN" } else { "AMBER" },
                team.living,
                team.initial,
                team.hp,
                team.max_hp
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn battle_status(
    summary: &BattleSummary,
    mode: ObserverCameraMode,
    paused: bool,
) -> String {
    let status = match &summary.result {
        Some(BattleResult::TeamWinner(team)) => format!("TEAM {team} WINS"),
        Some(BattleResult::Draw) => "DRAW / BOTH TEAMS ELIMINATED".to_owned(),
        Some(BattleResult::Timeout) => "TIME LIMIT / NO WINNER".to_owned(),
        Some(BattleResult::InvalidSetup(message)) => format!("BATTLE COULD NOT START\n{message}"),
        None if paused => "PAUSED".to_owned(),
        None => "LIVE".to_owned(),
    };
    let camera = match mode {
        ObserverCameraMode::Orbit => "ORBIT",
        ObserverCameraMode::Free => "FREE CAMERA",
    };
    format!(
        "{camera} / {status}\n{:.1}s / Seed {}",
        summary.seconds, summary.seed
    )
}

pub(super) fn active(session: &ArenaSession) -> bool {
    session.accepted_battle_setup().control == ArenaControl::Spectator
}

pub(super) fn team_color(session: &ArenaSession, team: u8) -> Color {
    let slot = session
        .accepted_battle_setup()
        .rosters
        .iter()
        .position(|roster| roster.team == team);
    if slot == Some(0) {
        Color::srgb(0.24, 0.82, 1.0)
    } else {
        Color::srgb(1.0, 0.62, 0.20)
    }
}

pub(super) fn camera(
    session: Res<ArenaSession>,
    mut state: ResMut<ViewState>,
    terrain: Res<ArenaTerrainView>,
    geometry: Res<ArenaVoxelGeometry>,
    reset: Res<ArenaReset>,
    mut cameras: Query<&mut Transform, With<ArenaCamera>>,
) {
    if !active(&session) {
        return;
    }
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if state.observer.generation != Some(reset.generation) {
        let pose =
            super::encounter::overview(*geometry, &terrain, state.capture_view.ends_with("-rear"));
        let target = if session.actors.is_empty() {
            (terrain.spawns.first().copied().unwrap_or(Vec3::ZERO)
                + terrain.spawns.get(1).copied().unwrap_or(Vec3::ZERO))
                * 0.5
        } else {
            session
                .actors
                .iter()
                .map(|actor| actor.center())
                .sum::<Vec3>()
                / f32::from(u16::try_from(session.actors.len()).unwrap_or(1))
        };
        state.observer.reset(reset.generation, pose, target);
        if state.capture_view == "observer-free" {
            state.observer.toggle_mode();
        }
    }
    if state.capture.is_some()
        && state.capture_view == "observer-free"
        && (5..=65).contains(&state.frames)
    {
        let previous = state.observer.position;
        let desired = state.observer.update_pose(
            Vec2::new(0.8, 0.0),
            Vec3::new(0.3, 0.0, 1.0),
            false,
            0.0,
            1.0 / 60.0,
        );
        state.observer.position = session.camera_position(previous, desired.translation);
    }
    *camera = Transform::from_translation(state.observer.position)
        .looking_to(state.observer.direction(), Vec3::Y);
}
