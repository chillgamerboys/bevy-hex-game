"""Temporal evidence rejects static, discontinuous, incomplete or stale records."""
from __future__ import annotations

import copy
from pathlib import Path
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
import grand_motion


def sequence(route="forest-forward"):
    rows = []
    for index in range(24):
        seconds = (index + 1) * 0.4
        x = seconds * 4.5
        rows.append({
            "route": route, "index": index,
            "package_identity": {"world_id": "grand-v4", "manifest_fingerprint": 42},
            "body_valid": True, "relocations": 1, "feet": [x, 140.0, 0.0],
            "eye": [x, 140.93, 0.0], "body_dimensions": [0.5, 1.2, 0.5],
            "camera": {"position": [x, 140.93, 0.0], "rotation": [0.0, 0.0, 0.0, 1.0]},
            "camera_mode": "ordinary-first-person", "tick": (index + 1) * 48,
            "frame": (index + 1) * 24, "start_tick": 0, "start_frame": 0,
            "start_feet": [0.0, 140.0, 0.0], "direction": [1.0, 0.0, 0.0],
            "ocean_generation": 0, "ocean_time_seconds": seconds,
            "rendered_ocean_time_seconds": seconds, "river_phase": seconds % 4 / 4,
            "readback_completion": {"tick": (index + 1) * 48 + 4, "frame": (index + 1) * 24 + 2},
            "progress_units": x, "chunk": [index // 10, 0],
            "forest_proxy_visible": ["oak"] if index < 12 else [],
            "forest_proxy_hidden": [] if index < 12 else ["oak"],
            "forest_proxy_in_view": ["oak"] if index < 12 else [],
        })
    return rows


class GrandMotionEvidenceTests(unittest.TestCase):
    def test_continuous_sequence_spans_real_time_distance_and_handoff(self):
        result = grand_motion.validate_sequence(sequence(), "forest-forward", 42)
        self.assertAlmostEqual(result["simulation_seconds"], 9.2)
        self.assertAlmostEqual(result["forward_progress_units"], 41.4)
        self.assertEqual(result["forest_handoff_objects"], 1)
        self.assertEqual(result["in_frustum_handoff_candidates"], 1)
        self.assertEqual(result["visible_handoff_confirmation"], "PENDING_IMAGE_REVIEW")
        self.assertAlmostEqual(result["sample_frequency_hz"], 2.5)
        self.assertIn("human control feel", result["evidence"])

    def test_missing_duplicate_and_reordered_frames_fail(self):
        original = sequence()
        for rows in (original[:-1], original[:10] + original[9:23], list(reversed(original))):
            with self.subTest(indices=[r["index"] for r in rows]), self.assertRaisesRegex(RuntimeError, "24 ordered"):
                grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_wrong_package_route_and_invalid_body_fail(self):
        for change in ({"route": "river-forward"}, {"package_identity": {"world_id": "grand-v4", "manifest_fingerprint": 9}},
                       {"body_valid": False}, {"relocations": 2}, {"feet": [float("nan"), 1, 2]}):
            rows = sequence()
            rows[5].update(change)
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_static_pose_with_advancing_clock_is_not_motion(self):
        rows = sequence()
        for row in rows:
            row["feet"] = [0, 140, 0]
        with self.assertRaisesRegex(RuntimeError, "continuous movement"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_static_camera_with_walking_body_is_not_camera_motion(self):
        rows = sequence()
        for row in rows:
            row["camera"]["position"] = [0, 140.93, 0]
        with self.assertRaisesRegex(RuntimeError, "camera did not follow"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_relabelled_clock_and_frozen_river_fail(self):
        for name in ("tick", "river_phase", "rendered_ocean_time_seconds"):
            rows = sequence()
            rows[12][name] = rows[11][name]
            with self.subTest(field=name), self.assertRaises(RuntimeError):
                grand_motion.validate_sequence(rows, "forest-forward", 42)
        rows = sequence()
        rows[12]["tick"] += 1
        with self.assertRaisesRegex(RuntimeError, "relabelled"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_injected_teleport_or_changed_start_fail(self):
        for change in ({"feet": [1000, 140, 0]}, {"start_feet": [10, 140, 0]}, {"ocean_generation": 2}):
            rows = sequence()
            rows[12].update(change)
            with self.subTest(change=change), self.assertRaises(RuntimeError):
                grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_forest_requires_real_visible_hidden_transition_not_just_changed_count(self):
        rows = sequence()
        for row in rows:
            row["forest_proxy_visible"] = ["other"]
        with self.assertRaisesRegex(RuntimeError, "actual proxy handoff"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)
        rows = sequence()
        for row in rows:
            row["chunk"] = [0, 0]
        with self.assertRaisesRegex(RuntimeError, "chunk boundary"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)

    def test_river_does_not_need_forest_handoff(self):
        rows = sequence("river-forward")
        for row in rows:
            row["forest_proxy_hidden"] = []
            row["forest_proxy_visible"] = []
        self.assertEqual(grand_motion.validate_sequence(rows, "river-forward", 42)["forest_handoff_objects"], 0)

    def test_readback_completion_is_distinct_from_request_and_cannot_precede_it(self):
        rows = sequence()
        self.assertGreater(rows[0]["readback_completion"]["tick"], rows[0]["tick"])
        for completion in ({}, {"frame": 0, "tick": 0}):
            bad = copy.deepcopy(rows)
            bad[10]["readback_completion"] = completion
            with self.subTest(completion=completion), self.assertRaisesRegex(RuntimeError, "readback"):
                grand_motion.validate_sequence(bad, "forest-forward", 42)

    def test_frame_budget_cannot_be_raised_by_a_partial_run(self):
        rows = sequence()
        rows[-1]["frame"] = 901
        rows[-1]["readback_completion"]["frame"] = 903
        with self.assertRaisesRegex(RuntimeError, "bounded frame"):
            grand_motion.validate_sequence(rows, "forest-forward", 42)


if __name__ == "__main__":
    unittest.main()
