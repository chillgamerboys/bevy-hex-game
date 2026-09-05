#!/usr/bin/env python3
"""Capture one V4 view through Cargo, with fail-closed provenance and no native window.

Example (from this checkout, after committing the candidate)::

    python3 tools/v4_review.py --package .context/v4/workspaces/two-regions \
        --output .context/v4/review-run-001 --name seam --focus 184,-88,30 \
        --radius 16 --parties 2 --walk assets/config/v4/walks/seam-reversal.ron

The supplied output directory must not exist. Artifacts are placed under its full
HEAD/matrix-name child. --dirty-diagnostic and --profile map-test are explicitly
unapprovable diagnostics. Mechanical completion never grants visual or native-motion
approval. Run --self-test for lightweight tests; it launches no Cargo or game.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import time
from typing import Any
import zlib


ROOT = Path(__file__).resolve().parents[1]
MAX_SCRIPT = 262_144
MAX_JSON = 32 * 1024 * 1024
MAX_LOG = 128 * 1024 * 1024
MAX_MEMORY_PROBE_FAILURE_DETAILS = 8
MAX_MEMORY_PROBE_DETAIL = 512
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
SKIP_SOURCE_DIRS = {".git", ".context", "target", "node_modules", ".venv", "__pycache__", ".pytest_cache", ".mypy_cache"}
RELEVANT_ENV = {"CARGO_TARGET_DIR", "CARGO_BUILD_JOBS", "CARGO_INCREMENTAL", "RUSTFLAGS", "RUSTDOCFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER", "RUSTUP_TOOLCHAIN", "CARGO_BUILD_TARGET", "CARGO_ENCODED_RUSTFLAGS", "BEVY_ASSET_ROOT"}


class ReviewError(RuntimeError):
    """A capture cannot be published as mechanically complete."""


class RunAborted(ReviewError):
    def __init__(self, error: BaseException, result: dict[str, Any]):
        super().__init__(f"{type(error).__name__}: {error}")
        self.result = result
        self.interrupted = isinstance(error, KeyboardInterrupt)


def canonical_json(value: Any) -> bytes:
    return (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()


def atomic_json(path: Path, value: Any) -> None:
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=path.parent, delete=False) as stream:
            temporary = Path(stream.name)
            stream.write(canonical_json(value))
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
        descriptor = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def bounded_bytes(path: Path, limit: int) -> bytes:
    if path.is_symlink() or not path.is_file():
        raise ReviewError(f"expected a regular input file: {path}")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    if len(data) > limit:
        raise ReviewError(f"input exceeds {limit} bytes: {path}")
    return data


def file_record(path: Path) -> dict[str, Any]:
    before = path.lstat()
    if not stat.S_ISREG(before.st_mode):
        raise ReviewError(f"source must be a regular file, not a link or special file: {path}")
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    after = path.lstat()
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
        after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns
    ):
        raise ReviewError(f"source changed while being hashed: {path}")
    return {"sha256": digest.hexdigest(), "bytes": after.st_size, "mode": stat.S_IMODE(after.st_mode),
            "mtime_ns": after.st_mtime_ns, "ctime_ns": after.st_ctime_ns}


def git(root: Path, *arguments: str) -> bytes:
    result = subprocess.run(["git", *arguments], cwd=root, capture_output=True, check=False)
    if result.returncode:
        raise ReviewError(result.stderr.decode(errors="replace").strip() or "Git command failed")
    return result.stdout


def source_snapshot(root: Path = ROOT) -> dict[str, Any]:
    """Hash exact checkout bytes, including ignored relevant source/assets.

    Git-visible files are all included. A second walk catches otherwise ignored
    additions under assets/crates/tools; build/cache/output directories are excluded.
    Ignored relevant additions also make a strict source state dirty.
    """
    tracked = {os.fsdecode(name) for name in git(root, "ls-files", "--cached", "-z").split(b"\0") if name}
    visible = {os.fsdecode(name) for name in git(root, "ls-files", "--cached", "--others", "--exclude-standard", "-z").split(b"\0") if name}
    paths = set(visible)
    for directory in ("assets", "crates", "tools"):
        for base, directories, files in os.walk(root / directory, followlinks=False):
            for name in directories:
                if name not in SKIP_SOURCE_DIRS and (Path(base) / name).is_symlink():
                    raise ReviewError(f"unhashable source-directory symlink: {Path(base) / name}")
            directories[:] = [name for name in directories if name not in SKIP_SOURCE_DIRS]
            paths.update(
                (Path(base) / name).relative_to(root).as_posix()
                for name in files
                if name != ".DS_Store" and not name.endswith((".pyc", ".pyo"))
            )
    for name in (".cargo/config", ".cargo/config.toml"):
        if (root / name).exists():
            paths.add(name)
    files = {}
    for name in sorted(paths):
        path = root / name
        files[name] = file_record(path) if path.exists() or path.is_symlink() else {"missing": True}
    status = git(root, "status", "--porcelain=v1", "--untracked-files=all").decode(errors="surrogateescape")
    extra = sorted(paths - tracked)
    identity = {
        "head": git(root, "rev-parse", "HEAD").decode().strip(),
        "status": status,
        "untracked_relevant": extra,
        "files": files,
    }
    return {**identity, "dirty": bool(status or extra), "sha256": hashlib.sha256(canonical_json(identity)).hexdigest()}


def package_snapshot(directory: Path) -> dict[str, Any]:
    """Bind the immutable manifest chosen by a workspace pointer, without a world scan."""
    pointer = directory / "current.ron"
    manifest = directory / "manifest.ron"
    result: dict[str, Any] = {"requested_directory": str(directory)}
    if not manifest.is_file():
        text = bounded_bytes(pointer, 1024 * 1024).decode()
        match = re.search(r'\bmanifest_path\s*:\s*("(?:\\.|[^"\\])*")', text)
        if match is None:
            raise ReviewError("workspace has no readable immutable manifest path")
        relative = json.loads(match.group(1))
        if not re.fullmatch(r"packages/[0-9a-f]{16}/manifest\.ron", relative):
            raise ReviewError("workspace manifest path is not a confined immutable revision")
        manifest = directory / relative
        if not manifest.resolve().is_relative_to(directory):
            raise ReviewError("workspace manifest escapes its package directory")
        result["pointer"] = file_record(pointer)
    data = bounded_bytes(manifest, 64 * 1024 * 1024)
    match = re.search(rb"\bfingerprint\s*:\s*(\d+)\s*,?\s*\)\s*$", data)
    if match is None or int(match.group(1)) >= 2**64:
        raise ReviewError("manifest has no terminal u64 package fingerprint")
    result.update(manifest=str(manifest), manifest_sha256=hashlib.sha256(data).hexdigest(),
                  fingerprint=f"{int(match.group(1)):016x}")
    return result


def parse_walk(data: bytes) -> dict[str, Any]:
    """Parse the bounded ordinary RON shape used by the strict walk schema.

    This is an independent receipt-binding reader, not an authoring compiler.
    Unsupported lexical constructs fail before launch; Rust remains schema authority.
    """
    if len(data) > MAX_SCRIPT:
        raise ReviewError("walk exceeds 256 KiB")
    source = data.decode()
    lexer = re.compile(r'\s+|//[^\n]*|/\*[\s\S]*?\*/|"(?:\\.|[^"\\])*"|-?\d+|[A-Za-z_]\w*|[(){}\[\]:,]')
    tokens = []
    offset = 0
    for match in lexer.finditer(source):
        if match.start() != offset:
            raise ReviewError(f"unsupported walk syntax at byte {offset}")
        offset = match.end()
        token = match.group()
        if not token.isspace() and not token.startswith(("//", "/*")):
            tokens.append(token)
    if offset != len(source):
        raise ReviewError("unsupported or incomplete walk syntax")
    cursor = 0

    def read(depth: int = 0) -> Any:
        nonlocal cursor
        if depth > 32 or cursor >= len(tokens):
            raise ReviewError("walk nesting or input ended unexpectedly")
        token = tokens[cursor]
        cursor += 1
        if token.startswith('"'):
            return json.loads(token)
        if re.fullmatch(r"-?\d+", token):
            return int(token)
        if token in ("true", "false", "None"):
            return {"true": True, "false": False, "None": None}[token]
        if token in ("(", "[", "{"):
            close = {"(": ")", "[": "]", "{": "}"}[token]
            mapping = token == "{" or (token == "(" and cursor + 1 < len(tokens) and tokens[cursor + 1] == ":")
            value: Any = {} if mapping else []
            while cursor < len(tokens) and tokens[cursor] != close:
                child = read(depth + 1)
                if mapping:
                    if cursor >= len(tokens) or tokens[cursor] != ":" or not isinstance(child, str) or child in value:
                        raise ReviewError("duplicate or malformed walk field")
                    cursor += 1
                    value[child] = read(depth + 1)
                else:
                    value.append(child)
                if cursor < len(tokens) and tokens[cursor] == ",":
                    cursor += 1
                elif cursor >= len(tokens) or tokens[cursor] != close:
                    raise ReviewError("walk separator is missing")
            if cursor >= len(tokens):
                raise ReviewError("walk container was not closed")
            cursor += 1
            return value
        if re.fullmatch(r"[A-Za-z_]\w*", token):
            if cursor < len(tokens) and tokens[cursor] == "(":
                value = read(depth + 1)
                if token == "Some":
                    if not isinstance(value, list) or len(value) != 1:
                        raise ReviewError("malformed Some in walk")
                    return value[0]
                return {token: value}
            return token
        raise ReviewError(f"unexpected walk token {token!r}")

    value = read()
    if cursor != len(tokens) or not isinstance(value, dict) or set(value) != {"schema_version", "id", "max_ticks", "steps"}:
        raise ReviewError("walk root must contain only schema_version/id/max_ticks/steps")
    if value["schema_version"] != 1 or not isinstance(value["id"], str) or not value["id"]:
        raise ReviewError("invalid walk identity")
    positive_int(value["max_ticks"], "walk max_ticks", 100_000)
    steps = value["steps"]
    if not isinstance(steps, list) or not 1 <= len(steps) <= 2048:
        raise ReviewError("walk step count must be 1..2048")
    if any(not isinstance(step, dict) or len(step) != 1 for step in steps):
        raise ReviewError("each walk step must be one named command")
    if not any("MoveTo" in step for step in steps) or not any("WaitAt" in step or "AssertAt" in step for step in steps):
        raise ReviewError("walk contains no requested and observed movement")
    return value


def positive_int(value: Any, name: str, maximum: int | None = None) -> int:
    if type(value) is not int or value <= 0 or (maximum is not None and value > maximum):
        raise ReviewError(f"{name} must be a positive bounded integer")
    return value


def nonnegative_int(value: Any, name: str) -> int:
    if type(value) is not int or value < 0:
        raise ReviewError(f"{name} must be a nonnegative integer")
    return value


def validate_position(value: Any) -> None:
    if not isinstance(value, dict) or set(value) != {"column", "level"}:
        raise ReviewError("actor position is not an exact voxel coordinate")
    column = value["column"]
    if not isinstance(column, dict) or set(column) != {"q", "r"}:
        raise ReviewError("actor position is missing its horizontal coordinate")
    if any(type(column[key]) is not int or not -2**63 <= column[key] < 2**63 for key in ("q", "r")):
        raise ReviewError("actor coordinate exceeds the i64 domain")
    if type(value["level"]) is not int or not -2**31 <= value["level"] < 2**31:
        raise ReviewError("actor level exceeds the i32 domain")


def strict_json(path: Path) -> dict[str, Any]:
    def pairs(items: list[tuple[str, Any]]) -> dict[str, Any]:
        result = {}
        for key, value in items:
            if key in result:
                raise ReviewError(f"duplicate JSON receipt key: {key}")
            result[key] = value
        return result
    value = json.loads(bounded_bytes(path, MAX_JSON), object_pairs_hook=pairs,
                       parse_constant=lambda value: (_ for _ in ()).throw(ReviewError(f"nonfinite JSON: {value}")))
    if not isinstance(value, dict):
        raise ReviewError("game receipt must be an object")
    return value


def validate_receipt(receipt: dict[str, Any], package: dict[str, Any], walk: dict[str, Any] | None,
                     settle_frames: int = 120) -> None:
    if receipt.get("package") != package["requested_directory"] or receipt.get("world_fingerprint") != package["fingerprint"]:
        raise ReviewError("game receipt does not match the requested package identity")
    for field in ("frames", "resident_chunks", "rendered_chunks", "mesh_publications", "rendered_vertices"):
        positive_int(receipt.get(field), field)
    settled = positive_int(receipt.get("settled_frames"), "settled frames", receipt["frames"])
    if settled < settle_frames:
        raise ReviewError("capture did not reach the requested consecutive settled-frame budget")
    for field in ("discarded_mesh_jobs", "local_queue_peak"):
        nonnegative_int(receipt.get(field), field)
    if receipt["rendered_chunks"] > receipt["resident_chunks"]:
        raise ReviewError("receipt renders more chunks than are resident")
    samples = receipt.get("frame_samples_ms")
    if not isinstance(samples, list) or not 0 < len(samples) <= min(receipt["frames"], 100_000):
        raise ReviewError("receipt has no bounded frame measurements")
    rebase_samples = receipt.get("rebase_samples_ms")
    if not isinstance(rebase_samples, list) or len(rebase_samples) > receipt["frames"]:
        raise ReviewError("receipt has invalid rebase measurements")
    for value in [receipt.get("elapsed_seconds"), *samples, *rebase_samples]:
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ReviewError("invalid timing value in game receipt")
    if receipt["elapsed_seconds"] <= 0 or not any(value > 0 for value in samples):
        raise ReviewError("receipt records zero elapsed work")
    if receipt.get("static_review") != "UNREVIEWED" or receipt.get("native_motion") != "HUMAN-MOTION-PENDING":
        raise ReviewError("game receipt improperly grants review approval")
    rows = receipt.get("scripted_walk")
    if walk is None:
        if rows is not None:
            raise ReviewError("unexpected scripted walk in an unscripted capture")
        return
    if not isinstance(rows, list) or len(rows) != len(walk["steps"]):
        raise ReviewError("scripted walk receipt is missing or incomplete")
    fingerprint = None
    previous_tick = previous_moves = 0
    issued: dict[str, tuple[Any, Any]] = {}
    removals: dict[str, int] = {}
    for index, (row, command) in enumerate(zip(rows, walk["steps"])):
        if not isinstance(row, dict) or row.get("step") != index or row.get("command") != command or row.get("script") != walk["id"]:
            raise ReviewError(f"script step {index} does not match the supplied source")
        tick = positive_int(row.get("tick"), "walk tick", walk["max_ticks"])
        if tick <= previous_tick:
            raise ReviewError("walk ticks are not strictly increasing")
        previous_tick = tick
        current = row.get("script_fingerprint")
        if not isinstance(current, str) or not re.fullmatch(r"[0-9a-f]{16}", current) or (fingerprint is not None and current != fingerprint):
            raise ReviewError("walk source fingerprint changed or is missing")
        fingerprint = current
        if row.get("evidence") != "AUTOMATED-TYPED-MOTION" or row.get("native_motion") != "HUMAN-MOTION-PENDING":
            raise ReviewError("walk receipt grants unsupported motion approval")
        actors = row.get("actors")
        if not isinstance(actors, list) or not actors or any(not isinstance(actor, dict) for actor in actors):
            raise ReviewError("walk receipt is missing actor facts")
        by_id = {}
        for fact in actors:
            identity = fact.get("id")
            if not isinstance(identity, str) or not identity or identity in by_id:
                raise ReviewError("walk receipt has invalid actor identities")
            by_id[identity] = fact
            for field in ("position", "motion_to", "pending_goal"):
                if field not in fact:
                    raise ReviewError(f"actor receipt omits {field}")
                if fact[field] is not None:
                    validate_position(fact[field])
            nonnegative_int(fact.get("queued_steps"), "actor queued steps")
            if type(fact.get("turn_steps")) is not bool:
                raise ReviewError("actor receipt omits its step mode")
            fraction = fact.get("motion_fraction")
            if fact["motion_to"] is None:
                if fraction is not None:
                    raise ReviewError("actor has a motion fraction without a destination")
            elif type(fraction) not in (int, float) or not math.isfinite(fraction) or not 0 <= fraction <= 1:
                raise ReviewError("actor interpolation fraction is invalid")
        if len(by_id) != len(actors):
            raise ReviewError("walk receipt has invalid actor identities")
        kind, arguments = next(iter(command.items()))
        if kind in ("RemoveObject", "WaitObject"):
            edits = nonnegative_int(row.get("successful_object_edits"), "successful object edits")
            if kind == "RemoveObject":
                if row.get("observed_object_present") is not True or row.get("pending_object_request") != arguments["object_id"]:
                    raise ReviewError("object removal has no present exact source or queued intent")
                removals[arguments["object_id"]] = edits
            else:
                if row.get("observed_object_present") is not arguments["present"] or row.get("observed_revision") is None:
                    raise ReviewError("object assertion differs from available exact source")
                if any(row.get(field) for field in ("pending_object_request", "object_removal_pending", "cancel_object_edit_pending")):
                    raise ReviewError("object assertion completed with a pending command")
                before_remove = removals.get(arguments["object_id"])
                if not arguments["present"] and before_remove is not None and edits <= before_remove:
                    raise ReviewError("object removal has no successful transaction acknowledgement")
        actor = by_id.get(arguments.get("actor"))
        if "actor" in arguments and actor is None:
            raise ReviewError("walk command actor is absent from receipt")
        if kind == "MoveTo":
            issued[arguments["actor"]] = (actor.get("position"), arguments["goal"])
        if kind in ("WaitAt", "AssertAt") and actor.get("position") != arguments["position"]:
            raise ReviewError("exact actor assertion differs from its receipt")
        settled = actor is not None and actor.get("motion_to") is None and actor.get("pending_goal") is None and (actor.get("turn_steps") is True or actor.get("queued_steps") == 0)
        if kind == "WaitAt" and not settled:
            raise ReviewError("WaitAt completed while its actor was still moving or planning")
        moves = row.get("verified_moves")
        if type(moves) is not int or moves - previous_moves not in (0, 1):
            raise ReviewError("invalid verified movement count")
        if moves > previous_moves:
            old = issued.get(arguments.get("actor"))
            if kind not in ("WaitAt", "AssertAt") or not settled or old is None or old[0] is None or old[0] == old[1] or old[1] != arguments["position"]:
                raise ReviewError("movement count has no settled, changed-support witness")
            del issued[arguments["actor"]]
        previous_moves = moves
    positive_int(previous_moves, "completed verified movements")


def png_coverage(path: Path, expected: tuple[int, int] = (1920, 1080)) -> dict[str, Any]:
    """Decode the game's RGB/RGBA8 noninterlaced PNG and apply its coverage rule."""
    data = bounded_bytes(path, 64 * 1024 * 1024)
    if not data.startswith(PNG_SIGNATURE):
        raise ReviewError("capture is not a PNG")
    offset, compressed, header, ended = 8, bytearray(), None, False
    while offset < len(data):
        if offset + 12 > len(data):
            raise ReviewError("truncated PNG chunk")
        length = struct.unpack_from(">I", data, offset)[0]
        kind = data[offset + 4:offset + 8]
        end = offset + 12 + length
        if end > len(data):
            raise ReviewError("truncated PNG payload")
        payload = data[offset + 8:end - 4]
        crc = struct.unpack_from(">I", data, end - 4)[0]
        if zlib.crc32(kind + payload) & 0xFFFFFFFF != crc:
            raise ReviewError("PNG checksum mismatch")
        if kind == b"IHDR":
            if header is not None or offset != 8 or length != 13:
                raise ReviewError("invalid PNG header")
            header = struct.unpack(">IIBBBBB", payload)
        elif kind == b"IDAT":
            compressed.extend(payload)
        elif kind == b"IEND":
            if length or end != len(data):
                raise ReviewError("invalid PNG end marker")
            ended = True
        elif kind[:1].isupper():
            raise ReviewError(f"unsupported critical PNG chunk {kind!r}")
        offset = end
    if header is None or not ended:
        raise ReviewError("incomplete PNG")
    width, height, depth, color, compression, filtering, interlace = header
    if (width, height) != expected or width * height > 8_388_608 or depth != 8 or color not in (2, 6) or any((compression, filtering, interlace)):
        raise ReviewError("capture is not the expected bounded RGB/RGBA8 image target")
    channels = 3 if color == 2 else 4
    stride = width * channels
    expected_bytes = height * (stride + 1)
    inflater = zlib.decompressobj()
    packed = inflater.decompress(compressed, expected_bytes + 1)
    if len(packed) != expected_bytes or not inflater.eof or inflater.unused_data or inflater.unconsumed_tail:
        raise ReviewError("invalid or oversized PNG scanlines")
    minimum = [[255] * 3 for _ in range(32)]
    maximum = [[0] * 3 for _ in range(32)]
    histogram = [0] * 4096
    previous = bytearray(stride)
    brightest = 0
    for y in range(height):
        start = y * (stride + 1)
        filter_type = packed[start]
        row = bytearray(packed[start + 1:start + stride + 1])
        if filter_type not in range(5):
            raise ReviewError("unknown PNG filter")
        for index in range(stride):
            left = row[index - channels] if index >= channels else 0
            above = previous[index]
            upper_left = previous[index - channels] if index >= channels else 0
            predictor = 0
            if filter_type == 1:
                predictor = left
            elif filter_type == 2:
                predictor = above
            elif filter_type == 3:
                predictor = (left + above) // 2
            elif filter_type == 4:
                estimate = left + above - upper_left
                distances = (abs(estimate - left), abs(estimate - above), abs(estimate - upper_left))
                predictor = (left, above, upper_left)[distances.index(min(distances))]
            row[index] = (row[index] + predictor) & 255
        for x in range(width):
            red, green, blue = row[x * channels:x * channels + 3]
            if channels == 4 and row[x * channels + 3] != 255:
                raise ReviewError("capture target contains nonopaque pixels")
            brightest = max(brightest, red, green, blue)
            histogram[(red >> 4) * 256 + (green >> 4) * 16 + (blue >> 4)] += 1
            region = (y * 4 // height) * 8 + x * 8 // width
            for channel, value in enumerate((red, green, blue)):
                minimum[region][channel] = min(minimum[region][channel], value)
                maximum[region][channel] = max(maximum[region][channel], value)
        previous = row
    pixels = width * height
    variant = pixels - max(histogram)
    varied_regions = sum(any(high - low > 12 for low, high in zip(a, b)) for a, b in zip(minimum, maximum))
    coverage = brightest > 8 and variant * 100 >= pixels * 5 and varied_regions >= 8
    result = {"width": width, "height": height, "brightest": brightest, "variant_pixels": variant,
              "varied_regions": varied_regions, "has_coverage": coverage,
              "method": "hex_game/capture.rs 8x4 regional variation; not a framing or visual verdict"}
    if not coverage:
        raise ReviewError(f"capture failed image coverage: {result}")
    return result


def peak_rss(stderr: str) -> int | None:
    values = re.findall(r"^\s*(\d+)\s+maximum resident set size\s*$", stderr, re.MULTILINE)
    return int(values[-1]) if values and int(values[-1]) > 0 else None


def group_is_gone(group: int) -> bool:
    """Darwin can return EPERM for an already-empty reaped process group."""
    try:
        result = subprocess.run(["ps", "-axo", "pgid=,stat="], capture_output=True, timeout=2, check=False)
    except (subprocess.TimeoutExpired, OSError):
        return False  # Failure to inspect a group cannot prove it is gone.
    if result.returncode:
        return False  # Unknown ownership/cleanup remains a failure, never a pass.
    for line in result.stdout.decode().splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] == str(group) and not fields[1].startswith("Z"):
            return False
    return True


