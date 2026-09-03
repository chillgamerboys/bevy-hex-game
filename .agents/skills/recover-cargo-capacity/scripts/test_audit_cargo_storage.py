#!/usr/bin/env python3
"""Safety-contract tests for the read-only Cargo storage audit."""

from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock


SCRIPT_PATH = Path(__file__).with_name("audit_cargo_storage.py")
SPEC = importlib.util.spec_from_file_location("audit_cargo_storage", SCRIPT_PATH)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot import audit helper from {SCRIPT_PATH}")
AUDIT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(AUDIT)


EMPTY_PROCESS_SCAN = {"ok": True, "processes": [], "error": None}
FAILED_PROCESS_SCAN = {"ok": False, "processes": [], "error": "ps exited 1"}
APPEARED_PROCESS = {
    "ok": True,
    "processes": [{"pid": 4242, "rss_bytes": 4096, "process": "rustc"}],
    "error": None,
}
RSS_CHANGED_PROCESS = {
    "ok": True,
    "processes": [{"pid": 4242, "rss_bytes": 8192, "process": "rustc"}],
    "error": None,
}
FILESYSTEM = {
    "bytes_total": 10_000,
    "bytes_used": 4_000,
    "bytes_free": 6_000,
    "inodes_total": 1_000,
    "inodes_free": 900,
}
MEMORY = {
    "bytes_total": 8_000,
    "bytes_available_estimate": 6_000,
    "swap_bytes_total": 2_000,
    "swap_bytes_used": 0,
}


