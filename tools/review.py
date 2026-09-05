#!/usr/bin/env python3
"""Fail-closed launch and provenance wrapper for Hex review artifacts.

The first supported workflow is the renderer-free Grand V3 structural preview.  It
has two deliberately different build shapes:

* ``author`` uses the ordinary development profile and incremental compilation for
  the edit loop;
* ``checkpoint`` uses the release profile with incremental compilation disabled.

Both modes are strict unless ``--allow-structural-draft`` is supplied.  Draft output
is always recorded as unapprovable.  The wrapper invalidates prior owned artifacts
before Cargo starts, resolves the exact executable from Cargo JSON, supplies an
explicit asset root, records source and command provenance, and only removes the
wrapper-owned incomplete marker after every freshness check succeeds.  A strict,
clean checkpoint also requires a same-command Cargo no-op verification build.
"""

from __future__ import annotations

import argparse
import contextlib
import errno
import fcntl
import hashlib
import json
import os
import pathlib
import re
import shlex
import signal
import shutil
import stat
import struct
import subprocess
import sys
import time
from dataclasses import dataclass
from typing import Any, Iterator, Mapping, Optional, Sequence


REPOSITORY_ROOT = pathlib.Path(__file__).resolve().parents[1]
STRUCTURAL_OUTPUT_ROOT = REPOSITORY_ROOT / "target" / "grand-v3-structural-preview"
STRUCTURAL_EXAMPLE = "grand_v3_structural_preview"
STRUCTURAL_HERO_SEED = 1_592_598_566
MIN_FREE_GIB = 20
ASSET_SENTINELS = (
    pathlib.Path("assets/config/scenarios.ron"),
    pathlib.Path("assets/config/worlds/procedural-grand-v3-baseline.ron"),
)
RUN_MANIFEST = "review-run.json"
BUILD_LOG = "review-build.log"
VERIFICATION_BUILD_LOG = "review-verification-build.log"
EXECUTION_LOG = "review-execution.log"
RUST_INCOMPLETE_MARKER = "INCOMPLETE.txt"
WRAPPER_INCOMPLETE_MARKER = "REVIEW_INCOMPLETE.txt"
OUTPUT_LOCK = ".review.lock"
STRUCTURAL_OUTPUTS = (
    "height-map.csv",
    "terrain-height-map.pgm",
    "material-height-map.pgm",
    "cross-sections.csv",
    "profiles.svg",
    "waterfall-centerline.csv",
    "waterfall-gorge-width.csv",
    "manifest.txt",
)
OPERATIONAL_OUTPUTS = (
    RUN_MANIFEST,
    BUILD_LOG,
    VERIFICATION_BUILD_LOG,
    EXECUTION_LOG,
)
RUST_INCOMPLETE_NOTICE = (
    "INCOMPLETE — the latest Grand V3 structural preview attempt did not finish.\n"
    "Do not use any other file in this directory as current review evidence.\n"
)
WRAPPER_INCOMPLETE_NOTICE = (
    "INCOMPLETE — the review wrapper has not validated this artifact set.\n"
    "A child process cannot remove this marker. Do not approve this directory.\n"
)
BEHAVIOR_ENV_EXACT = {
    "BEVY_ASSET_ROOT",
    "CARGO_BUILD_RUSTC",
    "CARGO_BUILD_RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_BUILD_TARGET",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "RUSTC",
    "RUSTC_WRAPPER",
    "RUSTC_WORKSPACE_WRAPPER",
    "RUSTC_BOOTSTRAP",
    "RUSTDOCFLAGS",
    "RUSTFLAGS",
    "RUSTUP_TOOLCHAIN",
}
BEHAVIOR_ENV_PREFIXES = (
    "BEVY_",
    "CARGO_PROFILE_",
    "HEX_",
    "WGPU_",
)
SIGNAL_POLL_SECONDS = 0.25
_PENDING_INTERRUPTION: Optional[int] = None


class ReviewError(RuntimeError):
    """A review invocation cannot safely publish its artifacts."""


class ReviewInterrupted(KeyboardInterrupt):
    """A termination signal was converted into ordinary fail-closed cleanup."""

    def __init__(self, signum: int) -> None:
        self.signum = signum
        try:
            name = signal.Signals(signum).name
        except ValueError:
            name = str(signum)
        super().__init__(f"review interrupted by {name}")


def _raise_if_interrupted() -> None:
    global _PENDING_INTERRUPTION
    if _PENDING_INTERRUPTION is None:
        return
    signum = _PENDING_INTERRUPTION
    _PENDING_INTERRUPTION = None
    raise ReviewInterrupted(signum)


@dataclass(frozen=True)
class ProcessResult:
    """Result of one timed child process."""

    returncode: int
    duration_seconds: float
    started: bool = True
    timed_out: bool = False
    error: Optional[str] = None


@dataclass(frozen=True)
class OutputLease:
    """Held advisory lock plus identities used to detect pathname replacement."""

    output: pathlib.Path
    lock_path: pathlib.Path
    lock_fd: int
    output_device: int
    output_inode: int
    lock_device: int
    lock_inode: int

    def validate(self) -> None:
        _validate_output_location(self.output)
        try:
            output_status = self.output.lstat()
            lock_status = self.lock_path.lstat()
            held_status = os.fstat(self.lock_fd)
        except OSError as error:
            raise ReviewError(
                f"structural review output changed while locked: {error}"
            ) from error
        if not stat.S_ISDIR(output_status.st_mode) or self.output.is_symlink():
            raise ReviewError(f"locked review output is no longer a directory: {self.output}")
        if (output_status.st_dev, output_status.st_ino) != (
            self.output_device,
            self.output_inode,
        ):
            raise ReviewError(f"locked review output pathname was replaced: {self.output}")
        expected_lock = (self.lock_device, self.lock_inode)
        if (lock_status.st_dev, lock_status.st_ino) != expected_lock or (
            held_status.st_dev,
            held_status.st_ino,
        ) != expected_lock:
            raise ReviewError(f"review lock pathname was replaced: {self.lock_path}")


def _canonical_json(value: Any) -> str:
    return json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n"


