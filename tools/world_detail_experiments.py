#!/usr/bin/env python3
"""Deterministic capture harness for the small-geometry aesthetic study.

The harness is deliberately presentation-only.  It stages the already-selected
height/palette/light baseline in private asset copies, supplies one canonical
``HEX_REVIEW_WORLD_DETAIL`` JSON profile, and drives genuine 1920x1080 renderer
captures through Cargo's release-shaped ``map-review`` feature.  It never edits a
source asset, invents a review score, or treats pixels as world authority. Promotions
and recommendations are derived only from two hash-pinned blinded-review files.

The adaptive stages are driven by an explicit selection file.  Until two blinded
reviews populate that file, ``plan`` emits only the neutral 60-profile screen and
records every later stage as unresolved.
"""

from __future__ import annotations

import argparse
import contextlib
import csv
import copy
import dataclasses
import fcntl
import functools
import hashlib
import io
import json
import math
import os
import pathlib
import posixpath
import re
import secrets
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import zipfile
import xml.etree.ElementTree as ElementTree
from collections import defaultdict
from typing import Any, Dict, Iterable, Iterator, List, Mapping, MutableMapping, Optional, Sequence, Tuple


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
TOOLS_ROOT = pathlib.Path(__file__).resolve().parent
SPEC_ROOT = TOOLS_ROOT / "world_detail_experiments" / "specs"
FINAL_CAMERAS_PATH = SPEC_ROOT / "final-17-cameras-v2.json"
FOCUSED_CAMERAS_PATH = SPEC_ROOT / "focused-4-cameras-v1.json"
CAMERA_PROVENANCE_PATH = SPEC_ROOT / "camera-provenance-v1.json"
PROFILE_HASHES_PATH = SPEC_ROOT / "profile-hashes-v1.json"
REVIEW_SCHEMA_PATH = SPEC_ROOT / "review-evidence-schema-v1.json"
RUNTIME_EVIDENCE_CONTRACT_PATH = SPEC_ROOT / "runtime-evidence-contract-v1.json"
BASELINE_ORACLE_CONTRACT_PATH = SPEC_ROOT / "baseline-oracle-contract-v1.json"
BASELINE_ORACLE_ROOT = (
    REPOSITORY_ROOT.parent / "world-detail-study-runtime" / "baseline-oracle-v1"
)
CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH = (
    SPEC_ROOT / "control-equivalence-raster-contract-v1.json"
)
CONTROL_EQUIVALENCE_QUALIFICATION_ROOT = (
    REPOSITORY_ROOT.parent
    / "world-detail-study-runtime"
    / "control-equivalence-qualification-v7c"
)
PRIOR_AESTHETIC_REPORT_ROOT = pathlib.Path(
    "/Users/alberto/Documents/Codex/2026-09-01/i-want-to-plan-the-work/outputs/"
    "plan-hex-aesthetic-review-2026-09-02"
)
PRIOR_AESTHETIC_REPORT_HASHES = {
    "README.md": "56bda5011b93f32d06e727acf78852cc15e90307a7dd94374175d18438f46479",
    "manifest.json": "e9c1ee265c60d88f7e9ecb0cc0ae619bec97b372a3eecde0f6ed23e5941b0b3d",
}
BASELINE_ORACLE_MANIFEST_FILENAME = "manifest.json"
CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME = "manifest.json"
BASELINE_ORACLE_CAMERA_IDS = (
    "02-highlands-oblique",
    "03-coast-river-outlet",
    "14-cascade-basin-full-height",
    "16-deep-tree-shade",
)
CONTROL_EQUIVALENCE_QUALIFIED_PIXELS = (
    {
        "camera_id": "14-cascade-basin-full-height",
        "x": 1438,
        "y": 273,
        "allowed_rgb": [[164, 95, 66], [169, 133, 125]],
    },
)
ESTABLISHED_CAMERA_SOURCE_SHA256 = {
    "tools/visual_experiments/profiles.json": "1ae6921ca2b87bf66fa43fa04c0d13e924466b221e6b39595957e946b264e65b",
    "walks/camera_grand_v3_baseline.ron": "d794ee9f057fb9e0ad0264b9015a726decdb7bd02e29dc8d65dd3f8a3a7bb759",
    "assets/config/camera.ron": "a4b6221d48c9511e8a5e29a64467c061a7a234fde1d30db128c75cc0c646fa66",
}
DEFAULT_OUTPUT_ROOT = (
    REPOSITORY_ROOT.parents[1]
    / "outputs"
    / "plan-hex-small-geometry-aesthetic-report-2026-09-02"
)
DEFAULT_WORK_ROOT = REPOSITORY_ROOT.parent / ".crystal-ascent-world-detail-experiments-runtime"

WARNING = "UNAPPROVABLE STRUCTURAL DRAFT — AESTHETIC REVIEW ONLY"
SCENARIO = "Grand V3 Baseline"
SEED = 1_592_598_566
CAPTURE_WIDTH = 1920
CAPTURE_HEIGHT = 1080
FPS = 30
ORBIT_DEGREES = 20.0
MOTION_FRAME_COUNT = 90
PROFILE_VERSION = 1
REPORT_VERSION = 1
CONTROL_PROFILE_SHA256 = "c962b67a10570c64e4515780f4c9704ad41099851c8f928e27886e4eb8a7db8b"
EXPECTED_LOGICAL_SLOTS = 665
MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS = 611
MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS = 630
EXPECTED_SHARED_CONTROL_RENDERS = 25
EXPECTED_CONTROL_VERIFICATION_RENDERS = 4
EXPECTED_REPRODUCTION_RENDERS = 1
EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS = 4
EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS = 22
STRUCTURAL_DRAFT_ENVIRONMENT = "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"
STRUCTURAL_DRAFT_VALUE = "1"
HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE = 70
HARNESS_PROCESS_EXCEPTION_RETURN_CODE = 71
UNSCORED_CAMERA_IDS = ("07-tunnel-first-person", "08-crystal-bottom-chamber")
DIAGNOSTIC_REVIEW_CONTEXT = "diagnostic-4"
FINAL_REVIEW_CONTEXT = "final-17"
MOTION_REVIEW_CONTEXT = "paired-motion"
REVIEW_CONTEXTS = (DIAGNOSTIC_REVIEW_CONTEXT, FINAL_REVIEW_CONTEXT)
FAMILY_ORDER = (
    "snow",
    "water",
    "physical_clouds",
    "shore_and_falls",
    "alpine_vegetation",
    "cliff_strata",
    "terrain_props",
    "ice_fringe",
    "local_fog",
)
FAMILY_COUNTS = {
    "snow": 9,
    "water": 7,
    "physical_clouds": 8,
    "shore_and_falls": 6,
    "alpine_vegetation": 6,
    "cliff_strata": 6,
    "terrain_props": 6,
    "ice_fringe": 6,
    "local_fog": 6,
}
PRIMARY_CAMERAS = {
    "snow": "02-highlands-oblique",
    "water": "03-coast-river-outlet",
    "physical_clouds": "02-highlands-oblique",
    "shore_and_falls": "14-cascade-basin-full-height",
    # Alpine vegetation planning intentionally accepts only existing tree roots
    # at level 104+, with an even narrower snow/high-alpine crown-dust mask.  The
    # river-bend view remains a valuable lowland no-op check in the diagnostic
    # four, but it cannot be this family's named-difference or motion camera.
    "alpine_vegetation": "02-highlands-oblique",
    "cliff_strata": "02-highlands-oblique",
    "terrain_props": "16-deep-tree-shade",
    "ice_fringe": "03-coast-river-outlet",
    "local_fog": "16-deep-tree-shade",
}
RISKY_MOTION_FAMILIES = (
    "water",
    "physical_clouds",
    "shore_and_falls",
    "ice_fringe",
    "local_fog",
)
WINNER_MOTION_FAMILIES = (
    "snow",
    "alpine_vegetation",
    "cliff_strata",
    "terrain_props",
)
COMBINATION_IDS = ("restrained", "expressive", "score-leader")
CATEGORY_ORDER = (
    "terrain_route_water_edge_readability",
    "shadow_occlusion_preservation",
    "biome_material_separation",
    "atmosphere_depth_integration",
    "edge_temporal_quietness",
    "mood_cohesion",
)
CATEGORY_WEIGHTS = {
    "terrain_route_water_edge_readability": 0.25,
    "shadow_occlusion_preservation": 0.20,
    "biome_material_separation": 0.15,
    "atmosphere_depth_integration": 0.15,
    "edge_temporal_quietness": 0.15,
    "mood_cohesion": 0.10,
}
SHA256_RE = re.compile(r"^[0-9a-f]{64}$")
SAFE_ID_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
BLIND_CODE_RE = re.compile(r"^RV-[0-9A-F]{12}$")
BLINDING_DOMAIN = "crystal-ascent-world-detail-review-v1"
LAUNCH_NONCE_RE = re.compile(r"^[0-9a-f]{64}$")
LADDER_STEPS = (
    ("ladder-snow", "control", ("snow",)),
    (
        "ladder-snow-vegetation-cliff",
        "ladder-snow",
        ("alpine_vegetation", "cliff_strata"),
    ),
    (
        "ladder-snow-vegetation-cliff-props",
        "ladder-snow-vegetation-cliff",
        ("terrain_props",),
    ),
    ("ladder-water", "control", ("water",)),
    ("ladder-water-shore", "ladder-water", ("shore_and_falls",)),
    ("ladder-water-shore-ice", "ladder-water-shore", ("ice_fringe",)),
    ("ladder-clouds", "control", ("physical_clouds",)),
    ("ladder-clouds-fog", "ladder-clouds", ("local_fog",)),
)


class HarnessError(RuntimeError):
    """A study definition, capture, or evidence artifact is invalid."""


def _as_f32(value: float) -> float:
    """Round a Python number exactly as the runtime's f32 capture contract does."""

    return struct.unpack("!f", struct.pack("!f", value))[0]


@dataclasses.dataclass(frozen=True)
class CameraSpec:
    """One concrete camera entry understood by the map-review runtime."""

    id: str
    filename: str
    camera: str
    view: str
    focus_anchor: Optional[str] = None
    look_at_anchor: Optional[str] = None
    look_at_offset: Optional[Tuple[float, float, float]] = None
    character_radius_scale: float = 1.0
    full_cutaway: bool = False
    illumination_overlay: bool = False

    def runtime_entry(self, path: pathlib.Path) -> Dict[str, Any]:
        """Return the strict runtime capture-plan object for this camera."""

        result: Dict[str, Any] = {
            "path": str(path.resolve()),
            "camera": self.camera,
            "view": self.view,
        }
        if self.focus_anchor is not None:
            result["focus_anchor"] = self.focus_anchor
        if self.look_at_anchor is not None:
            result["look_at_anchor"] = self.look_at_anchor
            result["look_at_offset"] = list(self.look_at_offset or ())
        if self.character_radius_scale != 1.0:
            result["character_radius_scale"] = self.character_radius_scale
        if self.full_cutaway:
            result["full_cutaway"] = True
        if self.illumination_overlay:
            result["illumination_overlay"] = True
        return result

    def expected_report_capture(
        self,
        path: pathlib.Path,
        *,
        time_hours: Optional[float],
        liquid_phase_seconds: float,
        settle_frames: int,
    ) -> Dict[str, Any]:
        """Return the exact sidecar capture provenance expected from runtime."""

        return {
            "path": str(path.resolve()),
            "camera": self.camera,
            "view": self.view,
            "focus_anchor": self.focus_anchor,
            "look_at_anchor": self.look_at_anchor,
            "look_at_offset": [_as_f32(component) for component in self.look_at_offset]
            if self.look_at_offset is not None
            else None,
            "character_radius_scale": _as_f32(self.character_radius_scale),
            "full_cutaway": self.full_cutaway,
            "illumination_overlay": self.illumination_overlay,
            "time_hours": _as_f32(time_hours) if time_hours is not None else None,
            "liquid_phase_seconds": _as_f32(liquid_phase_seconds),
            "settle_frames": settle_frames,
        }


@dataclasses.dataclass(frozen=True)
class DetailProfile:
    """One canonical compact world-detail profile and its stable identity."""

    id: str
    label: str
    family: str
    body: Mapping[str, Any]
    canonical_json: str
    sha256: str

    @property
    def is_control(self) -> bool:
        """Return whether all nine sections are current."""

        return all(self.body[family]["kind"] == "current" for family in FAMILY_ORDER)


@dataclasses.dataclass(frozen=True)
class LightingCondition:
    """One immutable baseline or stress lighting/asset stage."""

    id: str
    label: str
    time_hours: Optional[float]
    asset_stage: str
    haze_density: Optional[float]
    static_lighting: Optional[str] = None


LIGHTING_CONDITIONS = {
    "neutral": LightingCondition(
        "neutral",
        "Balanced noon, haze 0.0003",
        12.0,
        "balanced-noon-haze-0003",
        0.0003,
    ),
    "golden": LightingCondition(
        "golden",
        "Golden hour 16:30, haze 0.0003",
        16.5,
        "golden-1630-haze-0003",
        0.0003,
    ),
    "overcast": LightingCondition(
        "overcast",
        "Existing static noon overcast profile",
        None,
        "overcast-noon",
        None,
        "assets/config/lighting/overcast.ron",
    ),
}


def sha256_bytes(value: bytes) -> str:
    """Return lowercase SHA-256 for bytes."""

    return hashlib.sha256(value).hexdigest()


def sha256_file(path: pathlib.Path) -> str:
    """Return lowercase SHA-256 for one regular file."""

    digest = hashlib.sha256()
    try:
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError as error:
        raise HarnessError(f"cannot hash {path}: {error}") from error
    return digest.hexdigest()


def compact_json(value: Any) -> str:
    """Serialize with the field order supplied by the versioned contract."""

    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
        )
    except (TypeError, ValueError) as error:
        raise HarnessError(f"cannot canonicalize JSON: {error}") from error


def canonical_object_json(value: Any) -> str:
    """Serialize a generic evidence object independently of insertion order."""

    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            allow_nan=False,
            separators=(",", ":"),
            sort_keys=True,
        )
    except (TypeError, ValueError) as error:
        raise HarnessError(f"cannot canonicalize evidence object: {error}") from error


def sha256_object(value: Any) -> str:
    return sha256_bytes(canonical_object_json(value).encode("utf-8"))


def pretty_json(value: Any) -> str:
    """Serialize a human-auditable manifest deterministically."""

    try:
        return json.dumps(
            value,
            ensure_ascii=False,
            allow_nan=False,
            indent=2,
            sort_keys=True,
        ) + "\n"
    except (TypeError, ValueError) as error:
        raise HarnessError(f"cannot serialize JSON: {error}") from error


def atomic_write(path: pathlib.Path, content: str) -> None:
    """Atomically replace one narrow harness-owned text artifact."""

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(content, encoding="utf-8")
    os.replace(temporary, path)


def _read_json(path: pathlib.Path, context: str) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise HarnessError(f"cannot read {context} {path}: {error}") from error


def _strict_object(
    value: Any,
    *,
    context: str,
    required: Iterable[str],
    optional: Iterable[str] = (),
) -> Dict[str, Any]:
    if not isinstance(value, dict):
        raise HarnessError(f"{context} must be an object")
    required_set = set(required)
    allowed = required_set | set(optional)
    unknown = sorted(set(value) - allowed)
    missing = sorted(required_set - set(value))
    if unknown:
        raise HarnessError(f"{context} has unknown fields: {unknown}")
    if missing:
        raise HarnessError(f"{context} is missing fields: {missing}")
    return dict(value)


def _finite_number(value: Any, context: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise HarnessError(f"{context} must be numeric")
    result = float(value)
    if not math.isfinite(result):
        raise HarnessError(f"{context} must be finite")
    return result


def _require_outside_source_tree(path: pathlib.Path, context: str) -> pathlib.Path:
    resolved = path.resolve()
    try:
        resolved.relative_to(REPOSITORY_ROOT.resolve())
    except ValueError:
        return resolved
    raise HarnessError(f"{context} must be outside the immutable source worktree: {resolved}")


@contextlib.contextmanager
def _exclusive_capture_runner_lock(work_root: pathlib.Path) -> Iterator[None]:
    """Hold one nonblocking process lock for every mutation under ``work_root``."""

    work_root.mkdir(parents=True, exist_ok=True)
    lock_path = work_root / "capture-runner.lock"
    flags = os.O_CREAT | os.O_RDWR
    flags |= getattr(os, "O_CLOEXEC", 0)
    flags |= getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(lock_path, flags, 0o600)
    except OSError as error:
        raise HarnessError(f"cannot open capture runner lock {lock_path}: {error}") from error
    try:
        handle = os.fdopen(descriptor, "r+b", buffering=0)
    except BaseException:
        os.close(descriptor)
        raise
    locked = False
    try:
        try:
            fcntl.flock(handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError as error:
            handle.seek(0)
            owner = handle.read(4096).decode("utf-8", errors="replace").strip()
            suffix = f"; current owner: {owner}" if owner else ""
            raise HarnessError(
                f"capture work root is already owned by another runner: {work_root}{suffix}"
            ) from error
        locked = True
        mode = os.fstat(handle.fileno()).st_mode
        if not stat.S_ISREG(mode):
            raise HarnessError(f"capture runner lock is not a regular file: {lock_path}")
        owner_record = compact_json(
            {
                "version": 1,
                "process_id": os.getpid(),
                "work_root": str(work_root),
            }
        ).encode("utf-8")
        handle.seek(0)
        handle.truncate(0)
        handle.write(owner_record)
        handle.flush()
        os.fsync(handle.fileno())
        yield
    finally:
        try:
            if locked:
                fcntl.flock(handle.fileno(), fcntl.LOCK_UN)
        finally:
            handle.close()


def default_raw_capture_root(output_root: pathlib.Path) -> pathlib.Path:
    """Return a deterministic non-public root for unwatermarked metric pixels."""

    output_root = _require_outside_source_tree(output_root, "output root")
    identity = sha256_bytes(str(output_root).encode("utf-8"))[:20]
    return _require_outside_source_tree(
        DEFAULT_WORK_ROOT / "raw-captures" / identity,
        "raw capture root",
    )


def _validate_raw_capture_root(
    output_root: pathlib.Path,
    raw_capture_root: Optional[pathlib.Path] = None,
) -> pathlib.Path:
    output_root = _require_outside_source_tree(output_root, "output root")
    raw_root = _require_outside_source_tree(
        raw_capture_root or default_raw_capture_root(output_root),
        "raw capture root",
    )
    try:
        raw_root.relative_to(output_root)
    except ValueError:
        pass
    else:
        raise HarnessError("unwatermarked raw captures must stay outside the published output root")
    try:
        output_root.relative_to(raw_root)
    except ValueError:
        pass
    else:
        raise HarnessError("published output root must stay outside the raw capture root")
    return raw_root


def _raw_artifact_path(raw_capture_root: pathlib.Path, artifact: str) -> pathlib.Path:
    if not isinstance(artifact, str) or not artifact:
        raise HarnessError("raw artifact must be a non-empty relative path")
    relative = pathlib.Path(artifact)
    if relative.is_absolute() or ".." in relative.parts:
        raise HarnessError(f"raw artifact path is unsafe: {artifact!r}")
    root = _require_outside_source_tree(raw_capture_root, "raw capture root")
    resolved = (root / relative).resolve()
    try:
        resolved.relative_to(root)
    except ValueError as error:
        raise HarnessError(f"raw artifact escaped its capture root: {artifact!r}") from error
    return resolved


def _current_sections() -> Dict[str, Dict[str, str]]:
    return {family: {"kind": "current"} for family in FAMILY_ORDER}


def _make_profile(profile_id: str, label: str, family: str, section: Mapping[str, Any]) -> DetailProfile:
    if family not in FAMILY_ORDER:
        raise HarnessError(f"unknown profile family {family}")
    if not SAFE_ID_RE.fullmatch(profile_id):
        raise HarnessError(f"invalid profile id {profile_id!r}")
    body: Dict[str, Any] = {"version": PROFILE_VERSION}
    sections = _current_sections()
    sections[family] = dict(section)
    body.update(sections)
    encoded = compact_json(body)
    return DetailProfile(profile_id, label, family, body, encoded, sha256_bytes(encoded.encode()))


@functools.lru_cache(maxsize=1)
def control_profile() -> DetailProfile:
    """Return the sole all-current control profile."""

    body: Dict[str, Any] = {"version": PROFILE_VERSION}
    body.update(_current_sections())
    encoded = compact_json(body)
    digest = sha256_bytes(encoded.encode())
    if digest != CONTROL_PROFILE_SHA256:
        raise HarnessError("Python/Rust control canonical JSON contract diverged")
    return DetailProfile("control", "Current presentation", "control", body, encoded, digest)


def _matrix_rows() -> Mapping[str, Sequence[Tuple[str, str, Mapping[str, Any]]]]:
    """Return the exact approved 60-treatment matrix in family order."""

    return {
        "snow": (
            ("snow-01-straight-128", "Straight snow line 128", {"kind": "straight_threshold", "level": 128}),
            ("snow-02-straight-140", "Straight snow line 140", {"kind": "straight_threshold", "level": 140}),
            ("snow-03-straight-152", "Straight snow line 152", {"kind": "straight_threshold", "level": 152}),
            ("snow-04-coherent-136", "Coherent snow line 136 ±8", {"kind": "coherent_line", "mean_level": 136, "amplitude_levels": 8, "correlation_hexes": 22}),
            ("snow-05-coherent-144", "Coherent snow line 144 ±8", {"kind": "coherent_line", "mean_level": 144, "amplitude_levels": 8, "correlation_hexes": 22}),
            ("snow-06-terrain-aware", "Terrain-aware snow line", {"kind": "terrain_aware", "vertical_shell_height": 0.0}),
            ("snow-07-terrain-aware-shell-004", "Terrain-aware snow, shell 0.04", {"kind": "terrain_aware", "vertical_shell_height": 0.04}),
            ("snow-08-terrain-aware-shell-008", "Terrain-aware snow, shell 0.08", {"kind": "terrain_aware", "vertical_shell_height": 0.08}),
            ("snow-09-terrain-aware-shell-012", "Terrain-aware snow, shell 0.12", {"kind": "terrain_aware", "vertical_shell_height": 0.12}),
        ),
        "water": (
            ("water-01-alpha-085", "Water alpha 0.85", {"kind": "uniform_alpha", "alpha": 0.85}),
            ("water-02-alpha-070", "Water alpha 0.70", {"kind": "uniform_alpha", "alpha": 0.70}),
            ("water-03-alpha-055", "Water alpha 0.55", {"kind": "uniform_alpha", "alpha": 0.55}),
            ("water-04-depth-short", "Water absorption half-distance 0.70", {"kind": "depth_absorption", "alpha": 0.70, "depth_half_distance": 0.70, "deep_value_multiplier": 0.62}),
            ("water-05-depth-long", "Water absorption half-distance 1.40", {"kind": "depth_absorption", "alpha": 0.70, "depth_half_distance": 1.40, "deep_value_multiplier": 0.82}),
            ("water-06-transmission", "Water transmission and restrained refraction", {"kind": "transmission", "ior": 1.333, "thickness": 0.08, "max_refraction_uv": 0.015}),
            ("water-07-rough-no-refraction", "Rough water, no refraction", {"kind": "rough_surface", "alpha": 0.70, "roughness": 0.40, "reflectance": 0.50}),
        ),
        "physical_clouds": (
            ("clouds-01-faceted-clear", "Faceted clouds clear of peaks", {"kind": "faceted_layer", "altitude_band": "clear", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-02-faceted-grazing", "Faceted clouds grazing peaks", {"kind": "faceted_layer", "altitude_band": "grazing", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-03-faceted-crossing", "Faceted clouds crossing peaks", {"kind": "faceted_layer", "altitude_band": "crossing", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-04-rounded-grazing", "Rounded grazing clouds", {"kind": "grazing_shape", "shape": "rounded", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-05-lenticular-grazing", "Lenticular grazing clouds", {"kind": "grazing_shape", "shape": "lenticular", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-06-rounded-coverage-010", "Rounded grazing clouds, coverage 10%", {"kind": "rounded_coverage", "projected_coverage": 0.10, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-07-rounded-coverage-028", "Rounded grazing clouds, coverage 28%", {"kind": "rounded_coverage", "projected_coverage": 0.28, "diameter_min": 16.0, "diameter_max": 32.0}),
            ("clouds-08-rounded-shadow", "Rounded grazing clouds with restrained shadow", {"kind": "rounded_shadow", "projected_coverage": 0.18, "diameter_min": 16.0, "diameter_max": 32.0, "max_projected_shadow": 0.20, "shadow_blur": 24.0}),
        ),
        "shore_and_falls": (
            ("shore-01-wet-rim-narrow", "Wet rim 0.12", {"kind": "wet_rim", "width": 0.12, "value_delta": -0.12, "roughness_delta": -0.15}),
            ("shore-02-wet-rim-wide", "Wet rim 0.25", {"kind": "wet_rim", "width": 0.25, "value_delta": -0.18, "roughness_delta": -0.15}),
            ("shore-03-foam-narrow", "Foam 0.10 / 0.35", {"kind": "foam", "width": 0.10, "opacity": 0.35}),
            ("shore-04-foam-wide", "Foam 0.20 / 0.55", {"kind": "foam", "width": 0.20, "opacity": 0.55}),
            ("shore-05-plunge-spray", "Plunge spray and pool foam", {"kind": "plunge_spray", "radius_hexes": 3, "height": 4.2, "opacity": 0.08, "pool_foam_radius_hexes": 2}),
            ("shore-06-restrained-combination", "Restrained wet rim, foam, and spray", {"kind": "restrained_combination", "wet_rim_width": 0.12, "wet_rim_value_delta": -0.12, "wet_rim_roughness_delta": -0.15, "foam_width": 0.10, "foam_opacity": 0.35, "spray_radius_hexes": 3, "spray_height": 4.2, "spray_opacity": 0.06, "pool_foam_radius_hexes": 2}),
        ),
        "alpine_vegetation": (
            ("vegetation-01-scale-light", "Vegetation scale jitter, light", {"kind": "scale_jitter", "horizontal_min": 0.90, "horizontal_max": 1.10, "vertical_min": 0.95, "vertical_max": 1.05}),
            ("vegetation-02-scale-wide", "Vegetation scale jitter, wide", {"kind": "scale_jitter", "horizontal_min": 0.80, "horizontal_max": 1.20, "vertical_min": 0.90, "vertical_max": 1.10}),
            ("vegetation-03-dust-light", "Crown snow dust, light", {"kind": "crown_snow_dust", "upper_fraction": 0.25, "shell_height": 0.02}),
            ("vegetation-04-dust-heavy", "Crown snow dust, heavy", {"kind": "crown_snow_dust", "upper_fraction": 0.50, "shell_height": 0.04}),
            ("vegetation-05-scale-light-dust-light", "Light scale jitter and dust", {"kind": "scale_jitter_with_dust", "horizontal_min": 0.90, "horizontal_max": 1.10, "vertical_min": 0.95, "vertical_max": 1.05, "upper_fraction": 0.25, "shell_height": 0.02}),
            ("vegetation-06-scale-heavy-dust-heavy", "Heavy scale jitter and dust", {"kind": "scale_jitter_with_dust", "horizontal_min": 0.85, "horizontal_max": 1.15, "vertical_min": 0.92, "vertical_max": 1.08, "upper_fraction": 0.50, "shell_height": 0.04}),
        ),
        "cliff_strata": (
            ("cliff-01-value-006", "Cliff side value -6%", {"kind": "side_value", "value_delta": -0.06}),
            ("cliff-02-value-012", "Cliff side value -12%", {"kind": "side_value", "value_delta": -0.12}),
            ("cliff-03-strata-24", "Cliff strata 24/2/8%", {"kind": "strata", "period_levels": 24, "width_levels": 2, "contrast": 0.08, "phase_variation_levels": 0, "correlation_hexes": 0}),
            ("cliff-04-strata-40", "Cliff strata 40/4/10%", {"kind": "strata", "period_levels": 40, "width_levels": 4, "contrast": 0.10, "phase_variation_levels": 0, "correlation_hexes": 0}),
            ("cliff-05-strata-coherent", "Coherent cliff strata 32/3/8%", {"kind": "strata", "period_levels": 32, "width_levels": 3, "contrast": 0.08, "phase_variation_levels": 4, "correlation_hexes": 22}),
            ("cliff-06-strata-coherent-value", "Coherent strata with value -8%", {"kind": "strata_with_value", "period_levels": 32, "width_levels": 3, "contrast": 0.08, "phase_variation_levels": 4, "correlation_hexes": 22, "value_delta": -0.08}),
        ),
        "terrain_props": (
            ("props-01-boulders-low", "Boulders 0.15%, cap 160", {"kind": "boulders", "density": 0.0015, "cap": 160}),
            ("props-02-boulders-high", "Boulders 0.35%, cap 370", {"kind": "boulders", "density": 0.0035, "cap": 370}),
            ("props-03-litter-low", "Grass/litter 0.30%, cap 320", {"kind": "grass_litter", "density": 0.0030, "cap": 320}),
            ("props-04-litter-high", "Grass/litter 0.65%, cap 690", {"kind": "grass_litter", "density": 0.0065, "cap": 690}),
            ("props-05-mixed", "Mixed boulders, tufts, deadwood", {"kind": "mixed", "boulder_density": 0.0012, "tuft_density": 0.0030, "deadwood_density": 0.0005, "cap": 500}),
            ("props-06-clustered", "Clustered terrain props", {"kind": "clustered", "center_density": 0.0005, "pieces_min": 3, "pieces_max": 5, "cap": 600}),
        ),
        "ice_fringe": (
            ("ice-01-level-narrow", "Level ≥140 ice 0.15 / 40%", {"kind": "level_fringe", "minimum_level": 140, "width": 0.15, "coverage": 0.40, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
            ("ice-02-level-medium", "Level ≥140 ice 0.30 / 65%", {"kind": "level_fringe", "minimum_level": 140, "width": 0.30, "coverage": 0.65, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
            ("ice-03-level-wide", "Level ≥140 ice 0.45 / 85%", {"kind": "level_fringe", "minimum_level": 140, "width": 0.45, "coverage": 0.85, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
            ("ice-04-snow-adjacent", "Snow-adjacent ice 0.25 / 65%", {"kind": "snow_adjacent", "include_frozen": False, "width": 0.25, "coverage": 0.65, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
            ("ice-05-frozen-or-snow", "Frozen-or-snow ice 0.25 / 65%", {"kind": "snow_adjacent", "include_frozen": True, "width": 0.25, "coverage": 0.65, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
            ("ice-06-frozen-or-snow-feathered", "Frozen-or-snow ice with inward feather", {"kind": "feathered", "include_frozen": True, "width": 0.35, "coverage": 0.75, "inward_feather": 0.10, "alpha": 0.82, "roughness": 0.32, "reflectance": 0.30, "y_bias": 0.006}),
        ),
        "local_fog": (
            ("fog-01-water-light", "Water-hugging fog, light", {"kind": "layer", "placement": "water_hugging", "radius_min": 12.0, "radius_max": 18.0, "height": 1.4, "coverage": 0.10, "opacity": 0.06, "bottom_offset": 0.15}),
            ("fog-02-water-heavy", "Water-hugging fog, heavy", {"kind": "layer", "placement": "water_hugging", "radius_min": 20.0, "radius_max": 30.0, "height": 2.8, "coverage": 0.20, "opacity": 0.10, "bottom_offset": 0.15}),
            ("fog-03-valley-light", "Valley-floor fog, light", {"kind": "layer", "placement": "valley_floor", "radius_min": 14.0, "radius_max": 22.0, "height": 1.8, "coverage": 0.12, "opacity": 0.06, "bottom_offset": 0.15}),
            ("fog-04-valley-heavy", "Valley-floor fog, heavy", {"kind": "layer", "placement": "valley_floor", "radius_min": 24.0, "radius_max": 36.0, "height": 3.5, "coverage": 0.24, "opacity": 0.10, "bottom_offset": 0.15}),
            ("fog-05-mixed", "Mixed water/valley fog", {"kind": "layer", "placement": "mixed", "radius_min": 16.0, "radius_max": 26.0, "height": 2.4, "coverage": 0.16, "opacity": 0.07, "bottom_offset": 0.15}),
            ("fog-06-mixed-cinematic", "Mixed cinematic fog", {"kind": "layer", "placement": "mixed", "radius_min": 28.0, "radius_max": 42.0, "height": 4.5, "coverage": 0.28, "opacity": 0.12, "bottom_offset": 0.15}),
        ),
    }


@functools.lru_cache(maxsize=1)
def atomic_profiles() -> Tuple[DetailProfile, ...]:
    """Return and self-validate the exact 60 non-control profiles."""

    rows = _matrix_rows()
    if tuple(rows) != FAMILY_ORDER:
        raise HarnessError("profile families changed order")
    profiles: List[DetailProfile] = []
    for family in FAMILY_ORDER:
        family_rows = rows[family]
        if len(family_rows) != FAMILY_COUNTS[family]:
            raise HarnessError(f"{family} treatment count changed")
        profiles.extend(_make_profile(profile_id, label, family, section) for profile_id, label, section in family_rows)
    if len(profiles) != 60 or len({profile.id for profile in profiles}) != 60:
        raise HarnessError("world-detail matrix must contain 60 unique profiles")
    for profile in profiles:
        reparsed = json.loads(profile.canonical_json)
        if compact_json(reparsed) != profile.canonical_json:
            raise HarnessError(f"profile {profile.id} is not canonical compact JSON")
        changed = [family for family in FAMILY_ORDER if profile.body[family]["kind"] != "current"]
        if changed != [profile.family]:
            raise HarnessError(f"profile {profile.id} changes more than one family")
    if any("bevel" in profile.id for profile in profiles):
        raise HarnessError("previously rejected geometric bevel work entered the matrix")
    golden = _strict_object(
        _read_json(PROFILE_HASHES_PATH, "profile hash registry"),
        context="profile hash registry",
        required=("version", "warning", "control", "profiles"),
    )
    if golden["version"] != 1 or golden["warning"] != WARNING:
        raise HarnessError("profile hash registry version or warning changed")
    if golden["control"] != CONTROL_PROFILE_SHA256:
        raise HarnessError("profile hash registry control changed")
    expected_hashes = _strict_object(
        golden["profiles"],
        context="profile hash registry profiles",
        required=[profile.id for profile in profiles],
    )
    actual_hashes = {profile.id: profile.sha256 for profile in profiles}
    if expected_hashes != actual_hashes:
        raise HarnessError("one or more exact treatment bodies/canonical hashes changed")
    return tuple(profiles)


def profile_lookup() -> Dict[str, DetailProfile]:
    """Return every atomic profile by stable id plus the control."""

    profiles = (control_profile(), *atomic_profiles())
    return {profile.id: profile for profile in profiles}


def validate_profile_json(profile_json: str) -> Dict[str, Any]:
    """Validate canonical JSON as a strict combination of the exact 60 sections."""

    if not isinstance(profile_json, str):
        raise HarnessError("world-detail profile JSON must be text")
    def reject_constant(value: str) -> None:
        raise HarnessError(f"world-detail profile contains non-finite {value}")

    try:
        raw = json.loads(profile_json, parse_constant=reject_constant)
    except json.JSONDecodeError as error:
        raise HarnessError(f"world-detail profile is malformed JSON: {error}") from error
    profile = _strict_object(
        raw,
        context="world-detail profile",
        required=("version", *FAMILY_ORDER),
    )
    if (
        type(profile["version"]) is not int
        or profile["version"] != PROFILE_VERSION
        or compact_json(profile) != profile_json
    ):
        raise HarnessError("world-detail profile is not version-1 canonical compact JSON")
    allowed_sections = {
        family: {compact_json({"kind": "current"})}
        | {
            compact_json(candidate.body[family])
            for candidate in atomic_profiles()
            if candidate.family == family
        }
        for family in FAMILY_ORDER
    }
    for family in FAMILY_ORDER:
        if compact_json(profile[family]) not in allowed_sections[family]:
            raise HarnessError(f"world-detail profile has a non-matrix {family} section")
    return profile


def _parse_camera(raw: Any, context: str) -> CameraSpec:
    value = _strict_object(
        raw,
        context=context,
        required=("id", "filename", "camera", "view"),
        optional=(
            "focus_anchor",
            "look_at_anchor",
            "look_at_offset",
            "character_radius_scale",
            "full_cutaway",
            "illumination_overlay",
        ),
    )
    camera_id = value["id"]
    if not isinstance(camera_id, str) or not re.fullmatch(r"[0-9]{2}-[a-z0-9-]+", camera_id):
        raise HarnessError(f"{context}.id is invalid")
    filename = value["filename"]
    if filename != f"{camera_id}.png":
        raise HarnessError(f"{context}.filename must be {camera_id}.png")
    camera = value["camera"]
    view = value["view"]
    if camera not in ("map", "character", "first-person"):
        raise HarnessError(f"{context}.camera is invalid")
    if view not in ("default", "rotated", "counter-rotated", "rear", "top-down"):
        raise HarnessError(f"{context}.view is invalid")
    focus = value.get("focus_anchor")
    look = value.get("look_at_anchor")
    offset_raw = value.get("look_at_offset")
    if focus is not None and (not isinstance(focus, str) or not focus):
        raise HarnessError(f"{context}.focus_anchor is invalid")
    if look is not None and (not isinstance(look, str) or not look):
        raise HarnessError(f"{context}.look_at_anchor is invalid")
    if (look is None) != (offset_raw is None):
        raise HarnessError(f"{context} look-at anchor and offset must appear together")
    if focus is not None and look is not None:
        raise HarnessError(f"{context} cannot mix focus and look-at framing")
    offset = None
    if offset_raw is not None:
        if not isinstance(offset_raw, list) or len(offset_raw) != 3:
            raise HarnessError(f"{context}.look_at_offset needs three components")
        offset = tuple(_finite_number(component, f"{context}.look_at_offset") for component in offset_raw)
        distance = math.sqrt(sum(component * component for component in offset))
        if not 1.0 <= distance <= 2048.0 or camera != "map":
            raise HarnessError(f"{context}.look_at_offset or camera is invalid")
    radius = _finite_number(value.get("character_radius_scale", 1.0), f"{context}.character_radius_scale")
    if not 1.0 <= radius <= 20.0 or (radius != 1.0 and camera != "character"):
        raise HarnessError(f"{context}.character_radius_scale is invalid")
    full_cutaway = value.get("full_cutaway", False)
    illumination_overlay = value.get("illumination_overlay", False)
    if not isinstance(full_cutaway, bool):
        raise HarnessError(f"{context}.full_cutaway must be boolean")
    if not isinstance(illumination_overlay, bool):
        raise HarnessError(f"{context}.illumination_overlay must be boolean")
    return CameraSpec(
        id=camera_id,
        filename=filename,
        camera=camera,
        view=view,
        focus_anchor=focus,
        look_at_anchor=look,
        look_at_offset=offset,
        character_radius_scale=radius,
        full_cutaway=full_cutaway,
        illumination_overlay=illumination_overlay,
    )


def _parse_camera_manifest_object(
    raw: Any,
    *,
    context: str,
    expected_id: str,
    expected_count: int,
) -> Tuple[CameraSpec, ...]:
    """Parse one camera authority after its raw/semantic hashes are checked."""

    value = _strict_object(
        raw,
        context=context,
        required=("version", "id", "captures"),
    )
    if value["version"] != 1 or value["id"] != expected_id:
        raise HarnessError(f"{context} identity changed")
    captures_raw = value["captures"]
    if not isinstance(captures_raw, list) or len(captures_raw) != expected_count:
        raise HarnessError(f"{context} count changed")
    cameras = tuple(
        _parse_camera(item, f"{context}.captures[{index}]")
        for index, item in enumerate(captures_raw)
    )
    if len({camera.id for camera in cameras}) != len(cameras):
        raise HarnessError(f"{context} repeats ids")
    return cameras


def load_camera_sets() -> Tuple[Tuple[CameraSpec, ...], Tuple[CameraSpec, ...], Dict[str, Any]]:
    """Load immutable vendored final/focused manifests and verify provenance."""

    provenance = _strict_object(
        _read_json(CAMERA_PROVENANCE_PATH, "camera provenance"),
        context="camera provenance",
        required=("version", "warning", "sources"),
    )
    if provenance["version"] != 1 or provenance["warning"] != WARNING:
        raise HarnessError("camera provenance version or warning changed")
    sources = provenance["sources"]
    if not isinstance(sources, list) or len(sources) != 2:
        raise HarnessError("camera provenance must contain two sources")
    by_path: Dict[pathlib.Path, Tuple[Dict[str, Any], Any]] = {}
    for index, raw in enumerate(sources):
        source = _strict_object(
            raw,
            context=f"camera provenance source {index}",
            required=(
                "vendored",
                "upstream_path",
                "upstream_sha256",
                "vendored_sha256",
                "semantic_sha256",
            ),
        )
        vendored_value = source["vendored"]
        vendored = (
            REPOSITORY_ROOT / vendored_value
            if isinstance(vendored_value, str)
            else pathlib.Path()
        )
        if (
            not isinstance(vendored_value, str)
            or pathlib.PurePosixPath(vendored_value).is_absolute()
            or ".." in pathlib.PurePosixPath(vendored_value).parts
            or vendored.resolve().parent != SPEC_ROOT.resolve()
            or not vendored.is_file()
            or vendored.is_symlink()
        ):
            raise HarnessError("vendored camera path escaped the owned spec directory")
        upstream_value = source["upstream_path"]
        upstream = pathlib.Path(upstream_value) if isinstance(upstream_value, str) else pathlib.Path()
        if (
            not isinstance(upstream_value, str)
            or not upstream.is_absolute()
            or not upstream.is_file()
            or upstream.is_symlink()
        ):
            raise HarnessError(f"upstream camera path is unavailable: {upstream_value!r}")
        for field in ("upstream_sha256", "vendored_sha256", "semantic_sha256"):
            if not isinstance(source[field], str) or not SHA256_RE.fullmatch(source[field]):
                raise HarnessError(f"camera provenance {field} is invalid for {vendored.name}")
        upstream_sha256 = sha256_file(upstream)
        if upstream_sha256 != source["upstream_sha256"]:
            raise HarnessError(f"upstream camera hash changed: {upstream}")
        vendored_sha256 = sha256_file(vendored)
        if vendored_sha256 != source["vendored_sha256"]:
            raise HarnessError(f"vendored camera hash changed: {vendored}")
        upstream_raw = _read_json(upstream, "upstream camera manifest")
        vendored_raw = _read_json(vendored, "vendored camera manifest")
        if (
            sha256_object(upstream_raw) != source["semantic_sha256"]
            or sha256_object(vendored_raw) != source["semantic_sha256"]
            or upstream_raw != vendored_raw
        ):
            raise HarnessError(
                f"upstream and vendored camera manifests are not semantically equivalent: {vendored.name}"
            )
        resolved_vendored = vendored.resolve()
        if resolved_vendored in by_path:
            raise HarnessError(f"camera provenance repeats vendored path: {vendored}")
        by_path[resolved_vendored] = (source, vendored_raw)

    def load(path: pathlib.Path, expected_id: str, expected_count: int) -> Tuple[CameraSpec, ...]:
        if path.resolve() not in by_path:
            raise HarnessError(f"camera manifest lacks provenance: {path}")
        return _parse_camera_manifest_object(
            by_path[path.resolve()][1],
            context=f"camera manifest {path.name}",
            expected_id=expected_id,
            expected_count=expected_count,
        )

    final = load(FINAL_CAMERAS_PATH, "final-17-cameras-v2", 17)
    focused = load(FOCUSED_CAMERAS_PATH, "focused-4-cameras-v1", 4)
    expected_focused_ids = (
        "02-highlands-oblique",
        "03-coast-river-outlet",
        "14-cascade-basin-full-height",
        "16-deep-tree-shade",
    )
    if tuple(camera.id for camera in focused) != expected_focused_ids:
        raise HarnessError("focused camera set changed")
    final_by_id = {camera.id: camera for camera in final}
    if any(final_by_id.get(camera.id) != camera for camera in focused):
        raise HarnessError("focused cameras must be exact final-17 entries")
    if any(camera.id not in final_by_id for camera in focused):
        raise HarnessError("focused camera is absent from final-17")
    return final, focused, provenance


def _validate_sha256_mapping(value: Any, context: str) -> Dict[str, str]:
    mapping = value if isinstance(value, dict) else None
    if mapping is None or not mapping:
        raise HarnessError(f"{context} must be a non-empty object")
    result: Dict[str, str] = {}
    for relative, digest in mapping.items():
        if not isinstance(relative, str):
            raise HarnessError(f"{context} contains a non-string path")
        parsed = pathlib.PurePosixPath(relative)
        if parsed.is_absolute() or ".." in parsed.parts or parsed.as_posix() != relative:
            raise HarnessError(f"{context} contains an unsafe path: {relative!r}")
        if not isinstance(digest, str) or not SHA256_RE.fullmatch(digest):
            raise HarnessError(f"{context} has an invalid hash for {relative}")
        result[relative] = digest
    return result


    return 20.0 * sum(categories[field] * CATEGORY_WEIGHTS[field] for field in CATEGORY_ORDER)


def validate_reviewer_review(raw: Any) -> Dict[str, Any]:
    """Validate one blinded reviewer's strict, independently-authored rating file."""

    review = _exact_keys(
        raw,
        (
            "version",
            "warning",
            "reviewer_id",
            "blinded",
            "independence_attestation",
            "unscored_camera_ids",
            "scoring_contract_sha256",
            "review_packet_sha256",
            "ratings",
        ),
        "reviewer review",
    )
    if review["version"] != 1 or review["warning"] != WARNING:
        raise HarnessError("reviewer review identity changed")
    if not isinstance(review["reviewer_id"], str) or not SAFE_ID_RE.fullmatch(review["reviewer_id"]):
        raise HarnessError("reviewer_id must be a non-identifying kebab-case token")
    if review["blinded"] is not True or review["independence_attestation"] is not True:
        raise HarnessError("review must attest blinded and independent scoring")
    if review["unscored_camera_ids"] != list(UNSCORED_CAMERA_IDS):
        raise HarnessError("review must record views 07 and 08 as unscored leak/regression checks")
    if review["scoring_contract_sha256"] != sha256_object(scoring_contract()):
        raise HarnessError("review was not performed against the current scoring contract")
    if (
        not isinstance(review["review_packet_sha256"], str)
        or not SHA256_RE.fullmatch(review["review_packet_sha256"])
    ):
        raise HarnessError("review_packet_sha256 must bind the blinded image packet")
    if not isinstance(review["ratings"], list):
        raise HarnessError("review ratings must be a list")
    ratings = []
    seen = set()
    for index, raw_rating in enumerate(review["ratings"]):
        rating = _exact_keys(
            raw_rating,
            ("code", "categories"),
            f"review rating {index}",
        )
        code = rating["code"]
        if not isinstance(code, str) or BLIND_CODE_RE.fullmatch(code) is None:
            raise HarnessError(f"review rating {index} code is not an opaque V1 code")
        categories = _exact_keys(
            rating["categories"],
            CATEGORY_ORDER,
            f"review rating {index} categories",
        )
        for field, score in categories.items():
            if isinstance(score, bool) or not isinstance(score, int) or not 1 <= score <= 5:
                raise HarnessError(f"review rating {index} {field} must be an integer from 1 to 5")
        if code in seen:
            raise HarnessError(f"review repeats opaque code {code}")
        seen.add(code)
        ratings.append(
            {
                "code": code,
                "categories": categories,
            }
        )
    return {**review, "ratings": ratings}


def _review_subject_kind(subject_id: str) -> str:
    if subject_id == "control":
        return "control"
    if subject_id in profile_lookup():
        return "atomic"
    if subject_id in {step[0] for step in LADDER_STEPS}:
        return "interaction_ladder"
    if subject_id in {f"winner-{family}" for family in FAMILY_ORDER}:
        return "final_atomic"
    if subject_id in {f"combination-{identifier}" for identifier in COMBINATION_IDS}:
        return "combination"
    raise HarnessError(f"review subject is outside the blinded registry: {subject_id!r}")


def _deterministic_plan_blinding_seed(plan: Mapping[str, Any]) -> str:
    """Return the stable label seed used only by published motion derivatives.

    Review outcomes are deliberately absent.  A no-change motion review adds
    findings and pass/fail evidence to the selection, but does not change the
    visual motion plan whose already-finalized frames those findings describe.
    """

    return sha256_bytes(
        (
            f"{BLINDING_DOMAIN}:published-motion-plan:{SEED}:"
            f"{sha256_object(plan['provenance'])}:"
            f"{sha256_object(plan['motion'])}"
        ).encode("utf-8")
    )


def _private_packet_blinding_seed(
    plan: Mapping[str, Any],
    salt: str,
    *,
    packet_kind: str,
    binding_sha256: str,
) -> str:
    if not isinstance(salt, str) or SHA256_RE.fullmatch(salt) is None:
        raise HarnessError("private packet blinding salt must be 32 random bytes in hex")
    if packet_kind not in (
        "opaque-world-detail-review",
        "opaque-world-detail-motion-review",
    ):
        raise HarnessError(f"unknown private packet kind {packet_kind!r}")
    if not isinstance(binding_sha256, str) or SHA256_RE.fullmatch(binding_sha256) is None:
        raise HarnessError("private packet seed requires its exact material binding")
    return sha256_bytes(
        (
            f"{BLINDING_DOMAIN}:private-packet:{packet_kind}:{salt}:{SEED}:"
            f"{sha256_object(plan['provenance'])}:"
            f"{binding_sha256}"
        ).encode("utf-8")
    )


def _blind_code(seed: str, subject_id: str, condition: str, scoring_context: str) -> str:
    digest = sha256_bytes(
        (
            f"{BLINDING_DOMAIN}:code:{seed}:{subject_id}:{condition}:"
            f"{scoring_context}"
        ).encode("utf-8")
    )
    return f"RV-{digest[:12].upper()}"


def _blind_order_key(
    seed: str,
    subject_id: str,
    condition: str,
    scoring_context: str,
) -> str:
    return sha256_bytes(
        (
            f"{BLINDING_DOMAIN}:order:{seed}:{subject_id}:{condition}:"
            f"{scoring_context}"
        ).encode("utf-8")
    )


def _contextualize_review_frames(
    grouped: Mapping[Tuple[str, str], Mapping[str, Mapping[str, Any]]],
    *,
    final_camera_ids: Sequence[str],
    diagnostic_camera_ids: Sequence[str],
) -> Dict[Tuple[str, str, str], List[Dict[str, Any]]]:
    """Split shared pixels into camera-matched opaque scoring contexts."""

    final_ids = tuple(final_camera_ids)
    diagnostic_ids = tuple(diagnostic_camera_ids)
    if len(final_ids) != 17 or len(set(final_ids)) != 17:
        raise HarnessError("final review context must contain exactly 17 unique cameras")
    if len(diagnostic_ids) != 4 or any(camera_id not in final_ids for camera_id in diagnostic_ids):
        raise HarnessError("diagnostic review context must contain four final-itinerary cameras")
    final_set = set(final_ids)
    diagnostic_set = set(diagnostic_ids)
    contextual: Dict[Tuple[str, str, str], List[Dict[str, Any]]] = {}

    def add_context(
        subject_id: str,
        condition: str,
        context: str,
        frames_by_camera: Mapping[str, Mapping[str, Any]],
        expected_ids: set,
    ) -> None:
        if set(frames_by_camera) != expected_ids:
            raise HarnessError(
                f"review subject {subject_id}/{condition}/{context} has a mismatched camera set"
            )
        contextual[(subject_id, condition, context)] = sorted(
            (dict(frame) for frame in frames_by_camera.values()),
            key=lambda frame: frame["camera_index"],
        )

    for (subject_id, condition), frames_by_camera in grouped.items():
        kind = _review_subject_kind(subject_id)
        present = set(frames_by_camera)
        if kind == "final_atomic":
            add_context(
                subject_id,
                condition,
                FINAL_REVIEW_CONTEXT,
                frames_by_camera,
                final_set,
            )
            continue
        if kind == "combination" and condition == "neutral":
            add_context(
                subject_id,
                condition,
                FINAL_REVIEW_CONTEXT,
                frames_by_camera,
                final_set,
            )
            add_context(
                subject_id,
                condition,
                DIAGNOSTIC_REVIEW_CONTEXT,
                {
                    camera_id: frames_by_camera[camera_id]
                    for camera_id in diagnostic_ids
                },
                diagnostic_set,
            )
            continue
        if subject_id == "control" and condition == "neutral" and present == final_set:
            add_context(
                subject_id,
                condition,
                FINAL_REVIEW_CONTEXT,
                frames_by_camera,
                final_set,
            )
            add_context(
                subject_id,
                condition,
                DIAGNOSTIC_REVIEW_CONTEXT,
                {
                    camera_id: frames_by_camera[camera_id]
                    for camera_id in diagnostic_ids
                },
                diagnostic_set,
            )
            continue
        add_context(
            subject_id,
            condition,
            DIAGNOSTIC_REVIEW_CONTEXT,
            frames_by_camera,
            diagnostic_set,
        )
    return contextual


def _review_subject_frames(
    plan: Mapping[str, Any],
) -> Dict[Tuple[str, str, str], List[Dict[str, Any]]]:
    """Resolve every currently reviewable subject to warning-labeled frames."""

    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    final_cameras, focused_cameras, _ = load_camera_sets()
    camera_order = {camera.id: index for index, camera in enumerate(final_cameras)}
    grouped: Dict[Tuple[str, str], Dict[str, Dict[str, Any]]] = defaultdict(dict)
    rows = [
        *plan["study"]["shared_control_references"],
        *plan["study"]["logical_slots"],
    ]
    for row in rows:
        subject_id = row.get("look_id", "control")
        try:
            subject_kind = _review_subject_kind(subject_id)
        except HarnessError:
            # The final winner projections are evidence for the published sheets,
            # but their atomic score already belongs to the original opaque code.
            continue
        condition = row["lighting"]
        if condition not in LIGHTING_CONDITIONS:
            raise HarnessError(f"review subject {subject_id} has unknown lighting {condition}")
        raw_path = _raw_artifact_path(raw_capture_root, row["artifact"])
        labeled_path = _labeled_capture_path(output_root, raw_path)
        labeled = _inspect_labeled_png(
            labeled_path,
            expected_source_sha256=sha256_file(raw_path),
        )
        camera_id = row["camera_id"]
        frame = {
            "camera_id": camera_id,
            "camera_index": camera_order[camera_id] + 1,
            "scored": camera_id not in UNSCORED_CAMERA_IDS,
            "labeled_path": str(labeled_path.resolve()),
            "sha256": labeled["sha256"],
        }
        existing = grouped[(subject_id, condition)].get(camera_id)
        if existing is not None and existing != frame:
            raise HarnessError(
                f"review subject {subject_id}/{condition} has conflicting {camera_id} frames"
            )
        grouped[(subject_id, condition)][camera_id] = frame
        del subject_kind
    normalized = _contextualize_review_frames(
        grouped,
        final_camera_ids=[camera.id for camera in final_cameras],
        diagnostic_camera_ids=[camera.id for camera in focused_cameras],
    )
    if plan["status"] == "READY_FOR_CAPTURE":
        for family in FAMILY_ORDER:
            key = (f"winner-{family}", "neutral", FINAL_REVIEW_CONTEXT)
            frames = normalized.get(key)
            if (
                frames is None
                or len(frames) != 17
                or sum(frame["scored"] for frame in frames) != 15
            ):
                raise HarnessError(
                    f"final blinded review lacks the exact 17-view atomic decision for {family}"
                )
    return normalized


def _review_material_sha256(
    subjects: Mapping[Tuple[str, str, str], Sequence[Mapping[str, Any]]],
) -> str:
    return sha256_object(
        [
            {
                "subject_id": subject_id,
                "condition": condition,
                "scoring_context": scoring_context,
                "frames": list(subjects[(subject_id, condition, scoring_context)]),
            }
            for subject_id, condition, scoring_context in sorted(subjects)
        ]
    )


def _path_is_within(path: pathlib.Path, root: pathlib.Path) -> bool:
    try:
        path.resolve().relative_to(root.resolve())
    except ValueError:
        return False
    return True


def _require_blinded_packet_root(
    packet_root: pathlib.Path,
    output_root: pathlib.Path,
    context: str,
) -> pathlib.Path:
    packet_root = _require_outside_source_tree(packet_root, context)
    if _path_is_within(packet_root, output_root):
        raise HarnessError(f"{context} may not live inside the unblinded output root")
    return packet_root


def _blinding_build_state(
    *,
    packet_kind: str,
    binding_sha256: str,
    blinding_salt: str,
) -> Dict[str, Any]:
    return {
        "version": 1,
        "warning": WARNING,
        "status": "BUILDING",
        "packet_kind": packet_kind,
        "binding_sha256": binding_sha256,
        "blinding_salt": blinding_salt,
    }


def _load_or_create_private_blinding_salt(
    path: pathlib.Path,
    *,
    packet_kind: str,
    binding_sha256: str,
) -> str:
    """Create or resume one private packet salt without ever publishing it."""

    path = path.resolve()
    if path.is_symlink():
        raise HarnessError(f"private blinding evidence may not be a symlink: {path}")
    if path.exists():
        raw = _read_json(path, "private blinding evidence")
        if not isinstance(raw, dict):
            raise HarnessError("private blinding evidence must be an object")
        salt = raw.get("blinding_salt")
        if (
            raw.get("version") != 1
            or raw.get("warning") != WARNING
            or raw.get("packet_kind") != packet_kind
            or raw.get("binding_sha256") != binding_sha256
            or raw.get("status") not in ("BUILDING", "FINALIZED")
            or not isinstance(salt, str)
            or SHA256_RE.fullmatch(salt) is None
        ):
            raise HarnessError("private blinding evidence cannot resume this packet build")
        if raw["status"] == "BUILDING" and raw != _blinding_build_state(
            packet_kind=packet_kind,
            binding_sha256=binding_sha256,
            blinding_salt=salt,
        ):
            raise HarnessError("private BUILDING evidence has unknown or stale fields")
        return salt
    salt = secrets.token_hex(32)
    atomic_write(
        path,
        pretty_json(
            _blinding_build_state(
                packet_kind=packet_kind,
                binding_sha256=binding_sha256,
                blinding_salt=salt,
            )
        ),
    )
    path.chmod(0o600)
    return salt


def _finalize_private_blinding_evidence(
    path: pathlib.Path,
    finalized: Mapping[str, Any],
) -> None:
    path = path.resolve()
    current = _read_json(path, "private blinding evidence")
    if current == finalized:
        return
    expected_building = _blinding_build_state(
        packet_kind=finalized["packet_kind"],
        binding_sha256=finalized["binding_sha256"],
        blinding_salt=finalized["blinding_salt"],
    )
    if current != expected_building:
        raise HarnessError("refusing to replace changed private blinding evidence")
    atomic_write(path, pretty_json(finalized))
    path.chmod(0o600)


def _inspect_opaque_review_png(
    path: pathlib.Path,
    opaque_code: str,
    *,
    exact_capture_size: bool = True,
) -> Dict[str, Any]:
    if BLIND_CODE_RE.fullmatch(opaque_code) is None:
        raise HarnessError("opaque review PNG needs a valid review code")
    rendered = _inspect_labeled_png(path, exact_capture_size=exact_capture_size)
    try:
        from PIL import Image  # pylint: disable=import-outside-toplevel

        with Image.open(path) as image:
            metadata = dict(image.info)
    except (ImportError, OSError) as error:
        raise HarnessError(f"cannot inspect opaque review PNG {path}") from error
    expected_metadata = {
        "structural_draft_warning": WARNING,
        "opaque_review_code": opaque_code,
    }
    if metadata != expected_metadata:
        raise HarnessError(f"public review PNG {opaque_code} contains non-opaque metadata")
    return rendered


def _materialize_opaque_review_png(
    source: pathlib.Path,
    destination: pathlib.Path,
    opaque_code: str,
) -> Dict[str, Any]:
    """Re-encode source pixels with an exact, correlation-free metadata set."""

    try:
        from PIL import Image, PngImagePlugin  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to materialize blinded review imagery") from error
    _inspect_labeled_png(source)
    if destination.exists():
        if destination.is_symlink():
            raise HarnessError(f"opaque review PNG may not be a symlink: {destination}")
        rendered = _inspect_opaque_review_png(destination, opaque_code)
        if decoded_rgb_sha256(destination) != decoded_rgb_sha256(source):
            raise HarnessError(f"existing opaque review PNG pixels are stale: {destination}")
        return rendered
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    if temporary.exists():
        raise HarnessError(f"stale opaque review PNG temporary exists: {temporary}")
    with Image.open(source) as image:
        pixels = image.convert("RGB")
    metadata = PngImagePlugin.PngInfo()
    metadata.add_text("structural_draft_warning", WARNING)
    metadata.add_text("opaque_review_code", opaque_code)
    pixels.save(temporary, format="PNG", pnginfo=metadata, optimize=False)
    os.replace(temporary, destination)
    return _inspect_opaque_review_png(destination, opaque_code)


def _validate_public_review_packet(packet_path: pathlib.Path) -> Dict[str, Any]:
    packet_path = packet_path.resolve()
    packet = _exact_keys(
        _read_json(packet_path, "blinded review packet"),
        (
            "version",
            "warning",
            "packet_kind",
            "blinding_salt_commitment",
            "scoring_contract_sha256",
            "unscored_camera_ids",
            "categories",
            "entries",
        ),
        "blinded review packet",
    )
    if (
        packet["version"] != 1
        or packet["warning"] != WARNING
        or packet["packet_kind"] != "opaque-world-detail-review"
        or packet["scoring_contract_sha256"] != sha256_object(scoring_contract())
        or packet["unscored_camera_ids"] != list(UNSCORED_CAMERA_IDS)
        or packet["categories"] != list(CATEGORY_ORDER)
        or not isinstance(packet["blinding_salt_commitment"], str)
        or SHA256_RE.fullmatch(packet["blinding_salt_commitment"]) is None
        or not isinstance(packet["entries"], list)
        or not packet["entries"]
    ):
        raise HarnessError("blinded review packet identity or contract changed")
    codes = set()
    referenced_pngs = set()
    normalized_entries = []
    for index, raw_entry in enumerate(packet["entries"], start=1):
        entry = _exact_keys(raw_entry, ("order", "code", "frames"), f"packet entry {index}")
        if entry["order"] != index:
            raise HarnessError("blinded packet order must be contiguous and one-based")
        code = entry["code"]
        if not isinstance(code, str) or BLIND_CODE_RE.fullmatch(code) is None or code in codes:
            raise HarnessError(f"blinded packet has invalid/repeated opaque code {code!r}")
        codes.add(code)
        if not isinstance(entry["frames"], list) or not entry["frames"]:
            raise HarnessError(f"blinded packet {code} has no frames")
        frames = []
        for frame_index, raw_frame in enumerate(entry["frames"], start=1):
            frame = _exact_keys(
                raw_frame,
                ("path", "sha256", "camera_index", "scored"),
                f"packet {code} frame {frame_index}",
            )
            relative = pathlib.Path(frame["path"])
            if relative.is_absolute() or ".." in relative.parts:
                raise HarnessError(f"blinded packet {code} contains an unsafe frame path")
            frame_path = (packet_path.parent / relative).resolve()
            try:
                frame_path.relative_to(packet_path.parent)
            except ValueError as error:
                raise HarnessError(f"blinded packet {code} frame escaped its packet") from error
            if frame_path in referenced_pngs:
                raise HarnessError(f"blinded packet repeats materialized frame {frame_path}")
            referenced_pngs.add(frame_path)
            if (
                not isinstance(frame["sha256"], str)
                or SHA256_RE.fullmatch(frame["sha256"]) is None
                or not isinstance(frame["camera_index"], int)
                or isinstance(frame["camera_index"], bool)
                or not 1 <= frame["camera_index"] <= 17
                or not isinstance(frame["scored"], bool)
            ):
                raise HarnessError(f"blinded packet {code} frame metadata is invalid")
            rendered = _inspect_opaque_review_png(frame_path, code)
            if rendered["sha256"] != frame["sha256"]:
                raise HarnessError(f"blinded packet {code} frame bytes changed")
            frames.append({**frame, "absolute_path": str(frame_path)})
        normalized_entries.append({**entry, "frames": frames})
    materialized_pngs = {
        path.resolve() for path in packet_path.parent.rglob("*.png") if path.is_file()
    }
    if materialized_pngs != referenced_pngs:
        raise HarnessError("blinded packet contains missing or unmanifested PNGs")
    return {
        **packet,
        "entries": normalized_entries,
        "path": str(packet_path),
        "sha256": sha256_file(packet_path),
    }


def validate_blinded_review_packet(
    plan_path: pathlib.Path,
    packet_path: pathlib.Path,
    unblind_map_path: pathlib.Path,
) -> Dict[str, Any]:
    """Validate public packet pixels and privately unblind every opaque code."""

    plan_path = plan_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = pathlib.Path(plan["output_root"]).resolve()
    _require_blinded_packet_root(
        packet_path.resolve().parent,
        output_root,
        "review packet root",
    )
    packet = _validate_public_review_packet(packet_path)
    unblind_map_path = unblind_map_path.resolve()
    for forbidden_root, label in ((output_root, "published output"), (packet_path.resolve().parent, "public packet")):
        try:
            unblind_map_path.relative_to(forbidden_root)
        except ValueError:
            pass
        else:
            raise HarnessError(f"private unblind map may not live inside the {label}")
    unblind = _exact_keys(
        _read_json(unblind_map_path, "private unblind map"),
        (
            "version",
            "warning",
            "status",
            "packet_kind",
            "binding_sha256",
            "review_material_sha256",
            "review_packet_sha256",
            "blinding_salt",
            "entries",
        ),
        "private unblind map",
    )
    expected = _review_subject_frames(plan)
    material_sha256 = _review_material_sha256(expected)
    salt = unblind["blinding_salt"]
    seed = _private_packet_blinding_seed(
        plan,
        salt,
        packet_kind="opaque-world-detail-review",
        binding_sha256=material_sha256,
    )
    if (
        unblind["version"] != 1
        or unblind["warning"] != WARNING
        or unblind["status"] != "FINALIZED"
        or unblind["packet_kind"] != "opaque-world-detail-review"
        or unblind["binding_sha256"] != material_sha256
        or unblind["review_material_sha256"] != material_sha256
        or unblind["review_packet_sha256"] != packet["sha256"]
        or sha256_bytes(salt.encode("utf-8")) != packet["blinding_salt_commitment"]
        or not isinstance(unblind["entries"], list)
    ):
        raise HarnessError("private unblind map does not bind the current packet and plan")

    public_by_code = {entry["code"]: entry for entry in packet["entries"]}
    mapped = {}
    identities = []
    for index, raw_entry in enumerate(unblind["entries"]):
        entry = _exact_keys(
            raw_entry,
            (
                "code",
                "subject_id",
                "subject_kind",
                "condition",
                "scoring_context",
                "source_frames",
                "public_frame_sha256",
            ),
            f"unblind entry {index}",
        )
        code = entry["code"]
        subject_id = entry["subject_id"]
        condition = entry["condition"]
        scoring_context = entry["scoring_context"]
        identity = (subject_id, condition, scoring_context)
        if (
            code not in public_by_code
            or code in mapped
            or condition not in LIGHTING_CONDITIONS
            or scoring_context not in REVIEW_CONTEXTS
            or entry["subject_kind"] != _review_subject_kind(subject_id)
            or code != _blind_code(seed, subject_id, condition, scoring_context)
            or identity not in expected
        ):
            raise HarnessError(f"private unblind entry {index} is invalid")
        if not isinstance(entry["source_frames"], list):
            raise HarnessError(f"private unblind entry {index} lacks source frames")
        source_frames = []
        for frame_index, raw_frame in enumerate(entry["source_frames"]):
            frame = _exact_keys(
                raw_frame,
                ("camera_id", "camera_index", "scored", "labeled_path", "sha256"),
                f"unblind entry {index} frame {frame_index}",
            )
            source_path = pathlib.Path(frame["labeled_path"]).resolve()
            if source_path.is_symlink() or sha256_file(source_path) != frame["sha256"]:
                raise HarnessError(f"unblind source frame changed for {code}")
            source_frames.append(frame)
        if source_frames != expected[identity]:
            raise HarnessError(f"unblind source mapping changed for {code}")
        public_frames = public_by_code[code]["frames"]
        public_hashes = [frame["sha256"] for frame in public_frames]
        if entry["public_frame_sha256"] != public_hashes:
            raise HarnessError(f"public/private packet hashes differ for {code}")
        if [frame["scored"] for frame in public_frames] != [frame["scored"] for frame in source_frames]:
            raise HarnessError(f"public/private score masks differ for {code}")
        if [frame["camera_index"] for frame in public_frames] != [
            frame["camera_index"] for frame in source_frames
        ]:
            raise HarnessError(f"public/private camera order differs for {code}")
        for public_frame, source_frame in zip(public_frames, source_frames):
            if public_frame["sha256"] == source_frame["sha256"]:
                raise HarnessError(f"opaque packet reused source PNG bytes for {code}")
            if decoded_rgb_sha256(pathlib.Path(public_frame["absolute_path"])) != decoded_rgb_sha256(
                pathlib.Path(source_frame["labeled_path"])
            ):
                raise HarnessError(f"opaque packet pixels differ from their source for {code}")
        mapped[code] = {
            "subject_id": subject_id,
            "subject_kind": entry["subject_kind"],
            "condition": condition,
            "scoring_context": scoring_context,
        }
        identities.append(identity)
    if set(mapped) != set(public_by_code) or set(identities) != set(expected):
        raise HarnessError("unblind map does not cover the exact current packet")
    expected_order = sorted(
        expected,
        key=lambda identity: _blind_order_key(seed, *identity),
    )
    actual_order = [
        (
            mapped[entry["code"]]["subject_id"],
            mapped[entry["code"]]["condition"],
            mapped[entry["code"]]["scoring_context"],
        )
        for entry in packet["entries"]
    ]
    if actual_order != expected_order:
        raise HarnessError("blinded packet randomization order changed")
    return {
        "version": 1,
        "warning": WARNING,
        "packet_path": str(packet_path.resolve()),
        "packet_sha256": packet["sha256"],
        "unblind_map_path": str(unblind_map_path),
        "unblind_map_sha256": sha256_file(unblind_map_path),
        "code_map": mapped,
        "entry_count": len(mapped),
    }


def build_blinded_review_packet(
    plan_path: pathlib.Path,
    packet_root: pathlib.Path,
    unblind_map_path: pathlib.Path,
) -> Dict[str, Any]:
    """Materialize anonymous labeled pixels plus a separately held unblind map."""

    plan_path = plan_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = pathlib.Path(plan["output_root"]).resolve()
    packet_root = _require_blinded_packet_root(
        packet_root,
        output_root,
        "review packet root",
    )
    unblind_map_path = _require_outside_source_tree(unblind_map_path, "private unblind map")
    for forbidden_root, label in ((output_root, "published output"), (packet_root, "public packet")):
        try:
            unblind_map_path.relative_to(forbidden_root)
        except ValueError:
            pass
        else:
            raise HarnessError(f"private unblind map may not live inside the {label}")
    subjects = _review_subject_frames(plan)
    material_sha256 = _review_material_sha256(subjects)
    salt = _load_or_create_private_blinding_salt(
        unblind_map_path,
        packet_kind="opaque-world-detail-review",
        binding_sha256=material_sha256,
    )
    seed = _private_packet_blinding_seed(
        plan,
        salt,
        packet_kind="opaque-world-detail-review",
        binding_sha256=material_sha256,
    )
    identities = sorted(
        subjects,
        key=lambda identity: _blind_order_key(seed, *identity),
    )
    entries = []
    private_entries = []
    seen_codes = set()
    for order, (subject_id, condition, scoring_context) in enumerate(identities, start=1):
        code = _blind_code(seed, subject_id, condition, scoring_context)
        if code in seen_codes:
            raise HarnessError("opaque review code collision")
        seen_codes.add(code)
        public_frames = []
        for frame_number, source in enumerate(
            subjects[(subject_id, condition, scoring_context)],
            start=1,
        ):
            relative = pathlib.Path("images") / code / f"frame-{frame_number:02d}.png"
            destination = packet_root / relative
            source_path = pathlib.Path(source["labeled_path"])
            rendered = _materialize_opaque_review_png(source_path, destination, code)
            public_frames.append(
                {
                    "path": relative.as_posix(),
                    "sha256": rendered["sha256"],
                    "camera_index": source["camera_index"],
                    "scored": source["scored"],
                }
            )
        entries.append({"order": order, "code": code, "frames": public_frames})
        private_entries.append(
            {
                "code": code,
                "subject_id": subject_id,
                "subject_kind": _review_subject_kind(subject_id),
                "condition": condition,
                "scoring_context": scoring_context,
                "source_frames": subjects[(subject_id, condition, scoring_context)],
                "public_frame_sha256": [frame["sha256"] for frame in public_frames],
            }
        )
    packet = {
        "version": 1,
        "warning": WARNING,
        "packet_kind": "opaque-world-detail-review",
        "blinding_salt_commitment": sha256_bytes(salt.encode("utf-8")),
        "scoring_contract_sha256": sha256_object(scoring_contract()),
        "unscored_camera_ids": list(UNSCORED_CAMERA_IDS),
        "categories": list(CATEGORY_ORDER),
        "entries": entries,
    }
    packet_path = packet_root / "packet.json"
    _write_if_absent_or_equal(packet_path, pretty_json(packet))
    private = {
        "version": 1,
        "warning": WARNING,
        "status": "FINALIZED",
        "packet_kind": "opaque-world-detail-review",
        "binding_sha256": material_sha256,
        "review_material_sha256": material_sha256,
        "review_packet_sha256": sha256_file(packet_path),
        "blinding_salt": salt,
        "entries": private_entries,
    }
    _finalize_private_blinding_evidence(unblind_map_path, private)
    return validate_blinded_review_packet(plan_path, packet_path, unblind_map_path)


def blinded_review_template(packet_path: pathlib.Path, reviewer_id: str) -> Dict[str, Any]:
    """Return an ID-free reviewer form for one public packet."""

    if not isinstance(reviewer_id, str) or SAFE_ID_RE.fullmatch(reviewer_id) is None:
        raise HarnessError("reviewer-id must be a non-identifying kebab-case token")
    identity = _read_json(packet_path, "blinded review packet")
    if isinstance(identity, dict) and identity.get("packet_kind") == "opaque-world-detail-motion-review":
        packet = _validate_public_motion_review_packet(packet_path)
    else:
        packet = _validate_public_review_packet(packet_path)
    return {
        "version": 1,
        "warning": WARNING,
        "reviewer_id": reviewer_id,
        "blinded": True,
        "independence_attestation": True,
        "unscored_camera_ids": list(UNSCORED_CAMERA_IDS),
        "scoring_contract_sha256": sha256_object(scoring_contract()),
        "review_packet_sha256": packet["sha256"],
        "ratings": [
            {
                "code": entry["code"],
                "categories": {category: None for category in CATEGORY_ORDER},
            }
            for entry in packet["entries"]
        ],
    }


def validate_metric_evidence(raw: Any) -> Dict[str, Any]:
    """Validate perceptual comparisons and recompute exact/near-duplicate labels."""

    evidence = _exact_keys(raw, ("version", "warning", "comparisons"), "metric evidence")
    if evidence["version"] != 1 or evidence["warning"] != WARNING:
        raise HarnessError("metric evidence identity changed")
    if not isinstance(evidence["comparisons"], list):
        raise HarnessError("metric comparisons must be a list")
    comparisons = []
    seen = set()
    for index, raw_comparison in enumerate(evidence["comparisons"]):
        comparison = _exact_keys(
            raw_comparison,
            (
                "subject_id",
                "camera_id",
                "control_sha256",
                "candidate_sha256",
                "control_rgb_sha256",
                "candidate_rgb_sha256",
                "ssim",
                "mean_delta_e00",
                "exact_duplicate",
                "near_duplicate",
            ),
            f"metric comparison {index}",
        )
        subject = comparison["subject_id"]
        if subject not in profile_lookup() or subject == "control":
            raise HarnessError(f"metric comparison {index} must name one atomic profile")
        profile = profile_lookup()[subject]
        if comparison["camera_id"] != PRIMARY_CAMERAS[profile.family] or subject in seen:
            raise HarnessError(f"metric comparison {index} changed its named diagnostic camera")
        seen.add(subject)
        for field in (
            "control_sha256",
            "candidate_sha256",
            "control_rgb_sha256",
            "candidate_rgb_sha256",
        ):
            if not isinstance(comparison[field], str) or not SHA256_RE.fullmatch(comparison[field]):
                raise HarnessError(f"metric comparison {index} has invalid {field}")
        ssim = _finite_number(comparison["ssim"], f"metric comparison {index} ssim")
        delta = _finite_number(
            comparison["mean_delta_e00"],
            f"metric comparison {index} mean_delta_e00",
        )
        if not -1.0 <= ssim <= 1.0 or not 0.0 <= delta <= 200.0:
            raise HarnessError(f"metric comparison {index} values are out of range")
        exact = comparison["control_rgb_sha256"] == comparison["candidate_rgb_sha256"]
        near = ssim >= 0.995 and delta <= 1.5
        if comparison["exact_duplicate"] is not exact or comparison["near_duplicate"] is not near:
            raise HarnessError(f"metric comparison {index} duplicate classification was not recomputed")
        if exact:
            raise HarnessError(
                f"fatal capture-harness failure: {subject} exactly duplicates control at its named camera"
            )
        comparisons.append({**comparison, "ssim": ssim, "mean_delta_e00": delta})
    expected_subjects = {profile.id for profile in atomic_profiles()}
    if seen != expected_subjects:
        raise HarnessError("metric evidence must contain exactly one named-camera comparison for all 60 profiles")
    return {**evidence, "comparisons": comparisons}


def validate_selection_performance_evidence(raw: Any) -> Dict[str, Any]:
    """Validate deterministic atomic runtime costs used only as the last score tie-break."""

    evidence = _exact_keys(raw, ("version", "warning", "subjects"), "selection performance evidence")
    if evidence["version"] != 1 or evidence["warning"] != WARNING:
        raise HarnessError("selection performance evidence identity changed")
    expected = ("control", *(profile.id for profile in atomic_profiles()))
    subjects = _strict_object(evidence["subjects"], context="selection performance subjects", required=expected)
    normalized = {}
    for subject_id in expected:
        row = _exact_keys(
            subjects[subject_id],
            ("p95_frame_time_ms", "max_resident_presentation_bytes"),
            f"selection performance {subject_id}",
        )
        frame_time = _finite_number(row["p95_frame_time_ms"], f"selection performance {subject_id} frame time")
        memory = row["max_resident_presentation_bytes"]
        if not 0.0 < frame_time <= 10_000.0:
            raise HarnessError(f"selection performance {subject_id} frame time is out of range")
        if isinstance(memory, bool) or not isinstance(memory, int) or memory <= 0:
            raise HarnessError(f"selection performance {subject_id} memory is invalid")
        normalized[subject_id] = {
            "p95_frame_time_ms": frame_time,
            "max_resident_presentation_bytes": memory,
        }
    return {**evidence, "subjects": normalized}


def _weighted_score(categories: Mapping[str, float]) -> float:
    return 20.0 * sum(categories[field] * CATEGORY_WEIGHTS[field] for field in CATEGORY_ORDER)


def _aggregate_reviewer_pair(reviews: Sequence[Mapping[str, Any]]) -> Dict[str, Dict[str, Any]]:
    if len(reviews) != 2 or reviews[0]["reviewer_id"] == reviews[1]["reviewer_id"]:
        raise HarnessError("exactly two distinct blinded reviewers are required")
    if reviews[0]["review_packet_sha256"] != reviews[1]["review_packet_sha256"]:
        raise HarnessError("the two reviewers must score the same hash-pinned blinded packet")
    indexed = []
    for review in reviews:
        indexed.append(
            {
                row["code"]: row["categories"]
                for row in review["ratings"]
            }
        )
    if set(indexed[0]) != set(indexed[1]):
        raise HarnessError("the two reviewers must rate the exact same subject/condition keys")
    aggregate = {}
    for key in sorted(indexed[0]):
        categories = {
            field: (indexed[0][key][field] + indexed[1][key][field]) / 2.0
            for field in CATEGORY_ORDER
        }
        minimum_categories = {
            field: min(indexed[0][key][field], indexed[1][key][field])
            for field in CATEGORY_ORDER
        }
        aggregate[key] = {
            "code": key,
            "categories": categories,
            "minimum_categories": minimum_categories,
            "weighted_score": _weighted_score(categories),
        }
    return aggregate


def _tactical_floor_passes(row: Mapping[str, Any]) -> bool:
    return (
        row["minimum_categories"]["terrain_route_water_edge_readability"] >= 3
        and row["minimum_categories"]["edge_temporal_quietness"] >= 3
    )


def _stress_passes(neutral: Mapping[str, Any], stress: Mapping[str, Any]) -> bool:
    return (
        _tactical_floor_passes(neutral)
        and _tactical_floor_passes(stress)
        and stress["weighted_score"] >= 65.0
        and min(stress["minimum_categories"].values()) > 1
        and neutral["weighted_score"] - stress["weighted_score"] <= 12.0
    )


def _selection_decisions_sha256(selection: Mapping[str, Any]) -> str:
    return sha256_object(
        {
            "promoted": selection["promoted"],
            "stress_diagnostics": selection["stress_diagnostics"],
            "ladder_inputs": selection["ladder_inputs"],
            "interaction_findings": selection["interaction_findings"],
            "pre_motion_atomic_winners": selection["pre_motion_atomic_winners"],
            "pre_motion_combinations": selection["pre_motion_combinations"],
            "motion_findings": selection["motion_findings"],
            "motion_combination_pass": selection["motion_combination_pass"],
            "atomic_winners": selection["atomic_winners"],
            "combinations": selection["combinations"],
        }
    )


def derive_selection_from_reviews(
    plan_path: pathlib.Path,
    review_paths: Sequence[pathlib.Path],
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
    packet_path: pathlib.Path,
    unblind_map_path: pathlib.Path,
    motion_review_paths: Optional[Sequence[pathlib.Path]] = None,
    motion_packet_path: Optional[pathlib.Path] = None,
    motion_unblind_map_path: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Derive each adaptive stage from validated, opaque-code reviewer evidence."""

    if len(review_paths) != 2:
        raise HarnessError("derive-selection requires exactly two reviewer files")
    plan_path = plan_path.resolve()
    plan_document = validate_capture_document(_read_json(plan_path, "capture plan"))
    motion_arguments = (
        motion_review_paths is not None,
        motion_packet_path is not None,
        motion_unblind_map_path is not None,
    )
    if any(motion_arguments) and not all(motion_arguments):
        raise HarnessError(
            "motion gating requires two reviews, a public packet, and a private unblind map"
        )
    if motion_review_paths is not None and len(motion_review_paths) != 2:
        raise HarnessError("motion gating requires exactly two reviewer files")
    packet_evidence = validate_blinded_review_packet(
        plan_path,
        packet_path,
        unblind_map_path,
    )
    reviews = [
        validate_reviewer_review(_read_json(path, "reviewer review"))
        for path in review_paths
    ]
    if any(
        review["review_packet_sha256"] != packet_evidence["packet_sha256"]
        for review in reviews
    ):
        raise HarnessError("review files do not bind the validated blinded packet")
    coded_aggregate = _aggregate_reviewer_pair(reviews)
    if set(coded_aggregate) != set(packet_evidence["code_map"]):
        raise HarnessError("both reviewers must score every and only current opaque packet code")
    aggregate: Dict[Tuple[str, str, str], Dict[str, Any]] = {}
    for code, row in coded_aggregate.items():
        identity = packet_evidence["code_map"][code]
        key = (
            identity["subject_id"],
            identity["condition"],
            identity["scoring_context"],
        )
        if key in aggregate:
            raise HarnessError(f"unblind map repeats review identity {key}")
        aggregate[key] = {
            **row,
            "subject_id": key[0],
            "subject_kind": identity["subject_kind"],
            "condition": key[1],
            "scoring_context": key[2],
        }

    metrics = validate_metric_evidence(_read_json(metric_path, "metric evidence"))
    performance = validate_selection_performance_evidence(
        _read_json(performance_path, "selection performance evidence")
    )
    metric_by_id = {row["subject_id"]: row for row in metrics["comparisons"]}
    required_base = {("control", "neutral", DIAGNOSTIC_REVIEW_CONTEXT)}
    required_base.update(
        (profile.id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
        for profile in atomic_profiles()
    )
    missing = sorted(required_base - set(aggregate))
    if missing:
        raise HarnessError(f"review pair lacks required base ratings: {missing[:5]}")

    def ranking_key(row: Mapping[str, Any]) -> Tuple[Any, ...]:
        return (
            -row["neutral"]["weighted_score"],
            -row["neutral"]["categories"]["terrain_route_water_edge_readability"],
            -row["neutral"]["categories"]["shadow_occlusion_preservation"],
            -row["neutral"]["categories"]["edge_temporal_quietness"],
            row["runtime"]["p95_frame_time_ms"],
            row["runtime"]["max_resident_presentation_bytes"],
            row["profile_id"],
        )

    selection = selection_template()
    selection["status"] = "PROMOTIONS_DERIVED_FROM_OPAQUE_TWO_REVIEWER_PACKET"
    rankings: Dict[str, Any] = {}
    finalists_by_family: Dict[str, List[Dict[str, Any]]] = {}
    all_missing_stress: List[Tuple[str, str, str]] = []
    control_neutral = aggregate[("control", "neutral", DIAGNOSTIC_REVIEW_CONTEXT)]
    for family in FAMILY_ORDER:
        candidates = []
        for profile in (item for item in atomic_profiles() if item.family == family):
            neutral = aggregate[(profile.id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)]
            metric = metric_by_id[profile.id]
            runtime = performance["subjects"][profile.id]
            floor_pass = (
                neutral["minimum_categories"]["terrain_route_water_edge_readability"] >= 3
                and neutral["minimum_categories"]["edge_temporal_quietness"] >= 3
                and not metric["exact_duplicate"]
            )
            candidates.append(
                {
                    "profile_id": profile.id,
                    "neutral": neutral,
                    "metric": metric,
                    "runtime": runtime,
                    "floor_pass": floor_pass,
                }
            )
        candidates.sort(key=ranking_key)
        passing = [row for row in candidates if row["floor_pass"]]
        visible = [row for row in passing if not row["metric"]["near_duplicate"]]
        promotion_pool = visible if len(visible) >= 2 else passing
        # A failed readability/edge/duplicate gate can never be relabelled as a
        # finalist merely to fill the nominal two-slot matrix. A family may
        # therefore promote zero or one candidate and resolve to control.
        promoted = sorted(promotion_pool, key=ranking_key)[:2]
        family_floor_viable = len(promoted) == 2
        selection["promoted"][family] = [row["profile_id"] for row in promoted]
        diagnostic_rows = list(promoted)
        diagnostic_rows.extend(
            row
            for row in candidates
            if row["profile_id"] not in {item["profile_id"] for item in diagnostic_rows}
        )
        diagnostic_rows = diagnostic_rows[:2]
        if len(diagnostic_rows) != 2:
            raise HarnessError(f"{family} cannot populate its two neutral stress diagnostics")
        selection["stress_diagnostics"][family] = [
            row["profile_id"] for row in diagnostic_rows
        ]

        missing_stress = [
            (row["profile_id"], condition, DIAGNOSTIC_REVIEW_CONTEXT)
            for row in diagnostic_rows
            for condition in ("golden", "overcast")
            if (row["profile_id"], condition, DIAGNOSTIC_REVIEW_CONTEXT) not in aggregate
        ]
        all_missing_stress.extend(missing_stress)
        passing_finalists = []
        if not missing_stress:
            for row in promoted:
                neutral = row["neutral"]
                stresses = {
                    condition: aggregate[
                        (row["profile_id"], condition, DIAGNOSTIC_REVIEW_CONTEXT)
                    ]
                    for condition in ("golden", "overcast")
                }
                stress_pass = all(
                    _stress_passes(neutral, stresses[condition])
                    for condition in ("golden", "overcast")
                )
                margin = neutral["weighted_score"] - control_neutral["weighted_score"]
                row["stress"] = stresses
                row["stress_pass"] = stress_pass
                row["margin"] = margin
                row["recommendable"] = (
                    family_floor_viable
                    and row["floor_pass"]
                    and stress_pass
                    and margin >= 2.0
                    and not row["metric"]["near_duplicate"]
                )
                if row["floor_pass"] and stress_pass:
                    passing_finalists.append(row)
            recommendable = sorted(
                [row for row in passing_finalists if row["recommendable"]],
                key=ranking_key,
            )
            selection["ladder_inputs"][family] = (
                recommendable[0]["profile_id"] if recommendable else "control"
            )
        finalists_by_family[family] = passing_finalists
        rankings[family] = candidates

    any_stress = any(
        condition in ("golden", "overcast") and subject_id != "control"
        for subject_id, condition, _scoring_context in aggregate
    )
    if all_missing_stress and any_stress:
        raise HarnessError("promoted stress reviews must be complete for all families or absent")
    stress_complete = not all_missing_stress
    complete = False
    missing_ladder: List[Tuple[str, str, str]] = []
    if stress_complete:
        selection["status"] = "INTERACTION_LADDER_INPUTS_DERIVED"
        required_ladder = {
            (step[0], "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
            for step in LADDER_STEPS
        }
        missing_ladder = sorted(required_ladder - set(aggregate))
        present_ladder = required_ladder & set(aggregate)
        if missing_ladder and present_ladder:
            raise HarnessError("interaction-ladder review must be complete or absent as one stage")
        if not missing_ladder:
            winners = dict(selection["ladder_inputs"])
            findings = []
            for subject_id, predecessor_id, introduced in LADDER_STEPS:
                subject = aggregate[
                    (subject_id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
                ]
                predecessor = aggregate[
                    (predecessor_id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
                ]
                weighted_delta = subject["weighted_score"] - predecessor["weighted_score"]
                passed = (
                    subject["minimum_categories"][
                        "terrain_route_water_edge_readability"
                    ]
                    >= 3
                    and subject["minimum_categories"]["edge_temporal_quietness"] >= 3
                    and weighted_delta >= -2.0
                )
                if not passed:
                    for family in introduced:
                        winners[family] = "control"
                findings.append(
                    {
                        "step_id": subject_id,
                        "predecessor": predecessor_id,
                        "introduced_families": list(introduced),
                        "weighted_score": subject["weighted_score"],
                        "predecessor_weighted_score": predecessor["weighted_score"],
                        "weighted_delta": weighted_delta,
                        "minimum_readability": subject["minimum_categories"][
                            "terrain_route_water_edge_readability"
                        ],
                        "minimum_edge_quietness": subject["minimum_categories"][
                            "edge_temporal_quietness"
                        ],
                        "passed": passed,
                        "vetoed_families": [] if passed else list(introduced),
                    }
                )
            selection["interaction_findings"] = findings
            selection["atomic_winners"] = winners
            for family in FAMILY_ORDER:
                if winners[family] == "control":
                    for combination_id in COMBINATION_IDS:
                        selection["combinations"][combination_id][family] = "control"
                    continue
                by_delta = sorted(
                    finalists_by_family[family],
                    key=lambda row: (
                        row["metric"]["mean_delta_e00"],
                        1.0 - row["metric"]["ssim"],
                        row["profile_id"],
                    ),
                )
                selection["combinations"]["restrained"][family] = by_delta[0]["profile_id"]
                selection["combinations"]["expressive"][family] = by_delta[-1]["profile_id"]
                selection["combinations"]["score-leader"][family] = winners[family]
            selection["pre_motion_atomic_winners"] = copy.deepcopy(
                selection["atomic_winners"]
            )
            selection["pre_motion_combinations"] = copy.deepcopy(
                selection["combinations"]
            )
            selection["status"] = "LADDER_GATED_FINALISTS_DERIVED"
            complete = True

    motion_evidence = None
    motion_revalidation_required = False
    if complete and motion_review_paths is not None:
        assert motion_packet_path is not None and motion_unblind_map_path is not None
        motion_packet = validate_blinded_motion_review_packet(
            plan_path,
            motion_packet_path,
            motion_unblind_map_path,
        )
        motion_reviews = [
            validate_reviewer_review(_read_json(path, "motion reviewer review"))
            for path in motion_review_paths
        ]
        if [review["reviewer_id"] for review in motion_reviews] != [
            review["reviewer_id"] for review in reviews
        ]:
            raise HarnessError("motion review must use the same two reviewers in the same order")
        if any(
            review["review_packet_sha256"] != motion_packet["packet_sha256"]
            for review in motion_reviews
        ):
            raise HarnessError("motion reviews do not bind the validated motion packet")
        motion_aggregate = _aggregate_reviewer_pair(motion_reviews)
        if set(motion_aggregate) != set(motion_packet["code_map"]):
            raise HarnessError("motion reviewers must score every and only the 22 opaque clip codes")
        tested_motion_winners = copy.deepcopy(
            plan_document["selection"]["pre_motion_atomic_winners"]
        )
        tested_motion_combinations = copy.deepcopy(
            plan_document["selection"]["pre_motion_combinations"]
        )
        selection["atomic_winners"] = copy.deepcopy(tested_motion_winners)
        selection["combinations"] = copy.deepcopy(tested_motion_combinations)
        selection["pre_motion_atomic_winners"] = copy.deepcopy(tested_motion_winners)
        selection["pre_motion_combinations"] = copy.deepcopy(
            tested_motion_combinations
        )

        clip_pass = {}
        findings = []
        for clip in plan_document["motion"]["clips"]:
            matches = [
                (code, row)
                for code, row in motion_aggregate.items()
                if motion_packet["code_map"][code]["clip_id"] == clip["id"]
            ]
            if len(matches) != 1:
                raise HarnessError(f"motion review cannot resolve clip {clip['id']}")
            code, row = matches[0]
            passed = (
                row["minimum_categories"]["terrain_route_water_edge_readability"] >= 3
                and row["minimum_categories"]["edge_temporal_quietness"] >= 3
            )
            clip_pass[clip["id"]] = passed
            findings.append(
                {
                    "clip_id": clip["id"],
                    "subject_code": code,
                    "minimum_readability": row["minimum_categories"][
                        "terrain_route_water_edge_readability"
                    ],
                    "minimum_edge_quietness": row["minimum_categories"][
                        "edge_temporal_quietness"
                    ],
                    "passed": passed,
                }
            )

        clips_by_profile = defaultdict(list)
        for clip in plan_document["motion"]["clips"]:
            clips_by_profile[clip["profile_id"]].append(clip)
        for family in RISKY_MOTION_FAMILIES:
            winner = selection["atomic_winners"][family]
            if winner == "control":
                continue
            winner_clips = [
                clip
                for clip in clips_by_profile[winner]
                if clip["id"].startswith(f"diagnostic-{family.replace('_', '-')}-")
            ]
            if len(winner_clips) != 1:
                raise HarnessError(f"motion gate cannot resolve {family} winner clip")
            if not clip_pass[winner_clips[0]["id"]]:
                alternatives = [
                    row
                    for row in finalists_by_family[family]
                    if row["profile_id"] != winner
                    and row.get("recommendable", False)
                    and any(
                        clip_pass[clip["id"]]
                        for clip in clips_by_profile[row["profile_id"]]
                        if clip["id"].startswith(
                            f"diagnostic-{family.replace('_', '-')}-"
                        )
                    )
                ]
                alternatives.sort(key=ranking_key)
                selection["atomic_winners"][family] = (
                    alternatives[0]["profile_id"] if alternatives else "control"
                )
            profile_motion_pass = {
                row["profile_id"]: any(
                    clip_pass[clip["id"]]
                    for clip in clips_by_profile[row["profile_id"]]
                    if clip["id"].startswith(
                        f"diagnostic-{family.replace('_', '-')}-"
                    )
                )
                for row in finalists_by_family[family]
            }
            for combination_id in ("restrained", "expressive"):
                selected_profile = selection["combinations"][combination_id][family]
                if selected_profile == "control" or profile_motion_pass.get(
                    selected_profile, False
                ):
                    continue
                passing_rows = [
                    row
                    for row in finalists_by_family[family]
                    if row.get("recommendable", False)
                    and profile_motion_pass.get(row["profile_id"], False)
                ]
                passing_rows.sort(
                    key=lambda row: (
                        row["metric"]["mean_delta_e00"],
                        1.0 - row["metric"]["ssim"],
                        row["profile_id"],
                    ),
                    reverse=combination_id == "expressive",
                )
                selection["combinations"][combination_id][family] = (
                    passing_rows[0]["profile_id"] if passing_rows else "control"
                )
        for family in WINNER_MOTION_FAMILIES:
            baseline_winner = tested_motion_winners[family]
            if baseline_winner == "control":
                continue
            matches = [
                clip
                for clip in plan_document["motion"]["clips"]
                if clip["id"].startswith(f"winner-{family.replace('_', '-')}-")
            ]
            if len(matches) != 1 or not clip_pass[matches[0]["id"]]:
                selection["atomic_winners"][family] = "control"
                for combination_id in ("restrained", "expressive"):
                    if (
                        selection["combinations"][combination_id][family]
                        == baseline_winner
                    ):
                        selection["combinations"][combination_id][family] = "control"
        selection["combinations"]["score-leader"] = copy.deepcopy(
            selection["atomic_winners"]
        )
        for combination_id in COMBINATION_IDS:
            relevant = [
                clip
                for clip in plan_document["motion"]["clips"]
                if clip["id"].startswith(f"combination-{combination_id}-")
                or (
                    combination_id == "score-leader"
                    and clip["id"] in ("leader-golden-03", "leader-overcast-02")
                )
            ]
            expected_count = 4 if combination_id == "score-leader" else 2
            if len(relevant) != expected_count:
                raise HarnessError(f"motion gate has incomplete {combination_id} clip coverage")
            selection["motion_combination_pass"][combination_id] = all(
                clip_pass[clip["id"]] for clip in relevant
            )
        motion_revalidation_required = (
            selection["atomic_winners"] != tested_motion_winners
            or selection["combinations"] != tested_motion_combinations
        )
        if motion_revalidation_required:
            # The changed projections have not been captured, reviewed, or
            # performance-tested.  Promote them to the next motion baseline and
            # discard every stale pass so a fresh plan/packet is mandatory.
            selection["pre_motion_atomic_winners"] = copy.deepcopy(
                selection["atomic_winners"]
            )
            selection["pre_motion_combinations"] = copy.deepcopy(
                selection["combinations"]
            )
            selection["motion_findings"] = []
            selection["motion_combination_pass"] = {
                identifier: None for identifier in COMBINATION_IDS
            }
            selection["status"] = "MOTION_RECAPTURE_REREVIEW_REPERF_REQUIRED"
        else:
            selection["motion_findings"] = findings
            selection["status"] = "MOTION_GATED_FINALISTS_DERIVED"
            motion_evidence = {
                "reviewer_ids": [review["reviewer_id"] for review in motion_reviews],
                "review_paths": [str(path.resolve()) for path in motion_review_paths],
                "review_sha256": [sha256_file(path) for path in motion_review_paths],
                "packet_path": str(motion_packet_path.resolve()),
                "packet_sha256": motion_packet["packet_sha256"],
                "unblind_map_path": str(motion_unblind_map_path.resolve()),
                "unblind_map_sha256": motion_packet["unblind_map_sha256"],
            }

    review_sha256 = [sha256_file(path) for path in review_paths]
    selection["review_sources"] = [str(path.resolve()) for path in review_paths]
    selection["review_evidence"] = {
        "reviewer_ids": [review["reviewer_id"] for review in reviews],
        "review_sha256": review_sha256,
        "metrics_sha256": sha256_file(metric_path),
        "performance_sha256": sha256_file(performance_path),
        "review_packet_path": str(packet_path.resolve()),
        "review_packet_sha256": packet_evidence["packet_sha256"],
        "unblind_map_path": str(unblind_map_path.resolve()),
        "unblind_map_sha256": packet_evidence["unblind_map_sha256"],
        "motion_review_evidence": motion_evidence,
        "scoring_contract_sha256": sha256_object(scoring_contract()),
        "decisions_sha256": _selection_decisions_sha256(selection),
    }
    if motion_revalidation_required:
        suffix = "motion-driven projections require fresh still/motion capture, review, and performance evidence"
    elif complete and motion_evidence is not None:
        suffix = "stress, interaction, and paired-motion gates resolved"
    elif complete:
        suffix = "stress and interaction gates resolved"
    elif stress_complete:
        suffix = "interaction-ladder ratings remain to be reviewed"
    else:
        suffix = "stress finalists remain to be reviewed"
    selection["notes"] = (
        "Deterministically unblinded only after two opaque-code reviews; " + suffix + "."
    )
    validate_selection(selection, complete=complete)
    return {
        "version": 1,
        "warning": WARNING,
        "selection": selection,
        "complete": complete,
        "packet_evidence": packet_evidence,
        "missing_promoted_stress_ratings": [
            list(key) for key in sorted(set(all_missing_stress))
        ],
        "missing_interaction_ladder_ratings": [list(key) for key in missing_ladder],
        "aggregate": list(aggregate.values()),
        "interaction_findings": selection["interaction_findings"],
        "motion_revalidation_required": motion_revalidation_required,
        "rankings": rankings,
    }


def validate_final_review_evidence(
    plan_path: pathlib.Path,
    plan: Mapping[str, Any],
    review_paths: Sequence[pathlib.Path],
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
) -> Dict[str, Any]:
    """Re-derive selection and enforce atomic/combined recommendation margins and stress gates."""

    recomputed_evidence = validate_recomputed_selection_evidence(
        plan_path,
        metric_path,
        performance_path,
    )
    evidence = plan["selection"]["review_evidence"]
    motion_evidence = evidence["motion_review_evidence"]
    expected_motion_clips = len(plan["motion"]["clips"])
    if (
        motion_evidence is None
        or len(plan["selection"]["motion_findings"]) != expected_motion_clips
    ):
        raise HarnessError("final review requires two-reviewer evidence for every planned paired clip")
    derived = derive_selection_from_reviews(
        plan_path,
        review_paths,
        metric_path,
        performance_path,
        pathlib.Path(evidence["review_packet_path"]),
        pathlib.Path(evidence["unblind_map_path"]),
        [pathlib.Path(path) for path in motion_evidence["review_paths"]],
        pathlib.Path(motion_evidence["packet_path"]),
        pathlib.Path(motion_evidence["unblind_map_path"]),
    )
    if derived["selection"] != plan["selection"]:
        raise HarnessError("capture-plan selection differs from deterministic review derivation")
    aggregate = {
        (row["subject_id"], row["condition"], row["scoring_context"]): row
        for row in derived["aggregate"]
    }
    expected_final_keys = {
        ("control", condition, DIAGNOSTIC_REVIEW_CONTEXT)
        for condition in LIGHTING_CONDITIONS
    }
    expected_final_keys.add(("control", "neutral", FINAL_REVIEW_CONTEXT))
    expected_final_keys.update(
        (profile.id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
        for profile in atomic_profiles()
    )
    expected_final_keys.update(
        (profile_id, condition, DIAGNOSTIC_REVIEW_CONTEXT)
        for family in FAMILY_ORDER
        for profile_id in plan["selection"]["stress_diagnostics"][family]
        for condition in ("golden", "overcast")
    )
    expected_final_keys.update(
        (step[0], "neutral", DIAGNOSTIC_REVIEW_CONTEXT) for step in LADDER_STEPS
    )
    expected_final_keys.update(
        (f"winner-{family}", "neutral", FINAL_REVIEW_CONTEXT)
        for family in FAMILY_ORDER
    )
    expected_final_keys.update(
        (f"combination-{combination_id}", condition, DIAGNOSTIC_REVIEW_CONTEXT)
        for combination_id in COMBINATION_IDS
        for condition in LIGHTING_CONDITIONS
    )
    expected_final_keys.update(
        (f"combination-{combination_id}", "neutral", FINAL_REVIEW_CONTEXT)
        for combination_id in COMBINATION_IDS
    )
    if set(aggregate) != expected_final_keys:
        extras = sorted(set(aggregate) - expected_final_keys)
        missing = sorted(expected_final_keys - set(aggregate))
        raise HarnessError(
            f"final review key set changed; missing={missing[:5]} extras={extras[:5]}"
        )
    missing_control = [
        condition
        for condition in LIGHTING_CONDITIONS
        if ("control", condition, DIAGNOSTIC_REVIEW_CONTEXT) not in aggregate
    ]
    if missing_control:
        raise HarnessError(f"final reviews lack control ratings for {missing_control}")
    diagnostic_control = {
        condition: aggregate[("control", condition, DIAGNOSTIC_REVIEW_CONTEXT)]
        for condition in LIGHTING_CONDITIONS
    }
    final_control = aggregate[("control", "neutral", FINAL_REVIEW_CONTEXT)]
    atomic_recommendations = {}
    atomic_final_view_pass = {}
    for family in FAMILY_ORDER:
        winner = plan["selection"]["atomic_winners"][family]
        promoted = plan["selection"]["promoted"][family]
        finalist_rows = []
        for profile_id in promoted:
            neutral = aggregate[(profile_id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)]
            stresses = {
                condition: aggregate[
                    (profile_id, condition, DIAGNOSTIC_REVIEW_CONTEXT)
                ]
                for condition in ("golden", "overcast")
            }
            finalist_rows.append(
                {
                    "profile_id": profile_id,
                    "neutral_score": neutral["weighted_score"],
                    "margin_over_control": (
                        neutral["weighted_score"]
                        - diagnostic_control["neutral"]["weighted_score"]
                    ),
                    "golden_score": stresses["golden"]["weighted_score"],
                    "overcast_score": stresses["overcast"]["weighted_score"],
                    "stress_pass": all(
                        _stress_passes(neutral, stresses[condition])
                        for condition in ("golden", "overcast")
                    ),
                }
            )
        final_view = aggregate[
            (f"winner-{family}", "neutral", FINAL_REVIEW_CONTEXT)
        ]
        final_view_margin = (
            final_view["weighted_score"] - final_control["weighted_score"]
        )
        final_view_pass = (
            winner == "control"
            or (
                final_view["minimum_categories"][
                    "terrain_route_water_edge_readability"
                ]
                >= 3
                and final_view["minimum_categories"]["edge_temporal_quietness"] >= 3
                and final_view_margin >= 2.0
            )
        )
        atomic_final_view_pass[family] = final_view_pass
        final_decision = winner if final_view_pass else "control"
        atomic_recommendations[family] = {
            "candidate": winner,
            "decision": final_decision,
            "recommended_change": final_decision != "control",
            "minimum_margin_over_control": 2.0,
            "final_17_weighted_score": final_view["weighted_score"],
            "final_17_margin_over_control": final_view_margin,
            "final_17_minimum_readability": final_view["minimum_categories"][
                "terrain_route_water_edge_readability"
            ],
            "final_17_minimum_edge_quietness": final_view["minimum_categories"][
                "edge_temporal_quietness"
            ],
            "final_17_pass": final_view_pass,
            "final_17_frames_reviewed": 17,
            "finalists": finalist_rows,
        }
    combination_recommendations = {}
    for combination_id in COMBINATION_IDS:
        subject_id = f"combination-{combination_id}"
        missing = [
            condition
            for condition in LIGHTING_CONDITIONS
            if (subject_id, condition, DIAGNOSTIC_REVIEW_CONTEXT) not in aggregate
        ]
        if (subject_id, "neutral", FINAL_REVIEW_CONTEXT) not in aggregate:
            missing.append(FINAL_REVIEW_CONTEXT)
        if missing:
            raise HarnessError(f"final reviews lack {subject_id} ratings for {missing}")
        diagnostic_neutral = aggregate[
            (subject_id, "neutral", DIAGNOSTIC_REVIEW_CONTEXT)
        ]
        final_neutral = aggregate[(subject_id, "neutral", FINAL_REVIEW_CONTEXT)]
        stresses = {
            condition: aggregate[(subject_id, condition, DIAGNOSTIC_REVIEW_CONTEXT)]
            for condition in ("golden", "overcast")
        }
        stress_pass = all(
            _stress_passes(diagnostic_neutral, stresses[condition])
            for condition in ("golden", "overcast")
        )
        final_tactical_pass = _tactical_floor_passes(final_neutral)
        margin = final_neutral["weighted_score"] - final_control["weighted_score"]
        motion_pass = plan["selection"]["motion_combination_pass"][combination_id]
        atomic_compatible = _combination_atomic_final_17_compatible(
            plan,
            combination_id,
            atomic_final_view_pass,
        )
        recommended = (
            final_tactical_pass
            and stress_pass
            and margin >= 3.0
            and motion_pass is True
            and atomic_compatible
        )
        combination_recommendations[combination_id] = {
            "recommended": recommended,
            "neutral_score": final_neutral["weighted_score"],
            "control_neutral_score": final_control["weighted_score"],
            "diagnostic_neutral_score": diagnostic_neutral["weighted_score"],
            "margin_over_control": margin,
            "minimum_margin": 3.0,
            "golden_score": stresses["golden"]["weighted_score"],
            "overcast_score": stresses["overcast"]["weighted_score"],
            "final_tactical_pass": final_tactical_pass,
            "stress_pass": stress_pass,
            "motion_pass": motion_pass,
            "atomic_final_17_compatible": atomic_compatible,
        }
    return {
        "version": 1,
        "warning": WARNING,
        "reviewer_ids": plan["selection"]["review_evidence"]["reviewer_ids"],
        "atomic": atomic_recommendations,
        "combinations": combination_recommendations,
        "interaction_findings": derived["interaction_findings"],
        "motion_findings": plan["selection"]["motion_findings"],
        "recomputed_evidence": recomputed_evidence,
        "blinded_packet": derived["packet_evidence"],
        "aggregate": derived["aggregate"],
        "rankings": derived["rankings"],
    }


def _combination_atomic_final_17_compatible(
    plan: Mapping[str, Any],
    combination_id: str,
    atomic_final_view_pass: Mapping[str, bool],
) -> bool:
    """Require every active constituent family to retain its final-17 admission."""

    combination = plan["selection"]["combinations"][combination_id]
    return all(
        selected == "control" or atomic_final_view_pass[family]
        for family, selected in combination.items()
    )


def selection_template() -> Dict[str, Any]:
    """Return a blank, non-authoritative adaptive selection hook."""

    empty_family = {family: None for family in FAMILY_ORDER}
    return {
        "version": 1,
        "warning": WARNING,
        "status": "AWAITING_TWO_BLINDED_REVIEWS",
        "promoted": {family: [] for family in FAMILY_ORDER},
        "stress_diagnostics": {family: [] for family in FAMILY_ORDER},
        "ladder_inputs": dict(empty_family),
        "interaction_findings": [],
        "pre_motion_atomic_winners": dict(empty_family),
        "pre_motion_combinations": {
            "restrained": dict(empty_family),
            "expressive": dict(empty_family),
            "score-leader": dict(empty_family),
        },
        "motion_findings": [],
        "motion_combination_pass": {identifier: None for identifier in COMBINATION_IDS},
        "atomic_winners": dict(empty_family),
        "combinations": {
            "restrained": dict(empty_family),
            "expressive": dict(empty_family),
            "score-leader": dict(empty_family),
        },
        "review_sources": [],
        "review_evidence": None,
        "notes": "Populate only from recorded reviews; control means no change.",
    }


def validate_selection(raw: Any, *, complete: bool = False) -> Dict[str, Any]:
    """Validate a partial or complete human/reviewer-supplied selection."""

    value = _strict_object(
        raw,
        context="selection",
        required=(
            "version",
            "warning",
            "status",
            "promoted",
            "stress_diagnostics",
            "ladder_inputs",
            "interaction_findings",
            "pre_motion_atomic_winners",
            "pre_motion_combinations",
            "motion_findings",
            "motion_combination_pass",
            "atomic_winners",
            "combinations",
            "review_sources",
            "review_evidence",
            "notes",
        ),
    )
    if value["version"] != 1 or value["warning"] != WARNING:
        raise HarnessError("selection version or warning changed")
    if not isinstance(value["status"], str) or not value["status"]:
        raise HarnessError("selection.status must be non-empty text")
    if not isinstance(value["notes"], str):
        raise HarnessError("selection.notes must be text")
    lookup = profile_lookup()
    family_ids = {
        family: {profile.id for profile in lookup.values() if profile.family == family}
        for family in FAMILY_ORDER
    }

    promoted = _strict_object(value["promoted"], context="selection.promoted", required=FAMILY_ORDER)
    stress_diagnostics = _strict_object(
        value["stress_diagnostics"],
        context="selection.stress_diagnostics",
        required=FAMILY_ORDER,
    )
    for family in FAMILY_ORDER:
        ids = promoted[family]
        if not isinstance(ids, list) or len(ids) > 2 or len(set(ids)) != len(ids):
            raise HarnessError(f"selection.promoted.{family} must contain zero to two unique ids")
        if any(profile_id not in family_ids[family] for profile_id in ids):
            raise HarnessError(f"selection.promoted.{family} crosses families or names control")
        diagnostic_ids = stress_diagnostics[family]
        if (
            not isinstance(diagnostic_ids, list)
            or len(diagnostic_ids) not in (0, 2)
            or len(set(diagnostic_ids)) != len(diagnostic_ids)
        ):
            raise HarnessError(
                f"selection.stress_diagnostics.{family} must be empty or contain two unique ids"
            )
        if any(profile_id not in family_ids[family] for profile_id in diagnostic_ids):
            raise HarnessError(f"selection.stress_diagnostics.{family} crosses families")
        if diagnostic_ids and not set(ids).issubset(diagnostic_ids):
            raise HarnessError(
                f"selection.stress_diagnostics.{family} omits a promoted candidate"
            )
        if complete and len(diagnostic_ids) != 2:
            raise HarnessError(f"selection.stress_diagnostics.{family} is unresolved")

    ladder_inputs = _strict_object(
        value["ladder_inputs"],
        context="selection.ladder_inputs",
        required=FAMILY_ORDER,
    )
    for family in FAMILY_ORDER:
        selected = ladder_inputs[family]
        if selected is not None and selected != "control" and selected not in family_ids[family]:
            raise HarnessError(f"selection.ladder_inputs.{family} is invalid")
        if selected not in (None, "control") and selected not in promoted[family]:
            raise HarnessError(
                f"selection.ladder_inputs.{family} was not a promoted finalist"
            )
        if complete and selected is None:
            raise HarnessError(f"selection.ladder_inputs.{family} is unresolved")

    findings = value["interaction_findings"]
    if not isinstance(findings, list):
        raise HarnessError("selection.interaction_findings must be a list")
    if findings:
        if len(findings) != len(LADDER_STEPS):
            raise HarnessError("selection.interaction_findings must cover all eight ladder steps")
        for index, (raw_finding, expected_step) in enumerate(zip(findings, LADDER_STEPS)):
            finding = _exact_keys(
                raw_finding,
                (
                    "step_id",
                    "predecessor",
                    "introduced_families",
                    "weighted_score",
                    "predecessor_weighted_score",
                    "weighted_delta",
                    "minimum_readability",
                    "minimum_edge_quietness",
                    "passed",
                    "vetoed_families",
                ),
                f"selection.interaction_findings[{index}]",
            )
            step_id, predecessor, introduced = expected_step
            if (
                finding["step_id"] != step_id
                or finding["predecessor"] != predecessor
                or finding["introduced_families"] != list(introduced)
            ):
                raise HarnessError(f"interaction finding {index} changed ladder topology")
            for field in (
                "weighted_score",
                "predecessor_weighted_score",
                "weighted_delta",
            ):
                _finite_number(finding[field], f"interaction finding {index}.{field}")
            for field in ("minimum_readability", "minimum_edge_quietness"):
                score = finding[field]
                if isinstance(score, bool) or not isinstance(score, int) or not 1 <= score <= 5:
                    raise HarnessError(f"interaction finding {index}.{field} is invalid")
            if not isinstance(finding["passed"], bool):
                raise HarnessError(f"interaction finding {index}.passed must be boolean")
            expected_veto = [] if finding["passed"] else list(introduced)
            if finding["vetoed_families"] != expected_veto:
                raise HarnessError(f"interaction finding {index} veto set changed")
    if complete and len(findings) != len(LADDER_STEPS):
        raise HarnessError("complete selection requires all ladder interaction findings")

    pre_motion_winners = _strict_object(
        value["pre_motion_atomic_winners"],
        context="selection.pre_motion_atomic_winners",
        required=FAMILY_ORDER,
    )
    for family in FAMILY_ORDER:
        selected = pre_motion_winners[family]
        if selected is not None and selected != "control" and selected not in promoted[family]:
            raise HarnessError(f"selection.pre_motion_atomic_winners.{family} is invalid")
        if complete and selected is None:
            raise HarnessError(f"selection.pre_motion_atomic_winners.{family} is unresolved")

    pre_motion_combinations = _strict_object(
        value["pre_motion_combinations"],
        context="selection.pre_motion_combinations",
        required=COMBINATION_IDS,
    )
    for combination_id in COMBINATION_IDS:
        row = _strict_object(
            pre_motion_combinations[combination_id],
            context=f"selection.pre_motion_combinations.{combination_id}",
            required=FAMILY_ORDER,
        )
        for family in FAMILY_ORDER:
            selected = row[family]
            if selected is not None and selected != "control" and selected not in promoted[family]:
                raise HarnessError(
                    f"selection.pre_motion_combinations.{combination_id}.{family} is invalid"
                )
            if complete and selected is None:
                raise HarnessError(
                    f"selection.pre_motion_combinations.{combination_id}.{family} is unresolved"
                )

    motion_findings = value["motion_findings"]
    if not isinstance(motion_findings, list):
        raise HarnessError("selection.motion_findings must be a list")
    seen_motion = set()
    for index, raw_finding in enumerate(motion_findings):
        finding = _exact_keys(
            raw_finding,
            (
                "clip_id",
                "subject_code",
                "minimum_readability",
                "minimum_edge_quietness",
                "passed",
            ),
            f"selection.motion_findings[{index}]",
        )
        if (
            not isinstance(finding["clip_id"], str)
            or not finding["clip_id"]
            or finding["clip_id"] in seen_motion
            or not isinstance(finding["subject_code"], str)
            or BLIND_CODE_RE.fullmatch(finding["subject_code"]) is None
            or not isinstance(finding["passed"], bool)
        ):
            raise HarnessError(f"selection.motion_findings[{index}] is invalid")
        seen_motion.add(finding["clip_id"])
        for field in ("minimum_readability", "minimum_edge_quietness"):
            score = finding[field]
            if isinstance(score, bool) or not isinstance(score, int) or not 1 <= score <= 5:
                raise HarnessError(f"selection.motion_findings[{index}].{field} is invalid")
    if len(motion_findings) > 22:
        raise HarnessError("motion findings exceed the maximum 22 paired clips")

    motion_combination_pass = _strict_object(
        value["motion_combination_pass"],
        context="selection.motion_combination_pass",
        required=COMBINATION_IDS,
    )
    for combination_id, passed in motion_combination_pass.items():
        if passed is not None and not isinstance(passed, bool):
            raise HarnessError(
                f"selection.motion_combination_pass.{combination_id} must be null or boolean"
            )
    if motion_findings and any(value is None for value in motion_combination_pass.values()):
        raise HarnessError("complete motion findings require all combination motion decisions")

    winners = _strict_object(value["atomic_winners"], context="selection.atomic_winners", required=FAMILY_ORDER)
    for family in FAMILY_ORDER:
        winner = winners[family]
        if winner is not None and winner != "control" and winner not in family_ids[family]:
            raise HarnessError(f"selection.atomic_winners.{family} is invalid")
        if winner not in (None, "control") and winner not in promoted[family]:
            raise HarnessError(f"selection.atomic_winners.{family} was not a promoted finalist")
        if complete and winner is None:
            raise HarnessError(f"selection.atomic_winners.{family} is unresolved")

    combinations = _strict_object(value["combinations"], context="selection.combinations", required=COMBINATION_IDS)
    for combination_id in COMBINATION_IDS:
        combination = _strict_object(
            combinations[combination_id],
            context=f"selection.combinations.{combination_id}",
            required=FAMILY_ORDER,
        )
        for family in FAMILY_ORDER:
            selected = combination[family]
            if selected is not None and selected != "control" and selected not in family_ids[family]:
                raise HarnessError(f"selection.combinations.{combination_id}.{family} is invalid")
            if selected not in (None, "control") and selected not in promoted[family]:
                raise HarnessError(
                    f"selection.combinations.{combination_id}.{family} was not a promoted finalist"
                )
            if complete and selected is None:
                raise HarnessError(f"selection.combinations.{combination_id}.{family} is unresolved")
    if all(winners[family] is not None for family in FAMILY_ORDER):
        leader = combinations["score-leader"]
        if all(leader[family] is not None for family in FAMILY_ORDER) and leader != winners:
            raise HarnessError("score-leader must contain the supplied atomic winner or control for every family")
    if complete:
        for family in FAMILY_ORDER:
            if len(promoted[family]) == 2:
                continue
            forced_control_values = [
                ladder_inputs[family],
                pre_motion_winners[family],
                winners[family],
                *(pre_motion_combinations[identifier][family] for identifier in COMBINATION_IDS),
                *(combinations[identifier][family] for identifier in COMBINATION_IDS),
            ]
            if any(selected != "control" for selected in forced_control_values):
                raise HarnessError(
                    f"selection.{family} must resolve to control when fewer than two candidates pass"
                )
    if not isinstance(value["review_sources"], list) or not all(isinstance(item, str) and item for item in value["review_sources"]):
        raise HarnessError("selection.review_sources must be a list of paths/identities")
    if complete and (len(value["review_sources"]) != 2 or len(set(value["review_sources"])) != 2):
        raise HarnessError("complete selection requires two independent blinded review sources")
    evidence = value["review_evidence"]
    if evidence is not None:
        evidence = _exact_keys(
            evidence,
            (
                "reviewer_ids",
                "review_sha256",
                "metrics_sha256",
                "performance_sha256",
                "review_packet_path",
                "review_packet_sha256",
                "unblind_map_path",
                "unblind_map_sha256",
                "motion_review_evidence",
                "scoring_contract_sha256",
                "decisions_sha256",
            ),
            "selection.review_evidence",
        )
        if (
            not isinstance(evidence["reviewer_ids"], list)
            or len(evidence["reviewer_ids"]) != 2
            or len(set(evidence["reviewer_ids"])) != 2
            or any(not isinstance(item, str) or not SAFE_ID_RE.fullmatch(item) for item in evidence["reviewer_ids"])
        ):
            raise HarnessError("selection review evidence needs two distinct reviewer ids")
        if (
            not isinstance(evidence["review_sha256"], list)
            or len(evidence["review_sha256"]) != 2
            or any(not isinstance(item, str) or not SHA256_RE.fullmatch(item) for item in evidence["review_sha256"])
        ):
            raise HarnessError("selection review evidence needs two review hashes")
        for field in (
            "metrics_sha256",
            "performance_sha256",
            "review_packet_sha256",
            "unblind_map_sha256",
            "scoring_contract_sha256",
            "decisions_sha256",
        ):
            if not isinstance(evidence[field], str) or not SHA256_RE.fullmatch(evidence[field]):
                raise HarnessError(f"selection review evidence {field} is invalid")
        for field in ("review_packet_path", "unblind_map_path"):
            if not isinstance(evidence[field], str) or not evidence[field]:
                raise HarnessError(f"selection review evidence {field} is invalid")
        motion_evidence = evidence["motion_review_evidence"]
        if motion_evidence is not None:
            motion_evidence = _exact_keys(
                motion_evidence,
                (
                    "reviewer_ids",
                    "review_paths",
                    "review_sha256",
                    "packet_path",
                    "packet_sha256",
                    "unblind_map_path",
                    "unblind_map_sha256",
                ),
                "selection.review_evidence.motion_review_evidence",
            )
            if motion_evidence["reviewer_ids"] != evidence["reviewer_ids"]:
                raise HarnessError("motion review must use the same two independent reviewers")
            if (
                not isinstance(motion_evidence["review_paths"], list)
                or len(motion_evidence["review_paths"]) != 2
                or any(
                    not isinstance(item, str) or not item
                    for item in motion_evidence["review_paths"]
                )
                or not isinstance(motion_evidence["review_sha256"], list)
                or len(motion_evidence["review_sha256"]) != 2
            ):
                raise HarnessError("motion review evidence must bind two review files")
            for field in ("packet_sha256", "unblind_map_sha256"):
                if (
                    not isinstance(motion_evidence[field], str)
                    or SHA256_RE.fullmatch(motion_evidence[field]) is None
                ):
                    raise HarnessError(f"motion review evidence {field} is invalid")
            if any(
                not isinstance(item, str) or SHA256_RE.fullmatch(item) is None
                for item in motion_evidence["review_sha256"]
            ):
                raise HarnessError("motion review hashes are invalid")
            for field in ("packet_path", "unblind_map_path"):
                if not isinstance(motion_evidence[field], str) or not motion_evidence[field]:
                    raise HarnessError(f"motion review evidence {field} is invalid")
        if evidence["scoring_contract_sha256"] != sha256_object(scoring_contract()):
            raise HarnessError("selection scoring contract hash changed")
        if evidence["decisions_sha256"] != _selection_decisions_sha256(value):
            raise HarnessError("selection decisions differ from their evidence hash")
    if complete and evidence is None:
        raise HarnessError("complete selection requires hash-linked review evidence")
    return value


def _combined_profile(profile_id: str, label: str, selected: Mapping[str, str]) -> DetailProfile:
    lookup = profile_lookup()
    body: Dict[str, Any] = {"version": PROFILE_VERSION}
    body.update(_current_sections())
    for family in FAMILY_ORDER:
        selected_id = selected.get(family, "control")
        if selected_id != "control":
            profile = lookup.get(selected_id)
            if profile is None or profile.family != family:
                raise HarnessError(f"combined profile {profile_id} has invalid {family} selection")
            body[family] = copy.deepcopy(profile.body[family])
    encoded = compact_json(body)
    return DetailProfile(profile_id, label, "combination", body, encoded, sha256_bytes(encoded.encode()))


def _interaction_profiles(selected: Mapping[str, str]) -> List[DetailProfile]:
    """Build the fixed nine-profile/36-slot neutral interaction ladder."""

    def chosen(*families: str) -> Dict[str, str]:
        return {family: selected.get(family) or "control" for family in families}

    return [
        control_profile(),
        _combined_profile("ladder-snow", "Snow", chosen("snow")),
        _combined_profile(
            "ladder-snow-vegetation-cliff",
            "Snow + vegetation + cliff",
            chosen("snow", "alpine_vegetation", "cliff_strata"),
        ),
        _combined_profile(
            "ladder-snow-vegetation-cliff-props",
            "Snow + vegetation + cliff + props",
            chosen("snow", "alpine_vegetation", "cliff_strata", "terrain_props"),
        ),
        _combined_profile("ladder-water", "Water", chosen("water")),
        _combined_profile(
            "ladder-water-shore",
            "Water + shore/falls",
            chosen("water", "shore_and_falls"),
        ),
        _combined_profile(
            "ladder-water-shore-ice",
            "Water + shore/falls + ice",
            chosen("water", "shore_and_falls", "ice_fringe"),
        ),
        _combined_profile(
            "ladder-clouds",
            "Clouds",
            chosen("physical_clouds"),
        ),
        _combined_profile(
            "ladder-clouds-fog",
            "Clouds + local fog",
            chosen("physical_clouds", "local_fog"),
        ),
    ]


def _selection_progress(selection: Mapping[str, Any]) -> Dict[str, bool]:
    # Every family retains two explicit stress diagnostics for fixed gallery
    # accounting even when only zero or one is genuinely promoted.
    promotions = all(
        len(selection["stress_diagnostics"][family]) == 2 for family in FAMILY_ORDER
    )
    ladder_inputs = all(
        selection["ladder_inputs"][family] is not None for family in FAMILY_ORDER
    )
    winners = all(selection["atomic_winners"][family] is not None for family in FAMILY_ORDER)
    combinations = all(
        selection["combinations"][combination_id][family] is not None
        for combination_id in COMBINATION_IDS
        for family in FAMILY_ORDER
    )
    return {
        "promotions": promotions,
        "ladder_inputs": ladder_inputs,
        "winners": winners,
        "combinations": combinations,
    }


def _slot(
    stage: str,
    look_id: str,
    profile: DetailProfile,
    lighting: LightingCondition,
    camera: CameraSpec,
) -> Dict[str, Any]:
    return {
        "stage": stage,
        "look_id": look_id,
        "profile_id": profile.id,
        "profile_family": profile.family,
        "profile_sha256": profile.sha256,
        "profile_json": profile.canonical_json,
        "lighting": lighting.id,
        "camera_id": camera.id,
        "semantic_key": sha256_object(
            {
                "profile_sha256": profile.sha256,
                "asset_stage": lighting.asset_stage,
                "time_hours": lighting.time_hours,
                "camera": dataclasses.asdict(camera),
                "liquid_phase_seconds": 0.0,
            }
        ),
    }


def _validate_still_png_ceilings(
    unique_non_control_treatment_pngs: int,
    total_accounted_evidence_pngs: int,
) -> None:
    """Keep the treatment ceiling independent from mandatory control/evidence PNGs."""

    if (
        isinstance(unique_non_control_treatment_pngs, bool)
        or not isinstance(unique_non_control_treatment_pngs, int)
        or unique_non_control_treatment_pngs < 0
    ):
        raise HarnessError("unique non-control treatment-PNG accounting is invalid")
    if (
        isinstance(total_accounted_evidence_pngs, bool)
        or not isinstance(total_accounted_evidence_pngs, int)
        or total_accounted_evidence_pngs < unique_non_control_treatment_pngs
    ):
        raise HarnessError("total accounted evidence-PNG accounting is invalid")
    if unique_non_control_treatment_pngs > MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS:
        raise HarnessError(
            "selected evidence exceeds the "
            f"{MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS} unique non-control treatment-PNG "
            "ceiling: "
            f"{unique_non_control_treatment_pngs} treatment PNGs are required"
        )
    if total_accounted_evidence_pngs > MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS:
        raise HarnessError(
            "selected evidence exceeds the "
            f"{MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS} total accounted evidence-PNG ceiling: "
            f"{total_accounted_evidence_pngs} PNGs are required"
        )


def build_still_plan(
    output_root: pathlib.Path,
    selection_raw: Optional[Any] = None,
    *,
    raw_capture_root: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Build the resolved portion of the adaptive 665-slot still plan."""

    output_root = _require_outside_source_tree(output_root, "output root")
    raw_capture_root = _validate_raw_capture_root(output_root, raw_capture_root)
    final_cameras, focused_cameras, camera_provenance = load_camera_sets()
    profiles = atomic_profiles()
    lookup = profile_lookup()
    selection = validate_selection(selection_raw or selection_template())
    progress = _selection_progress(selection)
    slots: List[Dict[str, Any]] = []
    reference_pairs = {("neutral", camera.id) for camera in focused_cameras}
    if progress["promotions"]:
        reference_pairs.update(
            (lighting_id, camera.id)
            for lighting_id in ("golden", "overcast")
            for camera in focused_cameras
        )
    if progress["winners"] and progress["combinations"]:
        reference_pairs.update(("neutral", camera.id) for camera in final_cameras)
    final_by_id = {camera.id: camera for camera in final_cameras}
    reference_slots = [
        _slot(
            "00-shared-control",
            "control",
            control_profile(),
            LIGHTING_CONDITIONS[lighting_id],
            final_by_id[camera_id],
        )
        for lighting_id, camera_id in sorted(reference_pairs)
    ]

    for profile in profiles:
        for camera in focused_cameras:
            slots.append(_slot("01-neutral-screen", profile.id, profile, LIGHTING_CONDITIONS["neutral"], camera))

    if progress["promotions"]:
        for family in FAMILY_ORDER:
            for profile_id in selection["stress_diagnostics"][family]:
                for lighting_id in ("golden", "overcast"):
                    for camera in focused_cameras:
                        slots.append(_slot("03-stress-finalists", profile_id, lookup[profile_id], LIGHTING_CONDITIONS[lighting_id], camera))

    interaction_profiles: List[DetailProfile] = []
    if progress["ladder_inputs"]:
        interaction_profiles = _interaction_profiles(selection["ladder_inputs"])
        for profile in interaction_profiles:
            for camera in focused_cameras:
                slots.append(_slot("04-interaction-ladders", profile.id, profile, LIGHTING_CONDITIONS["neutral"], camera))

    combination_profiles: Dict[str, DetailProfile] = {}
    if progress["combinations"]:
        for combination_id in COMBINATION_IDS:
            combination_profiles[combination_id] = _combined_profile(
                f"combination-{combination_id}",
                combination_id.replace("-", " ").title(),
                selection["combinations"][combination_id],
            )

    if progress["winners"] and progress["combinations"]:
        finalists: List[DetailProfile] = [control_profile()]
        for family in FAMILY_ORDER:
            winner_id = selection["atomic_winners"][family]
            if winner_id == "control":
                finalists.append(_combined_profile(f"winner-{family}", f"{family} — no change", {}))
            else:
                winner = lookup[winner_id]
                finalists.append(_combined_profile(f"winner-{family}", winner.label, {family: winner_id}))
        finalists.extend(combination_profiles[combination_id] for combination_id in COMBINATION_IDS)
        if len(finalists) != 13:
            raise HarnessError("final review must contain control, nine atomic decisions, and three combinations")
        for profile in finalists:
            for camera in final_cameras:
                slots.append(_slot("07-final-17", profile.id, profile, LIGHTING_CONDITIONS["neutral"], camera))
        for profile in combination_profiles.values():
            for lighting_id in ("golden", "overcast"):
                for camera in focused_cameras:
                    slots.append(_slot("08-combination-stress", profile.id, profile, LIGHTING_CONDITIONS[lighting_id], camera))

    expected_by_stage = {
        "01-neutral-screen": 240,
        "03-stress-finalists": 144,
        "04-interaction-ladders": 36,
        "07-final-17": 221,
        "08-combination-stress": 24,
    }
    expected_complete_logical_slots = EXPECTED_LOGICAL_SLOTS
    stage_counts = {stage: sum(slot["stage"] == stage for slot in slots) for stage in expected_by_stage}
    unresolved = []
    if not progress["promotions"]:
        unresolved.append("review-derived non-failing promotion set per family")
    if not progress["ladder_inputs"]:
        unresolved.append("stress-passing provisional inputs for the three interaction ladders")
    if not progress["winners"]:
        unresolved.append("ladder-gated atomic winner or control/no-change per family")
    if not progress["combinations"]:
        unresolved.append("restrained, expressive, and score-leader family selections")

    # Every control is rendered by this plan from the same source provenance as
    # its candidates. Logical comparisons alias those fresh pixels by exact
    # profile, lighting, phase, and camera semantics.
    artifact_by_semantic: Dict[str, str] = {}
    unique_slots: List[Dict[str, Any]] = []
    logical_slots: List[Dict[str, Any]] = []
    for source_slot in reference_slots:
        slot = dict(source_slot)
        artifact = (
            pathlib.Path("runtime")
            / "raw-stills"
            / "shared-controls"
            / slot["lighting"]
            / f"{slot['camera_id']}.png"
        ).as_posix()
        artifact_by_semantic[slot["semantic_key"]] = artifact
        slot["artifact"] = artifact
        slot["reuse"] = False
        slot["fresh_control"] = True
        unique_slots.append(slot)
    for index, source_slot in enumerate(slots, start=1):
        slot = dict(source_slot)
        slot["logical_slot"] = index
        key = slot["semantic_key"]
        if key in artifact_by_semantic:
            slot["artifact"] = artifact_by_semantic[key]
            slot["reuse"] = True
            slot["fresh_control"] = slot["profile_sha256"] == CONTROL_PROFILE_SHA256
            logical_slots.append(slot)
            continue
        if slot["profile_sha256"] == CONTROL_PROFILE_SHA256:
            artifact = (
                pathlib.Path("runtime")
                / "raw-stills"
                / "shared-controls"
                / slot["lighting"]
                / f"{slot['camera_id']}.png"
            ).as_posix()
        else:
            artifact = (
                pathlib.Path("runtime")
                / "raw-stills"
                / slot["stage"]
                / slot["lighting"]
                / f"{slot['look_id']}-{slot['profile_sha256'][:12]}"
                / f"{slot['camera_id']}.png"
            ).as_posix()
        artifact_by_semantic[key] = artifact
        slot["artifact"] = artifact
        slot["reuse"] = False
        slot["fresh_control"] = False
        unique_slots.append(slot)
        logical_slots.append(slot)

    cameras_by_id = {camera.id: camera for camera in final_cameras}
    focused_camera_ids = {camera.id for camera in focused_cameras}
    grouped: MutableMapping[
        Tuple[str, str, str, str, str], List[Dict[str, Any]]
    ] = defaultdict(list)
    for slot in unique_slots:
        # Each neutral focused control mirrors the clean oracle's
        # fresh-process-per-PNG recipe. These four one-camera jobs exist in stage
        # one and remain byte-stable when final review adds thirteen cameras.
        control_bucket = ""
        if slot["stage"] == "00-shared-control":
            if (
                slot["lighting"] == "neutral"
                and slot["camera_id"] in focused_camera_ids
            ):
                control_bucket = f"focused-{slot['camera_id']}"
            else:
                control_bucket = (
                    "focused"
                    if slot["camera_id"] in focused_camera_ids
                    else "final-extra"
                )
        grouped[
            (
                slot["stage"],
                slot["look_id"],
                slot["profile_sha256"],
                slot["lighting"],
                control_bucket,
            )
        ].append(slot)
    jobs = []
    for (
        stage,
        look_id,
        profile_sha,
        lighting_id,
        control_bucket,
    ), job_slots in grouped.items():
        profile_json = job_slots[0]["profile_json"]
        lighting = LIGHTING_CONDITIONS[lighting_id]
        bucket_suffix = f"-{control_bucket}" if control_bucket else ""
        entries = []
        for slot in job_slots:
            camera = cameras_by_id[slot["camera_id"]]
            entries.append(
                camera.runtime_entry(_raw_artifact_path(raw_capture_root, slot["artifact"]))
            )
        capture_plan = {"version": 1, "captures": entries}
        jobs.append(
            {
                "id": (
                    f"still-{stage}-{look_id}-{lighting_id}"
                    f"{bucket_suffix}-{profile_sha[:12]}"
                ),
                "kind": "still",
                "stage": stage,
                "look_id": look_id,
                "profile_sha256": profile_sha,
                "profile_json": profile_json,
                "control_profile_omitted": profile_sha == CONTROL_PROFILE_SHA256,
                "lighting": lighting_id,
                "asset_stage": lighting.asset_stage,
                "time_hours": lighting.time_hours,
                "liquid_phase_seconds": 0.0,
                "capture_plan": capture_plan,
                "raw_capture_root": str(raw_capture_root),
                "cameras": [dataclasses.asdict(cameras_by_id[slot["camera_id"]]) for slot in job_slots],
                "artifacts": [slot["artifact"] for slot in job_slots],
            }
        )
    # Fail fast against the clean-source oracle even when a fully resolved plan
    # is captured from scratch. The sort is stable for every non-oracle job.
    jobs.sort(
        key=lambda job: 0
        if (
            job["stage"] == "00-shared-control"
            and job["lighting"] == "neutral"
            and "-focused-" in job["id"]
        )
        else 1
    )

    # The omitted-profile side is the four fresh neutral shared-control jobs.
    # Give every explicit-current mate its own process as well, preserving four
    # additional PNGs while matching the oracle's process isolation.
    artifacts = [
        (
            pathlib.Path("runtime")
            / "raw-stills"
            / "control-verification"
            / "explicit-current"
            / camera.filename
        ).as_posix()
        for camera in focused_cameras
    ]
    verification_jobs = [
            {
                "id": f"control-verification-explicit-current-{camera.id}",
                "kind": "control-verification",
                "stage": "00-control-verification",
                "look_id": "control-explicit-current",
                "profile_sha256": control_profile().sha256,
                "profile_json": control_profile().canonical_json,
                "control_profile_omitted": False,
                "lighting": "neutral",
                "asset_stage": LIGHTING_CONDITIONS["neutral"].asset_stage,
                "time_hours": LIGHTING_CONDITIONS["neutral"].time_hours,
                "liquid_phase_seconds": 0.0,
                "capture_plan": {
                    "version": 1,
                    "captures": [
                        camera.runtime_entry(
                            _raw_artifact_path(raw_capture_root, artifact)
                        )
                    ],
                },
                "raw_capture_root": str(raw_capture_root),
                "cameras": [dataclasses.asdict(camera)],
                "artifacts": [artifact],
            }
            for camera, artifact in zip(focused_cameras, artifacts)
    ]

    complete = not unresolved
    reproduction_jobs = []
    if complete:
        reproduction_camera = cameras_by_id["02-highlands-oblique"]
        reproduction_profile = combination_profiles["score-leader"]
        if reproduction_profile.is_control:
            explicit_matches = [
                job
                for job in verification_jobs
                if job["cameras"][0]["id"] == reproduction_camera.id
            ]
            if len(explicit_matches) != 1:
                raise HarnessError(
                    "control score-leader cannot resolve one camera-02 explicit-current source"
                )
            explicit_job = explicit_matches[0]
            reference_artifact = explicit_job["artifacts"][0]
            reference_job_id = explicit_job["id"]
        else:
            reference_slots_for_reproduction = [
                slot
                for slot in logical_slots
                if slot["stage"] == "07-final-17"
                and slot["look_id"] == "combination-score-leader"
                and slot["lighting"] == "neutral"
                and slot["camera_id"] == reproduction_camera.id
            ]
            if len(reference_slots_for_reproduction) != 1:
                raise HarnessError("score-leader reproduction cannot resolve its final-17 source")
            reference_artifact = reference_slots_for_reproduction[0]["artifact"]
            matching_jobs = [job for job in jobs if reference_artifact in job["artifacts"]]
            if len(matching_jobs) != 1:
                raise HarnessError("score-leader reproduction source job is ambiguous")
            reference_job_id = matching_jobs[0]["id"]
        reproduction_artifact = (
            pathlib.Path("runtime")
            / "raw-stills"
            / "reproduction"
            / f"score-leader-{reproduction_profile.sha256[:12]}-neutral-02.png"
        ).as_posix()
        reproduction_jobs.append(
            {
                "id": (
                    "reproduction-score-leader-neutral-02-"
                    f"{reproduction_profile.sha256[:12]}"
                ),
                "kind": "reproduction",
                "stage": "09-deterministic-reproduction",
                "look_id": "combination-score-leader",
                "profile_sha256": reproduction_profile.sha256,
                "profile_json": reproduction_profile.canonical_json,
                # A control fallback is explicit-current so the rerun itself
                # still exercises strict profile parsing.
                "control_profile_omitted": False,
                "lighting": "neutral",
                "asset_stage": LIGHTING_CONDITIONS["neutral"].asset_stage,
                "time_hours": LIGHTING_CONDITIONS["neutral"].time_hours,
                "liquid_phase_seconds": 0.0,
                "capture_plan": {
                    "version": 1,
                    "captures": [
                        reproduction_camera.runtime_entry(
                            _raw_artifact_path(raw_capture_root, reproduction_artifact)
                        )
                    ],
                },
                "raw_capture_root": str(raw_capture_root),
                "cameras": [dataclasses.asdict(reproduction_camera)],
                "artifacts": [reproduction_artifact],
                "reference_job_id": reference_job_id,
                "reference_artifact": reference_artifact,
                "reference_camera_id": reproduction_camera.id,
            }
        )

    reference_artifacts = {
        artifact_by_semantic[slot["semantic_key"]]
        for slot in reference_slots
    }
    unique_non_control_treatment_pngs = len(unique_slots) - len(reference_artifacts)
    new_unique_study_renders = len(unique_slots)
    new_unique_still_renders = new_unique_study_renders + sum(
        len(job["artifacts"])
        for job in (*verification_jobs, *reproduction_jobs)
    )
    total_accounted_evidence_pngs = (
        new_unique_still_renders + EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS
    )
    if complete:
        if len(slots) != expected_complete_logical_slots or stage_counts != expected_by_stage:
            raise HarnessError(f"complete gallery slot accounting changed: {stage_counts}")
        if len(reference_artifacts) != EXPECTED_SHARED_CONTROL_RENDERS:
            raise HarnessError("complete plan must render exactly 25 fresh shared controls")
        _validate_still_png_ceilings(
            unique_non_control_treatment_pngs,
            total_accounted_evidence_pngs,
        )
    return {
        "version": 1,
        "warning": WARNING,
        "status": "READY_FOR_CAPTURE" if complete else "ADAPTIVE_STAGES_UNRESOLVED",
        "raw_capture_root": str(raw_capture_root),
        "scenario": SCENARIO,
        "seed": SEED,
        "baseline": {
            "generator": "grand-v3",
            "radius": 187,
            "level_height": 0.35,
            "palette": "high-separation",
            "lighting": "balanced noon",
            "time_hours": 12.0,
            "haze_density": 0.0003,
            "normals": "current-hard",
            "materials": "current",
            "tactical_fog": "current",
            "crystal_lighting": "current",
            "liquid_effect_phase": 0.0,
            "resolution": [CAPTURE_WIDTH, CAPTURE_HEIGHT],
            "midnight_included": False,
        },
        "lighting_conditions": {key: dataclasses.asdict(value) for key, value in LIGHTING_CONDITIONS.items()},
        "camera_provenance": camera_provenance,
        "camera_sets": {
            "focused-4": [camera.id for camera in focused_cameras],
            "final-17": [camera.id for camera in final_cameras],
            "unscored-leak-checks": list(UNSCORED_CAMERA_IDS),
        },
        "camera_specs": {
            "focused-4": [dataclasses.asdict(camera) for camera in focused_cameras],
            "final-17": [dataclasses.asdict(camera) for camera in final_cameras],
        },
        "profile_matrix": [
            {
                "id": profile.id,
                "family": profile.family,
                "label": profile.label,
                "canonical_json": profile.canonical_json,
                "sha256": profile.sha256,
            }
            for profile in profiles
        ],
        "adaptive_progress": progress,
        "unresolved": unresolved,
        "slot_accounting": {
            "expected_complete_logical_slots": expected_complete_logical_slots,
            "expected_by_stage": expected_by_stage,
            "resolved_logical_slots": len(slots),
            "resolved_by_stage": stage_counts,
            "materialized_unique_paths": len(unique_slots),
            "unique_non_control_treatment_pngs": unique_non_control_treatment_pngs,
            "genuine_shared_control_references": len(reference_artifacts),
            "fresh_shared_control_renders": len(reference_artifacts),
            "new_unique_study_renders": new_unique_study_renders,
            "new_unique_control_verification_renders": EXPECTED_CONTROL_VERIFICATION_RENDERS,
            "new_unique_reproduction_renders": len(reproduction_jobs),
            "new_unique_still_renders": new_unique_still_renders,
            "baseline_oracle_primary_renders": EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS,
            "baseline_oracle_stability_diagnostic_renders": (
                EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
            ),
            "total_accounted_evidence_pngs": total_accounted_evidence_pngs,
            "unique_non_control_treatment_png_ceiling": (
                MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS
            ),
            "total_accounted_evidence_png_ceiling": MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS,
            "accounting_notice": (
                "All 25 shared controls are fresh omitted-profile bc06 renders from current source "
                "provenance and are gated against the clean bc06 source oracle. The four neutral "
                "focused controls are four fresh one-camera processes and also serve as the omitted-profile "
                "verification side; four one-camera explicit-current process mates, one deterministic "
                "reproduction, and four primary clean-source oracle PNGs are additional. The 611-PNG "
                "ceiling applies only to unique non-control treatment renders (596 in the maximally "
                "distinct valid outcome); the separate total accounted evidence ceiling is 630 PNGs. "
                "No blinded outcome is rejected or altered to manufacture reuse. The 22 clean-source "
                "repeats are pre-study raster-stability "
                "qualification evidence and are recorded separately, not gallery slots."
            ),
            "shared_control_policy": (
                "render all neutral, golden, and overcast controls from current source provenance; "
                "reuse each exact control semantic state across every logical comparison"
            ),
        },
        "shared_control_references": [
            {
                "camera_id": slot["camera_id"],
                "lighting": slot["lighting"],
                "semantic_key": slot["semantic_key"],
                "artifact": artifact_by_semantic[slot["semantic_key"]],
                "source_kind": "fresh-runtime-control",
            }
            for slot in reference_slots
        ],
        "logical_slots": logical_slots,
        "jobs": jobs,
        "verification_jobs": verification_jobs,
        "reproduction_jobs": reproduction_jobs,
    }


def rotate_camera(camera: CameraSpec, angle_degrees: float, frame_index: int) -> CameraSpec:
    """Rotate one map camera's world-space offset around its look-at anchor."""

    if camera.camera != "map" or camera.look_at_anchor is None or camera.look_at_offset is None:
        raise HarnessError(f"motion camera {camera.id} is not an anchored map camera")
    angle = math.radians(_finite_number(angle_degrees, "orbit angle"))
    x_value, y_value, z_value = camera.look_at_offset
    rotated = (
        x_value * math.cos(angle) - z_value * math.sin(angle),
        y_value,
        x_value * math.sin(angle) + z_value * math.cos(angle),
    )
    motion_id = f"{camera.id}-orbit-{frame_index + 1:04d}"
    return dataclasses.replace(
        camera,
        id=motion_id,
        filename=f"{motion_id}.png",
        look_at_offset=rotated,
    )


def _selected_atomic_profile(family: str, profile_id: str) -> DetailProfile:
    if profile_id == "control":
        return control_profile()
    profile = profile_lookup().get(profile_id)
    if profile is None or profile.family != family:
        raise HarnessError(f"invalid selected {family} profile {profile_id!r}")
    return profile


def build_motion_plan(
    output_root: pathlib.Path,
    selection_raw: Optional[Any] = None,
    *,
    raw_capture_root: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Build the genuine 22-clip, 20-degree deterministic motion capture plan."""

    output_root = _require_outside_source_tree(output_root, "output root")
    raw_capture_root = _validate_raw_capture_root(output_root, raw_capture_root)
    selection = validate_selection(selection_raw or selection_template())
    progress = _selection_progress(selection)
    if not all(progress.values()):
        return {
            "version": 1,
            "warning": WARNING,
            "status": "ADAPTIVE_STAGES_UNRESOLVED",
            "raw_capture_root": str(raw_capture_root),
            "expected_clips": 22,
            "resolved_clips": 0,
            "orbit_degrees": ORBIT_DEGREES,
            "fps": FPS,
            "frames_per_clip": MOTION_FRAME_COUNT,
            "paired_comparison_clips": True,
            "shared_control_orbits": 0,
            "candidate_sequence_launches": 0,
            "total_sequence_launches": 0,
            "total_frame_captures": 0,
            "clips": [],
            "jobs": [],
        }
    validate_selection(selection, complete=True)
    final_cameras, _, _ = load_camera_sets()
    cameras = {camera.id: camera for camera in final_cameras}
    lookup = profile_lookup()
    motion_winners = selection["pre_motion_atomic_winners"]
    motion_combinations = selection["pre_motion_combinations"]
    combination_profiles = {
        combination_id: _combined_profile(
            f"combination-{combination_id}",
            combination_id.replace("-", " ").title(),
            motion_combinations[combination_id],
        )
        for combination_id in COMBINATION_IDS
    }
    clip_specs: List[Tuple[str, str, DetailProfile, str, str]] = []

    for family in RISKY_MOTION_FAMILIES:
        for diagnostic_index, profile_id in enumerate(
            selection["stress_diagnostics"][family], start=1
        ):
            clip_specs.append(
                (
                    f"diagnostic-{family.replace('_', '-')}-{diagnostic_index}-{profile_id}",
                    f"{family} stress diagnostic {diagnostic_index}",
                    lookup[profile_id],
                    "neutral",
                    PRIMARY_CAMERAS[family],
                )
            )
    for family in WINNER_MOTION_FAMILIES:
        profile_id = motion_winners[family]
        clip_specs.append(
            (
                f"winner-{family.replace('_', '-')}-{profile_id}",
                f"{family} atomic decision",
                _selected_atomic_profile(family, profile_id),
                "neutral",
                PRIMARY_CAMERAS[family],
            )
        )
    for combination_id in COMBINATION_IDS:
        for camera_id in ("02-highlands-oblique", "14-cascade-basin-full-height"):
            clip_specs.append(
                (
                    f"combination-{combination_id}-{camera_id[:2]}",
                    f"{combination_id} combination at {camera_id}",
                    combination_profiles[combination_id],
                    "neutral",
                    camera_id,
                )
            )
    clip_specs.extend(
        (
            (
                "leader-golden-03",
                "score leader, golden hour",
                combination_profiles["score-leader"],
                "golden",
                "03-coast-river-outlet",
            ),
            (
                "leader-overcast-02",
                "score leader, overcast",
                combination_profiles["score-leader"],
                "overcast",
                "02-highlands-oblique",
            ),
        )
    )
    expected_clips = 22
    if len(clip_specs) != expected_clips or len({row[0] for row in clip_specs}) != expected_clips:
        raise HarnessError("motion matrix does not match the non-failing finalist set")

    clips = []
    jobs = []
    control_jobs_by_condition: Dict[Tuple[str, str], str] = {}
    control_frames_by_condition: Dict[Tuple[str, str], List[str]] = {}
    control = control_profile()
    for lighting_id, camera_id in sorted({(row[3], row[4]) for row in clip_specs}):
        lighting = LIGHTING_CONDITIONS[lighting_id]
        base_camera = cameras[camera_id]
        entries = []
        camera_records = []
        artifacts = []
        for frame_index in range(MOTION_FRAME_COUNT):
            fraction = frame_index / (MOTION_FRAME_COUNT - 1)
            angle = -ORBIT_DEGREES / 2.0 + ORBIT_DEGREES * fraction
            phase = frame_index / FPS
            camera = rotate_camera(base_camera, angle, frame_index)
            relative_path = (
                pathlib.Path("runtime")
                / "raw-motion-controls"
                / lighting_id
                / camera_id
                / "frames"
                / f"frame-{frame_index + 1:04d}.png"
            )
            entry = camera.runtime_entry(
                _raw_artifact_path(raw_capture_root, relative_path.as_posix())
            )
            entry["liquid_phase_seconds"] = phase
            entry["settle_frames"] = 90 if frame_index == 0 else 2
            entries.append(entry)
            camera_records.append(dataclasses.asdict(camera))
            artifacts.append(relative_path.as_posix())
        job_id = f"motion-control-{lighting_id}-{camera_id}"
        jobs.append(
            {
                "id": job_id,
                "kind": "motion-control-sequence",
                "clip_id": None,
                "profile_sha256": control.sha256,
                "profile_json": control.canonical_json,
                "control_profile_omitted": True,
                "lighting": lighting_id,
                "asset_stage": lighting.asset_stage,
                "time_hours": lighting.time_hours,
                "liquid_phase_seconds": None,
                "capture_plan": {"version": 2, "captures": entries},
                "raw_capture_root": str(raw_capture_root),
                "cameras": camera_records,
                "artifacts": artifacts,
            }
        )
        control_jobs_by_condition[(lighting_id, camera_id)] = job_id
        control_frames_by_condition[(lighting_id, camera_id)] = artifacts

    for clip_index, (clip_id, label, profile, lighting_id, camera_id) in enumerate(clip_specs, start=1):
        lighting = LIGHTING_CONDITIONS[lighting_id]
        base_camera = cameras[camera_id]
        profile_token = profile.sha256[:12]
        entries = []
        camera_records = []
        frame_paths = []
        angles = []
        for frame_index in range(MOTION_FRAME_COUNT):
            fraction = frame_index / (MOTION_FRAME_COUNT - 1)
            angle = -ORBIT_DEGREES / 2.0 + ORBIT_DEGREES * fraction
            phase = frame_index / FPS
            camera = rotate_camera(base_camera, angle, frame_index)
            relative_path = (
                pathlib.Path("runtime")
                / "raw-motion"
                / f"{clip_id}-{profile_token}"
                / "frames"
                / f"frame-{frame_index + 1:04d}.png"
            )
            entry = camera.runtime_entry(
                _raw_artifact_path(raw_capture_root, relative_path.as_posix())
            )
            entry["liquid_phase_seconds"] = phase
            entry["settle_frames"] = 90 if frame_index == 0 else 2
            entries.append(entry)
            camera_records.append(dataclasses.asdict(camera))
            frame_paths.append(relative_path.as_posix())
            angles.append(angle)
        job_id = f"motion-{clip_index:02d}-{clip_id}-{profile_token}"
        jobs.append(
            {
                "id": job_id,
                "kind": "motion-candidate-sequence",
                "clip_id": clip_id,
                "profile_sha256": profile.sha256,
                "profile_json": profile.canonical_json,
                "control_profile_omitted": profile.is_control,
                "lighting": lighting_id,
                "asset_stage": lighting.asset_stage,
                "time_hours": lighting.time_hours,
                "liquid_phase_seconds": None,
                "capture_plan": {"version": 2, "captures": entries},
                "raw_capture_root": str(raw_capture_root),
                "cameras": camera_records,
                "artifacts": frame_paths,
            }
        )
        clips.append(
            {
                "id": clip_id,
                "label": label,
                "profile_id": profile.id,
                "profile_sha256": profile.sha256,
                "lighting": lighting_id,
                "camera_id": camera_id,
                "fps": FPS,
                "frame_count": MOTION_FRAME_COUNT,
                "orbit_start_degrees": angles[0],
                "orbit_end_degrees": angles[-1],
                "liquid_phase_start_seconds": 0.0,
                "liquid_phase_end_seconds": (MOTION_FRAME_COUNT - 1) / FPS,
                "candidate_raw_frames": frame_paths,
                "control_raw_frames": control_frames_by_condition[(lighting_id, camera_id)],
                "candidate_job_ids": [job_id],
                "control_job_ids": [control_jobs_by_condition[(lighting_id, camera_id)]],
                "labeled_frame_directory": (
                    pathlib.Path("motion-frames") / f"{clip_id}-{profile_token}" / "frames"
                ).as_posix(),
                "paired_frame_directory": (
                    pathlib.Path("motion-paired-frames")
                    / f"{clip_id}-{profile_token}"
                    / "frames"
                ).as_posix(),
                "mp4": (
                    pathlib.Path("motion-clips") / f"{clip_id}-{profile_token}.mp4"
                ).as_posix(),
                "eight_frame_strip": (
                    pathlib.Path("motion-strips") / f"{clip_id}-{profile_token}.png"
                ).as_posix(),
            }
        )
    control_orbits = len(control_jobs_by_condition)
    if len(jobs) != expected_clips + control_orbits:
        raise HarnessError("paired motion sequence launch accounting changed")
    return {
        "version": 1,
        "warning": WARNING,
        "status": "READY_FOR_CAPTURE",
        "raw_capture_root": str(raw_capture_root),
        "expected_clips": expected_clips,
        "resolved_clips": len(clips),
        "orbit_degrees": ORBIT_DEGREES,
        "fps": FPS,
        "frames_per_clip": MOTION_FRAME_COUNT,
        "paired_comparison_clips": True,
        "shared_control_orbits": control_orbits,
        "candidate_sequence_launches": len(clips),
        "total_sequence_launches": len(jobs),
        "total_frame_captures": len(jobs) * MOTION_FRAME_COUNT,
        "animated_phases_only_for_motion": True,
        "clips": clips,
        "jobs": jobs,
    }


def _legacy_harness() -> Any:
    """Load the established asset-staging and PNG validators from this checkout."""

    tools = str(TOOLS_ROOT)
    if tools not in sys.path:
        sys.path.insert(0, tools)
    try:
        import visual_experiments  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError(f"cannot import established visual experiment helpers: {error}") from error
    return visual_experiments


def _baseline_oracle_documents(
    root: pathlib.Path = BASELINE_ORACLE_ROOT,
) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """Load the source-controlled oracle contract and its private manifest."""

    if (
        not BASELINE_ORACLE_CONTRACT_PATH.is_file()
        or BASELINE_ORACLE_CONTRACT_PATH.is_symlink()
    ):
        raise HarnessError("source-controlled baseline-oracle contract is unavailable")
    contract = _exact_keys(
        _read_json(BASELINE_ORACLE_CONTRACT_PATH, "baseline-oracle contract"),
        (
            "version",
            "warning",
            "pack_id",
            "external_manifest_filename",
            "external_manifest_sha256",
            "inventory_manifest_sha256",
            "stability_evidence_sha256",
            "source_git_head",
            "source_git_tree",
            "camera_ids",
            "primary_render_count",
            "stability_diagnostic_render_count",
        ),
        "baseline-oracle contract",
    )
    if (
        contract["version"] != 1
        or contract["warning"] != WARNING
        or contract["external_manifest_filename"] != BASELINE_ORACLE_MANIFEST_FILENAME
        or contract["camera_ids"] != list(BASELINE_ORACLE_CAMERA_IDS)
        or contract["primary_render_count"] != EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS
        or contract["stability_diagnostic_render_count"]
        != EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
    ):
        raise HarnessError("baseline-oracle source contract identity changed")
    for field in (
        "external_manifest_sha256",
        "inventory_manifest_sha256",
        "stability_evidence_sha256",
    ):
        if not isinstance(contract[field], str) or SHA256_RE.fullmatch(contract[field]) is None:
            raise HarnessError(f"baseline-oracle contract has invalid {field}")
    for field in ("source_git_head", "source_git_tree"):
        if (
            not isinstance(contract[field], str)
            or re.fullmatch(r"[0-9a-f]{40}", contract[field]) is None
        ):
            raise HarnessError(f"baseline-oracle contract has invalid {field}")

    if root.is_symlink():
        raise HarnessError(f"private baseline-oracle root may not be a symlink: {root}")
    root = root.resolve()
    if not root.is_dir():
        raise HarnessError(f"private baseline-oracle root is unavailable: {root}")
    manifest_path = root / BASELINE_ORACLE_MANIFEST_FILENAME
    if (
        not manifest_path.is_file()
        or manifest_path.is_symlink()
        or sha256_file(manifest_path) != contract["external_manifest_sha256"]
    ):
        raise HarnessError("private baseline-oracle manifest is unavailable or changed")
    manifest = _exact_keys(
        _read_json(manifest_path, "private baseline-oracle manifest"),
        (
            "version",
            "warning",
            "pack_id",
            "source",
            "recipe",
            "asset_stage",
            "producer",
            "cameras",
            "inventory",
            "stability_contract",
        ),
        "private baseline-oracle manifest",
    )
    if (
        manifest["version"] != 1
        or manifest["warning"] != WARNING
        or manifest["pack_id"] != contract["pack_id"]
    ):
        raise HarnessError("private baseline-oracle manifest identity changed")
    source = _exact_keys(
        manifest.get("source"),
        (
            "git_head",
            "git_tree",
            "git_commit_subject",
            "git_commit_timestamp",
            "worktree_clean",
            "worktree_status_porcelain_bytes",
            "untracked_file_count",
            "workspace_content_sha256",
            "workspace_content_hash_convention",
            "source_asset_tree_sha256",
            "asset_tree_hash_convention",
            "cargo_lock_sha256",
            "map_review_source_sha256",
            "camera_runtime_source_sha256",
            "grand_v3_world_sha256",
            "source_palette_sha256",
            "source_lighting_sha256",
            "scenario_catalog_sha256",
            "high_separation_candidate_sha256",
            "camera_config_sha256",
            "camera_walk_sha256",
            "camera_profile_manifest_sha256",
        ),
        "baseline-oracle source",
    )
    recipe = _exact_keys(
        manifest.get("recipe"),
        (
            "scenario",
            "seed",
            "radius",
            "level_height",
            "palette",
            "lighting",
            "time_hours",
            "haze_density",
            "liquid_phase_seconds",
            "resolution",
            "settle_frames",
            "control_profile",
            "structural_draft_environment",
            "camera_ids",
            "camera_manifest_provenance",
            "capture_process_policy",
        ),
        "baseline-oracle recipe",
    )
    inventory = manifest.get("inventory")
    stability = manifest.get("stability_contract")
    cameras = manifest.get("cameras")
    if not all(isinstance(value, dict) for value in (inventory, stability, cameras)):
        raise HarnessError("private baseline-oracle manifest sections are malformed")
    if (
        source.get("git_head") != contract["source_git_head"]
        or source.get("git_tree") != contract["source_git_tree"]
        or source.get("worktree_clean") is not True
        or source.get("worktree_status_porcelain_bytes") != 0
        or source.get("untracked_file_count") != 0
        or recipe.get("scenario") != SCENARIO
        or recipe.get("seed") != SEED
        or recipe.get("radius") != 187
        or recipe.get("level_height") != 0.35
        or recipe.get("time_hours") != 12.0
        or recipe.get("haze_density") != 0.0003
        or recipe.get("liquid_phase_seconds") != 0.0
        or recipe.get("settle_frames") != 90
        or recipe.get("camera_ids") != list(BASELINE_ORACLE_CAMERA_IDS)
        or recipe.get("resolution")
        != {"width": CAPTURE_WIDTH, "height": CAPTURE_HEIGHT}
        or set(cameras) != set(BASELINE_ORACLE_CAMERA_IDS)
        or inventory.get("manifest_excluded_path") != BASELINE_ORACLE_MANIFEST_FILENAME
        or inventory.get("manifest_sha256") != contract["inventory_manifest_sha256"]
        or stability.get("stability_evidence_sha256")
        != contract["stability_evidence_sha256"]
        or stability.get("broad_numeric_thresholds_allowed") is not False
        or stability.get("modified_worktree_samples_allowed") is not False
        or stability.get("raw_png_byte_identity_satisfiable") is not False
    ):
        raise HarnessError("private baseline-oracle source or recipe contract changed")
    for field, value in source.items():
        if field.endswith("_sha256") and (
            not isinstance(value, str) or SHA256_RE.fullmatch(value) is None
        ):
            raise HarnessError(f"private baseline-oracle source has invalid {field}")
    if (
        source["camera_config_sha256"]
        != ESTABLISHED_CAMERA_SOURCE_SHA256["assets/config/camera.ron"]
        or source["camera_walk_sha256"]
        != ESTABLISHED_CAMERA_SOURCE_SHA256["walks/camera_grand_v3_baseline.ron"]
        or source["camera_profile_manifest_sha256"]
        != ESTABLISHED_CAMERA_SOURCE_SHA256["tools/visual_experiments/profiles.json"]
        or source["source_asset_tree_sha256"]
        != manifest["asset_stage"].get("source_asset_tree_sha256")
    ):
        raise HarnessError("private baseline-oracle source hashes changed")
    return contract, manifest


def _baseline_oracle_provenance_binding() -> Dict[str, Any]:
    """Return the compact oracle identity hash-bound into every capture plan."""

    contract, manifest = _baseline_oracle_documents()
    producer_files = manifest["producer"].get("files")
    executable = (
        producer_files.get("producer/hex_game-bc06-map-review")
        if isinstance(producer_files, dict)
        else None
    )
    if not isinstance(executable, dict):
        raise HarnessError("baseline-oracle producer executable receipt is malformed")
    return {
        "version": 1,
        "pack_id": contract["pack_id"],
        "contract_path": BASELINE_ORACLE_CONTRACT_PATH.relative_to(
            REPOSITORY_ROOT
        ).as_posix(),
        "contract_sha256": sha256_file(BASELINE_ORACLE_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": contract["inventory_manifest_sha256"],
        "stability_evidence_sha256": contract["stability_evidence_sha256"],
        "source_git_head": contract["source_git_head"],
        "source_git_tree": contract["source_git_tree"],
        "recipe_sha256": sha256_object(manifest["recipe"]),
        "source_sha256": sha256_object(manifest["source"]),
        "asset_stage_sha256": sha256_object(manifest["asset_stage"]),
        "producer_sha256": sha256_object(manifest["producer"]),
        "producer_executable_sha256": executable.get("sha256"),
        "camera_evidence_sha256": manifest["stability_contract"].get(
            "camera_evidence_sha256"
        ),
        "primary_render_count": contract["primary_render_count"],
        "stability_diagnostic_render_count": contract[
            "stability_diagnostic_render_count"
        ],
        "private_pack_files_are_not_published": True,
    }


def _control_equivalence_documents(
    root: pathlib.Path = CONTROL_EQUIVALENCE_QUALIFICATION_ROOT,
) -> Tuple[Dict[str, Any], Dict[str, Any]]:
    """Load the narrow source contract and external six-run qualification pack."""

    if (
        not CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH.is_file()
        or CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH.is_symlink()
    ):
        raise HarnessError("source-controlled control-equivalence raster contract is unavailable")
    contract = _exact_keys(
        _read_json(
            CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH,
            "control-equivalence raster contract",
        ),
        (
            "version",
            "warning",
            "pack_id",
            "scope",
            "baseline_oracle_binding",
            "external_manifest_filename",
            "external_manifest_sha256",
            "inventory_manifest_sha256",
            "qualification_evidence_sha256",
            "qualification_evidence_hash_convention",
            "source_provenance_sha256",
            "executable_sha256",
            "asset_stage",
            "raster_acceptance",
            "camera_id",
            "pair_count",
            "run_count",
            "qualified_pixels",
            "stable_capture_sha256",
            "stable_report_sha256",
        ),
        "control-equivalence raster contract",
    )
    expected_qualified = [dict(row) for row in CONTROL_EQUIVALENCE_QUALIFIED_PIXELS]
    if (
        contract["version"] != 1
        or contract["warning"] != WARNING
        or contract["scope"]
        != "omitted-versus-explicit-current-control-raster-equivalence-only"
        or contract["external_manifest_filename"]
        != CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME
        or contract["camera_id"] != "14-cascade-basin-full-height"
        or contract["pair_count"] != 3
        or contract["run_count"] != 6
        or contract["qualified_pixels"] != expected_qualified
    ):
        raise HarnessError("control-equivalence raster contract identity changed")
    for field in (
        "external_manifest_sha256",
        "inventory_manifest_sha256",
        "qualification_evidence_sha256",
        "source_provenance_sha256",
        "executable_sha256",
        "stable_capture_sha256",
        "stable_report_sha256",
    ):
        if not isinstance(contract[field], str) or SHA256_RE.fullmatch(contract[field]) is None:
            raise HarnessError(f"control-equivalence raster contract has invalid {field}")
    evidence_hash_convention = (
        "SHA-256 of UTF-8 canonical compact JSON with lexicographically sorted object "
        "keys for {baseline_oracle_binding,source,asset_stage,recipe,"
        "stable_report_contract,raster_acceptance,qualified_pixels,pairs}"
    )
    if contract["qualification_evidence_hash_convention"] != evidence_hash_convention:
        raise HarnessError("control-equivalence qualification hash convention changed")
    contract_stage = _exact_keys(
        contract["asset_stage"],
        (
            "id",
            "external_manifest_sha256",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
        ),
        "control-equivalence raster contract asset stage",
    )
    if contract_stage["id"] != "balanced-noon-haze-0003" or any(
        not isinstance(contract_stage[field], str)
        or SHA256_RE.fullmatch(contract_stage[field]) is None
        for field in (
            "external_manifest_sha256",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
        )
    ):
        raise HarnessError("control-equivalence raster contract asset stage changed")
    raster_acceptance = _exact_keys(
        contract["raster_acceptance"],
        (
            "coordinate_convention",
            "decoded_rgb_hash_convention",
            "rule",
            "broad_numeric_thresholds_allowed",
            "spatial_expansion_allowed",
            "baseline_oracle_contract_modified",
        ),
        "control-equivalence raster acceptance",
    )
    if (
        raster_acceptance["coordinate_convention"]
        != "zero-based (x,y) from the top-left decoded pixel"
        or raster_acceptance["decoded_rgb_hash_convention"]
        != 'SHA-256 of exactly width*height*3 row-major bytes after Pillow convert("RGB"), without dimensions or padding'
        or raster_acceptance["rule"]
        != "Every decoded RGB pixel outside the union of the camera's clean baseline-oracle ambiguity coordinates and the separately qualified coordinates must match exactly; every differing pixel inside the union must use its exact enumerated endpoint RGB values."
        or raster_acceptance["broad_numeric_thresholds_allowed"] is not False
        or raster_acceptance["spatial_expansion_allowed"] is not False
        or raster_acceptance["baseline_oracle_contract_modified"] is not False
    ):
        raise HarnessError("control-equivalence raster acceptance changed")
    baseline_binding = _exact_keys(
        contract["baseline_oracle_binding"],
        (
            "version",
            "pack_id",
            "contract_sha256",
            "manifest_sha256",
            "inventory_manifest_sha256",
            "stability_evidence_sha256",
            "camera_id",
            "camera_evidence_sha256",
            "ambiguous_pixels",
        ),
        "control-equivalence baseline-oracle binding",
    )
    baseline_contract, baseline_manifest = _baseline_oracle_documents()
    baseline_camera = baseline_manifest["cameras"][contract["camera_id"]]
    if baseline_binding != {
        "version": 1,
        "pack_id": baseline_contract["pack_id"],
        "contract_sha256": sha256_file(BASELINE_ORACLE_CONTRACT_PATH),
        "manifest_sha256": baseline_contract["external_manifest_sha256"],
        "inventory_manifest_sha256": baseline_contract["inventory_manifest_sha256"],
        "stability_evidence_sha256": baseline_contract["stability_evidence_sha256"],
        "camera_id": contract["camera_id"],
        "camera_evidence_sha256": baseline_camera["clean_run_evidence_sha256"],
        "ambiguous_pixels": baseline_camera["ambiguous_pixels"],
    }:
        raise HarnessError("control-equivalence baseline-oracle binding changed")

    if root.is_symlink():
        raise HarnessError(f"control-equivalence qualification root may not be a symlink: {root}")
    root = root.resolve()
    if not root.is_dir():
        raise HarnessError(f"control-equivalence qualification root is unavailable: {root}")
    manifest_path = root / CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME
    if (
        not manifest_path.is_file()
        or manifest_path.is_symlink()
        or sha256_file(manifest_path) != contract["external_manifest_sha256"]
    ):
        raise HarnessError("control-equivalence qualification manifest is unavailable or changed")
    manifest = _exact_keys(
        _read_json(manifest_path, "control-equivalence qualification manifest"),
        (
            "version",
            "warning",
            "pack_id",
            "scope",
            "baseline_oracle_binding",
            "source",
            "asset_stage",
            "recipe",
            "stable_report_contract",
            "raster_acceptance",
            "qualified_pixels",
            "pairs",
            "qualification_evidence_hash_convention",
            "missing_evidence",
            "inventory",
        ),
        "control-equivalence qualification manifest",
    )
    if (
        manifest["version"] != 1
        or manifest["warning"] != WARNING
        or manifest["pack_id"] != contract["pack_id"]
        or manifest["scope"] != contract["scope"]
    ):
        raise HarnessError("control-equivalence qualification manifest identity changed")
    source = _exact_keys(
        manifest["source"],
        (
            "git_head",
            "workspace_content_sha256",
            "worktree_status_sha256",
            "tracked_dirty_diff_sha256",
            "untracked_manifest_sha256",
            "source_provenance_sha256",
            "harness_sha256",
            "executable_sha256",
        ),
        "control-equivalence qualification source",
    )
    stage = _exact_keys(
        manifest["asset_stage"],
        (
            "id",
            "external_manifest_path",
            "external_manifest_sha256",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
        ),
        "control-equivalence qualification asset stage",
    )
    recipe = _exact_keys(
        manifest["recipe"],
        (
            "scenario",
            "seed",
            "radius",
            "level_height",
            "palette",
            "lighting",
            "time_hours",
            "haze_density",
            "liquid_phase_seconds",
            "resolution",
            "settle_frames",
            "camera",
            "control_profile_sha256",
            "profile_modes",
            "pair_count",
            "fresh_process_count",
            "fresh_process_per_png",
            "structural_draft_environment",
        ),
        "control-equivalence qualification recipe",
    )
    stable = _exact_keys(
        manifest["stable_report_contract"],
        (
            "capture_fields",
            "report_fields",
            "excluded_capture_fields",
            "excluded_report_fields",
            "capture_sha256",
            "report_sha256",
            "all_six_capture_states_equal",
            "all_six_stable_reports_equal",
            "hash_convention",
        ),
        "control-equivalence qualification stable-report contract",
    )
    normalized_qualified = []
    if not isinstance(manifest["qualified_pixels"], list):
        raise HarnessError("control-equivalence qualified pixels must be an array")
    for index, raw in enumerate(manifest["qualified_pixels"]):
        row = _exact_keys(
            raw,
            (
                "camera_id",
                "x",
                "y",
                "allowed_rgb",
                "clean_source_baseline_oracle_coordinate",
                "observations",
            ),
            f"control-equivalence qualified pixel {index}",
        )
        normalized_qualified.append(
            {field: row[field] for field in ("camera_id", "x", "y", "allowed_rgb")}
        )
        if row["clean_source_baseline_oracle_coordinate"] is not False:
            raise HarnessError("control-equivalence pixel was incorrectly attributed to clean source")
    evidence_payload = {
        field: manifest[field]
        for field in (
            "baseline_oracle_binding",
            "source",
            "asset_stage",
            "recipe",
            "stable_report_contract",
            "raster_acceptance",
            "qualified_pixels",
            "pairs",
        )
    }
    if (
        source["source_provenance_sha256"] != contract["source_provenance_sha256"]
        or source["executable_sha256"] != contract["executable_sha256"]
        or {field: stage[field] for field in contract_stage} != contract_stage
        or recipe["scenario"] != SCENARIO
        or recipe["seed"] != SEED
        or recipe["radius"] != 187
        or recipe["level_height"] != 0.35
        or recipe["time_hours"] != 12.0
        or recipe["haze_density"] != 0.0003
        or recipe["liquid_phase_seconds"] != 0.0
        or recipe["resolution"] != {"width": CAPTURE_WIDTH, "height": CAPTURE_HEIGHT}
        or recipe["settle_frames"] != 90
        or recipe["pair_count"] != contract["pair_count"]
        or recipe["fresh_process_count"] != contract["run_count"]
        or recipe["fresh_process_per_png"] is not True
        or normalized_qualified != expected_qualified
        or manifest["baseline_oracle_binding"] != contract["baseline_oracle_binding"]
        or manifest["qualification_evidence_hash_convention"]
        != contract["qualification_evidence_hash_convention"]
        or manifest["raster_acceptance"] != contract["raster_acceptance"]
        or not isinstance(manifest["missing_evidence"], list)
        or len(manifest["missing_evidence"]) != 2
        or stable["capture_sha256"] != contract["stable_capture_sha256"]
        or stable["report_sha256"] != contract["stable_report_sha256"]
        or stable["all_six_capture_states_equal"] is not True
        or stable["all_six_stable_reports_equal"] is not True
        or stable["hash_convention"]
        != "SHA-256 of UTF-8 canonical compact JSON with lexicographically sorted object keys for exactly the named fields"
        or manifest["inventory"].get("manifest_sha256")
        != contract["inventory_manifest_sha256"]
        or sha256_object(evidence_payload) != contract["qualification_evidence_sha256"]
    ):
        raise HarnessError("control-equivalence qualification evidence binding changed")
    return contract, manifest


def _control_equivalence_provenance_binding() -> Dict[str, Any]:
    """Return the narrow qualification identity embedded in every new plan."""

    contract, manifest = _control_equivalence_documents()
    return {
        "version": 1,
        "pack_id": contract["pack_id"],
        "scope": contract["scope"],
        "contract_path": CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH.relative_to(
            REPOSITORY_ROOT
        ).as_posix(),
        "contract_sha256": sha256_file(CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": contract["inventory_manifest_sha256"],
        "qualification_evidence_sha256": contract["qualification_evidence_sha256"],
        "qualification_evidence_hash_convention": contract[
            "qualification_evidence_hash_convention"
        ],
        "baseline_oracle_binding": contract["baseline_oracle_binding"],
        "qualified_pixels": contract["qualified_pixels"],
        "source_provenance_sha256": contract["source_provenance_sha256"],
        "producer_executable_sha256": contract["executable_sha256"],
        "asset_stage": contract["asset_stage"],
        "raster_acceptance": contract["raster_acceptance"],
        "pair_count": contract["pair_count"],
        "run_count": contract["run_count"],
        "stable_capture_sha256": contract["stable_capture_sha256"],
        "stable_report_sha256": contract["stable_report_sha256"],
        "missing_evidence": manifest["missing_evidence"],
        "private_pack_files_are_not_published": True,
        "baseline_oracle_contract_is_unchanged": True,
    }


def _source_asset_paths() -> Tuple[pathlib.Path, ...]:
    return (
        REPOSITORY_ROOT / "assets/config/worlds/procedural-grand-v3-baseline.ron",
        REPOSITORY_ROOT / "assets/art/palette.ron",
        REPOSITORY_ROOT / "assets/config/lighting.ron",
        REPOSITORY_ROOT / "assets/config/lighting/overcast.ron",
        REPOSITORY_ROOT / "assets/config/scenarios.ron",
        REPOSITORY_ROOT / "assets/config/camera.ron",
        REPOSITORY_ROOT / "tools/visual_experiments/palettes/p02-high-separation.json",
        REPOSITORY_ROOT / "tools/visual_experiments/lighting/z01-haze-light.json",
        REPOSITORY_ROOT / "tools/visual_experiments/profiles.json",
        REPOSITORY_ROOT / "walks/camera_grand_v3_baseline.ron",
        FINAL_CAMERAS_PATH,
        FOCUSED_CAMERAS_PATH,
        CAMERA_PROVENANCE_PATH,
        PROFILE_HASHES_PATH,
        REVIEW_SCHEMA_PATH,
        RUNTIME_EVIDENCE_CONTRACT_PATH,
        BASELINE_ORACLE_CONTRACT_PATH,
        CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH,
        pathlib.Path(__file__).resolve(),
    )


def _untracked_file_hashes(legacy: Any) -> Dict[str, str]:
    """Record each untracked source identity, rather than only a dirty boolean."""

    listed = legacy.git_output(
        REPOSITORY_ROOT,
        "ls-files",
        "-z",
        "--others",
        "--exclude-standard",
    )
    identities = {}
    for raw in sorted(item for item in listed.split(b"\0") if item):
        relative = raw.decode("utf-8", errors="strict")
        candidate = REPOSITORY_ROOT / relative
        try:
            candidate.resolve(strict=False).relative_to(REPOSITORY_ROOT.resolve())
        except ValueError as error:
            raise HarnessError(f"untracked path escaped the source worktree: {relative}") from error
        if candidate.is_symlink():
            payload = b"SYMLINK\0" + os.readlink(candidate).encode("utf-8")
            identity = sha256_bytes(payload)
        elif candidate.is_file():
            identity = sha256_file(candidate)
        else:
            raise HarnessError(f"untracked source path is not a regular file or symlink: {relative}")
        identities[relative] = identity
    return identities


def source_provenance() -> Dict[str, Any]:
    """Hash the exact source, dirty tree, harness, and camera inputs."""

    legacy = _legacy_harness()
    baseline_oracle = _baseline_oracle_provenance_binding()
    control_equivalence_raster = _control_equivalence_provenance_binding()
    workspace = legacy.workspace_provenance(REPOSITORY_ROOT)
    worktree_status = legacy.git_output(
        REPOSITORY_ROOT,
        "status",
        "--porcelain=v1",
        "-z",
        "--",
        ".",
        ":(exclude).context/grand-v3-visual-experiments/**",
    )
    tracked_diff = legacy.git_output(
        REPOSITORY_ROOT,
        "diff",
        "--binary",
        "--no-ext-diff",
        "HEAD",
        "--",
        ".",
        ":(exclude).context/grand-v3-visual-experiments/**",
    )
    untracked_file_sha256 = _untracked_file_hashes(legacy)
    hashes = {
        path.relative_to(REPOSITORY_ROOT).as_posix(): sha256_file(path)
        for path in _source_asset_paths()
    }
    return {
        "git_head": workspace["git_head"],
        "worktree_dirty": workspace["worktree_dirty"],
        "worktree_status_sha256": sha256_bytes(worktree_status),
        "tracked_dirty_diff_sha256": sha256_bytes(tracked_diff),
        "untracked_file_sha256": untracked_file_sha256,
        "untracked_manifest_sha256": sha256_object(untracked_file_sha256),
        "workspace_content_sha256": workspace["workspace_content_sha256"],
        "source_hashes": hashes,
        "camera_manifest_sha256": {
            "final-17": hashes[FINAL_CAMERAS_PATH.relative_to(REPOSITORY_ROOT).as_posix()],
            "focused-4": hashes[FOCUSED_CAMERAS_PATH.relative_to(REPOSITORY_ROOT).as_posix()],
        },
        "baseline_oracle": baseline_oracle,
        "control_equivalence_raster": control_equivalence_raster,
        "structural_draft_runtime": {
            "environment": STRUCTURAL_DRAFT_ENVIRONMENT,
            "value": STRUCTURAL_DRAFT_VALUE,
            "scope": "map-review-only",
        },
    }


def _assert_source_provenance(expected: Mapping[str, Any]) -> None:
    current = source_provenance()
    if current != expected:
        raise HarnessError("source, dirty-tree, camera, or harness provenance changed after plan creation")


def _stage_asset_root(
    work_root: pathlib.Path,
    condition: LightingCondition,
) -> Tuple[pathlib.Path, Dict[str, Any]]:
    """Create or validate one immutable baseline/stress asset stage."""

    legacy = _legacy_harness()
    stages = work_root / "asset-stages"
    stage_root = stages / condition.asset_stage / "asset-root"
    manifest_path = stages / condition.asset_stage / "stage-manifest.json"
    if stage_root.is_symlink() or manifest_path.is_symlink():
        raise HarnessError("asset stages may not be symlinks")
    if stage_root.exists() or manifest_path.exists():
        if not stage_root.is_dir() or not manifest_path.is_file():
            raise HarnessError(f"partial asset stage exists: {stage_root}")
        manifest = _exact_keys(
            _read_json(manifest_path, "asset-stage manifest"),
            (
                "version",
                "warning",
                "asset_stage",
                "lighting_condition",
                "source_asset_tree_sha256",
                "staged_asset_tree_sha256",
                "modified_assets",
            ),
            "asset-stage manifest",
        )
        if (
            manifest["version"] != 1
            or manifest["warning"] != WARNING
            or manifest["asset_stage"] != condition.asset_stage
            or manifest["lighting_condition"] != dataclasses.asdict(condition)
        ):
            raise HarnessError(f"asset stage manifest changed: {manifest_path}")
        current_source_digest = legacy.tree_digest(REPOSITORY_ROOT / "assets")
        if manifest["source_asset_tree_sha256"] != current_source_digest:
            raise HarnessError(
                f"asset stage {condition.id} was built from a different source asset tree"
            )
        if legacy.tree_digest(stage_root / "assets") != manifest.get("staged_asset_tree_sha256"):
            raise HarnessError(f"asset stage content changed: {stage_root}")
        allowed_modified = {
            "config/worlds/procedural-grand-v3-baseline.ron",
            "art/palette.ron",
            (
                "config/lighting.ron"
                if condition.static_lighting is None
                else "config/scenarios.ron"
            ),
        }
        source_hashes = legacy.tree_file_hashes(REPOSITORY_ROOT / "assets")
        staged_hashes = legacy.tree_file_hashes(stage_root / "assets")
        expected_modified = {
            path for path in source_hashes if source_hashes[path] != staged_hashes[path]
        }
        if not expected_modified <= allowed_modified:
            raise HarnessError(
                f"asset stage {condition.id} changed files outside its allowed set"
            )
        modified_rows = manifest["modified_assets"]
        if not isinstance(modified_rows, list):
            raise HarnessError(f"asset stage {condition.id} modified-file contract is malformed")
        observed_modified = set()
        for index, raw_row in enumerate(modified_rows):
            row = _exact_keys(
                raw_row,
                ("path", "source_sha256", "staged_sha256"),
                f"asset stage {condition.id} modified asset {index}",
            )
            if not isinstance(row["path"], str) or not row["path"].startswith("assets/"):
                raise HarnessError(f"asset stage {condition.id} has an unsafe modified path")
            relative = row["path"][len("assets/") :]
            if relative in observed_modified or relative not in source_hashes or relative not in staged_hashes:
                raise HarnessError(f"asset stage {condition.id} repeats or invents a modified asset")
            if (
                row["source_sha256"] != source_hashes[relative]
                or row["staged_sha256"] != staged_hashes[relative]
                or row["source_sha256"] == row["staged_sha256"]
            ):
                raise HarnessError(f"asset stage {condition.id} modified-asset hashes are stale")
            observed_modified.add(relative)
        if observed_modified != expected_modified:
            raise HarnessError(
                f"asset stage {condition.id} modified-file contract changed: {observed_modified}"
            )
        if set(source_hashes) != set(staged_hashes):
            raise HarnessError(f"asset stage {condition.id} changed the asset file set")
        return stage_root, manifest

    stage_root.parent.mkdir(parents=True, exist_ok=False)
    source_asset_digest = legacy.tree_digest(REPOSITORY_ROOT / "assets")
    legacy.copy_asset_tree(REPOSITORY_ROOT / "assets", stage_root / "assets")
    source_hashes = legacy.tree_file_hashes(REPOSITORY_ROOT / "assets")
    modified: List[str] = []

    world_relative = "config/worlds/procedural-grand-v3-baseline.ron"
    world_path = stage_root / "assets" / world_relative
    world_source = world_path.read_text(encoding="utf-8")
    world_staged = legacy.replace_level_height(world_source, 0.35)
    world_path.write_text(world_staged, encoding="utf-8")
    if world_source != world_staged:
        modified.append(world_relative)

    palette_relative = "art/palette.ron"
    palette_path = stage_root / "assets" / palette_relative
    palette_id, colors = legacy.load_palette_candidate(
        REPOSITORY_ROOT / "tools/visual_experiments/palettes/p02-high-separation.json",
        legacy.parse_palette_colors(palette_path.read_text(encoding="utf-8")),
    )
    if palette_id != "p02-high-separation":
        raise HarnessError("high-separation palette identity changed")
    palette_path.write_text(
        legacy.replace_palette_colors(palette_path.read_text(encoding="utf-8"), colors),
        encoding="utf-8",
    )
    modified.append(palette_relative)

    if condition.static_lighting is None:
        if condition.time_hours is None or condition.haze_density != 0.0003:
            raise HarnessError(f"cyclic condition {condition.id} lost its exact time/haze")
        candidate_id, overrides = legacy.load_lighting_candidate(
            REPOSITORY_ROOT / "tools/visual_experiments/lighting/z01-haze-light.json"
        )
        if candidate_id != "z01-haze-light" or overrides.get("fog_density") != 0.0003:
            raise HarnessError("haze candidate identity or density changed")
        lighting_relative = "config/lighting.ron"
        lighting_path = stage_root / "assets" / lighting_relative
        lighting_path.write_text(
            legacy.patch_cycle_noon_lighting(
                lighting_path.read_text(encoding="utf-8"), overrides
            ),
            encoding="utf-8",
        )
        modified.append(lighting_relative)
    else:
        scenario_relative = "config/scenarios.ron"
        scenario_path = stage_root / "assets" / scenario_relative
        lighting_relative = pathlib.PurePosixPath(condition.static_lighting)
        if lighting_relative.parts[:1] != ("assets",):
            raise HarnessError("static lighting path must be assets-relative")
        scenario_path.write_text(
            legacy.patch_scenario_lighting(
                scenario_path.read_text(encoding="utf-8"),
                SCENARIO,
                pathlib.PurePosixPath(*lighting_relative.parts[1:]).as_posix(),
            ),
            encoding="utf-8",
        )
        modified.append(scenario_relative)

    staged_hashes = legacy.tree_file_hashes(stage_root / "assets")
    if set(staged_hashes) != set(source_hashes):
        raise HarnessError(f"asset stage {condition.id} changed the asset file set")
    changed = sorted(path for path in source_hashes if source_hashes[path] != staged_hashes[path])
    if changed != sorted(set(modified)):
        raise HarnessError(
            f"asset stage {condition.id} changed unexpected files; expected={sorted(set(modified))}, got={changed}"
        )
    if legacy.read_level_height(world_path.read_text(encoding="utf-8")) != 0.35:
        raise HarnessError(f"asset stage {condition.id} did not resolve level_height 0.35")
    staged_world = world_path.read_text(encoding="utf-8")
    if len(re.findall(r"\bgrid_radius\s*:\s*187\s*,", staged_world)) != 1:
        raise HarnessError(f"asset stage {condition.id} did not preserve radius 187")
    if len(re.findall(r"\bgenerator_version\s*:\s*3\s*,", staged_world)) != 1:
        raise HarnessError(f"asset stage {condition.id} did not preserve Grand V3")
    if legacy.tree_digest(REPOSITORY_ROOT / "assets") != source_asset_digest:
        raise HarnessError("source asset tree changed while staging")
    manifest = {
        "version": 1,
        "warning": WARNING,
        "asset_stage": condition.asset_stage,
        "lighting_condition": dataclasses.asdict(condition),
        "source_asset_tree_sha256": source_asset_digest,
        "staged_asset_tree_sha256": legacy.tree_digest(stage_root / "assets"),
        "modified_assets": [
            {
                "path": f"assets/{relative}",
                "source_sha256": source_hashes[relative],
                "staged_sha256": staged_hashes[relative],
            }
            for relative in changed
        ],
    }
    atomic_write(manifest_path, pretty_json(manifest))
    legacy.make_tree_read_only(stage_root)
    return stage_root, manifest


def _validated_asset_stage_binding(
    work_root: pathlib.Path,
    condition: LightingCondition,
    *,
    expected_root: Optional[pathlib.Path] = None,
    expected_binding: Optional[Mapping[str, str]] = None,
) -> Tuple[pathlib.Path, Dict[str, str]]:
    """Revalidate a staged tree and return its immutable semantic hash binding."""

    stage_root, manifest = _stage_asset_root(work_root, condition)
    stage_root = stage_root.resolve()
    if expected_root is not None and stage_root != expected_root.resolve():
        raise HarnessError(f"asset stage root changed during capture: {condition.asset_stage}")
    staged_tree_sha256 = manifest.get("staged_asset_tree_sha256")
    source_tree_sha256 = manifest.get("source_asset_tree_sha256")
    for field, value in (
        ("staged_asset_tree_sha256", staged_tree_sha256),
        ("source_asset_tree_sha256", source_tree_sha256),
    ):
        if not isinstance(value, str) or SHA256_RE.fullmatch(value) is None:
            raise HarnessError(f"asset stage manifest has invalid {field}: {condition.asset_stage}")
    if manifest.get("asset_stage") != condition.asset_stage:
        raise HarnessError(f"asset stage manifest identity changed: {condition.asset_stage}")
    manifest_path = stage_root.parent / "stage-manifest.json"
    if not manifest_path.is_file() or manifest_path.is_symlink():
        raise HarnessError(f"asset stage manifest file is unavailable: {manifest_path}")
    binding = {
        "asset_stage": condition.asset_stage,
        "asset_stage_manifest_sha256": sha256_file(manifest_path),
        "asset_stage_tree_sha256": staged_tree_sha256,
        "source_asset_tree_sha256": source_tree_sha256,
    }
    if expected_binding is not None and dict(expected_binding) != binding:
        raise HarnessError(f"asset stage binding changed during capture: {condition.asset_stage}")
    return stage_root, binding


def runtime_report_path(png_path: pathlib.Path) -> pathlib.Path:
    """Return the deterministic runtime sidecar path for one PNG."""

    return png_path.with_name(png_path.stem + ".world-detail-report.json")


def inspect_png(path: pathlib.Path) -> Dict[str, Any]:
    """Validate PNG structure, dimensions, and non-blank pixel variation."""

    legacy = _legacy_harness()
    width, height = legacy.inspect_png(path)
    try:
        from PIL import Image, ImageStat  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required for blank-frame validation") from error
    try:
        with Image.open(path) as image:
            image.load()
            rgb = image.convert("RGB")
            stat = ImageStat.Stat(rgb)
            extrema = rgb.getextrema()
            entropy = float(rgb.entropy())
    except OSError as error:
        raise HarnessError(f"cannot decode PNG pixels {path}: {error}") from error
    if all(low == high for low, high in extrema) or max(stat.stddev) < 1.0 or entropy < 1.0:
        raise HarnessError(f"capture is blank or effectively uniform: {path}")
    return {
        "width": width,
        "height": height,
        "sha256": sha256_file(path),
        "entropy": entropy,
        "channel_stddev": list(stat.stddev),
    }


def _exact_keys(value: Any, keys: Iterable[str], context: str) -> Dict[str, Any]:
    return _strict_object(value, context=context, required=tuple(keys))


def _profile_requirements(profile: Mapping[str, Any]) -> Dict[str, bool]:
    shore_kind = profile["shore_and_falls"]["kind"]
    water_kind = profile["water"]["kind"]
    requires_oit = (
        water_kind not in ("current", "transmission")
        or profile["physical_clouds"]["kind"] != "current"
        or shore_kind in ("foam", "plunge_spray", "restrained_combination")
        or profile["ice_fringe"]["kind"] != "current"
    )
    requires_transmission = water_kind == "transmission"
    return {
        "oit": requires_oit,
        "medium_transmission": requires_transmission,
        "depth_texture": requires_oit or requires_transmission,
        "volumetrics": profile["local_fog"]["kind"] != "current",
    }


RUNTIME_RECEIPT_FIELDS = (
    "version",
    "launch_nonce",
    "process_id",
    "executable_sha256",
    "source_provenance_sha256",
    "capture_plan_sha256",
    "profile_sha256",
)


def _validate_runtime_receipt(
    raw: Any,
    *,
    source_provenance_sha256: str,
    capture_plan_json: str,
    profile_sha256: str,
    expected_launch_nonce: Optional[str] = None,
) -> Dict[str, Any]:
    """Validate the runtime-authored process receipt and its ordered hash body."""

    receipt = _exact_keys(
        raw,
        (*RUNTIME_RECEIPT_FIELDS, "receipt_sha256"),
        "runtime receipt",
    )
    if receipt["version"] != 1:
        raise HarnessError("runtime receipt version changed")
    nonce = receipt["launch_nonce"]
    if not isinstance(nonce, str) or LAUNCH_NONCE_RE.fullmatch(nonce) is None:
        raise HarnessError("runtime receipt launch nonce must be 64 lowercase hex")
    if expected_launch_nonce is not None and nonce != expected_launch_nonce:
        raise HarnessError("runtime receipt does not bind the freshly launched process")
    process_id = receipt["process_id"]
    if isinstance(process_id, bool) or not isinstance(process_id, int) or process_id <= 0:
        raise HarnessError("runtime receipt process_id must be a positive integer")
    expected_hashes = {
        "source_provenance_sha256": source_provenance_sha256,
        "capture_plan_sha256": sha256_bytes(capture_plan_json.encode("utf-8")),
        "profile_sha256": profile_sha256,
    }
    for field in (
        "executable_sha256",
        "source_provenance_sha256",
        "capture_plan_sha256",
        "profile_sha256",
        "receipt_sha256",
    ):
        value = receipt[field]
        if not isinstance(value, str) or SHA256_RE.fullmatch(value) is None:
            raise HarnessError(f"runtime receipt {field} must be lowercase SHA-256")
    for field, expected in expected_hashes.items():
        if receipt[field] != expected:
            raise HarnessError(f"runtime receipt {field} does not bind this capture")
    hash_body = {field: receipt[field] for field in RUNTIME_RECEIPT_FIELDS}
    expected_receipt_sha256 = sha256_bytes(compact_json(hash_body).encode("utf-8"))
    if receipt["receipt_sha256"] != expected_receipt_sha256:
        raise HarnessError("runtime receipt ordered hash body changed")
    return receipt


def _validate_runtime_receipt_hash_only(
    raw: Any,
    *,
    source_provenance_sha256: str,
    profile_sha256: str,
    expected_launch_nonce: Optional[str] = None,
) -> Dict[str, Any]:
    """Validate a receipt when its exact capture-plan bytes live in launch evidence."""

    receipt = _exact_keys(
        raw,
        (*RUNTIME_RECEIPT_FIELDS, "receipt_sha256"),
        "runtime receipt",
    )
    if receipt["version"] != 1:
        raise HarnessError("runtime receipt version changed")
    if (
        not isinstance(receipt["launch_nonce"], str)
        or LAUNCH_NONCE_RE.fullmatch(receipt["launch_nonce"]) is None
        or (
            expected_launch_nonce is not None
            and receipt["launch_nonce"] != expected_launch_nonce
        )
    ):
        raise HarnessError("runtime receipt launch nonce is invalid or stale")
    if (
        isinstance(receipt["process_id"], bool)
        or not isinstance(receipt["process_id"], int)
        or receipt["process_id"] <= 0
    ):
        raise HarnessError("runtime receipt process_id must be a positive integer")
    for field in (
        "executable_sha256",
        "source_provenance_sha256",
        "capture_plan_sha256",
        "profile_sha256",
        "receipt_sha256",
    ):
        if not isinstance(receipt[field], str) or SHA256_RE.fullmatch(receipt[field]) is None:
            raise HarnessError(f"runtime receipt {field} must be lowercase SHA-256")
    if receipt["source_provenance_sha256"] != source_provenance_sha256:
        raise HarnessError("runtime receipt does not bind source provenance")
    if receipt["profile_sha256"] != profile_sha256:
        raise HarnessError("runtime receipt does not bind tested profile")
    hash_body = {field: receipt[field] for field in RUNTIME_RECEIPT_FIELDS}
    if receipt["receipt_sha256"] != sha256_bytes(
        compact_json(hash_body).encode("utf-8")
    ):
        raise HarnessError("runtime receipt ordered hash body changed")
    return receipt


def _validate_effect_validation(
    raw: Any,
    *,
    profile: Mapping[str, Any],
    anchor_names: Iterable[str],
) -> Dict[str, Any]:
    """Validate measured cloud/ice/fog coverage and named waterfall ownership."""

    evidence = _exact_keys(
        raw,
        ("cloud_coverage", "ice_coverage", "fog_coverage", "waterfall_anchors"),
        "runtime report effect_validation",
    )
    cloud_profile = profile["physical_clouds"]
    cloud = evidence["cloud_coverage"]
    if cloud_profile["kind"] == "current":
        if cloud is not None:
            raise HarnessError("control cloud profile must not report cloud coverage")
    else:
        cloud = _exact_keys(
            cloud,
            (
                "field_radius",
                "target_fraction",
                "measured_fraction",
                "tolerance",
                "sample_count",
                "cloud_clusters",
                "peak_intersection_required",
                "peak_intersecting_puffs",
            ),
            "runtime report cloud coverage",
        )
        field_radius = _finite_number(cloud["field_radius"], "cloud field_radius")
        target = _finite_number(cloud["target_fraction"], "cloud target_fraction")
        measured = _finite_number(cloud["measured_fraction"], "cloud measured_fraction")
        tolerance = _finite_number(cloud["tolerance"], "cloud tolerance")
        if (
            abs(field_radius - min(120.0, max(64.0, 187.0 * 0.52))) > 1.0e-4
            or abs(target - float(cloud_profile["projected_coverage"])) > 1.0e-6
            or not 0.0 <= measured <= 1.0
            or abs(tolerance - 0.01) > 1.0e-9
            or abs(measured - target) > tolerance + 1.0e-7
        ):
            raise HarnessError(
                "runtime cloud coverage differs from its rendered-puff silhouette contract"
            )
        for field in ("sample_count", "cloud_clusters"):
            if isinstance(cloud[field], bool) or not isinstance(cloud[field], int) or cloud[field] <= 0:
                raise HarnessError(f"runtime cloud coverage {field} must be positive")
        expected_peak_intersection = (
            cloud_profile["kind"] != "faceted_layer"
            or cloud_profile["altitude_band"] != "clear"
        )
        if cloud["peak_intersection_required"] is not expected_peak_intersection:
            raise HarnessError("runtime cloud peak-intersection requirement differs from altitude profile")
        intersecting = cloud["peak_intersecting_puffs"]
        if isinstance(intersecting, bool) or not isinstance(intersecting, int) or intersecting < 0:
            raise HarnessError("runtime cloud peak-intersecting puff count is invalid")
        if expected_peak_intersection and intersecting == 0:
            raise HarnessError("grazing/crossing clouds must prove a peak intersection")

    ice_profile = profile["ice_fringe"]
    ice = evidence["ice_coverage"]
    if ice_profile["kind"] == "current":
        if ice is not None:
            raise HarnessError("control ice profile must not report ice coverage")
    else:
        ice = _exact_keys(
            ice,
            ("target_fraction", "eligible_edges", "selected_edges"),
            "runtime report ice coverage",
        )
        target = _finite_number(ice["target_fraction"], "ice target_fraction")
        if abs(target - float(ice_profile["coverage"])) > 1.0e-6:
            raise HarnessError("runtime ice coverage target differs from its profile")
        for field in ("eligible_edges", "selected_edges"):
            if isinstance(ice[field], bool) or not isinstance(ice[field], int) or ice[field] < 0:
                raise HarnessError(f"runtime ice coverage {field} is invalid")
        if ice["eligible_edges"] <= 0:
            raise HarnessError("runtime ice coverage requires eligible shoreline edges")
        expected_selected = min(
            ice["eligible_edges"],
            math.ceil(ice["eligible_edges"] * target),
        )
        if ice["selected_edges"] != expected_selected:
            raise HarnessError("runtime ice selected-edge count is not the exact ceiling subset")

    fog_profile = profile["local_fog"]
    fog = evidence["fog_coverage"]
    if fog_profile["kind"] == "current":
        if fog is not None:
            raise HarnessError("control local-fog profile must not report fog coverage")
    else:
        fog = _exact_keys(
            fog,
            (
                "target_fraction",
                "measured_fraction",
                "sample_count",
                "active_samples",
                "fog_volumes",
            ),
            "runtime report fog coverage",
        )
        target = _finite_number(fog["target_fraction"], "fog target_fraction")
        measured = _finite_number(fog["measured_fraction"], "fog measured_fraction")
        if abs(target - float(fog_profile["coverage"])) > 1.0e-6:
            raise HarnessError("runtime fog coverage target differs from its profile")
        for field in ("sample_count", "active_samples", "fog_volumes"):
            if isinstance(fog[field], bool) or not isinstance(fog[field], int) or fog[field] <= 0:
                raise HarnessError(f"runtime fog coverage {field} must be positive")
        expected_active = min(fog["sample_count"], math.ceil(fog["sample_count"] * target))
        if fog["active_samples"] != expected_active:
            raise HarnessError("runtime fog occupancy is not the exact ceiling subset")
        expected_measured = fog["active_samples"] / fog["sample_count"]
        if abs(measured - expected_measured) > 1.0e-7:
            raise HarnessError("runtime fog measured coverage differs from its occupancy counts")

    waterfall_profile = profile["shore_and_falls"]
    waterfall_rows = evidence["waterfall_anchors"]
    if not isinstance(waterfall_rows, list):
        raise HarnessError("runtime waterfall anchor evidence must be an array")
    normalized_waterfalls = []
    names = []
    known_anchors = set(anchor_names)
    for index, raw_row in enumerate(waterfall_rows):
        row = _exact_keys(
            raw_row,
            (
                "anchor_name",
                "anchor_position",
                "landing_position",
                "distance_hexes",
            ),
            f"runtime waterfall anchor {index}",
        )
        name = row["anchor_name"]
        if not isinstance(name, str) or not name or name not in known_anchors:
            raise HarnessError("runtime waterfall evidence names an unknown authored anchor")
        for field in ("anchor_position", "landing_position"):
            position = row[field]
            if (
                not isinstance(position, list)
                or len(position) != 3
                or any(isinstance(value, bool) or not isinstance(value, int) for value in position)
            ):
                raise HarnessError(f"runtime waterfall {field} must be [q,r,level]")
        distance = row["distance_hexes"]
        if (
            isinstance(distance, bool)
            or not isinstance(distance, int)
            or row["anchor_position"] == row["landing_position"]
        ):
            raise HarnessError("runtime waterfall landing displacement is invalid")
        delta_q = row["landing_position"][0] - row["anchor_position"][0]
        delta_r = row["landing_position"][1] - row["anchor_position"][1]
        axial_distance = max(abs(delta_q), abs(delta_r), abs(delta_q + delta_r))
        if distance != axial_distance or axial_distance > 13:
            raise HarnessError(
                "runtime waterfall landing must report its exact axial distance "
                "within the authored displacement bound"
            )
        names.append(name)
        normalized_waterfalls.append(row)
    if names != sorted(set(names)):
        raise HarnessError("runtime waterfall anchors must be sorted and unique")
    requires_waterfalls = waterfall_profile["kind"] in (
        "plunge_spray",
        "restrained_combination",
    )
    if requires_waterfalls != bool(normalized_waterfalls):
        raise HarnessError("runtime waterfall evidence presence differs from shore profile")
    return {
        "cloud_coverage": cloud,
        "ice_coverage": ice,
        "fog_coverage": fog,
        "waterfall_anchors": normalized_waterfalls,
    }


def validate_runtime_report(
    report_path: pathlib.Path,
    *,
    png_path: pathlib.Path,
    camera: CameraSpec,
    profile_json: str,
    time_hours: Optional[float],
    liquid_phase_seconds: float,
    settle_frames: int = 90,
    source_provenance_sha256: str,
    capture_plan_json: str,
    expected_launch_nonce: Optional[str] = None,
    expected_completed_cycles: Optional[int] = None,
) -> Dict[str, Any]:
    """Strictly validate one runtime wrapper and its authority/report payload."""

    wrapper = _exact_keys(
        _read_json(report_path, "world-detail runtime report"),
        ("version", "warning", "capture", "report"),
        "world-detail runtime wrapper",
    )
    if wrapper["version"] != 1 or wrapper["warning"] != WARNING:
        raise HarnessError(f"runtime report version or warning changed: {report_path}")
    expected_capture = camera.expected_report_capture(
        png_path,
        time_hours=time_hours,
        liquid_phase_seconds=liquid_phase_seconds,
        settle_frames=settle_frames,
    )
    capture = _exact_keys(wrapper["capture"], expected_capture, "runtime report capture")
    if capture != expected_capture:
        raise HarnessError(f"runtime report capture provenance differs for {png_path}")

    profile = validate_profile_json(profile_json)
    profile_hash = sha256_bytes(profile_json.encode())
    report = _exact_keys(
        wrapper["report"],
        (
            "version",
            "profile_hash_sha256",
            "runtime_receipt",
            "authority",
            "counts",
            "anchor_heights",
            "anchor_classes",
            "projection_hashes",
            "effect_validation",
            "camera_features",
            "performance",
            "cleanup",
        ),
        "runtime report payload",
    )
    if report["version"] != REPORT_VERSION or report["profile_hash_sha256"] != profile_hash:
        raise HarnessError(f"runtime report profile identity differs for {png_path}")
    runtime_receipt = _validate_runtime_receipt(
        report["runtime_receipt"],
        source_provenance_sha256=source_provenance_sha256,
        capture_plan_json=capture_plan_json,
        profile_sha256=profile_hash,
        expected_launch_nonce=expected_launch_nonce,
    )

    authority_fields = (
        "voxel_map",
        "structural",
        "materialized",
        "liquid_graph",
        "topology",
        "traversal",
        "blockers",
        "anchors",
        "biomes",
        "feature_roots",
        "logical_terrain_picking",
        "gameplay_state",
    )
    authority = _exact_keys(report["authority"], authority_fields, "runtime report authority")
    if any(not isinstance(authority[field], str) or not authority[field] for field in authority_fields):
        raise HarnessError(f"runtime report has an empty authority fingerprint: {report_path}")

    count_fields = ("entities", "materials", "vertices", "triangles")
    family_count_fields = ("total", *FAMILY_ORDER)
    counts = _exact_keys(report["counts"], family_count_fields, "runtime report counts")
    normalized_counts = {}
    for row_name in family_count_fields:
        row = _exact_keys(counts[row_name], count_fields, f"runtime report counts.{row_name}")
        if any(isinstance(row[field], bool) or not isinstance(row[field], int) or row[field] < 0 for field in count_fields):
            raise HarnessError(f"runtime report has invalid counts.{row_name}")
        normalized_counts[row_name] = row
    for field in count_fields:
        expected = sum(normalized_counts[family][field] for family in FAMILY_ORDER)
        if normalized_counts["total"][field] != expected:
            raise HarnessError(f"runtime report total.{field} does not sum family rows")

    heights = report["anchor_heights"]
    classes = report["anchor_classes"]
    if not isinstance(heights, dict) or not isinstance(classes, dict) or set(heights) != set(classes):
        raise HarnessError("runtime report anchor height/class key sets differ")
    for anchor, height in heights.items():
        if not isinstance(anchor, str) or not anchor or not math.isfinite(_finite_number(height, f"anchor {anchor}")):
            raise HarnessError("runtime report anchor height is invalid")
        if classes[anchor] not in ("gameplay", "observation"):
            raise HarnessError(f"runtime report anchor {anchor} has invalid namespace")

    projection_hashes = _exact_keys(
        report["projection_hashes"],
        ("terrain_plan", "liquid_atmosphere_plan", "mesh_projection"),
        "runtime report projection_hashes",
    )
    if any(
        not isinstance(projection_hashes[field], str)
        or re.fullmatch(r"[0-9a-f]{16}", projection_hashes[field]) is None
        for field in projection_hashes
    ):
        raise HarnessError("runtime report projection hashes must be lowercase 64-bit hex")

    effect_validation = _validate_effect_validation(
        report["effect_validation"],
        profile=profile,
        anchor_names=heights,
    )

    features = _exact_keys(
        report["camera_features"],
        ("oit", "medium_transmission", "depth_texture", "volumetrics"),
        "runtime report camera_features",
    )
    if any(not isinstance(features[field], bool) for field in features):
        raise HarnessError("runtime report camera feature flags must be booleans")
    requirements = _profile_requirements(profile)
    for feature, required in requirements.items():
        if required and not features[feature]:
            raise HarnessError(f"runtime report failed to enable required camera feature {feature}")

    performance = _exact_keys(
        report["performance"],
        ("frame_time_ms", "resident_presentation_bytes", "warmup_complete"),
        "runtime report performance",
    )
    frame_time_ms = _finite_number(performance["frame_time_ms"], "performance.frame_time_ms")
    if not 0.0 < frame_time_ms <= 10_000.0:
        raise HarnessError("runtime report frame_time_ms must be in (0, 10000]")
    resident_bytes = performance["resident_presentation_bytes"]
    if isinstance(resident_bytes, bool) or not isinstance(resident_bytes, int) or resident_bytes <= 0:
        raise HarnessError("runtime report resident_presentation_bytes must be a positive integer")
    if performance["warmup_complete"] is not True:
        raise HarnessError("runtime report warmup_complete must be true")

    cleanup_fields = (
        "completed_cycles",
        "entities_remaining",
        "materials_remaining",
        "meshes_remaining",
        "target_images_remaining",
        "camera_state_restored",
        "oit_state_restored",
        "transmission_state_restored",
        "depth_state_restored",
        "volumetric_state_restored",
    )
    cleanup = _exact_keys(report["cleanup"], cleanup_fields, "runtime report cleanup")
    for field in (
        "completed_cycles",
        "entities_remaining",
        "materials_remaining",
        "meshes_remaining",
        "target_images_remaining",
    ):
        if isinstance(cleanup[field], bool) or not isinstance(cleanup[field], int) or cleanup[field] < 0:
            raise HarnessError(f"runtime report cleanup.{field} is invalid")
    if cleanup["completed_cycles"] < 1:
        raise HarnessError("runtime capture sidecar must represent a completed teardown cycle")
    if (
        expected_completed_cycles is not None
        and cleanup["completed_cycles"] != expected_completed_cycles
    ):
        raise HarnessError(
            "runtime capture sidecar completed_cycles differs from the required "
            f"teardown count {expected_completed_cycles}"
        )
    for field in (
        "entities_remaining",
        "materials_remaining",
        "meshes_remaining",
        "target_images_remaining",
    ):
        if cleanup[field] != 0:
            raise HarnessError(f"runtime report cleanup.{field} must be zero after teardown")
    restored_fields = cleanup_fields[5:]
    if any(cleanup[field] is not True for field in restored_fields):
        raise HarnessError("runtime report cleanup restoration flags must all be true")
    return {
        "path": str(report_path),
        "sha256": sha256_file(report_path),
        "profile_hash_sha256": profile_hash,
        "runtime_receipt": runtime_receipt,
        "authority": authority,
        "counts": normalized_counts,
        "anchor_count": len(heights),
        "anchor_heights": heights,
        "anchor_classes": classes,
        "projection_hashes": projection_hashes,
        "effect_validation": effect_validation,
        "camera_features": features,
        "performance": {
            "frame_time_ms": frame_time_ms,
            "resident_presentation_bytes": resident_bytes,
            "warmup_complete": performance["warmup_complete"],
        },
        "cleanup": cleanup,
        "post_teardown_cleanup_complete": True,
        "hundred_cycle_proof_source": "standalone hash-linked lifecycle certificate",
    }


def label_png(source: pathlib.Path, destination: pathlib.Path) -> Dict[str, Any]:
    """Create a visibly and metadata-labeled copy of one genuine game render."""

    try:
        from PIL import Image, ImageDraw, ImageFont, PngImagePlugin  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to label review imagery") from error
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        raise HarnessError(f"labeled image already exists: {destination}")
    with Image.open(source) as image:
        rendered = image.convert("RGBA")
        draw = ImageDraw.Draw(rendered, "RGBA")
        banner_height = 46
        draw.rectangle((0, 0, rendered.width, banner_height), fill=(8, 8, 10, 224))
        try:
            font = ImageFont.truetype("Arial.ttf", 24)
        except OSError:
            font = ImageFont.load_default()
        draw.text((18, 10), WARNING, fill=(255, 214, 72, 255), font=font)
        metadata = PngImagePlugin.PngInfo()
        metadata.add_text("structural_draft_warning", WARNING)
        metadata.add_text("source_render_sha256", sha256_file(source))
        rendered.convert("RGB").save(destination, format="PNG", pnginfo=metadata, optimize=False)
    details = inspect_png(destination)
    with Image.open(destination) as labeled:
        if labeled.info.get("structural_draft_warning") != WARNING:
            raise HarnessError(f"labeled PNG lost structural-draft metadata: {destination}")
        if labeled.info.get("source_render_sha256") != sha256_file(source):
            raise HarnessError(f"labeled PNG lost source-render provenance: {destination}")
    return details


def _labeled_still_path(output_root: pathlib.Path, raw_path: pathlib.Path) -> pathlib.Path:
    parts = raw_path.resolve().parts
    marker = ("runtime", "raw-stills")
    matches = [
        index for index in range(len(parts) - 1) if tuple(parts[index : index + 2]) == marker
    ]
    if len(matches) != 1 or matches[0] + 2 >= len(parts):
        raise HarnessError(f"raw still lacks one planned runtime marker: {raw_path}")
    relative = pathlib.Path(*parts[matches[0] + 2 :])
    return output_root / "source-pngs" / relative


def _labeled_capture_path(output_root: pathlib.Path, raw_path: pathlib.Path) -> pathlib.Path:
    """Map a raw still or motion frame to its user-facing labeled copy."""

    parts = raw_path.resolve().parts
    for marker_name, output_name in (
        ("raw-motion", "motion-frames"),
        ("raw-motion-controls", "motion-control-frames"),
    ):
        marker = ("runtime", marker_name)
        matches = [
            index
            for index in range(len(parts) - 1)
            if tuple(parts[index : index + 2]) == marker
        ]
        if matches:
            if len(matches) != 1 or matches[0] + 2 >= len(parts):
                raise HarnessError(f"raw motion path has an ambiguous runtime marker: {raw_path}")
            return output_root / output_name / pathlib.Path(*parts[matches[0] + 2 :])
    return _labeled_still_path(output_root, raw_path)


def _published_runtime_report_path(
    output_root: pathlib.Path, raw_report_path: pathlib.Path
) -> pathlib.Path:
    """Mirror an external raw sidecar into the self-contained publication tree."""

    if raw_report_path.name.endswith(".world-detail-report.json") is False:
        raise HarnessError(f"runtime report has the wrong sibling suffix: {raw_report_path}")
    parts = raw_report_path.resolve().parts
    allowed = {"raw-stills", "raw-motion", "raw-motion-controls"}
    matches = [
        index
        for index in range(len(parts) - 1)
        if parts[index] == "runtime" and parts[index + 1] in allowed
    ]
    if len(matches) != 1 or matches[0] + 2 >= len(parts):
        raise HarnessError(f"runtime report lacks one planned runtime marker: {raw_report_path}")
    marker_index = matches[0]
    relative = pathlib.Path(*parts[marker_index + 2 :])
    return output_root / "runtime-reports" / parts[marker_index + 1] / relative


def _materialize_runtime_report(
    output_root: pathlib.Path,
    raw_report_path: pathlib.Path,
    *,
    create: bool,
) -> Dict[str, Any]:
    """Copy or verify one byte-identical, standalone runtime report."""

    if not raw_report_path.is_file() or raw_report_path.is_symlink():
        raise HarnessError(f"raw runtime report is unavailable: {raw_report_path}")
    destination = _published_runtime_report_path(output_root, raw_report_path)
    source_sha256 = sha256_file(raw_report_path)
    if destination.exists():
        if destination.is_symlink() or not destination.is_file():
            raise HarnessError(f"published runtime report is not a regular file: {destination}")
        if sha256_file(destination) != source_sha256:
            raise HarnessError(f"published runtime report differs from raw evidence: {destination}")
    elif create:
        destination.parent.mkdir(parents=True, exist_ok=True)
        temporary = destination.with_name(destination.name + ".tmp")
        if temporary.exists():
            raise HarnessError(f"stale runtime-report temporary exists: {temporary}")
        shutil.copyfile(raw_report_path, temporary)
        os.replace(temporary, destination)
    else:
        raise HarnessError(f"standalone published runtime report is missing: {destination}")
    return {
        "path": str(destination),
        "sha256": source_sha256,
        "raw_path": str(raw_report_path),
    }


def _sanitized_environment() -> Dict[str, str]:
    blocked_exact = {
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
    blocked_prefixes = ("BEVY_", "HEX_", "WGPU_", "CARGO_PROFILE_")
    environment = {
        key: value
        for key, value in os.environ.items()
        if key not in blocked_exact and not any(key.startswith(prefix) for prefix in blocked_prefixes)
    }
    environment["CARGO_INCREMENTAL"] = "0"
    return environment


def _map_review_environment() -> Dict[str, str]:
    """Build a scrubbed environment carrying the mandatory structural-draft label."""

    environment = _sanitized_environment()
    environment[STRUCTURAL_DRAFT_ENVIRONMENT] = STRUCTURAL_DRAFT_VALUE
    return environment


def _camera_from_record(raw: Any, context: str) -> CameraSpec:
    """Decode an internally generated camera record, including orbit frames."""

    value = _strict_object(
        raw,
        context=context,
        required=(
            "id",
            "filename",
            "camera",
            "view",
            "focus_anchor",
            "look_at_anchor",
            "look_at_offset",
            "character_radius_scale",
            "full_cutaway",
            "illumination_overlay",
        ),
    )
    offset_raw = value["look_at_offset"]
    offset = None
    if offset_raw is not None:
        if not isinstance(offset_raw, (list, tuple)) or len(offset_raw) != 3:
            raise HarnessError(f"{context}.look_at_offset must contain three values")
        offset = tuple(_finite_number(component, f"{context}.look_at_offset") for component in offset_raw)
    radius = _finite_number(value["character_radius_scale"], f"{context}.character_radius_scale")
    if value["camera"] not in ("map", "character", "first-person"):
        raise HarnessError(f"{context}.camera is invalid")
    if value["view"] not in ("default", "rotated", "counter-rotated", "rear", "top-down"):
        raise HarnessError(f"{context}.view is invalid")
    if any(not isinstance(value[field], bool) for field in ("full_cutaway", "illumination_overlay")):
        raise HarnessError(f"{context} boolean camera fields are invalid")
    return CameraSpec(
        id=value["id"],
        filename=value["filename"],
        camera=value["camera"],
        view=value["view"],
        focus_anchor=value["focus_anchor"],
        look_at_anchor=value["look_at_anchor"],
        look_at_offset=offset,
        character_radius_scale=radius,
        full_cutaway=value["full_cutaway"],
        illumination_overlay=value["illumination_overlay"],
    )


def _job_cameras(
    job: Mapping[str, Any],
    _final_cameras: Sequence[CameraSpec],
) -> Dict[pathlib.Path, Tuple[CameraSpec, float, int]]:
    camera_records = job.get("cameras")
    captures = job["capture_plan"]["captures"]
    if not isinstance(camera_records, list) or len(camera_records) != len(captures):
        raise HarnessError(f"job {job['id']} camera records do not match captures")
    result = {}
    for index, (entry, raw_camera) in enumerate(zip(captures, camera_records)):
        camera = _camera_from_record(raw_camera, f"job {job['id']}.cameras[{index}]")
        expected = camera.runtime_entry(pathlib.Path(entry["path"]))
        if job["capture_plan"]["version"] == 2:
            expected["liquid_phase_seconds"] = entry.get("liquid_phase_seconds")
            expected["settle_frames"] = entry.get("settle_frames")
        if entry != expected:
            raise HarnessError(f"job {job['id']} camera record differs from runtime capture {index}")
        path = pathlib.Path(entry["path"]).resolve()
        if path in result:
            raise HarnessError(f"job {job['id']} repeats capture path {path}")
        phase = (
            entry["liquid_phase_seconds"]
            if job["capture_plan"]["version"] == 2
            else job["liquid_phase_seconds"]
        )
        settle_frames = entry.get("settle_frames", 90)
        if (
            isinstance(settle_frames, bool)
            or not isinstance(settle_frames, int)
            or not 1 <= settle_frames <= 90
        ):
            raise HarnessError(f"job {job['id']} settle_frames must be an integer in 1..=90")
        result[path] = (
            camera,
            _finite_number(phase, f"job {job['id']} liquid phase"),
            settle_frames,
        )
    return result


def _validate_job_artifacts(
    job: Mapping[str, Any],
    output_root: pathlib.Path,
    *,
    source_provenance_sha256: str,
    expected_launch_nonce: Optional[str] = None,
    label: bool,
    require_labeled: bool = False,
) -> Dict[str, Any]:
    final_cameras, _, _ = load_camera_sets()
    camera_by_path = _job_cameras(job, final_cameras)
    if not isinstance(job.get("raw_capture_root"), str):
        raise HarnessError(f"job {job['id']} lacks its external raw capture root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(job["raw_capture_root"])
    )
    artifacts = job.get("artifacts")
    if not isinstance(artifacts, list) or len(artifacts) != len(camera_by_path):
        raise HarnessError(f"job {job['id']} raw artifact list changed")
    expected_paths = [_raw_artifact_path(raw_capture_root, artifact) for artifact in artifacts]
    if list(camera_by_path) != expected_paths:
        raise HarnessError(f"job {job['id']} capture paths differ from external raw artifacts")
    png_records = []
    report_records = []
    authority = None
    anchor_heights = None
    anchor_classes = None
    presentation_counts = None
    camera_features = None
    resident_baseline = None
    performance_samples = []
    projection_states = []
    runtime_receipt = None
    capture_plan_json = compact_json(job["capture_plan"])
    for png_path, (camera, liquid_phase_seconds, settle_frames) in camera_by_path.items():
        try:
            png_path.relative_to(raw_capture_root)
        except ValueError as error:
            raise HarnessError(f"capture path escaped external raw root: {png_path}") from error
        png_record = inspect_png(png_path)
        raw_report_path = runtime_report_path(png_path)
        report_record = validate_runtime_report(
            raw_report_path,
            png_path=png_path,
            camera=camera,
            profile_json=job["profile_json"],
            time_hours=job["time_hours"],
            liquid_phase_seconds=liquid_phase_seconds,
            settle_frames=settle_frames,
            source_provenance_sha256=source_provenance_sha256,
            capture_plan_json=capture_plan_json,
            expected_launch_nonce=expected_launch_nonce,
        )
        if runtime_receipt is None:
            runtime_receipt = report_record["runtime_receipt"]
        elif runtime_receipt != report_record["runtime_receipt"]:
            raise HarnessError(f"runtime process receipt varies inside job {job['id']}")
        if authority is None:
            authority = report_record["authority"]
        elif authority != report_record["authority"]:
            raise HarnessError(f"authority fingerprints vary inside multi-camera job {job['id']}")
        if anchor_heights is None:
            anchor_heights = report_record["anchor_heights"]
            anchor_classes = report_record["anchor_classes"]
        elif (
            anchor_heights != report_record["anchor_heights"]
            or anchor_classes != report_record["anchor_classes"]
        ):
            raise HarnessError(f"anchor evidence varies inside multi-camera job {job['id']}")
        if presentation_counts is None:
            presentation_counts = report_record["counts"]
            camera_features = report_record["camera_features"]
        elif presentation_counts != report_record["counts"]:
            raise HarnessError(
                f"presentation entity/material/mesh counts grow or vary inside job {job['id']}"
            )
        elif camera_features != report_record["camera_features"]:
            raise HarnessError(f"camera feature state varies inside job {job['id']}")
        performance = report_record["performance"]
        if not performance["warmup_complete"]:
            raise HarnessError(f"capture was emitted before warm-up completed in job {job['id']}")
        if resident_baseline is None:
            resident_baseline = performance["resident_presentation_bytes"]
        elif performance["resident_presentation_bytes"] > resident_baseline:
            raise HarnessError(f"resident presentation memory grew after warm-up in job {job['id']}")
        performance_samples.append(
            {
                "liquid_phase_seconds": liquid_phase_seconds,
                **performance,
            }
        )
        projection_states.append(
            {
                "liquid_phase_seconds": liquid_phase_seconds,
                "projection_hashes": report_record["projection_hashes"],
            }
        )
        if label:
            labeled_path = _labeled_capture_path(output_root, png_path)
            if labeled_path.exists():
                labeled_record = _inspect_labeled_png(
                    labeled_path,
                    expected_source_sha256=png_record["sha256"],
                )
            else:
                labeled_record = label_png(png_path, labeled_path)
            png_record["labeled_path"] = str(labeled_path)
            png_record["labeled_sha256"] = labeled_record["sha256"]
            report_record["published"] = _materialize_runtime_report(
                output_root,
                raw_report_path,
                create=True,
            )
        elif require_labeled:
            labeled_path = _labeled_capture_path(output_root, png_path)
            if not labeled_path.is_file():
                raise HarnessError(f"required warning-labeled derivative is missing: {labeled_path}")
            labeled_record = _inspect_labeled_png(
                labeled_path,
                expected_source_sha256=png_record["sha256"],
            )
            png_record["labeled_path"] = str(labeled_path)
            png_record["labeled_sha256"] = labeled_record["sha256"]
            report_record["published"] = _materialize_runtime_report(
                output_root,
                raw_report_path,
                create=False,
            )
        png_record["path"] = str(png_path)
        png_records.append(png_record)
        report_records.append(report_record)
    return {
        "pngs": png_records,
        "reports": report_records,
        "authority": authority,
        "anchor_heights": anchor_heights,
        "anchor_classes": anchor_classes,
        "presentation_counts": presentation_counts,
        "camera_features": camera_features,
        "performance_samples": performance_samples,
        "projection_states": projection_states,
        "runtime_receipt": runtime_receipt,
    }


def _record_authority(
    authority: Mapping[str, str],
    *,
    condition_key: Tuple[str, Optional[float]],
    world_reference: Optional[Mapping[str, str]],
    gameplay_by_condition: MutableMapping[Tuple[str, Optional[float]], str],
) -> Mapping[str, str]:
    """Compare immutable world authority globally and gameplay state per lighting."""

    world = {key: value for key, value in authority.items() if key != "gameplay_state"}
    if world_reference is not None and world != world_reference:
        raise HarnessError("world authority fingerprints changed across capture jobs")
    previous_gameplay = gameplay_by_condition.setdefault(condition_key, authority["gameplay_state"])
    if previous_gameplay != authority["gameplay_state"]:
        raise HarnessError(f"gameplay-state fingerprint changed within condition {condition_key}")
    return world


def _record_projection_hashes(
    hashes: Mapping[str, str],
    *,
    profile_sha256: str,
    liquid_phase_seconds: float,
    references: MutableMapping[Tuple[str, float], Mapping[str, str]],
) -> None:
    """Require identical plan/mesh hashes for repeated profile/phase projections."""

    key = (profile_sha256, _as_f32(liquid_phase_seconds))
    previous = references.setdefault(key, dict(hashes))
    if previous != hashes:
        raise HarnessError(f"projection hashes changed for repeated profile/phase {key}")


def _nearest_rank_p95(values: Sequence[float]) -> float:
    if not values:
        raise HarnessError("cannot compute p95 without samples")
    ordered = sorted(_finite_number(value, "performance sample") for value in values)
    return ordered[max(0, math.ceil(0.95 * len(ordered)) - 1)]


def _job_performance_summary(record: Mapping[str, Any]) -> Dict[str, Any]:
    samples = record["performance_samples"]
    if not samples:
        raise HarnessError("capture job has no performance samples")
    return {
        "sample_count": len(samples),
        "p95_frame_time_ms": _nearest_rank_p95(
            [sample["frame_time_ms"] for sample in samples]
        ),
        "max_resident_presentation_bytes": max(
            sample["resident_presentation_bytes"] for sample in samples
        ),
        "warmup_complete": all(sample["warmup_complete"] for sample in samples),
        "resident_growth_after_first": max(
            sample["resident_presentation_bytes"] for sample in samples
        )
        - samples[0]["resident_presentation_bytes"],
    }


def _ratio_with_zero_guard(candidate: float, control: float, context: str) -> float:
    if control < 0.0 or candidate < 0.0:
        raise HarnessError(f"negative performance value in {context}")
    if control == 0.0:
        if candidate != 0.0:
            raise HarnessError(f"nonzero candidate has zero control baseline in {context}")
        return 1.0
    return candidate / control


def validate_performance_evidence(
    plan: Mapping[str, Any],
    records_by_job: Mapping[str, Mapping[str, Any]],
) -> Dict[str, Any]:
    """Aggregate per-capture telemetry and enforce the final leader's 15% cap."""

    summaries = {
        job_id: _job_performance_summary(record)
        for job_id, record in sorted(records_by_job.items())
    }
    for job_id, summary in summaries.items():
        if not summary["warmup_complete"] or summary["resident_growth_after_first"] > 0:
            raise HarnessError(f"warm-up growth gate failed for {job_id}")

    leader_clip_ids = {
        "combination-score-leader-02",
        "combination-score-leader-14",
        "leader-golden-03",
        "leader-overcast-02",
    }
    leader_comparisons = []
    clips = plan["motion"]["clips"]
    if clips:
        by_clip = {clip["id"]: clip for clip in clips}
        if not leader_clip_ids <= set(by_clip):
            raise HarnessError("motion plan is missing a final score-leader performance condition")
        for clip_id in sorted(leader_clip_ids):
            clip = by_clip[clip_id]
            if len(clip["candidate_job_ids"]) != 1 or len(clip["control_job_ids"]) != 1:
                raise HarnessError(f"performance clip {clip_id} must use one v2 job per side")
            candidate_id = clip["candidate_job_ids"][0]
            control_id = clip["control_job_ids"][0]
            if candidate_id not in summaries or control_id not in summaries:
                raise HarnessError(f"performance evidence is incomplete for {clip_id}")
            candidate = summaries[candidate_id]
            control = summaries[control_id]
            if candidate["sample_count"] != MOTION_FRAME_COUNT or control["sample_count"] != MOTION_FRAME_COUNT:
                raise HarnessError(f"performance clip {clip_id} lacks 90 samples per side")
            frame_ratio = _ratio_with_zero_guard(
                candidate["p95_frame_time_ms"],
                control["p95_frame_time_ms"],
                f"{clip_id} p95 frame time",
            )
            memory_ratio = _ratio_with_zero_guard(
                candidate["max_resident_presentation_bytes"],
                control["max_resident_presentation_bytes"],
                f"{clip_id} resident memory",
            )
            passed = frame_ratio <= 1.15 and memory_ratio <= 1.15
            if not passed:
                raise HarnessError(
                    f"final score leader exceeds the 15% performance cap at {clip_id}"
                )
            leader_comparisons.append(
                {
                    "clip_id": clip_id,
                    "candidate_job_id": candidate_id,
                    "control_job_id": control_id,
                    "p95_frame_time_ratio": frame_ratio,
                    "resident_presentation_memory_ratio": memory_ratio,
                    "passed": passed,
                }
            )
    return {
        "jobs": summaries,
        "leader_comparisons": leader_comparisons,
        "maximum_ratio": 1.15,
        "p95_method": "nearest-rank",
        "resident_memory_definition": (
            "total resident presentation asset bytes for the matched scene; unique live Mesh3d "
            "buffers, relevant live material allocations, non-capture image mip payloads, and "
            "review entity/component/name payloads; capture target excluded or symmetric"
        ),
    }


def _initialize_capture_executable_pin(
    state: MutableMapping[str, Any],
    completed: Mapping[str, Any],
    attempts: Sequence[Mapping[str, Any]],
) -> Optional[str]:
    """Validate or recover the one executable identity shared by all captures."""

    pin = state.get("pinned_executable_sha256")
    if pin is not None and (not isinstance(pin, str) or SHA256_RE.fullmatch(pin) is None):
        raise HarnessError("capture state executable pin is malformed")
    observed: List[Tuple[str, Any]] = []
    for job_id, raw in completed.items():
        if not isinstance(raw, dict):
            raise HarnessError(f"capture state completed record is malformed for {job_id}")
        observed.append((f"completed job {job_id}", raw.get("executable_sha256")))
    for index, attempt in enumerate(attempts):
        if attempt.get("status") == "COMPLETE":
            observed.append((f"complete attempt {index}", attempt.get("executable_sha256")))
    for context, executable_sha256 in observed:
        if (
            not isinstance(executable_sha256, str)
            or SHA256_RE.fullmatch(executable_sha256) is None
        ):
            raise HarnessError(f"{context} lacks a valid executable SHA-256")
        if pin is None:
            pin = executable_sha256
        elif executable_sha256 != pin:
            raise HarnessError(
                f"{context} used executable {executable_sha256}, not pinned executable {pin}"
            )
    state["pinned_executable_sha256"] = pin
    return pin


def _assert_capture_executable_matches_pin(
    state: Mapping[str, Any], executable_sha256: Any, *, context: str
) -> str:
    if not isinstance(executable_sha256, str) or SHA256_RE.fullmatch(executable_sha256) is None:
        raise HarnessError(f"{context} returned an invalid executable SHA-256")
    pin = state.get("pinned_executable_sha256")
    if pin is not None and executable_sha256 != pin:
        raise HarnessError(
            f"{context} used executable {executable_sha256}, not pinned executable {pin}"
        )
    return executable_sha256


def _compare_raster_stable_rgb(
    reference_path: pathlib.Path,
    candidate_path: pathlib.Path,
    *,
    camera_id: str,
    ambiguous_pixels: Sequence[Mapping[str, Any]],
    context: str,
    require_clean_observed_ambiguous_values: bool = True,
) -> Dict[str, Any]:
    """Compare exact RGB bytes, except for enumerated clean-source raster ambiguity.

    This is deliberately not a perceptual or numeric-threshold comparison. Every
    non-enumerated byte must match the oracle's primary clean-source render. At an
    enumerated shared-vertex coordinate, control/oracle comparisons require each
    differing RGB tuple to be one of the values observed in independent clean-source
    launches. Treatment reproduction can instead use the same empirically established
    coordinates as a spatial mask and record its treatment-specific endpoint tuples.
    """

    try:
        import numpy as np  # pylint: disable=import-outside-toplevel
        from PIL import Image  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("NumPy and Pillow are required for oracle pixel validation") from error

    def load(path: pathlib.Path) -> Any:
        if not path.is_file() or path.is_symlink():
            raise HarnessError(f"{context} requires a regular PNG: {path}")
        try:
            with Image.open(path) as image:
                image.load()
                pixels = np.asarray(image.convert("RGB"), dtype=np.uint8).copy()
        except OSError as error:
            raise HarnessError(f"cannot decode {context} PNG {path}: {error}") from error
        if pixels.shape != (CAPTURE_HEIGHT, CAPTURE_WIDTH, 3):
            raise HarnessError(f"{context} PNG has unexpected dimensions: {path}")
        return pixels

    allowed_by_coordinate: Dict[Tuple[int, int], Tuple[Tuple[int, int, int], ...]] = {}
    for index, raw in enumerate(ambiguous_pixels):
        row = _exact_keys(
            raw,
            ("x", "y", "allowed_rgb"),
            f"{context} ambiguous pixel {index}",
        )
        x_value = row["x"]
        y_value = row["y"]
        if (
            isinstance(x_value, bool)
            or not isinstance(x_value, int)
            or not 0 <= x_value < CAPTURE_WIDTH
            or isinstance(y_value, bool)
            or not isinstance(y_value, int)
            or not 0 <= y_value < CAPTURE_HEIGHT
        ):
            raise HarnessError(f"{context} has an out-of-range ambiguous pixel")
        coordinate = (x_value, y_value)
        if coordinate in allowed_by_coordinate:
            raise HarnessError(f"{context} repeats ambiguous pixel {coordinate}")
        allowed_raw = row["allowed_rgb"]
        if not isinstance(allowed_raw, list) or len(allowed_raw) < 2:
            raise HarnessError(f"{context} ambiguous pixel lacks two observed values")
        allowed_values: List[Tuple[int, int, int]] = []
        for value in allowed_raw:
            if (
                not isinstance(value, list)
                or len(value) != 3
                or any(
                    isinstance(channel, bool)
                    or not isinstance(channel, int)
                    or not 0 <= channel <= 255
                    for channel in value
                )
            ):
                raise HarnessError(f"{context} has malformed allowed RGB values")
            normalized = tuple(value)
            if normalized in allowed_values:
                raise HarnessError(f"{context} repeats an allowed RGB value")
            allowed_values.append(normalized)
        if allowed_values != sorted(allowed_values):
            raise HarnessError(f"{context} allowed RGB values are not canonical")
        allowed_by_coordinate[coordinate] = tuple(allowed_values)

    reference = load(reference_path)
    candidate = load(candidate_path)
    differing = np.any(reference != candidate, axis=2)
    differing_coordinates = {
        (int(x_value), int(y_value))
        for y_value, x_value in np.argwhere(differing)
    }
    foreign = differing_coordinates - set(allowed_by_coordinate)
    if foreign:
        first = min(foreign, key=lambda coordinate: (coordinate[1], coordinate[0]))
        raise HarnessError(
            f"{context} differs at raster-stable pixel {first[0]},{first[1]} "
            f"for {camera_id}"
        )

    ambiguous_values = []
    for (x_value, y_value), allowed in sorted(
        allowed_by_coordinate.items(), key=lambda item: (item[0][1], item[0][0])
    ):
        reference_rgb = tuple(int(value) for value in reference[y_value, x_value])
        candidate_rgb = tuple(int(value) for value in candidate[y_value, x_value])
        # An exactly equal pair needs no exception, even when the subject is a
        # non-control treatment whose ordinary color at this coordinate is not
        # present in the clean-control oracle. Control/oracle comparisons also
        # constrain differing endpoints to observed values; reproduction uses
        # only the empirically established coordinate mask because the chosen
        # treatment may legitimately alter the endpoint colors.
        if (
            reference_rgb != candidate_rgb
            and require_clean_observed_ambiguous_values
            and (reference_rgb not in allowed or candidate_rgb not in allowed)
        ):
            raise HarnessError(
                f"{context} has an unobserved shared-vertex RGB value at "
                f"{x_value},{y_value} for {camera_id}"
            )
        ambiguous_values.append(
            {
                "x": x_value,
                "y": y_value,
                "reference_rgb": list(reference_rgb),
                "candidate_rgb": list(candidate_rgb),
                "equal": reference_rgb == candidate_rgb,
            }
        )
    pixel_count = CAPTURE_WIDTH * CAPTURE_HEIGHT
    return {
        "camera_id": camera_id,
        "reference_png_sha256": sha256_file(reference_path),
        "reference_decoded_rgb_sha256": decoded_rgb_sha256(reference_path),
        "candidate_png_sha256": sha256_file(candidate_path),
        "candidate_decoded_rgb_sha256": decoded_rgb_sha256(candidate_path),
        "decoded_rgb_identical": not bool(differing_coordinates),
        "stable_pixel_identical": True,
        "pixel_count": pixel_count,
        "stable_pixel_count": pixel_count - len(allowed_by_coordinate),
        "ambiguous_pixel_count": len(allowed_by_coordinate),
        "differing_ambiguous_pixel_count": len(differing_coordinates),
        "ambiguous_value_policy": (
            "clean-source-observed-rgb-only"
            if require_clean_observed_ambiguous_values
            else "oracle-coordinate-mask-with-endpoints-recorded"
        ),
        "ambiguous_values": ambiguous_values,
    }


def _decoded_rgb_bytes_sha256(path: pathlib.Path) -> str:
    """Hash only decoded RGB bytes, matching BaselineOraclePackV1."""

    try:
        from PIL import Image  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required for oracle RGB hashing") from error
    try:
        with Image.open(path) as image:
            image.load()
            rgb = image.convert("RGB")
            if rgb.size != (CAPTURE_WIDTH, CAPTURE_HEIGHT):
                raise HarnessError(f"baseline-oracle PNG has unexpected dimensions: {path}")
            payload = rgb.tobytes()
    except OSError as error:
        raise HarnessError(f"cannot decode baseline-oracle PNG {path}: {error}") from error
    return sha256_bytes(payload)


def _validate_baseline_oracle_pack(
    root: pathlib.Path = BASELINE_ORACLE_ROOT,
) -> Dict[str, Any]:
    """Validate the complete private BaselineOraclePackV1 and clean-run evidence."""

    contract, manifest = _baseline_oracle_documents(root)
    root = root.resolve()
    inventory = _exact_keys(
        manifest["inventory"],
        (
            "directories",
            "directory_count",
            "files",
            "regular_file_count",
            "total_file_bytes",
            "manifest_excluded_path",
            "manifest_sha256",
            "no_symlinks",
            "entry_policy",
            "hash_convention",
        ),
        "baseline-oracle inventory",
    )
    declared_directories = inventory["directories"]
    declared_files = inventory["files"]
    if (
        inventory["no_symlinks"] is not True
        or not isinstance(declared_directories, list)
        or declared_directories != sorted(declared_directories)
        or len(declared_directories) != len(set(declared_directories))
        or inventory["directory_count"] != len(declared_directories)
        or not isinstance(declared_files, dict)
        or inventory["regular_file_count"] != len(declared_files)
        or BASELINE_ORACLE_MANIFEST_FILENAME in declared_files
    ):
        raise HarnessError("baseline-oracle declared inventory is malformed")

    def validate_relative(relative: Any, context: str) -> str:
        if not isinstance(relative, str) or not relative:
            raise HarnessError(f"{context} is not a non-empty relative path")
        pure = pathlib.PurePosixPath(relative)
        if (
            pure.is_absolute()
            or pure.as_posix() != relative
            or any(part in ("", ".", "..") for part in pure.parts)
        ):
            raise HarnessError(f"{context} is unsafe: {relative!r}")
        return relative

    for index, relative in enumerate(declared_directories):
        validate_relative(relative, f"baseline-oracle directory {index}")
    for relative, digest in declared_files.items():
        validate_relative(relative, "baseline-oracle file")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise HarnessError(f"baseline-oracle file hash is malformed: {relative}")

    actual_directories = []
    actual_files: Dict[str, str] = {}
    total_file_bytes = 0
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise HarnessError(f"baseline-oracle pack contains a symlink: {relative}")
        if path.is_dir():
            actual_directories.append(relative)
        elif path.is_file():
            if relative == BASELINE_ORACLE_MANIFEST_FILENAME:
                continue
            actual_files[relative] = sha256_file(path)
            total_file_bytes += path.stat().st_size
        else:
            raise HarnessError(f"baseline-oracle pack contains a special entry: {relative}")
    if actual_directories != declared_directories or actual_files != declared_files:
        raise HarnessError("baseline-oracle exact file/directory inventory changed")
    inventory_payload = {
        "directories": actual_directories,
        "files": actual_files,
    }
    if (
        total_file_bytes != inventory["total_file_bytes"]
        or sha256_object(inventory_payload) != inventory["manifest_sha256"]
        or inventory["manifest_sha256"] != contract["inventory_manifest_sha256"]
    ):
        raise HarnessError("baseline-oracle inventory bytes or canonical hash changed")

    recipe = manifest["recipe"]
    policy = _exact_keys(
        recipe["capture_process_policy"],
        (
            "producer",
            "asset_root",
            "fresh_process_per_png",
            "runtime_data_policy",
            "log_policy",
            "common_environment",
            "explicitly_absent_environment",
            "per_camera_environment",
        ),
        "baseline-oracle capture-process policy",
    )
    expected_common_environment = {
        "BEVY_ASSET_ROOT": "asset-stages/balanced-noon-haze-0003/asset-root",
        "HEX_GAME_DATA_DIR": "$RUN_RUNTIME_DATA_DIRECTORY",
        "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT": "1",
        "HEX_REVIEW_CAMERA": "map",
        "HEX_REVIEW_CAPTURE": "$RUN_CAPTURE_PATH",
        "HEX_REVIEW_LIQUID_PHASE": "0.0",
        "HEX_REVIEW_SCENARIO": SCENARIO,
        "HEX_REVIEW_SEED": str(SEED),
        "HEX_REVIEW_TIME": "12",
        "RUST_LOG": "warn",
    }
    expected_absent_environment = [
        "HEX_REVIEW_CHARACTER_RADIUS_SCALE",
        "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE",
        "HEX_REVIEW_CUTAWAY",
        "HEX_REVIEW_EDGE",
        "HEX_REVIEW_FOCUS_ANCHOR",
        "HEX_REVIEW_FOG",
        "HEX_REVIEW_ILLUMINATION",
        "HEX_REVIEW_MATERIAL",
        "HEX_REVIEW_WORLD_DETAIL",
    ]
    _, focused_cameras, _ = load_camera_sets()
    expected_per_camera_environment = {
        camera.id: {
            "HEX_REVIEW_LOOK_AT_ANCHOR": camera.look_at_anchor,
            "HEX_REVIEW_LOOK_AT_OFFSET": ",".join(
                str(int(component)) if float(component).is_integer() else str(component)
                for component in (camera.look_at_offset or ())
            ),
            "HEX_REVIEW_VIEW": camera.view,
        }
        for camera in focused_cameras
    }
    if (
        policy["producer"] != "producer/hex_game-bc06-map-review"
        or policy["asset_root"]
        != "asset-stages/balanced-noon-haze-0003/asset-root"
        or policy["fresh_process_per_png"] is not True
        or policy["common_environment"] != expected_common_environment
        or policy["explicitly_absent_environment"] != expected_absent_environment
        or policy["per_camera_environment"] != expected_per_camera_environment
        or recipe["structural_draft_environment"]
        != {STRUCTURAL_DRAFT_ENVIRONMENT: STRUCTURAL_DRAFT_VALUE}
    ):
        raise HarnessError("baseline-oracle capture environment recipe changed")
    palette = _exact_keys(
        recipe["palette"],
        ("id", "candidate_path", "candidate_sha256", "staged_path", "staged_sha256"),
        "baseline-oracle palette recipe",
    )
    lighting = _exact_keys(
        recipe["lighting"],
        ("id", "mode", "staged_path", "staged_sha256"),
        "baseline-oracle lighting recipe",
    )
    control_profile_recipe = _exact_keys(
        recipe["control_profile"],
        ("edge", "material", "fog", "crystal_light", "illumination", "review_world_detail"),
        "baseline-oracle control-profile recipe",
    )
    if (
        palette["id"] != "p02-high-separation"
        or palette["staged_path"] != "assets/art/palette.ron"
        or lighting["id"] != "balanced-noon-haze-0003"
        or lighting["mode"] != "cycle"
        or lighting["staged_path"] != "assets/config/lighting.ron"
        or not all("absent" in value or "current" in value for value in control_profile_recipe.values())
    ):
        raise HarnessError("baseline-oracle presentation recipe changed")
    camera_provenance = recipe["camera_manifest_provenance"]
    if not isinstance(camera_provenance, list) or len(camera_provenance) != 3:
        raise HarnessError("baseline-oracle camera provenance is malformed")
    camera_source_sha256 = {
        row.get("id"): row.get("sha256")
        for row in camera_provenance
        if isinstance(row, dict)
    }
    if camera_source_sha256 != {
        "grand-v3-visual-profiles-v1": ESTABLISHED_CAMERA_SOURCE_SHA256[
            "tools/visual_experiments/profiles.json"
        ],
        "final-17-cameras-v2": sha256_file(FINAL_CAMERAS_PATH),
        "focused-4-cameras-v1": sha256_file(FOCUSED_CAMERAS_PATH),
    }:
        raise HarnessError("baseline-oracle camera manifest hashes changed")

    asset_stage = _exact_keys(
        manifest["asset_stage"],
        (
            "path",
            "manifest_path",
            "manifest_sha256",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
            "regular_file_count",
            "modified_assets",
            "no_symlinks",
            "tree_hash_convention",
        ),
        "baseline-oracle asset stage",
    )
    stage_manifest_path = root / validate_relative(
        asset_stage["manifest_path"], "baseline-oracle stage manifest path"
    )
    validate_relative(asset_stage["path"], "baseline-oracle asset-stage path")
    stage_manifest = _exact_keys(
        _read_json(stage_manifest_path, "baseline-oracle asset-stage manifest"),
        (
            "version",
            "warning",
            "asset_stage",
            "lighting_condition",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
            "modified_assets",
        ),
        "baseline-oracle asset-stage manifest",
    )
    stage_prefix = f"{asset_stage['path']}/assets/"
    stage_file_hashes = {
        relative[len(stage_prefix) :]: digest
        for relative, digest in actual_files.items()
        if relative.startswith(stage_prefix)
    }
    stage_digest = hashlib.sha256()
    for relative, digest in sorted(stage_file_hashes.items()):
        encoded = relative.encode("utf-8")
        stage_digest.update(struct.pack(">I", len(encoded)))
        stage_digest.update(encoded)
        stage_digest.update(bytes.fromhex(digest))
    if (
        asset_stage["no_symlinks"] is not True
        or sha256_file(stage_manifest_path) != asset_stage["manifest_sha256"]
        or stage_manifest["version"] != 1
        or stage_manifest["warning"] != WARNING
        or stage_manifest["asset_stage"] != LIGHTING_CONDITIONS["neutral"].asset_stage
        or stage_manifest["lighting_condition"]
        != dataclasses.asdict(LIGHTING_CONDITIONS["neutral"])
        or stage_manifest["source_asset_tree_sha256"]
        != asset_stage["source_asset_tree_sha256"]
        or stage_manifest["staged_asset_tree_sha256"]
        != asset_stage["staged_asset_tree_sha256"]
        or stage_manifest["modified_assets"] != asset_stage["modified_assets"]
        or {
            row["path"]: row["staged_sha256"]
            for row in asset_stage["modified_assets"]
        }
        != {
            palette["staged_path"]: palette["staged_sha256"],
            lighting["staged_path"]: lighting["staged_sha256"],
        }
        or len(stage_file_hashes) != asset_stage["regular_file_count"]
        or stage_digest.hexdigest() != asset_stage["staged_asset_tree_sha256"]
    ):
        raise HarnessError("baseline-oracle asset-stage evidence changed")

    producer = _exact_keys(
        manifest["producer"],
        (
            "build_command",
            "features",
            "target_triple",
            "rustc",
            "cargo",
            "platform",
            "binary_format",
            "binary_lc_uuid",
            "files",
            "artifact_receipt",
            "dependency_file",
            "runtime_smoke_log",
            "dependency_path_aliases",
            "missing_evidence",
        ),
        "baseline-oracle producer",
    )
    producer_files = _strict_object(
        producer["files"],
        context="baseline-oracle producer files",
        required=(
            "producer/bin-hex_game.json",
            "producer/hex_game-bc06-map-review",
            "producer/hex_game.d",
            "producer/hex_game.log",
        ),
    )
    for relative, raw in producer_files.items():
        row = _exact_keys(
            raw,
            ("bytes", "sha256", "executable"),
            f"baseline-oracle producer file {relative}",
        )
        path = root / relative
        executable = bool(path.stat().st_mode & stat.S_IXUSR)
        if (
            isinstance(row["bytes"], bool)
            or not isinstance(row["bytes"], int)
            or row["bytes"] <= 0
            or row["bytes"] != path.stat().st_size
            or row["sha256"] != actual_files.get(relative)
            or not isinstance(row["executable"], bool)
            or row["executable"] != executable
        ):
            raise HarnessError(f"baseline-oracle producer receipt changed: {relative}")
    if (
        producer["build_command"]
        != [
            "cargo",
            "build",
            "--locked",
            "--release",
            "-p",
            "hex_game",
            "--features",
            "map-review",
        ]
        or producer["features"] != ["default", "map-review"]
        or producer["target_triple"] != "aarch64-apple-darwin"
        or producer["artifact_receipt"] != "producer/bin-hex_game.json"
        or producer["dependency_file"] != "producer/hex_game.d"
        or producer["runtime_smoke_log"] != "producer/hex_game.log"
        or not isinstance(producer["missing_evidence"], list)
        or not producer["missing_evidence"]
    ):
        raise HarnessError("baseline-oracle producer provenance changed")

    stability = _exact_keys(
        manifest["stability_contract"],
        (
            "method",
            "acceptance",
            "coordinate_convention",
            "decoded_rgb_hash_convention",
            "evidence_hash_convention",
            "modified_worktree_samples_allowed",
            "broad_numeric_thresholds_allowed",
            "raw_png_byte_identity_satisfiable",
            "clean_run_counts",
            "per_camera_ambiguous_pixels",
            "camera_evidence_sha256",
            "stability_evidence_sha256",
            "total_ambiguous_pixels",
        ),
        "baseline-oracle stability contract",
    )
    camera_evidence = {}
    primary_paths = {}
    total_runs = 0
    total_ambiguous = 0
    focused_by_id = {camera.id: camera for camera in focused_cameras}
    for camera_id in BASELINE_ORACLE_CAMERA_IDS:
        camera = _exact_keys(
            manifest["cameras"][camera_id],
            (
                "definition",
                "runs",
                "run_count",
                "ambiguous_pixels",
                "stable_pixel_count",
                "clean_run_evidence_sha256",
            ),
            f"baseline-oracle camera {camera_id}",
        )
        runs = camera["runs"]
        ambiguous_pixels = camera["ambiguous_pixels"]
        camera_spec = focused_by_id[camera_id]
        definition = _exact_keys(
            camera["definition"],
            (
                "camera",
                "view",
                "filename",
                "look_at_anchor",
                "look_at_offset",
                "source_manifest_ids",
            ),
            f"baseline-oracle camera definition {camera_id}",
        )
        if (
            not isinstance(runs, list)
            or camera["run_count"] != len(runs)
            or camera["run_count"] < 3
            or not isinstance(ambiguous_pixels, list)
            or camera["stable_pixel_count"]
            != CAPTURE_WIDTH * CAPTURE_HEIGHT - len(ambiguous_pixels)
            or stability["clean_run_counts"].get(camera_id) != len(runs)
            or stability["per_camera_ambiguous_pixels"].get(camera_id)
            != len(ambiguous_pixels)
            or definition
            != {
                "camera": camera_spec.camera,
                "view": camera_spec.view,
                "filename": camera_spec.filename,
                "look_at_anchor": camera_spec.look_at_anchor,
                "look_at_offset": list(camera_spec.look_at_offset or ()),
                "source_manifest_ids": (
                    ["grand-v3-visual-profiles-v1"]
                    if camera_id in ("02-highlands-oblique", "03-coast-river-outlet")
                    else []
                )
                + ["final-17-cameras-v2", "focused-4-cameras-v1"],
            }
        ):
            raise HarnessError(f"baseline-oracle camera run counts changed: {camera_id}")
        total_runs += len(runs)
        total_ambiguous += len(ambiguous_pixels)
        normalized_runs = []
        observed_values: Dict[Tuple[int, int], set] = defaultdict(set)
        primary_path = None
        for index, raw_run in enumerate(runs):
            run = _exact_keys(
                raw_run,
                (
                    "run_id",
                    "role",
                    "path",
                    "png_sha256",
                    "decoded_rgb_sha256",
                    "log_path",
                    "log_sha256",
                    "runtime_data_path",
                    "runtime_data_sha256",
                ),
                f"baseline-oracle {camera_id} run {index}",
            )
            path = root / validate_relative(run["path"], "baseline-oracle capture path")
            log_path = root / validate_relative(
                run["log_path"], "baseline-oracle capture log path"
            )
            runtime_data_path = root / validate_relative(
                run["runtime_data_path"], "baseline-oracle runtime-data path"
            )
            if (
                run["png_sha256"] != actual_files.get(run["path"])
                or run["decoded_rgb_sha256"] != _decoded_rgb_bytes_sha256(path)
                or run["log_sha256"] != actual_files.get(run["log_path"])
                or run["runtime_data_sha256"] != actual_files.get(run["runtime_data_path"])
                or sha256_file(log_path) != run["log_sha256"]
                or sha256_file(runtime_data_path) != run["runtime_data_sha256"]
            ):
                raise HarnessError(f"baseline-oracle run evidence changed: {camera_id}/{index}")
            if index == 0:
                if run["role"] != "primary_reference":
                    raise HarnessError(f"baseline-oracle primary run moved: {camera_id}")
                primary_path = path
            elif run["role"] != "diagnostic_clean_source_stability":
                raise HarnessError(f"baseline-oracle diagnostic role changed: {camera_id}/{index}")
            normalized_runs.append(
                {
                    "path": run["path"],
                    "png_sha256": run["png_sha256"],
                    "decoded_rgb_sha256": run["decoded_rgb_sha256"],
                }
            )
        if primary_path is None:
            raise HarnessError(f"baseline-oracle camera lacks a primary run: {camera_id}")
        for run in runs:
            comparison = _compare_raster_stable_rgb(
                primary_path,
                root / run["path"],
                camera_id=camera_id,
                ambiguous_pixels=ambiguous_pixels,
                context="clean-source oracle stability evidence",
            )
            for row in comparison["ambiguous_values"]:
                coordinate = (row["x"], row["y"])
                observed_values[coordinate].add(tuple(row["candidate_rgb"]))
        for raw_pixel in ambiguous_pixels:
            coordinate = (raw_pixel["x"], raw_pixel["y"])
            allowed = {tuple(value) for value in raw_pixel["allowed_rgb"]}
            if observed_values[coordinate] != allowed:
                raise HarnessError(
                    f"baseline-oracle ambiguity is not exactly observed: {camera_id}/{coordinate}"
                )
        evidence_payload = {
            "camera_id": camera_id,
            "resolution": {"width": CAPTURE_WIDTH, "height": CAPTURE_HEIGHT},
            "runs": normalized_runs,
            "ambiguous_pixels": ambiguous_pixels,
        }
        evidence_sha256 = sha256_object(evidence_payload)
        if (
            evidence_sha256 != camera["clean_run_evidence_sha256"]
            or evidence_sha256 != stability["camera_evidence_sha256"].get(camera_id)
        ):
            raise HarnessError(f"baseline-oracle camera evidence hash changed: {camera_id}")
        camera_evidence[camera_id] = evidence_sha256
        primary_paths[camera_id] = str(primary_path)
    if (
        total_runs
        != EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS
        + EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
        or total_ambiguous != stability["total_ambiguous_pixels"]
        or stability["total_ambiguous_pixels"] != 8
        or sha256_object({"camera_evidence_sha256": camera_evidence})
        != stability["stability_evidence_sha256"]
    ):
        raise HarnessError("baseline-oracle overall stability evidence changed")
    body = {
        "version": 1,
        "warning": WARNING,
        "pack_id": contract["pack_id"],
        "contract_sha256": sha256_file(BASELINE_ORACLE_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": inventory["manifest_sha256"],
        "source_git_head": manifest["source"]["git_head"],
        "source_git_tree": manifest["source"]["git_tree"],
        "recipe_sha256": sha256_object(manifest["recipe"]),
        "asset_stage_sha256": sha256_object(asset_stage),
        "producer_sha256": sha256_object(producer),
        "producer_executable_sha256": producer_files[
            "producer/hex_game-bc06-map-review"
        ]["sha256"],
        "stability_evidence_sha256": stability["stability_evidence_sha256"],
        "camera_evidence_sha256": camera_evidence,
        "primary_paths": primary_paths,
        "regular_file_count": len(actual_files),
        "directory_count": len(actual_directories),
        "total_file_bytes": total_file_bytes,
        "primary_render_count": EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS,
        "stability_diagnostic_render_count": (
            EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
        ),
        "all_declared_files_hash_verified": True,
        "exact_inventory_verified": True,
        "no_symlinks_verified": True,
    }
    return {**body, "validation_sha256": sha256_object(body)}


def _validate_control_equivalence_qualification_pack(
    root: pathlib.Path = CONTROL_EQUIVALENCE_QUALIFICATION_ROOT,
) -> Dict[str, Any]:
    """Validate the immutable six-run qualification for one control-path pixel.

    This evidence is intentionally separate from BaselineOraclePackV1. It can
    authorize an exact endpoint pair only when comparing the omitted profile to
    explicit-current; it cannot change clean-source oracle or reproduction masks.
    """

    contract, manifest = _control_equivalence_documents(root)
    root = root.resolve()
    inventory = _exact_keys(
        manifest["inventory"],
        (
            "directories",
            "directory_count",
            "files",
            "regular_file_count",
            "total_file_bytes",
            "manifest_excluded_path",
            "manifest_sha256",
            "no_symlinks",
            "entry_policy",
            "hash_convention",
        ),
        "control-equivalence qualification inventory",
    )
    declared_directories = inventory["directories"]
    declared_files = inventory["files"]
    if (
        inventory["no_symlinks"] is not True
        or inventory["manifest_excluded_path"]
        != CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME
        or not isinstance(declared_directories, list)
        or declared_directories != sorted(declared_directories)
        or len(declared_directories) != len(set(declared_directories))
        or inventory["directory_count"] != len(declared_directories)
        or not isinstance(declared_files, dict)
        or list(declared_files) != sorted(declared_files)
        or inventory["regular_file_count"] != len(declared_files)
        or CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME in declared_files
    ):
        raise HarnessError("control-equivalence qualification inventory is malformed")

    def validate_relative(relative: Any, context: str) -> str:
        if not isinstance(relative, str) or not relative:
            raise HarnessError(f"{context} is not a non-empty relative path")
        pure = pathlib.PurePosixPath(relative)
        if (
            pure.is_absolute()
            or pure.as_posix() != relative
            or any(part in ("", ".", "..") for part in pure.parts)
        ):
            raise HarnessError(f"{context} is unsafe: {relative!r}")
        return relative

    for index, relative in enumerate(declared_directories):
        validate_relative(relative, f"control-equivalence directory {index}")
    for relative, digest in declared_files.items():
        validate_relative(relative, "control-equivalence file")
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise HarnessError(f"control-equivalence file hash is malformed: {relative}")

    actual_directories = []
    actual_files: Dict[str, str] = {}
    total_file_bytes = 0
    for path in sorted(root.rglob("*"), key=lambda item: item.relative_to(root).as_posix()):
        relative = path.relative_to(root).as_posix()
        if path.is_symlink():
            raise HarnessError(f"control-equivalence qualification contains a symlink: {relative}")
        if path.is_dir():
            actual_directories.append(relative)
        elif path.is_file():
            if relative == CONTROL_EQUIVALENCE_QUALIFICATION_MANIFEST_FILENAME:
                continue
            actual_files[relative] = sha256_file(path)
            total_file_bytes += path.stat().st_size
        else:
            raise HarnessError(
                f"control-equivalence qualification contains a special entry: {relative}"
            )
    if actual_directories != declared_directories or actual_files != declared_files:
        raise HarnessError("control-equivalence qualification exact inventory changed")
    if (
        total_file_bytes != inventory["total_file_bytes"]
        or sha256_object(
            {"directories": actual_directories, "files": actual_files}
        )
        != inventory["manifest_sha256"]
        or inventory["manifest_sha256"] != contract["inventory_manifest_sha256"]
    ):
        raise HarnessError("control-equivalence qualification inventory hash changed")

    runtime_root = CONTROL_EQUIVALENCE_QUALIFICATION_ROOT.parent.resolve()
    stage = manifest["asset_stage"]
    stage_relative = validate_relative(
        stage["external_manifest_path"],
        "control-equivalence external asset-stage manifest",
    )
    stage_path = (runtime_root / stage_relative).resolve()
    try:
        stage_path.relative_to(runtime_root)
    except ValueError as error:
        raise HarnessError("control-equivalence asset-stage manifest escaped runtime root") from error
    if (
        not stage_path.is_file()
        or stage_path.is_symlink()
        or sha256_file(stage_path) != stage["external_manifest_sha256"]
    ):
        raise HarnessError("control-equivalence asset-stage manifest is unavailable or changed")
    stage_document = _exact_keys(
        _read_json(stage_path, "control-equivalence asset-stage manifest"),
        (
            "version",
            "warning",
            "asset_stage",
            "lighting_condition",
            "source_asset_tree_sha256",
            "staged_asset_tree_sha256",
            "modified_assets",
        ),
        "control-equivalence asset-stage manifest",
    )
    if (
        stage_document["version"] != 1
        or stage_document["warning"] != WARNING
        or stage_document["asset_stage"] != stage["id"]
        or stage_document["source_asset_tree_sha256"]
        != stage["source_asset_tree_sha256"]
        or stage_document["staged_asset_tree_sha256"]
        != stage["staged_asset_tree_sha256"]
    ):
        raise HarnessError("control-equivalence asset-stage provenance changed")

    stable = manifest["stable_report_contract"]
    expected_capture_fields = [
        "camera",
        "character_radius_scale",
        "focus_anchor",
        "full_cutaway",
        "illumination_overlay",
        "liquid_phase_seconds",
        "look_at_anchor",
        "look_at_offset",
        "settle_frames",
        "time_hours",
        "view",
    ]
    expected_report_fields = [
        "version",
        "profile_hash_sha256",
        "authority",
        "counts",
        "anchor_heights",
        "anchor_classes",
        "projection_hashes",
        "effect_validation",
        "camera_features",
        "cleanup",
    ]
    if (
        stable["capture_fields"] != expected_capture_fields
        or stable["report_fields"] != expected_report_fields
        or stable["excluded_capture_fields"] != ["path"]
        or stable["excluded_report_fields"] != ["runtime_receipt", "performance"]
    ):
        raise HarnessError("control-equivalence stable-report field contract changed")

    _, oracle_manifest = _baseline_oracle_documents()
    oracle_pixels = oracle_manifest["cameras"][contract["camera_id"]][
        "ambiguous_pixels"
    ]
    qualified_pixels = contract["qualified_pixels"]
    oracle_coordinates = {(row["x"], row["y"]) for row in oracle_pixels}
    qualified_coordinates = {(row["x"], row["y"]) for row in qualified_pixels}
    if oracle_coordinates & qualified_coordinates:
        raise HarnessError("control-equivalence qualification overlaps clean oracle coordinates")
    combined_pixels = [copy.deepcopy(row) for row in oracle_pixels] + [
        {field: copy.deepcopy(row[field]) for field in ("x", "y", "allowed_rgb")}
        for row in qualified_pixels
    ]

    pairs = manifest["pairs"]
    if not isinstance(pairs, list) or len(pairs) != contract["pair_count"]:
        raise HarnessError("control-equivalence qualification pair count changed")
    receipts = []
    observation_by_coordinate: Dict[Tuple[int, int], List[Dict[str, Any]]] = {
        coordinate: [] for coordinate in qualified_coordinates
    }
    for pair_index, raw_pair in enumerate(pairs, start=1):
        pair = _exact_keys(
            raw_pair,
            (
                "pair_id",
                "omitted",
                "explicit",
                "stable_capture_sha256",
                "stable_report_sha256",
                "stable_report_equal",
                "differing_pixels",
            ),
            f"control-equivalence pair {pair_index}",
        )
        pair_id = f"pair-{pair_index:02}"
        if (
            pair["pair_id"] != pair_id
            or pair["stable_capture_sha256"] != stable["capture_sha256"]
            or pair["stable_report_sha256"] != stable["report_sha256"]
            or pair["stable_report_equal"] is not True
        ):
            raise HarnessError(f"control-equivalence qualification pair identity changed: {pair_id}")
        run_documents = {}
        run_paths = {}
        for mode, profile_mode in (("omitted", "omitted"), ("explicit", "explicit-current")):
            run = _exact_keys(
                pair[mode],
                (
                    "directory",
                    "profile_mode",
                    "png_path",
                    "decoded_rgb_sha256",
                    "report_path",
                    "runtime_receipt",
                ),
                f"control-equivalence {pair_id} {mode}",
            )
            expected_directory = f"run-{pair_index:02}-{mode}"
            expected_png = f"{expected_directory}/{contract['camera_id']}.png"
            expected_report = (
                f"{expected_directory}/{contract['camera_id']}.world-detail-report.json"
            )
            if (
                run["directory"] != expected_directory
                or run["profile_mode"] != profile_mode
                or run["png_path"] != expected_png
                or run["report_path"] != expected_report
            ):
                raise HarnessError(f"control-equivalence run paths changed: {pair_id}/{mode}")
            png_path = root / validate_relative(run["png_path"], "qualification PNG path")
            report_path = root / validate_relative(
                run["report_path"], "qualification report path"
            )
            if (
                actual_files.get(run["png_path"]) != sha256_file(png_path)
                or actual_files.get(run["report_path"]) != sha256_file(report_path)
                or run["decoded_rgb_sha256"] != _decoded_rgb_bytes_sha256(png_path)
            ):
                raise HarnessError(f"control-equivalence run evidence changed: {pair_id}/{mode}")
            wrapper = _exact_keys(
                _read_json(report_path, "control-equivalence world-detail report"),
                ("version", "warning", "capture", "report"),
                "control-equivalence world-detail report",
            )
            capture = _strict_object(
                wrapper["capture"],
                context="control-equivalence report capture",
                required=(*expected_capture_fields, "path"),
            )
            report = _strict_object(
                wrapper["report"],
                context="control-equivalence report payload",
                required=(*expected_report_fields, "runtime_receipt", "performance"),
            )
            stable_capture = {field: capture[field] for field in expected_capture_fields}
            stable_report = {field: report[field] for field in expected_report_fields}
            if (
                wrapper["version"] != 1
                or wrapper["warning"] != WARNING
                or sha256_object(stable_capture) != stable["capture_sha256"]
                or sha256_object(stable_report) != stable["report_sha256"]
            ):
                raise HarnessError(f"control-equivalence stable report changed: {pair_id}/{mode}")
            receipt = _validate_runtime_receipt_hash_only(
                report["runtime_receipt"],
                source_provenance_sha256=manifest["source"]["source_provenance_sha256"],
                profile_sha256=manifest["recipe"]["control_profile_sha256"],
            )
            if receipt != run["runtime_receipt"]:
                raise HarnessError(f"control-equivalence receipt binding changed: {pair_id}/{mode}")
            receipts.append(receipt)
            run_documents[mode] = stable_report
            run_paths[mode] = png_path
        if run_documents["omitted"] != run_documents["explicit"]:
            raise HarnessError(f"control-equivalence stable reports differ: {pair_id}")
        comparison = _compare_raster_stable_rgb(
            run_paths["omitted"],
            run_paths["explicit"],
            camera_id=contract["camera_id"],
            ambiguous_pixels=combined_pixels,
            context="qualified omitted versus explicit-current control",
        )
        actual_differences = []
        for row in comparison["ambiguous_values"]:
            coordinate = (row["x"], row["y"])
            if coordinate in qualified_coordinates:
                observation_by_coordinate[coordinate].append(
                    {
                        "pair_id": pair_id,
                        "omitted_rgb": row["reference_rgb"],
                        "explicit_rgb": row["candidate_rgb"],
                    }
                )
            if row["equal"]:
                continue
            actual_differences.append(
                {
                    "x": row["x"],
                    "y": row["y"],
                    "omitted_rgb": row["reference_rgb"],
                    "explicit_rgb": row["candidate_rgb"],
                    "classification": (
                        "control-equivalence-qualified"
                        if coordinate in qualified_coordinates
                        else "baseline-oracle-known"
                    ),
                }
            )
        if actual_differences != pair["differing_pixels"]:
            raise HarnessError(f"control-equivalence exact raster differences changed: {pair_id}")

    manifest_qualified_by_coordinate = {}
    for row in manifest["qualified_pixels"]:
        coordinate = (row["x"], row["y"])
        manifest_qualified_by_coordinate[coordinate] = row
    for qualified in qualified_pixels:
        coordinate = (qualified["x"], qualified["y"])
        manifest_row = manifest_qualified_by_coordinate.get(coordinate)
        if (
            manifest_row is None
            or manifest_row["observations"] != observation_by_coordinate[coordinate]
            or manifest_row["allowed_rgb"] != qualified["allowed_rgb"]
            or not any(
                row["omitted_rgb"] != row["explicit_rgb"]
                for row in observation_by_coordinate[coordinate]
            )
        ):
            raise HarnessError("control-equivalence qualified RGB observations changed")
    if (
        len(receipts) != contract["run_count"]
        or len({row["launch_nonce"] for row in receipts}) != contract["run_count"]
        or len({row["receipt_sha256"] for row in receipts}) != contract["run_count"]
        or len({row["process_id"] for row in receipts}) != contract["run_count"]
        or {row["source_provenance_sha256"] for row in receipts}
        != {contract["source_provenance_sha256"]}
        or {row["executable_sha256"] for row in receipts}
        != {contract["executable_sha256"]}
        or {row["profile_sha256"] for row in receipts}
        != {manifest["recipe"]["control_profile_sha256"]}
    ):
        raise HarnessError("control-equivalence qualification process isolation changed")

    body = {
        "version": 1,
        "warning": WARNING,
        "pack_id": contract["pack_id"],
        "scope": contract["scope"],
        "contract_sha256": sha256_file(CONTROL_EQUIVALENCE_RASTER_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": contract["inventory_manifest_sha256"],
        "qualification_evidence_sha256": contract["qualification_evidence_sha256"],
        "source_provenance_sha256": contract["source_provenance_sha256"],
        "producer_executable_sha256": contract["executable_sha256"],
        "asset_stage": contract["asset_stage"],
        "qualified_pixels": contract["qualified_pixels"],
        "pair_count": len(pairs),
        "run_count": len(receipts),
        "stable_capture_sha256": contract["stable_capture_sha256"],
        "stable_report_sha256": contract["stable_report_sha256"],
        "regular_file_count": len(actual_files),
        "directory_count": len(actual_directories),
        "total_file_bytes": total_file_bytes,
        "all_declared_files_hash_verified": True,
        "exact_inventory_verified": True,
        "no_symlinks_verified": True,
        "baseline_oracle_contract_unchanged": True,
        "broad_numeric_threshold_used": False,
    }
    return {**body, "validation_sha256": sha256_object(body)}


def _control_equivalence_ambiguous_pixels(
    camera_id: str,
    oracle_pixels: Sequence[Mapping[str, Any]],
    *,
    qualification_validation: Optional[Mapping[str, Any]] = None,
) -> List[Dict[str, Any]]:
    """Return the exact mask used only for omitted↔explicit-current controls."""

    validation = (
        qualification_validation
        if qualification_validation is not None
        else _validate_control_equivalence_qualification_pack()
    )
    qualified = [
        row
        for row in validation["qualified_pixels"]
        if row["camera_id"] == camera_id
    ]
    result = [copy.deepcopy(dict(row)) for row in oracle_pixels]
    coordinates = {(row["x"], row["y"]) for row in result}
    for row in qualified:
        coordinate = (row["x"], row["y"])
        if coordinate in coordinates:
            raise HarnessError("control-equivalence mask overlaps baseline-oracle ambiguity")
        coordinates.add(coordinate)
        result.append(
            {
                "x": row["x"],
                "y": row["y"],
                "allowed_rgb": copy.deepcopy(row["allowed_rgb"]),
            }
        )
    return result


def _baseline_oracle_control_jobs(
    jobs: Sequence[Mapping[str, Any]],
) -> Tuple[Mapping[str, Any], ...]:
    """Resolve the four one-camera omitted-profile oracle-control jobs."""

    resolved = tuple(
        job
        for job in jobs
        if job.get("stage") == "00-shared-control"
        and job.get("look_id") == "control"
        and job.get("lighting") == "neutral"
        and "-focused-" in str(job.get("id", ""))
    )
    if len(resolved) != len(BASELINE_ORACLE_CAMERA_IDS):
        raise HarnessError("capture plan must contain four focused oracle-control jobs")
    expected_fields = {
        "kind": "still",
        "profile_sha256": CONTROL_PROFILE_SHA256,
        "profile_json": control_profile().canonical_json,
        "control_profile_omitted": True,
        "asset_stage": LIGHTING_CONDITIONS["neutral"].asset_stage,
        "time_hours": 12.0,
        "liquid_phase_seconds": 0.0,
    }
    _, focused_cameras, _ = load_camera_sets()
    focused_by_id = {camera.id: camera for camera in focused_cameras}
    camera_ids = []
    for job in resolved:
        for field, expected in expected_fields.items():
            if job.get(field) != expected:
                raise HarnessError(
                    f"focused oracle-control job changed recipe field {field}"
                )
        cameras = job.get("cameras")
        captures = job.get("capture_plan", {}).get("captures")
        artifacts = job.get("artifacts")
        if (
            not isinstance(cameras, list)
            or len(cameras) != 1
            or not isinstance(captures, list)
            or len(captures) != 1
            or not isinstance(artifacts, list)
            or len(artifacts) != 1
        ):
            raise HarnessError(
                "each focused oracle-control job must contain exactly one camera and PNG"
            )
        camera_id = cameras[0].get("id") if isinstance(cameras[0], dict) else None
        camera = (
            _camera_from_record(
                cameras[0], f"focused oracle-control camera {camera_id}"
            )
            if camera_id in focused_by_id
            else None
        )
        # JSON round-tripping converts the optional look-at tuple to a list.
        # Compare the decoded semantic camera rather than Python container types.
        if camera is None or camera != focused_by_id[camera_id]:
            raise HarnessError("focused oracle-control camera definition changed")
        expected_id = (
            "still-00-shared-control-control-neutral-focused-"
            f"{camera_id}-{CONTROL_PROFILE_SHA256[:12]}"
        )
        if job.get("id") != expected_id:
            raise HarnessError(f"focused oracle-control job id changed for {camera_id}")
        camera_ids.append(camera_id)
    if tuple(camera_ids) != BASELINE_ORACLE_CAMERA_IDS:
        raise HarnessError("focused oracle-control job order changed")
    return resolved


def _validated_control_record_receipt(
    job: Mapping[str, Any], record: Mapping[str, Any]
) -> Dict[str, Any]:
    """Recheck the already-validated receipt fields used by oracle aggregation."""

    raw = record.get("runtime_receipt")
    if not isinstance(raw, dict):
        raise HarnessError(f"control record lacks a runtime receipt: {job['id']}")
    receipt = _exact_keys(
        raw,
        (*RUNTIME_RECEIPT_FIELDS, "receipt_sha256"),
        f"control runtime receipt {job['id']}",
    )
    capture_plan_json = compact_json(job["capture_plan"])
    return _validate_runtime_receipt(
        receipt,
        source_provenance_sha256=receipt["source_provenance_sha256"],
        capture_plan_json=capture_plan_json,
        profile_sha256=CONTROL_PROFILE_SHA256,
        expected_launch_nonce=receipt["launch_nonce"],
    )


def _validate_baseline_oracle_equivalence(
    jobs: Sequence[Mapping[str, Any]],
    records_by_job: Mapping[str, Mapping[str, Any]],
    *,
    pack_validation: Optional[Mapping[str, Any]] = None,
) -> Optional[Dict[str, Any]]:
    """Aggregate four fresh-process controls against the clean bc06 oracle."""

    oracle_jobs = _baseline_oracle_control_jobs(jobs)
    resolved_pack_validation = dict(
        pack_validation if pack_validation is not None else _validate_baseline_oracle_pack()
    )
    validation_body = {
        key: value
        for key, value in resolved_pack_validation.items()
        if key != "validation_sha256"
    }
    if resolved_pack_validation.get("validation_sha256") != sha256_object(
        validation_body
    ):
        raise HarnessError("baseline-oracle pack validation receipt changed")
    contract, manifest = _baseline_oracle_documents()
    if (
        resolved_pack_validation.get("manifest_sha256")
        != contract["external_manifest_sha256"]
        or resolved_pack_validation.get("inventory_manifest_sha256")
        != contract["inventory_manifest_sha256"]
        or resolved_pack_validation.get("stability_evidence_sha256")
        != contract["stability_evidence_sha256"]
    ):
        raise HarnessError("baseline-oracle pack validation identity changed")

    comparisons = []
    receipts = []
    for job in oracle_jobs:
        job_id = job["id"]
        record = records_by_job.get(job_id)
        if record is None:
            continue
        pngs = record.get("pngs")
        if not isinstance(pngs, list) or len(pngs) != 1:
            raise HarnessError(
                f"focused oracle-control job lacks one validated PNG: {job_id}"
            )
        camera_id = job["cameras"][0]["id"]
        camera = manifest["cameras"][camera_id]
        primary_runs = [
            run for run in camera["runs"] if run["role"] == "primary_reference"
        ]
        if len(primary_runs) != 1:
            raise HarnessError(f"baseline oracle lacks one primary PNG at {camera_id}")
        primary = primary_runs[0]
        oracle_png = BASELINE_ORACLE_ROOT / primary["path"]
        current = pngs[0]
        current_path = pathlib.Path(current.get("path", ""))
        if not current_path.is_file() or current_path.is_symlink():
            raise HarnessError(f"current oracle-control PNG is unavailable at {camera_id}")
        comparison = _compare_raster_stable_rgb(
            oracle_png,
            current_path,
            camera_id=camera_id,
            ambiguous_pixels=camera["ambiguous_pixels"],
            context="current control versus clean-source baseline oracle",
        )
        if current.get("sha256") != comparison["candidate_png_sha256"]:
            raise HarnessError(f"validated current PNG hash changed at {camera_id}")
        receipt = _validated_control_record_receipt(job, record)
        receipts.append(receipt)
        comparisons.append(
            {
                **comparison,
                "job_id": job_id,
                "launch_nonce": receipt["launch_nonce"],
                "process_id": receipt["process_id"],
                "runtime_receipt_sha256": receipt["receipt_sha256"],
                "executable_sha256": receipt["executable_sha256"],
            }
        )
    if len(comparisons) < len(oracle_jobs):
        return None
    if (
        len({receipt["launch_nonce"] for receipt in receipts}) != len(receipts)
        or len({receipt["receipt_sha256"] for receipt in receipts}) != len(receipts)
        or len({receipt["source_provenance_sha256"] for receipt in receipts}) != 1
        or len({receipt["executable_sha256"] for receipt in receipts}) != 1
    ):
        raise HarnessError(
            "focused oracle controls do not prove four fresh launches of one source/executable"
        )
    body = {
        "version": 1,
        "warning": WARNING,
        "pack_id": contract["pack_id"],
        "contract_sha256": sha256_file(BASELINE_ORACLE_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": contract["inventory_manifest_sha256"],
        "source_git_head": contract["source_git_head"],
        "source_git_tree": contract["source_git_tree"],
        "recipe_sha256": sha256_object(manifest["recipe"]),
        "asset_stage_sha256": sha256_object(manifest["asset_stage"]),
        "producer_sha256": sha256_object(manifest["producer"]),
        "producer_executable_sha256": resolved_pack_validation[
            "producer_executable_sha256"
        ],
        "pack_validation_sha256": resolved_pack_validation["validation_sha256"],
        "stability_evidence_sha256": contract["stability_evidence_sha256"],
        "fresh_process_per_camera": True,
        "fresh_process_count": len(receipts),
        "raw_png_byte_identity_satisfiable": False,
        "raster_stable_pixel_equality": True,
        "broad_numeric_threshold_used": False,
        "comparisons": comparisons,
    }
    return {**body, "evidence_sha256": sha256_object(body)}


def _validate_current_baseline_oracle_evidence(
    evidence: Mapping[str, Any],
    jobs: Sequence[Mapping[str, Any]],
    completed: Mapping[str, Mapping[str, Any]],
    *,
    pack_validation: Mapping[str, Any],
    pinned_executable_sha256: Optional[str],
) -> None:
    """Reject resumable oracle evidence not bound to the current four-job funnel."""

    value = _exact_keys(
        evidence,
        (
            "version",
            "warning",
            "pack_id",
            "contract_sha256",
            "manifest_sha256",
            "inventory_manifest_sha256",
            "source_git_head",
            "source_git_tree",
            "recipe_sha256",
            "asset_stage_sha256",
            "producer_sha256",
            "producer_executable_sha256",
            "pack_validation_sha256",
            "stability_evidence_sha256",
            "fresh_process_per_camera",
            "fresh_process_count",
            "raw_png_byte_identity_satisfiable",
            "raster_stable_pixel_equality",
            "broad_numeric_threshold_used",
            "comparisons",
            "evidence_sha256",
        ),
        "capture state baseline-oracle evidence",
    )
    body = {key: item for key, item in value.items() if key != "evidence_sha256"}
    if value["evidence_sha256"] != sha256_object(body):
        raise HarnessError("capture state baseline-oracle evidence hash changed")
    oracle_jobs = _baseline_oracle_control_jobs(jobs)
    contract, manifest = _baseline_oracle_documents()
    expected_identity = {
        "version": 1,
        "warning": WARNING,
        "pack_id": contract["pack_id"],
        "contract_sha256": sha256_file(BASELINE_ORACLE_CONTRACT_PATH),
        "manifest_sha256": contract["external_manifest_sha256"],
        "inventory_manifest_sha256": contract["inventory_manifest_sha256"],
        "source_git_head": contract["source_git_head"],
        "source_git_tree": contract["source_git_tree"],
        "recipe_sha256": sha256_object(manifest["recipe"]),
        "asset_stage_sha256": sha256_object(manifest["asset_stage"]),
        "producer_sha256": sha256_object(manifest["producer"]),
        "producer_executable_sha256": pack_validation["producer_executable_sha256"],
        "pack_validation_sha256": pack_validation["validation_sha256"],
        "stability_evidence_sha256": contract["stability_evidence_sha256"],
        "fresh_process_per_camera": True,
        "fresh_process_count": len(oracle_jobs),
        "raw_png_byte_identity_satisfiable": False,
        "raster_stable_pixel_equality": True,
        "broad_numeric_threshold_used": False,
    }
    for field, expected in expected_identity.items():
        if value[field] != expected:
            raise HarnessError(
                f"capture state baseline-oracle evidence changed field {field}"
            )
    comparisons = value["comparisons"]
    if not isinstance(comparisons, list) or len(comparisons) != len(oracle_jobs):
        raise HarnessError(
            "capture state baseline-oracle evidence must contain four comparisons"
        )
    expected_ids = [job["id"] for job in oracle_jobs]
    actual_ids = [
        row.get("job_id") if isinstance(row, dict) else None for row in comparisons
    ]
    actual_cameras = [
        row.get("camera_id") if isinstance(row, dict) else None for row in comparisons
    ]
    if actual_ids != expected_ids or tuple(actual_cameras) != BASELINE_ORACLE_CAMERA_IDS:
        raise HarnessError(
            "capture state baseline-oracle evidence is not bound to current focused jobs"
        )
    launch_nonces = []
    receipt_hashes = []
    for job, row in zip(oracle_jobs, comparisons):
        if not isinstance(row, dict):
            raise HarnessError("capture state baseline-oracle comparison is malformed")
        job_id = job["id"]
        completed_row = completed.get(job_id)
        if not isinstance(completed_row, dict):
            raise HarnessError(
                f"capture state baseline-oracle evidence lacks completed job {job_id}"
            )
        if completed_row.get("job_sha256") != sha256_object(job):
            raise HarnessError(
                f"capture state baseline-oracle job definition changed for {job_id}"
            )
        candidate_sha256 = row.get("candidate_png_sha256")
        if completed_row.get("artifact_sha256") != [candidate_sha256]:
            raise HarnessError(
                f"capture state baseline-oracle artifact binding changed for {job_id}"
            )
        bindings = (
            ("launch_nonce", "launch_nonce"),
            ("process_id", "process_id"),
            ("runtime_receipt_sha256", "runtime_receipt_sha256"),
            ("executable_sha256", "executable_sha256"),
        )
        for evidence_field, completed_field in bindings:
            if row.get(evidence_field) != completed_row.get(completed_field):
                raise HarnessError(
                    f"capture state baseline-oracle process binding changed for {job_id}"
                )
        if row.get("executable_sha256") != pinned_executable_sha256:
            raise HarnessError(
                f"capture state baseline-oracle executable is not the current pin for {job_id}"
            )
        camera = manifest["cameras"][row["camera_id"]]
        primary = [run for run in camera["runs"] if run["role"] == "primary_reference"]
        if len(primary) != 1 or (
            row.get("reference_png_sha256") != primary[0]["png_sha256"]
            or row.get("reference_decoded_rgb_sha256")
            != decoded_rgb_sha256(BASELINE_ORACLE_ROOT / primary[0]["path"])
            or row.get("stable_pixel_identical") is not True
            or row.get("ambiguous_pixel_count") != len(camera["ambiguous_pixels"])
        ):
            raise HarnessError(
                f"capture state baseline-oracle raster binding changed for {job_id}"
            )
        launch_nonces.append(row.get("launch_nonce"))
        receipt_hashes.append(row.get("runtime_receipt_sha256"))
    if len(set(launch_nonces)) != len(oracle_jobs) or len(set(receipt_hashes)) != len(
        oracle_jobs
    ):
        raise HarnessError(
            "capture state baseline-oracle evidence does not prove four fresh processes"
        )


def _capture_jobs_in_fail_fast_order(
    plan: Mapping[str, Any],
    oracle_jobs: Sequence[Mapping[str, Any]],
    *,
    include_motion: bool,
) -> List[Mapping[str, Any]]:
    """Run all eight focused control processes before any treatment capture."""

    oracle_ids = {job["id"] for job in oracle_jobs}
    study_jobs = list(plan["study"]["jobs"])
    verification_jobs = list(plan["study"]["verification_jobs"])
    reproduction_jobs = list(plan["study"]["reproduction_jobs"])
    remaining_study_jobs = [job for job in study_jobs if job["id"] not in oracle_ids]
    ordered = [
        *oracle_jobs,
        *verification_jobs,
        *remaining_study_jobs,
        *reproduction_jobs,
    ]
    if include_motion:
        ordered.extend(plan.get("motion", {}).get("jobs", ()))
    expected = [
        *study_jobs,
        *verification_jobs,
        *reproduction_jobs,
        *(plan.get("motion", {}).get("jobs", ()) if include_motion else ()),
    ]
    if sorted(job["id"] for job in ordered) != sorted(job["id"] for job in expected):
        raise HarnessError("fail-fast capture ordering changed the planned job inventory")
    return ordered


def run_capture_plan(
    plan_path: pathlib.Path,
    *,
    work_root: pathlib.Path,
    max_jobs: Optional[int] = None,
    timeout_seconds: int = 1800,
    include_motion: bool = False,
) -> Dict[str, Any]:
    """Run or resume genuine Cargo map-review jobs from a generated plan."""

    work_root = _require_outside_source_tree(work_root, "capture work root")
    with _exclusive_capture_runner_lock(work_root):
        return _run_capture_plan_locked(
            plan_path,
            work_root=work_root,
            max_jobs=max_jobs,
            timeout_seconds=timeout_seconds,
            include_motion=include_motion,
        )


def _run_capture_plan_locked(
    plan_path: pathlib.Path,
    *,
    work_root: pathlib.Path,
    max_jobs: Optional[int],
    timeout_seconds: int,
    include_motion: bool,
) -> Dict[str, Any]:
    """Mutate capture state while the caller holds the work-root runner lock."""

    baseline_oracle_pack_validation = _validate_baseline_oracle_pack()
    _validate_control_equivalence_qualification_pack()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    provenance = plan["provenance"]
    source_provenance_sha256 = sha256_object(provenance)
    oracle_candidates = tuple(
        job
        for job in plan["study"]["jobs"]
        if job.get("stage") == "00-shared-control"
        and job.get("look_id") == "control"
        and job.get("lighting") == "neutral"
        and "-focused-" in str(job.get("id", ""))
    )
    oracle_plan_jobs = (
        _baseline_oracle_control_jobs(plan["study"]["jobs"])
        if oracle_candidates
        else ()
    )
    oracle_job_ids = {job["id"] for job in oracle_plan_jobs}
    still_jobs = _capture_jobs_in_fail_fast_order(
        plan,
        oracle_plan_jobs,
        include_motion=False,
    )
    all_plan_jobs = _capture_jobs_in_fail_fast_order(
        plan,
        oracle_plan_jobs,
        include_motion=True,
    )
    all_plan_job_ids = [job["id"] for job in all_plan_jobs]
    if len(all_plan_job_ids) != len(set(all_plan_job_ids)):
        raise HarnessError("capture plan repeats a job id")
    jobs = all_plan_jobs if include_motion else still_jobs
    if max_jobs is not None:
        if max_jobs < 0:
            raise HarnessError("max_jobs must be non-negative")
        jobs = jobs[:max_jobs]
    state_path = work_root / "capture-state.json"
    current_plan_sha256 = sha256_file(plan_path)
    state = (
        _read_json(state_path, "capture state")
        if state_path.exists()
        else {
            "version": 1,
            "warning": WARNING,
            "plan_sha256_history": [],
            "pinned_executable_sha256": None,
            "baseline_oracle_equivalence": None,
            "completed": {},
            "attempts": [],
        }
    )
    if state.get("version") != 1 or state.get("warning") != WARNING:
        raise HarnessError("resume state version or warning changed")
    if "frozen_prior_baseline" in state:
        raise HarnessError(
            "capture state uses the retired September-2 pixel baseline; use a fresh work root"
        )
    history = state.get("plan_sha256_history")
    if not isinstance(history, list) or any(not isinstance(item, str) or not SHA256_RE.fullmatch(item) for item in history):
        raise HarnessError("capture state plan history is malformed")
    if current_plan_sha256 not in history:
        history.append(current_plan_sha256)
    completed = state["completed"]
    if not isinstance(completed, dict):
        raise HarnessError("capture state completed field must be an object")
    foreign_completed = set(completed) - set(all_plan_job_ids)
    if foreign_completed:
        raise HarnessError(
            "capture state contains jobs absent from the current plan: "
            + ", ".join(sorted(foreign_completed))
        )
    attempts = state.get("attempts")
    if not isinstance(attempts, list) or any(not isinstance(item, dict) for item in attempts):
        raise HarnessError("capture state attempts field must be a list of objects")
    _initialize_capture_executable_pin(state, completed, attempts)
    if "baseline_oracle_equivalence" not in state:
        state["baseline_oracle_equivalence"] = None
    baseline_oracle_equivalence = state["baseline_oracle_equivalence"]
    if baseline_oracle_equivalence is not None:
        if not isinstance(baseline_oracle_equivalence, dict):
            raise HarnessError("capture state baseline-oracle evidence is malformed")
        _validate_current_baseline_oracle_evidence(
            baseline_oracle_equivalence,
            oracle_plan_jobs,
            completed,
            pack_validation=baseline_oracle_pack_validation,
            pinned_executable_sha256=state["pinned_executable_sha256"],
        )
    atomic_write(state_path, pretty_json(state))

    source_world_authority = None
    gameplay_authority: Dict[Tuple[str, Optional[float]], str] = {}
    source_anchor_heights = None
    source_anchor_classes = None
    projection_hashes_by_profile_phase: Dict[Tuple[str, float], Mapping[str, str]] = {}
    oracle_records_by_job: Dict[str, Mapping[str, Any]] = {}
    executed = 0
    for job in jobs:
        job_id = job["id"]
        job_sha256 = sha256_object(job)
        condition = LIGHTING_CONDITIONS[job["lighting"]]
        if job_id in completed:
            completed_job = completed[job_id]
            if completed_job.get("job_sha256") != job_sha256:
                raise HarnessError(f"resumed job definition changed for {job_id}")
            _, stage_binding = _validated_asset_stage_binding(work_root, condition)
            for field, expected in stage_binding.items():
                if completed_job.get(field) != expected:
                    raise HarnessError(f"resumed asset stage binding changed for {job_id}: {field}")
            launch_nonce = completed_job.get("launch_nonce")
            record = _validate_job_artifacts(
                job,
                output_root,
                source_provenance_sha256=source_provenance_sha256,
                expected_launch_nonce=launch_nonce,
                label=True,
            )
            if completed_job.get("artifact_sha256") != [item["sha256"] for item in record["pngs"]]:
                raise HarnessError(f"resume artifacts changed for {job_id}")
            receipt_executable_sha256 = _assert_capture_executable_matches_pin(
                state,
                record["runtime_receipt"]["executable_sha256"],
                context=f"resumed job {job_id}",
            )
            if completed_job.get("executable_sha256") != receipt_executable_sha256:
                raise HarnessError(f"resumed executable receipt changed for {job_id}")
            oracle_evidence = None
            if job_id in oracle_job_ids:
                oracle_records_by_job[job_id] = record
                oracle_evidence = _validate_baseline_oracle_equivalence(
                    oracle_plan_jobs,
                    oracle_records_by_job,
                    pack_validation=baseline_oracle_pack_validation,
                )
                if oracle_evidence is not None:
                    if state["baseline_oracle_equivalence"] is None:
                        state["baseline_oracle_equivalence"] = oracle_evidence
                        atomic_write(state_path, pretty_json(state))
                    elif state["baseline_oracle_equivalence"] != oracle_evidence:
                        raise HarnessError("resumed baseline-oracle evidence changed")
            source_world_authority = _record_authority(
                record["authority"],
                condition_key=(job["lighting"], job["time_hours"]),
                world_reference=source_world_authority,
                gameplay_by_condition=gameplay_authority,
            )
            if source_anchor_heights is None:
                source_anchor_heights = record["anchor_heights"]
                source_anchor_classes = record["anchor_classes"]
            elif (
                source_anchor_heights != record["anchor_heights"]
                or source_anchor_classes != record["anchor_classes"]
            ):
                raise HarnessError(f"anchor heights/classes changed at resumed job {job_id}")
            for projection_state in record["projection_states"]:
                _record_projection_hashes(
                    projection_state["projection_hashes"],
                    profile_sha256=job["profile_sha256"],
                    liquid_phase_seconds=projection_state["liquid_phase_seconds"],
                    references=projection_hashes_by_profile_phase,
                )
            continue
        stage_root, _ = _validated_asset_stage_binding(work_root, condition)
        camera_paths = [pathlib.Path(entry["path"]) for entry in job["capture_plan"]["captures"]]
        if any(path.exists() or runtime_report_path(path).exists() for path in camera_paths):
            raise HarnessError(f"unrecorded output already exists for {job_id}; preserve it for audit")
        for path in camera_paths:
            path.parent.mkdir(parents=True, exist_ok=True)
        attempt_parent = work_root / "runtime-data" / job_id
        attempt_number = 1
        while (attempt_parent / f"attempt-{attempt_number:03d}").exists():
            attempt_number += 1
        data_root = attempt_parent / f"attempt-{attempt_number:03d}"
        data_root.mkdir(parents=True, exist_ok=False)
        log_path = work_root / "logs" / job_id / f"attempt-{attempt_number:03d}.log"
        log_path.parent.mkdir(parents=True, exist_ok=True)
        launch_nonce = secrets.token_hex(32)
        capture_plan_json = compact_json(job["capture_plan"])
        environment = _map_review_environment()
        environment.update(
            {
                "BEVY_ASSET_ROOT": str(stage_root),
                "HEX_GAME_DATA_DIR": str(data_root),
                "HEX_REVIEW_SCENARIO": SCENARIO,
                "HEX_REVIEW_SEED": str(SEED),
                "HEX_REVIEW_CAPTURE_PLAN": capture_plan_json,
                "HEX_REVIEW_LAUNCH_NONCE": launch_nonce,
                "HEX_REVIEW_SOURCE_PROVENANCE_SHA256": source_provenance_sha256,
            }
        )
        if job["capture_plan"]["version"] == 1:
            environment["HEX_REVIEW_LIQUID_PHASE"] = str(job["liquid_phase_seconds"])
        if job["time_hours"] is not None:
            environment["HEX_REVIEW_TIME"] = str(job["time_hours"])
        if not job["control_profile_omitted"]:
            environment["HEX_REVIEW_WORLD_DETAIL"] = job["profile_json"]
        command = (
            "cargo",
            "run",
            "--locked",
            "--release",
            "-p",
            "hex_game",
            "--features",
            "map-review",
        )
        stage_root, stage_binding = _validated_asset_stage_binding(
            work_root,
            condition,
            expected_root=stage_root,
        )
        attempt_record = {
            "job_id": job_id,
            "job_sha256": job_sha256,
            "attempt_number": attempt_number,
            "status": "RUNNING",
            "command": list(command),
            "log": str(log_path),
            "launch_nonce": launch_nonce,
            "source_provenance_sha256": source_provenance_sha256,
            "capture_plan_sha256": sha256_bytes(capture_plan_json.encode("utf-8")),
            "structural_draft_environment": STRUCTURAL_DRAFT_ENVIRONMENT,
            "structural_draft_value": STRUCTURAL_DRAFT_VALUE,
            **stage_binding,
        }
        attempts.append(attempt_record)
        atomic_write(state_path, pretty_json(state))
        process_error: Optional[BaseException] = None
        returncode: Optional[int] = None
        try:
            legacy = _legacy_harness()
            returncode = legacy.run_logged_process(
                command,
                cwd=REPOSITORY_ROOT,
                environment=environment,
                log_path=log_path,
                timeout_seconds=timeout_seconds,
            )
        except BaseException as error:
            process_error = error
        try:
            _validated_asset_stage_binding(
                work_root,
                condition,
                expected_root=stage_root,
                expected_binding=stage_binding,
            )
        except BaseException as error:
            attempt_record["status"] = "FAILED"
            attempt_record["returncode"] = HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE
            attempt_record["failure_phase"] = "asset-stage-post-process"
            attempt_record["failure_type"] = type(error).__name__
            atomic_write(state_path, pretty_json(state))
            if process_error is not None:
                raise error from process_error
            raise
        if process_error is not None:
            # Timeouts, launch failures, and interruptions are just as terminal
            # for this immutable attempt as a renderer exit or a post-render
            # validation failure. Persist that fact before propagating the
            # exception so a resume can never mistake a poisoned RUNNING row
            # for valid evidence. The staged assets have already been rechecked.
            attempt_record["status"] = "FAILED"
            attempt_record["returncode"] = HARNESS_PROCESS_EXCEPTION_RETURN_CODE
            attempt_record["failure_phase"] = "process"
            attempt_record["failure_type"] = type(process_error).__name__
            atomic_write(state_path, pretty_json(state))
            raise process_error
        if isinstance(returncode, bool) or not isinstance(returncode, int):
            attempt_record["status"] = "FAILED"
            attempt_record["returncode"] = HARNESS_PROCESS_EXCEPTION_RETURN_CODE
            attempt_record["failure_phase"] = "process"
            attempt_record["failure_type"] = "InvalidReturnCode"
            atomic_write(state_path, pretty_json(state))
            raise HarnessError(f"capture job {job_id} returned a non-integer process status")
        if returncode != 0:
            attempt_record["status"] = "FAILED"
            attempt_record["returncode"] = returncode
            atomic_write(state_path, pretty_json(state))
            tail = log_path.read_text(encoding="utf-8", errors="replace")[-6000:]
            raise HarnessError(f"capture job {job_id} failed with {returncode}:\n{tail}")
        try:
            record = _validate_job_artifacts(
                job,
                output_root,
                source_provenance_sha256=source_provenance_sha256,
                expected_launch_nonce=launch_nonce,
                label=True,
            )
            receipt_executable_sha256 = _assert_capture_executable_matches_pin(
                state,
                record["runtime_receipt"]["executable_sha256"],
                context=f"new job {job_id}",
            )
            # Pin the challenger before the oracle comparison. If the post-render
            # gate fails, the immutable failed attempt still identifies the exact
            # executable whose pixels were rejected.
            if state["pinned_executable_sha256"] is None:
                state["pinned_executable_sha256"] = receipt_executable_sha256
                atomic_write(state_path, pretty_json(state))
            oracle_evidence = None
            if job_id in oracle_job_ids:
                oracle_records_by_job[job_id] = record
                oracle_evidence = _validate_baseline_oracle_equivalence(
                    oracle_plan_jobs,
                    oracle_records_by_job,
                    pack_validation=baseline_oracle_pack_validation,
                )
            if (
                oracle_evidence is not None
                and state["baseline_oracle_equivalence"] is not None
                and state["baseline_oracle_equivalence"] != oracle_evidence
            ):
                raise HarnessError("baseline-oracle evidence changed")
            source_world_authority = _record_authority(
                record["authority"],
                condition_key=(job["lighting"], job["time_hours"]),
                world_reference=source_world_authority,
                gameplay_by_condition=gameplay_authority,
            )
            if source_anchor_heights is None:
                source_anchor_heights = record["anchor_heights"]
                source_anchor_classes = record["anchor_classes"]
            elif (
                source_anchor_heights != record["anchor_heights"]
                or source_anchor_classes != record["anchor_classes"]
            ):
                raise HarnessError(f"anchor heights/classes changed at job {job_id}")
            for projection_state in record["projection_states"]:
                _record_projection_hashes(
                    projection_state["projection_hashes"],
                    profile_sha256=job["profile_sha256"],
                    liquid_phase_seconds=projection_state["liquid_phase_seconds"],
                    references=projection_hashes_by_profile_phase,
                )
            completed_record = {
                "job_sha256": job_sha256,
                "artifact_sha256": [item["sha256"] for item in record["pngs"]],
                "report_sha256": [item["sha256"] for item in record["reports"]],
                "log": str(log_path),
                "launch_nonce": launch_nonce,
                "runtime_receipt_sha256": record["runtime_receipt"]["receipt_sha256"],
                "process_id": record["runtime_receipt"]["process_id"],
                "executable_sha256": receipt_executable_sha256,
                **stage_binding,
            }
            _assert_source_provenance(provenance)
        except BaseException as error:
            attempt_record["status"] = "FAILED"
            attempt_record["returncode"] = (
                HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE
            )
            attempt_record["failure_phase"] = "post-render-validation"
            attempt_record["failure_type"] = type(error).__name__
            atomic_write(state_path, pretty_json(state))
            raise
        if oracle_evidence is not None and state["baseline_oracle_equivalence"] is None:
            state["baseline_oracle_equivalence"] = oracle_evidence
        completed[job_id] = completed_record
        attempt_record["status"] = "COMPLETE"
        attempt_record["returncode"] = 0
        attempt_record["artifact_sha256"] = [item["sha256"] for item in record["pngs"]]
        attempt_record["runtime_receipt_sha256"] = record["runtime_receipt"]["receipt_sha256"]
        attempt_record["process_id"] = record["runtime_receipt"]["process_id"]
        attempt_record["executable_sha256"] = record["runtime_receipt"]["executable_sha256"]
        atomic_write(state_path, pretty_json(state))
        executed += 1
    return {
        "version": 1,
        "warning": WARNING,
        "jobs_considered": len(jobs),
        "jobs_executed": executed,
        "jobs_complete": len(completed),
        "renderer_invocations_total": len(attempts),
        "pinned_executable_sha256": state["pinned_executable_sha256"],
        "baseline_oracle_verified": (
            state["baseline_oracle_equivalence"] is not None
            and all(job_id in completed for job_id in oracle_job_ids)
        ),
        "fresh_shared_control_assets": plan["study"]["slot_accounting"][
            "fresh_shared_control_renders"
        ],
        "state": str(state_path),
    }


def _inspect_labeled_png(
    path: pathlib.Path,
    *,
    exact_capture_size: bool = True,
    expected_source_sha256: Optional[str] = None,
) -> Dict[str, Any]:
    """Require a visible-deliverable PNG to carry the immutable warning metadata."""

    try:
        from PIL import Image, ImageStat  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to validate labeled imagery") from error
    try:
        with Image.open(path) as image:
            image.load()
            width, height = image.size
            metadata_warning = image.info.get("structural_draft_warning")
            source_sha256 = image.info.get("source_render_sha256")
            rgb = image.convert("RGB")
            stddev = ImageStat.Stat(rgb).stddev
            banner = rgb.crop((0, 0, width, min(46, height)))
            visible_warning_pixels = sum(
                1
                for red, green, blue in banner.getdata()
                if red >= 180 and green >= 140 and blue <= 150
            )
    except OSError as error:
        raise HarnessError(f"cannot decode labeled PNG {path}: {error}") from error
    if exact_capture_size and (width, height) != (CAPTURE_WIDTH, CAPTURE_HEIGHT):
        raise HarnessError(f"labeled capture dimensions changed: {path}")
    if metadata_warning != WARNING:
        raise HarnessError(f"labeled PNG lacks the exact warning metadata: {path}")
    if visible_warning_pixels < 12:
        raise HarnessError(f"labeled PNG lacks a visible warning overlay: {path}")
    if expected_source_sha256 is not None and source_sha256 != expected_source_sha256:
        raise HarnessError(f"labeled PNG does not derive from the current raw render: {path}")
    if max(stddev) < 1.0:
        raise HarnessError(f"labeled PNG is effectively blank: {path}")
    return {
        "path": str(path),
        "width": width,
        "height": height,
        "sha256": sha256_file(path),
        "warning": metadata_warning,
        "visible_warning_overlay": True,
        "source_render_sha256": source_sha256,
    }


def _validate_mp4(path: pathlib.Path, *, expected_frames: int, expected_fps: int) -> Dict[str, Any]:
    ffprobe = shutil.which("ffprobe")
    if ffprobe is None:
        raise HarnessError("ffprobe is required to validate motion deliverables")
    result = subprocess.run(
        (
            ffprobe,
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-count_frames",
            "-show_entries",
            "stream=codec_name,width,height,r_frame_rate,nb_frames,nb_read_frames,duration:format=duration",
            "-of",
            "json",
            str(path),
        ),
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise HarnessError(f"ffprobe rejected {path}: {result.stderr[-2000:]}")
    try:
        payload = json.loads(result.stdout)
        stream = payload["streams"][0]
        format_record = payload["format"]
    except (json.JSONDecodeError, KeyError, IndexError, TypeError) as error:
        raise HarnessError(f"ffprobe returned malformed metadata for {path}") from error
    if stream.get("codec_name") != "h264":
        raise HarnessError(f"motion clip is not H.264: {path}")
    if (stream.get("width"), stream.get("height")) != (CAPTURE_WIDTH, CAPTURE_HEIGHT):
        raise HarnessError(f"motion clip dimensions changed: {path}")
    if stream.get("r_frame_rate") != f"{expected_fps}/1":
        raise HarnessError(f"motion clip frame rate changed: {path}")
    frame_count_values = [
        value
        for value in (stream.get("nb_read_frames"), stream.get("nb_frames"))
        if value not in (None, "N/A")
    ]
    if not frame_count_values or any(int(value) != expected_frames for value in frame_count_values):
        raise HarnessError(f"motion clip frame count changed: {path}")
    duration_raw = stream.get("duration")
    if duration_raw in (None, "N/A"):
        duration_raw = format_record.get("duration")
    try:
        duration = float(duration_raw)
    except (TypeError, ValueError) as error:
        raise HarnessError(f"motion clip duration is unavailable: {path}") from error
    expected_duration = expected_frames / expected_fps
    if not math.isfinite(duration) or abs(duration - expected_duration) > (0.5 / expected_fps):
        raise HarnessError(
            f"motion clip duration changed: {path} ({duration} != {expected_duration})"
        )
    if not path.is_file() or path.stat().st_size < 1024:
        raise HarnessError(f"motion clip is missing or implausibly small: {path}")
    return {
        "path": str(path),
        "sha256": sha256_file(path),
        "bytes": path.stat().st_size,
        "stream": stream,
        "frame_count": expected_frames,
        "duration_seconds": duration,
    }


def _encode_motion_mp4(frame_directory: pathlib.Path, destination: pathlib.Path) -> None:
    """Encode a deterministic 90-frame H.264 artifact from exact paired PNGs."""

    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg is None:
        raise HarnessError("ffmpeg is required to encode motion deliverables")
    if destination.exists():
        raise HarnessError(f"motion encode destination already exists: {destination}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    result = subprocess.run(
        (
            ffmpeg,
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-framerate",
            str(FPS),
            "-start_number",
            "1",
            "-i",
            str(frame_directory / "frame-%04d.png"),
            "-frames:v",
            str(MOTION_FRAME_COUNT),
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            "1",
            "-map_metadata",
            "-1",
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-movflags",
            "+faststart",
            str(destination),
        ),
        check=False,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise HarnessError(f"ffmpeg failed for {destination.name}: {result.stderr[-3000:]}")


def _write_paired_motion_frame(
    control_path: pathlib.Path,
    candidate_path: pathlib.Path,
    destination: pathlib.Path,
    opaque_code: str,
) -> Dict[str, Any]:
    """Compose a full-HD pair whose only treatment label is an opaque code."""

    if BLIND_CODE_RE.fullmatch(opaque_code) is None:
        raise HarnessError("paired motion frame label must be an opaque review code")

    try:
        from PIL import Image, ImageDraw, ImageFont, PngImagePlugin  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to build paired motion frames") from error
    control_record = _inspect_labeled_png(control_path)
    candidate_record = _inspect_labeled_png(candidate_path)
    canvas = Image.new("RGB", (CAPTURE_WIDTH, CAPTURE_HEIGHT), (8, 8, 10))
    resampling = getattr(Image, "Resampling", Image)
    tile_size = (CAPTURE_WIDTH // 2, CAPTURE_HEIGHT // 2)
    tile_y = 190
    for index, path in enumerate((control_path, candidate_path)):
        with Image.open(path) as source:
            tile = source.convert("RGB").resize(tile_size, resampling.LANCZOS)
        canvas.paste(tile, (index * tile_size[0], tile_y))
    draw = ImageDraw.Draw(canvas)
    draw.rectangle((0, 0, CAPTURE_WIDTH, 48), fill=(8, 8, 10))
    try:
        warning_font = ImageFont.truetype("Arial.ttf", 24)
        label_font = ImageFont.truetype("Arial.ttf", 30)
    except OSError:
        warning_font = ImageFont.load_default()
        label_font = ImageFont.load_default()
    draw.text((18, 10), WARNING, fill=(255, 214, 72), font=warning_font)
    draw.text((20, 92), "CURRENT / CONTROL", fill=(220, 229, 238), font=label_font)
    draw.text((CAPTURE_WIDTH // 2 + 20, 92), opaque_code, fill=(220, 229, 238), font=label_font)
    metadata = PngImagePlugin.PngInfo()
    metadata.add_text("structural_draft_warning", WARNING)
    metadata.add_text("control_frame_sha256", control_record["sha256"])
    metadata.add_text("candidate_frame_sha256", candidate_record["sha256"])
    metadata.add_text("opaque_review_code", opaque_code)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    canvas.save(temporary, format="PNG", pnginfo=metadata, optimize=False)
    os.replace(temporary, destination)
    return _inspect_labeled_png(destination)


def _write_eight_frame_strip(
    frame_paths: Sequence[pathlib.Path],
    destination: pathlib.Path,
    opaque_code: str,
) -> Dict[str, Any]:
    """Build a source-hash-bound 4x2 strip labeled only by opaque code."""

    try:
        from PIL import Image, ImageDraw, ImageFont, PngImagePlugin  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to build motion strips") from error
    if len(frame_paths) != 8:
        raise HarnessError("motion strip requires exactly eight frames")
    if BLIND_CODE_RE.fullmatch(opaque_code) is None:
        raise HarnessError("motion strip label must be an opaque review code")
    source_frame_sha256 = [sha256_file(path) for path in frame_paths]
    tile_width = CAPTURE_WIDTH // 4
    tile_height = CAPTURE_HEIGHT // 4
    banner_height = 46
    canvas = Image.new("RGB", (CAPTURE_WIDTH, banner_height + tile_height * 2), (8, 8, 10))
    resampling = getattr(Image, "Resampling", Image)
    for index, frame_path in enumerate(frame_paths):
        with Image.open(frame_path) as frame:
            tile = frame.convert("RGB").resize((tile_width, tile_height), resampling.LANCZOS)
        canvas.paste(tile, ((index % 4) * tile_width, banner_height + (index // 4) * tile_height))
    draw = ImageDraw.Draw(canvas)
    try:
        font = ImageFont.truetype("Arial.ttf", 24)
    except OSError:
        font = ImageFont.load_default()
    draw.text((18, 10), WARNING, fill=(255, 214, 72), font=font)
    draw.text((CAPTURE_WIDTH - 230, 10), opaque_code, fill=(220, 229, 238), font=font)
    metadata = PngImagePlugin.PngInfo()
    metadata.add_text("structural_draft_warning", WARNING)
    metadata.add_text("opaque_review_code", opaque_code)
    metadata.add_text("strip_source_frame_sha256", sha256_object(source_frame_sha256))
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    canvas.save(temporary, format="PNG", pnginfo=metadata, optimize=False)
    os.replace(temporary, destination)
    return _inspect_labeled_png(destination, exact_capture_size=False)


def finalize_motion_clip(plan_path: pathlib.Path, clip_id: str) -> Dict[str, Any]:
    """Encode one captured clip and publish its deterministic eight-frame strip."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    motion = plan.get("motion", {})
    matching = [clip for clip in motion.get("clips", ()) if clip.get("id") == clip_id]
    if len(matching) != 1:
        raise HarnessError(f"motion plan does not contain exactly one clip {clip_id!r}")
    clip = matching[0]
    opaque_code = _blind_code(
        _motion_blinding_seed(plan),
        f"motion-{clip['id']}",
        clip["lighting"],
        MOTION_REVIEW_CONTEXT,
    )
    all_jobs = {job["id"]: job for job in motion.get("jobs", ())}
    required_ids = list(dict.fromkeys((*clip["control_job_ids"], *clip["candidate_job_ids"])))
    try:
        jobs = [all_jobs[job_id] for job_id in required_ids]
    except KeyError as error:
        raise HarnessError(f"motion clip {clip_id} refers to a missing frame job") from error
    if len(clip["control_raw_frames"]) != MOTION_FRAME_COUNT or len(clip["candidate_raw_frames"]) != MOTION_FRAME_COUNT:
        raise HarnessError(f"motion clip {clip_id} does not contain two complete frame sequences")
    authority = None
    for job in jobs:
        record = _validate_job_artifacts(
            job,
            output_root,
            source_provenance_sha256=sha256_object(plan["provenance"]),
            label=True,
        )
        if authority is None:
            authority = record["authority"]
        elif record["authority"] != authority:
            raise HarnessError(f"authority fingerprint changed during motion clip {clip_id}")
    paired_frames = []
    for frame_index, (control_relative, candidate_relative) in enumerate(
        zip(clip["control_raw_frames"], clip["candidate_raw_frames"])
    ):
        control_path = _labeled_capture_path(
            output_root, _raw_artifact_path(raw_capture_root, control_relative)
        )
        candidate_path = _labeled_capture_path(
            output_root, _raw_artifact_path(raw_capture_root, candidate_relative)
        )
        control_record = _inspect_labeled_png(control_path)
        candidate_record = _inspect_labeled_png(candidate_path)
        paired_path = output_root / clip["paired_frame_directory"] / f"frame-{frame_index + 1:04d}.png"
        if paired_path.exists():
            paired_record = _inspect_labeled_png(paired_path)
            try:
                from PIL import Image  # pylint: disable=import-outside-toplevel
                with Image.open(paired_path) as paired_image:
                    control_hash = paired_image.info.get("control_frame_sha256")
                    candidate_hash = paired_image.info.get("candidate_frame_sha256")
                    recorded_code = paired_image.info.get("opaque_review_code")
            except (ImportError, OSError) as error:
                raise HarnessError(f"cannot inspect paired frame provenance {paired_path}") from error
            if (
                control_hash != control_record["sha256"]
                or candidate_hash != candidate_record["sha256"]
                or recorded_code != opaque_code
            ):
                raise HarnessError(f"paired frame is stale: {paired_path}")
        else:
            paired_record = _write_paired_motion_frame(
                control_path,
                candidate_path,
                paired_path,
                opaque_code,
            )
        del paired_record
        paired_frames.append(paired_path)

    paired_sha256 = [sha256_file(path) for path in paired_frames]
    destination = output_root / clip["mp4"]
    strip_path = output_root / clip["eight_frame_strip"]
    finalization_state_path = (
        output_root
        / "runtime"
        / "motion-finalization"
        / f"{clip_id}-{clip['profile_sha256'][:12]}.json"
    )
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        if not finalization_state_path.exists():
            raise HarnessError(f"motion clip exists without finalization provenance: {destination}")
        finalization_state = _read_json(finalization_state_path, "motion finalization state")
        if (
            finalization_state.get("version") != 1
            or finalization_state.get("warning") != WARNING
            or finalization_state.get("paired_frame_sha256") != paired_sha256
            or finalization_state.get("mp4_sha256") != sha256_file(destination)
        ):
            raise HarnessError(f"motion finalization provenance is stale: {destination}")
    else:
        temporary = destination.with_name(destination.stem + ".tmp.mp4")
        if temporary.exists():
            raise HarnessError(f"stale temporary motion clip exists: {temporary}")
        _encode_motion_mp4(paired_frames[0].parent, temporary)
        os.replace(temporary, destination)
    video_record = _validate_mp4(destination, expected_frames=MOTION_FRAME_COUNT, expected_fps=FPS)

    indices = [round(index * (MOTION_FRAME_COUNT - 1) / 7) for index in range(8)]
    strip_sources = [paired_frames[index] for index in indices]
    strip_source_hash = sha256_object([sha256_file(path) for path in strip_sources])
    if strip_path.exists():
        strip_record = _inspect_labeled_png(strip_path, exact_capture_size=False)
        try:
            from PIL import Image  # pylint: disable=import-outside-toplevel
            with Image.open(strip_path) as strip_image:
                recorded_strip_sources = strip_image.info.get("strip_source_frame_sha256")
                recorded_strip_code = strip_image.info.get("opaque_review_code")
        except (ImportError, OSError) as error:
            raise HarnessError(f"cannot inspect motion strip provenance {strip_path}") from error
        if recorded_strip_sources != strip_source_hash or recorded_strip_code != opaque_code:
            raise HarnessError(f"motion strip is stale or discloses treatment identity: {strip_path}")
    else:
        strip_record = _write_eight_frame_strip(strip_sources, strip_path, opaque_code)
    finalization_state = {
        "version": 1,
        "warning": WARNING,
        "clip_id": clip_id,
        "opaque_review_code": opaque_code,
        "paired_frame_sha256": paired_sha256,
        "strip_source_frame_sha256": strip_source_hash,
        "mp4_sha256": video_record["sha256"],
        "strip_sha256": strip_record["sha256"],
    }
    if finalization_state_path.exists():
        existing_state = _read_json(finalization_state_path, "motion finalization state")
        if existing_state != finalization_state:
            raise HarnessError(f"motion strip or clip changed after finalization: {clip_id}")
    else:
        atomic_write(finalization_state_path, pretty_json(finalization_state))
    return {
        "version": 1,
        "warning": WARNING,
        "clip_id": clip_id,
        "frame_indices_in_strip": indices,
        "authority": authority,
        "mp4": video_record,
        "strip": strip_record,
    }


def _reproduce_motion_derivatives(
    *,
    plan: Mapping[str, Any],
    clip: Mapping[str, Any],
    paired_paths: Sequence[pathlib.Path],
    expected_code: str,
    published_mp4: pathlib.Path,
    published_strip: pathlib.Path,
) -> Dict[str, Any]:
    """Rebuild a clip and strip from planned source frames and compare exact bytes."""

    output_root = pathlib.Path(plan["output_root"])
    raw_capture_root = pathlib.Path(plan["raw_capture_root"])
    with tempfile.TemporaryDirectory(prefix="world-detail-motion-verify-") as temporary:
        temporary_root = pathlib.Path(temporary)
        generated_frames = temporary_root / "paired"
        generated_hashes = []
        for index, (control_relative, candidate_relative, published_pair) in enumerate(
            zip(
                clip["control_raw_frames"],
                clip["candidate_raw_frames"],
                paired_paths,
            )
        ):
            control = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, control_relative),
            )
            candidate = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, candidate_relative),
            )
            generated = generated_frames / f"frame-{index + 1:04d}.png"
            _write_paired_motion_frame(control, candidate, generated, expected_code)
            digest = sha256_file(generated)
            if digest != sha256_file(published_pair):
                raise HarnessError(
                    f"motion paired frame is not reproducible from planned sources: {clip['id']} frame {index + 1}"
                )
            generated_hashes.append(digest)

        generated_mp4 = temporary_root / "clip.mp4"
        _encode_motion_mp4(generated_frames, generated_mp4)
        _validate_mp4(
            generated_mp4,
            expected_frames=MOTION_FRAME_COUNT,
            expected_fps=FPS,
        )
        if sha256_file(generated_mp4) != sha256_file(published_mp4):
            raise HarnessError(
                f"motion MP4 is not the deterministic encoding of planned frames: {clip['id']}"
            )

        indices = [round(index * (MOTION_FRAME_COUNT - 1) / 7) for index in range(8)]
        generated_strip = temporary_root / "strip.png"
        _write_eight_frame_strip(
            [generated_frames / f"frame-{index + 1:04d}.png" for index in indices],
            generated_strip,
            expected_code,
        )
        if sha256_file(generated_strip) != sha256_file(published_strip):
            raise HarnessError(
                f"motion strip is not reproducible from the exact planned frames: {clip['id']}"
            )
    return {
        "paired_frame_manifest_sha256": sha256_object(generated_hashes),
        "mp4_sha256": sha256_file(published_mp4),
        "strip_sha256": sha256_file(published_strip),
        "reproduced_from_planned_frames": True,
    }


def _validate_motion_deliverables_for_plan(plan: Mapping[str, Any]) -> Dict[str, Any]:
    """Require every planned paired frame set, clip, strip, and provenance state."""

    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    clips = plan["motion"]["clips"]
    if len(clips) != plan["motion"]["expected_clips"]:
        raise HarnessError("complete motion validation requires every planned clip")
    labeled_motion_paths = {
        _labeled_capture_path(
            output_root,
            pathlib.Path(capture["path"]).resolve(),
        ).resolve()
        for job in plan["motion"]["jobs"]
        for capture in job["capture_plan"]["captures"]
    }
    motion_frame_root = (output_root / "motion-frames").resolve()
    motion_control_root = (output_root / "motion-control-frames").resolve()
    expected_motion_frames = {
        path for path in labeled_motion_paths if motion_frame_root in path.parents
    }
    expected_motion_control_frames = {
        path for path in labeled_motion_paths if motion_control_root in path.parents
    }
    if (
        expected_motion_frames | expected_motion_control_frames != labeled_motion_paths
        or len(expected_motion_frames) != len(clips) * MOTION_FRAME_COUNT
        or len(expected_motion_control_frames)
        != plan["motion"]["shared_control_orbits"] * MOTION_FRAME_COUNT
    ):
        raise HarnessError("planned motion frames do not map into the two publication frame trees")
    expected_paired_frames = {
        (
            output_root
            / clip["paired_frame_directory"]
            / f"frame-{index + 1:04d}.png"
        ).resolve()
        for clip in clips
        for index in range(MOTION_FRAME_COUNT)
    }
    expected_clips = {(output_root / clip["mp4"]).resolve() for clip in clips}
    expected_strips = {
        (output_root / clip["eight_frame_strip"]).resolve() for clip in clips
    }
    expected_finalization_states = {
        (
            output_root
            / "runtime"
            / "motion-finalization"
            / f"{clip['id']}-{clip['profile_sha256'][:12]}.json"
        ).resolve()
        for clip in clips
    }
    for root, expected, context in (
        (output_root / "motion-frames", expected_motion_frames, "candidate motion-frame"),
        (
            output_root / "motion-control-frames",
            expected_motion_control_frames,
            "control motion-frame",
        ),
        (
            output_root / "motion-paired-frames",
            expected_paired_frames,
            "paired motion-frame",
        ),
        (output_root / "motion-clips", expected_clips, "motion MP4"),
        (output_root / "motion-strips", expected_strips, "motion strip"),
        (
            output_root / "runtime" / "motion-finalization",
            expected_finalization_states,
            "motion finalization-state",
        ),
    ):
        _validate_exact_file_inventory(root, expected, context=context)
    video_hashes = []
    strip_hashes = []
    paired_content_hashes = set()
    reproduction_records = []
    for clip in clips:
        paired_paths = [
            output_root / clip["paired_frame_directory"] / f"frame-{index + 1:04d}.png"
            for index in range(MOTION_FRAME_COUNT)
        ]
        paired_hashes = []
        for index, paired_path in enumerate(paired_paths):
            paired_record = _inspect_labeled_png(paired_path)
            control_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, clip["control_raw_frames"][index]),
            )
            candidate_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, clip["candidate_raw_frames"][index]),
            )
            control_hash = sha256_file(control_path)
            candidate_hash = sha256_file(candidate_path)
            try:
                from PIL import Image  # pylint: disable=import-outside-toplevel
                with Image.open(paired_path) as paired_image:
                    recorded_control = paired_image.info.get("control_frame_sha256")
                    recorded_candidate = paired_image.info.get("candidate_frame_sha256")
                    recorded_code = paired_image.info.get("opaque_review_code")
            except (ImportError, OSError) as error:
                raise HarnessError(f"cannot inspect paired frame {paired_path}") from error
            expected_code = _blind_code(
                _motion_blinding_seed(plan),
                f"motion-{clip['id']}",
                clip["lighting"],
                MOTION_REVIEW_CONTEXT,
            )
            if (
                recorded_control != control_hash
                or recorded_candidate != candidate_hash
                or recorded_code != expected_code
            ):
                raise HarnessError(f"paired frame provenance changed: {paired_path}")
            paired_hashes.append(paired_record["sha256"])
            paired_content_hashes.add(paired_record["sha256"])

        video = _validate_mp4(
            output_root / clip["mp4"],
            expected_frames=MOTION_FRAME_COUNT,
            expected_fps=FPS,
        )
        strip = _inspect_labeled_png(
            output_root / clip["eight_frame_strip"],
            exact_capture_size=False,
        )
        strip_indices = [round(index * (MOTION_FRAME_COUNT - 1) / 7) for index in range(8)]
        expected_strip_source_hash = sha256_object(
            [paired_hashes[index] for index in strip_indices]
        )
        try:
            from PIL import Image  # pylint: disable=import-outside-toplevel
            with Image.open(output_root / clip["eight_frame_strip"]) as strip_image:
                strip_code = strip_image.info.get("opaque_review_code")
                strip_source_hash = strip_image.info.get("strip_source_frame_sha256")
        except (ImportError, OSError) as error:
            raise HarnessError(f"cannot inspect motion strip provenance for {clip['id']}") from error
        if strip_code != expected_code or strip_source_hash != expected_strip_source_hash:
            raise HarnessError(f"motion strip does not bind its planned source frames: {clip['id']}")
        state_path = (
            output_root
            / "runtime"
            / "motion-finalization"
            / f"{clip['id']}-{clip['profile_sha256'][:12]}.json"
        )
        state = _exact_keys(
            _read_json(state_path, "motion finalization state"),
            (
                "version",
                "warning",
                "clip_id",
                "opaque_review_code",
                "paired_frame_sha256",
                "strip_source_frame_sha256",
                "mp4_sha256",
                "strip_sha256",
            ),
            "motion finalization state",
        )
        if (
            state["version"] != 1
            or state["warning"] != WARNING
            or state["clip_id"] != clip["id"]
            or state["opaque_review_code"] != expected_code
            or state["paired_frame_sha256"] != paired_hashes
            or state["strip_source_frame_sha256"] != expected_strip_source_hash
            or state["mp4_sha256"] != video["sha256"]
            or state["strip_sha256"] != strip["sha256"]
        ):
            raise HarnessError(f"motion finalization state changed: {clip['id']}")
        reproduction_records.append(
            _reproduce_motion_derivatives(
                plan=plan,
                clip=clip,
                paired_paths=paired_paths,
                expected_code=expected_code,
                published_mp4=output_root / clip["mp4"],
                published_strip=output_root / clip["eight_frame_strip"],
            )
        )
        video_hashes.append(video["sha256"])
        strip_hashes.append(strip["sha256"])
    return {
        "clips": len(clips),
        "candidate_frames": len(expected_motion_frames),
        "control_frames": len(expected_motion_control_frames),
        "paired_frames": len(clips) * MOTION_FRAME_COUNT,
        "mp4_files": len(expected_clips),
        "strip_files": len(expected_strips),
        "finalization_states": len(expected_finalization_states),
        "unique_paired_frame_content_hashes": len(paired_content_hashes),
        "video_sha256": video_hashes,
        "strip_sha256": strip_hashes,
        "reproduction_records": reproduction_records,
    }


def validate_motion_deliverables(plan_path: pathlib.Path) -> Dict[str, Any]:
    """Public strict validator for every finalized paired motion deliverable."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, _ = _require_publication_capture_plan_path(plan_path, plan)
    result = _validate_motion_deliverables_for_plan(plan)
    return {"version": 1, "warning": WARNING, **result}


def _motion_blinding_seed(plan: Mapping[str, Any]) -> str:
    return sha256_bytes(
        f"{_deterministic_plan_blinding_seed(plan)}:published-motion-source".encode(
            "utf-8"
        )
    )


def _install_generated_packet_media(
    generated: pathlib.Path,
    destination: pathlib.Path,
) -> str:
    generated_sha256 = sha256_file(generated)
    if destination.exists():
        if destination.is_symlink() or sha256_file(destination) != generated_sha256:
            raise HarnessError(f"refusing to overwrite changed review media {destination}")
        return generated_sha256
    destination.parent.mkdir(parents=True, exist_ok=True)
    os.replace(generated, destination)
    return generated_sha256


def _materialize_opaque_motion_packet_media(
    plan: Mapping[str, Any],
    clip: Mapping[str, Any],
    packet_root: pathlib.Path,
    opaque_code: str,
) -> Dict[str, Any]:
    """Rebuild reviewer media so published source hashes/codes cannot be joined."""

    output_root = pathlib.Path(plan["output_root"]).resolve()
    raw_capture_root = _validate_raw_capture_root(
        output_root,
        pathlib.Path(plan["raw_capture_root"]),
    )
    strip_relative = pathlib.Path("images") / opaque_code / "eight-frame-strip.png"
    mp4_relative = pathlib.Path("clips") / f"{opaque_code}.mp4"
    packet_root.parent.mkdir(parents=True, exist_ok=True)
    source_pairs = []
    with tempfile.TemporaryDirectory(
        prefix=f".{opaque_code.lower()}-",
        dir=packet_root.parent,
    ) as temporary_name:
        temporary = pathlib.Path(temporary_name)
        frame_root = temporary / "frames"
        paired_paths = []
        for frame_index, (control_relative, candidate_relative) in enumerate(
            zip(clip["control_raw_frames"], clip["candidate_raw_frames"]),
            start=1,
        ):
            control_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, control_relative),
            )
            candidate_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, candidate_relative),
            )
            control = _inspect_labeled_png(control_path)
            candidate = _inspect_labeled_png(candidate_path)
            source_pairs.append(
                {
                    "control_sha256": control["sha256"],
                    "candidate_sha256": candidate["sha256"],
                }
            )
            paired_path = frame_root / f"frame-{frame_index:04d}.png"
            _write_paired_motion_frame(
                control_path,
                candidate_path,
                paired_path,
                opaque_code,
            )
            paired_paths.append(paired_path)
        if len(paired_paths) != MOTION_FRAME_COUNT:
            raise HarnessError(f"motion review media for {clip['id']} lacks 90 paired frames")
        generated_mp4 = temporary / "review.mp4"
        _encode_motion_mp4(frame_root, generated_mp4)
        indices = [round(index * (MOTION_FRAME_COUNT - 1) / 7) for index in range(8)]
        strip_sources = [paired_paths[index] for index in indices]
        strip_source_sha256 = sha256_object(
            [sha256_file(path) for path in strip_sources]
        )
        generated_strip = temporary / "review-strip.png"
        _write_eight_frame_strip(strip_sources, generated_strip, opaque_code)
        mp4_sha256 = _install_generated_packet_media(
            generated_mp4,
            packet_root / mp4_relative,
        )
        strip_sha256 = _install_generated_packet_media(
            generated_strip,
            packet_root / strip_relative,
        )
    _inspect_opaque_motion_strip(packet_root / strip_relative, opaque_code)
    _validate_mp4(
        packet_root / mp4_relative,
        expected_frames=MOTION_FRAME_COUNT,
        expected_fps=FPS,
    )
    return {
        "strip_relative": strip_relative.as_posix(),
        "strip_sha256": strip_sha256,
        "mp4_relative": mp4_relative.as_posix(),
        "mp4_sha256": mp4_sha256,
        "source_pair_manifest_sha256": sha256_object(source_pairs),
        "strip_source_frame_sha256": strip_source_sha256,
    }


def _inspect_opaque_motion_strip(path: pathlib.Path, opaque_code: str) -> Dict[str, Any]:
    rendered = _inspect_labeled_png(path, exact_capture_size=False)
    try:
        from PIL import Image  # pylint: disable=import-outside-toplevel

        with Image.open(path) as strip_image:
            metadata = dict(strip_image.info)
    except (ImportError, OSError) as error:
        raise HarnessError(f"cannot inspect opaque motion strip {path}") from error
    if set(metadata) != {
        "structural_draft_warning",
        "opaque_review_code",
        "strip_source_frame_sha256",
    }:
        raise HarnessError(f"public motion strip {opaque_code} contains non-opaque metadata")
    if (
        metadata["opaque_review_code"] != opaque_code
        or metadata["structural_draft_warning"] != WARNING
        or not isinstance(metadata["strip_source_frame_sha256"], str)
        or SHA256_RE.fullmatch(metadata["strip_source_frame_sha256"]) is None
    ):
        raise HarnessError(f"public motion strip {opaque_code} lost opaque binding")
    return {
        **rendered,
        "strip_source_frame_sha256": metadata["strip_source_frame_sha256"],
    }


def _validate_public_motion_review_packet(packet_path: pathlib.Path) -> Dict[str, Any]:
    packet_path = packet_path.resolve()
    packet = _exact_keys(
        _read_json(packet_path, "blinded motion review packet"),
        (
            "version",
            "warning",
            "packet_kind",
            "blinding_salt_commitment",
            "scoring_contract_sha256",
            "unscored_camera_ids",
            "categories",
            "entries",
        ),
        "blinded motion review packet",
    )
    if (
        packet["version"] != 1
        or packet["warning"] != WARNING
        or packet["packet_kind"] != "opaque-world-detail-motion-review"
        or packet["scoring_contract_sha256"] != sha256_object(scoring_contract())
        or packet["unscored_camera_ids"] != list(UNSCORED_CAMERA_IDS)
        or packet["categories"] != list(CATEGORY_ORDER)
        or not isinstance(packet["blinding_salt_commitment"], str)
        or SHA256_RE.fullmatch(packet["blinding_salt_commitment"]) is None
        or not isinstance(packet["entries"], list)
        or len(packet["entries"]) != 22
    ):
        raise HarnessError("blinded motion packet identity or inventory changed")
    codes = set()
    files = set()
    normalized = []
    for index, raw_entry in enumerate(packet["entries"], start=1):
        entry = _exact_keys(
            raw_entry,
            ("order", "code", "strip", "mp4"),
            f"motion packet entry {index}",
        )
        code = entry["code"]
        if (
            entry["order"] != index
            or not isinstance(code, str)
            or BLIND_CODE_RE.fullmatch(code) is None
            or code in codes
        ):
            raise HarnessError(f"motion packet entry {index} code/order is invalid")
        codes.add(code)
        media = {}
        for media_kind, expected_suffix in (("strip", ".png"), ("mp4", ".mp4")):
            item = _exact_keys(
                entry[media_kind],
                ("path", "sha256"),
                f"motion packet {code}.{media_kind}",
            )
            relative = pathlib.Path(item["path"])
            if (
                relative.is_absolute()
                or ".." in relative.parts
                or relative.suffix != expected_suffix
                or not isinstance(item["sha256"], str)
                or SHA256_RE.fullmatch(item["sha256"]) is None
            ):
                raise HarnessError(f"motion packet {code}.{media_kind} is malformed")
            path = (packet_path.parent / relative).resolve()
            try:
                path.relative_to(packet_path.parent)
            except ValueError as error:
                raise HarnessError(f"motion packet {code}.{media_kind} escaped packet") from error
            if path in files or path.is_symlink() or sha256_file(path) != item["sha256"]:
                raise HarnessError(f"motion packet {code}.{media_kind} bytes are invalid")
            files.add(path)
            if media_kind == "strip":
                _inspect_opaque_motion_strip(path, code)
            else:
                _validate_mp4(path, expected_frames=MOTION_FRAME_COUNT, expected_fps=FPS)
            media[media_kind] = {**item, "absolute_path": str(path)}
        normalized.append({**entry, **media})
    materialized = {
        path.resolve()
        for suffix in ("*.png", "*.mp4")
        for path in packet_path.parent.rglob(suffix)
        if path.is_file()
    }
    if materialized != files:
        raise HarnessError("motion review packet contains unmanifested media")
    return {
        **packet,
        "entries": normalized,
        "path": str(packet_path),
        "sha256": sha256_file(packet_path),
    }


def validate_blinded_motion_review_packet(
    plan_path: pathlib.Path,
    packet_path: pathlib.Path,
    unblind_map_path: pathlib.Path,
) -> Dict[str, Any]:
    plan_path = plan_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    if len(plan["motion"]["clips"]) != plan["motion"]["expected_clips"]:
        raise HarnessError("motion review requires the complete resolved clip plan")
    output_root = pathlib.Path(plan["output_root"]).resolve()
    _require_blinded_packet_root(
        packet_path.resolve().parent,
        output_root,
        "motion review packet root",
    )
    packet = _validate_public_motion_review_packet(packet_path)
    unblind_map_path = unblind_map_path.resolve()
    for forbidden_root, label in ((output_root, "published output"), (packet_path.resolve().parent, "public packet")):
        try:
            unblind_map_path.relative_to(forbidden_root)
        except ValueError:
            pass
        else:
            raise HarnessError(f"private motion unblind map may not live inside the {label}")
    unblind = _exact_keys(
        _read_json(unblind_map_path, "private motion unblind map"),
        (
            "version",
            "warning",
            "status",
            "packet_kind",
            "binding_sha256",
            "motion_plan_sha256",
            "review_packet_sha256",
            "blinding_salt",
            "entries",
        ),
        "private motion unblind map",
    )
    motion_plan_sha256 = sha256_object(plan["motion"])
    salt = unblind["blinding_salt"]
    seed = _private_packet_blinding_seed(
        plan,
        salt,
        packet_kind="opaque-world-detail-motion-review",
        binding_sha256=motion_plan_sha256,
    )
    if (
        unblind["version"] != 1
        or unblind["warning"] != WARNING
        or unblind["status"] != "FINALIZED"
        or unblind["packet_kind"] != "opaque-world-detail-motion-review"
        or unblind["binding_sha256"] != motion_plan_sha256
        or unblind["motion_plan_sha256"] != motion_plan_sha256
        or unblind["review_packet_sha256"] != packet["sha256"]
        or sha256_bytes(salt.encode("utf-8")) != packet["blinding_salt_commitment"]
        or not isinstance(unblind["entries"], list)
        or len(unblind["entries"]) != 22
    ):
        raise HarnessError("private motion unblind map binding changed")
    clips = {clip["id"]: clip for clip in plan["motion"]["clips"]}
    public = {entry["code"]: entry for entry in packet["entries"]}
    mapped = {}
    for index, raw_entry in enumerate(unblind["entries"]):
        entry = _exact_keys(
            raw_entry,
            (
                "code",
                "clip_id",
                "profile_id",
                "lighting",
                "camera_id",
                "strip_source",
                "mp4_source",
                "review_strip_sha256",
                "review_mp4_sha256",
                "source_pair_manifest_sha256",
                "strip_source_frame_sha256",
            ),
            f"motion unblind entry {index}",
        )
        clip = clips.get(entry["clip_id"])
        subject_id = f"motion-{entry['clip_id']}"
        if (
            clip is None
            or entry["code"] in mapped
            or entry["code"] not in public
            or entry["profile_id"] != clip["profile_id"]
            or entry["lighting"] != clip["lighting"]
            or entry["camera_id"] != clip["camera_id"]
            or entry["code"]
            != _blind_code(
                seed,
                subject_id,
                clip["lighting"],
                MOTION_REVIEW_CONTEXT,
            )
        ):
            raise HarnessError(f"motion unblind entry {index} is invalid")
        sources = {}
        for field, relative_key in (
            ("strip_source", "eight_frame_strip"),
            ("mp4_source", "mp4"),
        ):
            source = _exact_keys(
                entry[field],
                ("path", "sha256"),
                f"motion unblind entry {index}.{field}",
            )
            expected_path = (output_root / clip[relative_key]).resolve()
            if (
                pathlib.Path(source["path"]).resolve() != expected_path
                or expected_path.is_symlink()
                or sha256_file(expected_path) != source["sha256"]
            ):
                raise HarnessError(f"motion unblind source changed for {entry['code']}")
            sources[field] = source
        public_entry = public[entry["code"]]
        if (
            entry["review_strip_sha256"] != public_entry["strip"]["sha256"]
            or entry["review_mp4_sha256"] != public_entry["mp4"]["sha256"]
            or entry["review_strip_sha256"] == sources["strip_source"]["sha256"]
            or entry["review_mp4_sha256"] == sources["mp4_source"]["sha256"]
        ):
            raise HarnessError(
                f"motion packet was not independently re-encoded for {entry['code']}"
            )
        strip = _inspect_opaque_motion_strip(
            pathlib.Path(public_entry["strip"]["absolute_path"]),
            entry["code"],
        )
        if strip["strip_source_frame_sha256"] != entry["strip_source_frame_sha256"]:
            raise HarnessError(f"motion strip source binding changed for {entry['code']}")
        source_pairs = []
        raw_capture_root = _validate_raw_capture_root(
            output_root,
            pathlib.Path(plan["raw_capture_root"]),
        )
        for control_relative, candidate_relative in zip(
            clip["control_raw_frames"],
            clip["candidate_raw_frames"],
        ):
            control_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, control_relative),
            )
            candidate_path = _labeled_capture_path(
                output_root,
                _raw_artifact_path(raw_capture_root, candidate_relative),
            )
            source_pairs.append(
                {
                    "control_sha256": sha256_file(control_path),
                    "candidate_sha256": sha256_file(candidate_path),
                }
            )
        if sha256_object(source_pairs) != entry["source_pair_manifest_sha256"]:
            raise HarnessError(f"motion paired source manifest changed for {entry['code']}")
        mapped[entry["code"]] = {
            "clip_id": entry["clip_id"],
            "profile_id": entry["profile_id"],
            "lighting": entry["lighting"],
            "camera_id": entry["camera_id"],
        }
    if set(mapped) != set(public) or set(row["clip_id"] for row in mapped.values()) != set(clips):
        raise HarnessError("motion unblind map does not cover every planned clip")
    expected_order = sorted(
        clips,
        key=lambda clip_id: _blind_order_key(
            seed,
            f"motion-{clip_id}",
            clips[clip_id]["lighting"],
            MOTION_REVIEW_CONTEXT,
        ),
    )
    if [mapped[entry["code"]]["clip_id"] for entry in packet["entries"]] != expected_order:
        raise HarnessError("motion packet randomization order changed")
    return {
        "version": 1,
        "warning": WARNING,
        "packet_path": str(packet_path.resolve()),
        "packet_sha256": packet["sha256"],
        "unblind_map_path": str(unblind_map_path),
        "unblind_map_sha256": sha256_file(unblind_map_path),
        "code_map": mapped,
        "entry_count": 22,
    }


def build_blinded_motion_review_packet(
    plan_path: pathlib.Path,
    packet_root: pathlib.Path,
    unblind_map_path: pathlib.Path,
) -> Dict[str, Any]:
    """Materialize all 22 anonymous paired strips/clips for temporal review."""

    plan_path = plan_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    _validate_motion_deliverables_for_plan(plan)
    output_root = pathlib.Path(plan["output_root"]).resolve()
    packet_root = _require_blinded_packet_root(
        packet_root,
        output_root,
        "motion review packet root",
    )
    unblind_map_path = _require_outside_source_tree(
        unblind_map_path,
        "private motion unblind map",
    )
    for forbidden_root, label in ((output_root, "published output"), (packet_root, "public packet")):
        try:
            unblind_map_path.relative_to(forbidden_root)
        except ValueError:
            pass
        else:
            raise HarnessError(f"private motion unblind map may not live inside the {label}")
    motion_plan_sha256 = sha256_object(plan["motion"])
    salt = _load_or_create_private_blinding_salt(
        unblind_map_path,
        packet_kind="opaque-world-detail-motion-review",
        binding_sha256=motion_plan_sha256,
    )
    seed = _private_packet_blinding_seed(
        plan,
        salt,
        packet_kind="opaque-world-detail-motion-review",
        binding_sha256=motion_plan_sha256,
    )
    clips = {clip["id"]: clip for clip in plan["motion"]["clips"]}
    ordered = sorted(
        clips.values(),
        key=lambda clip: _blind_order_key(
            seed,
            f"motion-{clip['id']}",
            clip["lighting"],
            MOTION_REVIEW_CONTEXT,
        ),
    )
    entries = []
    private_entries = []
    for order, clip in enumerate(ordered, start=1):
        code = _blind_code(
            seed,
            f"motion-{clip['id']}",
            clip["lighting"],
            MOTION_REVIEW_CONTEXT,
        )
        strip_source = (output_root / clip["eight_frame_strip"]).resolve()
        mp4_source = (output_root / clip["mp4"]).resolve()
        media = _materialize_opaque_motion_packet_media(
            plan,
            clip,
            packet_root,
            code,
        )
        entries.append(
            {
                "order": order,
                "code": code,
                "strip": {
                    "path": media["strip_relative"],
                    "sha256": media["strip_sha256"],
                },
                "mp4": {
                    "path": media["mp4_relative"],
                    "sha256": media["mp4_sha256"],
                },
            }
        )
        private_entries.append(
            {
                "code": code,
                "clip_id": clip["id"],
                "profile_id": clip["profile_id"],
                "lighting": clip["lighting"],
                "camera_id": clip["camera_id"],
                "strip_source": {"path": str(strip_source), "sha256": sha256_file(strip_source)},
                "mp4_source": {"path": str(mp4_source), "sha256": sha256_file(mp4_source)},
                "review_strip_sha256": media["strip_sha256"],
                "review_mp4_sha256": media["mp4_sha256"],
                "source_pair_manifest_sha256": media["source_pair_manifest_sha256"],
                "strip_source_frame_sha256": media["strip_source_frame_sha256"],
            }
        )
    packet = {
        "version": 1,
        "warning": WARNING,
        "packet_kind": "opaque-world-detail-motion-review",
        "blinding_salt_commitment": sha256_bytes(salt.encode("utf-8")),
        "scoring_contract_sha256": sha256_object(scoring_contract()),
        "unscored_camera_ids": list(UNSCORED_CAMERA_IDS),
        "categories": list(CATEGORY_ORDER),
        "entries": entries,
    }
    packet_path = packet_root / "packet.json"
    _write_if_absent_or_equal(packet_path, pretty_json(packet))
    unblind = {
        "version": 1,
        "warning": WARNING,
        "status": "FINALIZED",
        "packet_kind": "opaque-world-detail-motion-review",
        "binding_sha256": motion_plan_sha256,
        "motion_plan_sha256": motion_plan_sha256,
        "review_packet_sha256": sha256_file(packet_path),
        "blinding_salt": salt,
        "entries": private_entries,
    }
    _finalize_private_blinding_evidence(unblind_map_path, unblind)
    return validate_blinded_motion_review_packet(plan_path, packet_path, unblind_map_path)


def _load_pixels(path: pathlib.Path) -> Any:
    try:
        import numpy as np  # pylint: disable=import-outside-toplevel
        from PIL import Image  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("NumPy and Pillow are required for image metrics") from error
    with Image.open(path) as image:
        pixels = np.asarray(image.convert("RGB"), dtype=np.float64) / 255.0
    if pixels.shape != (CAPTURE_HEIGHT, CAPTURE_WIDTH, 3):
        raise HarnessError(f"metric image has unexpected dimensions: {path}")
    return pixels


def decoded_rgb_sha256(path: pathlib.Path) -> str:
    """Hash decoded RGB pixels plus dimensions, independent of PNG encoding/metadata."""

    try:
        from PIL import Image  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to hash decoded RGB pixels") from error
    try:
        with Image.open(path) as image:
            rgb = image.convert("RGB")
            payload = struct.pack(">II", rgb.width, rgb.height) + rgb.tobytes()
    except OSError as error:
        raise HarnessError(f"cannot decode RGB pixels from {path}: {error}") from error
    return sha256_bytes(payload)


def _rgb_to_lab(rgb: Any) -> Any:
    import numpy as np  # pylint: disable=import-outside-toplevel

    linear = np.where(rgb <= 0.04045, rgb / 12.92, ((rgb + 0.055) / 1.055) ** 2.4)
    matrix = np.array(
        [
            [0.4124564, 0.3575761, 0.1804375],
            [0.2126729, 0.7151522, 0.0721750],
            [0.0193339, 0.1191920, 0.9503041],
        ]
    )
    xyz = linear @ matrix.T
    xyz = xyz / np.array([0.95047, 1.0, 1.08883])
    delta = 6.0 / 29.0
    transformed = np.where(
        xyz > delta**3,
        np.cbrt(xyz),
        xyz / (3.0 * delta * delta) + 4.0 / 29.0,
    )
    return np.stack(
        (
            116.0 * transformed[..., 1] - 16.0,
            500.0 * (transformed[..., 0] - transformed[..., 1]),
            200.0 * (transformed[..., 1] - transformed[..., 2]),
        ),
        axis=-1,
    )


def _delta_e_2000(lab1: Any, lab2: Any) -> Any:
    """Vectorized Sharma et al. CIEDE2000 with unit parametric weights."""

    import numpy as np  # pylint: disable=import-outside-toplevel

    l1, a1, b1 = (lab1[..., index] for index in range(3))
    l2, a2, b2 = (lab2[..., index] for index in range(3))
    c1 = np.hypot(a1, b1)
    c2 = np.hypot(a2, b2)
    c_bar = (c1 + c2) / 2.0
    c_bar_7 = c_bar**7
    g = 0.5 * (1.0 - np.sqrt(c_bar_7 / (c_bar_7 + 25.0**7)))
    a1p = (1.0 + g) * a1
    a2p = (1.0 + g) * a2
    c1p = np.hypot(a1p, b1)
    c2p = np.hypot(a2p, b2)
    h1p = np.mod(np.degrees(np.arctan2(b1, a1p)), 360.0)
    h2p = np.mod(np.degrees(np.arctan2(b2, a2p)), 360.0)
    dl = l2 - l1
    dc = c2p - c1p
    dh = h2p - h1p
    dh = np.where(c1p * c2p == 0.0, 0.0, dh)
    dh = np.where(dh > 180.0, dh - 360.0, dh)
    dh = np.where(dh < -180.0, dh + 360.0, dh)
    d_h = 2.0 * np.sqrt(c1p * c2p) * np.sin(np.radians(dh) / 2.0)
    l_bar = (l1 + l2) / 2.0
    cp_bar = (c1p + c2p) / 2.0
    hp_sum = h1p + h2p
    hp_diff = np.abs(h1p - h2p)
    hp_bar = np.where(c1p * c2p == 0.0, hp_sum, hp_sum / 2.0)
    hp_bar = np.where((c1p * c2p != 0.0) & (hp_diff > 180.0) & (hp_sum < 360.0), (hp_sum + 360.0) / 2.0, hp_bar)
    hp_bar = np.where((c1p * c2p != 0.0) & (hp_diff > 180.0) & (hp_sum >= 360.0), (hp_sum - 360.0) / 2.0, hp_bar)
    t = (
        1.0
        - 0.17 * np.cos(np.radians(hp_bar - 30.0))
        + 0.24 * np.cos(np.radians(2.0 * hp_bar))
        + 0.32 * np.cos(np.radians(3.0 * hp_bar + 6.0))
        - 0.20 * np.cos(np.radians(4.0 * hp_bar - 63.0))
    )
    sl = 1.0 + 0.015 * (l_bar - 50.0) ** 2 / np.sqrt(20.0 + (l_bar - 50.0) ** 2)
    sc = 1.0 + 0.045 * cp_bar
    sh = 1.0 + 0.015 * cp_bar * t
    delta_theta = 30.0 * np.exp(-((hp_bar - 275.0) / 25.0) ** 2)
    cp_bar_7 = cp_bar**7
    rc = 2.0 * np.sqrt(cp_bar_7 / (cp_bar_7 + 25.0**7))
    rt = -np.sin(np.radians(2.0 * delta_theta)) * rc
    return np.sqrt((dl / sl) ** 2 + (dc / sc) ** 2 + (d_h / sh) ** 2 + rt * (dc / sc) * (d_h / sh))


def image_metrics(control: pathlib.Path, candidate: pathlib.Path) -> Dict[str, Any]:
    """Compute exact hash, standard Gaussian-window SSIM, and mean CIEDE2000."""

    import numpy as np  # pylint: disable=import-outside-toplevel
    try:
        from scipy.ndimage import gaussian_filter  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("SciPy is required for windowed SSIM") from error

    control_pixels = _load_pixels(control)
    candidate_pixels = _load_pixels(candidate)
    weights = np.array([0.2126, 0.7152, 0.0722])
    x = control_pixels @ weights
    y = candidate_pixels @ weights
    mu_x = gaussian_filter(x, sigma=1.5, mode="reflect", truncate=3.5)
    mu_y = gaussian_filter(y, sigma=1.5, mode="reflect", truncate=3.5)
    var_x = gaussian_filter(x * x, sigma=1.5, mode="reflect", truncate=3.5) - mu_x * mu_x
    var_y = gaussian_filter(y * y, sigma=1.5, mode="reflect", truncate=3.5) - mu_y * mu_y
    covariance = gaussian_filter(x * y, sigma=1.5, mode="reflect", truncate=3.5) - mu_x * mu_y
    c1 = 0.01**2
    c2 = 0.03**2
    ssim_map = ((2.0 * mu_x * mu_y + c1) * (2.0 * covariance + c2)) / (
        (mu_x * mu_x + mu_y * mu_y + c1) * (var_x + var_y + c2)
    )
    ssim = float(ssim_map[5:-5, 5:-5].mean())
    mean_delta_e = float(_delta_e_2000(_rgb_to_lab(control_pixels), _rgb_to_lab(candidate_pixels)).mean())
    try:
        import numpy as np  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("NumPy is required to compare decoded RGB pixels") from error
    exact_duplicate = bool(np.array_equal(control_pixels, candidate_pixels))
    return {
        "control": str(control),
        "candidate": str(candidate),
        "control_sha256": sha256_file(control),
        "candidate_sha256": sha256_file(candidate),
        "control_rgb_sha256": decoded_rgb_sha256(control),
        "candidate_rgb_sha256": decoded_rgb_sha256(candidate),
        "exact_duplicate": exact_duplicate,
        "ssim": ssim,
        "mean_delta_e00": mean_delta_e,
        "near_duplicate": ssim >= 0.995 and mean_delta_e <= 1.5,
        "thresholds": {"ssim_min": 0.995, "mean_delta_e00_max": 1.5},
    }


def build_metric_evidence(plan_path: pathlib.Path) -> Dict[str, Any]:
    """Compute the strict named-camera control comparison for all 60 atomic treatments."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = pathlib.Path(plan["raw_capture_root"])
    stage_one = [
        slot for slot in plan["study"]["logical_slots"] if slot["stage"] == "01-neutral-screen"
    ]
    controls = {
        (row["lighting"], row["camera_id"]): _raw_artifact_path(
            raw_capture_root, row["artifact"]
        )
        for row in plan["study"]["shared_control_references"]
    }
    comparisons = []
    for profile in atomic_profiles():
        matches = [
            slot
            for slot in stage_one
            if slot["profile_id"] == profile.id
            and slot["camera_id"] == PRIMARY_CAMERAS[profile.family]
        ]
        if len(matches) != 1:
            raise HarnessError(f"metric evidence cannot resolve {profile.id} named capture")
        camera_id = matches[0]["camera_id"]
        control_path = controls[("neutral", camera_id)]
        candidate_path = _raw_artifact_path(raw_capture_root, matches[0]["artifact"])
        metric = image_metrics(control_path, candidate_path)
        comparisons.append(
            {
                "subject_id": profile.id,
                "camera_id": camera_id,
                "control_sha256": sha256_file(control_path),
                "candidate_sha256": sha256_file(candidate_path),
                "control_rgb_sha256": metric["control_rgb_sha256"],
                "candidate_rgb_sha256": metric["candidate_rgb_sha256"],
                "ssim": metric["ssim"],
                "mean_delta_e00": metric["mean_delta_e00"],
                "exact_duplicate": metric["exact_duplicate"],
                "near_duplicate": metric["near_duplicate"],
            }
        )
    result = {"version": 1, "warning": WARNING, "comparisons": comparisons}
    return validate_metric_evidence(result)


def build_selection_performance_evidence(plan_path: pathlib.Path) -> Dict[str, Any]:
    """Aggregate atomic neutral capture telemetry for deterministic score tie-breaking."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    subjects = {}
    for job in plan["study"]["jobs"]:
        if job["stage"] != "01-neutral-screen":
            continue
        record = _validate_job_artifacts(
            job,
            output_root,
            source_provenance_sha256=sha256_object(plan["provenance"]),
            label=False,
            require_labeled=True,
        )
        summary = _job_performance_summary(record)
        subjects[job["look_id"]] = {
            "p95_frame_time_ms": summary["p95_frame_time_ms"],
            "max_resident_presentation_bytes": summary["max_resident_presentation_bytes"],
        }
    control_reference = next(
        row
        for row in plan["study"]["shared_control_references"]
        if row["lighting"] == "neutral" and row["camera_id"] == "02-highlands-oblique"
    )
    control_jobs = [
        job
        for job in plan["study"]["jobs"]
        if job["stage"] == "00-shared-control"
        and control_reference["artifact"] in job["artifacts"]
    ]
    if len(control_jobs) != 1:
        raise HarnessError("selection performance cannot resolve the fresh neutral control job")
    control_job = control_jobs[0]
    control_index = control_job["artifacts"].index(control_reference["artifact"])
    if control_job["cameras"][control_index]["id"] != "02-highlands-oblique":
        raise HarnessError("selection performance fresh control camera binding changed")
    control_record = _validate_job_artifacts(
        control_job,
        output_root,
        source_provenance_sha256=sha256_object(plan["provenance"]),
        label=False,
        require_labeled=True,
    )
    control_sample = control_record["performance_samples"][control_index]
    subjects["control"] = {
        "p95_frame_time_ms": control_sample["frame_time_ms"],
        "max_resident_presentation_bytes": control_sample["resident_presentation_bytes"],
    }
    result = {"version": 1, "warning": WARNING, "subjects": subjects}
    return validate_selection_performance_evidence(result)


def validate_recomputed_selection_evidence(
    plan_path: pathlib.Path,
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
) -> Dict[str, Any]:
    """Rebuild metric/performance evidence from raw pixels and runtime sidecars.

    Supplied JSON is an audit/export format, never an authority.  Finalization
    accepts it only when byte-independent deterministic recomputation produces
    the exact same normalized objects.
    """

    supplied_metrics = validate_metric_evidence(_read_json(metric_path, "metric evidence"))
    supplied_performance = validate_selection_performance_evidence(
        _read_json(performance_path, "selection performance evidence")
    )
    recomputed_metrics = build_metric_evidence(plan_path)
    recomputed_performance = build_selection_performance_evidence(plan_path)
    if supplied_metrics != recomputed_metrics:
        raise HarnessError(
            "metric evidence differs from SSIM/DeltaE recomputed from bound raw frames"
        )
    if supplied_performance != recomputed_performance:
        raise HarnessError(
            "performance evidence differs from bound runtime capture sidecars"
        )
    return {
        "version": 1,
        "warning": WARNING,
        "metric_path": str(metric_path.resolve()),
        "metric_sha256": sha256_file(metric_path),
        "performance_path": str(performance_path.resolve()),
        "performance_sha256": sha256_file(performance_path),
        "comparisons_recomputed": len(recomputed_metrics["comparisons"]),
        "performance_subjects_recomputed": len(recomputed_performance["subjects"]),
        "raw_pixels_recomputed": True,
        "runtime_sidecars_recomputed": True,
    }


def scoring_contract() -> Dict[str, Any]:
    """Return the immutable scoring/gating contract without review outcomes."""

    return {
        "independent_blinded_reviews_required": 2,
        "rating_scale": {"minimum": 1, "maximum": 5},
        "weights": CATEGORY_WEIGHTS,
        "weighted_score_formula": "20 * sum(mean_two_reviewers_rating * weight)",
        "promotion_floor": {
            "terrain_route_water_edge_readability": 3,
            "edge_temporal_quietness": 3,
            "applies_to_each_reviewer_individually": True,
        },
        "near_duplicate": {"ssim_min": 0.995, "mean_delta_e00_max": 1.5},
        "near_duplicate_handling": (
            "prefer two visible passing candidates when available; near-duplicates cannot be recommended"
        ),
        "tie_break_order": (
            "terrain_route_water_edge_readability",
            "shadow_occlusion_preservation",
            "edge_temporal_quietness",
            "runtime_cost",
        ),
        "stress_gate": {
            "minimum_weighted_score": 65,
            "minimum_category_rating": 2,
            "maximum_drop_from_neutral": 12,
            "per_reviewer_readability_minimum": 3,
            "per_reviewer_edge_temporal_quietness_minimum": 3,
        },
        "recommendation_margin": {"atomic_over_control": 2, "combination_over_control": 3},
        "final_combination_performance": {
            "p95_frame_time_maximum_over_control": 0.15,
            "resident_presentation_memory_maximum_over_control": 0.15,
        },
        "p95_method": "nearest-rank",
        "selection_derivation": "deterministic recomputation from two hash-pinned blinded reviews",
        "blinding": {
            "review_rows_keyed_only_by_opaque_code": True,
            "public_packet_and_private_unblind_map_both_hash_required": True,
            "private_random_salt_with_public_commitment": True,
            "packet_media_reencoded_with_opaque_only_metadata": True,
            "packet_root_disjoint_from_unblinded_output": True,
            "randomness_scope": "review-packet-only",
        },
        "scoring_contexts": {
            DIAGNOSTIC_REVIEW_CONTEXT: {
                "camera_count": 4,
                "camera_ids": [
                    "02-highlands-oblique",
                    "03-coast-river-outlet",
                    "14-cascade-basin-full-height",
                    "16-deep-tree-shade",
                ],
            },
            FINAL_REVIEW_CONTEXT: {
                "camera_count": 17,
                "unscored_camera_ids": list(UNSCORED_CAMERA_IDS),
            },
            "same_context_required_for_score_deltas": True,
        },
        "interaction_ladders": {
            "neutral_logical_slots": 36,
            "cumulative_steps": len(LADDER_STEPS),
            "per_reviewer_readability_minimum": 3,
            "per_reviewer_edge_quietness_minimum": 3,
            "maximum_weighted_drop_from_predecessor": 2.0,
            "failed_step_vetoes_introduced_families_to_control": True,
            "vegetation_and_cliff_are_coupled": True,
        },
        "paired_motion_review": {
            "clips": 22,
            "same_two_reviewer_ids_required": True,
            "review_rows_keyed_only_by_opaque_code": True,
            "per_reviewer_readability_minimum": 3,
            "per_reviewer_edge_temporal_quietness_minimum": 3,
            "risky_family_failed_winner_tries_other_passing_finalist": True,
            "single_winner_family_failure_resolves_to_control": True,
            "failed_combination_is_not_recommendable": True,
        },
        "unrecorded_scores_or_winners_supplied_by_harness": False,
    }


def build_capture_document(
    output_root: pathlib.Path,
    selection_raw: Optional[Any] = None,
    *,
    raw_capture_root: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Build a self-contained capture document around still and motion plans."""

    output_root = _require_outside_source_tree(output_root, "output root")
    raw_capture_root = _validate_raw_capture_root(output_root, raw_capture_root)
    selection = validate_selection(selection_raw or selection_template())
    provenance = source_provenance()
    study = build_still_plan(output_root, selection, raw_capture_root=raw_capture_root)
    motion = build_motion_plan(output_root, selection, raw_capture_root=raw_capture_root)
    return {
        "version": 1,
        "warning": WARNING,
        "status": study["status"],
        "output_root": str(output_root),
        "raw_capture_root": str(raw_capture_root),
        "provenance": provenance,
        "selection": selection,
        "capture_contract": {
            "cargo_command": [
                "cargo",
                "run",
                "--locked",
                "--release",
                "-p",
                "hex_game",
                "--features",
                "map-review",
            ],
            "profile_environment": "HEX_REVIEW_WORLD_DETAIL",
            "capture_plan_environment": "HEX_REVIEW_CAPTURE_PLAN",
            "launch_nonce_environment": "HEX_REVIEW_LAUNCH_NONCE",
            "source_provenance_environment": "HEX_REVIEW_SOURCE_PROVENANCE_SHA256",
            "structural_draft_environment": {
                "name": STRUCTURAL_DRAFT_ENVIRONMENT,
                "value": STRUCTURAL_DRAFT_VALUE,
                "scope": "map-review-only",
                "inherited_state_scrubbed_before_injection": True,
            },
            "capture_plan_versions": {
                "still_multi_camera": 1,
                "motion_per_entry_phase_sequence": 2,
            },
            "profile_omitted_for_control": True,
            "explicit_current_control_verification": True,
            "focused_control_process_policy": {
                "camera_ids": list(BASELINE_ORACLE_CAMERA_IDS),
                "omitted_profile_jobs": 4,
                "explicit_current_jobs": 4,
                "captures_per_job": 1,
                "fresh_process_per_png": True,
                "runtime_receipts_must_be_distinct": True,
            },
            "review_evidence_schema": REVIEW_SCHEMA_PATH.relative_to(REPOSITORY_ROOT).as_posix(),
            "runtime_evidence_contract": RUNTIME_EVIDENCE_CONTRACT_PATH.relative_to(
                REPOSITORY_ROOT
            ).as_posix(),
            "runtime_sidecar_rule": "foo.png -> foo.world-detail-report.json",
            "runtime_sidecar": {
                "version": 1,
                "warning": WARNING,
                "capture_fields": [
                    "path",
                    "camera",
                    "view",
                    "focus_anchor",
                    "look_at_anchor",
                    "look_at_offset",
                    "character_radius_scale",
                    "full_cutaway",
                    "illumination_overlay",
                    "time_hours",
                    "liquid_phase_seconds",
                    "settle_frames",
                ],
                "optional_capture_values_serialize_as_null": True,
                "report_fields": [
                    "version",
                    "profile_hash_sha256",
                    "runtime_receipt",
                    "authority",
                    "counts",
                    "anchor_heights",
                    "anchor_classes",
                    "projection_hashes",
                    "effect_validation",
                    "camera_features",
                    "performance",
                    "cleanup",
                ],
                "performance": {
                    "frame_time_ms": (
                        "nearest-rank p95 from the final 60 frames of the first 90-frame settle "
                        "window, cached across the same process sequence; "
                        "finite f32 in (0,10000]"
                    ),
                    "resident_presentation_bytes": (
                        "positive u64 total resident presentation assets for the matched scene; "
                        "unique live Mesh3d buffers, relevant live materials, non-capture image "
                        "mip payloads, and review entity/component/name payloads"
                    ),
                    "warmup_complete": "bool",
                },
                "cleanup": {
                    "completed_cycles": "positive integer; finalized after verified teardown",
                    "entities_remaining": "exactly 0 after teardown",
                    "materials_remaining": "exactly 0 after teardown",
                    "meshes_remaining": "exactly 0 after teardown",
                    "target_images_remaining": "exactly 0 after teardown",
                    "restoration_flags": [
                        "camera_state_restored",
                        "oit_state_restored",
                        "transmission_state_restored",
                        "depth_state_restored",
                        "volumetric_state_restored",
                    ],
                    "all_restoration_flags_must_be_true": True,
                    "hundred_cycle_evidence": "standalone hash-linked lifecycle certificate",
                },
            },
            "lifecycle_certificate": {
                "version": 1,
                "cycles": 100,
                "hash_linked_to_capture_plan_source_profile_matrix_and_tested_profile": True,
                "per_cycle_remaining_counts": [
                    "entities_remaining",
                    "materials_remaining",
                    "meshes_remaining",
                    "fog_density_images_remaining",
                    "target_images_remaining",
                    "terrain_material_overrides_remaining",
                    "liquid_visibility_overrides_remaining",
                    "vegetation_scale_overrides_remaining",
                ],
                "all_per_cycle_remaining_counts_must_equal": 0,
                "hash_chain": "sha256(canonical compact cycle without cycle_sha256)",
            },
            "deterministic_reproduction": {
                "new_renders": 1,
                "preferred_subject": "combination-score-leader",
                "camera_id": "02-highlands-oblique",
                "lighting": "neutral",
                "control_fallback": "explicit-current",
                "stable_fields": [
                    "all raster-stable decoded RGB pixels",
                    "spatial exception limited to clean-source-proven ambiguous shared-vertex coordinates",
                    "treatment-specific differing endpoint RGB tuples recorded without a numeric threshold",
                    "profile and camera state",
                    "authority and counts",
                    "anchor heights/classes",
                    "projection hashes",
                    "effect validation",
                    "camera features",
                ],
                "fresh_runtime_receipts_required": True,
            },
            "resolution": [CAPTURE_WIDTH, CAPTURE_HEIGHT],
            "genuine_game_renders_only": True,
            "generative_substitution_allowed": False,
        },
        "study": study,
        "motion": motion,
        "scoring": scoring_contract(),
        "acceptance_hooks": {
            "fresh_runtime_controls": "study.shared_control_references",
            "selection": "provenance/selection.json",
            "reviewer_evidence": ["external blinded-review-a.json", "external blinded-review-b.json"],
            "metric_evidence": "provenance/metric-evidence.json",
            "selection_performance_evidence": "provenance/selection-performance-evidence.json",
            "runtime_capture_state": "external --work-root/capture-state.json",
            "lifecycle_certificate": "external runtime-written lifecycle-certificate.json",
            "deterministic_reproduction": "study.reproduction_jobs",
            "no_midnight": True,
            "cleanup_cycles_required": 100,
            "publication_validation_required": True,
        },
    }


def validate_capture_document(raw: Any) -> Dict[str, Any]:
    """Require a capture document to be an exact regeneration of its inputs."""

    plan = _strict_object(
        raw,
        context="capture document",
        required=(
            "version",
            "warning",
            "status",
            "output_root",
            "raw_capture_root",
            "provenance",
            "selection",
            "capture_contract",
            "study",
            "motion",
            "scoring",
            "acceptance_hooks",
        ),
    )
    if plan["version"] != 1 or plan["warning"] != WARNING:
        raise HarnessError("capture document version or warning changed")
    _assert_source_provenance(plan["provenance"])
    expected = json.loads(
        compact_json(
            build_capture_document(
                pathlib.Path(plan["output_root"]),
                plan["selection"],
                raw_capture_root=pathlib.Path(plan["raw_capture_root"]),
            )
        )
    )
    if plan != expected:
        raise HarnessError("capture document differs from deterministic regeneration")
    return plan


def _write_if_absent_or_equal(path: pathlib.Path, content: str) -> None:
    if path.exists():
        try:
            current = path.read_text(encoding="utf-8")
        except OSError as error:
            raise HarnessError(f"cannot read existing scaffold file {path}: {error}") from error
        if current != content:
            raise HarnessError(f"refusing to overwrite changed scaffold file {path}")
        return
    atomic_write(path, content)


def _replace_generated_json(path: pathlib.Path, value: Mapping[str, Any]) -> None:
    """Refresh one harness-generated JSON file while protecting foreign evidence."""

    if path.exists():
        existing = _read_json(path, "existing generated scaffold")
        if not isinstance(existing, dict) or existing.get("version") != 1 or existing.get("warning") != WARNING:
            raise HarnessError(f"refusing to replace non-harness JSON {path}")
    atomic_write(path, pretty_json(value))


def _replace_generated_gallery_html(path: pathlib.Path, content: str) -> None:
    """Refresh only a recognizable harness-owned gallery document."""

    if path.exists():
        try:
            current = path.read_text(encoding="utf-8")
        except OSError as error:
            raise HarnessError(f"cannot read existing gallery HTML {path}: {error}") from error
        if (
            "<title>Crystal Ascent — small-geometry aesthetic review</title>" not in current
            or WARNING not in current
        ):
            raise HarnessError(f"refusing to replace non-harness gallery HTML {path}")
    atomic_write(path, content)


def _gallery_data(plan: Mapping[str, Any]) -> Dict[str, Any]:
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    slots = []
    for slot in plan["study"]["logical_slots"]:
        raw_path = _raw_artifact_path(raw_capture_root, slot["artifact"])
        labeled_path = _labeled_capture_path(output_root, raw_path)
        slots.append(
            {
                "logical_slot": slot["logical_slot"],
                "stage": slot["stage"],
                "look_id": slot["look_id"],
                "profile_id": slot["profile_id"],
                "family": slot["profile_family"],
                "lighting": slot["lighting"],
                "camera_id": slot["camera_id"],
                "image": os.path.relpath(labeled_path, output_root / "gallery"),
                "reuse": slot["reuse"],
                "fresh_control": slot["fresh_control"],
            }
        )
    return {
        "version": 1,
        "warning": WARNING,
        "status": plan["status"],
        "slots": slots,
    }


def _gallery_html(gallery_data: Mapping[str, Any]) -> str:
    """Return a genuinely ``file://``-safe gallery with its ledger embedded.

    Browsers commonly block ``fetch('data.json')`` for a local HTML file.  The
    synchronized JSON is still published as a separately inspectable artifact,
    but the exact same value is embedded here so opening ``index.html`` directly
    never depends on an HTTP server.
    """

    if gallery_data.get("version") != 1 or gallery_data.get("warning") != WARNING:
        raise HarnessError("gallery HTML input has the wrong identity")
    embedded_data = compact_json(gallery_data)
    return f"""<!doctype html>
<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\">
<title>Crystal Ascent — small-geometry aesthetic review</title>
<style>body{{margin:0;background:#11151b;color:#edf1f5;font:15px system-ui,sans-serif}}header{{position:sticky;top:0;background:#171d26;padding:16px;z-index:2}}.warning{{color:#ffd648;font-weight:800}}label{{margin-right:16px}}select{{margin-left:5px}}main{{display:grid;grid-template-columns:repeat(auto-fill,minmax(360px,1fr));gap:14px;padding:14px}}figure{{margin:0;background:#202834;padding:10px}}img{{display:block;width:100%;background:#08090b}}figcaption{{padding-top:8px;line-height:1.35}}small{{color:#aeb9c6}}</style></head>
<body><header><div class=\"warning\">{WARNING}</div><p>Offline synchronized logical-slot gallery. Missing images remain visibly unresolved.</p><p><strong>Accounting:</strong> all 25 controls are fresh current-source renders gated against the clean bc06 oracle. Each of the four focused neutral controls and each of its four explicit-current mates runs in a separate one-camera process. The omitted controls double as verification pixels. The 611 ceiling applies only to unique non-control treatment PNGs (596 maximum); 25 controls, four explicit-current PNGs, one deterministic reproduction, and four primary oracle PNGs are retained under the separate 630 total-evidence ceiling. No blinded outcome is altered to create reuse. The 22 stability repeats are separately disclosed pre-study qualification evidence.</p>
<label>Stage <select id=\"stage\"><option value=\"\">all</option></select></label><label>Family <select id=\"family\"><option value=\"\">all</option></select></label><label>Camera <select id=\"camera\"><option value=\"\">all</option></select></label><label>Lighting <select id=\"lighting\"><option value=\"\">all</option></select></label></header><main id=\"gallery\"></main>
<script>
const exactWarning={json.dumps(WARNING)};const data={embedded_data};let rows=[];
const filters=['stage','family','camera','lighting'];
function optionValues(key){{return [...new Set(rows.map(r=>r[key==='camera'?'camera_id':key]))].sort()}}
function render(){{const chosen=Object.fromEntries(filters.map(k=>[k,document.getElementById(k).value]));const root=document.getElementById('gallery');root.textContent='';for(const row of rows){{if(filters.some(k=>chosen[k]&&row[k==='camera'?'camera_id':k]!==chosen[k]))continue;const f=document.createElement('figure');const im=document.createElement('img');im.src=row.image;im.alt=exactWarning+' — '+row.profile_id+' — '+row.camera_id;const cap=document.createElement('figcaption');cap.textContent=`${{row.logical_slot}} · ${{row.profile_id}} · ${{row.camera_id}}`;const sm=document.createElement('small');sm.textContent=`${{row.stage}} · ${{row.lighting}}${{row.reuse?' · semantic reuse':''}}`;cap.append(document.createElement('br'),sm);f.append(im,cap);root.append(f)}}}}
if(data.warning!==exactWarning)throw Error('warning contract mismatch');rows=data.slots;for(const key of filters){{const select=document.getElementById(key);for(const value of optionValues(key)){{const o=document.createElement('option');o.value=value;o.textContent=value;select.append(o)}}select.addEventListener('change',render)}}render();
</script></body></html>"""


def _write_contact_sheet(
    items: Sequence[Tuple[str, pathlib.Path]],
    destination: pathlib.Path,
    *,
    title: str,
) -> Dict[str, Any]:
    """Compose one warning-labelled comparison sheet from genuine labeled captures."""

    try:
        from PIL import Image, ImageDraw, ImageFont, PngImagePlugin  # pylint: disable=import-outside-toplevel
    except ImportError as error:
        raise HarnessError("Pillow is required to build report comparison sheets") from error
    if not items:
        raise HarnessError(f"comparison sheet {title!r} has no source images")
    source_records = []
    for label, path in items:
        if not isinstance(label, str) or not label:
            raise HarnessError(f"comparison sheet {title!r} has an empty label")
        source_records.append((label, path, _inspect_labeled_png(path)))
    source_manifest = [
        {"label": label, "path": str(path), "sha256": record["sha256"]}
        for label, path, record in source_records
    ]
    source_manifest_sha256 = sha256_object(source_manifest)
    if destination.exists():
        existing = _inspect_labeled_png(destination, exact_capture_size=False)
        try:
            with Image.open(destination) as image:
                bound_hash = image.info.get("source_render_manifest_sha256")
                bound_title = image.info.get("comparison_sheet_title")
        except OSError as error:
            raise HarnessError(f"cannot inspect comparison sheet {destination}: {error}") from error
        if bound_hash != source_manifest_sha256 or bound_title != title:
            raise HarnessError(f"comparison sheet is stale: {destination}")
        return {**existing, "title": title, "sources": source_manifest}

    columns = min(4, len(items))
    tile_width = 480
    tile_image_height = 270
    tile_label_height = 42
    header_height = 86
    rows = math.ceil(len(items) / columns)
    canvas = Image.new(
        "RGB",
        (columns * tile_width, header_height + rows * (tile_image_height + tile_label_height)),
        (12, 15, 20),
    )
    draw = ImageDraw.Draw(canvas)
    try:
        warning_font = ImageFont.truetype("Arial.ttf", 22)
        title_font = ImageFont.truetype("Arial.ttf", 20)
        label_font = ImageFont.truetype("Arial.ttf", 16)
    except OSError:
        warning_font = ImageFont.load_default()
        title_font = ImageFont.load_default()
        label_font = ImageFont.load_default()
    draw.rectangle((0, 0, canvas.width, 44), fill=(8, 8, 10))
    draw.text((16, 10), WARNING, fill=(255, 214, 72), font=warning_font)
    draw.text((16, 54), title, fill=(230, 236, 242), font=title_font)
    resampling = getattr(Image, "Resampling", Image)
    for index, (label, source_path, _) in enumerate(source_records):
        column = index % columns
        row = index // columns
        x = column * tile_width
        y = header_height + row * (tile_image_height + tile_label_height)
        with Image.open(source_path) as source:
            tile = source.convert("RGB").resize((tile_width, tile_image_height), resampling.LANCZOS)
        canvas.paste(tile, (x, y))
        draw.rectangle(
            (x, y + tile_image_height, x + tile_width, y + tile_image_height + tile_label_height),
            fill=(24, 30, 39),
        )
        draw.text((x + 10, y + tile_image_height + 11), label, fill=(220, 228, 237), font=label_font)
    metadata = PngImagePlugin.PngInfo()
    metadata.add_text("structural_draft_warning", WARNING)
    metadata.add_text("source_render_manifest_sha256", source_manifest_sha256)
    metadata.add_text("comparison_sheet_title", title)
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    canvas.save(temporary, format="PNG", pnginfo=metadata, optimize=False)
    os.replace(temporary, destination)
    rendered = _inspect_labeled_png(destination, exact_capture_size=False)
    return {**rendered, "title": title, "sources": source_manifest}


def _require_publication_capture_plan_path(
    plan_path: pathlib.Path,
    plan: Mapping[str, Any],
) -> Tuple[pathlib.Path, pathlib.Path]:
    """Require publication to use the report root's one canonical capture plan."""

    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    resolved_plan_path = plan_path.resolve()
    expected_plan_path = (output_root / "capture-plan.json").resolve()
    if (
        resolved_plan_path != expected_plan_path
        or not plan_path.is_file()
        or plan_path.is_symlink()
    ):
        raise HarnessError(
            "publication capture plan must be the regular output-root/capture-plan.json file"
        )
    return resolved_plan_path, output_root


def _publication_sheet_specs(plan: Mapping[str, Any]) -> Dict[str, List[Dict[str, Any]]]:
    """Return the one ordered source/title/path specification for every report sheet."""

    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = pathlib.Path(plan["raw_capture_root"])
    controls = {
        (row["lighting"], row["camera_id"]): _labeled_capture_path(
            output_root, _raw_artifact_path(raw_capture_root, row["artifact"])
        )
        for row in plan["study"]["shared_control_references"]
    }
    slots = plan["study"]["logical_slots"]
    family_specs = []
    for family in FAMILY_ORDER:
        camera_id = PRIMARY_CAMERAS[family]
        candidates = [
            slot
            for slot in slots
            if slot["stage"] == "01-neutral-screen"
            and slot["profile_family"] == family
            and slot["camera_id"] == camera_id
        ]
        expected_ids = [profile.id for profile in atomic_profiles() if profile.family == family]
        by_id = {slot["profile_id"]: slot for slot in candidates}
        if set(by_id) != set(expected_ids):
            raise HarnessError(f"family contact sheet inputs changed for {family}")
        items = [("CURRENT / CONTROL", controls[("neutral", camera_id)])]
        items.extend(
            (
                profile_id,
                _labeled_capture_path(
                    output_root,
                    _raw_artifact_path(raw_capture_root, by_id[profile_id]["artifact"]),
                ),
            )
            for profile_id in expected_ids
        )
        destination = output_root / "contact-sheets" / f"{family}.png"
        family_specs.append(
            {
                "path": destination,
                "title": f"{family} — neutral named camera {camera_id}",
                "items": items,
            }
        )

    final_specs = []
    final_cameras, focused_cameras, _ = load_camera_sets()
    for camera in final_cameras:
        camera_slots = [
            slot
            for slot in slots
            if slot["stage"] == "07-final-17"
            and slot["lighting"] == "neutral"
            and slot["camera_id"] == camera.id
        ]
        if len(camera_slots) != 13:
            raise HarnessError(f"neutral final sheet does not have 13 looks at {camera.id}")
        items = [
            (
                slot["look_id"],
                _labeled_capture_path(
                    output_root, _raw_artifact_path(raw_capture_root, slot["artifact"])
                ),
            )
            for slot in camera_slots
        ]
        final_specs.append(
            {
                "path": (
                    output_root
                    / "final-comparison-sheets"
                    / "neutral"
                    / f"{camera.id}.png"
                ),
                "title": f"Final atomic/combination comparison — neutral — {camera.id}",
                "items": items,
            }
        )
    for lighting_id in ("golden", "overcast"):
        for camera in focused_cameras:
            combination_slots = [
                slot
                for slot in slots
                if slot["stage"] == "08-combination-stress"
                and slot["lighting"] == lighting_id
                and slot["camera_id"] == camera.id
            ]
            if len(combination_slots) != 3:
                raise HarnessError(
                    f"final stress sheet does not have three combinations at {lighting_id}/{camera.id}"
                )
            items = [("CURRENT / CONTROL", controls[(lighting_id, camera.id)])]
            items.extend(
                (
                    slot["look_id"],
                    _labeled_capture_path(
                        output_root, _raw_artifact_path(raw_capture_root, slot["artifact"])
                    ),
                )
                for slot in combination_slots
            )
            final_specs.append(
                {
                    "path": (
                        output_root
                        / "final-comparison-sheets"
                        / lighting_id
                        / f"{camera.id}.png"
                    ),
                    "title": f"Final combinations — {lighting_id} — {camera.id}",
                    "items": items,
                }
            )
    return {"family_sheets": family_specs, "final_comparison_sheets": final_specs}


def build_publication_sheets(plan_path: pathlib.Path) -> Dict[str, Any]:
    """Build nine family sheets and 25 final comparison sheets from labeled renders."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    if plan["status"] != "READY_FOR_CAPTURE":
        raise HarnessError("publication sheets require a fully resolved capture plan")
    specs = _publication_sheet_specs(plan)
    family_sheets = [
        _write_contact_sheet(spec["items"], spec["path"], title=spec["title"])
        for spec in specs["family_sheets"]
    ]
    final_sheets = [
        _write_contact_sheet(spec["items"], spec["path"], title=spec["title"])
        for spec in specs["final_comparison_sheets"]
    ]
    result = {
        "version": 1,
        "warning": WARNING,
        "capture_plan_sha256": sha256_file(plan_path),
        "family_sheets": family_sheets,
        "final_comparison_sheets": final_sheets,
    }
    atomic_write(output_root / "publication-sheets.json", pretty_json(result))
    return result


def _materialize_prior_aesthetic_evidence(
    output_root: pathlib.Path,
    *,
    create: bool,
) -> Dict[str, Any]:
    """Vendor the hash-pinned prior bevel rejection evidence for offline review."""

    destination_root = output_root / "provenance" / "prior-aesthetic-report"
    records = []
    for filename, expected_sha256 in PRIOR_AESTHETIC_REPORT_HASHES.items():
        source = PRIOR_AESTHETIC_REPORT_ROOT / filename
        destination = destination_root / filename
        if not source.is_file() or source.is_symlink() or sha256_file(source) != expected_sha256:
            raise HarnessError(f"prior aesthetic evidence changed or is unavailable: {source}")
        if destination.exists():
            if (
                destination.is_symlink()
                or not destination.is_file()
                or sha256_file(destination) != expected_sha256
            ):
                raise HarnessError(f"offline prior aesthetic evidence changed: {destination}")
        elif create:
            destination.parent.mkdir(parents=True, exist_ok=True)
            temporary = destination.with_name(destination.name + ".tmp")
            if temporary.exists():
                raise HarnessError(f"stale prior-evidence temporary exists: {temporary}")
            shutil.copyfile(source, temporary)
            os.replace(temporary, destination)
        else:
            raise HarnessError(f"offline prior aesthetic evidence is missing: {destination}")
        records.append(
            {
                "filename": filename,
                "source": str(source),
                "destination": str(destination),
                "sha256": expected_sha256,
            }
        )
    return {
        "version": 1,
        "warning": WARNING,
        "records": records,
        "evidence_sha256": sha256_object(records),
    }


def _incomplete_readme_content() -> str:
    return f"""# Crystal Ascent small-geometry aesthetic report — capture scaffold

> {WARNING}

This report is incomplete. The harness has created a deterministic capture plan and an offline gallery index, but it has not assigned review scores, promoted candidates, selected winners, or written recommendations. Populate `provenance/selection.json` only from two recorded blinded reviews, regenerate the plan, capture genuine game renders, and pass artifact validation before publishing conclusions.

Accounting note: all 25 shared controls are rendered from the same current source provenance as their candidates and gated against the clean bc06 oracle. Each of the four focused neutral controls and each of its four explicit-current mates uses a separate one-camera process. The omitted controls double as the verification side. A separately scoped, hash-pinned six-run v7c qualification permits exactly one additional omitted-versus-explicit-current raster coordinate, camera `14` pixel `(1438,273)`, and only the exact RGB endpoints `[164,95,66]` and `[169,133,125]`. It does not alter the clean baseline-oracle contract, reproduction mask, or any treatment comparison, and permits no numeric or spatial tolerance. The qualification pack also records that per-process environment snapshots and standalone exit-code receipts were not retained; its six runtime receipts, reports, teardown reports, logs, PNGs, and runtime-data files are immutable evidence. The 611-PNG ceiling applies only to unique non-control treatment renders, whose maximally distinct valid outcome is 596 PNGs. Four explicit-current checks, one deterministic reproduction, the 25 controls, and four primary oracle PNGs remain in the separate 630-PNG total evidence ledger. No blinded outcome is rejected or altered to manufacture reuse. The 22 repeated clean-source captures are separately recorded renderer-qualification evidence, not gallery slots.

The prior bevel rejection evidence is preserved offline at
[`provenance/prior-aesthetic-report/README.md`](provenance/prior-aesthetic-report/README.md)
with its hash-pinned [`manifest.json`](provenance/prior-aesthetic-report/manifest.json).
That older report is historical research, camera, and rejected-bevel evidence only; its pixels are not this study's baseline oracle.
"""


def scaffold_report(
    output_root: pathlib.Path,
    selection_raw: Optional[Any] = None,
    *,
    raw_capture_root: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Create resumable plan/provenance/gallery scaffolding, never conclusions."""

    output_root = _require_outside_source_tree(output_root, "output root")
    plan = build_capture_document(
        output_root,
        selection_raw,
        raw_capture_root=raw_capture_root,
    )
    output_root.mkdir(parents=True, exist_ok=True)
    prior_evidence = _materialize_prior_aesthetic_evidence(output_root, create=True)
    selection_path = output_root / "provenance" / "selection.json"
    plan_path = output_root / "capture-plan.json"
    template_path = output_root / "provenance" / "selection-template.json"
    gallery_data_path = output_root / "gallery" / "data.json"
    gallery_html_path = output_root / "gallery" / "index.html"
    readme = _incomplete_readme_content()
    manifest = {
        "version": 1,
        "warning": WARNING,
        "status": plan["status"],
        "capture_plan": "capture-plan.json",
        "capture_plan_sha256": sha256_bytes(pretty_json(plan).encode()),
        "provenance": plan["provenance"],
        "logical_slots_resolved": plan["study"]["slot_accounting"]["resolved_logical_slots"],
        "logical_slots_expected_complete": plan["study"]["slot_accounting"][
            "expected_complete_logical_slots"
        ],
        "motion_clips_resolved": plan["motion"]["resolved_clips"],
        "motion_clips_expected": plan["motion"]["expected_clips"],
        "render_accounting": {
            **plan["study"]["slot_accounting"],
            "planned_still_renderer_invocations": sum(
                len(plan["study"][field])
                for field in ("jobs", "verification_jobs", "reproduction_jobs")
            ),
            "planned_motion_renderer_invocations": len(plan["motion"]["jobs"]),
            "actual_renderer_invocations": None,
            "unique_still_content_hashes": None,
        },
        "render_accounting_notice": plan["study"]["slot_accounting"]["accounting_notice"],
        "recommendations": None,
        "rankings": None,
        "review_results": None,
        "prior_aesthetic_report": prior_evidence,
    }
    _write_if_absent_or_equal(template_path, pretty_json(selection_template()))
    if selection_path.exists():
        existing_selection = validate_selection(_read_json(selection_path, "scaffold selection"))
        if existing_selection != plan["selection"]:
            raise HarnessError("scaffold selection differs from the selected input")
    else:
        atomic_write(selection_path, pretty_json(plan["selection"]))
    _replace_generated_json(plan_path, plan)
    manifest_path = output_root / "manifest.json"
    if manifest_path.exists():
        existing_manifest = _read_json(manifest_path, "report manifest")
        if not isinstance(existing_manifest, dict):
            raise HarnessError("existing report manifest is not an object")
        if any(existing_manifest.get(field) is not None for field in ("recommendations", "rankings", "review_results")):
            raise HarnessError("refusing to refresh a manifest that already carries review conclusions")
    _replace_generated_json(manifest_path, manifest)
    _write_if_absent_or_equal(output_root / "README-INCOMPLETE.md", readme)
    gallery_data = _gallery_data(plan)
    _replace_generated_json(gallery_data_path, gallery_data)
    _replace_generated_gallery_html(gallery_html_path, _gallery_html(gallery_data))
    return {
        "version": 1,
        "warning": WARNING,
        "status": plan["status"],
        "output_root": str(output_root),
        "capture_plan": str(plan_path),
        "gallery": str(gallery_html_path),
        "prior_aesthetic_report": prior_evidence,
    }


def _validate_control_equivalence_for_plan(
    plan: Mapping[str, Any],
    records_by_job: Optional[Mapping[str, Mapping[str, Any]]] = None,
) -> Dict[str, Any]:
    """Prove fresh omitted-profile controls equal explicit-current renders."""

    qualification = _validate_control_equivalence_qualification_pack()
    if plan.get("provenance", {}).get("control_equivalence_raster") != (
        _control_equivalence_provenance_binding()
    ):
        raise HarnessError("capture plan control-equivalence qualification binding changed")
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    references = {
        (row["lighting"], row["camera_id"]): row
        for row in plan["study"]["shared_control_references"]
    }
    verification = plan["study"]["verification_jobs"]
    _, focused, _ = load_camera_sets()
    if len(verification) != len(focused):
        raise HarnessError(
            "control verification must contain four one-camera explicit-current jobs"
        )
    explicit_by_camera = {}
    for job, camera in zip(verification, focused):
        expected_id = f"control-verification-explicit-current-{camera.id}"
        camera_records = job.get("cameras")
        decoded_camera = (
            _camera_from_record(
                camera_records[0],
                f"explicit-current control camera {camera.id}",
            )
            if isinstance(camera_records, list) and len(camera_records) == 1
            else None
        )
        if (
            job.get("id") != expected_id
            or job.get("kind") != "control-verification"
            or job.get("stage") != "00-control-verification"
            or job.get("look_id") != "control-explicit-current"
            or job.get("profile_sha256") != CONTROL_PROFILE_SHA256
            or job.get("profile_json") != control_profile().canonical_json
            or job.get("control_profile_omitted") is not False
            or job.get("lighting") != "neutral"
            or job.get("asset_stage")
            != LIGHTING_CONDITIONS["neutral"].asset_stage
            or job.get("time_hours") != 12.0
            or job.get("liquid_phase_seconds") != 0.0
            or decoded_camera != camera
            or len(job.get("capture_plan", {}).get("captures", ())) != 1
            or len(job.get("artifacts", ())) != 1
        ):
            raise HarnessError(
                f"explicit-current fresh-process job changed for {camera.id}"
            )
        explicit_by_camera[camera.id] = job

    control_jobs = [
        job for job in plan["study"]["jobs"] if job["stage"] == "00-shared-control"
    ]
    if sum(len(job["artifacts"]) for job in control_jobs) != len(references):
        raise HarnessError("fresh shared-control jobs do not cover every control reference")
    if any(not job["control_profile_omitted"] for job in control_jobs):
        raise HarnessError("fresh shared controls must exercise the omitted-profile path")

    resolved_records = dict(records_by_job or {})
    for job in (*control_jobs, *verification):
        if job["id"] not in resolved_records:
            resolved_records[job["id"]] = _validate_job_artifacts(
                job,
                output_root,
                source_provenance_sha256=sha256_object(plan["provenance"]),
                label=False,
                require_labeled=True,
            )
    comparisons = []
    process_receipts = []
    stable_report_fields = (
        "profile_hash_sha256",
        "authority",
        "counts",
        "anchor_heights",
        "anchor_classes",
        "projection_hashes",
        "effect_validation",
        "camera_features",
        "cleanup",
    )
    _, oracle_manifest = _baseline_oracle_documents()
    for camera in focused:
        reference = references[("neutral", camera.id)]
        matches = [job for job in control_jobs if reference["artifact"] in job["artifacts"]]
        if len(matches) != 1:
            raise HarnessError(f"fresh neutral control owner is ambiguous at {camera.id}")
        omitted_job = matches[0]
        omitted_index = omitted_job["artifacts"].index(reference["artifact"])
        omitted = resolved_records[omitted_job["id"]]
        omitted_png = omitted["pngs"][omitted_index]
        explicit_job = explicit_by_camera[camera.id]
        explicit = resolved_records[explicit_job["id"]]
        if len(explicit.get("pngs", ())) != 1 or len(explicit.get("reports", ())) != 1:
            raise HarnessError(
                f"explicit-current job lacks one validated pair at {camera.id}"
            )
        explicit_png = explicit["pngs"][0]
        pixel_equivalence = _compare_raster_stable_rgb(
            pathlib.Path(omitted_png["path"]),
            pathlib.Path(explicit_png["path"]),
            camera_id=camera.id,
            ambiguous_pixels=_control_equivalence_ambiguous_pixels(
                camera.id,
                oracle_manifest["cameras"][camera.id]["ambiguous_pixels"],
                qualification_validation=qualification,
            ),
            context="omitted versus explicit-current control",
        )
        omitted_report = omitted["reports"][omitted_index]
        explicit_report = explicit["reports"][0]
        if any(omitted_report[field] != explicit_report[field] for field in stable_report_fields):
            raise HarnessError(f"omitted and explicit-current report state differs at {camera.id}")
        omitted_receipt = _validated_control_record_receipt(omitted_job, omitted)
        explicit_receipt = _validated_control_record_receipt(explicit_job, explicit)
        process_receipts.extend((omitted_receipt, explicit_receipt))
        comparisons.append(
            {
                "camera_id": camera.id,
                "omitted_decoded_rgb_sha256": pixel_equivalence[
                    "reference_decoded_rgb_sha256"
                ],
                "explicit_decoded_rgb_sha256": pixel_equivalence[
                    "candidate_decoded_rgb_sha256"
                ],
                "decoded_rgb_identical": pixel_equivalence["decoded_rgb_identical"],
                "stable_pixel_identical": pixel_equivalence["stable_pixel_identical"],
                "stable_pixel_count": pixel_equivalence["stable_pixel_count"],
                "ambiguous_pixel_count": pixel_equivalence["ambiguous_pixel_count"],
                "differing_ambiguous_pixel_count": pixel_equivalence[
                    "differing_ambiguous_pixel_count"
                ],
                "ambiguous_values": pixel_equivalence["ambiguous_values"],
                "omitted_png_sha256": omitted_png["sha256"],
                "explicit_png_sha256": explicit_png["sha256"],
                "omitted_report_sha256": omitted_report["sha256"],
                "explicit_report_sha256": explicit_report["sha256"],
                "omitted_job_id": omitted_job["id"],
                "explicit_job_id": explicit_job["id"],
                "omitted_runtime_receipt_sha256": omitted_receipt["receipt_sha256"],
                "explicit_runtime_receipt_sha256": explicit_receipt["receipt_sha256"],
            }
        )
    if (
        len({receipt["launch_nonce"] for receipt in process_receipts})
        != 2 * len(focused)
        or len({receipt["receipt_sha256"] for receipt in process_receipts})
        != 2 * len(focused)
        or len({receipt["source_provenance_sha256"] for receipt in process_receipts}) != 1
        or len({receipt["executable_sha256"] for receipt in process_receipts}) != 1
    ):
        raise HarnessError(
            "control equivalence does not prove eight fresh launches of one source/executable"
        )
    return {
        "version": 1,
        "warning": WARNING,
        "fresh_current_source_controls": True,
        "fresh_process_per_png": True,
        "fresh_process_count": len(process_receipts),
        "raster_contract": {
            "scope": qualification["scope"],
            "contract_sha256": qualification["contract_sha256"],
            "manifest_sha256": qualification["manifest_sha256"],
            "inventory_manifest_sha256": qualification[
                "inventory_manifest_sha256"
            ],
            "qualification_evidence_sha256": qualification[
                "qualification_evidence_sha256"
            ],
            "qualified_pixels": qualification["qualified_pixels"],
            "baseline_oracle_contract_unchanged": True,
        },
        "comparisons": comparisons,
        "raster_stable_pixel_identical": True,
        "raw_decoded_rgb_identical": all(
            comparison["decoded_rgb_identical"] for comparison in comparisons
        ),
        "broad_numeric_threshold_used": False,
    }


def validate_control_equivalence(plan_path: pathlib.Path) -> Dict[str, Any]:
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    return _validate_control_equivalence_for_plan(plan)


LIFECYCLE_CYCLE_FIELDS = (
    "cycle_index",
    "launch_nonce",
    "runtime_receipt_sha256",
    "profile_hash_sha256",
    "authority_before_sha256",
    "authority_after_sha256",
    "entities_remaining",
    "materials_remaining",
    "meshes_remaining",
    "fog_density_images_remaining",
    "target_images_remaining",
    "terrain_material_overrides_remaining",
    "liquid_visibility_overrides_remaining",
    "vegetation_scale_overrides_remaining",
    "camera_state_restored",
    "oit_state_restored",
    "transmission_state_restored",
    "depth_state_restored",
    "volumetric_state_restored",
    "previous_cycle_sha256",
)


def lifecycle_launch_evidence_path(certificate_path: pathlib.Path) -> pathlib.Path:
    return certificate_path.with_name(certificate_path.stem + ".launch.json")


def _lifecycle_profile(plan: Mapping[str, Any]) -> DetailProfile:
    """Resolve the exact profile exercised by the genuine teardown loop."""

    progress = plan["study"]["adaptive_progress"]
    if all(progress.values()):
        return _combined_profile(
            "combination-score-leader",
            "Score Leader",
            plan["selection"]["combinations"]["score-leader"],
        )
    return control_profile()


def _lifecycle_profile_hash(plan: Mapping[str, Any]) -> str:
    return _lifecycle_profile(plan).sha256


def _lifecycle_request(
    plan_path: pathlib.Path,
    plan: Mapping[str, Any],
    certificate_path: pathlib.Path,
) -> Dict[str, Any]:
    """Build the runtime's canonical, ordered lifecycle request object."""

    return {
        "version": 1,
        "certificate_path": str(certificate_path),
        "capture_plan_sha256": sha256_file(plan_path),
        "source_provenance_sha256": sha256_object(plan["provenance"]),
        "profile_matrix_sha256": sha256_object(plan["study"]["profile_matrix"]),
        "tested_profile_sha256": _lifecycle_profile_hash(plan),
        "cycles_requested": 100,
    }


def run_lifecycle(
    plan_path: pathlib.Path,
    certificate_path: pathlib.Path,
    *,
    work_root: pathlib.Path,
    timeout_seconds: int = 14_400,
) -> Dict[str, Any]:
    """Launch one genuine runtime process that performs 100 enter/exit cycles.

    Python supplies only a hash-linked request. The Rust runtime owns every cycle,
    teardown receipt, hash-chain entry, and the atomic certificate write.
    """

    _validate_baseline_oracle_pack()
    _validate_control_equivalence_qualification_pack()
    if timeout_seconds <= 0:
        raise HarnessError("lifecycle timeout-seconds must be positive")
    if not certificate_path.is_absolute():
        raise HarnessError("lifecycle certificate path must be absolute")
    certificate_path = _require_outside_source_tree(
        certificate_path,
        "lifecycle certificate",
    )
    if certificate_path.suffix != ".json":
        raise HarnessError("lifecycle certificate path must end in .json")
    if certificate_path.is_symlink():
        raise HarnessError("lifecycle certificate may not be a symlink")
    certificate_path.parent.mkdir(parents=True, exist_ok=True)

    plan_path = plan_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    _validate_raw_capture_root(output_root, pathlib.Path(plan["raw_capture_root"]))
    work_root = _require_outside_source_tree(work_root, "lifecycle work root")

    launch_evidence_path = lifecycle_launch_evidence_path(certificate_path)
    if certificate_path.exists() or launch_evidence_path.exists():
        raise HarnessError(
            "run-lifecycle requires a fresh certificate destination; preexisting proof is forbidden"
        )

    profile = _lifecycle_profile(plan)
    _, focused_cameras, _ = load_camera_sets()
    camera = focused_cameras[0]
    if camera.full_cutaway or camera.illumination_overlay:
        raise HarnessError("lifecycle camera may not use cutaway or illumination overlay")
    condition = LIGHTING_CONDITIONS["neutral"]
    stage_root, stage_manifest = _stage_asset_root(work_root, condition)

    plan_identity = sha256_file(plan_path)[:20]
    attempt_parent = work_root / "lifecycle-runtime" / plan_identity
    attempt_number = 1
    while (attempt_parent / f"attempt-{attempt_number:03d}").exists():
        attempt_number += 1
    attempt_root = attempt_parent / f"attempt-{attempt_number:03d}"
    data_root = attempt_root / "game-data"
    data_root.mkdir(parents=True, exist_ok=False)
    raw_png = (attempt_root / "cycle-capture.png").resolve()
    runtime_capture_plan = {
        "version": 1,
        "captures": [camera.runtime_entry(raw_png)],
    }
    request = _lifecycle_request(plan_path, plan, certificate_path)
    request_json = compact_json(request)
    capture_plan_json = compact_json(runtime_capture_plan)
    launch_nonce = secrets.token_hex(32)
    source_provenance_sha256 = sha256_object(plan["provenance"])
    atomic_write(attempt_root / "lifecycle-request.json", request_json + "\n")
    atomic_write(attempt_root / "runtime-capture-plan.json", capture_plan_json + "\n")
    log_path = attempt_root / "runtime.log"

    environment = _map_review_environment()
    environment.update(
        {
            "BEVY_ASSET_ROOT": str(stage_root),
            "HEX_GAME_DATA_DIR": str(data_root),
            "HEX_REVIEW_SCENARIO": SCENARIO,
            "HEX_REVIEW_SEED": str(SEED),
            "HEX_REVIEW_TIME": str(condition.time_hours),
            "HEX_REVIEW_LIQUID_PHASE": "0.0",
            "HEX_REVIEW_CAPTURE_PLAN": capture_plan_json,
            "HEX_REVIEW_LIFECYCLE": request_json,
            "HEX_REVIEW_LAUNCH_NONCE": launch_nonce,
            "HEX_REVIEW_SOURCE_PROVENANCE_SHA256": source_provenance_sha256,
        }
    )
    if not profile.is_control:
        environment["HEX_REVIEW_WORLD_DETAIL"] = profile.canonical_json
    command = (
        "cargo",
        "run",
        "--locked",
        "--release",
        "-p",
        "hex_game",
        "--features",
        "map-review",
    )
    _assert_source_provenance(plan["provenance"])
    returncode = _legacy_harness().run_logged_process(
        command,
        cwd=REPOSITORY_ROOT,
        environment=environment,
        log_path=log_path,
        timeout_seconds=timeout_seconds,
    )
    _assert_source_provenance(plan["provenance"])
    if returncode != 0:
        tail = (
            log_path.read_text(encoding="utf-8", errors="replace")[-6000:]
            if log_path.is_file()
            else ""
        )
        raise HarnessError(
            "genuine lifecycle runtime failed; no Python certificate was generated "
            f"(exit {returncode}):\n{tail}"
        )
    if not certificate_path.is_file() or certificate_path.is_symlink():
        raise HarnessError("runtime exited successfully without an atomic lifecycle certificate")
    if not raw_png.is_file() or not runtime_report_path(raw_png).is_file():
        raise HarnessError("lifecycle runtime omitted its genuine final-cycle capture or sidecar")

    capture = inspect_png(raw_png)
    final_cycle_report_path = runtime_report_path(raw_png)
    report = validate_runtime_report(
        final_cycle_report_path,
        png_path=raw_png,
        camera=camera,
        profile_json=profile.canonical_json,
        time_hours=condition.time_hours,
        liquid_phase_seconds=0.0,
        settle_frames=90,
        source_provenance_sha256=source_provenance_sha256,
        capture_plan_json=capture_plan_json,
        expected_launch_nonce=launch_nonce,
        expected_completed_cycles=request["cycles_requested"],
    )
    validation = validate_lifecycle_certificate(
        plan_path,
        certificate_path,
        expected_launch_nonce=launch_nonce,
        runtime_capture_plan_json=capture_plan_json,
        require_launch_evidence=False,
    )
    if report["cleanup"]["completed_cycles"] != validation["cycles_completed"]:
        raise HarnessError(
            "final lifecycle sidecar teardown count differs from its certificate"
        )
    launch_evidence = {
        "version": 1,
        "warning": WARNING,
        "launch_nonce": launch_nonce,
        "process_id": report["runtime_receipt"]["process_id"],
        "executable_sha256": report["runtime_receipt"]["executable_sha256"],
        "runtime_receipt_sha256": report["runtime_receipt"]["receipt_sha256"],
        "source_provenance_sha256": source_provenance_sha256,
        "outer_capture_plan_sha256": sha256_file(plan_path),
        "runtime_capture_plan_sha256": sha256_bytes(capture_plan_json.encode("utf-8")),
        "lifecycle_request_sha256": sha256_bytes(request_json.encode("utf-8")),
        "certificate_sha256": sha256_file(certificate_path),
        "final_cycle_capture_path": str(raw_png),
        "final_cycle_capture_sha256": sha256_file(raw_png),
        "final_cycle_report_path": str(final_cycle_report_path),
        "final_cycle_report_sha256": sha256_file(final_cycle_report_path),
        "log_path": str(log_path),
        "log_sha256": sha256_file(log_path),
        "structural_draft_environment": STRUCTURAL_DRAFT_ENVIRONMENT,
        "structural_draft_value": STRUCTURAL_DRAFT_VALUE,
    }
    atomic_write(launch_evidence_path, pretty_json(launch_evidence))
    validation = validate_lifecycle_certificate(plan_path, certificate_path)
    return {
        "version": 1,
        "warning": WARNING,
        "status": "COMPLETED_AND_VALIDATED",
        "renderer_invocations": 1,
        "command": list(command),
        "attempt": attempt_number,
        "attempt_root": str(attempt_root),
        "log": str(log_path),
        "log_sha256": sha256_file(log_path),
        "asset_stage_manifest_sha256": sha256_object(stage_manifest),
        "request": request,
        "request_canonical_json": request_json,
        "request_sha256": sha256_bytes(request_json.encode("utf-8")),
        "runtime_capture_plan_sha256": sha256_bytes(capture_plan_json.encode("utf-8")),
        "final_cycle_capture": capture,
        "final_cycle_report": report,
        "certificate": validation,
        "launch_evidence": launch_evidence,
        "launch_evidence_path": str(launch_evidence_path),
    }


def _validate_lifecycle_launch_evidence(
    launch_path: pathlib.Path,
    *,
    certificate_path: pathlib.Path,
    plan_path: pathlib.Path,
    runtime_receipt: Mapping[str, Any],
) -> Dict[str, Any]:
    """Bind a runtime certificate to the runner-observed fresh launch and log."""

    if launch_path.is_symlink():
        raise HarnessError("lifecycle launch evidence may not be a symlink")
    launch_path = launch_path.resolve()
    launch = _exact_keys(
        _read_json(launch_path, "lifecycle launch evidence"),
        (
            "version",
            "warning",
            "launch_nonce",
            "process_id",
            "executable_sha256",
            "runtime_receipt_sha256",
            "source_provenance_sha256",
            "outer_capture_plan_sha256",
            "runtime_capture_plan_sha256",
            "lifecycle_request_sha256",
            "certificate_sha256",
            "final_cycle_capture_path",
            "final_cycle_capture_sha256",
            "final_cycle_report_path",
            "final_cycle_report_sha256",
            "log_path",
            "log_sha256",
            "structural_draft_environment",
            "structural_draft_value",
        ),
        "lifecycle launch evidence",
    )
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    request = _lifecycle_request(plan_path, plan, certificate_path)
    expected = {
        "version": 1,
        "warning": WARNING,
        "launch_nonce": runtime_receipt["launch_nonce"],
        "process_id": runtime_receipt["process_id"],
        "executable_sha256": runtime_receipt["executable_sha256"],
        "runtime_receipt_sha256": runtime_receipt["receipt_sha256"],
        "source_provenance_sha256": sha256_object(plan["provenance"]),
        "outer_capture_plan_sha256": sha256_file(plan_path),
        "runtime_capture_plan_sha256": runtime_receipt["capture_plan_sha256"],
        "lifecycle_request_sha256": sha256_bytes(
            compact_json(request).encode("utf-8")
        ),
        "certificate_sha256": sha256_file(certificate_path),
        "structural_draft_environment": STRUCTURAL_DRAFT_ENVIRONMENT,
        "structural_draft_value": STRUCTURAL_DRAFT_VALUE,
    }
    for field, value in expected.items():
        if launch[field] != value:
            raise HarnessError(f"lifecycle launch evidence {field} changed")
    final_paths = {}
    for field in ("final_cycle_capture_path", "final_cycle_report_path"):
        raw_path = launch[field]
        if not isinstance(raw_path, str) or not raw_path:
            raise HarnessError(f"lifecycle launch evidence {field} is invalid")
        path = pathlib.Path(raw_path)
        if not path.is_absolute() or not path.is_file() or path.is_symlink():
            raise HarnessError(f"lifecycle launch evidence {field} is unavailable")
        final_paths[field] = path
    final_capture_path = final_paths["final_cycle_capture_path"]
    final_report_path = final_paths["final_cycle_report_path"]
    if runtime_report_path(final_capture_path) != final_report_path:
        raise HarnessError("lifecycle final-cycle report is not the capture's sidecar")
    for path, field in (
        (final_capture_path, "final_cycle_capture_sha256"),
        (final_report_path, "final_cycle_report_sha256"),
    ):
        digest = launch[field]
        if not isinstance(digest, str) or SHA256_RE.fullmatch(digest) is None:
            raise HarnessError(f"lifecycle launch evidence {field} is not SHA-256")
        if digest != sha256_file(path):
            raise HarnessError(f"lifecycle launch evidence {field} changed")
    final_wrapper = _exact_keys(
        _read_json(final_report_path, "lifecycle final-cycle runtime report"),
        ("version", "warning", "capture", "report"),
        "lifecycle final-cycle runtime wrapper",
    )
    if final_wrapper["version"] != 1 or final_wrapper["warning"] != WARNING:
        raise HarnessError("lifecycle final-cycle runtime wrapper identity changed")
    final_capture = final_wrapper["capture"]
    final_capture_name = (
        final_capture.get("path") if isinstance(final_capture, dict) else None
    )
    if (
        not isinstance(final_capture_name, str)
        or pathlib.Path(final_capture_name).resolve() != final_capture_path.resolve()
    ):
        raise HarnessError("lifecycle final-cycle sidecar names a different capture")
    final_report = final_wrapper["report"]
    if not isinstance(final_report, dict) or final_report.get("runtime_receipt") != runtime_receipt:
        raise HarnessError("lifecycle final-cycle sidecar runtime receipt changed")
    final_cleanup = _exact_keys(
        final_report.get("cleanup"),
        (
            "completed_cycles",
            "entities_remaining",
            "materials_remaining",
            "meshes_remaining",
            "target_images_remaining",
            "camera_state_restored",
            "oit_state_restored",
            "transmission_state_restored",
            "depth_state_restored",
            "volumetric_state_restored",
        ),
        "lifecycle final-cycle cleanup",
    )
    if final_cleanup["completed_cycles"] != request["cycles_requested"]:
        raise HarnessError(
            "lifecycle final-cycle sidecar does not represent all requested teardown cycles"
        )
    log_path = pathlib.Path(launch["log_path"])
    if not log_path.is_absolute() or not log_path.is_file() or log_path.is_symlink():
        raise HarnessError("lifecycle launch evidence log is unavailable")
    if launch["log_sha256"] != sha256_file(log_path):
        raise HarnessError("lifecycle launch evidence log changed")
    return launch


def validate_lifecycle_certificate(
    plan_path: pathlib.Path,
    certificate_path: pathlib.Path,
    *,
    expected_launch_nonce: Optional[str] = None,
    runtime_capture_plan_json: Optional[str] = None,
    require_launch_evidence: bool = True,
) -> Dict[str, Any]:
    """Validate a post-teardown, hash-chained 100-cycle runtime certificate."""

    if certificate_path.is_symlink():
        raise HarnessError("lifecycle certificate may not be a symlink")
    plan_path = plan_path.resolve()
    certificate_path = certificate_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    certificate = _exact_keys(
        _read_json(certificate_path, "lifecycle certificate"),
        (
            "version",
            "warning",
            "runtime_receipt",
            "capture_plan_sha256",
            "source_provenance_sha256",
            "profile_matrix_sha256",
            "tested_profile_sha256",
            "cycles_requested",
            "cycles_completed",
            "cycles",
            "final_chain_sha256",
        ),
        "lifecycle certificate",
    )
    expected_links = {
        "capture_plan_sha256": sha256_file(plan_path),
        "source_provenance_sha256": sha256_object(plan["provenance"]),
        "profile_matrix_sha256": sha256_object(plan["study"]["profile_matrix"]),
        "tested_profile_sha256": _lifecycle_profile_hash(plan),
    }
    if certificate["version"] != 1 or certificate["warning"] != WARNING:
        raise HarnessError("lifecycle certificate identity changed")
    for field, expected in expected_links.items():
        if certificate[field] != expected:
            raise HarnessError(f"lifecycle certificate {field} does not bind the capture plan")
    if certificate["cycles_requested"] != 100 or certificate["cycles_completed"] != 100:
        raise HarnessError("lifecycle certificate must request and complete exactly 100 cycles")
    receipt_capture_json = runtime_capture_plan_json
    if receipt_capture_json is None:
        # Generic validation can still authenticate the ordered runtime receipt,
        # while the runner-led launch evidence below binds the exact plan bytes.
        receipt_capture_json = ""
        receipt_capture_sha256 = certificate["runtime_receipt"].get("capture_plan_sha256")
        if not isinstance(receipt_capture_sha256, str):
            raise HarnessError("lifecycle runtime receipt lacks capture-plan identity")
    else:
        receipt_capture_sha256 = sha256_bytes(receipt_capture_json.encode("utf-8"))
    runtime_receipt = _validate_runtime_receipt(
        certificate["runtime_receipt"],
        source_provenance_sha256=expected_links["source_provenance_sha256"],
        capture_plan_json=receipt_capture_json,
        profile_sha256=expected_links["tested_profile_sha256"],
        expected_launch_nonce=expected_launch_nonce,
    ) if runtime_capture_plan_json is not None else _validate_runtime_receipt_hash_only(
        certificate["runtime_receipt"],
        source_provenance_sha256=expected_links["source_provenance_sha256"],
        profile_sha256=expected_links["tested_profile_sha256"],
        expected_launch_nonce=expected_launch_nonce,
    )
    cycles = certificate["cycles"]
    if not isinstance(cycles, list) or len(cycles) != 100:
        raise HarnessError("lifecycle certificate must contain exactly 100 cycle records")
    previous = "0" * 64
    for index, raw_cycle in enumerate(cycles, start=1):
        cycle = _exact_keys(
            raw_cycle,
            (*LIFECYCLE_CYCLE_FIELDS, "cycle_sha256"),
            f"lifecycle cycle {index}",
        )
        if cycle["cycle_index"] != index:
            raise HarnessError("lifecycle cycle indices must be exactly 1 through 100")
        if cycle["launch_nonce"] != runtime_receipt["launch_nonce"]:
            raise HarnessError(f"lifecycle cycle {index} launch nonce changed")
        if cycle["runtime_receipt_sha256"] != runtime_receipt["receipt_sha256"]:
            raise HarnessError(f"lifecycle cycle {index} runtime receipt changed")
        if cycle["profile_hash_sha256"] != expected_links["tested_profile_sha256"]:
            raise HarnessError(f"lifecycle cycle {index} tested a different profile")
        for field in ("authority_before_sha256", "authority_after_sha256"):
            if not isinstance(cycle[field], str) or not SHA256_RE.fullmatch(cycle[field]):
                raise HarnessError(f"lifecycle cycle {index} has invalid {field}")
        if cycle["authority_before_sha256"] != cycle["authority_after_sha256"]:
            raise HarnessError(f"lifecycle cycle {index} changed authoritative world state")
        for field in (
            "entities_remaining",
            "materials_remaining",
            "meshes_remaining",
            "fog_density_images_remaining",
            "target_images_remaining",
            "terrain_material_overrides_remaining",
            "liquid_visibility_overrides_remaining",
            "vegetation_scale_overrides_remaining",
        ):
            if isinstance(cycle[field], bool) or not isinstance(cycle[field], int) or cycle[field] != 0:
                raise HarnessError(f"lifecycle cycle {index} leaked {field}")
        for field in (
            "camera_state_restored",
            "oit_state_restored",
            "transmission_state_restored",
            "depth_state_restored",
            "volumetric_state_restored",
        ):
            if cycle[field] is not True:
                raise HarnessError(f"lifecycle cycle {index} did not restore {field}")
        if cycle["previous_cycle_sha256"] != previous:
            raise HarnessError(f"lifecycle hash chain broke before cycle {index}")
        hash_body = {field: cycle[field] for field in LIFECYCLE_CYCLE_FIELDS}
        expected_cycle_hash = sha256_bytes(compact_json(hash_body).encode("utf-8"))
        if cycle["cycle_sha256"] != expected_cycle_hash:
            raise HarnessError(f"lifecycle cycle {index} hash changed")
        previous = expected_cycle_hash
    if certificate["final_chain_sha256"] != previous:
        raise HarnessError("lifecycle final chain hash changed")
    launch_evidence = None
    launch_path = lifecycle_launch_evidence_path(certificate_path)
    if require_launch_evidence:
        launch_evidence = _validate_lifecycle_launch_evidence(
            launch_path,
            certificate_path=certificate_path,
            plan_path=plan_path,
            runtime_receipt=runtime_receipt,
        )
    return {
        "version": 1,
        "warning": WARNING,
        "certificate_path": str(certificate_path),
        "certificate_sha256": sha256_file(certificate_path),
        "cycles_completed": 100,
        "tested_profile_sha256": expected_links["tested_profile_sha256"],
        "final_chain_sha256": previous,
        "runtime_receipt": runtime_receipt,
        "launch_evidence_path": str(launch_path) if launch_evidence is not None else None,
        "launch_evidence_sha256": sha256_file(launch_path) if launch_evidence is not None else None,
    }


def validate_capture_state(
    plan_path: pathlib.Path,
    state_path: pathlib.Path,
    *,
    include_motion: bool = True,
) -> Dict[str, Any]:
    """Validate the resumable runner ledger against every current job and artifact."""

    if state_path.is_symlink():
        raise HarnessError("capture state may not be a symlink")
    plan_path = plan_path.resolve()
    state_path = state_path.resolve()
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    state = _exact_keys(
        _read_json(state_path, "capture state"),
        (
            "version",
            "warning",
            "plan_sha256_history",
            "pinned_executable_sha256",
            "baseline_oracle_equivalence",
            "completed",
            "attempts",
        ),
        "capture state",
    )
    if state["version"] != 1 or state["warning"] != WARNING:
        raise HarnessError("capture state identity changed")
    history = state["plan_sha256_history"]
    if (
        not isinstance(history, list)
        or not history
        or len(history) != len(set(history))
        or any(not isinstance(item, str) or not SHA256_RE.fullmatch(item) for item in history)
        or sha256_file(plan_path) not in history
    ):
        raise HarnessError("capture state does not contain a unique hash history including this plan")
    pinned_executable_sha256 = state["pinned_executable_sha256"]
    if (
        not isinstance(pinned_executable_sha256, str)
        or SHA256_RE.fullmatch(pinned_executable_sha256) is None
    ):
        raise HarnessError("capture state lacks one valid pinned executable SHA-256")
    baseline_oracle_equivalence = state["baseline_oracle_equivalence"]
    if not isinstance(baseline_oracle_equivalence, dict):
        raise HarnessError("capture state lacks baseline-oracle equivalence evidence")
    oracle_body = {
        key: value
        for key, value in baseline_oracle_equivalence.items()
        if key != "evidence_sha256"
    }
    if baseline_oracle_equivalence.get("evidence_sha256") != sha256_object(oracle_body):
        raise HarnessError("capture state baseline-oracle evidence hash changed")
    jobs = [
        *plan["study"]["jobs"],
        *plan["study"]["verification_jobs"],
        *plan["study"]["reproduction_jobs"],
    ]
    if include_motion:
        jobs.extend(plan["motion"]["jobs"])
    jobs_by_id = {job["id"]: job for job in jobs}
    if len(jobs_by_id) != len(jobs):
        raise HarnessError("capture plan repeats a job id")
    completed = _strict_object(
        state["completed"],
        context="capture state completed",
        required=tuple(sorted(jobs_by_id)),
    )
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    completed_summary = {}
    validated_records_by_job: Dict[str, Mapping[str, Any]] = {}
    expected_source_provenance_sha256 = sha256_object(plan["provenance"])
    final_cameras, _, _ = load_camera_sets()
    for job_id, job in sorted(jobs_by_id.items()):
        row = _exact_keys(
            completed[job_id],
            (
                "job_sha256",
                "artifact_sha256",
                "report_sha256",
                "log",
                "launch_nonce",
                "runtime_receipt_sha256",
                "process_id",
                "executable_sha256",
                "asset_stage",
                "asset_stage_manifest_sha256",
                "asset_stage_tree_sha256",
                "source_asset_tree_sha256",
            ),
            f"capture state completed.{job_id}",
        )
        job_sha = sha256_object(job)
        if row["job_sha256"] != job_sha:
            raise HarnessError(f"capture state job hash changed for {job_id}")
        expected_asset_stage = LIGHTING_CONDITIONS[job["lighting"]].asset_stage
        if row["asset_stage"] != expected_asset_stage or any(
            not isinstance(row[field], str) or SHA256_RE.fullmatch(row[field]) is None
            for field in (
                "asset_stage_manifest_sha256",
                "asset_stage_tree_sha256",
                "source_asset_tree_sha256",
            )
        ):
            raise HarnessError(f"capture state asset-stage binding is malformed for {job_id}")
        stage_manifest_path = (
            state_path.parent
            / "asset-stages"
            / expected_asset_stage
            / "stage-manifest.json"
        )
        if (
            not stage_manifest_path.is_file()
            or stage_manifest_path.is_symlink()
            or sha256_file(stage_manifest_path) != row["asset_stage_manifest_sha256"]
        ):
            raise HarnessError(f"capture state asset-stage manifest changed for {job_id}")
        stage_manifest = _exact_keys(
            _read_json(stage_manifest_path, "capture state asset-stage manifest"),
            (
                "version",
                "warning",
                "asset_stage",
                "lighting_condition",
                "source_asset_tree_sha256",
                "staged_asset_tree_sha256",
                "modified_assets",
            ),
            "capture state asset-stage manifest",
        )
        if (
            stage_manifest["version"] != 1
            or stage_manifest["warning"] != WARNING
            or stage_manifest["asset_stage"] != expected_asset_stage
            or stage_manifest["lighting_condition"]
            != dataclasses.asdict(LIGHTING_CONDITIONS[job["lighting"]])
            or not isinstance(stage_manifest["modified_assets"], list)
            or stage_manifest["staged_asset_tree_sha256"]
            != row["asset_stage_tree_sha256"]
            or stage_manifest["source_asset_tree_sha256"]
            != row["source_asset_tree_sha256"]
        ):
            raise HarnessError(f"capture state asset-stage content changed for {job_id}")
        captures = job["capture_plan"]["captures"]
        camera_by_path = _job_cameras(job, final_cameras)
        for field in ("artifact_sha256", "report_sha256"):
            if (
                not isinstance(row[field], list)
                or len(row[field]) != len(captures)
                or any(not isinstance(item, str) or not SHA256_RE.fullmatch(item) for item in row[field])
            ):
                raise HarnessError(f"capture state {field} is malformed for {job_id}")
        actual_png = []
        actual_report = []
        runtime_receipts = []
        capture_plan_json = compact_json(job["capture_plan"])
        for capture in captures:
            path = pathlib.Path(capture["path"]).resolve()
            try:
                path.relative_to(raw_capture_root)
            except ValueError as error:
                raise HarnessError(
                    f"capture state artifact escaped external raw capture root: {path}"
                ) from error
            actual_png.append(sha256_file(path))
            report_path = runtime_report_path(path)
            actual_report.append(sha256_file(report_path))
            camera, liquid_phase_seconds, settle_frames = camera_by_path[path]
            report_record = validate_runtime_report(
                report_path,
                png_path=path,
                camera=camera,
                profile_json=job["profile_json"],
                time_hours=job["time_hours"],
                liquid_phase_seconds=liquid_phase_seconds,
                settle_frames=settle_frames,
                source_provenance_sha256=expected_source_provenance_sha256,
                capture_plan_json=capture_plan_json,
                expected_launch_nonce=row["launch_nonce"],
            )
            runtime_receipts.append(report_record["runtime_receipt"])
        if row["artifact_sha256"] != actual_png or row["report_sha256"] != actual_report:
            raise HarnessError(f"capture state artifact/report hashes changed for {job_id}")
        if not runtime_receipts or any(receipt != runtime_receipts[0] for receipt in runtime_receipts):
            raise HarnessError(f"capture state runtime receipts vary for {job_id}")
        receipt = runtime_receipts[0]
        if (
            row["runtime_receipt_sha256"] != receipt["receipt_sha256"]
            or row["process_id"] != receipt["process_id"]
            or row["executable_sha256"] != receipt["executable_sha256"]
            or row["executable_sha256"] != pinned_executable_sha256
        ):
            raise HarnessError(f"capture state process receipt changed for {job_id}")
        validated_records_by_job[job_id] = {
            "pngs": [
                {"path": capture["path"], "sha256": digest}
                for capture, digest in zip(captures, actual_png)
            ],
            "runtime_receipt": receipt,
        }
        if not isinstance(row["log"], str) or not row["log"]:
            raise HarnessError(f"capture state log is invalid for {job_id}")
        log_path = pathlib.Path(row["log"])
        if not log_path.is_file() or log_path.is_symlink():
            raise HarnessError(f"capture state log is unavailable for {job_id}: {log_path}")
        completed_summary[job_id] = {
            "job_sha256": job_sha,
            "log": str(log_path),
            "log_sha256": sha256_file(log_path),
            "artifacts": len(captures),
            "launch_nonce": row["launch_nonce"],
            "runtime_receipt_sha256": row["runtime_receipt_sha256"],
            "process_id": row["process_id"],
            "executable_sha256": row["executable_sha256"],
            "asset_stage": row["asset_stage"],
            "asset_stage_manifest_sha256": row["asset_stage_manifest_sha256"],
            "asset_stage_tree_sha256": row["asset_stage_tree_sha256"],
            "source_asset_tree_sha256": row["source_asset_tree_sha256"],
        }

    regenerated_oracle_evidence = _validate_baseline_oracle_equivalence(
        plan["study"]["jobs"],
        validated_records_by_job,
    )
    if regenerated_oracle_evidence is None:
        raise HarnessError("capture state lacks all four baseline-oracle controls")
    if regenerated_oracle_evidence != baseline_oracle_equivalence:
        raise HarnessError("capture state baseline-oracle evidence does not bind current controls")

    attempts = state["attempts"]
    if not isinstance(attempts, list):
        raise HarnessError("capture state attempts must be a list")
    complete_attempts = defaultdict(int)
    attempt_numbers = set()
    normalized_attempts = []
    expected_command = [
        "cargo",
        "run",
        "--locked",
        "--release",
        "-p",
        "hex_game",
        "--features",
        "map-review",
    ]
    for index, raw_attempt in enumerate(attempts):
        if not isinstance(raw_attempt, dict):
            raise HarnessError(f"capture state attempt {index} is not an object")
        status = raw_attempt.get("status")
        fields = (
            "job_id",
            "job_sha256",
            "attempt_number",
            "status",
            "command",
            "log",
            "launch_nonce",
            "source_provenance_sha256",
            "capture_plan_sha256",
            "structural_draft_environment",
            "structural_draft_value",
            "asset_stage",
            "asset_stage_manifest_sha256",
            "asset_stage_tree_sha256",
            "source_asset_tree_sha256",
            "returncode",
        )
        if status == "COMPLETE":
            fields = (
                *fields,
                "artifact_sha256",
                "runtime_receipt_sha256",
                "process_id",
                "executable_sha256",
            )
        elif status != "FAILED":
            raise HarnessError(f"capture state attempt {index} is not terminal")
        attempt = _strict_object(
            raw_attempt,
            context=f"capture state attempt {index}",
            required=fields,
            optional=("failure_phase", "failure_type") if status == "FAILED" else (),
        )
        job_id = attempt["job_id"]
        if job_id not in jobs_by_id:
            raise HarnessError(f"capture state attempt {index} names an unknown current job")
        if attempt["job_sha256"] != sha256_object(jobs_by_id[job_id]):
            raise HarnessError(f"capture state attempt {index} job hash changed")
        number = attempt["attempt_number"]
        if isinstance(number, bool) or not isinstance(number, int) or number <= 0:
            raise HarnessError(f"capture state attempt {index} number is invalid")
        identity = (job_id, number)
        if identity in attempt_numbers:
            raise HarnessError(f"capture state repeats attempt {identity}")
        attempt_numbers.add(identity)
        if attempt["command"] != expected_command:
            raise HarnessError(f"capture state attempt {index} command changed")
        if (
            not isinstance(attempt["launch_nonce"], str)
            or LAUNCH_NONCE_RE.fullmatch(attempt["launch_nonce"]) is None
            or attempt["source_provenance_sha256"] != expected_source_provenance_sha256
            or attempt["capture_plan_sha256"]
            != sha256_bytes(compact_json(jobs_by_id[job_id]["capture_plan"]).encode("utf-8"))
            or attempt["structural_draft_environment"] != STRUCTURAL_DRAFT_ENVIRONMENT
            or attempt["structural_draft_value"] != STRUCTURAL_DRAFT_VALUE
        ):
            raise HarnessError(f"capture state attempt {index} launch binding changed")
        expected_asset_stage = LIGHTING_CONDITIONS[jobs_by_id[job_id]["lighting"]].asset_stage
        if attempt["asset_stage"] != expected_asset_stage or any(
            not isinstance(attempt[field], str) or SHA256_RE.fullmatch(attempt[field]) is None
            for field in (
                "asset_stage_manifest_sha256",
                "asset_stage_tree_sha256",
                "source_asset_tree_sha256",
            )
        ):
            raise HarnessError(f"capture state attempt {index} asset-stage binding changed")
        if not isinstance(attempt["returncode"], int) or isinstance(attempt["returncode"], bool):
            raise HarnessError(f"capture state attempt {index} returncode is invalid")
        if status == "COMPLETE":
            if attempt["returncode"] != 0 or attempt["artifact_sha256"] != completed[job_id]["artifact_sha256"]:
                raise HarnessError(f"capture state successful attempt {index} does not bind completed artifacts")
            if any(
                attempt[field] != completed[job_id][field]
                for field in (
                    "launch_nonce",
                    "runtime_receipt_sha256",
                    "process_id",
                    "executable_sha256",
                    "asset_stage",
                    "asset_stage_manifest_sha256",
                    "asset_stage_tree_sha256",
                    "source_asset_tree_sha256",
                )
            ):
                raise HarnessError(f"capture state successful attempt {index} process receipt changed")
            complete_attempts[job_id] += 1
        elif attempt["returncode"] == 0:
            raise HarnessError(f"capture state failed attempt {index} has a zero returncode")
        if status == "FAILED":
            failure_fields = ("failure_phase" in attempt, "failure_type" in attempt)
            if failure_fields[0] != failure_fields[1] or (
                failure_fields[0]
                and (
                    not isinstance(attempt["failure_phase"], str)
                    or not attempt["failure_phase"]
                    or not isinstance(attempt["failure_type"], str)
                    or not attempt["failure_type"]
                )
            ):
                raise HarnessError(f"capture state failed attempt {index} failure provenance is malformed")
        if attempt["log"] != completed[job_id]["log"] and status == "COMPLETE":
            raise HarnessError(f"capture state completed log differs at attempt {index}")
        normalized_attempts.append(attempt)
    missing_success = sorted(job_id for job_id in jobs_by_id if complete_attempts[job_id] < 1)
    if missing_success:
        raise HarnessError(f"capture state lacks successful attempts for {missing_success[:5]}")
    return {
        "version": 1,
        "warning": WARNING,
        "path": str(state_path),
        "sha256": sha256_file(state_path),
        "plan_sha256_history": history,
        "expected_jobs": len(jobs),
        "completed_jobs": len(completed),
        "renderer_invocations": len(attempts),
        "failed_attempts": sum(attempt["status"] == "FAILED" for attempt in normalized_attempts),
        "pinned_executable_sha256": pinned_executable_sha256,
        "baseline_oracle_equivalence": baseline_oracle_equivalence,
        "completed": completed_summary,
    }


def _validate_reproduction_for_plan(
    plan: Mapping[str, Any],
    records_by_job: Mapping[str, Mapping[str, Any]],
) -> Dict[str, Any]:
    """Prove one fresh process reproduces a final still and stable report state."""

    jobs = plan["study"]["reproduction_jobs"]
    if plan["study"]["status"] != "READY_FOR_CAPTURE":
        if jobs:
            raise HarnessError("an unresolved plan may not claim deterministic reproduction")
        return {
            "status": "NOT_YET_SCHEDULED",
            "jobs": 0,
        }
    if len(jobs) != EXPECTED_REPRODUCTION_RENDERS:
        raise HarnessError("complete plan needs exactly one genuine reproduction job")
    job = jobs[0]
    source_job_id = job["reference_job_id"]
    if source_job_id == job["id"] or source_job_id not in records_by_job:
        raise HarnessError("reproduction source job is absent or self-referential")
    if job["id"] not in records_by_job:
        raise HarnessError("reproduction rerun has no validated runtime record")
    source_job = next(
        (
            item
            for item in (
                *plan["study"]["jobs"],
                *plan["study"]["verification_jobs"],
            )
            if item["id"] == source_job_id
        ),
        None,
    )
    if source_job is None or job["reference_artifact"] not in source_job["artifacts"]:
        raise HarnessError("reproduction source artifact is not owned by its named job")
    source_index = source_job["artifacts"].index(job["reference_artifact"])
    source_record = records_by_job[source_job_id]
    rerun_record = records_by_job[job["id"]]
    if len(rerun_record["pngs"]) != 1 or len(rerun_record["reports"]) != 1:
        raise HarnessError("reproduction job must contain exactly one still/report pair")
    first_png = pathlib.Path(source_record["pngs"][source_index]["path"])
    second_png = pathlib.Path(rerun_record["pngs"][0]["path"])
    evidence = verify_reproduction(
        first_png,
        second_png,
        camera_id=job["reference_camera_id"],
    )
    source_receipt = source_record["runtime_receipt"]
    rerun_receipt = rerun_record["runtime_receipt"]
    if (
        source_receipt["launch_nonce"] == rerun_receipt["launch_nonce"]
        or source_receipt["receipt_sha256"] == rerun_receipt["receipt_sha256"]
        or source_receipt["process_id"] == rerun_receipt["process_id"]
    ):
        raise HarnessError("reproduction did not come from a fresh runtime launch")
    for field in ("executable_sha256", "source_provenance_sha256", "profile_sha256"):
        if source_receipt[field] != rerun_receipt[field]:
            raise HarnessError(f"reproduction runtime receipt changed {field}")
    if source_receipt["profile_sha256"] != job["profile_sha256"]:
        raise HarnessError("reproduction receipt does not bind the selected profile")
    return {
        "status": "REPRODUCED",
        "jobs": 1,
        "source_job_id": source_job_id,
        "reproduction_job_id": job["id"],
        "camera_id": job["reference_camera_id"],
        "profile_sha256": job["profile_sha256"],
        "source_png": str(first_png),
        "reproduction_png": str(second_png),
        "source_report_sha256": source_record["reports"][source_index]["sha256"],
        "reproduction_report_sha256": rerun_record["reports"][0]["sha256"],
        "source_runtime_receipt_sha256": source_receipt["receipt_sha256"],
        "reproduction_runtime_receipt_sha256": rerun_receipt["receipt_sha256"],
        "fresh_launch_nonces": True,
        **evidence,
    }


def validate_capture_artifacts(
    plan_path: pathlib.Path,
    *,
    include_motion: bool = False,
    include_runtime_records: bool = False,
) -> Dict[str, Any]:
    """Validate captured PNGs/sidecars, authority equality, reuse, and matrix deltas."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    output_root = _require_outside_source_tree(pathlib.Path(plan["output_root"]), "output root")
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    jobs = [
        *plan["study"]["jobs"],
        *plan["study"]["verification_jobs"],
        *plan["study"]["reproduction_jobs"],
    ]
    if include_motion:
        jobs.extend(plan["motion"]["jobs"])
    world_authority = None
    gameplay_authority: Dict[Tuple[str, Optional[float]], str] = {}
    anchor_heights = None
    anchor_classes = None
    projection_hashes_by_profile_phase: Dict[Tuple[str, float], Mapping[str, str]] = {}
    png_records = []
    report_records = []
    records_by_job = {}
    artifact_to_semantic: Dict[str, str] = {}
    for slot in plan["study"]["logical_slots"]:
        previous = artifact_to_semantic.setdefault(slot["artifact"], slot["semantic_key"])
        if previous != slot["semantic_key"]:
            raise HarnessError(f"one PNG is assigned to distinct semantic states: {slot['artifact']}")
    for job in jobs:
        record = _validate_job_artifacts(
            job,
            output_root,
            source_provenance_sha256=sha256_object(plan["provenance"]),
            label=False,
            require_labeled=True,
        )
        records_by_job[job["id"]] = record
        world_authority = _record_authority(
            record["authority"],
            condition_key=(job["lighting"], job["time_hours"]),
            world_reference=world_authority,
            gameplay_by_condition=gameplay_authority,
        )
        if anchor_heights is None:
            anchor_heights = record["anchor_heights"]
            anchor_classes = record["anchor_classes"]
        elif anchor_heights != record["anchor_heights"] or anchor_classes != record["anchor_classes"]:
            raise HarnessError(f"anchor heights/classes changed at {job['id']}")
        for projection_state in record["projection_states"]:
            _record_projection_hashes(
                projection_state["projection_hashes"],
                profile_sha256=job["profile_sha256"],
                liquid_phase_seconds=projection_state["liquid_phase_seconds"],
                references=projection_hashes_by_profile_phase,
            )
        png_records.extend(record["pngs"])
        report_records.extend(record["reports"])

    accounting = plan["study"]["slot_accounting"]
    new_study_paths = {
        str(pathlib.Path(entry["path"]).resolve())
        for job in plan["study"]["jobs"]
        for entry in job["capture_plan"]["captures"]
    }
    verification_paths = {
        str(pathlib.Path(entry["path"]).resolve())
        for job in plan["study"]["verification_jobs"]
        for entry in job["capture_plan"]["captures"]
    }
    reproduction_paths = {
        str(pathlib.Path(entry["path"]).resolve())
        for job in plan["study"]["reproduction_jobs"]
        for entry in job["capture_plan"]["captures"]
    }
    materialized_study_paths = {
        str(_raw_artifact_path(raw_capture_root, slot["artifact"]))
        for slot in plan["study"]["logical_slots"]
    } | {
        str(_raw_artifact_path(raw_capture_root, row["artifact"]))
        for row in plan["study"]["shared_control_references"]
    }
    if len(materialized_study_paths) != accounting["materialized_unique_paths"]:
        raise HarnessError("planned materialized still-path count changed")
    reference_paths = {
        str(_raw_artifact_path(raw_capture_root, row["artifact"]))
        for row in plan["study"]["shared_control_references"]
    }
    if new_study_paths != materialized_study_paths:
        raise HarnessError("fresh renderer jobs do not cover the materialized gallery")
    treatment_path_count = len(materialized_study_paths - reference_paths)
    if treatment_path_count != accounting["unique_non_control_treatment_pngs"]:
        raise HarnessError("unique non-control treatment-PNG count changed")
    if len(new_study_paths) != accounting["new_unique_study_renders"]:
        raise HarnessError("new unique study-render count changed")
    if len(verification_paths) != accounting["new_unique_control_verification_renders"]:
        raise HarnessError("control-verification render count changed")
    if len(reproduction_paths) != accounting["new_unique_reproduction_renders"]:
        raise HarnessError("deterministic-reproduction render count changed")
    new_unique_still_paths = new_study_paths | verification_paths | reproduction_paths
    if len(new_unique_still_paths) != accounting["new_unique_still_renders"]:
        raise HarnessError("new unique still-render accounting changed")
    if (
        accounting["baseline_oracle_primary_renders"]
        != EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS
        or accounting["baseline_oracle_stability_diagnostic_renders"]
        != EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
        or accounting["total_accounted_evidence_pngs"]
        != len(new_unique_still_paths) + EXPECTED_BASELINE_ORACLE_PRIMARY_RENDERS
    ):
        raise HarnessError("baseline-oracle still-render accounting changed")
    if (
        accounting["unique_non_control_treatment_png_ceiling"]
        != MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS
        or accounting["total_accounted_evidence_png_ceiling"]
        != MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS
    ):
        raise HarnessError("still-render ceiling contract changed")
    if plan["study"]["status"] == "READY_FOR_CAPTURE":
        if accounting["resolved_logical_slots"] != accounting["expected_complete_logical_slots"]:
            raise HarnessError("complete gallery does not contain every resolved logical slot")
        _validate_still_png_ceilings(
            accounting["unique_non_control_treatment_pngs"],
            accounting["total_accounted_evidence_pngs"],
        )

    control_by_camera = {
        (row["lighting"], row["camera_id"]): _raw_artifact_path(
            raw_capture_root, row["artifact"]
        )
        for row in plan["study"]["shared_control_references"]
    }
    stage_one = [slot for slot in plan["study"]["logical_slots"] if slot["stage"] == "01-neutral-screen"]
    if len(stage_one) != 240:
        raise HarnessError("neutral 60-by-4 screen is incomplete")
    for profile in atomic_profiles():
        candidates = [
            slot
            for slot in stage_one
            if slot["profile_id"] == profile.id and slot["camera_id"] == PRIMARY_CAMERAS[profile.family]
        ]
        if len(candidates) != 1:
            raise HarnessError(f"{profile.id} lacks its named diagnostic capture")
        candidate_path = _raw_artifact_path(raw_capture_root, candidates[0]["artifact"])
        control_path = control_by_camera[("neutral", PRIMARY_CAMERAS[profile.family])]
        if decoded_rgb_sha256(candidate_path) == decoded_rgb_sha256(control_path):
            raise HarnessError(f"{profile.id} exactly duplicates control at its named diagnostic camera")
    control_equivalence = _validate_control_equivalence_for_plan(plan, records_by_job)
    reproduction = _validate_reproduction_for_plan(plan, records_by_job)
    motion_deliverables = (
        _validate_motion_deliverables_for_plan(plan) if include_motion else None
    )
    performance = validate_performance_evidence(plan, records_by_job) if include_motion else {
        "jobs": {
            job_id: _job_performance_summary(record)
            for job_id, record in sorted(records_by_job.items())
        },
        "leader_comparisons": [],
        "maximum_ratio": 1.15,
        "p95_method": "nearest-rank",
    }
    all_still_hashes = {
        decoded_rgb_sha256(pathlib.Path(path)) for path in materialized_study_paths
    }
    result = {
        "version": 1,
        "warning": WARNING,
        "jobs_validated": len(jobs),
        "pngs_validated": len(png_records),
        "reports_validated": len(report_records),
        "materialized_still_paths": len(materialized_study_paths),
        "new_unique_still_renders": len(new_unique_still_paths),
        "total_accounted_evidence_pngs": accounting["total_accounted_evidence_pngs"],
        "fresh_shared_control_renders": len(reference_paths),
        "unique_non_control_treatment_pngs": treatment_path_count,
        "shared_control_reference_paths": len(reference_paths),
        "unique_still_content_hashes": len(all_still_hashes),
        "unique_all_validated_png_content_hashes": len(
            {decoded_rgb_sha256(pathlib.Path(record["path"])) for record in png_records}
            | all_still_hashes
        ),
        "planned_renderer_invocations": len(jobs),
        "logical_slots": accounting["resolved_logical_slots"],
        "world_authority": world_authority,
        "anchor_heights": anchor_heights,
        "anchor_classes": anchor_classes,
        "deterministic_projection_states": len(projection_hashes_by_profile_phase),
        "gameplay_authority_by_condition": {
            f"{lighting}:{time_hours}": fingerprint
            for (lighting, time_hours), fingerprint in sorted(gameplay_authority.items())
        },
        "control_equivalence": control_equivalence,
        "deterministic_reproduction": reproduction,
        "performance": performance,
        "motion_included": include_motion,
        "motion_deliverables": motion_deliverables,
        "per_capture_cleanup_is_verified_post_teardown": True,
    }
    if include_runtime_records:
        result["runtime_reports"] = report_records
        result["runtime_pngs"] = png_records
    return result


def _rankings_csv_content(review_results: Mapping[str, Any]) -> str:
    """Serialize the recomputed atomic ranking rows without editorial input."""

    destination = io.StringIO(newline="")
    fields = (
        "family",
        "rank",
        "profile_id",
        "neutral_weighted_score",
        "readability",
        "occlusion",
        "edge_quietness",
        "p95_frame_time_ms",
        "max_resident_presentation_bytes",
        "floor_pass",
        "exact_duplicate",
        "near_duplicate",
        "promoted",
        "stress_pass",
        "margin_over_control",
        "recommendable",
        "atomic_decision",
    )
    writer = csv.DictWriter(destination, fieldnames=fields, lineterminator="\n")
    writer.writeheader()
    for family in FAMILY_ORDER:
        decision = review_results["atomic"][family]["decision"]
        promoted_ids = {
            row["profile_id"] for row in review_results["atomic"][family]["finalists"]
        }
        for rank, row in enumerate(review_results["rankings"][family], start=1):
            categories = row["neutral"]["categories"]
            writer.writerow(
                {
                    "family": family,
                    "rank": rank,
                    "profile_id": row["profile_id"],
                    "neutral_weighted_score": f"{row['neutral']['weighted_score']:.6f}",
                    "readability": f"{categories['terrain_route_water_edge_readability']:.6f}",
                    "occlusion": f"{categories['shadow_occlusion_preservation']:.6f}",
                    "edge_quietness": f"{categories['edge_temporal_quietness']:.6f}",
                    "p95_frame_time_ms": f"{row['runtime']['p95_frame_time_ms']:.6f}",
                    "max_resident_presentation_bytes": row["runtime"]["max_resident_presentation_bytes"],
                    "floor_pass": str(row["floor_pass"]).lower(),
                    "exact_duplicate": str(row["metric"]["exact_duplicate"]).lower(),
                    "near_duplicate": str(row["metric"]["near_duplicate"]).lower(),
                    "promoted": str(row["profile_id"] in promoted_ids).lower(),
                    "stress_pass": (
                        "" if "stress_pass" not in row else str(row["stress_pass"]).lower()
                    ),
                    "margin_over_control": (
                        "" if "margin" not in row else f"{row['margin']:.6f}"
                    ),
                    "recommendable": (
                        "" if "recommendable" not in row else str(row["recommendable"]).lower()
                    ),
                    "atomic_decision": decision,
                }
            )
    return destination.getvalue()


def _write_review_derivatives(output_root: pathlib.Path, review_results: Mapping[str, Any]) -> Dict[str, Any]:
    review_path = output_root / "review.json"
    rankings_path = output_root / "rankings.csv"
    review_content = pretty_json(review_results)
    ranking_content = _rankings_csv_content(review_results)
    for path, content in ((review_path, review_content), (rankings_path, ranking_content)):
        if path.exists() and path.read_text(encoding="utf-8") != content:
            raise HarnessError(f"refusing to overwrite changed review derivative {path}")
        if not path.exists():
            atomic_write(path, content)
    return {
        "review": {"path": str(review_path), "sha256": sha256_file(review_path)},
        "rankings": {"path": str(rankings_path), "sha256": sha256_file(rankings_path)},
    }


def build_review_derivatives(
    plan_path: pathlib.Path,
    review_paths: Sequence[pathlib.Path],
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
) -> Dict[str, Any]:
    """Write recomputed review/ranking inputs before narrative/XLSX authoring."""

    if len(review_paths) != 2:
        raise HarnessError("build-review-derivatives requires exactly two reviewer files")
    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    if plan["status"] != "READY_FOR_CAPTURE":
        raise HarnessError("review derivatives require a fully resolved final capture plan")
    review_results = validate_final_review_evidence(
        plan_path,
        plan,
        review_paths,
        metric_path,
        performance_path,
    )
    artifacts = _write_review_derivatives(output_root, review_results)
    return {
        "version": 1,
        "warning": WARNING,
        "status": "REVIEW_DERIVATIVES_WRITTEN",
        "review_results_canonical_sha256": sha256_object(review_results),
        **artifacts,
    }


def _xml_local_name(tag: str) -> str:
    return tag.rsplit("}", 1)[-1]


def _validate_formula_workbook(
    path: pathlib.Path,
    *,
    review_results: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    """Prove XLSX formulas, cross-sheet linkage, and recalculation semantics.

    Sheet names and a few arbitrary ``<f>`` tags are insufficient evidence.  The
    contract therefore resolves workbook relationships, requires formulas on the
    intended sheets, checks their cross-sheet dependency direction, and requires
    a full automatic recalculation on load.  A finalized workbook also embeds the
    canonical-object SHA-256 of the review result as an immutable input binding;
    that digest is deliberately distinct from the byte hash of ``review.json``.
    """

    if not path.is_file() or path.is_symlink() or path.suffix.lower() != ".xlsx":
        raise HarnessError(f"formula-backed workbook is unavailable: {path}")
    if path.stat().st_size < 12_000:
        raise HarnessError("review workbook is an implausibly small package/stub")
    required_sheet_names = ("Ratings", "Rankings", "Performance", "Recommendations")
    try:
        with zipfile.ZipFile(path) as archive:
            archive_names = archive.namelist()
            names = set(archive_names)
            if len(names) != len(archive_names) or any(
                name.startswith("/") or ".." in pathlib.PurePosixPath(name).parts
                for name in archive_names
            ):
                raise HarnessError("review workbook has duplicate or unsafe package parts")
            if archive.testzip() is not None:
                raise HarnessError("review workbook contains a corrupt zip member")
            required_parts = {
                "[Content_Types].xml",
                "_rels/.rels",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
            }
            if not required_parts <= names:
                raise HarnessError("review workbook is not a valid XLSX package")
            forbidden_part_prefixes = (
                "xl/externalLinks/",
                "xl/queryTables/",
            )
            if "xl/connections.xml" in names or any(
                name.startswith(forbidden_part_prefixes) for name in names
            ):
                raise HarnessError("review workbook may not contain external-link/query connections")
            parsed = {}
            for xml_name in required_parts:
                try:
                    parsed[xml_name] = ElementTree.fromstring(archive.read(xml_name))
                except ElementTree.ParseError as error:
                    raise HarnessError(f"review workbook has malformed XML in {xml_name}") from error
            for relationship_name in sorted(
                name for name in names if name.endswith(".rels")
            ):
                try:
                    relationship_root = ElementTree.fromstring(
                        archive.read(relationship_name)
                    )
                except ElementTree.ParseError as error:
                    raise HarnessError(
                        f"review workbook has malformed relationships in {relationship_name}"
                    ) from error
                if any(
                    _xml_local_name(node.tag) == "Relationship"
                    and (
                        node.attrib.get("TargetMode", "Internal") != "Internal"
                        or re.match(
                            r"^(?:[A-Za-z][A-Za-z0-9+.-]*:|//|\\\\)",
                            node.attrib.get("Target", ""),
                        )
                        is not None
                    )
                    for node in relationship_root.iter()
                ):
                    raise HarnessError("review workbook may not use external relationships")

            content_types = parsed["[Content_Types].xml"]
            defaults = {
                node.attrib.get("Extension"): node.attrib.get("ContentType")
                for node in content_types
                if _xml_local_name(node.tag) == "Default"
            }
            overrides = {
                node.attrib.get("PartName"): node.attrib.get("ContentType")
                for node in content_types
                if _xml_local_name(node.tag) == "Override"
            }
            if (
                defaults.get("rels")
                != "application/vnd.openxmlformats-package.relationships+xml"
                or defaults.get("xml") != "application/xml"
                or overrides.get("/xl/workbook.xml")
                != "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"
            ):
                raise HarnessError("review workbook content-type declarations are invalid")

            root_relationships = [
                node
                for node in parsed["_rels/.rels"]
                if _xml_local_name(node.tag) == "Relationship"
            ]
            office_relationships = [
                node
                for node in root_relationships
                if node.attrib.get("Type", "").endswith("/officeDocument")
            ]
            if (
                len(office_relationships) != 1
                or office_relationships[0].attrib.get("Target") != "xl/workbook.xml"
                or office_relationships[0].attrib.get("TargetMode", "Internal") != "Internal"
            ):
                raise HarnessError("review workbook package root does not resolve its workbook")

            workbook_root = parsed["xl/workbook.xml"]
            for defined_name in workbook_root.iter():
                if _xml_local_name(defined_name.tag) != "definedName":
                    continue
                value = "".join(defined_name.itertext())
                if (
                    re.search(r"\[[^\]\r\n]+\][^!\r\n]*!", value) is not None
                    or re.search(r"(?:https?|file|ftp)://|\\\\", value, re.IGNORECASE)
                    is not None
                ):
                    raise HarnessError("review workbook defined name contains an external reference")
            calc_nodes = [node for node in workbook_root.iter() if _xml_local_name(node.tag) == "calcPr"]
            if len(calc_nodes) != 1:
                raise HarnessError("review workbook must define exactly one calcPr policy")
            calc = calc_nodes[0].attrib
            truthy = {"1", "true", "TRUE"}
            if (
                calc.get("calcMode") != "auto"
                or calc.get("fullCalcOnLoad") not in truthy
                or calc.get("forceFullCalc") not in truthy
            ):
                raise HarnessError(
                    "review workbook must use automatic full recalculation on load"
                )

            relationships = {}
            for relationship in parsed["xl/_rels/workbook.xml.rels"].iter():
                if _xml_local_name(relationship.tag) != "Relationship":
                    continue
                identifier = relationship.attrib.get("Id")
                target = relationship.attrib.get("Target")
                if identifier and target:
                    if not relationship.attrib.get("Type", "").endswith("/worksheet"):
                        continue
                    if relationship.attrib.get("TargetMode", "Internal") != "Internal":
                        raise HarnessError("review workbook may not use external sheet relationships")
                    if target.startswith("/"):
                        resolved = target.lstrip("/")
                    else:
                        resolved = posixpath.normpath(posixpath.join("xl", target))
                    if not resolved.startswith("xl/worksheets/"):
                        raise HarnessError(
                            f"review workbook sheet relationship escapes worksheets: {target}"
                        )
                    relationships[identifier] = resolved

            sheet_parts: Dict[str, str] = {}
            for sheet in workbook_root.iter():
                if _xml_local_name(sheet.tag) != "sheet":
                    continue
                name = sheet.attrib.get("name")
                relationship_id = next(
                    (value for key, value in sheet.attrib.items() if _xml_local_name(key) == "id"),
                    None,
                )
                if isinstance(name, str) and isinstance(relationship_id, str):
                    target = relationships.get(relationship_id)
                    if target is not None:
                        sheet_parts[name] = target
            if any(name not in sheet_parts for name in required_sheet_names):
                raise HarnessError(
                    "review workbook must link Ratings, Rankings, Performance, and Recommendations sheets"
                )

            formulas_by_sheet: Dict[str, List[str]] = {}
            worksheet_bytes: Dict[str, bytes] = {}
            worksheet_models: Dict[str, Dict[str, Any]] = {}
            shared_strings: List[str] = []
            if "xl/sharedStrings.xml" in names:
                try:
                    shared_root = ElementTree.fromstring(archive.read("xl/sharedStrings.xml"))
                except ElementTree.ParseError as error:
                    raise HarnessError("review workbook has malformed shared strings") from error
                shared_strings = [
                    "".join(node.itertext())
                    for node in shared_root.iter()
                    if _xml_local_name(node.tag) == "si"
                ]
            for sheet_name in required_sheet_names:
                worksheet_name = sheet_parts[sheet_name]
                if worksheet_name not in names:
                    raise HarnessError(f"review workbook sheet target is missing: {worksheet_name}")
                try:
                    worksheet_bytes[sheet_name] = archive.read(worksheet_name)
                    worksheet = ElementTree.fromstring(worksheet_bytes[sheet_name])
                except ElementTree.ParseError as error:
                    raise HarnessError(
                        f"review workbook has malformed XML in {worksheet_name}"
                    ) from error
                formulas_by_sheet[sheet_name] = [
                    "".join(node.itertext()).strip()
                    for node in worksheet.iter()
                    if _xml_local_name(node.tag) == "f"
                ]

                expected_content_type = (
                    "application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"
                )
                if overrides.get("/" + worksheet_name) != expected_content_type:
                    raise HarnessError(
                        f"review workbook lacks worksheet content type for {worksheet_name}"
                    )
                sheet_data = [
                    node for node in worksheet.iter() if _xml_local_name(node.tag) == "sheetData"
                ]
                if len(sheet_data) != 1:
                    raise HarnessError(f"review workbook {sheet_name} has invalid sheetData")
                rows: Dict[int, List[str]] = {}
                cells: Dict[str, Dict[str, Any]] = {}
                for row_node in sheet_data[0]:
                    if _xml_local_name(row_node.tag) != "row":
                        continue
                    try:
                        row_number = int(row_node.attrib["r"])
                    except (KeyError, TypeError, ValueError) as error:
                        raise HarnessError(
                            f"review workbook {sheet_name} has an unnumbered row"
                        ) from error
                    if row_number <= 0 or row_number in rows:
                        raise HarnessError(f"review workbook {sheet_name} repeats a row")
                    row_cells = []
                    for cell_node in row_node:
                        if _xml_local_name(cell_node.tag) != "c":
                            continue
                        reference = cell_node.attrib.get("r")
                        match = re.fullmatch(r"([A-Z]{1,3})([1-9][0-9]*)", reference or "")
                        if match is None or int(match.group(2)) != row_number or reference in cells:
                            raise HarnessError(
                                f"review workbook {sheet_name} has an invalid cell reference"
                            )
                        formula_nodes = [
                            node for node in cell_node if _xml_local_name(node.tag) == "f"
                        ]
                        value_nodes = [
                            node for node in cell_node if _xml_local_name(node.tag) == "v"
                        ]
                        inline_nodes = [
                            node for node in cell_node.iter() if _xml_local_name(node.tag) == "t"
                        ]
                        value_text = ""
                        if cell_node.attrib.get("t") == "s" and value_nodes:
                            try:
                                value_text = shared_strings[int(value_nodes[0].text or "")]
                            except (ValueError, IndexError) as error:
                                raise HarnessError(
                                    f"review workbook {sheet_name} has a bad shared-string index"
                                ) from error
                        elif inline_nodes:
                            value_text = "".join(node.text or "" for node in inline_nodes)
                        elif value_nodes:
                            value_text = value_nodes[0].text or ""
                        cells[reference] = {
                            "formula": "".join(formula_nodes[0].itertext()).strip()
                            if formula_nodes
                            else None,
                            "value": value_text,
                        }
                        row_cells.append(reference)
                    if not row_cells:
                        raise HarnessError(f"review workbook {sheet_name} contains an empty row")
                    rows[row_number] = row_cells
                if not rows or not cells:
                    raise HarnessError(f"review workbook {sheet_name} has no resolved cells")
                dimensions = [
                    node.attrib.get("ref")
                    for node in worksheet.iter()
                    if _xml_local_name(node.tag) == "dimension"
                ]
                if len(dimensions) != 1 or not isinstance(dimensions[0], str):
                    raise HarnessError(f"review workbook {sheet_name} lacks a worksheet dimension")
                worksheet_models[sheet_name] = {
                    "rows": rows,
                    "cells": cells,
                    "dimension": dimensions[0],
                    "max_row": max(rows),
                }

            minimum_rows = {
                "Ratings": 61,
                "Rankings": 61,
                "Performance": 5,
                "Recommendations": 14,
            }
            for sheet_name, minimum in minimum_rows.items():
                if worksheet_models[sheet_name]["max_row"] < minimum:
                    raise HarnessError(
                        f"review workbook {sheet_name} lacks substantive evidence rows"
                    )
            if len(formulas_by_sheet["Ratings"]) < 60:
                raise HarnessError("Ratings must contain at least 60 weighted-score formulas")
            if len(formulas_by_sheet["Rankings"]) < 60:
                raise HarnessError("Rankings must contain at least 60 formula-linked rows")
            if len(formulas_by_sheet["Performance"]) < 4:
                raise HarnessError("Performance must contain at least four aggregate formulas")
            if len(formulas_by_sheet["Recommendations"]) < 13:
                raise HarnessError("Recommendations must contain at least 13 formula-linked gates")
            if sum("SUMPRODUCT" in formula.upper() for formula in formulas_by_sheet["Ratings"]) < 60:
                raise HarnessError("Ratings formulas must implement weighted SUMPRODUCT scores")
            rankings_formula = "\n".join(formulas_by_sheet["Rankings"])
            performance_formula = "\n".join(formulas_by_sheet["Performance"])
            recommendations_formula = "\n".join(formulas_by_sheet["Recommendations"])
            if re.search(r"'?Ratings'?\s*!", rankings_formula, re.IGNORECASE) is None:
                raise HarnessError("Rankings formulas must depend on Ratings")
            if re.search(r"\b(RANK|RANK\.EQ|SORT|SORTBY)\s*\(", rankings_formula, re.IGNORECASE) is None:
                raise HarnessError("Rankings formulas must compute a deterministic rank/order")
            if re.search(r"\b(PERCENTILE(?:\.INC)?|MAX)\s*\(", performance_formula, re.IGNORECASE) is None:
                raise HarnessError("Performance formulas must aggregate measured samples")
            if re.search(r"'?Rankings'?\s*!", recommendations_formula, re.IGNORECASE) is None:
                raise HarnessError("Recommendations formulas must depend on Rankings")
            if re.search(r"'?Performance'?\s*!", recommendations_formula, re.IGNORECASE) is None:
                raise HarnessError("Recommendations formulas must depend on Performance")
            if re.search(r"\b(IF|AND)\s*\(", recommendations_formula, re.IGNORECASE) is None:
                raise HarnessError("Recommendations formulas must implement explicit decision gates")

            cross_sheet_reference = re.compile(
                r"'?((?:Ratings|Rankings|Performance|Recommendations))'?\s*!\s*"
                r"\$?[A-Z]{1,3}\$?([1-9][0-9]*)(?::\$?[A-Z]{1,3}\$?([1-9][0-9]*))?",
                re.IGNORECASE,
            )
            for source_sheet, formulas in formulas_by_sheet.items():
                for formula in formulas:
                    if (
                        re.search(r"\[[^\]\r\n]+\][^!\r\n]*!", formula) is not None
                        or re.search(
                            r"(?:https?|file|ftp)://|\\\\|\b(?:HYPERLINK|WEBSERVICE|RTD)\s*\(",
                            formula,
                            re.IGNORECASE,
                        )
                        is not None
                    ):
                        raise HarnessError(
                            f"review workbook {source_sheet} formula contains an external reference"
                        )
                    for match in cross_sheet_reference.finditer(formula):
                        target = next(
                            name
                            for name in required_sheet_names
                            if name.lower() == match.group(1).lower()
                        )
                        end_row = int(match.group(3) or match.group(2))
                        if end_row > worksheet_models[target]["max_row"]:
                            raise HarnessError(
                                f"review workbook {source_sheet} formula references absent {target} rows"
                            )

            searchable = b"\n".join(
                archive.read(name)
                for name in sorted(names)
                if name == "xl/sharedStrings.xml" or name.startswith("xl/worksheets/")
            )
            if WARNING.encode("utf-8") not in searchable:
                raise HarnessError("review workbook lacks the exact structural-draft warning")
            for sheet_name, model in worksheet_models.items():
                if not any(
                    cell["value"] == WARNING for cell in model["cells"].values()
                ):
                    raise HarnessError(
                        f"review workbook {sheet_name} sheet lacks the exact warning"
                    )
            review_results_canonical_sha256 = (
                sha256_object(review_results) if review_results is not None else None
            )
            if (
                review_results_canonical_sha256 is not None
                and review_results_canonical_sha256.encode("ascii") not in searchable
            ):
                raise HarnessError(
                    "review workbook is not bound to the canonical review-results object hash"
                )
            if review_results is not None:
                missing_profiles = [
                    profile.id
                    for profile in atomic_profiles()
                    if profile.id.encode("utf-8") not in searchable
                ]
                if missing_profiles:
                    raise HarnessError(
                        f"review workbook omits ranked profile IDs: {missing_profiles[:5]}"
                    )
                try:
                    atomic_decisions = [
                        review_results["atomic"][family]["decision"]
                        for family in FAMILY_ORDER
                    ]
                except (KeyError, TypeError) as error:
                    raise HarnessError("review results cannot bind workbook decisions") from error
                for decision in atomic_decisions:
                    expected_text = (
                        "control/no change" if decision == "control" else decision
                    )
                    if expected_text.encode("utf-8") not in searchable:
                        raise HarnessError(
                            f"review workbook omits atomic decision {expected_text!r}"
                        )
            formula_count = sum(len(formulas) for formulas in formulas_by_sheet.values())
    except zipfile.BadZipFile as error:
        raise HarnessError(f"review workbook is not a valid zip package: {path}") from error
    return {
        "path": str(path),
        "sha256": sha256_file(path),
        "bytes": path.stat().st_size,
        "formula_cells": formula_count,
        "formula_cells_by_sheet": {
            name: len(formulas_by_sheet[name]) for name in required_sheet_names
        },
        "resolved_rows_by_sheet": {
            name: len(worksheet_models[name]["rows"]) for name in required_sheet_names
        },
        "resolved_cells_by_sheet": {
            name: len(worksheet_models[name]["cells"]) for name in required_sheet_names
        },
        "required_sheets": list(required_sheet_names),
        "automatic_recalculation": True,
        "cross_sheet_linkage": {
            "rankings_from_ratings": True,
            "recommendations_from_rankings": True,
            "recommendations_from_performance": True,
        },
        "review_results_canonical_sha256": review_results_canonical_sha256,
    }


_NARRATIVE_SECTIONS = (
    "research",
    "recommendations",
    "rejected settings",
    "interaction findings",
    "limitations",
    "implementation cost",
)

_ATOMIC_NARRATIVE_LABELS = {
    "snow": "Snow",
    "water": "Water optics",
    "physical_clouds": "World-space clouds",
    "shore_and_falls": "Shoreline / falls",
    "alpine_vegetation": "Alpine vegetation",
    "cliff_strata": "Cliff / strata",
    "terrain_props": "Terrain props",
    "ice_fringe": "Ice fringes",
    "local_fog": "Local fog",
}

_COMBINATION_NARRATIVE_LABELS = {
    "restrained": "Restrained",
    "expressive": "Expressive",
    "score-leader": "Score leader",
}

_REQUIRED_RESEARCH_URLS = (
    "https://www.sidefx.com/docs/houdini/nodes/sop/heightfield_maskbyfeature.html",
    "https://doi.org/10.1145/344779.344809",
    "https://docs.unity.cn/Packages/com.unity.render-pipelines.high-definition@14.0/manual/WaterSystem-Materials.html",
    "https://bevy.org/examples/3d-rendering/transmission/",
    "https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-cloud-component-in-unreal-engine",
    "https://www.guerrilla-games.com/read/nubis-cubed",
    "https://www.nintendo.com/en-gb/News/2018/April/Interview-Taking-on-hordes-of-invading-Vikings-in-Bad-North-1368315.html",
    "https://www.gamedeveloper.com/business/sparking-joy-through-tile-placement-in-idyllic-village-builder-i-dorfromantik-i-",
)


def _validate_narrative(
    path: pathlib.Path,
    *,
    review_results: Optional[Mapping[str, Any]] = None,
) -> Dict[str, Any]:
    """Require a substantive, evidence-linked final report rather than a shell."""

    if not path.is_file() or path.is_symlink() or path.suffix.lower() != ".md":
        raise HarnessError(f"narrative Markdown is unavailable: {path}")
    narrative = path.read_text(encoding="utf-8")
    if WARNING not in narrative:
        raise HarnessError("narrative README.md lacks the exact structural-draft warning")
    placeholder_patterns = (
        r"\bTODO\b",
        r"\bTBD\b",
        r"\bFIXME\b",
        r"lorem ipsum",
        r"\[insert\b",
        r"this report is incomplete",
    )
    if any(re.search(pattern, narrative, re.IGNORECASE) for pattern in placeholder_patterns):
        raise HarnessError("narrative README.md contains placeholder or incomplete-report text")
    if re.search(r"\{\{[^{}\n]*\}\}", narrative) or "{{" in narrative or "}}" in narrative:
        raise HarnessError("narrative README.md contains an unresolved double-brace token")

    headers = list(re.finditer(r"(?m)^##\s+(.+?)\s*$", narrative))
    sections: Dict[str, str] = {}
    for index, header in enumerate(headers):
        title = re.sub(r"[`*_]", "", header.group(1)).strip().lower()
        end = headers[index + 1].start() if index + 1 < len(headers) else len(narrative)
        for required in _NARRATIVE_SECTIONS:
            if required == title and required not in sections:
                sections[required] = narrative[header.end() : end].strip()
    missing = [name for name in _NARRATIVE_SECTIONS if name not in sections]
    if missing:
        raise HarnessError(f"narrative README.md lacks substantive sections: {missing}")
    section_word_counts = {
        name: len(re.findall(r"\b[\w'-]+\b", content))
        for name, content in sections.items()
    }
    minimum_words = {name: (100 if name == "research" else 40) for name in _NARRATIVE_SECTIONS}
    thin = {
        name: count
        for name, count in section_word_counts.items()
        if count < minimum_words[name]
    }
    if thin:
        raise HarnessError(f"narrative README.md sections are too short for evidence review: {thin}")
    total_words = len(re.findall(r"\b[\w'-]+\b", narrative))
    if total_words < 500:
        raise HarnessError("narrative README.md is too short to be the final report")
    research_links = re.findall(r"https?://[^\s)>]+", sections["research"])
    missing_research = [url for url in _REQUIRED_RESEARCH_URLS if url not in sections["research"]]
    if missing_research:
        raise HarnessError(
            f"research section omits prescribed sources: {missing_research}"
        )
    family_patterns = {
        "snow": r"\bsnow\b",
        "water": r"\bwater\b",
        "physical_clouds": r"\bclouds?\b",
        "shore_and_falls": r"\bshore(?:line)?\b|\bfalls?\b|\bwaterfalls?\b",
        "alpine_vegetation": r"\bvegetation\b|\balpine\b",
        "cliff_strata": r"\bcliffs?\b|\bstrata\b",
        "terrain_props": r"\bprops?\b|\bboulders?\b|\bdeadwood\b",
        "ice_fringe": r"\bice\b|\bfringes?\b",
        "local_fog": r"\bfog\b",
    }
    missing_families = [
        family
        for family, pattern in family_patterns.items()
        if re.search(pattern, narrative, re.IGNORECASE) is None
    ]
    if missing_families:
        raise HarnessError(f"narrative README.md omits visual families: {missing_families}")
    rejected = sections["rejected settings"].lower()
    for finding in ("bevel", "0.04", "0.08", "black cracks", "vertical banding", "honeycomb noise"):
        if finding not in rejected:
            raise HarnessError(f"rejected-settings section omits prior bevel finding {finding!r}")
    for evidence_name in ("review.json", "rankings.csv", "review-workbook.xlsx", "manifest.json"):
        if evidence_name not in narrative:
            raise HarnessError(f"narrative README.md does not link or name {evidence_name}")
    for prior_path in (
        "provenance/prior-aesthetic-report/README.md",
        "provenance/prior-aesthetic-report/manifest.json",
    ):
        if prior_path not in narrative:
            raise HarnessError(f"narrative README.md does not link offline prior evidence {prior_path}")
    bound_decisions = None
    if review_results is not None:
        try:
            bound_decisions = {
                family: review_results["atomic"][family]["decision"]
                for family in FAMILY_ORDER
            }
            bound_combinations = {
                combination_id: review_results["combinations"][combination_id]["recommended"]
                for combination_id in COMBINATION_IDS
            }
        except (KeyError, TypeError) as error:
            raise HarnessError("review results cannot bind narrative decisions") from error
        atomic_heading = re.search(
            r"(?m)^###\s+Atomic recommendations\s*$", narrative, re.IGNORECASE
        )
        if atomic_heading is None:
            raise HarnessError("narrative README.md lacks an Atomic recommendations table")
        next_heading = re.search(
            r"(?m)^#{1,3}\s+", narrative[atomic_heading.end() :]
        )
        atomic_end = (
            atomic_heading.end() + next_heading.start()
            if next_heading is not None
            else len(narrative)
        )
        atomic_rows: Dict[str, str] = {}
        for line in narrative[atomic_heading.end() : atomic_end].splitlines():
            if not line.lstrip().startswith("|"):
                continue
            cells = [cell.strip().strip("`") for cell in line.strip().strip("|").split("|")]
            if len(cells) >= 2:
                atomic_rows[cells[0].casefold()] = cells[1]
        for family, decision in bound_decisions.items():
            expected = "control/no change" if decision == "control" else decision
            label = _ATOMIC_NARRATIVE_LABELS[family]
            observed = atomic_rows.get(label.casefold())
            if observed != expected:
                raise HarnessError(
                    f"narrative Atomic recommendations row for {family} must bind "
                    f"{expected!r}, observed {observed!r}"
                )
        combined_heading = re.search(
            r"(?m)^###\s+Combined recommendations\s*$", narrative, re.IGNORECASE
        )
        if combined_heading is None:
            raise HarnessError("narrative README.md lacks a Combined recommendations table")
        next_heading = re.search(
            r"(?m)^#{1,3}\s+", narrative[combined_heading.end() :]
        )
        combined_end = (
            combined_heading.end() + next_heading.start()
            if next_heading is not None
            else len(narrative)
        )
        combined_rows: Dict[str, str] = {}
        for line in narrative[combined_heading.end() : combined_end].splitlines():
            if not line.lstrip().startswith("|"):
                continue
            cells = [cell.strip().strip("`") for cell in line.strip().strip("|").split("|")]
            if len(cells) >= 2:
                combined_rows[cells[0].casefold()] = cells[-1]
        for combination_id, recommended in bound_combinations.items():
            if not isinstance(recommended, bool):
                raise HarnessError(
                    f"review results combination {combination_id} recommendation is not boolean"
                )
            expected = "recommend" if recommended else "do not recommend"
            label = _COMBINATION_NARRATIVE_LABELS[combination_id]
            observed = combined_rows.get(label.casefold())
            if (observed.casefold() if observed is not None else None) != expected:
                raise HarnessError(
                    f"narrative Combined recommendations row for {combination_id} must bind "
                    f"{expected!r}, observed {observed!r}"
                )
    return {
        "path": str(path),
        "sha256": sha256_file(path),
        "bytes": path.stat().st_size,
        "word_count": total_words,
        "section_word_counts": section_word_counts,
        "research_link_count": len(set(research_links)),
        "prescribed_research_sources_present": len(_REQUIRED_RESEARCH_URLS),
        "visual_families_covered": list(FAMILY_ORDER),
        "review_decisions": bound_decisions,
        "placeholder_free": True,
    }


def _install_publication_input(
    source: pathlib.Path,
    destination: pathlib.Path,
) -> None:
    if not source.is_file() or source.is_symlink():
        raise HarnessError(f"publication input is unavailable: {source}")
    source_sha256 = sha256_file(source)
    if destination.exists():
        if destination.is_symlink() or not destination.is_file():
            raise HarnessError(f"publication destination is not a regular file: {destination}")
        if sha256_file(destination) != source_sha256:
            raise HarnessError(f"refusing to overwrite changed publication artifact {destination}")
        return
    destination.parent.mkdir(parents=True, exist_ok=True)
    temporary = destination.with_name(destination.name + ".tmp")
    if temporary.exists():
        raise HarnessError(f"stale publication-input temporary exists: {temporary}")
    shutil.copyfile(source, temporary)
    os.replace(temporary, destination)


def install_narrative(input_path: pathlib.Path, output_root: pathlib.Path) -> Dict[str, Any]:
    """Validate and install an authored final narrative without generating claims."""

    output_root = _require_outside_source_tree(output_root, "output root")
    validation = _validate_narrative(input_path)
    destination = output_root / "README.md"
    _install_publication_input(input_path, destination)
    installed = _validate_narrative(destination)
    if installed["sha256"] != validation["sha256"]:
        raise HarnessError("installed narrative differs from the validated input")
    return {"version": 1, "warning": WARNING, "narrative": installed}


def install_workbook(
    input_path: pathlib.Path,
    output_root: pathlib.Path,
    review_results_path: pathlib.Path,
) -> Dict[str, Any]:
    """Validate and install a formula workbook bound to published review results."""

    output_root = _require_outside_source_tree(output_root, "output root")
    review_results = _read_json(review_results_path, "review results for workbook binding")
    validation = _validate_formula_workbook(input_path, review_results=review_results)
    destination = output_root / "review-workbook.xlsx"
    _install_publication_input(input_path, destination)
    installed = _validate_formula_workbook(destination, review_results=review_results)
    if installed["sha256"] != validation["sha256"]:
        raise HarnessError("installed workbook differs from the validated input")
    return {"version": 1, "warning": WARNING, "workbook": installed}


def _validate_exact_file_inventory(
    root: pathlib.Path,
    expected_paths: Iterable[pathlib.Path],
    *,
    context: str,
) -> None:
    """Reject missing, foreign, non-regular, or symlinked files in one artifact tree."""

    expected = {path.resolve() for path in expected_paths}
    if not root.is_dir() or root.is_symlink():
        raise HarnessError(f"{context} root is unavailable or not self-contained: {root}")
    resolved_root = root.resolve()
    expected_directories = set()
    for expected_path in expected:
        try:
            expected_path.relative_to(resolved_root)
        except ValueError as error:
            raise HarnessError(f"{context} expected path escapes its root: {expected_path}") from error
        parent = expected_path.parent
        while parent != resolved_root:
            expected_directories.add(parent)
            parent = parent.parent
    actual = set()
    for path in root.rglob("*"):
        if path.is_symlink():
            raise HarnessError(f"{context} contains a symlink: {path}")
        if path.is_file():
            actual.add(path.resolve())
        elif path.is_dir():
            if path.resolve() not in expected_directories:
                raise HarnessError(f"{context} contains a foreign directory: {path}")
        else:
            raise HarnessError(f"{context} contains a non-regular filesystem entry: {path}")
    if actual != expected:
        missing = sorted(str(path) for path in expected - actual)
        foreign = sorted(str(path) for path in actual - expected)
        raise HarnessError(
            f"{context} inventory differs from the plan; "
            f"missing={missing[:5]}, foreign={foreign[:5]}"
        )


def _validate_publication_sheet_reproduction(
    spec: Mapping[str, Any],
    record: Mapping[str, Any],
    *,
    index: int,
) -> None:
    """Recompose a sheet outside the publication tree and require identical bytes."""

    expected_sources = [
        {"label": label, "path": str(path), "sha256": sha256_file(path)}
        for label, path in spec["items"]
    ]
    if record["title"] != spec["title"] or record["sources"] != expected_sources:
        raise HarnessError(f"publication sheet {index} differs from its canonical source spec")
    with tempfile.TemporaryDirectory(prefix="world-detail-sheet-verify-") as temporary:
        reproduced = _write_contact_sheet(
            spec["items"],
            pathlib.Path(temporary) / "sheet.png",
            title=spec["title"],
        )
    if reproduced["sha256"] != record["sha256"]:
        raise HarnessError(f"publication sheet {index} is not deterministically reproducible")


def _validate_publication_files(
    plan_path: pathlib.Path,
    plan: Mapping[str, Any],
    review_results: Mapping[str, Any],
    sheets: Mapping[str, Any],
    *,
    allow_exact_incomplete_marker: bool = False,
) -> Dict[str, Any]:
    """Validate every non-manifest report artifact and its exact warning/provenance."""

    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    raw_capture_root = _validate_raw_capture_root(
        output_root, pathlib.Path(plan["raw_capture_root"])
    )
    incomplete_readme = output_root / "README-INCOMPLETE.md"
    if incomplete_readme.exists() or incomplete_readme.is_symlink():
        if (
            not allow_exact_incomplete_marker
            or incomplete_readme.is_symlink()
            or not incomplete_readme.is_file()
            or incomplete_readme.read_text(encoding="utf-8") != _incomplete_readme_content()
        ):
            raise HarnessError("final publication may not retain README-INCOMPLETE.md")
    prior_evidence = _materialize_prior_aesthetic_evidence(output_root, create=False)
    readme_path = output_root / "README.md"
    narrative_validation = _validate_narrative(
        readme_path,
        review_results=review_results,
    )

    gallery_data_path = output_root / "gallery" / "data.json"
    gallery_html_path = output_root / "gallery" / "index.html"
    _validate_exact_file_inventory(
        output_root / "gallery",
        (gallery_data_path, gallery_html_path),
        context="offline gallery",
    )
    gallery = _read_json(gallery_data_path, "gallery data")
    expected_gallery = _gallery_data(plan)
    expected_slots = plan["study"]["slot_accounting"]["expected_complete_logical_slots"]
    if gallery != expected_gallery or len(gallery["slots"]) != expected_slots:
        raise HarnessError("offline gallery does not contain the exact synchronized slot plan")
    if not gallery_html_path.is_file() or gallery_html_path.is_symlink():
        raise HarnessError("offline gallery HTML is unavailable")
    gallery_html = gallery_html_path.read_text(encoding="utf-8")
    if gallery_html != _gallery_html(expected_gallery):
        raise HarnessError("offline gallery HTML is stale or is not self-contained")
    if "fetch(" in gallery_html:
        raise HarnessError("offline gallery must not fetch companion data under file://")

    sheet_manifest_path = output_root / "publication-sheets.json"
    if not sheet_manifest_path.is_file() or sheet_manifest_path.is_symlink():
        raise HarnessError("publication sheet manifest is unavailable or not self-contained")
    persisted_sheets = _read_json(sheet_manifest_path, "publication sheets")
    if persisted_sheets != sheets:
        raise HarnessError("publication sheet manifest differs from the validated sheets")
    sheets = _exact_keys(
        sheets,
        ("version", "warning", "capture_plan_sha256", "family_sheets", "final_comparison_sheets"),
        "publication sheet manifest",
    )
    if (
        sheets["version"] != 1
        or sheets["warning"] != WARNING
        or sheets["capture_plan_sha256"] != sha256_file(plan_path)
        or not isinstance(sheets["family_sheets"], list)
        or not isinstance(sheets["final_comparison_sheets"], list)
        or len(sheets["family_sheets"]) != len(FAMILY_ORDER)
        or len(sheets["final_comparison_sheets"]) != 25
    ):
        raise HarnessError("publication sheet inventory is incomplete")
    sheet_records = [*sheets["family_sheets"], *sheets["final_comparison_sheets"]]
    sheet_specs = _publication_sheet_specs(plan)
    expected_family_paths = {
        spec["path"].resolve() for spec in sheet_specs["family_sheets"]
    }
    expected_final_paths = {
        spec["path"].resolve() for spec in sheet_specs["final_comparison_sheets"]
    }
    specs_by_path = {
        spec["path"].resolve(): spec
        for spec in (
            *sheet_specs["family_sheets"],
            *sheet_specs["final_comparison_sheets"],
        )
    }
    seen_sheets = set()
    for index, raw_record in enumerate(sheet_records):
        record = _exact_keys(
            raw_record,
            (
                "path",
                "width",
                "height",
                "sha256",
                "warning",
                "visible_warning_overlay",
                "source_render_sha256",
                "title",
                "sources",
            ),
            f"publication sheet {index}",
        )
        if (
            record["warning"] != WARNING
            or record["visible_warning_overlay"] is not True
            or not isinstance(record["width"], int)
            or isinstance(record["width"], bool)
            or record["width"] <= 0
            or not isinstance(record["height"], int)
            or isinstance(record["height"], bool)
            or record["height"] <= 0
            or not isinstance(record["sha256"], str)
            or not SHA256_RE.fullmatch(record["sha256"])
            or not isinstance(record["title"], str)
            or not record["title"]
            or not isinstance(record["sources"], list)
            or not record["sources"]
        ):
            raise HarnessError(f"publication sheet {index} metadata is malformed")
        for source_index, source in enumerate(record["sources"]):
            source = _exact_keys(
                source,
                ("label", "path", "sha256"),
                f"publication sheet {index} source {source_index}",
            )
            if (
                not isinstance(source["label"], str)
                or not source["label"]
                or not isinstance(source["path"], str)
                or not isinstance(source["sha256"], str)
                or not SHA256_RE.fullmatch(source["sha256"])
                or sha256_file(pathlib.Path(source["path"])) != source["sha256"]
            ):
                raise HarnessError(f"publication sheet {index} source provenance is invalid")
        path = pathlib.Path(record["path"]).resolve()
        if path in seen_sheets or path not in specs_by_path:
            raise HarnessError(f"publication sheet path is repeated: {path}")
        seen_sheets.add(path)
        rendered = _inspect_labeled_png(path, exact_capture_size=False)
        if rendered["sha256"] != record["sha256"]:
            raise HarnessError(f"publication sheet bytes changed: {path}")
        try:
            from PIL import Image  # pylint: disable=import-outside-toplevel
            with Image.open(path) as image:
                source_manifest_sha = image.info.get("source_render_manifest_sha256")
                title = image.info.get("comparison_sheet_title")
        except (ImportError, OSError) as error:
            raise HarnessError(f"cannot inspect publication sheet provenance: {path}") from error
        expected_source_sha = sha256_object(record["sources"])
        if source_manifest_sha != expected_source_sha or title != record["title"]:
            raise HarnessError(f"publication sheet source binding changed: {path}")
        _validate_publication_sheet_reproduction(
            specs_by_path[path],
            record,
            index=index,
        )
    if (
        {pathlib.Path(row["path"]).resolve() for row in sheets["family_sheets"]}
        != expected_family_paths
        or {pathlib.Path(row["path"]).resolve() for row in sheets["final_comparison_sheets"]}
        != expected_final_paths
    ):
        raise HarnessError("publication sheet paths differ from the exact family/final inventory")
    _validate_exact_file_inventory(
        output_root / "contact-sheets",
        expected_family_paths,
        context="contact-sheet",
    )
    _validate_exact_file_inventory(
        output_root / "final-comparison-sheets",
        expected_final_paths,
        context="final-comparison-sheet",
    )

    expected_raw_paths = {
        pathlib.Path(capture["path"]).resolve()
        for job in (
            *plan["study"]["jobs"],
            *plan["study"]["verification_jobs"],
            *plan["study"]["reproduction_jobs"],
        )
        for capture in job["capture_plan"]["captures"]
    }
    if len(expected_raw_paths) != plan["study"]["slot_accounting"]["new_unique_still_renders"]:
        raise HarnessError("source-PNG plan does not match exact still-render accounting")
    expected_labeled_paths = {
        _labeled_capture_path(output_root, raw_path).resolve()
        for raw_path in expected_raw_paths
    }
    _validate_exact_file_inventory(
        output_root / "source-pngs",
        expected_labeled_paths,
        context="source-PNG",
    )
    labeled_sources = []
    for raw_path in sorted(expected_raw_paths):
        labeled_path = _labeled_capture_path(output_root, raw_path)
        record = _inspect_labeled_png(
            labeled_path,
            expected_source_sha256=sha256_file(raw_path),
        )
        labeled_sources.append({"path": str(labeled_path), "sha256": record["sha256"]})

    raw_report_paths = {
        runtime_report_path(pathlib.Path(entry["path"]).resolve())
        for job in (
            *plan["study"]["jobs"],
            *plan["study"]["verification_jobs"],
            *plan["study"]["reproduction_jobs"],
            *plan["motion"]["jobs"],
        )
        for entry in job["capture_plan"]["captures"]
    }
    standalone_reports = [
        _materialize_runtime_report(output_root, report_path, create=False)
        for report_path in sorted(raw_report_paths)
    ]
    published_report_paths = {pathlib.Path(row["path"]).resolve() for row in standalone_reports}
    _validate_exact_file_inventory(
        output_root / "runtime-reports",
        published_report_paths,
        context="standalone runtime-report",
    )

    review_path = output_root / "review.json"
    rankings_path = output_root / "rankings.csv"
    if not review_path.is_file() or review_path.is_symlink():
        raise HarnessError("publication requires a regular review.json")
    if _read_json(review_path, "published review") != review_results:
        raise HarnessError("published review.json differs from recomputed review evidence")
    if not rankings_path.is_file() or rankings_path.is_symlink():
        raise HarnessError("publication requires rankings.csv")
    expected_rankings = _rankings_csv_content(review_results)
    if rankings_path.read_text(encoding="utf-8") != expected_rankings:
        raise HarnessError("published rankings.csv differs from recomputed rankings")
    workbook = _validate_formula_workbook(
        output_root / "review-workbook.xlsx",
        review_results=review_results,
    )
    motion = _validate_motion_deliverables_for_plan(plan)
    expected_output_mp4s = {
        (output_root / clip["mp4"]).resolve() for clip in plan["motion"]["clips"]
    }
    output_mp4_candidates = list(output_root.rglob("*.mp4"))
    if (
        any(path.is_symlink() or not path.is_file() for path in output_mp4_candidates)
        or {path.resolve() for path in output_mp4_candidates} != expected_output_mp4s
    ):
        raise HarnessError("publication-wide MP4 inventory has missing or foreign files")
    expected_motion_source_pngs = {
        _labeled_capture_path(
            output_root,
            pathlib.Path(capture["path"]).resolve(),
        ).resolve()
        for job in plan["motion"]["jobs"]
        for capture in job["capture_plan"]["captures"]
    }
    expected_motion_paired_pngs = {
        (
            output_root
            / clip["paired_frame_directory"]
            / f"frame-{index + 1:04d}.png"
        ).resolve()
        for clip in plan["motion"]["clips"]
        for index in range(MOTION_FRAME_COUNT)
    }
    expected_motion_strip_pngs = {
        (output_root / clip["eight_frame_strip"]).resolve()
        for clip in plan["motion"]["clips"]
    }
    expected_output_pngs = (
        expected_labeled_paths
        | expected_family_paths
        | expected_final_paths
        | expected_motion_source_pngs
        | expected_motion_paired_pngs
        | expected_motion_strip_pngs
    )
    output_png_candidates = list(output_root.rglob("*.png"))
    if any(path.is_symlink() or not path.is_file() for path in output_png_candidates):
        raise HarnessError("publication-wide PNG inventory contains a non-regular file")
    actual_output_pngs = {path.resolve() for path in output_png_candidates}
    if actual_output_pngs != expected_output_pngs:
        raise HarnessError("publication-wide PNG inventory has missing or foreign files")
    all_output_pngs = sorted(expected_output_pngs)
    output_png_records = []
    for png_path in all_output_pngs:
        if png_path.is_symlink():
            raise HarnessError(f"published PNG must be self-contained, not a symlink: {png_path}")
        record = _inspect_labeled_png(png_path, exact_capture_size=False)
        output_png_records.append({"path": str(png_path), "sha256": record["sha256"]})
    return {
        "narrative": narrative_validation,
        "prior_aesthetic_report": prior_evidence,
        "gallery": {
            "html_path": str(gallery_html_path),
            "html_sha256": sha256_file(gallery_html_path),
            "data_path": str(gallery_data_path),
            "data_sha256": sha256_file(gallery_data_path),
            "logical_slots": len(gallery["slots"]),
        },
        "source_pngs": {
            "count": len(labeled_sources),
            "study_and_controls": plan["study"]["slot_accounting"][
                "new_unique_study_renders"
            ],
            "verification": plan["study"]["slot_accounting"][
                "new_unique_control_verification_renders"
            ],
            "reproduction": plan["study"]["slot_accounting"][
                "new_unique_reproduction_renders"
            ],
            "files": labeled_sources,
        },
        "runtime_reports": {
            "count": len(standalone_reports),
            "files": standalone_reports,
            "byte_identical_to_external_raw_sidecars": True,
        },
        "contact_sheets": {"count": len(sheets["family_sheets"]), "files": sheets["family_sheets"]},
        "final_comparison_sheets": {
            "count": len(sheets["final_comparison_sheets"]),
            "files": sheets["final_comparison_sheets"],
        },
        "motion": motion,
        "review": {"path": str(review_path), "sha256": sha256_file(review_path)},
        "rankings": {"path": str(rankings_path), "sha256": sha256_file(rankings_path)},
        "workbook": workbook,
        "all_output_pngs": {"count": len(output_png_records), "files": output_png_records},
        "all_imagery_carries_warning": True,
        "all_imagery_has_visible_warning_overlay": True,
        "unwatermarked_metric_pixels_are_external": True,
    }


def _collect_final_manifest(
    plan_path: pathlib.Path,
    state_path: pathlib.Path,
    lifecycle_path: pathlib.Path,
    review_paths: Sequence[pathlib.Path],
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
    *,
    allow_exact_incomplete_marker: bool = False,
) -> Dict[str, Any]:
    """Recompute the complete immutable evidence ledger used by finalization."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    state_path = state_path.resolve()
    lifecycle_path = lifecycle_path.resolve()
    if plan["status"] != "READY_FOR_CAPTURE":
        raise HarnessError("final manifest requires a fully resolved plan")
    validate_selection(plan["selection"], complete=True)
    capture = validate_capture_artifacts(
        plan_path,
        include_motion=True,
        include_runtime_records=True,
    )
    runtime_reports = capture.pop("runtime_reports")
    runtime_pngs = capture.pop("runtime_pngs")
    capture_state = validate_capture_state(plan_path, state_path, include_motion=True)
    lifecycle = validate_lifecycle_certificate(plan_path, lifecycle_path)
    review_results = validate_final_review_evidence(
        plan_path,
        plan,
        review_paths,
        metric_path,
        performance_path,
    )
    review_documents = [
        validate_reviewer_review(_read_json(path, "reviewer review")) for path in review_paths
    ]
    review_evidence_links = plan["selection"]["review_evidence"]
    still_packet_validation = validate_blinded_review_packet(
        plan_path,
        pathlib.Path(review_evidence_links["review_packet_path"]),
        pathlib.Path(review_evidence_links["unblind_map_path"]),
    )
    motion_links = review_evidence_links["motion_review_evidence"]
    if motion_links is None:
        raise HarnessError("final manifest requires paired-motion review evidence")
    motion_review_paths = [pathlib.Path(path) for path in motion_links["review_paths"]]
    motion_review_documents = [
        validate_reviewer_review(_read_json(path, "motion reviewer review"))
        for path in motion_review_paths
    ]
    motion_packet_validation = validate_blinded_motion_review_packet(
        plan_path,
        pathlib.Path(motion_links["packet_path"]),
        pathlib.Path(motion_links["unblind_map_path"]),
    )
    metric_document = validate_metric_evidence(_read_json(metric_path, "metric evidence"))
    performance_document = validate_selection_performance_evidence(
        _read_json(performance_path, "selection performance evidence")
    )
    sheets = _read_json(output_root / "publication-sheets.json", "publication sheets")
    publication = _validate_publication_files(
        plan_path,
        plan,
        review_results,
        sheets,
        allow_exact_incomplete_marker=allow_exact_incomplete_marker,
    )
    state_document = _read_json(state_path, "capture state")
    lifecycle_document = _read_json(lifecycle_path, "lifecycle certificate")
    return {
        "version": 1,
        "warning": WARNING,
        "status": "FINALIZED",
        "capture_plan": str(plan_path.resolve()),
        "capture_plan_sha256": sha256_file(plan_path),
        "selection": plan["selection"],
        "scoring_contract": plan["scoring"],
        "provenance": plan["provenance"],
        "provenance_hashes": {
            "source_provenance_sha256": sha256_object(plan["provenance"]),
            "camera_manifest_sha256": plan["provenance"]["camera_manifest_sha256"],
            "profile_matrix_sha256": sha256_object(plan["study"]["profile_matrix"]),
            "fresh_control_plan_sha256": sha256_object(
                {
                    "shared_control_references": plan["study"][
                        "shared_control_references"
                    ],
                    "focused_omitted_profile_jobs": list(
                        _baseline_oracle_control_jobs(plan["study"]["jobs"])
                    ),
                    "focused_explicit_current_jobs": plan["study"][
                        "verification_jobs"
                    ],
                }
            ),
            "selection_decisions_sha256": _selection_decisions_sha256(plan["selection"]),
        },
        "fresh_control_evidence": {
            "classification": "current-source-runtime-controls",
            "count": plan["study"]["slot_accounting"]["fresh_shared_control_renders"],
            "source_provenance_sha256": sha256_object(plan["provenance"]),
            "runtime_attestation": True,
            "runtime_report_status": "emitted_valid",
            "focused_process_policy": plan["capture_contract"][
                "focused_control_process_policy"
            ],
            "focused_process_equivalence": capture_state[
                "baseline_oracle_equivalence"
            ],
            "omitted_explicit_current_equivalence": capture[
                "control_equivalence"
            ],
        },
        "baseline_oracle_evidence": {
            "provenance_binding": plan["provenance"]["baseline_oracle"],
            "equivalence": capture_state["baseline_oracle_equivalence"],
            "historical_aesthetic_report_used_as_pixel_oracle": False,
            "stable_pixels_are_exact": True,
            "broad_numeric_threshold_used": False,
        },
        "control_equivalence_raster_evidence": {
            "scope": "omitted-versus-explicit-current-control-raster-equivalence-only",
            "qualification_provenance_binding": plan["provenance"][
                "control_equivalence_raster"
            ],
            "current_source_equivalence": capture["control_equivalence"],
            "baseline_oracle_contract_modified": False,
            "broad_numeric_threshold_used": False,
            "spatial_expansion_used": False,
        },
        "render_accounting": {
            **plan["study"]["slot_accounting"],
            "planned_still_renderer_invocations": (
                len(plan["study"]["jobs"])
                + len(plan["study"]["verification_jobs"])
                + len(plan["study"]["reproduction_jobs"])
            ),
            "planned_motion_renderer_invocations": len(plan["motion"]["jobs"]),
            "actual_renderer_invocations": capture_state["renderer_invocations"],
            "unique_still_content_hashes": capture["unique_still_content_hashes"],
            "unique_all_validated_png_content_hashes": capture[
                "unique_all_validated_png_content_hashes"
            ],
        },
        "capture_validation": capture,
        "runtime_reports": runtime_reports,
        "runtime_pngs": runtime_pngs,
        "capture_state": {
            "path": str(state_path.resolve()),
            "sha256": sha256_file(state_path),
            "validation": capture_state,
            "document": state_document,
        },
        "lifecycle_certificate": {
            "path": str(lifecycle_path.resolve()),
            "sha256": sha256_file(lifecycle_path),
            "validation": lifecycle,
            "document": lifecycle_document,
        },
        "review_evidence": {
            "reviews": [
                {
                    "path": str(path.resolve()),
                    "sha256": sha256_file(path),
                    "document": document,
                }
                for path, document in zip(review_paths, review_documents)
            ],
            "metrics": {
                "path": str(metric_path.resolve()),
                "sha256": sha256_file(metric_path),
                "document": metric_document,
            },
            "selection_performance": {
                "path": str(performance_path.resolve()),
                "sha256": sha256_file(performance_path),
                "document": performance_document,
            },
            "blinded_still_packet": still_packet_validation,
            "blinded_motion_packet": motion_packet_validation,
            "motion_reviews": [
                {
                    "path": str(path.resolve()),
                    "sha256": sha256_file(path),
                    "document": document,
                }
                for path, document in zip(motion_review_paths, motion_review_documents)
            ],
            "auditable_unblinding": {
                "still_map_path": review_evidence_links["unblind_map_path"],
                "still_map_sha256": review_evidence_links["unblind_map_sha256"],
                "motion_map_path": motion_links["unblind_map_path"],
                "motion_map_sha256": motion_links["unblind_map_sha256"],
                "maps_were_external_to_public_packets_during_review": True,
            },
        },
        "review_results": review_results,
        "recommendations": {
            "atomic": review_results["atomic"],
            "combinations": review_results["combinations"],
        },
        "rankings": review_results["rankings"],
        "publication_sheets": sheets,
        "publication": publication,
    }


def finalize_manifest(
    plan_path: pathlib.Path,
    state_path: pathlib.Path,
    lifecycle_path: pathlib.Path,
    review_paths: Sequence[pathlib.Path],
    metric_path: pathlib.Path,
    performance_path: pathlib.Path,
) -> Dict[str, Any]:
    """Publish the deterministic evidence manifest only after every gate passes."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    review_results = validate_final_review_evidence(
        plan_path,
        plan,
        review_paths,
        metric_path,
        performance_path,
    )
    _write_review_derivatives(output_root, review_results)
    build_publication_sheets(plan_path)
    manifest = _collect_final_manifest(
        plan_path,
        state_path,
        lifecycle_path,
        review_paths,
        metric_path,
        performance_path,
        allow_exact_incomplete_marker=True,
    )
    incomplete_readme = output_root / "README-INCOMPLETE.md"
    if incomplete_readme.exists() or incomplete_readme.is_symlink():
        if (
            incomplete_readme.is_symlink()
            or not incomplete_readme.is_file()
            or incomplete_readme.read_text(encoding="utf-8") != _incomplete_readme_content()
        ):
            raise HarnessError("refusing to remove a changed README-INCOMPLETE.md")
        incomplete_readme.unlink()
    manifest_path = output_root / "manifest.json"
    if manifest_path.exists():
        existing = _read_json(manifest_path, "existing manifest")
        if existing.get("status") == "FINALIZED" and existing != manifest:
            raise HarnessError("refusing to overwrite a different finalized manifest")
    atomic_write(manifest_path, pretty_json(manifest))
    return {
        "version": 1,
        "warning": WARNING,
        "status": "FINALIZED",
        "manifest": str(manifest_path),
        "manifest_sha256": sha256_file(manifest_path),
        "runtime_reports": len(manifest["runtime_reports"]),
        "logical_slots": manifest["render_accounting"]["resolved_logical_slots"],
        "new_unique_still_renders": manifest["render_accounting"]["new_unique_still_renders"],
    }


def validate_publication(
    plan_path: pathlib.Path,
    manifest_path: Optional[pathlib.Path] = None,
) -> Dict[str, Any]:
    """Rebuild and compare the final ledger, proving the publication is complete."""

    plan = validate_capture_document(_read_json(plan_path, "capture plan"))
    plan_path, output_root = _require_publication_capture_plan_path(plan_path, plan)
    expected_manifest_path = (output_root / "manifest.json").resolve()
    manifest_path = manifest_path or expected_manifest_path
    if (
        manifest_path.resolve() != expected_manifest_path
        or not manifest_path.is_file()
        or manifest_path.is_symlink()
    ):
        raise HarnessError("publication manifest must be the regular output-root/manifest.json file")
    manifest_path = expected_manifest_path
    manifest = _read_json(manifest_path, "final manifest")
    if not isinstance(manifest, dict) or manifest.get("version") != 1 or manifest.get("warning") != WARNING:
        raise HarnessError("final manifest identity changed")
    if manifest.get("status") != "FINALIZED":
        raise HarnessError("manifest is not finalized")
    capture_state = manifest.get("capture_state")
    lifecycle = manifest.get("lifecycle_certificate")
    review_evidence = manifest.get("review_evidence")
    if not isinstance(capture_state, dict) or not isinstance(lifecycle, dict) or not isinstance(review_evidence, dict):
        raise HarnessError("final manifest is missing linked evidence")
    reviews = review_evidence.get("reviews")
    metrics = review_evidence.get("metrics")
    performance = review_evidence.get("selection_performance")
    if not isinstance(reviews, list) or len(reviews) != 2 or not isinstance(metrics, dict) or not isinstance(performance, dict):
        raise HarnessError("final manifest review evidence links are malformed")
    try:
        expected = _collect_final_manifest(
            plan_path,
            pathlib.Path(capture_state["path"]),
            pathlib.Path(lifecycle["path"]),
            [pathlib.Path(row["path"]) for row in reviews],
            pathlib.Path(metrics["path"]),
            pathlib.Path(performance["path"]),
        )
    except (KeyError, TypeError) as error:
        raise HarnessError("final manifest evidence paths are malformed") from error
    if manifest != expected:
        raise HarnessError("final manifest differs from regenerated capture/review/publication evidence")
    return {
        "version": 1,
        "warning": WARNING,
        "status": "VALIDATED_FOR_PUBLICATION",
        "manifest": str(manifest_path),
        "manifest_sha256": sha256_file(manifest_path),
        "logical_slots": manifest["render_accounting"]["resolved_logical_slots"],
        "new_unique_still_renders": manifest["render_accounting"]["new_unique_still_renders"],
        "runtime_reports": len(manifest["runtime_reports"]),
        "motion_clips": manifest["publication"]["motion"]["clips"],
        "cleanup_cycles": manifest["lifecycle_certificate"]["validation"]["cycles_completed"],
    }


def verify_reproduction(
    first_png: pathlib.Path,
    second_png: pathlib.Path,
    *,
    camera_id: str = "02-highlands-oblique",
) -> Dict[str, Any]:
    """Require stable pixels and deterministic plan/mesh/world state across reruns."""

    first_details = inspect_png(first_png)
    second_details = inspect_png(second_png)
    first_rgb_sha256 = decoded_rgb_sha256(first_png)
    second_rgb_sha256 = decoded_rgb_sha256(second_png)
    if camera_id not in BASELINE_ORACLE_CAMERA_IDS:
        raise HarnessError(f"reproduction camera lacks clean-source stability evidence: {camera_id}")
    _, oracle_manifest = _baseline_oracle_documents()
    pixel_equivalence = _compare_raster_stable_rgb(
        first_png,
        second_png,
        camera_id=camera_id,
        ambiguous_pixels=oracle_manifest["cameras"][camera_id]["ambiguous_pixels"],
        context="deterministic reproduction",
        require_clean_observed_ambiguous_values=False,
    )
    first_wrapper = _exact_keys(
        _read_json(runtime_report_path(first_png), "first reproduction report"),
        ("version", "warning", "capture", "report"),
        "first reproduction wrapper",
    )
    second_wrapper = _exact_keys(
        _read_json(runtime_report_path(second_png), "second reproduction report"),
        ("version", "warning", "capture", "report"),
        "second reproduction wrapper",
    )
    if any(
        wrapper["version"] != 1 or wrapper["warning"] != WARNING
        for wrapper in (first_wrapper, second_wrapper)
    ):
        raise HarnessError("reproduction wrapper identity changed")
    first_capture = dict(first_wrapper.get("capture", {}))
    second_capture = dict(second_wrapper.get("capture", {}))
    first_capture.pop("path", None)
    second_capture.pop("path", None)
    stable_report_fields = (
        "version",
        "profile_hash_sha256",
        "authority",
        "counts",
        "anchor_heights",
        "anchor_classes",
        "projection_hashes",
        "effect_validation",
        "camera_features",
    )
    all_report_fields = (
        "version",
        "profile_hash_sha256",
        "runtime_receipt",
        "authority",
        "counts",
        "anchor_heights",
        "anchor_classes",
        "projection_hashes",
        "effect_validation",
        "camera_features",
        "performance",
        "cleanup",
    )
    first_report = _exact_keys(first_wrapper["report"], all_report_fields, "first reproduction payload")
    second_report = _exact_keys(second_wrapper["report"], all_report_fields, "second reproduction payload")
    first_stable = {field: first_report[field] for field in stable_report_fields}
    second_stable = {field: second_report[field] for field in stable_report_fields}
    if first_capture != second_capture or first_stable != second_stable:
        raise HarnessError("reproduction camera/profile/authority/plan/mesh state differs")
    return {
        "version": 1,
        "warning": WARNING,
        "still_sha256": first_details["sha256"],
        "decoded_rgb_sha256": first_rgb_sha256,
        "source_still_sha256": first_details["sha256"],
        "reproduction_still_sha256": second_details["sha256"],
        "source_decoded_rgb_sha256": first_rgb_sha256,
        "reproduction_decoded_rgb_sha256": second_rgb_sha256,
        "raw_png_byte_identical": first_details["sha256"] == second_details["sha256"],
        "raw_decoded_rgb_identical": first_rgb_sha256 == second_rgb_sha256,
        "raster_stable_pixel_identical": pixel_equivalence["stable_pixel_identical"],
        "stable_pixel_count": pixel_equivalence["stable_pixel_count"],
        "ambiguous_pixel_count": pixel_equivalence["ambiguous_pixel_count"],
        "differing_ambiguous_pixel_count": pixel_equivalence[
            "differing_ambiguous_pixel_count"
        ],
        "ambiguous_value_policy": pixel_equivalence["ambiguous_value_policy"],
        "ambiguous_values": pixel_equivalence["ambiguous_values"],
        "broad_numeric_threshold_used": False,
        "report_state_sha256": sha256_object(first_stable),
        "projection_hashes": first_wrapper["report"]["projection_hashes"],
        "effect_validation": first_wrapper["report"]["effect_validation"],
        "source_runtime_receipt_sha256": first_report["runtime_receipt"]["receipt_sha256"],
        "reproduction_runtime_receipt_sha256": second_report["runtime_receipt"]["receipt_sha256"],
        "performance_and_post_teardown_cleanup_excluded_from_determinism_hash": True,
        "runtime_receipt_nonce_process_and_capture_path_excluded_from_determinism_hash": True,
    }


def self_check() -> Dict[str, Any]:
    """Exercise static matrix, camera, slot, motion, and metric invariants."""

    control_equivalence_qualification = (
        _validate_control_equivalence_qualification_pack()
    )
    profiles = atomic_profiles()
    if len(profiles) != 60 or len({profile.id for profile in profiles}) != 60:
        raise HarnessError("profile matrix is not exactly 60 unique treatments")
    actual_counts = {family: sum(profile.family == family for profile in profiles) for family in FAMILY_ORDER}
    if actual_counts != FAMILY_COUNTS:
        raise HarnessError(f"profile family counts changed: {actual_counts}")
    for profile in profiles:
        if compact_json(json.loads(profile.canonical_json)) != profile.canonical_json:
            raise HarnessError(f"profile is not canonical compact JSON: {profile.id}")
        if sha256_bytes(profile.canonical_json.encode()) != profile.sha256:
            raise HarnessError(f"profile hash changed: {profile.id}")
    control = control_profile()
    established_camera_hashes = {
        relative: sha256_file(REPOSITORY_ROOT / relative)
        for relative in ESTABLISHED_CAMERA_SOURCE_SHA256
    }
    if established_camera_hashes != ESTABLISHED_CAMERA_SOURCE_SHA256:
        raise HarnessError("established camera source hashes changed")
    final_cameras, focused_cameras, _ = load_camera_sets()
    partial = build_still_plan(pathlib.Path("/tmp/crystal-ascent-world-detail-self-check"))
    if partial["slot_accounting"]["resolved_logical_slots"] != 240:
        raise HarnessError("unreviewed plan must resolve only the 60-by-4 neutral screen")
    if partial["slot_accounting"]["materialized_unique_paths"] != 244:
        raise HarnessError("unreviewed plan must add four shared control references")
    partial_oracle_jobs = _baseline_oracle_control_jobs(partial["jobs"])
    partial_verification_jobs = partial["verification_jobs"]
    if (
        len(partial_oracle_jobs) != 4
        or len(partial_verification_jobs) != 4
        or any(
            len(job["cameras"]) != 1 or len(job["artifacts"]) != 1
            for job in (*partial_oracle_jobs, *partial_verification_jobs)
        )
        or tuple(job["cameras"][0]["id"] for job in partial_oracle_jobs)
        != BASELINE_ORACLE_CAMERA_IDS
        or tuple(job["cameras"][0]["id"] for job in partial_verification_jobs)
        != BASELINE_ORACLE_CAMERA_IDS
        or len(partial["jobs"]) + len(partial_verification_jobs) != 68
    ):
        raise HarnessError(
            "unreviewed plan must use four omitted and four explicit fresh camera processes"
        )

    selection = selection_template()
    selection["status"] = "COMPLETE_FOR_STATIC_SELF_CHECK"
    selection["review_sources"] = ["synthetic-schema-check-a", "synthetic-schema-check-b"]
    for family in FAMILY_ORDER:
        family_profiles = [profile for profile in profiles if profile.family == family]
        selection["promoted"][family] = [family_profiles[0].id, family_profiles[1].id]
        selection["stress_diagnostics"][family] = [
            family_profiles[0].id,
            family_profiles[1].id,
        ]
        selection["ladder_inputs"][family] = family_profiles[0].id
        selection["atomic_winners"][family] = family_profiles[0].id
        selection["pre_motion_atomic_winners"][family] = family_profiles[0].id
        for combination_id in COMBINATION_IDS:
            selection["combinations"][combination_id][family] = family_profiles[0].id
            selection["pre_motion_combinations"][combination_id][family] = family_profiles[0].id
    selection["interaction_findings"] = [
        {
            "step_id": step_id,
            "predecessor": predecessor,
            "introduced_families": list(introduced),
            "weighted_score": 80.0,
            "predecessor_weighted_score": 80.0,
            "weighted_delta": 0.0,
            "minimum_readability": 4,
            "minimum_edge_quietness": 4,
            "passed": True,
            "vetoed_families": [],
        }
        for step_id, predecessor, introduced in LADDER_STEPS
    ]
    selection["review_evidence"] = {
        "reviewer_ids": ["synthetic-review-a", "synthetic-review-b"],
        "review_sha256": ["0" * 64, "1" * 64],
        "metrics_sha256": "2" * 64,
        "performance_sha256": "3" * 64,
        "review_packet_path": "/tmp/synthetic-review-packet.json",
        "review_packet_sha256": "4" * 64,
        "unblind_map_path": "/tmp/synthetic-unblind-map.json",
        "unblind_map_sha256": "5" * 64,
        "motion_review_evidence": None,
        "scoring_contract_sha256": sha256_object(scoring_contract()),
        "decisions_sha256": _selection_decisions_sha256(selection),
    }
    complete = build_still_plan(pathlib.Path("/tmp/crystal-ascent-world-detail-self-check"), selection)
    motion = build_motion_plan(pathlib.Path("/tmp/crystal-ascent-world-detail-self-check"), selection)
    if complete["slot_accounting"]["resolved_logical_slots"] != EXPECTED_LOGICAL_SLOTS:
        raise HarnessError("complete plan does not resolve 665 logical slots")
    if (
        complete["slot_accounting"]["unique_non_control_treatment_pngs"]
        > MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS
        or complete["slot_accounting"]["total_accounted_evidence_pngs"]
        > MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS
    ):
        raise HarnessError("complete plan exceeds a still-PNG accounting ceiling")
    if (
        complete["slot_accounting"]["new_unique_reproduction_renders"]
        != EXPECTED_REPRODUCTION_RENDERS
        or len(complete["reproduction_jobs"]) != EXPECTED_REPRODUCTION_RENDERS
    ):
        raise HarnessError("complete plan lost its fresh-process deterministic reproduction")
    if (
        len(motion["clips"]) != 22
        or len(motion["jobs"]) != 28
        or motion["total_sequence_launches"] != 28
        or motion["total_frame_captures"] != 28 * MOTION_FRAME_COUNT
    ):
        raise HarnessError("complete motion matrix changed")
    maximally_distinct = copy.deepcopy(selection)
    for index, family in enumerate(FAMILY_ORDER):
        family_profiles = [profile for profile in profiles if profile.family == family]
        first = family_profiles[0].id
        last = family_profiles[-1].id
        maximally_distinct["promoted"][family] = [first, last]
        maximally_distinct["stress_diagnostics"][family] = [first, last]
        maximally_distinct["ladder_inputs"][family] = first
        maximally_distinct["atomic_winners"][family] = first
        maximally_distinct["pre_motion_atomic_winners"][family] = first
        maximally_distinct["combinations"]["score-leader"][family] = first
        maximally_distinct["combinations"]["restrained"][family] = last
        maximally_distinct["combinations"]["expressive"][family] = (
            first if index % 2 == 0 else last
        )
        maximally_distinct["pre_motion_combinations"]["score-leader"][family] = first
        maximally_distinct["pre_motion_combinations"]["restrained"][family] = last
        maximally_distinct["pre_motion_combinations"]["expressive"][family] = (
            first if index % 2 == 0 else last
        )
    maximally_distinct["review_evidence"]["decisions_sha256"] = (
        _selection_decisions_sha256(maximally_distinct)
    )
    maximally_distinct_plan = build_still_plan(
        pathlib.Path("/tmp/crystal-ascent-world-detail-self-check"),
        maximally_distinct,
    )
    maximum_accounting = maximally_distinct_plan["slot_accounting"]
    if (
        maximum_accounting["resolved_logical_slots"] != EXPECTED_LOGICAL_SLOTS
        or maximum_accounting["unique_non_control_treatment_pngs"] != 596
        or maximum_accounting["total_accounted_evidence_pngs"] != 630
    ):
        raise HarnessError("maximally distinct treatment/total PNG accounting changed")
    try:
        _validate_still_png_ceilings(
            MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS + 1,
            MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS,
        )
    except HarnessError as error:
        if "611 unique non-control treatment-PNG ceiling" not in str(error):
            raise HarnessError("over-cap treatment accounting reported the wrong gate") from error
    else:
        raise HarnessError("over-cap treatment accounting unexpectedly passed")
    if any(condition.id == "midnight" for condition in LIGHTING_CONDITIONS.values()):
        raise HarnessError("midnight entered the study")
    return {
        "version": 1,
        "warning": WARNING,
        "control_profile_sha256": control.sha256,
        "atomic_profiles": len(profiles),
        "family_counts": actual_counts,
        "final_cameras": len(final_cameras),
        "focused_cameras": len(focused_cameras),
        "established_camera_source_sha256": established_camera_hashes,
        "logical_slots": complete["slot_accounting"]["resolved_logical_slots"],
        "materialized_still_paths_for_schema_fixture": complete["slot_accounting"]["materialized_unique_paths"],
        "treatment_renders_for_schema_fixture": complete["slot_accounting"][
            "unique_non_control_treatment_pngs"
        ],
        "new_unique_still_renders": complete["slot_accounting"]["new_unique_still_renders"],
        "total_accounted_evidence_pngs": complete["slot_accounting"][
            "total_accounted_evidence_pngs"
        ],
        "baseline_oracle_stability_diagnostic_renders": (
            EXPECTED_BASELINE_ORACLE_STABILITY_DIAGNOSTIC_RENDERS
        ),
        "control_equivalence_qualification": {
            "contract_sha256": control_equivalence_qualification[
                "contract_sha256"
            ],
            "manifest_sha256": control_equivalence_qualification[
                "manifest_sha256"
            ],
            "pair_count": control_equivalence_qualification["pair_count"],
            "run_count": control_equivalence_qualification["run_count"],
            "qualified_pixel_count": len(
                control_equivalence_qualification["qualified_pixels"]
            ),
            "exact_inventory_verified": control_equivalence_qualification[
                "exact_inventory_verified"
            ],
            "baseline_oracle_contract_unchanged": (
                control_equivalence_qualification[
                    "baseline_oracle_contract_unchanged"
                ]
            ),
        },
        "focused_control_fresh_processes": 8,
        "partial_still_renderer_invocations": (
            len(partial["jobs"]) + len(partial["verification_jobs"])
        ),
        "maximally_distinct_treatment_pngs": maximum_accounting[
            "unique_non_control_treatment_pngs"
        ],
        "maximally_distinct_total_accounted_evidence_pngs": maximum_accounting[
            "total_accounted_evidence_pngs"
        ],
        "treatment_ceiling_rejection_verified": True,
        "deterministic_reproduction_jobs": len(complete["reproduction_jobs"]),
        "motion_clips": len(motion["clips"]),
        "motion_sequence_launches": len(motion["jobs"]),
        "motion_frame_captures": motion["total_frame_captures"],
    }


def _load_optional_selection(path: Optional[pathlib.Path]) -> Dict[str, Any]:
    if path is None:
        return selection_template()
    return validate_selection(_read_json(path, "adaptive selection"))


def _emit_json(value: Any, destination: Optional[pathlib.Path] = None) -> None:
    content = pretty_json(value)
    if destination is None:
        sys.stdout.write(content)
    else:
        atomic_write(destination, content)


def _argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)

    matrix_parser = subparsers.add_parser("matrix", help="print the canonical 60-profile matrix")
    matrix_parser.add_argument("--output", type=pathlib.Path)
    template_parser = subparsers.add_parser("selection-template", help="print a blank adaptive-selection file")
    template_parser.add_argument("--output", type=pathlib.Path)

    plan_parser = subparsers.add_parser("plan", help="write a deterministic still/motion capture document")
    plan_parser.add_argument("--root", type=pathlib.Path, default=DEFAULT_OUTPUT_ROOT)
    plan_parser.add_argument("--raw-capture-root", type=pathlib.Path)
    plan_parser.add_argument("--selection", type=pathlib.Path)
    plan_parser.add_argument("--output", type=pathlib.Path, required=True)

    scaffold_parser = subparsers.add_parser("scaffold", help="create report/provenance/gallery scaffolding")
    scaffold_parser.add_argument("--root", type=pathlib.Path, default=DEFAULT_OUTPUT_ROOT)
    scaffold_parser.add_argument("--raw-capture-root", type=pathlib.Path)
    scaffold_parser.add_argument("--selection", type=pathlib.Path)

    run_parser = subparsers.add_parser("run", help="run or resume genuine Cargo capture jobs")
    run_parser.add_argument("--plan", type=pathlib.Path, required=True)
    run_parser.add_argument("--work-root", type=pathlib.Path, default=DEFAULT_WORK_ROOT)
    run_parser.add_argument("--max-jobs", type=int)
    run_parser.add_argument("--timeout-seconds", type=int, default=1800)
    run_parser.add_argument("--include-motion", action="store_true")

    validate_parser = subparsers.add_parser("validate", help="strictly validate captured PNGs and reports")
    validate_parser.add_argument("--plan", type=pathlib.Path, required=True)
    validate_parser.add_argument("--include-motion", action="store_true")

    control_parser = subparsers.add_parser(
        "validate-control",
        help="prove fresh omitted-profile controls match four explicit-current renders",
    )
    control_parser.add_argument("--plan", type=pathlib.Path, required=True)

    state_parser = subparsers.add_parser(
        "validate-state",
        help="validate the completed resumable renderer ledger",
    )
    state_parser.add_argument("--plan", type=pathlib.Path, required=True)
    state_parser.add_argument("--state", type=pathlib.Path, required=True)
    state_parser.add_argument(
        "--stills-only",
        action="store_true",
        help="validate only still/control jobs for a deliberately partial run",
    )

    lifecycle_parser = subparsers.add_parser(
        "validate-lifecycle",
        help="validate the runtime-written hash-chained 100-cycle teardown certificate",
    )
    lifecycle_parser.add_argument("--plan", type=pathlib.Path, required=True)
    lifecycle_parser.add_argument("--certificate", type=pathlib.Path, required=True)

    run_lifecycle_parser = subparsers.add_parser(
        "run-lifecycle",
        help="launch one genuine process for 100 projection/teardown cycles and validate its certificate",
    )
    run_lifecycle_parser.add_argument("--plan", type=pathlib.Path, required=True)
    run_lifecycle_parser.add_argument("--certificate", type=pathlib.Path, required=True)
    run_lifecycle_parser.add_argument("--work-root", type=pathlib.Path, default=DEFAULT_WORK_ROOT)
    run_lifecycle_parser.add_argument("--timeout-seconds", type=int, default=14_400)

    motion_parser = subparsers.add_parser("finalize-motion", help="encode one captured motion clip and strip")
    motion_parser.add_argument("--plan", type=pathlib.Path, required=True)
    motion_parser.add_argument("--clip", required=True)

    validate_motion_parser = subparsers.add_parser(
        "validate-motion",
        help="require all 22 finalized paired clips, strips, frames, and provenance states",
    )
    validate_motion_parser.add_argument("--plan", type=pathlib.Path, required=True)

    metrics_parser = subparsers.add_parser("metrics", help="compute SSIM and mean DeltaE00")
    metrics_parser.add_argument("control", type=pathlib.Path)
    metrics_parser.add_argument("candidate", type=pathlib.Path)

    collect_metrics_parser = subparsers.add_parser(
        "collect-metrics",
        help="compute the strict 60-entry named-camera SSIM/DeltaE evidence file",
    )
    collect_metrics_parser.add_argument("--plan", type=pathlib.Path, required=True)
    collect_metrics_parser.add_argument("--output", type=pathlib.Path, required=True)

    collect_performance_parser = subparsers.add_parser(
        "collect-performance",
        help="collect atomic/control p95 and resident-memory tie-break evidence",
    )
    collect_performance_parser.add_argument("--plan", type=pathlib.Path, required=True)
    collect_performance_parser.add_argument("--output", type=pathlib.Path, required=True)

    packet_parser = subparsers.add_parser(
        "build-review-packet",
        help="materialize an opaque-code image packet and separate private unblind map",
    )
    packet_parser.add_argument("--plan", type=pathlib.Path, required=True)
    packet_parser.add_argument("--packet-root", type=pathlib.Path, required=True)
    packet_parser.add_argument("--unblind-map", type=pathlib.Path, required=True)

    motion_packet_parser = subparsers.add_parser(
        "build-motion-review-packet",
        help="materialize 22 anonymous paired strips/clips and a private unblind map",
    )
    motion_packet_parser.add_argument("--plan", type=pathlib.Path, required=True)
    motion_packet_parser.add_argument("--packet-root", type=pathlib.Path, required=True)
    motion_packet_parser.add_argument("--unblind-map", type=pathlib.Path, required=True)

    review_template_parser = subparsers.add_parser(
        "review-template",
        help="write an ID-free rating form for a public blinded packet",
    )
    review_template_parser.add_argument("--packet", type=pathlib.Path, required=True)
    review_template_parser.add_argument("--reviewer-id", required=True)
    review_template_parser.add_argument("--output", type=pathlib.Path, required=True)

    derive_parser = subparsers.add_parser(
        "derive-selection",
        help="derive promotions or final family decisions from exactly two blinded reviews",
    )
    derive_parser.add_argument("--plan", type=pathlib.Path, required=True)
    derive_parser.add_argument("--packet", type=pathlib.Path, required=True)
    derive_parser.add_argument("--unblind-map", type=pathlib.Path, required=True)
    derive_parser.add_argument("--motion-review", type=pathlib.Path, action="append")
    derive_parser.add_argument("--motion-packet", type=pathlib.Path)
    derive_parser.add_argument("--motion-unblind-map", type=pathlib.Path)
    derive_parser.add_argument("--review", type=pathlib.Path, action="append", required=True)
    derive_parser.add_argument("--metrics", type=pathlib.Path, required=True)
    derive_parser.add_argument("--performance", type=pathlib.Path, required=True)
    derive_parser.add_argument("--output", type=pathlib.Path, required=True)
    derive_parser.add_argument("--audit-output", type=pathlib.Path)

    derivatives_parser = subparsers.add_parser(
        "build-review-derivatives",
        help="write final review.json and rankings.csv before report/workbook authoring",
    )
    derivatives_parser.add_argument("--plan", type=pathlib.Path, required=True)
    derivatives_parser.add_argument("--review", type=pathlib.Path, action="append", required=True)
    derivatives_parser.add_argument("--metrics", type=pathlib.Path, required=True)
    derivatives_parser.add_argument("--performance", type=pathlib.Path, required=True)

    sheets_parser = subparsers.add_parser(
        "build-sheets",
        help="build family and final comparison sheets from genuine labeled renders",
    )
    sheets_parser.add_argument("--plan", type=pathlib.Path, required=True)

    narrative_parser = subparsers.add_parser(
        "install-narrative",
        help="validate and install a substantive authored README.md",
    )
    narrative_parser.add_argument("--input", type=pathlib.Path, required=True)
    narrative_parser.add_argument("--root", type=pathlib.Path, default=DEFAULT_OUTPUT_ROOT)

    workbook_parser = subparsers.add_parser(
        "install-workbook",
        help="validate and install a formula workbook bound to review.json",
    )
    workbook_parser.add_argument("--input", type=pathlib.Path, required=True)
    workbook_parser.add_argument("--root", type=pathlib.Path, default=DEFAULT_OUTPUT_ROOT)
    workbook_parser.add_argument("--review-results", type=pathlib.Path, required=True)

    finalize_parser = subparsers.add_parser(
        "finalize-manifest",
        help="validate and fold all evidence into the final report manifest",
    )
    finalize_parser.add_argument("--plan", type=pathlib.Path, required=True)
    finalize_parser.add_argument("--state", type=pathlib.Path, required=True)
    finalize_parser.add_argument("--lifecycle", type=pathlib.Path, required=True)
    finalize_parser.add_argument("--review", type=pathlib.Path, action="append", required=True)
    finalize_parser.add_argument("--metrics", type=pathlib.Path, required=True)
    finalize_parser.add_argument("--performance", type=pathlib.Path, required=True)

    publication_parser = subparsers.add_parser(
        "validate-publication",
        help="recompute the finalized evidence ledger and validate every publication artifact",
    )
    publication_parser.add_argument("--plan", type=pathlib.Path, required=True)
    publication_parser.add_argument("--manifest", type=pathlib.Path)

    reproduce_parser = subparsers.add_parser("verify-reproduction", help="compare repeat still/report hashes")
    reproduce_parser.add_argument("first", type=pathlib.Path)
    reproduce_parser.add_argument("second", type=pathlib.Path)
    subparsers.add_parser("self-check", help="run static harness invariants")
    return parser


def main(argv: Optional[Sequence[str]] = None) -> int:
    args = _argument_parser().parse_args(argv)
    try:
        if args.command == "matrix":
            _emit_json(
                {
                    "version": 1,
                    "warning": WARNING,
                    "control": dataclasses.asdict(control_profile()),
                    "profiles": [dataclasses.asdict(profile) for profile in atomic_profiles()],
                },
                args.output,
            )
        elif args.command == "selection-template":
            _emit_json(selection_template(), args.output)
        elif args.command == "plan":
            selection = _load_optional_selection(args.selection)
            _emit_json(
                build_capture_document(
                    args.root,
                    selection,
                    raw_capture_root=args.raw_capture_root,
                ),
                args.output,
            )
        elif args.command == "scaffold":
            selection_path = args.selection
            if selection_path is None:
                existing_selection = args.root / "provenance" / "selection.json"
                if existing_selection.exists():
                    selection_path = existing_selection
            selection = _load_optional_selection(selection_path)
            _emit_json(
                scaffold_report(
                    args.root,
                    selection,
                    raw_capture_root=args.raw_capture_root,
                )
            )
        elif args.command == "run":
            if args.timeout_seconds <= 0:
                raise HarnessError("timeout-seconds must be positive")
            _emit_json(
                run_capture_plan(
                    args.plan,
                    work_root=args.work_root,
                    max_jobs=args.max_jobs,
                    timeout_seconds=args.timeout_seconds,
                    include_motion=args.include_motion,
                )
            )
        elif args.command == "validate":
            _emit_json(
                validate_capture_artifacts(
                    args.plan,
                    include_motion=args.include_motion,
                )
            )
        elif args.command == "validate-control":
            _emit_json(validate_control_equivalence(args.plan))
        elif args.command == "validate-state":
            _emit_json(
                validate_capture_state(
                    args.plan,
                    args.state,
                    include_motion=not args.stills_only,
                )
            )
        elif args.command == "validate-lifecycle":
            _emit_json(validate_lifecycle_certificate(args.plan, args.certificate))
        elif args.command == "run-lifecycle":
            _emit_json(
                run_lifecycle(
                    args.plan,
                    args.certificate,
                    work_root=args.work_root,
                    timeout_seconds=args.timeout_seconds,
                )
            )
        elif args.command == "finalize-motion":
            _emit_json(finalize_motion_clip(args.plan, args.clip))
        elif args.command == "validate-motion":
            _emit_json(validate_motion_deliverables(args.plan))
        elif args.command == "metrics":
            _emit_json(image_metrics(args.control, args.candidate))
        elif args.command == "collect-metrics":
            _emit_json(build_metric_evidence(args.plan), args.output)
        elif args.command == "collect-performance":
            _emit_json(build_selection_performance_evidence(args.plan), args.output)
        elif args.command == "build-review-packet":
            _emit_json(
                build_blinded_review_packet(
                    args.plan,
                    args.packet_root,
                    args.unblind_map,
                )
            )
        elif args.command == "build-motion-review-packet":
            _emit_json(
                build_blinded_motion_review_packet(
                    args.plan,
                    args.packet_root,
                    args.unblind_map,
                )
            )
        elif args.command == "review-template":
            _emit_json(
                blinded_review_template(args.packet, args.reviewer_id),
                args.output,
            )
        elif args.command == "derive-selection":
            if len(args.review) != 2:
                raise HarnessError("derive-selection requires exactly two --review paths")
            derived = derive_selection_from_reviews(
                args.plan,
                args.review,
                args.metrics,
                args.performance,
                args.packet,
                args.unblind_map,
                args.motion_review,
                args.motion_packet,
                args.motion_unblind_map,
            )
            _emit_json(derived["selection"], args.output)
            if args.audit_output is not None:
                _emit_json(derived, args.audit_output)
        elif args.command == "build-review-derivatives":
            _emit_json(
                build_review_derivatives(
                    args.plan,
                    args.review,
                    args.metrics,
                    args.performance,
                )
            )
        elif args.command == "build-sheets":
            _emit_json(build_publication_sheets(args.plan))
        elif args.command == "install-narrative":
            _emit_json(install_narrative(args.input, args.root))
        elif args.command == "install-workbook":
            _emit_json(
                install_workbook(args.input, args.root, args.review_results)
            )
        elif args.command == "finalize-manifest":
            if len(args.review) != 2:
                raise HarnessError("finalize-manifest requires exactly two --review paths")
            _emit_json(
                finalize_manifest(
                    args.plan,
                    args.state,
                    args.lifecycle,
                    args.review,
                    args.metrics,
                    args.performance,
                )
            )
        elif args.command == "validate-publication":
            _emit_json(validate_publication(args.plan, args.manifest))
        elif args.command == "verify-reproduction":
            _emit_json(verify_reproduction(args.first, args.second))
        elif args.command == "self-check":
            _emit_json(self_check())
        else:  # pragma: no cover - argparse enforces a known command
            raise HarnessError(f"unsupported command {args.command}")
    except HarnessError as error:
        print(f"world-detail harness error: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
