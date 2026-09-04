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
import datetime
import errno
import hashlib
import html
import itertools
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
import time
import zlib
from dataclasses import dataclass
from typing import Any, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_REGISTRY = (
    REPOSITORY_ROOT / "tools" / "visual_experiments" / "profiles.json"
)
DEFAULT_SWEEP_SPEC = (
    REPOSITORY_ROOT
    / "tools"
    / "visual_experiments"
    / "sweeps"
    / "night-aesthetic-v1.json"
)
INDOOR_CRYSTAL_SPEC_RELATIVE = (
    "tools/visual_experiments/lighting/indoor-crystal-v1.json"
)
INDOOR_CRYSTAL_SPEC = REPOSITORY_ROOT / INDOOR_CRYSTAL_SPEC_RELATIVE
EXPERIMENT_ROOT = (
    REPOSITORY_ROOT / ".context" / "grand-v3-visual-experiments"
)
EXPECTED_PROFILE_IDS = (
    "e00-baseline",
    "l01-midnight",
    "l02-dawn",
    "l03-golden",
    "l04-overcast",
    "l05-soft-fill-noon",
    "l06-high-contrast-noon",
    "z01-haze-light",
    "z02-haze-medium",
    "i01-crystal-tight",
    "i02-crystal-broad",
    "i03-heart-feature-shadow",
    "v01-fog-none",
    "v02-fog-dimmed",
    "v03-fog-observed-only",
    "v04-fog-softened",
    "m01-matte-terrain",
    "m02-unified-matte",
    "e01-micro-bevel-004",
    "e02-micro-bevel-008",
    "h01-flat-030",
    "h02-tall-055",
    "p01-muted-earth",
    "p02-high-separation",
)
INITIAL_SCREEN_PROFILE_IDS = (
    "e00-baseline",
    "l03-golden",
    "l05-soft-fill-noon",
    "z02-haze-medium",
    "v03-fog-observed-only",
    "v04-fog-softened",
    "m01-matte-terrain",
    "h01-flat-030",
    "h02-tall-055",
    "p01-muted-earth",
)
PROFILE_SETS = {"initial": INITIAL_SCREEN_PROFILE_IDS}
FOG_MODES = ("current", "none", "dimmed", "observed-only", "softened")
VISIBILITY_CANDIDATE_MODES = ("none", "dimmed", "observed-only", "softened")
MATERIAL_TREATMENTS = ("current", "matte-terrain", "unified-matte")
MATERIAL_CANDIDATE_TREATMENTS = ("matte-terrain", "unified-matte")
EDGE_TREATMENTS = ("current", "micro-bevel-004", "micro-bevel-008")
EDGE_CANDIDATE_TREATMENTS = ("micro-bevel-004", "micro-bevel-008")
SWEEP_EDGE_TREATMENTS = EDGE_TREATMENTS + (
    "geometric-bevel-004",
    "geometric-bevel-008",
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
DEFAULT_CAPTURE_SET = "screen"
CAPTURE_SET_IDS = ("smoke", "screen")
SWEEP_AXIS_ORDER = ("height", "light", "palette", "haze", "edge")
SWEEP_TIER_IDS = ("broad", "golden")
SWEEP_SELECTION_STAGES = (
    "semifinal",
    "materials",
    "interior",
    "bevel",
    "finalist",
    "motion-samples",
)
SWEEP_SELECTION_MATERIALS = MATERIAL_TREATMENTS
SWEEP_SELECTION_FOG_MODES = ("current", "dimmed")
SWEEP_SELECTION_EDGE_MODES = ("inherit", *SWEEP_EDGE_TREATMENTS)
SWEEP_SCORE_FIELDS = (
    "terrain_readability",
    "shadow_detail",
    "biome_separation",
    "atmosphere_depth",
    "edge_quietness",
    "mood_coherence",
)
SWEEP_SCORE_WEIGHTS = {
    "terrain_readability": 0.25,
    "shadow_detail": 0.20,
    "biome_separation": 0.15,
    "atmosphere_depth": 0.15,
    "edge_quietness": 0.15,
    "mood_coherence": 0.10,
}
DEFAULT_TOTAL_TIMEOUT_SECONDS = 8 * 60 * 60
MAX_TOTAL_TIMEOUT_SECONDS = 12 * 60 * 60
DEFAULT_MAX_WORK_BYTES = 8 * 1024 * 1024 * 1024
MAX_MAX_WORK_BYTES = 32 * 1024 * 1024 * 1024
DEFAULT_MIN_FREE_BYTES = 20 * 1024 * 1024 * 1024
RUNTIME_REPORT_PLACEHOLDER = {
    "status": "NOT-EMITTED",
    "path": None,
    "reason": "map-review does not yet publish a typed runtime report",
}
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
    look_at_anchor: Optional[str] = None
    look_at_offset: Optional[Tuple[float, float, float]] = None
    cutaway: Optional[str] = None
    illumination_overlay: Optional[str] = None


@dataclass(frozen=True)
class ProfileSpec:
    id: str
    label: str
    axis: str
    time_hours: Optional[float]
    lighting_asset: Optional[str]
    lighting_candidate: Optional[str]
    fog_mode: str
    material_treatment: str
    edge_treatment: str
    crystal_light_profile: Optional[str]
    level_height: Optional[float]
    palette: Optional[str]
    baseline_alias: bool


@dataclass(frozen=True)
class Registry:
    path: pathlib.Path
    scenario: str
    seed: int
    baseline: Mapping[str, Any]
    captures: Tuple[CaptureSpec, ...]
    capture_sets: Mapping[str, Tuple[str, ...]]
    profiles: Tuple[ProfileSpec, ...]

    def profile(self, profile_id: str) -> ProfileSpec:
        for profile in self.profiles:
            if profile.id == profile_id:
                return profile
        raise ExperimentError(f"unknown profile: {profile_id}")

    def captures_for(self, capture_set: str) -> Tuple[CaptureSpec, ...]:
        try:
            capture_ids = self.capture_sets[capture_set]
        except KeyError as error:
            raise ExperimentError(f"unknown capture set: {capture_set}") from error
        captures = {capture.id: capture for capture in self.captures}
        return tuple(captures[capture_id] for capture_id in capture_ids)


@dataclass(frozen=True)
class ReviewBinary:
    path: pathlib.Path
    sha256: str
    command: Tuple[str, ...]


@dataclass(frozen=True)
class ResourceLimits:
    capture_timeout_seconds: int
    total_timeout_seconds: int
    max_work_bytes: int
    min_free_bytes: int


@dataclass(frozen=True)
class SweepLook:
    """One fully resolved, deterministic interaction-sweep recipe."""

    id: str
    label: str
    values: Mapping[str, Mapping[str, Any]]
    semantic_sha256: str


@dataclass(frozen=True)
class SweepTier:
    """One capture tier and its deterministic shard partition."""

    id: str
    capture_ids: Tuple[str, ...]
    shard_count: int
    looks: Tuple[SweepLook, ...]

    def looks_for_shard(self, shard: int) -> Tuple[SweepLook, ...]:
        if not 1 <= shard <= self.shard_count:
            raise ExperimentError(
                f"tier {self.id} shard must be in 1..={self.shard_count}"
            )
        base, remainder = divmod(len(self.looks), self.shard_count)
        start = (shard - 1) * base + min(shard - 1, remainder)
        length = base + (1 if shard <= remainder else 0)
        return self.looks[start : start + length]


@dataclass(frozen=True)
class SweepSpec:
    """Strict checked-in interaction sweep, separate from canonical profiles."""

    path: pathlib.Path
    id: str
    status: str
    registry: str
    axis_order: Tuple[str, ...]
    axes: Mapping[str, Tuple[Mapping[str, Any], ...]]
    tiers: Mapping[str, SweepTier]
    semantic_sha256: str

    def tier(self, tier_id: str) -> SweepTier:
        try:
            return self.tiers[tier_id]
        except KeyError as error:
            raise ExperimentError(f"unknown sweep tier: {tier_id}") from error


@dataclass(frozen=True)
class SelectionRecipe:
    """One adaptive recipe resolved from a broad look and explicit overrides."""

    id: str
    label: str
    base_look: SweepLook
    overrides: Mapping[str, str]
    semantic_sha256: str


@dataclass(frozen=True)
class SweepSelection:
    """Strict selection-driven capture matrix for one adaptive funnel stage."""

    path: pathlib.Path
    id: str
    stage: str
    sweep_id: str
    shard_count: int
    captures: Tuple[CaptureSpec, ...]
    recipes: Tuple[SelectionRecipe, ...]
    camera_manifest_path: Optional[pathlib.Path]
    semantic_sha256: str

    def recipes_for_shard(self, shard: int) -> Tuple[SelectionRecipe, ...]:
        if not 1 <= shard <= self.shard_count:
            raise ExperimentError(
                f"selection {self.id} shard must be in 1..={self.shard_count}"
            )
        base, remainder = divmod(len(self.recipes), self.shard_count)
        start = (shard - 1) * base + min(shard - 1, remainder)
        length = base + (1 if shard <= remainder else 0)
        return self.recipes[start : start + length]


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


LIGHTING_OVERRIDE_FIELDS = {
    "direct_illuminance": "number",
    "direct_color": "rgb",
    "ambient_brightness": "number",
    "ambient_color": "rgb",
    "sky_light_intensity": "number",
    "ground_color": "rgb",
    "sky_color": "rgb",
    "zenith_color": "rgb",
    "fog_color": "rgb",
    "fog_sun_color": "rgb",
    "fog_density": "number",
}

EXPECTED_INDOOR_CRYSTAL_IDS = (
    "i01-crystal-tight",
    "i02-crystal-broad",
    "i03-heart-feature-shadow",
)
SWEEP_SELECTION_CRYSTAL_PROFILES = (
    "current",
    *EXPECTED_INDOOR_CRYSTAL_IDS,
)
INDOOR_CRYSTAL_BASELINE = {
    "intensity_lumens": 4_500.0,
    "range": 4.5,
    "radius": 0.12,
    "shadow_maps_enabled": False,
    "contact_shadows_enabled": False,
}
INDOOR_CRYSTAL_CANDIDATES = {
    "i01-crystal-tight": (
        "all-crystal-point-lights",
        {"range": 3.0},
    ),
    "i02-crystal-broad": (
        "all-crystal-point-lights",
        {"range": 7.0},
    ),
    "i03-heart-feature-shadow": (
        "crystal-heart-offset-18",
        {"shadow_maps_enabled": True},
    ),
}


def load_indoor_crystal_spec(path: pathlib.Path) -> Dict[str, Any]:
    """Validate the active review-only crystal-light profile contract."""

    raw = _strict_object(
        _read_json(path, context="indoor crystal spec"),
        context=f"indoor crystal spec {path.name}",
        allowed=(
            "version",
            "status",
            "runtime_setting",
            "baseline",
            "candidates",
        ),
        required=(
            "version",
            "status",
            "runtime_setting",
            "baseline",
            "candidates",
        ),
    )
    if raw["version"] != 1:
        raise ExperimentError("indoor crystal spec version must be 1")
    if raw["status"] != "active":
        raise ExperimentError("indoor crystal spec must remain active")
    if raw["runtime_setting"] != "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE":
        raise ExperimentError(
            "indoor crystal spec must name the exact review runtime setting"
        )

    baseline_raw = _strict_object(
        raw["baseline"],
        context="indoor crystal baseline",
        allowed=tuple(INDOOR_CRYSTAL_BASELINE),
        required=tuple(INDOOR_CRYSTAL_BASELINE),
    )
    baseline = {
        "intensity_lumens": _number(
            baseline_raw["intensity_lumens"],
            context="indoor crystal baseline intensity_lumens",
        ),
        "range": _number(
            baseline_raw["range"],
            context="indoor crystal baseline range",
        ),
        "radius": _number(
            baseline_raw["radius"],
            context="indoor crystal baseline radius",
        ),
        "shadow_maps_enabled": baseline_raw["shadow_maps_enabled"],
        "contact_shadows_enabled": baseline_raw["contact_shadows_enabled"],
    }
    if not isinstance(baseline["shadow_maps_enabled"], bool) or not isinstance(
        baseline["contact_shadows_enabled"], bool
    ):
        raise ExperimentError("indoor crystal baseline shadows must be booleans")
    if baseline != INDOOR_CRYSTAL_BASELINE:
        raise ExperimentError(
            "indoor crystal baseline must match the shipped physical-light rig"
        )

    candidates_raw = raw["candidates"]
    if not isinstance(candidates_raw, list):
        raise ExperimentError("indoor crystal candidates must be a list")
    candidates: List[Dict[str, Any]] = []
    for index, value in enumerate(candidates_raw):
        candidate_raw = _strict_object(
            value,
            context=f"indoor crystal candidates[{index}]",
            allowed=("id", "label", "target", "overrides"),
            required=("id", "label", "target", "overrides"),
        )
        candidate_id = candidate_raw["id"]
        if not isinstance(candidate_id, str) or not PROFILE_ID_RE.fullmatch(candidate_id):
            raise ExperimentError(
                f"indoor crystal candidates[{index}].id is invalid"
            )
        label = candidate_raw["label"]
        if not isinstance(label, str) or not label:
            raise ExperimentError(
                f"indoor crystal candidate {candidate_id} label is invalid"
            )
        target = candidate_raw["target"]
        overrides_raw = _strict_object(
            candidate_raw["overrides"],
            context=f"indoor crystal candidate {candidate_id} overrides",
            allowed=("range", "shadow_maps_enabled"),
            required=(),
        )
        overrides: Dict[str, Any] = {}
        if "range" in overrides_raw:
            light_range = _number(
                overrides_raw["range"],
                context=f"indoor crystal candidate {candidate_id} range",
            )
            if light_range <= 0.0:
                raise ExperimentError(
                    f"indoor crystal candidate {candidate_id} range must be positive"
                )
            overrides["range"] = light_range
        if "shadow_maps_enabled" in overrides_raw:
            shadow_maps_enabled = overrides_raw["shadow_maps_enabled"]
            if not isinstance(shadow_maps_enabled, bool):
                raise ExperimentError(
                    f"indoor crystal candidate {candidate_id} shadow flag must be boolean"
                )
            overrides["shadow_maps_enabled"] = shadow_maps_enabled
        if not overrides:
            raise ExperimentError(
                f"indoor crystal candidate {candidate_id} needs one override"
            )
        expected = INDOOR_CRYSTAL_CANDIDATES.get(candidate_id)
        if expected is None or (target, overrides) != expected:
            raise ExperimentError(
                f"indoor crystal candidate {candidate_id} changed its exact one-factor contract"
            )
        candidates.append(
            {
                "id": candidate_id,
                "label": label,
                "target": target,
                "overrides": overrides,
            }
        )
    candidate_ids = tuple(candidate["id"] for candidate in candidates)
    if candidate_ids != EXPECTED_INDOOR_CRYSTAL_IDS:
        raise ExperimentError(
            "indoor crystal candidates must retain exact canonical order and coverage"
        )
    return {
        "version": 1,
        "status": raw["status"],
        "runtime_setting": raw["runtime_setting"],
        "baseline": baseline,
        "candidates": candidates,
    }


def load_lighting_candidate(path: pathlib.Path) -> Tuple[str, Dict[str, Any]]:
    raw = _strict_object(
        _read_json(path, context="lighting candidate"),
        context=f"lighting candidate {path.name}",
        allowed=("version", "candidate_id", "base", "noon_overrides"),
        required=("version", "candidate_id", "base", "noon_overrides"),
    )
    if raw["version"] != 1 or raw["base"] != "default-cycle":
        raise ExperimentError(
            f"lighting candidate {path.name} must be version 1 over default-cycle"
        )
    candidate_id = raw["candidate_id"]
    if not isinstance(candidate_id, str) or not PROFILE_ID_RE.fullmatch(candidate_id):
        raise ExperimentError(f"lighting candidate {path.name} has invalid candidate_id")
    overrides = raw["noon_overrides"]
    if not isinstance(overrides, dict) or not overrides:
        raise ExperimentError(f"lighting candidate {path.name} needs noon_overrides")
    unknown = sorted(set(overrides) - set(LIGHTING_OVERRIDE_FIELDS))
    if unknown:
        raise ExperimentError(
            f"lighting candidate {path.name} has unknown overrides: {unknown}"
        )
    resolved: Dict[str, Any] = {}
    for field in sorted(overrides):
        value = overrides[field]
        if LIGHTING_OVERRIDE_FIELDS[field] == "rgb":
            if not isinstance(value, list) or len(value) != 3:
                raise ExperimentError(f"lighting candidate {candidate_id} {field} needs RGB")
            channels = [
                _number(channel, context=f"lighting candidate {candidate_id} {field}")
                for channel in value
            ]
            if any(channel < 0.0 or channel > 1.0 for channel in channels):
                raise ExperimentError(
                    f"lighting candidate {candidate_id} {field} is outside [0, 1]"
                )
            resolved[field] = channels
        else:
            number = _number(value, context=f"lighting candidate {candidate_id} {field}")
            if number < 0.0:
                raise ExperimentError(
                    f"lighting candidate {candidate_id} {field} must be non-negative"
                )
            resolved[field] = number
    return candidate_id, resolved


def patch_cycle_noon_lighting(text: str, overrides: Mapping[str, Any]) -> str:
    block_start, block_end, block = _cycle_noon_block(text)
    for field, value in overrides.items():
        replacement = (
            "(" + ", ".join(_format_float(channel) for channel in value) + ")"
            if isinstance(value, list)
            else _format_float(value)
        )
        pattern = re.compile(
            rf"^(?P<indent>\s*){re.escape(field)}:\s*.*?,\s*$", re.MULTILINE
        )
        block, count = pattern.subn(
            lambda match: f'{match.group("indent")}{field}: {replacement},',
            block,
        )
        if count != 1:
            raise ExperimentError(
                f"default lighting noon keyframe needs exactly one {field} field"
            )
    return text[:block_start] + block + text[block_end:]


def _cycle_noon_block(text: str) -> Tuple[int, int, str]:
    markers = list(
        re.finditer(r"^\s*time_hours:\s*12\.0,\s*$", text, re.MULTILINE)
    )
    if len(markers) != 1:
        raise ExperimentError("default lighting needs exactly one noon keyframe")
    marker_index = markers[0].start()
    block_start = text.rfind("\n            (", 0, marker_index)
    if block_start < 0:
        raise ExperimentError("cannot isolate default lighting noon keyframe")
    block_start += len("\n            ")
    depth = 0
    block_end = None
    for index in range(block_start, len(text)):
        if text[index] == "(":
            depth += 1
        elif text[index] == ")":
            depth -= 1
            if depth == 0:
                block_end = index + 1
                break
    if block_end is None:
        raise ExperimentError("default lighting noon keyframe is unterminated")
    return block_start, block_end, text[block_start:block_end]


def cycle_noon_lighting_values(text: str, fields: Iterable[str]) -> Dict[str, Any]:
    """Read selected typed values from the unique noon keyframe."""

    _, _, block = _cycle_noon_block(text)
    values: Dict[str, Any] = {}
    for field in fields:
        pattern = re.compile(
            rf"^\s*{re.escape(field)}:\s*(?P<value>.*),\s*$", re.MULTILINE
        )
        matches = list(pattern.finditer(block))
        if len(matches) != 1:
            raise ExperimentError(
                f"default lighting noon keyframe needs exactly one {field} field"
            )
        raw = matches[0].group("value").strip()
        if LIGHTING_OVERRIDE_FIELDS[field] == "rgb":
            if not raw.startswith("(") or not raw.endswith(")"):
                raise ExperimentError(f"default lighting noon {field} must be RGB")
            channels = [part.strip() for part in raw[1:-1].split(",")]
            if len(channels) != 3:
                raise ExperimentError(f"default lighting noon {field} must be RGB")
            values[field] = [float(channel) for channel in channels]
        else:
            values[field] = float(raw)
    return values


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
        allowed=(
            "version",
            "scenario",
            "seed",
            "baseline",
            "captures",
            "capture_sets",
            "profiles",
        ),
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
    if baseline_height != 0.35:
        raise ExperimentError("baseline.level_height must remain the shipped 0.35")
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
            allowed=(
                "id",
                "filename",
                "camera",
                "view",
                "focus_anchor",
                "look_at_anchor",
                "look_at_offset",
                "cutaway",
                "illumination_overlay",
            ),
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
        look_at_anchor = capture_raw.get("look_at_anchor")
        if look_at_anchor is not None and (
            not isinstance(look_at_anchor, str) or not look_at_anchor
        ):
            raise ExperimentError(f"capture {capture_id} has invalid look_at_anchor")
        look_at_offset_raw = capture_raw.get("look_at_offset")
        look_at_offset = None
        if look_at_offset_raw is not None:
            if not isinstance(look_at_offset_raw, list) or len(look_at_offset_raw) != 3:
                raise ExperimentError(
                    f"capture {capture_id} look_at_offset needs three components"
                )
            look_at_offset = tuple(
                _number(value, context=f"capture {capture_id} look_at_offset")
                for value in look_at_offset_raw
            )
            distance_squared = sum(value * value for value in look_at_offset)
            if not 1.0 <= distance_squared <= 2048.0 * 2048.0:
                raise ExperimentError(
                    f"capture {capture_id} look_at_offset must be 1..=2048 units"
                )
        if (look_at_anchor is None) != (look_at_offset is None):
            raise ExperimentError(
                f"capture {capture_id} look-at anchor and offset must appear together"
            )
        if look_at_anchor is not None and camera != "map":
            raise ExperimentError(f"capture {capture_id} look-at requires Map camera")
        if focus_anchor is not None and look_at_anchor is not None:
            raise ExperimentError(
                f"capture {capture_id} focus and look-at anchors are mutually exclusive"
            )
        cutaway = capture_raw.get("cutaway")
        if cutaway not in (None, "full"):
            raise ExperimentError(f"capture {capture_id} cutaway must be full")
        illumination_overlay = capture_raw.get("illumination_overlay")
        if illumination_overlay not in (None, "overlay"):
            raise ExperimentError(
                f"capture {capture_id} illumination_overlay must be overlay"
            )
        if cutaway is not None and focus_anchor is None:
            raise ExperimentError(
                f"capture {capture_id} cutaway requires a focus_anchor"
            )
        if illumination_overlay is not None and cutaway != "full":
            raise ExperimentError(
                f"capture {capture_id} illumination overlay requires full cutaway"
            )
        captures.append(
            CaptureSpec(
                capture_id,
                filename,
                camera,
                view,
                focus_anchor,
                look_at_anchor,
                look_at_offset,
                cutaway,
                illumination_overlay,
            )
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

    capture_ids_ordered = tuple(capture.id for capture in captures)
    capture_sets_raw = raw.get("capture_sets")
    if capture_sets_raw is None:
        # Version-one registries before named sets retain their original all-capture
        # behavior. The checked-in registry declares both strict sets explicitly.
        capture_sets = {name: capture_ids_ordered for name in CAPTURE_SET_IDS}
    else:
        capture_sets_raw = _strict_object(
            capture_sets_raw,
            context="capture_sets",
            allowed=CAPTURE_SET_IDS,
            required=CAPTURE_SET_IDS,
        )
        capture_sets = {}
        for name in CAPTURE_SET_IDS:
            values = capture_sets_raw[name]
            if (
                not isinstance(values, list)
                or not values
                or not all(isinstance(value, str) for value in values)
                or len(values) != len(set(values))
            ):
                raise ExperimentError(f"capture_sets.{name} must be a unique non-empty list")
            unknown = sorted(set(values) - set(capture_ids_ordered))
            if unknown:
                raise ExperimentError(f"capture_sets.{name} has unknown captures: {unknown}")
            expected_order = tuple(
                capture_id for capture_id in capture_ids_ordered if capture_id in values
            )
            if tuple(values) != expected_order:
                raise ExperimentError(f"capture_sets.{name} must use canonical capture order")
            capture_sets[name] = tuple(values)
        if capture_sets["screen"] != capture_ids_ordered:
            raise ExperimentError("capture_sets.screen must contain all captures")
        if not set(capture_sets["smoke"]).issubset(capture_sets["screen"]):
            raise ExperimentError("capture_sets.smoke must be a screen subset")

    indoor_crystal_spec = load_indoor_crystal_spec(
        repository_root / INDOOR_CRYSTAL_SPEC_RELATIVE
    )
    indoor_crystal_candidates = {
        candidate["id"]: candidate for candidate in indoor_crystal_spec["candidates"]
    }

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
                "lighting_candidate",
                "fog_mode",
                "material_treatment",
                "edge_treatment",
                "crystal_light_profile",
                "level_height",
                "palette",
                "baseline_alias",
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
        if axis not in (
            "baseline",
            "lighting",
            "haze",
            "visibility",
            "materials",
            "edges",
            "indoor-lighting",
            "level_height",
            "palette",
        ):
            raise ExperimentError(f"profile {profile_id} has invalid axis")
        profile_ids.append(profile_id)
        time_hours = (
            _time(profile_raw["time_hours"], context=f"profile {profile_id} time_hours")
            if "time_hours" in profile_raw
            else None
        )
        lighting_asset = profile_raw.get("lighting_asset")
        lighting_candidate = profile_raw.get("lighting_candidate")
        declared_fog_mode = profile_raw.get("fog_mode")
        if declared_fog_mode is not None and declared_fog_mode not in FOG_MODES:
            raise ExperimentError(
                f"profile {profile_id} fog_mode must be one of {FOG_MODES!r}"
            )
        fog_mode = declared_fog_mode or "current"
        declared_material_treatment = profile_raw.get("material_treatment")
        if (
            declared_material_treatment is not None
            and declared_material_treatment not in MATERIAL_TREATMENTS
        ):
            raise ExperimentError(
                f"profile {profile_id} material_treatment must be one of {MATERIAL_TREATMENTS!r}"
            )
        material_treatment = declared_material_treatment or "current"
        declared_edge_treatment = profile_raw.get("edge_treatment")
        if (
            declared_edge_treatment is not None
            and declared_edge_treatment not in EDGE_TREATMENTS
        ):
            raise ExperimentError(
                f"profile {profile_id} edge_treatment must be one of {EDGE_TREATMENTS!r}"
            )
        edge_treatment = declared_edge_treatment or "current"
        crystal_light_profile = profile_raw.get("crystal_light_profile")
        if crystal_light_profile is not None and (
            not isinstance(crystal_light_profile, str)
            or crystal_light_profile not in indoor_crystal_candidates
        ):
            raise ExperimentError(
                f"profile {profile_id} crystal_light_profile must be one of "
                f"{EXPECTED_INDOOR_CRYSTAL_IDS!r}"
            )
        level_height = (
            _number(
                profile_raw["level_height"],
                context=f"profile {profile_id} level_height",
            )
            if "level_height" in profile_raw
            else None
        )
        palette = profile_raw.get("palette")
        baseline_alias = profile_raw.get("baseline_alias", False)
        if not isinstance(baseline_alias, bool):
            raise ExperimentError(f"profile {profile_id} baseline_alias must be boolean")
        if axis == "baseline":
            if time_hours != 12.0 or any(
                item is not None
                for item in (lighting_asset, lighting_candidate, level_height, palette)
            ) or (
                declared_fog_mode != "current"
                or declared_material_treatment != "current"
                or declared_edge_treatment != "current"
                or crystal_light_profile is not None
            ):
                raise ExperimentError(
                    f"baseline profile {profile_id} must set only noon time_hours, current fog, current materials, and current edges"
                )
        elif axis == "lighting":
            if (
                level_height is not None
                or palette is not None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
            ):
                raise ExperimentError(f"lighting profile {profile_id} mixes experiment axes")
            if lighting_candidate is not None:
                if time_hours != 12.0 or lighting_asset is not None:
                    raise ExperimentError(
                        f"lighting candidate profile {profile_id} must patch cycle noon only"
                    )
            elif (time_hours is None) == (lighting_asset is None):
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
        elif axis == "haze":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
                or level_height is not None
                or palette is not None
            ):
                raise ExperimentError(
                    f"haze profile {profile_id} must patch only the cycle-noon haze"
                )
        elif axis == "visibility":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or level_height is not None
                or palette is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
                or declared_fog_mode not in VISIBILITY_CANDIDATE_MODES
            ):
                raise ExperimentError(
                    f"visibility profile {profile_id} must change only one non-current fog mode"
                )
        elif axis == "materials":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or declared_fog_mode is not None
                or level_height is not None
                or palette is not None
                or declared_material_treatment not in MATERIAL_CANDIDATE_TREATMENTS
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
            ):
                raise ExperimentError(
                    f"material profile {profile_id} must change only one non-current material treatment"
                )
        elif axis == "edges":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment not in EDGE_CANDIDATE_TREATMENTS
                or crystal_light_profile is not None
                or level_height is not None
                or palette is not None
            ):
                raise ExperimentError(
                    f"edge profile {profile_id} must change only one non-current edge treatment"
                )
        elif axis == "indoor-lighting":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or level_height is not None
                or palette is not None
                or crystal_light_profile != profile_id
            ):
                raise ExperimentError(
                    f"indoor-lighting profile {profile_id} must select only its exact crystal-light profile"
                )
            if indoor_crystal_candidates[profile_id]["label"] != label:
                raise ExperimentError(
                    f"indoor-lighting profile {profile_id} label differs from its active spec"
                )
        elif axis == "level_height":
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
                or palette is not None
                or level_height not in (0.30, 0.55)
            ):
                raise ExperimentError(f"height profile {profile_id} mixes or changes axes")
        else:
            if (
                time_hours != 12.0
                or lighting_asset is not None
                or lighting_candidate is not None
                or declared_fog_mode is not None
                or declared_material_treatment is not None
                or declared_edge_treatment is not None
                or crystal_light_profile is not None
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
            palette_id, colors = load_palette_candidate(
                repository_root / palette, baseline_colors
            )
            if palette_id != profile_id:
                raise ExperimentError(
                    f"palette {palette} id {palette_id!r} does not match {profile_id!r}"
                )
            semantic_alias = colors == baseline_colors
            if semantic_alias != baseline_alias:
                qualifier = (
                    "must declare baseline_alias"
                    if semantic_alias
                    else "is not a baseline alias"
                )
                raise ExperimentError(f"palette profile {profile_id} {qualifier}")
        if lighting_candidate is not None:
            lighting_candidate = _existing_regular_file(
                repository_root,
                lighting_candidate,
                context=f"profile {profile_id} lighting_candidate",
                prefix="tools/visual_experiments/lighting",
            )
            candidate_id, overrides = load_lighting_candidate(
                repository_root / lighting_candidate
            )
            if candidate_id != profile_id:
                raise ExperimentError(
                    f"lighting candidate {lighting_candidate} id {candidate_id!r} "
                    f"does not match {profile_id!r}"
                )
            if axis == "haze" and set(overrides) != {
                "fog_color",
                "fog_sun_color",
                "fog_density",
            }:
                raise ExperimentError(
                    f"haze profile {profile_id} must change exactly the three haze fields"
                )
            if axis == "lighting" and set(overrides).issubset(
                {"fog_color", "fog_sun_color", "fog_density"}
            ):
                raise ExperimentError(
                    f"lighting profile {profile_id} must change a non-haze light field"
                )
            baseline_values = cycle_noon_lighting_values(
                (repository_root / baseline["default_lighting"]).read_text(
                    encoding="utf-8"
                ),
                overrides,
            )
            semantic_alias = overrides == baseline_values
            if semantic_alias != baseline_alias:
                qualifier = (
                    "must declare baseline_alias"
                    if semantic_alias
                    else "is not a baseline alias"
                )
                raise ExperimentError(f"lighting profile {profile_id} {qualifier}")
        elif baseline_alias and palette is None:
            raise ExperimentError(
                f"profile {profile_id} may alias only a palette or lighting candidate"
            )
        profiles.append(
            ProfileSpec(
                profile_id,
                label,
                axis,
                time_hours,
                lighting_asset,
                lighting_candidate,
                fog_mode,
                material_treatment,
                edge_treatment,
                crystal_light_profile,
                level_height,
                palette,
                baseline_alias,
            )
        )
    visibility_modes = tuple(
        profile.fog_mode for profile in profiles if profile.axis == "visibility"
    )
    if visibility_modes != VISIBILITY_CANDIDATE_MODES:
        raise ExperimentError(
            "visibility profiles must cover none, dimmed, observed-only, and softened exactly once"
        )
    material_treatments = tuple(
        profile.material_treatment for profile in profiles if profile.axis == "materials"
    )
    if material_treatments != MATERIAL_CANDIDATE_TREATMENTS:
        raise ExperimentError(
            "material profiles must cover matte-terrain and unified-matte exactly once"
        )
    edge_treatments = tuple(
        profile.edge_treatment for profile in profiles if profile.axis == "edges"
    )
    if edge_treatments != EDGE_CANDIDATE_TREATMENTS:
        raise ExperimentError(
            "edge profiles must cover micro-bevel-004 and micro-bevel-008 exactly once"
        )
    indoor_crystal_profiles = tuple(
        profile.crystal_light_profile
        for profile in profiles
        if profile.axis == "indoor-lighting"
    )
    if indoor_crystal_profiles != EXPECTED_INDOOR_CRYSTAL_IDS:
        raise ExperimentError(
            "indoor-lighting profiles must cover the active crystal-light candidates exactly once"
        )
    if tuple(profile_ids) != EXPECTED_PROFILE_IDS:
        raise ExperimentError(
            "profiles must be the canonical ordered twenty-four-profile matrix; got "
            + repr(tuple(profile_ids))
        )
    return Registry(
        path=path,
        scenario=raw["scenario"],
        seed=raw["seed"],
        baseline=baseline,
        captures=tuple(captures),
        capture_sets=capture_sets,
        profiles=tuple(profiles),
    )


def _sweep_candidate_path(
    repository_root: pathlib.Path,
    raw: Any,
    *,
    context: str,
    kind: str,
) -> str:
    prefix = f"tools/visual_experiments/{kind}"
    return _existing_regular_file(
        repository_root,
        raw,
        context=context,
        prefix=prefix,
    )


def _load_sweep_axis_value(
    axis: str,
    raw: Any,
    *,
    context: str,
    repository_root: pathlib.Path,
    baseline_colors: Iterable[str],
) -> Dict[str, Any]:
    common = ("id", "label")
    fields = {
        "height": ("id", "label", "level_height"),
        "light": (
            "id",
            "label",
            "time_hours",
            "lighting_candidate",
            "candidate_id",
        ),
        "palette": ("id", "label", "palette", "candidate_id"),
        "haze": ("id", "label", "lighting_candidate", "candidate_id"),
        "edge": ("id", "label", "edge_treatment"),
    }
    required = {
        "height": (*common, "level_height"),
        "light": (*common, "time_hours"),
        "palette": common,
        "haze": common,
        "edge": (*common, "edge_treatment"),
    }
    value = dict(
        _strict_object(
            raw,
            context=context,
            allowed=fields[axis],
            required=required[axis],
        )
    )
    value_id = value["id"]
    if not isinstance(value_id, str) or not PROFILE_ID_RE.fullmatch(value_id):
        raise ExperimentError(f"{context}.id is invalid")
    if not isinstance(value["label"], str) or not value["label"].strip():
        raise ExperimentError(f"{context}.label must be non-empty")

    if axis == "height":
        value["level_height"] = _number(
            value["level_height"], context=f"{context}.level_height"
        )
        if value["level_height"] not in (0.30, 0.35, 0.40):
            raise ExperimentError(f"{context}.level_height must be 0.30, 0.35, or 0.40")
    elif axis == "light":
        value["time_hours"] = _time(
            value["time_hours"], context=f"{context}.time_hours"
        )
        if ("lighting_candidate" in value) != ("candidate_id" in value):
            raise ExperimentError(
                f"{context} lighting_candidate and candidate_id must appear together"
            )
        if "lighting_candidate" in value:
            value["lighting_candidate"] = _sweep_candidate_path(
                repository_root,
                value["lighting_candidate"],
                context=f"{context}.lighting_candidate",
                kind="lighting",
            )
            candidate_id, overrides = load_lighting_candidate(
                repository_root / value["lighting_candidate"]
            )
            if value["candidate_id"] != candidate_id:
                raise ExperimentError(
                    f"{context}.candidate_id does not match its lighting source"
                )
            if set(overrides).issubset(
                {"fog_color", "fog_sun_color", "fog_density"}
            ):
                raise ExperimentError(f"{context} light rig changes only haze fields")
    elif axis == "palette":
        if ("palette" in value) != ("candidate_id" in value):
            raise ExperimentError(
                f"{context} palette and candidate_id must appear together"
            )
        if "palette" in value:
            value["palette"] = _sweep_candidate_path(
                repository_root,
                value["palette"],
                context=f"{context}.palette",
                kind="palettes",
            )
            candidate_id, _ = load_palette_candidate(
                repository_root / value["palette"], baseline_colors
            )
            if value["candidate_id"] != candidate_id:
                raise ExperimentError(
                    f"{context}.candidate_id does not match its palette source"
                )
    elif axis == "haze":
        if ("lighting_candidate" in value) != ("candidate_id" in value):
            raise ExperimentError(
                f"{context} lighting_candidate and candidate_id must appear together"
            )
        if "lighting_candidate" in value:
            value["lighting_candidate"] = _sweep_candidate_path(
                repository_root,
                value["lighting_candidate"],
                context=f"{context}.lighting_candidate",
                kind="lighting",
            )
            candidate_id, overrides = load_lighting_candidate(
                repository_root / value["lighting_candidate"]
            )
            if value["candidate_id"] != candidate_id:
                raise ExperimentError(
                    f"{context}.candidate_id does not match its haze source"
                )
            if set(overrides) != {"fog_color", "fog_sun_color", "fog_density"}:
                raise ExperimentError(
                    f"{context} must change exactly the three haze fields"
                )
    else:
        if value["edge_treatment"] not in SWEEP_EDGE_TREATMENTS:
            raise ExperimentError(
                f"{context}.edge_treatment must be one of {SWEEP_EDGE_TREATMENTS!r}"
            )
    return value


def _validate_checked_in_sweep_axes(
    axes: Mapping[str, Tuple[Mapping[str, Any], ...]]
) -> None:
    expected_ids = {
        "height": ("h030", "h035", "h040"),
        "light": ("lbalanced", "lsoft", "lcontrast", "lgolden"),
        "palette": ("pshipped", "pearth", "pseparate"),
        "haze": ("z000", "z003", "z007"),
        "edge": ("ehard", "e004", "e008"),
    }
    for axis in SWEEP_AXIS_ORDER:
        actual = tuple(value["id"] for value in axes[axis])
        if actual != expected_ids[axis]:
            raise ExperimentError(
                f"sweep axis {axis} must keep exact ordered values {expected_ids[axis]!r}"
            )
    heights = tuple(value["level_height"] for value in axes["height"])
    if heights != (0.30, 0.35, 0.40):
        raise ExperimentError("sweep heights must remain 0.30, 0.35, and 0.40")
    times = tuple(value["time_hours"] for value in axes["light"])
    if times != (12.0, 12.0, 12.0, 16.5):
        raise ExperimentError("sweep light times must remain noon/noon/noon/golden")
    edges = tuple(value["edge_treatment"] for value in axes["edge"])
    if edges != EDGE_TREATMENTS:
        raise ExperimentError("sweep edges must remain current, 0.04, and 0.08")


def _make_sweep_look(
    sweep_id: str,
    axis_order: Sequence[str],
    values: Sequence[Mapping[str, Any]],
) -> SweepLook:
    resolved = {axis: dict(value) for axis, value in zip(axis_order, values)}
    look_id = "-".join(str(resolved[axis]["id"]) for axis in axis_order)
    if not PROFILE_ID_RE.fullmatch(look_id):
        raise ExperimentError(f"generated sweep look id is invalid: {look_id!r}")
    body = {
        "sweep_id": sweep_id,
        "look_id": look_id,
        "axes": resolved,
    }
    label = " / ".join(str(resolved[axis]["label"]) for axis in axis_order)
    return SweepLook(
        id=look_id,
        label=label,
        values=resolved,
        semantic_sha256=sha256_bytes(canonical_json(body).encode("utf-8")),
    )


def load_sweep_spec(
    path: pathlib.Path = DEFAULT_SWEEP_SPEC,
    registry: Optional[Registry] = None,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> SweepSpec:
    """Load the strict night interaction sweep without touching canonical profiles."""

    registry = registry or load_registry(DEFAULT_REGISTRY, repository_root)
    allowed_root = (
        repository_root / "tools" / "visual_experiments" / "sweeps"
    ).resolve()
    try:
        path.resolve().relative_to(allowed_root)
    except ValueError as error:
        raise ExperimentError(f"sweep spec must stay under {allowed_root}") from error
    if path.is_symlink() or not path.is_file():
        raise ExperimentError(f"sweep spec is not a regular file: {path}")
    raw = _strict_object(
        _read_json(path, context="sweep spec"),
        context="sweep spec",
        allowed=(
            "version",
            "id",
            "status",
            "registry",
            "axis_order",
            "axes",
            "tiers",
        ),
        required=(
            "version",
            "id",
            "status",
            "registry",
            "axis_order",
            "axes",
            "tiers",
        ),
    )
    if raw["version"] != 1:
        raise ExperimentError("sweep spec version must be 1")
    if raw["id"] != "night-aesthetic-v1":
        raise ExperimentError("sweep id must remain 'night-aesthetic-v1'")
    status = raw["status"]
    if status != "historical":
        raise ExperimentError(
            "sweep 'night-aesthetic-v1' must remain historical because its "
            "captured recipes predate the promoted baseline"
        )
    registry_relative = _safe_relative_path(
        raw["registry"], context="sweep.registry"
    )
    expected_registry = registry.path.resolve().relative_to(
        repository_root.resolve()
    ).as_posix()
    if registry_relative != expected_registry:
        raise ExperimentError(
            f"sweep registry must match the loaded canonical registry {expected_registry!r}"
        )
    axis_order_raw = raw["axis_order"]
    if not isinstance(axis_order_raw, list) or tuple(axis_order_raw) != SWEEP_AXIS_ORDER:
        raise ExperimentError(f"sweep axis_order must be {SWEEP_AXIS_ORDER!r}")
    axes_raw = _strict_object(
        raw["axes"],
        context="sweep.axes",
        allowed=SWEEP_AXIS_ORDER,
        required=SWEEP_AXIS_ORDER,
    )
    baseline_colors = parse_palette_colors(
        (repository_root / registry.baseline["palette"]).read_text(encoding="utf-8")
    )
    axes: Dict[str, Tuple[Mapping[str, Any], ...]] = {}
    for axis in SWEEP_AXIS_ORDER:
        values_raw = axes_raw[axis]
        if not isinstance(values_raw, list) or not values_raw:
            raise ExperimentError(f"sweep.axes.{axis} must be a non-empty list")
        values = tuple(
            _load_sweep_axis_value(
                axis,
                value,
                context=f"sweep.axes.{axis}[{index}]",
                repository_root=repository_root,
                baseline_colors=baseline_colors,
            )
            for index, value in enumerate(values_raw)
        )
        ids = tuple(str(value["id"]) for value in values)
        if len(ids) != len(set(ids)):
            raise ExperimentError(f"sweep.axes.{axis} repeats ids")
        axes[axis] = values
    _validate_checked_in_sweep_axes(axes)

    tiers_raw = _strict_object(
        raw["tiers"],
        context="sweep.tiers",
        allowed=SWEEP_TIER_IDS,
        required=SWEEP_TIER_IDS,
    )
    captures_by_id = {capture.id: capture for capture in registry.captures}
    tiers: Dict[str, SweepTier] = {}
    for tier_id in SWEEP_TIER_IDS:
        tier_raw = _strict_object(
            tiers_raw[tier_id],
            context=f"sweep.tiers.{tier_id}",
            allowed=("capture_ids", "shard_count", "axis_values"),
            required=("capture_ids", "shard_count", "axis_values"),
        )
        capture_ids_raw = tier_raw["capture_ids"]
        if (
            not isinstance(capture_ids_raw, list)
            or not capture_ids_raw
            or not all(isinstance(value, str) for value in capture_ids_raw)
            or len(capture_ids_raw) != len(set(capture_ids_raw))
        ):
            raise ExperimentError(
                f"sweep.tiers.{tier_id}.capture_ids must be unique and non-empty"
            )
        unknown_captures = sorted(set(capture_ids_raw) - set(captures_by_id))
        if unknown_captures:
            raise ExperimentError(
                f"sweep.tiers.{tier_id} has unknown captures: {unknown_captures}"
            )
        if tuple(capture_ids_raw) != ("02-highlands-oblique",):
            raise ExperimentError(
                f"sweep.tiers.{tier_id} must use the highlands hero capture"
            )
        shard_count = tier_raw["shard_count"]
        if isinstance(shard_count, bool) or not isinstance(shard_count, int):
            raise ExperimentError(f"sweep.tiers.{tier_id}.shard_count must be an integer")
        expected_shards = 3 if tier_id == "broad" else 1
        if shard_count != expected_shards:
            raise ExperimentError(
                f"sweep.tiers.{tier_id}.shard_count must be {expected_shards}"
            )
        selected_raw = _strict_object(
            tier_raw["axis_values"],
            context=f"sweep.tiers.{tier_id}.axis_values",
            allowed=SWEEP_AXIS_ORDER,
            required=SWEEP_AXIS_ORDER,
        )
        selected_values: List[Tuple[Mapping[str, Any], ...]] = []
        for axis in SWEEP_AXIS_ORDER:
            ids_raw = selected_raw[axis]
            if (
                not isinstance(ids_raw, list)
                or not ids_raw
                or not all(isinstance(value, str) for value in ids_raw)
                or len(ids_raw) != len(set(ids_raw))
            ):
                raise ExperimentError(
                    f"sweep.tiers.{tier_id}.axis_values.{axis} must be unique and non-empty"
                )
            by_id = {str(value["id"]): value for value in axes[axis]}
            unknown = sorted(set(ids_raw) - set(by_id))
            if unknown:
                raise ExperimentError(
                    f"sweep.tiers.{tier_id}.axis_values.{axis} has unknown ids: {unknown}"
                )
            canonical = tuple(value["id"] for value in axes[axis] if value["id"] in ids_raw)
            if tuple(ids_raw) != canonical:
                raise ExperimentError(
                    f"sweep.tiers.{tier_id}.axis_values.{axis} must use canonical order"
                )
            selected_values.append(tuple(by_id[value_id] for value_id in ids_raw))
        looks = tuple(
            _make_sweep_look(raw["id"], SWEEP_AXIS_ORDER, combination)
            for combination in itertools.product(*selected_values)
        )
        expected_count = 243 if tier_id == "broad" else 81
        if len(looks) != expected_count or len({look.id for look in looks}) != expected_count:
            raise ExperimentError(
                f"sweep tier {tier_id} must generate exactly {expected_count} unique looks"
            )
        tier = SweepTier(
            id=tier_id,
            capture_ids=tuple(capture_ids_raw),
            shard_count=shard_count,
            looks=looks,
        )
        expected_per_shard = 81
        if any(
            len(tier.looks_for_shard(shard)) != expected_per_shard
            for shard in range(1, shard_count + 1)
        ):
            raise ExperimentError(f"sweep tier {tier_id} must have 81 looks per shard")
        tiers[tier_id] = tier
    semantic = {
        "version": raw["version"],
        "id": raw["id"],
        "status": status,
        "registry": registry_relative,
        "axis_order": list(SWEEP_AXIS_ORDER),
        "axes": axes,
        "tiers": {
            tier_id: {
                "capture_ids": list(tiers[tier_id].capture_ids),
                "shard_count": tiers[tier_id].shard_count,
                "look_ids": [look.id for look in tiers[tier_id].looks],
            }
            for tier_id in SWEEP_TIER_IDS
        },
    }
    return SweepSpec(
        path=path.resolve(),
        id=raw["id"],
        status=status,
        registry=registry_relative,
        axis_order=SWEEP_AXIS_ORDER,
        axes=axes,
        tiers=tiers,
        semantic_sha256=sha256_bytes(canonical_json(semantic).encode("utf-8")),
    )


def require_capturable_sweep(sweep: SweepSpec) -> None:
    """Reject captures from retained review provenance after its baseline moved."""

    if sweep.status != "active":
        raise ExperimentError(
            f"sweep {sweep.id!r} is historical review provenance and cannot be "
            "rerun against the promoted baseline"
        )


def _parse_selection_capture(raw: Any, *, context: str) -> CaptureSpec:
    value = _strict_object(
        raw,
        context=context,
        allowed=(
            "id",
            "filename",
            "camera",
            "view",
            "focus_anchor",
            "look_at_anchor",
            "look_at_offset",
            "cutaway",
            "illumination_overlay",
        ),
        required=("id", "filename", "camera", "view"),
    )
    capture_id = value["id"]
    if not isinstance(capture_id, str) or not CAPTURE_ID_RE.fullmatch(capture_id):
        raise ExperimentError(f"{context}.id is invalid")
    filename = _safe_relative_path(value["filename"], context=f"{context}.filename")
    if pathlib.PurePosixPath(filename).parent != pathlib.PurePosixPath("."):
        raise ExperimentError(f"{context}.filename must be a plain basename")
    if not filename.endswith(".png"):
        raise ExperimentError(f"{context}.filename must end in .png")
    camera = value["camera"]
    if camera not in ("map", "character", "first-person"):
        raise ExperimentError(f"{context}.camera is invalid")
    view = value["view"]
    if view not in (
        "default",
        "rotated",
        "counter-rotated",
        "rear",
        "top-down",
    ):
        raise ExperimentError(f"{context}.view is invalid")
    focus_anchor = value.get("focus_anchor")
    if focus_anchor is not None and (
        not isinstance(focus_anchor, str) or not focus_anchor
    ):
        raise ExperimentError(f"{context}.focus_anchor is invalid")
    look_at_anchor = value.get("look_at_anchor")
    if look_at_anchor is not None and (
        not isinstance(look_at_anchor, str) or not look_at_anchor
    ):
        raise ExperimentError(f"{context}.look_at_anchor is invalid")
    offset_raw = value.get("look_at_offset")
    look_at_offset = None
    if offset_raw is not None:
        if not isinstance(offset_raw, list) or len(offset_raw) != 3:
            raise ExperimentError(f"{context}.look_at_offset needs three components")
        look_at_offset = tuple(
            _number(component, context=f"{context}.look_at_offset")
            for component in offset_raw
        )
        distance_squared = sum(component * component for component in look_at_offset)
        if not 1.0 <= distance_squared <= 2048.0 * 2048.0:
            raise ExperimentError(f"{context}.look_at_offset must be 1..=2048 units")
    if (look_at_anchor is None) != (look_at_offset is None):
        raise ExperimentError(
            f"{context} look_at_anchor and look_at_offset must appear together"
        )
    if look_at_anchor is not None and camera != "map":
        raise ExperimentError(f"{context} look-at framing requires the Map camera")
    if focus_anchor is not None and look_at_anchor is not None:
        raise ExperimentError(f"{context} focus and look-at framing are exclusive")
    cutaway = value.get("cutaway")
    if cutaway not in (None, "full"):
        raise ExperimentError(f"{context}.cutaway must be full")
    overlay = value.get("illumination_overlay")
    if overlay not in (None, "overlay"):
        raise ExperimentError(f"{context}.illumination_overlay must be overlay")
    if cutaway is not None and focus_anchor is None:
        raise ExperimentError(f"{context}.cutaway requires a focus_anchor")
    if overlay is not None and cutaway != "full":
        raise ExperimentError(f"{context} illumination overlay requires full cutaway")
    return CaptureSpec(
        id=capture_id,
        filename=filename,
        camera=camera,
        view=view,
        focus_anchor=focus_anchor,
        look_at_anchor=look_at_anchor,
        look_at_offset=look_at_offset,
        cutaway=cutaway,
        illumination_overlay=overlay,
    )


def load_selection_camera_manifest(
    path: pathlib.Path,
    registry: Registry,
) -> Tuple[str, Tuple[CaptureSpec, ...]]:
    if path.is_symlink() or not path.is_file():
        raise ExperimentError(f"selection camera manifest is not a regular file: {path}")
    raw = _strict_object(
        _read_json(path, context="selection camera manifest"),
        context="selection camera manifest",
        allowed=("version", "id", "captures"),
        required=("version", "id", "captures"),
    )
    if raw["version"] != 1:
        raise ExperimentError("selection camera manifest version must be 1")
    manifest_id = raw["id"]
    if not isinstance(manifest_id, str) or not PROFILE_ID_RE.fullmatch(manifest_id):
        raise ExperimentError("selection camera manifest id is invalid")
    captures_raw = raw["captures"]
    if not isinstance(captures_raw, list) or not captures_raw:
        raise ExperimentError("selection camera manifest captures must be non-empty")
    captures = tuple(
        _parse_selection_capture(value, context=f"camera captures[{index}]")
        for index, value in enumerate(captures_raw)
    )
    ids = [capture.id for capture in captures]
    filenames = [capture.filename for capture in captures]
    if len(ids) != len(set(ids)) or len(filenames) != len(set(filenames)):
        raise ExperimentError("selection camera manifest repeats ids or filenames")
    canonical = {capture.id: capture for capture in registry.captures}
    for capture in captures:
        if capture.id in canonical and capture != canonical[capture.id]:
            raise ExperimentError(
                f"camera manifest redefines canonical capture {capture.id}"
            )
    return manifest_id, captures


def _selection_override_object(raw: Any, *, context: str) -> Dict[str, str]:
    value = _strict_object(
        raw,
        context=context,
        allowed=(
            "material_treatment",
            "fog_mode",
            "crystal_light_profile",
            "edge_treatment",
        ),
        required=(
            "material_treatment",
            "fog_mode",
            "crystal_light_profile",
            "edge_treatment",
        ),
    )
    allowed = {
        "material_treatment": SWEEP_SELECTION_MATERIALS,
        "fog_mode": SWEEP_SELECTION_FOG_MODES,
        "crystal_light_profile": SWEEP_SELECTION_CRYSTAL_PROFILES,
        "edge_treatment": SWEEP_SELECTION_EDGE_MODES,
    }
    resolved = {}
    for field, choices in allowed.items():
        choice = value[field]
        if not isinstance(choice, str) or choice not in choices:
            raise ExperimentError(f"{context}.{field} must be one of {choices!r}")
        resolved[field] = choice
    return resolved


def _selection_recipe(
    selection_id: str,
    base_look: SweepLook,
    overrides: Mapping[str, str],
) -> SelectionRecipe:
    aliases = {
        "material_treatment": {
            "current": "mcurrent",
            "matte-terrain": "mmatte",
            "unified-matte": "munified",
        },
        "fog_mode": {"current": "fcurrent", "dimmed": "fdimmed"},
        "crystal_light_profile": {
            "current": "icurrent",
            "i01-crystal-tight": "itight",
            "i02-crystal-broad": "ibroad",
            "i03-heart-feature-shadow": "iheart",
        },
        "edge_treatment": {
            "inherit": "einherit",
            "current": "ehard",
            "micro-bevel-004": "en004",
            "micro-bevel-008": "en008",
            "geometric-bevel-004": "eg004",
            "geometric-bevel-008": "eg008",
        },
    }
    suffix = "-".join(
        aliases[field][overrides[field]]
        for field in (
            "material_treatment",
            "fog_mode",
            "crystal_light_profile",
            "edge_treatment",
        )
    )
    recipe_id = f"{base_look.id}-{suffix}"
    if not PROFILE_ID_RE.fullmatch(recipe_id):
        raise ExperimentError(f"generated selection recipe id is invalid: {recipe_id}")
    body = {
        "selection_id": selection_id,
        "recipe_id": recipe_id,
        "base_look_id": base_look.id,
        "base_look_semantic_sha256": base_look.semantic_sha256,
        "overrides": dict(overrides),
    }
    label = base_look.label + " / " + " / ".join(
        f"{field}={overrides[field]}" for field in overrides
    )
    return SelectionRecipe(
        id=recipe_id,
        label=label,
        base_look=base_look,
        overrides=dict(overrides),
        semantic_sha256=sha256_bytes(canonical_json(body).encode("utf-8")),
    )


def load_sweep_selection(
    path: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
) -> SweepSelection:
    """Load an explicit adaptive-funnel selection and resolve every recipe."""

    if path.is_symlink() or not path.is_file():
        raise ExperimentError(f"sweep selection is not a regular file: {path}")
    raw = _strict_object(
        _read_json(path, context="adaptive sweep selection"),
        context="adaptive sweep selection",
        allowed=(
            "version",
            "id",
            "stage",
            "sweep_id",
            "shard_count",
            "base_look_ids",
            "capture_ids",
            "camera_manifest",
            "matrix",
            "recipes",
        ),
        required=(
            "version",
            "id",
            "stage",
            "sweep_id",
            "shard_count",
            "base_look_ids",
            "capture_ids",
        ),
    )
    if raw["version"] != 1:
        raise ExperimentError("adaptive sweep selection version must be 1")
    selection_id = raw["id"]
    if not isinstance(selection_id, str) or not PROFILE_ID_RE.fullmatch(selection_id):
        raise ExperimentError("adaptive sweep selection id is invalid")
    stage = raw["stage"]
    if stage not in SWEEP_SELECTION_STAGES:
        raise ExperimentError(
            f"adaptive sweep selection stage must be one of {SWEEP_SELECTION_STAGES!r}"
        )
    if raw["sweep_id"] != sweep.id:
        raise ExperimentError(f"adaptive sweep selection must target {sweep.id!r}")
    shard_count = raw["shard_count"]
    if (
        isinstance(shard_count, bool)
        or not isinstance(shard_count, int)
        or not 1 <= shard_count <= 32
    ):
        raise ExperimentError("adaptive sweep selection shard_count must be in 1..=32")
    all_looks = {
        look.id: look
        for tier_id in SWEEP_TIER_IDS
        for look in sweep.tier(tier_id).looks
    }
    base_ids = raw["base_look_ids"]
    if (
        not isinstance(base_ids, list)
        or not base_ids
        or not all(isinstance(value, str) for value in base_ids)
        or len(base_ids) != len(set(base_ids))
    ):
        raise ExperimentError("adaptive sweep selection base_look_ids must be unique")
    unknown_base = sorted(set(base_ids) - set(all_looks))
    if unknown_base:
        raise ExperimentError(f"adaptive sweep selection has unknown looks: {unknown_base}")
    has_matrix = "matrix" in raw
    has_recipes = "recipes" in raw
    if has_matrix == has_recipes:
        raise ExperimentError(
            "adaptive sweep selection needs exactly one of matrix or recipes"
        )
    recipes: List[SelectionRecipe] = []
    if has_matrix:
        matrix = _strict_object(
            raw["matrix"],
            context="adaptive sweep selection matrix",
            allowed=(
                "material_treatment",
                "fog_mode",
                "crystal_light_profile",
                "edge_treatment",
            ),
            required=(
                "material_treatment",
                "fog_mode",
                "crystal_light_profile",
                "edge_treatment",
            ),
        )
        allowed_matrix = {
            "material_treatment": SWEEP_SELECTION_MATERIALS,
            "fog_mode": SWEEP_SELECTION_FOG_MODES,
            "crystal_light_profile": SWEEP_SELECTION_CRYSTAL_PROFILES,
            "edge_treatment": SWEEP_SELECTION_EDGE_MODES,
        }
        matrix_values = []
        for field in (
            "material_treatment",
            "fog_mode",
            "crystal_light_profile",
            "edge_treatment",
        ):
            values = matrix[field]
            if (
                not isinstance(values, list)
                or not values
                or not all(isinstance(value, str) for value in values)
                or len(values) != len(set(values))
                or any(value not in allowed_matrix[field] for value in values)
            ):
                raise ExperimentError(
                    f"adaptive sweep selection matrix.{field} is invalid"
                )
            matrix_values.append(tuple(values))
        fields = (
            "material_treatment",
            "fog_mode",
            "crystal_light_profile",
            "edge_treatment",
        )
        for base_id in base_ids:
            for combination in itertools.product(*matrix_values):
                overrides = dict(zip(fields, combination))
                recipes.append(
                    _selection_recipe(selection_id, all_looks[base_id], overrides)
                )
    else:
        recipes_raw = raw["recipes"]
        if not isinstance(recipes_raw, list) or not recipes_raw:
            raise ExperimentError("adaptive sweep selection recipes must be non-empty")
        used_bases = []
        for index, item in enumerate(recipes_raw):
            recipe_raw = _strict_object(
                item,
                context=f"adaptive sweep selection recipes[{index}]",
                allowed=("base_look_id", "overrides"),
                required=("base_look_id", "overrides"),
            )
            base_id = recipe_raw["base_look_id"]
            if base_id not in all_looks or base_id not in base_ids:
                raise ExperimentError(
                    f"adaptive sweep selection recipe has undeclared base {base_id!r}"
                )
            used_bases.append(base_id)
            overrides = _selection_override_object(
                recipe_raw["overrides"],
                context=f"adaptive sweep selection recipes[{index}].overrides",
            )
            recipes.append(
                _selection_recipe(selection_id, all_looks[base_id], overrides)
            )
        if tuple(dict.fromkeys(used_bases)) != tuple(base_ids):
            raise ExperimentError(
                "adaptive sweep selection recipes must use every declared base in order"
            )
    recipe_ids = [recipe.id for recipe in recipes]
    if len(recipe_ids) != len(set(recipe_ids)):
        raise ExperimentError("adaptive sweep selection generates duplicate recipes")
    resolved_recipe_keys = []
    for recipe in recipes:
        profile = selection_profile(recipe)
        resolved_recipe_keys.append(
            (
                recipe.base_look.semantic_sha256,
                profile.material_treatment,
                profile.fog_mode,
                profile.crystal_light_profile or "current",
                profile.edge_treatment,
            )
        )
    if len(resolved_recipe_keys) != len(set(resolved_recipe_keys)):
        raise ExperimentError(
            "adaptive sweep selection contains duplicate resolved recipes"
        )
    if shard_count > len(recipes):
        raise ExperimentError("adaptive sweep selection cannot have empty shards")

    camera_manifest_path = None
    camera_manifest_id = None
    if "camera_manifest" in raw:
        relative = _safe_relative_path(
            raw["camera_manifest"], context="adaptive sweep selection camera_manifest"
        )
        candidate = path.parent / relative
        camera_manifest_id, available_captures = load_selection_camera_manifest(
            candidate, registry
        )
        camera_manifest_path = candidate.resolve()
    else:
        available_captures = registry.captures
    capture_ids = raw["capture_ids"]
    if (
        not isinstance(capture_ids, list)
        or not capture_ids
        or not all(isinstance(value, str) for value in capture_ids)
        or len(capture_ids) != len(set(capture_ids))
    ):
        raise ExperimentError("adaptive sweep selection capture_ids must be unique")
    captures_by_id = {capture.id: capture for capture in available_captures}
    unknown_captures = sorted(set(capture_ids) - set(captures_by_id))
    if unknown_captures:
        raise ExperimentError(
            f"adaptive sweep selection has unknown captures: {unknown_captures}"
        )
    expected_order = tuple(
        capture.id for capture in available_captures if capture.id in capture_ids
    )
    if tuple(capture_ids) != expected_order:
        raise ExperimentError(
            "adaptive sweep selection capture_ids must follow camera-manifest order"
        )
    captures = tuple(captures_by_id[capture_id] for capture_id in capture_ids)
    if stage == "finalist" and camera_manifest_path is not None and len(captures) != 17:
        raise ExperimentError(
            "finalist selection with a camera manifest must select exactly 17 captures"
        )
    semantic = {
        "version": raw["version"],
        "id": selection_id,
        "stage": stage,
        "sweep_id": sweep.id,
        "sweep_semantic_sha256": sweep.semantic_sha256,
        "shard_count": shard_count,
        "camera_manifest_id": camera_manifest_id,
        "captures": [capture.__dict__ for capture in captures],
        "recipes": [
            {
                "id": recipe.id,
                "semantic_sha256": recipe.semantic_sha256,
                "base_look_id": recipe.base_look.id,
                "overrides": dict(recipe.overrides),
            }
            for recipe in recipes
        ],
    }
    return SweepSelection(
        path=path.resolve(),
        id=selection_id,
        stage=stage,
        sweep_id=sweep.id,
        shard_count=shard_count,
        captures=captures,
        recipes=tuple(recipes),
        camera_manifest_path=camera_manifest_path,
        semantic_sha256=sha256_bytes(canonical_json(semantic).encode("utf-8")),
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


def expected_resolved_axis(registry: Registry, profile: ProfileSpec) -> Dict[str, Any]:
    if profile.baseline_alias:
        return {
            "kind": "promoted-baseline-alias",
            "source": profile.lighting_candidate or profile.palette,
        }
    if profile.axis == "visibility":
        return {"kind": "review-fog-mode", "mode": profile.fog_mode}
    if profile.axis == "materials":
        return {
            "kind": "review-material-treatment",
            "treatment": profile.material_treatment,
        }
    if profile.axis == "edges":
        return {
            "kind": "review-edge-treatment",
            "treatment": profile.edge_treatment,
        }
    if profile.axis == "indoor-lighting":
        source = INDOOR_CRYSTAL_SPEC_RELATIVE
        spec = load_indoor_crystal_spec(
            registry.path.resolve().parents[2] / source
        )
        candidate = next(
            candidate
            for candidate in spec["candidates"]
            if candidate["id"] == profile.crystal_light_profile
        )
        selected = dict(spec["baseline"])
        selected.update(candidate["overrides"])
        return {
            "kind": "review-crystal-light-profile",
            "runtime_setting": spec["runtime_setting"],
            "profile": profile.crystal_light_profile,
            "source": source,
            "baseline": spec["baseline"],
            "target": candidate["target"],
            "overrides": candidate["overrides"],
            "resolved_target_state": selected,
        }
    if profile.lighting_candidate is not None:
        repository_root = registry.path.resolve().parents[2]
        _, overrides = load_lighting_candidate(
            repository_root / profile.lighting_candidate
        )
        return {
            "kind": "cycle-noon-overrides",
            "source": profile.lighting_candidate,
            "values": overrides,
        }
    if profile.lighting_asset is not None:
        return {"kind": "static-lighting-asset", "source": profile.lighting_asset}
    if profile.axis in ("lighting", "baseline"):
        return {
            "kind": "cycle-time",
            "source": registry.baseline["default_lighting"],
            "time_hours": profile.time_hours,
        }
    if profile.level_height is not None:
        return {"kind": "level-height", "value": profile.level_height}
    return {
        "kind": "palette",
        "source": profile.palette or registry.baseline["palette"],
    }


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

    if profile.palette is not None and not profile.baseline_alias:
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

    lighting_overrides = None
    if profile.lighting_candidate is not None and not profile.baseline_alias:
        candidate_id, lighting_overrides = load_lighting_candidate(
            repository_root / profile.lighting_candidate
        )
        if candidate_id != profile.id:
            raise ExperimentError("profile lighting candidate identity changed after validation")
        relative = registry.baseline["default_lighting"]
        destination = staged_root / relative
        destination.write_text(
            patch_cycle_noon_lighting(
                destination.read_text(encoding="utf-8"), lighting_overrides
            ),
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
    resolved_axis = expected_resolved_axis(registry, profile)
    return {
        "axis": profile.axis,
        "label": profile.label,
        "level_height": resolved_height,
        "lighting_asset": lighting_asset,
        "lighting_candidate": profile.lighting_candidate,
        "lighting_mode": "static" if profile.lighting_asset else "cycle",
        "fog_mode": profile.fog_mode,
        "material_treatment": profile.material_treatment,
        "edge_treatment": profile.edge_treatment,
        "crystal_light_profile": profile.crystal_light_profile,
        "palette": profile.palette or registry.baseline["palette"],
        "time_hours": profile.time_hours,
        "resolved_axis": resolved_axis,
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


def sweep_profile(look: SweepLook) -> ProfileSpec:
    """Project a fully resolved sweep look onto the existing runtime seam."""

    height = look.values["height"]
    light = look.values["light"]
    palette = look.values["palette"]
    edge = look.values["edge"]
    return ProfileSpec(
        id=look.id,
        label=look.label,
        axis="interaction",
        time_hours=float(light["time_hours"]),
        lighting_asset=None,
        # Sweep lighting sources are composed in the staged asset tree. Keeping
        # this field empty prevents the one-factor identity guard from pretending
        # that one candidate owns the combined recipe.
        lighting_candidate=None,
        fog_mode="current",
        material_treatment="current",
        edge_treatment=str(edge["edge_treatment"]),
        crystal_light_profile=None,
        level_height=float(height["level_height"]),
        palette=str(palette["palette"]) if "palette" in palette else None,
        baseline_alias=False,
    )


def _sweep_lighting_overrides(
    repository_root: pathlib.Path, look: SweepLook
) -> Tuple[Dict[str, Any], List[Dict[str, Any]]]:
    overrides: Dict[str, Any] = {}
    sources: List[Dict[str, Any]] = []
    for axis in ("light", "haze"):
        value = look.values[axis]
        relative = value.get("lighting_candidate")
        if relative is None:
            continue
        candidate_id, candidate_overrides = load_lighting_candidate(
            repository_root / str(relative)
        )
        if candidate_id != value.get("candidate_id"):
            raise ExperimentError(
                f"sweep look {look.id} {axis} candidate identity changed"
            )
        overlap = sorted(set(overrides) & set(candidate_overrides))
        if overlap:
            raise ExperimentError(
                f"sweep look {look.id} lighting sources overlap fields: {overlap}"
            )
        overrides.update(candidate_overrides)
        sources.append(
            {
                "axis": axis,
                "candidate_id": candidate_id,
                "source": str(relative),
                "overrides": candidate_overrides,
            }
        )
    return overrides, sources


def apply_sweep_look(
    repository_root: pathlib.Path,
    staged_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    look: SweepLook,
) -> Dict[str, Any]:
    """Compose one interaction recipe only inside its private asset stage."""

    assets_root = staged_root / "assets"
    source_hashes = tree_file_hashes(repository_root / "assets")
    modified: List[str] = []
    height = float(look.values["height"]["level_height"])
    if height != float(registry.baseline["level_height"]):
        relative = str(registry.baseline["world"])
        destination = staged_root / relative
        destination.write_text(
            replace_level_height(destination.read_text(encoding="utf-8"), height),
            encoding="utf-8",
        )
        modified.append(_asset_relative(relative))

    palette_value = look.values["palette"]
    palette_source = palette_value.get("palette")
    if palette_source is not None:
        candidate_id, colors = load_palette_candidate(
            repository_root / str(palette_source),
            parse_palette_colors(
                (repository_root / str(registry.baseline["palette"])).read_text(
                    encoding="utf-8"
                )
            ),
        )
        if candidate_id != palette_value.get("candidate_id"):
            raise ExperimentError(f"sweep look {look.id} palette identity changed")
        relative = str(registry.baseline["palette"])
        destination = staged_root / relative
        destination.write_text(
            replace_palette_colors(destination.read_text(encoding="utf-8"), colors),
            encoding="utf-8",
        )
        modified.append(_asset_relative(relative))

    lighting_overrides, lighting_sources = _sweep_lighting_overrides(
        repository_root, look
    )
    if lighting_overrides:
        relative = str(registry.baseline["default_lighting"])
        destination = staged_root / relative
        destination.write_text(
            patch_cycle_noon_lighting(
                destination.read_text(encoding="utf-8"), lighting_overrides
            ),
            encoding="utf-8",
        )
        modified.append(_asset_relative(relative))

    staged_hashes = tree_file_hashes(assets_root)
    expected_keys = set(source_hashes)
    if set(staged_hashes) != expected_keys:
        raise ExperimentError(f"sweep look {look.id} changed the asset file set")
    changed = sorted(
        relative
        for relative in expected_keys
        if source_hashes[relative] != staged_hashes[relative]
    )
    if changed != sorted(set(modified)):
        raise ExperimentError(
            f"sweep look {look.id} changed unexpected assets; "
            f"expected={sorted(set(modified))}, got={changed}"
        )
    staged_world = staged_root / str(registry.baseline["world"])
    if read_level_height(staged_world.read_text(encoding="utf-8")) != height:
        raise ExperimentError(f"sweep look {look.id} staged the wrong level height")
    profile = sweep_profile(look)
    resolved_axes = {
        "height": {
            "value_id": look.values["height"]["id"],
            "level_height": height,
            "horizontal_circumradius": 1.0,
        },
        "light": {
            "value_id": look.values["light"]["id"],
            "time_hours": profile.time_hours,
            "candidate_id": look.values["light"].get("candidate_id"),
            "source": look.values["light"].get("lighting_candidate"),
        },
        "palette": {
            "value_id": palette_value["id"],
            "candidate_id": palette_value.get("candidate_id"),
            "source": palette_source or registry.baseline["palette"],
        },
        "haze": {
            "value_id": look.values["haze"]["id"],
            "candidate_id": look.values["haze"].get("candidate_id"),
            "source": look.values["haze"].get("lighting_candidate"),
        },
        "edge": {
            "value_id": look.values["edge"]["id"],
            "treatment": profile.edge_treatment,
        },
    }
    return {
        "axis": profile.axis,
        "label": profile.label,
        "look_semantic_sha256": look.semantic_sha256,
        "sweep": {
            "id": sweep.id,
            "semantic_sha256": sweep.semantic_sha256,
            "axis_order": list(sweep.axis_order),
        },
        "resolved_axes": resolved_axes,
        "level_height": height,
        "lighting_asset": registry.baseline["default_lighting"],
        "lighting_candidate": None,
        "lighting_sources": lighting_sources,
        "lighting_mode": "cycle",
        "fog_mode": profile.fog_mode,
        "material_treatment": profile.material_treatment,
        "edge_treatment": profile.edge_treatment,
        "crystal_light_profile": None,
        "palette": palette_source or registry.baseline["palette"],
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


def selection_profile(recipe: SelectionRecipe) -> ProfileSpec:
    base = sweep_profile(recipe.base_look)
    edge_override = recipe.overrides["edge_treatment"]
    crystal_override = recipe.overrides["crystal_light_profile"]
    return ProfileSpec(
        id=recipe.id,
        label=recipe.label,
        axis="adaptive-interaction",
        time_hours=base.time_hours,
        lighting_asset=None,
        lighting_candidate=None,
        fog_mode=recipe.overrides["fog_mode"],
        material_treatment=recipe.overrides["material_treatment"],
        edge_treatment=(
            base.edge_treatment if edge_override == "inherit" else edge_override
        ),
        crystal_light_profile=(
            None if crystal_override == "current" else crystal_override
        ),
        level_height=base.level_height,
        palette=base.palette,
        baseline_alias=False,
    )


def apply_selection_recipe(
    repository_root: pathlib.Path,
    staged_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    selection: SweepSelection,
    recipe: SelectionRecipe,
) -> Dict[str, Any]:
    """Stage a base look, then resolve presentation-only adaptive overrides."""

    state = apply_sweep_look(
        repository_root,
        staged_root,
        registry,
        sweep,
        recipe.base_look,
    )
    profile = selection_profile(recipe)
    resolved_axes = dict(state["resolved_axes"])
    resolved_axes["base_edge"] = resolved_axes["edge"]
    resolved_axes["edge"] = {
        "override": recipe.overrides["edge_treatment"],
        "treatment": profile.edge_treatment,
    }
    resolved_axes["material"] = {
        "treatment": profile.material_treatment,
    }
    resolved_axes["visibility"] = {"fog_mode": profile.fog_mode}
    resolved_axes["crystal_light"] = {
        "profile": profile.crystal_light_profile or "current"
    }
    state.update(
        {
            "axis": profile.axis,
            "label": profile.label,
            "selection": {
                "id": selection.id,
                "stage": selection.stage,
                "semantic_sha256": selection.semantic_sha256,
                "source": f"selection/{selection.path.name}",
            },
            "recipe_semantic_sha256": recipe.semantic_sha256,
            "base_look_id": recipe.base_look.id,
            "base_look_semantic_sha256": recipe.base_look.semantic_sha256,
            "overrides": dict(recipe.overrides),
            "resolved_axes": resolved_axes,
            "fog_mode": profile.fog_mode,
            "material_treatment": profile.material_treatment,
            "edge_treatment": profile.edge_treatment,
            "crystal_light_profile": profile.crystal_light_profile,
        }
    )
    return state


def relevant_source_hashes(
    repository_root: pathlib.Path, registry: Registry
) -> Dict[str, str]:
    paths = {
        registry.path.resolve(),
        pathlib.Path(__file__).resolve(),
        (repository_root / INDOOR_CRYSTAL_SPEC_RELATIVE).resolve(),
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
        *(
            (repository_root / profile.lighting_candidate).resolve()
            for profile in registry.profiles
            if profile.lighting_candidate is not None
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


def sweep_source_hashes(
    repository_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
) -> Dict[str, str]:
    """Hash every canonical and sweep-specific source used by a shard."""

    result = relevant_source_hashes(repository_root, registry)
    paths = {sweep.path.resolve()}
    for values in sweep.axes.values():
        for value in values:
            for field in ("lighting_candidate", "palette"):
                relative = value.get(field)
                if relative is not None:
                    paths.add((repository_root / str(relative)).resolve())
    for path in sorted(paths):
        relative = path.relative_to(repository_root.resolve()).as_posix()
        result[relative] = sha256_file(path)
    return dict(sorted(result.items()))


def load_stable_sweep_inputs(
    registry_path: pathlib.Path,
    sweep_path: pathlib.Path,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> Tuple[Registry, SweepSpec, Dict[str, str], Dict[str, Any]]:
    """Load a sweep only when its full source and worktree identity stay stable."""

    for _attempt in range(3):
        before = workspace_provenance(repository_root)
        registry = load_registry(registry_path, repository_root)
        sweep = load_sweep_spec(sweep_path, registry, repository_root)
        hashes = sweep_source_hashes(repository_root, registry, sweep)
        after_hashes = sweep_source_hashes(repository_root, registry, sweep)
        after = workspace_provenance(repository_root)
        if before == after and hashes == after_hashes:
            return registry, sweep, hashes, before
    raise ExperimentError("sweep inputs changed repeatedly while being loaded")


def selection_source_hashes(
    repository_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    selection: SweepSelection,
) -> Dict[str, str]:
    result = sweep_source_hashes(repository_root, registry, sweep)
    result[f"selection/{selection.path.name}"] = sha256_file(selection.path)
    if selection.camera_manifest_path is not None:
        result[
            f"camera-manifest/{selection.camera_manifest_path.name}"
        ] = sha256_file(selection.camera_manifest_path)
    return dict(sorted(result.items()))


def load_stable_selection_inputs(
    registry_path: pathlib.Path,
    sweep_path: pathlib.Path,
    selection_path: pathlib.Path,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> Tuple[
    Registry,
    SweepSpec,
    SweepSelection,
    Dict[str, str],
    Dict[str, Any],
]:
    for _attempt in range(3):
        before = workspace_provenance(repository_root)
        registry = load_registry(registry_path, repository_root)
        sweep = load_sweep_spec(sweep_path, registry, repository_root)
        selection = load_sweep_selection(selection_path, registry, sweep)
        hashes = selection_source_hashes(
            repository_root, registry, sweep, selection
        )
        after_hashes = selection_source_hashes(
            repository_root, registry, sweep, selection
        )
        after = workspace_provenance(repository_root)
        if before == after and hashes == after_hashes:
            return registry, sweep, selection, hashes, before
    raise ExperimentError("adaptive selection inputs changed repeatedly while loading")


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


def selected_profiles(
    registry: Registry,
    requested: Sequence[str],
    profile_set: Optional[str] = None,
) -> Tuple[ProfileSpec, ...]:
    if requested and profile_set is not None:
        raise ExperimentError("profile and profile-set selections are mutually exclusive")
    if profile_set is not None:
        try:
            requested = PROFILE_SETS[profile_set]
        except KeyError as error:
            raise ExperimentError(f"unknown profile set: {profile_set}") from error
    if not requested:
        return registry.profiles
    if len(set(requested)) != len(requested):
        raise ExperimentError("profile selection contains duplicates")
    requested_set = set(requested)
    unknown = sorted(requested_set - {profile.id for profile in registry.profiles})
    if unknown:
        raise ExperimentError(f"unknown profiles: {unknown}")
    # Every candidate review is comparative evidence. Include the canonical baseline
    # automatically so a requested subset cannot publish candidate-only HTML.
    if requested_set != {"e00-baseline"}:
        requested_set.add("e00-baseline")
    return tuple(profile for profile in registry.profiles if profile.id in requested_set)


def comparison_report_metadata(
    profiles: Sequence[ProfileSpec], captures: Sequence[CaptureSpec]
) -> Dict[str, Any]:
    """Describe a comparison matrix without machine- or artifact-specific state."""

    profile_ids = tuple(profile.id for profile in profiles)
    capture_ids = tuple(capture.id for capture in captures)
    if len(profile_ids) != len(set(profile_ids)) or len(capture_ids) != len(
        set(capture_ids)
    ):
        raise ExperimentError("comparison report identities must be unique")
    baseline_id = "e00-baseline"
    candidates = tuple(profile for profile in profiles if profile.id != baseline_id)
    if candidates and baseline_id not in profile_ids:
        raise ExperimentError("comparison report candidates require e00-baseline")
    selection_id = (
        "full"
        if profile_ids == EXPECTED_PROFILE_IDS
        else "initial"
        if profile_ids == INITIAL_SCREEN_PROFILE_IDS
        else "custom"
    )
    axis_order = tuple(dict.fromkeys(profile.axis for profile in candidates))
    body: Dict[str, Any] = {
        "schema_version": 1,
        "selection_id": selection_id,
        "baseline_profile_id": baseline_id if baseline_id in profile_ids else None,
        "profile_ids": list(profile_ids),
        "profile_behaviors": [
            {
                "id": profile.id,
                "label": profile.label,
                "axis": profile.axis,
                "time_hours": profile.time_hours,
                "lighting_asset": profile.lighting_asset,
                "lighting_candidate": profile.lighting_candidate,
                "fog_mode": profile.fog_mode,
                "material_treatment": profile.material_treatment,
                "edge_treatment": profile.edge_treatment,
                "crystal_light_profile": profile.crystal_light_profile,
                "level_height": profile.level_height,
                "palette": profile.palette,
                "baseline_alias": profile.baseline_alias,
            }
            for profile in profiles
        ],
        "capture_ids": list(capture_ids),
        "render_count": len(profile_ids) * len(capture_ids),
        "comparison_count": len(candidates) * len(capture_ids),
        "axes": [
            {
                "axis": axis,
                "candidate_profile_ids": [
                    profile.id for profile in candidates if profile.axis == axis
                ],
            }
            for axis in axis_order
        ],
    }
    return {
        **body,
        "semantic_sha256": sha256_bytes(canonical_json(body).encode("utf-8")),
    }


def build_capture_environment(
    inherited: Mapping[str, str],
    *,
    staged_root: pathlib.Path,
    data_root: pathlib.Path,
    capture_path: pathlib.Path,
    registry: Registry,
    profile: ProfileSpec,
    capture: CaptureSpec,
    allow_structural_draft: bool = False,
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
            "HEX_REVIEW_FOG": profile.fog_mode,
            "HEX_REVIEW_MATERIAL": profile.material_treatment,
            "HEX_REVIEW_EDGE": profile.edge_treatment,
            "HEX_REVIEW_VIEW": capture.view,
            "HEX_REVIEW_LIQUID_PHASE": "0.0",
        }
    )
    if profile.time_hours is not None:
        environment["HEX_REVIEW_TIME"] = _format_float(profile.time_hours)
    if profile.crystal_light_profile is not None:
        environment["HEX_REVIEW_CRYSTAL_LIGHT_PROFILE"] = (
            profile.crystal_light_profile
        )
    if capture.focus_anchor is not None:
        environment["HEX_REVIEW_FOCUS_ANCHOR"] = capture.focus_anchor
    if capture.look_at_anchor is not None:
        environment["HEX_REVIEW_LOOK_AT_ANCHOR"] = capture.look_at_anchor
        environment["HEX_REVIEW_LOOK_AT_OFFSET"] = ",".join(
            _format_float(value) for value in capture.look_at_offset or ()
        )
    if capture.cutaway is not None:
        environment["HEX_REVIEW_CUTAWAY"] = capture.cutaway
    if capture.illumination_overlay is not None:
        environment["HEX_REVIEW_ILLUMINATION"] = capture.illumination_overlay
    if allow_structural_draft:
        environment["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"] = "1"
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


def _sanitized_build_environment(inherited: Mapping[str, str]) -> Dict[str, str]:
    environment = {
        key: value
        for key, value in inherited.items()
        if key not in BEHAVIOR_ENV_EXACT
        and not any(key.startswith(prefix) for prefix in BEHAVIOR_ENV_PREFIXES)
        and not (key.startswith("CARGO_TARGET_") and key != "CARGO_TARGET_DIR")
    }
    environment["CARGO_INCREMENTAL"] = "0"
    return environment


def build_review_binary(
    repository_root: pathlib.Path,
    *,
    log_path: pathlib.Path,
    timeout_seconds: int,
) -> ReviewBinary:
    """Build once and identify the exact executable reused by every capture."""

    command = (
        "cargo",
        "build",
        "--release",
        "-p",
        "hex_game",
        "--features",
        "map-review",
        "--message-format=json-render-diagnostics",
    )
    returncode = run_logged_process(
        command,
        cwd=repository_root,
        environment=_sanitized_build_environment(os.environ),
        log_path=log_path,
        timeout_seconds=timeout_seconds,
    )
    if returncode != 0:
        tail = log_path.read_text(encoding="utf-8", errors="replace")[-4000:]
        raise CaptureError(f"review binary build failed with {returncode}:\n{tail}")
    executables = []
    for line in log_path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = event.get("target")
        executable = event.get("executable")
        if (
            event.get("reason") == "compiler-artifact"
            and isinstance(target, dict)
            and target.get("name") == "hex_game"
            and isinstance(executable, str)
        ):
            executables.append(pathlib.Path(executable))
    unique = tuple(dict.fromkeys(path.resolve() for path in executables))
    if len(unique) != 1 or not unique[0].is_file() or unique[0].is_symlink():
        raise CaptureError(
            "Cargo build did not report exactly one regular hex_game executable"
        )
    return ReviewBinary(unique[0], sha256_file(unique[0]), command)


def _recorded_capture_command(review_binary: Optional[ReviewBinary]) -> List[str]:
    if review_binary is not None:
        return ["$REVIEW_BINARY"]
    return [
        "cargo",
        "run",
        "--release",
        "-p",
        "hex_game",
        "--features",
        "map-review",
    ]


def _tree_size(root: pathlib.Path) -> int:
    return sum(path.stat().st_size for path in _tree_files(root))


def _enforce_resource_limits(
    work: pathlib.Path,
    output_parent: pathlib.Path,
    limits: ResourceLimits,
    deadline: float,
) -> None:
    if time.monotonic() >= deadline:
        raise CaptureError(
            f"matrix exceeded total timeout of {limits.total_timeout_seconds}s"
        )
    work_bytes = _tree_size(work)
    if work_bytes > limits.max_work_bytes:
        raise CaptureError(
            f"matrix work tree uses {work_bytes} bytes, over cap {limits.max_work_bytes}"
        )
    free = shutil.disk_usage(output_parent).free
    if free < limits.min_free_bytes:
        raise CaptureError(
            f"only {free} free bytes remain; minimum is {limits.min_free_bytes}"
        )


def _tokenized_environment(
    registry: Registry,
    profile: ProfileSpec,
    capture: CaptureSpec,
    *,
    allow_structural_draft: bool = False,
) -> Dict[str, str]:
    result = {
        "BEVY_ASSET_ROOT": "$STAGED_ASSET_ROOT",
        "CARGO_INCREMENTAL": "0",
        "HEX_GAME_DATA_DIR": "$ISOLATED_DATA_DIR",
        "HEX_REVIEW_CAMERA": capture.camera,
        "HEX_REVIEW_CAPTURE": "$UNPUBLISHED_PACK/" + profile.id + "/" + capture.filename,
        "HEX_REVIEW_FOG": profile.fog_mode,
        "HEX_REVIEW_MATERIAL": profile.material_treatment,
        "HEX_REVIEW_EDGE": profile.edge_treatment,
        "HEX_REVIEW_LIQUID_PHASE": "0.0",
        "HEX_REVIEW_SCENARIO": registry.scenario,
        "HEX_REVIEW_SEED": str(registry.seed),
        "HEX_REVIEW_VIEW": capture.view,
    }
    if profile.time_hours is not None:
        result["HEX_REVIEW_TIME"] = _format_float(profile.time_hours)
    if profile.crystal_light_profile is not None:
        result["HEX_REVIEW_CRYSTAL_LIGHT_PROFILE"] = profile.crystal_light_profile
    if capture.focus_anchor is not None:
        result["HEX_REVIEW_FOCUS_ANCHOR"] = capture.focus_anchor
    if capture.look_at_anchor is not None:
        result["HEX_REVIEW_LOOK_AT_ANCHOR"] = capture.look_at_anchor
        result["HEX_REVIEW_LOOK_AT_OFFSET"] = ",".join(
            _format_float(value) for value in capture.look_at_offset or ()
        )
    if capture.cutaway is not None:
        result["HEX_REVIEW_CUTAWAY"] = capture.cutaway
    if capture.illumination_overlay is not None:
        result["HEX_REVIEW_ILLUMINATION"] = capture.illumination_overlay
    if allow_structural_draft:
        result["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"] = "1"
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
    review_binary: Optional[ReviewBinary] = None,
    allow_structural_draft: bool = False,
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
        allow_structural_draft=allow_structural_draft,
    )
    if review_binary is not None:
        if sha256_file(review_binary.path) != review_binary.sha256:
            raise CaptureError("review binary changed after the build-once checkpoint")
        command = [str(review_binary.path)]
    else:
        command = _recorded_capture_command(None)
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
            "look_at_anchor": capture.look_at_anchor,
            "look_at_offset": list(capture.look_at_offset)
            if capture.look_at_offset is not None
            else None,
            "liquid_phase": 0.0,
            "cutaway": capture.cutaway,
            "illumination_overlay": capture.illumination_overlay,
            "command": _recorded_capture_command(review_binary),
            "environment": _tokenized_environment(
                registry,
                profile,
                capture,
                allow_structural_draft=allow_structural_draft,
            ),
            "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
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
    captures: Optional[Sequence[CaptureSpec]] = None,
    *,
    allow_structural_draft: bool = False,
) -> None:
    captures = tuple(captures or registry.captures)
    comparison_report = comparison_report_metadata(profiles, captures)
    lines = [
        "# Grand V3 visual experiment review",
        "",
        "COMPLETE CAPTURE SET — PIXEL REVIEW REQUIRED",
        "",
        f"Scenario: `{registry.scenario}`  ",
        f"Seed: `{registry.seed}`  ",
        f"Matrix: `{comparison_report['selection_id']}` / `{comparison_report['semantic_sha256']}`  ",
        "Static frames do not clear flicker, popping, animation, or camera motion.",
        "",
    ]
    if allow_structural_draft:
        lines.extend(
            (
                "UNAPPROVABLE STRUCTURAL DRAFT — fail-open world checks were explicitly enabled.",
                "Use this pack only to diagnose composition; recapture without the draft flag.",
                "",
            )
        )
    for profile in profiles:
        lines.extend(
            (
                f"## {profile.id} — {profile.label} — fog: {profile.fog_mode} — "
                f"material: {profile.material_treatment} — edge: {profile.edge_treatment} — crystal: "
                f"{profile.crystal_light_profile or 'current'}",
                "",
            )
        )
        for capture in captures:
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
    captures: Optional[Sequence[CaptureSpec]] = None,
    *,
    allow_structural_draft: bool = False,
) -> None:
    captures = tuple(captures or registry.captures)
    comparison_report = comparison_report_metadata(profiles, captures)
    baseline = next(
        (profile for profile in profiles if profile.id == "e00-baseline"), None
    )
    candidates = tuple(
        profile for profile in profiles if profile.id != "e00-baseline"
    )
    if candidates and baseline is None:
        raise ExperimentError("candidate HTML comparisons require e00-baseline")
    capture_sections = []
    for capture in captures:
        images = "\n".join(
            '<figure><img loading="lazy" src="profiles/{profile}/{filename}" '
            'alt="{profile} {capture}"><figcaption>{profile} — {label} — fog: {fog_mode} — material: {material} — edge: {edge} — crystal: {crystal}</figcaption></figure>'.format(
                profile=profile.id,
                filename=capture.filename,
                capture=capture.id,
                label=html.escape(profile.label),
                fog_mode=html.escape(profile.fog_mode),
                material=html.escape(profile.material_treatment),
                edge=html.escape(profile.edge_treatment),
                crystal=html.escape(profile.crystal_light_profile or "current"),
            )
            for profile in profiles
        )
        capture_sections.append(
            f'<section id="capture-{capture.id}"><h3>{capture.id}</h3><div class="grid">{images}</div></section>'
        )
    axis_sections = []
    if baseline is not None:
        baseline_images = "\n".join(
            '<figure><img loading="lazy" src="profiles/{profile}/{filename}" '
            'alt="{profile} {capture}"><figcaption>Baseline — {capture} — fog: {fog_mode} — material: {material} — edge: {edge} — crystal: {crystal}</figcaption></figure>'.format(
                profile=baseline.id,
                filename=capture.filename,
                capture=capture.id,
                fog_mode=html.escape(baseline.fog_mode),
                material=html.escape(baseline.material_treatment),
                edge=html.escape(baseline.edge_treatment),
                crystal=html.escape(baseline.crystal_light_profile or "current"),
            )
            for capture in captures
        )
        axis_sections.append(
            '<section id="axis-baseline"><h3>baseline reference — fog: current — material: current — edge: current — crystal: current</h3>'
            f'<div class="grid">{baseline_images}</div></section>'
        )
    axes = tuple(dict.fromkeys(profile.axis for profile in candidates))
    for axis in axes:
        cards = []
        for profile in (item for item in candidates if item.axis == axis):
            comparisons = "\n".join(
                '<section class="comparison" data-baseline="{baseline}" '
                'data-candidate="{candidate}" data-capture="{capture}">'
                '<h5>{capture}</h5><div class="comparison-grid">'
                '<figure><img loading="lazy" src="profiles/{baseline}/{filename}" '
                'alt="{baseline} {capture}"><figcaption>Baseline — {capture} — fog: {baseline_fog} — material: {baseline_material} — edge: {baseline_edge} — crystal: {baseline_crystal}</figcaption></figure>'
                '<figure><img loading="lazy" src="profiles/{candidate}/{filename}" '
                'alt="{candidate} {capture}"><figcaption>{candidate} — {capture} — fog: {candidate_fog} — material: {candidate_material} — edge: {candidate_edge} — crystal: {candidate_crystal}</figcaption></figure>'
                '</div></section>'.format(
                    baseline=baseline.id,
                    candidate=profile.id,
                    filename=capture.filename,
                    capture=capture.id,
                    baseline_fog=html.escape(baseline.fog_mode),
                    candidate_fog=html.escape(profile.fog_mode),
                    baseline_material=html.escape(baseline.material_treatment),
                    candidate_material=html.escape(profile.material_treatment),
                    baseline_edge=html.escape(baseline.edge_treatment),
                    candidate_edge=html.escape(profile.edge_treatment),
                    baseline_crystal=html.escape(
                        baseline.crystal_light_profile or "current"
                    ),
                    candidate_crystal=html.escape(
                        profile.crystal_light_profile or "current"
                    ),
                )
                for capture in captures
            )
            cards.append(
                '<section class="candidate-comparisons">'
                f'<h4>{profile.id} — {html.escape(profile.label)} — fog: {html.escape(profile.fog_mode)} — material: {html.escape(profile.material_treatment)} — edge: {html.escape(profile.edge_treatment)} — crystal: {html.escape(profile.crystal_light_profile or "current")}</h4>'
                f'{comparisons}</section>'
            )
        axis_sections.append(
            f'<section id="axis-{axis}"><h3>{axis}</h3>{"".join(cards)}</section>'
        )
    draft_warning = (
        '<p class="warning">UNAPPROVABLE STRUCTURAL DRAFT — fail-open world checks were explicitly enabled.</p>'
        if allow_structural_draft
        else ""
    )
    html_content = """<!doctype html>
<meta charset="utf-8">
<title>Grand V3 visual experiments</title>
<style>
body { font: 16px system-ui; margin: 24px; background: #171b22; color: #edf2f7; }
.grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
.comparison { margin-bottom: 20px; }
.comparison-grid { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
figure { margin: 0; padding: 8px; background: #252b35; }
img { display: block; width: 100%; height: auto; }
figcaption { margin-top: 6px; }
.warning { padding: 12px; background: #6b2b2b; font-weight: 700; }
</style>
<h1>Grand V3 visual experiments</h1>
""" + draft_warning + """
<p>Every frame is mechanically captured and remains UNREVIEWED.</p>
<p>Matrix: <code>""" + html.escape(comparison_report["selection_id"]) + """</code> / <code>""" + html.escape(comparison_report["semantic_sha256"]) + """</code></p>
<nav><a href="#capture-first">Capture-first</a> · <a href="#axis-first">Axis-first</a></nav>
<h2 id="capture-first">Capture-first comparisons</h2>
""" + "\n".join(capture_sections) + "\n<h2 id=\"axis-first\">Axis-first comparisons</h2>\n" + "\n".join(axis_sections) + "\n"
    atomic_write(pack_root / "index.html", html_content)


def validate_complete_pack(
    pack_root: pathlib.Path,
    registry: Registry,
    profiles: Sequence[ProfileSpec],
    expected_provenance: Optional[Mapping[str, Any]] = None,
    expected_source_hashes: Optional[Mapping[str, str]] = None,
    expected_asset_tree_sha256: Optional[str] = None,
    captures: Optional[Sequence[CaptureSpec]] = None,
) -> None:
    captures = tuple(captures or registry.captures)
    expected_files = {
        "index.html",
        "manifest.json",
        "review-index.md",
        "logs/build.log",
    }
    for profile in profiles:
        expected_files.add(f"profiles/{profile.id}/profile.json")
        for capture in captures:
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
        "capture_ids",
        "capture_set",
        "comparison_report",
        "structural_draft",
        "review_binary",
        "runtime_report",
        "profiles",
    }
    manifest_profiles = manifest.get("profiles")
    manifest_provenance = manifest.get("provenance")
    manifest_source_hashes = manifest.get("source_hashes")
    expected_capture_count = len(profiles) * len(captures)
    review_binary_record = manifest.get("review_binary")
    valid_binary_record = (
        isinstance(review_binary_record, dict)
        and set(review_binary_record)
        == {
            "mode",
            "sha256",
            "build_command",
            "run_command",
            "structural_draft",
        }
        and isinstance(review_binary_record.get("structural_draft"), bool)
        and (
            (
                review_binary_record.get("mode") == "build-once"
                and isinstance(review_binary_record.get("sha256"), str)
                and SHA256_RE.fullmatch(review_binary_record["sha256"])
                and review_binary_record.get("build_command")
                == [
                    "cargo",
                    "build",
                    "--release",
                    "-p",
                    "hex_game",
                    "--features",
                    "map-review",
                    "--message-format=json-render-diagnostics",
                ]
                and review_binary_record.get("run_command") == ["$REVIEW_BINARY"]
            )
            or (
                review_binary_record.get("mode") == "cargo-run"
                and review_binary_record.get("sha256") is None
                and review_binary_record.get("build_command") is None
                and review_binary_record.get("run_command")
                == _recorded_capture_command(None)
            )
        )
    )
    if (
        set(manifest) != expected_manifest_fields
        or manifest.get("schema_version") != 1
        or manifest.get("review_status") != "UNREVIEWED"
        or manifest.get("motion_status") != "HUMAN-MOTION-PENDING"
        or manifest.get("scenario") != registry.scenario
        or manifest.get("seed") != registry.seed
        or manifest.get("profile_count") != len(profiles)
        or manifest.get("capture_count") != expected_capture_count
        or manifest.get("capture_ids") != [capture.id for capture in captures]
        or manifest.get("capture_set") not in CAPTURE_SET_IDS
        or manifest.get("comparison_report")
        != comparison_report_metadata(profiles, captures)
        or not isinstance(manifest.get("structural_draft"), bool)
        or review_binary_record.get("structural_draft")
        != manifest.get("structural_draft")
        or tuple(manifest.get("capture_ids", ()))
        != registry.capture_sets.get(manifest.get("capture_set"))
        or manifest.get("runtime_report") != RUNTIME_REPORT_PLACEHOLDER
        or not valid_binary_record
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
        if profile.palette is not None and not profile.baseline_alias:
            expected_modified_paths.append(registry.baseline["palette"])
        if profile.lighting_asset is not None:
            expected_modified_paths.append(registry.baseline["scenarios"])
        if profile.lighting_candidate is not None and not profile.baseline_alias:
            expected_modified_paths.append(registry.baseline["default_lighting"])
        expected_profile_fields = {
            "id",
            "axis",
            "label",
            "level_height",
            "lighting_asset",
            "lighting_candidate",
            "lighting_mode",
            "fog_mode",
            "material_treatment",
            "edge_treatment",
            "crystal_light_profile",
            "palette",
            "time_hours",
            "resolved_axis",
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
            or root_profile.get("lighting_candidate") != profile.lighting_candidate
            or root_profile.get("lighting_mode")
            != ("static" if profile.lighting_asset else "cycle")
            or root_profile.get("fog_mode") != profile.fog_mode
            or root_profile.get("material_treatment") != profile.material_treatment
            or root_profile.get("edge_treatment") != profile.edge_treatment
            or root_profile.get("crystal_light_profile")
            != profile.crystal_light_profile
            or root_profile.get("palette") != expected_palette
            or root_profile.get("time_hours") != profile.time_hours
            or root_profile.get("resolved_axis") != expected_resolved_axis(registry, profile)
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
            != [capture.id for capture in captures]
        ):
            raise ExperimentError(f"profile manifest capture set is invalid: {profile_path}")
        for capture_index, capture in enumerate(captures):
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
                "look_at_anchor": capture.look_at_anchor,
                "look_at_offset": list(capture.look_at_offset)
                if capture.look_at_offset is not None
                else None,
                "liquid_phase": 0.0,
                "cutaway": capture.cutaway,
                "illumination_overlay": capture.illumination_overlay,
                "command": manifest["review_binary"].get("run_command"),
                "environment": _tokenized_environment(
                    registry,
                    profile,
                    capture,
                    allow_structural_draft=manifest["structural_draft"],
                ),
                "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
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
    draft_marker = "UNAPPROVABLE STRUCTURAL DRAFT"
    if manifest["structural_draft"] != (draft_marker in review_index):
        raise ExperimentError("pack review index structural-draft status is inconsistent")
    if manifest["structural_draft"] != (draft_marker in html_index):
        raise ExperimentError("pack HTML structural-draft status is inconsistent")
    comparison_digest = manifest["comparison_report"]["semantic_sha256"]
    if comparison_digest not in review_index or comparison_digest not in html_index:
        raise ExperimentError("pack indexes omit the comparison-report identity")
    for profile in profiles:
        resolved_label = (
            f"{profile.id} — {html.escape(profile.label)} — fog: "
            f"{html.escape(profile.fog_mode)} — material: "
            f"{html.escape(profile.material_treatment)} — edge: "
            f"{html.escape(profile.edge_treatment)} — crystal: "
            f"{html.escape(profile.crystal_light_profile or 'current')}"
        )
        if resolved_label not in html_index:
            raise ExperimentError(
                f"pack HTML omits resolved fog/material/edge/crystal state for {profile.id}"
            )
        for capture in captures:
            relative = f"profiles/{profile.id}/{capture.filename}"
            if relative not in review_index or relative not in html_index:
                raise ExperimentError(f"pack indexes omit capture {relative}")
            if profile.id != "e00-baseline":
                comparison_marker = (
                    'data-baseline="e00-baseline" '
                    f'data-candidate="{profile.id}" data-capture="{capture.id}"'
                )
                if html_index.count(comparison_marker) != 1:
                    raise ExperimentError(
                        "pack HTML must pair baseline exactly once with candidate "
                        f"{profile.id} capture {capture.id}"
                    )


def _default_output(
    provenance: Mapping[str, Any],
    registry: Registry,
    profiles: Sequence[ProfileSpec],
    capture_set: str = DEFAULT_CAPTURE_SET,
) -> pathlib.Path:
    state = (
        "dirty-" + str(provenance["workspace_content_sha256"])[:12]
        if provenance["worktree_dirty"]
        else "clean"
    )
    selection = (
        f"matrix-v3-{capture_set}"
        if tuple(profile.id for profile in profiles) == EXPECTED_PROFILE_IDS
        else f"initial-screen-{capture_set}"
        if tuple(profile.id for profile in profiles) == INITIAL_SCREEN_PROFILE_IDS
        else "profiles-" + "-".join(profile.id for profile in profiles) + "-" + capture_set
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
    captures: Optional[Sequence[CaptureSpec]] = None,
    capture_set: str = DEFAULT_CAPTURE_SET,
    resource_limits: Optional[ResourceLimits] = None,
    allow_structural_draft: bool = False,
) -> Dict[str, Any]:
    captures = tuple(captures or registry.captures)
    limits = resource_limits or ResourceLimits(
        1800,
        DEFAULT_TOTAL_TIMEOUT_SECONDS,
        DEFAULT_MAX_WORK_BYTES,
        DEFAULT_MIN_FREE_BYTES,
    )
    return {
        "schema_version": 1,
        "mode": "dry-run",
        "output": output.relative_to(REPOSITORY_ROOT).as_posix(),
        "provenance": dict(provenance),
        "scenario": registry.scenario,
        "seed": registry.seed,
        "capture_set": capture_set,
        "capture_ids": [capture.id for capture in captures],
        "comparison_report": comparison_report_metadata(profiles, captures),
        "structural_draft": allow_structural_draft,
        "build": {
            "command": [
                "cargo",
                "build",
                "--release",
                "-p",
                "hex_game",
                "--features",
                "map-review",
                "--message-format=json-render-diagnostics",
            ],
            "capture_command": ["$REVIEW_BINARY"],
            "reuse": "one exact SHA-256-verified executable for every capture",
            "structural_draft": allow_structural_draft,
        },
        "resource_limits": {
            "capture_timeout_seconds": limits.capture_timeout_seconds,
            "total_timeout_seconds": limits.total_timeout_seconds,
            "max_work_bytes": limits.max_work_bytes,
            "min_free_bytes": limits.min_free_bytes,
        },
        "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
        "source_hashes": dict(source_hashes),
        "profiles": [
            {
                "id": profile.id,
                "axis": profile.axis,
                "time_hours": profile.time_hours,
                "lighting_asset": profile.lighting_asset,
                "lighting_candidate": profile.lighting_candidate,
                "fog_mode": profile.fog_mode,
                "material_treatment": profile.material_treatment,
                "edge_treatment": profile.edge_treatment,
                "crystal_light_profile": profile.crystal_light_profile,
                "level_height": profile.level_height,
                "palette": profile.palette,
                "resolved_axis": expected_resolved_axis(registry, profile),
                "captures": [
                    {
                        "id": capture.id,
                        "command": ["$REVIEW_BINARY"],
                        "environment": _tokenized_environment(
                            registry,
                            profile,
                            capture,
                            allow_structural_draft=allow_structural_draft,
                        ),
                    }
                    for capture in captures
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
    captures: Optional[Sequence[CaptureSpec]] = None,
    resource_limits: Optional[ResourceLimits] = None,
    build_once: bool = False,
    capture_set: str = DEFAULT_CAPTURE_SET,
    allow_structural_draft: bool = False,
) -> None:
    captures = tuple(captures or registry.captures)
    if captures != registry.captures_for(capture_set):
        raise ExperimentError(
            f"captures do not exactly match the declared {capture_set!r} set"
        )
    limits = resource_limits or ResourceLimits(
        timeout_seconds,
        DEFAULT_TOTAL_TIMEOUT_SECONDS,
        DEFAULT_MAX_WORK_BYTES,
        0,
    )
    deadline = time.monotonic() + limits.total_timeout_seconds
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
        _enforce_resource_limits(work, output.parent, limits, deadline)
        build_log = pack_root / "logs" / "build.log"
        build_log.parent.mkdir(parents=True, exist_ok=True)
        if build_once:
            review_binary = build_review_binary(
                repository_root,
                log_path=build_log,
                timeout_seconds=min(
                    timeout_seconds,
                    max(1, int(deadline - time.monotonic())),
                ),
            )
        else:
            atomic_write(build_log, "legacy cargo-run capture mode\n")
            review_binary = None
        binary_record = {
            "mode": "build-once" if review_binary is not None else "cargo-run",
            "sha256": review_binary.sha256 if review_binary is not None else None,
            "build_command": list(review_binary.command)
            if review_binary is not None
            else None,
            "run_command": _recorded_capture_command(review_binary),
            "structural_draft": allow_structural_draft,
        }
        _enforce_resource_limits(work, output.parent, limits, deadline)
        for profile in profiles:
            _enforce_resource_limits(work, output.parent, limits, deadline)
            staged_root = runtime_root / profile.id / "asset-root"
            staged_root.parent.mkdir(parents=True, exist_ok=True)
            copy_asset_tree(repository_root / "assets", staged_root / "assets")
            state = apply_profile(repository_root, staged_root, registry, profile)
            staged_digest_before = state["staged_asset_tree_sha256"]
            make_tree_read_only(staged_root)
            profile_captures = []
            _enforce_resource_limits(work, output.parent, limits, deadline)
            for capture in captures:
                _enforce_resource_limits(work, output.parent, limits, deadline)
                remaining = max(1, int(deadline - time.monotonic()))
                record = _run_capture(
                    repository_root=repository_root,
                    staged_root=staged_root,
                    pack_root=pack_root,
                    runtime_root=runtime_root,
                    registry=registry,
                    profile=profile,
                    capture=capture,
                    timeout_seconds=min(timeout_seconds, remaining),
                    common_provenance={
                        **dict(provenance),
                        "source_hashes": dict(source_hashes),
                    },
                    profile_state={"id": profile.id, **state},
                    review_binary=review_binary,
                    allow_structural_draft=allow_structural_draft,
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

        if review_binary is not None and sha256_file(review_binary.path) != review_binary.sha256:
            raise CaptureError("review binary changed during the capture matrix")

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
            "capture_ids": [capture.id for capture in captures],
            "capture_set": capture_set,
            "comparison_report": comparison_report_metadata(profiles, captures),
            "structural_draft": allow_structural_draft,
            "review_binary": binary_record,
            "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
            "profiles": profile_records,
        }
        atomic_write(pack_root / "manifest.json", canonical_json(manifest))
        _write_review_index(
            pack_root,
            registry,
            profiles,
            captures,
            allow_structural_draft=allow_structural_draft,
        )
        _write_html_index(
            pack_root,
            registry,
            profiles,
            captures,
            allow_structural_draft=allow_structural_draft,
        )
        validate_complete_pack(
            pack_root,
            registry,
            profiles,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
            captures=captures,
        )
        _enforce_resource_limits(work, output.parent, limits, deadline)
        if tree_digest(repository_root / "assets") != asset_source_digest:
            raise CaptureError("tracked source assets changed before publication")
        if relevant_source_hashes(repository_root, registry) != dict(source_hashes):
            raise CaptureError("experiment sources changed before publication")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("Git head or worktree content changed before publication")
        atomic_publish_no_replace(pack_root, output)
    finally:
        _remove_tree(work)


def _utc_timestamp() -> str:
    return (
        datetime.datetime.now(datetime.timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z")
    )


def resolve_sweep_output_root(
    raw: str,
    repository_root: pathlib.Path = REPOSITORY_ROOT,
) -> pathlib.Path:
    """Resolve a named sweep root without allowing broad or tracked destinations."""

    candidate = pathlib.Path(raw)
    if candidate.is_symlink():
        raise ExperimentError("sweep output-root may not be a symlink")
    if not candidate.is_absolute():
        candidate = repository_root / candidate
    output = candidate.resolve()
    forbidden = {
        pathlib.Path("/").resolve(),
        pathlib.Path.home().resolve(),
        repository_root.resolve(),
    }
    if output in forbidden or not output.name:
        raise ExperimentError("sweep output-root must be a narrow named directory")
    try:
        output.relative_to(repository_root.resolve())
    except ValueError:
        pass
    else:
        try:
            output.relative_to(EXPERIMENT_ROOT.resolve())
        except ValueError as error:
            raise ExperimentError(
                "sweep output-root inside the repository must stay under "
                f"{EXPERIMENT_ROOT.resolve()}"
            ) from error
    return output


def sweep_shard_output(
    output_root: pathlib.Path,
    sweep: SweepSpec,
    tier: SweepTier,
    shard: int,
) -> pathlib.Path:
    tier.looks_for_shard(shard)
    return output_root / sweep.id / tier.id / f"shard-{shard:02d}"


def sweep_shard_semantic_sha256(
    sweep: SweepSpec, tier: SweepTier, shard: int
) -> str:
    body = {
        "sweep_id": sweep.id,
        "sweep_semantic_sha256": sweep.semantic_sha256,
        "tier": tier.id,
        "shard": shard,
        "shard_count": tier.shard_count,
        "capture_ids": list(tier.capture_ids),
        "look_ids": [look.id for look in tier.looks_for_shard(shard)],
    }
    return sha256_bytes(canonical_json(body).encode("utf-8"))


def sweep_source_relative(sweep: SweepSpec) -> str:
    return f"tools/visual_experiments/sweeps/{sweep.path.name}"


def sweep_look_plan(
    registry: Registry,
    sweep: SweepSpec,
    look: SweepLook,
    captures: Sequence[CaptureSpec],
    *,
    allow_structural_draft: bool,
) -> Dict[str, Any]:
    profile = sweep_profile(look)
    return {
        "id": look.id,
        "label": look.label,
        "semantic_sha256": look.semantic_sha256,
        "axes": {axis: dict(look.values[axis]) for axis in sweep.axis_order},
        "runtime": {
            "time_hours": profile.time_hours,
            "fog_mode": profile.fog_mode,
            "material_treatment": profile.material_treatment,
            "edge_treatment": profile.edge_treatment,
        },
        "captures": [
            {
                "id": capture.id,
                "command": ["$REVIEW_BINARY"],
                "environment": _tokenized_environment(
                    registry,
                    profile,
                    capture,
                    allow_structural_draft=allow_structural_draft,
                ),
            }
            for capture in captures
        ],
    }


def build_sweep_plan(
    registry: Registry,
    sweep: SweepSpec,
    tier: SweepTier,
    shard: int,
    provenance: Mapping[str, Any],
    source_hashes: Mapping[str, str],
    output_root: pathlib.Path,
    resource_limits: ResourceLimits,
    *,
    allow_structural_draft: bool,
) -> Dict[str, Any]:
    looks = tier.looks_for_shard(shard)
    captures_by_id = {capture.id: capture for capture in registry.captures}
    captures = tuple(captures_by_id[capture_id] for capture_id in tier.capture_ids)
    output = sweep_shard_output(output_root, sweep, tier, shard)
    return {
        "schema_version": 1,
        "mode": "sweep-dry-run",
        "output": output.as_posix(),
        "structural_draft": allow_structural_draft,
        "scenario": registry.scenario,
        "seed": registry.seed,
        "provenance": dict(provenance),
        "source_hashes": dict(source_hashes),
        "sweep": {
            "id": sweep.id,
            "source": sweep_source_relative(sweep),
            "semantic_sha256": sweep.semantic_sha256,
            "tier": tier.id,
            "tier_look_count": len(tier.looks),
            "shard": shard,
            "shard_count": tier.shard_count,
            "shard_look_count": len(looks),
            "shard_semantic_sha256": sweep_shard_semantic_sha256(
                sweep, tier, shard
            ),
        },
        "capture_ids": list(tier.capture_ids),
        "render_count": len(looks) * len(captures),
        "resource_limits": {
            "capture_timeout_seconds": resource_limits.capture_timeout_seconds,
            "total_timeout_seconds": resource_limits.total_timeout_seconds,
            "max_work_bytes": resource_limits.max_work_bytes,
            "min_free_bytes": resource_limits.min_free_bytes,
        },
        "build": {
            "command": [
                "cargo",
                "build",
                "--release",
                "-p",
                "hex_game",
                "--features",
                "map-review",
                "--message-format=json-render-diagnostics",
            ],
            "reuse": "one exact SHA-256-verified executable for this shard",
        },
        "looks": [
            sweep_look_plan(
                registry,
                sweep,
                look,
                captures,
                allow_structural_draft=allow_structural_draft,
            )
            for look in looks
        ],
    }


def _run_sweep_capture_with_retry(
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
    review_binary: ReviewBinary,
    allow_structural_draft: bool,
) -> Dict[str, Any]:
    retry_logs: List[str] = []
    for attempt in (1, 2):
        started = _utc_timestamp()
        try:
            record = _run_capture(
                repository_root=repository_root,
                staged_root=staged_root,
                pack_root=pack_root,
                runtime_root=runtime_root,
                registry=registry,
                profile=profile,
                capture=capture,
                timeout_seconds=timeout_seconds,
                common_provenance=common_provenance,
                profile_state=profile_state,
                review_binary=review_binary,
                allow_structural_draft=allow_structural_draft,
            )
            sidecar_path = pack_root / str(record["sidecar"])
            sidecar = _read_json(sidecar_path, context="sweep capture sidecar")
            sidecar["capture_started_at_utc"] = started
            sidecar["capture_completed_at_utc"] = _utc_timestamp()
            sidecar["attempt_count"] = attempt
            sidecar["retry_logs"] = list(retry_logs)
            atomic_write(sidecar_path, canonical_json(sidecar))
            return {
                **record,
                "attempt_count": attempt,
                "retry_logs": list(retry_logs),
            }
        except (CaptureError, ExperimentError):
            profile_root = pack_root / "profiles" / profile.id
            log_path = profile_root / "logs" / f"{capture.id}.log"
            if attempt == 1 and log_path.is_file():
                retry_path = profile_root / "logs" / f"{capture.id}.attempt-1.log"
                log_path.replace(retry_path)
                retry_logs.append(
                    retry_path.relative_to(pack_root).as_posix()
                )
            _remove_tree(runtime_root / "data" / profile.id / capture.id)
            for artifact in (
                profile_root / capture.filename,
                (profile_root / capture.filename).with_suffix(".manifest.json"),
                log_path,
            ):
                try:
                    artifact.unlink()
                except FileNotFoundError:
                    pass
            if attempt == 2:
                raise
    raise CaptureError("unreachable sweep capture retry state")


def _write_sweep_indexes(
    pack_root: pathlib.Path,
    sweep: SweepSpec,
    tier: SweepTier,
    shard: int,
    captures: Sequence[CaptureSpec],
    *,
    allow_structural_draft: bool,
) -> None:
    warning = (
        "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY"
        if allow_structural_draft
        else "AESTHETIC REVIEW ONLY — STRUCTURAL CLAIMS ARE OUT OF SCOPE"
    )
    lines = [
        f"# {sweep.id} — {tier.id} shard {shard}/{tier.shard_count}",
        "",
        warning,
        "",
        f"Sweep: `{sweep.semantic_sha256}`  ",
        f"Shard: `{sweep_shard_semantic_sha256(sweep, tier, shard)}`  ",
        "Every still remains `UNREVIEWED`; static frames do not clear motion.",
        "",
    ]
    figures = []
    for look in tier.looks_for_shard(shard):
        lines.append(f"## {look.id} — {look.label}")
        lines.append("")
        for capture in captures:
            relative = f"profiles/{look.id}/{capture.filename}"
            lines.append(f"- `UNREVIEWED` [{capture.id}]({relative})")
            lines.append("  - Notes:")
            figures.append(
                '<figure data-look="{look}" data-height="{height}" '
                'data-light="{light}" data-palette="{palette}" '
                'data-haze="{haze}" data-edge="{edge}">'
                '<img loading="lazy" src="{path}" alt="{look} {capture}">'
                '<figcaption>{look}<br>{label}</figcaption></figure>'.format(
                    look=html.escape(look.id),
                    height=html.escape(str(look.values["height"]["id"])),
                    light=html.escape(str(look.values["light"]["id"])),
                    palette=html.escape(str(look.values["palette"]["id"])),
                    haze=html.escape(str(look.values["haze"]["id"])),
                    edge=html.escape(str(look.values["edge"]["id"])),
                    path=html.escape(relative),
                    capture=html.escape(capture.id),
                    label=html.escape(look.label),
                )
            )
        lines.append("")
    atomic_write(pack_root / "review-index.md", "\n".join(lines))
    document = """<!doctype html>
<meta charset="utf-8">
<title>Interaction sweep shard</title>
<style>
body {{ font: 15px system-ui; margin: 24px; background: #171b22; color: #edf2f7; }}
.warning {{ padding: 12px; background: #6b2b2b; font-weight: 700; }}
.grid {{ display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }}
figure {{ margin: 0; padding: 7px; background: #252b35; }}
img {{ display: block; width: 100%; height: auto; }}
figcaption {{ margin-top: 6px; overflow-wrap: anywhere; }}
</style>
<h1>Interaction sweep shard</h1>
<p class="warning">{warning}</p>
<p>Sweep <code>{sweep}</code>; shard <code>{shard_hash}</code>.</p>
<div class="grid">{figures}</div>
""".format(
        warning=html.escape(warning),
        sweep=html.escape(sweep.semantic_sha256),
        shard_hash=html.escape(sweep_shard_semantic_sha256(sweep, tier, shard)),
        figures="\n".join(figures),
    )
    atomic_write(pack_root / "index.html", document)


def validate_sweep_pack(
    pack_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    tier: SweepTier,
    shard: int,
    *,
    expected_provenance: Mapping[str, Any],
    expected_source_hashes: Mapping[str, str],
    expected_asset_tree_sha256: Optional[str] = None,
) -> None:
    """Validate one published or unpublished sweep shard for safe resume."""

    if pack_root.is_symlink() or not pack_root.is_dir():
        raise ExperimentError(f"sweep pack is not a regular directory: {pack_root}")
    manifest = _read_json(pack_root / "manifest.json", context="sweep manifest")
    looks = tier.looks_for_shard(shard)
    expected_look_ids = [look.id for look in looks]
    if manifest.get("schema_version") != 1 or manifest.get("kind") != "interaction-sweep-shard":
        raise ExperimentError("sweep manifest has an unsupported schema or kind")
    if manifest.get("review_status") != "UNREVIEWED":
        raise ExperimentError("sweep manifest review status must remain UNREVIEWED")
    if manifest.get("provenance") != dict(expected_provenance):
        raise ExperimentError("sweep manifest provenance differs from current inputs")
    if manifest.get("source_hashes") != dict(expected_source_hashes):
        raise ExperimentError("sweep manifest source hashes differ from current inputs")
    if manifest.get("scenario") != registry.scenario or manifest.get("seed") != registry.seed:
        raise ExperimentError("sweep manifest scenario or seed differs")
    sweep_record = manifest.get("sweep")
    expected_sweep_record = {
        "id": sweep.id,
        "source": sweep_source_relative(sweep),
        "semantic_sha256": sweep.semantic_sha256,
        "tier": tier.id,
        "tier_look_count": len(tier.looks),
        "shard": shard,
        "shard_count": tier.shard_count,
        "shard_look_count": len(looks),
        "shard_semantic_sha256": sweep_shard_semantic_sha256(sweep, tier, shard),
    }
    if sweep_record != expected_sweep_record:
        raise ExperimentError("sweep manifest identity differs from the requested shard")
    if manifest.get("capture_ids") != list(tier.capture_ids):
        raise ExperimentError("sweep manifest capture ids differ")
    if manifest.get("look_ids") != expected_look_ids:
        raise ExperimentError("sweep manifest look ids differ")
    if expected_asset_tree_sha256 is not None and manifest.get(
        "source_asset_tree_sha256"
    ) != expected_asset_tree_sha256:
        raise ExperimentError("sweep manifest source asset tree differs")
    profile_records = manifest.get("profiles")
    if not isinstance(profile_records, list) or [
        record.get("id") for record in profile_records if isinstance(record, dict)
    ] != expected_look_ids:
        raise ExperimentError("sweep manifest profile records differ")
    captures_by_id = {capture.id: capture for capture in registry.captures}
    expected_files = {
        "manifest.json",
        "review-index.md",
        "index.html",
        "logs/build.log",
    }
    profile_record_by_id = {record["id"]: record for record in profile_records}
    for look in looks:
        record = profile_record_by_id[look.id]
        if record.get("look_semantic_sha256") != look.semantic_sha256:
            raise ExperimentError(f"sweep profile {look.id} semantic identity differs")
        expected_files.add(f"profiles/{look.id}/profile.json")
        captures = record.get("captures")
        if not isinstance(captures, list) or [
            capture.get("id") for capture in captures if isinstance(capture, dict)
        ] != list(tier.capture_ids):
            raise ExperimentError(f"sweep profile {look.id} captures differ")
        for capture_record in captures:
            capture = captures_by_id[capture_record["id"]]
            image_relative = f"profiles/{look.id}/{capture.filename}"
            sidecar_relative = f"profiles/{look.id}/{pathlib.Path(capture.filename).with_suffix('.manifest.json').name}"
            log_relative = f"profiles/{look.id}/logs/{capture.id}.log"
            expected_files.update((image_relative, sidecar_relative, log_relative))
            for retry_log in capture_record.get("retry_logs", []):
                expected_files.add(str(retry_log))
            image_path = pack_root / image_relative
            inspect_png(image_path)
            if capture_record.get("sha256") != sha256_file(image_path):
                raise ExperimentError(f"sweep capture hash differs: {image_relative}")
            sidecar = _read_json(pack_root / sidecar_relative, context="sweep sidecar")
            if sidecar.get("profile", {}).get("id") != look.id:
                raise ExperimentError(f"sweep sidecar profile differs: {sidecar_relative}")
            if sidecar.get("profile", {}).get("look_semantic_sha256") != look.semantic_sha256:
                raise ExperimentError(f"sweep sidecar semantic identity differs: {sidecar_relative}")
            if sidecar.get("provenance") != {
                **dict(expected_provenance),
                "source_hashes": dict(expected_source_hashes),
            }:
                raise ExperimentError(f"sweep sidecar provenance differs: {sidecar_relative}")
            if not isinstance(sidecar.get("capture_started_at_utc"), str) or not isinstance(
                sidecar.get("capture_completed_at_utc"), str
            ):
                raise ExperimentError(f"sweep sidecar timestamps missing: {sidecar_relative}")
            if sidecar.get("artifact", {}).get("sha256") != sha256_file(image_path):
                raise ExperimentError(f"sweep sidecar artifact hash differs: {sidecar_relative}")
    actual_files = {
        path.relative_to(pack_root).as_posix()
        for path in _tree_files(pack_root)
    }
    if actual_files != expected_files:
        raise ExperimentError(
            "sweep pack file set differs; missing="
            f"{sorted(expected_files - actual_files)}, extra={sorted(actual_files - expected_files)}"
        )
    review_index = (pack_root / "review-index.md").read_text(encoding="utf-8")
    html_index = (pack_root / "index.html").read_text(encoding="utf-8")
    for identity in (sweep.semantic_sha256, sweep_shard_semantic_sha256(sweep, tier, shard)):
        if identity not in review_index or identity not in html_index:
            raise ExperimentError("sweep indexes omit their semantic identity")


def run_sweep_shard(
    *,
    repository_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    tier: SweepTier,
    shard: int,
    provenance: Mapping[str, Any],
    source_hashes: Mapping[str, str],
    output_root: pathlib.Path,
    resource_limits: ResourceLimits,
    allow_structural_draft: bool,
) -> Tuple[pathlib.Path, bool]:
    """Capture one atomic shard; return ``(path, resumed)``."""

    require_capturable_sweep(sweep)
    looks = tier.looks_for_shard(shard)
    captures_by_id = {capture.id: capture for capture in registry.captures}
    captures = tuple(captures_by_id[capture_id] for capture_id in tier.capture_ids)
    output = sweep_shard_output(output_root, sweep, tier, shard)
    asset_source_digest = tree_digest(repository_root / "assets")
    if output.exists():
        validate_sweep_pack(
            output,
            registry,
            sweep,
            tier,
            shard,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
        )
        return output, True
    output.parent.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(
        tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)
    )
    pack_root = work / "pack"
    runtime_root = work / "runtime"
    pack_root.mkdir()
    atomic_write(
        pack_root / "review-index.md",
        "# Interaction sweep shard\n\nINCOMPLETE — NOT REVIEWABLE\n",
    )
    deadline = time.monotonic() + resource_limits.total_timeout_seconds
    started_at = _utc_timestamp()
    profile_records: List[Dict[str, Any]] = []
    try:
        _enforce_resource_limits(work, output.parent, resource_limits, deadline)
        build_log = pack_root / "logs" / "build.log"
        build_log.parent.mkdir(parents=True, exist_ok=True)
        review_binary = build_review_binary(
            repository_root,
            log_path=build_log,
            timeout_seconds=min(
                resource_limits.capture_timeout_seconds,
                max(1, int(deadline - time.monotonic())),
            ),
        )
        binary_record = {
            "mode": "build-once",
            "sha256": review_binary.sha256,
            "build_command": list(review_binary.command),
            "run_command": ["$REVIEW_BINARY"],
            "structural_draft": allow_structural_draft,
        }
        for look in looks:
            _enforce_resource_limits(work, output.parent, resource_limits, deadline)
            profile = sweep_profile(look)
            staged_root = runtime_root / look.id / "asset-root"
            staged_root.parent.mkdir(parents=True, exist_ok=True)
            copy_asset_tree(repository_root / "assets", staged_root / "assets")
            state = apply_sweep_look(
                repository_root, staged_root, registry, sweep, look
            )
            staged_digest_before = state["staged_asset_tree_sha256"]
            make_tree_read_only(staged_root)
            capture_records = []
            for capture in captures:
                _enforce_resource_limits(work, output.parent, resource_limits, deadline)
                remaining = max(1, int(deadline - time.monotonic()))
                record = _run_sweep_capture_with_retry(
                    repository_root=repository_root,
                    staged_root=staged_root,
                    pack_root=pack_root,
                    runtime_root=runtime_root,
                    registry=registry,
                    profile=profile,
                    capture=capture,
                    timeout_seconds=min(
                        resource_limits.capture_timeout_seconds, remaining
                    ),
                    common_provenance={
                        **dict(provenance),
                        "source_hashes": dict(source_hashes),
                    },
                    profile_state={"id": look.id, **state},
                    review_binary=review_binary,
                    allow_structural_draft=allow_structural_draft,
                )
                capture_records.append(record)
            if tree_digest(staged_root / "assets") != staged_digest_before:
                raise ExperimentError(f"runtime mutated staged assets for {look.id}")
            profile_record = {
                "id": look.id,
                **state,
                "captures": capture_records,
            }
            atomic_write(
                pack_root / "profiles" / look.id / "profile.json",
                canonical_json({"schema_version": 1, **profile_record}),
            )
            profile_records.append(profile_record)
            _remove_tree(staged_root.parent)
        if sha256_file(review_binary.path) != review_binary.sha256:
            raise CaptureError("review binary changed during the sweep shard")
        if tree_digest(repository_root / "assets") != asset_source_digest:
            raise ExperimentError("tracked source assets changed during the sweep shard")
        if sweep_source_hashes(repository_root, registry, sweep) != dict(source_hashes):
            raise ExperimentError("sweep sources changed during the sweep shard")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("Git head or worktree content changed during the sweep shard")
        manifest = {
            "schema_version": 1,
            "kind": "interaction-sweep-shard",
            "review_status": "UNREVIEWED",
            "motion_status": "HUMAN-MOTION-PENDING",
            "structural_draft": allow_structural_draft,
            "started_at_utc": started_at,
            "completed_at_utc": _utc_timestamp(),
            "provenance": dict(provenance),
            "scenario": registry.scenario,
            "seed": registry.seed,
            "source_asset_tree_sha256": asset_source_digest,
            "source_hashes": dict(source_hashes),
            "sweep": {
                "id": sweep.id,
                "source": sweep_source_relative(sweep),
                "semantic_sha256": sweep.semantic_sha256,
                "tier": tier.id,
                "tier_look_count": len(tier.looks),
                "shard": shard,
                "shard_count": tier.shard_count,
                "shard_look_count": len(looks),
                "shard_semantic_sha256": sweep_shard_semantic_sha256(
                    sweep, tier, shard
                ),
            },
            "capture_ids": list(tier.capture_ids),
            "look_ids": [look.id for look in looks],
            "render_count": len(looks) * len(captures),
            "review_binary": binary_record,
            "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
            "profiles": profile_records,
        }
        atomic_write(pack_root / "manifest.json", canonical_json(manifest))
        _write_sweep_indexes(
            pack_root,
            sweep,
            tier,
            shard,
            captures,
            allow_structural_draft=allow_structural_draft,
        )
        validate_sweep_pack(
            pack_root,
            registry,
            sweep,
            tier,
            shard,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
        )
        _enforce_resource_limits(work, output.parent, resource_limits, deadline)
        if sweep_source_hashes(repository_root, registry, sweep) != dict(source_hashes):
            raise CaptureError("sweep sources changed before shard publication")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("worktree changed before shard publication")
        atomic_publish_no_replace(pack_root, output)
        return output, False
    finally:
        _remove_tree(work)


def selection_shard_output(
    output_root: pathlib.Path,
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
) -> pathlib.Path:
    selection.recipes_for_shard(shard)
    return (
        output_root
        / sweep.id
        / "selections"
        / selection.id
        / f"shard-{shard:02d}"
    )


def selection_shard_semantic_sha256(
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
) -> str:
    body = {
        "sweep_id": sweep.id,
        "sweep_semantic_sha256": sweep.semantic_sha256,
        "selection_id": selection.id,
        "selection_semantic_sha256": selection.semantic_sha256,
        "stage": selection.stage,
        "shard": shard,
        "shard_count": selection.shard_count,
        "capture_ids": [capture.id for capture in selection.captures],
        "recipe_ids": [
            recipe.id for recipe in selection.recipes_for_shard(shard)
        ],
    }
    return sha256_bytes(canonical_json(body).encode("utf-8"))


def selection_recipe_plan(
    registry: Registry,
    recipe: SelectionRecipe,
    captures: Sequence[CaptureSpec],
    *,
    allow_structural_draft: bool,
) -> Dict[str, Any]:
    profile = selection_profile(recipe)
    return {
        "id": recipe.id,
        "label": recipe.label,
        "semantic_sha256": recipe.semantic_sha256,
        "base_look_id": recipe.base_look.id,
        "base_look_semantic_sha256": recipe.base_look.semantic_sha256,
        "base_axes": {
            axis: dict(value) for axis, value in recipe.base_look.values.items()
        },
        "overrides": dict(recipe.overrides),
        "resolved_runtime": {
            "time_hours": profile.time_hours,
            "fog_mode": profile.fog_mode,
            "material_treatment": profile.material_treatment,
            "edge_treatment": profile.edge_treatment,
            "crystal_light_profile": profile.crystal_light_profile or "current",
        },
        "captures": [
            {
                "id": capture.id,
                "command": ["$REVIEW_BINARY"],
                "environment": _tokenized_environment(
                    registry,
                    profile,
                    capture,
                    allow_structural_draft=allow_structural_draft,
                ),
            }
            for capture in captures
        ],
    }


def build_selection_plan(
    registry: Registry,
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
    provenance: Mapping[str, Any],
    source_hashes: Mapping[str, str],
    output_root: pathlib.Path,
    resource_limits: ResourceLimits,
    *,
    allow_structural_draft: bool,
) -> Dict[str, Any]:
    recipes = selection.recipes_for_shard(shard)
    output = selection_shard_output(output_root, sweep, selection, shard)
    return {
        "schema_version": 1,
        "mode": "selection-dry-run",
        "output": output.as_posix(),
        "structural_draft": allow_structural_draft,
        "scenario": registry.scenario,
        "seed": registry.seed,
        "provenance": dict(provenance),
        "source_hashes": dict(source_hashes),
        "sweep": {
            "id": sweep.id,
            "semantic_sha256": sweep.semantic_sha256,
        },
        "selection": {
            "id": selection.id,
            "stage": selection.stage,
            "source": f"selection/{selection.path.name}",
            "semantic_sha256": selection.semantic_sha256,
            "recipe_count": len(selection.recipes),
            "shard": shard,
            "shard_count": selection.shard_count,
            "shard_recipe_count": len(recipes),
            "shard_semantic_sha256": selection_shard_semantic_sha256(
                sweep, selection, shard
            ),
            "camera_manifest": (
                f"camera-manifest/{selection.camera_manifest_path.name}"
                if selection.camera_manifest_path is not None
                else None
            ),
        },
        "capture_ids": [capture.id for capture in selection.captures],
        "render_count": len(recipes) * len(selection.captures),
        "resource_limits": {
            "capture_timeout_seconds": resource_limits.capture_timeout_seconds,
            "total_timeout_seconds": resource_limits.total_timeout_seconds,
            "max_work_bytes": resource_limits.max_work_bytes,
            "min_free_bytes": resource_limits.min_free_bytes,
        },
        "build": {
            "command": [
                "cargo",
                "build",
                "--release",
                "-p",
                "hex_game",
                "--features",
                "map-review",
                "--message-format=json-render-diagnostics",
            ],
            "reuse": "one exact SHA-256-verified executable for this selection shard",
        },
        "recipes": [
            selection_recipe_plan(
                registry,
                recipe,
                selection.captures,
                allow_structural_draft=allow_structural_draft,
            )
            for recipe in recipes
        ],
    }


def _write_selection_indexes(
    pack_root: pathlib.Path,
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
    *,
    allow_structural_draft: bool,
) -> None:
    warning = (
        "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY"
        if allow_structural_draft
        else "AESTHETIC REVIEW ONLY — STRUCTURAL CLAIMS ARE OUT OF SCOPE"
    )
    shard_hash = selection_shard_semantic_sha256(sweep, selection, shard)
    lines = [
        f"# {selection.id} — {selection.stage} shard {shard}/{selection.shard_count}",
        "",
        warning,
        "",
        f"Sweep: `{sweep.semantic_sha256}`  ",
        f"Selection: `{selection.semantic_sha256}`  ",
        f"Shard: `{shard_hash}`  ",
        "Every still remains `UNREVIEWED`.",
        "",
    ]
    if selection.stage == "motion-samples":
        lines.extend(
            (
                "These are static route samples only. They do not clear shimmer, "
                "popping, animation, or camera-motion review.",
                "",
            )
        )
    figures = []
    for recipe in selection.recipes_for_shard(shard):
        lines.extend((f"## {recipe.id} — {recipe.label}", ""))
        for capture in selection.captures:
            relative = f"profiles/{recipe.id}/{capture.filename}"
            lines.extend(
                (
                    f"- `UNREVIEWED` [{capture.id}]({relative})",
                    "  - Notes:",
                )
            )
            figures.append(
                '<figure data-recipe="{recipe}" data-base-look="{base}" '
                'data-material="{material}" data-fog="{fog}" '
                'data-crystal="{crystal}" data-edge="{edge}" '
                'data-capture="{capture}"><img loading="lazy" src="{path}" '
                'alt="{recipe} {capture}"><figcaption>{recipe}<br>{capture}'
                '<br>{overrides}</figcaption></figure>'.format(
                    recipe=html.escape(recipe.id),
                    base=html.escape(recipe.base_look.id),
                    material=html.escape(recipe.overrides["material_treatment"]),
                    fog=html.escape(recipe.overrides["fog_mode"]),
                    crystal=html.escape(recipe.overrides["crystal_light_profile"]),
                    edge=html.escape(recipe.overrides["edge_treatment"]),
                    capture=html.escape(capture.id),
                    path=html.escape(relative),
                    overrides=html.escape(
                        ", ".join(
                            f"{field}={value}"
                            for field, value in recipe.overrides.items()
                        )
                    ),
                )
            )
        lines.append("")
    atomic_write(pack_root / "review-index.md", "\n".join(lines))
    document = """<!doctype html>
<meta charset="utf-8">
<title>Adaptive aesthetic selection</title>
<style>
body {{ font: 15px system-ui; margin: 24px; background: #171b22; color: #edf2f7; }}
.warning {{ padding: 12px; background: #6b2b2b; font-weight: 700; }}
.grid {{ display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }}
figure {{ margin: 0; padding: 7px; background: #252b35; }}
img {{ display: block; width: 100%; height: auto; }}
figcaption {{ margin-top: 6px; overflow-wrap: anywhere; }}
</style>
<h1>{selection} — {stage}</h1>
<p class="warning">{warning}</p>
<p>Sweep <code>{sweep}</code>; selection <code>{selection_hash}</code>; shard <code>{shard_hash}</code>.</p>
<div class="grid">{figures}</div>
""".format(
        selection=html.escape(selection.id),
        stage=html.escape(selection.stage),
        warning=html.escape(warning),
        sweep=html.escape(sweep.semantic_sha256),
        selection_hash=html.escape(selection.semantic_sha256),
        shard_hash=html.escape(shard_hash),
        figures="\n".join(figures),
    )
    atomic_write(pack_root / "index.html", document)


def validate_selection_pack(
    pack_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
    *,
    expected_provenance: Mapping[str, Any],
    expected_source_hashes: Mapping[str, str],
    expected_asset_tree_sha256: Optional[str] = None,
) -> None:
    if pack_root.is_symlink() or not pack_root.is_dir():
        raise ExperimentError(f"selection pack is not a regular directory: {pack_root}")
    manifest = _read_json(pack_root / "manifest.json", context="selection manifest")
    recipes = selection.recipes_for_shard(shard)
    expected_recipe_ids = [recipe.id for recipe in recipes]
    if (
        manifest.get("schema_version") != 1
        or manifest.get("kind") != "interaction-sweep-selection-shard"
    ):
        raise ExperimentError("selection manifest has an unsupported schema or kind")
    if manifest.get("review_status") != "UNREVIEWED":
        raise ExperimentError("selection manifest review status must be UNREVIEWED")
    if manifest.get("provenance") != dict(expected_provenance):
        raise ExperimentError("selection manifest provenance differs")
    if manifest.get("source_hashes") != dict(expected_source_hashes):
        raise ExperimentError("selection manifest source hashes differ")
    if manifest.get("scenario") != registry.scenario or manifest.get("seed") != registry.seed:
        raise ExperimentError("selection manifest scenario or seed differs")
    if manifest.get("sweep") != {
        "id": sweep.id,
        "semantic_sha256": sweep.semantic_sha256,
    }:
        raise ExperimentError("selection manifest sweep identity differs")
    expected_selection_record = {
        "id": selection.id,
        "stage": selection.stage,
        "source": f"selection/{selection.path.name}",
        "semantic_sha256": selection.semantic_sha256,
        "recipe_count": len(selection.recipes),
        "shard": shard,
        "shard_count": selection.shard_count,
        "shard_recipe_count": len(recipes),
        "shard_semantic_sha256": selection_shard_semantic_sha256(
            sweep, selection, shard
        ),
        "camera_manifest": (
            f"camera-manifest/{selection.camera_manifest_path.name}"
            if selection.camera_manifest_path is not None
            else None
        ),
    }
    if manifest.get("selection") != expected_selection_record:
        raise ExperimentError("selection manifest identity differs")
    expected_capture_ids = [capture.id for capture in selection.captures]
    if manifest.get("capture_ids") != expected_capture_ids:
        raise ExperimentError("selection manifest capture ids differ")
    if manifest.get("recipe_ids") != expected_recipe_ids:
        raise ExperimentError("selection manifest recipe ids differ")
    if expected_asset_tree_sha256 is not None and manifest.get(
        "source_asset_tree_sha256"
    ) != expected_asset_tree_sha256:
        raise ExperimentError("selection manifest source asset tree differs")
    profile_records = manifest.get("profiles")
    if not isinstance(profile_records, list) or [
        record.get("id") for record in profile_records if isinstance(record, dict)
    ] != expected_recipe_ids:
        raise ExperimentError("selection manifest profile records differ")
    expected_files = {
        "manifest.json",
        "review-index.md",
        "index.html",
        "logs/build.log",
    }
    record_by_id = {record["id"]: record for record in profile_records}
    recipe_by_id = {recipe.id: recipe for recipe in recipes}
    for recipe_id in expected_recipe_ids:
        recipe = recipe_by_id[recipe_id]
        record = record_by_id[recipe_id]
        if record.get("recipe_semantic_sha256") != recipe.semantic_sha256:
            raise ExperimentError(f"selection recipe {recipe_id} identity differs")
        if record.get("selection", {}).get("semantic_sha256") != selection.semantic_sha256:
            raise ExperimentError(f"selection profile {recipe_id} source differs")
        expected_files.add(f"profiles/{recipe_id}/profile.json")
        captures = record.get("captures")
        if not isinstance(captures, list) or [
            capture.get("id") for capture in captures if isinstance(capture, dict)
        ] != expected_capture_ids:
            raise ExperimentError(f"selection profile {recipe_id} captures differ")
        capture_by_id = {capture.id: capture for capture in selection.captures}
        for capture_record in captures:
            capture = capture_by_id[capture_record["id"]]
            image_relative = f"profiles/{recipe_id}/{capture.filename}"
            sidecar_name = pathlib.Path(capture.filename).with_suffix(
                ".manifest.json"
            ).name
            sidecar_relative = f"profiles/{recipe_id}/{sidecar_name}"
            log_relative = f"profiles/{recipe_id}/logs/{capture.id}.log"
            expected_files.update((image_relative, sidecar_relative, log_relative))
            for retry_log in capture_record.get("retry_logs", []):
                expected_files.add(str(retry_log))
            image_path = pack_root / image_relative
            inspect_png(image_path)
            image_hash = sha256_file(image_path)
            if capture_record.get("sha256") != image_hash:
                raise ExperimentError(f"selection capture hash differs: {image_relative}")
            sidecar = _read_json(pack_root / sidecar_relative, context="selection sidecar")
            if sidecar.get("profile", {}).get("id") != recipe_id:
                raise ExperimentError(f"selection sidecar recipe differs: {sidecar_relative}")
            if sidecar.get("profile", {}).get("recipe_semantic_sha256") != recipe.semantic_sha256:
                raise ExperimentError(f"selection sidecar identity differs: {sidecar_relative}")
            if sidecar.get("provenance") != {
                **dict(expected_provenance),
                "source_hashes": dict(expected_source_hashes),
            }:
                raise ExperimentError(f"selection sidecar provenance differs: {sidecar_relative}")
            if not isinstance(sidecar.get("capture_started_at_utc"), str) or not isinstance(
                sidecar.get("capture_completed_at_utc"), str
            ):
                raise ExperimentError(f"selection sidecar timestamps missing: {sidecar_relative}")
            if sidecar.get("artifact", {}).get("sha256") != image_hash:
                raise ExperimentError(f"selection artifact hash differs: {sidecar_relative}")
    actual_files = {
        path.relative_to(pack_root).as_posix() for path in _tree_files(pack_root)
    }
    if actual_files != expected_files:
        raise ExperimentError(
            "selection pack file set differs; missing="
            f"{sorted(expected_files - actual_files)}, extra={sorted(actual_files - expected_files)}"
        )
    review_index = (pack_root / "review-index.md").read_text(encoding="utf-8")
    html_index = (pack_root / "index.html").read_text(encoding="utf-8")
    for identity in (
        sweep.semantic_sha256,
        selection.semantic_sha256,
        selection_shard_semantic_sha256(sweep, selection, shard),
    ):
        if identity not in review_index or identity not in html_index:
            raise ExperimentError("selection indexes omit their semantic identity")


def run_selection_shard(
    *,
    repository_root: pathlib.Path,
    registry: Registry,
    sweep: SweepSpec,
    selection: SweepSelection,
    shard: int,
    provenance: Mapping[str, Any],
    source_hashes: Mapping[str, str],
    output_root: pathlib.Path,
    resource_limits: ResourceLimits,
    allow_structural_draft: bool,
) -> Tuple[pathlib.Path, bool]:
    """Capture one adaptive selection shard atomically or validate its resume."""

    require_capturable_sweep(sweep)
    recipes = selection.recipes_for_shard(shard)
    output = selection_shard_output(output_root, sweep, selection, shard)
    asset_source_digest = tree_digest(repository_root / "assets")
    if output.exists():
        validate_selection_pack(
            output,
            registry,
            sweep,
            selection,
            shard,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
        )
        return output, True
    output.parent.mkdir(parents=True, exist_ok=True)
    work = pathlib.Path(
        tempfile.mkdtemp(prefix=f".{output.name}.staging-", dir=output.parent)
    )
    pack_root = work / "pack"
    runtime_root = work / "runtime"
    pack_root.mkdir()
    atomic_write(
        pack_root / "review-index.md",
        "# Adaptive selection shard\n\nINCOMPLETE — NOT REVIEWABLE\n",
    )
    deadline = time.monotonic() + resource_limits.total_timeout_seconds
    started_at = _utc_timestamp()
    profile_records: List[Dict[str, Any]] = []
    try:
        _enforce_resource_limits(work, output.parent, resource_limits, deadline)
        build_log = pack_root / "logs" / "build.log"
        build_log.parent.mkdir(parents=True, exist_ok=True)
        review_binary = build_review_binary(
            repository_root,
            log_path=build_log,
            timeout_seconds=min(
                resource_limits.capture_timeout_seconds,
                max(1, int(deadline - time.monotonic())),
            ),
        )
        binary_record = {
            "mode": "build-once",
            "sha256": review_binary.sha256,
            "build_command": list(review_binary.command),
            "run_command": ["$REVIEW_BINARY"],
            "structural_draft": allow_structural_draft,
        }
        for recipe in recipes:
            _enforce_resource_limits(work, output.parent, resource_limits, deadline)
            profile = selection_profile(recipe)
            staged_root = runtime_root / recipe.id / "asset-root"
            staged_root.parent.mkdir(parents=True, exist_ok=True)
            copy_asset_tree(repository_root / "assets", staged_root / "assets")
            state = apply_selection_recipe(
                repository_root,
                staged_root,
                registry,
                sweep,
                selection,
                recipe,
            )
            staged_digest_before = state["staged_asset_tree_sha256"]
            make_tree_read_only(staged_root)
            capture_records = []
            for capture in selection.captures:
                _enforce_resource_limits(work, output.parent, resource_limits, deadline)
                remaining = max(1, int(deadline - time.monotonic()))
                record = _run_sweep_capture_with_retry(
                    repository_root=repository_root,
                    staged_root=staged_root,
                    pack_root=pack_root,
                    runtime_root=runtime_root,
                    registry=registry,
                    profile=profile,
                    capture=capture,
                    timeout_seconds=min(
                        resource_limits.capture_timeout_seconds, remaining
                    ),
                    common_provenance={
                        **dict(provenance),
                        "source_hashes": dict(source_hashes),
                    },
                    profile_state={"id": recipe.id, **state},
                    review_binary=review_binary,
                    allow_structural_draft=allow_structural_draft,
                )
                capture_records.append(record)
            if tree_digest(staged_root / "assets") != staged_digest_before:
                raise ExperimentError(f"runtime mutated staged assets for {recipe.id}")
            profile_record = {
                "id": recipe.id,
                **state,
                "captures": capture_records,
            }
            atomic_write(
                pack_root / "profiles" / recipe.id / "profile.json",
                canonical_json({"schema_version": 1, **profile_record}),
            )
            profile_records.append(profile_record)
            _remove_tree(staged_root.parent)
        if sha256_file(review_binary.path) != review_binary.sha256:
            raise CaptureError("review binary changed during the selection shard")
        if tree_digest(repository_root / "assets") != asset_source_digest:
            raise ExperimentError("tracked assets changed during the selection shard")
        if selection_source_hashes(
            repository_root, registry, sweep, selection
        ) != dict(source_hashes):
            raise ExperimentError("selection sources changed during the shard")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("worktree changed during the selection shard")
        manifest = {
            "schema_version": 1,
            "kind": "interaction-sweep-selection-shard",
            "review_status": "UNREVIEWED",
            "motion_status": "HUMAN-MOTION-PENDING",
            "structural_draft": allow_structural_draft,
            "started_at_utc": started_at,
            "completed_at_utc": _utc_timestamp(),
            "provenance": dict(provenance),
            "scenario": registry.scenario,
            "seed": registry.seed,
            "source_asset_tree_sha256": asset_source_digest,
            "source_hashes": dict(source_hashes),
            "sweep": {
                "id": sweep.id,
                "semantic_sha256": sweep.semantic_sha256,
            },
            "selection": {
                "id": selection.id,
                "stage": selection.stage,
                "source": f"selection/{selection.path.name}",
                "semantic_sha256": selection.semantic_sha256,
                "recipe_count": len(selection.recipes),
                "shard": shard,
                "shard_count": selection.shard_count,
                "shard_recipe_count": len(recipes),
                "shard_semantic_sha256": selection_shard_semantic_sha256(
                    sweep, selection, shard
                ),
                "camera_manifest": (
                    f"camera-manifest/{selection.camera_manifest_path.name}"
                    if selection.camera_manifest_path is not None
                    else None
                ),
            },
            "capture_ids": [capture.id for capture in selection.captures],
            "recipe_ids": [recipe.id for recipe in recipes],
            "render_count": len(recipes) * len(selection.captures),
            "review_binary": binary_record,
            "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
            "profiles": profile_records,
        }
        atomic_write(pack_root / "manifest.json", canonical_json(manifest))
        _write_selection_indexes(
            pack_root,
            sweep,
            selection,
            shard,
            allow_structural_draft=allow_structural_draft,
        )
        validate_selection_pack(
            pack_root,
            registry,
            sweep,
            selection,
            shard,
            expected_provenance=provenance,
            expected_source_hashes=source_hashes,
            expected_asset_tree_sha256=asset_source_digest,
        )
        _enforce_resource_limits(work, output.parent, resource_limits, deadline)
        if selection_source_hashes(
            repository_root, registry, sweep, selection
        ) != dict(source_hashes):
            raise CaptureError("selection sources changed before publication")
        if workspace_provenance(repository_root) != dict(provenance):
            raise CaptureError("worktree changed before selection publication")
        atomic_publish_no_replace(pack_root, output)
        return output, False
    finally:
        _remove_tree(work)


def build_sweep_scorecard(manifest_paths: Sequence[pathlib.Path]) -> Dict[str, Any]:
    """Build a blind human-score worksheet after mechanical PNG validation."""

    if not manifest_paths:
        raise ExperimentError("score-sweep needs at least one manifest")
    manifests = []
    sweep_identity: Optional[Tuple[str, str]] = None
    rows: List[Dict[str, Any]] = []
    seen_looks = set()
    artifact_hashes: Dict[str, List[str]] = {}
    for path in manifest_paths:
        resolved = path.resolve()
        if path.is_symlink() or not resolved.is_file():
            raise ExperimentError(f"score manifest is not a regular file: {path}")
        manifest = _read_json(resolved, context="sweep score manifest")
        if manifest.get("kind") not in (
            "interaction-sweep-shard",
            "interaction-sweep-selection-shard",
        ):
            raise ExperimentError(f"score manifest is not a sweep shard: {path}")
        sweep_record = manifest.get("sweep")
        if not isinstance(sweep_record, dict):
            raise ExperimentError(f"score manifest has no sweep identity: {path}")
        identity = (
            str(sweep_record.get("id")),
            str(sweep_record.get("semantic_sha256")),
        )
        if sweep_identity is None:
            sweep_identity = identity
        elif identity != sweep_identity:
            raise ExperimentError("score manifests belong to different sweep definitions")
        manifest_reference = resolved.as_posix()
        selection_record = manifest.get("selection")
        tier_label = sweep_record.get("tier")
        shard_index = sweep_record.get("shard")
        shard_identity = sweep_record.get("shard_semantic_sha256")
        if isinstance(selection_record, dict):
            tier_label = (
                f"{selection_record.get('stage')}:{selection_record.get('id')}"
            )
            shard_index = selection_record.get("shard")
            shard_identity = selection_record.get("shard_semantic_sha256")
        manifests.append(
            {
                "path": manifest_reference,
                "tier": tier_label,
                "shard": shard_index,
                "shard_semantic_sha256": shard_identity,
            }
        )
        profiles = manifest.get("profiles")
        if not isinstance(profiles, list):
            raise ExperimentError(f"score manifest profiles are invalid: {path}")
        for profile in profiles:
            if not isinstance(profile, dict):
                raise ExperimentError(f"score manifest has an invalid profile: {path}")
            look_id = profile.get("id")
            if not isinstance(look_id, str) or look_id in seen_looks:
                raise ExperimentError(f"score manifests repeat or omit look id: {look_id!r}")
            seen_looks.add(look_id)
            captures = profile.get("captures")
            if not isinstance(captures, list) or not captures:
                raise ExperimentError(f"score profile {look_id} has no captures")
            artifacts = []
            technical_failures = []
            for capture in captures:
                if not isinstance(capture, dict) or not isinstance(capture.get("path"), str):
                    raise ExperimentError(f"score profile {look_id} has an invalid capture")
                image = resolved.parent / capture["path"]
                try:
                    width, height = inspect_png(image)
                except ExperimentError as error:
                    technical_failures.append(str(error))
                    width = height = None
                artifact_hash = sha256_file(image) if image.is_file() else None
                if artifact_hash is not None and artifact_hash != capture.get("sha256"):
                    technical_failures.append(
                        f"capture hash differs from manifest: {capture['path']}"
                    )
                if artifact_hash is not None:
                    artifact_hashes.setdefault(artifact_hash, []).append(look_id)
                artifacts.append(
                    {
                        "capture_id": capture.get("id"),
                        "path": image.resolve().as_posix(),
                        "sha256": artifact_hash,
                        "width": width,
                        "height": height,
                    }
                )
            blind_id = "blind-" + sha256_bytes(
                f"{identity[1]}:{look_id}".encode("utf-8")
            )[:12]
            rows.append(
                {
                    "blind_id": blind_id,
                    "look_id": look_id,
                    "look_semantic_sha256": profile.get(
                        "recipe_semantic_sha256",
                        profile.get("look_semantic_sha256"),
                    ),
                    "tier": tier_label,
                    "shard": shard_index,
                    "technical_gate": "PASS" if not technical_failures else "FAIL",
                    "technical_failures": technical_failures,
                    "axes": profile.get("resolved_axes"),
                    "artifacts": artifacts,
                    "reviewer_1": {field: None for field in SWEEP_SCORE_FIELDS},
                    "reviewer_2": {field: None for field in SWEEP_SCORE_FIELDS},
                    "reviewer_3_if_needed": {
                        field: None for field in SWEEP_SCORE_FIELDS
                    },
                    "must_fail_reason": None,
                    "notes": None,
                }
            )
    duplicate_groups = [
        {"sha256": digest, "look_ids": sorted(set(look_ids))}
        for digest, look_ids in sorted(artifact_hashes.items())
        if len(set(look_ids)) > 1
    ]
    rows.sort(key=lambda row: str(row["blind_id"]))
    assert sweep_identity is not None
    semantic_body = {
        "sweep_id": sweep_identity[0],
        "sweep_semantic_sha256": sweep_identity[1],
        "manifest_shards": [
            {
                "tier": item["tier"],
                "shard": item["shard"],
                "shard_semantic_sha256": item["shard_semantic_sha256"],
            }
            for item in manifests
        ],
        "look_ids": sorted(seen_looks),
    }
    return {
        "schema_version": 1,
        "kind": "interaction-sweep-scorecard",
        "generated_at_utc": _utc_timestamp(),
        "sweep_id": sweep_identity[0],
        "sweep_semantic_sha256": sweep_identity[1],
        "semantic_sha256": sha256_bytes(
            canonical_json(semantic_body).encode("utf-8")
        ),
        "manifests": manifests,
        "rubric": {
            "scale": "integer 1..5",
            "weights": dict(SWEEP_SCORE_WEIGHTS),
            "must_fail_conditions": [
                "missing geometry or invalid capture",
                "blank or clipped render",
                "unreadable foreground or tunnel",
                "lost water or path boundary",
                "severe edge shimmer",
            ],
            "adjudication": "third review when any dimension differs by more than one point",
        },
        "exact_duplicate_groups": duplicate_groups,
        "looks": rows,
    }


def _score_selection_entry(
    raw: Any,
    *,
    context: str,
    representative: bool,
) -> Dict[str, Any]:
    allowed = ("look_id", "scores", "notes", "role") if representative else (
        "look_id",
        "rank",
        "scores",
        "notes",
    )
    required = allowed
    entry = dict(
        _strict_object(raw, context=context, allowed=allowed, required=required)
    )
    look_id = entry["look_id"]
    if not isinstance(look_id, str) or not PROFILE_ID_RE.fullmatch(look_id):
        raise ExperimentError(f"{context}.look_id is invalid")
    if representative:
        if not isinstance(entry["role"], str) or not entry["role"].strip():
            raise ExperimentError(f"{context}.role must be non-empty")
    elif isinstance(entry["rank"], bool) or not isinstance(entry["rank"], int):
        raise ExperimentError(f"{context}.rank must be an integer")
    scores = _strict_object(
        entry["scores"],
        context=f"{context}.scores",
        allowed=SWEEP_SCORE_FIELDS,
        required=SWEEP_SCORE_FIELDS,
    )
    resolved_scores = {}
    for field in SWEEP_SCORE_FIELDS:
        score = _number(scores[field], context=f"{context}.scores.{field}")
        if not 1.0 <= score <= 5.0:
            raise ExperimentError(f"{context}.scores.{field} must be in 1..=5")
        resolved_scores[field] = score
    if not isinstance(entry["notes"], str) or not entry["notes"].strip():
        raise ExperimentError(f"{context}.notes must be non-empty")
    entry["scores"] = resolved_scores
    entry["weighted_score"] = sum(
        resolved_scores[field] * SWEEP_SCORE_WEIGHTS[field]
        for field in SWEEP_SCORE_FIELDS
    )
    return entry


def render_sweep_selection(selection_path: pathlib.Path) -> str:
    """Validate a 12+4 selection and render a provenance-linked Markdown report."""

    if selection_path.is_symlink() or not selection_path.is_file():
        raise ExperimentError(f"selection is not a regular file: {selection_path}")
    raw = _strict_object(
        _read_json(selection_path, context="sweep selection"),
        context="sweep selection",
        allowed=(
            "version",
            "title",
            "scorecard",
            "winners",
            "representatives",
        ),
        required=(
            "version",
            "title",
            "scorecard",
            "winners",
            "representatives",
        ),
    )
    if raw["version"] != 1:
        raise ExperimentError("sweep selection version must be 1")
    if not isinstance(raw["title"], str) or not raw["title"].strip():
        raise ExperimentError("sweep selection title must be non-empty")
    scorecard_relative = _safe_relative_path(
        raw["scorecard"], context="sweep selection scorecard"
    )
    scorecard_candidate = selection_path.parent / scorecard_relative
    if scorecard_candidate.is_symlink():
        raise ExperimentError("sweep selection scorecard may not be a symlink")
    scorecard_path = scorecard_candidate.resolve()
    if not scorecard_path.is_file():
        raise ExperimentError("sweep selection scorecard is not a regular file")
    scorecard = _read_json(scorecard_path, context="sweep scorecard")
    if scorecard.get("kind") != "interaction-sweep-scorecard":
        raise ExperimentError("sweep selection references an unsupported scorecard")
    winners_raw = raw["winners"]
    representatives_raw = raw["representatives"]
    if not isinstance(winners_raw, list) or len(winners_raw) != 12:
        raise ExperimentError("sweep selection must contain exactly 12 winners")
    if not isinstance(representatives_raw, list) or len(representatives_raw) != 4:
        raise ExperimentError("sweep selection must contain exactly 4 representatives")
    winners = [
        _score_selection_entry(
            item, context=f"winners[{index}]", representative=False
        )
        for index, item in enumerate(winners_raw)
    ]
    representatives = [
        _score_selection_entry(
            item, context=f"representatives[{index}]", representative=True
        )
        for index, item in enumerate(representatives_raw)
    ]
    if [entry["rank"] for entry in winners] != list(range(1, 13)):
        raise ExperimentError("winner ranks must be exactly 1 through 12")
    selected_ids = [entry["look_id"] for entry in winners + representatives]
    if len(selected_ids) != len(set(selected_ids)):
        raise ExperimentError("winner and representative look ids must be unique")
    score_rows = {
        row["look_id"]: row
        for row in scorecard.get("looks", [])
        if isinstance(row, dict) and isinstance(row.get("look_id"), str)
    }
    unknown = sorted(set(selected_ids) - set(score_rows))
    if unknown:
        raise ExperimentError(f"selection names looks absent from scorecard: {unknown}")
    lines = [
        f"# {raw['title']}",
        "",
        "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY",
        "",
        f"Sweep: `{scorecard.get('sweep_id')}` / `{scorecard.get('sweep_semantic_sha256')}`  ",
        f"Scorecard: `{scorecard.get('semantic_sha256')}`  ",
        "",
        "## Ranked winners",
        "",
        "| Rank | Look | Weighted score | Recipe | Notes |",
        "|---:|---|---:|---|---|",
    ]
    for entry in winners:
        axes = score_rows[entry["look_id"]].get("axes") or {}
        recipe = ", ".join(
            f"{axis}={values.get('value_id', values.get('treatment', '?'))}"
            for axis, values in axes.items()
            if isinstance(values, dict)
        )
        lines.append(
            f"| {entry['rank']} | `{entry['look_id']}` | "
            f"{entry['weighted_score']:.3f} | {recipe} | {entry['notes']} |"
        )
    lines.extend(
        (
            "",
            "## Representative alternatives",
            "",
            "| Role | Look | Weighted score | Notes |",
            "|---|---|---:|---|",
        )
    )
    for entry in representatives:
        lines.append(
            f"| {entry['role']} | `{entry['look_id']}` | "
            f"{entry['weighted_score']:.3f} | {entry['notes']} |"
        )
    lines.extend(
        (
            "",
            "## Evidence boundary",
            "",
            "These stills compare presentation only. They do not approve map topology, "
            "traversal, collision, picking, persistence, or motion quality.",
            "",
        )
    )
    return "\n".join(lines)


def write_new_text_output(path: pathlib.Path, content: str) -> None:
    if path.exists() or path.is_symlink():
        raise ExperimentError(f"output already exists and will not be replaced: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    atomic_write(path, content)


def _add_resource_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--timeout-seconds",
        type=int,
        default=1800,
        help="per-capture process timeout (default: 1800)",
    )
    parser.add_argument(
        "--total-timeout-seconds",
        type=int,
        default=DEFAULT_TOTAL_TIMEOUT_SECONDS,
        help="whole build-and-capture deadline (default: 28800; max: 43200)",
    )
    parser.add_argument(
        "--max-work-gib",
        type=float,
        default=DEFAULT_MAX_WORK_BYTES / (1024**3),
        help="maximum unpublished work-tree size (default: 8; max: 32)",
    )
    parser.add_argument(
        "--min-free-gib",
        type=float,
        default=DEFAULT_MIN_FREE_BYTES / (1024**3),
        help="minimum free disk retained throughout the run (default: 20)",
    )


def _resource_limits_from_arguments(arguments: argparse.Namespace) -> ResourceLimits:
    if arguments.timeout_seconds <= 0:
        raise ExperimentError("timeout-seconds must be positive")
    if not 0 < arguments.total_timeout_seconds <= MAX_TOTAL_TIMEOUT_SECONDS:
        raise ExperimentError(
            f"total-timeout-seconds must be in 1..={MAX_TOTAL_TIMEOUT_SECONDS}"
        )
    max_work_bytes = int(
        _number(arguments.max_work_gib, context="max-work-gib") * 1024**3
    )
    min_free_bytes = int(
        _number(arguments.min_free_gib, context="min-free-gib") * 1024**3
    )
    if not 0 < max_work_bytes <= MAX_MAX_WORK_BYTES:
        raise ExperimentError("max-work-gib must be in (0, 32]")
    if min_free_bytes < 0:
        raise ExperimentError("min-free-gib must be non-negative")
    return ResourceLimits(
        arguments.timeout_seconds,
        arguments.total_timeout_seconds,
        max_work_bytes,
        min_free_bytes,
    )


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
    validate_sweep = subparsers.add_parser(
        "validate-sweep",
        help="validate the separate deterministic interaction-sweep specification",
    )
    validate_sweep.add_argument(
        "--spec",
        type=pathlib.Path,
        default=DEFAULT_SWEEP_SPEC,
        help="strict interaction-sweep specification",
    )
    validate_selection = subparsers.add_parser(
        "validate-selection",
        help="validate an adaptive selection, overrides, and optional camera manifest",
    )
    validate_selection.add_argument(
        "--spec",
        type=pathlib.Path,
        default=DEFAULT_SWEEP_SPEC,
        help="strict base interaction-sweep specification",
    )
    validate_selection.add_argument(
        "--selection",
        type=pathlib.Path,
        required=True,
        help="strict adaptive selection JSON",
    )
    run = subparsers.add_parser("run", help="capture and atomically publish a matrix")
    run.add_argument(
        "--profile",
        action="append",
        default=[],
        help=(
            "capture one profile; repeat for a subset (default: all twenty-four; "
            "candidate subsets automatically include baseline)"
        ),
    )
    run.add_argument(
        "--profile-set",
        choices=tuple(PROFILE_SETS),
        help=(
            "use a canonical bounded profile selection; initial covers outdoor light, "
            "atmosphere, visibility, materials, voxel height, and palette"
        ),
    )
    run.add_argument(
        "--capture-set",
        choices=CAPTURE_SET_IDS,
        default=DEFAULT_CAPTURE_SET,
        help="strict capture set (default: screen; smoke is the bounded preflight)",
    )
    run.add_argument(
        "--output",
        help="new output directory below .context/grand-v3-visual-experiments",
    )
    _add_resource_arguments(run)
    run.add_argument(
        "--dry-run",
        action="store_true",
        help="print the canonical plan without staging, output, or Cargo",
    )
    run.add_argument(
        "--allow-structural-draft",
        action="store_true",
        help=(
            "explicitly enable Grand V3's fail-open structural-review draft; "
            "outputs are diagnostic and cannot be approval evidence"
        ),
    )
    sweep_run = subparsers.add_parser(
        "run-sweep",
        help="capture or safely resume one atomic interaction-sweep shard",
    )
    sweep_run.add_argument(
        "--spec",
        type=pathlib.Path,
        default=DEFAULT_SWEEP_SPEC,
        help="strict interaction-sweep specification",
    )
    sweep_run.add_argument("--tier", choices=SWEEP_TIER_IDS, required=True)
    sweep_run.add_argument(
        "--shard",
        type=int,
        required=True,
        help="one-based shard index within the selected tier",
    )
    sweep_run.add_argument(
        "--output-root",
        required=True,
        help="root that receives <sweep>/<tier>/shard-NN",
    )
    _add_resource_arguments(sweep_run)
    sweep_run.add_argument(
        "--dry-run",
        action="store_true",
        help="print the exact shard plan without staging, output, or Cargo",
    )
    sweep_run.add_argument(
        "--allow-structural-draft",
        action="store_true",
        help=(
            "explicitly enable Grand V3's fail-open structural-review draft; "
            "outputs are diagnostic and cannot be approval evidence"
        ),
    )
    selection_run = subparsers.add_parser(
        "run-selection",
        help="capture or safely resume one adaptive selection shard",
    )
    selection_run.add_argument(
        "--spec",
        type=pathlib.Path,
        default=DEFAULT_SWEEP_SPEC,
        help="strict base interaction-sweep specification",
    )
    selection_run.add_argument(
        "--selection",
        type=pathlib.Path,
        required=True,
        help="strict adaptive selection JSON",
    )
    selection_run.add_argument(
        "--shard",
        type=int,
        required=True,
        help="one-based shard index from the selection",
    )
    selection_run.add_argument(
        "--output-root",
        required=True,
        help="root that receives <sweep>/selections/<selection>/shard-NN",
    )
    _add_resource_arguments(selection_run)
    selection_run.add_argument(
        "--dry-run",
        action="store_true",
        help="print the exact selection plan without staging, output, or Cargo",
    )
    selection_run.add_argument(
        "--allow-structural-draft",
        action="store_true",
        help=(
            "explicitly enable Grand V3's fail-open structural-review draft; "
            "outputs are diagnostic and cannot be approval evidence"
        ),
    )
    score = subparsers.add_parser(
        "score-sweep",
        help="validate shard images and create a blinded two-reviewer scorecard",
    )
    score.add_argument(
        "--manifest",
        type=pathlib.Path,
        action="append",
        required=True,
        help="published sweep shard manifest; repeat to aggregate shards",
    )
    score.add_argument(
        "--output",
        type=pathlib.Path,
        help="new JSON scorecard path (default: stdout)",
    )
    report = subparsers.add_parser(
        "report-sweep",
        help="validate and render a 12-winner plus 4-representative selection",
    )
    report.add_argument("--selection", type=pathlib.Path, required=True)
    report.add_argument(
        "--output",
        type=pathlib.Path,
        help="new Markdown report path (default: stdout)",
    )
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    parser = build_parser()
    arguments = parser.parse_args(argv)
    try:
        if arguments.command == "score-sweep":
            scorecard = build_sweep_scorecard(arguments.manifest)
            rendered = canonical_json(scorecard)
            if arguments.output is None:
                print(rendered, end="")
            else:
                write_new_text_output(arguments.output.resolve(), rendered)
                print(arguments.output.resolve())
            return 0
        if arguments.command == "report-sweep":
            rendered = render_sweep_selection(arguments.selection)
            if arguments.output is None:
                print(rendered, end="")
            else:
                write_new_text_output(arguments.output.resolve(), rendered)
                print(arguments.output.resolve())
            return 0
        if arguments.command in ("validate-selection", "run-selection"):
            (
                registry,
                sweep,
                selection,
                source_hashes,
                provenance,
            ) = load_stable_selection_inputs(
                arguments.registry,
                arguments.spec,
                arguments.selection,
                REPOSITORY_ROOT,
            )
            if arguments.command == "validate-selection":
                print(
                    canonical_json(
                        {
                            "schema_version": 1,
                            "status": "valid",
                            "sweep_id": sweep.id,
                            "sweep_semantic_sha256": sweep.semantic_sha256,
                            "selection_id": selection.id,
                            "selection_semantic_sha256": selection.semantic_sha256,
                            "stage": selection.stage,
                            "recipe_count": len(selection.recipes),
                            "capture_count": len(selection.captures),
                            "capture_ids": [
                                capture.id for capture in selection.captures
                            ],
                            "shard_count": selection.shard_count,
                            "recipes_per_shard": [
                                len(selection.recipes_for_shard(shard))
                                for shard in range(1, selection.shard_count + 1)
                            ],
                            "render_count": len(selection.recipes)
                            * len(selection.captures),
                            "source_hashes": source_hashes,
                        }
                    ),
                    end="",
                )
                return 0
            require_capturable_sweep(sweep)
            limits = _resource_limits_from_arguments(arguments)
            selection.recipes_for_shard(arguments.shard)
            output_root = resolve_sweep_output_root(arguments.output_root)
            if arguments.dry_run:
                print(
                    canonical_json(
                        build_selection_plan(
                            registry,
                            sweep,
                            selection,
                            arguments.shard,
                            provenance,
                            source_hashes,
                            output_root,
                            limits,
                            allow_structural_draft=arguments.allow_structural_draft,
                        )
                    ),
                    end="",
                )
                return 0
            output, resumed = run_selection_shard(
                repository_root=REPOSITORY_ROOT,
                registry=registry,
                sweep=sweep,
                selection=selection,
                shard=arguments.shard,
                provenance=provenance,
                source_hashes=source_hashes,
                output_root=output_root,
                resource_limits=limits,
                allow_structural_draft=arguments.allow_structural_draft,
            )
            print(
                canonical_json(
                    {
                        "output": output.as_posix(),
                        "status": "resumed" if resumed else "captured",
                    }
                ),
                end="",
            )
            return 0
        if arguments.command in ("validate-sweep", "run-sweep"):
            registry, sweep, source_hashes, provenance = load_stable_sweep_inputs(
                arguments.registry, arguments.spec, REPOSITORY_ROOT
            )
            if arguments.command == "validate-sweep":
                print(
                    canonical_json(
                        {
                            "schema_version": 1,
                            "status": "valid",
                            "sweep_id": sweep.id,
                            "sweep_status": sweep.status,
                            "semantic_sha256": sweep.semantic_sha256,
                            "canonical_profile_ids": [
                                profile.id for profile in registry.profiles
                            ],
                            "tiers": {
                                tier_id: {
                                    "look_count": len(sweep.tier(tier_id).looks),
                                    "shard_count": sweep.tier(tier_id).shard_count,
                                    "looks_per_shard": [
                                        len(
                                            sweep.tier(tier_id).looks_for_shard(
                                                shard
                                            )
                                        )
                                        for shard in range(
                                            1,
                                            sweep.tier(tier_id).shard_count + 1,
                                        )
                                    ],
                                    "capture_ids": list(
                                        sweep.tier(tier_id).capture_ids
                                    ),
                                }
                                for tier_id in SWEEP_TIER_IDS
                            },
                            "source_hashes": source_hashes,
                        }
                    ),
                    end="",
                )
                return 0
            require_capturable_sweep(sweep)
            limits = _resource_limits_from_arguments(arguments)
            tier = sweep.tier(arguments.tier)
            tier.looks_for_shard(arguments.shard)
            output_root = resolve_sweep_output_root(arguments.output_root)
            if arguments.dry_run:
                print(
                    canonical_json(
                        build_sweep_plan(
                            registry,
                            sweep,
                            tier,
                            arguments.shard,
                            provenance,
                            source_hashes,
                            output_root,
                            limits,
                            allow_structural_draft=arguments.allow_structural_draft,
                        )
                    ),
                    end="",
                )
                return 0
            output, resumed = run_sweep_shard(
                repository_root=REPOSITORY_ROOT,
                registry=registry,
                sweep=sweep,
                tier=tier,
                shard=arguments.shard,
                provenance=provenance,
                source_hashes=source_hashes,
                output_root=output_root,
                resource_limits=limits,
                allow_structural_draft=arguments.allow_structural_draft,
            )
            print(
                canonical_json(
                    {
                        "output": output.as_posix(),
                        "status": "resumed" if resumed else "captured",
                    }
                ),
                end="",
            )
            return 0
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
                        "profile_sets": {
                            name: list(profile_ids)
                            for name, profile_ids in PROFILE_SETS.items()
                        },
                        "captures": [capture.id for capture in registry.captures],
                        "capture_sets": {
                            name: list(registry.capture_sets[name])
                            for name in CAPTURE_SET_IDS
                        },
                        "runtime_report": dict(RUNTIME_REPORT_PLACEHOLDER),
                        "source_hashes": source_hashes,
                    }
                ),
                end="",
            )
            return 0
        limits = _resource_limits_from_arguments(arguments)
        profiles = selected_profiles(
            registry, arguments.profile, profile_set=arguments.profile_set
        )
        captures = registry.captures_for(arguments.capture_set)
        output = resolve_output(
            arguments.output,
            _default_output(provenance, registry, profiles, arguments.capture_set),
        )
        if arguments.dry_run:
            print(
                canonical_json(
                    build_plan(
                        registry,
                        profiles,
                        provenance,
                        output,
                        source_hashes,
                        captures,
                        arguments.capture_set,
                        limits,
                        arguments.allow_structural_draft,
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
            captures=captures,
            resource_limits=limits,
            build_once=True,
            capture_set=arguments.capture_set,
            allow_structural_draft=arguments.allow_structural_draft,
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
