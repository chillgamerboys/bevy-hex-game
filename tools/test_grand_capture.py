"""Capture provenance regressions; no renderer, Cargo, or existing package required."""
from __future__ import annotations

import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

# Support direct script, discovery, and repository-root module invocations.
sys.path.insert(0, str(Path(__file__).resolve().parent))
import grand_capture


class GrandCaptureProvenanceTests(unittest.TestCase):
    def test_profile_commands_keep_legacy_features_and_allow_lean_ci(self):
        dev = grand_capture.cargo_arguments("dev")
        ci = grand_capture.cargo_arguments("ci")
        self.assertIn("dev,arena-prototype", dev)
        self.assertIn("arena-prototype", ci)
        self.assertNotIn("dev,arena-prototype", ci)
        self.assertIn("arena-prototype,test-support", grand_capture.cargo_arguments("ci", test_support=True))
        original = grand_capture.arena.CARGO_ARGS
        for supplied, expected in ((None, original), (ci, ci)):
            with patch.object(grand_capture.arena.subprocess, "Popen", side_effect=RuntimeError("launch intercepted")) as launch:
                with self.assertRaisesRegex(RuntimeError, "launch intercepted"):
                    grand_capture.arena.run_cargo({}, None, 1, args=supplied)
                self.assertEqual(launch.call_args.args[0], ("cargo", *expected))
        self.assertEqual(grand_capture.arena.CARGO_ARGS, original)

    def package(self, directory: Path, identity: dict | None) -> None:
        for name in ("manifest.ron", "grand-overview.ron", "arena-sites.ron", "grand-biomes.ron"):
            (directory / name).write_text("fixture")
        receipt = {"strict": True, "world_id": "grand-v4", "package_fingerprint": 42,
                   "mainland_columns": 653261, "canonical_mainland_columns": 93326,
                   "mainland_target_columns": 653282, "mainland_tolerance_columns": 65,
                   "mainland_area_ratio": 653261 / 93326,
                   "crystal_columns": 22201, "canonical_crystal_columns": 3169,
                   "crystal_target_columns": 22183, "crystal_authored_columns": 22201,
                   "crystal_area_ratio": 22201 / 3169,
                   "crystal_footprint_basis": "authored_outer_hex"}
        (directory / "compile-receipt.json").write_text(json.dumps(receipt))
        if identity is not None:
            (directory / "authoring-identity.json").write_text(json.dumps(identity))

    def test_missing_authoring_identity_rejects_otherwise_strict_old_package(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.package(directory, None)
            with self.assertRaisesRegex(RuntimeError, "authoring-identity.json"):
                grand_capture.package_state(directory)

    def test_stale_or_plain_packages_cannot_borrow_current_source_provenance(self):
        for identity in ({"signature": "old-dressed", "plain": False},
                         {"signature": "current-plain", "plain": True},
                         {"signature": "current-dressed"}):
            with self.subTest(identity=identity), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.package(directory, identity)
                with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                    with self.assertRaisesRegex(RuntimeError, "stale or differs from requested dressing mode"):
                        grand_capture.package_state(directory)

    def test_matching_dressed_identity_is_recorded_with_frozen_file_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            identity = {"signature": "current-dressed", "plain": False, "compiler_mode": "cargo-current-source", "cargo_profile": "ci"}
            self.package(directory, identity)
            with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                result = grand_capture.package_state(directory)
            self.assertEqual(result["authoring_identity"], identity)
            self.assertIn("sha256", result["files"]["authoring-identity.json"])

    def test_plain_capture_requires_explicit_matching_mode(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            identity = {"signature": "current-plain", "plain": True,
                        "compiler_mode": "cargo-current-source", "cargo_profile": "ci"}
            self.package(directory, identity)
            with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                with self.assertRaisesRegex(RuntimeError, "dressing mode"):
                    grand_capture.package_state(directory)
                result = grand_capture.package_state(directory, plain=True)
                self.assertTrue(result["authoring_identity"]["plain"])
                identity.update(signature="current-dressed", plain=False)
                self.package(directory, identity)
                with self.assertRaisesRegex(RuntimeError, "dressing mode"):
                    grand_capture.package_state(directory, plain=True)

    def test_surface_sample_cannot_borrow_ordinary_capture_provenance(self):
        mode = "crystal-four"
        chunks = [{"q": 28, "r": -20}, {"q": 28, "r": -19},
                  {"q": 29, "r": -20}, {"q": 29, "r": -19}]
        identity = {"presentation_sample": mode}
        receipt = {"diagnostic_presentation_sample": mode, "diagnostic_surface_chunks": chunks}
        grand_capture.validate_presentation_sample(identity, receipt, mode)
        for requested, stamped, compiled in (
                (None, identity, receipt), (mode, {}, receipt), (mode, identity, {}),
                (mode, identity, receipt | {"diagnostic_surface_chunks": chunks[:-1]}),
                (mode, identity, receipt | {"diagnostic_surface_chunks": chunks[::-1]})):
            with self.subTest(requested=requested, compiled=compiled):
                with self.assertRaises(RuntimeError):
                    grand_capture.validate_presentation_sample(stamped, compiled, requested)
        grand_capture.validate_presentation_sample({}, {}, None)

    def test_common_landform_area_uses_fixed_source_policy(self):
        measurement = json.loads((grand_capture.ROOT / "assets/config/v4/grand-v4/measurement.json").read_text())
        geography = json.loads((grand_capture.ROOT / "assets/config/v4/grand-v4/geography-r02.json").read_text())
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            self.package(directory, {})
            receipt = json.loads((directory / "compile-receipt.json").read_text())
        receipt.update(mainland_columns=657763, mainland_area_ratio=657763 / 93326,
                       mainland_tolerance_columns=13065)
        grand_capture.validate_area(receipt, measurement, geography | {"landform_coast": True})
        for current in (geography | {"landform_coast": False},
                        {k: v for k, v in geography.items() if k != "landform_coast"}):
            with self.assertRaisesRegex(RuntimeError, "area contract"):
                grand_capture.validate_area(receipt, measurement, current)
        # The receipt cannot choose a wider allowance than the source policy.
        with self.assertRaisesRegex(RuntimeError, "area contract"):
            grand_capture.validate_area(receipt | {"mainland_tolerance_columns": 13066},
                                        measurement, geography | {"landform_coast": True})

    def test_surface_render_needs_actual_visible_retained_products(self):
        chunks = [{"q": 28, "r": -20}, {"q": 28, "r": -19},
                  {"q": 29, "r": -20}, {"q": 29, "r": -19}]
        package = {"authoring_identity": {"presentation_sample": "crystal-four"},
                   "compiler_receipt": {"package_fingerprint": 42, "source_fingerprint": 43,
                                        "diagnostic_surface_chunks": chunks}}
        snapshot = {"mode": "crystal-four", "package_fingerprint": 42, "source_fingerprint": 43,
                    "selected_chunks": chunks, "source_chunks": 14, "converted_chunks": 4,
                    "published_chunks": 4, "visible_chunks": 2, "vertices": 100, "triangles": 50,
                    "packed_bytes": 4600, "packed_byte_limit": 16 * 1024 * 1024,
                    "exact_edited_fallback": False}
        grand_capture.validate_surface_snapshot(snapshot, package, "candidate")
        grand_capture.validate_surface_snapshot(None, package, "baseline")
        for invalid in (None, snapshot | {"visible_chunks": 0},
                        snapshot | {"published_chunks": 3}, snapshot | {"packed_bytes": 4601},
                        snapshot | {"source_fingerprint": 44},
                        snapshot | {"exact_edited_fallback": True}):
            with self.subTest(snapshot=invalid), self.assertRaises(RuntimeError):
                grand_capture.validate_surface_snapshot(invalid, package, "candidate")
        with self.assertRaises(RuntimeError):
            grand_capture.validate_surface_snapshot(snapshot, package, "baseline")

    def test_current_signature_cannot_approve_unverified_prebuilt_compiler(self):
        for compiler_mode in (None, "prebuilt-unverified"):
            with self.subTest(mode=compiler_mode), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.package(directory, {"signature": "current-dressed", "plain": False,
                                         "compiler_mode": compiler_mode})
                with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                    with self.assertRaisesRegex(RuntimeError, "current-source compiler provenance"):
                        grand_capture.package_state(directory)

    def test_reserved_or_mismatched_crystal_area_cannot_approve_emitted_geometry(self):
        invalid = (
            {"crystal_columns": 22183},  # Old reservation is not emitted geometry.
            {"crystal_columns": 22200, "crystal_authored_columns": 22200},
            {"crystal_columns": 22202, "crystal_authored_columns": 22202},
            {"crystal_target_columns": 22201},
            {"canonical_crystal_columns": 3171},
            {"crystal_area_ratio": 7.0},
            {"crystal_area_ratio": float("nan")},
            {"crystal_footprint_basis": "legacy_reserved_footprint"},
            {"mainland_columns": 700, "canonical_mainland_columns": 100},
            {"mainland_columns": 653216, "mainland_area_ratio": 653216 / 93326},
            {"mainland_tolerance_columns": 66},
            {"mainland_area_ratio": 7.0},
        )
        for change in invalid:
            with self.subTest(change=change), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.package(directory, {"signature": "current-dressed", "plain": False,
                                         "compiler_mode": "cargo-current-source"})
                path = directory / "compile-receipt.json"
                receipt = json.loads(path.read_text())
                receipt.update(change)
                path.write_text(json.dumps(receipt))
                with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                    with self.assertRaisesRegex(RuntimeError, "area contract"):
                        grand_capture.package_state(directory)

    def receipt(self, selected=None, captured=None):
        receipt = grand_capture.matrix_contract(selected)
        receipt["frames"] = [{"view": view, "mechanical_status": "CAPTURED"}
                             for view in (receipt["requested_views"] if captured is None else captured)]
        return receipt

    def test_selected_subset_is_explicit_partial_diagnostic(self):
        receipt = self.receipt(["grand-garden", "grand-library"])
        grand_capture.complete_matrix(receipt)
        self.assertEqual(receipt["matrix_scope"], "FOCUSED-DIAGNOSTIC")
        self.assertEqual(receipt["mechanical_status"], "PARTIAL_COMPLETE")
        self.assertFalse(receipt["full_matrix_completed"])
        self.assertEqual(len(receipt["missing_views"]), 45)

    def test_full_matrix_needs_every_declared_frame(self):
        self.assertEqual(len(grand_capture.VIEWS), 47)
        receipt = self.receipt()
        grand_capture.complete_matrix(receipt)
        self.assertTrue(receipt["full_matrix_completed"])
        self.assertEqual(receipt["mechanical_status"], "COMPLETE")
        self.assertEqual(receipt["missing_views"], [])
        unfinished = self.receipt(captured=list(grand_capture.VIEWS[:-1]))
        with self.assertRaisesRegex(RuntimeError, "not all captured"):
            grand_capture.complete_matrix(unfinished)
        self.assertFalse(unfinished["full_matrix_completed"])

    def test_duplicate_unknown_and_empty_subsets_are_rejected(self):
        for views in ([], ["grand-garden", "grand-garden"], ["unknown"]):
            with self.subTest(views=views), self.assertRaises(RuntimeError):
                grand_capture.matrix_contract(views)

    def test_source_or_package_failure_after_frames_cannot_claim_full_completion(self):
        receipt = self.receipt()
        receipt["mechanical_status"] = "BLOCKED"
        receipt["error"] = "Source/package changed during capture"
        grand_capture.update_coverage(receipt)
        self.assertEqual(len(receipt["completed_views"]), 47)
        self.assertFalse(receipt["full_matrix_completed"])


if __name__ == "__main__":
    unittest.main()
