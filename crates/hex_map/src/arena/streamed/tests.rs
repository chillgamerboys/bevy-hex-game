//! Opt-in actual-package authority benchmark. No renderer, camera, or actor simulation.
use super::*;
use hex_core::arena::ArenaMap;
use serde::Serialize;
use std::io::Write;

#[derive(Default, Serialize)]
struct Peaks {
    residents: usize,
    source_packages: usize,
    source_arc_references: usize,
    workers: usize,
    queued: usize,
    published_columns: usize,
    solid_runs: usize,
    object_runs: usize,
    liquid_runs: usize,
    dense_voxels: usize,
}
#[derive(Serialize)]
struct Lap {
    number: usize,
    simulated_steps: usize,
    waiting_pumps: usize,
    publication_cpu_p95_ms: f64,
    publication_cpu_max_ms: f64,
    active_pumps: usize,
    active_publication_cpu_p95_ms: f64,
    parked_residents: usize,
    parked_sources: usize,
    parked_columns: usize,
    parked_solid_runs: usize,
    parked_process_rss: ProcessRss,
}
#[derive(Serialize)]
struct Receipt {
    kind: &'static str,
    package_fingerprint: u64,
    adapter_source_fingerprint: u64,
    simulated_speed: f32,
    simulated_step_seconds: f32,
    initial_parked_process_rss: ProcessRss,
    laps: Vec<Lap>,
    peaks: Peaks,
    revisited_carve_after_retirement: bool,
    reset_restored_carve: bool,
    duel_and_fort_switches: bool,
    parked_sources_growth_last_lap: isize,
    rss_warmup_growth_kib: Option<i64>,
    rss_post_warmup_growth_kib: Option<i64>,
    rss_last_lap_growth_kib: Option<i64>,
    cpu_target_under_two_ms: bool,
    scope: &'static str,
}

