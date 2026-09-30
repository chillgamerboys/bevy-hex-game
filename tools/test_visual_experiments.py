"""Focused tests for the fail-closed Grand V3 visual experiment harness."""

from __future__ import annotations

import importlib.util
import dataclasses
import json
import os
import pathlib
import re
import stat
import struct
import sys
import tempfile
import unittest
import zlib
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "tools" / "visual_experiments.py"
SPEC = importlib.util.spec_from_file_location("visual_experiments", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load visual_experiments")
visual_experiments = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = visual_experiments
SPEC.loader.exec_module(visual_experiments)


class VisualExperimentTests(unittest.TestCase):
    """Profiles, staging, provenance, and publication fail closed."""

    def make_fixture(self, directory: str):
        fixture = pathlib.Path(directory)
        registry_source = ROOT / "tools" / "visual_experiments" / "profiles.json"
        registry_raw = json.loads(registry_source.read_text(encoding="utf-8"))
        paths = {
            registry_raw["baseline"][field]
            for field in (
                "world",
                "palette",
                "scenarios",
                "default_lighting",
                "overcast_lighting",
            )
        }
        paths.update(
            profile["palette"]
            for profile in registry_raw["profiles"]
            if "palette" in profile
        )
        paths.update(
            profile["lighting_candidate"]
            for profile in registry_raw["profiles"]
            if "lighting_candidate" in profile
        )
        paths.add(visual_experiments.INDOOR_CRYSTAL_SPEC_RELATIVE)
        registry_path = fixture / "tools" / "visual_experiments" / "profiles.json"
        registry_path.parent.mkdir(parents=True)
        registry_path.write_text(
            json.dumps(registry_raw, indent=2) + "\n", encoding="utf-8"
        )
        for relative in paths:
            source = ROOT / relative
            destination = fixture / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(source.read_bytes())
        return fixture, registry_path, registry_raw

    def test_checked_in_matrix_is_exact_and_valid(self) -> None:
        registry = visual_experiments.load_registry()
        self.assertEqual(
            tuple(profile.id for profile in registry.profiles),
            visual_experiments.EXPECTED_PROFILE_IDS,
        )
        self.assertEqual(len(registry.profiles), 24)
        self.assertEqual(registry.profile("e00-baseline").fog_mode, "current")
        self.assertEqual(
            registry.profile("e00-baseline").material_treatment, "current"
        )
        self.assertEqual(registry.profile("e00-baseline").edge_treatment, "current")
        self.assertEqual(
            tuple(
                profile.fog_mode
                for profile in registry.profiles
                if profile.axis == "visibility"
            ),
            visual_experiments.VISIBILITY_CANDIDATE_MODES,
        )
        self.assertEqual(
            tuple(
                profile.material_treatment
                for profile in registry.profiles
                if profile.axis == "materials"
            ),
            visual_experiments.MATERIAL_CANDIDATE_TREATMENTS,
        )
        self.assertEqual(
            tuple(
                profile.edge_treatment
                for profile in registry.profiles
                if profile.axis == "edges"
            ),
            visual_experiments.EDGE_CANDIDATE_TREATMENTS,
        )
        self.assertEqual(
            tuple(
                profile.crystal_light_profile
                for profile in registry.profiles
                if profile.axis == "indoor-lighting"
            ),
            visual_experiments.EXPECTED_INDOOR_CRYSTAL_IDS,
        )
        self.assertEqual(len(registry.captures), 8)
        self.assertEqual(
            tuple(capture.id for capture in registry.captures),
            (
                "01-world-topdown",
                "02-highlands-oblique",
                "03-coast-river-outlet",
                "04-garden-island-oblique",
                "05-treeline-character",
                "06-waterfall-character",
                "07-tunnel-first-person",
                "08-crystal-bottom-chamber",
            ),
        )
        self.assertEqual(
            tuple(capture.id for capture in registry.captures_for("screen")),
            tuple(capture.id for capture in registry.captures),
        )
        self.assertEqual(
            tuple(capture.id for capture in registry.captures_for("smoke")),
            (
                "01-world-topdown",
                "02-highlands-oblique",
                "03-coast-river-outlet",
                "04-garden-island-oblique",
            ),
        )
        self.assertEqual(
            {capture.camera for capture in registry.captures},
            {"map", "character", "first-person"},
        )
        coast = registry.captures[2]
        self.assertEqual(coast.camera, "map")
        self.assertEqual(coast.look_at_anchor, "grand_v3.coast")
        self.assertEqual(coast.look_at_offset, (50.0, 30.0, 55.0))
        self.assertIsNone(coast.cutaway)
        self.assertIsNone(coast.illumination_overlay)
        chamber = registry.captures[-1]
        self.assertEqual(chamber.camera, "character")
        self.assertEqual(chamber.focus_anchor, "crystal_ascent.bottom_chamber")
        self.assertIsNone(chamber.look_at_anchor)
        self.assertIsNone(chamber.cutaway)
        self.assertIsNone(chamber.illumination_overlay)

    def test_registry_rejects_unknown_fields_and_path_traversal(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            raw["surprise"] = True
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "unknown fields"
            ):
                visual_experiments.load_registry(path, root)

            del raw["surprise"]
            palette_profile = next(
                profile for profile in raw["profiles"] if profile["id"] == "p01-muted-earth"
            )
            palette_profile["palette"] = (
                "tools/visual_experiments/palettes/../p01-muted-earth.json"
            )
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "canonical relative path"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_mixed_axes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            raw["profiles"][1]["level_height"] = 0.3
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "mixes experiment axes"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_requires_explicit_markers_for_promoted_baseline_aliases(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            palette = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "p02-high-separation"
            )
            palette.pop("baseline_alias")
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "must declare baseline_alias"
            ):
                visual_experiments.load_registry(path, root)

            palette["baseline_alias"] = True
            haze = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "z01-haze-light"
            )
            haze["baseline_alias"] = False
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "must declare baseline_alias"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_invalid_or_mixed_visibility_modes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            visibility = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "v01-fog-none"
            )
            visibility["fog_mode"] = "mysterious"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "fog_mode must be one of"
            ):
                visual_experiments.load_registry(path, root)

            visibility["fog_mode"] = "current"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "non-current fog mode"
            ):
                visual_experiments.load_registry(path, root)

            visibility["fog_mode"] = "none"
            visibility["level_height"] = 0.3
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "must change only one"
            ):
                visual_experiments.load_registry(path, root)

            visibility.pop("level_height")
            lighting = next(
                profile for profile in raw["profiles"] if profile["id"] == "l01-midnight"
            )
            lighting["fog_mode"] = "dimmed"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "mixes experiment axes"
            ):
                visual_experiments.load_registry(path, root)

            lighting.pop("fog_mode")
            softened = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "v04-fog-softened"
            )
            softened["fog_mode"] = "none"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "must cover none, dimmed"
            ):
                visual_experiments.load_registry(path, root)

            softened["fog_mode"] = "softened"
            baseline = next(
                profile for profile in raw["profiles"] if profile["id"] == "e00-baseline"
            )
            baseline.pop("fog_mode")
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "current fog"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_invalid_capture_sets_and_look_at_pairs(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            raw["capture_sets"]["smoke"].append("missing")
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "unknown captures"
            ):
                visual_experiments.load_registry(path, root)

            raw["capture_sets"]["smoke"].pop()
            raw["captures"][1].pop("look_at_offset")
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "must appear together"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_invalid_or_mixed_material_treatments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            matte = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "m01-matte-terrain"
            )
            matte["material_treatment"] = "glossy"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "material_treatment must be one of",
            ):
                visual_experiments.load_registry(path, root)

            matte["material_treatment"] = "current"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "non-current material treatment",
            ):
                visual_experiments.load_registry(path, root)

            matte["material_treatment"] = "matte-terrain"
            matte["fog_mode"] = "dimmed"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must change only one",
            ):
                visual_experiments.load_registry(path, root)

            matte.pop("fog_mode")
            unified = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "m02-unified-matte"
            )
            unified["material_treatment"] = "matte-terrain"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must cover matte-terrain and unified-matte",
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_invalid_or_mixed_indoor_light_profiles(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            tight = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "i01-crystal-tight"
            )
            tight["crystal_light_profile"] = "unknown"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "crystal_light_profile must be one of",
            ):
                visual_experiments.load_registry(path, root)

            tight["crystal_light_profile"] = "i01-crystal-tight"
            tight["fog_mode"] = "dimmed"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must select only its exact crystal-light profile",
            ):
                visual_experiments.load_registry(path, root)

            tight.pop("fog_mode")
            broad = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "i02-crystal-broad"
            )
            broad["crystal_light_profile"] = "i01-crystal-tight"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must select only its exact crystal-light profile",
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_rejects_invalid_or_mixed_edge_treatments(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            bevel = next(
                profile
                for profile in raw["profiles"]
                if profile["id"] == "e01-micro-bevel-004"
            )
            bevel["edge_treatment"] = "rounded"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "edge_treatment must be one of",
            ):
                visual_experiments.load_registry(path, root)

            bevel["edge_treatment"] = "current"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "non-current edge treatment",
            ):
                visual_experiments.load_registry(path, root)

            bevel["edge_treatment"] = "micro-bevel-004"
            bevel["fog_mode"] = "dimmed"
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must change only one",
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_proves_the_grand_scenario_world_and_seed(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            scenarios = root / raw["baseline"]["scenarios"]
            text = scenarios.read_text(encoding="utf-8")
            text = text.replace(
                'world: "config/worlds/procedural-grand-v3-baseline.ron",',
                'world: "config/worlds/procedural-hills.ron",',
                1,
            )
            scenarios.write_text(text, encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "baseline world exactly"
            ):
                visual_experiments.load_registry(path, root)

    def test_registry_pins_every_runtime_baseline_path(self) -> None:
        replacements = {
            "world": "assets/config/lighting.ron",
            "palette": "assets/config/lighting.ron",
            "scenarios": "assets/art/palette.ron",
            "default_lighting": "assets/config/lighting/overcast.ron",
            "overcast_lighting": "assets/config/lighting.ron",
        }
        for field, replacement in replacements.items():
            with self.subTest(field=field), tempfile.TemporaryDirectory() as directory:
                root, path, raw = self.make_fixture(directory)
                raw["baseline"][field] = replacement
                path.write_text(json.dumps(raw), encoding="utf-8")
                with self.assertRaisesRegex(
                    visual_experiments.ExperimentError, "canonical path"
                ):
                    visual_experiments.load_registry(path, root)

    def test_palette_candidate_must_cover_every_shipped_swatch(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, raw = self.make_fixture(directory)
            palette_profile = next(
                profile for profile in raw["profiles"] if profile["id"] == "p01-muted-earth"
            )
            candidate = root / palette_profile["palette"]
            candidate_raw = json.loads(candidate.read_text(encoding="utf-8"))
            candidate_raw["colors"].pop("terrain/grass")
            candidate.write_text(json.dumps(candidate_raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "swatches differ"
            ):
                visual_experiments.load_registry(path, root)

    def test_level_height_replacement_requires_exactly_one_field(self) -> None:
        text = "(\n    level_height: 0.4,\n)\n"
        replaced = visual_experiments.replace_level_height(text, 0.55)
        self.assertEqual(visual_experiments.read_level_height(replaced), 0.55)
        with self.assertRaisesRegex(
            visual_experiments.ExperimentError, "exactly one"
        ):
            visual_experiments.replace_level_height("()", 0.3)
        with self.assertRaisesRegex(
            visual_experiments.ExperimentError, "exactly one"
        ):
            visual_experiments.replace_level_height(text + text, 0.3)

    def test_overcast_patch_changes_only_the_named_staged_scenario(self) -> None:
        text = (ROOT / "assets" / "config" / "scenarios.ron").read_text(
            encoding="utf-8"
        )
        patched = visual_experiments.patch_scenario_lighting(
            text, "Grand V3 Baseline", "config/lighting/overcast.ron"
        )
        self.assertEqual(patched.count('lighting: "config/lighting/overcast.ron"'), 3)
        before = text.index('name: "Grand V3 Baseline"')
        after = patched.index('name: "Grand V3 Baseline"')
        self.assertEqual(before, after)
        with self.assertRaisesRegex(
            visual_experiments.ExperimentError, "already selects lighting"
        ):
            visual_experiments.patch_scenario_lighting(
                patched, "Grand V3 Baseline", "config/lighting/overcast.ron"
            )

    def test_cycle_noon_candidate_changes_only_declared_noon_fields(self) -> None:
        source = (ROOT / "assets" / "config" / "lighting.ron").read_text(
            encoding="utf-8"
        )
        _, overrides = visual_experiments.load_lighting_candidate(
            ROOT
            / "tools"
            / "visual_experiments"
            / "lighting"
            / "l05-soft-fill-noon.json"
        )
        patched = visual_experiments.patch_cycle_noon_lighting(source, overrides)
        self.assertNotEqual(patched, source)
        self.assertEqual(
            len(
                re.findall(
                    r"^\s*time_hours:\s*12\.0,\s*$",
                    patched,
                    re.MULTILINE,
                )
            ),
            1,
        )
        self.assertEqual(patched.count("default_time_hours: 12.0,"), 1)
        self.assertIn("ambient_brightness: 115.0,", patched)
        self.assertEqual(source.count("ambient_brightness: 115.0,"), 0)
        # Other keyframes remain byte-identical around their authored values.
        self.assertEqual(patched.count("ambient_brightness: 70.0,"), 1)
        self.assertEqual(patched.count("ambient_brightness: 110.0,"), 1)

    def test_active_indoor_crystal_candidates_are_exact_profiles(self) -> None:
        active = visual_experiments.load_indoor_crystal_spec(
            visual_experiments.INDOOR_CRYSTAL_SPEC
        )
        self.assertEqual(active["status"], "active")
        self.assertEqual(
            active["runtime_setting"],
            "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE",
        )
        self.assertEqual(
            active["baseline"], visual_experiments.INDOOR_CRYSTAL_BASELINE
        )
        self.assertEqual(
            tuple(candidate["id"] for candidate in active["candidates"]),
            visual_experiments.EXPECTED_INDOOR_CRYSTAL_IDS,
        )
        registry = visual_experiments.load_registry()
        self.assertEqual(
            tuple(
                profile.id
                for profile in registry.profiles
                if profile.axis == "indoor-lighting"
            ),
            visual_experiments.EXPECTED_INDOOR_CRYSTAL_IDS,
        )

    def test_active_indoor_crystal_spec_rejects_contract_drift(self) -> None:
        source = json.loads(
            visual_experiments.INDOOR_CRYSTAL_SPEC.read_text(encoding="utf-8")
        )
        mutations = (
            ("baseline", lambda raw: raw["baseline"].update({"range": 4.0})),
            (
                "mixed tight candidate",
                lambda raw: raw["candidates"][0]["overrides"].update(
                    {"shadow_maps_enabled": True}
                ),
            ),
            (
                "non-selective shadow",
                lambda raw: raw["candidates"][2].update(
                    {"target": "all-crystal-point-lights"}
                ),
            ),
            ("unknown field", lambda raw: raw.update({"surprise": True})),
        )
        for label, mutate in mutations:
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                raw = json.loads(json.dumps(source))
                mutate(raw)
                path = pathlib.Path(directory) / "indoor-crystal-v1.json"
                path.write_text(json.dumps(raw), encoding="utf-8")
                with self.assertRaises(visual_experiments.ExperimentError):
                    visual_experiments.load_indoor_crystal_spec(path)

    def test_cycle_noon_loader_rejects_crystal_point_light_overrides(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "invalid-indoor-light.json"
            path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "candidate_id": "i01-crystal-tight",
                        "base": "default-cycle",
                        "noon_overrides": {"point_light_range": 3.0},
                    }
                ),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "unknown overrides.*point_light_range",
            ):
                visual_experiments.load_lighting_candidate(path)

    def test_palette_replacement_preserves_exact_swatch_vocabulary(self) -> None:
        registry = visual_experiments.load_registry()
        baseline_path = ROOT / registry.baseline["palette"]
        baseline = visual_experiments.parse_palette_colors(
            baseline_path.read_text(encoding="utf-8")
        )
        _, candidate = visual_experiments.load_palette_candidate(
            ROOT / registry.profile("p01-muted-earth").palette, baseline
        )
        replaced = visual_experiments.replace_palette_colors(
            baseline_path.read_text(encoding="utf-8"), candidate
        )
        self.assertEqual(
            visual_experiments.parse_palette_colors(replaced), candidate
        )
        self.assertEqual(set(candidate), set(baseline))

    def test_asset_copy_is_independent_and_can_be_made_read_only(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "source"
            source.mkdir()
            source_file = source / "asset.ron"
            source_file.write_text("baseline", encoding="utf-8")
            destination = root / "destination"
            visual_experiments.copy_asset_tree(source, destination)
            copied = destination / "asset.ron"
            self.assertNotEqual(
                (source_file.stat().st_dev, source_file.stat().st_ino),
                (copied.stat().st_dev, copied.stat().st_ino),
            )
            copied.write_text("candidate", encoding="utf-8")
            self.assertEqual(source_file.read_text(encoding="utf-8"), "baseline")
            visual_experiments.make_tree_read_only(destination)
            self.assertEqual(copied.stat().st_mode & 0o222, 0)
            visual_experiments._remove_tree(destination)

    def test_asset_copy_rejects_symlinks(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "source"
            source.mkdir()
            target = source / "target.ron"
            target.write_text("baseline", encoding="utf-8")
            (source / "alias.ron").symlink_to(target)
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "symlinks"
            ):
                visual_experiments.copy_asset_tree(source, root / "destination")

    def test_staged_profiles_change_only_the_allowlisted_asset(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, _ = self.make_fixture(directory)
            registry = visual_experiments.load_registry(path, root)
            expected = {
                "e00-baseline": [],
                "l04-overcast": ["assets/config/scenarios.ron"],
                "l05-soft-fill-noon": ["assets/config/lighting.ron"],
                "z01-haze-light": [],
                "v02-fog-dimmed": [],
                "m01-matte-terrain": [],
                "e01-micro-bevel-004": [],
                "i02-crystal-broad": [],
                "h01-flat-030": [
                    "assets/config/worlds/procedural-grand-v3-baseline.ron"
                ],
                "p02-high-separation": [],
            }
            for profile_id, changed in expected.items():
                with self.subTest(profile=profile_id):
                    staged = root / ("stage-" + profile_id)
                    visual_experiments.copy_asset_tree(
                        root / "assets", staged / "assets"
                    )
                    state = visual_experiments.apply_profile(
                        root, staged, registry, registry.profile(profile_id)
                    )
                    self.assertEqual(
                        [item["path"] for item in state["modified_assets"]], changed
                    )
                    if profile_id in ("l05-soft-fill-noon", "z01-haze-light"):
                        self.assertEqual(
                            state["resolved_axis"]["source"],
                            registry.profile(profile_id).lighting_candidate,
                        )
                    if profile_id == "v02-fog-dimmed":
                        self.assertEqual(state["fog_mode"], "dimmed")
                        self.assertEqual(
                            state["resolved_axis"],
                            {"kind": "review-fog-mode", "mode": "dimmed"},
                        )
                    if profile_id == "m01-matte-terrain":
                        self.assertEqual(
                            state["material_treatment"], "matte-terrain"
                        )
                        self.assertEqual(
                            state["resolved_axis"],
                            {
                                "kind": "review-material-treatment",
                                "treatment": "matte-terrain",
                            },
                        )
                    if profile_id == "i02-crystal-broad":
                        self.assertEqual(
                            state["crystal_light_profile"],
                            "i02-crystal-broad",
                        )
                        self.assertEqual(
                            state["resolved_axis"],
                            {
                                "kind": "review-crystal-light-profile",
                                "runtime_setting": "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE",
                                "profile": "i02-crystal-broad",
                                "source": visual_experiments.INDOOR_CRYSTAL_SPEC_RELATIVE,
                                "baseline": visual_experiments.INDOOR_CRYSTAL_BASELINE,
                                "target": "all-crystal-point-lights",
                                "overrides": {"range": 7.0},
                                "resolved_target_state": {
                                    **visual_experiments.INDOOR_CRYSTAL_BASELINE,
                                    "range": 7.0,
                                },
                            },
                        )

    def test_capture_environment_scrubs_inherited_review_and_walk_state(self) -> None:
        registry = visual_experiments.load_registry()
        profile = registry.profile("l01-midnight")
        capture = registry.captures[0]
        environment = visual_experiments.build_capture_environment(
            {
                "PATH": os.environ.get("PATH", ""),
                "HEX_REVIEW_TIME": "wrong",
                "HEX_REVIEW_FOG": "wrong",
                "HEX_REVIEW_MATERIAL": "wrong",
                "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE": "i03-heart-feature-shadow",
                "HEX_REVIEW_CUTAWAY": "full",
                "HEX_WALK_SCRIPT": "stale.ron",
                "HEX_GRAND_PROFILE": "1",
                "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT": "1",
                "BEVY_ASSET_ROOT": "wrong",
                "HEX_GAME_DATA_DIR": "wrong",
                "RUSTFLAGS": "-C target-cpu=native",
                "RUSTUP_TOOLCHAIN": "nightly",
                "RUSTC": "/tmp/custom-rustc",
                "CARGO_BUILD_TARGET": "custom-target",
                "WGPU_BACKEND": "vulkan",
            },
            staged_root=ROOT / ".context" / "stage",
            data_root=ROOT / ".context" / "data",
            capture_path=ROOT / ".context" / "capture.png",
            registry=registry,
            profile=profile,
            capture=capture,
        )
        self.assertEqual(environment["HEX_REVIEW_TIME"], "0.0")
        self.assertEqual(environment["HEX_REVIEW_FOG"], "current")
        self.assertEqual(environment["HEX_REVIEW_MATERIAL"], "current")
        self.assertNotIn("HEX_REVIEW_CRYSTAL_LIGHT_PROFILE", environment)
        self.assertNotIn("HEX_REVIEW_CUTAWAY", environment)
        self.assertNotIn("HEX_WALK_SCRIPT", environment)
        self.assertNotIn("HEX_GRAND_PROFILE", environment)
        self.assertNotIn("HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT", environment)
        self.assertNotIn("RUSTFLAGS", environment)
        self.assertNotIn("RUSTUP_TOOLCHAIN", environment)
        self.assertNotIn("RUSTC", environment)
        self.assertNotIn("CARGO_BUILD_TARGET", environment)
        self.assertNotIn("WGPU_BACKEND", environment)
        self.assertEqual(environment["BEVY_ASSET_ROOT"], str(ROOT / ".context" / "stage"))

    def test_structural_draft_is_strictly_opt_in_and_recorded(self) -> None:
        registry = visual_experiments.load_registry()
        profile = registry.profile("e00-baseline")
        capture = registry.captures[0]
        arguments = {
            "staged_root": ROOT / ".context" / "stage",
            "data_root": ROOT / ".context" / "data",
            "capture_path": ROOT / ".context" / "capture.png",
            "registry": registry,
            "profile": profile,
            "capture": capture,
        }
        strict = visual_experiments.build_capture_environment(
            {"HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT": "1"},
            **arguments,
        )
        self.assertNotIn("HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT", strict)
        draft = visual_experiments.build_capture_environment(
            {},
            allow_structural_draft=True,
            **arguments,
        )
        self.assertEqual(draft["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"], "1")
        tokenized = visual_experiments._tokenized_environment(
            registry,
            profile,
            capture,
            allow_structural_draft=True,
        )
        self.assertEqual(
            tokenized["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"],
            "1",
        )
        plan = visual_experiments.build_plan(
            registry,
            (profile,),
            {
                "git_head": "a" * 40,
                "worktree_dirty": True,
                "workspace_content_sha256": "b" * 64,
            },
            visual_experiments.EXPERIMENT_ROOT / "draft-plan",
            visual_experiments.relevant_source_hashes(ROOT, registry),
            captures=(capture,),
            allow_structural_draft=True,
        )
        self.assertTrue(plan["structural_draft"])
        self.assertTrue(plan["build"]["structural_draft"])
        self.assertEqual(
            plan["profiles"][0]["captures"][0]["environment"]
            ["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"],
            "1",
        )

    def test_visibility_mode_is_emitted_in_real_and_tokenized_environments(self) -> None:
        registry = visual_experiments.load_registry()
        capture = registry.captures[0]
        expected = {
            "e00-baseline": "current",
            "v01-fog-none": "none",
            "v02-fog-dimmed": "dimmed",
            "v03-fog-observed-only": "observed-only",
            "v04-fog-softened": "softened",
        }
        for profile_id, fog_mode in expected.items():
            with self.subTest(profile=profile_id):
                profile = registry.profile(profile_id)
                environment = visual_experiments.build_capture_environment(
                    {"HEX_REVIEW_FOG": "stale"},
                    staged_root=ROOT / ".context" / "stage",
                    data_root=ROOT / ".context" / "data",
                    capture_path=ROOT / ".context" / "capture.png",
                    registry=registry,
                    profile=profile,
                    capture=capture,
                )
                self.assertEqual(environment["HEX_REVIEW_FOG"], fog_mode)
                self.assertEqual(
                    visual_experiments._tokenized_environment(
                        registry, profile, capture
                    )["HEX_REVIEW_FOG"],
                    fog_mode,
                )

    def test_material_treatment_is_emitted_in_real_and_tokenized_environments(self) -> None:
        registry = visual_experiments.load_registry()
        capture = registry.captures[0]
        expected = {
            "e00-baseline": "current",
            "m01-matte-terrain": "matte-terrain",
            "m02-unified-matte": "unified-matte",
        }
        for profile_id, treatment in expected.items():
            with self.subTest(profile=profile_id):
                profile = registry.profile(profile_id)
                environment = visual_experiments.build_capture_environment(
                    {"HEX_REVIEW_MATERIAL": "stale"},
                    staged_root=ROOT / ".context" / "stage",
                    data_root=ROOT / ".context" / "data",
                    capture_path=ROOT / ".context" / "capture.png",
                    registry=registry,
                    profile=profile,
                    capture=capture,
                )
                self.assertEqual(environment["HEX_REVIEW_MATERIAL"], treatment)
                self.assertEqual(
                    visual_experiments._tokenized_environment(
                        registry, profile, capture
                    )["HEX_REVIEW_MATERIAL"],
                    treatment,
                )

    def test_edge_treatment_is_emitted_in_real_and_tokenized_environments(self) -> None:
        registry = visual_experiments.load_registry()
        capture = registry.captures[0]
        expected = {
            "e00-baseline": "current",
            "e01-micro-bevel-004": "micro-bevel-004",
            "e02-micro-bevel-008": "micro-bevel-008",
        }
        for profile_id, treatment in expected.items():
            with self.subTest(profile=profile_id):
                profile = registry.profile(profile_id)
                environment = visual_experiments.build_capture_environment(
                    {"HEX_REVIEW_EDGE": "stale"},
                    staged_root=ROOT / ".context" / "stage",
                    data_root=ROOT / ".context" / "data",
                    capture_path=ROOT / ".context" / "capture.png",
                    registry=registry,
                    profile=profile,
                    capture=capture,
                )
                self.assertEqual(environment["HEX_REVIEW_EDGE"], treatment)
                self.assertEqual(
                    visual_experiments._tokenized_environment(
                        registry, profile, capture
                    )["HEX_REVIEW_EDGE"],
                    treatment,
                )

    def test_crystal_light_profile_is_emitted_only_for_indoor_candidates(self) -> None:
        registry = visual_experiments.load_registry()
        capture = registry.captures[-1]
        for profile in registry.profiles:
            with self.subTest(profile=profile.id):
                environment = visual_experiments.build_capture_environment(
                    {"HEX_REVIEW_CRYSTAL_LIGHT_PROFILE": "stale"},
                    staged_root=ROOT / ".context" / "stage",
                    data_root=ROOT / ".context" / "data",
                    capture_path=ROOT / ".context" / "capture.png",
                    registry=registry,
                    profile=profile,
                    capture=capture,
                )
                tokenized = visual_experiments._tokenized_environment(
                    registry, profile, capture
                )
                if profile.axis == "indoor-lighting":
                    self.assertEqual(
                        environment["HEX_REVIEW_CRYSTAL_LIGHT_PROFILE"],
                        profile.id,
                    )
                    self.assertEqual(
                        tokenized["HEX_REVIEW_CRYSTAL_LIGHT_PROFILE"],
                        profile.id,
                    )
                else:
                    self.assertNotIn(
                        "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE", environment
                    )
                    self.assertNotIn(
                        "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE", tokenized
                    )

    def test_indoor_light_plan_records_exact_resolved_contract(self) -> None:
        registry = visual_experiments.load_registry()
        profile = registry.profile("i03-heart-feature-shadow")
        plan = visual_experiments.build_plan(
            registry,
            (registry.profile("e00-baseline"), profile),
            {
                "git_head": "a" * 40,
                "worktree_dirty": False,
                "workspace_content_sha256": "b" * 64,
            },
            visual_experiments.EXPERIMENT_ROOT / "plan",
            visual_experiments.relevant_source_hashes(ROOT, registry),
            captures=(registry.captures[-1],),
        )
        record = plan["profiles"][1]
        self.assertEqual(record["crystal_light_profile"], profile.id)
        self.assertEqual(
            record["resolved_axis"],
            {
                "kind": "review-crystal-light-profile",
                "runtime_setting": "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE",
                "profile": profile.id,
                "source": visual_experiments.INDOOR_CRYSTAL_SPEC_RELATIVE,
                "baseline": visual_experiments.INDOOR_CRYSTAL_BASELINE,
                "target": "crystal-heart-offset-18",
                "overrides": {"shadow_maps_enabled": True},
                "resolved_target_state": {
                    **visual_experiments.INDOOR_CRYSTAL_BASELINE,
                    "shadow_maps_enabled": True,
                },
            },
        )
        self.assertEqual(
            record["captures"][0]["environment"][
                "HEX_REVIEW_CRYSTAL_LIGHT_PROFILE"
            ],
            profile.id,
        )
        self.assertIn(
            visual_experiments.INDOOR_CRYSTAL_SPEC_RELATIVE,
            plan["source_hashes"],
        )

    def test_capture_environment_projects_coast_and_normal_chamber(self) -> None:
        registry = visual_experiments.load_registry()
        look_at = registry.captures[2]
        environment = visual_experiments.build_capture_environment(
            {},
            staged_root=ROOT / ".context" / "stage",
            data_root=ROOT / ".context" / "data",
            capture_path=ROOT / ".context" / "capture.png",
            registry=registry,
            profile=registry.profile("e00-baseline"),
            capture=look_at,
        )
        self.assertEqual(environment["HEX_REVIEW_LOOK_AT_ANCHOR"], "grand_v3.coast")
        self.assertEqual(environment["HEX_REVIEW_LOOK_AT_OFFSET"], "50.0,30.0,55.0")
        interior = registry.captures[-1]
        environment = visual_experiments.build_capture_environment(
            {},
            staged_root=ROOT / ".context" / "stage",
            data_root=ROOT / ".context" / "data",
            capture_path=ROOT / ".context" / "capture.png",
            registry=registry,
            profile=registry.profile("e00-baseline"),
            capture=interior,
        )
        self.assertEqual(
            environment["HEX_REVIEW_FOCUS_ANCHOR"],
            "crystal_ascent.bottom_chamber",
        )
        self.assertNotIn("HEX_REVIEW_CUTAWAY", environment)
        self.assertNotIn("HEX_REVIEW_ILLUMINATION", environment)

    def test_capture_environment_retains_optional_interior_diagnostics(self) -> None:
        registry = visual_experiments.load_registry()
        interior = visual_experiments.CaptureSpec(
            id="99-interior-diagnostic",
            filename="99-interior-diagnostic.png",
            camera="map",
            view="top-down",
            focus_anchor="grand_v3.tunnel_midpoint",
            cutaway="full",
            illumination_overlay="overlay",
        )
        environment = visual_experiments.build_capture_environment(
            {},
            staged_root=ROOT / ".context" / "stage",
            data_root=ROOT / ".context" / "data",
            capture_path=ROOT / ".context" / "capture.png",
            registry=registry,
            profile=registry.profile("e00-baseline"),
            capture=interior,
        )
        self.assertEqual(environment["HEX_REVIEW_CUTAWAY"], "full")
        self.assertEqual(environment["HEX_REVIEW_ILLUMINATION"], "overlay")

    def test_overcast_capture_never_receives_a_time_override(self) -> None:
        registry = visual_experiments.load_registry()
        environment = visual_experiments.build_capture_environment(
            {"HEX_REVIEW_TIME": "12.0"},
            staged_root=ROOT / ".context" / "stage",
            data_root=ROOT / ".context" / "data",
            capture_path=ROOT / ".context" / "capture.png",
            registry=registry,
            profile=registry.profile("l04-overcast"),
            capture=registry.captures[0],
        )
        self.assertNotIn("HEX_REVIEW_TIME", environment)

    def test_capture_timeout_stops_the_complete_process_group(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            log = pathlib.Path(directory) / "capture.log"
            process = mock.Mock(pid=24680)
            process.wait.side_effect = [
                visual_experiments.subprocess.TimeoutExpired(["cargo"], 1),
                visual_experiments.subprocess.TimeoutExpired(["cargo"], 5),
                0,
            ]
            with mock.patch.object(
                visual_experiments.subprocess, "Popen", return_value=process
            ) as popen, mock.patch.object(visual_experiments.os, "killpg") as killpg:
                with self.assertRaisesRegex(
                    visual_experiments.CaptureError, "process group was stopped"
                ):
                    visual_experiments.run_logged_process(
                        ["cargo"],
                        cwd=ROOT,
                        environment={"PATH": os.environ.get("PATH", "")},
                        log_path=log,
                        timeout_seconds=1,
                    )
            self.assertTrue(popen.call_args.kwargs["start_new_session"])
            self.assertEqual(
                killpg.call_args_list,
                [
                    mock.call(24680, visual_experiments.signal.SIGTERM),
                    mock.call(24680, visual_experiments.signal.SIGKILL),
                ],
            )

    def test_build_once_resolves_and_hashes_one_exact_review_binary(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            executable = root / "hex_game"
            executable.write_bytes(b"review-binary")
            log = root / "build.log"

            def fake_build(_command, **arguments):
                arguments["log_path"].write_text(
                    json.dumps(
                        {
                            "reason": "compiler-artifact",
                            "target": {"name": "hex_game"},
                            "executable": str(executable),
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                )
                return 0

            with mock.patch.object(
                visual_experiments, "run_logged_process", side_effect=fake_build
            ):
                review = visual_experiments.build_review_binary(
                    root, log_path=log, timeout_seconds=10
                )
            self.assertEqual(review.path, executable.resolve())
            self.assertEqual(review.sha256, visual_experiments.sha256_file(executable))
            self.assertEqual(
                visual_experiments._recorded_capture_command(review),
                ["$REVIEW_BINARY"],
            )

    def test_resource_limits_reject_disk_growth_before_publication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "large.bin").write_bytes(b"12345")
            limits = visual_experiments.ResourceLimits(10, 10, 4, 0)
            with self.assertRaisesRegex(visual_experiments.CaptureError, "over cap"):
                visual_experiments._enforce_resource_limits(
                    root,
                    root,
                    limits,
                    visual_experiments.time.monotonic() + 10,
                )

    def test_stable_input_load_retries_a_provenance_race(self) -> None:
        registry = visual_experiments.load_registry()
        first = {
            "git_head": "a" * 40,
            "worktree_dirty": True,
            "workspace_content_sha256": "1" * 64,
        }
        second = {**first, "workspace_content_sha256": "2" * 64}
        hashes = {"profiles.json": "3" * 64}
        with mock.patch.object(
            visual_experiments,
            "workspace_provenance",
            side_effect=[first, second, second, second],
        ), mock.patch.object(
            visual_experiments, "load_registry", return_value=registry
        ) as loader, mock.patch.object(
            visual_experiments, "relevant_source_hashes", return_value=hashes
        ):
            loaded, loaded_hashes, provenance = visual_experiments.load_stable_inputs(
                visual_experiments.DEFAULT_REGISTRY
            )
        self.assertIs(loaded, registry)
        self.assertEqual(loaded_hashes, hashes)
        self.assertEqual(provenance, second)
        self.assertEqual(loader.call_count, 2)

    def test_png_validation_rejects_wrong_dimensions(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "capture.png"
            path.write_bytes(self.make_png(1920, 1080))
            self.assertEqual(visual_experiments.inspect_png(path), (1920, 1080))
            path.write_bytes(self.make_png(640, 480))
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "expected 1920x1080"
            ):
                visual_experiments.inspect_png(path)
            path.write_bytes(self.make_png(1920, 1080)[:-5])
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "truncated|missing"
            ):
                visual_experiments.inspect_png(path)

    @unittest.skipUnless(
        sys.platform == "darwin" or sys.platform.startswith("linux"),
        "atomic no-replace publication is supported on macOS and Linux",
    )
    def test_atomic_publication_never_replaces_a_destination(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            source = root / "source"
            source.mkdir()
            (source / "manifest.json").write_text("{}", encoding="utf-8")
            destination = root / "published"
            visual_experiments.atomic_publish_no_replace(source, destination)
            self.assertTrue((destination / "manifest.json").is_file())

            competing = root / "competing"
            competing.mkdir()
            (competing / "manifest.json").write_text("new", encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.CaptureError, "was not replaced"
            ):
                visual_experiments.atomic_publish_no_replace(competing, destination)
            self.assertEqual(
                (destination / "manifest.json").read_text(encoding="utf-8"), "{}"
            )
            self.assertTrue(competing.exists())

    @staticmethod
    def make_png(width: int, height: int) -> bytes:
        def chunk(kind: bytes, data: bytes) -> bytes:
            checksum = zlib.crc32(kind + data) & 0xFFFFFFFF
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", checksum)

        ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
        pixels = (b"\x00" + b"\x00" * (width * 4)) * height
        return (
            visual_experiments.PNG_SIGNATURE
            + chunk(b"IHDR", ihdr)
            + chunk(b"IDAT", zlib.compress(pixels, 9))
            + chunk(b"IEND", b"")
        )

    def _fake_capture(self, **arguments):
        profile = arguments["profile"]
        capture = arguments["capture"]
        registry = arguments["registry"]
        pack_root = arguments["pack_root"]
        profile_output = pack_root / "profiles" / profile.id
        profile_output.mkdir(parents=True, exist_ok=True)
        png = profile_output / capture.filename
        png.write_bytes(self.make_png(1920, 1080))
        digest = visual_experiments.sha256_file(png)
        sidecar = png.with_suffix(".manifest.json")
        sidecar.write_text(
            visual_experiments.canonical_json(
                {
                    "schema_version": 1,
                    "review_status": "UNREVIEWED",
                    "provenance": arguments["common_provenance"],
                    "profile": arguments["profile_state"],
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
                        "command": [
                            "cargo",
                            "run",
                            "--release",
                            "-p",
                            "hex_game",
                            "--features",
                            "map-review",
                        ],
                        "environment": visual_experiments._tokenized_environment(
                            registry, profile, capture
                        ),
                        "runtime_report": dict(
                            visual_experiments.RUNTIME_REPORT_PLACEHOLDER
                        ),
                    },
                    "artifact": {
                        "path": f"profiles/{profile.id}/{capture.filename}",
                        "sha256": digest,
                        "width": 1920,
                        "height": 1080,
                    },
                }
            ),
            encoding="utf-8",
        )
        log = profile_output / "logs" / f"{capture.id}.log"
        log.parent.mkdir(parents=True, exist_ok=True)
        log.write_text("fake capture\n", encoding="utf-8")
        return {
            "id": capture.id,
            "path": f"profiles/{profile.id}/{capture.filename}",
            "sha256": digest,
            "sidecar": f"profiles/{profile.id}/{sidecar.name}",
        }

    def test_matrix_publishes_only_after_the_complete_fake_capture_set(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, _ = self.make_fixture(directory)
            registry = visual_experiments.load_registry(path, root)
            profile = registry.profile("e00-baseline")
            alias = registry.profile("p02-high-separation")
            profiles = (profile, alias)
            provenance = {
                "git_head": "a" * 40,
                "worktree_dirty": False,
                "workspace_content_sha256": "b" * 64,
            }
            source_hashes = visual_experiments.relevant_source_hashes(root, registry)
            output = root / "published" / "matrix"
            with mock.patch.object(
                visual_experiments, "_run_capture", side_effect=self._fake_capture
            ), mock.patch.object(
                visual_experiments,
                "workspace_provenance",
                return_value=provenance,
            ), mock.patch.object(
                visual_experiments,
                "relevant_source_hashes",
                return_value=source_hashes,
            ):
                visual_experiments.run_matrix(
                    repository_root=root,
                    registry=registry,
                    profiles=profiles,
                    provenance=provenance,
                    output=output,
                    source_hashes=source_hashes,
                    timeout_seconds=1,
                )
            self.assertTrue((output / "manifest.json").is_file())
            self.assertEqual(
                len(list((output / "profiles" / profile.id).glob("*.png"))), 8
            )
            self.assertEqual(
                len(list((output / "profiles" / alias.id).glob("*.png"))), 8
            )
            manifest = json.loads(
                (output / "manifest.json").read_text(encoding="utf-8")
            )
            alias_state = next(
                item for item in manifest["profiles"] if item["id"] == alias.id
            )
            self.assertEqual(alias_state["modified_assets"], [])
            self.assertFalse((output / "runtime").exists())
            self.assertIn(
                "COMPLETE CAPTURE SET",
                (output / "review-index.md").read_text(encoding="utf-8"),
            )
            html = (output / "index.html").read_text(encoding="utf-8")
            self.assertLess(
                html.index("Capture-first comparisons"),
                html.index("Axis-first comparisons"),
            )
            self.assertIn("axis-baseline", html)
            (output / "unexpected.txt").write_text("extra", encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "file set differs"
            ):
                visual_experiments.validate_complete_pack(
                    output, registry, profiles
                )

            (output / "unexpected.txt").unlink()
            sidecar = (
                output
                / "profiles"
                / profile.id
                / registry.captures[0].filename
            ).with_suffix(".manifest.json")
            sidecar_raw = json.loads(sidecar.read_text(encoding="utf-8"))
            sidecar_raw["provenance"]["git_head"] = "corrupt"
            sidecar.write_text(json.dumps(sidecar_raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "schema/status"
            ):
                visual_experiments.validate_complete_pack(
                    output, registry, profiles
                )

    def test_candidate_html_pairs_every_capture_with_baseline_by_axis(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            pack_root = pathlib.Path(directory)
            registry = visual_experiments.load_registry()
            profiles = visual_experiments.selected_profiles(
                registry,
                (
                    "l05-soft-fill-noon",
                    "i03-heart-feature-shadow",
                    "v03-fog-observed-only",
                    "p01-muted-earth",
                ),
            )
            self.assertEqual(
                tuple(profile.id for profile in profiles),
                (
                    "e00-baseline",
                    "l05-soft-fill-noon",
                    "i03-heart-feature-shadow",
                    "v03-fog-observed-only",
                    "p01-muted-earth",
                ),
            )
            captures = registry.captures_for("smoke")
            visual_experiments._write_html_index(
                pack_root, registry, profiles, captures
            )
            rendered = (pack_root / "index.html").read_text(encoding="utf-8")
            self.assertIn('id="axis-lighting"', rendered)
            self.assertIn('id="axis-indoor-lighting"', rendered)
            self.assertIn('id="axis-visibility"', rendered)
            self.assertIn('id="axis-palette"', rendered)
            self.assertIn("v03-fog-observed-only — 01-world-topdown — fog: observed-only", rendered)
            self.assertIn("Baseline — 01-world-topdown — fog: current", rendered)
            self.assertIn(
                "i03-heart-feature-shadow — 01-world-topdown — fog: current — "
                "material: current — edge: current — crystal: i03-heart-feature-shadow",
                rendered,
            )
            for candidate in (
                "l05-soft-fill-noon",
                "i03-heart-feature-shadow",
                "v03-fog-observed-only",
                "p01-muted-earth",
            ):
                for capture in captures:
                    marker = (
                        f'data-baseline="e00-baseline" '
                        f'data-candidate="{candidate}" data-capture="{capture.id}"'
                    )
                    self.assertEqual(rendered.count(marker), 1)
                    comparison = rendered[rendered.index(marker) :]
                    comparison = comparison[: comparison.index("</section>")]
                    baseline_path = f"profiles/e00-baseline/{capture.filename}"
                    candidate_path = f"profiles/{candidate}/{capture.filename}"
                    self.assertIn(baseline_path, comparison)
                    self.assertIn(candidate_path, comparison)
                    self.assertLess(
                        comparison.index(baseline_path),
                        comparison.index(candidate_path),
                    )

    def test_candidate_only_selection_automatically_includes_baseline(self) -> None:
        registry = visual_experiments.load_registry()
        profiles = visual_experiments.selected_profiles(
            registry, ("v04-fog-softened",)
        )
        self.assertEqual(
            tuple(profile.id for profile in profiles),
            ("e00-baseline", "v04-fog-softened"),
        )

    def test_initial_profile_set_is_bounded_and_covers_requested_axes(self) -> None:
        registry = visual_experiments.load_registry()
        profiles = visual_experiments.selected_profiles(
            registry, (), profile_set="initial"
        )
        self.assertEqual(
            tuple(profile.id for profile in profiles),
            visual_experiments.INITIAL_SCREEN_PROFILE_IDS,
        )
        self.assertEqual(len(profiles), 10)
        self.assertEqual(
            {profile.axis for profile in profiles},
            {
                "baseline",
                "lighting",
                "haze",
                "visibility",
                "materials",
                "level_height",
                "palette",
            },
        )
        with self.assertRaisesRegex(
            visual_experiments.ExperimentError, "mutually exclusive"
        ):
            visual_experiments.selected_profiles(
                registry, ("p01-muted-earth",), profile_set="initial"
            )

    def test_comparison_report_metadata_is_semantic_and_deterministic(self) -> None:
        registry = visual_experiments.load_registry()
        profiles = visual_experiments.selected_profiles(
            registry, (), profile_set="initial"
        )
        captures = registry.captures_for("smoke")
        first = visual_experiments.comparison_report_metadata(profiles, captures)
        second = visual_experiments.comparison_report_metadata(profiles, captures)
        self.assertEqual(first, second)
        self.assertEqual(first["selection_id"], "initial")
        self.assertEqual(first["render_count"], 40)
        self.assertEqual(first["comparison_count"], 36)
        self.assertEqual(
            [axis["axis"] for axis in first["axes"]],
            [
                "lighting",
                "haze",
                "visibility",
                "materials",
                "level_height",
                "palette",
            ],
        )
        self.assertRegex(first["semantic_sha256"], r"^[0-9a-f]{64}$")

        changed = visual_experiments.comparison_report_metadata(
            profiles, registry.captures_for("screen")
        )
        self.assertNotEqual(first["semantic_sha256"], changed["semantic_sha256"])

        baseline = registry.profile("e00-baseline")
        alias = registry.profile("p02-high-separation")
        alias_report = visual_experiments.comparison_report_metadata(
            (baseline, alias), captures
        )
        redefined_alias = visual_experiments.comparison_report_metadata(
            (baseline, dataclasses.replace(alias, baseline_alias=False)), captures
        )
        self.assertNotEqual(
            alias_report["semantic_sha256"], redefined_alias["semantic_sha256"]
        )

    def test_matrix_failure_leaves_no_published_or_staged_pack(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, _ = self.make_fixture(directory)
            registry = visual_experiments.load_registry(path, root)
            profile = registry.profile("e00-baseline")
            provenance = {
                "git_head": "a" * 40,
                "worktree_dirty": False,
                "workspace_content_sha256": "b" * 64,
            }
            source_hashes = visual_experiments.relevant_source_hashes(root, registry)
            output = root / "published" / "matrix"

            def fail_capture(**arguments):
                if arguments["capture"].id == "02-highlands-oblique":
                    raise visual_experiments.CaptureError("injected capture failure")
                return self._fake_capture(**arguments)

            with mock.patch.object(
                visual_experiments, "_run_capture", side_effect=fail_capture
            ):
                with self.assertRaisesRegex(
                    visual_experiments.CaptureError, "injected"
                ):
                    visual_experiments.run_matrix(
                        repository_root=root,
                        registry=registry,
                        profiles=(profile,),
                        provenance=provenance,
                        output=output,
                        source_hashes=source_hashes,
                        timeout_seconds=1,
                    )
            self.assertFalse(output.exists())
            self.assertEqual(list(output.parent.glob(".matrix.staging-*")), [])

    def test_late_worktree_drift_prevents_publication(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, path, _ = self.make_fixture(directory)
            registry = visual_experiments.load_registry(path, root)
            profile = registry.profile("e00-baseline")
            provenance = {
                "git_head": "a" * 40,
                "worktree_dirty": False,
                "workspace_content_sha256": "b" * 64,
            }
            drifted = {**provenance, "workspace_content_sha256": "c" * 64}
            source_hashes = visual_experiments.relevant_source_hashes(root, registry)
            output = root / "published" / "matrix"
            with mock.patch.object(
                visual_experiments, "_run_capture", side_effect=self._fake_capture
            ), mock.patch.object(
                visual_experiments,
                "workspace_provenance",
                side_effect=[provenance, drifted],
            ), mock.patch.object(
                visual_experiments,
                "relevant_source_hashes",
                return_value=source_hashes,
            ):
                with self.assertRaisesRegex(
                    visual_experiments.CaptureError, "before publication"
                ):
                    visual_experiments.run_matrix(
                        repository_root=root,
                        registry=registry,
                        profiles=(profile,),
                        provenance=provenance,
                        output=output,
                        source_hashes=source_hashes,
                        timeout_seconds=1,
                    )
            self.assertFalse(output.exists())

    def test_output_is_confined_to_the_experiment_root(self) -> None:
        with self.assertRaisesRegex(
            visual_experiments.ExperimentError, "must stay under"
        ):
            visual_experiments.resolve_output("/tmp/not-a-review", ROOT / "unused")

    def test_sidecar_serialization_is_canonical(self) -> None:
        value = {"profile": "e00-baseline", "axes": {"view": "top-down", "time": 12.0}}
        first = visual_experiments.canonical_json(value)
        second = visual_experiments.canonical_json(
            {"axes": {"time": 12.0, "view": "top-down"}, "profile": "e00-baseline"}
        )
        self.assertEqual(first, second)
        self.assertNotIn(str(ROOT), first)


if __name__ == "__main__":
    unittest.main()
