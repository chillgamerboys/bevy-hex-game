#!/usr/bin/env python3
"""Verify Grand save/resume across separate test processes, without a window.

Build the current source once through Cargo, then launch its exact test artifact
as independent writer/reader processes for land, moving boat, and gliding flight.
Every case has explicit disposable HEX_GAME_DATA_DIR storage and a real package.
With --circuit, the same current-source binary also performs three streaming loops
with CPU terrain presentation, checking typed residency and sparse-edit retention.
With --admissions, it verifies all fourteen authored enemy parties and their codec.
With --walking, both dry walking and the separate walk/swim/walk crossing are required.
With --sailing, it exercises ordinary boat movement through the same build.
This checks persistence, not visual quality or native control feel.
"""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
TEST = "arena::grand::tests::process_checkpoint_child"
CIRCUIT_TEST = "arena::grand::tests::circuit_tests::actual_grand_streaming_circuit"
ADMISSION_TEST = "arena::grand::tests::admission_tests::actual_grand_all_authored_parties_admit_and_checkpoint"
TRAVERSAL_TESTS = {
    "walking": "arena::grand::tests::walking_tests::actual_grand_ordinary_walking",
    "sailing": "arena::grand::tests::sailing_tests::actual_grand_unupgraded_authored_sailing",
    "crossing": "arena::grand::tests::walking_tests::actual_grand_lake_walk_swim_walk",
}


def git(*arguments: str) -> str:
    return subprocess.check_output(["git", *arguments], cwd=ROOT, text=True).strip()


def source_snapshot() -> dict[str, str]:
    """Catch content edits even when HEAD and an existing 'M' status stay the same."""
    digest = hashlib.sha256()
    digest.update(subprocess.check_output(["git", "diff", "--binary", "HEAD", "--"], cwd=ROOT))
    untracked = subprocess.check_output(
        ["git", "ls-files", "--others", "--exclude-standard", "-z"], cwd=ROOT,
    )
    for relative in sorted(name for name in untracked.split(b"\0") if name):
        digest.update(relative + b"\0")
        path = ROOT / os.fsdecode(relative)
        if path.is_symlink():
            digest.update(b"symlink\0" + os.fsencode(os.readlink(path)))
        else:
            digest.update(b"file\0")
            with path.open("rb") as source:
                for block in iter(lambda: source.read(1024 * 1024), b""):
                    digest.update(block)
        digest.update(b"\0")
    return {
        "head": git("rev-parse", "HEAD"),
        "source_status": git("status", "--short"),
        "working_tree_sha256": digest.hexdigest(),
    }


def changed_source(before: dict[str, str], after: dict[str, str]) -> list[str]:
    return [name for name in before if before[name] != after[name]]


def require_same_source(before: dict[str, str], after: dict[str, str], phase: str) -> None:
    changed = changed_source(before, after)
    if changed:
        raise RuntimeError(
            f"Source changed during {phase} ({', '.join(changed)}). "
            "This run cannot report PASS; stop source edits and run fresh acceptance. "
            "See source_provenance in report.json."
        )


def build_test(target: Path, output: Path, environment: dict[str, str], profile: str) -> Path:
    command = [
        "cargo", "test", "-p", "hex_game", "--lib",
        "--features", "arena-prototype,test-support", "--offline", "--no-run",
        "--message-format=json", "--target-dir", str(target), "--profile", profile,
    ]
    with (output / "build.jsonl").open("w") as stdout, (output / "build.stderr.log").open("w") as stderr:
        result = subprocess.run(command, cwd=ROOT, env=environment, stdout=stdout, stderr=stderr, check=False)
    if result.returncode:
        raise RuntimeError(f"Current-source test build failed; see {output / 'build.stderr.log'} and build.jsonl")
    artifacts = []
    for line in (output / "build.jsonl").read_text().splitlines():
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue
        if (message.get("reason") == "compiler-artifact"
                and message.get("target", {}).get("name") == "hex_game"
                and "lib" in message.get("target", {}).get("kind", [])
                and message.get("profile", {}).get("test")
                and message.get("executable")):
            artifacts.append(Path(message["executable"]).resolve())
    if len(artifacts) != 1 or not artifacts[0].is_file():
        raise RuntimeError("Cargo did not identify exactly one current-source hex_game test executable")
    return artifacts[0]