#[derive(Serialize)]
struct ProcessRss {
    kib: Option<u64>,
    unavailable_reason: Option<String>,
}
impl ProcessRss {
    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            kib: None,
            unavailable_reason: Some(reason.into()),
        }
    }
}
#[cfg(target_os = "macos")]
fn process_rss() -> ProcessRss {
    // The child's ps call reads this test process. Collection happens only at
    // parked lap boundaries, outside the production publication CPU interval.
    let output = match std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
    {
        Ok(output) => output,
        Err(error) => return ProcessRss::unavailable(format!("ps launch failed: {error}")),
    };
    if !output.status.success() {
        return ProcessRss::unavailable(format!("ps exited with {}", output.status));
    }
    match String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u64>()
    {
        Ok(kib) => ProcessRss {
            kib: Some(kib),
            unavailable_reason: None,
        },
        Err(error) => ProcessRss::unavailable(format!("ps RSS was not integer KiB: {error}")),
    }
}
#[cfg(not(target_os = "macos"))]
fn process_rss() -> ProcessRss {
    ProcessRss::unavailable("process RSS collection is implemented only for macOS")
}
fn rss_growth(later: &ProcessRss, earlier: &ProcessRss) -> Option<i64> {
    i64::try_from(later.kib?)
        .ok()?
        .checked_sub(i64::try_from(earlier.kib?).ok()?)
}
#[derive(Default)]
struct Measurements {
    samples: Vec<f64>,
    active_samples: Vec<f64>,
    peaks: Peaks,
    waiting_pumps: usize,
    saw_carve_retired: bool,
}
impl Measurements {
    fn pump(&mut self, world: &mut World, carved: TilePos) {
        let previous_interest = world.resource::<StreamedArena>().interest_key;
        let previous_revision = world.resource::<ArenaTerrainView>().revision;
        super::pump(world);
        let state = world.resource::<StreamedArena>();
        assert!(
            state.failure.is_none(),
            "stream admission failed: {:?}",
            state.failure
        );
        let counts = state.runtime.counts();
        let sources = state.edits.resident_source_count();
        assert!(counts.resident_chunks <= 512, "resident cap");
        assert_eq!(
            sources, counts.resident_chunks,
            "overlay retains only resident source packages"
        );
        assert!(counts.in_flight_jobs <= 2, "worker cap");
        assert!(counts.queued_chunks <= 512, "bounded desired queue");
        self.peaks.residents = self.peaks.residents.max(counts.resident_chunks);
        self.peaks.source_packages = self.peaks.source_packages.max(sources);
        self.peaks.workers = self.peaks.workers.max(counts.in_flight_jobs);
        self.peaks.queued = self.peaks.queued.max(counts.queued_chunks);
        self.samples.push(state.publication_ms);
        if self.samples.len().is_multiple_of(60) {
            for product in state.runtime.resident_chunks() {
                self.peaks.source_arc_references = self
                    .peaks
                    .source_arc_references
                    .max(Arc::strong_count(&product.package));
            }
        }
        let view = world.resource::<ArenaTerrainView>();
        if state.interest_key != previous_interest || view.revision != previous_revision {
            self.active_samples.push(state.publication_ms);
        }
        assert!(
            view.voxels.is_empty(),
            "streamed terrain must never expand into a dense voxel map"
        );
        assert!(
            view.columns.len() <= 512 * 256,
            "bounded solid column publication"
        );
        assert!(
            view.object_columns.len() <= 512 * 256,
            "bounded object column publication"
        );
        self.peaks.published_columns = self.peaks.published_columns.max(view.columns.len());
        self.peaks.dense_voxels = self.peaks.dense_voxels.max(view.voxels.len());
        // Cardinality scans stay outside the measured production pump interval.
        if self.samples.len().is_multiple_of(60) {
            self.peaks.solid_runs = self
                .peaks
                .solid_runs
                .max(view.columns.values().map(Vec::len).sum());
            self.peaks.object_runs = self
                .peaks
                .object_runs
                .max(view.object_columns.values().map(Vec::len).sum());
            self.peaks.liquid_runs = self.peaks.liquid_runs.max(view.liquids.len());
        }
        self.saw_carve_retired |= view
            .residency
            .as_ref()
            .expect("residency")
            .at(carved.coord, *world.resource::<ArenaVoxelGeometry>())
            != ArenaAvailability::Ready;
    }
    fn settle(&mut self, world: &mut World, carved: TilePos) {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            self.pump(world, carved);
            let counts = world.resource::<StreamedArena>().runtime.counts();
            if counts.queued_chunks == 0 && counts.in_flight_jobs == 0 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "actual package residency did not settle"
            );
            self.waiting_pumps += 1;
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    fn wait_for_column(&mut self, world: &mut World, column: HexCoord, carved: TilePos) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while world
            .resource::<ArenaTerrainView>()
            .residency
            .as_ref()
            .expect("residency")
            .at(column, *world.resource::<ArenaVoxelGeometry>())
            != ArenaAvailability::Ready
        {
            assert!(
                Instant::now() < deadline,
                "travel dependency did not become ready"
            );
            self.waiting_pumps += 1;
            std::thread::sleep(Duration::from_millis(1));
            self.pump(world, carved);
        }
    }
}
fn percentile95(values: &[f64]) -> f64 {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    values.get(index).copied().unwrap_or(0.0)
}
fn test_world() -> World {
    let mut world = World::new();
    world.insert_resource(ArenaSelection {
        map: ArenaMap::NorthernArchipelago,
        ..default()
    });
    world.init_resource::<ArenaReset>();
    world.init_resource::<ArenaInbox>();
    world.init_resource::<crate::terrain_damage::TerrainDamageState>();
    world.init_resource::<DamagedVoxels>();
    world.init_resource::<Messages<TerrainImpactOutcome>>();
    world.init_resource::<Messages<bevy::app::AppExit>>();
    super::initialize(
        &mut world,
        super::super::load_content().expect("accepted battle catalogs"),
    )
    .expect("actual Northern package initialization");
    world
}

#[test]
#[ignore = "requires HEX_NORTHERN_WORLD pointing to the full-scale package"]
fn actual_northern_menu_selection_commits_before_same_frame_reset() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(super::super::plugin);
    app.update();
    // Match the native menu: change selection in Update, then drive ArenaTick
    // immediately, without another PreUpdate to change the world adapter.
    for map in [
        ArenaMap::NorthernArchipelago,
        ArenaMap::Fort,
        ArenaMap::NorthernArchipelago,
        ArenaMap::Duel,
    ] {
        app.world_mut().resource_mut::<ArenaSelection>().map = map;
        app.world_mut().resource_mut::<ArenaReset>().generation += 1;
        app.world_mut().run_schedule(ArenaTick);
        let world = app.world();
        assert_eq!(world.resource::<ArenaTerrainView>().selection.map, map);
        assert_eq!(
            world.contains_resource::<StreamedArena>(),
            map.capabilities().streamed
        );
        assert_eq!(
            world.contains_resource::<crate::VoxelMap>(),
            !map.capabilities().streamed
        );
        assert!(world.resource::<Messages<AppExit>>().is_empty());
    }
}

