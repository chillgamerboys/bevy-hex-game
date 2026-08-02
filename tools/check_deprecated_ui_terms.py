#!/usr/bin/env python3
"""Fail when superseded player-facing UI vocabulary escapes its history allowlist."""

from __future__ import annotations

import pathlib
import re
import subprocess
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]

# Keep the scanner's own source from containing the strings it detects.
DEPRECATED = (
    "Combat" + " Lab",
    "Combat" + "Lab",
    "combat" + "_" + "lab",
    "combat" + "-" + "lab",
    "Map " + "Scenarios",
    "Scenario " + "Browser",
    "Fixed " + "Fixtures",
    "Saved " + "Reports",
    "Screen::" + "Scenarios",
    "Scenario " + "catalog",
    "Scenarios " + "navigation",
    "Scenarios " + "menu",
    "Scenario " + "route",
    "Open" + "De" + "mos",
    "De" + "mos " + "menu",
    "De" + "mos",
)

# These files are historical provenance, not current product contracts. Adding a
# new path here is an explicit compatibility decision and should receive review.
HISTORICAL_ALLOWLIST = frozenset(
    {
        "CHANGELOG.md",
        "docs/planning/audit-log.md",
        "docs/planning/foundation-hardening.md",
    }
)

# Current roadmap/status projections contain explicitly labelled historical sections.
# Only lines beneath one of those headings receive an exception; current contracts in
# the same file remain subject to the gate.
SECTION_SCOPED_HISTORY = frozenset(
    {
        "docs/planning/roadmap.md",
        "docs/planning/status.md",
    }
)
HISTORY_MARKERS = ("historical", "superseded")
MARKDOWN_HEADING = re.compile(r"^(#{1,6})\s+(.+?)\s*$")


def contains_term(text: str, term: str) -> bool:
    """Match phrases as substrings, but standalone retired menu nouns as words."""

    if term == "De" + "mos":
        return re.search(r"\bdemos\b", text, flags=re.IGNORECASE) is not None
    return term.casefold() in text.casefold()


def historical_section_lines(lines: list[str]) -> set[int]:
    """Return one-based lines nested under explicitly historical headings."""

    stack: list[tuple[int, bool]] = []
    allowed: set[int] = set()
    for number, line in enumerate(lines, start=1):
        match = MARKDOWN_HEADING.match(line)
        if match:
            level = len(match.group(1))
            while stack and stack[-1][0] >= level:
                stack.pop()
            heading = match.group(2).casefold()
            stack.append(
                (level, any(marker in heading for marker in HISTORY_MARKERS))
            )
        if any(is_historical for _, is_historical in stack):
            allowed.add(number)
    return allowed


def repository_files() -> list[str]:
    """Return tracked and new non-ignored files, including pre-commit candidates."""

    result = subprocess.run(
        [
            "git",
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
    )
    return [path for path in result.stdout.decode().split("\0") if path]


def violations() -> list[str]:
    """Find non-historical path or source occurrences in stable order."""

    found: list[str] = []
    for relative in repository_files():
        path = ROOT / relative
        if relative in HISTORICAL_ALLOWLIST or not path.is_file():
            continue
        for term in DEPRECATED:
            if contains_term(relative, term):
                found.append(f"{relative}: path contains {term!r}")
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except (UnicodeDecodeError, OSError):
            continue
        allowed_lines = (
            historical_section_lines(lines)
            if relative in SECTION_SCOPED_HISTORY
            else set()
        )
        for number, line in enumerate(lines, start=1):
            if number in allowed_lines:
                continue
            for term in DEPRECATED:
                if contains_term(line, term):
                    found.append(f"{relative}:{number}: contains {term!r}")
    return sorted(found)


def main() -> int:
    """Print actionable violations and return a CI-friendly status."""

    found = violations()
    if found:
        print("superseded UI vocabulary escaped the historical allowlist:", file=sys.stderr)
        for violation in found:
            print(f"  {violation}", file=sys.stderr)
        return 1
    print("deprecated UI terminology scan passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
