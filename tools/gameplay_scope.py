#!/usr/bin/env python3
"""Fail-closed gameplay test-scope selection.

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
DEFAULT_CONFIG = REPOSITORY_ROOT / ".config" / "gameplay-test-scopes.json"


class ScopeConfigurationError(ValueError):
    """The scope manifest cannot be applied safely."""


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
            "concerns": {
                concern: concern in selected for concern in all_concerns
            },
        }


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

    for concern, definition in concerns.items():
        if not isinstance(definition, dict):
            raise ScopeConfigurationError(f"concern {concern} must be an object")
        command = definition.get("command")
        if not isinstance(command, list) or not all(
            isinstance(value, str) and value for value in command
        ):
            raise ScopeConfigurationError(
                f"concern {concern} command must be non-empty strings"
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


def classify(paths: Iterable[str], config: dict[str, Any]) -> ScopeDecision:
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
    return ScopeDecision(
        changed_files=changed_files,
        concerns=tuple(
            concern for concern in all_concerns if concern in selected
        ),
        full=full,
        code=not documentation_only,
        matched_rules=tuple(sorted(matched_rules)),
        reasons=tuple(sorted(reasons)),
        unknown_files=tuple(unknown_files),
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

    selected = subparsers.add_parser(
        "selected-tests", help="print selected local test concerns in run order"
    )
    selected.add_argument("--base", default="origin/dev")
    selected.add_argument("--head", default="HEAD")

    command = subparsers.add_parser("command", help="print one canonical command")
    command.add_argument("concern")

    run = subparsers.add_parser("run", help="run one canonical concern command")
    run.add_argument("concern")
    run.add_argument("--timing-out", type=pathlib.Path)

    graph = subparsers.add_parser(
        "check-graph", help="verify one narrow workspace dependency ceiling"
    )
    graph.add_argument("concern")
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
            print(shlex.join(definition["command"]))
            return 0
        if arguments.subcommand == "run":
            definition = config["concerns"].get(arguments.concern)
            if definition is None:
                raise ScopeConfigurationError(
                    f"unknown concern: {arguments.concern}"
                )
            command = definition["command"]
            environment = os.environ.copy()
            environment.update(definition.get("environment", {}))
            print(
                f"scope command [{arguments.concern}]: {shlex.join(command)}",
                flush=True,
            )
            started = time.monotonic()
            result = subprocess.run(
                command,
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
            )
            timing = {
                "concern": arguments.concern,
                "elapsed_seconds": round(time.monotonic() - started, 3),
                "exit_code": result.returncode,
            }
            rendered_timing = json.dumps(timing, sort_keys=True)
            print(f"scope timing: {rendered_timing}")
            if arguments.timing_out is not None:
                arguments.timing_out.write_text(
                    rendered_timing + "\n", encoding="utf-8"
                )
            return result.returncode
        if arguments.subcommand == "selected-tests":
            decision = classify(
                changed_paths(arguments.base, arguments.head), config
            )
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

        if arguments.paths_file is not None:
            paths = arguments.paths_file.read_text(encoding="utf-8").splitlines()
        elif arguments.path:
            paths = arguments.path
        else:
            paths = changed_paths(arguments.base, arguments.head)
        decision = classify(paths, config)
        output = decision.as_dict(config["all_concerns"])
        rendered = json.dumps(output, indent=2, sort_keys=True)
        print(rendered)
        if arguments.json_out is not None:
            arguments.json_out.write_text(rendered + "\n", encoding="utf-8")
        if arguments.github_output is not None:
            write_github_outputs(
                arguments.github_output, decision, config["all_concerns"]
            )
        return 0
    except (ScopeConfigurationError, OSError, subprocess.CalledProcessError) as error:
        print(f"gameplay scope error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
