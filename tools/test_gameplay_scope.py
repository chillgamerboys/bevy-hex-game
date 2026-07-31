"""Tests for the fail-closed gameplay scope selector."""

from __future__ import annotations

import importlib.util
import pathlib
import sys
import tempfile
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "tools" / "gameplay_scope.py"
SPEC = importlib.util.spec_from_file_location("gameplay_scope", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load gameplay_scope")
gameplay_scope = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = gameplay_scope
SPEC.loader.exec_module(gameplay_scope)


class GameplayScopeTests(unittest.TestCase):
    """The selector chooses the required concern closure and fails closed."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.config = gameplay_scope.load_config()

    def classify(self, *paths: str):
        return gameplay_scope.classify(paths, self.config)

    def test_lattice_change_selects_rules_contracts_and_simulation(self) -> None:
        decision = self.classify("crates/hex_lattice/src/cast.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("rules", "contracts", "simulation", "clippy", "docs"),
        )

    def test_combat_lab_change_selects_app_and_shipping(self) -> None:
        decision = self.classify("crates/hex_game/src/screens/combat_lab.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns, ("app", "clippy", "docs", "shipping")
        )

    def test_combat_authority_change_selects_its_downstream_closure(self) -> None:
        decision = self.classify("crates/hex_combat_core/src/authority.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("contracts", "simulation", "app", "clippy", "docs", "shipping"),
        )

    def test_gameplay_asset_rule_precedes_shared_asset_fallback(self) -> None:
        decision = self.classify("crates/hex_assets/src/combat_rules.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            (
                "rules",
                "contracts",
                "simulation",
                "app",
                "clippy",
                "docs",
                "shipping",
            ),
        )

    def test_generic_loader_change_uses_shared_asset_fallback(self) -> None:
        decision = self.classify("crates/hex_assets/src/loader.rs")
        self.assertTrue(decision.full)

    def test_shared_core_change_selects_full_gate(self) -> None:
        decision = self.classify("crates/hex_core/src/position.rs")
        self.assertTrue(decision.full)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))

    def test_map_change_retains_full_owner_coverage(self) -> None:
        decision = self.classify("crates/hex_map/src/procedural_v3/ring7.rs")
        self.assertTrue(decision.full)

    def test_markdown_only_change_does_not_select_rust_code(self) -> None:
        decision = self.classify("docs/development/gameplay-testing.md")
        self.assertFalse(decision.code)
        self.assertEqual(decision.concerns, ("docs",))

    def test_root_markdown_is_recognized(self) -> None:
        decision = self.classify("README.md")
        self.assertFalse(decision.code)
        self.assertEqual(decision.concerns, ("docs",))

    def test_contributor_instruction_change_is_documentation_only(self) -> None:
        decision = self.classify(".claude/skills/test-quick/SKILL.md")
        self.assertFalse(decision.code)
        self.assertEqual(decision.concerns, ("docs",))

    def test_unknown_path_fails_closed(self) -> None:
        decision = self.classify("unexpected/new-system/file.ron")
        self.assertTrue(decision.full)
        self.assertEqual(
            decision.unknown_files, ("unexpected/new-system/file.ron",)
        )

    def test_empty_diff_fails_closed(self) -> None:
        decision = self.classify()
        self.assertTrue(decision.full)
        self.assertTrue(decision.code)

    def test_mixed_diff_unions_concerns(self) -> None:
        decision = self.classify(
            "crates/hex_lattice/src/cast.rs",
            "crates/hex_game/src/screens/combat_lab.rs",
        )
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            (
                "rules",
                "contracts",
                "simulation",
                "app",
                "clippy",
                "docs",
                "shipping",
            ),
        )

    def test_scope_infrastructure_change_runs_everything(self) -> None:
        decision = self.classify(".config/gameplay-test-scopes.json")
        self.assertTrue(decision.full)

    def test_push_gate_promotes_a_narrow_decision_to_full(self) -> None:
        narrow = self.classify("crates/hex_lattice/src/cast.rs")
        decision = gameplay_scope.force_full(narrow, self.config["all_concerns"])
        self.assertTrue(decision.full)
        self.assertTrue(decision.code)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))
        self.assertIn("forced-full-integration", decision.matched_rules)

    def test_rules_command_has_an_exact_package_graph(self) -> None:
        command = self.config["concerns"]["rules"]["command"]
        self.assertNotIn("--workspace", command)
        self.assertNotIn("--all-features", command)
        packages = [
            value
            for index, value in enumerate(command)
            if index > 0 and command[index - 1] == "--package"
        ]
        self.assertEqual(packages, ["hex_core", "hex_lattice", "hex_ai"])

    def test_simulation_command_selects_only_the_dedicated_target(self) -> None:
        command = self.config["concerns"]["simulation"]["command"]
        self.assertIn("--test", command)
        self.assertEqual(command[command.index("--test") + 1], "simulation")

    def test_local_test_order_contains_only_test_concerns(self) -> None:
        self.assertEqual(
            self.config["local_test_order"],
            ["rules", "contracts", "simulation", "app", "residual"],
        )

    def test_repository_relative_paths_are_required(self) -> None:
        with self.assertRaises(gameplay_scope.ScopeConfigurationError):
            self.classify("../outside")

    def test_artifact_output_creates_its_parent_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "nested" / "timing.json"
            gameplay_scope.write_output(output, "{}\n")
            self.assertEqual(output.read_text(encoding="utf-8"), "{}\n")


if __name__ == "__main__":
    unittest.main()
