#!/usr/bin/env python3
"""Fail-closed repository test-scope selection.

The JSON manifest is the authority. This script only validates and applies it.
Unknown paths, malformed configuration, and empty diffs select the full gate.
"""

from __future__ import annotations

import argparse
import fnmatch
import json
import os
import pathlib
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any, Iterable


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = REPOSITORY_ROOT / ".config" / "test-scopes.json"
WAIVER_DIRECTORY = ".config/test-waivers/"
WAIVER_KEYS = {
    "version",
    "name",
    "description",
    "concerns",
    "allowed_patterns",
    "applies_to",
}


class ScopeConfigurationError(ValueError):
    """The scope manifest cannot be applied safely."""


def write_output(path: pathlib.Path, content: str) -> None:
    """Write one selector artifact, creating its repository-local parent."""

    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


@dataclass(frozen=True)
class ScopeDecision:
    """One fail-closed selection result."""

    changed_files: tuple[str, ...]
    concerns: tuple[str, ...]
    full: bool
    code: bool
    matched_rules: tuple[str, ...]
    reasons: tuple[str, ...]
    unknown_files: tuple[str, ...]
    waiver: str | None

    def as_dict(self, all_concerns: Iterable[str]) -> dict[str, Any]:
        """Return stable machine-readable output."""

        selected = set(self.concerns)
        return {
            "changed_files": list(self.changed_files),
            "code": self.code,
            "full": self.full,
            "matched_rules": list(self.matched_rules),
            "reasons": list(self.reasons),
            "unknown_files": list(self.unknown_files),
            "waiver": self.waiver,
            "concerns": {
                concern: concern in selected for concern in all_concerns
            },
        }


@dataclass(frozen=True)
class ScopeContext:
    """GitHub event identity used to constrain one-wave waivers."""

    event_name: str
    base_ref: str = ""
    head_ref: str = ""
    ref: str = ""
    pull_request_number: int | None = None


