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
                   "mainland_columns": 653282, "canonical_mainland_columns": 93326,
                   "crystal_columns": 22183, "canonical_crystal_columns": 3169}
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

    def test_current_signature_cannot_approve_unverified_prebuilt_compiler(self):
        for compiler_mode in (None, "prebuilt-unverified"):
            with self.subTest(mode=compiler_mode), tempfile.TemporaryDirectory() as temporary:
                directory = Path(temporary)
                self.package(directory, {"signature": "current-dressed", "plain": False,
                                         "compiler_mode": compiler_mode})
                with patch.object(grand_capture.grand_package, "signature", return_value="current"):
                    with self.assertRaisesRegex(RuntimeError, "current-source compiler provenance"):
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
        self.assertEqual(len(receipt["missing_views"]), 37)

    def test_full_matrix_needs_every_declared_frame(self):
        self.assertEqual(len(grand_capture.VIEWS), 39)
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
        self.assertEqual(len(receipt["completed_views"]), 39)
        self.assertFalse(receipt["full_matrix_completed"])


if __name__ == "__main__":
    unittest.main()