def terminate_group(process: subprocess.Popen[bytes]) -> None:
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    except PermissionError:
        if not group_is_gone(process.pid):
            raise
        process.wait(timeout=5)
        return
    # A Cargo/time parent may exit while a child ignores SIGTERM. Reap the parent,
    # but test the whole owned group before deciding cleanup is finished.
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        process.poll()
        try:
            os.killpg(process.pid, 0)
        except ProcessLookupError:
            break
        except PermissionError:
            if not group_is_gone(process.pid):
                raise
            break
        time.sleep(0.02)
    else:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            if not group_is_gone(process.pid):
                raise
    process.wait(timeout=5)


def parse_game_memory(rows: str, group: int, timestamp: float) -> list[dict[str, Any]]:
    """Only the exact game executable in the driver's owned process group counts."""
    samples = []
    for line in rows.splitlines():
        fields = line.strip().split(None, 3)
        if len(fields) != 4 or fields[0] != str(group) or Path(fields[3]).name != "hex_v4":
            continue
        try:
            pid, kibibytes = int(fields[1]), int(fields[2])
        except ValueError:
            continue
        if pid > 0 and kibibytes > 0:
            samples.append({"unix_seconds": timestamp, "pid": pid, "rss_bytes": kibibytes * 1024})
    return samples


