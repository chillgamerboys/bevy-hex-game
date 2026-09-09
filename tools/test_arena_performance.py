"""Admission settings for the explicitly synthetic encounter workload."""
import unittest

from arena import validate_encounter_stress_home_leashes


class StressLeashGuards(unittest.TestCase):
    def test_every_tick_discloses_the_three_configured_home_leashes(self):
        rows = [{"stimulus": {"home_leashes": [150.0, 150.0, 150.0]}} for _ in range(3)]
        validate_encounter_stress_home_leashes(rows)
        rows[-1]["stimulus"]["home_leashes"] = [18.0, 24.0, 32.0]
        with self.assertRaisesRegex(RuntimeError, "every tick"):
            validate_encounter_stress_home_leashes(rows)

    def test_missing_or_partial_settings_do_not_inherit_a_pass(self):
        for rows in ([], None, [{}], [{"stimulus": {}}],
                     [{"stimulus": {"home_leashes": [150.0, 150.0]}}]):
            with self.subTest(rows=rows), self.assertRaises(RuntimeError):
                validate_encounter_stress_home_leashes(rows)


if __name__ == "__main__":
    unittest.main()
