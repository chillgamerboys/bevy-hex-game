"""Unit contracts for the fail-closed review entrypoint.

These tests never invoke Cargo, a generator, or the renderer. Child launches are
mocked except for one bounded POSIX fork that proves inherited lock lifetime.
"""

from __future__ import annotations

import argparse
import io
import json
import os
import pathlib
import tempfile
import unittest
from unittest import mock

from tools import review


GIB = 1024 * 1024 * 1024


class ReviewToolTests(unittest.TestCase):
    def provenance(self, *, dirty: bool = True, suffix: str = "1") -> dict:
        return {
            "git_head": "a" * 40,
            "worktree_dirty": dirty,
            "diff_sha256": suffix * 64,
            "workspace_content_sha256": "b" * 64,
            "index_overrides": {
                "assume_unchanged": [],
                "skip_worktree": [],
            },
        }

    def toolchain(self, *, status: str = "VERIFIED") -> dict:
        def result(command, stdout, returncode=0):
            return {
                "command": command,
                "started": True,
                "returncode": returncode,
                "timed_out": False,
                "duration_seconds": 0.01,
                "stdout": stdout,
                "stderr": "",
            }

        return {
            "status": status,
            "errors": [] if status == "VERIFIED" else ["cargo identity failed"],
            "cargo": result(
                ["cargo", "--version", "--verbose"],
                "cargo 1.97.1\nrelease: 1.97.1\nhost: test-host\n",
            ),
            "rustc": result(
                ["rustc", "--version", "--verbose"],
                "rustc 1.97.1\nrelease: 1.97.1\nhost: test-host\n",
            ),
            "rustup": {
                **result(
                    ["rustup", "show", "active-toolchain"],
                    "1.97.1-test-host (overridden)\n",
                ),
                "available": True,
            },
        }

    def arguments(
        self,
        output: pathlib.Path,
        *,
        mode: str = "author",
        draft: bool = False,
    ) -> argparse.Namespace:
        return argparse.Namespace(
            command="structural",
            mode=mode,
            seed=91,
            output=output,
            allow_structural_draft=draft,
            build_timeout_seconds=17,
            run_timeout_seconds=19,
            min_free_gib=20,
        )

    def make_repository(self, directory: str) -> pathlib.Path:
        root = pathlib.Path(directory) / "repository"
        for sentinel in review.ASSET_SENTINELS:
            path = root / sentinel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text("fixture\n", encoding="utf-8")
        (root / "target" / "grand-v3-structural-preview").mkdir(
            parents=True, exist_ok=True
        )
        return root

    def test_provenance_hashes_diff_and_untracked_contents(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "tracked.txt").write_text("tracked\n", encoding="utf-8")
            (root / "new.txt").write_text("first\n", encoding="utf-8")

            def fake_git(_root, *arguments):
                if arguments[:2] == ("rev-parse", "--verify"):
                    return b"A" * 40 + b"\n"
                if arguments[0] == "status":
                    return b"?? new.txt\0"
                if arguments[:3] == ("ls-files", "-v", "-z"):
                    return b"H tracked.txt\0"
                if arguments[0] == "diff":
                    return b"diff --git a/tracked.txt b/tracked.txt\n"
                if arguments[:3] == ("ls-files", "-z", "--others"):
                    return b"new.txt\0"
                if arguments[:3] == ("ls-files", "-z", "--cached"):
                    return b"new.txt\0tracked.txt\0"
                self.fail(f"unexpected git arguments: {arguments}")

            with mock.patch.object(review, "_git", side_effect=fake_git):
                first = review._workspace_provenance(root)
                (root / "new.txt").write_text("second\n", encoding="utf-8")
                second = review._workspace_provenance(root)

            self.assertEqual(first["git_head"], "a" * 40)
            self.assertTrue(first["worktree_dirty"])
            self.assertNotEqual(first["diff_sha256"], second["diff_sha256"])
            self.assertNotEqual(
                first["workspace_content_sha256"],
                second["workspace_content_sha256"],
            )

    def test_stable_provenance_retries_a_concurrent_edit(self) -> None:
        first = self.provenance(suffix="1")
        second = self.provenance(suffix="2")
        with mock.patch.object(
            review,
            "_workspace_provenance",
            side_effect=[first, second, second, second],
        ) as snapshot:
            resolved = review._stable_workspace_provenance(pathlib.Path("/unused"))
        self.assertEqual(resolved, second)
        self.assertEqual(snapshot.call_count, 4)

    def test_hidden_index_flags_are_recorded_hashed_and_treated_as_dirty(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "tracked.txt").write_text("tracked\n", encoding="utf-8")
            (root / "sparse.txt").write_text("sparse\n", encoding="utf-8")

            def provenance_with(metadata: bytes) -> dict:
                def fake_git(_root, *arguments):
                    if arguments[:2] == ("rev-parse", "--verify"):
                        return b"a" * 40 + b"\n"
                    if arguments[0] == "status" or arguments[0] == "diff":
                        return b""
                    if arguments[:3] == ("ls-files", "-v", "-z"):
                        return metadata
                    if arguments[:3] == ("ls-files", "-z", "--others"):
                        return b""
                    if arguments[:3] == ("ls-files", "-z", "--cached"):
                        return b"sparse.txt\0tracked.txt\0"
                    self.fail(f"unexpected git arguments: {arguments}")

                with mock.patch.object(review, "_git", side_effect=fake_git):
                    return review._workspace_provenance(root)

            hidden = provenance_with(b"h tracked.txt\0S sparse.txt\0")
            ordinary = provenance_with(b"H tracked.txt\0H sparse.txt\0")
            self.assertTrue(hidden["worktree_dirty"])
            self.assertEqual(
                hidden["index_overrides"],
                {
                    "assume_unchanged": ["tracked.txt"],
                    "skip_worktree": ["sparse.txt"],
                },
            )
            self.assertNotEqual(hidden["diff_sha256"], ordinary["diff_sha256"])
            self.assertNotEqual(
                hidden["workspace_content_sha256"],
                ordinary["workspace_content_sha256"],
            )
            arguments = self.arguments(root / "unused", mode="checkpoint")
            manifest = review._base_manifest(
                arguments=arguments,
                output=arguments.output,
                provenance=hidden,
                build_command=review._structural_build_command("checkpoint"),
                resource_preflight={"status": "PENDING"},
            )
            self.assertIn("dirty-worktree", manifest["unapprovable_reasons"])
            self.assertFalse(manifest["verification"]["required"])

    def test_environment_scrubs_inherited_state_and_sets_explicit_policy(self) -> None:
        inherited = {
            "PATH": "/usr/bin",
            "BEVY_ASSET_ROOT": "/wrong",
            "HEX_GRAND_PROFILE": "1",
            "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT": "1",
            "HEX_REVIEW_SCENARIO": "wrong",
            "WGPU_BACKEND": "wrong",
            "RUSTFLAGS": "-C target-cpu=native",
            "RUSTC_BOOTSTRAP": "1",
            "RUSTC_WRAPPER": "/tmp/wrapper",
            "RUSTC_WORKSPACE_WRAPPER": "/tmp/workspace-wrapper",
            "RUSTUP_TOOLCHAIN": "nightly",
            "CARGO_BUILD_RUSTC": "/tmp/custom-rustc",
            "CARGO_BUILD_RUSTFLAGS": "-C opt-level=0",
            "CARGO_BUILD_RUSTC_WRAPPER": "/tmp/cargo-wrapper",
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "/tmp/cargo-workspace-wrapper",
            "CARGO_PROFILE_RELEASE_LTO": "off",
            "CARGO_TARGET_DIR": "/shared-target",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER": "wrong",
        }
        strict = review._sanitized_environment(
            inherited, mode="author", allow_structural_draft=False
        )
        self.assertEqual(strict["PATH"], "/usr/bin")
        self.assertEqual(strict["BEVY_ASSET_ROOT"], str(review.REPOSITORY_ROOT))
        self.assertEqual(strict["CARGO_INCREMENTAL"], "1")
        self.assertEqual(
            strict["CARGO_TARGET_DIR"], str(review.REPOSITORY_ROOT / "target")
        )
        for key in (
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_RUSTFLAGS",
            "CARGO_BUILD_RUSTC_WRAPPER",
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            "RUSTFLAGS",
            "RUSTC_WRAPPER",
            "RUSTC_WORKSPACE_WRAPPER",
        ):
            self.assertEqual(strict[key], "")
        for key in (
            "HEX_GRAND_PROFILE",
            "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT",
            "HEX_REVIEW_SCENARIO",
            "WGPU_BACKEND",
            "RUSTUP_TOOLCHAIN",
            "CARGO_BUILD_RUSTC",
            "CARGO_PROFILE_RELEASE_LTO",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER",
            "RUSTC_BOOTSTRAP",
        ):
            self.assertNotIn(key, strict)

        draft = review._sanitized_environment(
            inherited, mode="checkpoint", allow_structural_draft=True
        )
        self.assertEqual(draft["CARGO_INCREMENTAL"], "0")
        self.assertEqual(draft["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"], "1")

    def test_toolchain_identity_uses_exact_commands_and_allows_missing_rustup(self) -> None:
        calls = []

        def capture(command, *, cwd, environment, timeout_seconds=30, pass_fds=()):
            del cwd, timeout_seconds
            calls.append((tuple(command), dict(environment), tuple(pass_fds)))
            name = command[0]
            if name == "rustup":
                return {
                    "command": list(command),
                    "started": False,
                    "returncode": 127,
                    "timed_out": False,
                    "duration_seconds": 0.01,
                    "stdout": "",
                    "stderr": "missing rustup",
                }
            return {
                "command": list(command),
                "started": True,
                "returncode": 0,
                "timed_out": False,
                "duration_seconds": 0.01,
                "stdout": f"{name} 1.97.1\nrelease: 1.97.1\nhost: test-host\n",
                "stderr": "",
            }

        environment = {"PATH": "/usr/bin", "RUSTFLAGS": ""}
        with mock.patch.object(
            review, "_capture_identity_command", side_effect=capture
        ):
            identity = review._toolchain_identity(environment, pass_fds=(42,))
        self.assertEqual(identity["status"], "VERIFIED")
        self.assertFalse(identity["rustup"]["available"])
        self.assertEqual(
            [call[0] for call in calls],
            [
                ("cargo", "--version", "--verbose"),
                ("rustc", "--version", "--verbose"),
                ("rustup", "show", "active-toolchain"),
            ],
        )
        self.assertTrue(all(call[1] == environment for call in calls))
        self.assertTrue(all(call[2] == (42,) for call in calls))

    def test_author_and_checkpoint_build_shapes_are_explicit(self) -> None:
        author = review._structural_build_command("author")
        checkpoint = review._structural_build_command("checkpoint")
        self.assertNotIn("--release", author)
        self.assertIn("--release", checkpoint)
        for command in (author, checkpoint):
            self.assertEqual(command[:2], ("cargo", "build"))
            self.assertIn(review.STRUCTURAL_EXAMPLE, command)
            self.assertEqual(command[-1], "--message-format=json-render-diagnostics")

    def test_logged_process_timeout_is_bounded_and_timed_without_a_real_child(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            process = mock.Mock(pid=24680)
            process.wait.side_effect = [
                review.subprocess.TimeoutExpired(["cargo"], 3),
                0,
            ]
            with mock.patch.object(
                review.subprocess, "Popen", return_value=process
            ), mock.patch.object(review.os, "killpg") as killpg, mock.patch.object(
                review.time, "monotonic", side_effect=[10.0, 10.1, 14.0, 14.25]
            ):
                result = review._run_logged_process(
                    ["cargo", "build"],
                    cwd=pathlib.Path(directory),
                    environment={"PATH": "/usr/bin"},
                    log_path=pathlib.Path(directory) / "build.log",
                    timeout_seconds=3,
                )
            self.assertEqual(result.returncode, 124)
            self.assertTrue(result.started)
            self.assertTrue(result.timed_out)
            self.assertEqual(result.duration_seconds, 4.25)
            killpg.assert_called_once_with(24680, review.signal.SIGTERM)

    def test_logged_process_spawn_failure_retains_duration_and_not_started(self) -> None:
        with tempfile.TemporaryDirectory() as directory, mock.patch.object(
            review.subprocess, "Popen", side_effect=OSError("missing cargo")
        ), mock.patch.object(review.time, "monotonic", side_effect=[5.0, 5.5]):
            result = review._run_logged_process(
                ["cargo", "build"],
                cwd=pathlib.Path(directory),
                environment={"PATH": "/usr/bin"},
                log_path=pathlib.Path(directory) / "build.log",
                timeout_seconds=3,
            )
        self.assertEqual(result.returncode, 127)
        self.assertFalse(result.started)
        self.assertEqual(result.duration_seconds, 0.5)
        self.assertIn("missing cargo", result.error)

    def test_child_wait_interruption_terminates_process_group_and_reraises(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            process = mock.Mock(pid=13579)
            interruption = review.ReviewInterrupted(review.signal.SIGTERM)
            process.wait.side_effect = [interruption, 0]
            with mock.patch.object(
                review.subprocess, "Popen", return_value=process
            ), mock.patch.object(review.os, "killpg") as killpg:
                with self.assertRaises(review.ReviewInterrupted):
                    review._run_logged_process(
                        ["cargo", "build"],
                        cwd=pathlib.Path(directory),
                        environment={"PATH": "/usr/bin"},
                        log_path=pathlib.Path(directory) / "cancelled.log",
                        timeout_seconds=3,
                    )
            killpg.assert_called_once_with(13579, review.signal.SIGTERM)

    def test_cli_maps_sigterm_to_graceful_interruption_status(self) -> None:
        def interrupt(_arguments):
            review.signal.raise_signal(review.signal.SIGTERM)
            return 0

        stderr = io.StringIO()
        with mock.patch.object(
            review.sys, "argv", ["review.py", "structural", "author"]
        ), mock.patch.object(
            review, "run_structural", side_effect=interrupt
        ), mock.patch.object(review.sys, "stderr", stderr):
            self.assertEqual(review.main(), 128 + review.signal.SIGTERM)
        self.assertIn("SIGTERM", stderr.getvalue())

    def test_signal_handler_defers_interruption_until_after_durable_write(self) -> None:
        with tempfile.TemporaryDirectory() as directory, review._cli_signal_handlers():
            receipt = pathlib.Path(directory) / "receipt.json"
            review.signal.raise_signal(review.signal.SIGTERM)
            review._atomic_write(receipt, '{"status":"INCOMPLETE"}\n')
            self.assertTrue(receipt.is_file())
            with self.assertRaises(review.ReviewInterrupted):
                review._raise_if_interrupted()

    def test_exact_binary_resolution_requires_one_regular_example(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            binary = root / "grand_v3_structural_preview"
            binary.write_bytes(b"binary")
            log = root / "build.log"
            log.write_text(
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {
                            "name": review.STRUCTURAL_EXAMPLE,
                            "kind": ["example"],
                        },
                        "executable": str(binary),
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            self.assertEqual(review._resolve_structural_binary(log), binary.resolve())

            symlink = root / "linked-preview"
            symlink.symlink_to(binary)
            log.write_text(
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {
                            "name": review.STRUCTURAL_EXAMPLE,
                            "kind": ["example"],
                        },
                        "executable": str(symlink),
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(review.ReviewError, "must not be a symlink"):
                review._resolve_structural_binary(log)

            second = root / "other"
            second.write_bytes(b"other")
            log.write_text(
                json.dumps(
                    {
                        "reason": "compiler-artifact",
                        "target": {
                            "name": review.STRUCTURAL_EXAMPLE,
                            "kind": ["example"],
                        },
                        "executable": str(binary),
                    }
                )
                + "\n",
                encoding="utf-8",
            )
            with log.open("a", encoding="utf-8") as output:
                output.write(
                    json.dumps(
                        {
                            "reason": "compiler-artifact",
                            "target": {
                                "name": review.STRUCTURAL_EXAMPLE,
                                "kind": ["example"],
                            },
                            "executable": str(second),
                        }
                    )
                    + "\n"
                )
            with self.assertRaisesRegex(review.ReviewError, "exactly one"):
                review._resolve_structural_binary(log)

    def test_invalidation_removes_only_owned_outputs_and_marks_incomplete(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "review"
            output.mkdir()
            for name in (*review.STRUCTURAL_OUTPUTS, *review.OPERATIONAL_OUTPUTS):
                (output / name).write_text("stale\n", encoding="utf-8")
            notes = output / "review-notes.txt"
            notes.write_text("preserve\n", encoding="utf-8")

            review._invalidate_structural_output(output)

            self.assertEqual(
                (output / review.WRAPPER_INCOMPLETE_MARKER).read_text(encoding="utf-8"),
                review.WRAPPER_INCOMPLETE_NOTICE,
            )
            self.assertTrue((output / review.RUST_INCOMPLETE_MARKER).is_file())
            self.assertTrue(notes.is_file())
            self.assertTrue(
                all(
                    not (output / name).exists()
                    for name in (*review.STRUCTURAL_OUTPUTS, *review.OPERATIONAL_OUTPUTS)
                )
            )

    def test_atomic_receipt_fsyncs_file_and_directory_around_replace(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "receipt.json"
            events = []
            original_fsync = review.os.fsync
            original_replace = review.os.replace

            def fsync(descriptor):
                events.append("fsync")
                return original_fsync(descriptor)

            def replace(source, destination):
                events.append("replace")
                return original_replace(source, destination)

            with mock.patch.object(review.os, "fsync", side_effect=fsync), mock.patch.object(
                review.os, "replace", side_effect=replace
            ):
                review._atomic_write(path, '{"status":"COMPLETE"}\n')
            self.assertEqual(events, ["fsync", "replace", "fsync"])
            self.assertEqual(path.read_text(encoding="utf-8"), '{"status":"COMPLETE"}\n')

    def test_output_lock_is_nonblocking_and_preserves_existing_evidence(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "locked"
            output.mkdir()
            receipt = output / review.RUN_MANIFEST
            receipt.write_text('{"status":"COMPLETE"}\n', encoding="utf-8")
            arguments = self.arguments(output, mode="checkpoint")
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review,
                "STRUCTURAL_OUTPUT_ROOT",
                root / "target" / "grand-v3-structural-preview",
            ):
                with review._locked_output(output):
                    with mock.patch.object(
                        review,
                        "_stable_workspace_provenance",
                        return_value=self.provenance(dirty=False),
                    ), mock.patch.object(
                        review, "_invalidate_structural_output"
                    ) as invalidate, mock.patch.object(
                        review, "_free_space_preflight"
                    ) as preflight, mock.patch.object(
                        review, "_run_logged_process"
                    ) as runner:
                        with self.assertRaisesRegex(
                            review.ReviewError, "already owns"
                        ):
                            review.run_structural(arguments)
                    invalidate.assert_not_called()
                    preflight.assert_not_called()
                    runner.assert_not_called()
                    self.assertEqual(
                        receipt.read_text(encoding="utf-8"),
                        '{"status":"COMPLETE"}\n',
                    )
            self.assertTrue((output / review.OUTPUT_LOCK).is_file())

    @unittest.skipUnless(hasattr(os, "fork"), "requires POSIX inherited descriptors")
    def test_inherited_child_fd_keeps_lock_after_parent_lease_closes(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            review_root = root / "target" / "grand-v3-structural-preview"
            output = review_root / "inherited-lock"
            output.mkdir()
            ready_read, ready_write = os.pipe()
            release_read, release_write = os.pipe()
            child_pid = None
            try:
                with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                    review, "STRUCTURAL_OUTPUT_ROOT", review_root
                ):
                    with review._locked_output(output):
                        child_pid = os.fork()
                        if child_pid == 0:
                            os.close(ready_read)
                            os.close(release_write)
                            os.write(ready_write, b"1")
                            os.read(release_read, 1)
                            os._exit(0)
                        os.close(ready_write)
                        ready_write = -1
                        os.close(release_read)
                        release_read = -1
                        self.assertEqual(os.read(ready_read, 1), b"1")

                    with self.assertRaisesRegex(review.ReviewError, "already owns"):
                        with review._locked_output(output):
                            self.fail("the inherited child fd must retain the lock")

                    os.write(release_write, b"1")
                    os.close(release_write)
                    release_write = -1
                    _waited_pid, status = os.waitpid(child_pid, 0)
                    child_pid = None
                    self.assertTrue(os.WIFEXITED(status))
                    self.assertEqual(os.WEXITSTATUS(status), 0)
                    with review._locked_output(output):
                        pass
            finally:
                if release_write >= 0:
                    try:
                        os.write(release_write, b"1")
                    except OSError:
                        pass
                for descriptor in (ready_read, ready_write, release_read, release_write):
                    if descriptor >= 0:
                        try:
                            os.close(descriptor)
                        except OSError:
                            pass
                if child_pid is not None:
                    os.waitpid(child_pid, 0)

    def test_lock_revalidation_rejects_directory_swap_and_lock_symlink(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            review_root = root / "target" / "grand-v3-structural-preview"
            output = review_root / "swap"
            output.mkdir()
            outside = root / "outside"
            outside.mkdir()
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review, "STRUCTURAL_OUTPUT_ROOT", review_root
            ):
                with review._locked_output(output) as lease:
                    held = review_root / "held-original"
                    output.rename(held)
                    output.symlink_to(outside, target_is_directory=True)
                    with self.assertRaisesRegex(review.ReviewError, "symlink ancestor"):
                        lease.validate()
                self.assertEqual(list(outside.iterdir()), [])

                linked_output = review_root / "linked-lock"
                linked_output.mkdir()
                target = root / "do-not-touch.txt"
                target.write_text("safe\n", encoding="utf-8")
                (linked_output / review.OUTPUT_LOCK).symlink_to(target)
                with self.assertRaisesRegex(review.ReviewError, "cannot open"):
                    with review._locked_output(linked_output):
                        self.fail("symlink lock must not be acquired")
                self.assertEqual(target.read_text(encoding="utf-8"), "safe\n")

    def test_free_space_preflight_fails_before_cargo_with_actionable_message(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory) / "future" / "review"
            with mock.patch.object(
                review.shutil,
                "disk_usage",
                return_value=mock.Mock(free=int(1.6 * GIB)),
            ):
                with self.assertRaisesRegex(
                    review.ReviewError, "only 1.6 GiB.*requires at least 20 GiB"
                ):
                    review._free_space_preflight(output, 20)
            with self.assertRaisesRegex(review.ReviewError, "cannot be lower"):
                review._free_space_preflight(output, 19)

    def test_low_disk_invalidates_preexisting_complete_evidence_before_cargo(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "disk-blocked"
            output.mkdir()
            for name in review.STRUCTURAL_OUTPUTS:
                (output / name).write_text("stale evidence\n", encoding="utf-8")
            (output / review.RUN_MANIFEST).write_text(
                '{"status":"COMPLETE","approvable":true}\n', encoding="utf-8"
            )
            notes = output / "notes.txt"
            notes.write_text("preserve\n", encoding="utf-8")
            arguments = self.arguments(output, mode="checkpoint")
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review,
                "STRUCTURAL_OUTPUT_ROOT",
                root / "target" / "grand-v3-structural-preview",
            ), mock.patch.object(
                review,
                "_stable_workspace_provenance",
                return_value=self.provenance(dirty=False),
            ), mock.patch.object(
                review,
                "_free_space_preflight",
                side_effect=review.ReviewError("only 1.6 GiB is free"),
            ), mock.patch.object(review, "_run_logged_process") as runner:
                with self.assertRaisesRegex(review.ReviewError, "only 1.6 GiB"):
                    review.run_structural(arguments)
            runner.assert_not_called()
            self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())
            self.assertTrue((output / review.RUST_INCOMPLETE_MARKER).is_file())
            self.assertTrue(notes.is_file())
            self.assertTrue(
                all(not (output / name).exists() for name in review.STRUCTURAL_OUTPUTS)
            )
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "INCOMPLETE")
            self.assertFalse(manifest["approvable"])
            self.assertEqual(manifest["resource_preflight"]["status"], "FAILED")
            self.assertIn("only 1.6 GiB", manifest["error"])

    def test_missing_cargo_or_rustc_identity_fails_before_build(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            review_root = root / "target" / "grand-v3-structural-preview"
            output = review_root / "toolchain-fail"
            arguments = self.arguments(output, mode="author")
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review, "STRUCTURAL_OUTPUT_ROOT", review_root
            ), mock.patch.object(
                review,
                "_stable_workspace_provenance",
                return_value=self.provenance(dirty=False),
            ), mock.patch.object(
                review,
                "_free_space_preflight",
                return_value={"free_bytes": 25 * GIB, "minimum_free_bytes": 20 * GIB},
            ), mock.patch.object(
                review,
                "_toolchain_identity",
                return_value=self.toolchain(status="FAILED"),
            ), mock.patch.object(review, "_run_logged_process") as runner:
                with self.assertRaisesRegex(
                    review.ReviewError, "cannot establish cargo/rustc identity"
                ):
                    review.run_structural(arguments)
            runner.assert_not_called()
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "INCOMPLETE")
            self.assertEqual(manifest["toolchain"]["status"], "FAILED")
            self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())

    def test_manifest_schema_and_seed_are_exact_key_value_records(self) -> None:
        valid = (
            "grand_v3_structural_preview_version=1\n"
            "seed=91\n"
            "section=first,10\n"
            "section=second,20\n"
        )
        invalid = (
            "xgrand_v3_structural_preview_version=1\nseed=91\n",
            "grand_v3_structural_preview_version=2\nseed=91\n",
            "grand_v3_structural_preview_version=1\nseed=091\n",
            "grand_v3_structural_preview_version=1\nseed=91\nseed=91\n",
        )
        with tempfile.TemporaryDirectory() as directory:
            output = pathlib.Path(directory)
            for name in review.STRUCTURAL_OUTPUTS:
                (output / name).write_text("artifact\n", encoding="utf-8")
            (output / "manifest.txt").write_text(valid, encoding="utf-8")
            self.assertEqual(
                len(review._artifact_records(output, 91)),
                len(review.STRUCTURAL_OUTPUTS),
            )
            for contents in invalid:
                with self.subTest(contents=contents):
                    (output / "manifest.txt").write_text(contents, encoding="utf-8")
                    with self.assertRaises(review.ReviewError):
                        review._artifact_records(output, 91)

    def test_path_not_found_diagnostic_is_rejected_case_insensitively(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            log = pathlib.Path(directory) / "execution.log"
            log.write_text("renderer: PaTh NoT FoUnD: capture.png\n", encoding="utf-8")
            with self.assertRaisesRegex(review.ReviewError, "Path not found"):
                review._reject_known_diagnostics(log)

    def _fake_success_runner(
        self,
        binary: pathlib.Path,
        *,
        mutate_binary: bool = False,
        leave_producer_marker: bool = False,
        execution_diagnostic: str = "structural preview completed\n",
        verification_fresh: bool = True,
        mutate_artifact_on_verification: bool = False,
    ):
        calls = []
        build_calls = 0

        def run(
            command,
            *,
            cwd,
            environment,
            log_path,
            timeout_seconds,
            pass_fds=(),
        ):
            nonlocal build_calls
            del cwd, environment, timeout_seconds
            self.assertEqual(len(pass_fds), 1)
            calls.append(tuple(command))
            if command[0] == "cargo":
                build_calls += 1
                output = log_path.parent
                self.assertTrue(
                    (output / review.WRAPPER_INCOMPLETE_MARKER).is_file()
                )
                if build_calls == 1:
                    self.assertTrue(
                        all(
                            not (output / name).exists()
                            for name in review.STRUCTURAL_OUTPUTS
                        )
                    )
                log_path.write_text(
                    json.dumps(
                        {
                            "reason": "compiler-artifact",
                            "target": {
                                "name": review.STRUCTURAL_EXAMPLE,
                                "kind": ["example"],
                            },
                            "executable": str(binary),
                            "fresh": build_calls > 1 and verification_fresh,
                        }
                    )
                    + "\n",
                    encoding="utf-8",
                )
                if build_calls > 1 and mutate_artifact_on_verification:
                    (output / "height-map.csv").write_text(
                        "changed by verification\n", encoding="utf-8"
                    )
                return review.ProcessResult(0, 0.5 if build_calls > 1 else 1.25)

            output = pathlib.Path(command[command.index("--output") + 1])
            for name in review.STRUCTURAL_OUTPUTS:
                contents = "artifact\n"
                if name == "manifest.txt":
                    contents = (
                        "grand_v3_structural_preview_version=1\n"
                        f"seed={command[command.index('--seed') + 1]}\n"
                    )
                (output / name).write_text(contents, encoding="utf-8")
            if not leave_producer_marker:
                (output / review.RUST_INCOMPLETE_MARKER).unlink()
            log_path.write_text(execution_diagnostic, encoding="utf-8")
            if mutate_binary:
                binary.write_bytes(b"changed")
            return review.ProcessResult(0, 2.5)

        return calls, run

    def _run_success_case(
        self,
        root: pathlib.Path,
        arguments: argparse.Namespace,
        *,
        provenance: dict,
        mutate_binary: bool = False,
        final_provenance: dict = None,
        leave_producer_marker: bool = False,
        execution_diagnostic: str = "structural preview completed\n",
        verification_fresh: bool = True,
        mutate_artifact_on_verification: bool = False,
        provenance_sequence=None,
    ):
        binary = root / "target" / "debug" / "examples" / review.STRUCTURAL_EXAMPLE
        binary.parent.mkdir(parents=True, exist_ok=True)
        binary.write_bytes(b"binary")
        calls, runner = self._fake_success_runner(
            binary,
            mutate_binary=mutate_binary,
            leave_producer_marker=leave_producer_marker,
            execution_diagnostic=execution_diagnostic,
            verification_fresh=verification_fresh,
            mutate_artifact_on_verification=mutate_artifact_on_verification,
        )
        snapshots = provenance_sequence or [
            provenance,
            provenance,
            provenance,
            provenance,
        ]
        if final_provenance is not None:
            snapshots = [provenance, final_provenance]
        with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
            review,
            "STRUCTURAL_OUTPUT_ROOT",
            root / "target" / "grand-v3-structural-preview",
        ), mock.patch.object(
            review, "_stable_workspace_provenance", side_effect=snapshots
        ), mock.patch.object(
            review,
            "_free_space_preflight",
            return_value={"free_bytes": 25 * GIB, "minimum_free_bytes": 20 * GIB},
        ), mock.patch.object(
            review, "_toolchain_identity", return_value=self.toolchain()
        ), mock.patch.object(
            review, "_run_logged_process", side_effect=runner
        ):
            result = review.run_structural(arguments)
        return result, calls, binary

    def test_structural_author_success_records_complete_fresh_provenance(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "author-run"
            arguments = self.arguments(output, mode="author")
            provenance = self.provenance(dirty=False)

            result, calls, _binary = self._run_success_case(
                root, arguments, provenance=provenance
            )

            self.assertEqual(result, 0)
            self.assertEqual(len(calls), 2)
            self.assertNotIn("--release", calls[0])
            self.assertFalse((output / review.WRAPPER_INCOMPLETE_MARKER).exists())
            self.assertFalse((output / review.RUST_INCOMPLETE_MARKER).exists())
            self.assertTrue((output / review.OUTPUT_LOCK).is_file())
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "COMPLETE")
            self.assertEqual(manifest["workflow_mode"], "author")
            self.assertEqual(manifest["admission_mode"], "strict")
            self.assertEqual(
                manifest["commands"]["entrypoint"][:4],
                ["python3", "tools/review.py", "structural", "author"],
            )
            self.assertFalse(manifest["approvable"])
            self.assertEqual(manifest["unapprovable_reasons"], ["authoring-profile"])
            self.assertEqual(manifest["work"], {"selected_count": 1, "executed_count": 1})
            self.assertEqual(manifest["timings"]["build_seconds"], 1.25)
            self.assertEqual(manifest["timings"]["run_seconds"], 2.5)
            self.assertFalse(manifest["verification"]["required"])
            self.assertEqual(len(manifest["artifacts"]), len(review.STRUCTURAL_OUTPUTS))
            self.assertEqual(len(manifest["logs"]), 2)
            self.assertTrue(manifest["freshness"]["complete"])
            self.assertTrue(manifest["freshness"]["output_location_unchanged"])
            self.assertTrue(manifest["freshness"]["producer_complete"])
            self.assertEqual(manifest["provenance_start"], manifest["provenance_end"])
            self.assertEqual(
                manifest["provenance_observations"][-1]["stage"],
                "structural publication boundary",
            )
            self.assertIsNotNone(manifest["timings"]["total_seconds"])
            self.assertEqual(manifest["toolchain"]["status"], "VERIFIED")

    def test_checkpoint_strict_clean_is_approvable_and_release_nonincremental(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "checkpoint"
            arguments = self.arguments(output, mode="checkpoint")

            _result, calls, _binary = self._run_success_case(
                root, arguments, provenance=self.provenance(dirty=False)
            )
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertIn("--release", calls[0])
            self.assertEqual(len(calls), 3)
            self.assertEqual(calls[0], calls[2])
            self.assertEqual(manifest["environment"]["CARGO_INCREMENTAL"], "0")
            self.assertEqual(manifest["timings"]["verification_build_seconds"], 0.5)
            self.assertEqual(
                manifest["verification"],
                {
                    "required": True,
                    "performed": True,
                    "no_op": True,
                    "binary_unchanged": True,
                },
            )
            self.assertTrue(manifest["approvable"])
            self.assertEqual(manifest["unapprovable_reasons"], [])
            self.assertEqual(len(manifest["logs"]), 3)

    def test_explicit_draft_and_dirty_checkpoint_are_unapprovable(self) -> None:
        for label, draft, dirty, expected_reason in (
            ("draft", True, False, "structural-draft"),
            ("dirty", False, True, "dirty-worktree"),
        ):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = self.make_repository(directory)
                output = root / "target" / "grand-v3-structural-preview" / label
                arguments = self.arguments(output, mode="checkpoint", draft=draft)

                _result, calls, _binary = self._run_success_case(
                    root, arguments, provenance=self.provenance(dirty=dirty)
                )
                manifest = json.loads((output / review.RUN_MANIFEST).read_text())
                self.assertEqual(manifest["status"], "COMPLETE")
                self.assertFalse(manifest["approvable"])
                self.assertIn(expected_reason, manifest["unapprovable_reasons"])
                self.assertFalse(manifest["verification"]["required"])
                self.assertEqual(len(calls), 2)
                if draft:
                    self.assertEqual(manifest["admission_mode"], "structural-draft")
                    self.assertEqual(
                        manifest["environment"][
                            "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"
                        ],
                        "1",
                    )

    def test_success_exit_requires_producer_marker_removal_and_clean_diagnostics(self) -> None:
        for label, options, message in (
            ("marker", {"leave_producer_marker": True}, "retained its INCOMPLETE"),
            (
                "diagnostic",
                {"execution_diagnostic": "Path not found: review/output.png\n"},
                "Path not found",
            ),
        ):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = self.make_repository(directory)
                output = (
                    root / "target" / "grand-v3-structural-preview" / f"bad-{label}"
                )
                arguments = self.arguments(output, mode="author")
                with self.assertRaisesRegex(review.ReviewError, message):
                    self._run_success_case(
                        root,
                        arguments,
                        provenance=self.provenance(dirty=False),
                        **options,
                    )
                manifest = json.loads((output / review.RUN_MANIFEST).read_text())
                self.assertEqual(manifest["status"], "INCOMPLETE")
                self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())

    def test_non_noop_checkpoint_verification_is_unapprovable(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "not-noop"
            arguments = self.arguments(output, mode="checkpoint")
            with self.assertRaisesRegex(review.ReviewError, "not a complete Cargo no-op"):
                self._run_success_case(
                    root,
                    arguments,
                    provenance=self.provenance(dirty=False),
                    verification_fresh=False,
                )
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "INCOMPLETE")
            self.assertTrue(manifest["verification"]["performed"])
            self.assertFalse(manifest["verification"]["no_op"])

    def test_checkpoint_verification_cannot_change_structural_artifacts(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "mutated-pack"
            arguments = self.arguments(output, mode="checkpoint")
            with self.assertRaisesRegex(review.ReviewError, "artifacts changed"):
                self._run_success_case(
                    root,
                    arguments,
                    provenance=self.provenance(dirty=False),
                    mutate_artifact_on_verification=True,
                )
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "INCOMPLETE")
            self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())

    def test_publication_boundary_drift_records_observed_end_and_false_freshness(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "late-drift"
            arguments = self.arguments(output, mode="checkpoint")
            start = self.provenance(dirty=False)
            changed = self.provenance(dirty=False, suffix="2")
            with self.assertRaisesRegex(review.ReviewError, "publication boundary"):
                self._run_success_case(
                    root,
                    arguments,
                    provenance=start,
                    provenance_sequence=[start, start, start, changed],
                )
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["status"], "INCOMPLETE")
            self.assertEqual(manifest["provenance_end"], changed)
            self.assertFalse(manifest["freshness"]["source_unchanged"])
            self.assertFalse(manifest["freshness"]["complete"])
            self.assertEqual(
                manifest["provenance_observations"][-1]["stage"],
                "structural publication boundary",
            )
            self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())

    def test_unstable_provenance_records_last_observed_snapshot_as_changed(self) -> None:
        start = self.provenance(dirty=False)
        observed = self.provenance(dirty=False, suffix="3")
        arguments = self.arguments(pathlib.Path("/unused"), mode="author")
        manifest = review._base_manifest(
            arguments=arguments,
            output=arguments.output,
            provenance=start,
            build_command=review._structural_build_command("author"),
            resource_preflight={"status": "PASSED"},
        )
        with mock.patch.object(
            review,
            "_stable_workspace_provenance",
            side_effect=review.ReviewError("workspace changed repeatedly"),
        ), mock.patch.object(
            review, "_workspace_provenance", return_value=observed
        ):
            with self.assertRaisesRegex(review.ReviewError, "was unstable"):
                review._require_unchanged_provenance(
                    manifest, start, stage="test observation"
                )
        self.assertEqual(manifest["provenance_end"], observed)
        self.assertFalse(manifest["freshness"]["source_unchanged"])

    def test_source_drift_or_binary_mutation_retains_incomplete_evidence(self) -> None:
        for label, mutation, final in (
            ("source", False, self.provenance(suffix="2")),
            ("binary", True, None),
        ):
            with self.subTest(label=label), tempfile.TemporaryDirectory() as directory:
                root = self.make_repository(directory)
                output = (
                    root / "target" / "grand-v3-structural-preview" / f"failed-{label}"
                )
                arguments = self.arguments(output, mode="checkpoint")
                with self.assertRaises(review.ReviewError):
                    self._run_success_case(
                        root,
                        arguments,
                        provenance=self.provenance(dirty=False),
                        mutate_binary=mutation,
                        final_provenance=final,
                    )
                self.assertTrue(
                    (output / review.WRAPPER_INCOMPLETE_MARKER).is_file()
                )
                manifest = json.loads((output / review.RUN_MANIFEST).read_text())
                self.assertEqual(manifest["status"], "INCOMPLETE")
                self.assertFalse(manifest["freshness"]["complete"])
                self.assertTrue(manifest["error"])
                if label == "source":
                    self.assertEqual(manifest["provenance_end"], final)
                    self.assertFalse(manifest["freshness"]["source_unchanged"])

    def test_build_failure_retains_marker_and_zero_executed_count(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "build-fail"
            arguments = self.arguments(output, mode="checkpoint")
            provenance = self.provenance(dirty=False)
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review,
                "STRUCTURAL_OUTPUT_ROOT",
                root / "target" / "grand-v3-structural-preview",
            ), mock.patch.object(
                review, "_stable_workspace_provenance", return_value=provenance
            ), mock.patch.object(
                review,
                "_free_space_preflight",
                return_value={"free_bytes": 25 * GIB, "minimum_free_bytes": 20 * GIB},
            ), mock.patch.object(
                review, "_toolchain_identity", return_value=self.toolchain()
            ), mock.patch.object(
                review,
                "_run_logged_process",
                return_value=review.ProcessResult(7, 0.75),
            ):
                with self.assertRaisesRegex(review.ReviewError, "build failed"):
                    review.run_structural(arguments)
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["work"]["executed_count"], 0)
            self.assertEqual(manifest["timings"]["build_seconds"], 0.75)
            self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())

    def test_cancellation_records_failure_and_releases_output_lock(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            review_root = root / "target" / "grand-v3-structural-preview"
            output = review_root / "cancelled"
            arguments = self.arguments(output, mode="checkpoint")
            provenance = self.provenance(dirty=False)
            interruption = review.ReviewInterrupted(review.signal.SIGTERM)
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review, "STRUCTURAL_OUTPUT_ROOT", review_root
            ), mock.patch.object(
                review, "_stable_workspace_provenance", return_value=provenance
            ), mock.patch.object(
                review,
                "_free_space_preflight",
                return_value={"free_bytes": 25 * GIB, "minimum_free_bytes": 20 * GIB},
            ), mock.patch.object(
                review, "_toolchain_identity", return_value=self.toolchain()
            ), mock.patch.object(
                review, "_run_logged_process", side_effect=interruption
            ):
                with self.assertRaises(review.ReviewInterrupted):
                    review.run_structural(arguments)
                manifest = json.loads((output / review.RUN_MANIFEST).read_text())
                self.assertEqual(manifest["status"], "INCOMPLETE")
                self.assertIn("SIGTERM", manifest["error"])
                self.assertTrue((output / review.WRAPPER_INCOMPLETE_MARKER).is_file())
                # A second lock can be acquired only after run_structural cleanup.
                with review._locked_output(output):
                    pass

    def test_timeout_duration_and_started_state_are_retained(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            output = root / "target" / "grand-v3-structural-preview" / "timeout"
            arguments = self.arguments(output, mode="checkpoint")
            provenance = self.provenance(dirty=False)
            timed_out = review.ProcessResult(
                124,
                4.75,
                started=True,
                timed_out=True,
                error="command exceeded 17s and was stopped",
            )
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review,
                "STRUCTURAL_OUTPUT_ROOT",
                root / "target" / "grand-v3-structural-preview",
            ), mock.patch.object(
                review, "_stable_workspace_provenance", return_value=provenance
            ), mock.patch.object(
                review,
                "_free_space_preflight",
                return_value={"free_bytes": 25 * GIB, "minimum_free_bytes": 20 * GIB},
            ), mock.patch.object(
                review, "_toolchain_identity", return_value=self.toolchain()
            ), mock.patch.object(
                review, "_run_logged_process", return_value=timed_out
            ):
                with self.assertRaisesRegex(review.ReviewError, "exceeded 17s"):
                    review.run_structural(arguments)
            manifest = json.loads((output / review.RUN_MANIFEST).read_text())
            self.assertEqual(manifest["timings"]["build_seconds"], 4.75)
            self.assertEqual(manifest["work"]["executed_count"], 0)
            self.assertIn("exceeded 17s", manifest["error"])

    def test_output_is_confined_to_structural_review_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = self.make_repository(directory)
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review,
                "STRUCTURAL_OUTPUT_ROOT",
                root / "target" / "grand-v3-structural-preview",
            ):
                with self.assertRaisesRegex(review.ReviewError, "must stay under"):
                    review._resolve_output(
                        root / "assets" / "bad",
                        mode="author",
                        seed=1,
                        provenance=self.provenance(),
                    )

    def test_prelock_asset_validation_is_not_an_output_attempt(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory) / "repository"
            review_root = root / "target" / "grand-v3-structural-preview"
            output = review_root / "existing"
            output.mkdir(parents=True)
            receipt = output / review.RUN_MANIFEST
            receipt.write_text('{"status":"COMPLETE"}\n', encoding="utf-8")
            arguments = self.arguments(output, mode="author")
            with mock.patch.object(review, "REPOSITORY_ROOT", root), mock.patch.object(
                review, "STRUCTURAL_OUTPUT_ROOT", review_root
            ):
                with self.assertRaisesRegex(review.ReviewError, "asset root is incomplete"):
                    review.run_structural(arguments)
            self.assertEqual(
                receipt.read_text(encoding="utf-8"), '{"status":"COMPLETE"}\n'
            )
            self.assertFalse((output / review.WRAPPER_INCOMPLETE_MARKER).exists())
            self.assertFalse((output / review.OUTPUT_LOCK).exists())


if __name__ == "__main__":
    unittest.main()
