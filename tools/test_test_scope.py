"""Tests for the fail-closed repository scope selector."""

from __future__ import annotations

import importlib.util
import json
import pathlib
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest import mock


ROOT = pathlib.Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "tools" / "test_scope.py"
SPEC = importlib.util.spec_from_file_location("test_scope", MODULE_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("cannot load test_scope")
test_scope = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = test_scope
SPEC.loader.exec_module(test_scope)


class TestScopeTests(unittest.TestCase):
    """The selector chooses the required concern closure and fails closed."""

    PR_180_CONTEXT = test_scope.ScopeContext(
        event_name="pull_request",
        base_ref="dev",
        head_ref="wave/spell-resolution",
        ref="refs/pull/180/merge",
        pull_request_number=180,
    )
    DEV_PUSH_CONTEXT = test_scope.ScopeContext(
        event_name="push",
        ref="refs/heads/dev",
    )

    @classmethod
    def setUpClass(cls) -> None:
        cls.config = test_scope.load_config()

    def classify(self, *paths: str):
        return test_scope.classify(paths, self.config)

    def assert_cli_rejects_config(self, config: dict, expected: str) -> None:
        """Malformed manifests fail with a concise configuration error."""

        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "test-scopes.json"
            path.write_text(json.dumps(config), encoding="utf-8")
            result = subprocess.run(
                [
                    sys.executable,
                    str(MODULE_PATH),
                    "--config",
                    str(path),
                    "plan",
                    "--path",
                    "README.md",
                ],
                cwd=ROOT,
                check=False,
                capture_output=True,
                text=True,
            )

        self.assertEqual(result.returncode, 2)
        self.assertIn(expected, result.stderr)
        self.assertNotIn("Traceback", result.stderr)

    def fresh_config(self) -> dict:
        """Return an independent mutable copy of the checked-in manifest."""

        return json.loads(
            (ROOT / ".config" / "test-scopes.json").read_text(encoding="utf-8")
        )

    def classify_with_waiver(
        self,
        *paths: str,
        waiver: dict | str | None = None,
        tracked: bool = True,
        remove_after_track: bool = False,
        context: test_scope.ScopeContext | None = PR_180_CONTEXT,
    ):
        """Classify against an isolated tracked or deliberately invalid waiver."""

        manifest_path = self.config["waiver_manifests"][0]
        if waiver is None:
            waiver = json.loads(
                (ROOT / manifest_path).read_text(encoding="utf-8")
            )
        rendered = waiver if isinstance(waiver, str) else json.dumps(waiver)
        with tempfile.TemporaryDirectory() as directory:
            repository = pathlib.Path(directory)
            destination = repository / manifest_path
            destination.parent.mkdir(parents=True)
            destination.write_text(rendered, encoding="utf-8")
            subprocess.run(
                ["git", "init", "--quiet"],
                cwd=repository,
                check=True,
            )
            if tracked:
                subprocess.run(
                    ["git", "add", "--", manifest_path],
                    cwd=repository,
                    check=True,
                )
            if remove_after_track:
                destination.unlink()
            return test_scope.classify(
                paths,
                self.config,
                repository,
                context=context,
            )

    def test_lattice_change_selects_rules_contracts_and_simulation(self) -> None:
        decision = self.classify("crates/hex_lattice/src/cast.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("rules", "contracts", "simulation", "clippy", "docs"),
        )

    def test_sandbox_change_selects_app_and_shipping(self) -> None:
        decision = self.classify("crates/hex_game/src/screens/sandbox.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns, ("app", "clippy", "docs", "shipping")
        )

    def test_gameplay_screen_model_change_selects_app_and_shipping(self) -> None:
        decision = self.classify("crates/hex_gameplay_model/src/sandbox.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns, ("app", "clippy", "docs", "shipping")
        )

    def test_gameplay_visual_change_does_not_select_map_corpora(self) -> None:
        decision = self.classify("walks/gameplay_ui.ron")
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

    def test_ai_change_omits_independent_pure_simulation(self) -> None:
        decision = self.classify("crates/hex_ai/src/lib.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("rules", "contracts", "app", "clippy", "docs", "shipping"),
        )

    def test_units_change_omits_independent_pure_simulation(self) -> None:
        decision = self.classify("crates/hex_units/src/movement.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("contracts", "app", "clippy", "docs", "shipping"),
        )

    def test_trajectory_geometry_omits_application_and_ui_tests(self) -> None:
        for path in (
            "crates/hex_units/src/trajectories.rs",
            "crates/hex_units/src/volumes.rs",
        ):
            with self.subTest(path=path):
                decision = self.classify(path)
                self.assertFalse(decision.full)
                self.assertEqual(
                    decision.concerns,
                    ("trajectory_contracts", "clippy", "docs", "shipping"),
                )

    def test_combat_adapter_change_omits_independent_pure_simulation(self) -> None:
        decision = self.classify("crates/hex_combat/src/authority_host.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("contracts", "app", "clippy", "docs", "shipping"),
        )

    def test_test_support_change_runs_only_its_consuming_partitions(self) -> None:
        decision = self.classify("crates/hex_test_support/src/lib.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("contracts", "app", "map_contracts", "clippy", "docs"),
        )

    def test_neutral_app_harness_fails_closed_to_every_consumer(self) -> None:
        decision = self.classify("crates/hex_test_app/src/lib.rs")

        self.assertTrue(decision.full)
        self.assertEqual(set(decision.concerns), set(self.config["all_concerns"]))
        self.assertIn("shared-test-app", decision.matched_rules)

    def test_animation_change_keeps_its_inline_tests_in_residual(self) -> None:
        decision = self.classify("crates/hex_anim/src/lib.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("contracts", "app", "residual", "clippy", "docs", "shipping"),
        )

    def test_gameplay_asset_rule_precedes_shared_asset_fallback(self) -> None:
        decision = self.classify("crates/hex_assets/src/combat_rules.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            (
                "contracts",
                "app",
                "residual",
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

    def test_combined_terrain_impact_file_remains_fail_closed(self) -> None:
        decision = self.classify("crates/hex_core/src/terrain_impact.rs")
        self.assertTrue(decision.full)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))
        self.assertIn("terrain-impact-contract", decision.matched_rules)

    def test_spell_resolution_foundation_bootstrap_remains_full(
        self,
    ) -> None:
        decision = self.classify(
            "crates/hex_core/src/terrain_impact.rs",
            "crates/hex_map/src/grid.rs",
            "crates/hex_map/tests/contracts/terrain_damage.rs",
        )
        self.assertTrue(decision.full)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))

    def test_map_generation_selects_corpus_and_publication_contracts(self) -> None:
        decision = self.classify("crates/hex_map/src/procedural_v3/ring7.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("map_generation", "map_contracts", "clippy", "docs", "shipping"),
        )

    def test_map_contract_test_change_is_narrow(self) -> None:
        decision = self.classify(
            "crates/hex_map/tests/contracts/publication.rs"
        )
        self.assertFalse(decision.full)
        self.assertEqual(decision.concerns, ("map_contracts", "clippy"))

    def test_map_publication_selects_unit_and_contract_evidence(self) -> None:
        decision = self.classify("crates/hex_map/src/grid.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("map_unit", "map_contracts", "clippy", "docs", "shipping"),
        )

    def test_map_damage_resolver_selects_unit_and_contract_evidence(self) -> None:
        decision = self.classify("crates/hex_map/src/terrain_damage.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("map_unit", "map_contracts", "clippy", "docs", "shipping"),
        )

    def test_map_foundation_selects_every_map_partition(self) -> None:
        decision = self.classify("crates/hex_map/src/voxel.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            (
                "map_unit",
                "map_generation",
                "map_contracts",
                "clippy",
                "docs",
                "shipping",
            ),
        )

    def test_other_world_crate_retains_full_gate(self) -> None:
        decision = self.classify("crates/hex_world/src/camera.rs")
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

    def test_tracked_one_wave_waiver_selects_only_exact_non_ui_evidence(
        self,
    ) -> None:
        manifest = self.config["waiver_manifests"][0]
        decision = self.classify_with_waiver(
            manifest,
            ".config/test-scopes.json",
            "crates/hex_combat/src/spell_resolution.rs",
            "crates/hex_units/src/terrain_reconciliation.rs",
            "crates/hex_game/tests/spell_resolution.rs",
            "docs/development/gameplay-testing.md",
        )

        self.assertFalse(decision.full)
        self.assertEqual(decision.waiver, "spell-resolution-wave")
        self.assertEqual(
            decision.concerns,
            (
                "selector",
                "trajectory_contracts",
                "spell_resolution_contracts",
                "clippy",
                "docs",
                "shipping",
            ),
        )
        for omitted in (
            "app",
            "simulation",
            "map_unit",
            "map_generation",
            "map_contracts",
            "residual",
        ):
            self.assertNotIn(omitted, decision.concerns)

    def test_waiver_is_inactive_when_its_manifest_is_not_in_the_diff(self) -> None:
        decision = self.classify(
            ".config/test-scopes.json",
            "crates/hex_combat/src/spell_resolution.rs",
        )

        self.assertTrue(decision.full)
        self.assertIsNone(decision.waiver)

    def test_waiver_requires_the_exact_pr_number_base_and_head(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        invalid_contexts = (
            None,
            test_scope.ScopeContext(
                event_name="pull_request",
                base_ref="dev",
                head_ref="wave/spell-resolution",
                pull_request_number=181,
            ),
            test_scope.ScopeContext(
                event_name="pull_request",
                base_ref="main",
                head_ref="dev",
                pull_request_number=180,
            ),
            test_scope.ScopeContext(
                event_name="pull_request",
                base_ref="main",
                head_ref="wave/spell-resolution",
                pull_request_number=180,
            ),
        )
        for context in invalid_contexts:
            with self.subTest(context=context):
                decision = self.classify_with_waiver(
                    manifest,
                    "crates/hex_combat/src/spell_resolution.rs",
                    context=context,
                )
                self.assertTrue(decision.full)
                self.assertIsNone(decision.waiver)
                self.assertIn(
                    "fail-closed-waiver-context", decision.matched_rules
                )

    def test_waiver_applies_only_to_the_exact_dev_push(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        dev = self.classify_with_waiver(
            manifest,
            "crates/hex_combat/src/spell_resolution.rs",
            context=self.DEV_PUSH_CONTEXT,
        )
        main = self.classify_with_waiver(
            manifest,
            "crates/hex_combat/src/spell_resolution.rs",
            context=test_scope.ScopeContext(
                event_name="push",
                ref="refs/heads/main",
            ),
        )

        self.assertFalse(dev.full)
        self.assertEqual(dev.waiver, "spell-resolution-wave")
        self.assertTrue(main.full)
        self.assertIsNone(main.waiver)
        self.assertIn("fail-closed-waiver-context", main.matched_rules)

    def test_untracked_or_missing_waiver_manifest_fails_closed(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        for tracked, remove_after_track, marker in (
            (False, False, "fail-closed-untracked-waiver"),
            (True, True, "fail-closed-invalid-waiver"),
        ):
            with self.subTest(marker=marker):
                decision = self.classify_with_waiver(
                    manifest,
                    "crates/hex_combat/src/spell_resolution.rs",
                    tracked=tracked,
                    remove_after_track=remove_after_track,
                )
                self.assertTrue(decision.full)
                self.assertIsNone(decision.waiver)
                self.assertIn(marker, decision.matched_rules)

    def test_malformed_or_unknown_waiver_content_fails_closed(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        unknown = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        unknown["concerns"].append("unknown_concern")
        unhashable_concern = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        unhashable_concern["concerns"] = [{}]
        unhashable_pattern = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        unhashable_pattern["allowed_patterns"] = [{}]
        main_push = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        main_push["applies_to"]["push"]["ref"] = "refs/heads/main"
        main_target = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        main_target["applies_to"]["pull_request"]["base_ref"] = "main"
        for waiver in (
            "{",
            unknown,
            unhashable_concern,
            unhashable_pattern,
            main_push,
            main_target,
        ):
            with self.subTest(waiver=waiver):
                decision = self.classify_with_waiver(
                    manifest,
                    "crates/hex_combat/src/spell_resolution.rs",
                    waiver=waiver,
                )
                self.assertTrue(decision.full)
                self.assertIsNone(decision.waiver)
                self.assertIn(
                    "fail-closed-invalid-waiver", decision.matched_rules
                )

    def test_waiver_cannot_admit_out_of_allowlist_or_unknown_paths(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        outside = self.classify_with_waiver(
            manifest,
            "crates/hex_combat/src/spell_resolution.rs",
            "crates/hex_ui/src/lib.rs",
        )
        self.assertTrue(outside.full)
        self.assertIn(
            "fail-closed-waiver-outside-allowlist", outside.matched_rules
        )

        waiver = json.loads(
            (ROOT / manifest).read_text(encoding="utf-8")
        )
        waiver["allowed_patterns"].append("unexpected/**")
        unknown = self.classify_with_waiver(
            manifest,
            "crates/hex_combat/src/spell_resolution.rs",
            "unexpected/new-system.rs",
            waiver=waiver,
        )
        self.assertTrue(unknown.full)
        self.assertIn("unexpected/new-system.rs", unknown.unknown_files)
        self.assertIn(
            "fail-closed-waiver-unknown-path", unknown.matched_rules
        )

    def test_waiver_rejects_unrelated_files_in_its_changed_crates(self) -> None:
        manifest = self.config["waiver_manifests"][0]
        for path in (
            "crates/hex_combat/src/commands/channel.rs",
            "crates/hex_units/src/animation.rs",
        ):
            with self.subTest(path=path):
                decision = self.classify_with_waiver(
                    manifest,
                    "crates/hex_combat/src/spell_resolution.rs",
                    path,
                )
                self.assertTrue(decision.full)
                self.assertIn(
                    "fail-closed-waiver-outside-allowlist",
                    decision.matched_rules,
                )

        waiver = json.loads((ROOT / manifest).read_text(encoding="utf-8"))
        self.assertNotIn("crates/hex_combat/**", waiver["allowed_patterns"])
        self.assertNotIn("crates/hex_units/**", waiver["allowed_patterns"])
        self.assertIn(
            "crates/hex_game/tests/spell_resolution.rs",
            waiver["allowed_patterns"],
        )

    def test_unknown_waiver_manifest_path_fails_closed(self) -> None:
        decision = self.classify(
            ".config/test-waivers/not-approved.json",
            "crates/hex_combat/src/spell_resolution.rs",
        )
        self.assertTrue(decision.full)
        self.assertIn("fail-closed-unknown-waiver", decision.matched_rules)

    def test_empty_command_arrays_are_rejected_without_a_traceback(self) -> None:
        cases = (
            (
                lambda config: config["concerns"]["map_unit"].update(command=[]),
                "concern map_unit command must be non-empty strings",
            ),
            (
                lambda config: config["partition_checks"]["map"].update(
                    full_command=[]
                ),
                "partition check map has invalid full_command",
            ),
            (
                lambda config: config["partition_checks"]["map"].update(
                    all_tests_command=[]
                ),
                "partition check map has invalid all_tests_command",
            ),
            (
                lambda config: config["concerns"]["app"].update(
                    postflight_command=[]
                ),
                "concern app postflight_command must be non-empty strings",
            ),
            (
                lambda config: config["selection_checks"][
                    "spell_resolution_contracts"
                ].update(command=[]),
                "selection check spell_resolution_contracts command needs "
                "unique test identities",
            ),
        )
        for mutate, expected in cases:
            with self.subTest(expected=expected):
                config = self.fresh_config()
                mutate(config)
                self.assert_cli_rejects_config(config, expected)

    def test_non_boolean_documentation_flag_is_rejected_without_a_traceback(
        self,
    ) -> None:
        config = self.fresh_config()
        config["rules"][-1]["documentation_only"] = "false"
        self.assert_cli_rejects_config(
            config,
            "documentation_only must be a boolean",
        )

    def test_invalid_waiver_manifest_config_is_rejected_without_a_traceback(
        self,
    ) -> None:
        config = self.fresh_config()
        config["waiver_manifests"] = [{}]
        self.assert_cli_rejects_config(
            config,
            "waiver_manifests must be unique safe JSON paths",
        )

    def test_mixed_diff_unions_concerns(self) -> None:
        decision = self.classify(
            "crates/hex_lattice/src/cast.rs",
            "crates/hex_game/src/screens/sandbox.rs",
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

    def test_mixed_gameplay_and_map_diff_unions_owner_closures(self) -> None:
        decision = self.classify(
            "crates/hex_lattice/src/cast.rs",
            "crates/hex_map/src/grid.rs",
        )
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            (
                "rules",
                "contracts",
                "simulation",
                "map_unit",
                "map_contracts",
                "clippy",
                "docs",
                "shipping",
            ),
        )

    def test_command_manifest_changes_fail_closed(self) -> None:
        decision = self.classify(
            ".config/test-scopes.json",
            "tools/test_test_scope.py",
        )
        self.assertTrue(decision.full)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))

    def test_selector_regression_change_selects_only_its_own_target(self) -> None:
        decision = self.classify("tools/test_test_scope.py")
        self.assertFalse(decision.full)
        self.assertEqual(decision.concerns, ("selector",))

    def test_scope_engine_change_runs_everything(self) -> None:
        decision = self.classify("tools/test_scope.py")
        self.assertTrue(decision.full)

    def test_deprecated_ui_checker_is_explicit_scope_infrastructure(self) -> None:
        decision = self.classify("tools/check_deprecated_ui_terms.py")
        self.assertTrue(decision.full)
        self.assertEqual(decision.unknown_files, ())
        self.assertIn("scope-infrastructure", decision.matched_rules)

    def test_push_gate_promotes_a_narrow_decision_to_full(self) -> None:
        narrow = self.classify("crates/hex_lattice/src/cast.rs")
        decision = test_scope.force_full(narrow, self.config["all_concerns"])
        self.assertTrue(decision.full)
        self.assertTrue(decision.code)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))
        self.assertIn("forced-full-integration", decision.matched_rules)

    def test_exact_one_wave_waiver_remains_narrow_on_its_integration_push(
        self,
    ) -> None:
        manifest = self.config["waiver_manifests"][0]
        waived = self.classify_with_waiver(
            manifest,
            "crates/hex_combat/src/spell_resolution.rs",
            context=self.DEV_PUSH_CONTEXT,
        )
        decision = test_scope.force_full(
            waived, self.config["all_concerns"]
        )

        self.assertFalse(decision.full)
        self.assertEqual(decision.concerns, waived.concerns)
        self.assertEqual(decision.waiver, "spell-resolution-wave")

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

    def test_simulation_command_selects_inline_rules_and_dedicated_target(self) -> None:
        command = self.config["concerns"]["simulation"]["command"]
        self.assertIn("--lib", command)
        self.assertIn("--test", command)
        self.assertEqual(command[command.index("--test") + 1], "simulation")

    def test_app_command_selects_model_lib_and_headless_target(self) -> None:
        command = self.config["concerns"]["app"]["command"]
        packages = [
            value
            for index, value in enumerate(command)
            if index > 0 and command[index - 1] == "--package"
        ]
        self.assertEqual(packages, ["hex_gameplay_model", "hex_ui", "hex_game"])
        self.assertIn("--lib", command)
        self.assertIn("--test", command)
        self.assertEqual(command[command.index("--test") + 1], "gameplay_app")
        features = command[command.index("--features") + 1].split(",")
        self.assertEqual(features, ["hex_game/test-support"])
        self.assertNotIn("hex_ui/dev-tools", command)

    def test_app_dev_tools_are_exercised_only_by_a_focused_followup(self) -> None:
        command = self.config["concerns"]["app"]["postflight_command"]
        self.assertEqual(command[command.index("--package") + 1], "hex_ui")
        self.assertIn("--lib", command)
        features = command[command.index("--features") + 1].split(",")
        self.assertEqual(set(features), {"test-support", "dev-tools"})
        self.assertEqual(command[-1], "dev_time::tests::")

    def test_app_preflight_compiles_default_feature_library_tests(self) -> None:
        command = self.config["concerns"]["app"]["preflight_command"]
        self.assertEqual(
            command,
            [
                "cargo",
                "test",
                "--package",
                "hex_game",
                "--lib",
                "--no-run",
                "--profile",
                "ci",
            ],
        )

    def test_game_test_support_forwards_ui_test_support(self) -> None:
        manifest = tomllib.loads(
            (ROOT / "crates" / "hex_game" / "Cargo.toml").read_text(
                encoding="utf-8"
            )
        )
        self.assertIn(
            "hex_ui/test-support",
            manifest["features"]["test-support"],
        )

    def test_map_commands_select_package_and_target_before_filters(self) -> None:
        unit = self.config["concerns"]["map_unit"]["command"]
        generation = self.config["concerns"]["map_generation"]["command"]
        contracts = self.config["concerns"]["map_contracts"]["command"]
        for command in (unit, generation, contracts):
            self.assertNotIn("--workspace", command)
            self.assertEqual(command[command.index("--package") + 1], "hex_map")
        self.assertIn("--lib", unit)
        self.assertIn("--lib", generation)
        self.assertEqual(contracts[contracts.index("--test") + 1], "contracts")

    def test_map_partition_contract_freezes_current_evidence(self) -> None:
        partition = self.config["partition_checks"]["map"]
        self.assertEqual(
            partition["expected_counts"],
            {"map_unit": 94, "map_generation": 384, "map_contracts": 75},
        )
        self.assertEqual(partition["expected_ignored"], 25)

    def test_map_commands_share_the_optimized_test_profile(self) -> None:
        for concern in ("map_unit", "map_generation", "map_contracts"):
            command = self.config["concerns"][concern]["command"]
            self.assertEqual(
                command[command.index("--cargo-profile") + 1], "map-test"
            )

    def test_hex_ui_manifest_enforces_the_presentation_dependency_ceiling(self) -> None:
        manifest = tomllib.loads(
            (ROOT / "crates" / "hex_ui" / "Cargo.toml").read_text(encoding="utf-8")
        )
        allowed = {"bevy", "hex_assets", "hex_core", "hex_gameplay_model", "serde"}
        self.assertEqual(set(manifest["dependencies"]), allowed)
        forbidden = {
            "hex_game",
            "hex_combat",
            "hex_units",
            "hex_lattice",
            "hex_map",
            "hex_world",
            "hex_perception",
        }
        self.assertTrue(forbidden.isdisjoint(manifest["dependencies"]))

    def test_residual_excludes_every_owned_gameplay_partition(self) -> None:
        command = self.config["concerns"]["residual"]["command"]
        expression = command[command.index("-E") + 1]
        for package in (
            "hex_core",
            "hex_lattice",
            "hex_ai",
            "hex_units",
            "hex_combat_core",
            "hex_combat",
            "hex_test_support",
            "hex_gameplay_model",
            "hex_ui",
            "hex_map",
        ):
            self.assertIn(f"package({package})", expression)
        self.assertIn("package(hex_game) & binary(gameplay_app)", expression)

    def test_local_test_order_contains_only_test_concerns(self) -> None:
        self.assertEqual(
            self.config["local_test_order"],
            [
                "selector",
                "rules",
                "trajectory_contracts",
                "spell_resolution_contracts",
                "contracts",
                "simulation",
                "app",
                "map_unit",
                "map_generation",
                "map_contracts",
                "residual",
            ],
        )

    def test_trajectory_command_is_one_junit_producing_non_ui_wedge(self) -> None:
        definition = self.config["concerns"]["trajectory_contracts"]
        command = definition["command"]

        self.assertEqual(command[:3], ["cargo", "nextest", "run"])
        packages = [
            command[index + 1]
            for index, value in enumerate(command)
            if value == "--package"
        ]
        self.assertEqual(packages, ["hex_units", "hex_combat", "hex_game"])
        self.assertIn("--lib", command)
        self.assertEqual(command[command.index("--test") + 1], "contracts")
        self.assertEqual(
            command[command.index("--profile") + 1], "gameplay-trajectory"
        )
        self.assertNotIn("preflight_command", definition)
        self.assertNotIn("postflight_command", definition)
        self.assertNotIn("hex_ui", command)
        self.assertNotIn("gameplay_app", command)

        nextest = (ROOT / ".config" / "nextest.toml").read_text(encoding="utf-8")
        profile = nextest.split("[profile.gameplay-trajectory]", maxsplit=1)[1].split(
            "[profile.gameplay-trajectory.junit]", maxsplit=1
        )[0]
        for direct_consumer in (
            "a_two_level_column_starts_wholly_above_the_selected_floor",
            "a_selected_lower_stack_never_jumps_to_the_highest_run",
            "ai::tests::ai_legality_uses_only_faction_authorized_material",
            "an_observed_anchor_allows_area_spillover_into_unknown_space",
            "a_blocked_direct_trajectory_refuses_before_payment",
            "an_authored_arc_clears_the_same_wall",
            "a_confirmed_cast_names_its_anchor_and_only_the_facing_it_needs",
        ):
            self.assertIn(direct_consumer, profile)
        for unrelated_consumer in (
            "existing_material_blocks_the_complete_creation_volume",
            "a_body_and_its_support_are_both_protected",
            "settled_construction_replaces_stale_movement_authority",
            "a_facing_points_at_what_was_aimed_at",
        ):
            self.assertNotIn(unrelated_consumer, profile)
        self.assertIn('path = "junit.xml"', nextest.split(
            "[profile.gameplay-trajectory.junit]", maxsplit=1
        )[1])

    def test_spell_resolution_concern_uses_exact_non_ui_targets(self) -> None:
        definition = self.config["concerns"]["spell_resolution_contracts"]
        domain = definition["preflight_command"]
        map_seams = definition["command"]
        composition = definition["postflight_command"]

        for command in (domain, map_seams, composition):
            self.assertEqual(command[:3], ["cargo", "nextest", "run"])
        packages = [
            domain[index + 1]
            for index, value in enumerate(domain)
            if value == "--package"
        ]
        self.assertEqual(
            packages,
            [
                "hex_core",
                "hex_assets",
                "hex_units",
                "hex_combat_core",
                "hex_combat",
            ],
        )
        self.assertIn("--lib", domain)
        self.assertEqual(domain[domain.index("--test") + 1], "contracts")
        self.assertEqual(
            domain[domain.index("--profile") + 1],
            "gameplay-spell-resolution",
        )
        self.assertEqual(
            map_seams[map_seams.index("--package") + 1], "hex_map"
        )
        self.assertNotIn("--lib", map_seams)
        self.assertEqual(
            map_seams[map_seams.index("--test") + 1], "contracts"
        )
        self.assertEqual(
            map_seams[map_seams.index("--profile") + 1],
            "gameplay-spell-resolution-map",
        )
        self.assertEqual(
            composition[composition.index("--package") + 1], "hex_game"
        )
        self.assertIn("--lib", composition)
        self.assertEqual(
            composition[composition.index("--test") + 1],
            "spell_resolution",
        )
        self.assertEqual(
            composition[composition.index("--profile") + 1],
            "gameplay-spell-resolution-composition",
        )
        rendered = " ".join((*domain, *map_seams, *composition))
        for forbidden in (
            "--workspace",
            "--all-features",
            "hex_ui",
            "gameplay_app",
            "simulation",
        ):
            self.assertNotIn(forbidden, rendered)

        nextest = (ROOT / ".config" / "nextest.toml").read_text(
            encoding="utf-8"
        )
        profile = nextest.split(
            "[profile.gameplay-spell-resolution]", maxsplit=1
        )[1].split("[profile.gameplay-spell-resolution.junit]", maxsplit=1)[0]
        for required in (
            "terrain_impact::tests::",
            "impact_round_trips_reflects_and_has_complete_fingerprint_fields",
            "terrain_occupancy::tests::",
            "terrain_reconciliation::",
            "external_resolution_holds_turn_and_outcome_between_area_answers",
            "spell_resolution::tests::",
            "a_damage_cast_on_a_downed_unit_is_refused_before_payment",
        ):
            self.assertIn(required, profile)
        for forbidden in (
            "package(hex_ui)",
            "binary(gameplay_app)",
            "binary(simulation)",
            "procedural",
            "commands::spell_resolution::tests::",
            "test(/^loop::",
        ):
            self.assertNotIn(forbidden, profile)
        self.assertIn(
            "loop_contract::adapter_spending_the_turn_advances_both_projections",
            profile,
        )
        self.assertIn(
            'path = "junit.xml"',
            nextest.split(
                "[profile.gameplay-spell-resolution.junit]", maxsplit=1
            )[1],
        )
        map_profile = nextest.split(
            "[profile.gameplay-spell-resolution-map]", maxsplit=1
        )[1].split(
            "[profile.gameplay-spell-resolution-map.junit]", maxsplit=1
        )[0]
        self.assertIn(
            "terrain_protocol_orders_reserved_phases_before_perception",
            map_profile,
        )
        self.assertIn(
            "overkill_is_capped_and_empty_voxels_report_no_material",
            map_profile,
        )
        self.assertNotIn("procedural", map_profile)
        composition_profile = nextest.split(
            "[profile.gameplay-spell-resolution-composition]", maxsplit=1
        )[1].split(
            "[profile.gameplay-spell-resolution-composition.junit]",
            maxsplit=1,
        )[0]
        for required in (
            "package(hex_game)",
            "binary(hex_game)",
            "binary(spell_resolution)",
            "binary(game_content_contracts)",
            "shipped_fireball_is_available_to_creator_characters",
            "shipped_fireball_is_admitted_and_castable_from_a_full_fire_ring",
        ):
            self.assertIn(required, composition_profile)
        self.assertNotIn("kind(test)", composition_profile)

        expected = self.config["selection_checks"][
            "spell_resolution_contracts"
        ]
        self.assertEqual(len(expected["preflight_command"]), 56)
        self.assertEqual(len(expected["command"]), 2)
        self.assertEqual(len(expected["postflight_command"]), 7)
        self.assertIn(
            "hex_combat::contracts loop_contract::"
            "adapter_spending_the_turn_advances_both_projections_in_the_same_frame",
            expected["preflight_command"],
        )

    def test_spell_resolution_list_check_is_exact_and_fails_on_drift(
        self,
    ) -> None:
        expected_commands = self.config["selection_checks"][
            "spell_resolution_contracts"
        ]
        listed = [set(tests) for tests in expected_commands.values()]
        with mock.patch.object(
            test_scope,
            "_listed_tests",
            side_effect=listed,
        ) as list_tests:
            test_scope.check_selection(
                "spell_resolution_contracts", self.config
            )
        self.assertEqual(list_tests.call_count, 3)
        for call in list_tests.call_args_list:
            self.assertEqual(call.args[0][:3], ["cargo", "nextest", "list"])

        missing = [set(tests) for tests in expected_commands.values()]
        missing[0].remove(next(iter(missing[0])))
        with mock.patch.object(
            test_scope,
            "_listed_tests",
            side_effect=missing,
        ), self.assertRaisesRegex(
            test_scope.ScopeConfigurationError,
            "differs from its 56 reviewed tests",
        ):
            test_scope.check_selection(
                "spell_resolution_contracts", self.config
            )

    def test_nextest_listing_disables_color_for_stable_identities(self) -> None:
        completed = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="hex_core::contracts exact_identity: test\n",
            stderr="",
        )
        with mock.patch.object(
            test_scope.subprocess,
            "run",
            return_value=completed,
        ) as run:
            listed = test_scope._listed_tests(
                ["cargo", "nextest", "list", "--profile", "ci"]
            )

        self.assertEqual(
            listed, {"hex_core::contracts exact_identity: test"}
        )
        self.assertEqual(
            run.call_args.args[0],
            [
                "cargo",
                "nextest",
                "list",
                "--color",
                "never",
                "--profile",
                "ci",
            ],
        )

    def test_local_skill_checks_selection_and_splits_portably(self) -> None:
        skill = (ROOT / ".claude" / "skills" / "test-local" / "SKILL.md").read_text(
            encoding="utf-8"
        )
        self.assertIn("--head HEAD) || exit $?", skill)
        self.assertIn("REMAINING=$SELECTED", skill)
        self.assertIn("concern=${REMAINING%% *}", skill)
        self.assertNotIn("for concern in $SELECTED", skill)

    def test_repository_relative_paths_are_required(self) -> None:
        with self.assertRaises(test_scope.ScopeConfigurationError):
            self.classify("../outside")

    def test_artifact_output_creates_its_parent_directory(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "nested" / "timing.json"
            test_scope.write_output(output, "{}\n")
            self.assertEqual(output.read_text(encoding="utf-8"), "{}\n")

    def test_manual_runtime_gate_tracks_the_screen_model_crate(self) -> None:
        workflow = (
            ROOT / ".github" / "workflows" / "manual-runtime-signoff.yaml"
        ).read_text(encoding="utf-8")
        self.assertIn("hex_gameplay_model", workflow)

    def test_manual_runtime_gate_exempts_pure_trajectory_contract_paths(self) -> None:
        workflow = (
            ROOT / ".github" / "workflows" / "manual-runtime-signoff.yaml"
        ).read_text(encoding="utf-8")
        self.assertNotIn("crates\\/hex_core\\/src\\/terrain_impact\\.rs", workflow)
        self.assertIn(
            "crates\\/hex_units\\/src\\/(trajectories|volumes)\\.rs",
            workflow,
        )
        self.assertIn('if ! changed="$(git diff --name-only', workflow)
        self.assertIn('result="$(field_value "Manual runtime result")"', workflow)
        self.assertIn('require_named_field "Manual runtime findings/waiver"', workflow)
        self.assertIn('reviewer" != "$WAIVER_ACTOR', workflow)
        self.assertIn('collaborators/$reviewer/permission', workflow)
        self.assertIn('admin|maintain)', workflow)

    def test_shipping_job_primes_cold_windows_dependencies(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yaml").read_text(
            encoding="utf-8"
        )
        shipping_jobs = workflow.split("\n  build:\n", maxsplit=1)[1].split(
            "\n  coverage:\n", maxsplit=1
        )[0]
        self.assertIn("\n  windows_dependencies:\n", shipping_jobs)
        self.assertIn("shared-key: windows-shipping", shipping_jobs)
        self.assertIn("save-if: false", shipping_jobs)
        self.assertIn(
            "if: needs.changes.outputs.shipping == 'true'", shipping_jobs
        )

    def test_map_ci_uses_canonical_partitions_not_workspace_filtering(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yaml").read_text(
            encoding="utf-8"
        )
        map_job = workflow.split("\n  map_tests:\n", maxsplit=1)[1].split(
            "\n  docs:\n", maxsplit=1
        )[0]
        self.assertIn("name: Map partitions", map_job)
        self.assertIn("tools/test_scope.py run map_unit", map_job)
        self.assertIn("tools/test_scope.py run map_generation", map_job)
        self.assertIn("tools/test_scope.py run map_contracts", map_job)
        self.assertIn("tools/test_scope.py check-partitions map", map_job)
        self.assertNotIn("--workspace", map_job)
        self.assertNotIn("outputs.residual", map_job)

    def test_gameplay_ci_runs_the_canonical_trajectory_concern(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yaml").read_text(
            encoding="utf-8"
        )
        gameplay_job = workflow.split("\n  gameplay:\n", maxsplit=1)[1].split(
            "\n  build:\n", maxsplit=1
        )[0]
        self.assertIn(
            "trajectory_contracts: ${{ steps.filter.outputs.trajectory_contracts }}",
            workflow,
        )
        self.assertIn("tools/test_scope.py run trajectory_contracts", gameplay_job)
        self.assertIn(
            "target/nextest/gameplay-trajectory/junit.xml", gameplay_job
        )
        self.assertNotIn("tools/test_scope.py run app", gameplay_job.split(
            "- name: Trajectory and volume contracts", maxsplit=1
        )[1].split("- name: Gameplay simulation", maxsplit=1)[0])

    def test_ci_publishes_and_runs_the_spell_resolution_waiver_wedge(
        self,
    ) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yaml").read_text(
            encoding="utf-8"
        )
        gameplay_job = workflow.split("\n  gameplay:\n", maxsplit=1)[1].split(
            "\n  build:\n", maxsplit=1
        )[0]
        self.assertIn(
            "waiver: ${{ steps.filter.outputs.waiver }}", workflow
        )
        for context_argument in (
            '--event-name "${{ github.event_name }}"',
            '--base-ref "${{ github.base_ref }}"',
            '--head-ref "${{ github.head_ref }}"',
            '--ref "${{ github.ref }}"',
            '--pull-request-number "${{ github.event.pull_request.number }}"',
        ):
            self.assertIn(context_argument, workflow)
        self.assertIn(
            "spell_resolution_contracts: "
            "${{ steps.filter.outputs.spell_resolution_contracts }}",
            workflow,
        )
        self.assertIn(
            "tools/test_scope.py run spell_resolution_contracts", gameplay_job
        )
        self.assertIn(
            "target/nextest/gameplay-spell-resolution/junit.xml",
            gameplay_job,
        )
        self.assertIn(
            "target/nextest/gameplay-spell-resolution-map/junit.xml",
            gameplay_job,
        )
        self.assertIn(
            "target/nextest/gameplay-spell-resolution-composition/junit.xml",
            gameplay_job,
        )
        spell_step = gameplay_job.split(
            "- name: Spell resolution contracts", maxsplit=1
        )[1].split("- name: Gameplay simulation", maxsplit=1)[0]
        for forbidden in (
            "tools/test_scope.py run app",
            "tools/test_scope.py run simulation",
            "tools/test_scope.py run map_",
            "tools/test_scope.py run residual",
        ):
            self.assertNotIn(forbidden, spell_step)

    def test_gameplay_scope_wrapper_preserves_the_old_entry_point(self) -> None:
        wrapper = (ROOT / "tools" / "gameplay_scope.py").read_text(
            encoding="utf-8"
        )
        self.assertIn("from test_scope import main", wrapper)


if __name__ == "__main__":
    unittest.main()
