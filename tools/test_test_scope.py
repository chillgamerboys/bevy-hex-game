"""Tests for the fail-closed repository scope selector."""

from __future__ import annotations

import fnmatch
import importlib.util
import json
import pathlib
import re
import subprocess
import sys
import tempfile
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


def toml_table(source: str, name: str) -> str:
    """Return one top-level TOML table without requiring Python 3.11 tomllib."""

    marker = f"[{name}]"
    if marker not in source:
        raise AssertionError(f"missing TOML table {marker}")
    body = source.split(marker, maxsplit=1)[1]
    return body.split("\n[", maxsplit=1)[0]


def toml_table_keys(source: str, name: str) -> set[str]:
    """Return assignment keys in one simple top-level TOML table."""

    return set(re.findall(r"(?m)^([A-Za-z0-9_-]+)\s*=", toml_table(source, name)))


def toml_string_array(source: str, table: str, key: str) -> list[str]:
    """Read one string array used by the focused manifest contract tests."""

    match = re.search(
        rf"(?ms)^{re.escape(key)}\s*=\s*\[(.*?)\]",
        toml_table(source, table),
    )
    if match is None:
        raise AssertionError(f"missing TOML array {table}.{key}")
    return re.findall(r'"([^"]+)"', match.group(1))


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
        for path in ("walks/gameplay_ui.ron", "walks/multiplayer_session.ron"):
            with self.subTest(path=path):
                decision = self.classify(path)
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

    def test_schematic_planner_stays_in_its_pure_tooling_partition(self) -> None:
        for path in (
            "crates/hex_schematic/src/generator.rs",
            "assets/config/schematics/grand-v3-template.ron",
        ):
            with self.subTest(path=path):
                decision = self.classify(path)
                self.assertFalse(decision.full)
                self.assertEqual(
                    decision.concerns,
                    ("residual", "clippy", "docs"),
                )
                self.assertEqual(decision.unknown_files, ())
                self.assertIn("schematic-planner", decision.matched_rules)

    def test_map_contract_test_change_is_narrow(self) -> None:
        decision = self.classify(
            "crates/hex_map/tests/contracts/publication.rs"
        )
        self.assertFalse(decision.full)
        self.assertEqual(decision.concerns, ("map_contracts", "clippy"))

    def test_schematic_map_compiler_contract_selects_generation(self) -> None:
        decision = self.classify("crates/hex_map/tests/schematic_compile.rs")
        self.assertFalse(decision.full)
        self.assertEqual(decision.concerns, ("map_generation", "clippy"))
        self.assertIn("schematic-map-compiler-contract", decision.matched_rules)

    def test_map_publication_selects_unit_and_contract_evidence(self) -> None:
        decision = self.classify("crates/hex_map/src/grid.rs")
        self.assertFalse(decision.full)
        self.assertEqual(
            decision.concerns,
            ("map_unit", "map_contracts", "clippy", "docs", "shipping"),
        )

    def test_map_world_snapshot_fails_closed_across_every_consumer(self) -> None:
        decision = self.classify("crates/hex_map/src/world_snapshot.rs")
        self.assertTrue(decision.full)
        self.assertEqual(decision.concerns, tuple(self.config["all_concerns"]))
        self.assertEqual(decision.unknown_files, ())
        self.assertIn("map-world-snapshot", decision.matched_rules)

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
                lambda config: config["partition_checks"]["map"].update(
                    required_ignored_patterns=[]
                ),
                "partition check map has invalid concerns or ignored patterns",
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

    def test_review_tool_changes_select_their_python_contracts(self) -> None:
        for path in (
            "tools/review.py",
            "tools/test_review.py",
            "tools/run_grand_v3_structural_review.sh",
        ):
            with self.subTest(path=path):
                decision = self.classify(path)
                self.assertFalse(decision.full)
                self.assertEqual(decision.concerns, ("selector",))

        self.assertEqual(
            self.config["concerns"]["selector"]["command"],
            [
                "python3",
                "-m",
                "unittest",
                "tools/test_test_scope.py",
                "tools/test_review.py",
            ],
        )

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
        manifest = (ROOT / "crates" / "hex_game" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "hex_ui/test-support",
            toml_string_array(manifest, "features", "test-support"),
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
        self.assertEqual(
            generation[generation.index("--test") + 1], "schematic_compile"
        )
        self.assertEqual(contracts[contracts.index("--test") + 1], "contracts")

    def test_map_partition_contract_uses_identities_not_frozen_counts(self) -> None:
        partition = self.config["partition_checks"]["map"]
        self.assertNotIn("expected_counts", partition)
        self.assertNotIn("expected_ignored", partition)
        patterns = partition["required_ignored_patterns"]
        self.assertEqual(len(patterns), len(set(patterns)))
        self.assertIn(
            "*procedural_v3::crystal_ascent::tests::"
            "crystal_ascent_boundary_rise_benchmark_tracks_timing_and_plan_counts",
            patterns,
        )
        self.assertIn(
            "*grand_v3_full_world_release_corpus_compiles_32_seeds",
            patterns,
        )
        self.assertEqual(
            partition["all_tests_command"][:3],
            ["cargo", "nextest", "list"],
        )
        self.assertEqual(
            partition["all_tests_command"][-2:],
            ["--run-ignored", "all"],
        )
        for command_name in ("full_command", "all_tests_command"):
            self.assertIn(
                "--ignore-default-filter",
                partition[command_name],
            )

    def test_required_ignored_patterns_name_every_retained_map_gate(self) -> None:
        candidates: list[str] = []
        crate = ROOT / "crates" / "hex_map"
        paths = (
            *crate.joinpath("src").rglob("*.rs"),
            *crate.joinpath("tests").rglob("*.rs"),
        )
        for path in paths:
            source = path.read_text(encoding="utf-8")
            ignored_functions = re.findall(
                r'#\[ignore(?:\s*=\s*"[^"]*")?\]\s*'
                r"(?:#\[[^\]]+\]\s*)*fn\s+([A-Za-z0-9_]+)",
                source,
            )
            relative = path.relative_to(crate)
            for function in ignored_functions:
                if relative.parts[0] == "src":
                    module = "::".join(relative.with_suffix("").parts[1:])
                    identity = f"ignored {module}::tests::{function}"
                elif relative.as_posix() == "tests/schematic_compile.rs":
                    identity = f"ignored {function}"
                elif relative.parts[:2] == ("tests", "contracts"):
                    identity = f"ignored {relative.stem}::{function}"
                else:
                    identity = f"ignored {function}"
                candidates.append(identity)

        patterns = self.config["partition_checks"]["map"][
            "required_ignored_patterns"
        ]
        for pattern in patterns:
            with self.subTest(pattern=pattern):
                matches = [
                    identity
                    for identity in candidates
                    if fnmatch.fnmatchcase(identity, pattern)
                ]
                self.assertEqual(len(matches), 1, matches)
        for identity in candidates:
            with self.subTest(identity=identity):
                self.assertTrue(
                    any(
                        fnmatch.fnmatchcase(identity, pattern)
                        for pattern in patterns
                    )
                )

    def test_partition_completeness_accepts_new_tests_without_count_edits(self) -> None:
        config = self.fresh_config()
        config["partition_checks"]["map"]["required_ignored_patterns"] = [
            "*required_stress"
        ]
        full = {"map unit_a", "map generation_a", "map generation_new", "map contract_a"}
        listings = (
            full,
            {"map unit_a"},
            {"map generation_a", "map generation_new"},
            {"map contract_a"},
            full | {"map required_stress", "map optional_new_stress"},
        )
        with mock.patch.object(test_scope, "_listed_tests", side_effect=listings):
            evidence = test_scope.check_partitions("map", config)

        self.assertEqual(evidence["full_count"], 4)
        self.assertEqual(
            evidence["partition_counts"],
            {"map_unit": 1, "map_generation": 2, "map_contracts": 1},
        )
        self.assertEqual(evidence["ignored_count"], 2)

    def test_partition_completeness_rejects_overlap_and_union_gaps(self) -> None:
        config = self.fresh_config()
        config["partition_checks"]["map"]["required_ignored_patterns"] = [
            "*required_stress"
        ]
        cases = (
            (
                (
                    {"unit", "generation"},
                    {"unit"},
                    {"unit"},
                    {"generation"},
                ),
                "overlaps existing tests",
            ),
            (
                (
                    {"unit", "generation", "contract"},
                    {"unit"},
                    {"generation"},
                    set(),
                ),
                "partition union differs from full test set",
            ),
        )
        for listings, expected in cases:
            with self.subTest(expected=expected), mock.patch.object(
                test_scope, "_listed_tests", side_effect=listings
            ):
                with self.assertRaisesRegex(
                    test_scope.ScopeConfigurationError, expected
                ):
                    test_scope.check_partitions("map", config)

    def test_partition_completeness_requires_named_ignored_stress_tests(self) -> None:
        config = self.fresh_config()
        config["partition_checks"]["map"]["required_ignored_patterns"] = [
            "*required_stress"
        ]
        full = {"unit", "generation", "contract"}
        listings = (
            full,
            {"unit"},
            {"generation"},
            {"contract"},
            full | {"renamed stress"},
        )
        with mock.patch.object(test_scope, "_listed_tests", side_effect=listings):
            with self.assertRaisesRegex(
                test_scope.ScopeConfigurationError,
                "required ignored test pattern must match exactly one",
            ):
                test_scope.check_partitions("map", config)

    def test_partition_listing_keeps_cargo_stderr_live(self) -> None:
        result = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="hex_map::hex_map one_test\n",
            stderr=None,
        )
        command = ["cargo", "nextest", "list", "--package", "hex_map"]
        with mock.patch.object(
            test_scope.subprocess, "run", return_value=result
        ) as run_mock:
            identities = test_scope._listed_tests(command)

        self.assertEqual(identities, {"hex_map::hex_map one_test"})
        call = run_mock.call_args
        self.assertIs(call.kwargs["stdout"], subprocess.PIPE)
        self.assertNotIn("capture_output", call.kwargs)
        self.assertNotIn("stderr", call.kwargs)

    def test_run_records_exact_selection_provenance(self) -> None:
        definition = {
            "command": [
                "cargo",
                "nextest",
                "run",
                "--package",
                "hex_map",
                "--profile",
                "map-unit",
            ]
        }
        selection = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="hex_map::hex_map test_one\nhex_map::hex_map test_two\n",
            stderr="",
        )
        execution = subprocess.CompletedProcess(args=[], returncode=0)
        with tempfile.TemporaryDirectory() as directory:
            timing_path = pathlib.Path(directory) / "timing.json"
            with mock.patch.object(
                test_scope.subprocess,
                "run",
                side_effect=(selection, execution),
            ) as run_mock, mock.patch.object(
                test_scope.time,
                "monotonic",
                side_effect=(0.0, 1.0, 2.0, 5.0, 6.0, 10.0, 12.0, 15.0),
            ):
                returncode = test_scope.run_concern("map_unit", definition, timing_path)
            timing = json.loads(timing_path.read_text(encoding="utf-8"))

        self.assertEqual(returncode, 0)
        self.assertEqual(run_mock.call_count, 2)
        selection_call = run_mock.call_args_list[0]
        self.assertIs(selection_call.kwargs["stdout"], subprocess.PIPE)
        self.assertNotIn("capture_output", selection_call.kwargs)
        self.assertNotIn("stderr", selection_call.kwargs)
        evidence = timing["commands"][0]
        self.assertEqual(evidence["command"], definition["command"])
        self.assertEqual(evidence["selected_count"], 2)
        self.assertEqual(evidence["selection_elapsed_seconds"], 3.0)
        self.assertEqual(evidence["execution_elapsed_seconds"], 4.0)
        self.assertEqual(evidence["elapsed_seconds"], 11.0)
        self.assertEqual(timing["elapsed_seconds"], 15.0)
        self.assertEqual(
            evidence["selection_command"][:3],
            ["cargo", "nextest", "list"],
        )
        self.assertEqual(
            evidence["selected_identity_sha256"],
            test_scope._identity_fingerprint(
                {"hex_map::hex_map test_one", "hex_map::hex_map test_two"}
            ),
        )

    def test_run_rejects_zero_selected_tests_before_execution(self) -> None:
        definition = {
            "command": ["cargo", "nextest", "run", "--package", "hex_map"]
        }
        selection = subprocess.CompletedProcess(
            args=[], returncode=0, stdout="", stderr=""
        )
        with tempfile.TemporaryDirectory() as directory:
            timing_path = pathlib.Path(directory) / "timing.json"
            with mock.patch.object(
                test_scope.subprocess, "run", return_value=selection
            ) as run_mock:
                returncode = test_scope.run_concern(
                    "map_unit", definition, timing_path
                )
            timing = json.loads(timing_path.read_text(encoding="utf-8"))

        self.assertEqual(returncode, 4)
        self.assertEqual(run_mock.call_count, 1)
        self.assertEqual(timing["exit_code"], 4)
        self.assertEqual(timing["commands"][0]["selected_count"], 0)
        self.assertFalse(timing["commands"][0]["executed"])

    def test_selection_failure_replays_stdout_without_duplicating_stderr(self) -> None:
        definition = {
            "command": ["cargo", "nextest", "run", "--package", "hex_map"]
        }
        selection = subprocess.CompletedProcess(
            args=[],
            returncode=101,
            stdout="captured selection stdout\n",
            stderr="already streamed cargo stderr\n",
        )
        with mock.patch.object(
            test_scope.subprocess, "run", return_value=selection
        ) as run_mock, mock.patch.object(
            test_scope, "_replay_captured_stdout"
        ) as replay_stdout, mock.patch.object(
            test_scope, "_replay_captured_output"
        ) as replay_both:
            returncode = test_scope.run_concern("selection-failure", definition)

        self.assertEqual(returncode, 101)
        replay_stdout.assert_called_once_with(selection)
        replay_both.assert_not_called()
        call = run_mock.call_args
        self.assertIs(call.kwargs["stdout"], subprocess.PIPE)
        self.assertNotIn("capture_output", call.kwargs)
        self.assertNotIn("stderr", call.kwargs)

    def test_libtest_followup_is_listed_with_its_name_filter(self) -> None:
        command = [
            "cargo",
            "test",
            "--package",
            "hex_ui",
            "--lib",
            "dev_time::tests::",
        ]
        selection = test_scope._selection_command(command)

        self.assertIsNotNone(selection)
        listed, format_name = selection
        self.assertEqual(format_name, "libtest")
        self.assertEqual(listed[-2:], ["--", "--list"])
        self.assertIn("dev_time::tests::", listed)
        self.assertEqual(
            test_scope._parse_listed_tests(
                "dev_time::tests::first: test\n0 tests, 0 benchmarks\n",
                format_name,
            ),
            {"dev_time::tests::first"},
        )
        self.assertIsNone(
            test_scope._selection_command(
                ["cargo", "test", "--package", "hex_game", "--no-run"]
            )
        )

    def test_cargo_test_sums_all_final_libtest_summaries(self) -> None:
        definition = {
            "command": ["cargo", "test", "--package", "hex_ui", "focused"]
        }
        selection = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="suite::focused_one: test\nsuite::focused_two: test\n",
            stderr="",
        )
        execution = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=(
                "\x1b[32mtest result: ok. 0 passed; 0 failed; 1 ignored; "
                "0 measured; 1 filtered out\x1b[0m\n"
                "test result: ok. 2 passed; 0 failed; 0 ignored; "
                "0 measured; 0 filtered out\n"
            ),
            stderr="",
        )
        with tempfile.TemporaryDirectory() as directory:
            timing_path = pathlib.Path(directory) / "timing.json"
            with mock.patch.object(
                test_scope.subprocess,
                "run",
                side_effect=(selection, execution),
            ) as run_mock:
                returncode = test_scope.run_concern(
                    "cargo-test", definition, timing_path
                )
            timing = json.loads(timing_path.read_text(encoding="utf-8"))

        self.assertEqual(returncode, 0)
        evidence = timing["commands"][0]
        self.assertEqual(evidence["libtest_summary_count"], 2)
        self.assertEqual(evidence["passed_test_count"], 2)
        self.assertEqual(evidence["failed_test_count"], 0)
        self.assertEqual(evidence["ignored_test_count"], 1)
        self.assertEqual(evidence["executed_test_count"], 2)
        self.assertTrue(run_mock.call_args_list[1].kwargs["capture_output"])
        self.assertTrue(run_mock.call_args_list[1].kwargs["text"])

    def test_cargo_test_ignored_only_success_is_rejected(self) -> None:
        definition = {
            "command": ["cargo", "test", "--package", "hex_ui", "ignored_only"]
        }
        selection = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="suite::ignored_only: test\n",
            stderr="",
        )
        execution = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout=(
                "test result: ok. 0 passed; 0 failed; 1 ignored; "
                "0 measured; 9 filtered out\n"
            ),
            stderr="",
        )
        with tempfile.TemporaryDirectory() as directory:
            timing_path = pathlib.Path(directory) / "timing.json"
            with mock.patch.object(
                test_scope.subprocess,
                "run",
                side_effect=(selection, execution),
            ):
                returncode = test_scope.run_concern(
                    "ignored-only", definition, timing_path
                )
            timing = json.loads(timing_path.read_text(encoding="utf-8"))

        self.assertEqual(returncode, 4)
        evidence = timing["commands"][0]
        self.assertEqual(evidence["selected_count"], 1)
        self.assertEqual(evidence["executed_test_count"], 0)
        self.assertEqual(evidence["ignored_test_count"], 1)
        self.assertEqual(evidence["exit_code"], 4)

    def test_cargo_test_no_run_remains_a_compile_only_preflight(self) -> None:
        definition = {
            "command": [
                "cargo",
                "test",
                "--package",
                "hex_game",
                "--lib",
                "--no-run",
            ]
        }
        execution = subprocess.CompletedProcess(args=[], returncode=0)
        with tempfile.TemporaryDirectory() as directory:
            timing_path = pathlib.Path(directory) / "timing.json"
            with mock.patch.object(
                test_scope.subprocess, "run", return_value=execution
            ) as run_mock:
                returncode = test_scope.run_concern(
                    "compile-only", definition, timing_path
                )
            timing = json.loads(timing_path.read_text(encoding="utf-8"))

        self.assertEqual(returncode, 0)
        self.assertEqual(run_mock.call_count, 1)
        evidence = timing["commands"][0]
        self.assertNotIn("selection_command", evidence)
        self.assertNotIn("executed_test_count", evidence)
        self.assertTrue(evidence["executed"])

    def test_unittest_success_with_zero_tests_is_rejected(self) -> None:
        definition = {
            "command": [sys.executable, "-m", "unittest", "missing_suite.py"]
        }
        execution = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="",
            stderr="Ran 0 tests in 0.000s\n\nOK\n",
        )
        with mock.patch.object(
            test_scope.subprocess, "run", return_value=execution
        ):
            returncode = test_scope.run_concern("selector", definition)

        self.assertEqual(returncode, 4)

    def test_unittest_uses_the_outermost_reported_test_count(self) -> None:
        definition = {
            "command": [sys.executable, "-m", "unittest", "nested_suite.py"]
        }
        execution = subprocess.CompletedProcess(
            args=[],
            returncode=0,
            stdout="nested failure injection: Ran 0 tests\n",
            stderr="Ran 7 tests in 0.001s\n\nOK\n",
        )
        with mock.patch.object(
            test_scope.subprocess, "run", return_value=execution
        ):
            returncode = test_scope.run_concern("selector", definition)

        self.assertEqual(returncode, 0)

    def test_map_commands_share_the_optimized_test_profile(self) -> None:
        for concern in ("map_unit", "map_generation", "map_contracts"):
            command = self.config["concerns"][concern]["command"]
            self.assertEqual(
                command[command.index("--cargo-profile") + 1], "map-test"
            )

    def test_hex_ui_manifest_enforces_the_presentation_dependency_ceiling(self) -> None:
        manifest = (ROOT / "crates" / "hex_ui" / "Cargo.toml").read_text(
            encoding="utf-8"
        )
        allowed = {"bevy", "hex_assets", "hex_core", "hex_gameplay_model", "serde"}
        dependencies = toml_table_keys(manifest, "dependencies")
        self.assertEqual(dependencies, allowed)
        forbidden = {
            "hex_game",
            "hex_combat",
            "hex_units",
            "hex_lattice",
            "hex_map",
            "hex_world",
            "hex_perception",
        }
        self.assertTrue(forbidden.isdisjoint(dependencies))

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
        self.assertLess(
            map_job.index("tools/test_scope.py check-partitions map"),
            map_job.index("tools/test_scope.py run map_unit"),
        )
        self.assertNotIn("--workspace", map_job)
        self.assertNotIn("outputs.residual", map_job)

    def test_ci_runs_selector_through_its_zero_test_guard(self) -> None:
        workflow = (ROOT / ".github" / "workflows" / "ci.yaml").read_text(
            encoding="utf-8"
        )
        selector_step = workflow.split(
            "- name: Test the fail-closed selector", maxsplit=1
        )[1].split("- name:", maxsplit=1)[0]
        self.assertIn("python3 tools/test_scope.py run selector", selector_step)
        self.assertNotIn("python3 -m unittest", selector_step)

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
