#!/usr/bin/env python3
"""Fail-closed repository test-scope selection.

The JSON manifest is the authority. This script only validates and applies it.
Unknown paths, malformed configuration, and empty diffs select the full gate.
"""

from __future__ import annotations

import argparse
import fnmatch
import hashlib
import json
import os
import pathlib
import re
import shlex
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any, Iterable, Optional


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = REPOSITORY_ROOT / ".config" / "test-scopes.json"


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

    partition_checks = raw.get("partition_checks", {})
    if not isinstance(partition_checks, dict):
        raise ScopeConfigurationError("partition_checks must be an object")
    for name, definition in partition_checks.items():
        if not isinstance(name, str) or not name or not isinstance(definition, dict):
            raise ScopeConfigurationError("partition checks need named objects")
        partition_concerns = definition.get("concerns")
        required_ignored_patterns = definition.get("required_ignored_patterns")
        if (
            not isinstance(partition_concerns, list)
            or not partition_concerns
            or set(partition_concerns) - set(all_concerns)
            or not isinstance(required_ignored_patterns, list)
            or not required_ignored_patterns
            or len(required_ignored_patterns) != len(set(required_ignored_patterns))
            or not all(
                isinstance(pattern, str) and pattern
                for pattern in required_ignored_patterns
            )
        ):
            raise ScopeConfigurationError(
                f"partition check {name} has invalid concerns or ignored patterns"
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


def force_full(decision: ScopeDecision, all_concerns: Iterable[str]) -> ScopeDecision:
    """Promote a decision to the complete integration gate."""

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


def _nextest_oneline_command(command: list[str]) -> list[str]:
    """Make a nextest list command emit only stable, machine-readable identities."""

    rendered = list(command)
    if rendered[:3] == ["cargo", "nextest", "list"]:
        if "--message-format" not in rendered and not any(
            value.startswith("--message-format=") for value in rendered
        ):
            rendered.extend(("--message-format", "oneline"))
        if "--color" not in rendered and not any(
            value.startswith("--color=") for value in rendered
        ):
            rendered.extend(("--color", "never"))
    return rendered


def _selection_command(
    command: list[str],
) -> Optional[tuple[list[str], str]]:
    """Return the non-executing selection command for a canonical test command."""

    if command[:3] == ["cargo", "nextest", "run"]:
        listed = list(command)
        listed[2] = "list"
        return _nextest_oneline_command(listed), "nextest"
    if command[:2] == ["cargo", "test"] and "--no-run" not in command:
        listed = list(command)
        if "--" in listed:
            listed.append("--list")
        else:
            listed.extend(("--", "--list"))
        return listed, "libtest"
    return None


def _parse_listed_tests(output: str, format_name: str) -> set[str]:
    """Parse stable test identities from one supported listing format."""

    if format_name == "nextest":
        return {
            line.strip()
            for line in output.splitlines()
            if line.strip() and not line.startswith("warning:")
        }
    if format_name == "libtest":
        return {
            line.strip()[: -len(": test")]
            for line in output.splitlines()
            if line.strip().endswith(": test")
        }
    raise ScopeConfigurationError(f"unknown test listing format: {format_name}")


def _identity_fingerprint(identities: Iterable[str]) -> str:
    """Fingerprint an identity set without inflating timing artifacts with every name."""

    canonical = "\n".join(sorted(identities)).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def _listed_tests(command: list[str]) -> set[str]:
    """Run one nextest list command and return its stable test identities."""

    listed = _nextest_oneline_command(command)
    if listed[:3] != ["cargo", "nextest", "list"]:
        raise ScopeConfigurationError(
            f"partition listing is not a nextest list command: {shlex.join(command)}"
        )

    result = subprocess.run(
        listed,
        cwd=REPOSITORY_ROOT,
        check=True,
        stdout=subprocess.PIPE,
        text=True,
    )
    tests = _parse_listed_tests(result.stdout, "nextest")
    print(
        "scope partition selection: "
        f"selected_count={len(tests)} command={shlex.join(listed)}",
        flush=True,
    )
    if not tests:
        raise ScopeConfigurationError(
            f"test listing selected zero tests: {shlex.join(listed)}"
        )
    return tests


def check_partitions(name: str, config: dict[str, Any]) -> dict[str, Any]:
    """Prove a configured test partition is exhaustive and disjoint."""

    definition = config.get("partition_checks", {}).get(name)
    if definition is None:
        raise ScopeConfigurationError(f"unknown partition check: {name}")

    full = _listed_tests(definition["full_command"])
    selected: set[str] = set()
    partition_counts: dict[str, int] = {}
    for concern in definition["concerns"]:
        command = list(config["concerns"][concern]["command"])
        if command[:3] != ["cargo", "nextest", "run"]:
            raise ScopeConfigurationError(
                f"partition concern {concern} is not a nextest command"
            )
        command[2] = "list"
        tests = _listed_tests(command)
        partition_counts[concern] = len(tests)
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
    if not full <= all_tests:
        raise ScopeConfigurationError(
            "ordinary full test set is not contained in the ignored-inclusive set: "
            f"missing={sorted(full - all_tests)}"
        )
    ignored = all_tests - full
    for pattern in definition["required_ignored_patterns"]:
        matches = sorted(
            identity
            for identity in ignored
            if fnmatch.fnmatchcase(identity, pattern)
        )
        if len(matches) != 1:
            raise ScopeConfigurationError(
                "required ignored test pattern must match exactly one ignored identity: "
                f"pattern={pattern!r}, matches={matches}"
            )

    return {
        "full_count": len(full),
        "full_identity_sha256": _identity_fingerprint(full),
        "ignored_count": len(ignored),
        "ignored_identity_sha256": _identity_fingerprint(ignored),
        "partition_counts": partition_counts,
    }


def _is_unittest_command(command: list[str]) -> bool:
    """Return whether a command uses Python's built-in unittest runner."""

    return len(command) >= 3 and command[1:3] == ["-m", "unittest"]


def _is_executed_cargo_test_command(command: list[str]) -> bool:
    """Return whether Cargo will execute rather than only compile libtest targets."""

    return command[:2] == ["cargo", "test"] and "--no-run" not in command


def _libtest_summary_counts(output: str) -> dict[str, int]:
    """Sum every final libtest result line emitted by a cargo test command."""

    without_ansi = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", output)
    summaries = re.findall(
        r"(?m)^\s*test result:\s+(?:ok|FAILED)\.\s+"
        r"(\d+)\s+passed;\s+(\d+)\s+failed;\s+(\d+)\s+ignored;",
        without_ansi,
    )
    passed = sum(int(summary[0]) for summary in summaries)
    failed = sum(int(summary[1]) for summary in summaries)
    ignored = sum(int(summary[2]) for summary in summaries)
    return {
        "executed_test_count": passed + failed,
        "failed_test_count": failed,
        "ignored_test_count": ignored,
        "libtest_summary_count": len(summaries),
        "passed_test_count": passed,
    }


def _replay_captured_output(result: subprocess.CompletedProcess[str]) -> None:
    """Preserve ordinary command output after a short command is captured for evidence."""

    if result.stdout:
        print(result.stdout, end="", flush=True)
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr, flush=True)


