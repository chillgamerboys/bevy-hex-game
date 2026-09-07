#!/usr/bin/env python3
"""Explicit native launch and windowless, provenance-bearing arena captures."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import shlex
import shutil
import signal
import struct
import subprocess
import sys
import time
from datetime import datetime, timezone


ROOT = Path(__file__).resolve().parents[1]
DEFAULT_TARGET = Path(
    "/Users/alberto/Documents/Codex/2026-09-04/there-were-a-few-issues-i/"
    "work/cargo-target-explore"
)
VIEWS = (
    "overview", "first", "third", "rear", "shield", "fireball", "blast", "tuning",
    "shield-compact", "shield-large", "fireball-compact", "fireball-large",
    "blast-compact", "blast-large",
    "shield-first", "shield-third", "fireball-first", "fireball-third",
    "blast-first", "blast-third", "shield-preview-first", "shield-preview-third", "start",
)
MATRIX = "arena-v5-release-casting"
MENU_VIEWS = ("start", "tuning", "first", "third", "overview", "rear")
CHARGE_VIEWS = (
    "start", "tuning", "first", "third",
    "shield-charge-partial-first", "shield-charge-partial-third",
    "shield-charge-full-first", "shield-charge-full-third",
    "fireball-charge-partial-first", "fireball-charge-partial-third",
    "fireball-charge-full-first", "fireball-charge-full-third",
    "blast-armed-first", "blast-armed-third",
    "shield-partial-preview-first", "shield-partial-preview-third",
)
CANVAS = [1600, 900]
CARGO_ARGS = ("run", "-p", "hex_game", "--features", "dev,arena-prototype", "--", "--arena")


def utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def write_json(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, ensure_ascii=True) + "\n", encoding="utf-8")


def git(*args: str) -> bytes:
    return subprocess.check_output(("git", *args), cwd=ROOT, stderr=subprocess.PIPE)


def source_state() -> tuple[dict, bytes, bytes]:
    """Fingerprint staged, unstaged, and untracked source without printing contents."""
    staged = git("diff", "--cached", "--binary", "HEAD", "--")
    unstaged = git("diff", "--binary", "--")
    status = git("status", "--porcelain=v1", "-z", "--untracked-files=all")
    untracked = {}
    for encoded in git("ls-files", "--others", "--exclude-standard", "-z").split(b"\0"):
        if encoded:
            name = os.fsdecode(encoded)
            path = ROOT / name
            data = os.fsencode(os.readlink(path)) if path.is_symlink() else path.read_bytes()
            untracked[name] = digest(data)
    state = {
        "head": git("rev-parse", "HEAD").decode().strip(),
        "status_porcelain_z": os.fsdecode(status),
        "staged_diff_sha256": digest(staged),
        "unstaged_diff_sha256": digest(unstaged),
        "untracked_sha256": untracked,
        "dirty": bool(status),
    }
    state["state_sha256"] = digest(json.dumps(state, sort_keys=True).encode())
    return state, staged, unstaged


def environment(target: Path) -> tuple[dict[str, str], list[str]]:
    """Discard inherited game capabilities; only this invocation may opt them in."""
    env = dict(os.environ)
    removed = sorted(key for key in env if key.startswith("HEX_"))
    for key in removed:
        del env[key]
    # Let .cargo/config.toml supply the checkout's asset root, even from Finder.
    env.pop("BEVY_ASSET_ROOT", None)
    env.update(CARGO_TARGET_DIR=str(target), CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="1")
    cargo = shutil.which("cargo", path=env.get("PATH"))
    if cargo is None:
        fallback = Path.home() / ".cargo" / "bin" / "cargo"
        if not fallback.is_file():
            raise RuntimeError("Cargo is unavailable. Install the repository's pinned Rust toolchain.")
        env["PATH"] = str(fallback.parent) + os.pathsep + env.get("PATH", "")
    return env, removed


def stop_process(process: subprocess.Popen) -> None:
    if process.poll() is not None:
        return
    os.killpg(process.pid, signal.SIGTERM)
    try:
        process.wait(timeout=3)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()


def run_cargo(env: dict[str, str], log_path: Path | None, timeout: float | None) -> int:
    """Stream native output, retain capture logs, and stop on missing asset errors."""
    command = ("cargo", *CARGO_ARGS)
    log = log_path.open("wb") if log_path else None
    process = None
    started = time.monotonic()
    tail = b""
    try:
        process = subprocess.Popen(
            command, cwd=ROOT, env=env, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, start_new_session=True,
        )
        assert process.stdout is not None
        with selectors.DefaultSelector() as selector:
            selector.register(process.stdout, selectors.EVENT_READ)
            while selector.get_map():
                if timeout is not None and time.monotonic() - started > timeout:
                    raise RuntimeError(f"Capture exceeded {timeout:g} seconds; native fallback is forbidden.")
                for key, _ in selector.select(timeout=0.25):
                    chunk = os.read(key.fileobj.fileno(), 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    if log:
                        log.write(chunk)
                        log.flush()
                    else:
                        sys.stdout.buffer.write(chunk)
                        sys.stdout.buffer.flush()
                    if b"path not found" in (tail + chunk).lower():
                        raise RuntimeError("Asset path not found; Cargo launch failed. See process output.")
                    tail = chunk[-64:]
        return process.wait()
    finally:
        if process is not None:
            stop_process(process)
            if process.stdout:
                process.stdout.close()
        if log:
            log.close()


def png_info(path: Path) -> dict:
    data = path.read_bytes()
    if len(data) < 33 or data[:8] != b"\x89PNG\r\n\x1a\n" or data[12:16] != b"IHDR":
        raise RuntimeError(f"Missing or invalid PNG: {path}")
    width, height = struct.unpack(">II", data[16:24])
    if not width or not height:
        raise RuntimeError(f"Empty PNG dimensions: {path}")
    return {"file": path.name, "sha256": digest(data), "bytes": len(data),
            "physical_pixels": [width, height]}


def native_receipt_info(png: Path, view: str, pixels: list[int]) -> dict:
    path = png.with_suffix(".json")
    data = path.read_bytes()
    try:
        state = json.loads(data)
    except (ValueError, UnicodeDecodeError) as error:
        raise RuntimeError(f"Invalid native state receipt: {path}") from error
    if not isinstance(state, dict) or state.get("view") != view:
        raise RuntimeError(f"Native state receipt does not identify view {view}: {path}")
    if [state.get("width"), state.get("height")] != pixels or pixels != CANVAS:
        raise RuntimeError(f"Native receipt/PNG dimensions disagree with the {CANVAS} capture canvas.")
    if view in CHARGE_VIEWS and ("charge-" in view or "armed" in view or "partial-preview" in view):
        human = next((actor for actor in state.get("actors", []) if actor.get("id") == 0), {})
        charge = human.get("charge")
        expected_spell = "Shield" if view.startswith("shield") else "Fireball" if view.startswith("fireball") else "AreaBlast"
        if not isinstance(charge, dict) or charge.get("spell") != expected_spell:
            raise RuntimeError(f"{view} did not retain the expected authoritative {expected_spell} charge.")
        progress = charge.get("progress")
        if not isinstance(progress, (int, float)) or not 0 <= progress <= 1:
            raise RuntimeError(f"{view} has no valid charge progress in its native receipt.")
        if "charge-partial" in view and not 0.35 <= progress <= 0.65:
            raise RuntimeError(f"{view} missed the partial-charge capture interval: {progress}.")
        if ("charge-full" in view or "partial-preview" in view) and progress < 0.99:
            raise RuntimeError(f"{view} did not reach full charge: {progress}.")
        samples = state.get("capture_inputs", [])
        if not any(sample.get("pressed") for sample in samples) or any(sample.get("released") for sample in samples):
            raise RuntimeError(f"{view} must retain a recorded press/hold sequence without a release.")
        if "partial-preview" in view:
            fixture = state.get("fixture_voxels", [])
            preview = state.get("preview", {})
            if len(fixture) != 2 or not preview.get("valid") or not preview.get("wall_voxels"):
                raise RuntimeError(f"{view} is missing its world-published partial shield fixture.")
            if any(voxel in preview["wall_voxels"] for voxel in fixture):
                raise RuntimeError(f"{view} preview failed to omit its occupied fixture slots.")
    return {"file": path.name, "sha256": digest(data), "bytes": len(data),
            "frame": state.get("frame"), "tick": state.get("tick")}


def capture(args: argparse.Namespace) -> int:
    views = CHARGE_VIEWS if args.charge_review else MENU_VIEWS if args.menu_review else VIEWS
    matrix = "arena-charge-v1" if args.charge_review else "arena-menu-v2" if args.menu_review else MATRIX
    output = args.output
    if not output.is_absolute():
        raise RuntimeError("--output must be an absolute path to a new directory.")
    if output.exists() or output.is_symlink():
        raise RuntimeError(f"Refusing to reuse an existing capture directory: {output}")
    output = output.resolve()
    if output.is_relative_to(ROOT):
        ignored = subprocess.run(
            ("git", "check-ignore", "--quiet", str(output)), cwd=ROOT, check=False,
        ).returncode == 0
        if not ignored:
            raise RuntimeError("Capture output inside the checkout must be Git-ignored (use .context/).")
    env, removed = environment(args.target_dir)
    initial, staged, unstaged = source_state()
    output.mkdir(parents=True, exist_ok=False)
    # The state-derived child is the actual pack; the requested directory is never reused.
    state_id = initial["head"]
    if initial["dirty"]:
        state_id += "-dirty-" + initial["state_sha256"][:12]
    pack = output / f"{state_id}-{matrix}"
    pack.mkdir()
    (pack / "staged.patch").write_bytes(staged)
    (pack / "unstaged.patch").write_bytes(unstaged)
    write_json(pack / "source-state.json", initial)
    receipt = {
        "schema_version": 1, "matrix": matrix, "pack": str(pack), "repository": str(ROOT),
        "started_at": utc_now(), "source": initial,
        "source_label": "UNAPPROVABLE-DIRTY" if initial["dirty"] else "COMMITTED-CANDIDATE",
        "scenario": "Spell Combat Arena / authored radius-12 arena",
        "terrain_seed": None, "terrain_seed_note": "Authored arena; no terrain seed override.",
        "capture_method": "windowless Bevy arena image-target hook",
        "logical_canvas": CANVAS, "device_scale": 1.0,
        "changed_surfaces": ["charge bar", "release guidance", "partial shield footprint", "ready screen", "paused menu", "actor cameras"] if args.charge_review else ["ready screen", "paused menu", "HUD key guidance"] if args.menu_review else ["terrain", "actor cameras", "cover", "spell effects", "HUD", "tuning", "ready screen"],
        "expected_views": list(views), "mechanical_status": "INCOMPLETE",
        "static_review": "UNREVIEWED", "human_motion": "HUMAN-MOTION-PENDING",
        "human_route": "Move, jump, sprint, look near walls, toggle camera; tap, partially charge and fully charge Shield/Fireball, release Area Blast, cancel holds with pause/focus/spell changes, and reset.",
        "gameplay_evidence": "Not established by captures; use typed tests and simulation receipts.",
        "inherited_capability_names_removed": removed,
        "environment": {key: env[key] for key in ("CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS")},
        "frames": [],
    }
    write_json(pack / "receipt.json", receipt)
    print(f"Windowless capture pack: {pack}", flush=True)
    try:
        seen = {}
        for view in views:
            if source_state()[0] != initial:
                raise RuntimeError("Source changed during capture; this pack is stale.")
            png = pack / f"{view}.png"
            frame_env = dict(env, HEX_ARENA_CAPTURE=str(png), HEX_ARENA_VIEW=view)
            frame = {
                "view": view, "started_at": utc_now(), "static_review": "UNREVIEWED",
                "command": ["cargo", *CARGO_ARGS], "cwd": str(ROOT),
                "capabilities": {"HEX_ARENA_CAPTURE": str(png), "HEX_ARENA_VIEW": view},
                "log": f"{view}.log", "mechanical_status": "INCOMPLETE",
            }
            receipt["frames"].append(frame)
            write_json(pack / "receipt.json", receipt)
            print(f"Capturing {view}…", flush=True)
            frame["exit_code"] = run_cargo(frame_env, pack / frame["log"], args.timeout)
            if frame["exit_code"]:
                raise RuntimeError(f"{view} exited with code {frame['exit_code']}; inspect {frame['log']}.")
            frame.update(png_info(png))
            frame["native_state_receipt"] = native_receipt_info(png, view, frame["physical_pixels"])
            frame.update(logical_canvas=CANVAS, device_scale=1.0)
            if frame["sha256"] in seen:
                raise RuntimeError(f"Unexpected duplicate frames: {view} and {seen[frame['sha256']]}.")
            seen[frame["sha256"]] = view
            frame.update(mechanical_status="CAPTURED", finished_at=utc_now())
            write_json(pack / "receipt.json", receipt)
        final = source_state()[0]
        receipt["source_after"] = final
        if final != initial:
            raise RuntimeError("Source changed during capture; this pack is stale.")
        receipt["mechanical_status"] = "COMPLETE"
    except (Exception, KeyboardInterrupt) as error:
        receipt["mechanical_status"] = "BLOCKED"
        receipt["error"] = str(error) or "Interrupted"
        raise
    finally:
        receipt["finished_at"] = utc_now()
        write_json(pack / "receipt.json", receipt)
        for frame in receipt["frames"]:
            log = pack / frame["log"]
            if log.is_file():
                frame["log_sha256"] = digest(log.read_bytes())
            write_json(pack / f"{frame['view']}.receipt.json", frame)
        write_json(pack / "receipt.json", receipt)
    print("Capture matrix complete. Static review: UNREVIEWED. Native motion: HUMAN-MOTION-PENDING.")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    launch = commands.add_parser("launch", help="Explicitly open the native playable arena through Cargo.")
    captures = commands.add_parser("capture", help="Capture all 23 views without a native window.")
    for command in (launch, captures):
        command.add_argument("--target-dir", type=Path, default=DEFAULT_TARGET,
                             help="Explicit shared Cargo target directory (absolute path).")
    captures.add_argument("--output", type=Path, required=True,
                          help="New absolute parent directory; receives a state-named capture pack.")
    captures.add_argument("--timeout", type=float, default=300,
                          help="Maximum seconds per capture, including any Cargo work (default: 300).")
    review = captures.add_mutually_exclusive_group()
    review.add_argument("--menu-review", action="store_true",
                        help="Capture the six ready/menu/HUD review views.")
    review.add_argument("--charge-review", action="store_true",
                        help="Capture 16 charge/release, clipped shield, ready/menu, and actor-camera views.")
    args = parser.parse_args(argv)
    try:
        if not args.target_dir.is_absolute():
            raise RuntimeError("--target-dir must be absolute.")
        if args.command == "capture":
            if not 0 < args.timeout < float("inf"):
                raise RuntimeError("--timeout must be a finite positive number.")
            return capture(args)
        env, _ = environment(args.target_dir)
        print(f"Opening native Spell Combat Arena: {shlex.join(('cargo', *CARGO_ARGS))}", flush=True)
        return run_cargo(env, None, None)
    except KeyboardInterrupt:
        print("Arena helper interrupted.", file=sys.stderr)
        return 130
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"Arena helper: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