fn travel(
    world: &mut World,
    points: &[Vec3],
    measures: &mut Measurements,
    carved: TilePos,
) -> usize {
    let mut steps = 0;
    let mut position = points.first().copied().expect("route start");
    for target in points.iter().skip(1) {
        while position.distance(*target) > 0.01 {
            let direction = (*target - position).normalize_or_zero();
            let next = position + direction * position.distance(*target).min(160.0 / 60.0);
            world.insert_resource(ArenaStreamInterest {
                position,
                velocity: direction * 160.0,
            });
            measures.pump(world, carved);
            // Keep the simulated traveller at its previous safe point until the
            // next column is Ready; never turn an unloaded interval into air.
            measures.wait_for_column(world, HexCoord::from_world(next), carved);
            position = next;
            steps += 1;
        }
    }
    world.insert_resource(ArenaStreamInterest {
        position,
        velocity: Vec3::ZERO,
    });
    measures.settle(world, carved);
    steps
}

#[test]
#[ignore = "requires HEX_NORTHERN_WORLD; explicit actual-package CPU/residency benchmark"]
fn actual_northern_three_circuits_carve_restart_and_map_switch() {
    assert!(
        std::env::var_os("HEX_NORTHERN_WORLD").is_some(),
        "set HEX_NORTHERN_WORLD to the immutable package"
    );
    let mut world = test_world();
    let overview = world.resource::<StreamedArena>().overview.clone();
    let manifest = world.resource::<StreamedArena>().runtime.manifest().clone();
    let mut bad = (*overview).clone();
    bad.package_fingerprint ^= 1;
    assert!(
        validate_overview(&bad, &manifest).is_err(),
        "stale overview rejected"
    );
    bad.package_fingerprint = overview.package_fingerprint;
    bad.bed_heights.pop();
    assert!(
        validate_overview(&bad, &manifest).is_err(),
        "truncated grid rejected"
    );
    let geometry = *world.resource::<ArenaVoxelGeometry>();
    let spawn = Vec3::from_array(overview.player_spawn);
    let spawn_column = HexCoord::from_world(spawn);
    let carved = world
        .resource::<ArenaTerrainView>()
        .columns
        .get(&spawn_column)
        .and_then(|spans| spans.last())
        .map(|span| TilePos::new(spawn_column, span.top_level))
        .expect("supported real bay spawn");
    let original = world
        .resource::<ArenaTerrainView>()
        .solid_at(carved)
        .expect("solid source");
    let mut measures = Measurements::default();
    measures.settle(&mut world, carved);
    let initial_parked_process_rss = process_rss();
    world
        .resource_mut::<ArenaInbox>()
        .edits
        .push(TerrainEdit::Clear { pos: carved });
    super::apply(&mut world);
    assert_eq!(
        world.resource::<ArenaTerrainView>().solid_at(carved),
        None,
        "actual source voxel carved"
    );
    let height = geometry.top(TilePos::new(spawn_column, geometry.max_level - 10));
    let elevated = |key: &str| {
        let mut point =
            Vec3::from_array(*overview.anchors.get(key).expect("published route anchor"));
        point.y = height;
        point
    };
    let start = elevated("party_start");
    let mountains = elevated("old_mountains");
    let settlement = elevated("settlement");
    let reversal = start.lerp(mountains, 0.32);
    // The short out/back segment forces a rapid prefetch-direction reversal each lap.
    let points = [start, reversal, start, mountains, settlement, start];
    let mut laps = Vec::new();
    for number in 1..=3 {
        let sample_start = measures.samples.len();
        let active_start = measures.active_samples.len();
        let waits = measures.waiting_pumps;
        let steps = travel(&mut world, &points, &mut measures, carved);
        let view = world.resource::<ArenaTerrainView>();
        assert_eq!(
            view.solid_at(carved),
            None,
            "carve survives unload/reload on lap{number}"
        );
        let samples = measures.samples.get(sample_start..).expect("lap samples");
        let active_samples = measures
            .active_samples
            .get(active_start..)
            .expect("active lap samples");
        assert!(
            !active_samples.is_empty(),
            "circuit must exercise active pumps"
        );
        laps.push(Lap {
            number,
            simulated_steps: steps,
            waiting_pumps: measures.waiting_pumps - waits,
            publication_cpu_p95_ms: percentile95(samples),
            publication_cpu_max_ms: samples.iter().copied().fold(0.0, f64::max),
            active_pumps: active_samples.len(),
            active_publication_cpu_p95_ms: percentile95(active_samples),
            parked_residents: world
                .resource::<StreamedArena>()
                .runtime
                .counts()
                .resident_chunks,
            parked_sources: world
                .resource::<StreamedArena>()
                .edits
                .resident_source_count(),
            parked_columns: view.columns.len(),
            parked_solid_runs: view.columns.values().map(Vec::len).sum(),
            parked_process_rss: process_rss(),
        });
    }
    assert!(
        measures.saw_carve_retired,
        "route must actually retire the damaged chunk"
    );
    world.resource_mut::<ArenaReset>().generation += 1;
    super::apply(&mut world);
    assert_eq!(
        world.resource::<ArenaTerrainView>().solid_at(carved),
        Some(original),
        "Restart restores original solid"
    );
    assert!(
        world.resource::<DamagedVoxels>().is_empty(),
        "Restart resets partial HP"
    );
    for map in [
        ArenaMap::Duel,
        ArenaMap::NorthernArchipelago,
        ArenaMap::Fort,
        ArenaMap::NorthernArchipelago,
    ] {
        world.resource_mut::<ArenaSelection>().map = map;
        super::super::switch_mode(&mut world);
        assert_eq!(world.resource::<ArenaTerrainView>().selection.map, map);
        assert_eq!(
            world.contains_resource::<StreamedArena>(),
            map == ArenaMap::NorthernArchipelago
        );
        assert_eq!(
            world.contains_resource::<crate::VoxelMap>(),
            map != ArenaMap::NorthernArchipelago
        );
    }
    let growth = match laps.as_slice() {
        [_, previous, last] => {
            isize::try_from(last.parked_sources).expect("bounded count")
                - isize::try_from(previous.parked_sources).expect("bounded count")
        }
        _ => unreachable!("three circuits"),
    };
    let cpu_target = laps
        .iter()
        .all(|lap| lap.publication_cpu_p95_ms < 2.0 && lap.active_publication_cpu_p95_ms < 2.0);
    let (rss_warmup_growth_kib, rss_post_warmup_growth_kib, rss_last_lap_growth_kib) =
        match laps.as_slice() {
            [first, second, last] => (
                rss_growth(&first.parked_process_rss, &initial_parked_process_rss),
                rss_growth(&last.parked_process_rss, &first.parked_process_rss),
                rss_growth(&last.parked_process_rss, &second.parked_process_rss),
            ),
            _ => unreachable!("three circuits"),
        };
    let receipt = Receipt {
        kind: "actual-northern-production-pump",
        package_fingerprint: overview.package_fingerprint,
        adapter_source_fingerprint: hex_world_contracts::hash_serializable(&include_str!("mod.rs"))
            .expect("source identity"),
        simulated_speed: 160.0,
        simulated_step_seconds: 1.0 / 60.0,
        initial_parked_process_rss,
        laps,
        peaks: measures.peaks,
        revisited_carve_after_retirement: measures.saw_carve_retired,
        reset_restored_carve: true,
        duel_and_fort_switches: true,
        parked_sources_growth_last_lap: growth,
        rss_warmup_growth_kib,
        rss_post_warmup_growth_kib,
        rss_last_lap_growth_kib,
        cpu_target_under_two_ms: cpu_target,
        scope: "World authority/pump CPU, bounded cardinalities, and test-process RSS only. RSS includes allocator/test overhead and is reported without an invented pass threshold. No renderer/GPU/FPS, actor flight physics, wave or native-feel claim.",
    };
    let report = ron::ser::to_string_pretty(&receipt, ron::ser::PrettyConfig::default())
        .expect("receipt encoding");
    if let Some(path) = std::env::var_os("HEX_NORTHERN_BENCH_REPORT") {
        std::fs::write(path, &report).expect("write requested benchmark report");
    }
    writeln!(std::io::stdout().lock(), "{report}").expect("benchmark output");
    assert!(
        cpu_target,
        "all-pump or active-pump publication p95 exceeds 2 ms; receipt is retained as failed performance evidence"
    );
    assert!(
        growth <= 0,
        "parked source cardinality still grew on the third identical circuit"
    );
}
