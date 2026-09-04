# V4 resident world authority

This crate has no Bevy, renderer, encounter, actor, transport, or global pause
dependency. It owns exact world residency and terrain revisions. A host drives
`pump`, publishes its returned immutable `ChunkProduct`s, and uses `WorldQuery`
for availability-aware movement and collision.

```rust,no_run
use std::sync::Arc;
use hex_world_contracts::{ResidencyRequest, WorldHex, WorldQuery};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldRuntime};

# fn run() -> Result<(), Box<dyn std::error::Error>> {
let source = FileChunkSource::open("compiled/manifest.ron", IoLimits::default())?;
let mut world = WorldRuntime::new(Arc::new(source), RuntimeConfig::default())?;
world.set_interests(vec![ResidencyRequest {
    id: "party-a".into(), center: WorldHex::new(-1, 4),
    radius: 24, retention_radius: 32, priority: 10,
}])?;
let products = world.pump(); // never waits for source IO
// Remove retired products, then publish loaded/changed products in the host.
let exact = world.surfaces(WorldHex::new(-1, 4));
# let _ = (products, exact);
# Ok(())
# }
```

## Sources and publication

`publish_package(directory, &WorldPackage, IoLimits)` creates an immutable package
directory. Chunk RON files and the manifest are flushed inside a staging directory
before a directory rename makes the package visible. Existing different packages
are rejected. Identical retries verify all existing chunk payloads.

`FileChunkSource::open(manifest_path, limits)` reads the manifest alone. Each job
reads one addressed chunk within its byte budget. Wire shape, canonical ordering,
world identity, exact declared coverage, semantic consequences, and descriptor
fingerprints are checked before publication. Relative path traversal and symlink
escapes are rejected. `MemoryChunkSource` serves explicitly supplied test/embedded
packages; `ChunkSource` is the extension point for another finite or generated store.

## Active work and availability

Interests union across separated parties. Higher priorities load first. Retention
preserves an already resident hysteresis band without loading that band itself.
Explicit named pins hold shared chunks independently of actor interests. Requests
that exceed configured budgets leave the preceding interest plan intact.

Workers have cooperative cancellation and independent publication tickets/source
epochs/revisions. A canceled noncooperative worker keeps its slot until it actually
finishes. `pump` never joins an unfinished thread. Loading failures publish no partial
chunk and require explicit `retry`. `WorldRuntime` is `Send + Sync`.

`Ready(None)` is known air; `Unloaded` is unavailable world terrain; `OutsideWorld`
is outside the declared finite footprint. All exposed solid stacks retain exact
air clearance. Compiler-projected object occupancy is merged locally, with object
material taking precedence over overlapping terrain. A foreign object's root chunk
can unload without removing its collision or support surfaces.

The manifest and its region index are constructed once. Queries and interest disk
enumeration use local chunk/region entries. Normal pumps inspect active interests,
resident chunks, and bounded jobs, without traversing the dormant chunk catalogue.

## Edits, replication and persistence

`apply_transaction` validates complete revision expectations and stages all affected
chunks before replacing any. Transactions are idempotent by ID plus exact request
fingerprint. No-op commands are rejected. Ordinary edits preserve indestructible
materials and refuse unsupported object/boundary or anchor/interior/liquid semantic
regeneration with a concrete error. Unrelated resident products retain their `Arc`.

`transaction_delta` exposes the local changed-column payload. `apply_delta` checks
the exact source, prior revision/package fingerprint, next revision and resulting
package fingerprint. Duplicates return the original outcome; reordered or mismatched
messages fail atomically. These two methods change memory only.

Use `apply_transaction_durable` or `apply_delta_durable` when the returned outcome
is an acknowledgment that must survive restart. `save` checkpoints accumulated
memory changes. Both write immutable changed-partition and transaction files, flush
them, then atomically replace `current.ron`. Unrelated partition files retain their
path and are not rewritten. An OS file lock serializes writers; a stale authority
cannot overwrite acknowledged transactions it has not restored.

`restore_save` checks the exact fresh V4 source and complete idempotency journal
before replacing authority state. Modified chunk payloads stay on disk until needed;
a corrupt lazy partition fails its load and never becomes queryable. Restore requires
drained jobs and no operation pins. Unloaded saved terrain drops its column payload.
Unsaved partition backlog has an explicit budget; checkpointing releases it.

## Deliberate remaining boundaries

The finite manifest/catalogue and historical transaction records are currently
in memory. The full prior delta journal should gain independently paged lookup for
very long sessions. Save-head metadata is rewritten as one bounded file; immutable
orphan files are retained for crash safety until a separate garbage collector exists.

Knowledge/disclosure persistence, transport session sequence/reconnect orchestration,
semantic terrain regeneration, actor/encounter partitions, procedural unbounded
catalogues, and renderer asset budgets belong to their respective future adapters.
This crate's delta protocol supplies the local revision and durable idempotency
foundation; it does not claim to implement those larger features.
