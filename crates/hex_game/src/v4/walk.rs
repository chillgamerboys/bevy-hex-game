//! Bounded, runtime-loaded commands and typed observations for windowless walks.
//!
//! This module never moves an actor or changes terrain directly. The composition
//! root calls it before its ordinary movement/edit systems, once per update.

use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

use bevy::prelude::Resource;
use hex_world_contracts::{ChunkId, QueryResult, VoxelPosition, WorldQuery};
use hex_world_runtime::WorldRuntime;
use serde::{Deserialize, Serialize};

use super::{EditRequest, MoveRequest, Session};

const MAX_SCRIPT_BYTES: u64 = 262_144;
const MAX_STEPS: usize = 2_048;
const MAX_TOTAL_TICKS: u64 = 100_000;
const MAX_STEP_TICKS: u64 = 20_000;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct WalkScript {
    schema_version: u32,
    id: String,
    max_ticks: u64,
    steps: Vec<WalkStep>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
enum WalkStep {
    SelectActor {
        actor: String,
    },
    SetStepMode {
        actor: String,
        enabled: bool,
    },
    MoveTo {
        actor: String,
        goal: VoxelPosition,
    },
    StepOnce {
        actor: String,
    },
    WaitAt {
        actor: String,
        position: VoxelPosition,
        max_ticks: u64,
    },
    AssertAt {
        actor: String,
        position: VoxelPosition,
    },
    EditVoxel {
        position: VoxelPosition,
        max_ticks: u64,
    },
    AssertVoxel {
        position: VoxelPosition,
        material: Option<String>,
    },
    Save {
        max_ticks: u64,
    },
    WaitChunk {
        coordinate: ChunkId,
        resident: bool,
        max_ticks: u64,
    },
    WaitTicks {
        ticks: u64,
    },
}

impl WalkStep {
    fn actor(&self) -> Option<&str> {
        match self {
            Self::SelectActor { actor }
            | Self::SetStepMode { actor, .. }
            | Self::MoveTo { actor, .. }
            | Self::StepOnce { actor }
            | Self::WaitAt { actor, .. }
            | Self::AssertAt { actor, .. } => Some(actor),
            _ => None,
        }
    }

    fn position(&self) -> Option<VoxelPosition> {
        match self {
            Self::MoveTo { goal, .. } => Some(*goal),
            Self::WaitAt { position, .. }
            | Self::AssertAt { position, .. }
            | Self::EditVoxel { position, .. }
            | Self::AssertVoxel { position, .. } => Some(*position),
            _ => None,
        }
    }

    fn budget(&self) -> Option<u64> {
        match self {
            Self::WaitAt { max_ticks, .. }
            | Self::EditVoxel { max_ticks, .. }
            | Self::Save { max_ticks }
            | Self::WaitChunk { max_ticks, .. } => Some(*max_ticks),
            _ => None,
        }
    }
}

#[derive(Debug, Serialize)]
pub(super) struct ActorReceipt {
    id: String,
    position: Option<VoxelPosition>,
    motion_to: Option<VoxelPosition>,
    motion_fraction: Option<f64>,
    queued_steps: usize,
    turn_steps: bool,
    pending_goal: Option<VoxelPosition>,
}

/// Typed evidence for one completed instruction, separate from native input review.
#[derive(Debug, Serialize)]
pub(super) struct WalkReceipt {
    script: String,
    script_fingerprint: String,
    step: usize,
    tick: u64,
    command: WalkStep,
    evidence: &'static str,
    native_motion: &'static str,
    verified_moves: usize,
    actors: Vec<ActorReceipt>,
    resident_chunks: usize,
    pinned_chunks: usize,
    observed_chunk: Option<ChunkId>,
    observed_revision: Option<u64>,
    successful_saves: u64,
    gameplay_revision: u64,
}

enum PendingOperation {
    Edit { revision: u64 },
    Save { successful_saves: u64 },
}

/// Script state only; movement, residency and edits retain their existing owners.
#[derive(Resource)]
pub(super) struct WalkHarness {
    script: WalkScript,
    script_fingerprint: u64,
    cursor: usize,
    ticks: u64,
    step_started: u64,
    checked: bool,
    pending: Option<PendingOperation>,
    issued_moves: BTreeMap<String, (VoxelPosition, VoxelPosition)>,
    verified_moves: usize,
    receipts: Vec<WalkReceipt>,
    seen_resident: std::collections::BTreeSet<ChunkId>,
}

impl WalkHarness {
    pub(super) fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let mut source = String::new();
        File::open(path.as_ref())
            .map_err(|error| format!("walk {}: {error}", path.as_ref().display()))?
            .take(MAX_SCRIPT_BYTES + 1)
            .read_to_string(&mut source)
            .map_err(|error| format!("walk {}: {error}", path.as_ref().display()))?;
        Self::parse(&source)
    }

    fn parse(source: &str) -> Result<Self, String> {
        if source.len() as u64 > MAX_SCRIPT_BYTES {
            return Err("walk script exceeds 256 KiB".into());
        }
        let script: WalkScript =
            ron::from_str(source).map_err(|error| format!("invalid walk script: {error}"))?;
        if script.schema_version != 1
            || script.id.trim().is_empty()
            || script.id.len() > 128
            || !(1..=MAX_TOTAL_TICKS).contains(&script.max_ticks)
            || !(1..=MAX_STEPS).contains(&script.steps.len())
        {
            return Err(
                "walk requires schema_version1, an ID, 1..2048 steps and 1..100000 max_ticks"
                    .into(),
            );
        }
        let mut moves = Vec::new();
        let mut checked_move = false;
        for step in &script.steps {
            if step
                .actor()
                .is_some_and(|actor| actor.trim().is_empty() || actor.len() > 256)
            {
                return Err("walk actor IDs must contain 1..256 bytes".into());
            }
            if step.budget().is_some_and(|ticks| {
                !(1..=MAX_STEP_TICKS).contains(&ticks) || ticks > script.max_ticks
            }) {
                return Err("walk wait budget must be 1..20000 and within script max_ticks".into());
            }
            match step {
                WalkStep::MoveTo { actor, goal } => moves.push((actor, goal)),
                WalkStep::WaitAt {
                    actor, position, ..
                }
                | WalkStep::AssertAt { actor, position } => {
                    checked_move |= moves.contains(&(actor, position));
                }
                WalkStep::WaitTicks { ticks }
                    if !(1..=MAX_STEP_TICKS).contains(ticks) || *ticks > script.max_ticks =>
                {
                    return Err(
                        "walk WaitTicks must be 1..20000 and within script max_ticks".into(),
                    );
                }
                _ => {}
            }
        }
        if !checked_move {
            return Err(
                "walk must MoveTo and then WaitAt/AssertAt that actor's exact destination".into(),
            );
        }
        let script_fingerprint =
            hex_world_contracts::hash_serializable(&script).map_err(|error| error.to_string())?;
        Ok(Self {
            script,
            script_fingerprint,
            cursor: 0,
            ticks: 0,
            step_started: 0,
            checked: false,
            pending: None,
            issued_moves: BTreeMap::new(),
            verified_moves: 0,
            receipts: Vec::new(),
            seen_resident: Default::default(),
        })
    }

    pub(super) fn completed(&self) -> bool {
        self.cursor == self.script.steps.len() && self.verified_moves > 0
    }

    pub(super) fn receipts(&self) -> &[WalkReceipt] {
        &self.receipts
    }

    fn preflight(&self, session: &Session, runtime: &WorldRuntime) -> Result<(), String> {
        for (index, step) in self.script.steps.iter().enumerate() {
            if let Some(actor) = step.actor() {
                actor_index(session, actor)
                    .map_err(|error| format!("walk step {index}: {error}"))?;
            }
            if let Some(position) = step.position() {
                if matches!(runtime.surfaces(position.column), QueryResult::OutsideWorld) {
                    return Err(format!(
                        "walk step {index}: destination {position:?} is outside the world"
                    ));
                }
            }
            if let WalkStep::WaitChunk { coordinate, .. } = step {
                if runtime
                    .manifest()
                    .chunks
                    .binary_search_by_key(coordinate, |chunk| chunk.coordinate)
                    .is_err()
                {
                    return Err(format!("walk step {index}: unknown chunk {coordinate:?}"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn tick(
        &mut self,
        session: &mut Session,
        runtime: &mut WorldRuntime,
    ) -> Result<(), String> {
        if self.completed() {
            return Ok(());
        }
        if !self.checked {
            self.preflight(session, runtime)?;
            self.checked = true;
        }
        if let Some(error) = &session.error {
            return Err(format!("walk {}: explorer failed: {error}", self.script.id));
        }
        self.ticks += 1;
        if self.ticks > self.script.max_ticks {
            return Err(format!(
                "walk {} exceeded its total tick budget at step {}",
                self.script.id, self.cursor
            ));
        }
        self.seen_resident
            .extend(runtime.resident_chunks().map(|chunk| chunk.coordinate));
        let step = self
            .script
            .steps
            .get(self.cursor)
            .ok_or("walk exhausted without verified motion")?
            .clone();
        if self.step_started == 0 {
            self.step_started = self.ticks;
        }
        let elapsed = self.ticks - self.step_started;
        if step.budget().is_some_and(|budget| elapsed >= budget) {
            return Err(format!(
                "walk {} step {} timed out: {step:?}",
                self.script.id, self.cursor
            ));
        }
        let finished = self
            .execute(&step, elapsed, session, runtime)
            .map_err(|error| format!("walk {} step {}: {error}", self.script.id, self.cursor))?;
        if finished {
            self.record(step, session, runtime);
            self.cursor += 1;
            self.step_started = 0;
            self.pending = None;
            if self.cursor == self.script.steps.len() && self.verified_moves == 0 {
                return Err("walk completed no successful exact movement observation".into());
            }
        }
        Ok(())
    }

    fn execute(
        &mut self,
        step: &WalkStep,
        elapsed: u64,
        session: &mut Session,
        runtime: &WorldRuntime,
    ) -> Result<bool, String> {
        match step {
            WalkStep::SelectActor { actor } => {
                session.selected = actor_index(session, actor)?;
            }
            WalkStep::SetStepMode { actor, enabled } => {
                let index = actor_index(session, actor)?;
                session
                    .actors
                    .get_mut(index)
                    .ok_or("missing actor")?
                    .turn_steps = *enabled;
            }
            WalkStep::MoveTo { actor, goal } => {
                let index = actor_index(session, actor)?;
                let actor_state = session.actors.get_mut(index).ok_or("missing actor")?;
                let Some(from) = actor_state.standing else {
                    return Ok(false);
                };
                if from == *goal && actor_state.motion.is_none() {
                    return Err(
                        "MoveTo must request actual movement, not the current support".into(),
                    );
                }
                actor_state.requested = Some(MoveRequest {
                    goal: *goal,
                    waiting: Default::default(),
                });
                self.issued_moves.insert(actor.clone(), (from, *goal));
            }
            WalkStep::StepOnce { actor } => {
                let index = actor_index(session, actor)?;
                let state = session.actors.get(index).ok_or("missing actor")?;
                if !state.turn_steps {
                    return Err(format!("StepOnce requires {actor} to be in step mode"));
                }
                if state.motion.is_some() || state.requested.is_some() {
                    return Ok(false);
                }
                if state.route.is_empty() {
                    return Err(format!("StepOnce has no accepted route for {actor}"));
                }
                session.selected = index;
                session.step_requested = true;
            }
            WalkStep::WaitAt {
                actor, position, ..
            } => {
                let state = session
                    .actors
                    .get(actor_index(session, actor)?)
                    .ok_or("missing actor")?;
                if state.standing != Some(*position)
                    || state.motion.is_some()
                    || state.requested.is_some()
                    || (!state.turn_steps && !state.route.is_empty())
                {
                    return Ok(false);
                }
                self.observe_move(actor, *position);
            }
            WalkStep::AssertAt { actor, position } => {
                let state = session
                    .actors
                    .get(actor_index(session, actor)?)
                    .ok_or("missing actor")?;
                if state.standing != Some(*position) {
                    return Err(format!(
                        "actor {actor} is at {:?}, expected {position:?}",
                        state.standing
                    ));
                }
                self.observe_move(actor, *position);
            }
            WalkStep::EditVoxel { position, .. } => {
                if let Some(PendingOperation::Edit { revision }) = self.pending {
                    let expected = revision.checked_add(1).ok_or("edit revision overflow")?;
                    match runtime.revision(position.column.chunk()) {
                        Some(actual) if actual == expected => {
                            if runtime.voxel(*position) != QueryResult::Ready(None) {
                                return Err(
                                    "edit revision advanced without removing the exact voxel"
                                        .into(),
                                );
                            }
                        }
                        Some(actual) if actual == revision && session.edit_requested.is_none() => {
                            return Err("edit was refused or had no effect".into());
                        }
                        Some(actual) if actual != revision => {
                            return Err(format!(
                                "edit observed unexpected revision {actual}; expected {expected}"
                            ));
                        }
                        _ => return Ok(false),
                    }
                } else {
                    if session.edit_requested.is_some() {
                        return Ok(false);
                    }
                    match runtime.voxel(*position) {
                        QueryResult::Ready(Some(_)) => {}
                        QueryResult::Unloaded(_) => return Ok(false),
                        _ => return Err("EditVoxel requires an existing exact voxel".into()),
                    }
                    let revision = runtime
                        .revision(position.column.chunk())
                        .ok_or("edit chunk not resident")?;
                    session.edit_requested = Some(EditRequest {
                        position: *position,
                        observed_revision: revision,
                    });
                    self.pending = Some(PendingOperation::Edit { revision });
                    return Ok(false);
                }
            }
            WalkStep::AssertVoxel { position, material } => {
                let actual = runtime.voxel(*position);
                if actual != QueryResult::Ready(material.clone()) {
                    return Err(format!(
                        "voxel {position:?}: expected {material:?}, got {actual:?}"
                    ));
                }
            }
            WalkStep::Save { .. } => {
                if let Some(PendingOperation::Save { successful_saves }) = self.pending {
                    if session.successful_saves <= successful_saves {
                        if !session.save_requested {
                            return Err(
                                "save was refused or did not publish its typed completion".into()
                            );
                        }
                        return Ok(false);
                    }
                } else {
                    if session.save_requested {
                        return Ok(false);
                    }
                    self.pending = Some(PendingOperation::Save {
                        successful_saves: session.successful_saves,
                    });
                    session.save_requested = true;
                    return Ok(false);
                }
            }
            WalkStep::WaitChunk {
                coordinate,
                resident,
                ..
            } => {
                if !resident && !self.seen_resident.contains(coordinate) {
                    return Err(format!(
                        "cannot witness unloading {coordinate:?} before it was resident"
                    ));
                }
                if runtime.revision(*coordinate).is_some() != *resident {
                    return Ok(false);
                }
            }
            WalkStep::WaitTicks { ticks } => return Ok(elapsed >= *ticks),
        }
        Ok(true)
    }

    fn observe_move(&mut self, actor: &str, position: VoxelPosition) {
        if self
            .issued_moves
            .get(actor)
            .is_some_and(|(from, goal)| *goal == position && *from != position)
        {
            self.verified_moves += 1;
            self.issued_moves.remove(actor);
        }
    }

    fn record(&mut self, step: WalkStep, session: &Session, runtime: &WorldRuntime) {
        let observed_chunk = match &step {
            WalkStep::WaitChunk { coordinate, .. } => Some(*coordinate),
            _ => step.position().map(|position| position.column.chunk()),
        };
        let counts = runtime.counts();
        self.receipts.push(WalkReceipt {
            script: self.script.id.clone(),
            script_fingerprint: format!("{:016x}", self.script_fingerprint),
            step: self.cursor,
            tick: self.ticks,
            command: step,
            evidence: "AUTOMATED-TYPED-MOTION",
            native_motion: "HUMAN-MOTION-PENDING",
            verified_moves: self.verified_moves,
            actors: session
                .actors
                .iter()
                .map(|actor| ActorReceipt {
                    id: actor.id.clone(),
                    position: actor.standing,
                    motion_to: actor.motion.map(|motion| motion.to),
                    motion_fraction: actor.motion.map(|motion| motion.fraction),
                    queued_steps: actor.route.len(),
                    turn_steps: actor.turn_steps,
                    pending_goal: actor.requested.as_ref().map(|request| request.goal),
                })
                .collect(),
            resident_chunks: counts.resident_chunks,
            pinned_chunks: counts.pinned_chunks,
            observed_chunk,
            observed_revision: observed_chunk.and_then(|chunk| runtime.revision(chunk)),
            successful_saves: session.successful_saves,
            gameplay_revision: session.gameplay_revision,
        });
    }
}

fn actor_index(session: &Session, actor: &str) -> Result<usize, String> {
    session
        .actors
        .iter()
        .position(|candidate| candidate.id == actor)
        .ok_or_else(|| format!("unknown actor {actor:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::VecDeque, sync::Arc, time::Instant};

    use hex_world_contracts::*;
    use hex_world_runtime::{MemoryChunkSource, RuntimeConfig};

    use crate::v4::{ExplorerActor, move_actors};

    fn script(steps: &str) -> WalkHarness {
        WalkHarness::parse(&format!(
            "(schema_version:1,id:\"test-walk\",max_ticks:200,steps:[{steps}])"
        ))
        .expect("valid runtime walk source")
    }

    fn position(q: i64) -> VoxelPosition {
        VoxelPosition {
            column: WorldHex::new(q, 0),
            level: 2,
        }
    }

    fn fixture() -> (Session, WorldRuntime) {
        let mut chunks = BTreeMap::<ChunkId, ChunkPackage>::new();
        let mut regions = Vec::new();
        for (id, q) in [("a", 14), ("b", 100)] {
            let origin = WorldHex::new(q, 0);
            regions.push(RegionDescriptor {
                id: id.into(),
                origin,
                radius: 3,
                source_fingerprint: 1,
            });
            for dq in -3_i64..=3 {
                for dr in -3_i64..=3 {
                    if dq.abs().max(dr.abs()).max((dq + dr).abs()) > 3 {
                        continue;
                    }
                    let column = origin
                        .checked_add(WorldHex::new(dq, dr))
                        .expect("small disk");
                    chunks
                        .entry(column.chunk())
                        .or_insert_with(|| ChunkPackage {
                            schema_version: SCHEMA_VERSION,
                            world_id: "walk-test".into(),
                            coordinate: column.chunk(),
                            source_fingerprint: 1,
                            columns: Vec::new(),
                            features: Vec::new(),
                            semantics: Default::default(),
                            fingerprint: 0,
                        })
                        .columns
                        .push(ColumnData {
                            position: column,
                            runs: vec![VoxelRun {
                                bottom: 0,
                                top: 3,
                                material: "stone".into(),
                            }],
                        });
                }
            }
        }
        let mut package = WorldPackage {
            manifest: WorldManifest {
                schema_version: SCHEMA_VERSION,
                world_id: "walk-test".into(),
                compiler_version: "walk-test".into(),
                source_fingerprint: 1,
                materials: vec![MaterialSpec {
                    id: "stone".into(),
                    solid: true,
                    diggable: true,
                    color: [90, 90, 90, 255],
                }],
                regions,
                chunks: chunks
                    .keys()
                    .map(|coordinate| ChunkDescriptor {
                        coordinate: *coordinate,
                        fingerprint: 0,
                        path: format!("chunks/{}_{}.ron", coordinate.q, coordinate.r),
                    })
                    .collect(),
                boundaries: Vec::new(),
                summary: Vec::new(),
                features: Vec::new(),
                fingerprint: 0,
            },
            chunks,
        };
        package.seal().expect("exact fixture footprint");
        let mut runtime = WorldRuntime::new(
            Arc::new(MemoryChunkSource::new(package).expect("memory fixture")),
            RuntimeConfig::default(),
        )
        .expect("runtime");
        runtime
            .set_interests(
                [("a", 14), ("b", 100)]
                    .into_iter()
                    .map(|(id, q)| ResidencyRequest {
                        id: id.into(),
                        center: WorldHex::new(q, 0),
                        radius: 3,
                        retention_radius: 3,
                        priority: 1,
                    })
                    .collect(),
            )
            .expect("two separate party interests");
        for _ in 0..1_000 {
            let update = runtime.pump();
            assert!(update.failures.is_empty());
            let counts = runtime.counts();
            if counts.in_flight_jobs == 0 && counts.queued_chunks == 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(runtime.revision(position(14).column.chunk()).is_some());
        assert!(runtime.revision(position(103).column.chunk()).is_some());
        let actors = [("a", 14), ("b", 100)]
            .into_iter()
            .map(|(id, q)| ExplorerActor {
                id: id.into(),
                column: position(q).column,
                standing: Some(position(q)),
                requested_level: Some(2),
                entity: None,
                route: VecDeque::new(),
                motion: None,
                pinned: false,
                turn_steps: false,
                requested: None,
                planning_pinned: false,
            })
            .collect();
        (
            Session {
                actors,
                selected: 0,
                edit_requested: None,
                save_requested: false,
                interests: Vec::new(),
                rendered: Default::default(),
                desired: Default::default(),
                status: String::new(),
                error: None,
                frames: 0,
                settled_frames: 0,
                step_requested: false,
                yaw: 0.0,
                pitch: 0.0,
                distance: 0.0,
                target: None,
                capture_requested: false,
                started: Instant::now(),
                frame_milliseconds: Vec::new(),
                rebase_milliseconds: Vec::new(),
                successful_saves: 0,
                gameplay_revision: 0,
            },
            runtime,
        )
    }

    #[test]
    fn step_once_uses_its_actor_and_another_party_continues_through_real_controller() {
        let (mut session, mut runtime) = fixture();
        let mut walk = script(
            r#"
            SetStepMode(actor:"a",enabled:true),
            MoveTo(actor:"a",goal:(column:(q:17,r:0),level:2)),
            MoveTo(actor:"b",goal:(column:(q:103,r:0),level:2)),
            StepOnce(actor:"a"),
            WaitAt(actor:"a",position:(column:(q:15,r:0),level:2),max_ticks:20),
            AssertAt(actor:"a",position:(column:(q:15,r:0),level:2)),
            WaitAt(actor:"b",position:(column:(q:103,r:0),level:2),max_ticks:30),
            StepOnce(actor:"a"),
            WaitAt(actor:"a",position:(column:(q:16,r:0),level:2),max_ticks:20),
            StepOnce(actor:"a"),
            WaitAt(actor:"a",position:(column:(q:17,r:0),level:2),max_ticks:20)
        "#,
        );
        for _ in 0..100 {
            walk.tick(&mut session, &mut runtime)
                .expect("bounded scripted command");
            move_actors(&mut session, &mut runtime, 0.1).expect("ordinary motion controller");
            if walk.completed() {
                break;
            }
        }
        assert!(walk.completed());
        assert_eq!(walk.verified_moves, 2);
        let b_arrived = walk
            .receipts()
            .iter()
            .find(|receipt| {
                matches!(
                    &receipt.command, WalkStep::WaitAt { actor, .. } if actor == "b"
                )
            })
            .expect("separated continuous party arrived while a waits in step mode");
        let a = b_arrived
            .actors
            .iter()
            .find(|actor| actor.id == "a")
            .expect("actor a");
        assert_eq!(a.position, Some(position(15)));
        assert_eq!(a.queued_steps, 2);
        assert!(a.motion_to.is_none());
        assert_eq!(
            runtime.counts().pinned_chunks,
            0,
            "both route owners released their pins"
        );
    }

    #[test]
    fn script_never_teleports_and_unmet_exact_position_times_out() {
        let (mut session, mut runtime) = fixture();
        let mut walk = script(
            r#"
            MoveTo(actor:"a",goal:(column:(q:17,r:0),level:2)),
            WaitAt(actor:"a",position:(column:(q:17,r:0),level:2),max_ticks:2)
        "#,
        );
        walk.tick(&mut session, &mut runtime)
            .expect("request issued");
        let a = session
            .actors
            .iter()
            .find(|actor| actor.id == "a")
            .expect("actor a");
        assert_eq!(a.standing, Some(position(14)));
        assert_eq!(
            a.requested.as_ref().expect("owned command").goal,
            position(17)
        );
        walk.tick(&mut session, &mut runtime).expect("first wait");
        walk.tick(&mut session, &mut runtime).expect("bounded wait");
        assert!(
            walk.tick(&mut session, &mut runtime)
                .expect_err("controller was not run")
                .contains("timed out")
        );
        assert!(!walk.completed());
    }

    #[test]
    fn unknown_actor_and_outside_destination_fail_before_any_command() {
        let (mut session, mut runtime) = fixture();
        for (actor, q, expected) in [
            ("missing", 17, "unknown actor"),
            ("a", 999, "outside the world"),
        ] {
            let mut walk = script(&format!(
                "MoveTo(actor:\"{actor}\",goal:(column:(q:{q},r:0),level:2)),WaitAt(actor:\"{actor}\",position:(column:(q:{q},r:0),level:2),max_ticks:20)"
            ));
            assert!(
                walk.tick(&mut session, &mut runtime)
                    .expect_err("bad request")
                    .contains(expected)
            );
            assert!(session.actors.iter().all(|actor| actor.requested.is_none()));
        }
    }

    #[test]
    fn strict_source_rejects_unknown_fields_no_work_and_unbounded_waits() {
        let valid = r#"(schema_version:1,id:"test",max_ticks:200,steps:[
            MoveTo(actor:"a",goal:(column:(q:17,r:0),level:2)),
            WaitAt(actor:"a",position:(column:(q:17,r:0),level:2),max_ticks:20)
        ])"#;
        assert!(WalkHarness::parse(valid).is_ok());
        for invalid in [
            valid.replace("schema_version:1", "schema_version:1,typo:true"),
            valid.replace("MoveTo(actor:", "MoveTo(typo:true,actor:"),
            valid.replace("max_ticks:20)", "max_ticks:20001)"),
            valid.replace("max_ticks:200,", "max_ticks:100001,"),
            "(schema_version:1,id:\"none\",max_ticks:10,steps:[WaitTicks(ticks:1)])".into(),
        ] {
            assert!(WalkHarness::parse(&invalid).is_err(), "{invalid}");
        }
        assert!(WalkHarness::parse(&" ".repeat(MAX_SCRIPT_BYTES as usize + 1)).is_err());
    }
}
