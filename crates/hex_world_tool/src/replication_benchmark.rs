//! Bounded two-process loopback acceptance for durable, local terrain deltas.
//! This harness is not production authentication or online gameplay replication.

use hex_world_contracts::{
    hash_serializable, ChunkId, QueryResult, ResidencyRequest, VoxelEdit, VoxelPosition,
    WorldEditTransaction, WorldHex, WorldQuery,
};
use hex_world_runtime::{FileChunkSource, IoLimits, RuntimeConfig, WorldDelta, WorldRuntime};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    error::Error,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const PROTOCOL: u32 = 1;
const FRAME_LIMIT: usize = 1024 * 1024;
const CONTROL_LIMIT: usize = 4096;
const TIMEOUT: Duration = Duration::from_secs(45);
const RADIUS: u32 = 18;
const MAX_CHUNKS: usize = 32;
const MAX_CANDIDATES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProductWitness {
    coordinate: ChunkId,
    revision: u64,
    fingerprint: u64,
    columns: usize,
    terrain_runs: usize,
    object_projection_runs: usize,
    root_objects: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    protocol: u32,
    session: String,
    process_id: u32,
    world_id: String,
    manifest_fingerprint: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    protocol: u32,
    center: WorldHex,
    radius: u32,
    edited_voxel: VoxelPosition,
    delta: WorldDelta,
    baseline: Vec<ProductWitness>,
    expected: Vec<ProductWitness>,
    replay: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ack {
    protocol: u32,
    process_id: u32,
    manifest_fingerprint: u64,
    delta_fingerprint: u64,
    transaction_id: String,
    replay: bool,
    before: Vec<ProductWitness>,
    after: Vec<ProductWitness>,
    published_changes: Vec<ChunkId>,
    restored_delta_identical: bool,
    save_head_before: Option<u64>,
    save_head_after: u64,
    durable_apply_milliseconds: f64,
}

#[derive(Serialize)]
struct SessionReceipt {
    process_id: u32,
    process_status: String,
    killed_after_durable_ack: bool,
    process_exit_success: bool,
    total_tcp_wire_bytes: usize,
    request_frame_bytes: usize,
    durable_ack_milliseconds: f64,
    ack: Ack,
}

#[derive(Serialize)]
struct Receipt {
    kind: &'static str,
    accepted: bool,
    scope: &'static str,
    world_id: String,
    package_fingerprint: String,
    source_path: PathBuf,
    started_unix_milliseconds: u128,
    work_directory: PathBuf,
    delta_fingerprint: String,
    delta_payload_bytes: usize,
    wire_chunks: usize,
    wire_columns: usize,
    edited_voxel: VoxelPosition,
    base_revision: u64,
    final_revision: u64,
    untouched_products: Vec<ProductWitness>,
    unchanged_products_are: &'static str,
    first_delivery: SessionReceipt,
    restarted_replay: SessionReceipt,
}

/// Runs a fresh two-process durable replication/replay acceptance case.
pub(super) fn run(package: &Path, output: &Path) -> Result<String> {
    if output.try_exists()? {
        return Err("replication receipt already exists; choose a fresh output path".into());
    }
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let name = output
        .file_name()
        .ok_or("receipt must name a file")?
        .to_string_lossy();
    let lock = parent.join(format!(".{name}.replication.lock"));
    let _lease = OutputLease::new(lock)?;
    if output.try_exists()? {
        return Err("replication receipt appeared while acquiring its lease".into());
    }
    let started_unix_milliseconds = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let run_id = nonce()?;
    let work = parent.join(format!(
        "{name}.work-{}",
        run_id.get(..16).ok_or("invalid run identity")?
    ));
    fs::create_dir(&work)?;
    let work = work.canonicalize()?;
    let package = package.canonicalize()?;
    let source = FileChunkSource::open_workspace(&package, IoLimits::default())?;
    let world_id = source.manifest().world_id.clone();
    let fingerprint = source.manifest().fingerprint;
    let center = source
        .manifest()
        .regions
        .first()
        .ok_or("source has no region")?
        .origin;
    let mut host = WorldRuntime::new(Arc::new(source), config())?;
    load_footprint(&mut host, center)?;
    let baseline = witness(&host);
    if baseline.len() < 2 {
        return Err(
            "replication acceptance requires a changed and an untouched resident chunk".into(),
        );
    }
    let edited_voxel = legal_edit(&mut host, center)?;
    let delta = host
        .transaction_delta("loopback-one-column")?
        .ok_or("successful host edit lacks a transaction delta")?;
    validate_scope(&delta)?;
    if !matches!(host.voxel(edited_voxel), QueryResult::Ready(None)) {
        return Err("host edit did not expose the chosen exact voxel as air".into());
    }
    let expected = witness(&host);
    let changed = delta.chunks.first().ok_or("missing changed partition")?;
    validate_products(&baseline, &expected, changed.coordinate)?;
    host.save(work.join("host"), IoLimits::default())?;
    let delta_bytes = encode(&delta, FRAME_LIMIT)?;
    let request = Request {
        protocol: PROTOCOL,
        center,
        radius: RADIUS,
        edited_voxel,
        delta,
        baseline,
        expected,
        replay: false,
    };
    let first = session(&package, &work, &world_id, fingerprint, &request)?;
    let mut request = request;
    request.replay = true;
    let replay = session(&package, &work, &world_id, fingerprint, &request)?;
    if first.process_id == replay.process_id
        || replay.ack.before != first.ack.after
        || replay.ack.after != first.ack.after
        || replay.ack.save_head_before != Some(first.ack.save_head_after)
        || replay.ack.save_head_after != first.ack.save_head_after
        || !replay.ack.restored_delta_identical
    {
        return Err("restarted receiver failed exact durable replay/idempotency witnesses".into());
    }
    let changed = request
        .delta
        .chunks
        .first()
        .ok_or("missing final partition")?;
    let untouched_products = request
        .baseline
        .iter()
        .filter(|product| product.coordinate != changed.coordinate)
        .cloned()
        .collect();
    let receipt = Receipt {
        kind: "v4-two-process-loopback-replication/1", accepted: true,
        scope: "Loopback transport acceptance only; not production authentication, entity replication, or online V4 gameplay.",
        world_id, package_fingerprint: format!("{fingerprint:016x}"), source_path: package,
        started_unix_milliseconds, work_directory: work,
        delta_fingerprint: format!("{:016x}", request.delta.fingerprint),
        delta_payload_bytes: delta_bytes.len(), wire_chunks: request.delta.chunks.len(),
        wire_columns: request.delta.chunks.iter().map(|chunk| chunk.columns.len()).sum(),
        edited_voxel, base_revision: changed.base_revision, final_revision: changed.revision,
        untouched_products,
        unchanged_products_are: "Exact canonical resident chunk products, including terrain, features and object/light projections; these are engine publication inputs, not live ECS entity IDs.",
        first_delivery: first, restarted_replay: replay,
    };
    let bytes = serde_json::to_vec_pretty(&receipt)?;
    atomicwrites::AtomicFile::new(output, atomicwrites::DisallowOverwrite).write(|file| {
        file.write_all(&bytes)?;
        file.sync_all()
    })?;
    #[cfg(unix)]
    File::open(parent)?.sync_all()?;
    Ok(format!(
        "Accepted two-process durable loopback replay: 1 chunk / 1 column, receipt {}",
        output.display()
    ))
}

/// Hidden child entry point. The private session token arrives over the parent's stdin pipe.
pub(super) fn worker(package: &Path, save: &Path, connect: &str) -> Result<String> {
    let address: SocketAddr = connect.parse()?;
    if address.ip() != Ipv4Addr::LOCALHOST || address.port() == 0 {
        return Err("replica-worker accepts only an explicit IPv4 loopback address".into());
    }
    let session_bytes = read_frame(&mut io::stdin().lock(), 128)?;
    let session = String::from_utf8(session_bytes)?;
    if session.len() != 64 || !session.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid private worker session token".into());
    }
    let source = FileChunkSource::open_workspace(package, IoLimits::default())?;
    let mut stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    configure(&stream)?;
    write_network_frame(
        &mut stream,
        &encode(
            &Hello {
                protocol: PROTOCOL,
                session,
                process_id: std::process::id(),
                world_id: source.manifest().world_id.clone(),
                manifest_fingerprint: source.manifest().fingerprint,
            },
            CONTROL_LIMIT,
        )?,
    )?;
    let request: Request = decode(&read_network_frame(&mut stream, FRAME_LIMIT)?)?;
    if request.protocol != PROTOCOL
        || request.radius != RADIUS
        || request.baseline.len() < 2
        || request.baseline.len() > MAX_CHUNKS
        || request.expected.len() != request.baseline.len()
        || request.delta.world_id != source.manifest().world_id
        || request.delta.manifest_fingerprint != source.manifest().fingerprint
        || request
            .center
            .checked_distance(request.edited_voxel.column)?
            > u64::from(RADIUS)
    {
        return Err("replica request is outside the exact bounded harness authority".into());
    }
    validate_scope(&request.delta)?;
    let delta_chunk = request.delta.chunks.first().ok_or("missing partition")?;
    let delta_column = delta_chunk.columns.first().ok_or("missing column")?;
    if delta_column.position != request.edited_voxel.column {
        return Err("replica request delta differs from its authorized column".into());
    }
    let mut runtime = WorldRuntime::new(Arc::new(source), config())?;
    let head_before = if request.replay {
        let hash = head_hash(save)?;
        runtime.restore_save(save, IoLimits::default())?;
        Some(hash)
    } else {
        if save.try_exists()? {
            return Err("first receiver save must be fresh".into());
        }
        None
    };
    load_footprint(&mut runtime, request.center)?;
    let before = witness(&runtime);
    if before
        != if request.replay {
            &request.expected
        } else {
            &request.baseline
        }
        .clone()
    {
        return Err("receiver baseline differs from the host's exact resident products".into());
    }
    let restored_delta_identical = runtime
        .transaction_delta(&request.delta.transaction_id)?
        .as_ref()
        == Some(&request.delta);
    if restored_delta_identical != request.replay {
        return Err(
            "receiver transaction history disagrees with the requested fresh/replay phase".into(),
        );
    }
    let start = Instant::now();
    let change = runtime.apply_delta_durable(&request.delta, save, IoLimits::default())?;
    let durable_apply_milliseconds = start.elapsed().as_secs_f64() * 1000.0;
    if change.changed_columns != vec![request.edited_voxel.column]
        || change.revisions != BTreeMap::from([(delta_chunk.coordinate, delta_chunk.revision)])
        || !matches!(
            runtime.voxel(request.edited_voxel),
            QueryResult::Ready(None)
        )
    {
        return Err("durable receiver change is empty or differs from the exact request".into());
    }
    let update = runtime.pump();
    if !update.loaded.is_empty() || !update.removed.is_empty() || !update.failures.is_empty() {
        return Err("receiver residency changed during local delta publication".into());
    }
    let published_changes = update
        .changed
        .iter()
        .map(|product| product.coordinate)
        .collect::<Vec<_>>();
    let expected_publications = if request.replay {
        Vec::new()
    } else {
        vec![delta_chunk.coordinate]
    };
    if published_changes != expected_publications {
        return Err("local delta rebuilt an unrelated chunk or replay republished terrain".into());
    }
    let after = witness(&runtime);
    if after != request.expected {
        return Err("receiver products differ after durable application".into());
    }
    let ack = Ack {
        protocol: PROTOCOL,
        process_id: std::process::id(),
        manifest_fingerprint: runtime.manifest().fingerprint,
        delta_fingerprint: request.delta.fingerprint,
        transaction_id: request.delta.transaction_id,
        replay: request.replay,
        before,
        after,
        published_changes,
        restored_delta_identical,
        save_head_before: head_before,
        save_head_after: head_hash(save)?,
        durable_apply_milliseconds,
    };
    // This is deliberately after durable application, head verification, and product witnesses.
    write_network_frame(&mut stream, &encode(&ack, FRAME_LIMIT)?)?;
    if !request.replay {
        // The parent forcibly terminates this process after reading its durable ACK.
        // An orderly runtime Drop cannot supply additional persistence guarantees.
        let mut stop = [0_u8; 1];
        stream.read_exact(&mut stop)?;
        return Err("first receiver should have been terminated after its durable ACK".into());
    }
    Ok("restarted receiver acknowledged an identical durable replay".into())
}

