#!/usr/bin/env python3
"""Build isolated, deterministic Grand V3 visual-experiment review packs.

The harness never edits tracked assets.  Each profile receives a fresh copy of the
complete asset tree, each capture runs in a fresh process with isolated local data,
and the requested output directory appears only after every capture and sidecar has
validated.  Use ``run --dry-run`` to inspect the exact matrix without staging assets
or invoking Cargo.
"""

from __future__ import annotations

import argparse
import ctypes
import errno
import hashlib
import json
import os
import pathlib
import re
import shutil
import signal
import stat
import struct
import subprocess
import sys
import tempfile
import zlib
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = (
    REPOSITORY_ROOT / "tools" / "visual_experiments" / "profiles.json"
)
EXPERIMENT_ROOT = (
    REPOSITORY_ROOT / ".context" / "grand-v3-visual-experiments"
)
EXPECTED_PROFILE_IDS = (
    "e00-baseline",
    "l01-midnight",
    "l02-dawn",
    "l03-golden",
    "l04-overcast",
    "h01-flat-030",
    "h02-tall-055",
    "p01-muted-earth",
    "p02-high-separation",
)
CANONICAL_BASELINE_PATHS = {
    "world": "assets/config/worlds/procedural-grand-v3-baseline.ron",
    "palette": "assets/art/palette.ron",
    "scenarios": "assets/config/scenarios.ron",
    "default_lighting": "assets/config/lighting.ron",
    "overcast_lighting": "assets/config/lighting/overcast.ron",
}
PROFILE_ID_RE = re.compile(r"^[a-z][a-z0-9]*(?:-[a-z0-9]+)*$")
CAPTURE_ID_RE = re.compile(r"^[0-9]{2}-[a-z0-9]+(?:-[a-z0-9]+)*$")
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
HEIGHT_RE = re.compile(
    r"^(?P<indent>\s*)level_height:\s*(?P<value>[0-9]+(?:\.[0-9]+)?),\s*$",
    re.MULTILINE,
)
PALETTE_COLOR_RE = re.compile(
    r'(?P<prefix>^\s*"(?P<id>[^"]+)":\s*\(\s*\n'
    r'\s*display_name:\s*"[^"]+",\s*\n'
    r"(?P<color_indent>\s*)color:\s*)"
    r"\(red:\s*(?P<red>[0-9]+(?:\.[0-9]+)?),\s*"
    r"green:\s*(?P<green>[0-9]+(?:\.[0-9]+)?),\s*"
    r"blue:\s*(?P<blue>[0-9]+(?:\.[0-9]+)?)\)",
    re.MULTILINE,
)
PNG_SIGNATURE = b"\x89PNG\r\n\x1a\n"
CAPTURE_WIDTH = 1920
CAPTURE_HEIGHT = 1080
BEHAVIOR_ENV_EXACT = {
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTDOCFLAGS",
    "RUSTFLAGS",
    "RUSTUP_TOOLCHAIN",
}
BEHAVIOR_ENV_PREFIXES = (
    "BEVY_",
    "CARGO_PROFILE_",
    "HEX_",
    "WGPU_",
)


class ExperimentError(ValueError):
    """An experiment definition or publication is unsafe or incomplete."""


class CaptureError(RuntimeError):
    """A configured runtime capture or final publication failed."""


@dataclass(frozen=True)
class CaptureSpec:
    id: str
    filename: str
    camera: str
    view: str
    focus_anchor: Optional[str]


@dataclass(frozen=True)
class ProfileSpec:
    id: str
    label: str
    axis: str
    time_hours: Optional[float]
    lighting_asset: Optional[str]
    level_height: Optional[float]
    palette: Optional[str]


@dataclass(frozen=True)
class Registry:
    path: pathlib.Path
    scenario: str
    seed: int
    baseline: Mapping[str, Any]
    captures: Tuple[CaptureSpec, ...]
    profiles: Tuple[ProfileSpec, ...]

    def profile(self, profile_id: str) -> ProfileSpec:
        for profile in self.profiles:
            if profile.id == profile_id:
                return profile
        raise ExperimentError(f"unknown profile: {profile_id}")


def _strict_object(
    value: Any,
    *,
    context: str,
    allowed: Iterable[str],
    required: Iterable[str],
) -> Dict[str, Any]:
    if not isinstance(value, dict):
        raise ExperimentError(f"{context} must be an object")
    allowed_set = set(allowed)
    required_set = set(required)
    unknown = sorted(set(value) - allowed_set)
    missing = sorted(required_set - set(value))
    if unknown:
        raise ExperimentError(f"{context} has unknown fields: {unknown}")
    if missing:
        raise ExperimentError(f"{context} is missing fields: {missing}")
    return value


def _safe_relative_path(raw: Any, *, context: str) -> str:
    if not isinstance(raw, str) or not raw:
        raise ExperimentError(f"{context} must be a non-empty relative path")
    candidate = pathlib.PurePosixPath(raw)
    if candidate.is_absolute() or ".." in candidate.parts or candidate.as_posix() != raw:
        raise ExperimentError(f"{context} must be a canonical relative path: {raw!r}")
    return raw


def _existing_regular_file(
    repository_root: pathlib.Path, raw: Any, *, context: str, prefix: str
) -> str:
    relative = _safe_relative_path(raw, context=context)
    if relative != prefix and not relative.startswith(prefix + "/"):
        raise ExperimentError(f"{context} must stay under {prefix}/")
    path = repository_root / relative
    if path.is_symlink() or not path.is_file():
        raise ExperimentError(f"{context} does not name a regular file: {relative}")
    return relative


