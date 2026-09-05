"""Focused contract tests for the small-geometry external capture harness."""

from __future__ import annotations

import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile
from unittest import mock


TOOLS_ROOT = pathlib.Path(__file__).resolve().parents[1]
if str(TOOLS_ROOT) not in sys.path:
    sys.path.insert(0, str(TOOLS_ROOT))

import world_detail_experiments as harness  # noqa: E402


EXPECTED_IDS = (
    "snow-01-straight-128", "snow-02-straight-140", "snow-03-straight-152",
    "snow-04-coherent-136", "snow-05-coherent-144", "snow-06-terrain-aware",
    "snow-07-terrain-aware-shell-004", "snow-08-terrain-aware-shell-008",
    "snow-09-terrain-aware-shell-012", "water-01-alpha-085", "water-02-alpha-070",
    "water-03-alpha-055", "water-04-depth-short", "water-05-depth-long",
    "water-06-transmission", "water-07-rough-no-refraction", "clouds-01-faceted-clear",
    "clouds-02-faceted-grazing", "clouds-03-faceted-crossing", "clouds-04-rounded-grazing",
    "clouds-05-lenticular-grazing", "clouds-06-rounded-coverage-010",
    "clouds-07-rounded-coverage-028", "clouds-08-rounded-shadow",
    "shore-01-wet-rim-narrow", "shore-02-wet-rim-wide", "shore-03-foam-narrow",
    "shore-04-foam-wide", "shore-05-plunge-spray", "shore-06-restrained-combination",
    "vegetation-01-scale-light", "vegetation-02-scale-wide", "vegetation-03-dust-light",
    "vegetation-04-dust-heavy", "vegetation-05-scale-light-dust-light",
    "vegetation-06-scale-heavy-dust-heavy", "cliff-01-value-006", "cliff-02-value-012",
    "cliff-03-strata-24", "cliff-04-strata-40", "cliff-05-strata-coherent",
    "cliff-06-strata-coherent-value", "props-01-boulders-low", "props-02-boulders-high",
    "props-03-litter-low", "props-04-litter-high", "props-05-mixed", "props-06-clustered",
    "ice-01-level-narrow", "ice-02-level-medium", "ice-03-level-wide",
    "ice-04-snow-adjacent", "ice-05-frozen-or-snow", "ice-06-frozen-or-snow-feathered",
    "fog-01-water-light", "fog-02-water-heavy", "fog-03-valley-light",
    "fog-04-valley-heavy", "fog-05-mixed", "fog-06-mixed-cinematic",
)


def fixture_stage_manifest(*, tree_sha256: str = "a" * 64) -> dict:
    condition = harness.LIGHTING_CONDITIONS["neutral"]
    return {
        "asset_stage": condition.asset_stage,
        "source_asset_tree_sha256": "b" * 64,
        "staged_asset_tree_sha256": tree_sha256,
    }


def fixture_asset_stage(root: pathlib.Path, manifest=None) -> tuple:
    stage_root = root / "asset-root"
    stage_root.mkdir(parents=True, exist_ok=True)
    resolved_manifest = manifest or fixture_stage_manifest()
    (stage_root.parent / "stage-manifest.json").write_text(
        harness.pretty_json(resolved_manifest),
        encoding="utf-8",
    )
    return stage_root, resolved_manifest


def fixture_capture_job(root: pathlib.Path, job_id: str) -> dict:
    raw_root = root / "raw"
    raw_path = raw_root / "runtime" / "raw-stills" / f"{job_id}.png"
    condition = harness.LIGHTING_CONDITIONS["neutral"]
    return {
        "id": job_id,
        "kind": "still",
        "stage": "01-neutral-screen",
        "look_id": "snow-01-straight-128",
        "profile_sha256": harness.atomic_profiles()[0].sha256,
        "profile_json": harness.atomic_profiles()[0].canonical_json,
        "control_profile_omitted": False,
        "lighting": "neutral",
        "asset_stage": condition.asset_stage,
        "time_hours": condition.time_hours,
        "liquid_phase_seconds": 0.0,
        "capture_plan": {
            "version": 1,
            "captures": [{"path": str(raw_path)}],
        },
        "raw_capture_root": str(raw_root),
        "cameras": [],
        "artifacts": [f"runtime/raw-stills/{job_id}.png"],
    }


def fixture_capture_plan(root: pathlib.Path, jobs: list[dict]) -> dict:
    return {
        "output_root": str(root / "publication"),
        "raw_capture_root": str(root / "raw"),
        "provenance": {"fixture": True},
        "study": {
            "jobs": jobs,
            "verification_jobs": [],
            "reproduction_jobs": [],
            "slot_accounting": {"fresh_shared_control_renders": 0},
        },
        "motion": {"jobs": []},
    }


def fixture_capture_record(executable_sha256: str, marker: str = "1") -> dict:
    return {
        "pngs": [{"sha256": marker * 64}],
        "reports": [{"sha256": chr(ord(marker) + 1) * 64}],
        "authority": {"world": "c" * 64, "gameplay_state": "d" * 64},
        "anchor_heights": {},
        "anchor_classes": {},
        "projection_states": [],
        "runtime_receipt": {
            "receipt_sha256": "e" * 64,
            "process_id": 123,
            "executable_sha256": executable_sha256,
        },
    }


def fixture_runtime_receipt(
    job: dict,
    index: int,
    *,
    executable_sha256: str = "a" * 64,
    source_provenance_sha256: str = "b" * 64,
) -> dict:
    body = {
        "version": 1,
        "launch_nonce": f"{index + 1:064x}",
        "process_id": 10_000 + index,
        "executable_sha256": executable_sha256,
        "source_provenance_sha256": source_provenance_sha256,
        "capture_plan_sha256": harness.sha256_bytes(
            harness.compact_json(job["capture_plan"]).encode("utf-8")
        ),
        "profile_sha256": job["profile_sha256"],
    }
    return {
        **body,
        "receipt_sha256": harness.sha256_bytes(
            harness.compact_json(body).encode("utf-8")
        ),
    }


def stamp_review_evidence(selection: dict) -> dict:
    selection["review_evidence"] = {
        "reviewer_ids": ["fixture-review-a", "fixture-review-b"],
        "review_sha256": ["0" * 64, "1" * 64],
        "metrics_sha256": "2" * 64,
        "performance_sha256": "3" * 64,
        "review_packet_path": "/tmp/fixture-review-packet.json",
        "review_packet_sha256": "4" * 64,
        "unblind_map_path": "/tmp/fixture-unblind-map.json",
        "unblind_map_sha256": "5" * 64,
        "motion_review_evidence": None,
        "scoring_contract_sha256": harness.sha256_object(harness.scoring_contract()),
        "decisions_sha256": harness._selection_decisions_sha256(selection),
    }
    return selection


def complete_selection() -> dict:
    selection = harness.selection_template()
    selection["status"] = "TEST_FIXTURE_ONLY"
    selection["review_sources"] = ["blind-review-a.json", "blind-review-b.json"]
    for family in harness.FAMILY_ORDER:
        profiles = [profile for profile in harness.atomic_profiles() if profile.family == family]
        selection["promoted"][family] = [profiles[0].id, profiles[1].id]
        selection["stress_diagnostics"][family] = [profiles[0].id, profiles[1].id]
        selection["ladder_inputs"][family] = profiles[0].id
        selection["atomic_winners"][family] = profiles[0].id
        selection["pre_motion_atomic_winners"][family] = profiles[0].id
        for combination_id in harness.COMBINATION_IDS:
            selection["combinations"][combination_id][family] = profiles[0].id
            selection["pre_motion_combinations"][combination_id][family] = profiles[0].id
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
        for step_id, predecessor, introduced in harness.LADDER_STEPS
    ]
    return stamp_review_evidence(selection)


def maximally_distinct_selection() -> dict:
    selection = harness.selection_template()
    selection["status"] = "MAXIMUM_ACCOUNTING_FIXTURE_ONLY"
    selection["review_sources"] = ["blind-review-a.json", "blind-review-b.json"]
    for index, family in enumerate(harness.FAMILY_ORDER):
        profiles = [profile for profile in harness.atomic_profiles() if profile.family == family]
        first, second = profiles[0].id, profiles[-1].id
        selection["promoted"][family] = [first, second]
        selection["stress_diagnostics"][family] = [first, second]
        selection["ladder_inputs"][family] = first
        selection["atomic_winners"][family] = first
        selection["pre_motion_atomic_winners"][family] = first
        selection["combinations"]["score-leader"][family] = first
        selection["combinations"]["restrained"][family] = second
        selection["combinations"]["expressive"][family] = first if index % 2 == 0 else second
        selection["pre_motion_combinations"]["score-leader"][family] = first
        selection["pre_motion_combinations"]["restrained"][family] = second
        selection["pre_motion_combinations"]["expressive"][family] = (
            first if index % 2 == 0 else second
        )
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
        for step_id, predecessor, introduced in harness.LADDER_STEPS
    ]
    return stamp_review_evidence(selection)


