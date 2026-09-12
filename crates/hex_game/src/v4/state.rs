//! Explorer-owned gameplay checkpoint; terrain remains owned by the world runtime.

use super::{ExplorerActor, Session};
use hex_world_contracts::{
    hash_serializable, QueryResult, VoxelPosition, WorldManifest, WorldQuery,
};
use hex_world_runtime::{AttachmentUpdate, IoLimits, WorldRuntime};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

const MAX_BYTES: u64 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct GameplayCheckpoint {
    version: u32,
    world_id: String,
    base_fingerprint: u64,
    revision: u64,
    selected_actor: String,
    actors: Vec<SavedActor>,
    fingerprint: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedActor {
    id: String,
    support: VoxelPosition,
    turn_steps: bool,
}

impl GameplayCheckpoint {
    fn fingerprint(&self) -> Result<u64, String> {
        hash_serializable(&(
            self.version,
            &self.world_id,
            self.base_fingerprint,
            self.revision,
            &self.selected_actor,
            &self.actors,
        ))
        .map_err(|error| error.to_string())
    }

    fn validate(&self, manifest: &WorldManifest) -> Result<(), String> {
        if self.version != 1
            || self.revision == 0
            || self.world_id != manifest.world_id
            || self.base_fingerprint != manifest.fingerprint
            || self.fingerprint != self.fingerprint()?
            || self.actors.is_empty()
            || self.actors.len() > 32
        {
            return Err("gameplay checkpoint format, world, revision or integrity mismatch".into());
        }
        let mut ids = BTreeSet::new();
        for actor in &self.actors {
            if actor.id.is_empty()
                || !ids.insert(&actor.id)
                || !manifest
                    .contains(actor.support.column)
                    .map_err(|error| error.to_string())?
            {
                return Err(
                    "gameplay checkpoint has duplicate actors or outside-world support".into(),
                );
            }
        }
        if !ids.contains(&self.selected_actor) {
            return Err("selected saved actor is missing".into());
        }
        Ok(())
    }
}

fn read(runtime: &WorldRuntime) -> Result<Option<(GameplayCheckpoint, u64)>, String> {
    let Some(attachment) = runtime
        .attachment("gameplay", "explorer")
        .map_err(|error| error.to_string())?
    else {
        return Ok(None);
    };
    if u64::try_from(attachment.bytes.len()).map_err(|error| error.to_string())? > MAX_BYTES {
        return Err("gameplay checkpoint exceeds its budget".into());
    }
    let checkpoint: GameplayCheckpoint =
        ron::de::from_bytes(&attachment.bytes).map_err(|error| error.to_string())?;
    checkpoint.validate(runtime.manifest())?;
    Ok(Some((checkpoint, attachment.fingerprint)))
}

/// Restore exact supports as pending queries. Residency validates them before play.
pub(super) fn restore(
    runtime: &WorldRuntime,
    actors: &mut [ExplorerActor],
) -> Result<(usize, u64), String> {
    let Some((checkpoint, _fingerprint)) = read(runtime)? else {
        return Ok((0, 0));
    };
    if checkpoint.actors.len() != actors.len()
        || actors
            .iter()
            .any(|actor| !checkpoint.actors.iter().any(|saved| saved.id == actor.id))
    {
        return Err("gameplay checkpoint roster differs from this explorer roster".into());
    }
    for actor in actors.iter_mut() {
        let saved = checkpoint
            .actors
            .iter()
            .find(|saved| saved.id == actor.id)
            .ok_or("saved actor is missing")?;
        actor.column = saved.support.column;
        actor.requested_level = Some(saved.support.level);
        actor.standing = None;
        actor.turn_steps = saved.turn_steps;
    }
    let selected = actors
        .iter()
        .position(|actor| actor.id == checkpoint.selected_actor)
        .ok_or("selected actor is missing")?;
    Ok((selected, checkpoint.revision))
}

/// A gameplay-owned payload for the same atomic head as a terrain save or edit.
pub(super) struct PreparedGameplay {
    pub update: AttachmentUpdate,
    pub revision: u64,
}

pub(super) fn prepare(
    runtime: &WorldRuntime,
    session: &Session,
) -> Result<PreparedGameplay, String> {
    let previous = read(runtime)?;
    let current = previous
        .as_ref()
        .map_or(0, |(checkpoint, _)| checkpoint.revision);
    if current != session.gameplay_revision {
        return Err("gameplay checkpoint changed in another session".into());
    }
    let actors = session
        .actors
        .iter()
        .map(|actor| {
            let support = actor.standing.ok_or("cannot save a character before exact support is loaded")?;
            if !matches!(runtime.surfaces(support.column), QueryResult::Ready(surfaces) if surfaces.iter().any(|surface| surface.position == support && surface.headroom.is_none_or(|headroom| headroom >= 2))) {
                return Err(format!("cannot save {} on unavailable or invalid support", actor.id));
            }
            Ok(SavedActor {
                id: actor.id.clone(),
                support,
                turn_steps: actor.turn_steps,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let manifest = runtime.manifest();
    let mut checkpoint = GameplayCheckpoint {
        version: 1,
        world_id: manifest.world_id.clone(),
        base_fingerprint: manifest.fingerprint,
        revision: current.checked_add(1).ok_or("gameplay revision overflow")?,
        selected_actor: session
            .actors
            .get(session.selected)
            .ok_or("selected actor is missing")?
            .id
            .clone(),
        actors,
        fingerprint: 0,
    };
    checkpoint.fingerprint = checkpoint.fingerprint()?;
    checkpoint.validate(manifest)?;
    let bytes = ron::ser::to_string_pretty(&checkpoint, ron::ser::PrettyConfig::default())
        .map_err(|error| error.to_string())?
        .into_bytes();
    if u64::try_from(bytes.len()).map_err(|error| error.to_string())? > MAX_BYTES {
        return Err("gameplay checkpoint exceeds its budget".into());
    }
    Ok(PreparedGameplay {
        update: AttachmentUpdate {
            owner: "gameplay".into(),
            key: "explorer".into(),
            expected_fingerprint: previous.map(|(_, fingerprint)| fingerprint),
            bytes: Some(bytes),
        },
        revision: checkpoint.revision,
    })
}

/// Terrain and gameplay publish through one atomic durable head. Transient routes
/// and interpolation are intentionally resumed at the last exact support.
pub(super) fn save(
    directory: &Path,
    runtime: &mut WorldRuntime,
    session: &mut Session,
) -> Result<(), String> {
    let prepared = prepare(runtime, session)?;
    runtime
        .save_with_attachments(directory, IoLimits::default(), &[prepared.update])
        .map_err(|error| error.to_string())?;
    session.gameplay_revision = prepared.revision;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_world_contracts::{QueryResult, VoxelEdit, WorldEditTransaction, WorldHex, WorldQuery};

    struct Scratch(std::path::PathBuf);
    impl Scratch {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "v4-gameplay-{}-{nanos}-{serial}",
                std::process::id()
            )))
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _removed = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn same_head_reopens_terrain_and_exact_actor_state_and_rejects_stale_session() {
        let (mut session, mut runtime) = crate::v4::walk::tests::fixture();
        let scratch = Scratch::new();
        session.selected = 1;
        session.actors.get_mut(1).expect("second party").turn_steps = true;
        let prepared = prepare(&runtime, &session).expect("gameplay payload");
        let position = VoxelPosition {
            column: WorldHex::new(13, 0),
            level: 2,
        };
        let transaction = WorldEditTransaction {
            id: "gameplay-atomic-test".into(),
            expected_revisions: std::collections::BTreeMap::from([(
                position.column.chunk(),
                runtime.revision(position.column.chunk()).expect("resident"),
            )]),
            edits: vec![VoxelEdit {
                position,
                material: None,
            }],
        };
        runtime
            .apply_transaction_durable_with_attachments(
                &transaction,
                &scratch.0,
                IoLimits::default(),
                &[prepared.update],
            )
            .expect("one head");
        assert!(
            prepare(&runtime, &session).is_err(),
            "old gameplay revision cannot overwrite committed actors"
        );
        session.gameplay_revision = prepared.revision;
        assert!(prepare(&runtime, &session).is_ok());
        let (mut restored, mut reopened) = crate::v4::walk::tests::fixture();
        reopened
            .restore_save(&scratch.0, IoLimits::default())
            .expect("restore same head");
        let (selected, revision) =
            restore(&reopened, &mut restored.actors).expect("restore gameplay");
        assert_eq!((selected, revision), (1, 1));
        let actor = restored.actors.get(1).expect("second party");
        assert!(actor.turn_steps);
        assert_eq!(actor.requested_level, Some(2));
        assert_eq!(actor.column, WorldHex::new(100, 0));
        assert!(
            actor.standing.is_none(),
            "restored support is revalidated through residency"
        );
        for _ in 0..1000 {
            let update = reopened.pump();
            assert!(update.failures.is_empty());
            if matches!(reopened.voxel(position), QueryResult::Ready(_)) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        assert_eq!(reopened.voxel(position), QueryResult::Ready(None));
    }

    #[test]
    fn invalid_roster_or_unresolved_support_cannot_replace_checkpoint() {
        let (mut session, mut runtime) = crate::v4::walk::tests::fixture();
        let scratch = Scratch::new();
        save(&scratch.0, &mut runtime, &mut session).expect("initial checkpoint");
        let before = runtime
            .attachment("gameplay", "explorer")
            .expect("attachment")
            .expect("present")
            .fingerprint;
        session.actors.get_mut(0).expect("actor").standing = None;
        assert!(save(&scratch.0, &mut runtime, &mut session).is_err());
        assert_eq!(
            runtime
                .attachment("gameplay", "explorer")
                .expect("attachment")
                .expect("present")
                .fingerprint,
            before
        );
        session.actors.pop();
        assert!(restore(&runtime, &mut session.actors).is_err());
    }
}
