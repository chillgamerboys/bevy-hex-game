#!/usr/bin/env python3
"""Verify Grand save/resume across separate test processes, without a window.

Build the current source once through Cargo, then launch its exact test artifact
as independent writer/reader processes for land, moving boat, and gliding flight.
Every case has explicit disposable HEX_GAME_DATA_DIR storage and a real package.
With --circuit, the same current-source binary also performs three streaming loops
with CPU terrain presentation, checking typed residency and sparse-edit retention.
With --admissions, it verifies all fourteen authored enemy parties and their codec.
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


def build_test(target: Path, output: Path, environment: dict[str, str]) -> Path:
    command = [
        "cargo", "test", "-p", "hex_game", "--lib",
        "--features", "arena-prototype,test-support", "--offline", "--no-run",
        "--message-format=json", "--target-dir", str(target),
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


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path, help="Explicit actual Grand compiled package directory")
    parser.add_argument("--target-dir", required=True, type=Path, help="Existing shared Cargo target (coordinate the build slot)")
    parser.add_argument("--output", type=Path, help="Fresh output directory; must not already exist")
    parser.add_argument("--case", action="append", choices=("land", "boat", "air"), dest="cases")
    parser.add_argument("--circuit", action="store_true", help="Also run three actual-package streaming loops using the same test build")
    parser.add_argument("--admissions", action="store_true", help="Also verify all fourteen actual-package enemy parties using the same test build")
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
        "cases": [],
    }
    report_path = output / "report.json"
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    try:
        try:
            binary = build_test(target, output, environment)
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
