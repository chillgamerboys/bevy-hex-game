"""Canonical measurement writes must never replace approved geography."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import grand_measure


class GrandMeasurementTests(unittest.TestCase):
    def test_refreshing_canonical_receipt_preserves_authored_world_and_geography(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "assets/config/v4/grand-v4"
            destination.mkdir(parents=True)
            expected = {"world.ron": b"(geography:Some(\"geography-r02.json\"))",
                        "geography-r02.json": b'{"approved":"connected massif"}'}
            for name, data in expected.items():
                (destination / name).write_bytes(data)
            with patch.object(grand_measure, "ROOT", root):
                grand_measure.write_measurement({"canonical_mainland_columns": 93326})
            self.assertTrue((destination / "measurement.json").is_file())
            for name, data in expected.items():
                self.assertEqual((destination / name).read_bytes(), data)


if __name__ == "__main__":
    unittest.main()