class GitRepository:
    """A temporary repository with a real, up-to-date local upstream."""

    def __init__(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        # macOS exposes /var as a symlink to /private/var. Resolve the temporary
        # root so that tests intending an ordinary path do not accidentally test
        # the separate symlink-component rejection contract.
        self.root = Path(self.temporary.name).resolve()
        self.remote = self.root / "remote.git"
        self.path = self.root / "checkout"
        self._run("git", "init", "--bare", str(self.remote), cwd=self.root)
        self._run(
            "git",
            "init",
            "--initial-branch=main",
            str(self.path),
            cwd=self.root,
        )
        self.git("config", "user.email", "cargo-audit-tests@example.invalid")
        self.git("config", "user.name", "Cargo Audit Tests")
        self.write("tracked.txt", "initial\n")
        self.git("add", "tracked.txt")
        self.git("commit", "-m", "initial")
        self.git("remote", "add", "origin", str(self.remote))
        self.git("push", "--set-upstream", "origin", "main")

    def close(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def _run(*args: str, cwd: Path) -> subprocess.CompletedProcess[str]:
        completed = subprocess.run(
            args,
            cwd=cwd,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if completed.returncode != 0:
            command = " ".join(args)
            raise AssertionError(
                f"command failed ({completed.returncode}): {command}\n"
                f"stdout:\n{completed.stdout}\nstderr:\n{completed.stderr}"
            )
        return completed

    def git(self, *args: str) -> subprocess.CompletedProcess[str]:
        return self._run("git", *args, cwd=self.path)

    def write(self, relative: str, content: str) -> Path:
        destination = self.path / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(content, encoding="utf-8")
        return destination

    def commit_and_sync(self, message: str) -> None:
        self.git("add", "--all")
        self.git("commit", "-m", message)
        self.git("push")


class RepositoryTestCase(unittest.TestCase):
    repository: GitRepository

    def setUp(self) -> None:
        self.repository = GitRepository()
        self.addCleanup(self.repository.close)

    def snapshot(self, *targets: Path):
        return AUDIT.status_snapshot(self.repository.path, list(targets))

    def configure_ignores(self, patterns: str) -> None:
        self.repository.write(".gitignore", patterns)
        self.repository.commit_and_sync("configure ignored paths")

    def assert_protected_for(self, snapshot, phrase: str) -> None:
        self.assertTrue(
            any(phrase in reason for reason in snapshot["protected_reasons"]),
            snapshot["protected_reasons"],
        )


class PathBoundaryTests(unittest.TestCase):
    def test_containment_is_path_aware_and_normalized(self) -> None:
        root = Path("/tmp/cargo-audit/repo/target")

        self.assertTrue(AUDIT.path_is_within(root, root))
        self.assertTrue(AUDIT.path_is_within(root / "debug" / "deps", root))
        self.assertTrue(
            AUDIT.path_is_within(root / "debug" / ".." / "release", root)
        )
        self.assertFalse(
            AUDIT.path_is_within(
                Path("/tmp/cargo-audit/repo/review/target/evidence.png"), root
            )
        )
        self.assertFalse(
            AUDIT.path_is_within(Path("/tmp/cargo-audit/repo/target-copy"), root)
        )

    def test_target_roots_are_normalized_and_indexed_by_owner(self) -> None:
        owner = Path(tempfile.gettempdir()).resolve() / "cargo-audit" / "repo"
        target = owner / "target"
        configured = owner / "build" / ".." / "cargo-cache"
        discoveries = [
            {
                "path": str(target),
                "lexical_paths": [str(owner / "." / "target")],
                "worktrees": [str(owner / ".")],
            },
            {
                "path": str(configured),
                "lexical_paths": [str(configured)],
                "worktrees": [str(owner)],
            },
        ]

        protected = {
            "worktree_roots": [str(AUDIT.normalized_path(owner))],
            "git_metadata": [str(AUDIT.normalized_path(owner / ".git"))],
        }
        indexed = AUDIT.target_roots_by_worktree(discoveries, protected)

        self.assertEqual(set(indexed), {str(AUDIT.normalized_path(owner))})
        self.assertEqual(
            set(indexed[str(AUDIT.normalized_path(owner))]),
            {
                AUDIT.normalized_path(target),
                AUDIT.normalized_path(configured),
            },
        )


class IgnoredPathClassificationTests(RepositoryTestCase):
    def test_unsafe_configured_root_cannot_mask_ignored_evidence(self) -> None:
        self.repository.write(
            ".cargo/config.toml",
            '[build]\ntarget-dir = "."\n',
        )
        self.repository.write(".gitignore", "/target/\n/.context/\n")
        self.repository.commit_and_sync("configure unsafe and conventional targets")
        target = self.repository.write(
            "target/CACHEDIR.TAG",
            f"{AUDIT.CACHE_SIGNATURE}\n",
        ).parent
        self.repository.write(".context/review/frame.png", "durable evidence\n")

        with (
            mock.patch.object(AUDIT, "running_rust_processes", return_value=EMPTY_PROCESS_SCAN),
            mock.patch.object(AUDIT, "du_bytes", return_value=512),
            mock.patch.object(AUDIT, "filesystem_snapshot", return_value=FILESYSTEM),
            mock.patch.object(AUDIT, "memory_snapshot", return_value=MEMORY),
        ):
            report = AUDIT.audit(self.repository.path)

        owner = next(
            item
            for item in report["worktrees"]
            if Path(item["path"]).resolve() == self.repository.path.resolve()
        )
        counts = owner["status"]["status_counts"]
        self.assertEqual(counts["ignored_within_target"], 1)
        self.assertEqual(counts["ignored_other"], 1)
        self.assert_protected_for(owner["status"], "ignored files")
        conventional = next(
            item
            for item in report["target_roots"]
            if Path(item["canonical_path"]) == target.resolve()
        )
        self.assertEqual(conventional["classification"], "WORKTREE STATE—PROTECT")
        self.assertTrue(
            all(
                not candidate["candidate_requires_manual_validation"]
                for candidate in conventional["candidates"]
            )
        )

    def test_only_normalized_conventional_target_is_not_protected(self) -> None:
        self.configure_ignores("/target/\n")
        self.repository.write("target/debug/dependency.rlib", "reconstructible\n")
        deliberately_lexical = (
            self.repository.path / "does-not-need-to-exist" / ".." / "target"
        )

        snapshot = self.snapshot(deliberately_lexical)

        self.assertEqual(snapshot["status_counts"]["ignored"], 1)
        self.assertEqual(snapshot["status_counts"]["ignored_within_target"], 1)
        self.assertEqual(snapshot["status_counts"]["ignored_other"], 0)
        self.assertEqual(snapshot["protected_reasons"], [])

    def test_only_normalized_configured_target_is_not_protected(self) -> None:
        self.repository.write(
            ".cargo/config.toml",
            '[build]\ntarget-dir = "build/../cargo-cache"\n',
        )
        self.repository.write(".gitignore", "/cargo-cache/\n")
        (self.repository.path / "build").mkdir()
        self.repository.commit_and_sync("configure a custom Cargo target")
        self.repository.write("cargo-cache/release/game", "reconstructible\n")
        configured = AUDIT.configured_target(self.repository.path)
        self.assertIsNotNone(configured)

        snapshot = self.snapshot(configured[0])

        self.assertEqual(snapshot["status_counts"]["ignored_within_target"], 1)
        self.assertEqual(snapshot["status_counts"]["ignored_other"], 0)
        self.assertEqual(snapshot["protected_reasons"], [])

    def test_ignored_evidence_outside_target_is_protected(self) -> None:
        self.configure_ignores("/.context/\n")
        self.repository.write(".context/review/frame.png", "durable evidence\n")

        snapshot = self.snapshot(self.repository.path / "target")

        self.assertEqual(snapshot["status_counts"]["ignored_within_target"], 0)
        self.assertEqual(snapshot["status_counts"]["ignored_other"], 1)
        self.assert_protected_for(snapshot, "ignored files")

    def test_target_and_ignored_evidence_remain_protected(self) -> None:
        self.configure_ignores("/target/\n/.context/\n")
        self.repository.write("target/debug/dependency.rlib", "reconstructible\n")
        self.repository.write(".context/review/frame.png", "durable evidence\n")

        snapshot = self.snapshot(self.repository.path / "target")

        self.assertEqual(snapshot["status_counts"]["ignored_within_target"], 1)
        self.assertEqual(snapshot["status_counts"]["ignored_other"], 1)
        self.assert_protected_for(snapshot, "ignored files")

    def test_literal_target_segment_outside_real_root_does_not_bypass_protection(self) -> None:
        self.configure_ignores("/review/target/\n")
        self.repository.write("review/target/frame.png", "durable evidence\n")

        snapshot = self.snapshot(self.repository.path / "target")

        self.assertEqual(snapshot["status_counts"]["ignored_within_target"], 0)
        self.assertEqual(snapshot["status_counts"]["ignored_other"], 1)
        self.assert_protected_for(snapshot, "ignored files")


class WorktreeProtectionTests(RepositoryTestCase):
    def test_tracked_change_is_protected(self) -> None:
        self.repository.write("tracked.txt", "changed\n")

        snapshot = self.snapshot()

        self.assertEqual(snapshot["status_counts"]["tracked"], 1)
        self.assert_protected_for(snapshot, "tracked or unmerged")

    def test_untracked_file_is_protected(self) -> None:
        self.repository.write("notes.txt", "not committed\n")

        snapshot = self.snapshot()

        self.assertEqual(snapshot["status_counts"]["untracked"], 1)
        self.assert_protected_for(snapshot, "untracked files")

    def test_commit_ahead_of_upstream_is_protected(self) -> None:
        self.repository.write("tracked.txt", "new commit\n")
        self.repository.git("add", "tracked.txt")
        self.repository.git("commit", "-m", "local-only commit")

        snapshot = self.snapshot()

        self.assertEqual(snapshot["branch"]["ahead"], 1)
        self.assert_protected_for(snapshot, "commits ahead")

    def test_branch_without_upstream_is_protected(self) -> None:
        self.repository.git("branch", "--unset-upstream")

        snapshot = self.snapshot()

        self.assertNotIn("upstream", snapshot["branch"])
        self.assert_protected_for(snapshot, "no upstream")

    def test_unmerged_change_is_protected(self) -> None:
        self.repository.git("checkout", "-b", "conflict-side")
        self.repository.write("tracked.txt", "side\n")
        self.repository.git("add", "tracked.txt")
        self.repository.git("commit", "-m", "side change")
        self.repository.git("checkout", "main")
        self.repository.write("tracked.txt", "main\n")
        self.repository.git("add", "tracked.txt")
        self.repository.git("commit", "-m", "main change")
        self.repository.git("push")
        merge = subprocess.run(
            ["git", "merge", "conflict-side"],
            cwd=self.repository.path,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        self.assertNotEqual(merge.returncode, 0)

        snapshot = self.snapshot()

        self.assertEqual(snapshot["status_counts"]["unmerged"], 1)
        self.assert_protected_for(snapshot, "tracked or unmerged")


class TargetEligibilityTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        # Keep baseline target paths free of the macOS /var -> /private/var
        # indirection; dedicated cases below introduce the symlinks they test.
        self.root = Path(self.temporary.name).resolve()
        self.owner = self.root / "checkout"
        self.owner.mkdir()
        (self.owner / ".git").mkdir()

    def write_valid_tag(self, target: Path) -> None:
        target.mkdir(parents=True, exist_ok=True)
        (target / "CACHEDIR.TAG").write_text(
            f"{AUDIT.CACHE_SIGNATURE}\n", encoding="utf-8"
        )

    def snapshot(
        self,
        target: Path,
        *,
        active=None,
        process_scan_ok: bool = True,
    ):
        owner = str(self.owner.resolve())
        discovery = {
            "path": str(target),
            "sources": ["test target"],
            "worktrees": [owner],
            "lexical_paths": [str(target)],
            "discovery_confidence": ["test-exact"],
        }
        statuses = {owner: {"protected_reasons": []}}
        protected = {
            "worktree_roots": [owner],
            "git_metadata": [str((self.owner / ".git").resolve())],
        }
        with (
            mock.patch.object(AUDIT, "du_bytes", return_value=512),
            mock.patch.object(AUDIT, "filesystem_snapshot", return_value=FILESYSTEM),
        ):
            return AUDIT.target_snapshot(
                discovery,
                [] if active is None else active,
                process_scan_ok,
                statuses,
                os.stat(self.owner).st_dev,
                protected,
            )

    def assert_ineligible(self, snapshot) -> None:
        self.assertTrue(snapshot["candidates"])
        self.assertTrue(
            all(
                not candidate["candidate_requires_manual_validation"]
                for candidate in snapshot["candidates"]
            )
        )

    def test_exact_regular_cache_tag_is_baseline_manual_candidate(self) -> None:
        target = self.owner / "target-valid"
        self.write_valid_tag(target)

        snapshot = self.snapshot(target)

        self.assertTrue(snapshot["tag"]["exact"])
        self.assertTrue(snapshot["candidates"][-1]["candidate_requires_manual_validation"])

    def test_profile_roots_are_reported_as_manual_candidates(self) -> None:
        target = self.owner / "target-profiles"
        self.write_valid_tag(target)
        (target / "debug" / "incremental").mkdir(parents=True)
        (target / "release").mkdir()

        snapshot = self.snapshot(target)

        candidates = {candidate["kind"]: candidate for candidate in snapshot["candidates"]}
        self.assertIn("debug incremental artifacts", candidates)
        self.assertIn("debug profile artifacts", candidates)
        self.assertIn("release profile artifacts", candidates)
        self.assertTrue(
            all(
                candidate["candidate_requires_manual_validation"]
                for candidate in candidates.values()
            )
        )

    def test_missing_cache_tag_is_ineligible(self) -> None:
        target = self.owner / "target-missing-tag"
        target.mkdir()

        snapshot = self.snapshot(target)

        self.assertFalse(snapshot["tag"]["exact"])
        self.assert_ineligible(snapshot)

    def test_invalid_cache_tag_is_ineligible(self) -> None:
        target = self.owner / "target-invalid-tag"
        target.mkdir()
        (target / "CACHEDIR.TAG").write_text("not Cargo's signature\n", encoding="utf-8")

        snapshot = self.snapshot(target)

        self.assertFalse(snapshot["tag"]["exact"])
        self.assert_ineligible(snapshot)

    def test_symlink_cache_tag_is_ineligible(self) -> None:
        target = self.owner / "target-symlink-tag"
        target.mkdir()
        real_tag = self.root / "real-cache-tag"
        real_tag.write_text(f"{AUDIT.CACHE_SIGNATURE}\n", encoding="utf-8")
        try:
            (target / "CACHEDIR.TAG").symlink_to(real_tag)
        except OSError as error:
            self.skipTest(f"symlinks unavailable: {error}")

        snapshot = self.snapshot(target)

        self.assertTrue(snapshot["tag"]["symlink"])
        self.assertFalse(snapshot["tag"]["exact"])
        self.assert_ineligible(snapshot)

    def test_symlink_target_is_ineligible(self) -> None:
        real_target = self.root / "real-target"
        self.write_valid_tag(real_target)
        target = self.owner / "target-link"
        try:
            target.symlink_to(real_target, target_is_directory=True)
        except OSError as error:
            self.skipTest(f"symlinks unavailable: {error}")

        snapshot = self.snapshot(target)

        self.assertTrue(snapshot["target_path_is_symlink"])
        self.assertTrue(snapshot["unsafe_path_reasons"])
        self.assert_ineligible(snapshot)

    def test_git_metadata_overlap_is_ineligible(self) -> None:
        target = self.owner / ".git"
        self.write_valid_tag(target)

        snapshot = self.snapshot(target)

        self.assertTrue(
            any("Git metadata" in reason for reason in snapshot["unsafe_path_reasons"])
        )
        self.assert_ineligible(snapshot)

    def test_active_process_is_ineligible(self) -> None:
        target = self.owner / "target-active"
        self.write_valid_tag(target)

        snapshot = self.snapshot(target, active=APPEARED_PROCESS["processes"])

        self.assertEqual(snapshot["classification"], "ACTIVE/LOCKED—DO NOT TOUCH")
        self.assert_ineligible(snapshot)

    def test_process_scan_failure_is_ineligible(self) -> None:
        target = self.owner / "target-scan-failure"
        self.write_valid_tag(target)

        snapshot = self.snapshot(target, process_scan_ok=False)

        self.assert_ineligible(snapshot)
        self.assertTrue(
            any(
                "inspection failed" in reason
                for candidate in snapshot["candidates"]
                for reason in candidate["reasons"]
            )
        )


class AuditProcessRaceTests(RepositoryTestCase):
    def setUp(self) -> None:
        super().setUp()
        self.configure_ignores("/target/\n")
        self.target = self.repository.path / "target"
        self.target.mkdir()
        (self.target / "CACHEDIR.TAG").write_text(
            f"{AUDIT.CACHE_SIGNATURE}\n", encoding="utf-8"
        )

    def run_audit_with_scans(self, scans):
        with (
            mock.patch.object(AUDIT, "running_rust_processes", side_effect=scans),
            mock.patch.object(AUDIT, "du_bytes", return_value=512),
            mock.patch.object(AUDIT, "filesystem_snapshot", return_value=FILESYSTEM),
            mock.patch.object(AUDIT, "memory_snapshot", return_value=MEMORY),
        ):
            return AUDIT.audit(self.repository.path)

    def only_target(self, report):
        targets = [
            target
            for target in report["target_roots"]
            if target["canonical_path"] == str(self.target.resolve())
        ]
        self.assertEqual(len(targets), 1)
        return targets[0]

    def test_process_appearing_between_scans_revokes_eligibility(self) -> None:
        report = self.run_audit_with_scans([EMPTY_PROCESS_SCAN, APPEARED_PROCESS])
        target = self.only_target(report)

        self.assertTrue(report["process_snapshot_changed"])
        self.assertEqual(target["classification"], "ACTIVE/LOCKED—DO NOT TOUCH")
        self.assertTrue(
            all(
                not candidate["candidate_requires_manual_validation"]
                for candidate in target["candidates"]
            )
        )
        self.assertTrue(
            any(
                "appeared during audit" in reason
                for candidate in target["candidates"]
                for reason in candidate["reasons"]
            )
        )

    def test_rss_change_does_not_claim_process_set_changed(self) -> None:
        report = self.run_audit_with_scans([APPEARED_PROCESS, RSS_CHANGED_PROCESS])
        target = self.only_target(report)

        self.assertFalse(report["process_snapshot_changed"])
        self.assertEqual(target["classification"], "ACTIVE/LOCKED—DO NOT TOUCH")

    def test_failed_process_scans_protect_every_candidate(self) -> None:
        report = self.run_audit_with_scans([FAILED_PROCESS_SCAN, FAILED_PROCESS_SCAN])
        target = self.only_target(report)

        self.assertTrue(
            all(
                not candidate["candidate_requires_manual_validation"]
                for candidate in target["candidates"]
            )
        )
        self.assertTrue(
            any("inspection failed" in warning for warning in report["warnings"])
        )


if __name__ == "__main__":
    unittest.main()