def sample_game_memory(group: int) -> tuple[list[dict[str, Any]], dict[str, Any] | None]:
    """Optional telemetry cannot abort the owned build/game or invent RSS samples."""
    timestamp = time.time()
    try:
        sampled = subprocess.run(["ps", "-axo", "pgid=,pid=,rss=,comm="], capture_output=True, timeout=2, check=False)
    except (subprocess.TimeoutExpired, OSError) as error:
        return [], {"unix_seconds": timestamp, "type": type(error).__name__,
                    "message": str(error)[:MAX_MEMORY_PROBE_DETAIL]}
    if sampled.returncode:
        return [], {"unix_seconds": timestamp, "type": "NonzeroExit", "exit_code": sampled.returncode,
                    "message": sampled.stderr.decode(errors="replace")[:MAX_MEMORY_PROBE_DETAIL]}
    return parse_game_memory(sampled.stdout.decode(errors="replace"), group, timestamp), None


def run_process(argv: list[str], cwd: Path, environment: dict[str, str], output: Path, timeout: float) -> dict[str, Any]:
    started = time.monotonic()
    memory: list[dict[str, Any]] = []
    probe_attempts = 0
    probe_failures = 0
    failure_details: list[dict[str, Any]] = []
    with (output / "cargo.stdout.log").open("xb") as stdout, (output / "cargo.stderr.log").open("xb") as stderr:
        process = subprocess.Popen(argv, cwd=cwd, env=environment, stdout=stdout, stderr=stderr, start_new_session=True)
        timed_out = False

        def result() -> dict[str, Any]:
            return {"exit_code": process.returncode, "timed_out": timed_out,
                    "elapsed_seconds": time.monotonic() - started,
                    "owned_process_group": process.pid, "game_memory_samples": memory,
                    "game_memory_probe_attempts": probe_attempts, "game_memory_probe_failure_count": probe_failures,
                    "game_memory_probe_failures": failure_details,
                    "game_memory_probe_failure_details_omitted": probe_failures - len(failure_details),
                    "game_memory_scope": "owned hex_v4 process RSS sampled by ps at approximately 100 ms; excludes compiler, is not a separate GPU allocation measure; failed probes supply no samples"}

        def aborted(error: BaseException, cleanup_error: BaseException | None) -> RunAborted:
            details = result()
            details["original_error"] = {"type": type(error).__name__, "message": str(error)[:2048]}
            details["cleanup_error"] = None if cleanup_error is None else {
                "type": type(cleanup_error).__name__, "message": str(cleanup_error)[:2048]}
            details["cleanup_status"] = "completed" if cleanup_error is None else "failed-or-unconfirmed"
            return RunAborted(error, details)

        try:
            while True:
                remaining = timeout - (time.monotonic() - started)
                if remaining <= 0:
                    timed_out = True
                    code = process.poll()
                    break
                try:
                    code = process.wait(timeout=min(0.1, remaining))
                    break
                except subprocess.TimeoutExpired:
                    # Only optional metadata is sampled here, never other tasks' RSS.
                    if platform.system() == "Darwin" and "hex_v4" in argv and len(memory) < 100_000:
                        probe_attempts += 1
                        samples, failure = sample_game_memory(process.pid)
                        memory.extend(samples[:100_000 - len(memory)])
                        if failure is not None:
                            probe_failures += 1
                            if len(failure_details) < MAX_MEMORY_PROBE_FAILURE_DETAILS:
                                failure_details.append(failure)
        except BaseException as error:
            cleanup_error = None
            try:
                terminate_group(process)
            except BaseException as failed_cleanup:
                cleanup_error = failed_cleanup
            raise aborted(error, cleanup_error) from error
        if timed_out or code:
            original = (subprocess.TimeoutExpired(argv, timeout) if timed_out else
                        ReviewError(f"owned Cargo/game process exited with code {code}"))
            try:
                terminate_group(process)
            except BaseException as cleanup_error:
                raise aborted(original, cleanup_error) from original
        return result()