def load_config(path: pathlib.Path = DEFAULT_CONFIG) -> dict[str, Any]:
    """Load and validate the scope manifest."""

    try:
        raw = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ScopeConfigurationError(f"cannot load {path}: {error}") from error

    if raw.get("version") != 1:
        raise ScopeConfigurationError("scope manifest version must be 1")

    all_concerns = raw.get("all_concerns")
    concerns = raw.get("concerns")
    rules = raw.get("rules")
    if not isinstance(all_concerns, list) or not all(
        isinstance(value, str) and value for value in all_concerns
    ):
        raise ScopeConfigurationError("all_concerns must be non-empty strings")
    if len(set(all_concerns)) != len(all_concerns):
        raise ScopeConfigurationError("all_concerns contains duplicates")
    local_test_order = raw.get("local_test_order")
    if (
        not isinstance(local_test_order, list)
        or not local_test_order
        or set(local_test_order) - set(all_concerns)
    ):
        raise ScopeConfigurationError(
            "local_test_order must contain known concerns"
        )
    if not isinstance(concerns, dict) or set(concerns) != set(all_concerns):
        raise ScopeConfigurationError(
            "concerns must define exactly every all_concerns entry"
        )
    if not isinstance(rules, list) or not rules:
        raise ScopeConfigurationError("rules must be a non-empty list")

    waiver_manifests = raw.get("waiver_manifests")
    if not isinstance(waiver_manifests, list) or not waiver_manifests:
        raise ScopeConfigurationError(
            "waiver_manifests must contain approved repository-relative paths"
        )
    if (
        not all(
            isinstance(path, str)
            and path.startswith(WAIVER_DIRECTORY)
            and path.endswith(".json")
            and pathlib.PurePosixPath(path).as_posix() == path
            and ".." not in pathlib.PurePosixPath(path).parts
            for path in waiver_manifests
        )
        or len(set(waiver_manifests)) != len(waiver_manifests)
    ):
        raise ScopeConfigurationError(
            "waiver_manifests must be unique safe JSON paths under "
            f"{WAIVER_DIRECTORY}"
        )

    partition_checks = raw.get("partition_checks", {})
    if not isinstance(partition_checks, dict):
        raise ScopeConfigurationError("partition_checks must be an object")
    for name, definition in partition_checks.items():
        if not isinstance(name, str) or not name or not isinstance(definition, dict):
            raise ScopeConfigurationError("partition checks need named objects")
        partition_concerns = definition.get("concerns")
        expected_counts = definition.get("expected_counts")
        if (
            not isinstance(partition_concerns, list)
            or not partition_concerns
            or set(partition_concerns) - set(all_concerns)
            or not isinstance(expected_counts, dict)
            or set(expected_counts) != set(partition_concerns)
            or not all(
                isinstance(value, int) and value >= 0
                for value in expected_counts.values()
            )
        ):
            raise ScopeConfigurationError(
                f"partition check {name} has invalid concerns or counts"
            )
        for command_name in ("full_command", "all_tests_command"):
            partition_command = definition.get(command_name)
            if (
                not isinstance(partition_command, list)
                or not partition_command
                or not all(
                    isinstance(value, str) and value for value in partition_command
                )
            ):
                raise ScopeConfigurationError(
                    f"partition check {name} has invalid {command_name}"
                )
        if not isinstance(definition.get("expected_ignored"), int):
            raise ScopeConfigurationError(
                f"partition check {name} needs expected_ignored"
            )

    selection_checks = raw.get("selection_checks", {})
    if not isinstance(selection_checks, dict):
        raise ScopeConfigurationError("selection_checks must be an object")
    for concern, expected_commands in selection_checks.items():
        definition = concerns.get(concern)
        if not isinstance(definition, dict) or not isinstance(
            expected_commands, dict
        ):
            raise ScopeConfigurationError(
                "selection checks need known concern objects"
            )
        command_names = {
            name
            for name in (
                "preflight_command",
                "command",
                "postflight_command",
            )
            if name in definition
        }
        if set(expected_commands) != command_names:
            raise ScopeConfigurationError(
                f"selection check {concern} must cover exactly "
                f"{sorted(command_names)}"
            )
        for command_name, expected_tests in expected_commands.items():
            if (
                not isinstance(expected_tests, list)
                or not expected_tests
                or not all(
                    isinstance(test, str) and test for test in expected_tests
                )
                or len(set(expected_tests)) != len(expected_tests)
            ):
                raise ScopeConfigurationError(
                    f"selection check {concern} {command_name} needs "
                    "unique test identities"
                )

    for concern, definition in concerns.items():
        if not isinstance(definition, dict):
            raise ScopeConfigurationError(f"concern {concern} must be an object")
        command = definition.get("command")
        if (
            not isinstance(command, list)
            or not command
            or not all(isinstance(value, str) and value for value in command)
        ):
            raise ScopeConfigurationError(
                f"concern {concern} command must be non-empty strings"
            )
        for command_name in ("preflight_command", "postflight_command"):
            extra_command = definition.get(command_name)
            if extra_command is not None and (
                not isinstance(extra_command, list)
                or not extra_command
                or not all(
                    isinstance(value, str) and value for value in extra_command
                )
            ):
                raise ScopeConfigurationError(
                    f"concern {concern} {command_name} must be non-empty strings"
                )
        environment = definition.get("environment", {})
        if not isinstance(environment, dict) or not all(
            isinstance(key, str)
            and key
            and isinstance(value, str)
            for key, value in environment.items()
        ):
            raise ScopeConfigurationError(
                f"concern {concern} environment must map strings to strings"
            )

    names: set[str] = set()
    for rule in rules:
        if not isinstance(rule, dict):
            raise ScopeConfigurationError("each rule must be an object")
        name = rule.get("name")
        patterns = rule.get("patterns")
        reason = rule.get("reason")
        if not isinstance(name, str) or not name or name in names:
            raise ScopeConfigurationError("rule names must be unique non-empty strings")
        names.add(name)
        if not isinstance(patterns, list) or not all(
            isinstance(pattern, str) and pattern for pattern in patterns
        ):
            raise ScopeConfigurationError(f"rule {name} patterns are invalid")
        if not isinstance(reason, str) or not reason:
            raise ScopeConfigurationError(f"rule {name} needs a reason")
        full = rule.get("full", False)
        selected = rule.get("concerns", [])
        if not isinstance(full, bool) or not isinstance(selected, list):
            raise ScopeConfigurationError(f"rule {name} selection is invalid")
        documentation_only = rule.get("documentation_only", False)
        if not isinstance(documentation_only, bool):
            raise ScopeConfigurationError(
                f"rule {name} documentation_only must be a boolean"
            )
        unknown = set(selected) - set(all_concerns)
        if unknown:
            raise ScopeConfigurationError(
                f"rule {name} selects unknown concerns: {sorted(unknown)}"
            )
        if not full and not selected:
            raise ScopeConfigurationError(f"rule {name} selects nothing")

    return raw