def _replay_captured_stdout(result: subprocess.CompletedProcess[str]) -> None:
    """Replay stdout when stderr was inherited live by the child process."""

    if result.stdout:
        print(result.stdout, end="", flush=True)


def run_concern(
    concern: str,
    definition: dict[str, Any],
    timing_out: Optional[pathlib.Path] = None,
) -> int:
    """Run one concern, failing closed when a test command selects zero tests."""

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
    command_evidence: list[dict[str, Any]] = []

    for label, command in commands:
        command = list(command)
        record: dict[str, Any] = {
            "command": command,
            "executed": False,
            "execution_elapsed_seconds": 0.0,
            "label": label,
            "rendered_command": shlex.join(command),
            "selection_elapsed_seconds": 0.0,
        }
        command_started = time.monotonic()
        selection = _selection_command(command)
        if selection is not None:
            selection_command, format_name = selection
            record["selection_command"] = selection_command
            record["rendered_selection_command"] = shlex.join(selection_command)
            print(
                f"scope selection [{concern}/{label}]: "
                f"{shlex.join(selection_command)}",
                flush=True,
            )
            selection_started = time.monotonic()
            selection_result = subprocess.run(
                selection_command,
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
                stdout=subprocess.PIPE,
                text=True,
            )
            record["selection_elapsed_seconds"] = round(
                time.monotonic() - selection_started, 3
            )
            record["selection_exit_code"] = selection_result.returncode
            if selection_result.returncode != 0:
                _replay_captured_stdout(selection_result)
                returncode = selection_result.returncode
                record["exit_code"] = returncode
                record["elapsed_seconds"] = round(
                    time.monotonic() - command_started, 3
                )
                command_evidence.append(record)
                break
            identities = _parse_listed_tests(selection_result.stdout, format_name)
            selected_count = len(identities)
            record["selected_count"] = selected_count
            record["selected_identity_sha256"] = _identity_fingerprint(identities)
            print(
                f"scope selected [{concern}/{label}]: count={selected_count} "
                f"sha256={record['selected_identity_sha256']}",
                flush=True,
            )
            if selected_count == 0:
                print(
                    "test scope error: canonical test command selected zero tests: "
                    f"{shlex.join(command)}",
                    file=sys.stderr,
                )
                returncode = 4
                record["exit_code"] = returncode
                record["elapsed_seconds"] = round(
                    time.monotonic() - command_started, 3
                )
                command_evidence.append(record)
                break

        print(
            f"scope {label} [{concern}]: {shlex.join(command)}",
            flush=True,
        )
        record["executed"] = True
        execution_started = time.monotonic()
        if _is_unittest_command(command) or _is_executed_cargo_test_command(command):
            result = subprocess.run(
                command,
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
                capture_output=True,
                text=True,
            )
            record["execution_elapsed_seconds"] = round(
                time.monotonic() - execution_started, 3
            )
            _replay_captured_output(result)
        else:
            result = subprocess.run(
                command,
                cwd=REPOSITORY_ROOT,
                env=environment,
                check=False,
            )
            record["execution_elapsed_seconds"] = round(
                time.monotonic() - execution_started, 3
            )

        if _is_unittest_command(command):
            reported_counts = re.findall(
                r"\bRan\s+(\d+)\s+tests?\b",
                result.stdout + result.stderr,
            )
            # A test may itself exercise a unittest subprocess. The runner's own
            # summary is last, after every captured child-test diagnostic.
            selected_count = int(reported_counts[-1]) if reported_counts else 0
            record["selected_count"] = selected_count
            if result.returncode == 0 and selected_count == 0:
                print(
                    "test scope error: unittest command reported success without "
                    f"running tests: {shlex.join(command)}",
                    file=sys.stderr,
                )
                returncode = 4
            else:
                returncode = result.returncode
        elif _is_executed_cargo_test_command(command):
            counts = _libtest_summary_counts(result.stdout + "\n" + result.stderr)
            record.update(counts)
            if result.returncode == 0 and counts["executed_test_count"] == 0:
                print(
                    "test scope error: cargo test reported success without "
                    f"running tests: {shlex.join(command)}",
                    file=sys.stderr,
                )
                returncode = 4
            else:
                returncode = result.returncode
        else:
            returncode = result.returncode

        record["exit_code"] = returncode
        record["elapsed_seconds"] = round(time.monotonic() - command_started, 3)
        command_evidence.append(record)
        if returncode != 0:
            break

    timing = {
        "commands": command_evidence,
        "concern": concern,
        "elapsed_seconds": round(time.monotonic() - started, 3),
        "exit_code": returncode,
    }
    rendered_timing = json.dumps(timing, sort_keys=True)
    print(f"scope timing: {rendered_timing}")
    if timing_out is not None:
        write_output(timing_out, rendered_timing + "\n")
    return returncode


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
    plan.add_argument(
        "--force-full",
        action="store_true",
        help="promote the decision to the complete integration gate",
    )

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
    partitions = subparsers.add_parser(
        "check-partitions", help="verify one exhaustive test partition"
    )
    partitions.add_argument("name")
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
            return run_concern(
                arguments.concern,
                definition,
                arguments.timing_out,
            )
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
        if arguments.subcommand == "check-partitions":
            evidence = check_partitions(arguments.name, config)
            print(f"{arguments.name} test partitions are exhaustive and disjoint")
            print(f"partition evidence: {json.dumps(evidence, sort_keys=True)}")
            return 0

        if arguments.paths_file is not None:
            paths = arguments.paths_file.read_text(encoding="utf-8").splitlines()
        elif arguments.path:
            paths = arguments.path
        else:
            paths = changed_paths(arguments.base, arguments.head)
        decision = classify(paths, config)
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