def parser() -> argparse.ArgumentParser:
    result = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    result.add_argument("--package", type=Path)
    result.add_argument("--output", type=Path, help="new output container; full-HEAD/name artifacts are created beneath it")
    result.add_argument("--name", default="capture")
    result.add_argument("--focus", help="exact q,r,level; omit when testing saved actor restoration")
    result.add_argument("--view", choices=("orbit", "top", "first", "atlas"), default="orbit")
    result.add_argument("--radius", type=int, default=56)
    result.add_argument("--parties", type=int, default=2)
    result.add_argument("--azimuth", type=float, default=35.0)
    result.add_argument("--frames", type=int, default=3600)
    result.add_argument("--settle-frames", type=int, default=120,
                        help="required consecutive fully settled updates, independent of the overall deadline")
    result.add_argument("--walk", type=Path)
    result.add_argument("--save", type=Path)
    result.add_argument("--profile", choices=("release", "map-test"), default="release")
    result.add_argument("--target-dir", type=Path, help="reuse an explicitly coordinated Cargo target")
    result.add_argument("--timeout-seconds", type=float, default=1800.0)
    result.add_argument("--dirty-diagnostic", action="store_true", help="allow changing checkout inputs only as UNAPPROVABLE-DIRTY diagnostic evidence; mid-run changes still fail")
    result.add_argument("--self-test", action="store_true")
    return result