def normalize_paths(paths: Iterable[str]) -> tuple[str, ...]:
    """Normalize repository-relative paths and reject unsafe inputs."""

    normalized: set[str] = set()
    for raw_path in paths:
        path = raw_path.strip().replace("\\", "/")
        if not path:
            continue
        candidate = pathlib.PurePosixPath(path)
        if candidate.is_absolute() or ".." in candidate.parts:
            raise ScopeConfigurationError(
                f"changed path must be repository-relative: {raw_path}"
            )
        normalized.add(candidate.as_posix())
    return tuple(sorted(normalized))


def _matches(path: str, patterns: Iterable[str]) -> bool:
    return any(fnmatch.fnmatchcase(path, pattern) for pattern in patterns)


def _full_decision(
    decision: ScopeDecision,
    all_concerns: tuple[str, ...],
    matched_rule: str,
    reason: str,
) -> ScopeDecision:
    """Return an explicit fail-closed decision while retaining its evidence."""

    return ScopeDecision(
        changed_files=decision.changed_files,
        concerns=all_concerns,
        full=True,
        code=True,
        matched_rules=tuple(sorted((*decision.matched_rules, matched_rule))),
        reasons=tuple(sorted((*decision.reasons, reason))),
        unknown_files=decision.unknown_files,
        waiver=None,
    )


def _load_waiver(
    path: pathlib.Path,
    manifest_path: str,
    all_concerns: tuple[str, ...],
) -> dict[str, Any]:
    """Load one exact waiver manifest or reject it as unsafe."""

    try:
        waiver = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ScopeConfigurationError(
            f"cannot load waiver {manifest_path}: {error}"
        ) from error
    if not isinstance(waiver, dict) or set(waiver) != WAIVER_KEYS:
        raise ScopeConfigurationError(
            f"waiver {manifest_path} must define exactly {sorted(WAIVER_KEYS)}"
        )
    if waiver.get("version") != 1:
        raise ScopeConfigurationError(f"waiver {manifest_path} version must be 1")
    name = waiver.get("name")
    description = waiver.get("description")
    concerns = waiver.get("concerns")
    patterns = waiver.get("allowed_patterns")
    applies_to = waiver.get("applies_to")
    if (
        not isinstance(name, str)
        or not name
        or name != pathlib.PurePosixPath(manifest_path).stem
        or not isinstance(description, str)
        or not description
    ):
        raise ScopeConfigurationError(
            f"waiver {manifest_path} needs a matching name and description"
        )
    if (
        not isinstance(concerns, list)
        or not concerns
        or not all(isinstance(concern, str) for concern in concerns)
        or len(set(concerns)) != len(concerns)
        or set(concerns) - set(all_concerns)
        or "selector" not in concerns
    ):
        raise ScopeConfigurationError(
            f"waiver {manifest_path} has unknown, duplicate, or unsafe concerns"
        )
    if (
        not isinstance(patterns, list)
        or not patterns
        or not all(
            isinstance(pattern, str)
            and pattern
            and not pathlib.PurePosixPath(pattern).is_absolute()
            and ".." not in pathlib.PurePosixPath(pattern).parts
            for pattern in patterns
        )
        or len(set(patterns)) != len(patterns)
        or not _matches(manifest_path, patterns)
    ):
        raise ScopeConfigurationError(
            f"waiver {manifest_path} has unsafe patterns or does not admit itself"
        )
    if not isinstance(applies_to, dict) or set(applies_to) != {
        "pull_request",
        "push",
    }:
        raise ScopeConfigurationError(
            f"waiver {manifest_path} needs exact pull_request and push contexts"
        )
    pull_request = applies_to.get("pull_request")
    push = applies_to.get("push")
    if (
        not isinstance(pull_request, dict)
        or set(pull_request) != {"number", "base_ref", "head_ref"}
        or not isinstance(pull_request.get("number"), int)
        or isinstance(pull_request["number"], bool)
        or pull_request["number"] <= 0
        or not isinstance(pull_request.get("base_ref"), str)
        or pull_request["base_ref"] != "dev"
        or not isinstance(pull_request.get("head_ref"), str)
        or not pull_request["head_ref"]
        or not isinstance(push, dict)
        or set(push) != {"ref"}
        or not isinstance(push.get("ref"), str)
        or push["ref"] != "refs/heads/dev"
    ):
        raise ScopeConfigurationError(
            f"waiver {manifest_path} has invalid event context values"
        )
    return waiver


