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
                "contracts",
                "simulation",
                "app",
                "map_unit",
                "map_generation",
                "map_contracts",
                "residual",
            ],
        )

    def test_contracts_keep_the_spell_resolution_composition_seam(self) -> None:
        definition = self.config["concerns"]["contracts"]
        self.assertEqual(
            definition["postflight_command"],
            [
                "cargo",
                "test",
                "--package",
                "hex_game",
                "--test",
                "spell_resolution",
                "--profile",
                "ci",
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

    def test_gameplay_scope_wrapper_preserves_the_old_entry_point(self) -> None:
        wrapper = (ROOT / "tools" / "gameplay_scope.py").read_text(
            encoding="utf-8"
        )
        self.assertIn("from test_scope import main", wrapper)


if __name__ == "__main__":
    unittest.main()