class MatrixTests(unittest.TestCase):
    def test_alpine_vegetation_primary_camera_is_the_highlands_view(self) -> None:
        self.assertEqual(
            harness.PRIMARY_CAMERAS["alpine_vegetation"],
            "02-highlands-oblique",
        )
        vegetation = [
            profile
            for profile in harness.atomic_profiles()
            if profile.family == "alpine_vegetation"
        ]
        self.assertEqual(len(vegetation), 6)
        self.assertTrue(
            all(
                harness.PRIMARY_CAMERAS[profile.family]
                == "02-highlands-oblique"
                for profile in vegetation
            )
        )

    def test_exact_sixty_ids_and_canonical_hashes(self) -> None:
        profiles = harness.atomic_profiles()
        self.assertEqual(tuple(profile.id for profile in profiles), EXPECTED_IDS)
        self.assertEqual(len(profiles), 60)
        self.assertEqual(len({profile.sha256 for profile in profiles}), 60)
        for profile in profiles:
            self.assertEqual(
                harness.compact_json(json.loads(profile.canonical_json)),
                profile.canonical_json,
            )
            self.assertEqual(
                harness.sha256_bytes(profile.canonical_json.encode()),
                profile.sha256,
            )
        self.assertEqual(harness.control_profile().sha256, harness.CONTROL_PROFILE_SHA256)

    def test_transmission_water_uses_transmission_and_depth_without_oit(self) -> None:
        profile = next(
            profile
            for profile in harness.atomic_profiles()
            if profile.id == "water-06-transmission"
        )
        self.assertEqual(
            harness._profile_requirements(json.loads(profile.canonical_json)),
            {
                "oit": False,
                "medium_transmission": True,
                "depth_texture": True,
                "volumetrics": False,
            },
        )

    def test_opaque_cliff_shell_needs_no_camera_feature(self) -> None:
        profile = next(
            profile
            for profile in harness.atomic_profiles()
            if profile.family == "cliff_strata"
        )
        self.assertEqual(
            harness._profile_requirements(json.loads(profile.canonical_json)),
            {
                "oit": False,
                "medium_transmission": False,
                "depth_texture": False,
                "volumetrics": False,
            },
        )

    def test_opaque_wet_rims_do_not_request_transparency(self) -> None:
        profiles = {profile.id: profile for profile in harness.atomic_profiles()}
        self.assertEqual(
            harness._profile_requirements(
                json.loads(profiles["shore-01-wet-rim-narrow"].canonical_json)
            ),
            {
                "oit": False,
                "medium_transmission": False,
                "depth_texture": False,
                "volumetrics": False,
            },
        )
        self.assertTrue(
            harness._profile_requirements(
                json.loads(profiles["shore-03-foam-narrow"].canonical_json)
            )["oit"]
        )

    def test_camera_manifests_are_vendored_and_exact(self) -> None:
        final, focused, provenance = harness.load_camera_sets()
        self.assertEqual(len(final), 17)
        self.assertEqual(len(focused), 4)
        self.assertEqual(
            [camera.id for camera in focused],
            ["02-highlands-oblique", "03-coast-river-outlet", "14-cascade-basin-full-height", "16-deep-tree-shade"],
        )
        self.assertEqual(provenance["warning"], harness.WARNING)

    def test_camera_provenance_rejects_missing_and_mismatched_upstream_authority(self) -> None:
        original = json.loads(harness.CAMERA_PROVENANCE_PATH.read_text(encoding="utf-8"))
        mutations = (
            lambda document: document["sources"][0].__setitem__("upstream_path", "/missing/cameras.json"),
            lambda document: document["sources"][0].__setitem__("upstream_sha256", "0" * 64),
            lambda document: document["sources"][0].__setitem__("vendored_sha256", "0" * 64),
        )
        for mutate in mutations:
            with self.subTest(mutate=mutate):
                document = json.loads(json.dumps(original))
                mutate(document)
                with tempfile.TemporaryDirectory() as temporary:
                    provenance_path = pathlib.Path(temporary) / "camera-provenance.json"
                    provenance_path.write_text(json.dumps(document), encoding="utf-8")
                    with mock.patch.object(harness, "CAMERA_PROVENANCE_PATH", provenance_path):
                        with self.assertRaises(harness.HarnessError):
                            harness.load_camera_sets()

    def test_camera_provenance_rejects_semantic_drift_after_raw_repin(self) -> None:
        provenance = json.loads(harness.CAMERA_PROVENANCE_PATH.read_text(encoding="utf-8"))
        upstream = pathlib.Path(provenance["sources"][0]["upstream_path"])
        camera_document = json.loads(upstream.read_text(encoding="utf-8"))
        camera_document["captures"][0]["view"] = "default"
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            upstream_path = root / "upstream-camera.json"
            upstream_path.write_text(harness.pretty_json(camera_document), encoding="utf-8")
            provenance["sources"][0]["upstream_path"] = str(upstream_path)
            provenance["sources"][0]["upstream_sha256"] = harness.sha256_file(upstream_path)
            provenance_path = root / "camera-provenance.json"
            provenance_path.write_text(json.dumps(provenance), encoding="utf-8")
            with mock.patch.object(harness, "CAMERA_PROVENANCE_PATH", provenance_path):
                with self.assertRaisesRegex(harness.HarnessError, "semantically equivalent"):
                    harness.load_camera_sets()

    def test_review_and_runtime_contract_specs_are_pinned(self) -> None:
        review_schema = json.loads(harness.REVIEW_SCHEMA_PATH.read_text(encoding="utf-8"))
        runtime_contract = json.loads(
            harness.RUNTIME_EVIDENCE_CONTRACT_PATH.read_text(encoding="utf-8")
        )
        self.assertEqual(
            review_schema["properties"]["scoring_contract_sha256"]["const"],
            harness.sha256_object(harness.scoring_contract()),
        )
        self.assertEqual(runtime_contract["warning"], harness.WARNING)
        self.assertEqual(
            runtime_contract["focused_control_funnel"],
            {
                "camera_ids": list(harness.BASELINE_ORACLE_CAMERA_IDS),
                "omitted_profile_jobs": 4,
                "explicit_current_jobs": 4,
                "captures_per_job": 1,
                "fresh_process_per_png": True,
                "distinct_runtime_receipts_required": 8,
                "oracle_equivalence": (
                    "aggregate only after all four omitted-profile one-camera jobs "
                    "pass their clean-source stable-pixel gates"
                ),
            },
        )
        self.assertEqual(
            runtime_contract["launch_environment"],
            {
                "inherited_hex_state_scrubbed": True,
                "required_name": harness.STRUCTURAL_DRAFT_ENVIRONMENT,
                "required_value": harness.STRUCTURAL_DRAFT_VALUE,
                "scope": "map-review-only",
                "applies_to": [
                    "still",
                    "control-verification",
                    "reproduction",
                    "motion",
                    "lifecycle",
                ],
            },
        )
        lifecycle_cycle_fields = runtime_contract["lifecycle_certificate"][
            "cycle_exact_fields_before_hash"
        ]
        self.assertEqual(lifecycle_cycle_fields, list(harness.LIFECYCLE_CYCLE_FIELDS))
        self.assertEqual(
            runtime_contract["sidecar"]["effect_validation"]["exact_fields"],
            ["cloud_coverage", "ice_coverage", "fog_coverage", "waterfall_anchors"],
        )
        cloud_coverage = runtime_contract["sidecar"]["effect_validation"][
            "cloud_coverage"
        ]
        self.assertEqual(
            cloud_coverage["exact_fields"],
            [
                "field_radius",
                "target_fraction",
                "measured_fraction",
                "tolerance",
                "sample_count",
                "cloud_clusters",
                "peak_intersection_required",
                "peak_intersecting_puffs",
            ],
        )
        self.assertIn("emitted low-poly puff XZ silhouettes", cloud_coverage["target_fraction"])
        self.assertIn("not cluster envelopes", cloud_coverage["target_fraction"])

    def test_prior_bevel_evidence_is_copied_and_hash_pinned(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            output = pathlib.Path(temporary).resolve()
            evidence = harness._materialize_prior_aesthetic_evidence(output, create=True)
            self.assertEqual(len(evidence["records"]), 2)
            copied = output / "provenance" / "prior-aesthetic-report" / "README.md"
            copied.write_text("tampered", encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "offline prior aesthetic"):
                harness._materialize_prior_aesthetic_evidence(output, create=False)

    def test_profile_parser_rejects_non_matrix_and_noncanonical_json(self) -> None:
        profile = json.loads(harness.control_profile().canonical_json)
        profile["snow"] = {"kind": "straight_threshold", "level": 129}
        with self.assertRaises(harness.HarnessError):
            harness.validate_profile_json(harness.compact_json(profile))
        profile["snow"] = {"kind": "straight_threshold", "level": 128}
        with self.assertRaises(harness.HarnessError):
            harness.validate_profile_json(json.dumps(profile, ensure_ascii=False))
        with self.assertRaises(harness.HarnessError):
            harness.validate_profile_json(
                harness.control_profile().canonical_json.replace(
                    '"snow":{"kind":"current"}',
                    '"snow":{"kind":"terrain_aware","vertical_shell_height":NaN}',
                )
            )
        with self.assertRaises(harness.HarnessError):
            harness.validate_profile_json(
                harness.control_profile().canonical_json.replace('"version":1', '"version":true')
            )
        float_profile = next(
            profile
            for profile in harness.atomic_profiles()
            if profile.id == "fog-01-water-light"
        )
        with self.assertRaises(harness.HarnessError):
            harness.validate_profile_json(float_profile.canonical_json.replace(":12.0", ":12"))


class PlanTests(unittest.TestCase):
    def test_map_review_environment_scrubs_then_reasserts_structural_draft(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                harness.STRUCTURAL_DRAFT_ENVIRONMENT: "inherited-wrong-value",
                "HEX_REVIEW_WORLD_DETAIL": "inherited-profile",
            },
        ):
            environment = harness._map_review_environment()
        self.assertEqual(
            environment[harness.STRUCTURAL_DRAFT_ENVIRONMENT],
            harness.STRUCTURAL_DRAFT_VALUE,
        )
        self.assertNotIn("HEX_REVIEW_WORLD_DETAIL", environment)

        plan = harness.build_capture_document(pathlib.Path("/tmp/world-detail-draft-contract"))
        expected = {
            "environment": harness.STRUCTURAL_DRAFT_ENVIRONMENT,
            "value": harness.STRUCTURAL_DRAFT_VALUE,
            "scope": "map-review-only",
        }
        self.assertEqual(plan["provenance"]["structural_draft_runtime"], expected)
        self.assertEqual(
            plan["capture_contract"]["structural_draft_environment"],
            {
                "name": harness.STRUCTURAL_DRAFT_ENVIRONMENT,
                "value": harness.STRUCTURAL_DRAFT_VALUE,
                "scope": "map-review-only",
                "inherited_state_scrubbed_before_injection": True,
            },
        )

    def test_partial_and_complete_slot_accounting(self) -> None:
        root = pathlib.Path("/tmp/world-detail-plan-test")
        partial = harness.build_still_plan(root)
        self.assertEqual(partial["slot_accounting"]["resolved_logical_slots"], 240)
        self.assertEqual(partial["slot_accounting"]["materialized_unique_paths"], 244)
        self.assertEqual(partial["slot_accounting"]["fresh_shared_control_renders"], 4)
        self.assertEqual(partial["slot_accounting"]["new_unique_still_renders"], 248)
        self.assertEqual(partial["slot_accounting"]["unique_non_control_treatment_pngs"], 240)
        self.assertEqual(partial["slot_accounting"]["total_accounted_evidence_pngs"], 252)
        complete = harness.build_still_plan(root, complete_selection())
        self.assertEqual(complete["slot_accounting"]["resolved_logical_slots"], 665)
        self.assertLessEqual(
            complete["slot_accounting"]["unique_non_control_treatment_pngs"],
            harness.MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS,
        )
        self.assertLessEqual(
            complete["slot_accounting"]["total_accounted_evidence_pngs"],
            harness.MAX_TOTAL_ACCOUNTED_EVIDENCE_PNGS,
        )
        self.assertFalse(complete["baseline"]["midnight_included"])

    def test_stage_one_jobs_remain_immutable_when_final_controls_expand(self) -> None:
        output_root = pathlib.Path("/tmp/world-detail-stable-control-jobs")
        raw_root = pathlib.Path("/tmp/world-detail-stable-control-raw")
        partial = harness.build_still_plan(
            output_root,
            raw_capture_root=raw_root,
        )
        complete = harness.build_still_plan(
            output_root,
            complete_selection(),
            raw_capture_root=raw_root,
        )
        partial_jobs = {job["id"]: job for job in partial["jobs"]}
        complete_jobs = {job["id"]: job for job in complete["jobs"]}
        self.assertLessEqual(set(partial_jobs), set(complete_jobs))
        for job_id, partial_job in partial_jobs.items():
            self.assertEqual(partial_job, complete_jobs[job_id], job_id)

        initial_neutral_controls = [
            job
            for job in partial["jobs"]
            if job["stage"] == "00-shared-control"
            and job["lighting"] == "neutral"
        ]
        complete_neutral_controls = [
            job
            for job in complete["jobs"]
            if job["stage"] == "00-shared-control"
            and job["lighting"] == "neutral"
        ]
        self.assertEqual(len(initial_neutral_controls), 4)
        self.assertEqual(len(complete_neutral_controls), 5)
        focused = [
            job for job in initial_neutral_controls if "-focused-" in job["id"]
        ]
        self.assertEqual(len(focused), 4)
        self.assertTrue(all(len(job["cameras"]) == 1 for job in focused))
        self.assertEqual(
            [job["cameras"][0]["id"] for job in focused],
            list(harness.BASELINE_ORACLE_CAMERA_IDS),
        )
        final_extra = next(
            job for job in complete_neutral_controls if "-final-extra-" in job["id"]
        )
        self.assertEqual(len(final_extra["cameras"]), 13)
        self.assertFalse(
            {job["cameras"][0]["id"] for job in focused}
            & {camera["id"] for camera in final_extra["cameras"]}
        )
        verification = partial["verification_jobs"]
        self.assertEqual(len(verification), 4)
        self.assertTrue(all(len(job["cameras"]) == 1 for job in verification))
        self.assertEqual(
            [job["cameras"][0]["id"] for job in verification],
            list(harness.BASELINE_ORACLE_CAMERA_IDS),
        )

    def test_runner_orders_eight_focused_controls_before_treatments(self) -> None:
        plan = harness.build_capture_document(
            pathlib.Path("/tmp/world-detail-fail-fast-ordering"),
            complete_selection(),
        )
        oracle_jobs = harness._baseline_oracle_control_jobs(plan["study"]["jobs"])
        ordered = harness._capture_jobs_in_fail_fast_order(
            plan,
            oracle_jobs,
            include_motion=True,
        )
        expected = [
            *oracle_jobs,
            *plan["study"]["verification_jobs"],
            *[
                job
                for job in plan["study"]["jobs"]
                if job["id"] not in {oracle["id"] for oracle in oracle_jobs}
            ],
            *plan["study"]["reproduction_jobs"],
            *plan["motion"]["jobs"],
        ]
        self.assertEqual(ordered, expected)
        self.assertEqual(
            [job["id"] for job in ordered[:4]],
            [job["id"] for job in oracle_jobs],
        )
        self.assertEqual(
            [job["id"] for job in ordered[4:8]],
            [job["id"] for job in plan["study"]["verification_jobs"]],
        )
        self.assertTrue(all(job["control_profile_omitted"] for job in ordered[:4]))
        self.assertTrue(all(not job["control_profile_omitted"] for job in ordered[4:8]))

    def test_post_render_validation_failure_terminalizes_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            output_root = root / "publication"
            raw_root = root / "raw"
            work_root = root / "work"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            raw_path = raw_root / "runtime" / "raw-stills" / "control.png"
            job = {
                "id": "fixture-render-success-validation-failure",
                "kind": "still",
                "stage": "00-shared-control",
                "look_id": "control",
                "profile_sha256": harness.control_profile().sha256,
                "profile_json": harness.control_profile().canonical_json,
                "control_profile_omitted": True,
                "lighting": "neutral",
                "asset_stage": harness.LIGHTING_CONDITIONS["neutral"].asset_stage,
                "time_hours": harness.LIGHTING_CONDITIONS["neutral"].time_hours,
                "liquid_phase_seconds": 0.0,
                "capture_plan": {
                    "version": 1,
                    "captures": [{"path": str(raw_path)}],
                },
                "raw_capture_root": str(raw_root),
                "cameras": [],
                "artifacts": ["runtime/raw-stills/control.png"],
            }
            plan = {
                "output_root": str(output_root),
                "raw_capture_root": str(raw_root),
                "provenance": {"fixture": True},
                "study": {
                    "jobs": [job],
                    "verification_jobs": [],
                    "reproduction_jobs": [],
                    "slot_accounting": {"fresh_shared_control_renders": 1},
                },
                "motion": {"jobs": []},
            }
            legacy = mock.Mock()

            def renderer_success(*_args, **kwargs):
                pathlib.Path(kwargs["log_path"]).write_text(
                    "renderer exited successfully\n",
                    encoding="utf-8",
                )
                return 0

            legacy.run_logged_process.side_effect = renderer_success
            with mock.patch.object(
                harness,
                "validate_capture_document",
                return_value=plan,
            ), mock.patch.object(
                harness,
                "_stage_asset_root",
                return_value=fixture_asset_stage(root),
            ), mock.patch.object(
                harness,
                "_legacy_harness",
                return_value=legacy,
            ), mock.patch.object(
                harness,
                "_validate_job_artifacts",
                side_effect=harness.HarnessError("post-render validation failed"),
            ):
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "post-render validation failed",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root)

            state = json.loads(
                (work_root / "capture-state.json").read_text(encoding="utf-8")
            )
            self.assertEqual(state["completed"], {})
            self.assertEqual(len(state["attempts"]), 1)
            attempt = state["attempts"][0]
            self.assertEqual(attempt["status"], "FAILED")
            self.assertEqual(
                attempt["returncode"],
                harness.HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE,
            )
            self.assertNotEqual(attempt["returncode"], 0)
            self.assertNotIn("RUNNING", {row["status"] for row in state["attempts"]})

    def test_process_exception_terminalizes_attempt_before_propagation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            output_root = root / "publication"
            raw_root = root / "raw"
            work_root = root / "work"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            raw_path = raw_root / "runtime" / "raw-stills" / "control.png"
            job = {
                "id": "fixture-render-process-exception",
                "kind": "still",
                "stage": "00-shared-control",
                "look_id": "control",
                "profile_sha256": harness.control_profile().sha256,
                "profile_json": harness.control_profile().canonical_json,
                "control_profile_omitted": True,
                "lighting": "neutral",
                "asset_stage": harness.LIGHTING_CONDITIONS["neutral"].asset_stage,
                "time_hours": harness.LIGHTING_CONDITIONS["neutral"].time_hours,
                "liquid_phase_seconds": 0.0,
                "capture_plan": {
                    "version": 1,
                    "captures": [{"path": str(raw_path)}],
                },
                "raw_capture_root": str(raw_root),
                "cameras": [],
                "artifacts": ["runtime/raw-stills/control.png"],
            }
            plan = {
                "output_root": str(output_root),
                "raw_capture_root": str(raw_root),
                "provenance": {"fixture": True},
                "study": {
                    "jobs": [job],
                    "verification_jobs": [],
                    "reproduction_jobs": [],
                    "slot_accounting": {"fresh_shared_control_renders": 1},
                },
                "motion": {"jobs": []},
            }
            legacy = mock.Mock()
            legacy.run_logged_process.side_effect = harness.HarnessError(
                "capture launch failed"
            )
            with mock.patch.object(
                harness,
                "validate_capture_document",
                return_value=plan,
            ), mock.patch.object(
                harness,
                "_stage_asset_root",
                return_value=fixture_asset_stage(root),
            ), mock.patch.object(
                harness,
                "_legacy_harness",
                return_value=legacy,
            ):
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "capture launch failed",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root)

            state = json.loads(
                (work_root / "capture-state.json").read_text(encoding="utf-8")
            )
            self.assertEqual(state["completed"], {})
            self.assertEqual(len(state["attempts"]), 1)
            attempt = state["attempts"][0]
            self.assertEqual(attempt["status"], "FAILED")
            self.assertEqual(
                attempt["returncode"],
                harness.HARNESS_PROCESS_EXCEPTION_RETURN_CODE,
            )
            self.assertEqual(attempt["failure_phase"], "process")
            self.assertEqual(attempt["failure_type"], "HarnessError")
            self.assertNotIn("RUNNING", {row["status"] for row in state["attempts"]})

    def test_capture_runner_lock_is_exclusive_nonblocking_and_reusable(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work_root = pathlib.Path(temporary) / "work"
            with harness._exclusive_capture_runner_lock(work_root):
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "already owned by another runner",
                ):
                    with harness._exclusive_capture_runner_lock(work_root):
                        self.fail("a second runner acquired the same work-root lock")
            with harness._exclusive_capture_runner_lock(work_root):
                pass

    def test_runner_pins_executable_and_rejects_new_and_resumed_mismatches(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            work_root = root / "work"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            first = fixture_capture_job(root, "fixture-first")
            second = fixture_capture_job(root, "fixture-second")
            plan = fixture_capture_plan(root, [first, second])
            executable_a = "a" * 64
            executable_b = "b" * 64
            legacy = mock.Mock()

            def renderer_success(*_args, **kwargs):
                pathlib.Path(kwargs["log_path"]).write_text("ok\n", encoding="utf-8")
                return 0

            legacy.run_logged_process.side_effect = renderer_success
            common_patches = (
                mock.patch.object(harness, "validate_capture_document", return_value=plan),
                mock.patch.object(
                    harness,
                    "_stage_asset_root",
                    return_value=fixture_asset_stage(root),
                ),
                mock.patch.object(harness, "_legacy_harness", return_value=legacy),
                mock.patch.object(harness, "_assert_source_provenance", return_value=None),
            )
            with common_patches[0], common_patches[1], common_patches[2], common_patches[3], mock.patch.object(
                harness,
                "_validate_job_artifacts",
                return_value=fixture_capture_record(executable_a),
            ):
                harness.run_capture_plan(plan_path, work_root=work_root, max_jobs=1)

            state_path = work_root / "capture-state.json"
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(state["pinned_executable_sha256"], executable_a)
            completed = state["completed"][first["id"]]
            attempt = state["attempts"][0]
            for row in (completed, attempt):
                self.assertEqual(row["executable_sha256"], executable_a)
                self.assertEqual(row["asset_stage_tree_sha256"], "a" * 64)
                self.assertEqual(
                    row["asset_stage_manifest_sha256"],
                    harness.sha256_file(root / "stage-manifest.json"),
                )

            with common_patches[0], common_patches[1], common_patches[2], common_patches[3], mock.patch.object(
                harness,
                "_validate_job_artifacts",
                side_effect=[
                    fixture_capture_record(executable_a),
                    fixture_capture_record(executable_b),
                ],
            ):
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "not pinned executable",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root)
            state = json.loads(state_path.read_text(encoding="utf-8"))
            self.assertEqual(state["pinned_executable_sha256"], executable_a)
            self.assertEqual(set(state["completed"]), {first["id"]})
            self.assertEqual(state["attempts"][-1]["status"], "FAILED")
            self.assertEqual(
                state["attempts"][-1]["returncode"],
                harness.HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE,
            )

            state["completed"][first["id"]]["executable_sha256"] = executable_b
            state_path.write_text(harness.pretty_json(state), encoding="utf-8")
            with common_patches[0], common_patches[1], common_patches[2], common_patches[3]:
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "not pinned executable",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root, max_jobs=1)

    def test_runner_pins_challenger_before_oracle_comparison_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            work_root = root / "work"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            still = harness.build_still_plan(
                root / "publication",
                raw_capture_root=root / "raw",
            )
            oracle_jobs = harness._baseline_oracle_control_jobs(still["jobs"])
            plan = fixture_capture_plan(root, list(oracle_jobs))
            executable_sha256 = "a" * 64
            legacy = mock.Mock()

            def renderer_success(*_args, **kwargs):
                pathlib.Path(kwargs["log_path"]).write_text("ok\n", encoding="utf-8")
                return 0

            legacy.run_logged_process.side_effect = renderer_success
            with mock.patch.object(
                harness,
                "validate_capture_document",
                return_value=plan,
            ), mock.patch.object(
                harness,
                "_stage_asset_root",
                return_value=fixture_asset_stage(root),
            ), mock.patch.object(
                harness,
                "_legacy_harness",
                return_value=legacy,
            ), mock.patch.object(
                harness,
                "_validate_job_artifacts",
                return_value=fixture_capture_record(executable_sha256),
            ), mock.patch.object(
                harness,
                "_validate_baseline_oracle_equivalence",
                side_effect=harness.HarnessError("raster-stable oracle rejection"),
            ):
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "raster-stable oracle rejection",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root)

            state = json.loads(
                (work_root / "capture-state.json").read_text(encoding="utf-8")
            )
            self.assertEqual(state["pinned_executable_sha256"], executable_sha256)
            self.assertEqual(state["completed"], {})
            attempt = state["attempts"][0]
            self.assertEqual(attempt["status"], "FAILED")
            self.assertEqual(attempt["failure_phase"], "post-render-validation")
            self.assertEqual(attempt["failure_type"], "HarnessError")
            self.assertEqual(
                attempt["returncode"],
                harness.HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE,
            )

    def test_scaffolded_partial_plan_json_reaches_first_renderer_launch(self) -> None:
        """Exercise the serialized scaffold shape, including JSON list offsets."""

        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary).resolve()
            output_root = root / "publication"
            raw_root = root / "raw"
            work_root = root / "work"
            scaffold = harness.scaffold_report(
                output_root,
                raw_capture_root=raw_root,
            )
            plan_path = pathlib.Path(scaffold["capture_plan"])
            serialized = json.loads(plan_path.read_text(encoding="utf-8"))
            first_job = harness._baseline_oracle_control_jobs(
                serialized["study"]["jobs"]
            )[0]
            self.assertIsInstance(first_job["cameras"][0]["look_at_offset"], list)
            changed_camera = json.loads(json.dumps(serialized))
            changed_camera["study"]["jobs"][0]["cameras"][0][
                "look_at_offset"
            ][0] += 1.0
            with self.assertRaisesRegex(
                harness.HarnessError, "camera definition changed"
            ):
                harness._baseline_oracle_control_jobs(
                    changed_camera["study"]["jobs"]
                )

            stage_root, stage_manifest = fixture_asset_stage(root)
            legacy = harness._legacy_harness()
            with mock.patch.object(
                harness,
                "_validate_baseline_oracle_pack",
                return_value={"fixture": "not-consumed-before-launch"},
            ), mock.patch.object(
                harness,
                "_stage_asset_root",
                return_value=(stage_root, stage_manifest),
            ), mock.patch.object(
                legacy,
                "run_logged_process",
                side_effect=harness.HarnessError(
                    "serialized scaffold reached renderer launch"
                ),
            ) as run_logged_process:
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "serialized scaffold reached renderer launch",
                ):
                    harness.run_capture_plan(
                        plan_path,
                        work_root=work_root,
                        max_jobs=1,
                    )
            run_logged_process.assert_called_once()
            command = run_logged_process.call_args.args[0]
            environment = run_logged_process.call_args.kwargs["environment"]
            self.assertEqual(
                command,
                (
                    "cargo",
                    "run",
                    "--locked",
                    "--release",
                    "-p",
                    "hex_game",
                    "--features",
                    "map-review",
                ),
            )
            self.assertEqual(
                environment["HEX_REVIEW_CAPTURE_PLAN"],
                harness.compact_json(first_job["capture_plan"]),
            )
            state = json.loads(
                (work_root / "capture-state.json").read_text(encoding="utf-8")
            )
            self.assertEqual(state["attempts"][0]["job_id"], first_job["id"])
            self.assertEqual(state["attempts"][0]["failure_phase"], "process")

    def test_asset_stage_is_revalidated_after_process_and_mutation_fails_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            work_root = root / "work"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            job = fixture_capture_job(root, "fixture-stage-mutation")
            plan = fixture_capture_plan(root, [job])
            original = fixture_stage_manifest(tree_sha256="a" * 64)
            changed = fixture_stage_manifest(tree_sha256="f" * 64)
            stage_root, _ = fixture_asset_stage(root, original)
            legacy = mock.Mock()

            def renderer_success(*_args, **kwargs):
                pathlib.Path(kwargs["log_path"]).write_text("ok\n", encoding="utf-8")
                return 0

            legacy.run_logged_process.side_effect = renderer_success
            with mock.patch.object(
                harness,
                "validate_capture_document",
                return_value=plan,
            ), mock.patch.object(
                harness,
                "_stage_asset_root",
                side_effect=[
                    (stage_root, original),
                    (stage_root, original),
                    (stage_root, changed),
                ],
            ), mock.patch.object(
                harness,
                "_legacy_harness",
                return_value=legacy,
            ), mock.patch.object(
                harness,
                "_validate_job_artifacts",
            ) as validate_artifacts:
                with self.assertRaisesRegex(
                    harness.HarnessError,
                    "asset stage binding changed during capture",
                ):
                    harness.run_capture_plan(plan_path, work_root=work_root)
            validate_artifacts.assert_not_called()
            state = json.loads(
                (work_root / "capture-state.json").read_text(encoding="utf-8")
            )
            self.assertEqual(state["completed"], {})
            attempt = state["attempts"][0]
            self.assertEqual(attempt["status"], "FAILED")
            self.assertEqual(attempt["failure_phase"], "asset-stage-post-process")
            self.assertEqual(attempt["asset_stage_tree_sha256"], "a" * 64)
            self.assertEqual(
                attempt["returncode"],
                harness.HARNESS_POST_VALIDATION_FAILURE_RETURN_CODE,
            )

    def test_clean_source_oracle_allows_only_enumerated_raster_ambiguity(self) -> None:
        output_root = pathlib.Path("/tmp/world-detail-baseline-oracle-test")
        plan = harness.build_capture_document(output_root)
        jobs = harness._baseline_oracle_control_jobs(plan["study"]["jobs"])
        contract, oracle = harness._baseline_oracle_documents()
        validation = harness._validate_baseline_oracle_pack()
        self.assertTrue(validation["all_declared_files_hash_verified"])
        self.assertTrue(validation["exact_inventory_verified"])
        self.assertTrue(validation["no_symlinks_verified"])
        self.assertEqual(validation["regular_file_count"], 222)
        self.assertEqual(validation["directory_count"], 89)
        self.assertEqual(validation["total_file_bytes"], 284_483_586)
        self.assertEqual(
            validation["inventory_manifest_sha256"],
            "36dbaa153e4c8cfffbd3dce2246e6acbc9695d8f329b5e3ea997f84f2561c93d",
        )
        provenance = harness._baseline_oracle_provenance_binding()
        self.assertEqual(provenance["manifest_sha256"], contract["external_manifest_sha256"])
        self.assertEqual(provenance["source_git_head"], contract["source_git_head"])
        records = {}
        for index, job in enumerate(jobs):
            camera_id = job["cameras"][0]["id"]
            camera = oracle["cameras"][camera_id]
            records[job["id"]] = {
                "pngs": [
                    {
                        "path": str(
                            harness.BASELINE_ORACLE_ROOT / camera["runs"][0]["path"]
                        ),
                        "sha256": camera["runs"][0]["png_sha256"],
                    }
                ],
                "runtime_receipt": fixture_runtime_receipt(job, index),
            }
        partial = harness._validate_baseline_oracle_equivalence(
            jobs,
            dict(list(records.items())[:3]),
            pack_validation=validation,
        )
        self.assertIsNone(partial)
        evidence = harness._validate_baseline_oracle_equivalence(
            jobs,
            records,
            pack_validation=validation,
        )
        self.assertIsNotNone(evidence)
        self.assertTrue(evidence["fresh_process_per_camera"])
        self.assertEqual(evidence["fresh_process_count"], 4)
        self.assertEqual(
            [row["camera_id"] for row in evidence["comparisons"]],
            list(harness.BASELINE_ORACLE_CAMERA_IDS),
        )
        self.assertTrue(evidence["raster_stable_pixel_equality"])
        self.assertFalse(evidence["broad_numeric_threshold_used"])
        self.assertEqual(
            evidence["evidence_sha256"],
            harness.sha256_object(
                {key: value for key, value in evidence.items() if key != "evidence_sha256"}
            ),
        )
        completed = {
            job["id"]: {
                "job_sha256": harness.sha256_object(job),
                "artifact_sha256": [records[job["id"]]["pngs"][0]["sha256"]],
                "launch_nonce": records[job["id"]]["runtime_receipt"]["launch_nonce"],
                "process_id": records[job["id"]]["runtime_receipt"]["process_id"],
                "runtime_receipt_sha256": records[job["id"]]["runtime_receipt"][
                    "receipt_sha256"
                ],
                "executable_sha256": records[job["id"]]["runtime_receipt"][
                    "executable_sha256"
                ],
            }
            for job in jobs
        }
        harness._validate_current_baseline_oracle_evidence(
            evidence,
            jobs,
            completed,
            pack_validation=validation,
            pinned_executable_sha256="a" * 64,
        )
        stale_grouped = json.loads(json.dumps(evidence))
        stale_grouped["fresh_process_per_camera"] = False
        stale_grouped["fresh_process_count"] = 1
        stale_body = {
            key: value
            for key, value in stale_grouped.items()
            if key != "evidence_sha256"
        }
        stale_grouped["evidence_sha256"] = harness.sha256_object(stale_body)
        with self.assertRaisesRegex(harness.HarnessError, "fresh_process_per_camera"):
            harness._validate_current_baseline_oracle_evidence(
                stale_grouped,
                jobs,
                completed,
                pack_validation=validation,
                pinned_executable_sha256="a" * 64,
            )

        with tempfile.TemporaryDirectory() as temporary:
            from PIL import Image

            changed_path = pathlib.Path(temporary) / "changed.png"
            first_job_id = jobs[0]["id"]
            source_path = pathlib.Path(records[first_job_id]["pngs"][0]["path"])
            with Image.open(source_path) as source:
                changed = source.convert("RGB")
            red, green, blue = changed.getpixel((0, 0))
            changed.putpixel((0, 0), ((red + 1) % 256, green, blue))
            changed.save(changed_path)
            changed_records = json.loads(json.dumps(records))
            changed_records[first_job_id]["pngs"][0]["path"] = str(changed_path)
            changed_records[first_job_id]["pngs"][0]["sha256"] = (
                harness.sha256_file(changed_path)
            )
            with self.assertRaisesRegex(
                harness.HarnessError,
                "differs at raster-stable pixel 0,0",
            ):
                harness._validate_baseline_oracle_equivalence(
                    jobs,
                    changed_records,
                    pack_validation=validation,
                )

            camera = oracle["cameras"][harness.BASELINE_ORACLE_CAMERA_IDS[0]]
            pixel = camera["ambiguous_pixels"][0]
            allowed_path = pathlib.Path(temporary) / "allowed.png"
            with Image.open(source_path) as source:
                allowed = source.convert("RGB")
            allowed.putpixel((pixel["x"], pixel["y"]), tuple(pixel["allowed_rgb"][1]))
            allowed.save(allowed_path)
            allowed_records = json.loads(json.dumps(records))
            allowed_records[first_job_id]["pngs"][0]["path"] = str(allowed_path)
            allowed_records[first_job_id]["pngs"][0]["sha256"] = (
                harness.sha256_file(allowed_path)
            )
            allowed_evidence = harness._validate_baseline_oracle_equivalence(
                jobs,
                allowed_records,
                pack_validation=validation,
            )
            self.assertTrue(allowed_evidence["comparisons"][0]["stable_pixel_identical"])

        duplicate_receipts = json.loads(json.dumps(records))
        duplicate_receipts[jobs[1]["id"]]["runtime_receipt"] = duplicate_receipts[
            jobs[0]["id"]
        ]["runtime_receipt"]
        with self.assertRaisesRegex(harness.HarnessError, "runtime receipt"):
            harness._validate_baseline_oracle_equivalence(
                jobs,
                duplicate_receipts,
                pack_validation=validation,
            )

    def test_control_equivalence_pairs_eight_fresh_one_camera_processes(self) -> None:
        plan = json.loads(
            harness.compact_json(
                harness.build_capture_document(
                    pathlib.Path("/tmp/world-detail-control-equivalence-test")
                )
            )
        )
        omitted_jobs = harness._baseline_oracle_control_jobs(
            plan["study"]["jobs"]
        )
        explicit_jobs = plan["study"]["verification_jobs"]
        self.assertEqual(len(omitted_jobs), 4)
        self.assertEqual(len(explicit_jobs), 4)
        _, oracle = harness._baseline_oracle_documents()
        stable_report = {
            "sha256": "d" * 64,
            "profile_hash_sha256": harness.CONTROL_PROFILE_SHA256,
            "authority": {"fixture": "unchanged"},
            "counts": {"fixture": 1},
            "anchor_heights": {"fixture": 2},
            "anchor_classes": {"fixture": "natural"},
            "projection_hashes": {"fixture": "e" * 64},
            "effect_validation": {
                "cloud_coverage": None,
                "ice_coverage": None,
                "fog_coverage": None,
                "waterfall_anchors": [],
            },
            "camera_features": {"fixture": False},
            "cleanup": {"fixture": "complete"},
        }
        records = {}
        for index, job in enumerate((*omitted_jobs, *explicit_jobs)):
            camera_id = job["cameras"][0]["id"]
            primary = next(
                run
                for run in oracle["cameras"][camera_id]["runs"]
                if run["role"] == "primary_reference"
            )
            records[job["id"]] = {
                "pngs": [
                    {
                        "path": str(harness.BASELINE_ORACLE_ROOT / primary["path"]),
                        "sha256": primary["png_sha256"],
                    }
                ],
                "reports": [dict(stable_report)],
                "runtime_receipt": fixture_runtime_receipt(job, index),
            }
        camera_id = "14-cascade-basin-full-height"
        omitted_camera_job = next(
            job for job in omitted_jobs if job["cameras"][0]["id"] == camera_id
        )
        explicit_camera_job = next(
            job for job in explicit_jobs if job["cameras"][0]["id"] == camera_id
        )
        qualified_root = harness.CONTROL_EQUIVALENCE_QUALIFICATION_ROOT
        omitted_qualified = (
            qualified_root / "run-03-omitted" / f"{camera_id}.png"
        )
        explicit_qualified = (
            qualified_root / "run-03-explicit" / f"{camera_id}.png"
        )
        records[omitted_camera_job["id"]]["pngs"][0] = {
            "path": str(omitted_qualified),
            "sha256": harness.sha256_file(omitted_qualified),
        }
        records[explicit_camera_job["id"]]["pngs"][0] = {
            "path": str(explicit_qualified),
            "sha256": harness.sha256_file(explicit_qualified),
        }
        evidence = harness._validate_control_equivalence_for_plan(plan, records)
        self.assertTrue(evidence["fresh_process_per_png"])
        self.assertEqual(evidence["fresh_process_count"], 8)
        self.assertEqual(len(evidence["comparisons"]), 4)
        self.assertEqual(
            [row["camera_id"] for row in evidence["comparisons"]],
            list(harness.BASELINE_ORACLE_CAMERA_IDS),
        )
        camera_comparison = next(
            row for row in evidence["comparisons"] if row["camera_id"] == camera_id
        )
        self.assertEqual(camera_comparison["ambiguous_pixel_count"], 5)
        self.assertEqual(camera_comparison["differing_ambiguous_pixel_count"], 4)
        self.assertEqual(
            evidence["raster_contract"]["qualified_pixels"],
            [dict(row) for row in harness.CONTROL_EQUIVALENCE_QUALIFIED_PIXELS],
        )
        self.assertTrue(
            evidence["raster_contract"]["baseline_oracle_contract_unchanged"]
        )

        with tempfile.TemporaryDirectory() as temporary:
            from PIL import Image

            unknown_path = pathlib.Path(temporary) / "unknown-coordinate.png"
            with Image.open(explicit_qualified) as source:
                unknown = source.convert("RGB")
            red, green, blue = unknown.getpixel((0, 0))
            unknown.putpixel((0, 0), ((red + 1) % 256, green, blue))
            unknown.save(unknown_path)
            unknown_records = json.loads(json.dumps(records))
            unknown_records[explicit_camera_job["id"]]["pngs"][0] = {
                "path": str(unknown_path),
                "sha256": harness.sha256_file(unknown_path),
            }
            with self.assertRaisesRegex(
                harness.HarnessError,
                "differs at raster-stable pixel 0,0",
            ):
                harness._validate_control_equivalence_for_plan(plan, unknown_records)

            wrong_rgb_path = pathlib.Path(temporary) / "wrong-qualified-rgb.png"
            with Image.open(explicit_qualified) as source:
                wrong_rgb = source.convert("RGB")
            wrong_rgb.putpixel((1438, 273), (165, 95, 66))
            wrong_rgb.save(wrong_rgb_path)
            wrong_records = json.loads(json.dumps(records))
            wrong_records[explicit_camera_job["id"]]["pngs"][0] = {
                "path": str(wrong_rgb_path),
                "sha256": harness.sha256_file(wrong_rgb_path),
            }
            with self.assertRaisesRegex(
                harness.HarnessError,
                "unobserved shared-vertex RGB value at 1438,273",
            ):
                harness._validate_control_equivalence_for_plan(plan, wrong_records)

        duplicate_nonce = json.loads(json.dumps(records))
        first_receipt = duplicate_nonce[omitted_jobs[0]["id"]]["runtime_receipt"]
        second_receipt = duplicate_nonce[explicit_jobs[0]["id"]]["runtime_receipt"]
        second_receipt["launch_nonce"] = first_receipt["launch_nonce"]
        receipt_body = {
            key: value
            for key, value in second_receipt.items()
            if key != "receipt_sha256"
        }
        second_receipt["receipt_sha256"] = harness.sha256_bytes(
            harness.compact_json(receipt_body).encode("utf-8")
        )
        with self.assertRaisesRegex(harness.HarnessError, "eight fresh launches"):
            harness._validate_control_equivalence_for_plan(plan, duplicate_nonce)

    def test_control_equivalence_qualification_pack_rejects_tampering(self) -> None:
        validation = harness._validate_control_equivalence_qualification_pack()
        self.assertEqual(validation["pair_count"], 3)
        self.assertEqual(validation["run_count"], 6)
        self.assertEqual(validation["regular_file_count"], 30)
        self.assertEqual(validation["directory_count"], 12)
        self.assertEqual(validation["total_file_bytes"], 17_607_909)
        self.assertTrue(validation["exact_inventory_verified"])
        self.assertTrue(validation["no_symlinks_verified"])
        self.assertFalse(validation["broad_numeric_threshold_used"])

        with tempfile.TemporaryDirectory() as temporary:
            copied = pathlib.Path(temporary) / "control-equivalence-qualification-v7c"
            shutil.copytree(harness.CONTROL_EQUIVALENCE_QUALIFICATION_ROOT, copied)
            log_path = copied / "run-01-omitted" / "launch.log"
            log_path.write_bytes(log_path.read_bytes() + b"tamper\n")
            with self.assertRaisesRegex(
                harness.HarnessError,
                "exact inventory changed",
            ):
                harness._validate_control_equivalence_qualification_pack(copied)

    def test_every_combination_inherits_active_family_final_17_vetoes(self) -> None:
        selection = complete_selection()
        plan = {"selection": selection}
        passes = {family: True for family in harness.FAMILY_ORDER}
        passes["snow"] = False
        for combination_id in harness.COMBINATION_IDS:
            self.assertFalse(
                harness._combination_atomic_final_17_compatible(
                    plan,
                    combination_id,
                    passes,
                )
            )

        selection["combinations"]["restrained"]["snow"] = "control"
        self.assertTrue(
            harness._combination_atomic_final_17_compatible(
                plan,
                "restrained",
                passes,
            )
        )

    def test_maximally_distinct_selection_passes_both_honest_ceilings(self) -> None:
        plan = harness.build_still_plan(
            pathlib.Path("/tmp/world-detail-maximum-accounting-test"),
            maximally_distinct_selection(),
        )
        accounting = plan["slot_accounting"]
        self.assertEqual(accounting["resolved_logical_slots"], 665)
        self.assertEqual(accounting["unique_non_control_treatment_pngs"], 596)
        self.assertEqual(accounting["unique_non_control_treatment_png_ceiling"], 611)
        self.assertEqual(accounting["total_accounted_evidence_pngs"], 630)
        self.assertEqual(accounting["total_accounted_evidence_png_ceiling"], 630)

    def test_over_cap_non_control_treatment_accounting_is_rejected(self) -> None:
        with mock.patch.object(
            harness,
            "MAX_UNIQUE_NON_CONTROL_TREATMENT_PNGS",
            595,
        ), self.assertRaisesRegex(
            harness.HarnessError,
            r"595 unique non-control treatment-PNG ceiling: 596 treatment PNGs",
        ):
            harness.build_still_plan(
                pathlib.Path("/tmp/world-detail-over-cap-treatment-test"),
                maximally_distinct_selection(),
            )

    def test_selection_performance_uses_fresh_camera_02_control(self) -> None:
        plan = harness.build_capture_document(
            pathlib.Path("/tmp/world-detail-selection-performance-test")
        )

        def record_for(job, *_args, **_kwargs):
            return {
                "performance_samples": [
                    {
                        "frame_time_ms": 10.0 + index,
                        "resident_presentation_bytes": 100 + index,
                        "warmup_complete": True,
                    }
                    for index, _artifact in enumerate(job["artifacts"])
                ]
            }

        with tempfile.TemporaryDirectory() as temporary:
            plan_path = pathlib.Path(temporary) / "plan.json"
            plan_path.write_text("{}", encoding="utf-8")
            with mock.patch.object(
                harness, "validate_capture_document", return_value=plan
            ), mock.patch.object(
                harness, "_validate_job_artifacts", side_effect=record_for
            ) as validate_job:
                evidence = harness.build_selection_performance_evidence(plan_path)
        self.assertEqual(
            evidence["subjects"]["control"],
            {
                "p95_frame_time_ms": 10.0,
                "max_resident_presentation_bytes": 100,
            },
        )
        control_calls = [
            call.args[0]
            for call in validate_job.call_args_list
            if call.args[0]["stage"] == "00-shared-control"
        ]
        self.assertEqual(len(control_calls), 1)
        self.assertTrue(
            any(
                artifact.endswith("/02-highlands-oblique.png")
                for artifact in control_calls[0]["artifacts"]
            )
        )

    def test_reproduction_requires_a_distinct_runtime_receipt(self) -> None:
        plan = harness.build_capture_document(
            pathlib.Path("/tmp/world-detail-reproduction-test"), complete_selection()
        )
        reproduction_job = plan["study"]["reproduction_jobs"][0]
        source_job = next(
            job
            for job in (
                *plan["study"]["jobs"],
                *plan["study"]["verification_jobs"],
            )
            if job["id"] == reproduction_job["reference_job_id"]
        )

        def receipt(nonce: str, digest: str) -> dict:
            return {
                "version": 1,
                "launch_nonce": nonce,
                "process_id": 100 if nonce.startswith("1") else 101,
                "executable_sha256": "a" * 64,
                "source_provenance_sha256": "b" * 64,
                "capture_plan_sha256": digest,
                "profile_sha256": reproduction_job["profile_sha256"],
                "receipt_sha256": digest,
            }

        source_record = {
            "pngs": [
                {"path": f"/tmp/source-{index}.png"}
                for index in range(len(source_job["artifacts"]))
            ],
            "reports": [
                {"sha256": f"{index:064x}"}
                for index in range(len(source_job["artifacts"]))
            ],
            "runtime_receipt": receipt("1" * 64, "c" * 64),
        }
        rerun_record = {
            "pngs": [{"path": "/tmp/reproduction.png"}],
            "reports": [{"sha256": "d" * 64}],
            "runtime_receipt": receipt("2" * 64, "e" * 64),
        }
        records = {
            source_job["id"]: source_record,
            reproduction_job["id"]: rerun_record,
        }
        stable = {
            "version": 1,
            "warning": harness.WARNING,
            "still_sha256": "f" * 64,
            "decoded_rgb_sha256": "0" * 64,
            "report_state_sha256": "1" * 64,
            "projection_hashes": {},
            "effect_validation": {},
        }
        with mock.patch.object(harness, "verify_reproduction", return_value=stable):
            result = harness._validate_reproduction_for_plan(plan, records)
            self.assertEqual(result["status"], "REPRODUCED")
            rerun_record["runtime_receipt"] = dict(source_record["runtime_receipt"])
            with self.assertRaisesRegex(harness.HarnessError, "fresh runtime launch"):
                harness._validate_reproduction_for_plan(plan, records)

    def test_control_score_leader_reproduces_fresh_explicit_camera_02(self) -> None:
        selection = complete_selection()
        for family in harness.FAMILY_ORDER:
            selection["atomic_winners"][family] = "control"
            selection["pre_motion_atomic_winners"][family] = "control"
            for combination_id in harness.COMBINATION_IDS:
                selection["combinations"][combination_id][family] = "control"
                selection["pre_motion_combinations"][combination_id][family] = (
                    "control"
                )
        stamp_review_evidence(selection)
        plan = harness.build_capture_document(
            pathlib.Path("/tmp/world-detail-control-reproduction-test"), selection
        )
        reproduction = plan["study"]["reproduction_jobs"][0]
        self.assertEqual(
            reproduction["reference_job_id"],
            "control-verification-explicit-current-02-highlands-oblique",
        )
        reference = next(
            job
            for job in plan["study"]["verification_jobs"]
            if job["id"] == reproduction["reference_job_id"]
        )
        self.assertEqual(len(reference["artifacts"]), 1)
        self.assertEqual(
            reference["artifacts"].index(reproduction["reference_artifact"]), 0
        )

    def test_motion_matrix_has_twenty_two_exact_orbits(self) -> None:
        motion = harness.build_motion_plan(pathlib.Path("/tmp/world-detail-motion-test"), complete_selection())
        self.assertEqual(len(motion["clips"]), 22)
        self.assertEqual(len(motion["jobs"]), motion["total_sequence_launches"])
        self.assertEqual(motion["total_sequence_launches"], 28)
        self.assertEqual(motion["total_frame_captures"], 28 * 90)
        self.assertEqual(motion["shared_control_orbits"], 6)
        for clip in motion["clips"]:
            self.assertEqual(clip["orbit_start_degrees"], -10.0)
            self.assertEqual(clip["orbit_end_degrees"], 10.0)
            by_id = {job["id"]: job for job in motion["jobs"]}
            clip_job = by_id[clip["candidate_job_ids"][0]]
            first_offset = clip_job["cameras"][0]["look_at_offset"]
            last_offset = clip_job["cameras"][-1]["look_at_offset"]
            self.assertAlmostEqual(
                sum(value * value for value in first_offset),
                sum(value * value for value in last_offset),
                places=8,
            )
            captures = clip_job["capture_plan"]["captures"]
            self.assertEqual(clip_job["capture_plan"]["version"], 2)
            self.assertEqual(captures[0]["liquid_phase_seconds"], 0.0)
            self.assertAlmostEqual(captures[-1]["liquid_phase_seconds"], 89 / 30)
            self.assertEqual(captures[0]["settle_frames"], 90)
            self.assertTrue(all(entry["settle_frames"] == 2 for entry in captures[1:]))
        invalid = json.loads(json.dumps(motion["jobs"][0]))
        invalid["capture_plan"]["captures"][0]["settle_frames"] = 91
        with self.assertRaises(harness.HarnessError):
            harness._job_cameras(invalid, harness.load_camera_sets()[0])

    def test_capture_document_roundtrip_and_tamper_detection(self) -> None:
        plan = harness.build_capture_document(pathlib.Path("/tmp/world-detail-integrity-test"))
        serialized = json.loads(harness.compact_json(plan))
        harness.validate_capture_document(serialized)
        self.assertEqual(
            plan["capture_contract"]["cargo_command"],
            [
                "cargo",
                "run",
                "--locked",
                "--release",
                "-p",
                "hex_game",
                "--features",
                "map-review",
            ],
        )
        for field in (
            "worktree_status_sha256",
            "tracked_dirty_diff_sha256",
            "untracked_file_sha256",
            "untracked_manifest_sha256",
        ):
            self.assertIn(field, plan["provenance"])
        oracle_provenance = plan["provenance"]["baseline_oracle"]
        self.assertEqual(oracle_provenance["version"], 1)
        self.assertEqual(
            oracle_provenance["contract_sha256"],
            harness.sha256_file(harness.BASELINE_ORACLE_CONTRACT_PATH),
        )
        self.assertTrue(oracle_provenance["private_pack_files_are_not_published"])
        self.assertEqual(
            plan["capture_contract"]["focused_control_process_policy"],
            {
                "camera_ids": list(harness.BASELINE_ORACLE_CAMERA_IDS),
                "omitted_profile_jobs": 4,
                "explicit_current_jobs": 4,
                "captures_per_job": 1,
                "fresh_process_per_png": True,
                "runtime_receipts_must_be_distinct": True,
            },
        )
        serialized["study"]["seed"] += 1
        with self.assertRaises(harness.HarnessError):
            harness.validate_capture_document(serialized)

    def test_gallery_html_embeds_data_without_fetch(self) -> None:
        gallery = {
            "version": 1,
            "warning": harness.WARNING,
            "status": "TEST",
            "slots": [
                {
                    "logical_slot": "test-slot",
                    "stage": "test-stage",
                    "look_id": "test-look",
                    "profile_id": "control",
                    "family": "control",
                    "lighting": "neutral",
                    "camera_id": "02-highlands-oblique",
                    "image": "../source-pngs/test.png",
                    "reuse": True,
                    "fresh_control": True,
                }
            ],
        }
        html = harness._gallery_html(gallery)
        self.assertNotIn("fetch(", html)
        self.assertIn(harness.compact_json(gallery), html)

    def test_reused_asset_stage_is_bound_to_current_source_and_condition(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            work_root = pathlib.Path(temporary).resolve()
            condition = harness.LIGHTING_CONDITIONS["neutral"]
            _stage, manifest = harness._stage_asset_root(work_root, condition)
            manifest_path = (
                work_root
                / "asset-stages"
                / condition.asset_stage
                / "stage-manifest.json"
            )
            tampered = json.loads(json.dumps(manifest))
            tampered["source_asset_tree_sha256"] = "0" * 64
            manifest_path.write_text(json.dumps(tampered), encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness._stage_asset_root(work_root, condition)


class ReviewAndLifecycleTests(unittest.TestCase):
    @staticmethod
    def _code(
        subject_id: str,
        condition: str,
        scoring_context: str = harness.DIAGNOSTIC_REVIEW_CONTEXT,
    ) -> str:
        return "RV-" + harness.sha256_bytes(
            f"{subject_id}:{condition}:{scoring_context}".encode()
        )[:12].upper()

    @classmethod
    def _review(cls, reviewer_id: str, *, phase: int) -> dict:
        ratings = []

        def add(
            subject_id: str,
            condition: str,
            score: int,
            scoring_context: str = harness.DIAGNOSTIC_REVIEW_CONTEXT,
        ) -> None:
            ratings.append(
                {
                    "code": cls._code(subject_id, condition, scoring_context),
                    "categories": {field: score for field in harness.CATEGORY_ORDER},
                }
            )

        add("control", "neutral", 3)
        for family in harness.FAMILY_ORDER:
            profiles = [profile for profile in harness.atomic_profiles() if profile.family == family]
            for index, profile in enumerate(profiles):
                add(profile.id, "neutral", 5 if index == 0 else 4 if index == 1 else 3)
            if phase >= 2:
                for profile, score in ((profiles[0], 5), (profiles[1], 4)):
                    add(profile.id, "golden", score)
                    add(profile.id, "overcast", score)
        if phase >= 2:
            add("control", "golden", 3)
            add("control", "overcast", 3)
        if phase >= 3:
            for subject_id, _predecessor, _families in harness.LADDER_STEPS:
                add(subject_id, "neutral", 5)
        if phase >= 4:
            add(
                "control",
                "neutral",
                3,
                harness.FINAL_REVIEW_CONTEXT,
            )
            for family in harness.FAMILY_ORDER:
                add(
                    f"winner-{family}",
                    "neutral",
                    4,
                    harness.FINAL_REVIEW_CONTEXT,
                )
            for combination_id in harness.COMBINATION_IDS:
                for condition in harness.LIGHTING_CONDITIONS:
                    add(f"combination-{combination_id}", condition, 4)
                add(
                    f"combination-{combination_id}",
                    "neutral",
                    4,
                    harness.FINAL_REVIEW_CONTEXT,
                )
        return {
            "version": 1,
            "warning": harness.WARNING,
            "reviewer_id": reviewer_id,
            "blinded": True,
            "independence_attestation": True,
            "unscored_camera_ids": list(harness.UNSCORED_CAMERA_IDS),
            "scoring_contract_sha256": harness.sha256_object(harness.scoring_contract()),
            "review_packet_sha256": "a" * 64,
            "ratings": ratings,
        }

    @classmethod
    def _packet_evidence(cls, review: dict) -> dict:
        identities = {}
        for rating in review["ratings"]:
            code = rating["code"]
            # Tests own the reverse map; reviewer rows remain opaque-code only.
            for condition in harness.LIGHTING_CONDITIONS:
                candidates = ["control", *(profile.id for profile in harness.atomic_profiles())]
                candidates.extend(step[0] for step in harness.LADDER_STEPS)
                candidates.extend(f"winner-{family}" for family in harness.FAMILY_ORDER)
                candidates.extend(
                    f"combination-{identifier}" for identifier in harness.COMBINATION_IDS
                )
                match = next(
                    (
                        (subject_id, scoring_context)
                        for subject_id in candidates
                        for scoring_context in (
                            harness.DIAGNOSTIC_REVIEW_CONTEXT,
                            harness.FINAL_REVIEW_CONTEXT,
                        )
                        if cls._code(subject_id, condition, scoring_context) == code
                    ),
                    None,
                )
                if match is not None:
                    subject_id, scoring_context = match
                    identities[code] = {
                        "subject_id": subject_id,
                        "subject_kind": harness._review_subject_kind(subject_id),
                        "condition": condition,
                        "scoring_context": scoring_context,
                    }
                    break
        return {
            "version": 1,
            "warning": harness.WARNING,
            "packet_path": "/tmp/fixture-packet/packet.json",
            "packet_sha256": "a" * 64,
            "unblind_map_path": "/tmp/fixture-unblind.json",
            "unblind_map_sha256": "b" * 64,
            "code_map": identities,
            "entry_count": len(identities),
        }

    @classmethod
    def _motion_review(
        cls,
        reviewer_id: str,
        plan: dict,
        *,
        fail_clip_id=None,
    ) -> dict:
        return {
            "version": 1,
            "warning": harness.WARNING,
            "reviewer_id": reviewer_id,
            "blinded": True,
            "independence_attestation": True,
            "unscored_camera_ids": list(harness.UNSCORED_CAMERA_IDS),
            "scoring_contract_sha256": harness.sha256_object(harness.scoring_contract()),
            "review_packet_sha256": "d" * 64,
            "ratings": [
                {
                    "code": cls._code(
                        f"motion-{clip['id']}",
                        clip["lighting"],
                        harness.MOTION_REVIEW_CONTEXT,
                    ),
                    "categories": {
                        field: (2 if clip["id"] == fail_clip_id else 4)
                        for field in harness.CATEGORY_ORDER
                    },
                }
                for clip in plan["motion"]["clips"]
            ],
        }

    @classmethod
    def _motion_packet_evidence(cls, plan: dict) -> dict:
        code_map = {
            cls._code(
                f"motion-{clip['id']}",
                clip["lighting"],
                harness.MOTION_REVIEW_CONTEXT,
            ): {
                "clip_id": clip["id"],
                "profile_id": clip["profile_id"],
                "lighting": clip["lighting"],
                "camera_id": clip["camera_id"],
            }
            for clip in plan["motion"]["clips"]
        }
        return {
            "version": 1,
            "warning": harness.WARNING,
            "packet_path": "/tmp/fixture-motion-packet/packet.json",
            "packet_sha256": "d" * 64,
            "unblind_map_path": "/tmp/fixture-motion-unblind.json",
            "unblind_map_sha256": "e" * 64,
            "code_map": code_map,
            "entry_count": len(code_map),
        }

    @staticmethod
    def _metrics() -> dict:
        comparisons = []
        for index, profile in enumerate(harness.atomic_profiles()):
            comparisons.append(
                {
                    "subject_id": profile.id,
                    "camera_id": harness.PRIMARY_CAMERAS[profile.family],
                    "control_sha256": "a" * 64,
                    "candidate_sha256": f"{index + 1:064x}",
                    "control_rgb_sha256": "c" * 64,
                    "candidate_rgb_sha256": f"{index + 1001:064x}",
                    "ssim": 0.98,
                    "mean_delta_e00": float(index + 2),
                    "exact_duplicate": False,
                    "near_duplicate": False,
                }
            )
        return {"version": 1, "warning": harness.WARNING, "comparisons": comparisons}

    @staticmethod
    def _performance() -> dict:
        subjects = {
            "control": {
                "p95_frame_time_ms": 10.0,
                "max_resident_presentation_bytes": 1_000_000,
            }
        }
        for index, profile in enumerate(harness.atomic_profiles()):
            subjects[profile.id] = {
                "p95_frame_time_ms": 10.0 + index / 100.0,
                "max_resident_presentation_bytes": 1_000_000 + index,
            }
        return {"version": 1, "warning": harness.WARNING, "subjects": subjects}

    def test_review_contexts_reuse_pixels_without_mixing_camera_sets(self) -> None:
        final_cameras, diagnostic_cameras, _ = harness.load_camera_sets()
        final_ids = [camera.id for camera in final_cameras]
        diagnostic_ids = [camera.id for camera in diagnostic_cameras]

        def frames(camera_ids: list[str]) -> dict:
            return {
                camera_id: {
                    "camera_id": camera_id,
                    "camera_index": final_ids.index(camera_id) + 1,
                    "scored": camera_id not in harness.UNSCORED_CAMERA_IDS,
                    "labeled_path": f"/fixture/{camera_id}.png",
                    "sha256": f"{final_ids.index(camera_id) + 1:064x}",
                }
                for camera_id in camera_ids
            }

        grouped = {
            ("control", "neutral"): frames(final_ids),
            ("snow-01-straight-128", "neutral"): frames(diagnostic_ids),
            ("winner-snow", "neutral"): frames(final_ids),
            ("combination-restrained", "neutral"): frames(final_ids),
            ("combination-restrained", "golden"): frames(diagnostic_ids),
        }
        contextual = harness._contextualize_review_frames(
            grouped,
            final_camera_ids=final_ids,
            diagnostic_camera_ids=diagnostic_ids,
        )
        self.assertEqual(
            len(contextual[("control", "neutral", harness.FINAL_REVIEW_CONTEXT)]),
            17,
        )
        self.assertEqual(
            len(
                contextual[
                    ("control", "neutral", harness.DIAGNOSTIC_REVIEW_CONTEXT)
                ]
            ),
            4,
        )
        self.assertEqual(
            len(
                contextual[
                    (
                        "combination-restrained",
                        "neutral",
                        harness.FINAL_REVIEW_CONTEXT,
                    )
                ]
            ),
            17,
        )
        self.assertEqual(
            {
                frame["sha256"]
                for frame in contextual[
                    (
                        "combination-restrained",
                        "neutral",
                        harness.DIAGNOSTIC_REVIEW_CONTEXT,
                    )
                ]
            },
            {
                frame["sha256"]
                for frame in contextual[
                    (
                        "combination-restrained",
                        "neutral",
                        harness.FINAL_REVIEW_CONTEXT,
                    )
                ]
                if frame["camera_id"] in diagnostic_ids
            },
        )
        final_control = contextual[
            ("control", "neutral", harness.FINAL_REVIEW_CONTEXT)
        ]
        diagnostic_control = contextual[
            ("control", "neutral", harness.DIAGNOSTIC_REVIEW_CONTEXT)
        ]
        self.assertEqual(
            {
                frame["camera_id"]
                for frame in final_control
                if not frame["scored"]
            },
            set(harness.UNSCORED_CAMERA_IDS),
        )
        self.assertTrue(all(frame["scored"] for frame in diagnostic_control))

    def test_opaque_review_png_reencodes_pixels_and_strips_source_metadata(self) -> None:
        try:
            from PIL import Image, ImageDraw, PngImagePlugin
        except ImportError:
            self.skipTest("Pillow unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            source = root / "labeled-source.png"
            destination = root / "packet" / "frame.png"
            image = Image.new(
                "RGB",
                (harness.CAPTURE_WIDTH, harness.CAPTURE_HEIGHT),
                (24, 48, 72),
            )
            ImageDraw.Draw(image).rectangle((0, 0, 300, 45), fill=(255, 214, 72))
            metadata = PngImagePlugin.PngInfo()
            metadata.add_text("structural_draft_warning", harness.WARNING)
            metadata.add_text("source_render_sha256", "a" * 64)
            metadata.add_text("profile_id", "identity-leak")
            image.save(source, format="PNG", pnginfo=metadata, optimize=False)
            code = "RV-0123456789AB"
            first = harness._materialize_opaque_review_png(source, destination, code)
            second = harness._materialize_opaque_review_png(source, destination, code)
            self.assertEqual(first, second)
            self.assertEqual(
                harness.decoded_rgb_sha256(source),
                harness.decoded_rgb_sha256(destination),
            )
            with Image.open(destination) as rendered:
                self.assertEqual(
                    dict(rendered.info),
                    {
                        "structural_draft_warning": harness.WARNING,
                        "opaque_review_code": code,
                    },
                )
            tampered = PngImagePlugin.PngInfo()
            tampered.add_text("structural_draft_warning", harness.WARNING)
            tampered.add_text("opaque_review_code", code)
            tampered.add_text("source_render_sha256", "a" * 64)
            image.save(destination, format="PNG", pnginfo=tampered, optimize=False)
            with self.assertRaisesRegex(harness.HarnessError, "non-opaque metadata"):
                harness._inspect_opaque_review_png(destination, code)

    def test_opaque_motion_strip_metadata_cannot_disclose_source_identity(self) -> None:
        try:
            from PIL import Image, ImageDraw, PngImagePlugin
        except ImportError:
            self.skipTest("Pillow unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            strip = pathlib.Path(temporary) / "strip.png"
            code = "RV-ABCDEF012345"
            image = Image.new("RGB", (640, 360), (20, 40, 80))
            ImageDraw.Draw(image).rectangle((0, 0, 300, 45), fill=(255, 214, 72))
            metadata = PngImagePlugin.PngInfo()
            metadata.add_text("structural_draft_warning", harness.WARNING)
            metadata.add_text("opaque_review_code", code)
            metadata.add_text("strip_source_frame_sha256", "a" * 64)
            image.save(strip, format="PNG", pnginfo=metadata, optimize=False)
            inspected = harness._inspect_opaque_motion_strip(strip, code)
            self.assertEqual(inspected["strip_source_frame_sha256"], "a" * 64)
            metadata.add_text("profile_id", "identity-leak")
            image.save(strip, format="PNG", pnginfo=metadata, optimize=False)
            with self.assertRaisesRegex(harness.HarnessError, "non-opaque metadata"):
                harness._inspect_opaque_motion_strip(strip, code)

    def test_private_blinding_salt_resume_is_idempotent_and_binding_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            private_path = root / "private" / "unblind.json"
            binding = "a" * 64
            with mock.patch.object(harness.secrets, "token_hex", return_value="b" * 64):
                salt = harness._load_or_create_private_blinding_salt(
                    private_path,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256=binding,
                )
            self.assertEqual(salt, "b" * 64)
            self.assertEqual(
                harness._load_or_create_private_blinding_salt(
                    private_path,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256=binding,
                ),
                salt,
            )
            finalized = {
                **harness._blinding_build_state(
                    packet_kind="opaque-world-detail-review",
                    binding_sha256=binding,
                    blinding_salt=salt,
                ),
                "status": "FINALIZED",
                "review_material_sha256": binding,
                "review_packet_sha256": "c" * 64,
                "entries": [],
            }
            harness._finalize_private_blinding_evidence(private_path, finalized)
            harness._finalize_private_blinding_evidence(private_path, finalized)
            self.assertEqual(
                harness._load_or_create_private_blinding_salt(
                    private_path,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256=binding,
                ),
                salt,
            )
            with self.assertRaisesRegex(harness.HarnessError, "cannot resume"):
                harness._load_or_create_private_blinding_salt(
                    private_path,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256="d" * 64,
                )

            plan = harness.build_capture_document(
                root / "publication",
                complete_selection(),
            )
            still_seed = harness._private_packet_blinding_seed(
                plan,
                salt,
                packet_kind="opaque-world-detail-review",
                binding_sha256=binding,
            )
            self.assertNotEqual(
                still_seed,
                harness._private_packet_blinding_seed(
                    plan,
                    salt,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256="d" * 64,
                ),
            )
            self.assertNotEqual(
                still_seed,
                harness._private_packet_blinding_seed(
                    plan,
                    salt,
                    packet_kind="opaque-world-detail-motion-review",
                    binding_sha256=binding,
                ),
            )

    def test_packet_root_cannot_be_inside_unblinded_output(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary).resolve()
            output_root = root / "publication"
            output_root.mkdir()
            with self.assertRaisesRegex(harness.HarnessError, "unblinded output root"):
                harness._require_blinded_packet_root(
                    output_root / "review-packet",
                    output_root,
                    "review packet root",
                )

    def test_stress_gate_applies_tactical_floor_to_each_condition(self) -> None:
        passing = {
            "weighted_score": 80.0,
            "minimum_categories": {field: 4 for field in harness.CATEGORY_ORDER},
        }
        low_readability = json.loads(json.dumps(passing))
        low_readability["minimum_categories"][
            "terrain_route_water_edge_readability"
        ] = 2
        low_edge = json.loads(json.dumps(passing))
        low_edge["minimum_categories"]["edge_temporal_quietness"] = 2
        self.assertTrue(harness._stress_passes(passing, passing))
        self.assertFalse(harness._stress_passes(passing, low_readability))
        self.assertFalse(harness._stress_passes(passing, low_edge))
        self.assertFalse(harness._stress_passes(low_readability, passing))

    def test_two_phase_review_derivation_and_gates(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            plan_path = root / "plan.json"
            packet_path = root / "packet.json"
            unblind_path = root / "unblind.json"
            review_paths = [root / "review-a.json", root / "review-b.json"]
            metrics_path = root / "metrics.json"
            performance_path = root / "performance.json"
            metrics_path.write_text(json.dumps(self._metrics()), encoding="utf-8")
            performance_path.write_text(json.dumps(self._performance()), encoding="utf-8")
            plan_path.write_text(
                harness.pretty_json(harness.build_capture_document(root / "publication")),
                encoding="utf-8",
            )
            for path, reviewer_id in zip(review_paths, ("review-a", "review-b")):
                path.write_text(json.dumps(self._review(reviewer_id, phase=1)), encoding="utf-8")
            packet = self._packet_evidence(self._review("review-a", phase=1))
            with mock.patch.object(
                harness,
                "validate_blinded_review_packet",
                return_value=packet,
            ):
                partial = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                )
            self.assertFalse(partial["complete"])
            self.assertTrue(all(len(ids) == 2 for ids in partial["selection"]["promoted"].values()))
            self.assertTrue(all(value is None for value in partial["selection"]["atomic_winners"].values()))

            plan_path.write_text(
                harness.pretty_json(
                    harness.build_capture_document(root / "publication", partial["selection"])
                ),
                encoding="utf-8",
            )
            for path, reviewer_id in zip(review_paths, ("review-a", "review-b")):
                path.write_text(json.dumps(self._review(reviewer_id, phase=2)), encoding="utf-8")
            packet = self._packet_evidence(self._review("review-a", phase=2))
            with mock.patch.object(harness, "validate_blinded_review_packet", return_value=packet):
                ladder = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                )
            self.assertFalse(ladder["complete"])
            self.assertTrue(all(value is not None for value in ladder["selection"]["ladder_inputs"].values()))

            plan_path.write_text(
                harness.pretty_json(
                    harness.build_capture_document(root / "publication", ladder["selection"])
                ),
                encoding="utf-8",
            )
            for path, reviewer_id in zip(review_paths, ("review-a", "review-b")):
                path.write_text(json.dumps(self._review(reviewer_id, phase=3)), encoding="utf-8")
            packet = self._packet_evidence(self._review("review-a", phase=3))
            with mock.patch.object(harness, "validate_blinded_review_packet", return_value=packet):
                complete = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                )
            self.assertTrue(complete["complete"])
            harness.validate_selection(complete["selection"], complete=True)
            for family in harness.FAMILY_ORDER:
                self.assertEqual(
                    complete["selection"]["atomic_winners"][family],
                    complete["selection"]["promoted"][family][0],
                )
            pre_motion_plan = harness.build_capture_document(
                root / "publication", complete["selection"]
            )
            plan_path.write_text(harness.pretty_json(pre_motion_plan), encoding="utf-8")
            for path, reviewer_id in zip(review_paths, ("review-a", "review-b")):
                path.write_text(json.dumps(self._review(reviewer_id, phase=4)), encoding="utf-8")
            packet = self._packet_evidence(self._review("review-a", phase=4))
            motion_review_paths = [root / "motion-review-a.json", root / "motion-review-b.json"]
            for path, reviewer_id in zip(motion_review_paths, ("review-a", "review-b")):
                path.write_text(
                    json.dumps(self._motion_review(reviewer_id, pre_motion_plan)),
                    encoding="utf-8",
                )
            motion_packet = self._motion_packet_evidence(pre_motion_plan)
            failed_clip = next(
                clip["id"]
                for clip in pre_motion_plan["motion"]["clips"]
                if clip["id"].startswith("diagnostic-water-")
                and clip["profile_id"]
                == pre_motion_plan["selection"]["atomic_winners"]["water"]
            )
            failed_motion_paths = [root / "motion-failed-a.json", root / "motion-failed-b.json"]
            for path, reviewer_id in zip(failed_motion_paths, ("review-a", "review-b")):
                path.write_text(
                    json.dumps(
                        self._motion_review(
                            reviewer_id,
                            pre_motion_plan,
                            fail_clip_id=failed_clip,
                        )
                    ),
                    encoding="utf-8",
                )
            with mock.patch.object(
                harness, "validate_blinded_review_packet", return_value=packet
            ), mock.patch.object(
                harness,
                "validate_blinded_motion_review_packet",
                return_value=motion_packet,
            ):
                invalidated = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                    failed_motion_paths,
                    root / "motion-packet.json",
                    root / "motion-unblind.json",
                )

            self.assertTrue(invalidated["motion_revalidation_required"])
            self.assertEqual(
                invalidated["selection"]["status"],
                "MOTION_RECAPTURE_REREVIEW_REPERF_REQUIRED",
            )
            self.assertIsNone(
                invalidated["selection"]["review_evidence"]["motion_review_evidence"]
            )
            self.assertEqual(
                invalidated["selection"]["pre_motion_combinations"]["score-leader"],
                invalidated["selection"]["pre_motion_atomic_winners"],
            )
            invalidated_plan = harness.build_capture_document(
                root / "publication", invalidated["selection"]
            )
            old_score_slot = next(
                slot
                for slot in pre_motion_plan["study"]["logical_slots"]
                if slot["stage"] == "07-final-17"
                and slot["look_id"] == "combination-score-leader"
                and slot["camera_id"] == "02-highlands-oblique"
            )
            new_score_slot = next(
                slot
                for slot in invalidated_plan["study"]["logical_slots"]
                if slot["stage"] == "07-final-17"
                and slot["look_id"] == "combination-score-leader"
                and slot["camera_id"] == "02-highlands-oblique"
            )
            self.assertNotEqual(old_score_slot["artifact"], new_score_slot["artifact"])
            old_score_clip = next(
                clip
                for clip in pre_motion_plan["motion"]["clips"]
                if clip["id"] == "combination-score-leader-02"
            )
            new_score_clip = next(
                clip
                for clip in invalidated_plan["motion"]["clips"]
                if clip["id"] == "combination-score-leader-02"
            )
            self.assertNotEqual(old_score_clip["mp4"], new_score_clip["mp4"])
            with mock.patch.object(
                harness, "validate_blinded_review_packet", return_value=packet
            ), mock.patch.object(
                harness,
                "validate_blinded_motion_review_packet",
                return_value=motion_packet,
            ), mock.patch.object(
                harness,
                "validate_recomputed_selection_evidence",
                return_value={"metrics_exact": True, "performance_exact": True},
            ):
                final_selection = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                    motion_review_paths,
                    root / "motion-packet.json",
                    root / "motion-unblind.json",
                )
            plan = harness.build_capture_document(
                root / "publication", final_selection["selection"]
            )
            self.assertNotEqual(
                harness._selection_decisions_sha256(pre_motion_plan["selection"]),
                harness._selection_decisions_sha256(plan["selection"]),
            )
            self.assertEqual(pre_motion_plan["motion"], plan["motion"])
            motion_plan_sha256 = harness.sha256_object(pre_motion_plan["motion"])
            self.assertEqual(motion_plan_sha256, harness.sha256_object(plan["motion"]))

            salt = "9" * 64
            review_material_sha256 = "8" * 64
            pre_still_seed = harness._private_packet_blinding_seed(
                pre_motion_plan,
                salt,
                packet_kind="opaque-world-detail-review",
                binding_sha256=review_material_sha256,
            )
            final_still_seed = harness._private_packet_blinding_seed(
                plan,
                salt,
                packet_kind="opaque-world-detail-review",
                binding_sha256=review_material_sha256,
            )
            self.assertEqual(pre_still_seed, final_still_seed)
            pre_motion_private_seed = harness._private_packet_blinding_seed(
                pre_motion_plan,
                salt,
                packet_kind="opaque-world-detail-motion-review",
                binding_sha256=motion_plan_sha256,
            )
            final_motion_private_seed = harness._private_packet_blinding_seed(
                plan,
                salt,
                packet_kind="opaque-world-detail-motion-review",
                binding_sha256=motion_plan_sha256,
            )
            self.assertEqual(pre_motion_private_seed, final_motion_private_seed)
            self.assertEqual(
                harness._motion_blinding_seed(pre_motion_plan),
                harness._motion_blinding_seed(plan),
            )
            self.assertEqual(
                [
                    harness._blind_code(
                        harness._motion_blinding_seed(pre_motion_plan),
                        f"motion-{clip['id']}",
                        clip["lighting"],
                        harness.MOTION_REVIEW_CONTEXT,
                    )
                    for clip in pre_motion_plan["motion"]["clips"]
                ],
                [
                    harness._blind_code(
                        harness._motion_blinding_seed(plan),
                        f"motion-{clip['id']}",
                        clip["lighting"],
                        harness.MOTION_REVIEW_CONTEXT,
                    )
                    for clip in plan["motion"]["clips"]
                ],
            )

            changed_motion_plan = json.loads(json.dumps(plan))
            changed_motion_plan["motion"]["clips"][0]["orbit_end_degrees"] += 0.125
            changed_motion_sha256 = harness.sha256_object(
                changed_motion_plan["motion"]
            )
            self.assertNotEqual(motion_plan_sha256, changed_motion_sha256)
            self.assertNotEqual(
                final_motion_private_seed,
                harness._private_packet_blinding_seed(
                    changed_motion_plan,
                    salt,
                    packet_kind="opaque-world-detail-motion-review",
                    binding_sha256=changed_motion_sha256,
                ),
            )
            self.assertNotEqual(
                harness._motion_blinding_seed(plan),
                harness._motion_blinding_seed(changed_motion_plan),
            )
            self.assertNotEqual(
                final_still_seed,
                harness._private_packet_blinding_seed(
                    plan,
                    salt,
                    packet_kind="opaque-world-detail-review",
                    binding_sha256="7" * 64,
                ),
            )

            canonical_plan_path = root / "publication" / "capture-plan.json"
            canonical_plan_path.parent.mkdir(parents=True, exist_ok=True)
            canonical_plan_path.write_text(
                harness.pretty_json(pre_motion_plan),
                encoding="utf-8",
            )
            finalized_motion = {
                "clips": 22,
                "candidate_frames": 22 * harness.MOTION_FRAME_COUNT,
            }
            with mock.patch.object(
                harness,
                "_validate_motion_deliverables_for_plan",
                return_value=finalized_motion,
            ) as validate_motion:
                self.assertEqual(
                    harness.validate_motion_deliverables(canonical_plan_path)["clips"],
                    22,
                )
                canonical_plan_path.write_text(
                    harness.pretty_json(plan),
                    encoding="utf-8",
                )
                self.assertEqual(
                    harness.validate_motion_deliverables(canonical_plan_path)["clips"],
                    22,
                )
            self.assertEqual(validate_motion.call_count, 2)
            plan_path.write_text(harness.pretty_json(plan), encoding="utf-8")
            with mock.patch.object(
                harness, "validate_blinded_review_packet", return_value=packet
            ), mock.patch.object(
                harness,
                "validate_blinded_motion_review_packet",
                return_value=motion_packet,
            ), mock.patch.object(
                harness,
                "validate_recomputed_selection_evidence",
                return_value={"metrics_exact": True, "performance_exact": True},
            ):
                final = harness.validate_final_review_evidence(
                    plan_path,
                    plan,
                    review_paths,
                    metrics_path,
                    performance_path,
                )
            self.assertTrue(all(row["recommended"] for row in final["combinations"].values()))
            self.assertTrue(
                all(row["final_17_frames_reviewed"] == 17 for row in final["atomic"].values())
            )

    def test_failed_family_is_never_backfilled_into_promotions(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            plan_path = root / "plan.json"
            packet_path = root / "packet.json"
            unblind_path = root / "unblind.json"
            review_paths = [root / "review-a.json", root / "review-b.json"]
            metrics_path = root / "metrics.json"
            performance_path = root / "performance.json"
            metrics_path.write_text(json.dumps(self._metrics()), encoding="utf-8")
            performance_path.write_text(json.dumps(self._performance()), encoding="utf-8")
            plan_path.write_text(
                harness.pretty_json(harness.build_capture_document(root / "publication")),
                encoding="utf-8",
            )
            reviews = []
            for path, reviewer_id in zip(review_paths, ("review-a", "review-b")):
                review = self._review(reviewer_id, phase=1)
                snow_codes = {
                    self._code(profile.id, "neutral")
                    for profile in harness.atomic_profiles()
                    if profile.family == "snow"
                }
                for rating in review["ratings"]:
                    if rating["code"] in snow_codes:
                        rating["categories"][
                            "terrain_route_water_edge_readability"
                        ] = 2
                path.write_text(json.dumps(review), encoding="utf-8")
                reviews.append(review)
            packet = self._packet_evidence(reviews[0])
            with mock.patch.object(
                harness, "validate_blinded_review_packet", return_value=packet
            ):
                derived = harness.derive_selection_from_reviews(
                    plan_path,
                    review_paths,
                    metrics_path,
                    performance_path,
                    packet_path,
                    unblind_path,
                )
            self.assertEqual(derived["selection"]["promoted"]["snow"], [])
            self.assertEqual(
                len(derived["selection"]["stress_diagnostics"]["snow"]), 2
            )
            self.assertTrue(
                all(not row["floor_pass"] for row in derived["rankings"]["snow"])
            )

    def test_diagnostic_fillers_preserve_fixed_capture_ledgers_without_promotion(self) -> None:
        selection = complete_selection()
        snow_first = selection["promoted"]["snow"][0]
        selection["promoted"]["snow"] = [snow_first]
        selection["promoted"]["water"] = []
        for family in ("snow", "water"):
            selection["ladder_inputs"][family] = "control"
            selection["atomic_winners"][family] = "control"
            selection["pre_motion_atomic_winners"][family] = "control"
            for combination_id in harness.COMBINATION_IDS:
                selection["combinations"][combination_id][family] = "control"
                selection["pre_motion_combinations"][combination_id][family] = "control"
        stamp_review_evidence(selection)
        harness.validate_selection(selection, complete=True)
        with tempfile.TemporaryDirectory() as temporary:
            output = pathlib.Path(temporary).resolve() / "report"
            still = harness.build_still_plan(output, selection)
            motion = harness.build_motion_plan(output, selection)
        self.assertEqual(
            still["slot_accounting"]["resolved_by_stage"]["03-stress-finalists"],
            144,
        )
        self.assertEqual(still["slot_accounting"]["resolved_logical_slots"], 665)
        self.assertEqual(motion["expected_clips"], 22)
        self.assertEqual(motion["resolved_clips"], 22)
        self.assertEqual(selection["promoted"]["water"], [])
        self.assertEqual(len(selection["stress_diagnostics"]["water"]), 2)
        self.assertTrue(
            any(clip["id"].startswith("diagnostic-water-") for clip in motion["clips"])
        )

    def test_finalization_rejects_supplied_metric_or_performance_numbers(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            plan_path = root / "plan.json"
            plan_path.write_text(
                harness.pretty_json(harness.build_capture_document(root / "publication")),
                encoding="utf-8",
            )
            metrics_path = root / "metrics.json"
            performance_path = root / "performance.json"
            supplied_metrics = self._metrics()
            supplied_performance = self._performance()
            metrics_path.write_text(json.dumps(supplied_metrics), encoding="utf-8")
            performance_path.write_text(json.dumps(supplied_performance), encoding="utf-8")
            recomputed_metrics = json.loads(json.dumps(supplied_metrics))
            recomputed_metrics["comparisons"][0]["ssim"] = 0.97
            with mock.patch.object(
                harness, "build_metric_evidence", return_value=recomputed_metrics
            ), mock.patch.object(
                harness,
                "build_selection_performance_evidence",
                return_value=supplied_performance,
            ):
                with self.assertRaisesRegex(harness.HarnessError, "metric evidence"):
                    harness.validate_recomputed_selection_evidence(
                        plan_path,
                        metrics_path,
                        performance_path,
                    )
            recomputed_performance = json.loads(json.dumps(supplied_performance))
            recomputed_performance["subjects"]["control"]["p95_frame_time_ms"] = 99.0
            with mock.patch.object(
                harness, "build_metric_evidence", return_value=supplied_metrics
            ), mock.patch.object(
                harness,
                "build_selection_performance_evidence",
                return_value=recomputed_performance,
            ):
                with self.assertRaisesRegex(harness.HarnessError, "performance evidence"):
                    harness.validate_recomputed_selection_evidence(
                        plan_path,
                        metrics_path,
                        performance_path,
                    )

    def test_fabricated_or_preexisting_lifecycle_certificate_is_not_accepted(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            plan_path = root / "plan.json"
            plan = harness.build_capture_document(root / "publication")
            plan_path.write_text(harness.pretty_json(plan), encoding="utf-8")
            profile_sha = harness.control_profile().sha256
            certificate = {
                "version": 1,
                "warning": harness.WARNING,
                "capture_plan_sha256": harness.sha256_file(plan_path),
                "source_provenance_sha256": harness.sha256_object(plan["provenance"]),
                "profile_matrix_sha256": harness.sha256_object(plan["study"]["profile_matrix"]),
                "tested_profile_sha256": profile_sha,
                "cycles_requested": 100,
                "cycles_completed": 100,
                "cycles": [],
                "final_chain_sha256": "0" * 64,
            }
            certificate_path = root / "lifecycle.json"
            certificate_path.write_text(json.dumps(certificate), encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness.validate_lifecycle_certificate(plan_path, certificate_path)
            with self.assertRaisesRegex(harness.HarnessError, "fresh certificate destination"):
                harness.run_lifecycle(
                    plan_path,
                    certificate_path.resolve(),
                    work_root=(root / "lifecycle-work").resolve(),
                )

    def test_lifecycle_launch_evidence_binds_the_final_sidecar_to_cycle_100(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary).resolve()
            plan_path = root / "plan.json"
            plan = harness.build_capture_document(root / "publication")
            plan_path.write_text(harness.pretty_json(plan), encoding="utf-8")
            certificate_path = root / "lifecycle.json"
            certificate_path.write_text("{}\n", encoding="utf-8")
            capture_path = root / "cycle-capture.png"
            capture_path.write_bytes(b"genuine-render-fixture")
            report_path = harness.runtime_report_path(capture_path)
            log_path = root / "runtime.log"
            log_path.write_text("completed\n", encoding="utf-8")

            receipt_body = {
                "version": 1,
                "launch_nonce": "a" * 64,
                "process_id": 4242,
                "executable_sha256": "b" * 64,
                "source_provenance_sha256": harness.sha256_object(plan["provenance"]),
                "capture_plan_sha256": "c" * 64,
                "profile_sha256": harness.control_profile().sha256,
            }
            receipt = {
                **receipt_body,
                "receipt_sha256": harness.sha256_bytes(
                    harness.compact_json(receipt_body).encode("utf-8")
                ),
            }
            cleanup = {
                "completed_cycles": 100,
                "entities_remaining": 0,
                "materials_remaining": 0,
                "meshes_remaining": 0,
                "target_images_remaining": 0,
                "camera_state_restored": True,
                "oit_state_restored": True,
                "transmission_state_restored": True,
                "depth_state_restored": True,
                "volumetric_state_restored": True,
            }
            wrapper = {
                "version": 1,
                "warning": harness.WARNING,
                "capture": {"path": str(capture_path)},
                "report": {"runtime_receipt": receipt, "cleanup": cleanup},
            }
            report_path.write_text(json.dumps(wrapper), encoding="utf-8")
            request = harness._lifecycle_request(
                plan_path, plan, certificate_path
            )
            launch = {
                "version": 1,
                "warning": harness.WARNING,
                "launch_nonce": receipt["launch_nonce"],
                "process_id": receipt["process_id"],
                "executable_sha256": receipt["executable_sha256"],
                "runtime_receipt_sha256": receipt["receipt_sha256"],
                "source_provenance_sha256": receipt["source_provenance_sha256"],
                "outer_capture_plan_sha256": harness.sha256_file(plan_path),
                "runtime_capture_plan_sha256": receipt["capture_plan_sha256"],
                "lifecycle_request_sha256": harness.sha256_bytes(
                    harness.compact_json(request).encode("utf-8")
                ),
                "certificate_sha256": harness.sha256_file(certificate_path),
                "final_cycle_capture_path": str(capture_path),
                "final_cycle_capture_sha256": harness.sha256_file(capture_path),
                "final_cycle_report_path": str(report_path),
                "final_cycle_report_sha256": harness.sha256_file(report_path),
                "log_path": str(log_path),
                "log_sha256": harness.sha256_file(log_path),
                "structural_draft_environment": harness.STRUCTURAL_DRAFT_ENVIRONMENT,
                "structural_draft_value": harness.STRUCTURAL_DRAFT_VALUE,
            }
            launch_path = harness.lifecycle_launch_evidence_path(certificate_path)
            launch_path.write_text(json.dumps(launch), encoding="utf-8")
            harness._validate_lifecycle_launch_evidence(
                launch_path,
                certificate_path=certificate_path,
                plan_path=plan_path,
                runtime_receipt=receipt,
            )

            wrapper["report"]["cleanup"]["completed_cycles"] = 1
            report_path.write_text(json.dumps(wrapper), encoding="utf-8")
            launch["final_cycle_report_sha256"] = harness.sha256_file(report_path)
            launch_path.write_text(json.dumps(launch), encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "all requested teardown cycles"):
                harness._validate_lifecycle_launch_evidence(
                    launch_path,
                    certificate_path=certificate_path,
                    plan_path=plan_path,
                    runtime_receipt=receipt,
                )

    def test_capture_state_accepts_and_binds_new_provenance_schema(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            output_root = root / "publication"
            raw_root = root / "raw"
            work_root = root / "work"
            state_path = work_root / "capture-state.json"
            plan_path = root / "plan.json"
            plan_path.write_text("{}\n", encoding="utf-8")
            camera_ids = list(harness.BASELINE_ORACLE_CAMERA_IDS)
            raw_paths = [raw_root / f"{camera_id}.png" for camera_id in camera_ids]
            for index, raw_path in enumerate(raw_paths):
                raw_path.parent.mkdir(parents=True, exist_ok=True)
                raw_path.write_bytes(f"raw-{index}".encode("ascii"))
                harness.runtime_report_path(raw_path).write_text(
                    json.dumps({"report": {}}),
                    encoding="utf-8",
                )
            condition = harness.LIGHTING_CONDITIONS["neutral"]
            jobs = [
                {
                    "id": f"shared-control-neutral-focused-{camera_id}-fixture",
                    "kind": "still",
                    "stage": "00-shared-control",
                    "look_id": "control",
                    "lighting": "neutral",
                    "asset_stage": condition.asset_stage,
                    "profile_sha256": harness.CONTROL_PROFILE_SHA256,
                    "profile_json": harness.control_profile().canonical_json,
                    "control_profile_omitted": True,
                    "time_hours": 12.0,
                    "liquid_phase_seconds": 0.0,
                    "capture_plan": {
                        "version": 1,
                        "captures": [{"path": str(path)}],
                    },
                    "raw_capture_root": str(raw_root),
                    "cameras": [{"id": camera_id}],
                    "artifacts": [f"runtime/raw-stills/{camera_id}.png"],
                }
                for camera_id, path in zip(camera_ids, raw_paths)
            ]
            plan = {
                "output_root": str(output_root),
                "raw_capture_root": str(raw_root),
                "provenance": {"fixture": True},
                "study": {
                    "jobs": jobs,
                    "verification_jobs": [],
                    "reproduction_jobs": [],
                },
                "motion": {"jobs": []},
            }
            stage_manifest = {
                "version": 1,
                "warning": harness.WARNING,
                "asset_stage": condition.asset_stage,
                "lighting_condition": harness.dataclasses.asdict(condition),
                "source_asset_tree_sha256": "b" * 64,
                "staged_asset_tree_sha256": "c" * 64,
                "modified_assets": [],
            }
            stage_manifest_path = (
                work_root
                / "asset-stages"
                / condition.asset_stage
                / "stage-manifest.json"
            )
            stage_manifest_path.parent.mkdir(parents=True, exist_ok=True)
            stage_manifest_path.write_text(
                harness.pretty_json(stage_manifest),
                encoding="utf-8",
            )
            stage_binding = {
                "asset_stage": condition.asset_stage,
                "asset_stage_manifest_sha256": harness.sha256_file(stage_manifest_path),
                "asset_stage_tree_sha256": "c" * 64,
                "source_asset_tree_sha256": "b" * 64,
            }
            executable_sha256 = "d" * 64
            source_provenance_sha256 = harness.sha256_object(plan["provenance"])
            receipts = {
                job["id"]: fixture_runtime_receipt(
                    job,
                    index,
                    executable_sha256=executable_sha256,
                    source_provenance_sha256=source_provenance_sha256,
                )
                for index, job in enumerate(jobs)
            }
            completed = {}
            attempts = []
            for index, (job, raw_path) in enumerate(zip(jobs, raw_paths)):
                receipt = receipts[job["id"]]
                artifact_hashes = [harness.sha256_file(raw_path)]
                report_hashes = [
                    harness.sha256_file(harness.runtime_report_path(raw_path))
                ]
                log_path = work_root / f"capture-{index}.log"
                log_path.write_text("complete\n", encoding="utf-8")
                completed[job["id"]] = {
                    "job_sha256": harness.sha256_object(job),
                    "artifact_sha256": artifact_hashes,
                    "report_sha256": report_hashes,
                    "log": str(log_path),
                    "launch_nonce": receipt["launch_nonce"],
                    "runtime_receipt_sha256": receipt["receipt_sha256"],
                    "process_id": receipt["process_id"],
                    "executable_sha256": executable_sha256,
                    **stage_binding,
                }
                attempt_base = {
                    "job_id": job["id"],
                    "job_sha256": harness.sha256_object(job),
                    "command": [
                        "cargo", "run", "--locked", "--release", "-p", "hex_game",
                        "--features", "map-review",
                    ],
                    "launch_nonce": receipt["launch_nonce"],
                    "source_provenance_sha256": source_provenance_sha256,
                    "capture_plan_sha256": harness.sha256_bytes(
                        harness.compact_json(job["capture_plan"]).encode("utf-8")
                    ),
                    "structural_draft_environment": harness.STRUCTURAL_DRAFT_ENVIRONMENT,
                    "structural_draft_value": harness.STRUCTURAL_DRAFT_VALUE,
                    **stage_binding,
                }
                if index == 0:
                    attempts.append(
                        {
                            **attempt_base,
                            "attempt_number": 1,
                            "status": "FAILED",
                            "log": str(work_root / "failed.log"),
                            "returncode": harness.HARNESS_PROCESS_EXCEPTION_RETURN_CODE,
                            "failure_phase": "process",
                            "failure_type": "TimeoutExpired",
                        }
                    )
                attempts.append(
                    {
                        **attempt_base,
                        "attempt_number": 2 if index == 0 else 1,
                        "status": "COMPLETE",
                        "log": str(log_path),
                        "returncode": 0,
                        "artifact_sha256": artifact_hashes,
                        "runtime_receipt_sha256": receipt["receipt_sha256"],
                        "process_id": receipt["process_id"],
                        "executable_sha256": executable_sha256,
                    }
                )
            oracle_body = {
                "version": 1,
                "fixture": "baseline-oracle-equivalence",
            }
            oracle_evidence = {
                **oracle_body,
                "evidence_sha256": harness.sha256_object(oracle_body),
            }
            state = {
                "version": 1,
                "warning": harness.WARNING,
                "plan_sha256_history": [harness.sha256_file(plan_path)],
                "pinned_executable_sha256": executable_sha256,
                "baseline_oracle_equivalence": oracle_evidence,
                "completed": completed,
                "attempts": attempts,
            }
            state_path.write_text(harness.pretty_json(state), encoding="utf-8")
            camera_specs = {
                camera.id: camera for camera in harness.load_camera_sets()[0]
            }
            camera_by_path = {
                path.resolve(): (camera_specs[camera_id], 0.0, 90)
                for camera_id, path in zip(camera_ids, raw_paths)
            }
            receipt_by_path = {
                path.resolve(): receipts[job["id"]]
                for job, path in zip(jobs, raw_paths)
            }

            def report_for_capture(*args, **kwargs):
                return {
                    "runtime_receipt": receipt_by_path[
                        pathlib.Path(kwargs["png_path"]).resolve()
                    ]
                }

            def aggregate_oracle(job_values, records_by_job, **kwargs):
                self.assertEqual(
                    [job["id"] for job in job_values],
                    [job["id"] for job in jobs],
                )
                self.assertEqual(set(records_by_job), {job["id"] for job in jobs})
                return oracle_evidence

            with mock.patch.object(
                harness,
                "validate_capture_document",
                return_value=plan,
            ), mock.patch.object(
                harness,
                "_job_cameras",
                return_value=camera_by_path,
            ), mock.patch.object(
                harness,
                "validate_runtime_report",
                side_effect=report_for_capture,
            ) as strict_report_validator, mock.patch.object(
                harness,
                "_validate_baseline_oracle_equivalence",
                side_effect=aggregate_oracle,
            ):
                result = harness.validate_capture_state(
                    plan_path,
                    state_path,
                    include_motion=False,
                )
                self.assertEqual(strict_report_validator.call_count, len(raw_paths))
                strict_report_validator.side_effect = harness.HarnessError(
                    "pre-teardown runtime sidecar"
                )
                with self.assertRaisesRegex(harness.HarnessError, "pre-teardown"):
                    harness.validate_capture_state(
                        plan_path,
                        state_path,
                        include_motion=False,
                    )
            self.assertEqual(result["pinned_executable_sha256"], executable_sha256)
            self.assertEqual(result["failed_attempts"], 1)
            self.assertEqual(result["path"], str(state_path.resolve()))

    def test_formula_workbook_contract(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            workbook = pathlib.Path(temporary) / "review-workbook.xlsx"

            def inline_cell(reference: str, value: str) -> str:
                return f'<c r="{reference}" t="inlineStr"><is><t>{value}</t></is></c>'

            def number_cell(reference: str, value: float) -> str:
                return f'<c r="{reference}"><v>{value}</v></c>'

            def formula_cell(reference: str, formula: str) -> str:
                return f'<c r="{reference}"><f>{formula}</f><v>0</v></c>'

            def worksheet(rows: list[str], dimension: str) -> str:
                return (
                    '<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">'
                    f'<dimension ref="{dimension}"/><sheetData>{"".join(rows)}</sheetData>'
                    '</worksheet>'
                )

            ratings_rows = [
                '<row r="1">'
                + inline_cell("A1", harness.WARNING)
                + "".join(number_cell(f"{column}1", weight) for column, weight in zip(
                    "BCDEFG", (0.25, 0.20, 0.15, 0.15, 0.15, 0.10)
                ))
                + '</row>'
            ]
            for row, profile in enumerate(harness.atomic_profiles(), start=2):
                ratings_rows.append(
                    f'<row r="{row}">'
                    + inline_cell(f"A{row}", profile.id)
                    + "".join(number_cell(f"{column}{row}", 4) for column in "BCDEFG")
                    + formula_cell(f"H{row}", f"SUMPRODUCT(B{row}:G{row},$B$1:$G$1)")
                    + '</row>'
                )
            rankings_rows = ['<row r="1">' + inline_cell("A1", harness.WARNING) + '</row>']
            for row in range(2, 62):
                rankings_rows.append(
                    f'<row r="{row}">'
                    + formula_cell(f"A{row}", f"'Ratings'!A{row}")
                    + formula_cell(
                        f"B{row}",
                        f"RANK.EQ('Ratings'!H{row},'Ratings'!$H$2:$H$61)",
                    )
                    + '</row>'
                )
            performance_rows = ['<row r="1">' + inline_cell("A1", harness.WARNING) + '</row>']
            for row in range(2, 62):
                performance_rows.append(
                    f'<row r="{row}">'
                    + number_cell(f"A{row}", row)
                    + formula_cell(f"B{row}", f"MAX(A{row}:A{row})")
                    + '</row>'
                )
            recommendation_rows = ['<row r="1">' + inline_cell("A1", harness.WARNING) + '</row>']
            for row in range(2, 15):
                performance_row = 2 + ((row - 2) % 4)
                recommendation_rows.append(
                    f'<row r="{row}">'
                    + formula_cell(
                        f"A{row}",
                        f"IF(AND('Rankings'!B{row}&lt;=60,'Performance'!B{performance_row}&gt;=0),1,0)",
                    )
                    + '</row>'
                )
            with zipfile.ZipFile(workbook, "w") as archive:
                archive.writestr(
                    "[Content_Types].xml",
                    '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                    '<Default Extension="xml" ContentType="application/xml"/>'
                    '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
                    + "".join(
                        f'<Override PartName="/xl/worksheets/sheet{index}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
                        for index in range(1, 5)
                    )
                    + '</Types>',
                )
                archive.writestr(
                    "_rels/.rels",
                    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                    '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
                    '</Relationships>',
                )
                archive.writestr(
                    "xl/_rels/workbook.xml.rels",
                    '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                    + "".join(
                        f'<Relationship Id="rId{index}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet{index}.xml"/>'
                        for index in range(1, 5)
                    )
                    + "</Relationships>",
                )
                archive.writestr(
                    "xl/workbook.xml",
                    '<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
                    'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">'
                    '<sheets><sheet name="Ratings" sheetId="1" r:id="rId1"/><sheet name="Rankings" sheetId="2" r:id="rId2"/>'
                    '<sheet name="Performance" sheetId="3" r:id="rId3"/><sheet name="Recommendations" sheetId="4" r:id="rId4"/>'
                    '</sheets><calcPr calcMode="auto" fullCalcOnLoad="1" forceFullCalc="1"/></workbook>',
                )
                archive.writestr("xl/worksheets/sheet1.xml", worksheet(ratings_rows, "A1:H61"))
                archive.writestr("xl/worksheets/sheet2.xml", worksheet(rankings_rows, "A1:B61"))
                archive.writestr("xl/worksheets/sheet3.xml", worksheet(performance_rows, "A1:B61"))
                archive.writestr("xl/worksheets/sheet4.xml", worksheet(recommendation_rows, "A1:A14"))
            result = harness._validate_formula_workbook(workbook)
            self.assertGreaterEqual(result["formula_cells"], 253)
            self.assertTrue(result["automatic_recalculation"])
            self.assertIn("review_results_canonical_sha256", result)
            self.assertNotIn("review_results_sha256", result)

            with zipfile.ZipFile(workbook, "r") as archive:
                members = {name: archive.read(name) for name in archive.namelist()}

            shared_string_members = dict(members)
            inline_warning = (
                f'<c r="A1" t="inlineStr"><is><t>{harness.WARNING}</t></is></c>'
            ).encode("utf-8")
            shared_warning = b'<c r="A1" t="s"><v>0</v></c>'
            for index in range(1, 5):
                sheet_name = f"xl/worksheets/sheet{index}.xml"
                self.assertIn(inline_warning, shared_string_members[sheet_name])
                shared_string_members[sheet_name] = shared_string_members[sheet_name].replace(
                    inline_warning,
                    shared_warning,
                )
            shared_string_members["xl/sharedStrings.xml"] = (
                '<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" '
                'count="1" uniqueCount="1"><si><t>'
                + harness.WARNING
                + "</t></si></sst>"
            ).encode("utf-8")
            shared_string_members["[Content_Types].xml"] = shared_string_members[
                "[Content_Types].xml"
            ].replace(
                b"</Types>",
                b'<Override PartName="/xl/sharedStrings.xml" '
                b'ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>'
                b"</Types>",
            )
            shared_string_members["xl/_rels/workbook.xml.rels"] = shared_string_members[
                "xl/_rels/workbook.xml.rels"
            ].replace(
                b"</Relationships>",
                b'<Relationship Id="rId5" '
                b'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" '
                b'Target="sharedStrings.xml"/></Relationships>',
            )
            shared_string_workbook = pathlib.Path(temporary) / "shared-warning.xlsx"
            with zipfile.ZipFile(shared_string_workbook, "w") as archive:
                for name, data in shared_string_members.items():
                    archive.writestr(name, data)
            harness._validate_formula_workbook(shared_string_workbook)

            external_members = dict(members)
            external_members["_rels/.rels"] = external_members["_rels/.rels"].replace(
                b"</Relationships>",
                b'<Relationship Id="rIdExternal" '
                b'Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink" '
                b'Target="https://example.invalid/workbook" TargetMode="External"/>'
                b"</Relationships>",
            )
            external_workbook = pathlib.Path(temporary) / "external-link.xlsx"
            with zipfile.ZipFile(external_workbook, "w") as archive:
                for name, data in external_members.items():
                    archive.writestr(name, data)
            with self.assertRaisesRegex(harness.HarnessError, "external relationships"):
                harness._validate_formula_workbook(external_workbook)

            semantic_stub = pathlib.Path(temporary) / "semantic-stub.xlsx"
            members["xl/worksheets/sheet1.xml"] = members[
                "xl/worksheets/sheet1.xml"
            ].replace(b"SUMPRODUCT", b"SUM")
            with zipfile.ZipFile(semantic_stub, "w") as archive:
                for name, data in members.items():
                    archive.writestr(name, data)
            with self.assertRaisesRegex(harness.HarnessError, "SUMPRODUCT"):
                harness._validate_formula_workbook(semantic_stub)

            stub = pathlib.Path(temporary) / "stub.xlsx"
            with zipfile.ZipFile(stub, "w") as archive:
                archive.writestr("[Content_Types].xml", "<Types/>")
            with self.assertRaises(harness.HarnessError):
                harness._validate_formula_workbook(stub)

    def test_publication_sheet_record_and_recomposition_are_exact(self) -> None:
        try:
            from PIL import Image, ImageDraw, PngImagePlugin
        except ImportError:
            self.skipTest("Pillow unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            sources = []
            for index, color in enumerate(((24, 48, 72), (72, 48, 24)), start=1):
                path = root / f"source-{index}.png"
                image = Image.new("RGB", (harness.CAPTURE_WIDTH, harness.CAPTURE_HEIGHT), color)
                ImageDraw.Draw(image).rectangle((0, 0, 250, 45), fill=(255, 214, 72))
                metadata = PngImagePlugin.PngInfo()
                metadata.add_text("structural_draft_warning", harness.WARNING)
                metadata.add_text("source_render_sha256", str(index) * 64)
                image.save(path, format="PNG", pnginfo=metadata, optimize=False)
                sources.append((f"source {index}", path))
            destination = root / "published" / "sheet.png"
            title = "Deterministic test sheet"
            record = harness._write_contact_sheet(sources, destination, title=title)
            self.assertTrue(record["visible_warning_overlay"])
            self.assertEqual(
                set(record),
                {
                    "path",
                    "width",
                    "height",
                    "sha256",
                    "warning",
                    "visible_warning_overlay",
                    "source_render_sha256",
                    "title",
                    "sources",
                },
            )
            harness._validate_publication_sheet_reproduction(
                {"path": destination, "title": title, "items": sources},
                record,
                index=0,
            )
            changed = dict(record)
            changed["title"] = "Changed title"
            with self.assertRaisesRegex(harness.HarnessError, "canonical source spec"):
                harness._validate_publication_sheet_reproduction(
                    {"path": destination, "title": title, "items": sources},
                    changed,
                    index=0,
                )

    def test_publication_paths_and_file_inventories_are_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            output_root = root / "publication"
            output_root.mkdir()
            canonical_plan = output_root / "capture-plan.json"
            canonical_plan.write_text("{}\n", encoding="utf-8")
            resolved, resolved_root = harness._require_publication_capture_plan_path(
                canonical_plan,
                {"output_root": str(output_root)},
            )
            self.assertEqual(resolved, canonical_plan.resolve())
            self.assertEqual(resolved_root, output_root.resolve())
            foreign_plan = root / "capture-plan.json"
            foreign_plan.write_text("{}\n", encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "output-root/capture-plan.json"):
                harness._require_publication_capture_plan_path(
                    foreign_plan,
                    {"output_root": str(output_root)},
                )

            inventory_root = output_root / "closed-tree"
            inventory_root.mkdir()
            expected = inventory_root / "expected.bin"
            expected.write_bytes(b"expected")
            harness._validate_exact_file_inventory(
                inventory_root,
                (expected,),
                context="test",
            )
            (inventory_root / "foreign.bin").write_bytes(b"foreign")
            with self.assertRaisesRegex(harness.HarnessError, "foreign"):
                harness._validate_exact_file_inventory(
                    inventory_root,
                    (expected,),
                    context="test",
                )

    def test_narrative_contract_rejects_stubs_and_placeholders(self) -> None:
        filler = (
            "Evidence was compared against the fresh current-source control with both reviewers, "
            "the named diagnostic camera, stress lighting, motion, and authority checks. "
        )
        research = " ".join(harness._REQUIRED_RESEARCH_URLS) + " " + filler * 12
        recommendations = (
            "Snow, water, clouds, shoreline and falls, alpine vegetation, cliff strata, "
            "terrain props and boulders, ice fringes, and local fog are each resolved. "
            + filler * 5
            + "review.json rankings.csv review-workbook.xlsx manifest.json "
            + "provenance/prior-aesthetic-report/README.md "
            + "provenance/prior-aesthetic-report/manifest.json"
        )
        rejected = (
            "The prior bevel 0.04 and 0.08 variants remain rejected for black cracks, "
            "vertical banding, and honeycomb noise. "
            + filler * 5
        )
        bodies = {
            "Research": research,
            "Recommendations": recommendations,
            "Rejected settings": rejected,
            "Interaction findings": filler * 6,
            "Limitations": filler * 6,
            "Implementation cost": filler * 6,
        }
        narrative = "# Final report\n\n> " + harness.WARNING + "\n\n" + "\n\n".join(
            f"## {heading}\n\n{body}" for heading, body in bodies.items()
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = pathlib.Path(temporary) / "README.md"
            path.write_text(narrative, encoding="utf-8")
            result = harness._validate_narrative(path)
            self.assertGreaterEqual(result["word_count"], 500)
            path.write_text(narrative + "\n\n{{unresolved.token}}\n", encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness._validate_narrative(path)
            path.write_text(narrative + "\n\nTODO\n", encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness._validate_narrative(path)
            path.write_text(
                narrative.replace("## Recommendations", "### Recommendations"),
                encoding="utf-8",
            )
            with self.assertRaises(harness.HarnessError):
                harness._validate_narrative(path)

            decisions = {
                family: next(
                    profile.id
                    for profile in harness.atomic_profiles()
                    if profile.family == family
                )
                for family in harness.FAMILY_ORDER
            }
            review_results = {
                "atomic": {
                    family: {"decision": decision}
                    for family, decision in decisions.items()
                },
                "combinations": {
                    combination_id: {"recommended": True}
                    for combination_id in harness.COMBINATION_IDS
                },
            }
            rows = ["| Family | Decision |", "| --- | --- |"]
            rows.extend(
                f"| {harness._ATOMIC_NARRATIVE_LABELS[family]} | `{decision}` |"
                for family, decision in decisions.items()
            )
            combined_rows = ["| Preset | Decision |", "| --- | --- |"]
            combined_rows.extend(
                f"| {harness._COMBINATION_NARRATIVE_LABELS[combination_id]} | `recommend` |"
                for combination_id in harness.COMBINATION_IDS
            )
            bound = narrative.replace(
                "## Research",
                "### Atomic recommendations\n\n"
                + "\n".join(rows)
                + "\n\n### Combined recommendations\n\n"
                + "\n".join(combined_rows)
                + "\n\n## Research",
            )
            path.write_text(bound, encoding="utf-8")
            harness._validate_narrative(path, review_results=review_results)
            path.write_text(
                bound.replace(
                    f"| Snow | `{decisions['snow']}` |",
                    "| Snow | `control/no change` |",
                ),
                encoding="utf-8",
            )
            with self.assertRaises(harness.HarnessError):
                harness._validate_narrative(path, review_results=review_results)
            path.write_text(
                bound.replace(
                    "| Restrained | `recommend` |",
                    "| Restrained | `do not recommend` |",
                ),
                encoding="utf-8",
            )
            with self.assertRaises(harness.HarnessError):
                harness._validate_narrative(path, review_results=review_results)

    def test_reviewer_rows_cannot_expose_real_subject_ids(self) -> None:
        review = self._review("review-a", phase=1)
        review["ratings"][0] = {
            "subject_id": "water-01-alpha-085",
            "condition": "neutral",
            "categories": {field: 4 for field in harness.CATEGORY_ORDER},
        }
        with self.assertRaises(harness.HarnessError):
            harness.validate_reviewer_review(review)

    def test_motion_performance_gate_uses_nearest_rank_and_fifteen_percent_cap(self) -> None:
        plan = harness.build_capture_document(
            pathlib.Path("/tmp/world-detail-performance-plan"), complete_selection()
        )
        records = {}
        control_ids = {
            job_id
            for clip in plan["motion"]["clips"]
            for job_id in clip["control_job_ids"]
        }
        for job in plan["motion"]["jobs"]:
            factor = 1.0 if job["id"] in control_ids else 1.10
            records[job["id"]] = {
                "performance_samples": [
                    {
                        "frame_time_ms": (10.0 + index / 1000.0) * factor,
                        "resident_presentation_bytes": int(1_000_000 * factor),
                        "warmup_complete": True,
                    }
                    for index in range(harness.MOTION_FRAME_COUNT)
                ]
            }
        result = harness.validate_performance_evidence(plan, records)
        self.assertEqual(len(result["leader_comparisons"]), 4)
        self.assertTrue(all(row["passed"] for row in result["leader_comparisons"]))
        leader_job_id = next(
            clip["candidate_job_ids"][0]
            for clip in plan["motion"]["clips"]
            if clip["id"] == "leader-golden-03"
        )
        for sample in records[leader_job_id]["performance_samples"]:
            sample["frame_time_ms"] *= 1.10
        with self.assertRaises(harness.HarnessError):
            harness.validate_performance_evidence(plan, records)


class ReportAndMetricTests(unittest.TestCase):
    SOURCE_PROVENANCE_SHA256 = "a" * 64
    CAPTURE_PLAN_JSON = '{"version":1,"captures":[]}'
    LAUNCH_NONCE = "b" * 64

    @classmethod
    def _runtime_receipt(cls, profile_json: str) -> dict:
        body = {
            "version": 1,
            "launch_nonce": cls.LAUNCH_NONCE,
            "process_id": 4242,
            "executable_sha256": "c" * 64,
            "source_provenance_sha256": cls.SOURCE_PROVENANCE_SHA256,
            "capture_plan_sha256": harness.sha256_bytes(cls.CAPTURE_PLAN_JSON.encode()),
            "profile_sha256": harness.sha256_bytes(profile_json.encode()),
        }
        return {
            **body,
            "receipt_sha256": harness.sha256_bytes(harness.compact_json(body).encode()),
        }

    def _valid_wrapper(self, png: pathlib.Path, camera: harness.CameraSpec, profile_json: str) -> dict:
        zero = {field: 0 for field in ("entities", "materials", "vertices", "triangles")}
        authority = {
            field: f"fingerprint-{field}"
            for field in (
                "voxel_map", "structural", "materialized", "liquid_graph", "topology",
                "traversal", "blockers", "anchors", "biomes", "feature_roots",
                "logical_terrain_picking", "gameplay_state",
            )
        }
        return {
            "version": 1,
            "warning": harness.WARNING,
            "capture": camera.expected_report_capture(
                png,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                settle_frames=90,
            ),
            "report": {
                "version": 1,
                "profile_hash_sha256": harness.sha256_bytes(profile_json.encode()),
                "runtime_receipt": self._runtime_receipt(profile_json),
                "authority": authority,
                "counts": {name: dict(zero) for name in ("total", *harness.FAMILY_ORDER)},
                "anchor_heights": {"grand_v3.massif_crest": 42.5},
                "anchor_classes": {"grand_v3.massif_crest": "observation"},
                "projection_hashes": {
                    "terrain_plan": "0123456789abcdef",
                    "liquid_atmosphere_plan": "1123456789abcdef",
                    "mesh_projection": "2123456789abcdef",
                },
                "effect_validation": {
                    "cloud_coverage": None,
                    "ice_coverage": None,
                    "fog_coverage": None,
                    "waterfall_anchors": [],
                },
                "camera_features": {
                    "oit": False,
                    "medium_transmission": False,
                    "depth_texture": False,
                    "volumetrics": False,
                },
                "performance": {
                    "frame_time_ms": 16.0,
                    "resident_presentation_bytes": 1_000_000,
                    "warmup_complete": True,
                },
                "cleanup": {
                    "completed_cycles": 1,
                    "entities_remaining": 0,
                    "materials_remaining": 0,
                    "meshes_remaining": 0,
                    "target_images_remaining": 0,
                    "camera_state_restored": True,
                    "oit_state_restored": True,
                    "transmission_state_restored": True,
                    "depth_state_restored": True,
                    "volumetric_state_restored": True,
                },
            },
        }

    def test_strict_runtime_wrapper_rejects_unknown_fields(self) -> None:
        camera = harness.load_camera_sets()[1][0]
        profile_json = harness.control_profile().canonical_json
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            png = (root / "frame.png").resolve()
            report = root / "frame.world-detail-report.json"
            wrapper = self._valid_wrapper(png, camera, profile_json)
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            result = harness.validate_runtime_report(
                report,
                png_path=png,
                camera=camera,
                profile_json=profile_json,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                capture_plan_json=self.CAPTURE_PLAN_JSON,
                expected_launch_nonce=self.LAUNCH_NONCE,
            )
            self.assertTrue(result["post_teardown_cleanup_complete"])
            wrapper["report"]["cleanup"]["target_images_remaining"] = 1
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=profile_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )

            stale = self._valid_wrapper(png, camera, profile_json)
            stale_body = {
                field: stale["report"]["runtime_receipt"][field]
                for field in harness.RUNTIME_RECEIPT_FIELDS
            }
            stale_body["launch_nonce"] = "d" * 64
            stale["report"]["runtime_receipt"] = {
                **stale_body,
                "receipt_sha256": harness.sha256_bytes(
                    harness.compact_json(stale_body).encode()
                ),
            }
            report.write_text(json.dumps(stale), encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "freshly launched"):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=profile_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )
            wrapper["report"]["cleanup"]["target_images_remaining"] = 0
            wrapper["report"]["unknown"] = 1
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=profile_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )

    def test_runtime_report_can_require_the_exact_lifecycle_teardown_count(self) -> None:
        camera = harness.load_camera_sets()[1][0]
        profile_json = harness.control_profile().canonical_json
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            png = (root / "cycle.png").resolve()
            report = root / "cycle.world-detail-report.json"
            wrapper = self._valid_wrapper(png, camera, profile_json)
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "teardown count 100"):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=profile_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                    expected_completed_cycles=100,
                )
            wrapper["report"]["cleanup"]["completed_cycles"] = 100
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            result = harness.validate_runtime_report(
                report,
                png_path=png,
                camera=camera,
                profile_json=profile_json,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                capture_plan_json=self.CAPTURE_PLAN_JSON,
                expected_launch_nonce=self.LAUNCH_NONCE,
                expected_completed_cycles=100,
            )
            self.assertEqual(result["cleanup"]["completed_cycles"], 100)

    def test_runtime_sidecars_are_published_byte_identically(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            output = root / "publication"
            for marker in ("raw-stills", "raw-motion", "raw-motion-controls"):
                raw = (
                    root
                    / "capture-work"
                    / "runtime"
                    / marker
                    / "nested"
                    / "frame.world-detail-report.json"
                )
                raw.parent.mkdir(parents=True, exist_ok=True)
                raw.write_text('{"runtime":true}', encoding="utf-8")
                record = harness._materialize_runtime_report(output, raw, create=True)
                published = pathlib.Path(record["path"])
                self.assertEqual(published.read_bytes(), raw.read_bytes())
                self.assertIn(f"runtime-reports/{marker}", published.as_posix())
                published.write_text('{"runtime":false}', encoding="utf-8")
                with self.assertRaises(harness.HarnessError):
                    harness._materialize_runtime_report(output, raw, create=False)
                published.unlink()
                with self.assertRaises(harness.HarnessError):
                    harness._materialize_runtime_report(output, raw, create=False)

    def test_motion_public_pixels_and_strip_expose_only_opaque_code(self) -> None:
        try:
            from PIL import Image
        except ImportError:
            self.skipTest("Pillow unavailable")
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            raw_control = root / "control.png"
            raw_candidate = root / "candidate.png"
            gradient = Image.linear_gradient("L").resize(
                (harness.CAPTURE_WIDTH, harness.CAPTURE_HEIGHT)
            )
            Image.merge("RGB", (gradient, gradient.transpose(Image.Transpose.FLIP_TOP_BOTTOM), gradient)).save(raw_control)
            Image.merge("RGB", (gradient.transpose(Image.Transpose.FLIP_LEFT_RIGHT), gradient, gradient)).save(raw_candidate)
            control = root / "control-labeled.png"
            candidate = root / "candidate-labeled.png"
            harness.label_png(raw_control, control)
            harness.label_png(raw_candidate, candidate)
            code = "RV-0123456789AB"
            pair = root / "pair.png"
            strip = root / "strip.png"
            harness._write_paired_motion_frame(control, candidate, pair, code)
            harness._write_eight_frame_strip([pair] * 8, strip, code)
            with Image.open(strip) as image:
                self.assertEqual(image.info["opaque_review_code"], code)
                self.assertEqual(
                    set(image.info),
                    {
                        "structural_draft_warning",
                        "opaque_review_code",
                        "strip_source_frame_sha256",
                    },
                )
            payload = strip.read_bytes()
            for forbidden in (b"water-", b"snow-", b"score-leader", b"golden", b"camera"):
                self.assertNotIn(forbidden, payload)

    def test_mp4_validation_requires_exact_frame_count_and_duration(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            clip = pathlib.Path(temporary) / "clip.mp4"
            clip.write_bytes(b"0" * 2048)
            payload = {
                "streams": [
                    {
                        "codec_name": "h264",
                        "width": harness.CAPTURE_WIDTH,
                        "height": harness.CAPTURE_HEIGHT,
                        "r_frame_rate": "30/1",
                        "nb_frames": "N/A",
                        "nb_read_frames": "N/A",
                        "duration": "3.0",
                    }
                ],
                "format": {"duration": "3.0"},
            }
            completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(payload), stderr="")
            with mock.patch.object(harness.shutil, "which", return_value="/fake/ffprobe"), mock.patch.object(
                harness.subprocess, "run", return_value=completed
            ):
                with self.assertRaisesRegex(harness.HarnessError, "frame count"):
                    harness._validate_mp4(clip, expected_frames=90, expected_fps=30)
            payload["streams"][0]["nb_read_frames"] = "90"
            payload["streams"][0]["duration"] = "2.5"
            completed = subprocess.CompletedProcess([], 0, stdout=json.dumps(payload), stderr="")
            with mock.patch.object(harness.shutil, "which", return_value="/fake/ffprobe"), mock.patch.object(
                harness.subprocess, "run", return_value=completed
            ):
                with self.assertRaisesRegex(harness.HarnessError, "duration"):
                    harness._validate_mp4(clip, expected_frames=90, expected_fps=30)

    def test_physical_clouds_require_oit_and_depth(self) -> None:
        camera = harness.load_camera_sets()[1][0]
        profile = next(
            profile for profile in harness.atomic_profiles() if profile.id == "clouds-01-faceted-clear"
        )
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            png = (root / "cloud.png").resolve()
            report = root / "cloud.world-detail-report.json"
            wrapper = self._valid_wrapper(png, camera, profile.canonical_json)
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaises(harness.HarnessError):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=profile.canonical_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )
            wrapper["report"]["camera_features"]["oit"] = True
            wrapper["report"]["camera_features"]["depth_texture"] = True
            wrapper["report"]["effect_validation"]["cloud_coverage"] = {
                "field_radius": 97.24,
                "target_fraction": 0.18,
                "measured_fraction": 0.18,
                "tolerance": 0.01,
                "sample_count": 4096,
                "cloud_clusters": 12,
                "peak_intersection_required": False,
                "peak_intersecting_puffs": 0,
            }
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            harness.validate_runtime_report(
                report,
                png_path=png,
                camera=camera,
                profile_json=profile.canonical_json,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                capture_plan_json=self.CAPTURE_PLAN_JSON,
                expected_launch_nonce=self.LAUNCH_NONCE,
            )

    def test_ice_and_waterfall_effect_measurements_are_exact(self) -> None:
        camera = harness.load_camera_sets()[1][0]
        profiles = {profile.id: profile for profile in harness.atomic_profiles()}
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            png = (root / "effect.png").resolve()
            report = root / "effect.world-detail-report.json"

            ice = profiles["ice-01-level-narrow"]
            wrapper = self._valid_wrapper(png, camera, ice.canonical_json)
            wrapper["report"]["camera_features"]["oit"] = True
            wrapper["report"]["camera_features"]["depth_texture"] = True
            wrapper["report"]["effect_validation"]["ice_coverage"] = {
                "target_fraction": 0.4,
                "eligible_edges": 3,
                "selected_edges": 2,
            }
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            harness.validate_runtime_report(
                report,
                png_path=png,
                camera=camera,
                profile_json=ice.canonical_json,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                capture_plan_json=self.CAPTURE_PLAN_JSON,
                expected_launch_nonce=self.LAUNCH_NONCE,
            )
            wrapper["report"]["effect_validation"]["ice_coverage"]["selected_edges"] = 1
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "exact ceiling"):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=ice.canonical_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )

            shore = profiles["shore-05-plunge-spray"]
            wrapper = self._valid_wrapper(png, camera, shore.canonical_json)
            wrapper["report"]["anchor_heights"]["grand_v3.waterfall_base"] = 12.0
            wrapper["report"]["anchor_classes"]["grand_v3.waterfall_base"] = "observation"
            wrapper["report"]["camera_features"]["oit"] = True
            wrapper["report"]["camera_features"]["depth_texture"] = True
            wrapper["report"]["effect_validation"]["waterfall_anchors"] = [
                {
                    "anchor_name": "grand_v3.waterfall_base",
                    "anchor_position": [1, 2, 3],
                    "landing_position": [2, 2, 1],
                    "distance_hexes": 1,
                }
            ]
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            harness.validate_runtime_report(
                report,
                png_path=png,
                camera=camera,
                profile_json=shore.canonical_json,
                time_hours=12.0,
                liquid_phase_seconds=0.0,
                source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                capture_plan_json=self.CAPTURE_PLAN_JSON,
                expected_launch_nonce=self.LAUNCH_NONCE,
            )
            wrapper["report"]["effect_validation"]["waterfall_anchors"][0][
                "distance_hexes"
            ] = 2
            report.write_text(json.dumps(wrapper), encoding="utf-8")
            with self.assertRaisesRegex(harness.HarnessError, "exact axial distance"):
                harness.validate_runtime_report(
                    report,
                    png_path=png,
                    camera=camera,
                    profile_json=shore.canonical_json,
                    time_hours=12.0,
                    liquid_phase_seconds=0.0,
                    source_provenance_sha256=self.SOURCE_PROVENANCE_SHA256,
                    capture_plan_json=self.CAPTURE_PLAN_JSON,
                    expected_launch_nonce=self.LAUNCH_NONCE,
                )

    def test_reproduction_compares_current_report_contract_and_pixels(self) -> None:
        try:
            from PIL import Image
        except ImportError as error:  # pragma: no cover - dependency contract
            self.skipTest(str(error))
        camera = harness.load_camera_sets()[1][0]
        profile_json = harness.control_profile().canonical_json
        with tempfile.TemporaryDirectory() as temporary:
            root = pathlib.Path(temporary)
            first = (root / "first.png").resolve()
            second = (root / "second.png").resolve()
            gradient = Image.linear_gradient("L").resize(
                (harness.CAPTURE_WIDTH, harness.CAPTURE_HEIGHT)
            )
            Image.merge(
                "RGB",
                (
                    gradient,
                    gradient.transpose(Image.Transpose.FLIP_TOP_BOTTOM),
                    gradient,
                ),
            ).save(first)
            second.write_bytes(first.read_bytes())
            first_wrapper = self._valid_wrapper(first, camera, profile_json)
            second_wrapper = self._valid_wrapper(second, camera, profile_json)
            harness.runtime_report_path(first).write_text(
                json.dumps(first_wrapper), encoding="utf-8"
            )
            harness.runtime_report_path(second).write_text(
                json.dumps(second_wrapper), encoding="utf-8"
            )
            evidence = harness.verify_reproduction(first, second)
            self.assertEqual(
                evidence["decoded_rgb_sha256"], harness.decoded_rgb_sha256(first)
            )
            self.assertTrue(evidence["raw_decoded_rgb_identical"])
            self.assertTrue(evidence["raster_stable_pixel_identical"])

            with Image.open(first) as source:
                first_image = source.convert("RGB")
            with Image.open(second) as source:
                second_image = source.convert("RGB")
            _, oracle = harness._baseline_oracle_documents()
            ambiguous = oracle["cameras"][camera.id]["ambiguous_pixels"][0]
            coordinate = (ambiguous["x"], ambiguous["y"])
            reference_rgb = first_image.getpixel(coordinate)
            treatment_specific_rgb = (
                (reference_rgb[0] + 1) % 256,
                reference_rgb[1],
                reference_rgb[2],
            )
            self.assertNotIn(list(reference_rgb), ambiguous["allowed_rgb"])
            self.assertNotIn(list(treatment_specific_rgb), ambiguous["allowed_rgb"])
            second_image.putpixel(coordinate, treatment_specific_rgb)
            first_image.save(first)
            second_image.save(second)
            ambiguous_evidence = harness.verify_reproduction(first, second)
            self.assertFalse(ambiguous_evidence["raw_decoded_rgb_identical"])
            self.assertTrue(ambiguous_evidence["raster_stable_pixel_identical"])
            self.assertEqual(ambiguous_evidence["differing_ambiguous_pixel_count"], 1)
            self.assertEqual(
                ambiguous_evidence["ambiguous_value_policy"],
                "oracle-coordinate-mask-with-endpoints-recorded",
            )

            second_wrapper["report"]["projection_hashes"]["mesh_projection"] = (
                "ffffffffffffffff"
            )
            harness.runtime_report_path(second).write_text(
                json.dumps(second_wrapper), encoding="utf-8"
            )
            with self.assertRaisesRegex(harness.HarnessError, "plan/mesh"):
                harness.verify_reproduction(first, second)

    def test_ciede2000_reference_pair(self) -> None:
        try:
            import numpy as np
        except ImportError:
            self.skipTest("NumPy unavailable")
        first = np.array([50.0, 2.6772, -79.7751])
        second = np.array([50.0, 0.0, -82.7485])
        self.assertAlmostEqual(float(harness._delta_e_2000(first, second)), 2.0425, places=4)


if __name__ == "__main__":
    unittest.main()