def execute_case(binary: Path, mode: str, output: Path, environment: dict[str, str], timeout: float) -> dict:
    data = output / mode
    data.mkdir()
    (data / "grand-verification-only").write_text("Disposable process-restart verification.\n")
    for phase in ("write", "read"):
        child = environment | {
            "HEX_GAME_DATA_DIR": str(data),
            "HEX_GRAND_VERIFY_MODE": mode,
            "HEX_GRAND_VERIFY_PHASE": phase,
        }
        command = [str(binary), TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
        with (data / f"{phase}.log").open("w") as log:
            result = subprocess.run(command, cwd=ROOT, env=child, stdout=log, stderr=subprocess.STDOUT,
                                    timeout=timeout, check=False)
        if result.returncode:
            raise RuntimeError(f"{mode} {phase} failed; see {data / f'{phase}.log'}")
        if "running 1 test" not in (data / f"{phase}.log").read_text():
            raise RuntimeError(f"{mode} {phase}: exact child test did not execute")
    receipt = json.loads((data / "verified.json").read_text())
    if receipt["write_pid"] == receipt["read_pid"] or receipt["mode"] != mode:
        raise RuntimeError(f"{mode}: receipt does not prove a process restart")
    for field in ("full_owner_state_equal", "partial_health_retained", "carve_revisited"):
        if receipt.get(field) is not True:
            raise RuntimeError(f"{mode}: missing proof for {field}")
    head = data / "grand-v4-resume" / "session.ron"
    receipt["head_sha256"] = hashlib.sha256(head.read_bytes()).hexdigest()
    return receipt


def execute_circuit(binary: Path, output: Path, environment: dict[str, str], timeout: float) -> dict:
    data = output / "circuit"
    data.mkdir()
    (data / "grand-verification-only").write_text("Disposable streaming-circuit acceptance.\n")
    child = environment | {"HEX_GAME_DATA_DIR": str(data)}
    for key in ("HEX_GRAND_VERIFY_MODE", "HEX_GRAND_VERIFY_PHASE"):
        child.pop(key, None)
    command = [str(binary), CIRCUIT_TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    log_path = data / "circuit.log"
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=ROOT, env=child, stdout=log, stderr=subprocess.STDOUT,
                                timeout=timeout, check=False)
    if result.returncode:
        raise RuntimeError(f"Streaming circuit failed; see {log_path}")
    log_text = log_path.read_text()
    if "running 1 test" not in log_text or "GRAND_CIRCUIT_PASS " not in log_text:
        raise RuntimeError("Exact streaming-circuit test did not execute to its completion marker")
    receipt = json.loads((data / "circuit.json").read_text())
    if receipt.get("kind") != "grand-streaming-circuit-v1" or receipt.get("status") != "PASS":
        raise RuntimeError("Streaming-circuit receipt is missing its successful typed result")
    circuits = receipt.get("circuits", 0)
    if circuits < 3 or any(receipt.get(key, 0) < circuits for key in ("old_area_evictions", "ready_revisits")):
        raise RuntimeError("Streaming-circuit receipt does not prove three eviction/revisit loops")
    if receipt.get("edited_detail_published_before_eviction") is not True:
        raise RuntimeError("Streaming-circuit receipt did not prove edited detail existed before eviction")
    if receipt.get("completed_stops", 0) < 31 or len(receipt.get("samples", [])) != receipt["completed_stops"]:
        raise RuntimeError("Streaming-circuit receipt is missing authored stops")
    expected = {"source_chunks": 512, "finite_sources": 512, "detailed_chunks": 256, "source_jobs": 2}
    if receipt.get("limits") != expected:
        raise RuntimeError("Streaming-circuit receipt changed the accepted residency limits")
    highwater = receipt["highwater"]
    for observed, limit in (("resident_chunks", 512), ("finite_sources", 512),
                            ("detailed_chunks", 256), ("in_flight_jobs", 2)):
        if not 0 <= highwater[observed] <= limit:
            raise RuntimeError(f"Streaming circuit exceeded its {observed} limit")
    if highwater["detailed_chunks"] == 0 or highwater["observed_frames"] == 0:
        raise RuntimeError("Streaming circuit did not exercise real terrain presentation")
    if Path(receipt["package"]).resolve() != Path(environment["HEX_GRAND_WORLD"]).resolve():
        raise RuntimeError("Streaming-circuit receipt used a different immutable package")
    return receipt


def execute_admissions(binary: Path, output: Path, environment: dict[str, str], timeout: float) -> dict:
    data = output / "admissions"
    data.mkdir()
    (data / "grand-verification-only").write_text("Disposable authored-party admission acceptance.\n")
    child = environment | {"HEX_GAME_DATA_DIR": str(data)}
    for key in ("HEX_GRAND_VERIFY_MODE", "HEX_GRAND_VERIFY_PHASE"):
        child.pop(key, None)
    command = [str(binary), ADMISSION_TEST, "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    log_path = data / "admission.log"
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=ROOT, env=child, stdout=log, stderr=subprocess.STDOUT,
                                timeout=timeout, check=False)
    if result.returncode:
        raise RuntimeError(f"Authored-party admission failed; see {log_path}")
    log_text = log_path.read_text()
    if "running 1 test" not in log_text or "GRAND_ADMISSION_PASS " not in log_text:
        raise RuntimeError("Exact authored-party admission test did not complete")
    receipt = json.loads((data / "admission.json").read_text())
    if receipt.get("kind") != "grand-authored-admission-v1" or receipt.get("status") != "PASS":
        raise RuntimeError("Authored-party receipt is missing its successful typed result")
    expected = [(f"grand_goblin_{index:02}", count) for index, count in enumerate((8, 12, 18, 20, 22, 24), 1)]
    expected += [("grand_shaman_01", 3), ("grand_dragon_01", 3), ("grand_golem_01", 3),
                 ("grand_wisp_01", 10), ("grand_worm_01", 1), ("grand_worm_02", 1),
                 ("grand_worm_03", 1), ("grand_shadow_tunnel", 1)]
    parties = receipt.get("parties", [])
    if len(parties) != len(expected):
        raise RuntimeError("Authored-party receipt does not contain all fourteen parties")
    for ordinal, (party, (site, count)) in enumerate(zip(parties, expected)):
        if (party.get("site") != site or party.get("party") != ordinal or party.get("members") != count
                or party.get("actor_ids") != list(range(ordinal * 32 + 1, ordinal * 32 + count + 1))):
            raise RuntimeError(f"Authored-party receipt has an incomplete or wrong roster for {site}")
        if not 0 <= party.get("simulation_ticks", -1) <= 12:
            raise RuntimeError(f"Authored-party admission exceeded its simulation budget at {site}")
    if (receipt.get("unique_enemies") != 127 or parties[-1].get("registered_enemies") != 127
            or receipt.get("checkpoint_bytes", 0) <= 0
            or not 0 < receipt.get("active_actors_at_checkpoint", 128) < 128):
        raise RuntimeError("Authored-party receipt does not prove the complete roster, dormancy and checkpoint")
    if Path(receipt["package"]).resolve() != Path(environment["HEX_GRAND_WORLD"]).resolve():
        raise RuntimeError("Authored-party receipt used a different immutable package")
    return receipt



def validate_walking_receipt(receipt: dict) -> None:
    """Require per-simulation-tick one-voxel fall limits, including landing/settling."""
    if receipt.get("kind") != "grand-ordinary-walking-r04" or receipt.get("status") != "PASS":
        raise RuntimeError("Ordinary walking requires the current bounded-fall receipt")
    routes = receipt.get("routes", [])
    expected = receipt.get("route_names", [])
    required_probes = {"forest_north", "forest_south", "forest_east", "forest_west",
                       "river_bank_escape", "river_bank_along", "valley_crossing",
                       "grand-west-foothill-crossing", "grand-west-foothill-uphill",
                       "grand-lake-foothill-uphill"}
    if (receipt.get("selected_route") is not None or "grand-lake-foothill-crossing" in expected
            or not required_probes <= set(expected)
            or not any(route.get("category") == "authored_connection" for route in routes)
            or receipt.get("expected_routes") != len(expected)
            or len(expected) != len(set(expected))
            or len(routes) != len(expected) or {route["name"] for route in routes} != set(expected)
            or any(route.get("status") != "PASS"
                   or route.get("completed_segments") != route.get("required_segments")
                   or route.get("simulation_ticks", 0) <= 40 for route in routes)):
        raise RuntimeError("Ordinary walking did not complete all declared r02 routes and independent crossings")
    for route in routes:
        validate_grounded_trace(route.get("grounding") or {}, route["simulation_ticks"], route["name"])


def validate_grounded_trace(grounding: dict, ticks: int, name: str) -> None:
    """The same unloosened dry-step contract also checks each mixed dry segment."""
    limits = grounding.get("limits") or {}
    positive = ("voxel_height", "automatic_step_height", "gravity", "ground_snap_distance",
                "collision_skin", "tick_seconds", "maximum_unsupported_descent")
    if (grounding.get("contract") != "one-voxel-grounded-walk-v1"
            or grounding.get("failure") is not None
            or grounding.get("airborne_at_end") is not False
            or any(type(limits.get(key)) not in (int, float)
                   or not math.isfinite(limits[key]) or limits[key] <= 0 for key in positive)):
        raise RuntimeError(f"{name}: missing or invalid grounded-walking contract")
    descent_limit = (min(limits["voxel_height"], limits["automatic_step_height"])
                     + limits["ground_snap_distance"] + limits["collision_skin"])
    airborne_limit = math.ceil(math.sqrt(2 * descent_limit / limits["gravity"])
                               / limits["tick_seconds"]) + 1
    if (not math.isclose(limits["maximum_unsupported_descent"], descent_limit, rel_tol=1e-6)
            or type(limits.get("maximum_airborne_ticks")) is not int
            or limits["maximum_airborne_ticks"] != airborne_limit):
        raise RuntimeError(f"{name}: fall allowance is not derived from the physical step contract")
    if (grounding.get("observed_simulation_ticks") != ticks
            or any(type(grounding.get(key)) is not int or not 0 <= grounding[key] <= ticks
                   for key in ("observed_simulation_ticks", "total_airborne_ticks",
                               "maximum_airborne_ticks", "completed_airborne_episodes"))
            or grounding["maximum_airborne_ticks"] > airborne_limit
            or type(grounding.get("maximum_unsupported_descent")) not in (int, float)
            or not math.isfinite(grounding["maximum_unsupported_descent"])
            or not 0 <= grounding["maximum_unsupported_descent"] <= limits["maximum_unsupported_descent"]):
        raise RuntimeError(f"{name}: walking exceeded or omitted its per-tick fall evidence")


def validate_crossing_receipt(receipt: dict) -> None:
    """Require the separate complete same-endpoint walk/swim/walk claim."""
    route = receipt.get("route") or {}
    trace = route.get("mixed_crossing") or {}
    points = receipt.get("authored_endpoints", [])
    if (receipt.get("kind") != "grand-mixed-water-crossing-v1"
            or receipt.get("status") != "PASS" or receipt.get("selected_route") is not None
            or receipt.get("source_frame") != "grand-lake-foothill-crossing"
            or route.get("name") != "grand-lake-foothill-water-crossing"
            or route.get("category") != "walk_swim_walk" or route.get("status") != "PASS"
            or route.get("completed_segments") != 1 or route.get("required_segments") != 1
            or len(points) != 2 or any(len(point) != 3 or any(type(v) not in (int, float)
                or not math.isfinite(v) for v in point) for point in points)
            or route.get("waypoints") != points):
        raise RuntimeError("Mixed crossing must complete its unchanged published endpoints")
    ticks = route.get("simulation_ticks", 0)
    transitions = trace.get("transitions", [])
    positive_counts = ("swimming_ticks", "wet_swimming_ticks", "dry_ticks", "entry_count", "exit_count")
    if (type(ticks) is not int or ticks <= 40
            or trace.get("contract") != "ordinary-walk-swim-walk-v1"
            or trace.get("failure") is not None or trace.get("swimming_at_end") is not False
            or trace.get("all_terrain_ready") is not True or trace.get("all_solid_bodies_clear") is not True
            or trace.get("observed_simulation_ticks") != ticks
            or any(type(trace.get(k)) is not int or not 0 < trace[k] <= ticks for k in positive_counts)
            or trace["dry_ticks"] + trace["swimming_ticks"] != ticks
            or trace["wet_swimming_ticks"] > trace["swimming_ticks"]
            or not isinstance(transitions, list) or len(transitions) < 3):
        raise RuntimeError("Mixed crossing lacks complete per-tick mode, terrain or body evidence")
    modes = [event.get("mode") for event in transitions]
    event_ticks = [event.get("tick") for event in transitions]
    if (modes[0] != "walking" or modes[-1] != "walking"
            or any(mode not in ("walking", "swimming") for mode in modes)
            or any(a == b for a, b in zip(modes, modes[1:]))
            or modes.count("swimming") != trace["entry_count"]
            or modes.count("walking") - 1 != trace["exit_count"]
            or any(type(tick) is not int for tick in event_ticks)
            or any(a >= b for a, b in zip(event_ticks, event_ticks[1:]))
            or event_ticks[-1] - event_ticks[0] > ticks
            or any(event.get("terrain_ready") is not True or event.get("solid_body_clear") is not True
                   for event in transitions[1:])):
        raise RuntimeError("Mixed crossing lacks ordered authoritative water entry and exit")
    dimensions = route.get("body_dimensions", [])
    if (len(dimensions) != 3 or any(type(v) not in (int, float) or not math.isfinite(v) or v <= 0 for v in dimensions)
            or type(trace.get("swimming_distance")) not in (int, float)
            or not math.isfinite(trace["swimming_distance"]) or trace["swimming_distance"] < dimensions[0]
            or type(trace.get("maximum_sampled_water_depth")) not in (int, float)
            or not math.isfinite(trace["maximum_sampled_water_depth"]) or trace["maximum_sampled_water_depth"] <= 0
            or type(trace.get("minimum_oxygen")) not in (int, float)
            or not math.isfinite(trace["minimum_oxygen"]) or trace["minimum_oxygen"] < 0):
        raise RuntimeError("Mixed crossing did not swim a body width through sampled water")
    segments = trace.get("dry_segments", [])
    current = trace.get("current_dry_segment")
    if not isinstance(segments, list) or len(segments) != trace["entry_count"] or not isinstance(current, dict):
        raise RuntimeError("Mixed crossing omitted dry segments or its water-entry fall sample")
    segments = [*segments, current]
    checked = 0
    for index, segment in enumerate(segments):
        count = segment.get("observed_simulation_ticks")
        if type(count) is not int or count <= 0:
            raise RuntimeError("Mixed dry segment has no completed simulation evidence")
        validate_grounded_trace(segment, count, f"mixed dry segment {index}")
        checked += count
    if checked != trace["dry_ticks"] + trace["entry_count"]:
        raise RuntimeError("Mixed dry coverage omitted or duplicated a tick")
    samples = route.get("samples", [])
    arrival = samples[-1] if samples else {}
    endpoint = arrival.get("endpoint", {})
    if (type(arrival.get("settling_ticks")) is not int or arrival["settling_ticks"] < 40
            or type(arrival.get("remaining")) not in (int, float)
            or not math.isfinite(arrival["remaining"]) or not 0 <= arrival["remaining"] <= 2.0
            or endpoint.get("grounded") is not True or endpoint.get("support_valid") is not True
            or endpoint.get("volume_valid") is not True or current["observed_simulation_ticks"] < 40):
        raise RuntimeError("Mixed crossing did not finish forty live ticks on clear dry support")


def execute_walking_bundle(binary: Path, output: Path, environment: dict[str, str], report: dict) -> None:
    """Collect both independent outcomes; neither can substitute for the other."""
    failures = []
    for mode in ("walking", "crossing"):
        try:
            report[mode] = execute_traversal(binary, mode, output, environment)
        except Exception as error:
            report[mode] = {"status": "FAIL", "error": str(error)}
            failures.append(f"{mode}: {error}")
    if failures:
        raise RuntimeError("; ".join(failures))


def execute_traversal(binary: Path, mode: str, output: Path, environment: dict[str, str],
                      *, sailing_start: str = "sailing_start") -> dict:
    if sailing_start not in ("sailing_start", "sailing_start_bay"):
        raise ValueError(f"Unsupported sailing anchor: {sailing_start}")
    case = "sailing-bay" if mode == "sailing" and sailing_start == "sailing_start_bay" else mode
    data = output / case
    data.mkdir()
    (data / "grand-verification-only").write_text("Disposable actual-package traversal verification.\n")
    child = environment | {"HEX_GAME_DATA_DIR": str(data)}
    for key in ("HEX_GRAND_VERIFY_MODE", "HEX_GRAND_VERIFY_PHASE", "HEX_GRAND_WALK_ROUTE",
                "HEX_GRAND_SAIL_START_ANCHOR", "HEX_GRAND_CROSSING_ROUTE"):
        child.pop(key, None)
    if mode == "sailing":
        child["HEX_GRAND_SAIL_START_ANCHOR"] = sailing_start
    command = [str(binary), TRAVERSAL_TESTS[mode], "--exact", "--ignored", "--nocapture", "--test-threads=1"]
    log_path = data / f"{mode}.log"
    with log_path.open("w") as log:
        result = subprocess.run(command, cwd=ROOT, env=child, stdout=log, stderr=subprocess.STDOUT,
                                timeout=1260 if mode == "walking" else 420, check=False)
    if result.returncode or "running 1 test" not in log_path.read_text():
        raise RuntimeError(f"Actual {mode} failed or did not execute; see {log_path}")
    receipt = json.loads((data / f"{mode}.json").read_text())
    kind = {"walking":"grand-ordinary-walking-r04", "sailing":"grand-authored-sailing-v1",
            "crossing":"grand-mixed-water-crossing-v1"}[mode]
    if receipt.get("kind") != kind or receipt.get("status") != "PASS":
        raise RuntimeError(f"Actual {mode} receipt did not report completion")
    if Path(receipt["package"]).resolve() != Path(environment["HEX_GRAND_WORLD"]).resolve():
        raise RuntimeError(f"Actual {mode} used a different immutable package")
    if mode == "walking":
        validate_walking_receipt(receipt)
    elif mode == "crossing":
        validate_crossing_receipt(receipt)
    else:
        measurement = receipt.get("measurement", {})
        if (measurement.get("unupgraded") is not True or measurement.get("status") != "PASS"
                or measurement.get("start_anchor") != sailing_start
                or measurement.get("simulation_ticks", 0) <= 0
                or not measurement.get("endpoint_liveness_probe")
                or measurement.get("remaining", math.inf) > measurement.get("arrival_radius", 0)):
            raise RuntimeError("Sailing did not complete the unupgraded live crossing")
    return receipt


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path, help="Explicit actual Grand compiled package directory")
    parser.add_argument("--target-dir", required=True, type=Path, help="Existing shared Cargo target (coordinate the build slot)")
    parser.add_argument("--cargo-profile", choices=("dev", "ci"), default="dev", help="Reuse the existing development or CI test profile")
    parser.add_argument("--output", type=Path, help="Fresh output directory; must not already exist")
    parser.add_argument("--case", action="append", choices=("land", "boat", "air"), dest="cases")
    parser.add_argument("--circuit", action="store_true", help="Also run three actual-package streaming loops using the same test build")
    parser.add_argument("--admissions", action="store_true", help="Also verify all fourteen actual-package enemy parties using the same test build")
    parser.add_argument("--walking", action="store_true", help="Require dry authored connections/foothill probes AND the separate same-endpoint walk-swim-walk crossing")
    parser.add_argument("--sailing", action="store_true", help="Measure both western-shore and starting-bay boat crossings with production input")
    parser.add_argument("--timeout", type=float, default=240.0, help="Maximum seconds for each writer/reader process")
    parser.add_argument("--circuit-timeout", type=float, default=420.0, help="Maximum seconds for the optional circuit (internal deadline: 360 seconds)")
    parser.add_argument("--admission-timeout", type=float, default=420.0, help="Maximum seconds for the optional all-party admission oracle")
    arguments = parser.parse_args()
    if any(not math.isfinite(value) or value <= 0 for value in
           (arguments.timeout, arguments.circuit_timeout, arguments.admission_timeout)):
        parser.error("timeouts must be positive finite seconds")
    package = arguments.package.resolve(strict=True)
    if not package.is_dir():
        parser.error("--package must be a compiled world directory")
    for name in ("grand-overview.ron", "arena-sites.ron", "grand-biomes.ron"):
        if not (package / name).is_file():
            parser.error(f"Package is missing required current companion {name}")
    target = arguments.target_dir.resolve(strict=True)
    if arguments.output:
        output = arguments.output.resolve()
        output.mkdir(parents=True, exist_ok=False)
    else:
        parent = ROOT / ".context" / "grand-resume"
        parent.mkdir(parents=True, exist_ok=True)
        output = Path(tempfile.mkdtemp(prefix=f"{git('rev-parse', '--short=12', 'HEAD')}-", dir=parent))
    environment = os.environ | {
        "CARGO_INCREMENTAL": "0", "CARGO_BUILD_JOBS": "2", "CARGO_TARGET_DIR": str(target),
        "BEVY_ASSET_ROOT": str(ROOT), "HEX_GRAND_WORLD": str(package),
    }
    # An inherited capture or alternate scenario must never change this test mode.
    for key in ("HEX_REVIEW_CAPTURE", "HEX_WALK_SCRIPT", "HEX_WALK_OUT", "HEX_ARENA_CAPTURE", "HEX_NORTHERN_START"):
        environment.pop(key, None)
    before_build = source_snapshot()
    provenance = {"before_build": before_build}
    report = {
        "kind": "grand-process-restart-v1", "status": "INCOMPLETE",
        "head": before_build["head"], "source_status": before_build["source_status"],
        "source_provenance": provenance,
        "started_utc": datetime.now(timezone.utc).isoformat(), "package": str(package),
        "scope": "Production app/world/gameplay checkpoint composition; no window, pixels, or native motion.",
        "circuit_requested": arguments.circuit,
        "admissions_requested": arguments.admissions,
        "walking_requested": arguments.walking,
        "sailing_requested": arguments.sailing,
        "cargo_profile": arguments.cargo_profile,
        "cases": [],
    }
    report_path = output / "report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    try:
        try:
            binary = build_test(target, output, environment, arguments.cargo_profile)
        finally:
            provenance["after_build"] = source_snapshot()
            provenance["build_changed_fields"] = changed_source(before_build, provenance["after_build"])
        require_same_source(before_build, provenance["after_build"], "the test build")
        report["test_executable"] = str(binary)
        try:
            for case in arguments.cases or ("land", "boat", "air"):
                report["cases"].append(execute_case(binary, case, output, environment, arguments.timeout))
                report_path.write_text(json.dumps(report, indent=2) + "\n")
            if arguments.circuit:
                report["circuit"] = execute_circuit(binary, output, environment, arguments.circuit_timeout)
            if arguments.admissions:
                report["admissions"] = execute_admissions(binary, output, environment, arguments.admission_timeout)
            for mode in ("walking", "sailing"):
                if getattr(arguments, mode):
                    if mode == "walking":
                        execute_walking_bundle(binary, output, environment, report)
                    else:
                        report[mode] = execute_traversal(binary, mode, output, environment)
                    report_path.write_text(json.dumps(report, indent=2) + "\n")
                    if mode == "sailing":
                        report["sailing_bay"] = execute_traversal(
                            binary, mode, output, environment, sailing_start="sailing_start_bay")
        finally:
            provenance["after_execution"] = source_snapshot()
            provenance["execution_changed_fields"] = changed_source(before_build, provenance["after_execution"])
        require_same_source(before_build, provenance["after_execution"], "test execution")
        report["status"] = "PASS"
    except Exception as error:
        report["status"] = "FAIL"
        report["error"] = str(error)
        raise
    finally:
        report_path.write_text(json.dumps(report, indent=2) + "\n")
        print(report_path)


if __name__ == "__main__":
    main()