fn session(
    package: &Path,
    work: &Path,
    world_id: &str,
    fingerprint: u64,
    request: &Request,
) -> Result<SessionReceipt> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    listener.set_nonblocking(true)?;
    let token = nonce()?;
    let phase = if request.replay { "replay" } else { "first" };
    let child = Command::new(std::env::current_exe()?)
        .arg("replica-worker")
        .arg("--package")
        .arg(package)
        .arg("--save")
        .arg(work.join("receiver"))
        .arg("--connect")
        .arg(listener.local_addr()?.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::from(File::create(
            work.join(format!("{phase}.stdout.log")),
        )?))
        .stderr(Stdio::from(File::create(
            work.join(format!("{phase}.stderr.log")),
        )?))
        .spawn()?;
    let mut child = ChildLease {
        child,
        finished: false,
    };
    let process_id = child.child.id();
    let mut stdin = child
        .child
        .stdin
        .take()
        .ok_or("worker stdin pipe is unavailable")?;
    write_frame(&mut stdin, token.as_bytes())?;
    drop(stdin);
    let deadline = Instant::now() + TIMEOUT;
    let (mut stream, peer) = loop {
        match listener.accept() {
            Ok(connection) => break connection,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if child.child.try_wait()?.is_some() {
                    return Err(format!(
                        "{phase} receiver exited before handshake; inspect its fresh log"
                    )
                    .into());
                }
                if Instant::now() >= deadline {
                    return Err("receiver loopback connection timed out".into());
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error.into()),
        }
    };
    if peer.ip() != Ipv4Addr::LOCALHOST {
        return Err("unexpected non-loopback receiver".into());
    }
    configure(&stream)?;
    let hello_bytes = read_network_frame(&mut stream, CONTROL_LIMIT)?;
    let hello: Hello = decode(&hello_bytes)?;
    if hello.protocol != PROTOCOL
        || hello.session != token
        || hello.process_id != process_id
        || hello.world_id != world_id
        || hello.manifest_fingerprint != fingerprint
    {
        return Err(
            "receiver package, process, or private session handshake does not match".into(),
        );
    }
    let request_bytes = encode(request, FRAME_LIMIT)?;
    let started = Instant::now();
    write_network_frame(&mut stream, &request_bytes)?;
    let ack_bytes = read_network_frame(&mut stream, FRAME_LIMIT)?;
    let durable_ack_milliseconds = started.elapsed().as_secs_f64() * 1000.0;
    let ack: Ack = decode(&ack_bytes)?;
    if ack.protocol != PROTOCOL
        || ack.process_id != process_id
        || ack.manifest_fingerprint != fingerprint
        || ack.delta_fingerprint != request.delta.fingerprint
        || ack.transaction_id != request.delta.transaction_id
        || ack.replay != request.replay
        || ack.after != request.expected
        || !ack.durable_apply_milliseconds.is_finite()
        || ack.durable_apply_milliseconds < 0.0
    {
        return Err("receiver ACK does not match the exact durable request".into());
    }
    if !request.replay {
        child.child.kill()?;
    }
    let status = child.wait()?;
    if status.success() != request.replay {
        return Err("receiver process did not terminate in the expected acceptance phase".into());
    }
    Ok(SessionReceipt {
        process_id,
        process_status: status.to_string(),
        killed_after_durable_ack: !request.replay,
        process_exit_success: status.success(),
        total_tcp_wire_bytes: hello_bytes.len() + request_bytes.len() + ack_bytes.len() + 12,
        request_frame_bytes: request_bytes.len() + 4,
        durable_ack_milliseconds,
        ack,
    })
}

