"""Focused tests for the fail-closed Grand V3 visual experiment harness."""

from __future__ import annotations

import importlib.util
import json
import os
import pathlib
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
        self.assertEqual(len(registry.captures), 4)
        self.assertEqual(
            {capture.camera for capture in registry.captures},
            {"map", "character", "first-person"},
        )

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
            raw["profiles"][7]["palette"] = (
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
            candidate = root / raw["profiles"][7]["palette"]
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
                "h01-flat-030": [
                    "assets/config/worlds/procedural-grand-v3-baseline.ron"
                ],
                "p02-high-separation": ["assets/art/palette.ron"],
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

    def test_capture_environment_scrubs_inherited_review_and_walk_state(self) -> None:
        registry = visual_experiments.load_registry()
        profile = registry.profile("l01-midnight")
        capture = registry.captures[0]
        environment = visual_experiments.build_capture_environment(
            {
                "PATH": os.environ.get("PATH", ""),
                "HEX_REVIEW_TIME": "wrong",
                "HEX_REVIEW_CUTAWAY": "full",
                "HEX_WALK_SCRIPT": "stale.ron",
                "HEX_GRAND_PROFILE": "1",
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
        self.assertNotIn("HEX_REVIEW_CUTAWAY", environment)
        self.assertNotIn("HEX_WALK_SCRIPT", environment)
        self.assertNotIn("HEX_GRAND_PROFILE", environment)
        self.assertNotIn("RUSTFLAGS", environment)
        self.assertNotIn("RUSTUP_TOOLCHAIN", environment)
        self.assertNotIn("RUSTC", environment)
        self.assertNotIn("CARGO_BUILD_TARGET", environment)
        self.assertNotIn("WGPU_BACKEND", environment)
        self.assertEqual(environment["BEVY_ASSET_ROOT"], str(ROOT / ".context" / "stage"))

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
                        "environment": visual_experiments._tokenized_environment(
                            registry, profile, capture
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
                    profiles=(profile,),
                    provenance=provenance,
                    output=output,
                    source_hashes=source_hashes,
                    timeout_seconds=1,
                )
            self.assertTrue((output / "manifest.json").is_file())
            self.assertEqual(
                len(list((output / "profiles" / profile.id).glob("*.png"))), 4
            )
            self.assertFalse((output / "runtime").exists())
            self.assertIn(
                "COMPLETE CAPTURE SET",
                (output / "review-index.md").read_text(encoding="utf-8"),
            )
            (output / "unexpected.txt").write_text("extra", encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "file set differs"
            ):
                visual_experiments.validate_complete_pack(
                    output, registry, (profile,)
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
                    output, registry, (profile,)
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
                if arguments["capture"].id == "02-massif-oblique":
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
