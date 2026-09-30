//! Actual-package, windowless residency acceptance. No timing or visual claims.
use super::*;
use hex_core::HexCoord;
use hex_map::v4::ResidentChunk;
use hex_world_contracts::{ChunkId, WorldHex};
use std::{collections::BTreeSet, io::Write};

const CIRCUITS: u32 = 3;
const SOURCE_LIMIT: usize = 512;
const DETAIL_LIMIT: usize = 256;
const JOB_LIMIT: usize = 2;
const TOTAL_LIMIT: Duration = Duration::from_secs(360);
const STOP_LIMIT: Duration = Duration::from_secs(30);

#[derive(Default, Serialize)]
struct Highwater {
    observed_frames: usize,
    resident_chunks: usize,
    finite_sources: usize,
    detailed_chunks: usize,
    in_flight_jobs: usize,
    queued_chunks: usize,
    mesh_assets: usize,
}

#[derive(Serialize)]
struct Sample {
    circuit: u32,
    site: String,
    center: ChunkId,
    resident_chunks: usize,
    finite_sources: usize,
    detailed_chunks: usize,
    in_flight_jobs: usize,
    queued_chunks: usize,
    mesh_assets: usize,
}

struct Observed {
    sample: Sample,
    detail: BTreeSet<ChunkId>,
}

fn chunk(position: Vec3) -> ChunkId {
    let coord = HexCoord::from_world(position);
    WorldHex::new(i64::from(coord.x()), i64::from(coord.y())).chunk()
}

fn detailed_revision(app: &mut App, coordinate: ChunkId) -> Option<u64> {
    let mut roots = app.world_mut().query::<&ResidentChunk>();
    roots
        .iter(app.world())
        .find(|root| root.coordinate == coordinate)
        .map(|root| root.revision)
}

fn observe(app: &mut App, highwater: &mut Highwater, circuit: u32, site: &str) -> Observed {
    let state = app.world().resource::<StreamedArena>();
    assert!(state.failure.is_none(), "{site}: {:?}", state.failure);
    let counts = state.runtime.counts();
    let finite_sources = state.edits.resident_source_count();
    assert!(
        counts.resident_chunks <= SOURCE_LIMIT,
        "source budget at {site}"
    );
    assert!(
        state.peak_resident <= SOURCE_LIMIT,
        "source highwater at {site}"
    );
    assert!(
        counts.in_flight_jobs <= JOB_LIMIT,
        "source worker budget at {site}"
    );
    assert_eq!(
        finite_sources, counts.resident_chunks,
        "retired fine sources retained at {site}"
    );
    let center = chunk(app.world().resource::<ArenaStreamInterest>().position);
    let mut roots = app.world_mut().query::<&ResidentChunk>();
    let published: Vec<_> = roots
        .iter(app.world())
        .map(|root| root.coordinate)
        .collect();
    let detail: BTreeSet<_> = published.iter().copied().collect();
    assert_eq!(
        published.len(),
        detail.len(),
        "duplicate detailed roots at {site}"
    );
    assert!(detail.len() <= DETAIL_LIMIT, "detail budget at {site}");
    let mesh_assets = app.world().resource::<Assets<Mesh>>().len();
    let sample = Sample {
        circuit,
        site: site.into(),
        center,
        resident_chunks: counts.resident_chunks,
        finite_sources,
        detailed_chunks: detail.len(),
        in_flight_jobs: counts.in_flight_jobs,
        queued_chunks: counts.queued_chunks,
        mesh_assets,
    };
    highwater.observed_frames += 1;
    highwater.resident_chunks = highwater.resident_chunks.max(sample.resident_chunks);
    highwater.finite_sources = highwater.finite_sources.max(sample.finite_sources);
    highwater.detailed_chunks = highwater.detailed_chunks.max(sample.detailed_chunks);
    highwater.in_flight_jobs = highwater.in_flight_jobs.max(sample.in_flight_jobs);
    highwater.queued_chunks = highwater.queued_chunks.max(sample.queued_chunks);
    highwater.mesh_assets = highwater.mesh_assets.max(sample.mesh_assets);
    Observed { sample, detail }
}