fn config() -> RuntimeConfig {
    RuntimeConfig {
        max_resident_chunks: MAX_CHUNKS,
        max_in_flight_jobs: 2,
        max_publications_per_pump: 2,
        max_interests: 1,
        max_interest_probes: 64,
        max_edits_per_transaction: 1,
        max_cached_transactions: 0,
        max_cached_transaction_bytes: 0,
        ..RuntimeConfig::default()
    }
}

fn load_footprint(runtime: &mut WorldRuntime, center: WorldHex) -> Result<()> {
    runtime.set_interests(vec![ResidencyRequest {
        id: "loopback-authorized-footprint".into(),
        center,
        radius: RADIUS,
        retention_radius: RADIUS,
        priority: 1,
    }])?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let update = runtime.pump();
        if let Some(failure) = update.failures.first() {
            return Err(format!("bounded replica source load failed: {}", failure.error).into());
        }
        let counts = runtime.counts();
        if counts.in_flight_jobs == 0 && counts.queued_chunks == 0 {
            if counts.resident_chunks == 0 || counts.resident_chunks > MAX_CHUNKS {
                return Err(
                    "bounded replica footprint performed zero or excessive residency work".into(),
                );
            }
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err("bounded replica source load timed out".into());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn legal_edit(runtime: &mut WorldRuntime, center: WorldHex) -> Result<VoxelPosition> {
    let mut candidates = Vec::new();
    for product in runtime.resident_chunks() {
        for column in &product.package.columns {
            if center.checked_distance(column.position)? > u64::from(RADIUS) {
                continue;
            }
            for run in &column.runs {
                let material = runtime
                    .manifest()
                    .materials
                    .iter()
                    .find(|material| material.id == run.material)
                    .ok_or("unknown edit material")?;
                if !material.solid || !material.diggable {
                    continue;
                }
                let position = VoxelPosition {
                    column: column.position,
                    level: run.bottom,
                };
                if product
                    .package
                    .semantics
                    .occupancy
                    .iter()
                    .filter(|occupancy| occupancy.position == position.column)
                    .flat_map(|occupancy| &occupancy.runs)
                    .any(|run| run.bottom <= position.level && position.level < run.top)
                {
                    continue;
                }
                candidates.push(position);
                if candidates.len() == MAX_CANDIDATES {
                    break;
                }
            }
            if candidates.len() == MAX_CANDIDATES {
                break;
            }
        }
        if candidates.len() == MAX_CANDIDATES {
            break;
        }
    }
    for position in candidates {
        let revision = runtime
            .revision(position.column.chunk())
            .ok_or("edit candidate unloaded")?;
        let edit = WorldEditTransaction {
            id: "loopback-one-column".into(),
            expected_revisions: BTreeMap::from([(position.column.chunk(), revision)]),
            edits: vec![VoxelEdit {
                position,
                material: None,
            }],
        };
        if runtime.apply_transaction(&edit).is_ok() {
            return Ok(position);
        }
    }
    Err("no legal one-column terrain edit in the bounded resident candidate set".into())
}

fn witness(runtime: &WorldRuntime) -> Vec<ProductWitness> {
    runtime
        .resident_chunks()
        .map(|product| ProductWitness {
            coordinate: product.coordinate,
            revision: product.revision,
            fingerprint: product.package.fingerprint,
            columns: product.package.columns.len(),
            terrain_runs: product
                .package
                .columns
                .iter()
                .map(|column| column.runs.len())
                .sum(),
            object_projection_runs: product
                .package
                .semantics
                .occupancy
                .iter()
                .map(|column| column.runs.len())
                .sum(),
            root_objects: product.package.semantics.objects.len(),
        })
        .collect()
}

fn validate_scope(delta: &WorldDelta) -> Result<()> {
    delta.validate()?;
    let chunk = delta.chunks.first().ok_or("empty delta")?;
    if delta.chunks.len() != 1
        || chunk.columns.len() != 1
        || chunk.base_revision != 0
        || chunk.revision != 1
        || chunk.base_fingerprint == chunk.target_fingerprint
    {
        return Err(
            "acceptance requires one nonempty, first-revision, changed-column delta".into(),
        );
    }
    Ok(())
}

fn validate_products(
    before: &[ProductWitness],
    after: &[ProductWitness],
    changed: ChunkId,
) -> Result<()> {
    if before.len() != after.len() || before.len() < 2 {
        return Err("resident product set changed during a local edit".into());
    }
    let mut touched = 0;
    for (before, after) in before.iter().zip(after) {
        if before.coordinate != after.coordinate {
            return Err("local edit replaced resident chunk identities".into());
        }
        if before.coordinate == changed {
            if before.fingerprint == after.fingerprint
                || before.revision.checked_add(1) != Some(after.revision)
            {
                return Err("accepted edit performed no exact partition revision work".into());
            }
            touched += 1;
        } else if before != after {
            return Err("local edit changed an unrelated resident engine product".into());
        }
    }
    if touched != 1 {
        return Err("expected exactly one revised resident product".into());
    }
    Ok(())
}

fn head_hash(save: &Path) -> Result<u64> {
    let maximum = IoLimits::default().max_manifest_bytes;
    let mut bytes = Vec::new();
    File::open(save.join("current.ron"))?
        .take(u64::try_from(maximum)? + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > maximum {
        return Err("invalid bounded receiver save head".into());
    }
    Ok(hash_serializable(&bytes)?)
}

fn configure(stream: &TcpStream) -> Result<()> {
    // Accepted sockets inherit the listener's nonblocking flag on some Unix hosts.
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    stream.set_nodelay(true)?;
    Ok(())
}

fn encode(value: &impl Serialize, maximum: usize) -> Result<Vec<u8>> {
    let bytes = ron::ser::to_string(value)?.into_bytes();
    if bytes.is_empty() || bytes.len() > maximum {
        return Err("outbound frame exceeds its operation bound".into());
    }
    Ok(bytes)
}
fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    Ok(ron::de::from_bytes(bytes)?)
}
fn write_frame(writer: &mut impl Write, bytes: &[u8]) -> Result<()> {
    writer.write_all(&u32::try_from(bytes.len())?.to_be_bytes())?;
    writer.write_all(bytes)?;
    writer.flush()?;
    Ok(())
}
fn read_frame(reader: &mut impl Read, maximum: usize) -> Result<Vec<u8>> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))?;
    if length == 0 || length > maximum {
        return Err("inbound frame length exceeds its operation bound".into());
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn read_network_frame(stream: &mut TcpStream, maximum: usize) -> Result<Vec<u8>> {
    let deadline = Instant::now() + TIMEOUT;
    let mut length = [0_u8; 4];
    read_network_exact(stream, &mut length, deadline)?;
    let length = usize::try_from(u32::from_be_bytes(length))?;
    if length == 0 || length > maximum {
        return Err("inbound network frame length exceeds its operation bound".into());
    }
    let mut bytes = vec![0; length];
    read_network_exact(stream, &mut bytes, deadline)?;
    Ok(bytes)
}

fn read_network_exact(stream: &mut TcpStream, bytes: &mut [u8], deadline: Instant) -> Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let target = bytes
            .get_mut(offset..)
            .ok_or("invalid bounded network read offset")?;
        let count = stream.read(target)?;
        if count == 0 {
            return Err("receiver closed a partial frame".into());
        }
        offset += count;
    }
    Ok(())
}