def _number(value: Any, *, context: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ExperimentError(f"{context} must be a number")
    number = float(value)
    if not (number == number and abs(number) != float("inf")):
        raise ExperimentError(f"{context} must be finite")
    return number


def _time(value: Any, *, context: str) -> float:
    hour = _number(value, context=context)
    if not 0.0 <= hour < 24.0:
        raise ExperimentError(f"{context} must be in [0, 24)")
    return hour


def _read_json(path: pathlib.Path, *, context: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ExperimentError(f"cannot load {context} {path}: {error}") from error


def parse_palette_colors(text: str) -> Dict[str, Tuple[float, float, float]]:
    """Read the intentionally narrow color surface of the shipped palette RON."""

    colors: Dict[str, Tuple[float, float, float]] = {}
    for match in PALETTE_COLOR_RE.finditer(text):
        swatch_id = match.group("id")
        if swatch_id in colors:
            raise ExperimentError(f"palette repeats swatch {swatch_id!r}")
        colors[swatch_id] = tuple(
            float(match.group(channel)) for channel in ("red", "green", "blue")
        )
    if not colors:
        raise ExperimentError("palette contains no recognized swatches")
    return colors


def load_palette_candidate(
    path: pathlib.Path, expected_swatches: Iterable[str]
) -> Tuple[str, Dict[str, Tuple[float, float, float]]]:
    raw = _strict_object(
        _read_json(path, context="palette candidate"),
        context=f"palette candidate {path.name}",
        allowed=("version", "palette_id", "colors"),
        required=("version", "palette_id", "colors"),
    )
    if raw["version"] != 1:
        raise ExperimentError(f"palette candidate {path.name} version must be 1")
    palette_id = raw["palette_id"]
    if not isinstance(palette_id, str) or not PROFILE_ID_RE.fullmatch(palette_id):
        raise ExperimentError(f"palette candidate {path.name} has invalid palette_id")
    colors_raw = raw["colors"]
    if not isinstance(colors_raw, dict):
        raise ExperimentError(f"palette candidate {path.name} colors must be an object")
    expected = set(expected_swatches)
    actual = set(colors_raw)
    if actual != expected:
        missing = sorted(expected - actual)
        extra = sorted(actual - expected)
        raise ExperimentError(
            f"palette candidate {path.name} swatches differ; missing={missing}, extra={extra}"
        )
    colors: Dict[str, Tuple[float, float, float]] = {}
    for swatch_id in sorted(colors_raw):
        value = colors_raw[swatch_id]
        if not isinstance(value, list) or len(value) != 3:
            raise ExperimentError(
                f"palette candidate {path.name} color {swatch_id!r} needs three channels"
            )
        channels = tuple(
            _number(channel, context=f"palette {palette_id} {swatch_id}")
            for channel in value
        )
        if any(channel < 0.0 or channel > 1.0 for channel in channels):
            raise ExperimentError(
                f"palette candidate {path.name} color {swatch_id!r} is outside [0, 1]"
            )
        colors[swatch_id] = channels
    return palette_id, colors


def load_registry(
    path: pathlib.Path = DEFAULT_REGISTRY,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> Registry:
    allowed_registry_root = (repository_root / "tools" / "visual_experiments").resolve()
    try:
        path.resolve().relative_to(allowed_registry_root)
    except ValueError as error:
        raise ExperimentError(
            f"profile registry must stay under {allowed_registry_root}"
        ) from error
    raw = _strict_object(
        _read_json(path, context="profile registry"),
        context="profile registry",
        allowed=("version", "scenario", "seed", "baseline", "captures", "profiles"),
        required=("version", "scenario", "seed", "baseline", "captures", "profiles"),
    )
    if raw["version"] != 1:
        raise ExperimentError("profile registry version must be 1")
    if raw["scenario"] != "Grand V3 Baseline":
        raise ExperimentError("scenario must remain 'Grand V3 Baseline'")
    if isinstance(raw["seed"], bool) or not isinstance(raw["seed"], int):
        raise ExperimentError("seed must be an integer")

    baseline = _strict_object(
        raw["baseline"],
        context="baseline",
        allowed=(
            "world",
            "palette",
            "scenarios",
            "default_lighting",
            "overcast_lighting",
            "level_height",
        ),
        required=(
            "world",
            "palette",
            "scenarios",
            "default_lighting",
            "overcast_lighting",
            "level_height",
        ),
    )
    baseline = dict(baseline)
    for field, expected_path in CANONICAL_BASELINE_PATHS.items():
        if baseline[field] != expected_path:
            raise ExperimentError(
                f"baseline.{field} must remain the canonical path {expected_path!r}"
            )
    for field in (
        "world",
        "palette",
        "scenarios",
        "default_lighting",
        "overcast_lighting",
    ):
        baseline[field] = _existing_regular_file(
            repository_root,
            baseline[field],
            context=f"baseline.{field}",
            prefix="assets",
        )
    baseline_height = _number(baseline["level_height"], context="baseline.level_height")
    if baseline_height != 0.4:
        raise ExperimentError("baseline.level_height must remain the shipped 0.4")
    baseline["level_height"] = baseline_height

    world_text = (repository_root / baseline["world"]).read_text(encoding="utf-8")
    if read_level_height(world_text) != baseline_height:
        raise ExperimentError("baseline world does not contain the declared level_height")
    palette_text = (repository_root / baseline["palette"]).read_text(encoding="utf-8")
    baseline_colors = parse_palette_colors(palette_text)
    overcast_text = (repository_root / baseline["overcast_lighting"]).read_text(
        encoding="utf-8"
    )
    if "profile: Cycle" in overcast_text:
        raise ExperimentError("overcast_lighting must remain a static lighting profile")
    scenario_text = (repository_root / baseline["scenarios"]).read_text(encoding="utf-8")
    scenario_block, _, _ = isolate_scenario_record(scenario_text, raw["scenario"])
    world_matches = re.findall(r'^\s*world:\s*"([^"]+)",\s*$', scenario_block, re.MULTILINE)
    if world_matches != [_asset_relative(baseline["world"])]:
        raise ExperimentError(
            "Grand V3 scenario must select the declared baseline world exactly"
        )
    if re.search(r"^\s*lighting:\s*", scenario_block, re.MULTILINE):
        raise ExperimentError(
            "Grand V3 baseline scenario must omit lighting and use the shipped default"
        )
    seed_matches = re.findall(
        r"^\s*generation_seed:\s*Some\(([0-9]+)\),\s*$",
        scenario_block,
        re.MULTILINE,
    )
    if seed_matches != [str(raw["seed"])]:
        raise ExperimentError(
            "Grand V3 scenario must select the declared generation seed exactly"
        )

    captures_raw = raw["captures"]
    if not isinstance(captures_raw, list) or not captures_raw:
        raise ExperimentError("captures must be a non-empty list")
    captures: List[CaptureSpec] = []
    capture_ids = set()
    filenames = set()
    for index, value in enumerate(captures_raw):
        capture_raw = _strict_object(
            value,
            context=f"captures[{index}]",
            allowed=("id", "filename", "camera", "view", "focus_anchor"),
            required=("id", "filename", "camera", "view"),
        )
        capture_id = capture_raw["id"]
        if not isinstance(capture_id, str) or not CAPTURE_ID_RE.fullmatch(capture_id):
            raise ExperimentError(f"captures[{index}].id is invalid")
        if capture_id in capture_ids:
            raise ExperimentError(f"duplicate capture id: {capture_id}")
        capture_ids.add(capture_id)
        filename = _safe_relative_path(
            capture_raw["filename"], context=f"captures[{index}].filename"
        )
        if pathlib.PurePosixPath(filename).parent != pathlib.PurePosixPath("."):
            raise ExperimentError("capture filenames must be plain basenames")
        if not filename.endswith(".png") or filename in filenames:
            raise ExperimentError(f"invalid or duplicate capture filename: {filename}")
        filenames.add(filename)
        camera = capture_raw["camera"]
        view = capture_raw["view"]
        if camera not in ("map", "character", "first-person"):
            raise ExperimentError(f"capture {capture_id} has invalid camera")
        if view not in ("default", "rotated", "counter-rotated", "rear", "top-down"):
            raise ExperimentError(f"capture {capture_id} has invalid view")
        focus_anchor = capture_raw.get("focus_anchor")
        if focus_anchor is not None and (
            not isinstance(focus_anchor, str) or not focus_anchor
        ):
            raise ExperimentError(f"capture {capture_id} has invalid focus_anchor")
        captures.append(
            CaptureSpec(capture_id, filename, camera, view, focus_anchor)
        )
    if not any(
        capture.camera == "map"
        and capture.view == "top-down"
        and capture.focus_anchor is None
        for capture in captures
    ):
        raise ExperimentError("capture matrix needs one unfocused full-footprint top-down Map")
    if {capture.camera for capture in captures} != {"map", "character", "first-person"}:
        raise ExperimentError("capture matrix must cover Map, Character, and First Person")

    profiles_raw = raw["profiles"]
    if not isinstance(profiles_raw, list):
        raise ExperimentError("profiles must be a list")
    profiles: List[ProfileSpec] = []
    profile_ids = []
    for index, value in enumerate(profiles_raw):
        profile_raw = _strict_object(
            value,
            context=f"profiles[{index}]",
            allowed=(
                "id",
                "label",
                "axis",
                "time_hours",
                "lighting_asset",
                "level_height",
                "palette",
            ),
            required=("id", "label", "axis"),
        )
        profile_id = profile_raw["id"]
        label = profile_raw["label"]
        axis = profile_raw["axis"]
        if not isinstance(profile_id, str) or not PROFILE_ID_RE.fullmatch(profile_id):
            raise ExperimentError(f"profiles[{index}].id is invalid")
        if not isinstance(label, str) or not label:
            raise ExperimentError(f"profile {profile_id} label is invalid")
        if axis not in ("baseline", "lighting", "level_height", "palette"):
            raise ExperimentError(f"profile {profile_id} has invalid axis")
        profile_ids.append(profile_id)
        time_hours = (
            _time(profile_raw["time_hours"], context=f"profile {profile_id} time_hours")
            if "time_hours" in profile_raw
            else None
        )
        lighting_asset = profile_raw.get("lighting_asset")
        level_height = (
            _number(
                profile_raw["level_height"],
                context=f"profile {profile_id} level_height",
            )
            if "level_height" in profile_raw
            else None
        )
        palette = profile_raw.get("palette")
        if axis == "baseline":
            if time_hours != 12.0 or any(
                item is not None for item in (lighting_asset, level_height, palette)
            ):
                raise ExperimentError(
                    f"baseline profile {profile_id} must only set noon time_hours"
                )
        elif axis == "lighting":
            if level_height is not None or palette is not None:
                raise ExperimentError(f"lighting profile {profile_id} mixes experiment axes")
            if (time_hours is None) == (lighting_asset is None):
                raise ExperimentError(
                    f"lighting profile {profile_id} needs exactly one time or static asset"
                )
            if lighting_asset is not None:
                lighting_asset = _existing_regular_file(
                    repository_root,
                    lighting_asset,
                    context=f"profile {profile_id} lighting_asset",
                    prefix="assets",
                )
                if lighting_asset != baseline["overcast_lighting"]:
                    raise ExperimentError(
                        f"lighting profile {profile_id} may only select shipped overcast"
                    )
        elif axis == "level_height":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or palette is not None
                or level_height not in (0.30, 0.55)
            ):
                raise ExperimentError(f"height profile {profile_id} mixes or changes axes")
        else:
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or level_height is not None
                or palette is None
            ):
                raise ExperimentError(f"palette profile {profile_id} mixes experiment axes")
            palette = _existing_regular_file(
                repository_root,
                palette,
                context=f"profile {profile_id} palette",
                prefix="tools/visual_experiments/palettes",
            )
            palette_id, _ = load_palette_candidate(
                repository_root / palette, baseline_colors
            )
            if palette_id != profile_id:
                raise ExperimentError(
                    f"palette {palette} id {palette_id!r} does not match {profile_id!r}"
                )
        profiles.append(
            ProfileSpec(
                profile_id,
                label,
                axis,
                time_hours,
                lighting_asset,
                level_height,
                palette,
            )
        )
    if tuple(profile_ids) != EXPECTED_PROFILE_IDS:
        raise ExperimentError(
            "profiles must be the canonical ordered nine-profile matrix; got "
            + repr(tuple(profile_ids))
        )
    return Registry(
        path=path,
        scenario=raw["scenario"],
        seed=raw["seed"],
        baseline=baseline,
        captures=tuple(captures),
        profiles=tuple(profiles),
    )


def read_level_height(text: str) -> float:
    matches = list(HEIGHT_RE.finditer(text))
    if len(matches) != 1:
        raise ExperimentError(
            f"world must contain exactly one level_height field; found {len(matches)}"
        )
    return float(matches[0].group("value"))


def _format_float(value: float) -> str:
    result = f"{value:.6f}".rstrip("0").rstrip(".")
    return result if "." in result else result + ".0"


def replace_level_height(text: str, value: float) -> str:
    if len(list(HEIGHT_RE.finditer(text))) != 1:
        read_level_height(text)
    return HEIGHT_RE.sub(
        lambda match: f"{match.group('indent')}level_height: {value:.2f},", text
    )


def replace_palette_colors(
    text: str, colors: Mapping[str, Tuple[float, float, float]]
) -> str:
    baseline = parse_palette_colors(text)
    if set(colors) != set(baseline):
        raise ExperimentError("replacement palette does not cover the baseline exactly")

    def replacement(match: re.Match[str]) -> str:
        red, green, blue = colors[match.group("id")]
        return (
            match.group("prefix")
            + "(red: "
            + _format_float(red)
            + ", green: "
            + _format_float(green)
            + ", blue: "
            + _format_float(blue)
            + ")"
        )

    replaced, count = PALETTE_COLOR_RE.subn(replacement, text)
    if count != len(baseline):
        raise ExperimentError("palette replacement did not update every swatch exactly once")
    if parse_palette_colors(replaced) != dict(colors):
        raise ExperimentError("palette replacement did not round-trip exactly")
    return replaced


def isolate_scenario_record(text: str, scenario: str) -> Tuple[str, int, int]:
    marker = f'            name: "{scenario}",'
    if text.count(marker) != 1:
        raise ExperimentError(
            f"scenario library must contain exactly one {scenario!r} record"
        )
    marker_at = text.index(marker)
    block_start = text.rfind("        (\n", 0, marker_at)
    block_end = text.find("\n        ),", marker_at)
    if block_start < 0 or block_end < 0:
        raise ExperimentError(f"cannot isolate scenario record {scenario!r}")
    block_end += len("\n        ),")
    return text[block_start:block_end], block_start, block_end


def patch_scenario_lighting(text: str, scenario: str, lighting_path: str) -> str:
    block, block_start, block_end = isolate_scenario_record(text, scenario)
    if re.search(r"^\s*lighting:\s*", block, re.MULTILINE):
        raise ExperimentError(f"scenario {scenario!r} already selects lighting")
    world_line = re.compile(r'^(?P<indent>\s*)world:\s*"[^"]+",\s*$', re.MULTILINE)
    world_matches = list(world_line.finditer(block))
    if len(world_matches) != 1:
        raise ExperimentError(f"scenario {scenario!r} must have exactly one world field")
    patched_block = world_line.sub(
        lambda match: match.group(0)
        + f'\n{match.group("indent")}lighting: "{lighting_path}",',
        block,
    )
    return text[:block_start] + patched_block + text[block_end:]


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _tree_files(root: pathlib.Path) -> Tuple[pathlib.Path, ...]:
    files: List[pathlib.Path] = []
    for path in root.rglob("*"):
        if path.is_symlink():
            raise ExperimentError(f"staged trees may not contain symlinks: {path}")
        if path.is_file():
            files.append(path)
        elif not path.is_dir():
            raise ExperimentError(f"staged trees may contain only files/directories: {path}")
    return tuple(sorted(files, key=lambda value: value.relative_to(root).as_posix()))


def tree_file_hashes(root: pathlib.Path) -> Dict[str, str]:
    return {
        path.relative_to(root).as_posix(): sha256_file(path)
        for path in _tree_files(root)
    }


def tree_digest(root: pathlib.Path) -> str:
    digest = hashlib.sha256()
    for relative, file_hash in tree_file_hashes(root).items():
        encoded = relative.encode("utf-8")
        digest.update(struct.pack(">I", len(encoded)))
        digest.update(encoded)
        digest.update(bytes.fromhex(file_hash))
    return digest.hexdigest()


def copy_asset_tree(source: pathlib.Path, destination: pathlib.Path) -> None:
    if destination.exists():
        raise ExperimentError(f"staged asset destination already exists: {destination}")
    _tree_files(source)
    shutil.copytree(source, destination, copy_function=shutil.copy2, symlinks=False)
    source_files = {path.relative_to(source): path for path in _tree_files(source)}
    destination_files = {
        path.relative_to(destination): path for path in _tree_files(destination)
    }
    if set(source_files) != set(destination_files):
        raise ExperimentError("staged asset copy changed the file set")
    for relative in source_files:
        source_stat = source_files[relative].stat()
        destination_stat = destination_files[relative].stat()
        if (
            source_stat.st_dev == destination_stat.st_dev
            and source_stat.st_ino == destination_stat.st_ino
        ):
            raise ExperimentError(f"staged asset is hardlinked to source: {relative}")
        if sha256_file(source_files[relative]) != sha256_file(destination_files[relative]):
            raise ExperimentError(f"staged asset copy is corrupt: {relative}")


def make_tree_read_only(root: pathlib.Path) -> None:
    for path in _tree_files(root):
        executable = path.stat().st_mode & 0o111
        path.chmod(0o444 | executable)
    directories = [root] + [path for path in root.rglob("*") if path.is_dir()]
    for path in sorted(directories, key=lambda value: len(value.parts), reverse=True):
        path.chmod(0o555)


def _remove_tree(path: pathlib.Path) -> None:
    if not path.exists():
        return

    for candidate in path.rglob("*"):
        if candidate.is_dir():
            candidate.chmod(stat.S_IRWXU)
        elif candidate.exists():
            candidate.chmod(stat.S_IRUSR | stat.S_IWUSR)
    path.chmod(stat.S_IRWXU)

    def repair_permissions(function: Any, raw_path: str, _info: Any) -> None:
        os.chmod(raw_path, stat.S_IRWXU)
        function(raw_path)

    shutil.rmtree(path, onerror=repair_permissions)


def canonical_json(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def atomic_write(path: pathlib.Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    if temporary.exists():
        temporary.unlink()
    temporary.write_text(content, encoding="utf-8")
    os.replace(temporary, path)


def git_output(repository_root: pathlib.Path, *arguments: str) -> bytes:
    result = subprocess.run(
        ["git", *arguments],
        cwd=repository_root,
        check=False,
        capture_output=True,
    )
    if result.returncode != 0:
        raise ExperimentError(
            "git " + " ".join(arguments) + " failed: " + result.stderr.decode(errors="replace")
        )
    return result.stdout


def workspace_provenance(repository_root: pathlib.Path) -> Dict[str, Any]:
    head = git_output(repository_root, "rev-parse", "HEAD").decode().strip()
    status = git_output(
        repository_root,
        "status",
        "--porcelain=v1",
        "-z",
        "--",
        ".",
        ":(exclude).context/grand-v3-visual-experiments/**",
    )
    listed = git_output(
        repository_root,
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
    )
    digest = hashlib.sha256()
    for raw in sorted(value for value in listed.split(b"\0") if value):
        relative = raw.decode("utf-8", errors="strict")
        if relative.startswith(".context/grand-v3-visual-experiments/"):
            continue
        encoded = relative.encode("utf-8")
        digest.update(struct.pack(">I", len(encoded)))
        digest.update(encoded)
        path = repository_root / relative
        if not path.exists() and not path.is_symlink():
            digest.update(b"DELETED")
        elif path.is_symlink():
            digest.update(b"SYMLINK\0" + os.readlink(path).encode("utf-8"))
        elif path.is_file():
            digest.update(bytes.fromhex(sha256_file(path)))
        else:
            digest.update(b"NON_FILE")
    return {
        "git_head": head,
        "worktree_dirty": bool(status),
        "workspace_content_sha256": digest.hexdigest(),
    }


def _asset_relative(path: str) -> str:
    relative = pathlib.PurePosixPath(path)
    if not relative.parts or relative.parts[0] != "assets":
        raise ExperimentError(f"expected assets-relative repository path: {path}")
    return pathlib.PurePosixPath(*relative.parts[1:]).as_posix()


def apply_profile(
    repository_root: pathlib.Path,
    staged_root: pathlib.Path,
    registry: Registry,
    profile: ProfileSpec,
) -> Dict[str, Any]:
    assets_root = staged_root / "assets"
    source_hashes = tree_file_hashes(repository_root / "assets")
    modified: List[str] = []

    if profile.level_height is not None:
        relative = registry.baseline["world"]
        destination = staged_root / relative
        text = destination.read_text(encoding="utf-8")
        destination.write_text(
            replace_level_height(text, profile.level_height), encoding="utf-8"
        )
        modified.append(_asset_relative(relative))

    if profile.palette is not None:
        palette_id, colors = load_palette_candidate(
            repository_root / profile.palette,
            parse_palette_colors(
                (repository_root / registry.baseline["palette"]).read_text(
                    encoding="utf-8"
                )
            ),
        )
        if palette_id != profile.id:
            raise ExperimentError("profile palette identity changed after validation")
        relative = registry.baseline["palette"]
        destination = staged_root / relative
        destination.write_text(
            replace_palette_colors(destination.read_text(encoding="utf-8"), colors),
            encoding="utf-8",
        )
        modified.append(_asset_relative(relative))

    if profile.lighting_asset is not None:
        relative = registry.baseline["scenarios"]
        destination = staged_root / relative
        destination.write_text(
            patch_scenario_lighting(
                destination.read_text(encoding="utf-8"),
                registry.scenario,
                _asset_relative(profile.lighting_asset),
            ),
            encoding="utf-8",
        )
        modified.append(_asset_relative(relative))

    staged_hashes = tree_file_hashes(assets_root)
    expected_keys = set(source_hashes)
    if set(staged_hashes) != expected_keys:
        raise ExperimentError("profile staging changed the asset file set")
    changed = sorted(
        relative
        for relative in expected_keys
        if source_hashes[relative] != staged_hashes[relative]
    )
    if changed != sorted(modified):
        raise ExperimentError(
            f"profile {profile.id} changed unexpected assets; expected={sorted(modified)}, got={changed}"
        )
    staged_world = staged_root / registry.baseline["world"]
    resolved_height = read_level_height(staged_world.read_text(encoding="utf-8"))
    expected_height = profile.level_height or registry.baseline["level_height"]
    if resolved_height != expected_height:
        raise ExperimentError(f"profile {profile.id} staged the wrong level_height")
    lighting_asset = profile.lighting_asset or registry.baseline["default_lighting"]
    return {
        "axis": profile.axis,
        "label": profile.label,
        "level_height": resolved_height,
        "lighting_asset": lighting_asset,
        "lighting_mode": "static" if profile.lighting_asset else "cycle",
        "palette": profile.palette or registry.baseline["palette"],
        "time_hours": profile.time_hours,
        "modified_assets": [
            {
                "path": "assets/" + relative,
                "source_sha256": source_hashes[relative],
                "staged_sha256": staged_hashes[relative],
            }
            for relative in changed
        ],
        "staged_asset_tree_sha256": tree_digest(assets_root),
    }


def relevant_source_hashes(
    repository_root: pathlib.Path, registry: Registry
) -> Dict[str, str]:
    paths = {
        registry.path.resolve(),
        pathlib.Path(__file__).resolve(),
        *(
            (repository_root / registry.baseline[field]).resolve()
            for field in (
                "world",
                "palette",
                "scenarios",
                "default_lighting",
                "overcast_lighting",
            )
        ),
        *(
            (repository_root / profile.palette).resolve()
            for profile in registry.profiles
            if profile.palette is not None
        ),
    }
    result: Dict[str, str] = {}
    for path in sorted(paths):
        try:
            relative = path.relative_to(repository_root.resolve()).as_posix()
        except ValueError:
            relative = path.as_posix()
        result[relative] = sha256_file(path)
    return result


def load_stable_inputs(
    path: pathlib.Path,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> Tuple[Registry, Dict[str, str], Dict[str, Any]]:
    """Parse only an input snapshot whose hashes and worktree identity stay stable."""

    for _attempt in range(3):
        before = workspace_provenance(repository_root)
        registry = load_registry(path, repository_root)
        source_hashes = relevant_source_hashes(repository_root, registry)
        after_source_hashes = relevant_source_hashes(repository_root, registry)
        after = workspace_provenance(repository_root)
        if before == after and source_hashes == after_source_hashes:
            return registry, source_hashes, before
    raise ExperimentError("experiment inputs changed repeatedly while being loaded")


def selected_profiles(registry: Registry, requested: Sequence[str]) -> Tuple[ProfileSpec, ...]:
    if not requested:
        return registry.profiles
    if len(set(requested)) != len(requested):
        raise ExperimentError("profile selection contains duplicates")
    requested_set = set(requested)
    unknown = sorted(requested_set - {profile.id for profile in registry.profiles})
    if unknown:
        raise ExperimentError(f"unknown profiles: {unknown}")
    return tuple(profile for profile in registry.profiles if profile.id in requested_set)


def build_capture_environment(
    inherited: Mapping[str, str],
    *,
    staged_root: pathlib.Path,
    data_root: pathlib.Path,
    capture_path: pathlib.Path,
    registry: Registry,
    profile: ProfileSpec,
    capture: CaptureSpec,
) -> Dict[str, str]:
    environment = {
        key: value
        for key, value in inherited.items()
        if key not in BEHAVIOR_ENV_EXACT
        and not any(key.startswith(prefix) for prefix in BEHAVIOR_ENV_PREFIXES)
        and not (key.startswith("CARGO_TARGET_") and key != "CARGO_TARGET_DIR")
    }
    environment.update(
        {
            "BEVY_ASSET_ROOT": str(staged_root),
            "CARGO_INCREMENTAL": "0",
            "HEX_GAME_DATA_DIR": str(data_root),
            "HEX_REVIEW_SCENARIO": registry.scenario,
            "HEX_REVIEW_SEED": str(registry.seed),
            "HEX_REVIEW_CAPTURE": str(capture_path),
            "HEX_REVIEW_CAMERA": capture.camera,
            "HEX_REVIEW_VIEW": capture.view,
            "HEX_REVIEW_LIQUID_PHASE": "0.0",
        }
    )
    if profile.time_hours is not None:
        environment["HEX_REVIEW_TIME"] = _format_float(profile.time_hours)
    if capture.focus_anchor is not None:
        environment["HEX_REVIEW_FOCUS_ANCHOR"] = capture.focus_anchor
    return environment


def _terminate_process_group(process: subprocess.Popen[Any]) -> None:
    """Terminate Cargo and every child in its fresh POSIX session."""

    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
        return
    except subprocess.TimeoutExpired:
        pass
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        return
    process.wait(timeout=5)


def run_logged_process(
    command: Sequence[str],
    *,
    cwd: pathlib.Path,
    environment: Mapping[str, str],
    log_path: pathlib.Path,
    timeout_seconds: int,
) -> int:
    """Run one capture in an isolated process group and leave no timed-out child."""

    if os.name != "posix":
        raise CaptureError("isolated capture process groups require a POSIX host")
    try:
        with log_path.open("wb") as log:
            process = subprocess.Popen(
                list(command),
                cwd=cwd,
                env=dict(environment),
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                return process.wait(timeout=timeout_seconds)
            except subprocess.TimeoutExpired as error:
                _terminate_process_group(process)
                raise CaptureError(
                    f"capture process exceeded {timeout_seconds}s and its process group was stopped"
                ) from error
            except BaseException:
                _terminate_process_group(process)
                raise
    except OSError as error:
        raise CaptureError(f"cannot launch capture process: {error}") from error


def _tokenized_environment(
    registry: Registry, profile: ProfileSpec, capture: CaptureSpec
) -> Dict[str, str]:
    result = {
        "BEVY_ASSET_ROOT": "$STAGED_ASSET_ROOT",
        "CARGO_INCREMENTAL": "0",
        "HEX_GAME_DATA_DIR": "$ISOLATED_DATA_DIR",
        "HEX_REVIEW_CAMERA": capture.camera,
        "HEX_REVIEW_CAPTURE": "$UNPUBLISHED_PACK/" + profile.id + "/" + capture.filename,
        "HEX_REVIEW_LIQUID_PHASE": "0.0",
        "HEX_REVIEW_SCENARIO": registry.scenario,
        "HEX_REVIEW_SEED": str(registry.seed),
        "HEX_REVIEW_VIEW": capture.view,
    }
    if profile.time_hours is not None:
        result["HEX_REVIEW_TIME"] = _format_float(profile.time_hours)
    if capture.focus_anchor is not None:
        result["HEX_REVIEW_FOCUS_ANCHOR"] = capture.focus_anchor
    return result


def inspect_png(path: pathlib.Path) -> Tuple[int, int]:
    try:
        contents = path.read_bytes()
    except OSError as error:
        raise ExperimentError(f"capture was not written: {path}: {error}") from error
    if not contents.startswith(PNG_SIGNATURE):
        raise ExperimentError(f"capture is not a valid PNG: {path}")
    offset = len(PNG_SIGNATURE)
    chunks: List[Tuple[bytes, bytes]] = []
    while offset < len(contents):
        if len(contents) - offset < 12:
            raise ExperimentError(f"capture has a truncated PNG chunk: {path}")
        length = struct.unpack(">I", contents[offset : offset + 4])[0]
        chunk_end = offset + 12 + length
        if chunk_end > len(contents):
            raise ExperimentError(f"capture has a truncated PNG chunk: {path}")
        chunk_type = contents[offset + 4 : offset + 8]
        chunk_data = contents[offset + 8 : offset + 8 + length]
        expected_crc = struct.unpack(">I", contents[offset + 8 + length : chunk_end])[0]
        actual_crc = zlib.crc32(chunk_type + chunk_data) & 0xFFFFFFFF
        if actual_crc != expected_crc:
            raise ExperimentError(f"capture has a corrupt PNG chunk: {path}")
        chunks.append((chunk_type, chunk_data))
        offset = chunk_end
        if chunk_type == b"IEND":
            break
    if offset != len(contents):
        raise ExperimentError(f"capture has data after PNG IEND: {path}")
    if not chunks or chunks[0][0] != b"IHDR" or len(chunks[0][1]) != 13:
        raise ExperimentError(f"capture is missing one canonical PNG IHDR: {path}")
    if chunks[-1] != (b"IEND", b""):
        raise ExperimentError(f"capture is missing PNG IEND: {path}")
    if sum(1 for chunk_type, _ in chunks if chunk_type == b"IHDR") != 1:
        raise ExperimentError(f"capture repeats PNG IHDR: {path}")
    ihdr = chunks[0][1]
    width, height, bit_depth, color_type, compression, filter_method, interlace = (
        struct.unpack(">IIBBBBB", ihdr)
    )
    if (width, height) != (CAPTURE_WIDTH, CAPTURE_HEIGHT):
        raise ExperimentError(
            f"capture has dimensions {width}x{height}, expected {CAPTURE_WIDTH}x{CAPTURE_HEIGHT}"
        )
    if compression != 0 or filter_method != 0 or interlace != 0:
        raise ExperimentError(f"capture uses an unsupported PNG encoding: {path}")
    channels_by_type = {0: 1, 2: 3, 3: 1, 4: 2, 6: 4}
    valid_depths = {
        0: {1, 2, 4, 8, 16},
        2: {8, 16},
        3: {1, 2, 4, 8},
        4: {8, 16},
        6: {8, 16},
    }
    if color_type not in channels_by_type or bit_depth not in valid_depths[color_type]:
        raise ExperimentError(f"capture uses an invalid PNG color layout: {path}")
    compressed = b"".join(
        chunk_data for chunk_type, chunk_data in chunks if chunk_type == b"IDAT"
    )
    if not compressed:
        raise ExperimentError(f"capture contains no PNG image data: {path}")
    row_bytes = (
        width * channels_by_type[color_type] * bit_depth + 7
    ) // 8
    expected_bytes = height * (row_bytes + 1)
    decoder = zlib.decompressobj()
    try:
        pixels = decoder.decompress(compressed, expected_bytes + 1)
    except zlib.error as error:
        raise ExperimentError(f"capture has invalid compressed PNG data: {path}") from error
    if (
        len(pixels) != expected_bytes
        or not decoder.eof
        or decoder.unconsumed_tail
        or decoder.unused_data
    ):
        raise ExperimentError(f"capture PNG data is truncated or oversized: {path}")
    for row in range(height):
        if pixels[row * (row_bytes + 1)] > 4:
            raise ExperimentError(f"capture uses an invalid PNG row filter: {path}")
    return width, height


def _run_capture(
    *,
    repository_root: pathlib.Path,
    staged_root: pathlib.Path,
    pack_root: pathlib.Path,
    runtime_root: pathlib.Path,
    registry: Registry,
    profile: ProfileSpec,
    capture: CaptureSpec,
    timeout_seconds: int,
    common_provenance: Mapping[str, Any],
    profile_state: Mapping[str, Any],
) -> Dict[str, Any]:
    profile_output = pack_root / "profiles" / profile.id
    profile_output.mkdir(parents=True, exist_ok=True)
    capture_path = profile_output / capture.filename
    log_path = profile_output / "logs" / (capture.id + ".log")
    log_path.parent.mkdir(parents=True, exist_ok=True)
    data_root = runtime_root / "data" / profile.id / capture.id
    data_root.mkdir(parents=True, exist_ok=False)
    environment = build_capture_environment(
        os.environ,
        staged_root=staged_root,
        data_root=data_root,
        capture_path=capture_path,
        registry=registry,
        profile=profile,
        capture=capture,
    )
    command = [
        "cargo",
        "run",
        "--release",
        "-p",
        "hex_game",
        "--features",
        "map-review",
    ]
    returncode = run_logged_process(
        command,
        cwd=repository_root,
        environment=environment,
        log_path=log_path,
        timeout_seconds=timeout_seconds,
    )
    if returncode != 0:
        tail = log_path.read_text(encoding="utf-8", errors="replace")[-4000:]
        raise CaptureError(
            f"capture {profile.id}/{capture.id} failed with {returncode}:\n{tail}"
        )
    width, height = inspect_png(capture_path)
    artifact_hash = sha256_file(capture_path)
    sidecar = {
        "schema_version": 1,
        "review_status": "UNREVIEWED",
        "provenance": common_provenance,
        "profile": dict(profile_state),
        "capture": {
            "id": capture.id,
            "camera": capture.camera,
            "view": capture.view,
            "focus_anchor": capture.focus_anchor,
            "liquid_phase": 0.0,
            "cutaway": None,
            "illumination_overlay": None,
            "command": command,
            "environment": _tokenized_environment(registry, profile, capture),
        },
        "artifact": {
            "path": "profiles/" + profile.id + "/" + capture.filename,
            "sha256": artifact_hash,
            "width": width,
            "height": height,
        },
    }
    sidecar_path = capture_path.with_suffix(".manifest.json")
    atomic_write(sidecar_path, canonical_json(sidecar))
    _remove_tree(data_root)
    return {
        "id": capture.id,
        "path": sidecar["artifact"]["path"],
        "sha256": artifact_hash,
        "sidecar": "profiles/"
        + profile.id
        + "/"
        + sidecar_path.name,
    }


def _write_review_index(
    pack_root: pathlib.Path,
    registry: Registry,
    profiles: Sequence[ProfileSpec],
) -> None:
    lines = [
        "# Grand V3 visual experiment review",
        "",
        "COMPLETE CAPTURE SET — PIXEL REVIEW REQUIRED",
        "",
        f"Scenario: `{registry.scenario}`  ",
        f"Seed: `{registry.seed}`  ",
        "Static frames do not clear flicker, popping, animation, or camera motion.",
        "",
    ]
    for profile in profiles:
        lines.extend((f"## {profile.id} — {profile.label}", ""))
        for capture in registry.captures:
            lines.append(
                f"- `UNREVIEWED` [{capture.filename}](profiles/{profile.id}/{capture.filename})"
            )
            lines.append("  - Notes:")
        lines.append("")
    lines.extend(
        (
            "## Motion evidence",
            "",
            "- `HUMAN-MOTION-PENDING`: orbit and walk the liquid, crystal, chunk-seam, "
            "and tall-terrain route in both directions.",
            "",
        )
    )
    atomic_write(pack_root / "review-index.md", "\n".join(lines))


def _write_html_index(
    pack_root: pathlib.Path,
    registry: Registry,
    profiles: Sequence[ProfileSpec],
) -> None:
    cards = []
    for profile in profiles:
        images = "\n".join(
            '<figure><img src="profiles/{profile}/{filename}" '
            'alt="{profile} {capture}"><figcaption>{capture}</figcaption></figure>'.format(
                profile=profile.id,
                filename=capture.filename,
                capture=capture.id,
            )
            for capture in registry.captures
        )
        cards.append(
            f"<section><h2>{profile.id} — {profile.label}</h2><div class=\"grid\">{images}</div></section>"
        )
    html = """<!doctype html>
<meta charset="utf-8">
<title>Grand V3 visual experiments</title>
<style>
body { font: 16px system-ui; margin: 24px; background: #171b22; color: #edf2f7; }
.grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
figure { margin: 0; padding: 8px; background: #252b35; }
img { display: block; width: 100%; height: auto; }
figcaption { margin-top: 6px; }
</style>
<h1>Grand V3 visual experiments</h1>
<p>Every frame is mechanically captured and remains UNREVIEWED.</p>
""" + "\n".join(cards) + "\n"
    atomic_write(pack_root / "index.html", html)


def validate_complete_pack(
    pack_root: pathlib.Path,
    registry: Registry,
    profiles: Sequence[ProfileSpec],
    expected_provenance: Optional[Mapping[str, Any]] = None,
    expected_source_hashes: Optional[Mapping[str, str]] = None,
    expected_asset_tree_sha256: Optional[str] = None,
) -> None:
    expected_files = {"index.html", "manifest.json", "review-index.md"}
    for profile in profiles:
        expected_files.add(f"profiles/{profile.id}/profile.json")
        for capture in registry.captures:
            expected_files.update(
                {
                    f"profiles/{profile.id}/{capture.filename}",
                    f"profiles/{profile.id}/{pathlib.PurePosixPath(capture.filename).stem}.manifest.json",
                    f"profiles/{profile.id}/logs/{capture.id}.log",
                }
            )
    actual_files = set(tree_file_hashes(pack_root))
    if actual_files != expected_files:
        raise ExperimentError(
            "review pack file set differs; missing="
            + repr(sorted(expected_files - actual_files))
            + ", extra="
            + repr(sorted(actual_files - expected_files))
        )

    manifest = _read_json(pack_root / "manifest.json", context="pack manifest")
    expected_manifest_fields = {
        "schema_version",
        "review_status",
        "motion_status",
        "provenance",
        "scenario",
        "seed",
        "source_asset_tree_sha256",
        "source_hashes",
        "profile_count",
        "capture_count",
        "profiles",
    }
    manifest_profiles = manifest.get("profiles")
    manifest_provenance = manifest.get("provenance")
    manifest_source_hashes = manifest.get("source_hashes")
    expected_capture_count = len(profiles) * len(registry.captures)
    if (
        set(manifest) != expected_manifest_fields
        or manifest.get("schema_version") != 1
        or manifest.get("review_status") != "UNREVIEWED"
        or manifest.get("motion_status") != "HUMAN-MOTION-PENDING"
        or manifest.get("scenario") != registry.scenario
        or manifest.get("seed") != registry.seed
        or manifest.get("profile_count") != len(profiles)
        or manifest.get("capture_count") != expected_capture_count
        or not isinstance(manifest_provenance, dict)
        or not isinstance(manifest_source_hashes, dict)
        or not all(
            isinstance(path, str)
            and isinstance(digest, str)
            and SHA256_RE.fullmatch(digest)
            for path, digest in manifest_source_hashes.items()
        )
        or not isinstance(manifest.get("source_asset_tree_sha256"), str)
        or not SHA256_RE.fullmatch(manifest["source_asset_tree_sha256"])
        or not isinstance(manifest_profiles, list)
        or not all(isinstance(item, dict) for item in manifest_profiles)
        or [item.get("id") for item in manifest_profiles]
        != [profile.id for profile in profiles]
    ):
        raise ExperimentError("pack manifest identity or capture set is incomplete")
    if expected_provenance is not None and manifest_provenance != dict(expected_provenance):
        raise ExperimentError("pack manifest provenance differs from the run")
    if (
        expected_source_hashes is not None
        and manifest_source_hashes != dict(expected_source_hashes)
    ):
        raise ExperimentError("pack manifest source hashes differ from the run")
    if (
        expected_asset_tree_sha256 is not None
        and manifest["source_asset_tree_sha256"] != expected_asset_tree_sha256
    ):
        raise ExperimentError("pack manifest asset-tree hash differs from the run")
    common_sidecar_provenance = {
        **manifest_provenance,
        "source_hashes": manifest_source_hashes,
    }

    expected = 0
    for profile, root_profile in zip(profiles, manifest_profiles):
        expected_height = profile.level_height or registry.baseline["level_height"]
        expected_lighting = profile.lighting_asset or registry.baseline["default_lighting"]
        expected_palette = profile.palette or registry.baseline["palette"]
        expected_modified_paths = []
        if profile.level_height is not None:
            expected_modified_paths.append(registry.baseline["world"])
        if profile.palette is not None:
            expected_modified_paths.append(registry.baseline["palette"])
        if profile.lighting_asset is not None:
            expected_modified_paths.append(registry.baseline["scenarios"])
        expected_profile_fields = {
            "id",
            "axis",
            "label",
            "level_height",
            "lighting_asset",
            "lighting_mode",
            "palette",
            "time_hours",
            "modified_assets",
            "staged_asset_tree_sha256",
            "captures",
        }
        modified_assets = root_profile.get("modified_assets")
        if (
            set(root_profile) != expected_profile_fields
            or root_profile.get("id") != profile.id
            or root_profile.get("axis") != profile.axis
            or root_profile.get("label") != profile.label
            or root_profile.get("level_height") != expected_height
            or root_profile.get("lighting_asset") != expected_lighting
            or root_profile.get("lighting_mode")
            != ("static" if profile.lighting_asset else "cycle")
            or root_profile.get("palette") != expected_palette
            or root_profile.get("time_hours") != profile.time_hours
            or not isinstance(root_profile.get("staged_asset_tree_sha256"), str)
            or not SHA256_RE.fullmatch(root_profile["staged_asset_tree_sha256"])
            or not isinstance(modified_assets, list)
            or [
                item.get("path")
                for item in modified_assets
                if isinstance(item, dict)
            ]
            != expected_modified_paths
        ):
            raise ExperimentError(f"root profile state is invalid: {profile.id}")
        for item in modified_assets:
            if (
                not isinstance(item, dict)
                or set(item) != {"path", "source_sha256", "staged_sha256"}
                or not isinstance(item.get("source_sha256"), str)
                or not isinstance(item.get("staged_sha256"), str)
                or not SHA256_RE.fullmatch(item["source_sha256"])
                or not SHA256_RE.fullmatch(item["staged_sha256"])
                or manifest_source_hashes.get(item["path"]) != item["source_sha256"]
                or item["source_sha256"] == item["staged_sha256"]
            ):
                raise ExperimentError(f"profile asset provenance is invalid: {profile.id}")
        profile_state = {
            key: value for key, value in root_profile.items() if key != "captures"
        }
        profile_path = pack_root / "profiles" / profile.id / "profile.json"
        profile_manifest = _read_json(profile_path, context="profile manifest")
        if profile_manifest != {"schema_version": 1, **root_profile}:
            raise ExperimentError(f"profile manifest differs from root: {profile_path}")
        profile_capture_records = profile_manifest.get("captures")
        if (
            not isinstance(profile_capture_records, list)
            or not all(isinstance(record, dict) for record in profile_capture_records)
            or [record.get("id") for record in profile_capture_records]
            != [capture.id for capture in registry.captures]
        ):
            raise ExperimentError(f"profile manifest capture set is invalid: {profile_path}")
        for capture_index, capture in enumerate(registry.captures):
            png = pack_root / "profiles" / profile.id / capture.filename
            sidecar = png.with_suffix(".manifest.json")
            width, height = inspect_png(png)
            raw = _read_json(sidecar, context="capture sidecar")
            profile_raw = raw.get("profile")
            capture_raw = raw.get("capture")
            artifact_raw = raw.get("artifact")
            expected_artifact_path = f"profiles/{profile.id}/{capture.filename}"
            expected_capture = {
                "id": capture.id,
                "camera": capture.camera,
                "view": capture.view,
                "focus_anchor": capture.focus_anchor,
                "liquid_phase": 0.0,
                "cutaway": None,
                "illumination_overlay": None,
                "command": [
                    "cargo",
                    "run",
                    "--release",
                    "-p",
                    "hex_game",
                    "--features",
                    "map-review",
                ],
                "environment": _tokenized_environment(registry, profile, capture),
            }
            expected_artifact = {
                "path": expected_artifact_path,
                "sha256": sha256_file(png),
                "width": width,
                "height": height,
            }
            if (
                set(raw)
                != {
                    "schema_version",
                    "review_status",
                    "provenance",
                    "profile",
                    "capture",
                    "artifact",
                }
                or raw.get("schema_version") != 1
                or raw.get("review_status") != "UNREVIEWED"
                or raw.get("provenance") != common_sidecar_provenance
            ):
                raise ExperimentError(f"capture sidecar schema/status is invalid: {sidecar}")
            if profile_raw != profile_state:
                raise ExperimentError(f"capture sidecar profile differs: {sidecar}")
            if capture_raw != expected_capture:
                raise ExperimentError(f"capture sidecar axes are invalid: {sidecar}")
            if artifact_raw != expected_artifact:
                raise ExperimentError(f"capture sidecar hash mismatch: {png}")
            capture_record = profile_capture_records[capture_index]
            expected_record = {
                "id": capture.id,
                "path": expected_artifact_path,
                "sha256": expected_artifact["sha256"],
                "sidecar": f"profiles/{profile.id}/{sidecar.name}",
            }
            if capture_record != expected_record:
                raise ExperimentError(f"profile capture projection differs: {sidecar}")
            expected += 1
    review_index = (pack_root / "review-index.md").read_text(encoding="utf-8")
    html_index = (pack_root / "index.html").read_text(encoding="utf-8")
    if "COMPLETE CAPTURE SET" not in review_index:
        raise ExperimentError("pack review index did not complete")
    for profile in profiles:
        for capture in registry.captures:
            relative = f"profiles/{profile.id}/{capture.filename}"
            if relative not in review_index or relative not in html_index:
                raise ExperimentError(f"pack indexes omit capture {relative}")


def _default_output(
    provenance: Mapping[str, Any],
    registry: Registry,
    profiles: Sequence[ProfileSpec],
) -> pathlib.Path:
    state = (
        "dirty-" + str(provenance["workspace_content_sha256"])[:12]
        if provenance["worktree_dirty"]
        else "clean"
    )
    selection = (
        "matrix-v1"
        if tuple(profile.id for profile in profiles) == EXPECTED_PROFILE_IDS
        else "profiles-" + "-".join(profile.id for profile in profiles)
    )
    return (
        EXPERIMENT_ROOT
        / str(provenance["git_head"])
        / state
        / f"seed-{registry.seed}"
        / selection
    )


def resolve_output(raw: Optional[str], default: pathlib.Path) -> pathlib.Path:
    output = pathlib.Path(raw) if raw is not None else default
    if not output.is_absolute():
        output = REPOSITORY_ROOT / output
    output = output.resolve()
    root = EXPERIMENT_ROOT.resolve()
    try:
        relative = output.relative_to(root)
    except ValueError as error:
        raise ExperimentError(f"output must stay under {root}") from error
    if not relative.parts or any(part in ("", ".", "..") for part in relative.parts):
        raise ExperimentError("output must be a named directory below the experiment root")
    return output


def atomic_publish_no_replace(source: pathlib.Path, destination: pathlib.Path) -> None:
    """Atomically publish a directory without replacing even an empty destination."""

    libc = ctypes.CDLL(None, use_errno=True)
    source_bytes = os.fsencode(source)
    destination_bytes = os.fsencode(destination)
    if sys.platform == "darwin":
        rename = getattr(libc, "renamex_np", None)
        if rename is None:
            raise CaptureError("renamex_np is unavailable; refusing non-atomic publication")
        rename.argtypes = [ctypes.c_char_p, ctypes.c_char_p, ctypes.c_uint]
        rename.restype = ctypes.c_int
        result = rename(source_bytes, destination_bytes, 0x00000004)  # RENAME_EXCL
    elif sys.platform.startswith("linux"):
        rename = getattr(libc, "renameat2", None)
        if rename is None:
            raise CaptureError("renameat2 is unavailable; refusing non-atomic publication")
        rename.argtypes = [
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_int,
            ctypes.c_char_p,
            ctypes.c_uint,
        ]
        rename.restype = ctypes.c_int
        result = rename(-100, source_bytes, -100, destination_bytes, 0x00000001)
    else:
        raise CaptureError(
            f"atomic no-replace publication is unsupported on {sys.platform}"
        )
    if result != 0:
        error_number = ctypes.get_errno()
        if error_number in (errno.EEXIST, errno.ENOTEMPTY):
            raise CaptureError(
                f"output appeared during publication and was not replaced: {destination}"
            )
        raise CaptureError(
            f"atomic publication failed for {destination}: {os.strerror(error_number)}"
        )


def build_plan(
    registry: Registry,
    profiles: Sequence[ProfileSpec],
    provenance: Mapping[str, Any],
    output: pathlib.Path,
    source_hashes: Mapping[str, str],
) -> Dict[str, Any]:
    return {
        "schema_version": 1,
        "mode": "dry-run",
        "output": output.relative_to(REPOSITORY_ROOT).as_posix(),
        "provenance": dict(provenance),
        "scenario": registry.scenario,
        "seed": registry.seed,
        "source_hashes": dict(source_hashes),
        "profiles": [
            {
                "id": profile.id,
                "axis": profile.axis,
                "time_hours": profile.time_hours,
                "lighting_asset": profile.lighting_asset,
                "level_height": profile.level_height,
                "palette": profile.palette,
                "captures": [
                    {
                        "id": capture.id,
                        "command": [
                            "cargo",
                            "run",
                            "--release",
                            "-p",
                            "hex_game",
                            "--features",
                            "map-review",
                        ],
                        "environment": _tokenized_environment(
                            registry, profile, capture
                        ),
                    }
                    for capture in registry.captures
                ],
            }
            for profile in profiles
        ],
    }


def run_matrix(
    *,
    repository_root: pathlib.Path,
    registry: Registry,
    profiles: Sequence[ProfileSpec],
    provenance: Mapping[str, Any],
    output: pathlib.Path,
    source_hashes: Mapping[str, str],
    timeout_seconds: int,
) -> None:
    if output.exists():
        raise ExperimentError(f"output already exists and will not be replaced: {output}")
    output.parent.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(
        tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)
    )
    pack_root = work / "pack"
    runtime_root = work / "runtime"
    pack_root.mkdir()
    atomic_write(
        pack_root / "review-index.md",
        "# Grand V3 visual experiment review\n\nINCOMPLETE — NOT REVIEWABLE\n",
    )
    asset_source_digest = tree_digest(repository_root / "assets")
    capture_records: List[Dict[str, Any]] = []
    profile_records: List[Dict[str, Any]] = []
    try:
        for profile in profiles:
            staged_root = runtime_root / profile.id / "asset-root"
            staged_root.parent.mkdir(parents=True, exist_ok=True)
            copy_asset_tree(repository_root / "assets", staged_root / "assets")
            state = apply_profile(repository_root, staged_root, registry, profile)
            staged_digest_before = state["staged_asset_tree_sha256"]
            make_tree_read_only(staged_root)
            profile_captures = []
            for capture in registry.captures:
                record = _run_capture(
                    repository_root=repository_root,
                    staged_root=staged_root,
                    pack_root=pack_root,
                    runtime_root=runtime_root,
                    registry=registry,
                    profile=profile,
                    capture=capture,
                    timeout_seconds=timeout_seconds,
                    common_provenance={
                        **dict(provenance),
                        "source_hashes": dict(source_hashes),
                    },
                    profile_state={"id": profile.id, **state},
                )
                profile_captures.append(record)
                capture_records.append({"profile": profile.id, **record})
            if tree_digest(staged_root / "assets") != staged_digest_before:
                raise ExperimentError(f"runtime mutated staged assets for {profile.id}")
            profile_record = {"id": profile.id, **state, "captures": profile_captures}
            atomic_write(
                pack_root / "profiles" / profile.id / "profile.json",
                canonical_json({"schema_version": 1, **profile_record}),
            )
            profile_records.append(profile_record)
            _remove_tree(staged_root.parent)

        if tree_digest(repository_root / "assets") != asset_source_digest:
            raise ExperimentError("tracked source assets changed while the matrix ran")
        if relevant_source_hashes(repository_root, registry) != dict(source_hashes):
            raise ExperimentError("experiment sources changed while the matrix ran")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("Git head or worktree content changed while the matrix ran")
        manifest = {
            "schema_version": 1,
            "review_status": "UNREVIEWED",
            "motion_status": "HUMAN-MOTION-PENDING",
            "provenance": dict(provenance),
            "scenario": registry.scenario,
            "seed": registry.seed,
            "source_asset_tree_sha256": asset_source_digest,
            "source_hashes": dict(source_hashes),
            "profile_count": len(profiles),
            "capture_count": len(capture_records),
            "profiles": profile_records,
        }
        atomic_write(pack_root / "manifest.json", canonical_json(manifest))
        _write_review_index(pack_root, registry, profiles)
        _write_html_index(pack_root, registry, profiles)
        validate_complete_pack(
            pack_root,
            registry,
            profiles,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
        )
        if tree_digest(repository_root / "assets") != asset_source_digest:
            raise CaptureError("tracked source assets changed before publication")
        if relevant_source_hashes(repository_root, registry) != dict(source_hashes):
            raise CaptureError("experiment sources changed before publication")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("Git head or worktree content changed before publication")
        atomic_publish_no_replace(pack_root, output)
    finally:
        _remove_tree(work)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--registry",
        type=pathlib.Path,
        default=DEFAULT_REGISTRY,
        help="strict profile registry (default: checked-in Grand V3 matrix)",
    )
    subparsers = parser.add_subparsers(dest="command", required=True)
    subparsers.add_parser("validate", help="validate profiles and source assets only")
    run = subparsers.add_parser("run", help="capture and atomically publish a matrix")
    run.add_argument(
        "--profile",
        action="append",
        default=[],
        help="capture one profile; repeat for a subset (default: all nine)",
    )
    run.add_argument(
        "--output",
        help="new output directory below .context/grand-v3-visual-experiments",
    )
    run.add_argument(
        "--timeout-seconds",
        type=int,
        default=1800,
        help="per-capture process timeout (default: 1800)",
    )
    run.add_argument(
        "--dry-run",
        action="store_true",
        help="print the canonical plan without staging, output, or Cargo",
    )
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = build_parser()
    arguments = parser.parse_args(argv)
    try:
        registry, source_hashes, provenance = load_stable_inputs(
            arguments.registry, REPOSITORY_ROOT
        )
        if arguments.command == "validate":
            print(
                canonical_json(
                    {
                        "schema_version": 1,
                        "status": "valid",
                        "profiles": [profile.id for profile in registry.profiles],
                        "captures": [capture.id for capture in registry.captures],
                        "source_hashes": source_hashes,
                    }
                ),
                end="",
            )
            return 0
        if arguments.timeout_seconds <= 0:
            raise ExperimentError("timeout-seconds must be positive")
        profiles = selected_profiles(registry, arguments.profile)
        output = resolve_output(
            arguments.output,
            _default_output(provenance, registry, profiles),
        )
        if arguments.dry_run:
            print(
                canonical_json(
                    build_plan(
                        registry, profiles, provenance, output, source_hashes
                    )
                ),
                end="",
            )
            return 0
        run_matrix(
            repository_root=REPOSITORY_ROOT,
            registry=registry,
            profiles=profiles,
            provenance=provenance,
            output=output,
            source_hashes=source_hashes,
            timeout_seconds=arguments.timeout_seconds,
        )
        print(output)
        return 0
    except ExperimentError as error:
        parser.exit(2, f"visual_experiments: {error}\n")
    except CaptureError as error:
        print(f"visual_experiments: {error}", file=sys.stderr)
        return 1
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