#[expect(
    clippy::expect_used,
    reason = "A circuit stop requires the production human and finite residency authority before its readiness checks can mean anything."
)]
fn visit(
    app: &mut App,
    highwater: &mut Highwater,
    circuit: u32,
    site: &str,
    position: Vec3,
    total_deadline: Instant,
) -> Observed {
    // This acceptance deliberately relocates a paused real player interest. It
    // measures world/presentation residency, not controls, AI or travel speed.
    app.world_mut().resource_mut::<ViewState>().pause();
    {
        let mut session = app.world_mut().resource_mut::<ArenaSession>();
        let player = session
            .actors
            .iter_mut()
            .find(|actor| actor.id == 0)
            .expect("human");
        player.feet = position;
        player.previous_feet = position;
        player.grounded = false;
    }
    *app.world_mut().resource_mut::<ArenaStreamInterest>() = ArenaStreamInterest {
        position,
        velocity: Vec3::ZERO,
    };
    let center = chunk(position);
    let area = HexCoord::from_world(position).within_radius(8);
    let geometry = *app.world().resource::<ArenaVoxelGeometry>();
    let stop_deadline = (Instant::now() + STOP_LIMIT).min(total_deadline);
    loop {
        app.update();
        let observed = observe(app, highwater, circuit, site);
        let terrain = app.world().resource::<ArenaTerrainView>();
        let residency = terrain.residency.as_ref().expect("finite residency");
        let ready = area
            .iter()
            .all(|coord| residency.at(*coord, geometry) == ArenaAvailability::Ready);
        if ready && observed.detail.contains(&center) {
            return observed;
        }
        assert!(Instant::now() < stop_deadline,
            "circuit {circuit} {site} timed out: collision_ready={ready}, detailed_center={}, source={}, detail={}, queued={}",
            observed.detail.contains(&center), observed.sample.resident_chunks,
            observed.sample.detailed_chunks, observed.sample.queued_chunks);
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[expect(
    clippy::expect_used,
    reason = "The eviction fixture requires an actual nearby solid and an acknowledged world edit rather than an invented empty cell."
)]
fn carve_beach(app: &mut App, beach: Vec3) -> (TilePos, Option<u64>) {
    let origin = HexCoord::from_world(beach);
    let materials = *app.world().resource::<ArenaMaterials>();
    let terrain = app.world().resource::<ArenaTerrainView>();
    let position = terrain
        .columns
        .iter()
        .filter(|(coord, _)| (3..=6).contains(&origin.distance(**coord)))
        .flat_map(|(_, spans)| spans)
        .find_map(|span| {
            (span.substance != materials.bedrock && span.top_level - span.bottom.level >= 2)
                .then_some(TilePos::new(span.bottom.coord, span.top_level - 1))
        })
        .expect("authored beach has a nearby destructible solid run");
    assert!(terrain.solid_at(position).is_some());
    let coordinate =
        WorldHex::new(i64::from(position.coord.x()), i64::from(position.coord.y())).chunk();
    let detail_before = detailed_revision(app, coordinate);
    app.world_mut()
        .write_message(TerrainEdit::Clear { pos: position });
    settle(app.world_mut()).expect("actual world edit boundary");
    assert!(app
        .world()
        .resource::<ArenaTerrainView>()
        .solid_at(position)
        .is_none());
    (position, detail_before)
}

#[expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "The isolated runner requires each machine-readable progress sample in its file and captured stdout without installing the game's windowed logging plugins."
)]
fn record(log: &mut std::fs::File, sample: &Sample) {
    let line = serde_json::to_string(sample).expect("sample");
    writeln!(log, "{line}").expect("sample log");
    log.flush().expect("durable sample log");
    println!("GRAND_CIRCUIT_SAMPLE {line}");
}