def _atomic_write(path: pathlib.Path, content: str) -> None:
    """Atomically replace a text receipt and durably order it on its directory."""

    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    try:
        temporary.unlink()
    except FileNotFoundError:
        pass
    with temporary.open("w", encoding="utf-8") as destination:
        destination.write(content)
        destination.flush()
        os.fsync(destination.fileno())
    os.replace(temporary, path)
    _fsync_directory(path.parent)


def _fsync_directory(directory: pathlib.Path) -> None:
    flags = os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
    descriptor = os.open(directory, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _durable_unlink(path: pathlib.Path) -> None:
    path.unlink()
    _fsync_directory(path.parent)


def _sha256_file(path: pathlib.Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        while chunk := source.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _durable_regular_file(
    path: pathlib.Path, *, capture_contents: bool = False
) -> tuple[dict[str, Any], Optional[bytes]]:
    """Fsync and hash one no-follow regular file through a single descriptor."""

    flags = os.O_RDONLY | getattr(os, "O_NOFOLLOW", 0) | getattr(os, "O_CLOEXEC", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise ReviewError(f"cannot open required review file {path}: {error}") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise ReviewError(f"required review file is not regular: {path}")
        digest = hashlib.sha256()
        collected: Optional[list[bytes]] = [] if capture_contents else None
        while True:
            chunk = os.read(descriptor, 1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
            if collected is not None:
                collected.append(chunk)
        os.fsync(descriptor)
        after = os.fstat(descriptor)
        if (
            before.st_dev,
            before.st_ino,
            before.st_size,
            before.st_mtime_ns,
        ) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
        ):
            raise ReviewError(f"required review file changed while hashing: {path}")
        return (
            {
                "path": path.name,
                "sha256": digest.hexdigest(),
                "size_bytes": after.st_size,
            },
            b"".join(collected) if collected is not None else None,
        )
    finally:
        os.close(descriptor)


def _framed_update(digest: Any, value: bytes) -> None:
    digest.update(struct.pack(">Q", len(value)))
    digest.update(value)


def _git(repository_root: pathlib.Path, *arguments: str) -> bytes:
    result = subprocess.run(
        ["git", *arguments],
        cwd=repository_root,
        check=False,
        capture_output=True,
    )
    if result.returncode != 0:
        detail = result.stderr.decode("utf-8", errors="replace").strip()
        raise ReviewError(f"git {shlex.join(arguments)} failed: {detail}")
    return result.stdout


def _path_identity(path: pathlib.Path) -> bytes:
    if not path.exists() and not path.is_symlink():
        return b"DELETED"
    if path.is_symlink():
        return b"SYMLINK\0" + os.readlink(path).encode("utf-8")
    if path.is_file():
        return b"FILE\0" + bytes.fromhex(_sha256_file(path))
    return b"NON_FILE"


def _index_override_metadata(repository_root: pathlib.Path) -> dict[str, list[str]]:
    """Return index flags that can hide worktree changes from ordinary status."""

    assume_unchanged: list[str] = []
    skip_worktree: list[str] = []
    records = _git(repository_root, "ls-files", "-v", "-z", "--cached")
    for record in records.split(b"\0"):
        if not record:
            continue
        if len(record) < 3 or record[1:2] != b" ":
            raise ReviewError("git ls-files returned malformed index metadata")
        tag = chr(record[0])
        path = record[2:].decode("utf-8", errors="strict")
        if tag.lower() == "s":
            skip_worktree.append(path)
        if tag.islower():
            assume_unchanged.append(path)
    return {
        "assume_unchanged": sorted(assume_unchanged),
        "skip_worktree": sorted(skip_worktree),
    }


def _workspace_provenance(repository_root: pathlib.Path) -> dict[str, Any]:
    """Return exact HEAD, dirty state, diff hash, and complete content identity."""

    head = _git(repository_root, "rev-parse", "--verify", "HEAD").decode().strip()
    if not re.fullmatch(r"[0-9a-fA-F]{40,64}", head):
        raise ReviewError(f"git returned an invalid HEAD identity: {head!r}")
    head = head.lower()
    status = _git(
        repository_root,
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
    )
    index_overrides = _index_override_metadata(repository_root)
    tracked_diff = _git(repository_root, "diff", "--binary", "HEAD", "--", ".")
    untracked = tuple(
        sorted(
            value
            for value in _git(
                repository_root,
                "ls-files",
                "-z",
                "--others",
                "--exclude-standard",
            ).split(b"\0")
            if value
        )
    )

    diff_digest = hashlib.sha256()
    _framed_update(diff_digest, b"tracked-diff-v1")
    _framed_update(diff_digest, tracked_diff)
    for flag_name in ("assume_unchanged", "skip_worktree"):
        for path in index_overrides[flag_name]:
            _framed_update(diff_digest, flag_name.encode("ascii"))
            _framed_update(diff_digest, path.encode("utf-8"))
    for raw_path in untracked:
        relative = raw_path.decode("utf-8", errors="strict")
        _framed_update(diff_digest, raw_path)
        _framed_update(diff_digest, _path_identity(repository_root / relative))

    listed = tuple(
        sorted(
            value
            for value in _git(
                repository_root,
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ).split(b"\0")
            if value
        )
    )
    content_digest = hashlib.sha256()
    _framed_update(content_digest, b"workspace-content-v1")
    for flag_name in ("assume_unchanged", "skip_worktree"):
        for path in index_overrides[flag_name]:
            _framed_update(content_digest, flag_name.encode("ascii"))
            _framed_update(content_digest, path.encode("utf-8"))
    for raw_path in listed:
        relative = raw_path.decode("utf-8", errors="strict")
        _framed_update(content_digest, raw_path)
        _framed_update(content_digest, _path_identity(repository_root / relative))

    return {
        "git_head": head,
        "worktree_dirty": bool(status)
        or bool(index_overrides["assume_unchanged"])
        or bool(index_overrides["skip_worktree"]),
        "diff_sha256": diff_digest.hexdigest(),
        "workspace_content_sha256": content_digest.hexdigest(),
        "index_overrides": index_overrides,
    }


def _stable_workspace_provenance(
    repository_root: pathlib.Path, attempts: int = 3
) -> dict[str, Any]:
    """Read provenance twice, retrying when a concurrent edit races the snapshot."""

    for _attempt in range(attempts):
        first = _workspace_provenance(repository_root)
        second = _workspace_provenance(repository_root)
        if first == second:
            return first
    raise ReviewError("workspace changed repeatedly while provenance was recorded")


def _sanitized_environment(
    inherited: Mapping[str, str], *, mode: str, allow_structural_draft: bool
) -> dict[str, str]:
    environment = {
        key: value
        for key, value in inherited.items()
        if key not in BEHAVIOR_ENV_EXACT
        and not any(key.startswith(prefix) for prefix in BEHAVIOR_ENV_PREFIXES)
        and not key.startswith("CARGO_TARGET_")
    }
    environment["BEVY_ASSET_ROOT"] = str(REPOSITORY_ROOT)
    environment["CARGO_INCREMENTAL"] = "1" if mode == "author" else "0"
    environment["CARGO_TARGET_DIR"] = str(REPOSITORY_ROOT / "target")
    # Empty values have defined disable semantics in Cargo and take precedence over
    # inherited/global wrapper and rustflag configuration. Other CARGO_HOME settings
    # remain visible for registry/source access and are disclosed in the runbook.
    environment["CARGO_ENCODED_RUSTFLAGS"] = ""
    environment["CARGO_BUILD_RUSTFLAGS"] = ""
    environment["CARGO_BUILD_RUSTC_WRAPPER"] = ""
    environment["CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER"] = ""
    environment["RUSTFLAGS"] = ""
    environment["RUSTC_WRAPPER"] = ""
    environment["RUSTC_WORKSPACE_WRAPPER"] = ""
    if allow_structural_draft:
        environment["HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT"] = "1"
    return environment


def _structural_build_command(
    mode: str, *, allow_structural_draft: bool
) -> tuple[str, ...]:
    if mode not in {"author", "checkpoint"}:
        raise ReviewError(f"unknown structural review mode: {mode}")
    command = [
        "cargo",
        "build",
        "-p",
        "hex_map",
        "--example",
        STRUCTURAL_EXAMPLE,
    ]
    if mode == "checkpoint":
        command.append("--release")
    if allow_structural_draft:
        command.extend(("--features", "map-review"))
    command.append("--message-format=json-render-diagnostics")
    return tuple(command)


def _terminate_process(process: subprocess.Popen[Any]) -> None:
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            return
    else:
        process.terminate()
    try:
        process.wait(timeout=5)
        return
    except subprocess.TimeoutExpired:
        pass
    if os.name == "posix":
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            return
    else:
        process.kill()
    process.wait(timeout=5)


def _run_logged_process(
    command: Sequence[str],
    *,
    cwd: pathlib.Path,
    environment: Mapping[str, str],
    log_path: pathlib.Path,
    timeout_seconds: int,
    pass_fds: Sequence[int] = (),
) -> ProcessResult:
    started_at = time.monotonic()
    log_path.parent.mkdir(parents=True, exist_ok=True)
    try:
        log = log_path.open("wb")
    except OSError as error:
        return ProcessResult(
            returncode=127,
            duration_seconds=round(time.monotonic() - started_at, 3),
            started=False,
            error=f"cannot open process log {log_path}: {error}",
        )
    with log:
        try:
            process = subprocess.Popen(
                list(command),
                cwd=cwd,
                env=dict(environment),
                stdin=subprocess.DEVNULL,
                stdout=log,
                stderr=subprocess.STDOUT,
                start_new_session=os.name == "posix",
                pass_fds=tuple(pass_fds) if os.name == "posix" else (),
            )
        except OSError as error:
            return ProcessResult(
                returncode=127,
                duration_seconds=round(time.monotonic() - started_at, 3),
                started=False,
                error=f"cannot launch {shlex.join(command)}: {error}",
            )
        deadline = started_at + timeout_seconds
        try:
            while True:
                _raise_if_interrupted()
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    _terminate_process(process)
                    return ProcessResult(
                        returncode=124,
                        duration_seconds=round(time.monotonic() - started_at, 3),
                        started=True,
                        timed_out=True,
                        error=f"command exceeded {timeout_seconds}s and was stopped",
                    )
                try:
                    returncode = process.wait(
                        timeout=min(SIGNAL_POLL_SECONDS, remaining)
                    )
                    break
                except subprocess.TimeoutExpired:
                    continue
        except BaseException:
            try:
                # The child owns a new session, so wrapper cancellation must stop
                # the complete Cargo/generator process group before cleanup runs.
                _terminate_process(process)
            except BaseException:
                pass
            raise
    return ProcessResult(
        returncode=returncode,
        duration_seconds=round(time.monotonic() - started_at, 3),
    )


def _capture_identity_command(
    command: Sequence[str],
    *,
    cwd: pathlib.Path,
    environment: Mapping[str, str],
    timeout_seconds: int = 30,
    pass_fds: Sequence[int] = (),
) -> dict[str, Any]:
    """Capture exact version output without invoking a build or generator."""

    started_at = time.monotonic()
    try:
        process = subprocess.Popen(
            list(command),
            cwd=cwd,
            env=dict(environment),
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=os.name == "posix",
            pass_fds=tuple(pass_fds) if os.name == "posix" else (),
        )
    except OSError as error:
        return {
            "command": list(command),
            "started": False,
            "returncode": 127,
            "timed_out": False,
            "duration_seconds": round(time.monotonic() - started_at, 3),
            "stdout": "",
            "stderr": str(error),
        }
    deadline = started_at + timeout_seconds
    try:
        while True:
            _raise_if_interrupted()
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                _terminate_process(process)
                return {
                    "command": list(command),
                    "started": True,
                    "returncode": 124,
                    "timed_out": True,
                    "duration_seconds": round(time.monotonic() - started_at, 3),
                    "stdout": "",
                    "stderr": (
                        f"identity command exceeded {timeout_seconds}s and was stopped"
                    ),
                }
            try:
                stdout, stderr = process.communicate(
                    timeout=min(SIGNAL_POLL_SECONDS, remaining)
                )
                break
            except subprocess.TimeoutExpired:
                continue
    except BaseException:
        try:
            _terminate_process(process)
        except BaseException:
            pass
        raise
    return {
        "command": list(command),
        "started": True,
        "returncode": process.returncode,
        "timed_out": False,
        "duration_seconds": round(time.monotonic() - started_at, 3),
        "stdout": stdout.decode("utf-8", errors="replace"),
        "stderr": stderr.decode("utf-8", errors="replace"),
    }


def _toolchain_identity(
    environment: Mapping[str, str], *, pass_fds: Sequence[int]
) -> dict[str, Any]:
    evidence = {
        "cargo": _capture_identity_command(
            ("cargo", "--version", "--verbose"),
            cwd=REPOSITORY_ROOT,
            environment=environment,
            pass_fds=pass_fds,
        ),
        "rustc": _capture_identity_command(
            ("rustc", "--version", "--verbose"),
            cwd=REPOSITORY_ROOT,
            environment=environment,
            pass_fds=pass_fds,
        ),
        "rustup": _capture_identity_command(
            ("rustup", "show", "active-toolchain"),
            cwd=REPOSITORY_ROOT,
            environment=environment,
            pass_fds=pass_fds,
        ),
    }
    errors = []
    for name in ("cargo", "rustc"):
        result = evidence[name]
        stdout = result["stdout"]
        if result["returncode"] != 0:
            errors.append(f"{name} identity command exited {result['returncode']}")
            continue
        if not stdout.startswith(name + " ") or "\nrelease:" not in stdout or "\nhost:" not in stdout:
            errors.append(f"{name} identity output was incomplete")
    rustup = evidence["rustup"]
    rustup["available"] = bool(
        rustup["returncode"] == 0 and rustup["stdout"].strip()
    )
    return {
        "status": "FAILED" if errors else "VERIFIED",
        "errors": errors,
        **evidence,
    }


def _resolve_structural_binary(build_log: pathlib.Path) -> pathlib.Path:
    executables: list[pathlib.Path] = []
    for line in build_log.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        target = event.get("target")
        executable = event.get("executable")
        kinds = target.get("kind") if isinstance(target, dict) else None
        if (
            event.get("reason") == "compiler-artifact"
            and isinstance(target, dict)
            and target.get("name") == STRUCTURAL_EXAMPLE
            and isinstance(kinds, list)
            and "example" in kinds
            and isinstance(executable, str)
        ):
            candidate = pathlib.Path(executable)
            if not candidate.is_absolute():
                raise ReviewError(f"Cargo executable path must be absolute: {candidate}")
            if candidate.is_symlink():
                raise ReviewError(f"Cargo executable must not be a symlink: {candidate}")
            executables.append(candidate.resolve())
    unique = tuple(dict.fromkeys(executables))
    if len(unique) != 1:
        raise ReviewError(
            "Cargo did not report exactly one Grand V3 structural preview executable"
        )
    executable = unique[0]
    try:
        mode = executable.lstat().st_mode
    except OSError as error:
        raise ReviewError(f"cannot inspect Cargo executable {executable}: {error}") from error
    if not stat.S_ISREG(mode) or executable.is_symlink():
        raise ReviewError(f"Cargo executable is not a regular file: {executable}")
    return executable


def _resolve_output(
    requested: Optional[pathlib.Path],
    *,
    mode: str,
    seed: int,
    provenance: Mapping[str, Any],
) -> pathlib.Path:
    if requested is None:
        dirty = (
            "dirty-" + str(provenance["diff_sha256"])[:12]
            if provenance["worktree_dirty"]
            else "clean"
        )
        requested = (
            STRUCTURAL_OUTPUT_ROOT
            / mode
            / str(provenance["git_head"])
            / dirty
            / f"seed-{seed}"
        )
    elif not requested.is_absolute():
        requested = REPOSITORY_ROOT / requested
    unresolved = requested
    probe = unresolved
    while probe != probe.parent and not probe.exists() and not probe.is_symlink():
        probe = probe.parent
    if probe.is_symlink():
        raise ReviewError(f"structural review output has a symlink ancestor: {probe}")
    resolved = unresolved.resolve(strict=False)
    allowed = STRUCTURAL_OUTPUT_ROOT.resolve(strict=False)
    try:
        relative = resolved.relative_to(allowed)
    except ValueError as error:
        raise ReviewError(
            f"structural review output must stay under {allowed}: {resolved}"
        ) from error
    if not relative.parts:
        raise ReviewError("structural review output must name a directory below its root")
    # Keep the lexical pathname so lock-held ancestry checks can still see any
    # symlink introduced after this initial resolved-confinement check.
    return unresolved.absolute()


def _validate_output_location(output: pathlib.Path) -> None:
    """Revalidate lexical confinement and reject symlinks inside the repository."""

    repository = REPOSITORY_ROOT.absolute()
    allowed = STRUCTURAL_OUTPUT_ROOT.absolute()
    candidate = output.absolute()
    try:
        candidate.relative_to(allowed)
        relative_to_repository = candidate.relative_to(repository)
    except ValueError as error:
        raise ReviewError(
            f"structural review output escaped its repository root: {candidate}"
        ) from error

    current = repository
    paths = [repository]
    for component in relative_to_repository.parts:
        current = current / component
        paths.append(current)
    for path in paths:
        try:
            mode = path.lstat().st_mode
        except OSError as error:
            raise ReviewError(f"cannot validate review output ancestor {path}: {error}") from error
        if stat.S_ISLNK(mode):
            raise ReviewError(f"structural review output has a symlink ancestor: {path}")
        if not stat.S_ISDIR(mode):
            raise ReviewError(f"structural review output ancestor is not a directory: {path}")

    try:
        candidate.resolve(strict=True).relative_to(allowed.resolve(strict=True))
    except (OSError, ValueError) as error:
        raise ReviewError(
            f"structural review output failed resolved confinement: {candidate}"
        ) from error


@contextlib.contextmanager
def _locked_output(output: pathlib.Path) -> Iterator[OutputLease]:
    """Hold a nonblocking advisory lock for one output directory."""

    try:
        output.mkdir(parents=True, exist_ok=True)
    except OSError as error:
        raise ReviewError(f"cannot create structural review output {output}: {error}") from error
    _validate_output_location(output)
    lock_path = output / OUTPUT_LOCK
    flags = os.O_RDWR | os.O_CREAT | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(lock_path, flags, 0o644)
    except OSError as error:
        raise ReviewError(f"cannot open structural review lock {lock_path}: {error}") from error
    try:
        try:
            lock_status = os.fstat(descriptor)
            if not stat.S_ISREG(lock_status.st_mode):
                raise ReviewError(f"structural review lock is not a regular file: {lock_path}")
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as error:
            if error.errno in (errno.EACCES, errno.EAGAIN, errno.EWOULDBLOCK):
                raise ReviewError(
                    f"another review invocation already owns {output}"
                ) from error
            raise ReviewError(f"cannot lock structural review output {output}: {error}") from error

        os.ftruncate(descriptor, 0)
        os.write(descriptor, f"pid={os.getpid()}\n".encode("ascii"))
        os.fsync(descriptor)
        output_status = output.lstat()
        lock_status = os.fstat(descriptor)
        lease = OutputLease(
            output=output,
            lock_path=lock_path,
            lock_fd=descriptor,
            output_device=output_status.st_dev,
            output_inode=output_status.st_ino,
            lock_device=lock_status.st_dev,
            lock_inode=lock_status.st_ino,
        )
        lease.validate()
        yield lease
    finally:
        # Do not issue LOCK_UN: children inherit this open file description. Closing
        # only our descriptor preserves serialization if an unkillable/orphaned
        # child is still writing after wrapper cleanup or wrapper SIGKILL.
        os.close(descriptor)


def _invalidate_structural_output(output: pathlib.Path) -> None:
    output.mkdir(parents=True, exist_ok=True)
    _atomic_write(output / WRAPPER_INCOMPLETE_MARKER, WRAPPER_INCOMPLETE_NOTICE)
    _atomic_write(output / RUST_INCOMPLETE_MARKER, RUST_INCOMPLETE_NOTICE)
    for name in (*STRUCTURAL_OUTPUTS, *OPERATIONAL_OUTPUTS):
        path = output / name
        try:
            if path.is_dir() and not path.is_symlink():
                raise ReviewError(f"owned review output unexpectedly names a directory: {path}")
            path.unlink()
        except FileNotFoundError:
            pass


def _free_space_preflight(output: pathlib.Path, minimum_gib: int) -> dict[str, int]:
    """Refuse an expensive build when the destination filesystem is too full."""

    if minimum_gib < MIN_FREE_GIB:
        raise ReviewError(
            f"minimum free-space policy cannot be lower than {MIN_FREE_GIB} GiB"
        )
    probe = output
    while not probe.exists() and probe != probe.parent:
        probe = probe.parent
    try:
        free_bytes = shutil.disk_usage(probe).free
    except OSError as error:
        raise ReviewError(f"cannot inspect free space for {probe}: {error}") from error
    minimum_bytes = minimum_gib * 1024 * 1024 * 1024
    if free_bytes < minimum_bytes:
        available_gib = free_bytes / (1024**3)
        raise ReviewError(
            f"only {available_gib:.1f} GiB is free on {probe}; structural review "
            f"requires at least {minimum_gib} GiB before Cargo. Remove or relocate "
            "reproducible inactive build/review artifacts, then retry."
        )
    return {"free_bytes": free_bytes, "minimum_free_bytes": minimum_bytes}


def _ensure_incomplete(output: pathlib.Path) -> None:
    _atomic_write(output / WRAPPER_INCOMPLETE_MARKER, WRAPPER_INCOMPLETE_NOTICE)


def _artifact_records(output: pathlib.Path, seed: int) -> list[dict[str, Any]]:
    records = []
    manifest_bytes: Optional[bytes] = None
    for name in STRUCTURAL_OUTPUTS:
        path = output / name
        record, contents = _durable_regular_file(
            path, capture_contents=name == "manifest.txt"
        )
        records.append(record)
        if name == "manifest.txt":
            manifest_bytes = contents
    if manifest_bytes is None:
        raise ReviewError("structural manifest was not captured")
    try:
        manifest = manifest_bytes.decode("utf-8", errors="strict")
    except UnicodeDecodeError as error:
        raise ReviewError("structural manifest is not valid UTF-8") from error
    fields: dict[str, list[str]] = {}
    for line_number, line in enumerate(manifest.splitlines(), start=1):
        if not line:
            continue
        key, separator, value = line.partition("=")
        if not separator or not key or key.strip() != key:
            raise ReviewError(
                f"structural manifest has malformed record on line {line_number}"
            )
        fields.setdefault(key, []).append(value)
    if fields.get("grand_v3_structural_preview_version") != ["1"]:
        raise ReviewError("structural manifest schema must be exactly version 1")
    if fields.get("seed") != [str(seed)]:
        raise ReviewError(f"structural manifest does not exactly describe seed {seed}")
    return records


def _operational_log_records(
    output: pathlib.Path, *, verification_required: bool
) -> list[dict[str, Any]]:
    names = [BUILD_LOG, EXECUTION_LOG]
    if verification_required:
        names.append(VERIFICATION_BUILD_LOG)
    records = []
    for name in names:
        record, _contents = _durable_regular_file(output / name)
        records.append(record)
    _fsync_directory(output)
    return records


def _reject_known_diagnostics(*logs: pathlib.Path) -> None:
    for log in logs:
        try:
            contents = log.read_text(encoding="utf-8", errors="replace")
        except OSError as error:
            raise ReviewError(f"cannot inspect review diagnostic log {log}: {error}") from error
        if re.search(r"\bpath not found\b", contents, flags=re.IGNORECASE):
            raise ReviewError(f"review log contains a Path not found diagnostic: {log}")


def _require_noop_build(build_log: pathlib.Path) -> None:
    artifacts: list[dict[str, Any]] = []
    exact_target_fresh = False
    for line in build_log.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get("reason") != "compiler-artifact":
            continue
        artifacts.append(event)
        target = event.get("target")
        if (
            isinstance(target, dict)
            and target.get("name") == STRUCTURAL_EXAMPLE
            and "example" in target.get("kind", [])
            and event.get("fresh") is True
        ):
            exact_target_fresh = True
    if not artifacts or any(event.get("fresh") is not True for event in artifacts):
        raise ReviewError("checkpoint verification build was not a complete Cargo no-op")
    if not exact_target_fresh:
        raise ReviewError("checkpoint verification did not report the example as fresh")


def _tokenized_path(path: pathlib.Path, output: pathlib.Path) -> str:
    if path == output:
        return "$OUTPUT"
    try:
        return "$REPOSITORY_ROOT/" + path.relative_to(REPOSITORY_ROOT).as_posix()
    except ValueError:
        return str(path)


def _base_manifest(
    *,
    arguments: argparse.Namespace,
    output: pathlib.Path,
    provenance: Mapping[str, Any],
    build_command: Sequence[str],
    resource_preflight: Mapping[str, Any],
) -> dict[str, Any]:
    admission = "structural-draft" if arguments.allow_structural_draft else "strict"
    reasons = ["incomplete"]
    if arguments.mode == "author":
        reasons.append("authoring-profile")
    if arguments.allow_structural_draft:
        reasons.append("structural-draft")
    if provenance["worktree_dirty"]:
        reasons.append("dirty-worktree")
    return {
        "schema_version": 2,
        "status": "INCOMPLETE",
        "workflow": "grand-v3-structural-review",
        "workflow_mode": arguments.mode,
        "admission_mode": admission,
        "approvable": False,
        "unapprovable_reasons": reasons,
        "seed": arguments.seed,
        "output": _tokenized_path(output, output),
        "incomplete_markers": {
            "wrapper": WRAPPER_INCOMPLETE_MARKER,
            "producer": RUST_INCOMPLETE_MARKER,
        },
        "provenance_start": dict(provenance),
        "provenance_end": None,
        "provenance_observations": [],
        "commands": {
            "entrypoint": [
                "python3",
                "tools/review.py",
                "structural",
                arguments.mode,
                "--seed",
                str(arguments.seed),
                "--output",
                "$OUTPUT",
                "--build-timeout-seconds",
                str(arguments.build_timeout_seconds),
                "--run-timeout-seconds",
                str(arguments.run_timeout_seconds),
                "--min-free-gib",
                str(arguments.min_free_gib),
                *(
                    ["--allow-structural-draft"]
                    if arguments.allow_structural_draft
                    else []
                ),
            ],
            "build": list(build_command),
            "run": None,
            "verification_build": None,
        },
        "environment": {
            "BEVY_ASSET_ROOT": "$REPOSITORY_ROOT",
            "CARGO_INCREMENTAL": "1" if arguments.mode == "author" else "0",
            "CARGO_TARGET_DIR": "$REPOSITORY_ROOT/target",
            "CARGO_ENCODED_RUSTFLAGS": "",
            "CARGO_BUILD_RUSTFLAGS": "",
            "CARGO_BUILD_RUSTC_WRAPPER": "",
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER": "",
            "RUSTFLAGS": "",
            "RUSTC_WRAPPER": "",
            "RUSTC_WORKSPACE_WRAPPER": "",
            "HEX_GRAND_V3_STRUCTURAL_REVIEW_DRAFT": (
                "1" if arguments.allow_structural_draft else "NOT-SET"
            ),
        },
        "binary": None,
        "timings": {
            "build_seconds": None,
            "run_seconds": None,
            "verification_build_seconds": None,
            "total_seconds": None,
        },
        "work": {"selected_count": 1, "executed_count": 0},
        "artifacts": [],
        "logs": [],
        "freshness": {
            "owned_outputs_invalidated_before_build": True,
            "source_unchanged": False,
            "binary_unchanged": False,
            "output_location_unchanged": False,
            "producer_complete": False,
            "expected_artifact_count": len(STRUCTURAL_OUTPUTS),
            "actual_artifact_count": 0,
            "complete": False,
        },
        "verification": {
            "required": (
                arguments.mode == "checkpoint"
                and not arguments.allow_structural_draft
                and not provenance["worktree_dirty"]
            ),
            "performed": False,
            "no_op": False,
            "binary_unchanged": False,
        },
        "toolchain": {"status": "PENDING", "cargo": None, "rustc": None, "rustup": None},
        "resource_preflight": dict(resource_preflight),
        "error": None,
    }


def _record_failure(
    output: pathlib.Path,
    manifest: dict[str, Any],
    error: BaseException,
    started_at: float,
) -> None:
    _ensure_incomplete(output)
    manifest["status"] = "INCOMPLETE"
    manifest["approvable"] = False
    manifest["unapprovable_reasons"] = sorted(
        set((*manifest.get("unapprovable_reasons", []), "incomplete"))
    )
    manifest["timings"]["total_seconds"] = round(time.monotonic() - started_at, 3)
    manifest["freshness"]["complete"] = False
    manifest["error"] = str(error)
    _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))


def _require_unchanged_provenance(
    manifest: dict[str, Any],
    expected: Mapping[str, Any],
    *,
    stage: str,
) -> dict[str, Any]:
    """Record the observed snapshot before accepting or rejecting it."""

    try:
        observed = _stable_workspace_provenance(REPOSITORY_ROOT)
    except (ReviewInterrupted, SystemExit):
        raise
    except ReviewError as unstable:
        # Even an unstable workspace gets a concrete last observation in the
        # failure receipt; it is never allowed to inherit an earlier true flag.
        try:
            observed = _workspace_provenance(REPOSITORY_ROOT)
        except ReviewError as observation_error:
            manifest["provenance_end"] = {
                "unavailable": str(observation_error),
            }
            manifest["provenance_observations"].append(
                {
                    "stage": stage,
                    "matches_start": False,
                    "snapshot": manifest["provenance_end"],
                }
            )
            manifest["freshness"]["source_unchanged"] = False
            raise ReviewError(f"workspace was unstable during {stage}: {unstable}") from unstable
        manifest["provenance_end"] = dict(observed)
        manifest["provenance_observations"].append(
            {"stage": stage, "matches_start": False, "snapshot": dict(observed)}
        )
        manifest["freshness"]["source_unchanged"] = False
        raise ReviewError(f"workspace was unstable during {stage}: {unstable}") from unstable
    matches = observed == expected
    manifest["provenance_end"] = dict(observed)
    manifest["provenance_observations"].append(
        {"stage": stage, "matches_start": matches, "snapshot": dict(observed)}
    )
    manifest["freshness"]["source_unchanged"] = matches
    if not matches:
        raise ReviewError(f"HEAD or worktree content changed during {stage}")
    return observed


@contextlib.contextmanager
def _cli_signal_handlers() -> Iterator[None]:
    """Defer HUP/TERM to explicit boundaries so durable cleanup is not interrupted."""

    global _PENDING_INTERRUPTION
    handled = [signal.SIGINT, signal.SIGTERM]
    if hasattr(signal, "SIGHUP"):
        handled.append(signal.SIGHUP)
    previous: dict[signal.Signals, Any] = {}

    def interrupt(signum: int, _frame: Any) -> None:
        global _PENDING_INTERRUPTION
        _PENDING_INTERRUPTION = signum

    try:
        _PENDING_INTERRUPTION = None
        for signum in handled:
            previous[signum] = signal.getsignal(signum)
            signal.signal(signum, interrupt)
        yield
    finally:
        for signum, handler in previous.items():
            signal.signal(signum, handler)
        _PENDING_INTERRUPTION = None


def run_structural(arguments: argparse.Namespace) -> int:
    """Run one author or checkpoint structural review invocation."""

    if arguments.seed < 0 or arguments.seed > (2**64 - 1):
        raise ReviewError("seed must fit an unsigned 64-bit integer")
    if arguments.build_timeout_seconds <= 0 or arguments.run_timeout_seconds <= 0:
        raise ReviewError("timeouts must be positive")
    missing_assets = [
        path.as_posix()
        for path in ASSET_SENTINELS
        if not (REPOSITORY_ROOT / path).is_file()
    ]
    if missing_assets:
        raise ReviewError(
            "repository asset root is incomplete; missing " + ", ".join(missing_assets)
        )

    started_at = time.monotonic()
    provenance = _stable_workspace_provenance(REPOSITORY_ROOT)
    _raise_if_interrupted()
    output = _resolve_output(
        arguments.output,
        mode=arguments.mode,
        seed=arguments.seed,
        provenance=provenance,
    )
    build_command = _structural_build_command(
        arguments.mode,
        allow_structural_draft=arguments.allow_structural_draft,
    )
    manifest = _base_manifest(
        arguments=arguments,
        output=output,
        provenance=provenance,
        build_command=build_command,
        resource_preflight={
            "status": "PENDING",
            "minimum_free_bytes": arguments.min_free_gib * 1024 * 1024 * 1024,
        },
    )
    environment = _sanitized_environment(
        os.environ,
        mode=arguments.mode,
        allow_structural_draft=arguments.allow_structural_draft,
    )
    with _locked_output(output) as lease:
        try:
            _raise_if_interrupted()
            lease.validate()
            _invalidate_structural_output(output)
            _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))
            _raise_if_interrupted()

            try:
                free_space = _free_space_preflight(output, arguments.min_free_gib)
            except BaseException:
                manifest["resource_preflight"]["status"] = "FAILED"
                raise
            manifest["resource_preflight"] = {"status": "PASSED", **free_space}
            _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))
            _raise_if_interrupted()

            lease.validate()
            manifest["toolchain"] = _toolchain_identity(
                environment, pass_fds=(lease.lock_fd,)
            )
            _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))
            _raise_if_interrupted()
            if manifest["toolchain"]["status"] != "VERIFIED":
                raise ReviewError(
                    "cannot establish cargo/rustc identity: "
                    + "; ".join(manifest["toolchain"]["errors"])
                )

            lease.validate()
            build_started_at = time.monotonic()
            try:
                build = _run_logged_process(
                    build_command,
                    cwd=REPOSITORY_ROOT,
                    environment=environment,
                    log_path=output / BUILD_LOG,
                    timeout_seconds=arguments.build_timeout_seconds,
                    pass_fds=(lease.lock_fd,),
                )
            except BaseException:
                manifest["timings"]["build_seconds"] = round(
                    time.monotonic() - build_started_at, 3
                )
                raise
            manifest["timings"]["build_seconds"] = build.duration_seconds
            if build.returncode != 0:
                detail = build.error or f"exit code {build.returncode}"
                raise ReviewError(f"structural review build failed: {detail}")
            _raise_if_interrupted()
            lease.validate()
            _reject_known_diagnostics(output / BUILD_LOG)

            executable = _resolve_structural_binary(output / BUILD_LOG)
            binary_sha256 = _sha256_file(executable)
            run_command = (
                str(executable),
                "--seed",
                str(arguments.seed),
                "--output",
                str(output),
            )
            manifest["commands"]["run"] = [
                "$STRUCTURAL_PREVIEW_BINARY",
                "--seed",
                str(arguments.seed),
                "--output",
                "$OUTPUT",
            ]
            manifest["binary"] = {
                "path": _tokenized_path(executable, output),
                "sha256": binary_sha256,
            }
            _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))

            lease.validate()
            run_started_at = time.monotonic()
            manifest["work"]["executed_count"] = 1
            try:
                run = _run_logged_process(
                    run_command,
                    cwd=REPOSITORY_ROOT,
                    environment=environment,
                    log_path=output / EXECUTION_LOG,
                    timeout_seconds=arguments.run_timeout_seconds,
                    pass_fds=(lease.lock_fd,),
                )
            except BaseException:
                manifest["timings"]["run_seconds"] = round(
                    time.monotonic() - run_started_at, 3
                )
                raise
            manifest["work"]["executed_count"] = 1 if run.started else 0
            manifest["timings"]["run_seconds"] = run.duration_seconds
            if run.returncode != 0:
                detail = run.error or f"exit code {run.returncode}"
                raise ReviewError(f"structural review execution failed: {detail}")
            _raise_if_interrupted()

            # This check deliberately precedes artifact parsing: producer success is
            # only credible when the Rust-owned marker was removed by the producer.
            lease.validate()
            producer_marker = output / RUST_INCOMPLETE_MARKER
            if producer_marker.exists() or producer_marker.is_symlink():
                raise ReviewError(
                    "structural preview exited successfully but retained its INCOMPLETE marker"
                )
            manifest["freshness"]["producer_complete"] = True
            wrapper_marker = output / WRAPPER_INCOMPLETE_MARKER
            if not wrapper_marker.is_file() or wrapper_marker.is_symlink():
                raise ReviewError("review wrapper incomplete marker disappeared during execution")
            _reject_known_diagnostics(output / EXECUTION_LOG)
            artifacts = _artifact_records(output, arguments.seed)
            if _sha256_file(executable) != binary_sha256:
                manifest["freshness"]["binary_unchanged"] = False
                raise ReviewError("structural preview executable changed during the run")
            manifest["freshness"]["binary_unchanged"] = True
            _require_unchanged_provenance(
                manifest, provenance, stage="structural generation"
            )

            if manifest["verification"]["required"]:
                lease.validate()
                manifest["commands"]["verification_build"] = list(build_command)
                verification_started_at = time.monotonic()
                try:
                    verification = _run_logged_process(
                        build_command,
                        cwd=REPOSITORY_ROOT,
                        environment=environment,
                        log_path=output / VERIFICATION_BUILD_LOG,
                        timeout_seconds=arguments.build_timeout_seconds,
                        pass_fds=(lease.lock_fd,),
                    )
                except BaseException:
                    manifest["timings"]["verification_build_seconds"] = round(
                        time.monotonic() - verification_started_at, 3
                    )
                    raise
                manifest["verification"]["performed"] = verification.started
                manifest["timings"][
                    "verification_build_seconds"
                ] = verification.duration_seconds
                if verification.returncode != 0:
                    detail = verification.error or f"exit code {verification.returncode}"
                    raise ReviewError(f"checkpoint verification build failed: {detail}")
                _raise_if_interrupted()
                lease.validate()
                _reject_known_diagnostics(output / VERIFICATION_BUILD_LOG)
                _require_noop_build(output / VERIFICATION_BUILD_LOG)
                manifest["verification"]["no_op"] = True
                verified_executable = _resolve_structural_binary(
                    output / VERIFICATION_BUILD_LOG
                )
                if verified_executable != executable:
                    raise ReviewError("checkpoint verification resolved a different executable")
                if _sha256_file(verified_executable) != binary_sha256:
                    manifest["freshness"]["binary_unchanged"] = False
                    raise ReviewError("checkpoint verification executable hash changed")
                manifest["verification"]["binary_unchanged"] = True
                _require_unchanged_provenance(
                    manifest, provenance, stage="checkpoint verification"
                )

            lease.validate()
            # A verification build is allowed to execute build scripts, so hash the
            # deterministic evidence again after that final Cargo boundary.
            final_artifacts = _artifact_records(output, arguments.seed)
            if final_artifacts != artifacts:
                raise ReviewError("structural artifacts changed during final verification")
            manifest["artifacts"] = final_artifacts
            manifest["logs"] = _operational_log_records(
                output,
                verification_required=manifest["verification"]["required"],
            )
            manifest["freshness"]["actual_artifact_count"] = len(final_artifacts)
            manifest["freshness"]["complete"] = True
            manifest["status"] = "COMPLETE"
            manifest["error"] = None
            reasons = []
            if arguments.mode == "author":
                reasons.append("authoring-profile")
            if arguments.allow_structural_draft:
                reasons.append("structural-draft")
            if provenance["worktree_dirty"]:
                reasons.append("dirty-worktree")
            manifest["unapprovable_reasons"] = reasons
            manifest["approvable"] = not reasons

            # The final observation and path/binary checks are installed in the
            # durable COMPLETE receipt before the wrapper marker is removed.
            lease.validate()
            manifest["freshness"]["output_location_unchanged"] = True
            _require_unchanged_provenance(
                manifest, provenance, stage="structural publication boundary"
            )
            if _sha256_file(executable) != binary_sha256:
                manifest["freshness"]["binary_unchanged"] = False
                raise ReviewError("binary changed at the structural publication boundary")
            if producer_marker.exists() or producer_marker.is_symlink():
                manifest["freshness"]["producer_complete"] = False
                raise ReviewError("producer incomplete marker reappeared before publication")
            if not wrapper_marker.is_file() or wrapper_marker.is_symlink():
                raise ReviewError("wrapper incomplete marker changed before publication")
            _raise_if_interrupted()
            lease.validate()
            manifest["timings"]["total_seconds"] = round(
                time.monotonic() - started_at, 3
            )
            _atomic_write(output / RUN_MANIFEST, _canonical_json(manifest))
            _raise_if_interrupted()
            lease.validate()
            _durable_unlink(wrapper_marker)
            _raise_if_interrupted()
            print(f"structural review complete: {output}")
            print(f"operational manifest: {output / RUN_MANIFEST}")
            return 0
        except BaseException as error:
            try:
                lease.validate()
            except ReviewError as unsafe_output:
                raise ReviewError(
                    f"{error}; cannot safely update failure receipt: {unsafe_output}"
                ) from error
            try:
                _record_failure(output, manifest, error, started_at)
            except OSError as manifest_error:
                raise ReviewError(
                    f"{error}; additionally could not retain incomplete manifest: {manifest_error}"
                ) from error
            if isinstance(error, (KeyboardInterrupt, SystemExit)):
                raise
            raise ReviewError(str(error)) from error


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    structural = subparsers.add_parser(
        "structural", help="run the renderer-free Grand V3 structural review"
    )
    structural.add_argument(
        "mode",
        choices=("author", "checkpoint"),
        help="development-profile author loop or release/nonincremental checkpoint",
    )
    structural.add_argument("--seed", type=int, default=STRUCTURAL_HERO_SEED)
    structural.add_argument(
        "--output",
        type=pathlib.Path,
        help=(
            "output directory below target/grand-v3-structural-preview; default is "
            "content-addressed by mode, HEAD, diff, and seed"
        ),
    )
    structural.add_argument(
        "--allow-structural-draft",
        action="store_true",
        help="explicitly enable diagnostic draft bypasses; output is unapprovable",
    )
    structural.add_argument("--build-timeout-seconds", type=int, default=3600)
    structural.add_argument("--run-timeout-seconds", type=int, default=1800)
    structural.add_argument(
        "--min-free-gib",
        type=int,
        default=MIN_FREE_GIB,
        help=f"fail before Cargo below this threshold (minimum {MIN_FREE_GIB} GiB)",
    )
    return parser


def main() -> int:
    arguments = build_parser().parse_args()
    try:
        with _cli_signal_handlers():
            if arguments.command == "structural":
                result = run_structural(arguments)
                _raise_if_interrupted()
                return result
            raise ReviewError(f"unsupported review command: {arguments.command}")
    except ReviewInterrupted as error:
        print(f"review interrupted: {error}", file=sys.stderr)
        return 128 + error.signum
    except KeyboardInterrupt:
        print("review interrupted by SIGINT", file=sys.stderr)
        return 130
    except ReviewError as error:
        print(f"review error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
