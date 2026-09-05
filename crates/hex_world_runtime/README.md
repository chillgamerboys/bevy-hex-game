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

`publish_revision(workspace, &package, limits)` maintains a stable authoring
workspace. Revisions live under immutable `packages/{fingerprint}/manifest.ron`;
an atomically replaced, bounded `current.ron` selects the revision only after the
complete package is durable. OS writer locking prevents concurrent publishers.
`FileChunkSource::open_workspace(workspace, limits)` accepts that workspace or a
direct immutable package directory. A failed publication preserves the old pointer.

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

The shared `ManifestIndex` is constructed once and retained by file sources, the
runtime, worker admission, edited chunk validation, and the knowledge store. Queries and interest disk
enumeration use local chunk/region entries. Normal pumps inspect active interests,
resident chunks, and bounded jobs, without traversing the dormant chunk catalogue.

## Edits, replication and persistence

`apply_transaction` validates complete revision expectations and stages all affected
chunks before replacing any. Transactions are idempotent by ID plus exact request
fingerprint. No-op commands are rejected. Ordinary edits preserve indestructible
materials and refuse unsupported object/boundary or anchor/interior/liquid semantic
regeneration with a concrete error. Unrelated resident products retain their `Arc`.

`transaction_delta(id) -> RuntimeResult<Option<WorldDelta>>` reads one owned local
changed-column payload, from a recent cache or its paged journal file. `apply_delta` checks
the exact source, prior revision/package fingerprint, next revision and resulting
package fingerprint. Duplicates return the original outcome; reordered or mismatched
messages fail atomically. These two methods change memory only.

Use `apply_transaction_durable` or `apply_delta_durable` when the returned outcome
is an acknowledgment that must survive restart. `save` checkpoints accumulated
memory changes. Both write immutable changed-partition and transaction files, flush
them, then atomically replace `current.ron`. Unrelated partition files retain their
path and are not rewritten. An OS file lock serializes writers; a stale authority
cannot overwrite acknowledged transactions it has not restored.

`restore_save` checks the exact fresh V4 source and lightweight idempotency index
before replacing authority state. Historical transaction bodies stay on disk and
are validated individually when requested; corruption fails that lookup. Modified chunk payloads stay on disk until needed;
a corrupt lazy partition fails its load and never becomes queryable. Restore requires
drained jobs and no operation pins. Unloaded saved terrain drops its column payload.
Unsaved partition and transaction backlogs have explicit count/byte budgets;
checkpointing releases them. Only a bounded recent durable transaction-body cache
stays in memory. `history_counts` separates light history metadata, recent bodies,
and unsaved bodies. Restoring a long history loads no transaction bodies.

## Atomic owner checkpoint attachments

`save_with_attachments(root, limits, updates)` and
`apply_transaction_durable_with_attachments(transaction, root, limits, updates)`
commit opaque owner bytes in the same atomic head as terrain. The delta variant
has the same boundary. An `AttachmentUpdate` supplies an owner namespace, key,
expected prior fingerprint, and replacement bytes. `None` bytes explicitly delete;
unmentioned keys survive ordinary saves and durable edits. Immutable body files
are bounded by `max_chunk_bytes`, update batches by `max_transaction_bytes`, and
update counts by `RuntimeConfig::max_attachment_updates`.

`attachment(owner, key)` reads one verified body from the last committed/restored
head. Payloads remain on disk otherwise. The owner must decode and validate its
format before applying gameplay state; the runtime never interprets actors or
encounters. Compare-and-write detects actor-only stale writers. Terrain transaction
IDs bind the exact attachment request, so altered retries fail and exact old retries
cannot roll back later actor movement. Previously committed terrain-only IDs cannot
retroactively acquire attachments. A failure before the head switch preserves both
the prior terrain and prior owner references, including when new immutable bodies
have already been flushed.

## Principal-private knowledge and reconnect

`KnowledgeStore::open(root, &manifest, limits, config)` reads metadata only.
`compare_and_write(principal, id, &expected_revisions, replacements)` atomically
persists only the selected principal/chunk partitions before returning a receipt.
A `KnowledgePartition` has an independent monotonic revision, exact discovered
columns, observed `Surface` identities and clearances with terrain revisions, and
observed stable landmark IDs/anchors. Terrain residency grants no observation.
Unsupported materials, unknown landmarks, revision rollback, invalid source
identity and conflicting idempotency IDs are rejected. Distinct writers merge
against the locked current head; one party does not replace another party's memory.

`read` loads one private partition. `discovered_chunks` and `discovered_columns`
read only that principal's compact discovery masks; these support a private atlas
independently of resident terrain. The public manifest geography is separate from
these private observations. Call `refresh` when another writer changes the store.

The host creates `AuthorizedInterest` after deciding the authenticated principal
and authorized chunks. It deliberately cannot be deserialized from a client's
claim. `DisclosureStream` sends only already-declassified partitions belonging to
that principal within those interests. Its retained replay is bounded by count
and bytes. Changed interests discard old-scope replay. `reconnect` returns retained
contiguous batches or requests `checkpoint_page` calls, each bounded by partition
count and bytes. Checkpoint fingerprints cover only the authorized private scope,
so unrelated party updates do not invalidate a reconnect. A changed scoped snapshot
requires restarting from the first page.

Receivers call `apply_sequence_durable` or `apply_checkpoint_page_durable` with a
host-approved scope. Gaps, conflicting duplicates and unauthorized payloads fail
atomically. Knowledge and sequence/page progress share one durable commit. Only the
final checkpoint page acknowledges the snapshot's sequence; interrupted paging
resumes after restart. Sequence state is per principal and stream, independent of
combat turns. This protocol supplies no sockets or authentication implementation.
A host restarting a stream must use a new identity or provide a host-verified
sequence to `DisclosureStream::resume`.

## Deliberate remaining boundaries

The finite manifest/catalogue, save-head metadata, private discovery masks, and
lightweight idempotency/sequence indexes are still in memory. Metadata heads are
rewritten as bounded files; they should gain sharded indexes when measured session
history warrants it. Fine terrain overlays, historical delta bodies, and private
knowledge bodies are independently paged. Immutable orphan files remain for crash
safety until a separate garbage collector exists.

Semantic terrain regeneration, actor/encounter partitions, procedural unbounded
catalogues, transport authentication/sockets, and renderer asset budgets belong to
future adapters. The host owns declassification and decides which observations are
true; this store validates representation, registered identities and revisions,
without reading hidden terrain to manufacture knowledge.