fn write_network_frame(stream: &mut TcpStream, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() || bytes.len() > FRAME_LIMIT {
        return Err("outbound network frame exceeds its operation bound".into());
    }
    let deadline = Instant::now() + TIMEOUT;
    let length = u32::try_from(bytes.len())?.to_be_bytes();
    for part in [length.as_slice(), bytes] {
        let mut offset = 0;
        while offset < part.len() {
            stream.set_write_timeout(Some(remaining(deadline)?))?;
            let count = stream.write(
                part.get(offset..)
                    .ok_or("invalid bounded network write offset")?,
            )?;
            if count == 0 {
                return Err("receiver stopped accepting a frame".into());
            }
            offset += count;
        }
    }
    Ok(())
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or_else(|| "absolute loopback frame deadline exceeded".into())
}

fn nonce() -> Result<String> {
    // OS randomness keeps the private pipe token unguessable without another crate dependency.
    // This diagnostic currently targets the Unix hosts supported by this project's CLI workflow.
    let mut bytes = [0_u8; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

struct ChildLease {
    child: Child,
    finished: bool,
}
impl ChildLease {
    fn wait(&mut self) -> Result<ExitStatus> {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait()? {
                self.finished = true;
                return Ok(status);
            }
            if Instant::now() >= deadline {
                return Err("receiver process exit timed out".into());
            }
            thread::sleep(Duration::from_millis(2));
        }
    }
}
impl Drop for ChildLease {
    fn drop(&mut self) {
        if !self.finished {
            let _kill = self.child.kill();
            let _wait = self.child.wait();
        }
    }
}
struct OutputLease {
    path: PathBuf,
    _file: File,
}
impl OutputLease {
    fn new(path: PathBuf) -> Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        Ok(Self { path, _file: file })
    }
}
impl Drop for OutputLease {
    fn drop(&mut self) {
        let _removed = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_rejects_zero_oversized_and_truncated_payloads_before_allocation() {
        assert!(read_frame(&mut 0_u32.to_be_bytes().as_slice(), 8).is_err());
        assert!(read_frame(&mut u32::MAX.to_be_bytes().as_slice(), 8).is_err());
        assert!(read_frame(&mut [0, 0, 0, 3, 1, 2].as_slice(), 8).is_err());
        let mut bytes = Vec::new();
        assert!(write_frame(&mut bytes, b"abc").is_ok());
        assert_eq!(
            read_frame(&mut bytes.as_slice(), 8).ok(),
            Some(b"abc".to_vec())
        );
    }

    #[test]
    fn accepted_nonblocking_listener_reads_a_delayed_complete_frame() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("loopback listener");
        listener
            .set_nonblocking(true)
            .expect("nonblocking accept loop");
        let mut client = TcpStream::connect(listener.local_addr().expect("listener address"))
            .expect("local client");
        // A completed client connect does not guarantee immediate nonblocking
        // accept readiness on every host. Keep the listener nonblocking while
        // allowing the kernel to publish its queued connection.
        let deadline = Instant::now() + Duration::from_secs(2);
        let (mut accepted, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "local accept deadline");
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("local accept failed: {error}"),
            }
        };
        configure(&accepted).expect("portable accepted-stream mode");
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(20));
            write_frame(&mut client, b"complete").expect("delayed frame");
        });
        assert_eq!(
            read_network_frame(&mut accepted, 8).expect("complete blocking read"),
            b"complete"
        );
        writer.join().expect("completed local sender");
    }

    #[test]
    fn zero_work_and_corrupt_source_fail_without_an_accepted_receipt() {
        use hex_world_contracts::{
            ChunkDescriptor, ChunkPackage, ChunkSemantics, ColumnData, MaterialSpec,
            RegionDescriptor, VoxelRun, WorldManifest, WorldPackage, SCHEMA_VERSION,
        };
        let root = std::env::temp_dir().join(format!(
            "v4-replica-zero-{}",
            nonce().expect("test identity")
        ));
        fs::create_dir(&root).expect("fresh fixture root");
        let mut chunks: BTreeMap<ChunkId, ChunkPackage> = BTreeMap::new();
        for (q, r) in [(0, 0), (1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)] {
            let column = WorldHex::new(q, r);
            chunks
                .entry(column.chunk())
                .or_insert_with(|| ChunkPackage {
                    schema_version: SCHEMA_VERSION,
                    world_id: "immutable-fixture".into(),
                    coordinate: column.chunk(),
                    source_fingerprint: 1,
                    columns: Vec::new(),
                    features: Vec::new(),
                    semantics: ChunkSemantics::default(),
                    fingerprint: 0,
                })
                .columns
                .push(ColumnData {
                    position: column,
                    runs: vec![VoxelRun {
                        bottom: 0,
                        top: 1,
                        material: "bedrock".into(),
                    }],
                });
        }
        let mut package = WorldPackage {
            manifest: WorldManifest {
                schema_version: SCHEMA_VERSION,
                world_id: "immutable-fixture".into(),
                compiler_version: "replication-test".into(),
                source_fingerprint: 1,
                materials: vec![MaterialSpec {
                    id: "bedrock".into(),
                    solid: true,
                    diggable: false,
                    color: [0; 4],
                }],
                regions: vec![RegionDescriptor {
                    id: "region".into(),
                    origin: WorldHex::new(0, 0),
                    radius: 1,
                    source_fingerprint: 1,
                }],
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
        package.seal().expect("strict immutable fixture");
        let source = root.join("source");
        hex_world_runtime::publish_package(&source, &package, IoLimits::default())
            .expect("publish fixture");
        let output = root.join("accepted.json");
        let error =
            run(&source, &output).expect_err("indestructible world cannot perform acceptance work");
        assert!(error.to_string().contains("no legal one-column"));
        assert!(!output.exists());
        fs::write(source.join("chunks/0_0.ron"), b"corrupt").expect("corrupt owned fixture chunk");
        assert!(run(&source, &output).is_err());
        assert!(!output.exists());
        fs::remove_dir_all(root).expect("owned fixture cleanup");
    }

    #[test]
    fn worker_rejects_non_loopback_before_reading_stdin_or_source() {
        assert!(worker(Path::new("absent"), Path::new("absent"), "192.0.2.1:9000").is_err());
        assert!(worker(Path::new("absent"), Path::new("absent"), "127.0.0.1:0").is_err());
    }

    #[test]
    fn receipt_refuses_stale_output_without_touching_existing_bytes() {
        let root = std::env::temp_dir().join(format!("v4-replica-stale-{}", std::process::id()));
        fs::create_dir(&root).expect("fresh test directory");
        let output = root.join("accepted.json");
        fs::write(&output, b"original").expect("stale receipt fixture");
        assert!(run(Path::new("absent"), &output).is_err());
        assert_eq!(fs::read(&output).expect("preserved receipt"), b"original");
        fs::remove_dir_all(root).expect("owned test cleanup");
    }
}