#[test]
#[ignore = "requires explicit actual Grand package and isolated HEX_GAME_DATA_DIR with grand-verification-only marker"]
fn actual_grand_streaming_circuit() {
    let data =
        PathBuf::from(std::env::var_os("HEX_GAME_DATA_DIR").expect("isolated data directory"));
    assert!(
        data.join("grand-verification-only").is_file(),
        "refuse real user data"
    );
    let package =
        PathBuf::from(std::env::var_os("HEX_GRAND_WORLD").expect("explicit actual package"));
    assert!(
        !data.join("circuit.json").exists(),
        "use fresh acceptance output"
    );
    let started = Instant::now();
    let total_deadline = started + TOTAL_LIMIT;
    let mut app = fixture();
    // Publish real CPU-prepared terrain roots without a window or GPU renderer.
    app.init_resource::<Assets<Mesh>>()
        .init_resource::<Assets<StandardMaterial>>();
    let beach = app
        .world()
        .resource::<ArenaTerrainView>()
        .spawns
        .first()
        .copied()
        .expect("authored player spawn");
    let anchors = app.world().resource::<ArenaTerrainView>().anchors.clone();
    let route: Vec<_> = [
        "forest",
        "world_tree",
        "garden",
        "library_entrance",
        "library_hall",
        "shadow_tunnel",
        "crystal_ascent",
        "shrine_air",
        "volcano",
    ]
    .into_iter()
    .map(|name| {
        (
            name,
            anchors.get(name).copied().expect("authored circuit anchor"),
        )
    })
    .collect();
    let mut highwater = Highwater::default();
    let mut log = std::fs::File::create_new(data.join("circuit.jsonl")).expect("fresh sample log");
    let initial = visit(
        &mut app,
        &mut highwater,
        0,
        "starting_beach",
        beach,
        total_deadline,
    );
    record(&mut log, &initial.sample);
    let (carved, detail_before) = carve_beach(&mut app, beach);
    let carved_column = WorldHex::new(i64::from(carved.coord.x()), i64::from(carved.coord.y()));
    let carved_chunk = carved_column.chunk();
    // A detailed root's revision is a renderer publication counter, not the
    // finite edit authority revision. Wait for a new publication relative to
    // the pre-edit root (or its first publication). The renderer rejects stale
    // authority completions before publishing, so this establishes that the
    // edited chunk was presented before later absence counts as eviction.
    let edit_deadline = (Instant::now() + STOP_LIMIT).min(total_deadline);
    let detail_after = loop {
        app.update();
        observe(&mut app, &mut highwater, 0, "edited_beach");
        if let Some(revision) = detailed_revision(&mut app, carved_chunk)
            .filter(|revision| Some(*revision) != detail_before)
        {
            break revision;
        }
        assert!(
            Instant::now() < edit_deadline,
            "edited beach detail was never published"
        );
        std::thread::sleep(Duration::from_millis(2));
    };
    let baseline = world_records(app.world());
    let mut samples = vec![initial.sample];
    let mut evictions = 0;
    let mut revisits = 0;
    for circuit in 1..=CIRCUITS {
        for &(name, position) in &route {
            let mut observed = visit(
                &mut app,
                &mut highwater,
                circuit,
                name,
                position,
                total_deadline,
            );
            if name == "volcano" {
                let deadline = (Instant::now() + STOP_LIMIT).min(total_deadline);
                loop {
                    let state = app.world().resource::<StreamedArena>();
                    if state.runtime.resident_chunk(carved_chunk).is_none()
                        && state.edits.terrain_column(carved_column).is_none()
                        && !observed.detail.contains(&carved_chunk)
                    {
                        break;
                    }
                    assert!(
                        Instant::now() < deadline,
                        "old beach authority/detail did not evict at volcano"
                    );
                    app.update();
                    observed = observe(&mut app, &mut highwater, circuit, name);
                    std::thread::sleep(Duration::from_millis(2));
                }
                assert_eq!(
                    world_records(app.world()),
                    baseline,
                    "unloaded sparse edits were lost"
                );
                evictions += 1;
            }
            record(&mut log, &observed.sample);
            samples.push(observed.sample);
        }
        let observed = visit(
            &mut app,
            &mut highwater,
            circuit,
            "return_beach",
            beach,
            total_deadline,
        );
        let terrain = app.world().resource::<ArenaTerrainView>();
        let geometry = *app.world().resource::<ArenaVoxelGeometry>();
        assert_eq!(
            terrain
                .residency
                .as_ref()
                .expect("residency")
                .at(carved.coord, geometry),
            ArenaAvailability::Ready
        );
        assert!(
            terrain.solid_at(carved).is_none(),
            "carve reappeared after returning to ready terrain"
        );
        assert!(app
            .world()
            .resource::<StreamedArena>()
            .runtime
            .resident_chunk(carved_chunk)
            .is_some());
        assert_eq!(
            world_records(app.world()),
            baseline,
            "revisit changed sparse edit truth"
        );
        revisits += 1;
        record(&mut log, &observed.sample);
        samples.push(observed.sample);
    }
    assert_eq!(evictions, CIRCUITS);
    assert_eq!(revisits, CIRCUITS);
    assert!(
        highwater.detailed_chunks > 0,
        "real terrain presenter must run"
    );
    let state = app.world().resource::<StreamedArena>();
    let receipt = serde_json::json!({
        "kind": "grand-streaming-circuit-v1", "status": "PASS",
        "scope": "Actual package; paused player-interest relocations with CPU terrain presentation. Typed residency, eviction and sparse-edit evidence only. No pixels, AI, travel-speed, native feel, FPS or process-memory claims.",
        "package": package, "manifest_fingerprint": state.runtime.manifest().fingerprint,
        "source_fingerprint": state.runtime.manifest().source_fingerprint,
        "circuits": CIRCUITS, "completed_stops": samples.len(),
        "stop_readiness": "217-column local collision disk and published detailed center chunk",
        "edited_detail_published_before_eviction": true,
        "edited_detail_revision_before": detail_before,
        "edited_detail_revision_after": detail_after,
        "carved_cell": carved, "old_area_evictions": evictions, "ready_revisits": revisits,
        "limits": { "source_chunks": SOURCE_LIMIT, "finite_sources": SOURCE_LIMIT,
            "detailed_chunks": DETAIL_LIMIT, "source_jobs": JOB_LIMIT },
        "highwater": highwater, "runtime_peak_resident": state.peak_resident,
        "samples": samples,
    });
    std::fs::write(
        data.join("circuit.json"),
        serde_json::to_vec_pretty(&receipt).expect("receipt"),
    )
    .expect("write acceptance receipt");
    println!("GRAND_CIRCUIT_PASS {}", data.join("circuit.json").display());
}