def _waiver_applies_to_context(
    waiver: dict[str, Any], context: ScopeContext | None
) -> bool:
    """Return whether one waiver is authorized for the exact GitHub event."""

    if context is None:
        return False
    applies_to = waiver["applies_to"]
    if context.event_name == "pull_request":
        expected = applies_to["pull_request"]
        return (
            context.pull_request_number == expected["number"]
            and context.base_ref == expected["base_ref"]
            and context.head_ref == expected["head_ref"]
        )
    if context.event_name == "push":
        return context.ref == applies_to["push"]["ref"]
    return False


def _apply_waiver(
    decision: ScopeDecision,
    config: dict[str, Any],
    repository_root: pathlib.Path,
    context: ScopeContext | None,
) -> ScopeDecision:
    """Apply one tracked, self-declared, exact-diff waiver or fail closed."""

    all_concerns = tuple(config["all_concerns"])
    configured = set(config["waiver_manifests"])
    changed_candidates = tuple(
        path
        for path in decision.changed_files
        if path.startswith(WAIVER_DIRECTORY)
    )
    if not changed_candidates:
        return decision
    if len(changed_candidates) != 1 or changed_candidates[0] not in configured:
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-unknown-waiver",
            "Unknown or multiple waiver manifests select the full gate.",
        )

    manifest_path = changed_candidates[0]
    try:
        tracked = subprocess.run(
            ["git", "ls-files", "--error-unmatch", "--", manifest_path],
            cwd=repository_root,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        tracked = None
    if tracked is None or tracked.returncode != 0:
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-untracked-waiver",
            "An untracked waiver manifest cannot narrow validation.",
        )

    try:
        waiver = _load_waiver(
            repository_root / manifest_path,
            manifest_path,
            all_concerns,
        )
    except ScopeConfigurationError as error:
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-invalid-waiver",
            str(error),
        )

    if not _waiver_applies_to_context(waiver, context):
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-waiver-context",
            "The one-wave waiver does not apply to this event, ref, or PR.",
        )

    if decision.unknown_files:
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-waiver-unknown-path",
            "A waiver cannot admit paths unknown to ordinary scope routing.",
        )
    outside = tuple(
        path
        for path in decision.changed_files
        if not _matches(path, waiver["allowed_patterns"])
    )
    if outside:
        return _full_decision(
            decision,
            all_concerns,
            "fail-closed-waiver-outside-allowlist",
            "Changed paths outside the waiver allowlist select the full gate: "
            + ", ".join(outside),
        )

    name = waiver["name"]
    selected = set(waiver["concerns"])
    return ScopeDecision(
        changed_files=decision.changed_files,
        concerns=tuple(
            concern for concern in all_concerns if concern in selected
        ),
        full=False,
        code=True,
        matched_rules=tuple(
            sorted((*decision.matched_rules, f"waiver:{name}"))
        ),
        reasons=(
            f"Tracked one-wave waiver {name}: {waiver['description']}",
        ),
        unknown_files=(),
        waiver=name,
    )


