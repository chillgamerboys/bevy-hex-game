"""Focused tests for the separate deterministic interaction-sweep surface."""

from __future__ import annotations

import contextlib
import io
import json
import pathlib
import tempfile
import unittest
from unittest import mock

from tools import test_visual_experiments as canonical_tests


visual_experiments = canonical_tests.visual_experiments
ROOT = pathlib.Path(__file__).resolve().parents[1]


class VisualExperimentSweepTests(unittest.TestCase):
    def make_fixture(self, directory: str):
        helper = canonical_tests.VisualExperimentTests()
        root, registry_path, _ = helper.make_fixture(directory)
        sweep_path = (
            root
            / "tools"
            / "visual_experiments"
            / "sweeps"
            / "night-aesthetic-v1.json"
        )
        sweep_path.parent.mkdir(parents=True)
        sweep_path.write_bytes(visual_experiments.DEFAULT_SWEEP_SPEC.read_bytes())
        registry = visual_experiments.load_registry(registry_path, root)
        return root, registry, sweep_path

    @staticmethod
    def capture_json(capture):
        value = {
            "id": capture.id,
            "filename": capture.filename,
            "camera": capture.camera,
            "view": capture.view,
        }
        for field in (
            "focus_anchor",
            "look_at_anchor",
            "cutaway",
            "illumination_overlay",
        ):
            field_value = getattr(capture, field)
            if field_value is not None:
                value[field] = field_value
        if capture.look_at_offset is not None:
            value["look_at_offset"] = list(capture.look_at_offset)
        return value

    def write_selection(
        self,
        root: pathlib.Path,
        *,
        selection_id: str,
        stage: str,
        base_ids,
        capture_ids,
        matrix=None,
        recipes=None,
        shard_count: int = 1,
        camera_manifest=None,
    ) -> pathlib.Path:
        value = {
            "version": 1,
            "id": selection_id,
            "stage": stage,
            "sweep_id": "night-aesthetic-v1",
            "shard_count": shard_count,
            "base_look_ids": list(base_ids),
            "capture_ids": list(capture_ids),
        }
        if matrix is not None:
            value["matrix"] = matrix
        if recipes is not None:
            value["recipes"] = recipes
        if camera_manifest is not None:
            value["camera_manifest"] = camera_manifest
        path = root / f"{selection_id}.json"
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    @staticmethod
    def current_matrix(**overrides):
        value = {
            "material_treatment": ["current"],
            "fog_mode": ["current"],
            "crystal_light_profile": ["current"],
            "edge_treatment": ["inherit"],
        }
        value.update(overrides)
        return value

    def test_checked_in_sweep_is_exact_and_keeps_canonical_registry(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        self.assertEqual(len(registry.profiles), 24)
        self.assertEqual(
            tuple(profile.id for profile in registry.profiles),
            visual_experiments.EXPECTED_PROFILE_IDS,
        )
        self.assertEqual(sweep.axis_order, visual_experiments.SWEEP_AXIS_ORDER)
        broad = sweep.tier("broad")
        golden = sweep.tier("golden")
        self.assertEqual(len(broad.looks), 243)
        self.assertEqual(len(golden.looks), 81)
        self.assertEqual(
            [len(broad.looks_for_shard(index)) for index in (1, 2, 3)],
            [81, 81, 81],
        )
        self.assertEqual(len({look.id for look in broad.looks}), 243)
        self.assertEqual(
            broad.looks[0].id,
            "h030-lbalanced-pshipped-z000-ehard",
        )
        self.assertEqual(
            broad.looks[-1].id,
            "h040-lcontrast-pseparate-z007-e008",
        )
        self.assertEqual(
            {look.values["height"]["id"] for look in broad.looks_for_shard(2)},
            {"h035"},
        )

    def test_sweep_ids_and_hashes_are_deterministic(self) -> None:
        registry = visual_experiments.load_registry()
        first = visual_experiments.load_sweep_spec(registry=registry)
        second = visual_experiments.load_sweep_spec(registry=registry)
        self.assertEqual(first.semantic_sha256, second.semantic_sha256)
        self.assertEqual(
            [look.semantic_sha256 for look in first.tier("broad").looks],
            [look.semantic_sha256 for look in second.tier("broad").looks],
        )
        self.assertEqual(
            visual_experiments.sweep_shard_semantic_sha256(
                first, first.tier("broad"), 1
            ),
            visual_experiments.sweep_shard_semantic_sha256(
                second, second.tier("broad"), 1
            ),
        )

    def test_sweep_rejects_matrix_drift_and_unknown_fields(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, registry, path = self.make_fixture(directory)
            raw = json.loads(path.read_text(encoding="utf-8"))
            raw["axes"]["height"][1]["level_height"] = 0.36
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError,
                "must be 0.30, 0.35, or 0.40",
            ):
                visual_experiments.load_sweep_spec(path, registry, root)

            raw["axes"]["height"][1]["level_height"] = 0.35
            raw["surprise"] = True
            path.write_text(json.dumps(raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "unknown fields"
            ):
                visual_experiments.load_sweep_spec(path, registry, root)

    def test_sweep_composes_height_palette_light_haze_and_runtime_edge(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, registry, path = self.make_fixture(directory)
            sweep = visual_experiments.load_sweep_spec(path, registry, root)
            look = next(
                item
                for item in sweep.tier("broad").looks
                if item.id == "h030-lsoft-pearth-z003-e004"
            )
            staged = pathlib.Path(directory) / "stage"
            visual_experiments.copy_asset_tree(root / "assets", staged / "assets")
            state = visual_experiments.apply_sweep_look(
                root, staged, registry, sweep, look
            )
            self.assertEqual(state["level_height"], 0.3)
            self.assertEqual(state["edge_treatment"], "micro-bevel-004")
            self.assertEqual(
                {item["path"] for item in state["modified_assets"]},
                {
                    registry.baseline["world"],
                    registry.baseline["palette"],
                    registry.baseline["default_lighting"],
                },
            )
            self.assertEqual(
                [source["axis"] for source in state["lighting_sources"]],
                ["light", "haze"],
            )
            profile = visual_experiments.sweep_profile(look)
            self.assertEqual(profile.axis, "interaction")
            self.assertIsNone(profile.lighting_candidate)
            self.assertEqual(profile.time_hours, 12.0)

    def test_sweep_plan_records_exact_resolved_sidecar_inputs(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        limits = visual_experiments.ResourceLimits(30, 600, 1024**3, 0)
        provenance = {
            "git_head": "a" * 40,
            "worktree_dirty": True,
            "workspace_content_sha256": "b" * 64,
        }
        plan = visual_experiments.build_sweep_plan(
            registry,
            sweep,
            sweep.tier("broad"),
            1,
            provenance,
            {"source": "c" * 64},
            pathlib.Path("/tmp/night-sweep"),
            limits,
            allow_structural_draft=True,
        )
        self.assertEqual(plan["render_count"], 81)
        self.assertEqual(plan["sweep"]["shard_look_count"], 81)
        self.assertEqual(len(plan["looks"]), 81)
        self.assertEqual(
            plan["looks"][0]["captures"][0]["environment"][
                "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"
            ],
            "1",
        )
        self.assertEqual(
            plan["looks"][0]["captures"][0]["environment"]["HEX_REVIEW_EDGE"],
            "current",
        )

    def test_semifinal_selection_uses_six_ordered_canonical_views(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        with tempfile.TemporaryDirectory() as directory:
            selection_path = self.write_selection(
                pathlib.Path(directory),
                selection_id="semifinal-test",
                stage="semifinal",
                base_ids=(sweep.tier("broad").looks[0].id,),
                capture_ids=tuple(
                    capture.id for capture in registry.captures[:6]
                ),
                matrix=self.current_matrix(),
            )
            selection = visual_experiments.load_sweep_selection(
                selection_path, registry, sweep
            )
            self.assertEqual(len(selection.recipes), 1)
            self.assertEqual(len(selection.captures), 6)
            self.assertEqual(
                tuple(capture.id for capture in selection.captures),
                tuple(capture.id for capture in registry.captures[:6]),
            )

    def test_selection_cli_validates_and_dry_runs_without_output(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            selection_path = self.write_selection(
                root,
                selection_id="selection-cli-test",
                stage="semifinal",
                base_ids=(sweep.tier("broad").looks[0].id,),
                capture_ids=("02-highlands-oblique",),
                matrix=self.current_matrix(),
            )
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = visual_experiments.main(
                    ["validate-selection", "--selection", str(selection_path)]
                )
            self.assertEqual(result, 0)
            validation = json.loads(output.getvalue())
            self.assertEqual(validation["recipe_count"], 1)
            self.assertEqual(validation["render_count"], 1)

            output_root = root / "dry-run-output"
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                result = visual_experiments.main(
                    [
                        "run-selection",
                        "--selection",
                        str(selection_path),
                        "--shard",
                        "1",
                        "--output-root",
                        str(output_root),
                        "--allow-structural-draft",
                        "--dry-run",
                    ]
                )
            self.assertEqual(result, 0)
            plan = json.loads(output.getvalue())
            self.assertEqual(plan["mode"], "selection-dry-run")
            self.assertEqual(plan["render_count"], 1)
            self.assertFalse(output_root.exists())

    def test_material_interior_and_geometric_matrices_resolve_runtime(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        base_id = sweep.tier("broad").looks[0].id
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            material_path = self.write_selection(
                root,
                selection_id="materials-test",
                stage="materials",
                base_ids=(base_id,),
                capture_ids=("02-highlands-oblique", "08-crystal-bottom-chamber"),
                matrix=self.current_matrix(
                    material_treatment=[
                        "current",
                        "matte-terrain",
                        "unified-matte",
                    ]
                ),
            )
            materials = visual_experiments.load_sweep_selection(
                material_path, registry, sweep
            )
            self.assertEqual(len(materials.recipes), 3)
            self.assertEqual(
                [
                    visual_experiments.selection_profile(recipe).material_treatment
                    for recipe in materials.recipes
                ],
                ["current", "matte-terrain", "unified-matte"],
            )

            interior_path = self.write_selection(
                root,
                selection_id="interior-test",
                stage="interior",
                base_ids=(base_id,),
                capture_ids=("07-tunnel-first-person", "08-crystal-bottom-chamber"),
                matrix=self.current_matrix(
                    fog_mode=["current", "dimmed"],
                    crystal_light_profile=[
                        "current",
                        "i01-crystal-tight",
                        "i02-crystal-broad",
                        "i03-heart-feature-shadow",
                    ],
                ),
            )
            interior = visual_experiments.load_sweep_selection(
                interior_path, registry, sweep
            )
            self.assertEqual(len(interior.recipes), 8)
            resolved = {
                (
                    visual_experiments.selection_profile(recipe).fog_mode,
                    visual_experiments.selection_profile(
                        recipe
                    ).crystal_light_profile
                    or "current",
                )
                for recipe in interior.recipes
            }
            self.assertEqual(
                resolved,
                {
                    (fog, crystal)
                    for fog in ("current", "dimmed")
                    for crystal in (
                        "current",
                        "i01-crystal-tight",
                        "i02-crystal-broad",
                        "i03-heart-feature-shadow",
                    )
                },
            )

            bevel_path = self.write_selection(
                root,
                selection_id="bevel-test",
                stage="bevel",
                base_ids=(base_id,),
                capture_ids=("02-highlands-oblique", "03-coast-river-outlet"),
                matrix=self.current_matrix(
                    edge_treatment=[
                        "inherit",
                        "geometric-bevel-004",
                        "geometric-bevel-008",
                    ]
                ),
            )
            bevel = visual_experiments.load_sweep_selection(
                bevel_path, registry, sweep
            )
            self.assertEqual(
                [
                    visual_experiments.selection_profile(recipe).edge_treatment
                    for recipe in bevel.recipes
                ],
                ["current", "geometric-bevel-004", "geometric-bevel-008"],
            )

    def test_finalist_selection_accepts_external_ordered_17_view_manifest(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            captures = [self.capture_json(capture) for capture in registry.captures]
            for index in range(9, 18):
                captures.append(
                    {
                        "id": f"{index:02d}-final-angle",
                        "filename": f"{index:02d}-final-angle.png",
                        "camera": "map",
                        "view": "default",
                        "look_at_anchor": "grand_v3.massif_crest",
                        "look_at_offset": [110 + index, 76, 115],
                    }
                )
            camera_path = root / "final-17-cameras.json"
            camera_path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "id": "final-17-cameras",
                        "captures": captures,
                    }
                ),
                encoding="utf-8",
            )
            base_id = sweep.tier("broad").looks[0].id
            selection_path = self.write_selection(
                root,
                selection_id="finalist-test",
                stage="finalist",
                base_ids=(base_id,),
                capture_ids=tuple(capture["id"] for capture in captures),
                recipes=(
                    {
                        "base_look_id": base_id,
                        "overrides": {
                            "material_treatment": "matte-terrain",
                            "fog_mode": "dimmed",
                            "crystal_light_profile": "i02-crystal-broad",
                            "edge_treatment": "geometric-bevel-004",
                        },
                    },
                ),
                camera_manifest=camera_path.name,
            )
            selection = visual_experiments.load_sweep_selection(
                selection_path, registry, sweep
            )
            self.assertEqual(len(selection.captures), 17)
            self.assertEqual(selection.captures[-1].id, "17-final-angle")
            plan = visual_experiments.build_selection_plan(
                registry,
                sweep,
                selection,
                1,
                {
                    "git_head": "a" * 40,
                    "worktree_dirty": True,
                    "workspace_content_sha256": "b" * 64,
                },
                {"source": "c" * 64},
                root / "output",
                visual_experiments.ResourceLimits(30, 60, 1024**3, 0),
                allow_structural_draft=True,
            )
            self.assertEqual(plan["render_count"], 17)
            self.assertEqual(
                plan["recipes"][0]["resolved_runtime"]["edge_treatment"],
                "geometric-bevel-004",
            )

            selection_raw = json.loads(selection_path.read_text(encoding="utf-8"))
            selection_raw["capture_ids"].pop()
            selection_path.write_text(json.dumps(selection_raw), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "exactly 17 captures"
            ):
                visual_experiments.load_sweep_selection(
                    selection_path, registry, sweep
                )

    def test_existing_valid_shard_resumes_without_build_or_capture(self) -> None:
        registry = visual_experiments.load_registry()
        sweep = visual_experiments.load_sweep_spec(registry=registry)
        tier = sweep.tier("golden")
        with tempfile.TemporaryDirectory() as directory:
            output_root = pathlib.Path(directory) / "sweeps"
            output = visual_experiments.sweep_shard_output(
                output_root, sweep, tier, 1
            )
            output.mkdir(parents=True)
            with mock.patch.object(
                visual_experiments, "validate_sweep_pack"
            ) as validator, mock.patch.object(
                visual_experiments, "build_review_binary"
            ) as builder:
                result, resumed = visual_experiments.run_sweep_shard(
                    repository_root=ROOT,
                    registry=registry,
                    sweep=sweep,
                    tier=tier,
                    shard=1,
                    provenance={
                        "git_head": "a" * 40,
                        "worktree_dirty": True,
                        "workspace_content_sha256": "b" * 64,
                    },
                    source_hashes={"source": "c" * 64},
                    output_root=output_root,
                    resource_limits=visual_experiments.ResourceLimits(
                        30, 60, 1024**3, 0
                    ),
                    allow_structural_draft=True,
                )
            self.assertEqual(result, output)
            self.assertTrue(resumed)
            validator.assert_called_once()
            builder.assert_not_called()

    @unittest.skipUnless(
        visual_experiments.sys.platform == "darwin"
        or visual_experiments.sys.platform.startswith("linux"),
        "atomic no-replace publication is supported on macOS and Linux",
    )
    def test_fake_complete_shard_publishes_and_then_resumes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, registry, path = self.make_fixture(directory)
            sweep = visual_experiments.load_sweep_spec(path, registry, root)
            tier = sweep.tier("golden")
            provenance = {
                "git_head": "a" * 40,
                "worktree_dirty": True,
                "workspace_content_sha256": "b" * 64,
            }
            source_hashes = visual_experiments.sweep_source_hashes(
                root, registry, sweep
            )
            binary = pathlib.Path(directory) / "hex_game"
            binary.write_bytes(b"fake-review-binary")

            def fake_build(_root, *, log_path, timeout_seconds):
                self.assertGreater(timeout_seconds, 0)
                log_path.write_text("fake build\n", encoding="utf-8")
                return visual_experiments.ReviewBinary(
                    binary,
                    visual_experiments.sha256_file(binary),
                    ("cargo", "build"),
                )

            capture_helper = canonical_tests.VisualExperimentTests()
            output_root = pathlib.Path(directory) / "published"
            limits = visual_experiments.ResourceLimits(30, 600, 1024**3, 0)
            with mock.patch.object(
                visual_experiments, "build_review_binary", side_effect=fake_build
            ), mock.patch.object(
                visual_experiments,
                "_run_capture",
                side_effect=capture_helper._fake_capture,
            ), mock.patch.object(
                visual_experiments,
                "workspace_provenance",
                return_value=provenance,
            ):
                output, resumed = visual_experiments.run_sweep_shard(
                    repository_root=root,
                    registry=registry,
                    sweep=sweep,
                    tier=tier,
                    shard=1,
                    provenance=provenance,
                    source_hashes=source_hashes,
                    output_root=output_root,
                    resource_limits=limits,
                    allow_structural_draft=True,
                )
                self.assertFalse(resumed)
                self.assertTrue((output / "manifest.json").is_file())
                output_again, resumed_again = visual_experiments.run_sweep_shard(
                    repository_root=root,
                    registry=registry,
                    sweep=sweep,
                    tier=tier,
                    shard=1,
                    provenance=provenance,
                    source_hashes=source_hashes,
                    output_root=output_root,
                    resource_limits=limits,
                    allow_structural_draft=True,
                )
            self.assertEqual(output_again, output)
            self.assertTrue(resumed_again)

    @unittest.skipUnless(
        visual_experiments.sys.platform == "darwin"
        or visual_experiments.sys.platform.startswith("linux"),
        "atomic no-replace publication is supported on macOS and Linux",
    )
    def test_fake_selection_shard_publishes_and_resumes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root, registry, path = self.make_fixture(directory)
            sweep = visual_experiments.load_sweep_spec(path, registry, root)
            base_id = sweep.tier("broad").looks[0].id
            selection_path = self.write_selection(
                root,
                selection_id="adaptive-publish-test",
                stage="bevel",
                base_ids=(base_id,),
                capture_ids=("02-highlands-oblique",),
                matrix=self.current_matrix(
                    edge_treatment=["geometric-bevel-004"]
                ),
            )
            selection = visual_experiments.load_sweep_selection(
                selection_path, registry, sweep
            )
            provenance = {
                "git_head": "a" * 40,
                "worktree_dirty": True,
                "workspace_content_sha256": "b" * 64,
            }
            source_hashes = visual_experiments.selection_source_hashes(
                root, registry, sweep, selection
            )
            binary = pathlib.Path(directory) / "selection-hex-game"
            binary.write_bytes(b"fake-selection-review-binary")

            def fake_build(_root, *, log_path, timeout_seconds):
                self.assertGreater(timeout_seconds, 0)
                log_path.write_text("fake selection build\n", encoding="utf-8")
                return visual_experiments.ReviewBinary(
                    binary,
                    visual_experiments.sha256_file(binary),
                    ("cargo", "build"),
                )

            capture_helper = canonical_tests.VisualExperimentTests()
            output_root = pathlib.Path(directory) / "selection-published"
            limits = visual_experiments.ResourceLimits(30, 600, 1024**3, 0)
            with mock.patch.object(
                visual_experiments, "build_review_binary", side_effect=fake_build
            ), mock.patch.object(
                visual_experiments,
                "_run_capture",
                side_effect=capture_helper._fake_capture,
            ), mock.patch.object(
                visual_experiments,
                "workspace_provenance",
                return_value=provenance,
            ):
                output, resumed = visual_experiments.run_selection_shard(
                    repository_root=root,
                    registry=registry,
                    sweep=sweep,
                    selection=selection,
                    shard=1,
                    provenance=provenance,
                    source_hashes=source_hashes,
                    output_root=output_root,
                    resource_limits=limits,
                    allow_structural_draft=True,
                )
                self.assertFalse(resumed)
                output_again, resumed_again = (
                    visual_experiments.run_selection_shard(
                        repository_root=root,
                        registry=registry,
                        sweep=sweep,
                        selection=selection,
                        shard=1,
                        provenance=provenance,
                        source_hashes=source_hashes,
                        output_root=output_root,
                        resource_limits=limits,
                        allow_structural_draft=True,
                    )
                )
            self.assertEqual(output_again, output)
            self.assertTrue(resumed_again)
            manifest = json.loads(
                (output / "manifest.json").read_text(encoding="utf-8")
            )
            self.assertEqual(
                manifest["profiles"][0]["edge_treatment"],
                "geometric-bevel-004",
            )
            scorecard = visual_experiments.build_sweep_scorecard(
                (output / "manifest.json",)
            )
            self.assertEqual(scorecard["looks"][0]["technical_gate"], "PASS")
            self.assertEqual(
                scorecard["looks"][0]["tier"],
                "bevel:adaptive-publish-test",
            )

    def test_scorecard_validates_pngs_and_blinds_look_ids(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            image = root / "profiles" / "look-a" / "hero.png"
            image.parent.mkdir(parents=True)
            image.write_bytes(
                canonical_tests.VisualExperimentTests.make_png(1920, 1080)
            )
            digest = visual_experiments.sha256_file(image)
            manifest = root / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "kind": "interaction-sweep-shard",
                        "sweep": {
                            "id": "night-aesthetic-v1",
                            "semantic_sha256": "a" * 64,
                            "tier": "broad",
                            "shard": 1,
                            "shard_semantic_sha256": "b" * 64,
                        },
                        "profiles": [
                            {
                                "id": "look-a",
                                "look_semantic_sha256": "c" * 64,
                                "resolved_axes": {"height": {"value_id": "h030"}},
                                "captures": [
                                    {
                                        "id": "hero",
                                        "path": "profiles/look-a/hero.png",
                                        "sha256": digest,
                                    }
                                ],
                            }
                        ],
                    }
                ),
                encoding="utf-8",
            )
            scorecard = visual_experiments.build_sweep_scorecard((manifest,))
            self.assertEqual(scorecard["looks"][0]["technical_gate"], "PASS")
            self.assertRegex(scorecard["looks"][0]["blind_id"], r"^blind-[0-9a-f]{12}$")
            self.assertEqual(
                set(scorecard["looks"][0]["reviewer_1"]),
                set(visual_experiments.SWEEP_SCORE_FIELDS),
            )

    def test_report_requires_and_renders_exactly_twelve_plus_four(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            look_ids = [f"look-{index:02d}" for index in range(1, 17)]
            scorecard = {
                "kind": "interaction-sweep-scorecard",
                "sweep_id": "night-aesthetic-v1",
                "sweep_semantic_sha256": "a" * 64,
                "semantic_sha256": "b" * 64,
                "looks": [
                    {
                        "look_id": look_id,
                        "axes": {
                            "height": {"value_id": "h030"},
                            "edge": {"treatment": "current"},
                        },
                    }
                    for look_id in look_ids
                ],
            }
            (root / "scorecard.json").write_text(
                json.dumps(scorecard), encoding="utf-8"
            )
            scores = {
                field: 3 for field in visual_experiments.SWEEP_SCORE_FIELDS
            }
            selection = {
                "version": 1,
                "title": "Night review",
                "scorecard": "scorecard.json",
                "winners": [
                    {
                        "look_id": look_id,
                        "rank": rank,
                        "scores": scores,
                        "notes": f"winner {rank}",
                    }
                    for rank, look_id in enumerate(look_ids[:12], start=1)
                ],
                "representatives": [
                    {
                        "look_id": look_id,
                        "role": f"representative {index}",
                        "scores": scores,
                        "notes": f"alternative {index}",
                    }
                    for index, look_id in enumerate(look_ids[12:], start=1)
                ],
            }
            selection_path = root / "selection.json"
            selection_path.write_text(json.dumps(selection), encoding="utf-8")
            report = visual_experiments.render_sweep_selection(selection_path)
            self.assertIn("# Night review", report)
            self.assertIn("| 12 | `look-12` | 3.000", report)
            self.assertIn("representative 4", report)

            selection["winners"].pop()
            selection_path.write_text(json.dumps(selection), encoding="utf-8")
            with self.assertRaisesRegex(
                visual_experiments.ExperimentError, "exactly 12 winners"
            ):
                visual_experiments.render_sweep_selection(selection_path)


if __name__ == "__main__":
    unittest.main()