def check_options(options: argparse.Namespace) -> None:
    if options.package is None or options.output is None:
        raise ReviewError("--package and --output are required")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,79}", options.name):
        raise ReviewError("--name must be a short path-safe matrix identifier")
    if not 16 <= options.radius <= 224 or not 1 <= options.parties <= 7 or (options.radius > 96 and options.parties != 1):
        raise ReviewError("radius must be 16..224; radius above 96 requires one party")
    if not 1 <= options.frames <= 100_000 or not math.isfinite(options.azimuth):
        raise ReviewError("invalid frame budget or azimuth")
    if not 12 <= options.settle_frames <= 10_000 or options.frames <= options.settle_frames:
        raise ReviewError("settle frames must be 12..10000 and smaller than the overall --frames deadline")
    if not math.isfinite(options.timeout_seconds) or not 1 <= options.timeout_seconds <= 7200:
        raise ReviewError("timeout must be 1..7200 seconds")
    if options.focus is not None:
        if not re.fullmatch(r"-?\d+,-?\d+,-?\d+", options.focus):
            raise ReviewError("focus must be exact q,r,level integers")
        q, r, level = map(int, options.focus.split(","))
        if not (-2**63 <= q < 2**63 and -2**63 <= r < 2**63 and -2**31 <= level < 2**31):
            raise ReviewError("focus exceeds i64 horizontal or i32 level range")


def cargo_command(options: argparse.Namespace, output: Path, cargo: str = "cargo") -> list[str]:
    command = [cargo, "run", "--locked"]
    command += ["--release"] if options.profile == "release" else ["--profile", "map-test"]
    command += ["-p", "hex_game", "--features", "v4-world", "--bin", "hex_v4", "--",
                "--world", str(options.package.resolve()), "--capture", str(output / "capture.png"),
                "--view", options.view, "--radius", str(options.radius), "--parties", str(options.parties),
                "--azimuth", str(options.azimuth), "--frames", str(options.frames),
                "--settle-frames", str(options.settle_frames)]
    for flag, value in (("--focus", options.focus), ("--walk", options.walk), ("--save", options.save)):
        if value is not None:
            command += [flag, str(value.resolve()) if isinstance(value, Path) else value]
    return command


def capture(options: argparse.Namespace) -> int:
    driver_started = time.monotonic()
    check_options(options)
    if os.name != "posix":
        raise ReviewError("this driver requires POSIX process-group containment; no game was launched")
    container = options.output.resolve()
    if container.exists():
        raise ReviewError(f"output already exists; refusing stale or overwrite-prone evidence: {container}")
    if container.is_relative_to(ROOT):
        relative = container.relative_to(ROOT)
        ignored = subprocess.run(["git", "check-ignore", "--quiet", "--", str(relative)], cwd=ROOT).returncode == 0
        if not ignored or relative.parts[0] in {"assets", "crates", "tools", ".cargo"}:
            raise ReviewError("output inside the checkout must be ignored capture/build output, not source")
    before = source_snapshot()
    identity = before["head"] + ("-dirty-" + before["sha256"][:16] if before["dirty"] else "")
    container.mkdir(parents=True, exist_ok=False)
    output = container / identity / options.name
    output.mkdir(parents=True, exist_ok=False)
    eligibility = "UNAPPROVABLE-DIRTY" if options.dirty_diagnostic else (
        "UNAPPROVABLE-DIAGNOSTIC-PROFILE" if options.profile == "map-test" else "COMMITTED-UNREVIEWED"
    )
    environment = dict(os.environ)
    environment["BEVY_ASSET_ROOT"] = str(ROOT)
    environment["CARGO_INCREMENTAL"] = "0"
    if options.target_dir is not None:
        environment["CARGO_TARGET_DIR"] = str(options.target_dir.resolve())
    command = cargo_command(options, output, shutil.which("cargo") or "cargo")
    mac_time = platform.system() == "Darwin" and Path("/usr/bin/time").is_file()
    argv = (["/usr/bin/time", "-l"] if mac_time else []) + command
    receipt: dict[str, Any] = {
        "version": 1, "status": "INCOMPLETE", "eligibility": eligibility,
        "visual_review": "UNREVIEWED", "native_motion": "HUMAN-MOTION-PENDING",
        "git_head": before["head"], "source_sha256": before["sha256"], "dirty": before["dirty"],
        "profile": options.profile, "matrix_name": options.name, "output": str(output),
        "settle_frames_requested": options.settle_frames, "overall_frame_deadline": options.frames,
        "cargo_argv": command, "timed_argv": argv, "cwd": str(ROOT),
        "environment": {key: value for key, value in environment.items() if key in RELEVANT_ENV or key.startswith(("CARGO_PROFILE_", "WGPU_"))},
        "platform": {"os": platform.system(), "release": platform.release(), "architecture": platform.machine(), "python": platform.python_version()},
        "timeout_seconds": options.timeout_seconds, "started_unix_seconds": time.time(),
        "exit_code": None, "peak_rss_bytes": None,
        "rss_scope": "macOS /usr/bin/time -l around cargo run, including compilation; not GPU or game-only memory" if mac_time else "unavailable: macOS time-l measurement not available",
    }
    atomic_json(output / "source-before.json", before)
    atomic_json(output / "incomplete.json", receipt)
    print(f"Windowless capture: {shlex.join(command)}\nArtifacts: {output}", flush=True)
    try:
        if before["dirty"] and not options.dirty_diagnostic:
            raise ReviewError("committed capture requires clean tracked and untracked source; use --dirty-diagnostic only for unapprovable scratch")
        package = package_snapshot(options.package.resolve())
        receipt["package_before"] = package
        walk_data = bounded_bytes(options.walk.resolve(), MAX_SCRIPT) if options.walk is not None else None
        walk = parse_walk(walk_data) if walk_data is not None else None
        if walk_data is not None:
            receipt["walk_source"] = {"path": str(options.walk.resolve()), "sha256": hashlib.sha256(walk_data).hexdigest(), "id": walk["id"], "steps": len(walk["steps"])}
        result = run_process(argv, ROOT, environment, output, options.timeout_seconds)
        receipt.update(result)
        stderr = bounded_bytes(output / "cargo.stderr.log", MAX_LOG).decode(errors="replace")
        receipt["peak_rss_bytes"] = peak_rss(stderr) if mac_time else None
        if result["timed_out"] or result["exit_code"] != 0:
            raise ReviewError(f"Cargo/game failed: exit={result['exit_code']} timeout={result['timed_out']}")
        if mac_time and receipt["peak_rss_bytes"] is None:
            raise ReviewError("macOS time-l completed without a usable peak RSS measurement")
        logs = stderr + bounded_bytes(output / "cargo.stdout.log", MAX_LOG).decode(errors="replace")
        if re.search(r"Path not found|failed to load[^\n]*(?:asset|font|shader)|V4 (?:world|capture) failed", logs, re.IGNORECASE):
            raise ReviewError("capture logs contain an asset or world/capture failure")
        game = strict_json(output / "capture.json")
        validate_receipt(game, package, walk, options.settle_frames)
        samples = [sample for sample in result.get("game_memory_samples", []) if sample["pid"] == game.get("process_id")]
        if samples:
            end = game.get("captured_unix_seconds", 0)
            duration = sum(game["frame_samples_ms"][-game["settled_frames"]:]) / 1000.0
            settled = [sample["rss_bytes"] for sample in samples if end - duration <= sample["unix_seconds"] <= end]
            receipt["game_memory_summary"] = {
                "process_id": game["process_id"], "samples": len(samples),
                "sampled_peak_rss_bytes": max(sample["rss_bytes"] for sample in samples),
                "settled_samples": len(settled), "settled_window_seconds": duration,
                "settled_min_rss_bytes": min(settled) if settled else None,
                "settled_max_rss_bytes": max(settled) if settled else None,
                "settled_last_rss_bytes": settled[-1] if settled else None,
            }
        coverage = png_coverage(output / "capture.png")
        after = source_snapshot()
        atomic_json(output / "source-after.json", after)
        if before != after:
            raise ReviewError("source or Git identity changed during the capture")
        if package_snapshot(options.package.resolve()) != package:
            raise ReviewError("requested package/workspace identity changed during the capture")
        if walk_data is not None and bounded_bytes(options.walk.resolve(), MAX_SCRIPT) != walk_data:
            raise ReviewError("supplied walk source changed during the capture")
        receipt.update(status="CAPTURED-UNREVIEWED", package_identity={"path": game["package"], "world_fingerprint": game["world_fingerprint"]},
                       game_receipt=game, coverage=coverage,
                       artifacts={name: file_record(output / name) for name in ("capture.png", "capture.json", "cargo.stdout.log", "cargo.stderr.log")},
                       completed_unix_seconds=time.time(), driver_elapsed_seconds=time.monotonic() - driver_started)
        atomic_json(output / "review-receipt.json", receipt)
        (output / "incomplete.json").unlink()
        print(f"CAPTURED-UNREVIEWED ({eligibility}): {output / 'review-receipt.json'}")
        return 0
    except BaseException as error:
        if isinstance(error, RunAborted):
            receipt.update(error.result)
        # Never leave two authoritative outcomes if post-publication cleanup failed.
        (output / "review-receipt.json").unlink(missing_ok=True)
        receipt.update(status="FAILED", error=f"{type(error).__name__}: {error}", failed_unix_seconds=time.time(),
                       driver_elapsed_seconds=time.monotonic() - driver_started)
        atomic_json(output / "failure.json", receipt)
        atomic_json(output / "incomplete.json", receipt)
        print(f"FAILED: {error}\nFailure record: {output / 'failure.json'}", file=sys.stderr)
        return 130 if isinstance(error, KeyboardInterrupt) or isinstance(error, RunAborted) and error.interrupted else 1


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    options = parser().parse_args(arguments)
    if options.self_test:
        if arguments != ["--self-test"]:
            raise ReviewError("--self-test must be used alone")
        return self_test()
    def interrupted(signum: int, _frame: Any) -> None:
        raise KeyboardInterrupt(f"signal {signum}")
    previous = signal.signal(signal.SIGTERM, interrupted)
    try:
        return capture(options)
    finally:
        signal.signal(signal.SIGTERM, previous)