def classify(
    paths: Iterable[str],
    config: dict[str, Any],
    repository_root: pathlib.Path = REPOSITORY_ROOT,
    context: ScopeContext | None = None,
) -> ScopeDecision:
    """Classify changed paths, failing closed to the full gate."""

    changed_files = normalize_paths(paths)
    all_concerns = tuple(config["all_concerns"])
    if not changed_files:
        return ScopeDecision(
            changed_files=(),
            concerns=all_concerns,
            full=True,
            code=True,
            matched_rules=("fail-closed-empty-diff",),
            reasons=("No changed paths were available; selecting the full gate.",),
            unknown_files=(),
            waiver=None,
        )

    selected: set[str] = set()
    matched_rules: set[str] = set()
    reasons: set[str] = set()
    unknown_files: list[str] = []
    full = False
    documentation_only = True

    for path in changed_files:
        path_matched = False
        path_documentation_only = False
        for rule in config["rules"]:
            if not _matches(path, rule["patterns"]):
                continue
            path_matched = True
            matched_rules.add(rule["name"])
            reasons.add(rule["reason"])
            if rule.get("full", False):
                full = True
            selected.update(rule.get("concerns", []))
            path_documentation_only = rule.get("documentation_only", False)
            # Rules are ordered from the narrowest authority to its fail-closed
            # fallback. One changed path has one owner; concern unions happen
            # across changed paths rather than by also applying broad fallbacks.
            break
        if not path_matched:
            unknown_files.append(path)
            full = True
            reasons.add("Unknown paths select the full gate.")
        if not path_documentation_only:
            documentation_only = False

    if full:
        selected.update(all_concerns)
    decision = ScopeDecision(
        changed_files=changed_files,
        concerns=tuple(
            concern for concern in all_concerns if concern in selected
        ),
        full=full,
        code=not documentation_only,
        matched_rules=tuple(sorted(matched_rules)),
        reasons=tuple(sorted(reasons)),
        unknown_files=tuple(unknown_files),
        waiver=None,
    )
    return _apply_waiver(decision, config, repository_root, context)


def force_full(decision: ScopeDecision, all_concerns: Iterable[str]) -> ScopeDecision:
    """Promote integration unless an exact tracked one-wave waiver is active."""

    if decision.waiver is not None:
        return ScopeDecision(
            changed_files=decision.changed_files,
            concerns=decision.concerns,
            full=False,
            code=True,
            matched_rules=decision.matched_rules,
            reasons=tuple(
                sorted(
                    (
                        *decision.reasons,
                        "The exact tracked one-wave waiver also applies to this "
                        "integration push.",
                    )
                )
            ),
            unknown_files=decision.unknown_files,
            waiver=decision.waiver,
        )

    return ScopeDecision(
        changed_files=decision.changed_files,
        concerns=tuple(all_concerns),
        full=True,
        code=True,
        matched_rules=tuple(sorted((*decision.matched_rules, "forced-full-integration"))),
        reasons=tuple(
            sorted(
                (
                    *decision.reasons,
                    "Pushes to dev/main require complete integration validation.",
                )
            )
        ),
        unknown_files=decision.unknown_files,
        waiver=None,
    )


def changed_paths(base: str, head: str) -> tuple[str, ...]:
    """Read committed, staged, working-tree, and untracked paths from Git."""

    commands = (
        ["git", "diff", "--name-only", "-z", f"{base}...{head}"],
        ["git", "diff", "--name-only", "-z", "--cached"],
        ["git", "diff", "--name-only", "-z"],
        ["git", "ls-files", "--others", "--exclude-standard", "-z"],
    )
    paths: list[str] = []
    for command in commands:
        result = subprocess.run(
            command,
            cwd=REPOSITORY_ROOT,
            check=True,
            capture_output=True,
        )
        paths.extend(
            value.decode("utf-8")
            for value in result.stdout.split(b"\0")
            if value
        )
    return normalize_paths(paths)


def write_github_outputs(
    path: pathlib.Path, decision: ScopeDecision, all_concerns: Iterable[str]
) -> None:
    """Append stable booleans and a compact reason to GitHub step outputs."""

    output = decision.as_dict(all_concerns)
    lines = [
        f"code={str(decision.code).lower()}",
        f"full={str(decision.full).lower()}",
        f"waiver={decision.waiver or ''}",
    ]
    for concern, enabled in output["concerns"].items():
        lines.append(f"{concern}={str(enabled).lower()}")
    lines.append(
        "reason="
        + " | ".join(decision.reasons).replace("\n", " ").replace("\r", " ")
    )
    with path.open("a", encoding="utf-8") as output_file:
        output_file.write("\n".join(lines))
        output_file.write("\n")


def check_workspace_graph(concern: str, config: dict[str, Any]) -> None:
    """Reject unexpected internal packages in a narrow concern graph."""

    definition = config["concerns"].get(concern)
    if definition is None:
        raise ScopeConfigurationError(f"unknown concern: {concern}")
    roots = definition.get("root_packages")
    allowed = definition.get("allowed_workspace_packages")
    if not roots or not allowed:
        raise ScopeConfigurationError(
            f"concern {concern} has no workspace graph contract"
        )

    result = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    metadata = json.loads(result.stdout)
    workspace_ids = set(metadata["workspace_members"])
    packages = {package["id"]: package["name"] for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    root_ids = {
        package_id
        for package_id in workspace_ids
        if packages.get(package_id) in set(roots)
    }
    if {packages[root_id] for root_id in root_ids} != set(roots):
        raise ScopeConfigurationError(f"cannot resolve every {concern} root package")

    visited: set[str] = set()
    pending = list(root_ids)
    while pending:
        package_id = pending.pop()
        if package_id in visited:
            continue
        visited.add(package_id)
        node = nodes.get(package_id)
        if node is None:
            raise ScopeConfigurationError(f"missing metadata node: {package_id}")
        pending.extend(
            dependency["pkg"]
            for dependency in node["deps"]
            if dependency["pkg"] not in visited
        )

    internal = {packages[package_id] for package_id in visited & workspace_ids}
    unexpected = internal - set(allowed)
    if unexpected:
        raise ScopeConfigurationError(
            f"{concern} graph contains unexpected workspace packages: "
            f"{sorted(unexpected)}"
        )


def _listed_tests(command: list[str]) -> set[str]:
    """Run one list command and return its stable test identities."""

    stable_command = list(command)
    if stable_command[:3] == ["cargo", "nextest", "list"]:
        stable_command[3:3] = ["--color", "never"]
    result = subprocess.run(
        stable_command,
        cwd=REPOSITORY_ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    return {
        line.strip()
        for line in result.stdout.splitlines()
        if line.strip() and not line.startswith("warning:")
    }


def check_selection(concern: str, config: dict[str, Any]) -> None:
    """Prove one concern's nextest filters select the exact reviewed tests."""

    expected_commands = config.get("selection_checks", {}).get(concern)
    if expected_commands is None:
        raise ScopeConfigurationError(f"unknown selection check: {concern}")
    definition = config["concerns"][concern]
    for command_name, expected_tests in expected_commands.items():
        command = list(definition[command_name])
        if command[:3] != ["cargo", "nextest", "run"]:
            raise ScopeConfigurationError(
                f"selection check {concern} {command_name} is not nextest"
            )
        command[2] = "list"
        actual = _listed_tests(command)
        expected = set(expected_tests)
        if actual != expected:
            raise ScopeConfigurationError(
                f"selection {concern} {command_name} differs from its "
                f"{len(expected)} reviewed tests: "
                f"missing={sorted(expected - actual)}, "
                f"extra={sorted(actual - expected)}"
            )


def check_partitions(name: str, config: dict[str, Any]) -> None:
    """Prove a configured test partition is exhaustive and disjoint."""

    definition = config.get("partition_checks", {}).get(name)
    if definition is None:
        raise ScopeConfigurationError(f"unknown partition check: {name}")

    full = _listed_tests(definition["full_command"])
    selected: set[str] = set()
    for concern in definition["concerns"]:
        command = list(config["concerns"][concern]["command"])
        if command[:3] != ["cargo", "nextest", "run"]:
            raise ScopeConfigurationError(
                f"partition concern {concern} is not a nextest command"
            )
        command[2] = "list"
        tests = _listed_tests(command)
        expected = definition["expected_counts"][concern]
        if len(tests) != expected:
            raise ScopeConfigurationError(
                f"partition {concern} has {len(tests)} tests, expected {expected}"
            )
        overlap = selected & tests
        if overlap:
            raise ScopeConfigurationError(
                f"partition {concern} overlaps existing tests: {sorted(overlap)}"
            )
        selected.update(tests)

    if selected != full:
        raise ScopeConfigurationError(
            "partition union differs from full test set: "
            f"missing={sorted(full - selected)}, extra={sorted(selected - full)}"
        )

    all_tests = _listed_tests(definition["all_tests_command"])
    discoverable = {line for line in all_tests if line.endswith(": test")}
    ignored = len(discoverable) - len(full)
    if ignored != definition["expected_ignored"]:
        raise ScopeConfigurationError(
            f"partition has {ignored} ignored tests, expected "
            f"{definition['expected_ignored']}"
        )


def build_parser() -> argparse.ArgumentParser:
    """Build the command-line interface."""

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=pathlib.Path, default=DEFAULT_CONFIG)
    subparsers = parser.add_subparsers(dest="subcommand", required=True)

    plan = subparsers.add_parser("plan", help="classify a diff or explicit paths")
    source = plan.add_mutually_exclusive_group()
    source.add_argument("--paths-file", type=pathlib.Path)
    source.add_argument("--path", action="append", default=[])
    plan.add_argument("--base", default="origin/dev")
    plan.add_argument("--head", default="HEAD")
    plan.add_argument("--json-out", type=pathlib.Path)
    plan.add_argument("--github-output", type=pathlib.Path)
    plan.add_argument("--event-name", default="")
    plan.add_argument("--base-ref", default="")
    plan.add_argument("--head-ref", default="")
    plan.add_argument("--ref", default="")
    plan.add_argument("--pull-request-number", type=int)
    plan.add_argument(
        "--force-full",
        action="store_true",
        help="promote the decision to the complete integration gate",
    )

    selected = subparsers.add_parser(
        "selected-tests", help="print selected local test concerns in run order"
    )
    selected_source = selected.add_mutually_exclusive_group()
    selected_source.add_argument("--paths-file", type=pathlib.Path)
    selected_source.add_argument("--path", action="append", default=[])
    selected.add_argument("--base", default="origin/dev")
    selected.add_argument("--head", default="HEAD")
    selected.add_argument("--event-name", default="")
    selected.add_argument("--base-ref", default="")
    selected.add_argument("--head-ref", default="")
    selected.add_argument("--ref", default="")
    selected.add_argument("--pull-request-number", type=int)
    selected.add_argument(
        "--force-full",
        action="store_true",
        help="promote the decision to the complete integration gate",
    )

    command = subparsers.add_parser("command", help="print one canonical command")
    command.add_argument("concern")

    run = subparsers.add_parser("run", help="run one canonical concern command")
    run.add_argument("concern")
    run.add_argument("--timing-out", type=pathlib.Path)

    graph = subparsers.add_parser(
        "check-graph", help="verify one narrow workspace dependency ceiling"
    )
    graph.add_argument("concern")
    partitions = subparsers.add_parser(
        "check-partitions", help="verify one exhaustive test partition"
    )
    partitions.add_argument("name")
    selection = subparsers.add_parser(
        "check-selection", help="verify one exact nextest selection"
    )
    selection.add_argument("concern")
    return parser


def main() -> int:
    """Apply the selected command and return a shell-compatible status."""

    parser = build_parser()
    arguments = parser.parse_args()
    try:
        config = load_config(arguments.config)
        if arguments.subcommand == "command":
            definition = config["concerns"].get(arguments.concern)
            if definition is None:
                raise ScopeConfigurationError(
                    f"unknown concern: {arguments.concern}"
                )
            commands = [
                *(
                    [definition["preflight_command"]]
                    if "preflight_command" in definition
                    else []
                ),
                definition["command"],
                *(
                    [definition["postflight_command"]]
                    if "postflight_command" in definition
                    else []
                ),
            ]
            print(" && ".join(shlex.join(command) for command in commands))
            return 0
        if arguments.subcommand == "run":
            definition = config["concerns"].get(arguments.concern)
            if definition is None:
                raise ScopeConfigurationError(
                    f"unknown concern: {arguments.concern}"
                )
            if arguments.concern in config.get("selection_checks", {}):
                check_selection(arguments.concern, config)
            commands = [
                *(
                    [("preflight", definition["preflight_command"])]
                    if "preflight_command" in definition
                    else []
                ),
                ("command", definition["command"]),
                *(
                    [("postflight", definition["postflight_command"])]
                    if "postflight_command" in definition
                    else []
                ),
            ]
            environment = os.environ.copy()
            environment.update(definition.get("environment", {}))
            started = time.monotonic()
            returncode = 0
            for label, command in commands:
                print(
                    f"scope {label} [{arguments.concern}]: {shlex.join(command)}",
                    flush=True,
                )
                result = subprocess.run(
                    command,
                    cwd=REPOSITORY_ROOT,
                    env=environment,
                    check=False,
                )
                returncode = result.returncode
                if returncode != 0:
                    break
            timing = {
                "concern": arguments.concern,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "exit_code": returncode,
            }
            rendered_timing = json.dumps(timing, sort_keys=True)
            print(f"scope timing: {rendered_timing}")
            if arguments.timing_out is not None:
                write_output(arguments.timing_out, rendered_timing + "\n")
            return returncode
        if arguments.subcommand == "selected-tests":
            if arguments.paths_file is not None:
                paths = arguments.paths_file.read_text(encoding="utf-8").splitlines()
            elif arguments.path:
                paths = arguments.path
            else:
                paths = changed_paths(arguments.base, arguments.head)
            decision = classify(
                paths,
                config,
                context=ScopeContext(
                    event_name=arguments.event_name,
                    base_ref=arguments.base_ref,
                    head_ref=arguments.head_ref,
                    ref=arguments.ref,
                    pull_request_number=arguments.pull_request_number,
                ),
            )
            if arguments.force_full:
                decision = force_full(decision, config["all_concerns"])
            selected_concerns = set(decision.concerns)
            print(
                " ".join(
                    concern
                    for concern in config["local_test_order"]
                    if concern in selected_concerns
                )
            )
            return 0
        if arguments.subcommand == "check-graph":
            check_workspace_graph(arguments.concern, config)
            print(f"{arguments.concern} workspace dependency graph is within ceiling")
            return 0
        if arguments.subcommand == "check-partitions":
            check_partitions(arguments.name, config)
            print(f"{arguments.name} test partitions are exhaustive and disjoint")
            return 0
        if arguments.subcommand == "check-selection":
            check_selection(arguments.concern, config)
            print(f"{arguments.concern} test selection matches reviewed identities")
            return 0

        if arguments.paths_file is not None:
            paths = arguments.paths_file.read_text(encoding="utf-8").splitlines()
        elif arguments.path:
            paths = arguments.path
        else:
            paths = changed_paths(arguments.base, arguments.head)
        decision = classify(
            paths,
            config,
            context=ScopeContext(
                event_name=arguments.event_name,
                base_ref=arguments.base_ref,
                head_ref=arguments.head_ref,
                ref=arguments.ref,
                pull_request_number=arguments.pull_request_number,
            ),
        )
        if arguments.force_full:
            decision = force_full(decision, config["all_concerns"])
        output = decision.as_dict(config["all_concerns"])
        rendered = json.dumps(output, indent=2, sort_keys=True)
        print(rendered)
        if arguments.json_out is not None:
            write_output(arguments.json_out, rendered + "\n")
        if arguments.github_output is not None:
            write_github_outputs(
                arguments.github_output, decision, config["all_concerns"]
            )
        return 0
    except (ScopeConfigurationError, OSError, subprocess.CalledProcessError) as error:
        print(f"test scope error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