def self_test() -> int:
    """Lightweight parser/provenance/PNG/timeout tests; no Cargo, GPU or window."""
    import unittest
    from unittest.mock import Mock, patch
    from contextlib import redirect_stderr, redirect_stdout
    import copy
    import io

    def game_receipt(package: str = "/world") -> dict[str, Any]:
        return {"package": package, "world_fingerprint": "0000000000000001", "frames": 130,
                "settled_frames": 120,
                "resident_chunks": 1, "rendered_chunks": 1, "mesh_publications": 1, "rendered_vertices": 12,
                "discarded_mesh_jobs": 0, "local_queue_peak": 1, "rebase_samples_ms": [],
                "elapsed_seconds": 0.1, "frame_samples_ms": [10.0, 20.0, 30.0],
                "static_review": "UNREVIEWED", "native_motion": "HUMAN-MOTION-PENDING", "scripted_walk": None}

    class DriverTests(unittest.TestCase):
        def test_only_explicit_windowless_cargo_commands(self) -> None:
            options = parser().parse_args(["--package", ".", "--output", "/tmp/fresh", "--radius", "224", "--parties", "1"])
            check_options(options)
            argv = cargo_command(options, Path("/tmp/fresh"))
            self.assertEqual(argv[:3], ["cargo", "run", "--locked"])
            self.assertIn("--release", argv)
            self.assertEqual(argv[argv.index("--bin") + 1], "hex_v4")
            self.assertEqual(argv[argv.index("--features") + 1], "v4-world")
            self.assertEqual(argv[argv.index("--capture") + 1], "/tmp/fresh/capture.png")
            self.assertEqual(argv[argv.index("--settle-frames") + 1], "120")
            options.settle_frames = 600
            self.assertEqual(cargo_command(options, Path("/tmp/fresh"))[-1], "600")
            options.profile = "map-test"
            self.assertIn("map-test", cargo_command(options, Path("/tmp/fresh")))
            options.parties = 2
            with self.assertRaises(ReviewError):
                check_options(options)
            options.parties = 1
            for settle in (11, 10_001, options.frames):
                options.settle_frames = settle
                with self.assertRaises(ReviewError):
                    check_options(options)

        def test_walk_reader_binds_full_commands_and_rejects_duplicates(self) -> None:
            source = b'(schema_version:1,id:"walk",max_ticks:20,steps:[MoveTo(actor:"a",goal:(column:(q:1,r:0),level:2)),WaitAt(actor:"a",position:(column:(q:1,r:0),level:2),max_ticks:10)])'
            walk = parse_walk(source)
            self.assertEqual(walk["steps"][0]["MoveTo"]["goal"]["column"], {"q": 1, "r": 0})
            with self.assertRaises(ReviewError):
                parse_walk(source.replace(b'id:"walk"', b'id:"walk",id:"other"'))
            with self.assertRaises(ReviewError):
                parse_walk(source[:-1])
            for path in (ROOT / "assets/config/v4/walks").glob("*.ron"):
                self.assertTrue(parse_walk(path.read_bytes())["steps"], path.name)

        def test_zero_work_or_wrong_package_receipts_fail(self) -> None:
            package = {"requested_directory": "/world", "fingerprint": "0000000000000001"}
            receipt = game_receipt()
            validate_receipt(receipt, package, None)
            with self.assertRaises(ReviewError):
                validate_receipt({**receipt, "rendered_vertices": 0}, package, None)
            with self.assertRaises(ReviewError):
                validate_receipt({**receipt, "world_fingerprint": "0" * 16}, package, None)
            with self.assertRaises(ReviewError):
                validate_receipt({**receipt, "frame_samples_ms": [float("nan")]}, package, None)
            with self.assertRaises(ReviewError):
                validate_receipt({**receipt, "settled_frames": 119}, package, None)
            with self.assertRaises(ReviewError):
                validate_receipt(receipt, package, None, 600)
            validate_receipt({**receipt, "frames": 730, "settled_frames": 600}, package, None, 600)
            with self.assertRaises(ReviewError):
                validate_receipt({**receipt, "settled_frames": 131}, package, None)

        def test_walk_receipts_require_complete_settled_changed_support_evidence(self) -> None:
            package = {"requested_directory": "/world", "fingerprint": "0000000000000001"}
            start = {"column": {"q": 0, "r": 0}, "level": 2}
            goal = {"column": {"q": 1, "r": 0}, "level": 2}
            steps = [{"MoveTo": {"actor": "a", "goal": goal}},
                     {"AssertAt": {"actor": "a", "position": goal}}]
            walk = {"schema_version": 1, "id": "w", "max_ticks": 10, "steps": steps}
            actor = {"id": "a", "position": start, "motion_to": None, "motion_fraction": None,
                     "pending_goal": goal, "queued_steps": 0, "turn_steps": False}
            first = {"script": "w", "script_fingerprint": "1234567890abcdef", "step": 0, "tick": 1,
                     "command": steps[0], "evidence": "AUTOMATED-TYPED-MOTION",
                     "native_motion": "HUMAN-MOTION-PENDING", "verified_moves": 0, "actors": [actor]}
            last = {**first, "step": 1, "tick": 3, "command": steps[1], "verified_moves": 1,
                    "actors": [{**actor, "position": goal, "pending_goal": None}]}
            receipt = {**game_receipt(), "scripted_walk": [first, last]}
            validate_receipt(receipt, package, walk)
            for mutate in (
                lambda rows: rows.pop(),
                lambda rows: rows[1].update(verified_moves=0),
                lambda rows: rows[1].update(tick=1),
                lambda rows: rows[1]["actors"][0].update(motion_to=start, motion_fraction=0.2),
                lambda rows: rows[1]["actors"][0].update(pending_goal=start),
                lambda rows: rows[1]["actors"][0].update(queued_steps=3),
                lambda rows: rows[1]["actors"][0].update(id="wrong-actor"),
                lambda rows: rows[0]["actors"][0].update(position=goal),
                lambda rows: rows[1].update(command={"AssertAt": {"actor": "a", "position": start}}),
            ):
                bad = copy.deepcopy(receipt)
                mutate(bad["scripted_walk"])
                with self.assertRaises(ReviewError):
                    validate_receipt(bad, package, walk)

            remove = {"RemoveObject": {"position": goal, "object_id": "tree"}}
            wait = {"WaitObject": {"column": goal["column"], "object_id": "tree", "present": False, "max_ticks": 5}}
            walk["steps"] = [*steps, remove, wait]
            removing = {**last, "step": 2, "tick": 4, "command": remove,
                        "successful_object_edits": 0, "observed_object_present": True,
                        "pending_object_request": "tree"}
            removed = {**last, "step": 3, "tick": 5, "command": wait,
                       "successful_object_edits": 1, "observed_object_present": False,
                       "observed_revision": 1, "pending_object_request": None,
                       "object_removal_pending": False, "cancel_object_edit_pending": False}
            receipt["scripted_walk"] = [first, last, removing, removed]
            validate_receipt(receipt, package, walk)
            for invalid in ({"successful_object_edits": 0}, {"observed_object_present": None},
                            {"observed_revision": None}, {"object_removal_pending": True}):
                bad = copy.deepcopy(receipt)
                bad["scripted_walk"][-1].update(invalid)
                with self.assertRaises(ReviewError):
                    validate_receipt(bad, package, walk)

        def test_strict_json_rejects_duplicate_and_nonfinite_receipts(self) -> None:
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "receipt.json"
                for text in ('{"frames":1,"frames":2}', '{"frames":NaN}', '[]'):
                    path.write_text(text)
                    with self.assertRaises(ReviewError):
                        strict_json(path)

        def test_package_pointer_is_confined_and_content_changes_change_identity(self) -> None:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory).resolve()
                manifest = root / "packages/0000000000000001/manifest.ron"
                manifest.parent.mkdir(parents=True)
                manifest.write_text('(world_id:"test",fingerprint:1)')
                pointer = root / "current.ron"
                pointer.write_text('(manifest_path:"packages/0000000000000001/manifest.ron")')
                before = package_snapshot(root)
                self.assertEqual(before["fingerprint"], "0000000000000001")
                manifest.write_text('(world_id:"changed",fingerprint:1)')
                self.assertNotEqual(before, package_snapshot(root))
                pointer.write_text('(manifest_path:"../manifest.ron")')
                with self.assertRaises(ReviewError):
                    package_snapshot(root)

        def test_stale_output_dirty_source_and_midrun_changes_never_publish(self) -> None:
            # Substitute process/artifact generation only to exercise final publication.
            # No Cargo, game, renderer or real capture is started by this test.
            module = sys.modules[__name__]
            before = {"head": "a" * 40, "dirty": False, "sha256": "b" * 64}
            with tempfile.TemporaryDirectory() as directory:
                base = Path(directory).resolve()
                package = base / "world"
                package.mkdir()
                (package / "manifest.ron").write_text("(fingerprint:1)")
                options = parser().parse_args(["--package", str(package), "--output", str(base / "run")])
                def fake_process(_argv: Any, _cwd: Any, _environment: Any, output: Path, _timeout: Any) -> dict[str, Any]:
                    (output / "cargo.stdout.log").write_text("fixture only\n")
                    (output / "cargo.stderr.log").write_text(" 12345 maximum resident set size\n")
                    atomic_json(output / "capture.json", game_receipt(str(package)))
                    (output / "capture.png").write_bytes(b"synthetic fixture; independent PNG tests cover decoding")
                    return {"exit_code": 0, "timed_out": False, "elapsed_seconds": 0.1}
                with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                    with patch.object(module, "source_snapshot", side_effect=[before, before]), \
                         patch.object(module, "run_process", side_effect=fake_process) as run, \
                         patch.object(module, "png_coverage", return_value={"has_coverage": True}):
                        self.assertEqual(capture(options), 0)
                        run.assert_called_once()
                    published = base / "run" / before["head"] / "capture"
                    self.assertTrue((published / "review-receipt.json").is_file())
                    self.assertFalse((published / "incomplete.json").exists())
                    with patch.object(module, "run_process") as run:
                        with self.assertRaises(ReviewError):
                            capture(options)
                        run.assert_not_called()
                    options.output = base / "changed-source"
                    after = {**before, "sha256": "c" * 64}
                    with patch.object(module, "source_snapshot", side_effect=[before, after]), \
                         patch.object(module, "run_process", side_effect=fake_process), \
                         patch.object(module, "png_coverage", return_value={"has_coverage": True}):
                        self.assertEqual(capture(options), 1)
                    failed = options.output / before["head"] / "capture"
                    self.assertFalse((failed / "review-receipt.json").exists())
                    self.assertEqual(strict_json(failed / "failure.json")["status"], "FAILED")
                    options.output = base / "dirty-source"
                    with patch.object(module, "source_snapshot", return_value={**before, "dirty": True}), \
                         patch.object(module, "run_process") as run:
                        self.assertEqual(capture(options), 1)
                        run.assert_not_called()

        def test_png_coverage_rejects_flat_black_corrupt_and_wrong_size(self) -> None:
            def chunk(kind: bytes, payload: bytes) -> bytes:
                return struct.pack(">I", len(payload)) + kind + payload + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
            def png(colorful: bool) -> bytes:
                rows = b"".join(b"\0" + b"".join(bytes((240, 100, 30) if colorful and x % 2 else (20, 30, 40)) for x in range(32)) for _ in range(16))
                return PNG_SIGNATURE + chunk(b"IHDR", struct.pack(">IIBBBBB", 32, 16, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
            with tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / "test.png"
                path.write_bytes(png(True))
                self.assertTrue(png_coverage(path, (32, 16))["has_coverage"])
                with self.assertRaises(ReviewError):
                    png_coverage(path)
                path.write_bytes(png(False))
                with self.assertRaises(ReviewError):
                    png_coverage(path, (32, 16))
                path.write_bytes(png(True)[:-1] + b"x")
                with self.assertRaises(ReviewError):
                    png_coverage(path, (32, 16))

        def test_untracked_and_ignored_relevant_source_changes_are_hashed(self) -> None:
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                subprocess.run(["git", "init", "--quiet", str(root)], check=True)
                (root / ".gitignore").write_text("assets/ignored.ron\n.context/\n")
                subprocess.run(["git", "add", ".gitignore"], cwd=root, check=True)
                subprocess.run(["git", "-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid", "commit", "--quiet", "-m", "fixture"], cwd=root, check=True)
                first = source_snapshot(root)
                self.assertFalse(first["dirty"])
                (root / "assets").mkdir()
                path = root / "assets/ignored.ron"
                path.write_text("first")
                second = source_snapshot(root)
                self.assertTrue(second["dirty"])
                self.assertNotEqual(first["sha256"], second["sha256"])
                path.write_text("second")
                self.assertNotEqual(second["sha256"], source_snapshot(root)["sha256"])

        def test_timeout_stops_an_owned_process_and_preserves_logs(self) -> None:
            if os.name != "posix":
                self.skipTest("POSIX process-group driver")
            with tempfile.TemporaryDirectory() as directory:
                result = run_process([sys.executable, "-c", "import time; time.sleep(3)"], ROOT, dict(os.environ), Path(directory), 0.05)
                self.assertTrue(result["timed_out"])
                self.assertNotEqual(result["exit_code"], 0)
                self.assertTrue((Path(directory) / "cargo.stderr.log").exists())
            self.assertEqual(peak_rss("   123456 maximum resident set size\n"), 123456)
            self.assertIsNone(peak_rss("no measurement"))

        def test_optional_memory_probe_failures_leave_owned_process_running(self) -> None:
            # A real small child must finish its work despite all three probe failures.
            failures = [subprocess.TimeoutExpired("ps", 2), PermissionError(1, "probe denied"),
                        subprocess.CompletedProcess(["ps"], 1, b"", b"metadata unavailable")]

            def probe(*args: Any, **kwargs: Any) -> subprocess.CompletedProcess[bytes]:
                self.assertEqual(args[0][0], "ps")
                outcome = failures.pop(0) if failures else subprocess.CompletedProcess(["ps"], 1, b"", b"unavailable")
                if isinstance(outcome, BaseException):
                    raise outcome
                return outcome

            with tempfile.TemporaryDirectory() as directory, \
                 patch.object(platform, "system", return_value="Darwin"), \
                 patch.object(subprocess, "run", side_effect=probe), \
                 patch.object(sys.modules[__name__], "terminate_group") as cleanup:
                record = run_process([sys.executable, "-c", "import time; time.sleep(0.55); print('owned-work-completed')", "hex_v4"],
                                     ROOT, dict(os.environ), Path(directory), 5)
                self.assertEqual(record["exit_code"], 0)
                self.assertFalse(record["timed_out"])
                self.assertIn("owned-work-completed", (Path(directory) / "cargo.stdout.log").read_text())
                self.assertEqual(record["game_memory_samples"], [])
                self.assertGreaterEqual(record["game_memory_probe_failure_count"], 3)
                self.assertEqual([failure["type"] for failure in record["game_memory_probe_failures"][:3]],
                                 ["TimeoutExpired", "PermissionError", "NonzeroExit"])
                cleanup.assert_not_called()

        def test_memory_probe_failure_details_are_bounded(self) -> None:
            owned = Mock(pid=4242, returncode=None)
            remaining = MAX_MEMORY_PROBE_FAILURE_DETAILS + 4

            def wait(**kwargs: Any) -> int:
                nonlocal remaining
                if remaining:
                    remaining -= 1
                    raise subprocess.TimeoutExpired("owned", 0.1)
                owned.returncode = 0
                return 0

            owned.wait.side_effect = wait
            with tempfile.TemporaryDirectory() as directory, \
                 patch.object(subprocess, "Popen", return_value=owned), \
                 patch.object(platform, "system", return_value="Darwin"), \
                 patch.object(subprocess, "run", side_effect=OSError("x" * 2000)):
                record = run_process(["hex_v4"], ROOT, {}, Path(directory), 5)
            self.assertEqual(record["exit_code"], 0)
            self.assertEqual(record["game_memory_probe_failure_count"], MAX_MEMORY_PROBE_FAILURE_DETAILS + 4)
            self.assertEqual(len(record["game_memory_probe_failures"]), MAX_MEMORY_PROBE_FAILURE_DETAILS)
            self.assertEqual(record["game_memory_probe_failure_details_omitted"], 4)
            self.assertTrue(all(len(row["message"]) <= MAX_MEMORY_PROBE_DETAIL for row in record["game_memory_probe_failures"]))
            self.assertEqual(record["game_memory_samples"], [])

        def test_cleanup_failure_preserves_original_error_and_owned_identity(self) -> None:
            original = RuntimeError("original wait failure")
            owned = Mock(pid=4242, returncode=None)
            owned.wait.side_effect = original
            with tempfile.TemporaryDirectory() as directory, \
                 patch.object(subprocess, "Popen", return_value=owned), \
                 patch.object(sys.modules[__name__], "terminate_group", side_effect=PermissionError(1, "cleanup denied")) as cleanup:
                with self.assertRaises(RunAborted) as caught:
                    run_process(["owned"], ROOT, {}, Path(directory), 5)
                cleanup.assert_called_once_with(owned)
            self.assertIs(caught.exception.__cause__, original)
            self.assertEqual(caught.exception.result["original_error"]["message"], "original wait failure")
            self.assertEqual(caught.exception.result["cleanup_error"]["type"], "PermissionError")
            self.assertEqual(caught.exception.result["cleanup_status"], "failed-or-unconfirmed")
            self.assertEqual(caught.exception.result["owned_process_group"], 4242)
            self.assertIsNone(caught.exception.result["exit_code"])

        def test_nonzero_exit_and_timeout_survive_failed_cleanup(self) -> None:
            for timeout, exit_code in [(5, 17), (0, None)]:
                with self.subTest(timeout=timeout), tempfile.TemporaryDirectory() as directory:
                    owned = Mock(pid=4242, returncode=exit_code)
                    owned.wait.return_value = exit_code
                    owned.poll.return_value = exit_code
                    with patch.object(subprocess, "Popen", return_value=owned), \
                         patch.object(sys.modules[__name__], "terminate_group", side_effect=PermissionError(1, "cleanup denied")) as cleanup:
                        with self.assertRaises(RunAborted) as caught:
                            run_process(["owned"], ROOT, {}, Path(directory), timeout)
                        cleanup.assert_called_once_with(owned)
                    record = caught.exception.result
                    self.assertEqual(record["exit_code"], exit_code)
                    self.assertEqual(record["timed_out"], timeout == 0)
                    self.assertEqual(record["original_error"]["type"], "TimeoutExpired" if timeout == 0 else "ReviewError")
                    self.assertEqual(record["cleanup_error"]["type"], "PermissionError")

        def test_failed_group_probe_cannot_claim_cleanup_complete(self) -> None:
            for failure in [PermissionError(1, "denied"), subprocess.TimeoutExpired("ps", 2)]:
                with self.subTest(failure=type(failure).__name__), patch.object(subprocess, "run", side_effect=failure):
                    self.assertFalse(group_is_gone(4242))

        def test_memory_samples_exclude_compilers_and_unrelated_games(self) -> None:
            rows = "42 81 100 /some workspace/map-test/hex_v4\n42 82 900 /bin/rustc\n43 83 500 /other/hex_v4\n42 84 200 /bin/hex_v4-helper\n42 bad 200 /bin/hex_v4\n"
            self.assertEqual(parse_game_memory(rows, 42, 3.0), [
                {"unix_seconds": 3.0, "pid": 81, "rss_bytes": 102400}
            ])

    result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(DriverTests))
    return 0 if result.wasSuccessful() else 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ReviewError, OSError, ValueError) as error:
        print(f"v4-review: {error}", file=sys.stderr)
        raise SystemExit(1) from error
